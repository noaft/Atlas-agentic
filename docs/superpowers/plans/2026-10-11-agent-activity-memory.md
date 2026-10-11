# Agent Activity Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist bounded, structured Atlas tool activity, show it as a per-turn desktop timeline, and send compact summaries to the agent on later turns.

**Architecture:** Keep user/assistant JSONL history unchanged and add one versioned activity batch per successful turn in `activities/<conversation-id>.jsonl`. `ConversationService` captures sanitized runtime events, commits them with successful turns, and supplies only bounded summaries through a separate model-context field; Tauri and the TypeScript store render the same records live and after reload.

**Tech Stack:** Rust 2021, serde/serde_json, chrono, UUID, Tauri v2, TypeScript, Vitest, vanilla DOM/CSS

**Spec:** `docs/superpowers/specs/2026-10-11-agent-activity-memory-design.md`

## Global Constraints

- Persist observable runtime status and tool activity, never hidden chain-of-thought or provider-internal reasoning.
- Do not persist screenshot bytes, base64, data URLs, or tool attachments.
- Keep existing conversation message JSONL compatible and optional activity sidecars readable as empty when missing.
- Persist activity only with a successful overall turn; include failed and denied tool attempts when that turn eventually succeeds.
- Send only bounded activity summaries to later model requests; raw local details never enter automatic provider context.
- Use these fixed limits: 2,048 characters per serialized argument value, 8,192 characters per result detail, 32,768 detail characters per turn, and 8,192 characters for later-turn activity context.
- Keep native storage, sanitization, runtime activity, and model-context behavior in Rust; the frontend receives only sanitized DTOs.
- Preserve the existing uncommitted `agents/atlas.toml` change and never include it in feature commits.

## Review Focus

- Repeated calls to the same tool in one turn must retain stable sequence and update the correct live row; Task 3 and Task 6 test two consecutive `search_files` calls.
- A crash-shaped store error between sidecar and message publication must not expose an activity batch for a turn whose messages are absent; Task 2 tests rollback and ignores batches beyond durable message count.
- Secret keys nested inside arrays and mixed-case objects must be redacted before desktop or disk exposure; Task 1 tests nested `Authorization` and `apiKey` values.
- Unicode truncation must operate on characters without corrupting UTF-8 or exceeding the specified caps; Task 1 tests multibyte arguments, details, per-turn totals, and context.
- Old conversations, tool-free turns, and deleted conversations with no sidecar must continue to load and delete normally; Task 2 and Task 5 test all three cases.

---

### Task 1: Activity Domain, Sanitization, and Context Rendering

**Files:**
- Create: `src/conversations/activity.rs`
- Modify: `src/conversations/mod.rs`
- Test: `tests/conversation_activity_behavior.rs`

**Interfaces:**
- Consumes: `chrono::DateTime<Utc>`, `serde_json::Value`.
- Produces: `ActivityKind`, `ActivityOutcome`, `ActivityRecord`, `ActivityTurn`, `ActivityCollector`, `sanitize_arguments(&Value) -> String`, `sanitize_result(&str) -> String`, and `render_activity_context(&[ActivityTurn]) -> String`.
- `ActivityRecord` contains `sequence: u32`, `kind`, optional `tool`, optional `arguments`, optional `outcome`, `summary`, optional `details`, and `created_at: DateTime<Utc>`.
- `ActivityTurn` contains `version: u32` fixed to `1`, `turn_id: String`, `assistant_message_index: usize`, and ordered `records: Vec<ActivityRecord>`; one JSONL line represents one complete turn batch.

- [ ] **Step 1: Write failing domain and sanitizer tests**

