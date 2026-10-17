# Task Registry and Slash Menu Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Make `/` discover available tasks from a shared Rust registry and dispatch selected tasks safely, starting with `/compact`.

**Architecture:** Rust owns task descriptors, argument/precondition validation, and typed handler bindings. Tauri exposes catalog/dispatch commands through `atlas-api.ts`; a reusable composer menu uses that catalog and a generic slash parser. Existing compaction service, progress, cancellation, memory state, and runtime automatic-compaction path remain the execution foundation.

**Tech Stack:** Rust, serde, Tauri, TypeScript, Vitest; no new dependency required.

**Spec:** `docs/superpowers/specs/2026-10-17-task-registry-slash-menu-design.md` (approved by the user's “do it”).

**Execution:** Preserve native execution in this session and the established local merge workflow. Review this written plan before implementation as required by writing-plans.

## Global Constraints

- Registry + slash menu only; automatic model routing and user-defined/plugin tasks remain future work.
- Register only the real `conversation.compact` task with slash name `compact`; no placeholder menu entries.
- One authoritative backend catalog; no production frontend fallback list or command-specific branch in `ChatStore.send()`.
- One block of memory remains one summary message; preserve all current compaction boundaries, storage, usage, cancellation, permission, and session-isolation behavior.
- Catalog listing requires no model API key. Task execution validates ID, arguments and saved-conversation requirements in Rust before performing work.
- Automatic compaction must not reacquire its existing lease or redispatch through Tauri.
- Choosing a menu row inserts its canonical command and never executes it; another Enter/Send executes.
- Unknown commands, bare `/`, and invalid compact arguments preserve the draft and never reach normal model chat.
- Prose, inline/code/quoted commands, URLs, and multi-segment absolute paths remain ordinary chat.
- All desktop Rust access goes through `atlas-api.ts`; do not broaden frontend permissions.
- Tests use fakes or loopback services, never real providers. One focused implementation commit after verification.

## Review Focus

1. A catalog failure or late retry response must not block ordinary chat, overwrite drafts, or replace newer catalog state (Task 2).
2. Enter/Tab while the menu is open must insert only once, without submitting or immediately reopening; IME and Shift+Enter remain normal input (Task 3).
3. A selected task becomes unavailable after a chat/model change: recompute frontend availability and still validate on the backend (Tasks 1–3).
4. A command typo, invalid arguments, or an absolute path must not accidentally run compact or leak a command into a model request (Tasks 1–3).
5. Background task progress/completion/cancellation must stay with its initiating chat, including after switching chats or a failed task (Tasks 2–3).

## Shared interfaces and file ownership

- Create `src/tasks/mod.rs`: `TaskDescriptor`, `TaskDefinition`, `TaskHandler`, `TaskRegistry`, `TaskRequest`, `TaskResult`, and `COMPACT_TASK_ID`.
- Descriptor wire fields: `id`, `command`, `title`, `description`, `argumentSchema`, `requiresConversation`, `requiresModel`, `cancellable`. Command is `compact` without a slash. Compact schema is `{ "type": "object", "properties": {}, "additionalProperties": false }`.
- `TaskRegistry::new(Vec<TaskDefinition>) -> Result<Self, String>` rejects duplicate IDs/names; `builtins() -> Self`, `descriptors() -> Vec<TaskDescriptor>`, `validate(&TaskRequest) -> Result<TaskHandler, String>` select a known typed handler and validate its arguments/conversation requirement.
- `TaskRequest { task_id: String, arguments: serde_json::Value, conversation_id: Option<ConversationId> }` serializes camelCase. Compact takes exactly an empty JSON object.
- `TaskRegistry::run(request, service: &ConversationService, cancelled: Option<&AtomicBool>, on_progress: &mut dyn FnMut(CompactionProgress)) -> Result<TaskResult, String>` dispatches through the validated handler. `TaskResult { task_id, outcome }`; `TaskOutcome::Compaction(CompactionResult)` is serde-tagged with `kind` and `result`, camelCase. Add only the execution resources needed by the current handler, without designing a plugin runtime.
- `AtlasApi.listTasks(): Promise<TaskDescriptor[]>`; `runTask(taskId, arguments, conversationId, onEvent): Promise<TaskResult>`. Existing cancellation stays available; existing compact-specific API may remain as a compatibility adapter, delegating to the registry rather than duplicating execution.
- Create `desktop/src/slash-commands.ts`: `parseSlashCommand(text): { name: string; argumentsText: string } | null`, `filterTasks(tasks, query)`, and `taskUnavailableReason(task, state): string | null`.
- Create `desktop/src/slash-menu.ts`: `createSlashMenu(textarea, host, onChoose, onRetry)` with `update({ tasks, status, error, state, sessionId })`, `handleKeydown(event): boolean`, and `destroy()`. It owns popup focus/selection/dismissal, not execution or the chat store.

### Task 1: Rust registry and shared dispatch

**Files:** Create `src/tasks/mod.rs` and `tests/task_registry_behavior.rs`; modify `src/lib.rs`, `src/main.rs`, `desktop/src-tauri/src/commands.rs`, `desktop/src-tauri/src/lib.rs`, and native tests.

- [x] Write failing tests for listing the compact descriptor without model configuration; rejecting duplicate IDs/commands, unknown IDs, non-object/extra arguments, and missing conversation; successful typed dispatch reuses compaction without creating chat messages. Cover same-conversation exclusion and existing automatic-compaction/no-nested-lease tests.
- [x] Run `cargo test --test task_registry_behavior`; observe expected failure before implementation.
- [x] Implement the shared interfaces above. Keep compact metadata/schema/handler binding in one definition. Handler calls `ConversationService::compact_cancellable`; it does not take another execution lease around the service call.
- [x] Add synchronous `list_tasks` and asynchronous `run_task` commands. Validate request and ID before model/workspace setup, then acquire the existing native active-request guard once, derive saved workspace authority, build the model runtime, and dispatch with that guard's cancellation flag. Register both commands. Preserve compact progress events on the channel and return the typed task result.
- [x] Make `compact_conversation` a compatibility adapter using the same validated task implementation, not a second service call. Move CLI `ConversationCompact` through `TaskRegistry::run` and `COMPACT_TASK_ID` while preserving its current CLI grammar. Automatic runtime compaction keeps its direct shared implementation and current lease ownership.
- [x] Run core/native tests, formatting and both Clippy commands; expected all pass. Record results; final focused commit is in Task 4.

### Task 2: Catalog state, slash parsing and generic desktop dispatch

**Files:** Create `desktop/src/slash-commands.ts` and its tests; modify `types.ts`, `atlas-api.ts`, `chat-state.ts`, and relevant adapter/state/fake-API tests.

- [x] Write failing tests: list/run Tauri payloads and typed outcomes; independent catalog load without API-key/model discovery; retry failure/stale-response guards; ordinary chat during catalog failure; case-insensitive matching; bare/unknown slash command and extra compact arguments do not call ask/run; absolute paths and prose stay normal chat; failed task preserves its original draft without overwriting newer text.
- [x] Run `npm --prefix desktop test -- src/slash-commands.test.ts src/atlas-api.test.ts src/chat-state.test.ts`; verify the expected missing-feature failures.
- [x] Add typed catalog state (`tasks`, `taskCatalogStatus`, `taskCatalogError`) and a request token for catalog reloads. Load it independently during store initialization; expose `retryTasks()`. A missing catalog disables command dispatch with a retryable message but does not disable ordinary asks. Update all fake APIs explicitly; fixture descriptors may be shared in a test utility, never a production fallback catalog.
- [x] Implement parsing: recognize only first-token `/[A-Za-z][A-Za-z0-9_-]*` followed by whitespace/end, plus bare `/` for incomplete command feedback. Reject embedded slashes in that token as command syntax so `/home/user/file.rs` stays chat. Normalize command matching to lowercase. Return the remaining argument text separately. Use the descriptor’s empty-object argument schema to require empty argument text and send `{}`; unsupported argument schemas produce local feedback. Do not branch on the `compact` command name or silently ignore or guess arguments.
- [x] Replace `send()`'s compact-specific branch with descriptor lookup and a generic `runTask` store path. Compute availability from descriptor requirements and current state. Set per-chat task ownership before the call, use the existing generation guard/events, and route the typed compaction outcome to memory/progress/notice state. Preserve actual chat usage and saved history. A compatibility `compact()` store method, if retained for existing callers, must delegate to generic dispatch.
- [x] Keep result mapping explicit by outcome kind so future task results can add adapters without changing parsing. Task errors restore the submitted command only if the owning draft is still empty; keep another chat's draft untouched. Test background completion, cancellation, catalog retry, and conversation/model precondition changes.
- [x] Run full frontend tests/build; expected all pass. Record results; final focused commit is in Task 4.

### Task 3: Accessible slash task menu in the composer

**Files:** Create `desktop/src/slash-menu.ts` and its tests; modify `chat-composer.ts`, `chat-composer.test.ts`, `chat-view.test.ts`, and `styles.css`.

- [x] Write failing interaction tests: `/` opens tasks above composer; name/title/description filtering; no-result and catalog loading/error/retry states; row descriptions and disabled reasons; pointer selection, Up/Down/Home/End, Enter/Tab insertion only, subsequent submission, Escape/outside dismissal, unchanged-draft suppression, IME/Shift+Enter, and cursor movement outside the first token.
- [x] Add chat-switch tests proving popup selection/dismissal resets for the next session and background tasks never insert a command into the current composer. Test a selected command becoming unavailable and long/narrow menus staying within the viewport.
- [x] Run `npm --prefix desktop test -- src/slash-menu.test.ts src/chat-composer.test.ts src/chat-view.test.ts`; confirm failures before implementing.
- [x] Implement the menu with catalog-driven rows, listbox/active-descendant semantics on the focused textarea, unique IDs, results announcements, rounded surface, soft shadow, lavender active state, bounded height/scroll and viewport positioning. Use textContent for backend labels/descriptions. Remove document/window listeners and accessibility attributes during destroy.
- [x] Wire menu input/cursor updates before normal composer submission handling. `handleKeydown` returns true only when it consumes the event. Enabled Enter/Tab selection replaces the first command token with the canonical command, updates the store draft, returns typing focus, closes and suppresses reopening until the draft changes. A disabled selection consumes Enter and announces why; Tab may close and move focus normally. Shift+Enter/IME never choose a task.
- [x] Keep Send as explicit submission: menu selection alone must not invoke it. Escape/outside click preserve input. Refresh availability and suggestions on store/session changes without recreating the textarea, losing typing focus, or resetting an accepted command. Preserve normal Send/workspace/busy behavior and existing custom Model/Access menus.
- [x] Run full frontend tests/build; expected all pass. Record results; final focused commit is in Task 4.

### Task 4: Documentation, independent review and integration

**Files:** Update `README.md`, `AGENTS.md`, and this plan's checkboxes/verification record.

- [x] Document task discovery, insert-then-run keyboard behavior, disabled requirements, command errors, and adding a future built-in descriptor/handler/test without a frontend command list. Explain that model routing remains future work.
- [x] Run `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, native formatting/tests/Clippy, `npm --prefix desktop test`, `npm --prefix desktop run build`, and `npm --prefix desktop run tauri -- build --debug --no-bundle`. Inspect every result, preserving fake/loopback-only testing.
- [x] Request one fresh whole-change review against the spec and Review Focus. Resolve important findings in one regression/fix pass and rerun affected suites; record any deferred minor findings explicitly.
- [ ] Review status and staged diff, excluding secrets, generated output, dependency folders and unrelated changes. Create one focused commit: `feat: add task registry and slash command menu`.
- [ ] Follow the established local merge-to-main workflow, verify merged core/native/frontend results, remove only this task's isolated worktree/branch, and report behavior, checks and commit ID.

## Self-review and handoff

Spec coverage: Task 1 owns registry/dispatch/authority and CLI compatibility; Task 2
owns catalog availability, parsing, argument checks and request/session state;
Task 3 owns discoverability, keyboard/pointer/focus behavior and composer integration;
Task 4 owns documentation, review, checks and local integration. Review Focus cases
are assigned to tests in their owning tasks. Implementation is complete through Task 3; final verification and independent review are tracked below.

Use `.worktrees/task-registry-slash-menu` on a feature branch at execution time,
reusing the repository build targets and desktop dependency installation. Keep an
ignored progress ledger with verification evidence and any justified rulings.
After this written plan is reviewed, execute continuously in this session.

## Verification record

- Core: 288 tests passed; formatting and Clippy passed.
- Native: 45 tests passed; formatting and Clippy passed.
- Desktop: 182 tests passed; TypeScript and Vite build passed.
- Existing `serde_json` native test dependency promoted to a runtime dependency for typed task request construction; no new crate/version.
- Debug Tauri build passed. One independent review identified keyboard Retry focus and asynchronous cancellation-error ownership; regressions failed first, both fixes passed the full 182-test frontend suite and build.
- Live native-webview layout and screen-reader interoperability were not manually verified; automated interaction/ARIA/viewport coverage and native build passed. No deferred minor findings.
