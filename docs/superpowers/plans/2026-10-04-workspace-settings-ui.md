# Workspace Settings UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the approved folder-first Atlas desktop UI, persist one workspace per conversation, load model settings from a private local file, and preserve Vietnamese IME input.

**Architecture:** A new root-library settings module owns versioned file storage and effective environment precedence. Conversation metadata gains an optional canonical workspace, while the Tauri bridge resolves each ask from saved metadata and exposes narrow settings/workspace DTOs. The TypeScript store remains the single UI state owner; the native dialog plugin is isolated behind `AtlasApi`.

**Tech Stack:** Rust 2021, serde/serde_json, directories, fs2, Tauri 2.12, Tauri dialog plugin v2, vanilla TypeScript 7, Vite 8, Vitest/jsdom.

**Spec:** `docs/superpowers/specs/2026-10-04-workspace-settings-ui-design.md`

## Global Constraints

- New desktop chats require a workspace; `New chat` remains unsaved until the first successful answer.
- Saved conversations use their stored workspace as the only tool/runtime authority.
- Existing conversation indexes without `workspacePath` must load unchanged.
- Environment variables override saved settings without overwriting the saved file.
- The API key is saved only in the private settings file and never returned as readable text, logged, or embedded in conversation data.
- Settings writes are atomic; Unix files and directories are user-private.
- `ATLAS_CONFIG_DIR` is the test/development override for the Atlas configuration directory.
- Keep CLI behavior compatible: CLI asks use the current working directory as their workspace.
- Add only the Tauri dialog `open` permission; do not add shell, process, or unrestricted filesystem permissions.
- Preserve the approved pastel-light interface and Atlas Compass icon direction.

## Review Focus

- A saved/default workspace deleted or replaced before an ask must fail before provider invocation or history mutation; Task 3 and Task 4 pin this.
- A non-UTF-8 workspace path returned on Unix must produce a stable validation error, never lossy persistence; Task 3 pins this.
- Concurrent settings writers must leave one complete valid private JSON document, never a torn file; Task 1 pins this.
- An environment-provided API key must stay masked and a blank Settings save must not erase or copy it into the file; Task 4 and Task 5 pin this.
- A list/settings completion during active IME composition must not replace the textarea or submit the composition-confirmation Enter key; Task 6 pins this.

---

### Task 1: Versioned Private Settings Store

**Files:**
- Create: `src/settings.rs`
- Create: `tests/settings_behavior.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `AtlasSettings`, `DeepSeekSettings`, `ConversationSettings`, `WorkspaceSettings`, and cloneable `SettingsStore`.
- Produces: `SettingsStore::from_user_config()`, `SettingsStore::at(PathBuf)`, `load() -> Result<AtlasSettings, String>`, and `save(&AtlasSettings) -> Result<(), String>`.
- Consumes: existing `directories`, `fs2`, `serde`, and `serde_json` dependencies; no new storage framework.

- [ ] **Step 1: Write failing settings-store tests**

Add tests named:

- `missing_settings_file_returns_version_one_defaults`
- `round_trip_preserves_api_base_model_history_and_default_workspace`
- `rejects_zero_history_blank_model_blank_base_url_and_unknown_version`
- `failed_publication_preserves_previous_settings`
- `concurrent_writers_leave_one_complete_valid_document`
- Unix: `settings_permissions_are_private_with_permissive_umask`

Use literal defaults: base URL `https://api.deepseek.com`, model `deepseek-flash`, history limit `40`, and no API key/default workspace.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test --test settings_behavior`

Expected: compilation fails because `atlas_agentic::settings` does not exist.

- [ ] **Step 3: Implement the typed schema and store**

Implement:

```rust
pub const SETTINGS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtlasSettings { /* version, deepseek, conversation, workspace */ }

#[derive(Clone, Debug)]
pub struct SettingsStore { /* settings path, publisher, lock path */ }

impl SettingsStore {
    pub fn from_user_config() -> Result<Self, String>;
    pub fn at(path: PathBuf) -> Self;
    pub fn load(&self) -> Result<AtlasSettings, String>;
    pub fn save(&self, settings: &AtlasSettings) -> Result<(), String>;
}
```

