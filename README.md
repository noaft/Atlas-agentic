# Atlas-agentic
Your personal AI companion with persistent memory, intelligent agents, and tools to simplify everyday life.

## Project Direction

Atlas is being shaped as a desktop-first agent system with a Jev-like router that chooses which agent should process each task.

- See [VISION.md](VISION.md) for the system direction.
- See [AGENTS.md](AGENTS.md) for repo structure and coding-agent guidance.
- See [RULE.md](RULE.md) for architecture rules.

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

Secrets must come from environment variables. Do not hard-code API keys.

Agent files choose the provider:

```toml
model_provider = "deepseek"
model = "deepseek-flash"
temperature = 0.3
top_p = 0.9
```

Atlas can execute structured tool calls from the agent. Tool definitions are passed to the model request at runtime, not hard-coded into the local agent prompt.

The default agent inspects relevant workspace context before making repository
claims, runs commands when needed for implementation and verification, preserves
unrelated changes, and reports completed work, verification, and blockers honestly.
Its behavior and separate `conversation_title_prompt` are configured in
`agents/atlas.toml`.

Default runtime tools:

- `pwd`: show the current workspace path
- `ls`: list files and directories inside the workspace
- `create_file`: create a new file with a relative path
- `exec_command` (Unix-like systems): execute an unrestricted shell command from the workspace

`exec_command` is available on Linux, WSL, and macOS. It supports shell operators, pipes, redirects, and commands that access paths outside the workspace. It runs with the same permissions as the Atlas process. Commands and inherited output capture time out after 30 seconds, and stdout and stderr are each limited to 64 KiB before truncation.

## Source layout and verification

CLI parsing and rendering live in `src/lib.rs`, with command routing and runtime
setup in `src/main.rs`. `src/conversations/` contains provider-independent types
and history-window selection (`types.rs`), local persistence (`store.rs`), ask/save
orchestration (`service.rs`), and title sanitization (`title.rs`). `src/runtime.rs`
coordinates agents and tools; agents and model providers pass typed user/assistant
history without owning persistence.

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Binary conversation tests use temporary data directories and a local fake model
server, so the test suite needs no real API key or external model service.
