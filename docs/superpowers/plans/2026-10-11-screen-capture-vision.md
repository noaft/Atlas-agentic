# Screen Capture Vision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an approved `capture_screen` tool whose in-memory primary-display PNG is delivered to Atlas as real model image input.

**Architecture:** A shared provider-neutral image value travels from `ComputerController` through `ToolResult`, the runtime's turn-local latest-image state, `LocalAgent`, and `ModelRequest`. The DeepSeek adapter alone converts bytes to a base64 data URL and emits mixed text/image content; screenshots never enter textual continuations, logs, disk, or saved conversation history.

**Tech Stack:** Rust, serde/serde_json, reqwest, base64, screenshots-rs, image/PNG encoding, existing Atlas tool/runtime/provider boundaries.

**Spec:** `docs/superpowers/specs/2026-10-11-screen-capture-vision-design.md`

## Global Constraints

- Register exactly one new default tool named `capture_screen`, accepting only `{}` and targeting the primary display.
- Use `ToolRisk::ComputerObserve`; Ask and Workspace confirm, while Full Access retains its existing bypass behavior.
- Keep screenshots in memory only, retain at most the latest successful screenshot in one turn, and never serialize image bytes into text or persistence.
- Send PNG input as a DeepSeek Chat Completions user-message `image_url` part with `detail: "original"`.
- Reject zero dimensions, dimensions above 8192 pixels per side, and PNG payloads above 32 MiB.
- Support Windows and macOS native capture plus Linux X11; unsupported sessions return actionable errors and no external shell utility is invoked.
- Tests must use injected images/fakes and must never call the production capture backend.
- Do not add Tauri frontend permissions or screenshot UI.
- Preserve the user's existing uncommitted `agents/atlas.toml` change (`max_tool_calls = 64`); create the implementation worktree from committed `HEAD`, and never stage or overwrite that main-checkout edit.
- Use task commits as review checkpoints, then fold all task commits plus the already committed spec/plan into one focused final feature commit if integration is chosen.

## Review Focus

- A screenshot containing JSON-looking text or secret-like strings must remain binary image input and never appear in textual tool continuation, approval, event, or saved-history payloads; Task 2 adds leakage assertions.
- Two captures separated by ordinary tool calls must keep the first image available until the second succeeds, then expose only the second; Task 2 tests the complete sequence.
- A later failed capture must not erase or replace the previous successful observation, and its failure text must still reach the model; Task 2 tests this mixed success/failure sequence.
- Headless runtime construction and tool enumeration must not enumerate displays or request capture permission; Task 3 tests lazy backend use.
- Provider-limit edge values must accept exactly 8192 pixels and exactly 32 MiB but reject one unit above either boundary without allocating another encoded copy; Task 1 pins all four boundaries.

---

### Task 1: Provider-neutral image input and DeepSeek serialization

**Files:**
- Create: `src/media.rs`
- Modify: `src/lib.rs`
- Modify: `src/tools/mod.rs`
- Modify: every existing `ToolResult` construction under `src/tools/` and affected tests
- Modify: `src/models/mod.rs`
- Modify: `src/agents/mod.rs`
- Modify: `src/agents/local.rs`
- Modify: `src/models/deepseek.rs`
- Test: `src/models/deepseek.rs`
- Test: `tests/models_behavior.rs`

**Interfaces:**
- Produces: `ImageDetail::{Low, Original}` and `ImageData { mime_type: String, bytes: Vec<u8>, width: u32, height: u32, detail: ImageDetail }` in `src/media.rs`.
- Produces: `ToolAttachment::Image(ImageData)`, `ToolResult { message, attachments }`, `ToolResult::text(message: impl Into<String>) -> ToolResult`, and `ToolResult::with_image(message: impl Into<String>, image: ImageData) -> ToolResult`.
- Produces: `ModelRequest::images: &'a [ImageData]`.
- Produces: `Agent::process_stream_with_images(&self, input: &str, history: &[ConversationMessage], tools: &[ToolDefinition], images: &[ImageData], on_delta: &mut dyn FnMut(&str)) -> Result<String, String>` with a default that delegates for an empty slice and errors clearly for nonempty input; `LocalAgent` overrides it and forwards images.
- Changes: `DeepSeekProvider::chat_request(&ModelRequest, bool) -> Result<ChatRequest, String>`.

- [ ] **Step 1: Write failing media and DeepSeek request tests**

Add tests named:

- `tool_result_text_has_no_attachments`
- `chat_request_serializes_text_only_content_as_a_string`
- `chat_request_serializes_png_after_text_as_original_image_input`
- `chat_request_accepts_exact_image_limits`
- `chat_request_rejects_oversized_image_dimensions_and_bytes`
- `default_agent_rejects_nonempty_images_but_local_agent_forwards_them`

Assert exact mixed content JSON:

