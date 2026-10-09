# Automatic Atlas Home Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the desktop app immediately usable as a general OS assistant by provisioning an automatic `Atlas` home folder while keeping project folders optional and Bash unrestricted within normal OS permissions.

**Architecture:** Add a shared Rust helper that provisions the automatic workspace, inject it into the desktop state, and preserve the existing workspace precedence at one backend boundary. Extend successful-turn persistence so legacy conversations can receive the resolved workspace with their next saved turn, then adapt frontend state and copy to treat the workspace as a shell starting directory rather than a sandbox.

**Tech Stack:** Rust 2021, `directories`, Tauri 2, TypeScript 7, Vite 8, Vitest 5, JSONL conversation storage.

**Spec:** `docs/superpowers/specs/2026-10-09-automatic-atlas-home-design.md`

## Global Constraints

- Use the platform home-directory API and the exact folder name `Atlas`; do not hard-code a platform path.
- Resolution order for a new desktop chat is explicit draft folder, saved custom default, then automatic Atlas home.
- A saved conversation's persisted workspace remains authoritative; frontend input cannot override it.
- Assign a legacy conversation's fallback workspace only with a successfully persisted turn.
- Keep `pwd`, `ls`, `read_file`, and `create_file` workspace-scoped under `RULE.md`.
- Keep `exec_command` unrestricted on Linux, WSL, and macOS and subject to the Atlas process's OS permissions.
- Keep the CLI's working directory behavior unchanged.
- Preserve the rule that failed or partial requests save neither messages nor workspace metadata.
- Do not persist the automatic fallback path in `settings.json`; persist only user-selected overrides.
- Create one focused implementation commit after all tasks pass, as required by `AGENTS.md`.

## Review Focus

- An unavailable home lookup, a home path that is a regular file, a non-Unicode result, or a directory-creation failure must fail before runtime construction or conversation mutation (Task 1 and Task 3 tests).
- A deleted explicit default must report that stale choice and must not silently create or use the automatic folder (Task 3 tests).
- A saved conversation, including one changed concurrently from unassigned to assigned, must keep its persisted assignment over a fallback or frontend path (Task 2 and Task 3 tests).
- Async settings completion must not replace a project folder or conversation selected while initialization is in flight (Task 4 tests).
- Resetting a custom default changes future new chats to Atlas home without moving the currently active saved conversation (Task 4 and Task 5 tests).

---

### Task 1: Automatic Workspace Provisioning

**Files:**
- Create: `src/workspace.rs`
- Modify: `src/lib.rs`
- Create: `tests/workspace_behavior.rs`

**Interfaces:**
- Consumes: `directories::BaseDirs`, `conversations::validate_workspace`.
- Produces: `pub const AUTOMATIC_WORKSPACE_DIR_NAME: &str = "Atlas"`, `pub fn provision_automatic_workspace(home: Option<&Path>) -> Result<PathBuf, String>`, and `pub fn automatic_workspace() -> Result<PathBuf, String>`.

- [ ] **Step 1: Write failing provisioning tests**

Add tests named `provisions_and_canonicalizes_atlas_folder_inside_home`, `rejects_an_unavailable_home_directory`, `rejects_a_home_path_that_is_not_a_directory`, and Unix-only `rejects_a_non_unicode_automatic_workspace`. Assert the exact child name `Atlas`, directory creation, canonical output, the exact unavailable-home error, path-bearing creation errors, and the existing Unicode validation error.

- [ ] **Step 2: Run the focused test and confirm failure**

Run: `cargo test --test workspace_behavior`

Expected: FAIL because `atlas_agentic::workspace` does not exist.

- [ ] **Step 3: Implement the focused workspace module**

Implement `provision_automatic_workspace` by rejecting `None` with `could not resolve the user home directory`, then applying `fs::create_dir_all(home.join(AUTOMATIC_WORKSPACE_DIR_NAME))` followed by `validate_workspace`. Implement `automatic_workspace` with `BaseDirs::new()` and delegate its optional home path. Export the module from `src/lib.rs`.

- [ ] **Step 4: Verify Task 1**

