# Atlas-agentic
Your personal AI companion with persistent memory, intelligent agents, and tools to simplify everyday life.

## Project Direction

Atlas is being shaped as a desktop-first agent system with a Jev-like router that chooses which agent should process each task.

- See [VISION.md](VISION.md) for the system direction.
- See [AGENTS.md](AGENTS.md) for repo structure and coding-agent guidance.
- See [RULE.md](RULE.md) for architecture rules.

## Desktop Chat

Atlas includes a Tauri v2 desktop chat application with a vanilla TypeScript
frontend. It uses the same Rust runtime, local conversations, default tools,
agent definition, and environment variables as the CLI.

Install the platform packages from the official
[Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/), then
install the frontend dependencies:

```bash
npm ci --prefix desktop
```

Linux builds also need the native libraries used by screen capture: `libxcb`,
`libxrandr`, and `dbus` (for Debian/Ubuntu: `libxcb1 libxrandr2 libdbus-1-3`).

Launch the development app from the repository root:

```bash
npm --prefix desktop run tauri dev
```

The app opens on a blank, unsaved draft and never automatically resumes the
latest conversation. It creates an `Atlas` folder inside the current user's home
directory and uses that Atlas home as the starting folder, so a new chat is ready
without setup. Atlas creates the conversation ID, workspace assignment, and
local history only after the first model response completes successfully; a
failed first request remains an unsaved, retryable draft.

Chats can work concurrently. You can start a new chat, switch conversations,
open Settings, type, and scroll while Atlas continues in another chat. Each
chat keeps its own draft, streamed answer, activity, error, and tool approval.
The sidebar shows **Working**, **Needs approval**, or **Failed**; first requests
appear under **Current chats** until they succeed and receive a saved ID.
Switching away never cancels an approval. Return to its chat to allow or deny it
before the existing two-minute expiry.

Only one request may run in a given chat at a time. Sending again, renaming,
deleting, changing its folder, and changing its tool-access mode wait until that
chat finishes; other chats remain usable. Settings changes affect subsequent
requests, while running requests retain their starting configuration. Unsent
drafts and running requests are held for the current app lifetime; restarting
or reloading the app does not restore unfinished requests or unsaved drafts.

The desktop window adapts from compact phone-width layouts through fullscreen.
Use the header fullscreen control or press **F11** to toggle fullscreen.

The sidebar groups saved conversations by starting folder. Reopening a
conversation restores its folder, and changing the folder updates only its
metadata—not its history. Project folders remain optional. A new chat uses an
explicitly chosen draft folder first, then a custom Settings default, then Atlas
home. Older unassigned conversations use the current default and persist it with
their next successful turn.

Use **Settings** to configure the DeepSeek API key, base URL, default model,
history limit, default tool-access mode, and an optional custom default folder. After provider settings
are saved, Atlas loads the available models from DeepSeek and shows a model
picker above the chat composer. The dropdown shows model names only. A compact
context ring beside Send opens token details on hover, keyboard focus, or tap,
including the selected model's provider-reported maximum context (input plus
output). After a model call, the ring shows its reported input plus output
usage, including system instructions, selected history, and tool context in the
input count. The tooltip separates input and output. Each completed call replaces
the previous reading; tool rounds are not summed into context occupancy, and
conversation title generation does not replace it. Successful turns save the
reading with their activity sidecar so reopening a chat restores it. A reading
from a different selected model, or a provider response without usage, shows an
em dash; missing model limits remain unknown. Choosing a model saves it globally and affects
the next request without restarting Atlas. If model discovery fails, the saved
model remains usable and the picker offers a retry. Resetting the default folder
returns new chats to Atlas home. Conversation management and Settings remain
available without a configured provider; only sending requires an API key.

The tool-access picker beside the model picker controls confirmations for the
current chat. New drafts inherit the Settings default and may override it without
saving anything. After the first successful response, the selected mode belongs
to that conversation; reopening it restores the same mode, and changing it does
not affect other chats.

- **Ask** automatically permits ordinary workspace reads and asks before file
  changes, searches outside the workspace, application launches, commands,
  desktop observation, or mouse and keyboard control.
- **Workspace** also permits non-destructive changes inside the selected
  workspace, but asks before outside-workspace search, deletion, overwrite,
  launch, command execution, desktop observation, or mouse and keyboard control.
