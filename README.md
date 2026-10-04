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

You can also put the same values in a local `.env` file. Start from `.env.example`; the real `.env` file is ignored by git.

Optional configuration:

```bash
export DEEPSEEK_BASE_URL="https://api.deepseek.com"
export DEEPSEEK_MODEL="deepseek-flash"
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

Default runtime tools:

- `pwd`: show the current workspace path
- `ls`: list files and directories inside the workspace
- `create_file`: create a new file with a relative path
- `exec_command` (Unix-like systems): execute an unrestricted shell command from the workspace

`exec_command` is available on Linux, WSL, and macOS. It supports shell operators, pipes, redirects, and commands that access paths outside the workspace. It runs with the same permissions as the Atlas process. Commands and inherited output capture time out after 30 seconds, and stdout and stderr are each limited to 64 KiB before truncation.