Run: `cargo test --test workspace_behavior`

Expected: all workspace behavior tests PASS.

### Task 2: Successful Legacy Workspace Assignment

**Files:**
- Modify: `src/conversations/store.rs`
- Modify: `src/conversations/service.rs`
- Modify: `tests/conversation_store_behavior.rs`
- Modify: `tests/conversation_service_behavior.rs`

**Interfaces:**
- Consumes: a validated resolved workspace supplied to `ConversationService::ask_with_workspace_and_fallible_sink`.
- Produces: `ConversationStore::append_turn_with_workspace_if_missing(&self, id: &ConversationId, user: &str, assistant: &str, workspace: Option<&Path>) -> Result<ConversationMetadata, String>`; existing `append_turn` delegates with `None`.

- [ ] **Step 1: Write failing store tests**

Add `append_assigns_workspace_only_when_missing` and `append_preserves_an_existing_workspace_over_a_fallback`. Assert one appended user/assistant pair, four total messages, a canonical assigned path for the legacy record, and unchanged assignment for an already assigned record.

- [ ] **Step 2: Confirm the store tests fail**

Run: `cargo test --test conversation_store_behavior append_`

Expected: FAIL because `append_turn_with_workspace_if_missing` does not exist.

- [ ] **Step 3: Implement the atomic store operation**

Within the existing exclusive lock, validate the optional candidate before history mutation, reconcile current metadata, set `workspace_path` only when it is `None`, append the messages, and publish the updated metadata. Retain `append_turn` as a compatibility wrapper.

- [ ] **Step 4: Verify the store behavior**

Run: `cargo test --test conversation_store_behavior`

Expected: all conversation store tests PASS.

- [ ] **Step 5: Write failing service tests**

Add `successful_legacy_continuation_assigns_the_resolved_workspace` and extend the existing provider/sink failure test to assert that an unassigned conversation remains unassigned after either failure.

- [ ] **Step 6: Connect successful continuation persistence**

Change the continuation branch in `ask_with_workspace_and_fallible_sink` to call `append_turn_with_workspace_if_missing` with its workspace argument. Leave the new-conversation branch and CLI wrapper behavior intact.

- [ ] **Step 7: Verify Task 2**

Run: `cargo test --test conversation_service_behavior --test conversation_store_behavior`

Expected: both suites PASS.

### Task 3: Desktop Workspace Resolution and Settings Contract

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `atlas_agentic::workspace::automatic_workspace` and Task 2's successful continuation assignment.
- Produces: injected `type AutomaticWorkspaceResolver = Arc<dyn Fn() -> Result<PathBuf, String> + Send + Sync>`; `SettingsViewDto.default_workspace: WorkspaceDto`; `SettingsViewDto.default_workspace_automatic: bool` serialized as `defaultWorkspaceAutomatic`.

- [ ] **Step 1: Write failing desktop backend tests**

Add tests for: automatic folder fallback on a new chat; requested folder precedence; configured default precedence; assigned-conversation authority over a conflicting request; legacy fallback and successful assignment; provider and sink failures preserving legacy metadata; stale configured default refusing fallback; and settings views marking automatic versus configured defaults. Inject temporary automatic-workspace resolvers so no test touches the real home directory.

