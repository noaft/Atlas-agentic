# Cross-Platform OS Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add user-selectable, runtime-enforced permissions and a consistent Linux, macOS, and Windows toolset for workspace file management, launching, and command execution.

**Architecture:** Extend tools with argument-sensitive authorization metadata and have `Runtime` enforce a conversation permission mode through an injected authorizer. Persist the default mode in settings and the authoritative per-chat mode in conversation metadata, then bridge approval requests through CLI prompts or typed Tauri events. Keep filesystem actions workspace-scoped and isolate platform launch/shell details behind conditional adapters.

**Tech Stack:** Rust, serde/serde_json, cap-std, Tauri 2 IPC channels, TypeScript DOM UI, Vitest.

**Spec:** `docs/superpowers/specs/2026-10-10-cross-platform-os-tools-design.md`

## Global Constraints

- Support Linux, macOS, and Windows from the first release.
- Structured filesystem tools reject absolute paths and `..` and remain workspace-scoped in every permission mode.
- `exec_command` is unrestricted in Full access but runs only with the Atlas process account.
- Existing settings and conversation indexes load as permission mode `ask`.
- Saved-conversation metadata is authoritative over frontend ask payloads.
- Only successful turns persist user/assistant messages and first-turn metadata.
- No shell, process, or unrestricted filesystem permission is exposed to frontend JavaScript.
- Follow TDD and preserve unrelated work.
- The final branch is squashed to one focused implementation commit after task-level execution commits, preserving the repository traceability rule alongside this committed design.

## Review Focus

- A symlink swapped between validation and execution must not let a structured file operation escape the workspace; Task 3 adds race-oriented containment tests.
- A stale or duplicate desktop approval response must not authorize another request; Task 6 tests single-use opaque IDs.
- A legacy conversation with no permission field must use Ask and acquire a mode only on its next successful turn; Task 1 tests both failure and success paths.
- Windows command/launcher arguments containing spaces, quotes, and shell metacharacters must preserve intended boundaries; Task 4 tests adapter construction without launching real apps.
- Leaving a chat while approval is pending must cancel the turn without persisted messages; Tasks 5 and 7 test backend cancellation and frontend state cleanup.

---

### Task 1: Permission settings and conversation persistence

**Files:**
- Modify: `src/settings.rs`
- Modify: `src/conversations/types.rs`
- Modify: `src/conversations/store.rs`
- Modify: `src/conversations/service.rs`
- Test: `tests/settings_behavior.rs`
- Test: `tests/conversation_store_behavior.rs`
- Test: `tests/conversation_service_behavior.rs`

**Interfaces:**
- Produces: `PermissionMode::{Ask, Workspace, FullAccess}`, serde value names `ask`, `workspace`, `fullAccess`, and `Default` returning `Ask`.
- Produces: `AtlasSettings.permissions.default_mode: PermissionMode` and `EffectiveSettings.default_permission_mode: PermissionMode`.
- Produces: `ConversationMetadata.permission_mode: Option<PermissionMode>` for legacy-aware storage.
- Produces: store methods `set_permission_mode(&ConversationId, PermissionMode)` and turn creation/append variants accepting a mode to assign only on successful persistence.
- Consumes: existing locked atomic settings/index update patterns and workspace assignment semantics.

- [ ] **Step 1: Write failing permission persistence tests**

  Add tests proving default settings resolve to Ask, all three values round-trip, legacy settings without `permissions` load as Ask, legacy conversation index entries load with `None`, failed first turns persist no mode, successful first turns store the selected mode, legacy continuations assign the effective mode only after success, and explicit updates affect only the selected conversation.

- [ ] **Step 2: Run the focused tests and verify RED**

  Run: `cargo test --test settings_behavior --test conversation_store_behavior --test conversation_service_behavior`

  Expected: compilation/test failures because permission types and APIs do not exist.

- [ ] **Step 3: Implement permission persistence**

  Add the exact interfaces above with serde defaults. Extend locked store mutations and `ConversationService` ask entry points so a caller supplies the effective mode, following the existing workspace rule that failed turns make no metadata changes.

- [ ] **Step 4: Run focused and full core tests**

  Run: `cargo test --test settings_behavior --test conversation_store_behavior --test conversation_service_behavior && cargo test`

  Expected: all tests pass.

- [ ] **Step 5: Commit task**

  Commit message: `feat: persist conversation permission modes`

