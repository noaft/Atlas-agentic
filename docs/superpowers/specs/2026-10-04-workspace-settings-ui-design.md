# Workspace-Aware Desktop and Settings Design

## Purpose

Atlas is a coding-focused desktop agent. Its desktop UI must let a user choose a code folder, keep each conversation attached to that folder, and configure the model without editing environment files by hand.

This phase expands the existing chat-only Tauri application. It also fixes Vietnamese and other IME input being interrupted by asynchronous UI renders.

Success means a user can launch Atlas, choose a folder, start a chat, reopen that conversation with the same workspace, and manage DeepSeek configuration through a local Settings screen. The CLI and desktop use the same saved settings.

## Product Rules

- A new desktop chat requires a workspace folder before the first message can be sent.
- `New chat` remains an in-memory draft and does not create conversation files.
- The selected workspace is persisted only with the first successful turn.
- Every saved conversation remembers one workspace path.
- Reopening a conversation restores its workspace automatically.
- A user can assign or change a conversation workspace without changing its message history.
- Existing conversations without a workspace remain readable but require a folder before another message can be sent.
- The model runtime and coding tools use the conversation workspace, never an incidental desktop process directory.
- Settings are stored locally in a private Atlas configuration file.
- Environment variables override saved settings so scripted and development workflows remain supported.
- Vietnamese and other composition-based input must survive asynchronous list, status, and stream updates.

## Scope

### Included

- Atlas Compass application/logo icon.
- Workspace-first new-chat flow using a native folder picker.
- Active workspace display and conversation grouping by workspace.
- Per-conversation workspace persistence and reassignment.
- Desktop Settings screen for:
  - DeepSeek API key
  - DeepSeek base URL
  - model name
  - conversation history limit
  - default workspace
- Shared file-backed configuration for the CLI and desktop.
- Atomic settings writes and private local permissions.
- Backward-compatible conversation migration.
- Vietnamese/IME composition preservation.
- Rust, TypeScript, integration, and packaging tests.

### Excluded

- Multiple workspaces attached to one conversation.
- Repository discovery, Git account connection, or cloning.
- File tree, terminal, diff, or editor panels.
- Per-agent model settings.
- Cloud synchronization of settings or conversations.
- OS keychain storage.
- Multiple visual themes. The approved pastel-light appearance remains the single theme.
- Opening a project in an external editor.

## Visual Direction

The approved UI uses the existing pastel-light visual language:

- lavender, sky-blue, and mint accents;
- warm off-white content surfaces;
- dark, accessible body text;
- soft borders and shadows;
- rounded controls without excessive ornament;
- the Atlas Compass logo: a lavender-to-sky rounded square with a white compass needle forming an abstract `A` and a small mint accent.

The icon is used for desktop application assets, window branding, and the sidebar mark. It must remain recognizable at small system-icon sizes and contain no text.

## Desktop Information Architecture

### Sidebar

The sidebar contains:

1. Atlas Compass branding.
2. An active-workspace card showing folder name and abbreviated path.
3. A workspace/folder action.
4. `New chat`.
5. Saved conversations grouped by workspace, with unassigned legacy conversations in a separate group.
6. A Settings action anchored at the bottom.

Conversation ordering remains most-recently-updated within each workspace group. Workspace groups are ordered by the newest conversation they contain.

### Chat Panel

The chat header shows:

- conversation title;
- workspace name and abbreviated path;
- rename, change-folder, and delete actions.

The composer placeholder names the active workspace. When no workspace is assigned, the composer is disabled and a `Choose folder to continue` action is shown.

### Folder-First Empty State

Starting `New chat` opens an unsaved draft, then requests a folder if no default workspace is configured. The folder picker is native to the operating system. Cancelling it leaves the user in an unsaved, unassigned draft with a visible `Choose folder` action.

If a valid default workspace exists, a new draft starts with it immediately. The user can change the folder before sending.

