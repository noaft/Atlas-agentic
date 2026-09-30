# Atlas Vision

Atlas is a desktop-first agent system for choosing the right AI worker for a task.

The first goal is not to build every agent at once. The first goal is to build a clean agent runtime where a router can look at a user request and choose the best agent name to process it.

## Core Idea

Atlas uses a Jev-like router as the decision layer.

```text
User task
  -> Router chooses agent name
  -> Selected agent processes the task
  -> Atlas returns the result
```

The router does not execute tools and does not complete the task itself. Its job is only to choose the right agent or model path.

## Why A Router

The main problem is agent selection. A single assistant prompt becomes messy when every task, tool, and model is described in one place. Atlas should keep the decision separate from the worker.

The router answers questions like:

- Which agent should process this task?
- Is this coding, planning, research, memory, or normal chat?
- Does the task need a specialist model?
- Is confidence high enough to continue?
- Should Atlas ask the user for clarification instead of guessing?

## Desktop First

Atlas will start as a desktop app because the user experience should feel like a local companion, not only a terminal script.

The current repo starts with a Rust CLI because it is the simplest runtime foundation. The CLI lets us build and test the agent core before adding the desktop interface.

Planned direction:

```text
Rust core
  -> CLI for development
  -> Desktop app shell
  -> Agent router
  -> Agent registry
  -> Model providers
  -> Memory later
```

## First Phase

Phase 1 builds the agent code structure.

The target is:

```text
atlas ask "build a login page"
  -> router chooses code_agent
  -> code_agent uses its model/provider
  -> response returns to user
```

For this phase, the router can be simple rules or a mock Jev-like router. Later it can call a real Jev-compatible decision model.

## Model Strategy

DeepSeek AI is the first planned worker model for agent processing.

The router chooses an agent name. The selected agent decides how to use its model prompt and provider.

Example:

```text
Task: "make a project plan"
Router decision: planner_agent
Worker model: DeepSeek
```

Later, different agents can use different models.

## Initial Agent Names

Atlas should start with a small set of clear agents:

- `chat_agent`: default conversation and simple questions
- `code_agent`: coding, repo edits, debugging, tests, and technical work
- `planner_agent`: breaks goals into steps and milestones
- `research_agent`: gathers information and summarizes findings
- `memory_agent`: stores and retrieves user/project memory later

More agents should only be added when there is a real routing need.

## Routing Contract

The router should return a typed decision:

```json
{
  "agent": "code_agent",
  "confidence": 0.92,
  "reason": "The user is asking to create or modify code."
}
```

The important field is `agent`. Confidence and reason help with debugging and future UI display.

If confidence is low, Atlas should ask a clarifying question instead of sending the task to the wrong agent.

## Design Rule

Atlas should keep these responsibilities separate:

- Router chooses the agent.
- Agent processes the task.
- Provider calls the model.
- Memory stores context.
- UI displays the interaction.

This keeps the system simple enough to grow.
