# Tauri Chat UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a cross-platform Tauri desktop chat interface that streams Atlas responses and manages local conversations while creating a new conversation only after its first successful answer.

**Architecture:** Add an isolated `desktop/` Tauri v2 application that depends on the existing root Rust library. A thin Rust command bridge delegates to `ConversationStore` and `ConversationService`; a vanilla TypeScript state store consumes those commands through one API adapter and renders the approved two-panel interface.

**Tech Stack:** Rust 2021, Tauri 2.12.1, Node 24, vanilla TypeScript 7.0.2, Vite 8.3.2, Vitest 5.0.3 with jsdom 30.1.2, existing Atlas conversation/runtime modules

**Spec:** `docs/superpowers/specs/2026-10-04-tauri-chat-ui-design.md`

## Global Constraints

- Keep the existing `atlas` CLI behavior and default streaming behavior unchanged.
- Do not add agent routing, research, news, scheduling, workspace selection, settings screens, IDE panels, Markdown rendering, or publishing work.
- `New chat` is frontend-only state; there is no `create_conversation` command.
- Persist the first turn only after its streamed answer succeeds, using `ConversationService::ask` with no ID.
- Use Tauri IPC channels for ordered response deltas; never send tool protocol messages to the frontend.
- The frontend accesses Tauri only through `desktop/src/atlas-api.ts` and renders messages as text, never raw HTML.
- Conversation CRUD remains usable without model credentials or agent configuration.
- Preserve existing environment-variable and user-local conversation-storage contracts.
- Work test-first and create one focused commit per task.

## Review Focus

- First-send failure: no conversation is stored, the active ID stays `null`, and the prompt is restored (Tasks 3 and 4).
- Rapid conversation selection: a slower earlier load cannot replace a later selection (Task 4).
- Late stream data: callbacks after settlement cannot corrupt the assistant message (Task 4).
- Hostile message/title text: HTML-like input appears literally and creates no executable DOM (Task 5).
- Missing provider configuration: CRUD works and send reports a recoverable error (Tasks 2 and 4).

---

### Task 1: Share Runtime Construction

**Files:**
- Create: `src/app.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Test: `tests/app_runtime_behavior.rs`
- Test: `tests/cli_binary.rs`

**Interfaces:**
- Consumes: `LocalAgentDefinition`, `provider_from_definition`, `Runtime`, and current default tools.
- Produces: `atlas_agentic::app::{build_runtime, load_dotenv}`.

- [ ] **Step 1: Add the failing shared-bootstrap test**

Add `build_runtime_reports_the_requested_missing_agent_file`; call `build_runtime(temp.path(), &missing_path)` and assert the error contains that exact path. Keep the existing fake-provider CLI test as the streaming/storage regression.

- [ ] **Step 2: Confirm the test fails**

Run: `cargo test --test app_runtime_behavior --test cli_binary`

Expected: FAIL because `atlas_agentic::app` does not exist.

- [ ] **Step 3: Extract the bootstrap**

Implement:

```rust
pub fn build_runtime(workspace_root: &Path, agent_file: &Path) -> Result<Runtime, String>;
pub fn load_dotenv();
```

Move provider, agent registry, and tool construction from `src/main.rs` to `src/app.rs`. Give every tool `workspace_root`, read the agent from `agent_file`, preserve the Unix `exec_command` gate, export `pub mod app`, and make the CLI call these functions with `.` and `agents/atlas.toml`.

- [ ] **Step 4: Verify bootstrap and CLI behavior**

Run: `cargo test --test app_runtime_behavior --test cli_binary`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/lib.rs src/main.rs tests/app_runtime_behavior.rs tests/cli_binary.rs
git commit -m "refactor: share Atlas runtime bootstrap"
```

### Task 2: Add the Tauri Shell and CRUD Bridge