Add tests named `activity_turn_round_trips_as_one_versioned_json_line`, `sanitizer_redacts_nested_mixed_case_secret_keys`, `sanitizer_truncates_unicode_at_argument_and_detail_limits`, `collector_enforces_the_per_turn_detail_budget`, `context_keeps_newest_complete_summaries_within_8192_characters`, and `capture_screen_record_drops_image_payload_details`. Assert the exact limits from Global Constraints, stable sequence values, and an explicit `… [truncated]` suffix.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test --test conversation_activity_behavior`

Expected: FAIL because `conversations::activity` and its types do not exist.

- [ ] **Step 3: Implement the activity domain and pure helpers**

Define serde camel-case enums and structs in `src/conversations/activity.rs`. `ActivityCollector::status`, `tool_requested`, `tool_denied`, and `tool_finished` assign monotonically increasing sequence numbers and return sanitized records. Store screenshot results as metadata-only summaries. Render later context with the exact heading `Recent Atlas activity (facts from earlier successful turns):` and newest complete bullet lines within `ACTIVITY_CONTEXT_LIMIT = 8_192`.

- [ ] **Step 4: Run focused tests and formatting**

Run: `cargo test --test conversation_activity_behavior && cargo fmt --check`

Expected: all activity behavior tests PASS and formatting exits 0.

- [ ] **Step 5: Commit the domain unit**

```bash
git add src/conversations/activity.rs src/conversations/mod.rs tests/conversation_activity_behavior.rs
git commit -m "feat: add conversation activity records"
```

### Task 2: Sidecar Storage and Coordinated Turn Persistence

**Files:**
- Modify: `src/conversations/types.rs`
- Modify: `src/conversations/store.rs`
- Modify: `src/conversations/service.rs`
- Modify: `tests/conversation_store_behavior.rs`
- Modify: `tests/conversation_service_behavior.rs`
- Modify: `tests/conversation_store_unix_behavior.rs`

**Interfaces:**
- Consumes: `ActivityTurn` from Task 1.
- Produces: `Conversation.activities: Vec<ActivityTurn>`, `ConversationStore::create_turn_with_activity(...)`, `ConversationStore::append_turn_with_activity(...)`, and existing store methods retained as empty-activity wrappers.
- Sidecar path: `<root>/activities/<conversation-id>.jsonl`; each line is a versioned `ActivityTurn` envelope.

- [ ] **Step 1: Write failing storage tests**

Add tests named `successful_turn_persists_and_loads_ordered_activity`, `missing_activity_sidecar_loads_as_empty`, `tool_free_turn_persists_an_empty_activity_batch_for_association`, `activity_batch_beyond_durable_message_count_is_ignored`, `activity_write_failure_leaves_messages_and_metadata_unchanged`, `delete_moves_activity_to_trash_when_present`, `delete_without_activity_remains_compatible`, and Unix permission coverage asserting the activity directory/file are private.

- [ ] **Step 2: Run storage tests and verify RED**

Run: `cargo test --test conversation_store_behavior --test conversation_store_unix_behavior`

Expected: FAIL because activity-aware store methods and loaded activity do not exist.

- [ ] **Step 3: Implement optional sidecar reads and atomic activity publication**

Add `activities_dir`, `activity_path`, JSONL read/write helpers with path-and-line parse errors, and activity-aware create/append methods. Set `assistant_message_index` to the zero-based index of the newly saved assistant message. Publish the replacement sidecar before appending history, roll it back on history failure, and ignore any batch whose assistant index is outside durable message history. Keep old methods as wrappers passing an empty batch.

- [ ] **Step 4: Extend delete rollback across both recoverable files**

Move a present sidecar to `trash/<id>.activity.jsonl`, roll back both moves if index publication fails, and keep the existing returned history path contract.

- [ ] **Step 5: Run storage tests and verify GREEN**

Run: `cargo test --test conversation_store_behavior --test conversation_store_unix_behavior`

Expected: all store tests PASS.

- [ ] **Step 6: Run existing conversation regression tests**

Run: `cargo test --test conversation_context_behavior --test conversation_service_behavior --test conversation_store_behavior`

Expected: all existing conversation tests PASS.

- [ ] **Step 7: Commit sidecar persistence**

```bash
git add src/conversations/types.rs src/conversations/store.rs src/conversations/service.rs tests/conversation_store_behavior.rs tests/conversation_service_behavior.rs tests/conversation_store_unix_behavior.rs
git commit -m "feat: persist per-turn activity sidecars"
```

### Task 3: Rich Runtime Activity Events

**Files:**
- Modify: `src/permissions.rs`
- Modify: `src/runtime.rs`
- Modify: `tests/tool_call_behavior.rs`
- Modify: `tests/permission_behavior.rs`

**Interfaces:**
- Consumes: existing tool arguments, authorization summaries, tool results, and attachments.
- Produces: `RuntimeStatus::{Thinking, PreparingAnswer}` and enriched `ToolEvent::{Status, Requested, ApprovalRequested, Denied, Started, Finished}` variants. Tool variants carry `sequence: u32`; requested carries sanitized argument text, and finished carries `success`, `summary`, and bounded sanitized details but no attachments.

- [ ] **Step 1: Write failing event-order tests**

Add tests named `runtime_emits_thinking_request_start_finish_and_preparing_statuses`, `repeated_same_tool_calls_have_distinct_monotonic_sequences`, `denied_tool_emits_denied_without_started`, `failed_tool_emits_sanitized_bounded_details`, and `screen_capture_event_contains_no_attachment_bytes`.

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --test tool_call_behavior --test permission_behavior`

Expected: FAIL because the enriched variants and fields do not exist.

- [ ] **Step 3: Emit complete runtime activity without exposing model reasoning**

Emit `Thinking` before the first provider request and `PreparingAnswer` before each provider continuation. Allocate a monotonic sequence when parsing each tool call, emit requested before authorization, denied on a negative decision, and finished with sanitized text after execution. Never place image attachments in an event.