- **Full access** runs all registered tools without an Atlas confirmation. It
  does not elevate privileges: every action still runs with the desktop process's
  current OS-user permissions.

When confirmation is required, the chat shows the tool, a bounded argument
summary, and why approval is needed. **Allow once** approves only that request;
**Deny** returns the denial to the agent. Approval requests expire after two
minutes. Tool activity remains visible while Atlas works.

Build a local debug executable without creating an installer:

```bash
npm --prefix desktop run tauri -- build --debug --no-bundle
```

The folder picker uses Tauri's native dialog plugin. Development therefore needs
the operating-system packages listed in the official Tauri prerequisites guide;
the frontend receives only the narrow dialog-open permission.

## CLI Runtime

The default agent is loaded from:

```text
agents/atlas.toml
```

Run help without any model setup:

```bash
cargo run -- help
```

Call DeepSeek through the default local Atlas agent:

```bash
export DEEPSEEK_API_KEY="your_api_key_here"
cargo run -- ask "hello atlas"
```

Responses stream by default. The existing `--stream` form remains available as an alias:

```bash
cargo run -- ask --stream "hello atlas"
```

Each `ask` without an ID creates a new conversation. After the streamed answer,
Atlas prints `Conversation: <id> | <title>`. Continue that conversation explicitly:

```bash
cargo run -- ask --conversation <id> "now explain the runtime"
```

`--stream` and `--conversation` can appear in either order before the prompt.
Atlas does not automatically resume the latest conversation.

Manage saved conversations without an API key or agent configuration:

```bash
cargo run -- conversation list
cargo run -- conversation show <id>
cargo run -- conversation rename <id> "Atlas runtime work"
cargo run -- conversation delete <id>
```

List shows the most recently updated conversations first, with their IDs, titles,
creation/update times, and message counts. Show displays saved user and assistant
messages with UTC timestamps. Quote titles containing spaces.

### Local conversation storage

Conversation data belongs to the current OS user and is stored outside the repo:

- Linux and WSL: `$XDG_DATA_HOME/atlas`, normally `~/.local/share/atlas`.
- macOS: `~/Library/Application Support/atlas`.
- Windows: `%LOCALAPPDATA%\atlas`.

Atlas resolves these locations with a platform directory library.
`ATLAS_DATA_DIR` can override the data root for development or tests. The root
contains `conversations.json` (metadata), `conversations.lock`, one
`conversations/<uuid>.jsonl` history file per conversation, optional
`activities/<uuid>.jsonl` activity sidecars, optional `memory/<uuid>.json`
checkpoints, per-conversation execution leases, and `trash/`. Delete removes the
conversation from the index and moves its history, memory checkpoint, and any activity sidecar to
`trash/`. The command prints the history location so the messages can be
recovered manually; there is no CLI restore command.

Only successful user/final-assistant turns are saved to ordinary message
history. Observable runtime status, tool calls, and tool results from a
successful turn are stored separately in a bounded activity sidecar; a failed
overall turn saves neither messages nor activity. Arguments are redacted and
bounded, result details are bounded and local-only, and screenshot pixels and
attachments are never persisted. The desktop shows this record in an expandable
Activity section above each Atlas answer and includes the elapsed runtime/model
work duration for new turns.

Atlas keeps complete original history on disk. The bundled Atlas agent sends all
history until context compaction is needed. Automatic compaction targets **90% of
the selected model's token capacity**, checking the next request before every
model call, including tool follow-ups, and reserving space for output. The current
DeepSeek preflight counter is a conservative UTF-8 byte estimate, not an exact
tokenizer, so compaction can happen earlier than the actual usage ring suggests.
The ring continues to show the last actual chat-model usage; summary usage is
separate. Image token accounting is currently unavailable and explicitly disables
automatic compaction for that request rather than pretending images use zero tokens.

Use `/compact` in a saved desktop chat, or `atlas conversation compact <id>` from
the CLI. Manual compaction produces one summary below 30% occupancy, two from 30%
to below 60%, and three at 60% or higher. Automatic compaction targets three.
Short sources use fewer nonempty blocks. Each block produces one summary message;
the latest **10 original messages** remain unchanged (extended to whole exchanges).
Later turns load the saved summaries plus originals from the saved boundary onward.
Repeated compaction replaces the previous summaries together with that boundary.
All original messages remain visible, and View memory shows each summary and its
covered range. Summary requests use the selected provider and consume tokens.

