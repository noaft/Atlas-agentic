# Atlas Agentic Repository Guide

## What This Repository Is

Atlas Agentic is a Rust project for a desktop-first AI agent system.

The current product is a command-line runtime that loads the default Atlas agent from `agents/atlas.toml`, sends requests through a configured model provider, and lets the runtime execute registered tools. It persists multiple conversations in the current user's OS-local data directory, with explicit continuation by ID and local management commands.

The planned direction is a multi-agent system with a router that selects the best agent for each user task. The router chooses an agent; it does not perform the task itself.

Read `RULE.md` before changing agent, provider, runtime, router, or tool architecture.

## General Repository Structure

```text
.
├── agents/                 Local agent definitions
│   └── atlas.toml          Default Atlas agent configuration
├── src/
│   ├── main.rs             CLI entrypoint and runtime setup
│   ├── lib.rs              CLI command parsing and rendering
│   ├── runtime.rs          Agent and tool orchestration
│   ├── agent_output.rs     Final-answer and tool-call parsing
│   ├── agents/             Agent traits and local agent implementation
│   ├── conversations/      Local conversation types, storage, and orchestration
│   │   ├── types.rs        UUIDs, messages, metadata, and context-window policy
│   │   ├── store.rs        Locked JSONL history and atomic metadata updates
│   │   ├── service.rs      Ask, successful-turn persistence, and one-time titles
│   │   └── title.rs        Title sanitization
│   ├── models/             Model-provider trait and DeepSeek provider
│   └── tools/              Tool trait, registry, and built-in tools
├── tests/                  Integration and behavior tests
├── Cargo.toml              Rust package configuration
├── README.md               User-facing project documentation
├── VISION.md               Product direction
└── RULE.md                 Coding and architecture rules
```

The built-in runtime tools are:

- `pwd`: show the current workspace path
- `ls`: list files and directories inside the workspace
- `create_file`: safely create a new file inside the workspace
- `exec_command` (Unix-like systems): execute an unrestricted shell command from the workspace

`exec_command` is available on Linux, WSL, and macOS and is intentionally unrestricted. It supports shell operators, pipes, redirects, and paths outside the workspace, and it runs with the same permissions as the Atlas process. Its default command-and-output-capture timeout is 30 seconds, with stdout and stderr each truncated after 64 KiB.

## Configure and Run

Show CLI help without configuring a model:

```bash
cargo run -- help
```

Set the DeepSeek API key in the environment or a local `.env` file:

```bash
export DEEPSEEK_API_KEY="your_api_key_here"
```

Optional model configuration:

```bash
export DEEPSEEK_BASE_URL="https://api.deepseek.com"
export DEEPSEEK_MODEL="deepseek-flash"
```

Run a request. Responses stream by default:

```bash
cargo run -- ask "hello atlas"
```

The existing `--stream` form remains available as an alias:

```bash
cargo run -- ask --stream "hello atlas"
```

Every `ask` without `--conversation` starts a new conversation. After the streamed
answer, Atlas prints its ID and title. Use that ID explicitly to continue:

```bash
cargo run -- ask --conversation <id> "now explain the runtime"
cargo run -- conversation list
cargo run -- conversation show <id>
cargo run -- conversation rename <id> "Atlas runtime work"
cargo run -- conversation delete <id>
```

`--stream` and `--conversation` accept either order before the prompt. Quote
multiword titles. Help and all conversation management commands parse and run
without a model API key or agent configuration. List is newest-update-first and
shows IDs, titles, creation/update times, and message counts; show includes saved
roles, content, and UTC timestamps.

Conversation storage is resolved using a platform directory library: Linux/WSL
uses `$XDG_DATA_HOME/atlas` (normally `~/.local/share/atlas`), macOS uses
`~/Library/Application Support/atlas`, and Windows uses `%LOCALAPPDATA%\atlas`.
`ATLAS_DATA_DIR` overrides the root for tests and development. The root contains
`conversations.json`, `conversations.lock`, `conversations/<uuid>.jsonl`, and
`trash/`. Delete moves history to `trash/` and reports its recovery location;
recovery is manual, with no CLI restore command.

`ConversationService` coordinates the store and runtime. Save only successful
user/final-assistant pairs, never internal tool calls/results or partial failed
answers. Keep full history on disk, passing at most the latest 40 messages to the
model by default. `ATLAS_HISTORY_MESSAGES` must be a positive integer; selected
history stays chronological and drops a leading assistant message when needed.
The current input is additional to that limit.

After the first successful answer, the agent makes one non-streaming, no-tool title
request using only the first input and `conversation_title_prompt` from
`agents/atlas.toml`. Sanitize generated titles to at most 60 Unicode characters;
failure leaves `Untitled conversation`. Continuing and manual rename never
regenerate the title. The default agent prompt guides workspace inspection,
command execution, verification, preservation of unrelated changes, and honest
reporting of results and blockers. Tool schemas and protocol remain runtime-injected.

Format and test the repository:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

CLI binary tests isolate their workspace and `ATLAS_DATA_DIR`, use child-only
environment configuration, and serve fake DeepSeek responses over loopback TCP.
Do not use real API keys or external model services in tests.

## Commit Traceability Rule

After completing a requested code or documentation change, create one focused Git commit so the work can be traced, unless the user explicitly asks not to commit.

Before committing:

- Run `cargo fmt --check`.
- Run `cargo test`.
- Check `git status --short`.
- Review the staged diff.
- Commit only files that belong to the current task.
- Never commit `.env`, API keys, secrets, `target/`, or unrelated changes.

Use a short commit message that describes the completed change, such as:

```text
feat: send tool results back to agents
fix: handle streamed tool responses
docs: simplify agent repository guide
```
