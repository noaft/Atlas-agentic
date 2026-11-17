# Atlas Coding Rules

Follow these rules when changing the repository. Detailed subsystem contracts are
linked from [AGENTS.md](AGENTS.md#read-by-change-area).

## 1. Configure values and behavior

Keep secrets, model names, paths, URLs, prompts, and agent behavior out of random
implementation code. Use configuration files or environment variables, such as
`DEEPSEEK_API_KEY` and `agents/atlas.toml`. Do not hard-code provider sampling settings.

## 2. Design for change

Keep small, replaceable boundaries:

```text
Runtime -> Agent -> ModelProvider
Runtime -> ToolRegistry -> Tool
Agent definition -> provider factory
```

New paths should support another provider/tool, fake test implementations, and
configuration changes. Avoid functions that combine parsing, validation,
execution, and rendering.

## 3. Test behavior first

Write or update focused tests before changing behavior. Every new behavior needs
a test proving it. Run `cargo test` and `cargo fmt --check` before finishing;
[AGENTS.md](AGENTS.md#verification) lists additional checks by scope.

## 4. Keep functions focused

Give each function one job. Split parsing, validation, execution, and formatting
when they become separate responsibilities.

## 5. Keep boundaries clear

- CLI reads input and renders output.
- Runtime coordinates agents and controls tool execution.
- Agents call model providers; providers call external APIs.
- Tools perform controlled side effects; config loads settings.

## 6. Keep providers behind ModelProvider

Core code depends on Atlas types, not provider response structs. Keep native
payloads, parsing, and provider-specific behavior inside adapters rather than
spreading them through runtime or UI.

## 7. Prefer local agent definitions

Use local TOML definitions when prompts differ. Add a Rust agent type only when
its behavior requires a different implementation.

## 8. Runtime controls tools

Agents request tools; runtime parses, validates, authorizes, and executes them.
Every tool needs typed input, validation, focused tests, and clear errors.
Tool schemas and protocol instructions belong to runtime, not agent TOML prompts.

## 9. Preserve file-operation safety

Structured workspace file helpers reject absolute paths, `..`, unsafe parents,
and symlink escapes. Keep writes within the validated workspace; overwrite only
when explicitly supported and authorized. Test path safety and stale approvals.

`exec_command` is intentionally unrestricted; `ripgrep` accepts approved absolute
roots outside the workspace. Preserve their separate authorization contracts in
[file and computer tools](docs/architecture/file-and-computer-tools.md).

## 10. Make small changes

Keep changes reviewable. Do not rewrite unrelated code while adding a feature.

## 11. Update the owning documentation

Update the relevant reference when behavior, configuration, or structure changes.
Keep `README.md` for quick start, `AGENTS.md` for contributor instructions and links,
`VISION.md` for product direction, and this file for coding rules. Avoid repeating
a detailed contract across these documents.
