# Atlas Coding Rules

Follow these rules when changing this repo.

## 1. Do Not Hard-Code

Do not hard-code secrets, API keys, model names, file paths, URLs, prompts, or agent behavior inside random code.

Use config files or environment variables.

Good:

```text
DEEPSEEK_API_KEY from .env or process env
agents/atlas.toml for default agent prompt and model
```

Bad:

```rust
let api_key = "sk-...";
let prompt = "big prompt inside main.rs";
```

## 2. Design For Change

Do not write code that only works for the first provider, first model, first tool, or first use case.

Build small boundaries that can be swapped later.

Good:

```text
Runtime -> Agent -> ModelProvider
Runtime -> ToolRegistry -> ModelRequest.tools
Agent definition -> model_provider -> provider factory
Runtime -> ToolRegistry -> Tool
agents/*.toml for local agent definitions
```

Bad:

```text
DeepSeek-specific logic inside Runtime
create_file-specific logic mixed into Agent
tool JSON instructions hard-coded in agents/*.toml
one giant function that parses, decides, executes, and prints
```

When adding behavior, ask:

- Can another provider use this path?
- Can another tool use this pattern?
- Can tests use a fake implementation?
- Can config change without code changes?

## 3. Test First

Write or update tests before changing behavior.

For every new behavior, add a focused test that proves it.

Use:

```bash
cargo test
```

Before finishing, also run:

```bash
cargo fmt --check
```

## 4. Keep Functions Small

Functions should do one job.

If a function starts doing parsing, validation, execution, and formatting together, split it.

Good shape:

```text
parse input
validate input
execute action
format result
```

Each step should be easy to test.

## 5. Keep Boundaries Clear

Do not mix responsibilities.

- CLI only reads input and prints output.
- Runtime coordinates agents and tools.
- Agents call model providers.
- Model providers call external APIs.
- Tools perform controlled side effects.
- Config loads settings.

## 6. Provider-Agnostic Code

Do not spread provider-specific logic through the app.

DeepSeek, OpenAI, local models, or any other provider should hide behind `ModelProvider`.

Core code should depend on Atlas types, not provider response structs.

## 7. Agents Are Local Config First

Do not create a new Rust agent class just because the prompt is different.

Use local files first:

```text
agents/atlas.toml
agents/coder.toml
agents/planner.toml
```

Create a new Rust agent type only when behavior is truly different.

## 8. Tools Are Runtime Controlled

Agents do not execute tools directly.

Agents can request a tool. Runtime parses, validates, and executes it.

Every tool needs:

- typed input
- validation
- tests
- clear errors

## 9. Keep File Operations Safe

For file tools:

- no absolute paths
- no `..`
- no writes outside workspace
- no overwrite unless explicitly supported
- create tests for path safety

## 10. No Big-Bang Refactors

Make small, reviewable changes.

Do not rewrite unrelated code while adding a feature.

## 11. Update Docs When Structure Changes

If repo structure, commands, env vars, or agent files change, update:

- `README.md`
- `AGENTS.md`
- `VISION.md` when product direction changes
- `RULE.md` when coding rules change