### Task 2: Runtime authorization policy

**Files:**
- Create: `src/permissions.rs`
- Modify: `src/lib.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/runtime.rs`
- Modify: `src/app.rs`
- Test: `tests/tool_call_behavior.rs`
- Create: `tests/permission_behavior.rs`

**Interfaces:**
- Consumes: `PermissionMode` from Task 1.
- Produces: `ToolRisk::{ReadOnly, ModifyWorkspace, Destructive, Launch, Command}`.
- Produces: `AuthorizationRequest { id, tool, arguments, reason }`, `AuthorizationDecision::{Allow, Deny}`, and `ToolAuthorizer::authorize(&AuthorizationRequest) -> Result<AuthorizationDecision, String>`.
- Produces: `Tool::risk(&serde_json::Value) -> Result<ToolRisk, String>` and bounded/redacted `Tool::summarize(&serde_json::Value)` defaults.
- Produces: runtime ask methods accepting `PermissionMode`, `&dyn ToolAuthorizer`, and an optional typed tool-event sink while preserving convenience entry points for tests without tools.

- [ ] **Step 1: Write failing policy and runtime tests**

  Cover every matrix row in the spec, including overwrite escalation, authorizer bypass for allowed calls, allow, denial returned as a tool result, authorizer error failing the turn, secret-key redaction, argument truncation, and tool-call limit behavior after denial.

- [ ] **Step 2: Run the focused tests and verify RED**

  Run: `cargo test --test permission_behavior --test tool_call_behavior`

  Expected: failures because policy types and runtime authorization do not exist.

- [ ] **Step 3: Implement policy enforcement**

  Add a pure `requires_confirmation(mode, risk) -> bool`, authorization data types, safe summaries, tool-event types, and the authorization gate immediately before registry execution. A denial record must say the user denied the named tool and must be added to the normal continuation.

- [ ] **Step 4: Run focused and full core tests**

  Run: `cargo test --test permission_behavior --test tool_call_behavior && cargo test`

  Expected: all tests pass.

- [ ] **Step 5: Commit task**

  Commit message: `feat: enforce runtime tool permissions`

### Task 3: Workspace filesystem tool family

**Files:**
- Create: `src/tools/workspace_path.rs`
- Create: `src/tools/search_files.rs`
- Create: `src/tools/create_directory.rs`
- Create: `src/tools/copy_file.rs`
- Create: `src/tools/move_file.rs`
- Create: `src/tools/delete_file.rs`
- Modify: `src/tools/create_file.rs`
- Modify: `src/tools/read_file.rs`
- Modify: `src/tools/list_dir.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/app.rs`
- Create: `tests/search_files_behavior.rs`
- Create: `tests/file_management_behavior.rs`
- Modify: `tests/tool_call_behavior.rs`

**Interfaces:**
- Consumes: `Tool`, `ToolRisk`, and runtime registry from Task 2.
- Produces: shared relative-path parsing and cap-std workspace traversal helpers used by all structured filesystem tools.
- Produces: `SearchFilesTool`, `CreateDirectoryTool`, `CopyFileTool`, `MoveFileTool`, and `DeleteFileTool` with the schemas and behavior fixed by the spec.

- [ ] **Step 1: Write failing file-tool tests**

  Cover definitions, successful nested operations, missing parents, existing targets, explicit overwrite, directories passed as files, absolute/parent paths, symlink escapes, atomic symlink swaps, same-source/destination, cross-filesystem fallback through an injectable move primitive, filename/content matches, UTF-8 line excerpts, binary skipping, hidden/generated defaults, glob filtering, deterministic ordering, and hard result limits.

- [ ] **Step 2: Run the focused tests and verify RED**

  Run: `cargo test --test search_files_behavior --test file_management_behavior`

  Expected: compilation failures because the tools do not exist.

- [ ] **Step 3: Implement shared path safety and tools**

  Extract only path validation shared by at least two tools. Use capability-directory operations rather than canonicalize-then-use for mutations. Search iteratively, never follow symlink directories, cap excerpts and total results, and return paths relative to the workspace.

- [ ] **Step 4: Register tools and update registry expectations**

  Register all five tools in `build_default_tools`; update exact built-in-name assertions and the default Atlas prompt test only where its documented tool list requires it.

