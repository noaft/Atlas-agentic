# See the process manager in action

These examples demonstrate command lifetime and native ownership. Command
`lifetime: job/background` is separate from execution wake_up cadence; see the
[hard/lazy architecture](../architecture/tool-execution-control.md). User-message queuing
while a turn is active is outside this implementation.

## 1. Demo without a model or API key

From the repository root:

```bash
cargo run --example process_sessions
```

The example binary supplies its own worker, which only prints ticks; it does not
modify files or call a model. It uses the same backend scope as desktop, without
adding background support to ordinary CLI `ask`. Expected steps:

```text
1. First runtime ended; background <id> is still running: true
2. Independent UI reader sees: tick ...
3. Second runtime inspected same session; resources: 1
4. Another chat cannot read this session: PASS
5. Explicit stop: stopped=true, running=false
PASS: process reused across runtimes, isolated by chat, and cleaned up.
```

Observe that two runtimes use the same session and resource count remains one;
the UI reader does not consume the agent's logs; foreign scopes are rejected;
Stop cleans up the worker. The demo cleans up automatically at the end.

## 2. Try Tauri with your configured model

Restart an old Tauri development session so the backend loads new commands, then
run from the repository root:

```bash
npm --prefix desktop run tauri dev
```

Open a chat, choose a folder, and make an explicit request, for example on
Linux/WSL with Python available:

> Run this command as background work in this chat, without adding `&`, and keep
> it across messages:
> `python3 -u -c "import time; [(print('tick', i, flush=True), time.sleep(1)) for i in range(600)]"`.
> After startup, report its process session ID. Do not start another copy.

Choose Allow in Ask/Workspace. A terminal-style tool card appears under the
in-progress answer while awaiting approval/running, showing tool name, state, and
command. It hides immediately on completion, failure, or denial, even while the
agent is still composing. After final, open **Activity** for output. **Call details**
shows raw information while the tool is active. There is no separate Processes
panel or automatic log polling.

Send the next message after the answer finishes:

> List this chat's processes and read new output from the session just created.
> Do not run the command again.

Session ID and process count stay unchanged. The `read_process` result shows new
ticks in that call's Activity after the answer completes. Switch to another chat
and request its process list: the first chat's process is absent. Return to the
first chat and request “Stop that process with stop_process”; approve if needed
and check confirmed stopped state. Stop does not undo earlier effects.

Models may choose the wrong tool arguments. Inspect Activity to confirm
`lifetime: background` rather than trusting the answer alone. A default job is
cleaned up at turn end. `list_processes`/`read_process` are reads and do not require
rerunning the command.

For native Windows, use an equivalent foreground command available on your
machine. Python is a dependency of this prompt example, not Atlas; the Rust demo
in section 1 needs no Python. Application Control may block native Windows tests;
do not disable policy or infer runtime acceptance from compilation.

## 3. Distinguish job lifetime

In another chat, request a long command with `lifetime: job`, read some output,
and finish the turn. The job is cleaned up at turn end, unlike section 2's explicit
background service. Under the hard/lazy final guard, required work must first reach
an appropriate completed/cancelled state; lazy does not permit leaving a required
job unfinished. Never promise that a job's dev server survives final.

## 4. If a process is missing

- Ask Atlas for `list_processes`, then `read_process` with the existing ID; inspect
  tool-card output or Activity after final.
- The command may have completed or failed: check status, exit code, and output.
- Old records may have been evicted; losing an ID never automatically reruns a command.
- New chat/navigation releases old empty scopes. Failed drafts with messages or
  background work keep an owner so you can return and Stop. **Conversation actions
  → Close draft** cleans up processes and closes the draft without deleting saved
  history. Reload closes old-client draft scopes.
- Deleting a saved conversation cleans its resources; app shutdown cleans the
  manager. Processes are not restored after restart, and allocating a native scope
  does not create conversation history.

Details: [Process sessions](../architecture/process-sessions.md).