Use a same-directory temporary file, flush/sync, atomic rename, an `fs2` lock, and Unix mode `0o600` with a private parent directory. Validate before publication. Keep test-only publisher injection hidden as in `ConversationStore`.

- [ ] **Step 4: Run focused and root regression tests**

Run: `cargo test --test settings_behavior`

Expected: all settings tests pass.

Run: `cargo test --all-targets`

Expected: all root tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/settings.rs src/lib.rs tests/settings_behavior.rs
git commit -m "feat: add private Atlas settings store"
```

### Task 2: Effective Settings and Shared Runtime Construction

**Files:**
- Modify: `src/settings.rs`
- Modify: `src/models/deepseek.rs`
- Modify: `src/models/mod.rs`
- Modify: `src/app.rs`
- Modify: `src/main.rs`
- Modify: `tests/settings_behavior.rs`
- Modify: `tests/app_runtime_behavior.rs`
- Modify: `tests/cli_binary.rs`

**Interfaces:**
- Consumes: `AtlasSettings` and `SettingsStore` from Task 1.
- Produces: `EffectiveSettings { api_key: Option<String>, base_url: String, model: String, history_limit: usize, default_workspace: Option<PathBuf>, environment_overrides: BTreeSet<SettingField> }`.
- Produces: `AtlasSettings::resolve_with(...)` for deterministic tests and `resolve_from_env()` for production.
- Produces: `build_runtime_with_settings(workspace_root, agent_file, &EffectiveSettings)`; preserves `build_runtime(workspace_root, agent_file)` as a compatibility wrapper.

- [ ] **Step 1: Write failing precedence and runtime tests**

Add tests proving:

- each supported environment variable overrides only its saved field;
- invalid `ATLAS_HISTORY_MESSAGES` still returns the existing positive-integer error;
- a saved API key builds a runtime when the environment key is absent;
- environment values are not written back to `settings.json`;
- CLI asks use `current_dir()` as the tool workspace and settings history limit;
- missing agent-file errors still win over provider construction errors.

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --test settings_behavior --test app_runtime_behavior --test cli_binary`

Expected: failures show saved settings are not consumed and the new runtime API is absent.

- [ ] **Step 3: Implement effective resolution and runtime injection**

Implement `AtlasSettings::resolve_with(&self, env: impl Fn(&str) -> Option<OsString>) -> Result<EffectiveSettings, String>` with `SettingField::{ApiKey, BaseUrl, Model, HistoryLimit}` override markers. Move environment precedence out of `DeepSeekConfig::from_env`. Construct `DeepSeekConfig` from `EffectiveSettings`, using the loaded agent model only when no saved/environment model override exists. Make `main.rs` load dotenv, settings, and effective values once per ask.

Do not expose the API key through `Debug` output. Keep management-only CLI commands credential-independent.

- [ ] **Step 4: Run focused and full root verification**

Run: `cargo test --test settings_behavior --test app_runtime_behavior --test cli_binary`

Expected: all focused tests pass.

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: exit 0 with no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/settings.rs src/models/deepseek.rs src/models/mod.rs src/app.rs src/main.rs tests/settings_behavior.rs tests/app_runtime_behavior.rs tests/cli_binary.rs
git commit -m "feat: load runtime configuration from settings"
```

### Task 3: Conversation Workspace Persistence

**Files:**
- Modify: `src/conversations/types.rs`
- Modify: `src/conversations/store.rs`
- Modify: `src/conversations/service.rs`
- Modify: `tests/conversation_store_behavior.rs`
- Modify: `tests/conversation_service_behavior.rs`

**Interfaces:**
- Produces: `ConversationMetadata.workspace_path: Option<PathBuf>` serialized as `workspacePath` and defaulting to `None`.
- Produces: `ConversationStore::create_turn_with_workspace(user, assistant, workspace: Option<&Path>)` and compatibility `create_turn(...)` wrapper.
- Produces: `ConversationStore::assign_workspace(id, workspace) -> Result<ConversationMetadata, String>`.
- Produces: fallible-sink service variants accepting `new_workspace: Option<&Path>` so first-turn persistence is atomic with conversation creation.

- [ ] **Step 1: Write failing workspace persistence tests**

Add tests proving:

- a version-one index without `workspacePath` loads as `None`;
- creation stores a canonical absolute directory path;
- assigning a workspace preserves title, title-attempt state, timestamps except `updated_at`, and all history;
- missing files, regular files, and non-UTF-8 Unix paths are rejected;
- a workspace removed after selection fails validation without conversation mutation;
- a first model or sink failure saves neither conversation nor workspace metadata;
- CLI-compatible `create_turn` still creates unassigned metadata.

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --test conversation_store_behavior --test conversation_service_behavior`