**Files:**
- Create: `desktop/package.json`, `desktop/package-lock.json`, `desktop/index.html`, `desktop/tsconfig.json`, `desktop/vite.config.ts`
- Create: `desktop/src/main.ts`, `desktop/src/styles.css`
- Create: `desktop/src-tauri/Cargo.toml`, `desktop/src-tauri/Cargo.lock`, `desktop/src-tauri/build.rs`, `desktop/src-tauri/tauri.conf.json`
- Create: `desktop/src-tauri/capabilities/default.json`
- Create: `desktop/src-tauri/src/main.rs`, `desktop/src-tauri/src/lib.rs`, `desktop/src-tauri/src/commands.rs`
- Modify: `.gitignore`
- Test: `desktop/src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `ConversationStore::{list, load, rename, delete}`.
- Produces: `DesktopState`, bridge-owned conversation DTOs, four CRUD commands, and `DeleteResult { id: String }`.

- [ ] **Step 1: Add failing CRUD bridge tests**

With a temporary store, add `crud_does_not_need_provider_configuration`, `list_returns_most_recent_first`, `load_rejects_invalid_id`, `rename_returns_updated_metadata_and_preserves_history`, and `delete_returns_id_and_removes_conversation`. Clear `DEEPSEEK_API_KEY` in the first test.

- [ ] **Step 2: Confirm the desktop test fails**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml`

Expected: FAIL because the package does not exist.

- [ ] **Step 3: Scaffold the minimal Tauri v2 application**

Pin `tauri = "2.12.1"`, `tauri-build = "2.7.1"`, and use `serde` plus `atlas-agentic = { path = "../.." }`. Pin npm dependencies `@tauri-apps/api` and `@tauri-apps/cli` to `2.12.1`, TypeScript to `7.0.2`, and Vite to `8.3.2`. The temporary frontend only renders an `Atlas` root so both halves build.

Ignore `/desktop/node_modules/`, `/desktop/dist/`, and `/desktop/src-tauri/target/` without broadening the existing root `/target/` rule.

Define npm scripts `dev: vite`, `build: tsc && vite build`, `test: vitest run`, and `tauri: tauri`. Configure Tauri's `beforeDevCommand`/`devUrl` for Vite and `frontendDist` for `desktop/dist`.

- [ ] **Step 4: Implement CRUD commands**

Use these signatures, with testable non-macro helpers underneath:

```rust
pub struct DesktopState { /* store; ask configuration follows in Task 3 */ }
pub fn list_conversations(state: State<'_, DesktopState>) -> Result<Vec<ConversationMetadataDto>, String>;
pub fn load_conversation(id: String, state: State<'_, DesktopState>) -> Result<ConversationDto, String>;
pub fn rename_conversation(id: String, title: String, state: State<'_, DesktopState>) -> Result<ConversationMetadataDto, String>;
pub fn delete_conversation(id: String, state: State<'_, DesktopState>) -> Result<DeleteResult, String>;
```

Parse IDs at the bridge. Map domain objects into bridge DTOs with camelCase serde fields, RFC 3339 timestamps, lowercase roles, and string IDs. This keeps the on-disk domain serialization unchanged. Desktop setup creates only a store; it must not initialize a provider.

- [ ] **Step 5: Register commands and minimal permissions**

Register commands in one `tauri::generate_handler!` call. Configure one main window and no shell, filesystem, or process plugins.

- [ ] **Step 6: Verify CRUD and shell compilation**

Run `cargo test --manifest-path desktop/src-tauri/Cargo.toml` and `npm --prefix desktop run build`.

Expected: both PASS.

- [ ] **Step 7: Commit**

```bash
git add .gitignore desktop
git commit -m "feat: add Tauri conversation shell"
```