```json
[
  {"type":"text","text":"inspect this screen"},
  {"type":"image_url","image_url":{"url":"data:image/png;base64,<encoded>","detail":"original"}}
]
```

Use tiny synthetic bytes, the exact boundaries `8192` and `32 * 1024 * 1024`, and one-unit-over cases. The over-limit byte test may construct a length-only `Vec<u8>` once; production must check length before base64 encoding.

- [ ] **Step 2: Run focused tests to verify RED**

Run: `cargo test --lib models::deepseek::tests && cargo test --test models_behavior`

Expected: compilation fails because image/media fields and methods do not exist.

- [ ] **Step 3: Implement shared media and attachment types**

Add the exact types and constructors from Interfaces. Mechanically replace existing `ToolResult { message: ... }` literals with `ToolResult::text(...)` so every ordinary tool produces an empty attachment list.

- [ ] **Step 4: Implement the image-aware agent/model path**

Add `ModelRequest::images`, pass `&[]` from text-only `process`, `process_stream`, and title generation, and implement `process_stream_with_images` in `LocalAgent`. The trait default must return `model provider does not support image input` for a nonempty image slice.

- [ ] **Step 5: Implement DeepSeek mixed-content serialization and limits**

Use an untagged `ChatContent::{Text(String), Parts(Vec<ChatContentPart>)}`. Encode accepted PNG bytes with `base64` only after validating MIME type, nonzero dimensions, `<= 8192` dimensions, and `<= 32 * 1024 * 1024` bytes. Keep system/history messages as `Text` and make only the current user message mixed.

- [ ] **Step 6: Run focused and regression tests to verify GREEN**

Run: `cargo test --lib models::deepseek::tests && cargo test --test models_behavior --test tool_call_behavior --test permission_behavior`

Expected: all selected tests pass and text-only request JSON remains unchanged.

- [ ] **Step 7: Commit the checkpoint**

```bash
git add Cargo.toml Cargo.lock src tests
git commit -m "feat: add model image input boundary"
```

### Task 2: Runtime image lifecycle and privacy

**Files:**
- Modify: `src/runtime.rs`
- Test: `tests/tool_call_behavior.rs`
- Test: `tests/conversation_service_behavior.rs`

**Interfaces:**
- Consumes: Task 1 `ToolAttachment`, `ImageData`, and `Agent::process_stream_with_images`.
- Produces: turn-local latest-successful-image forwarding without adding attachment bytes to `ToolExecutionRecord`.

- [ ] **Step 1: Write failing runtime lifecycle tests**

Add fake tools that return labeled synthetic images and fake models that capture `ModelRequest.images`. Add tests named:

- `runtime_forwards_capture_as_image_without_embedding_bytes_in_tool_results`
- `runtime_keeps_latest_image_across_text_tools_and_replaces_it_after_recapture`
- `runtime_keeps_last_successful_image_when_a_later_capture_fails`
- `successful_turn_persists_only_user_and_final_text_not_tool_images`

Use distinct byte markers for the first and second image. Assert the model sees `[first]`, then `[first]` after a text tool, then `[second]` after recapture; assert serialized continuation text and saved JSONL contain neither marker nor `data:image`.

- [ ] **Step 2: Run focused tests to verify RED**

Run: `cargo test --test tool_call_behavior runtime_ -- --nocapture && cargo test --test conversation_service_behavior successful_turn_persists_only_user_and_final_text_not_tool_images`

Expected: new tests fail because runtime never forwards attachments.

- [ ] **Step 3: Implement latest-successful-image state**

In `ask_stream_with_history_and_authorization`, keep `latest_images: Vec<ImageData>` scoped to the turn. Call `process_stream_with_images` on every model iteration. On a successful result containing image attachments, replace `latest_images`; on text-only success or any failure, leave it unchanged. Store only `result.message` in `ToolExecutionRecord`.

- [ ] **Step 4: Run focused tests to verify GREEN**

Run: `cargo test --test tool_call_behavior && cargo test --test conversation_service_behavior`

Expected: all runtime and persistence tests pass with no image bytes in text or saved data.

- [ ] **Step 5: Commit the checkpoint**

```bash
git add src/runtime.rs tests/tool_call_behavior.rs tests/conversation_service_behavior.rs
git commit -m "feat: forward tool images within a turn"
```

### Task 3: Primary-display capture tool and native adapter

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `desktop/src-tauri/Cargo.lock`
- Modify: `src/tools/computer/controller.rs`
- Create: `src/tools/computer/capture_screen_tool.rs`
- Modify: `src/tools/computer/system.rs`
- Modify: `src/tools/computer/mod.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/app.rs`
- Modify: `src/permissions.rs`
- Modify: `tests/computer_tools_behavior.rs`
- Modify: `tests/permission_behavior.rs`
- Modify: `tests/agents_behavior.rs`