- [ ] **Step 4: Run focused runtime tests and verify GREEN**

Run: `cargo test --test tool_call_behavior --test permission_behavior`

Expected: all tests PASS.

- [ ] **Step 5: Commit runtime events**

```bash
git add src/permissions.rs src/runtime.rs tests/tool_call_behavior.rs tests/permission_behavior.rs
git commit -m "feat: emit detailed agent activity events"
```

### Task 4: Successful-Turn Capture and Later Model Context

**Files:**
- Modify: `src/conversations/service.rs`
- Modify: `src/runtime.rs`
- Modify: `src/agents/mod.rs`
- Modify: `src/agents/local.rs`
- Modify: `src/models/mod.rs`
- Modify: `src/models/deepseek.rs`
- Modify: `tests/conversation_service_behavior.rs`
- Modify: `tests/conversation_context_behavior.rs`
- Modify: `tests/agents_behavior.rs`
- Modify: `tests/models_behavior.rs`

**Interfaces:**
- Consumes: enriched `ToolEvent` from Task 3, `ActivityCollector`/context renderer from Task 1, and activity-aware store methods from Task 2.
- Produces: `ConversationAskResult.activities: Vec<ActivityRecord>` for the completed turn; `Runtime::ask_stream_with_context_and_authorization(prompt, history, activity_context, ...)`; `ModelRequest.activity_context: Option<&str>`; existing public ask helpers pass `None`.

- [ ] **Step 1: Write failing service/context tests**

Add `successful_service_turn_commits_activity_with_messages`, `failed_service_turn_commits_neither_messages_nor_activity`, `continuation_selects_activity_for_the_same_recent_message_window`, `later_turn_receives_summaries_not_raw_details`, `failed_and_denied_tools_are_remembered_after_success`, and `failed_overall_turn_discards_collected_activity` using fake providers that emit the required tool protocol.

- [ ] **Step 2: Write failing provider-placement tests**

Assert that DeepSeek renders non-empty activity context as a labeled system section before historical messages, creates no fake user/assistant message, excludes it when empty, and never includes raw detail fixtures.

- [ ] **Step 3: Run focused tests and verify RED**

Run: `cargo test --test conversation_service_behavior --test conversation_context_behavior --test agents_behavior --test models_behavior`

Expected: FAIL because activity context is not carried through service, runtime, agent, and provider boundaries.

- [ ] **Step 4: Capture events and commit activity only after final success**

Wrap the caller's tool-event callback inside `ConversationService`, forwarding every event while feeding the collector. After a final answer, create one batch (including tool-free status activity), use the activity-aware store method, and return the canonical completed-turn records. Drop the collector on any runtime or stream-sink error.

- [ ] **Step 5: Thread separate activity context through model boundaries**

Add `activity_context: Option<&str>` to `ModelRequest` and corresponding agent/runtime methods. Render it under `Atlas memory from prior successful tool activity:` in the provider system prompt before tool-protocol instructions. Existing call sites pass `None`; title generation always passes `None`.

- [ ] **Step 6: Run focused and full Rust tests**

Run: `cargo test --test conversation_service_behavior --test conversation_context_behavior --test agents_behavior --test models_behavior && cargo test`

Expected: all tests PASS except the known baseline mismatch if the unrelated uncommitted `agents/atlas.toml` still says 64 while `tests/agents_behavior.rs` expects 32; do not alter either as part of this feature.

- [ ] **Step 7: Commit activity memory flow**

```bash
git add src/conversations/service.rs src/runtime.rs src/agents/mod.rs src/agents/local.rs src/models/mod.rs src/models/deepseek.rs tests/conversation_service_behavior.rs tests/conversation_context_behavior.rs tests/agents_behavior.rs tests/models_behavior.rs
git commit -m "feat: include recent activity in agent context"
```

### Task 5: Tauri DTOs and Live Activity Channel

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Test: existing `#[cfg(test)]` module in `desktop/src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `Conversation.activities`, completed-turn activity in `ConversationAskResult`, and enriched `ToolEvent`.
- Produces: camel-case `ActivityRecordDto`, `ActivityTurnDto`, `ConversationDto.activities`, `ConversationAskResultDto.activities`, and detailed `ChatEvent` status/tool variants.

- [ ] **Step 1: Write failing serialization and conversion tests**

Test persisted activity conversion, completed ask activity, exact camel-case live event JSON, repeated sequence preservation, denial state, bounded details, tool-free status activity, and loading/deleting a conversation whose sidecar is absent.

- [ ] **Step 2: Run Tauri tests and verify RED**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::tests`

Expected: FAIL because activity DTOs and new channel variants do not exist.

- [ ] **Step 3: Add DTO conversion and channel mapping**

