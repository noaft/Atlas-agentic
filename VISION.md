# Atlas Vision

Atlas is a desktop-first general assistant for conversation, files, applications,
OS tasks, and software projects. It should feel like a local companion, with a
shared Rust core supporting both the desktop app and CLI.

## Current foundation

The app already has local conversations, memory compaction, multiple model
providers, runtime-controlled tools, Skills, MCP connections, and reusable agents.
Users can invoke a saved agent with `/name task`; Atlas can also delegate with
explicit tool grants, fork/task-only context, and wait/parallel execution.
Child agents have scoped approvals and inline reports, without independent
cross-run memory. See the [current features](README.md#features) and
[delegation contracts](docs/architecture/delegated-agents.md).

Automatic routing is not implemented. Atlas remains the default entry agent.

## Next direction: agent selection

A router should choose the right worker for a task:

```text
User task -> Router chooses agent -> Selected agent works -> Result
```

The router selects an agent; it does not perform the task or execute tools.
Explicit user selection should bypass automatic routing. Keep the selection
boundary separate from worker prompts, tool execution, and model providers.

A future router decision should be typed, for example:

```json
{
  "agent": "code_agent",
  "confidence": 0.92,
  "reason": "The user is asking to create or modify code."
}
```

The selected agent is the essential field. Confidence and reason support
inspection; low confidence should lead to clarification rather than arbitrary
routing. This is a proposed contract, not an existing runtime API.

## Growth principles

- Add specialists for demonstrated needs such as coding, planning, or research;
  do not build a large fixed roster before routing needs justify it.
- Let workers use appropriate configured providers/models without coupling the
  router to a single provider. Existing profile and capacity boundaries still apply.
- Preserve the desktop experience: usable default starting folder, optional
  project context, independent chats, and visible approvals and results.
- Keep permission and ownership boundaries in the Rust runtime. A starting folder
  organizes work; it does not grant OS privileges or confine shell commands.
- Keep responsibilities clear: router selects, agent works, provider calls the
  model, memory supplies context, and UI presents the interaction.

Implementation constraints belong in [RULE.md](RULE.md) and the
[architecture references](docs/README.md), rather than being repeated here.
