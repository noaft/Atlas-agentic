# Agents Guide

This file tells coding agents what this repo is and how to work in it.

## Project

Atlas Agentic is a Rust project for a desktop-first AI agent system.

Read `RULE.md` before changing agent, provider, runtime, or tool architecture.

The first product idea is a Jev-like router that chooses which agent should process a user task. The router is not the worker. It only selects the agent name.

Current state: minimal Rust CLI skeleton.
Current runtime: CLI `ask` calls a `Runtime`, which sends the prompt to the default local Atlas agent loaded from `agents/atlas.toml`. `LocalAgent` calls a `ModelProvider`. The provider is selected from the local agent definition.

## Current Repo Structure

```text
.
├── Cargo.toml
├── Cargo.lock
├── README.md
├── VISION.md
├── AGENTS.md
├── agents
│   └── atlas.toml
├── src
│   ├── main.rs
│   ├── lib.rs
│   ├── runtime.rs
│   ├── agent_output.rs
│   ├── agents
│   │   ├── mod.rs
│   │   └── local.rs
│   ├── models
│       ├── mod.rs
│       └── deepseek.rs
│   └── tools
│       ├── mod.rs
│       ├── create_file.rs
│       ├── list_dir.rs
│       └── pwd.rs
└── tests
    ├── agents_behavior.rs
    ├── cli_behavior.rs
    ├── cli_binary.rs
    ├── models_behavior.rs
    └── tool_call_behavior.rs
```

## Current Code

`src/main.rs` is the CLI entrypoint. It reads command line args and prints the rendered result.

`src/lib.rs` currently contains:

- `AppResult`: output type for CLI behavior
- `handle_command`: parses CLI commands
- `render`: converts app result into display text

`src/runtime.rs` contains the first runtime boundary.

`src/agent_output.rs` parses raw model text into internal Atlas output types.

`src/agents/mod.rs` contains:

- `Agent`
- `AgentRegistry`

Concrete local agent definitions live in `agents/*.toml`.

The default agent is:

- `agents/atlas.toml`: Atlas, the default local agent

Rust agent runtime:

- `src/agents/local.rs`: `LocalAgent` and `LocalAgentDefinition`

Planned future agents should start as new local TOML definitions. Add new Rust agent types only when they need behavior beyond a different prompt/config.

`src/models/mod.rs` contains the `ModelProvider` trait.

`src/models/mod.rs` also contains the provider factory. `src/models/deepseek.rs` contains the DeepSeek chat-completions provider.

`src/tools/mod.rs` contains the `Tool` trait and registry.

`src/tools/create_file.rs` contains the first safe file creation tool.
`src/tools/list_dir.rs` contains the safe `ls` workspace listing tool.
`src/tools/pwd.rs` contains the `pwd` workspace path tool.

Tool definitions are passed to model providers through `ModelRequest.tools`. Do not hard-code tool protocol instructions in `agents/*.toml`.

`tests/cli_behavior.rs` covers the current CLI behavior.

## Commands

Use these before finishing changes:

```bash
cargo fmt --check
cargo test
```

Use this to run the current CLI:

```bash
cargo run -- ask "hello atlas"
```

The CLI loads `agents/atlas.toml` by default.

The real CLI needs `DEEPSEEK_API_KEY` for `ask`. It can come from the process environment or a local `.env` file:

```bash
export DEEPSEEK_API_KEY="your_api_key_here"
```

Optional env vars:

```bash
DEEPSEEK_BASE_URL=https://api.deepseek.com
DEEPSEEK_MODEL=deepseek-flash
```

Local agent files choose their provider:

```toml
model_provider = "deepseek"
model = "deepseek-flash"
```

## Near-Term Architecture

The next code phase should introduce explicit agent routing.

Target structure:

```text
src/
  main.rs
  lib.rs
  agents/
    mod.rs
    local.rs
  models/
    mod.rs
    deepseek.rs
```

Local agent definitions can grow like:

```text
agents/
  atlas.toml
  coder.toml
  planner.toml
```

Keep this simple at first. Do not build a large framework before the router and agent contracts are clear.

## Agent Selection Design

The router should choose one agent name for a task.

Planned local agent names:

- `chat_agent`
- `code_agent`
- `planner_agent`
- `research_agent`
- `memory_agent`

Expected routing flow:

```text
user input
  -> router
  -> route decision
  -> agent registry
  -> selected agent
  -> model provider
  -> response
```

The first implementation can use rules. Later implementation can call a Jev-compatible decision model.

## Model Direction

DeepSeek AI is the first planned worker model provider.

Do not make every agent call DeepSeek directly from random places. Add a provider boundary so agents can share model calling code.

Preferred idea:

```text
Agent
  -> ModelProvider trait
  -> DeepSeekProvider
```

## Development Guidelines

- Keep Rust modules small and readable.
- Add tests for routing behavior before changing routing code.
- Avoid adding async, HTTP clients, databases, or desktop dependencies until they are needed.
- Keep routing separate from agent execution.
- Keep model provider code separate from agent logic.
- Do not commit API keys or secrets.

## Commit Help

When a coding agent changes files, it should help prepare the commit.

Before committing:

- run `cargo fmt --check`
- run `cargo test`
- check `git status --short`
- summarize the important changed files
- suggest a clear commit message

If the user asks to commit, create one focused commit for the current task. Do not include `.env`, secrets, `target/`, or unrelated local changes.

## GitHub CI

The repo has GitHub Actions at `.github/workflows/ci.yml`.

CI checks:

- `cargo fmt --check`
- `cargo test`