Memory checkpoints publish atomically only after every bounded, no-tool summary
request succeeds and the prepared context fits. Empty, truncated, failed or
cancelled summaries retain the old checkpoint. Cancel compaction takes effect
between model requests, after an in-flight summary returns. Successful checkpoints
remain saved even if the subsequent ordinary answer fails. Deleting a conversation
moves memory to `trash/<uuid>.memory.json`; restoration is manual with its original
history/index. Per-conversation locks prevent competing asks/compactions across
processes while allowing other chats to continue.

Automatic compaction needs known context/output limits. When provider discovery
does not supply both, set **Model context limit** and **Model output limit** in
Settings for the active model, or configure `conversation.modelLimits` in the local
settings file. Both must be positive and output must be smaller than context.
Without usable limits/budgeting, Atlas shows compaction as unavailable and sends
full history; manual compaction returns a configuration error. Oversized requests
with known budgets fail without silently deleting protected recent messages.

The local `[memory]` policy and `conversation_summary_prompt` in `agents/atlas.toml`
configure thresholds, tail size, summary/output budgets, and bounded reduction.
Third-party agents without a memory policy keep their existing history behavior.
The saved legacy history-limit field remains compatible but is inactive for the
bundled memory mode. Explicit `ATLAS_HISTORY_MESSAGES` selects legacy bounded
history instead (positive integer, dropping a leading assistant when needed).
Only sanitized, separately capped activity belonging to the selected original
history is sent; covered activity is represented by memory rather than duplicated.

After the first successful answer, a separate, non-streaming LLM request generates
a title using only the first user input and the agent's configured title prompt.
This happens once per conversation. Titles are cleaned up and limited to 60 Unicode
characters; failed or empty title output leaves `Untitled conversation` without
failing the completed answer. Continuing or manually renaming a conversation
does not regenerate the title.

### Model and agent configuration

You can also put the same values in a local `.env` file. Start from `.env.example`; the real `.env` file is ignored by git.

Optional configuration:

```bash
export DEEPSEEK_BASE_URL="https://api.deepseek.com"
export DEEPSEEK_MODEL="deepseek-flash"
# Optional: opt into legacy bounded history instead of automatic memory
export ATLAS_HISTORY_MESSAGES="40"
```

The desktop Settings screen may store the API key in the user-local
`settings.json` file. This is a plaintext credential file protected by private
user filesystem permissions where supported; it is not an OS keychain. Use an
environment variable instead if that storage model is not appropriate. Never
commit `.env`, `settings.json`, or API keys.

Settings locations:

- Linux and WSL: `$XDG_CONFIG_HOME/atlas/settings.json`, normally `~/.config/atlas/settings.json`.
- macOS: `~/Library/Application Support/atlas/settings.json`.
- Windows: `%APPDATA%\atlas\settings.json`.

`ATLAS_CONFIG_DIR` overrides the containing directory for development and tests.
Environment variables take precedence field-by-field over saved Settings:
`DEEPSEEK_API_KEY`, `DEEPSEEK_BASE_URL`, `DEEPSEEK_MODEL`, and
`ATLAS_HISTORY_MESSAGES`. Environment values are shown as authoritative in the
UI but are never copied back into the settings file. Saved values come next,
followed by Atlas defaults and the local agent model configuration.

Agent files choose the provider:

```toml
model_provider = "deepseek"
model = "deepseek-flash"
temperature = 0.3
top_p = 0.9
```

Atlas can execute structured tool calls from the agent. `max_tool_calls` sets a
positive per-turn limit and includes failed or denied requests. Omit it to run
without an Atlas tool-call limit; the default Atlas agent omits it. Tool
definitions are passed to the model request at runtime, not hard-coded into the
local agent prompt.

The default agent is a general assistant for conversation, files, applications,
computer tasks, shell work, coding, and projects. It inspects relevant context,
uses tools when needed, preserves unrelated data, verifies material results, and
reports blockers honestly. It prefers the built-in `ripgrep` tool for content
search because ripgrep is generally faster, while `grep` remains available
through shell access. Native ripgrep is compiled into Atlas and does not require
an `rg` executable on `PATH`. Its behavior and separate
`conversation_title_prompt` are configured in `agents/atlas.toml`.

Default runtime tools:

