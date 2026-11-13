# Tool execution control and hard/lazy wakeups

Status: **design approved by the user on 2026-11-13; tool-only implementation verified**.
This document records the approved tool-execution architecture. It replaces the
former `execution.mode: job/task` protocol. [Wakeup](wakeup.md) describes the
current protocol. The hard/lazy protocol, unified execution tracking, and bounded durable records
are implemented and verified.
User-message queuing during an active turn is explicitly deferred and is outside
this implementation.

## 1. Main decisions

Every tool invocation is an owned execution managed through one backend entry
point. There is no synchronous tool branch outside execution tracking. Fast tools
may return immediately, but still require a record and result.

Execution-level `job/task` selection has been removed. The agent chooses only a **wake_up
cadence**:

| Mode | While running | On completion, error, or required interaction |
| --- | --- | --- |
| `hard` | Agent-selected interval, at least 0.1 seconds and strictly below 30 seconds | Queue a notification immediately |
| `lazy` | Remind every 10 minutes from the latest scheduling point | Queue immediately, without waiting 10 minutes |

Hard/lazy does not change tool permissions, timeouts, dependency order, or work
lifetime. Lazy does not permit ignoring results or continuing after a final
answer. The agent chooses cadence according to importance, intervention needs,
and expected duration, and can change it without invoking the tool again. The
backend still determines risk and applies the existing approval policy; selecting
lazy cannot reduce permissions.

Defaults live in `config/execution-defaults.toml`, with usage instructions rendered
from the same source. Omitted wake_up defaults to hard every two seconds; lazy
uses 600 seconds.

## 2. One manager, multiple executors

ToolExecutionManager is the logical ExecutionManager entry point. Production
default tools use managed workers; synchronous custom-tool fallbacks are tracked
too. Process Control uses the same listing, state, and controls
for every execution. ProcessManager remains the specialized OS-process
manager underneath it; reading a file or calling MCP does not create an OS process.

```mermaid
flowchart TD
    Agent -->|tool call + wake_up| Runtime[Runtime: validate and authorize]
    Runtime --> Manager[ExecutionManager: admission and ownership]
    Manager --> Local[Local tool worker]
    Manager --> Process[ProcessManager: OS process and descendants]
    Manager --> MCP[MCP adapter: request and connection]
    Manager --> Child[AgentRunManager: child agent]
    Local --> Results[Execution state and bounded result store]
    Process --> Results
    MCP --> Results
    Child --> Results
    Results --> Scheduler[Scheduler and mailbox]
    Scheduler -->|metadata without logs| Gate[Owner agent execution gate]
    Gate --> Agent
    Results --> History[Conversation Activity: references and summaries]
```

The manager holds execution ID, native owner, request ID, tool name, state,
timestamps, revision, wake_up policy, result reference, notification delivery
state, and cancellation request. Resource handles belong to adapters. The
scheduler does not execute shells, call providers, or replace runtime authorization.

Keep `execution_id` as the shared control ID. OS-process `session_id` and child
`run_id` are mapped internally and must not be interchanged. Native scope is the
authority; model arguments cannot choose a chat owner.

## 3. Protocol

`wake_up` is a top-level tool_call field, separate from args:

```json
{
  "type": "tool_call",
  "tool": "exec_command",
  "wake_up": {"mode": "hard", "interval_seconds": 5},
  "args": {"command": "cargo test"}
}
```

```json
{
  "type": "tool_call",
  "tool": "mcp_call_tool",
  "wake_up": {"mode": "lazy"},
  "args": {"server": "configured-server", "tool": "configured-tool", "arguments": {}}
}
```

The MCP example illustrates the envelope only; actual server/tool names must come
from the authorized catalog. Hard requires a finite interval in the valid range;
lazy accepts no custom interval. Reject invalid values rather than silently
converting them to lazy.

Existing `list_executions`, `read_execution`, and `cancel_execution` controls use
the same authority and preserve bounded reads and cancellation permissions. Add:

```json
{
  "type": "set_execution_wakeup",
  "execution_id": "opaque-id",
  "wake_up": {"mode": "hard", "interval_seconds": 2}
}
```