### Task 3: Stream Asks Through the Desktop Bridge

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Test: `desktop/src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: Task 1 `build_runtime`, Task 2 `DesktopState`, and `ConversationService::ask`.
- Produces: `ChatEvent`, `ConversationAskResultDto`, and the `ask` command.

- [ ] **Step 1: Add failing ask tests**

Inject a runtime factory that returns a `Runtime` backed by a fake streaming agent. Add `ask_forwards_deltas_in_order_and_returns_metadata`, `first_ask_failure_leaves_store_empty`, `continuing_ask_passes_history_and_keeps_id`, and `provider_failure_does_not_damage_history`. Assert the exact delta vector and store contents.

- [ ] **Step 2: Confirm focused tests fail**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::tests::ask`

Expected: FAIL because the ask bridge does not exist.

- [ ] **Step 3: Add lazy injectable runtime construction**

Add to `DesktopState`:

```rust
type RuntimeFactory = Arc<dyn Fn() -> Result<Runtime, String> + Send + Sync>;
```

Production closes over the bundled agent path and current working directory and calls Task 1's factory. State also holds `ConversationStore::from_user_data()` and `history_limit_from_env()`. Tests inject fake runtimes and temporary stores. Do not create a provider at window startup.

- [ ] **Step 4: Implement the tested synchronous helper**

```rust
fn ask_with_sink(
    &self,
    prompt: &str,
    conversation_id: Option<&ConversationId>,
    on_delta: &mut dyn FnMut(&str),
) -> Result<ConversationAskResultDto, String>;
```

Construct the runtime lazily, create `ConversationService`, forward deltas unchanged, and map the result into the bridge-owned `ConversationAskResultDto`.

- [ ] **Step 5: Add the asynchronous Tauri wrapper**

```rust
#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum ChatEvent { Delta { content: String } }

#[tauri::command(async)]
pub fn ask(
    prompt: String,
    conversation_id: Option<String>,
    channel: Channel<ChatEvent>,
    state: State<'_, DesktopState>,
) -> Result<ConversationAskResultDto, String>;
```

Parse the optional ID, call the synchronous helper on Tauri's asynchronous command executor, send one `Delta` per callback, and propagate channel/service errors. Register it in the existing handler. This keeps blocking provider work off the window thread without borrowing state across an async Rust future.

- [ ] **Step 6: Verify bridge and root regressions**

Run `cargo test --manifest-path desktop/src-tauri/Cargo.toml` and `cargo test --all-targets`.

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/lib.rs
git commit -m "feat: stream Atlas asks to desktop"
```

### Task 4: Implement the Frontend API and Chat State

**Files:**
- Create: `desktop/src/types.ts`, `desktop/src/atlas-api.ts`, `desktop/src/chat-state.ts`
- Create: `desktop/src/atlas-api.test.ts`, `desktop/src/chat-state.test.ts`
- Modify: `desktop/package.json`, `desktop/package-lock.json`, `desktop/tsconfig.json`

**Interfaces:**
- Consumes: Tasks 2–3 command names and JSON shapes.
- Produces: `AtlasApi`, `createTauriAtlasApi`, `ChatStore`, `ChatState`, and frontend wire types.

- [ ] **Step 1: Add failing IPC adapter tests**

Pin Vitest to `5.0.3` and jsdom to `30.1.2`. Use `@tauri-apps/api/mocks` to assert exact command names/camelCase payloads. Assert ask-channel deltas reach the callback and the promise resolves with metadata.

- [ ] **Step 2: Add failing state tests**

With a fake API, add: `new_chat_never_calls_backend`, `first_success_adopts_id`, `failed_first_send_stays_unsaved_marks_partial_failed_and_restores_prompt`, `retry_removes_the_previous_failed_attempt`, `continue_keeps_id_and_moves_item_to_top`, `rename_and_delete_commit_only_after_success`, `missing_provider_keeps_crud_and_retry`, `latest_selection_wins`, and `late_delta_is_ignored`.

- [ ] **Step 3: Confirm frontend tests fail**

Run: `npm --prefix desktop test`

Expected: FAIL because the adapter/store do not exist.

- [ ] **Step 4: Define wire and state types**

Define camelCase `ConversationMetadata`, `ConversationMessage`, `Conversation`, `ConversationAskResult`, `ChatEvent`, `ChatStatus`, and `ChatState`; IDs are strings. Define a UI-only `DisplayMessage` with `delivery: "saved" | "pending" | "failed"` so transient failures never alter persistence types.

- [ ] **Step 5: Implement the only Tauri adapter**

```ts
export interface AtlasApi {
  listConversations(): Promise<ConversationMetadata[]>;
  loadConversation(id: string): Promise<Conversation>;
  ask(prompt: string, conversationId: string | null, onDelta: (text: string) => void): Promise<ConversationAskResult>;
  renameConversation(id: string, title: string): Promise<ConversationMetadata>;
  deleteConversation(id: string): Promise<void>;
}
export function createTauriAtlasApi(): AtlasApi;
```

Create one `Channel<ChatEvent>` per ask and accept only delta events. Keep every Tauri import in this file.

- [ ] **Step 6: Implement deterministic state transitions**

```ts
export class ChatStore {
  constructor(api: AtlasApi);
  getState(): Readonly<ChatState>;
  subscribe(listener: (state: Readonly<ChatState>) => void): () => void;
  initialize(): Promise<void>;
  newChat(): void;
  selectConversation(id: string): Promise<void>;
  setDraft(text: string): void;
  send(): Promise<void>;
  renameActive(title: string): Promise<void>;
  deleteActive(): Promise<void>;
}
```

Use monotonic selection and ask tokens. Do not invoke on empty trimmed drafts. On failure retain the transient user/partial-assistant pair, mark the assistant failed, restore the prompt, and retain a null ID for a first chat. Starting a retry removes that failed pair before adding the new pending attempt.

- [ ] **Step 7: Verify frontend state**

Run: `npm --prefix desktop test`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add desktop/package.json desktop/package-lock.json desktop/tsconfig.json desktop/src
git commit -m "feat: add desktop chat state"
```