- `current_time`: show the current date and time from the user's machine, including its local UTC offset
- `pwd`: show the current workspace path
- `ls`: list files and directories inside the workspace
- `read_file`: safely read up to 64 KiB from a UTF-8 file inside the workspace
- `ripgrep`: search file contents with compiled-in ripgrep matching, including approved absolute paths outside the workspace
- `search_files`: search workspace filenames and UTF-8 file contents with optional glob and result limits
- `create_file`: create a new UTF-8 file with a relative path, without overwriting an existing file
- `create_directory`: create a relative directory path inside the workspace
- `copy_file`: copy a workspace file to a relative destination; overwrite must be explicit
- `move_file`: move a workspace file to a relative destination; overwrite must be explicit
- `delete_file`: delete a workspace file
- `open_file`: open a workspace file with the operating system's default application
- `open_application`: launch an application by name or executable path
- `exec_command`: execute an unrestricted platform shell command from the workspace
- `mouse_move`: move the pointer with absolute or relative desktop coordinates
- `mouse_click`: click a mouse button in the active application
- `mouse_scroll`: scroll horizontally or vertically in the active application
- `type_text`: type Unicode text into the active application
- `key_press`: press a named or Unicode key with optional modifiers
- `get_ui_state`: read pointer, primary-display, and active-window metadata
- `capture_screen`: capture the primary display as an in-memory PNG for model vision

Computer-control tools act on the application active when the tool executes;
focus may change after approval, and a successful result means the OS accepted
the input rather than proving the target application handled it. Windows blocks
input into applications running at a higher integrity level. macOS requires the
user to grant Atlas Accessibility access. Linux input currently requires a
usable X11 display; unsupported Wayland-only sessions return an actionable
connection error. Full access skips the Atlas confirmation but does not bypass
these operating-system protections.

`capture_screen` observes only the primary display. In Ask and Workspace modes,
Atlas requests approval before capture and explains that visible desktop data may
be sent to the configured model provider. The PNG is held only in memory for the
current agent turn and provider request; it is not written to disk or saved in
conversation history. DeepSeek image requests reject dimensions above 8192
pixels per side and PNGs above 32 MiB. Windows uses native capture, macOS requires
Screen Recording permission, and Linux capture currently requires X11; a
Wayland-only session returns an actionable error.

Desktop control and capture stay in the Rust backend. The frontend retains only its narrow
dialog and fullscreen capabilities; it receives no shell, process,
accessibility, screen-capture, or unrestricted filesystem permission.

The workspace shown in the app is the tools' starting directory and conversation
label. Most structured path-based file helpers reject absolute paths, parent
traversal, and symlink escapes and remain scoped to that workspace. The native
`ripgrep` tool is the read-only exception: it accepts workspace-relative or
absolute search roots, respects ignore files, skips hidden and binary files and
symlinks by default, and bounds results to 200 matches and 64 KiB. Searches whose
canonical root is outside the workspace require approval in Ask and Workspace
modes. They run automatically only in Full access, remain limited by OS-user
permissions, and do not require an external `rg` executable. `exec_command` is
available on Linux, WSL, macOS, and Windows and is intentionally unrestricted: it supports
the platform shell's operators, pipes, redirects, changing directories, and
absolute paths outside the starting folder. It runs with the same permissions as
the Atlas process, so normal OS access controls still apply. Commands and
inherited output capture time out after 30 seconds, and stdout and stderr are each
limited to 64 KiB before truncation.

## Source layout and verification

CLI parsing and rendering live in `src/lib.rs`, with command routing in
`src/main.rs` and shared runtime setup in `src/app.rs`. `src/conversations/`
contains provider-independent types
and history-window selection (`types.rs`), activity sanitization and context
rendering (`activity.rs`), local persistence (`store.rs`), ask/save orchestration
(`service.rs`), and title sanitization (`title.rs`). `src/runtime.rs`
coordinates agents and tools; agents and model providers pass typed user/assistant
history without owning persistence. `src/settings.rs` owns versioned private
settings and environment precedence; `src/workspace.rs` provisions Atlas home.
`desktop/` contains the TypeScript
workspace-aware chat/Settings UI and native dialog adapter; nested
`desktop/src-tauri/` contains the Rust bridge and packaging.

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

Binary conversation tests use temporary data directories and a local fake model
server, so the test suite needs no real API key or external model service.
