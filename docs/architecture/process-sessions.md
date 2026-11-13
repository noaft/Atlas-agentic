# Process sessions in Atlas

Status: desktop manager with per-chat ownership implemented on 2026-11-09.
See [Tool/resource lifecycle](tool-resource-lifecycle.md) for decisions and ownership,
and the [process manager walkthrough](../guides/process-manager-demo.md) to try it.

The architecture approved on 2026-11-13 is [Tool execution control and hard/lazy](tool-execution-control.md).
Every tool receives a common execution record; ProcessManager continues to manage
OS processes underneath. The replacement is implemented and verified. Command
`lifetime: job/background` below is separate from hard/lazy wake_up cadence.

## Three different concepts

- A **chat session** is a conversation/frontend draft.
- A **process session** is one command invocation, potentially a shell and multiple OS descendants.
- A **runtime request** is one agent turn containing multiple model/tool calls.

The OS manages actual processes. Atlas holds handles, owner scope, state, timeout,
and bounded output; the model uses a UUID `session_id`, never a PID for authority.

Desktop keeps ProcessManager with the backend app and a scope per chat. New
runtimes receive the current chat's scope handle. CLI resources remain request-local;
there is no daemon or terminal persisting across CLI invocations.

## Job and background command lifetimes

| Property | Meaning |
| --- | --- |
| `kind: process` | Resource type; currently process |
| `lifetime: job` | Default; cleanup when the owning turn completes, fails, or is cancelled |
| `lifetime: background` | Only on explicit user request; desktop retains it across turns in the same chat |

Background still has a timeout and is not an unlimited service. Run programs in
the foreground; never use `&` to detach them from management. When the shell exits,
remaining descendants in its process group/Windows Job are cleaned up even if their
output was redirected.

Draft scopes are allocated in memory without creating history. The first successful
saved answer binds the scope to the conversation while preserving session IDs.
A failed draft with background work remains inspectable/stoppable; New chat does
not adopt its resources. Close draft cleans its scope and discards the draft
without creating/deleting saved history. Empty abandoned drafts are released on
New chat/navigation. Webview reload closes old-client draft scopes; saved chats can
reattach to background work while the backend remains alive.

## Tool API

| Tool | Main arguments | Risk |
| --- | --- | --- |
| `exec_command` | `command`; optional `yield_time_ms`, `timeout_ms`, `lifetime` | Command |
| `list_processes` | Empty object | ReadOnly |
| `read_process` | `session_id`; optional `wait_ms`, `cursor` or `tail_lines` | ReadOnly |
| `stop_process` | `session_id` | Command |

Ask/Workspace require confirmation for starts and tool Stop. Full access skips
Atlas confirmation without elevating OS permissions. The native Stop API also
checks scope ownership; the current UI uses agent `stop_process` calls, with no
separate Stop button.

The schema/catalog exposes short scope-status metadata so the agent recognizes
existing resources. Do not preload commands or raw stdout/stderr. Before reusing
or duplicating a server, inspect cwd/state through the listing. Never automatically
replay or restart failed commands.

Desktop also manages synchronous commands: without `yield_time_ms`, it waits for
the result as before, but the process is tracked and counts against limits. With
yield, it returns a session if unfinished. Background without explicit yield
defaults to 1000 ms. Synchronous CLI behavior remains unchanged; CLI rejects background.

Argument examples, separate from the tool-call envelope/wake_up policy:

```json
{"command":"cargo test","yield_time_ms":1000,"lifetime":"job"}
```

```json
{"command":"npm run dev","yield_time_ms":1000,"lifetime":"background"}
```

```json
{"session_id":"<returned session ID>","wait_ms":1000}
```

```json
{"session_id":"<returned session ID>"}
```

The last two are `read_process` and `stop_process` respectively. Use the actual
returned ID; never guess another chat's PID/session. Scope/ID errors never restart commands.

## Output and cursors

Stdout/stderr use ring buffers with absolute increasing byte offsets. The agent's
default reader consumes only new output and retains its cursor across chat turns.
Native APIs support explicit cursors for independent readers. The current UI only
shows inline tool results and has no separate polling viewer. Optional cursor:

```json
{"stdout":0,"stderr":0}
```

To inspect the last retained lines:

```json
{"session_id":"<returned ID>","tail_lines":50,"wait_ms":0}
```

`tail_lines` accepts 1–1000 and selects up to N lines **separately for stdout and
stderr**, without merging stream order. It is exclusive with `cursor` and never
advances default/UI cursors. `wait_ms` remains 0–10000: return immediately when
readable output exists; otherwise wait for output or completion until deadline.
LF ends a line; a trailing LF does not invent an empty line, an unfinished line
counts, and CRLF is preserved. Tail sees only the retained 64 KiB per stream and
may begin mid-line.