Updating a policy increments schedule generation, invalidates
old timers, never redispatches the tool, and does not delete terminal notifications.
`await_execution` yields agent execution to wait for the mailbox; waiting does not
change an execution's policy without an explicit update.

## 4. Scheduler, mailbox, and agent execution authority

One backend scheduler manages deadlines for many executions. A generation-tagged
deadline heap ignores stale timers. At a hard/lazy deadline, queue metadata and
schedule the next deadline if still running. A terminal state cancels periodic
timers and queues the final notification. Do not poll the model every 30 seconds
while waiting for lazy. If an internal wait is limited to less than 30 seconds,
continue waiting inside the backend without calling the provider until a
deliverable event arrives.

Each agent owner has only one active consumer/model call. Short locks protect
state, mailbox, and turn acquisition; never hold them during I/O, approvals, or
model calls. When the agent is busy, notifications wait until the next safe
boundary; do not insert into a streaming response or start a second agent call.

Notifications contain metadata: execution ID, status, reason, elapsed time,
revision, unread lines, and retention/truncation flags. They contain no full logs
or raw errors. Keep batches at most 16 events and preserve Wakeup coalescing/ack:

- Coalesce periodic notices by execution ID and revision; terminal replaces periodic.
- Error, completion, and interaction-needed notices precede periodic ones; FIFO within priority.
- Events arriving during batch processing remain pending; acknowledge only that batch.
- Provider failures or invalid responses neither lose notifications nor replay tools.
- Terminal delivery is logically once; an unacknowledged delivery may repeat with
  the same identity for idempotent consumption, without creating a new terminal event.

Receiving additional user messages during an active turn and prioritizing those
messages against wakeups is deferred. This tool-only implementation must not claim
that user-message queuing exists. Explicit Stop uses native cancellation authority
rather than depending on an agent reading logs.

## 5. Verification responsibility and final answers

The backend blocks final answers while required work has running or cancelling
executions, or unprocessed terminal notifications. Apply this equally to hard and
lazy. Delivery and acknowledgement alone do not prove success: the agent must
inspect results and enough evidence before making a conclusion, without having
to read every log line.

For example, an agent can do independent work while tests run, but cannot claim a
fix is complete merely because tests started. Failure/cancellation may still lead
to an honest final answer describing unfinished work; never force retries until
success. On provider failure, budget exhaustion, user cancellation, or scope
closure, preserve actual state and apply lifecycle cleanup rather than calling
the model indefinitely beyond those limits.

Only a service explicitly requested by the user as background work may survive
final. That is a separate lifetime policy, never inferred from hard/lazy. The
agent must process startup results and clearly report that the service remains
running. Do not call the model automatically after final; retain background events
for the next authorized turn. Cross-turn autonomous agent activation needs separate
design/authorization and must not be enabled implicitly by lazy.

## 6. Results, conversation history, and restart

Store execution results separately from ordinary chat. Conversation history keeps
the successful turn's user/final-assistant pair; Activity keeps execution references
and bounded summaries. Do not insert every wakeup or all stdout into history.
Execution storage retains actual state/results even for failed turns absent from
successful chat history. Drafts use temporary native owners; promotion preserves
execution identity and links to the conversation without creating history just to
allocate a scope.

The implementation stores durable metadata and bounded UTF-8 results with private file
permissions, a schema version, and atomic publication. Persist a record before
dispatch; publish only successfully stored results. If terminal persistence fails,
retain an uncommitted record and report a recoverable storage error; never turn a
disk-write failure into success or automatically rerun the tool. Never store
credentials, approval tokens, or images in execution artifacts; preserve redaction.

Use existing bounds as the baseline: 64 KiB output/execution, reads at most 4 KiB
and 1–200 lines each; tail reads do not advance cursors. Oversized results require
truncation/retention flags. Do not automatically read every page upon wakeup.
Durable journal and trash collections are independently bounded by configured
record/byte quotas. Active executions and unprocessed results are not silently
evicted. Memory limits of 32 records/owner and 128/backend are separate.

After restart, committed results remain readable. Mark nonterminal executions
interrupted, with potentially unknown action outcomes; never restore handles,
leave them Running forever, or automatically replay them. Interrupted is the implemented recovery
state. History records the past; live state comes from the manager. Reading a retained artifact
after restart grants no control over an old execution/process ID. Chat deletion
uses the same trash/cleanup policy for owned artifacts and stops resources before
scope closure; late workers must not republish into deleted owners.

