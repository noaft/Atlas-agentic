# Tauri Chat UI Design

## Purpose

Atlas needs a focused desktop interface for its existing coding-agent and local-conversation behavior. The first desktop milestone is intentionally limited to chat and conversation management. It does not introduce agent routing, research, news, scheduling, workspace selection, or a general desktop settings system.

Success means a user can launch a Tauri desktop window, start an unsaved chat draft, stream an Atlas response, and manage saved conversations without changing the existing CLI behavior.

## Product Rules

- The desktop app uses Tauri v2 with a vanilla TypeScript, HTML, and CSS frontend.
- The left sidebar lists saved conversations in most-recently-updated order.
- The main panel shows the selected conversation and a message composer.
- `New chat` creates only an in-memory UI draft. It does not create an ID, index entry, or history file.
- Sending the first message uses the existing conversation service with no conversation ID.
- The first successful response creates the conversation. The returned metadata supplies its ID and generated title, and the UI then adds it to the sidebar.
- If the first request fails, the draft remains unsaved and the typed message remains available for retry.
- Existing conversations can be opened, continued, renamed, and deleted.
- The frontend never reads or writes Atlas conversation files directly.
- The CLI remains supported and keeps its current command behavior.

## Scope

### Included

- A cross-platform Tauri desktop shell.
- A responsive two-panel chat layout based on the approved mockup.
- Streaming final-assistant text.
- Conversation list, load, rename, and delete actions.
- Empty, loading, streaming, and error states.
- A thin Tauri command bridge to the existing Rust library.
- Developer documentation and build/test commands.

### Excluded

- Research, news, scheduled tasks, notifications, and background agents.
- Agent routing or an agent picker.
- Workspace selection or per-conversation workspace persistence.
- API-key or model settings screens.
- File trees, diffs, terminal panels, and other IDE features.
- Markdown rendering and syntax highlighting. Messages are rendered as safe text with whitespace preserved for this milestone.
- Installer publishing, signing, and automatic updates.

The desktop app retains the current runtime configuration and working-directory behavior. A later coding-workspace design can replace that behavior without changing the conversation UI contract.

## Repository Structure

The desktop app is isolated from the existing Rust package:

```text
desktop/
├── index.html
├── package.json
├── tsconfig.json
├── vite.config.ts
├── src/
│   ├── main.ts
│   ├── styles.css
│   ├── atlas-api.ts
│   ├── chat-state.ts
│   └── types.ts
└── src-tauri/
    ├── Cargo.toml
    ├── build.rs
    ├── tauri.conf.json
    ├── capabilities/
    │   └── default.json
    └── src/
        ├── main.rs
        ├── lib.rs
        └── commands.rs
```

`desktop/src-tauri` is a small Rust package with a path dependency on the root `atlas-agentic` library. It owns desktop-only Tauri annotations and serialization adapters. Core conversation and model behavior remains in the root library.

Reusable runtime construction currently in `src/main.rs` moves behind a root-library function. Both the CLI and desktop bridge call that function, preventing the two entrypoints from drifting.

## Rust/Desktop Boundary

The Tauri bridge exposes these commands:

```text
list_conversations() -> ConversationMetadata[]
load_conversation(id) -> Conversation
ask(prompt, conversation_id?, channel) -> ConversationAskResult
rename_conversation(id, title) -> ConversationMetadata
delete_conversation(id) -> DeleteResult
```

There is deliberately no `create_conversation` command. Creation is an outcome of the first successful `ask`, preserving the existing storage invariant.

The `ask` command accepts a Tauri IPC channel. It sends ordered delta events while `ConversationService::ask` runs, then resolves with the completed answer and conversation metadata. Tauri recommends channels for streamed responses because they preserve ordering and suit high-volume streaming data.

The current provider is blocking, so the bridge runs an ask away from the window thread. The UI permits one active ask at a time. While an ask is running, send and conversation mutation controls are disabled to avoid overlapping changes and ambiguous stream ownership.

Bridge errors are returned as stable displayable strings. The bridge does not expose filesystem paths, provider response types, or internal tool protocol messages to TypeScript.

## Frontend State

The frontend has one explicit state model:

```text
conversations: sidebar metadata
activeConversationId: saved ID or null
messages: active saved messages plus pending UI messages
draftText: composer contents
status: idle | loading | streaming | error
errorMessage: optional display text
```

`activeConversationId = null` represents the unsaved new-chat draft. This is a UI state, not a stored Atlas conversation.

The frontend accesses Tauri only through `atlas-api.ts`. UI state and rendering depend on that adapter rather than importing Tauri APIs throughout the application. Tests can replace the adapter with a fake.

## User Flows

### Startup

