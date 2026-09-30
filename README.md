# Atlas-agentic
Your personal AI companion with persistent memory, intelligent agents, and tools to simplify everyday life.

## Project Direction

Atlas is being shaped as a desktop-first agent system with a Jev-like router that chooses which agent should process each task.

- See [VISION.md](VISION.md) for the system direction.
- See [AGENTS.md](AGENTS.md) for repo structure and coding-agent guidance.

## CLI Runtime

Run help without any model setup:

```bash
cargo run -- help
```

Call DeepSeek through the first `chat_agent` runtime:

```bash
export DEEPSEEK_API_KEY="your_api_key_here"
cargo run -- ask "hello atlas"
```

You can also put the same values in a local `.env` file. Start from `.env.example`; the real `.env` file is ignored by git.

Optional configuration:

```bash
export DEEPSEEK_BASE_URL="https://api.deepseek.com"
export DEEPSEEK_MODEL="deepseek-flash"
```

Secrets must come from environment variables. Do not hard-code API keys.