Expected: compilation fails on missing `workspace_path` and workspace APIs.

- [ ] **Step 3: Implement canonical validation and metadata updates**

Add a shared `validate_workspace(path: &Path) -> Result<PathBuf, String>` that requires an existing directory, canonicalizes it, and rejects paths that cannot be represented losslessly for the desktop DTO contract.

Extend store creation and metadata updates under the existing index lock. Revalidate immediately before first-turn publication. Preserve old method signatures as wrappers so unrelated callers remain source-compatible.

- [ ] **Step 4: Run conversation and root suites**

Run: `cargo test --test conversation_store_behavior --test conversation_service_behavior`

Expected: all focused tests pass.

Run: `cargo test --all-targets`

Expected: all root tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/conversations/types.rs src/conversations/store.rs src/conversations/service.rs tests/conversation_store_behavior.rs tests/conversation_service_behavior.rs
git commit -m "feat: persist conversation workspaces"
```

### Task 4: Desktop Settings and Workspace Bridge

**Files:**
- Modify: `desktop/src-tauri/Cargo.toml`
- Modify: `desktop/src-tauri/Cargo.lock`
- Modify: `desktop/src-tauri/capabilities/default.json`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/package.json`
- Modify: `desktop/package-lock.json`

**Interfaces:**
- Consumes: `SettingsStore`, `EffectiveSettings`, workspace-aware conversation APIs, and `build_runtime_with_settings`.
- Produces commands: `load_settings`, `save_settings`, `validate_workspace`, and `assign_conversation_workspace`.
- Produces DTOs: `WorkspaceDto`, `SettingsViewDto`, `SaveSettingsInput`, and workspace-aware `ConversationMetadataDto`.
- Changes `ask` to accept `workspace_path: Option<String>` only for an unsaved first turn; saved IDs resolve workspace from metadata.

- [ ] **Step 1: Write failing Rust bridge tests**

Add tests proving:

- settings load returns `apiKeySource` as `missing`, `settings`, or `environment`, never the key;
- `apiKey: null` preserves the saved key and cannot copy an environment key into the file;
- `clearApiKey: true` removes only the saved key;
- invalid settings do not replace the prior file;
- workspace validation returns canonical `path` and final-component `name`;
- assignment returns workspace-aware metadata;
- saved conversation asks ignore a conflicting frontend workspace;
- deleted saved/default workspaces fail before fake-provider invocation and history mutation.

- [ ] **Step 2: Run desktop tests and verify RED**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml`

Expected: compilation fails because bridge DTOs and commands are absent.

- [ ] **Step 3: Implement the bridge and native dialog capability**

Use these DTO fields:

```text
WorkspaceDto { path, name }
SettingsViewDto { apiKeySource, baseUrl, model, historyLimit, defaultWorkspace, environmentOverrides }
SaveSettingsInput { apiKey: Option<String>, clearApiKey, baseUrl, model, historyLimit, defaultWorkspacePath }
```

Make `DesktopState` own `ConversationStore`, `SettingsStore`, agent path, and runtime-construction inputs. Load effective settings inside each blocking ask so a successful Settings save affects the next request without restarting.

Add Tauri dialog plugin v2, initialize it in `lib.rs`, install `@tauri-apps/plugin-dialog`, and grant only `dialog:allow-open`. Folder cancellation remains `None`, not an error.

- [ ] **Step 4: Verify bridge, permissions, and host build**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml`