### Task 5: Build the Accessible Two-Panel Interface

**Files:**
- Create: `desktop/src/chat-view.ts`, `desktop/src/chat-view.test.ts`
- Modify: `desktop/index.html`, `desktop/src/main.ts`, `desktop/src/styles.css`

**Interfaces:**
- Consumes: Task 4 `ChatStore`/`ChatState`.
- Produces: `mountChat(root: HTMLElement, store: ChatStore): () => void`.

- [ ] **Step 1: Add failing DOM tests**

Cover startup loading, draft-only New chat, selection/loading, Enter versus Shift+Enter, ordered stream display, rename success/failure, confirmed deletion, disabled streaming controls, narrow-window sidebar toggle, focus restoration, and literal rendering of `<img src=x onerror=...>` with no `img` element created.

- [ ] **Step 2: Confirm view tests fail**

Run: `npm --prefix desktop test -- chat-view.test.ts`

Expected: FAIL because `mountChat` does not exist.

- [ ] **Step 3: Implement semantic rendering and actions**

Use semantic buttons, navigation, main, message list, form, textarea, dialog, status, and alert elements. Subscribe once and return cleanup. Assign external text through `textContent`. Restore composer focus after New chat, successful send, and selection.

- [ ] **Step 4: Apply the approved responsive design**

Implement the neutral shell, 260px sidebar, readable message width, right-aligned user bubbles, document-style assistant messages, fixed composer, visible focus rings, mobile sidebar, whitespace preservation, and `prefers-reduced-motion`.

- [ ] **Step 5: Wire the production entrypoint**

Create `ChatStore(createTauriAtlasApi())`, mount into `#app`, and call `initialize()` without an unhandled rejection.

- [ ] **Step 6: Verify UI and build**

