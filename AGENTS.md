# Atlas Agentic Repository Guide

## What This Repository Is

Atlas Agentic is a Rust project for a desktop-first AI agent system.

The current product is a command-line runtime that loads the default Atlas agent from `agents/atlas.toml`, sends requests through a configured model provider, and lets the runtime execute registered tools.

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

Format and test the repository:

```bash
cargo fmt --check
cargo test
```

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