- [ ] **Step 2: Confirm the backend tests fail**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml`

Expected: FAIL on missing fallback resolver and Settings DTO fields.

- [ ] **Step 3: Inject and use the automatic resolver**

Add the resolver to `DesktopState`; production construction supplies `automatic_workspace`. Add a private `resolve_default_workspace(&self, effective: &EffectiveSettings) -> Result<(PathBuf, bool), String>` that validates a configured default and otherwise provisions the automatic folder. Update `resolve_workspace` so assigned conversations remain authoritative and unassigned conversations use this default.

- [ ] **Step 4: Pass resolved workspace through successful persistence**

Pass `Some(workspace.as_path())` to the conversation service for both new and continued desktop requests. Task 2 ensures it affects only new or currently unassigned records.

- [ ] **Step 5: Return the effective default in Settings**

Make `load_settings_impl` and `save_settings_impl` use the injected resolver. Return a non-null `default_workspace` and `default_workspace_automatic`; keep `SaveSettingsInput.default_workspace_path: Option<String>` so `None` means reset to automatic.

- [ ] **Step 6: Verify Task 3**

Run: `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check && cargo test --manifest-path desktop/src-tauri/Cargo.toml`

Expected: formatting and all desktop Rust tests PASS.

### Task 4: Frontend Default-Workspace State

**Files:**
- Modify: `desktop/src/types.ts`
- Modify: `desktop/src/chat-state.ts`
- Modify: `desktop/src/chat-state.test.ts`
- Modify: `desktop/src/chat-view.test.ts`

**Interfaces:**
- Consumes: Task 3's always-present `SettingsView.defaultWorkspace` and `defaultWorkspaceAutomatic` flag.
- Produces: drafts and legacy conversations immediately use the effective default; `SettingsDraft.defaultWorkspace: Workspace | null` continues to encode custom override versus automatic reset.

- [ ] **Step 1: Update fixtures and write failing state tests**

Change `SettingsView.defaultWorkspace` to `Workspace` and add `defaultWorkspaceAutomatic: boolean`. Add tests named `initialization_uses_automatic_workspace_without_picker`, `new_chat_restores_effective_default_without_picker`, `legacy_conversation_uses_default_until_successful_assignment`, `late_settings_do_not_override_a_selected_workspace`, and `deleting_or_resetting_keeps_saved_conversation_workspace_authoritative`.

- [ ] **Step 2: Confirm the frontend tests fail**

Run: `npm --prefix desktop test -- chat-state.test.ts chat-view.test.ts`

Expected: FAIL because state still opens or requires the picker and fixtures lack the new contract.

- [ ] **Step 3: Implement default state transitions**

Initialize and reset drafts from `settings.defaultWorkspace`; remove the picker call from `newChat`; resolve a loaded legacy conversation as `metadata.workspace ?? settings.defaultWorkspace`; keep token guards so late settings cannot override newer navigation; and reset a post-delete draft to the effective default. Build settings drafts with `null` only when `defaultWorkspaceAutomatic` is true.

- [ ] **Step 4: Keep send persistence inputs precise**

Continue passing the active workspace only for new conversations and `null` for saved conversations. Retain the no-workspace error only for settings initialization or provisioning failure states.

- [ ] **Step 5: Verify Task 4**

Run: `npm --prefix desktop test -- chat-state.test.ts chat-view.test.ts`

Expected: focused frontend tests PASS.

### Task 5: General Assistant UI and Settings Copy

**Files:**
- Modify: `desktop/src/chat-view.ts`
- Modify: `desktop/src/settings-view.ts`
- Modify: `desktop/src/chat-view.test.ts`
- Modify: `desktop/src/settings-view.test.ts`
- Modify if selectors need styling: `desktop/src/styles.css`

**Interfaces:**
- Consumes: automatic/custom state from Task 4.
- Produces: general-assistant welcome and truthful starting-folder language; Settings reset action that writes `defaultWorkspace: null`.

- [ ] **Step 1: Write failing copy and interaction tests**

Assert the brand subtitle `Computer assistant`, welcome heading `How can I help?`, enabled composer with the automatic workspace, workspace action remaining available, help text that calls the path a starting folder and mentions access elsewhere, automatic label `Atlas home (automatic)`, and custom-default action `Reset to Atlas home`.

- [ ] **Step 2: Confirm UI tests fail**

Run: `npm --prefix desktop test -- chat-view.test.ts settings-view.test.ts`

Expected: FAIL on the old coding-only and folder-required copy.

- [ ] **Step 3: Implement the chat copy and controls**

Replace coding-only brand, welcome, placeholder, and composer-note text. Keep the workspace card action as the optional project-folder picker and preserve accessibility labels and IME-safe composer behavior.

- [ ] **Step 4: Implement automatic/custom Settings presentation**

When the draft override is `null`, display `Atlas home (automatic)` with the effective path from `settings.defaultWorkspace`; label the choose action `Choose custom folder`; show `Reset to Atlas home` only for a custom draft and set it to `null` when clicked.

- [ ] **Step 5: Verify Task 5**

Run: `npm --prefix desktop test`

Expected: all frontend tests PASS.

### Task 6: Agent Behavior and Product Documentation

**Files:**
- Modify: `agents/atlas.toml`
- Modify: `tests/agents_behavior.rs`
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `VISION.md`

**Interfaces:**
- Consumes: the final workspace and tool semantics.
- Produces: the shared default-agent guidance and user/developer documentation for the delivered behavior.

- [ ] **Step 1: Write the failing prompt test**

Rename the default-prompt test for general assistant behavior and require language covering conversation, files, applications, shell/computer tasks, coding, relevant-context inspection, runtime-provided tools, unrelated-data preservation, verification, and honest blockers. Retain assertions that tool names and protocol JSON are not embedded.

- [ ] **Step 2: Confirm the prompt test fails**

Run: `cargo test --test agents_behavior default_atlas_prompt`

Expected: FAIL because the current prompt positions Atlas around coding and project guidance.

- [ ] **Step 3: Update the configurable agent prompt**

Revise only `agents/atlas.toml`; keep provider, model, sampling fields, and title prompt unchanged. Describe a general local assistant and state that a workspace is the starting context while runtime-provided shell access may reach other OS-permitted locations.

- [ ] **Step 4: Update repository documentation**

Document automatic Atlas home creation, optional project folders, resolution precedence, legacy assignment on success, unrestricted Bash versus scoped file helpers, unchanged CLI current-directory behavior, and general-assistant positioning. Remove instructions that require choosing a folder before the first desktop message.

- [ ] **Step 5: Verify Task 6**

Run: `cargo test --test agents_behavior && rg -n "Atlas home|starting (folder|directory)|general.*assistant|exec_command" README.md AGENTS.md VISION.md`

Expected: prompt tests PASS and each document contains the new behavior.

### Task 7: Full Verification and Focused Commit

**Files:**
- Review all files changed by Tasks 1–6.

**Interfaces:**
- Consumes: all prior task deliverables.
- Produces: one verified implementation commit with no unrelated files.

- [ ] **Step 1: Run root Rust quality gates**

Run: `cargo fmt --check && cargo test && cargo clippy --all-targets --all-features -- -D warnings`

Expected: all commands exit 0.

- [ ] **Step 2: Run desktop Rust quality gates**

Run: `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check && cargo test --manifest-path desktop/src-tauri/Cargo.toml && cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`

Expected: all commands exit 0.

- [ ] **Step 3: Run frontend and Tauri build gates**

Run: `npm --prefix desktop test && npm --prefix desktop run build && npm --prefix desktop run tauri -- build --debug --no-bundle`

Expected: all commands exit 0 and the debug desktop binary builds.

- [ ] **Step 4: Review repository state**

Run: `git status --short && git diff --check && git diff --stat && git diff`

Expected: only Task 1–6 files are changed; no `.env`, secret, generated build output, or unrelated changes appear.

- [ ] **Step 5: Create the traceability commit**

```bash
git add src/lib.rs src/workspace.rs src/conversations/store.rs src/conversations/service.rs tests/workspace_behavior.rs tests/conversation_store_behavior.rs tests/conversation_service_behavior.rs desktop/src-tauri/src/commands.rs desktop/src-tauri/src/lib.rs desktop/src/types.ts desktop/src/chat-state.ts desktop/src/chat-state.test.ts desktop/src/chat-view.ts desktop/src/chat-view.test.ts desktop/src/settings-view.ts desktop/src/settings-view.test.ts desktop/src/styles.css agents/atlas.toml tests/agents_behavior.rs README.md AGENTS.md VISION.md
git diff --cached --check
git diff --cached --stat
git commit -m "feat: add automatic Atlas home"
```

- [ ] **Step 6: Confirm the handoff state**

Run: `git status --short && git log -2 --oneline`

Expected: clean status and the implementation commit immediately above the design/plan commits.