### Settings Screen

Settings replace the main content panel while preserving the sidebar. The screen contains password-masked API key input, base URL, model, positive history limit, and default workspace picker.

`Cancel` discards unsaved form changes. `Save settings` validates the complete form, writes it atomically, and only then updates the active application state. A failed save leaves the form open with non-secret error text.

## Configuration Storage

### Location

The shared configuration file is named `settings.json` under the platform-specific Atlas configuration directory supplied by the existing directories abstraction.

The settings path can be overridden in tests through a dedicated Atlas test/config environment variable. Production code must not depend on the current working directory for configuration.

### Schema

```json
{
  "version": 1,
  "deepseek": {
    "apiKey": "...",
    "baseUrl": "https://api.deepseek.com",
    "model": "deepseek-flash"
  },
  "conversation": {
    "historyLimit": 40
  },
  "workspace": {
    "defaultPath": "/absolute/path/or/null"
  }
}
```

The Rust representation is typed and versioned. Unknown fields are tolerated for forward compatibility. Unsupported schema versions return an actionable error rather than silently resetting configuration.

### Precedence

Effective runtime configuration is resolved in this order:

1. Process environment variables.
2. Values from `settings.json`.
3. Existing built-in or agent-definition defaults.

Supported environment variables retain their current names: `DEEPSEEK_API_KEY`, `DEEPSEEK_BASE_URL`, `DEEPSEEK_MODEL`, and `ATLAS_HISTORY_MESSAGES`. An environment override affects runtime behavior but does not overwrite the saved Settings form value.

### Writes and Permissions

Settings are written to a temporary file in the same directory, flushed, and atomically renamed over the destination. On Unix, the directory and file use private user-only permissions. Other platforms use the most restrictive permissions available through their standard user configuration directory and filesystem APIs.

The API key is intentionally stored as plain text in this private file, following the approved product decision. It is never returned in general error messages, logged, included in conversation files, or exposed through a read command that is not specifically the Settings bridge.

## Conversation Workspace Data

Conversation metadata gains an optional workspace path:

```text
workspace_path: Option<PathBuf>
```

The persisted JSON field is `workspacePath`. Missing fields deserialize as `None`, allowing existing indexes and histories to load unchanged.

New-conversation creation accepts a required workspace path from the desktop conversation service. CLI-created conversations continue to use the current working directory for backward compatibility in this phase.

Continuations derive their runtime workspace from stored metadata. The UI does not send an arbitrary workspace on every continuation. This prevents a stale frontend selection from silently running tools in the wrong folder.

Changing a workspace validates that the target is an existing directory, updates metadata atomically, and does not modify message history, title, creation time, or title-attempt state. It updates the conversation modification time so grouping and ordering reflect the change.

## Runtime Construction

Runtime construction is split into two inputs:

- effective model configuration;
- validated workspace path.

The CLI and desktop both use the same settings loader and effective-configuration resolver. Desktop asks continue to run on the blocking worker boundary introduced previously.

Each desktop ask resolves the saved conversation workspace before building `PwdTool`, `ListDirTool`, `ReadFileTool`, `CreateFileTool`, and `ExecCommandTool`. A missing or inaccessible folder produces a stable error and does not call the model or mutate conversation history.

## Tauri Bridge

The desktop bridge adds commands equivalent to:

```text
load_settings() -> SettingsView
save_settings(input) -> SettingsView
choose_workspace() -> Option<WorkspaceView>
assign_conversation_workspace(id, path) -> ConversationMetadata
```

The native folder picker uses the maintained Tauri dialog integration with only the required capability permission. No shell-open, unrestricted filesystem, or process plugin permission is added.

The existing `ask` command accepts a workspace path only for an unsaved first turn. For a saved conversation it ignores frontend workspace authority and uses stored metadata. A successful first ask persists the validated workspace with the new conversation.

