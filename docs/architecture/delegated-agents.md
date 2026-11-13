# Delegated agents

Status: architecture approved for implementation on 2026-11-13. Implemented in the Rust runtime and desktop chat. Automatic top-level routing remains planned.

## Definitions and runs

Atlas remains the default entry agent from `agents/atlas.toml`. Reusable child definitions use a separate bounded TOML format. Personal definitions live in `<Atlas settings directory>/agents/`; Project definitions live in `<workspace>/.agents/agents/`. Their identities are `personal:<name>` and `project:<name>`. Each scope permits 64 definitions, 128 combined. Names contain 1–64 lowercase ASCII letters, digits or hyphens. Files are limited to128KiB. Creating a definition never overwrites an existing file or follows symlinks. Personal files/directories use private creation permissions on Unix.

```toml
name = "reviewer"
display_name = "Code Reviewer"
description = "Review code changes and report concrete findings."
system_prompt = "Review the assigned task. Cite files and evidence."
skills = []
max_tool_calls = 40
timeout_seconds = 600
# Optional paired override; omit both to inherit the parent model.
# provider = "anthropic"
# model = "your-model-id"
```

Definitions contain no tool grants, API keys, conversation history or independent memory. Skills are scoped IDs for enabled local skills; loading them still requires a granted `load_skill` tool. Provider/model must be configured together. Overrides resolve credentials and limits from that provider's Settings profile. Default tool-call limit is40 (1–1000), timeout600 seconds (1–3600).

`create_agent` accepts `{scope,definition}` after an explicit user request. Project creation uses ModifyWorkspace risk; Personal uses ModifyOutsideWorkspace. `list_agents({})` returns bounded metadata without system prompts. Definitions are loaded and frozen on invocation; editing a file affects later runs.

## Parent-selected authority

```json
{
  "agent": "personal:reviewer",
  "task": "Review the current change and report issues with evidence.",
  "tools": ["ls", "read_file", "ripgrep"],
  "context_mode": "fork",
  "execution_mode": "parallel"
}
```

Every call explicitly supplies tools, including an empty array for a model-only task. A caller cannot grant a tool absent from its own registry. The child registry is filtered after process/delegation registration, restricting execution as well as model-visible metadata. Granting `call_agent` permits further delegation within the same narrowing rule and a maximum depth of4. Run-management tools must also be granted when a child needs to inspect/wait/cancel delegated work.

Child tools retain the owning chat's workspace and permission-mode snapshot. Full access does not elevate OS permissions. Giving `exec_command` grants general shell capability, including file writes: omit it from read-only workers. Skills/instructions cannot grant tools.

`fork` freezes the parent's effective model context: selected history, existing compaction summary blocks, bounded activity and current pending request/tool continuation. It does not reconstruct original messages already represented by compaction, copy parent system authority, or follow subsequent conversation updates. Screenshot attachments are not copied. `task_only` receives its task without parent conversation/history. Neither mode carries private memory from earlier runs.

Known child model limits are checked before each model request, including repair/tool continuations. A snapshot that does not fit fails explicitly; the protected snapshot is never silently truncated or compacted by replaying tools. Unknown capacities remain unknown.

## Execution and lifecycle

`AgentRunManager` owns independent workers with native opaque `run_id`s. Its atomic limits are4 active runs/chat and16/app, including descendants. It retains at most32 records/chat and128/app, pruning finished records on admission. These are agent limits, separate from command-session limits.

`wait` waits for completion or the configured cooperative deadline. `parallel` returns immediately; desktop workers survive the parent's answer and later chat navigation while the app remains open. Runs do not resume after app restart. The CLI has no persistent app lifetime: parallel workers are cancelled and their command scope closed when its runtime exits.

Tools:

| Tool | Operation |
| --- | --- |
| `call_agent` | Start a frozen definition with task, tools and context/execution modes |
| `list_agent_runs` | Bounded metadata for this chat's retained runs |
| `read_agent_run` | Read state and bounded structured-report output pages |
| `wait_agent_run` | Wait0–10000ms; default1000ms |
| `cancel_agent_run` | Request cancellation; Command risk through the agent tool path |

Statuses are running, waiting_approval, cancelling, completed, failed, cancelled and timed_out; queued is reserved by the view contract. Cancellation/deadlines are cooperative at model/tool boundaries and approval waits. An already executing provider call or tool may finish within its existing timeout before the worker exits. Never treat cancelling as confirmation of stopped work. Failed calls are not automatically replayed. Cancelling a run also cancels its descendants; failed/timed-out runs cancel unfinished descendants. Successful parents may leave parallel children running.

All child commands share their root chat's ProcessScope and app ProcessManager; child creation cannot bypass per-chat/app command limits. Each runtime owns its own command request ID, so its cleanup cannot kill another request's jobs. Child `read_process` requires an explicit cursor or `tail_lines` to preserve the parent's default output cursor. CLI workers retain request-local command semantics and reject background command lifetime.

Successful draft promotion normally keeps the exact native scope identity, preserving children and resource IDs. Deletion, old draft-epoch cleanup and app shutdown cancel owned runs. A draft cannot be released during active requests/runs. Merging different scopes while children are active is rejected before moving process ownership; the request's transfer-failure cleanup stops source background work. Completed reports can be adopted into a saved scope. Model arguments cannot choose another chat owner.

## Approvals and UI

Child approvals are stored against their actual run, exact approval ID and native owner. Desktop renders active child cards inside the assistant conversation, showing agent identity, status and duration; there is no separate Agent runs bar above the composer. Delegation activity activates monitoring; a starting call or unfinished child refreshes once per second while its chat is visible. Completed rows hide automatically, and a running call_agent does not create a duplicate parent terminal. Child approval requests expand their card so Allow/Deny remains accessible. Navigation or unmounting stops refreshes and invalidates stale callbacks. A retry remains available after transient errors. This does not poll OS process output.

Each card shows actual state, task, context/execution modes, model, duration, latest progress, current tool, tool count, bounded tool-history summaries and result. Stop requests cancellation. Allow once/Deny resolve only that run's pending tool approval and preserve focus during updates. Parent stream, usage meter and approval row are unaffected. Foreground CLI waits service pending descendant approvals through the terminal authorizer.

## Final report

The runtime injects `config/agent-result-prompt.md`. A child's ordinary final-answer content must contain exactly this JSON shape:

```json
{
  "outcome": "done",
  "summary": "Reviewed the change; one issue needs attention.",
  "work_done": ["Read implementation and existing tests"],
  "findings": [{"severity": "medium", "location": "src/example.rs:42", "message": "Describe evidence"}],
  "artifacts": [],
  "verification": [{"action": "cargo test", "result": "passed"}],
  "remaining": []
}
```

`outcome` is done, partial or blocked. All arrays are required, limited to64 entries each. Summary is nonempty and at most8KiB; the complete report is at most64KiB. Unknown fields, invalid JSON and image payloads are rejected. Reported findings/checks are model claims; runtime completion confirms worker/protocol completion, not independent proof of those claims.

Runtime supplies run identity, status, errors, elapsed time, tool count and latest-call model usage separately. Child usage never replaces the root chat's context reading. A completed worker can correctly return partial/blocked. Failures, cancellation and timeout do not invent a successful report.

Model-facing listings exclude full tasks, results and approval arguments. Inspection returns `result` directly when it fits, plus `result_output`, UTF-8 byte offsets, total output bytes and `next_cursor`. Continue with `read_agent_run({run_id,output_offset:next_cursor,output_limit:16384})`; output_limit is1–16384 bytes and every response stays within64KiB. Retained native UI records contain the full bounded report. Runs are in-memory; a successful parent turn may retain inspected reports in ordinary bounded Activity, but no independent child conversation is persisted.