Run `npm --prefix desktop test` and `npm --prefix desktop run build`.

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add desktop/index.html desktop/src/main.ts desktop/src/styles.css desktop/src/chat-view.ts desktop/src/chat-view.test.ts
git commit -m "feat: build desktop conversation UI"
```

### Task 6: Bundle Configuration, Add CI, and Document Development

**Files:**
- Modify: `desktop/src-tauri/tauri.conf.json`, `desktop/src-tauri/src/lib.rs`
- Create: `desktop/src-tauri/icons/*`
- Modify: `.github/workflows/ci.yml`, `README.md`, `AGENTS.md`
- Test: `desktop/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Tauri resource resolution and `agents/atlas.toml`.
- Produces: deterministic resource-based agent-path resolution, documented commands, and CI coverage.

- [ ] **Step 1: Add failing resource-selection tests**

Test:

```rust
fn bundled_agent_path(resource_dir: &Path) -> PathBuf;
```

Assert the result is `<resource_dir>/agents/atlas.toml`. Add a setup test proving construction reads no API key.

- [ ] **Step 2: Confirm focused tests fail**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml bundled_agent_path`

Expected: FAIL because the helper does not exist.

- [ ] **Step 3: Bundle and resolve the agent file**

Map `../../agents/atlas.toml` to `agents/atlas.toml` in `bundle.resources`. Call Task 1's `load_dotenv`, resolve that path from Tauri's resource directory during setup, and keep agent/provider loading lazy until ask.

- [ ] **Step 4: Generate standard Tauri icons**

Create one simple Atlas source icon and use the Tauri icon generator for required cross-platform files. Add no unrelated brand assets.

- [ ] **Step 5: Extend CI**

Add `actions/setup-node` with Node 24, `npm ci --prefix desktop`, frontend test/build, the official Linux Tauri prerequisites (`libwebkit2gtk-4.1-dev`, `build-essential`, `curl`, `wget`, `file`, `libxdo-dev`, `libssl-dev`, `libayatana-appindicator3-dev`, and `librsvg2-dev`), and desktop Rust tests. Preserve root fmt/test steps.

- [ ] **Step 6: Update docs**

Document prerequisites, install, `npm --prefix desktop run tauri dev`, environment variables, and unsaved-draft behavior in `README.md`. Add the real desktop tree and verification commands to `AGENTS.md`.

- [ ] **Step 7: Run complete verification**

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm --prefix desktop test
npm --prefix desktop run build
npm --prefix desktop run tauri -- build --debug --no-bundle
git diff --check
```

Expected: all exit 0. If host Tauri packages are missing, install only documented prerequisites and rerun; do not claim completion without a host build.

- [ ] **Step 8: Commit**

```bash
git add desktop/src-tauri/tauri.conf.json desktop/src-tauri/src/lib.rs desktop/src-tauri/icons .github/workflows/ci.yml README.md AGENTS.md
git commit -m "docs: complete desktop app setup"
```

### Task 7: Final Cross-Layer Regression Review

**Files:**
- Modify only files required by issues found in this task.

**Interfaces:**
- Consumes: completed bridge, state/UI, resources, and docs.
- Produces: a verified desktop chat vertical slice and clean worktree.

- [ ] **Step 1: Inspect every spec rule against its owning automated test**

Confirm the Task 3 backend tests prove storage timing and credential-independent CRUD, the Task 4 tests prove state transitions and races, and the Task 5 tests prove DOM safety and keyboard behavior. Add a missing focused test before proceeding if any approved rule lacks coverage.

- [ ] **Step 2: Exercise responsive keyboard behavior in the development window**

At desktop and narrow widths verify New chat, selection, focus, Enter/Shift+Enter, rename dialog, delete confirmation, and sidebar toggle using the keyboard.

- [ ] **Step 3: Rerun Task 6 verification**

Expected: every command exits 0.

- [ ] **Step 4: Inspect scope and state**

```bash
git diff --check
git status --short
git log --oneline -8
```

Expected: no uncommitted changes or secrets and only focused planned commits.

- [ ] **Step 5: Commit only if final review found issues**

If fixes were required, rerun their focused tests, stage only the paths shown by `git diff --name-only`, inspect `git diff --cached`, and commit them as `fix: harden desktop conversation UI`. If no fixes were required, create no empty commit.