Each result includes `stdout_view` / `stderr_view`: `start_byte`, `end_byte`
(exclusive), `total_bytes`, `start_line` / `end_line` (null for no text),
`total_lines`, `returned_lines`, `lines_after`, `starts_mid_line`,
`older_output_dropped`, and `has_more`. Absolute line numbers are 1-based and count
LFs already evicted. `has_more` describes unread bytes at snapshot time, including
incomplete UTF-8, rather than predicting future output. `older_output_dropped`
distinguishes ring overflow from intentional tail selection; `stdout_truncated` /
`stderr_truncated` continue to mean requested-cursor gaps. Positions describe the
read snapshot, not a percentage of work complete.

Terminal cards and Activity display these byte/line ranges, tail selection, and
warnings. If output exceeds Activity budgets, preserve valid JSON and metadata,
shorten text toward its end, and mark `display_truncated`. Ranges still describe
the entire captured output rather than just the preview. Turn-wide budgets may
still omit/shorten details as before. There is no separate process panel or polling.

Explicit-cursor reads do not advance the default cursor. Results provide
`next_cursor` for continuation and `oldest_cursor` for retained bounds. If the
buffer has overtaken a cursor, `stdout_truncated`/`stderr_truncated` report loss.
Reading logs never reruns a command; an empty read may still describe a running process.

| Result field | Meaning |
| --- | --- |
| `session_id`, `running` | Selected resource and whether its worker is running/cleaning up |
| `stdout`, `stderr` | Bounded output selected by the reader cursor |
| `next_cursor`, `oldest_cursor` | Next offsets and oldest retained offsets |
| `tail_lines` | Requested tail count; null for ordinary reads |
| `stdout_view`, `stderr_view` | Output positions and retention state at read time |
| `stdout_truncated`, `stderr_truncated` | Cursor gap; do not claim the log is complete |
| `exit_code` | Exit code if available; null does not prove success |
| `timed_out`, `stopped` | Worker-confirmed timeout or stop |
| `stop_requested` | Stop requested, distinct from confirmed stopped |
| `error` | Worker/spawn error if present |

Stop may return `running: true` before cleanup confirmation; inspect state again.
Stopping/timeouts do not undo command side effects. Incomplete UTF-8 tails wait
for remaining bytes; invalid bytes use replacement characters. Inline terminal
cards show tool-event data in a scrollable region. Activity retains bounded output
under existing policy. There is no separate manager-output stream or unlimited
log archive replay.

## Limits

| Item | Value |
| --- | --- |
| Active / chat scope | Default 8; configurable 1–32 |
| Active / backend manager | Default 32; configurable 1–128 |
| Records / chat scope | 32 |
| Records / backend manager | 128 |
| Buffer / stdout or stderr | 64 KiB bytes; overflow drops oldest bytes |
| Yield | 0–10000 ms |
| Read wait | 0–10000 ms; default 1000 ms |
| Default synchronous command | 30 seconds |
| Job session | Default/maximum 10 minutes |
| Background | Default 1 hour; maximum 24 hours |

Set both limits in **Settings → Permissions → Concurrent commands**, then **Save
changes**. Version 2 settings store `processes:{perChat,total}`; older files lacking
this field use 8/32. Both caps apply, so the app cap may be smaller than a chat cap.
Admission rejects new commands before launch rather than queuing them. Lower caps
let existing commands drain, including during draft promotion; reads and Stop
remain available. Raising caps permits starts immediately after Save.
CLI also applies the saved policy: synchronous commands reserve a slot in the
same counters as yielded sessions, preserving synchronous output/timeouts.
These are command limits, not permission to parallelize ordinary tool calls.

Old completed records are evicted when space is needed; IDs may expire after
completion. Session limits do not bound total OS descendants or CPU/RAM consumption.

## Cleanup and scope

Jobs are cleaned only by their owning request. Background ends on Stop, timeout,
chat deletion, draft closure, or normal app shutdown; chat/Settings navigation does
not stop it. Processes retain spawn cwd. Workspace/permission changes do not change
cwd or grant new approvals to existing resources; later operations use current policy.

Shutdown cleanup runs on workers with deadlines, never joining on the Tauri event
thread. Unix signals groups before reaping leaders and uses WNOWAIT against PID
reuse. Windows creates suspended processes inside kill-on-close Job Objects before
resuming them. Cleanup cannot be guaranteed for every crash/SIGKILL/power loss or
commands deliberately escaping containment. No frontend shell/filesystem/process
permissions are added.

Process handles are never persisted, sessions are never restored, and commands
are never automatically replayed after restart. Durable execution-result artifacts
under hard/lazy execution control do not restore process handles. There is no stdin/PTY or
interactive terminal accepting successive commands. Successful Activity may retain
sanitized bounded details; it is neither a live process nor a complete log archive.

## Source map

- [Manager/scope](../../src/tools/process_sessions/manager.rs),
  [output/cursor](../../src/tools/process_sessions/output.rs),
  [tool handlers](../../src/tools/process_sessions.rs).
- [Native ownership/IPC](../../desktop/src-tauri/src/process_commands.rs),
  [UI](../../desktop/src/chat-view.ts), [API adapter](../../desktop/src/atlas-api.ts).
- [Local demo](../../examples/process_sessions.rs) and
  [scope/process tests](../../tests/chat_process_behavior.rs).
