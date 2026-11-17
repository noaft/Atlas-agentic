# Desktop session and UI contracts

Implemented contracts for chat state, native ownership, task dispatch, and IME.
Read [Live chat input](live-chat-input.md) for queuing, steering, and Stop;
read [Delegated agents](delegated-agents.md) for `/name task` dispatch.

## Inline tool activity

During desktop requests, show one current tool row below the pending assistant
message, using the newest requested tool rather than the most recently received
event. Older call results must not replace it. Keep status and expandable details
on that row, preserve its expansion across updates to the same session/call, and
collapse details when the call changes. Hide the live terminal immediately when
the newest tool succeeds, fails or is denied, including while the answer is still
streaming; never fall back to an older call. Keep the records for saved Activity.
Runtime statuses do not create tool rows.
Do not duplicate live activity in notices or render a growing live timeline. Keep
all session activity and saved records intact; the final answer replaces the live
row and retains the complete collapsed Activity disclosure.

## Built-in tasks

`src/tasks/mod.rs` is the authoritative built-in task registry, initially
`conversation.compact` (`/compact`, empty-object arguments). Keep descriptors,
argument validation and typed handlers together; reject duplicate IDs/commands.
Tauri `list_tasks` needs no model configuration, and `run_task` validates requests
before provider setup, obtains native ownership once, derives the saved workspace,
and delegates to the service's existing execution lease. CLI compact shares the
registry; automatic compaction retains its direct runtime path without nested leases.
All frontend task access stays in `atlas-api.ts`. The composer discovers tasks
from the backend catalog and filters command/title/description. Clicking a task
or pressing Enter on the highlighted row runs the selected descriptor directly;
never infer task execution or arguments from the text submitted through Send.
Sending literal `/compact`, bare `/`, unknown slash text, paths, or prose uses
ordinary chat; Send must not infer a built-in task from that text. Saved agent
`/name task` commands use the separate native dispatch described in
[Delegated agents](delegated-agents.md#direct-desktop-commands). Escape/outside click/Tab close the menu and preserve
input; Tab does not run a task. Explain each task and show its progress/result in
the owning chat. Keep unavailable tasks visible with reasons. Catalog
failure/retry cannot block ordinary messages or overwrite drafts. Keep task/menu
ownership per session. Do not add frontend fallback task names or automatic model
routing in this release. Future handlers need registry validation/dispatch tests,
argument support and typed outcome adapters as needed.

## Workspace and session ownership

The desktop UI calls Rust only through `desktop/src/atlas-api.ts`. Keep **New
chat** as an unsaved frontend draft; only a successful or explicitly stopped first `ask` may create
local conversation data or persist its validated starting folder. New chats use
an explicitly chosen draft folder, then a custom Settings default, then the
automatic `Atlas` folder in the user's home directory. Saved continuations derive
workspace authority from conversation metadata and must ignore a conflicting
frontend path. Legacy unassigned conversations use the current default and gain
that assignment only with their next successful saved turn. The CLI continues to
use its process's current directory.

Desktop chat sessions have stable frontend identities separate from saved
conversation IDs. Keep request callbacks, live messages, drafts, tool activity,
errors, and approvals owned by their session, even when another chat or Settings
is visible. Independent chats may run concurrently, but only one request may
run per saved conversation in the desktop backend. Navigation must not cancel
all pending approvals. First requests remain in-memory sidebar sessions until
successful or explicitly stopped persistence supplies an ID; other failures stay retryable without writing
conversation data. Session state and unfinished requests do not survive an app
reload or restart. Running requests retain their starting settings and folder.
Keep Settings inputs and user scroll position stable during background updates,
and coalesce visible streaming renders to animation frames. Blocking native
storage/settings operations run on background workers rather than the Tauri
event thread. Do not broaden frontend permissions to achieve concurrency.

## Native text input

Keep the chat textarea stationary and browser-owned for native Vietnamese/IME
input. Never collapse its live height to zero for measurement or rewrite an
unchanged draft. Measure auto-height in a hidden, inert mirror and defer live
layout changes while composition is active. Composition lifecycle, per-event
isComposing and key code 229 must all guard Send and slash tasks. Native input,
backspace, replacement and paste must remain unmodified; do not implement Telex
conversion in JavaScript. Clear cancelled composition state on blur and session
changes without erasing text, and remove measurement/listener resources on
unmount. Browser composition checks do not establish Windows UniKey acceptance;
report native verification separately.
When debugging through WSL, identify whether the window is a Linux app through
WSLg or a native Windows app. If host interop is available, test the actual
Windows UniKey path instead of declaring Windows testing unavailable. Compare
with an ordinary native GTK input to separate bridge failures from editor bugs.
For the reproduced WSLg backspace-without-Unicode case, UniKey's global clipboard
Unicode compatibility option worked; disclose any IME preference changes and
clipboard implications. Do not bypass Windows Application Control to run an
unsigned test build, and do not add JavaScript Telex conversion to mask a broken
OS input bridge.

## Frontend capabilities

Keep native platform access narrow. Folder selection uses
`@tauri-apps/plugin-dialog` with `dialog:allow-open`, and the fullscreen
control uses only `core:window:allow-set-fullscreen`; do not add shell,
process, or unrestricted filesystem permissions to the frontend.