Settings DTOs never return the API key after initial loading as ordinary readable text. The Settings bridge returns a masked/configured representation and accepts an unchanged-key sentinel or a replacement value on save. This prevents routine frontend state dumps from containing the stored secret longer than necessary.

## Frontend State and Flows

The frontend state adds:

```text
screen: chat | settings
activeWorkspace: workspace or null
settingsStatus: idle | loading | saving | error
settingsDraft: editable settings fields
```

Workspace information is included in conversation metadata DTOs. Grouping is derived from the metadata list rather than stored separately.

### New Chat

1. Clear the active conversation locally.
2. Use the valid default workspace when available.
3. Otherwise open the native folder picker.
4. If cancelled, keep an unassigned draft.
5. Do not write conversation or workspace metadata until the first successful ask.

### Open Conversation

1. Load messages and metadata.
2. Restore its saved workspace.
3. If unassigned, show history read-only with `Choose folder to continue`.
4. If the folder no longer exists, show a non-destructive reassignment prompt.

### Change Workspace

For an unsaved draft, changing folders is local UI state. For a saved conversation, the selection is persisted through the assignment command before the UI adopts it.

### Settings

Opening Settings loads current saved values and effective override indicators. Saving validates fields, persists through Rust, and updates subsequent runtime construction. Existing active streams cannot be reconfigured mid-request; Settings mutation controls are disabled while an ask is running.

## IME Composition Fix

The chat renderer currently replaces the composer DOM node when unrelated state changes arrive. Destroying that node during composition causes unfinished Vietnamese text to disappear.

The view tracks `compositionstart` and `compositionend`. While composition is active, unrelated full renders are deferred and only the newest state snapshot is retained. At `compositionend`, the final textarea value is stored, the queued state is rendered once, and the caret is restored. Enter-to-send continues to ignore composing events and IME key code `229`.

Automated DOM coverage reproduces a pending conversation-list update resolving during composition and verifies that:

- the textarea node remains attached during composition;
- the composed Vietnamese text remains present;
- no ask is submitted by the composition confirmation key;
- queued sidebar state appears after composition ends.

## Error Handling

- Cancelling a folder picker is not an error.
- Invalid or inaccessible workspaces do not call the model.
- Failed workspace assignment preserves the previous workspace.
- Failed settings validation or persistence preserves the prior saved settings.
- Missing model configuration does not block conversation browsing, folder assignment, or Settings access.
- API keys are redacted from bridge, filesystem, and provider errors shown to users.
- Existing conversations with absent workspace metadata remain loadable.
- A disappeared saved folder prompts reassignment without deleting history.

## Testing

Rust tests cover:

- settings defaults, parsing, validation, unsupported versions, and environment precedence;
- atomic settings replacement and private Unix permissions;
- secret redaction from errors and display DTOs;
- old conversation metadata loading without `workspacePath`;
- new conversation workspace persistence;
- workspace reassignment preserving history;
- runtime tools receiving the saved workspace;
- missing workspace failure before provider invocation;
- bridge settings and workspace DTO mapping;
- configured desktop asks remaining on the blocking worker.

TypeScript DOM/state tests cover:

- new-chat folder selection and cancellation;
- default workspace adoption without premature persistence;
- folder-grouped conversation ordering;
- legacy conversation read-only/reassignment flow;
- changing unsaved and saved workspaces;
- Settings load, validation, cancel, successful save, and failed save;
- secret masking and unchanged-key submission;
- responsive workspace and Settings controls;
- Vietnamese composition surviving asynchronous renders.

Completion verification includes:

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

## Documentation and Migration

`README.md` documents the folder-first desktop flow, settings location, credential caveat, precedence rules, and development commands. `AGENTS.md` documents the new settings/workspace modules and verification commands.

No eager migration rewrites existing conversations. Their absent `workspacePath` is interpreted as unassigned until the user selects a folder. Settings are created only after the first successful Settings save, preserving current environment-only setups.