- [ ] **Step 5: Run focused and full core tests**

  Run: `cargo test --test search_files_behavior --test file_management_behavior --test tool_call_behavior && cargo test`

  Expected: all tests pass.

- [ ] **Step 6: Commit task**

  Commit message: `feat: add workspace file management tools`

### Task 4: Cross-platform launch and command adapters

**Files:**
- Create: `src/tools/platform.rs`
- Create: `src/tools/open_file.rs`
- Create: `src/tools/open_application.rs`
- Modify: `src/tools/exec_command.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/app.rs`
- Modify: `Cargo.toml`
- Create: `tests/open_tools_behavior.rs`
- Modify: `tests/exec_command_behavior.rs`
- Modify: `tests/tool_call_behavior.rs`

**Interfaces:**
- Consumes: shared workspace path safety and permission risks.
- Produces: platform-neutral command specifications for opening files/apps and choosing the system shell.
- Produces: `OpenFileTool` and `OpenApplicationTool`.
- Produces: `ExecCommandTool` on all targets with Unix process-group and Windows job-object timeout cleanup.

- [ ] **Step 1: Write failing adapter and launch tests**

  Test platform command specifications for Linux, macOS, and Windows as pure functions, including spaces, quotes, and metacharacters. Test file containment/type checks, application empty-name/argument validation, asynchronous success, launcher spawn failure, and risk classification without opening a real application.

- [ ] **Step 2: Run focused tests and verify RED**

  Run: `cargo test --test open_tools_behavior --test exec_command_behavior`

  Expected: compilation/test failures because adapters and launch tools do not exist.

- [ ] **Step 3: Implement launch adapters and tools**

  Construct `std::process::Command` arguments directly. Keep platform-specific escaping isolated and expose injectable spawners for deterministic tests.

- [ ] **Step 4: Generalize `exec_command`**

  Remove Unix-only registration. Preserve the public schema/result format and existing Unix behavior. Add cfg-gated Windows process-tree cleanup with the smallest maintained dependency necessary, and compile-check cfg-neutral adapter code on the host.

- [ ] **Step 5: Run focused and full core tests**

  Run: `cargo test --test open_tools_behavior --test exec_command_behavior --test tool_call_behavior && cargo test`

  Expected: all host tests pass.

- [ ] **Step 6: Commit task**

  Commit message: `feat: add cross-platform launch and command tools`

### Task 5: Conversation orchestration and CLI approval

**Files:**
- Modify: `src/conversations/service.rs`
- Modify: `src/main.rs`
- Modify: `src/lib.rs`
- Test: `tests/conversation_service_behavior.rs`
- Test: `tests/cli_binary.rs`

**Interfaces:**
- Consumes: persisted effective permission mode, `ToolAuthorizer`, and runtime event sink.
- Produces: terminal authorizer that writes a bounded request to stderr, reads an exact yes/no response from stdin, and denies on EOF.
- Produces: conversation ask orchestration that resolves saved authority before runtime creation and persists only successful first-turn/draft assignments.

- [ ] **Step 1: Write failing service and CLI tests**

  Cover authoritative saved mode over a conflicting request, legacy fallback, allow, deny, EOF, invalid input retry, full-access no-prompt, and cancellation/error leaving storage unchanged. Use fake providers and child stdin; never call a real model service.

- [ ] **Step 2: Run tests and verify RED**

  Run: `cargo test --test conversation_service_behavior --test cli_binary`

  Expected: failures because orchestration does not pass permission or authorizers.

- [ ] **Step 3: Implement orchestration and terminal approval**

  Resolve the mode alongside workspace before building the runtime. Keep terminal interaction in the CLI and inject it through the shared trait. Render denial and transport errors distinctly.

- [ ] **Step 4: Run focused and full core tests**

  Run: `cargo test --test conversation_service_behavior --test cli_binary && cargo test`

  Expected: all tests pass.

- [ ] **Step 5: Commit task**

  Commit message: `feat: add CLI tool approvals`

### Task 6: Desktop backend approval bridge

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/capabilities/default.json`
- Test: `desktop/src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: runtime authorization/events and persisted permission modes.
- Produces: chat events `approvalRequested`, `toolStarted`, and `toolFinished`.
- Produces: Tauri commands `set_conversation_permission_mode` and `resolve_tool_approval`.
- Produces: a mutex-protected pending registry keyed by unpredictable single-use approval IDs, whose entries are scoped to an ask and removed on decision, channel failure, timeout, or ask completion.

