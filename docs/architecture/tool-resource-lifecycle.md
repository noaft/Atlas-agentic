# Tool and resource lifecycle decisions

Status: original resource-ownership design approved on 2026-11-09; desktop scopes
implemented. The execution design was superseded by
[Tool execution control](tool-execution-control.md) on 2026-11-13. The original
Processes disclosure was replaced with inline tool cards on 2026-11-10.

This document preserves the ownership rationale. The old direct-execution branch,
Processes panel proposal, and drafting chronology remain in Git history rather
than serving as current implementation instructions.

## Ownership boundaries

| Component | Responsibility |
| --- | --- |
| Tool catalog | Metadata/schema; loading never launches processes or connections |
| Per-turn runtime | Validates arguments, tool grants, workspace, and approvals |
| ToolExecutionManager | Owned execution records, scheduler, mailbox, durable results |
| ProcessManager / AgentRunManager | Resource handles, quotas, state, output, cleanup |

Do not turn a request's ToolRegistry into an app singleton carrying old settings
or workspace. Desktop runtimes receive native scope handles from app-owned
managers. The scheduler does not execute shells, call providers, or grant authority.

Native context determines the owner. Opaque resource IDs never authorize access
by themselves; the model cannot select another chat owner. Existing resources
retain their launch cwd, while later operations use current authorization policy.

## Lifetimes

- A runtime request is one agent turn. Job commands clean up on its exit.
- An explicitly requested background command can survive turns in its chat.
  It remains bounded and owned; running `&` is not managed background lifetime.
- Draft scopes exist without history. Saving binds the same scope and resource
  IDs to a conversation; failures leave recoverable drafts rather than ownerless work.
- MCP connections remain runtime-owned. This design does not migrate them into
  chat-owned background services.
- Restart restores neither processes nor child-run handles. Durable execution
  results are evidence, not live resources or permission to replay actions.

## Output and cleanup

Independent readers use explicit cursors over bounded stdout/stderr rings, so UI
inspection cannot consume agent output. Reads report loss and incomplete UTF-8.
Current UI receives tool-event output inline and does not poll a Processes panel.

Stop requests cancellation; confirmed cleanup comes from worker state. Preserve
Unix process groups and Windows Jobs. Cleanup runs on workers with deadlines,
without blocking the Tauri event thread or broadening frontend permissions.
Do not guarantee cleanup after every crash, power loss, or deliberate containment escape.

## Current contracts

- [Process sessions](process-sessions.md): arguments, lifetimes, output/cursors,
  quotas, draft ownership, OS containment, and source files.
- [Wakeup](wakeup.md): execution tracking, scheduling, final guards, persistence,
  cancellation, and restart recovery.
- [Delegated agents](delegated-agents.md): child ownership, grants, and reports.
- [Desktop state](desktop-state.md) and [Live chat input](live-chat-input.md): UI,
  workspace authority, queues, steering, and Stop.
- [Process demo](../guides/process-manager-demo.md): no-key ownership walkthrough.