Map only sanitized Rust activity fields. Preserve `sequence` in every live tool event so the frontend never correlates by tool name. Continue sending approval cards through `DesktopAuthorizer`, adding the matching tool sequence to approval events without exposing unsanitized arguments.

- [ ] **Step 4: Run Tauri tests and formatting**

Run: `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check && cargo test --manifest-path desktop/src-tauri/Cargo.toml`

Expected: formatting and all Tauri tests PASS.

- [ ] **Step 5: Commit the desktop bridge**

```bash
git add desktop/src-tauri/src/commands.rs
git commit -m "feat: expose conversation activity to desktop"
```

### Task 6: Desktop Activity State and Per-Turn Timeline

**Files:**
- Modify: `desktop/src/types.ts`
- Modify: `desktop/src/chat-state.ts`
- Modify: `desktop/src/chat-view.ts`
- Modify: `desktop/src/styles.css`
- Modify: `desktop/src/chat-state.test.ts`
- Modify: `desktop/src/chat-view.test.ts`
- Modify: `desktop/src/styles.test.js`

**Interfaces:**
- Consumes: Tauri DTO and channel shapes from Task 5.
- Produces: typed `ActivityRecord`, `ActivityTurn`, and live activity state keyed by `sequence`; each assistant `DisplayMessage` has optional `activity` for its message index.

- [ ] **Step 1: Write failing state tests**

Test status rows, requested/running/succeeded/failed/denied transitions, two consecutive `search_files` calls updating by sequence, stale event rejection, canonical result replacement after success, failed overall request retaining transient activity only until the next action, and restored association by `assistantMessageIndex`.

- [ ] **Step 2: Run state tests and verify RED**

Run: `npm --prefix desktop test -- chat-state.test.ts`

Expected: FAIL because the new DTOs and sequence-keyed state do not exist.

- [ ] **Step 3: Implement typed state transitions and persisted association**

Replace the current tool-name correlation with sequence-keyed records. When loading a conversation, attach each activity batch to the assistant display message at `assistantMessageIndex`. When ask succeeds, use the returned canonical activity rather than retaining transient copies.

- [ ] **Step 4: Write failing view and accessibility tests**

Test a collapsed `Activity` disclosure below the correct assistant message, expansion showing bounded arguments/details, text labels for every outcome, live `role="status"`, approval remaining prominent, and no raw screenshot or base64 content in the DOM fixture.

- [ ] **Step 5: Implement the activity disclosure and responsive styling**

Use native `<details><summary>Activity</summary>` per assistant turn. Render status rows and tool rows in sequence; put arguments/details in wrapping `<pre>` elements. Add status data attributes and accessible text without color-only meaning.

- [ ] **Step 6: Run all frontend tests and build**

Run: `npm --prefix desktop test && npm --prefix desktop run build`

Expected: all Vitest tests PASS and TypeScript/Vite build exits 0.

- [ ] **Step 7: Commit the desktop timeline**

```bash
git add desktop/src/types.ts desktop/src/chat-state.ts desktop/src/chat-view.ts desktop/src/styles.css desktop/src/chat-state.test.ts desktop/src/chat-view.test.ts desktop/src/styles.test.js
git commit -m "feat: show per-turn agent activity"
```

### Task 7: Documentation and Full Verification

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: completed storage, context, and desktop behavior from Tasks 1–6.
- Produces: user-facing and contributor-facing documentation matching shipped behavior.

- [ ] **Step 1: Update behavior documentation**

Document the optional `activities/<uuid>.jsonl` sidecar, successful-turn-only persistence, local bounded/redacted details, screenshot exclusion, compact later-turn summaries, and expandable desktop timeline. Replace statements saying tool calls/results are never persisted with the narrower rule that they never enter ordinary message history.

- [ ] **Step 2: Run documentation consistency tests**

Run: `cargo test --test agents_behavior --test conversation_store_behavior`

Expected: documentation assertions and storage behavior tests PASS, aside from the separately owned `max_tool_calls` baseline mismatch if still present.

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

Expected: every command exits 0. If the known unrelated `max_tool_calls` mismatch remains, report it with exact output and do not change or commit the user's file without direction.

- [ ] **Step 4: Review feature diff and repository status**

Run: `git status --short`, `git diff --check`, and review every feature file. Confirm `agents/atlas.toml`, `.env`, secrets, `target/`, and unrelated files are unstaged.

- [ ] **Step 5: Commit documentation**

```bash
git add README.md AGENTS.md
git commit -m "docs: explain agent activity memory"
```

- [ ] **Step 6: Request final code review**

Use the repository review workflow to check spec compliance, storage consistency, privacy boundaries, event ordering, UI association, and verification evidence. Address findings with focused tests and commits before declaring completion.
