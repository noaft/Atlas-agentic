# Atlas Agentic

A desktop-first AI assistant for chat, files, shell commands, computer control,
and coding. The Tauri desktop app and Rust CLI share a Rust runtime and local
conversation history.

**Status:** Atlas supports reusable child agents with parent-selected tools. Automatic routing is planned.

## Run the desktop app

Install Rust (rustup), Node.js/npm, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
Use the versions in [rust-toolchain.toml](rust-toolchain.toml) and
[.node-version](.node-version) to match CI. On Debian/Ubuntu, also install:

```bash
sudo apt install libxcb1 libxrandr2 libdbus-1-3 libdbus-1-dev pkg-config
```

From the repository root:

```bash
npm ci --prefix desktop
npm --prefix desktop run tauri dev
```

Open **Settings → Models & providers**, choose a provider, add its API key, and
select or enter a model ID. New chats start in an automatically created `Atlas`
folder in your home directory; you can choose another folder per chat.

## Features

- **Chat:** streaming replies, independent conversations, local history and Activity.
- **Models:** DeepSeek, OpenAI, Anthropic, Gemini, and OpenAI-compatible endpoints.
- **Files and coding:** search, read by line range, paginate directory listings,
  create, patch, move, copy, and delete files.
- **Commands:** run shell commands, inspect output, and manage per-chat background
  processes on desktop. Inspect the last N log lines and their positions.
  Active tools appear inline as terminal cards.
- **Computer control:** open applications, capture the screen, and use mouse/keyboard.
- **Skills:** Personal and Project libraries; import from GitHub, folders, or files;
  preview, edit, search, and enable/disable skills.
- **MCP:** local stdio and remote HTTP servers, OAuth sign-in, and explicit tool checks.
- **Agents:** reusable Personal/Project definitions, fork or task-only runs, parallel
  delegation, scoped approvals, and live run/results inspection.
- **Context:** token-usage display and automatic/manual conversation compaction.

## CLI

Set a provider API key in your environment or a local `.env` file
(see [.env.example](.env.example)):

```bash
export DEEPSEEK_API_KEY="your_api_key_here"
cargo run -- ask "hello atlas"
```

Replies stream by default. Each request starts a new conversation; use the ID
printed after a reply to continue it:

```bash
cargo run -- ask --conversation <id> "continue"
cargo run -- conversation list
cargo run -- conversation compact <id>
cargo run -- help
```

Help works without an API key. The CLI starts in your current directory.

## Permissions and storage

| Mode | Allowed without confirmation |
| --- | --- |
| Ask | Ordinary workspace reads |
| Workspace | Reads and non-destructive structured workspace changes |
| Full access | All registered tools within your OS-user permissions |

Set concurrent-command limits per chat and across the app in **Settings → Permissions**.

The starting folder is not a shell sandbox. Commands can access other paths with
normal OS permissions. Desktop observation/control requires approval in Ask and
Workspace modes. Screenshots stay in memory for the turn and may be sent to your
model provider.

History stays local. Saved model API keys are plaintext in a private user-local
settings file; MCP OAuth credentials use the OS credential store.

## Development

```bash
cargo fmt --check
cargo test
npm --prefix desktop test
npm audit --prefix desktop --audit-level=high
npm --prefix desktop run build
```

## Documentation

- [Detailed reference](docs/guides/reference.md): provider configuration, tools,
  Skills, MCP, memory, storage paths, and logs.
- [Delegated agents](docs/architecture/delegated-agents.md): definitions, tools, runs and reports.
- [Process demo](docs/guides/process-manager-demo.md): try command sessions without
  provider credentials.
- [Documentation index](docs/README.md), including the Vietnamese input fix for WSLg.
- [Repository guide](AGENTS.md) · [Coding rules](RULE.md) · [Product vision](VISION.md).


### Wakeup for long tool calls

Tools can opt into `execution: {"mode":"job","yield_ms":2000}` at the top level of
an Atlas tool call, or choose `task` to return an execution ID immediately. The
backend scheduler wakes the owning agent on its deadline (always below30seconds)
or completion. Wakeup messages contain metadata only. The agent explicitly reads
1–200 tail lines or a byte page, with a hard4KiB content limit; it can then change
its next wait. Starting an execution and checking it are separate operations:
waiting never replays a tool. Existing approvals and command lifetimes still apply.

Run `cargo run --example wakeup` for a deterministic, no-key walkthrough with an
in-memory tool and fake agent. See [Wakeup architecture](docs/architecture/wakeup.md)
for ownership, mailboxes, cancellation, output bounds and the model protocol.