1. Show the desktop shell immediately with a loading sidebar.
2. Request the conversation list.
3. Show a new-chat draft in the main panel.
4. Display stored conversations in the sidebar when loading completes.
5. If loading fails, keep the draft usable and show a retryable sidebar error.

Atlas does not automatically open or continue the newest conversation.

### New Chat and First Message

1. `New chat` clears the active ID and messages locally.
2. The user enters a non-empty message and sends it.
3. The UI displays the user message immediately, clears the composer, and starts an empty assistant message.
4. Stream deltas append to that assistant message in order.
5. On success, the result's ID becomes active and its metadata is inserted into the sidebar.
6. On failure, the partial assistant output is marked failed and is not treated as saved. The user message is restored to the composer for retry, and the active ID remains null.

### Continue a Conversation

1. Selecting a sidebar item loads its complete saved messages.
2. Sending supplies that conversation ID.
3. Streamed content appears immediately.
4. On success, returned metadata updates the title, timestamp, and message count and moves the item to the top.
5. On failure, saved history remains unchanged and the input is restored for retry.

### Rename

1. The conversation menu opens a small rename dialog prefilled with the current title.
2. A trimmed, non-empty title is submitted through the bridge.
3. Returned metadata updates both the header and sidebar.
4. Errors leave the old title in place and keep the dialog available.

### Delete

1. The conversation menu asks for confirmation.
2. Confirmation calls delete through the bridge.
3. The item is removed from the sidebar.
4. If it was active, the main panel becomes a new unsaved draft.
5. An error leaves the conversation visible and shows a non-destructive error message.

## Interface Design

The desktop window uses the approved two-panel layout:

- A compact left sidebar contains Atlas branding, `New chat`, recent conversation titles, and a local-storage note.
- The chat header shows the active title and a menu for rename/delete actions.
- The scrollable message area keeps a readable maximum text width.
- User messages use compact right-aligned bubbles. Assistant messages use an open document-like layout suitable for longer coding explanations.
- The composer stays at the bottom and supports Enter to send and Shift+Enter for a newline.
- During streaming, a stop action is not shown because the current Rust provider path has no cancellation contract.
- Narrow windows collapse the sidebar behind a menu while preserving the same state and actions.

All conversation and message text is inserted with safe DOM text APIs, never raw HTML. Keyboard focus returns to the composer after new-chat, successful send, and conversation selection. Buttons and dialogs have accessible labels and keyboard behavior.

## Configuration and Resources

The initial desktop build uses the same environment variables and `.env` behavior as the CLI, including `DEEPSEEK_API_KEY`, `DEEPSEEK_BASE_URL`, `DEEPSEEK_MODEL`, `ATLAS_DATA_DIR`, and `ATLAS_HISTORY_MESSAGES`.

The Atlas agent TOML remains configuration, not a prompt embedded in Rust or TypeScript. Development resolves the repository agent file. The Tauri bundle includes the same TOML as a resource for packaged builds, and the desktop bootstrap resolves that resource through Tauri's application resource path.

Missing model configuration does not prevent the window from opening or local conversation management from working. It fails only when the user sends a message and is reported in the chat error state.

## Error Handling

- Empty or whitespace-only messages are not sent.
- A startup list failure is retryable and does not disable new-chat composition.
- A load failure keeps the previously active view rather than replacing it with partial data.
- A failed ask never creates a first conversation or appends a turn to an existing one, matching `ConversationService` behavior.
- A failed rename or delete leaves current UI state unchanged.
- Partial streamed output from a failed request is visibly marked as failed and disappears when that input is retried.
- Unexpected bridge failures are shown in the UI and logged to the development console without exposing secrets.

## Testing

Implementation follows test-first development.

Rust tests cover:

- Tauri command adapters mapping list/load/rename/delete results correctly.
- `ask` forwarding ordered deltas and returning final metadata.
- First-message failure producing no stored conversation.
- Desktop runtime configuration failures not blocking CRUD commands.

TypeScript tests use a fake `atlas-api` adapter and cover:

- `New chat` performing no create call.
- A first successful send adopting the returned conversation ID.
- A failed first send remaining unsaved and restoring the draft.
- Loading and continuing a saved conversation.
- Rename and delete success/failure state transitions.
- Streaming deltas appearing in order.
- safe text rendering for message content.

Completion checks include:

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
npm --prefix desktop run tauri -- build --debug --no-bundle
npm --prefix desktop test
npm --prefix desktop run build
```

The Tauri build command compiles the nested desktop Rust package as well as the frontend. It is checked on the development host when its platform prerequisites are installed. Cross-platform installer builds and signing remain separate release work.

## Documentation

`README.md` will document desktop prerequisites, development commands, environment configuration, and the difference between an unsaved draft and a saved conversation. `AGENTS.md` will describe the new desktop directories and required verification commands. The existing CLI documentation remains intact.