- [ ] **Step 1: Write failing desktop Rust tests**

  Test DTO serialization, saved-mode authority, new-draft validation/persistence, allow/deny delivery, unknown/duplicate/stale ID rejection, timeout cleanup, channel-close cancellation, and no partial conversation storage.

- [ ] **Step 2: Run desktop tests and verify RED**

  Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml`

  Expected: compilation/test failures because commands and events do not exist.

- [ ] **Step 3: Implement pending approvals and IPC**

  Use standard synchronization channels between the blocking ask worker and async Tauri command. Generate opaque UUID approval IDs, remove before delivering a decision, and use a bounded timeout. Do not grant additional Tauri plugin permissions.

- [ ] **Step 4: Run desktop Rust tests**

  Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml`

  Expected: all tests pass.

- [ ] **Step 5: Commit task**

  Commit message: `feat: bridge desktop tool approvals`

### Task 7: Desktop permission and approval UI

**Files:**
- Modify: `desktop/src/types.ts`
- Modify: `desktop/src/atlas-api.ts`
- Modify: `desktop/src/chat-state.ts`
- Modify: `desktop/src/chat-composer.ts`
- Modify: `desktop/src/chat-view.ts`
- Modify: `desktop/src/settings-view.ts`
- Modify: `desktop/src/styles.css`
- Modify: `desktop/src/atlas-api.test.ts`
- Modify: `desktop/src/chat-state.test.ts`
- Modify: `desktop/src/chat-composer.test.ts`
- Modify: `desktop/src/chat-view.test.ts`
- Modify: `desktop/src/settings-view.test.ts`

**Interfaces:**
- Consumes: Task 6 DTOs, events, and commands.
- Produces: composer permission picker, settings default picker, approval cards, tool activity rows, and store actions for selecting modes and resolving approvals.

- [ ] **Step 1: Write failing frontend tests**

  Test picker labels/options, settings default hydration/save, draft override isolation, saved-chat restoration/update, disabled changes while busy, Full access warning, exact ask payload, approval allow/deny IPC, accessible waiting state, tool activity updates, stale event ignoring, and navigation cancellation without local message persistence.

- [ ] **Step 2: Run frontend tests and verify RED**

  Run: `npm --prefix desktop test`

  Expected: test failures because state, UI, and API contracts do not exist.

- [ ] **Step 3: Implement TypeScript state and API contracts**

  Extend discriminated event types and keep approval state scoped by ask token and approval ID. A saved chat updates through the backend before its local picker changes; a draft update remains local.

- [ ] **Step 4: Implement UI and styling**

  Reuse the model-picker visual language. Add concise descriptions, a warning treatment for Full access, keyboard-accessible approval buttons, and responsive wrapping without shrinking the send button.

- [ ] **Step 5: Run frontend tests and build**

  Run: `npm --prefix desktop test && npm --prefix desktop run build`

  Expected: all tests pass and build exits zero.

- [ ] **Step 6: Commit task**

  Commit message: `feat: add desktop permission controls`

### Task 8: Documentation, full verification, and focused history

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `VISION.md` only if the implemented behavior changes its stated direction
- Modify: implementation files only for test-first fixes found by verification

**Interfaces:**
- Consumes: all completed behavior.
- Produces: user/developer documentation and a verified branch with one focused implementation commit after the design commit.

- [ ] **Step 1: Write documentation checks or update existing doc assertions first**

  Add/update tests that assert the default agent documentation names the complete registered toolset and explains approval behavior where executable documentation already exists.

- [ ] **Step 2: Verify the documentation test fails, then update docs**

  Describe modes, selector persistence, tool schemas at a user level, cross-platform shell behavior, Full access authority, and structured-tool workspace containment.

- [ ] **Step 3: Run every repository verification command**

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

  Expected: every command exits zero with no warnings promoted to errors.

- [ ] **Step 4: Review requirements and staged diff**

  Check every spec section against code/tests, run `git status --short`, inspect the complete diff from the design commit, and confirm no secrets, `.env`, build output, or unrelated files are present.

- [ ] **Step 5: Squash task implementation commits and commit once**

  Preserve `a6d5633` as the approved design commit. Soft-reset only the implementation task commits to that commit, stage the reviewed implementation/docs, and commit as `feat: add cross-platform OS tool permissions`.
