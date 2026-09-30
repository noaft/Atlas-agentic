# Agents Guide

This file tells coding agents what this repo is and how to work in it.

## Project

Atlas Agentic is a Rust project for a desktop-first AI agent system.

The first product idea is a Jev-like router that chooses which agent should process a user task. The router is not the worker. It only selects the agent name.

Current state: minimal Rust CLI skeleton.

## Current Repo Structure

```text
.
├── Cargo.toml
├── Cargo.lock
├── README.md
├── VISION.md
├── AGENTS.md
├── src
│   ├── main.rs
│   └── lib.rs
└── tests
    └── cli_behavior.rs
```

## Current Code

`src/main.rs` is the CLI entrypoint. It reads command line args and prints the rendered result.

`src/lib.rs` currently contains:

- `AppResult`: output type for CLI behavior
- `handle_command`: parses CLI commands
- `render`: converts app result into display text
- a temporary internal `agent` module that echoes the prompt

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

## Near-Term Architecture

The next code phase should introduce explicit agent routing.

Target structure:

```text
src/
  main.rs
  lib.rs
  router.rs
  agents/
    mod.rs
    chat.rs
    code.rs
    planner.rs
    research.rs
    memory.rs
  models/
    mod.rs
    deepseek.rs
```

Keep this simple at first. Do not build a large framework before the router and agent contracts are clear.

## Agent Selection Design

The router should choose one agent name for a task.

Initial agent names:

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

## GitHub CI

The repo has GitHub Actions at `.github/workflows/ci.yml`.

CI checks:

- `cargo fmt --check`
- `cargo test`
