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

The sidebar groups saved conversations by starting folder. Reopening a
conversation restores its folder, and changing the folder updates only its
metadata—not its history. Project folders remain optional. A new chat uses an
explicitly chosen draft folder first, then a custom Settings default, then Atlas
home. Older unassigned conversations use the current default and persist it with
their next successful turn.

Use **Settings** to configure the DeepSeek API key, base URL, model, history
limit, and an optional custom default folder. Resetting that folder returns new
chats to Atlas home. Settings affect the next request without restarting Atlas.
Conversation management and Settings remain available without a configured
provider; only sending requires an API key.

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
`conversations/<uuid>.jsonl` history file per conversation, and `trash/`.
Delete removes the conversation from the index and moves its history to `trash/`.
The command prints that file's location so the messages can be recovered manually;
there is no CLI restore command.

Only successful user/final-assistant turns are saved. Tool calls, tool results,
and partial failed answers are not part of saved conversation history. Atlas
keeps complete history on disk and sends the most recent 40 messages by default
(up to 20 complete turns) to the model. Set `ATLAS_HISTORY_MESSAGES` to a positive
integer to change that limit. A cut window drops a leading assistant message so
the selected history begins with a user message; the new input is always added
after the window and does not count toward the limit.

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

Atlas can execute structured tool calls from the agent. Tool definitions are passed to the model request at runtime, not hard-coded into the local agent prompt.

The default agent is a general assistant for conversation, files, applications,
computer tasks, shell work, coding, and projects. It inspects relevant context,
uses tools when needed, preserves unrelated data, verifies material results, and
reports blockers honestly. Its behavior and separate
`conversation_title_prompt` are configured in `agents/atlas.toml`.

Default runtime tools:

- `pwd`: show the current workspace path
- `ls`: list files and directories inside the workspace
- `read_file`: safely read up to 64 KiB from a UTF-8 file inside the workspace
- `create_file`: create a new file with a relative path
- `exec_command` (Unix-like systems): execute an unrestricted shell command from the workspace

The workspace shown in the app is the tools' starting directory and conversation
label. The structured file helpers remain scoped to it. `exec_command` is
available on Linux, WSL, and macOS and is intentionally unrestricted: it supports
shell operators, pipes, redirects, changing directories, and absolute paths
outside the starting folder. It runs with the same permissions as the Atlas
process, so normal OS access controls still apply. Commands and inherited output
capture time out after 30 seconds, and stdout and stderr are each limited to 64
KiB before truncation.

## Source layout and verification

CLI parsing and rendering live in `src/lib.rs`, with command routing in
`src/main.rs` and shared runtime setup in `src/app.rs`. `src/conversations/`
contains provider-independent types
and history-window selection (`types.rs`), local persistence (`store.rs`), ask/save
orchestration (`service.rs`), and title sanitization (`title.rs`). `src/runtime.rs`
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