**Interfaces:**
- Consumes: Task 1 `ImageData`, `ImageDetail`, and `ToolResult::with_image`.
- Produces: `CapturedImage { png: Vec<u8>, width: u32, height: u32 }` and `ComputerController::capture_primary_display() -> Result<CapturedImage, String>`.
- Produces: `CaptureScreenTool::new(controller: SharedComputerController) -> CaptureScreenTool`.
- Produces: lazy production capture through the cross-platform `screenshots` crate and in-memory PNG encoding.

- [ ] **Step 1: Extend fakes and write failing tool behavior tests**

Add `capture_screen` to the exact default name list. Test `{}` succeeds with a tiny injected PNG, returns `image/png`, `ImageDetail::Original`, exact dimensions/bytes, and concise text. Assert `risk({}) == ComputerObserve`, summary contains `primary_display`, unknown fields fail before controller events, and building/listing default tools records no capture event.

Update the permission reason test to require wording that desktop information may be sent to the configured model provider.

- [ ] **Step 2: Write failing backend helper tests**

Add pure tests for primary-screen selection metadata, in-memory RGBA-to-PNG encoding (PNG signature plus decoded dimensions), zero-size rejection, and platform-specific actionable error mapping. Do not call display enumeration or capture.

- [ ] **Step 3: Run focused tests to verify RED**

Run: `cargo test --test computer_tools_behavior --test permission_behavior && cargo test --lib tools::computer::system::tests`

Expected: compilation fails because the capture interface and tool do not exist.

- [ ] **Step 4: Implement controller method and `CaptureScreenTool`**

The tool parser uses `#[serde(deny_unknown_fields)]`, returns `ComputerObserve`, summarizes as `{"scope":"primary_display"}`, locks the shared controller only after validation, rejects zero dimensions/empty bytes, and converts `CapturedImage` into `ImageData` with `image/png` and `Original`.

- [ ] **Step 5: Implement the lazy production capture adapter**

Add `screenshots` and a compatible direct `image` dependency. Enumerate screens only inside `capture_primary_display`, select `display_info.is_primary`, capture once, and encode through `std::io::Cursor<Vec<u8>>` as PNG. Map macOS permission errors to Screen Recording guidance, Linux session errors to X11 guidance, and otherwise retain native context.

- [ ] **Step 6: Register the tool on the shared controller**

Export the tool and register it in `build_default_tools` using another clone of the existing shared controller. Keep runtime construction lazy and frontend Tauri capabilities unchanged.

- [ ] **Step 7: Run focused and full core tests to verify GREEN**

Run: `cargo test --test computer_tools_behavior --test permission_behavior --test agents_behavior && cargo test`

Expected: all tests pass without requesting OS capture permission or producing a real screenshot.

- [ ] **Step 8: Commit the checkpoint**

```bash
git add Cargo.toml Cargo.lock desktop/src-tauri/Cargo.lock src tests
git commit -m "feat: add primary screen capture tool"
```

### Task 4: Documentation, independent review, and final verification

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `VISION.md`

**Interfaces:**
- Consumes: Tasks 1-3 shipped behavior.
- Produces: user/build documentation and final review evidence.

- [ ] **Step 1: Update documentation**

List `capture_screen`; state primary-display scope, provider upload after approval, no disk/history persistence, 8192-pixel/32-MiB limits, macOS Screen Recording permission, Windows native capture, Linux X11 requirement, and no frontend permission expansion. Document any Linux development packages introduced by the selected crate.

- [ ] **Step 2: Run documentation behavior tests**

Run: `cargo test --test agents_behavior`

Expected: the default tool list and permission documentation assertions pass.

- [ ] **Step 3: Run the complete verification matrix**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm --prefix desktop test
npm --prefix desktop run build
npm --prefix desktop run tauri -- build --debug --no-bundle
```

Expected: every command exits zero. No command invokes `capture_screen` against the host desktop.

- [ ] **Step 4: Request one whole-branch review**

Give the reviewer the spec, this plan, base/head diff, test evidence, and explicit focus on image leakage, lifecycle replacement, provider limits, native capture laziness, permission copy, and platform dependency/build risks. Fix Critical and Important findings once with regression tests; defer Minor findings explicitly.

- [ ] **Step 5: Commit documentation and review fixes**

```bash
git add README.md AGENTS.md VISION.md <review-fix-files>
git commit -m "docs: explain screen capture privacy"
```

- [ ] **Step 6: Prepare one traceable final commit**

Verify the feature worktree is clean, fold the plan/spec and task checkpoints into one commit based on the pre-feature `HEAD`, and use:

```text
feat: add screen capture vision
```

Do not include the main checkout's uncommitted `agents/atlas.toml` value change. Re-run `cargo fmt --check` and `cargo test`, review `git status --short` and the staged diff, then commit.