## 7. Cancellation, limits, and permissions

Cancellation is cooperative. OS processes use existing process groups/Windows
Jobs; MCP or noncancellable I/O may remain Cancelling until deadline. Do not report
cancellation before worker completion. Cancellation does not undo side effects,
and timeouts may have unknown outcomes. Sending Stop alone must not replace the
actual result with Cancelled.

Preserve approval snapshots, tool grants, workspace authority, agent/process
quotas, and frontend native permissions. Every execution passes admission before
launch. Execution and OS-process limits are separate quota layers; a common ID
does not mean one execution consumes multiple OS-process slots.

The UI remains inline in conversation, with no permanent Processes panel or
frontend polling. Native operations stay in atlas-api.ts. Running, Cancelling,
and Interrupted labels must reflect actual state.

## 8. Migration and acceptance criteria

The former baseline used `ExecutionMode::{Job, Task}`, an untracked synchronous
tool branch, and in-memory-only results. The parser now rejects legacy execution
job/task envelopes with a migration error. Use top-level wake_up; task is never
silently mapped to lazy. All tools are tracked even when wake_up is omitted.
Command `lifetime: job/background`, child `execution_mode: wait/parallel`, built-in
conversation tasks, and MCP Tasks remain separate concepts. A parent's active
call_agent execution still prevents an early final answer.

Durable records use per-manager UUID journals under the Atlas data root's
`tool-executions/`, with a lifetime lease per manager. A bounded 500 ms
`import.lock` coordinates cross-process import/publication, skipping live managers
and recovering inactive nonterminal records as interrupted. Native saved-chat
aliases include descendant prefixes; promotion preserves draft identities and
checked deletion moves records to `tool-executions/trash`. No process/agent handle
is restored. Defaults set 128 records and 12582912 bytes (12 MiB), each bound
applied independently to shared journals and trash. Valid storage options are
1–128 records and 4096–67108864 bytes. This does not create user-message queuing.

Sources include `src/executions/persistence.rs`,
`config/execution-defaults.toml`, `tests/execution_protocol.rs`, and
`tests/execution_manager_control.rs`. See [Wakeup](wakeup.md) for operational details.
Verification passed on Linux: 615 Rust tests, 74 native desktop tests, and 336 frontend tests. Root/native Clippy, formatting, frontend/native builds, and the no-key demo passed. The checklist below describes the coverage requirements.

Required implementation verification:

- Every tool, including synchronous and failed calls, has correctly owned records and stored results.
- Hard respects its interval; lazy uses 600 seconds; completion/error wakes early in both.
- Verify lazy with a fake clock, without ten-minute test sleeps.
- Cadence changes invalidate stale timers, preserve terminal events, and avoid duplicate tool/model calls.
- Lazy cannot bypass the final guard; metadata reads do not replace evidence verification.
- Preserve batch/ack, provider-failure, context, and output-budget contracts.
- Persistence failures/restart, unknown outcomes, truncation, quotas, draft promotion,
  deletion, and late publication preserve ownership and never replay actions.
- Background requires user intent; no automatic model calls after final or restart.
- Preserve approvals, command containment, cancellation, and child-scope cleanup.

## Await compatibility

`await_execution` waits for the selected executions' existing hard/lazy schedules
or terminal/interaction events. Its legacy `yield_ms` field is accepted and
validated in the 100–29999 ms range but never schedules an early model wakeup.
Use `set_execution_wakeup` to change cadence. In particular, `lazy` plus
`await_execution` with `yield_ms:100` still waits for the 10-minute reminder or an
earlier completion/error; it does not poll the model every 100 ms.

## Retired child scopes and recovered evidence

Retiring a child scope abandons its private pending notifications for retention,
without changing the recorded action outcome. The parent's managed `call_agent`
execution still reports the child's lifecycle and prevents an early final answer.
Frozen child results remain durable until bounded retention or chat deletion.
After restart, recovered descendant results appear as read-only archived evidence
to the owning root chat; unfinished results generate interrupted notifications
there. They cannot be cancelled, rescheduled, or used to restore live child handles.
Live children keep their separate mailboxes and native ownership.