Expected: all desktop Rust tests pass.

Run: `cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`

Expected: exit 0.

Run: `npm --prefix desktop run tauri -- build --debug --no-bundle`

Expected: the Tauri host builds with the dialog plugin and narrow permission.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock desktop/src-tauri/capabilities/default.json desktop/src-tauri/src/lib.rs desktop/src-tauri/src/commands.rs desktop/package.json desktop/package-lock.json
git commit -m "feat: bridge workspace and settings commands"
```

### Task 5: Workspace and Settings Frontend State

**Files:**
- Modify: `desktop/src/types.ts`
- Modify: `desktop/src/atlas-api.ts`
- Modify: `desktop/src/atlas-api.test.ts`
- Modify: `desktop/src/chat-state.ts`
- Modify: `desktop/src/chat-state.test.ts`

**Interfaces:**
- Consumes: Task 4 camelCase DTOs and native folder dialog.
- Produces: `Workspace`, `SettingsView`, `SettingsDraft`, and `screen: "chat" | "settings"` in `ChatState`.
- Produces API methods: `loadSettings`, `saveSettings`, `chooseWorkspace`, `validateWorkspace`, and `assignConversationWorkspace`.
- Produces store methods: `openSettings`, `cancelSettings`, `saveSettings`, `chooseWorkspaceForDraft`, and `changeActiveWorkspace`.

- [ ] **Step 1: Write failing adapter and state tests**

Cover:

- exact command names/camelCase payloads and dialog cancellation;
- startup loading conversations/settings independently;
- new chat adopts a valid default or requests a folder without creating persistence;
- cancelled folder selection keeps an unassigned draft;
- send is blocked without a workspace;
- first ask sends the draft workspace and adopts persisted metadata;
- opening a saved conversation restores its workspace;
- legacy conversations remain readable and require assignment before send;
- workspace groups sort by newest member while conversations remain newest-first;
- saved assignment commits only after backend success;
- Settings cancel discards changes; failure preserves draft; success adopts returned masked view;
- blank API input submits `apiKey: null`, not an empty replacement.
- Settings navigation and mutations are disabled while an ask is streaming.

- [ ] **Step 2: Run frontend tests and verify RED**

Run: `npm --prefix desktop test`

Expected: type/test failures show missing workspace and settings contracts.

- [ ] **Step 3: Implement adapter and race-safe state transitions**

Keep all Tauri/plugin calls in `atlas-api.ts`. Treat folder-picker cancellation as a successful `null`. Extend existing selection, ask, and mutation tokens so late settings or workspace results cannot overwrite a newer screen, draft, or active conversation.

- [ ] **Step 4: Run frontend suite and type build**

Run: `npm --prefix desktop test`

Expected: all TypeScript tests pass.

Run: `npm --prefix desktop run build`

Expected: TypeScript and Vite exit 0.

- [ ] **Step 5: Commit**

```bash
git add desktop/src/types.ts desktop/src/atlas-api.ts desktop/src/atlas-api.test.ts desktop/src/chat-state.ts desktop/src/chat-state.test.ts
git commit -m "feat: add workspace-aware desktop state"
```

### Task 6: Approved UI, Atlas Compass, and Vietnamese IME

**Files:**
- Create: `desktop/src/settings-view.ts`
- Create: `desktop/src/settings-view.test.ts`
- Create: `desktop/public/atlas-compass.svg`
- Modify: `desktop/src/chat-view.ts`
- Modify: `desktop/src/chat-view.test.ts`
- Modify: `desktop/src/styles.css`
- Regenerate: `desktop/src-tauri/icons/*`

**Interfaces:**
- Consumes: `ChatState` and store methods from Task 5.
- Produces: approved workspace card, grouped sidebar, folder-first empty state, workspace-aware header/composer, Settings panel, and Atlas Compass branding.
- Produces: composition deferral internal to `mountChat`; no new backend interface.

- [ ] **Step 1: Write failing DOM tests**

Add tests proving:

- workspace groups, active path, Settings button, and folder-first action render accessibly;
- folder picker cancellation and selection preserve composer focus;
- Settings password field is blank/masked, environment source is explained, and Save/Cancel keyboard behavior works;
- hostile workspace names/paths render as literal text;
- mobile sidebar exposes workspace and Settings controls;
- `compositionstart` followed by a pending list/settings resolution keeps the same textarea node and Vietnamese value;
- composing Enter, including key code `229`, never sends;
- `compositionend` stores the final value, applies only the newest queued state, and restores the caret.

- [ ] **Step 2: Run DOM tests and verify RED**

Run: `npm --prefix desktop test -- --run src/chat-view.test.ts src/settings-view.test.ts`

Expected: tests fail because the approved panels and composition deferral are absent.

- [ ] **Step 3: Implement the UI and icon**

Keep external values in `textContent`/form values, never `innerHTML`. Render Settings through `settings-view.ts` while the sidebar remains mounted. During active composition, retain the current textarea, store only the newest pending state, and render once after `compositionend`.

Create the simple vector Atlas Compass source and run:

```bash
npm --prefix desktop run tauri -- icon public/atlas-compass.svg
```

Use the SVG in the sidebar and generated platform assets for the desktop bundle.

- [ ] **Step 4: Run frontend and native verification**

Run: `npm --prefix desktop test`

Expected: all frontend tests pass.

Run: `npm --prefix desktop run build`

Expected: production frontend builds.

Run: `npm --prefix desktop run tauri -- build --debug --no-bundle`

Expected: native host builds with regenerated icons.

- [ ] **Step 5: Commit**

```bash
git add desktop/src/settings-view.ts desktop/src/settings-view.test.ts desktop/public/atlas-compass.svg desktop/src/chat-view.ts desktop/src/chat-view.test.ts desktop/src/styles.css desktop/src-tauri/icons
git commit -m "feat: build workspace settings interface"
```

### Task 7: Documentation and Cross-Layer Regression

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Test: `desktop/src-tauri/src/commands.rs`
- Modify only implementation files required by final regression findings.

**Interfaces:**
- Consumes: all completed settings, conversation, bridge, state, and UI interfaces.
- Produces: documented, packaged, clean workspace/settings vertical slice.

- [ ] **Step 1: Add end-to-end regression coverage for the shipped flow**

Use a temporary settings directory, temporary workspace, fake provider, and real stores to prove:

1. Settings save makes the next desktop ask provider-configured without restart.
2. A successful first ask persists the selected canonical workspace.
3. Reopening and continuing uses that workspace for tool definitions.
4. Removing the folder prevents provider invocation and leaves history unchanged.
5. The API key never appears in conversation files, metadata DTO JSON, or user-facing bridge errors.

- [ ] **Step 2: Update user and agent documentation**

Document Settings location, plaintext/private API-key caveat, environment precedence, folder-first flow, legacy reassignment, native dialog prerequisite, and all desktop verification commands. Update the repository structure in `AGENTS.md`.

- [ ] **Step 3: Run the complete verification matrix**

Run each command separately:

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

Expected: every command exits 0 with no warnings or failures.

- [ ] **Step 4: Exercise the native development window**

Verify folder selection/cancellation, new chat, grouped conversation selection, workspace reassignment, Settings save/cancel/error states, Vietnamese composition, narrow sidebar, and keyboard focus. Do not send a real model request; use the fake/local test path or stop before submission.

- [ ] **Step 5: Inspect scope and commit**

Run: `git status --short` and `git diff --check`.

Expected: only planned documentation or final fixes remain and no settings/API-key file is tracked.

```bash
git add README.md AGENTS.md desktop/src-tauri/src/commands.rs
git commit -m "docs: complete workspace settings setup"
```
