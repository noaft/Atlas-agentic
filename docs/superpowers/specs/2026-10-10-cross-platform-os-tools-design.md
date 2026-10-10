# Cross-Platform OS Tools Design

## Goal

Give Atlas a consistent Linux, macOS, and Windows toolset for searching and
managing workspace files, opening files and applications, and executing shell
commands, while making the user's selected permission mode authoritative in the
shared Rust runtime.

## Scope

This project adds these tools:

- `search_files`
- `copy_file`
- `move_file`
- `delete_file`
- `create_directory`
- `open_file`
- `open_application`

It also makes the existing `exec_command` tool available on Linux, macOS, and
Windows. Clipboard, screenshot, GUI automation, recursive directory deletion,
and long-lived process management are outside this project.

## Permission modes

Atlas exposes three user-selectable modes:

- **Ask**: read-only tools run automatically. Every modifying, launching, or
  command tool requires approval.
- **Workspace**: workspace-scoped creation, non-overwriting copy, and
  non-overwriting move run automatically. Delete, overwrite, application
  launching, and command execution require approval.
- **Full access**: every enabled tool runs without an Atlas confirmation.

Full access does not elevate operating-system privileges. Structured file tools
remain workspace-scoped in every mode. `exec_command` is the intentional
unrestricted escape hatch and runs with the Atlas process account.

The runtime owns policy enforcement. The UI, model, provider response, and tool
prompt cannot downgrade a tool call's required authorization.

## Permission matrix

| Tool or action | Ask | Workspace | Full access |
| --- | --- | --- | --- |
| `current_time`, `pwd`, `ls`, `read_file`, `search_files` | Run | Run | Run |
| `create_file`, `create_directory`, non-overwriting `copy_file` | Confirm | Run | Run |
| Non-overwriting `move_file` | Confirm | Run | Run |
| Delete or any overwrite | Confirm | Confirm | Run |
| `open_file`, `open_application` | Confirm | Confirm | Run |
| `exec_command` | Confirm | Confirm | Run |

Tool metadata describes its capability and an argument-sensitive risk. For
example, `copy_file` and `move_file` are elevated to destructive risk when
`overwrite` is true.

## Runtime authorization

Before executing a tool call, `Runtime` asks a `ToolAuthorizer` for a decision
using an `AuthorizationRequest` containing a unique request ID, tool name,
redacted bounded argument summary, and risk reason. The possible decisions are
allow and deny.

Calls allowed by the selected mode bypass the authorizer. Calls requiring
confirmation pause until the active interface decides. A denial becomes a
normal tool result sent back to the model, allowing it to explain the denial or
choose a safer action. An authorizer transport failure or timeout fails the
turn. Failed and cancelled turns do not persist partial messages.

The desktop implementation keeps pending approvals in Rust and emits typed
`approvalRequested`, `toolStarted`, and `toolFinished` chat events. The frontend
answers through a narrow Tauri command using the opaque approval ID. Closing or
cancelling the active chat rejects its pending approval. The CLI authorizer
prompts on the terminal. Full access never prompts.

Approval displays redact secret-like argument fields and truncate large values.
The user can allow once or deny; changing the persistent permission mode is a
separate action.

## Persistence and authority

Settings stores the default permission mode for new chats. The composer shows a
permission picker beside the model picker. A new-chat draft begins with the
settings default and may override it without changing settings.

The first successful turn stores the draft's permission mode in conversation
metadata. Loading a conversation restores that mode. Changing the picker for a
saved conversation updates only that conversation. Permission selection is
disabled while a turn is active.

For saved conversations, the backend ignores a conflicting mode supplied with
an ask and loads the authoritative value from metadata. New drafts send a mode
which Rust validates and persists only after the first successful turn. Legacy
settings and conversations default to Ask. Settings schema changes remain
backward compatible through serde defaults rather than rejecting existing
version-one files.

## Structured filesystem tools

All structured tools accept relative paths only, reject parent traversal, and
operate beneath the canonical workspace. They defend against symlink escapes
and validate file types. Overwrite is false by default and must be explicit.

- `search_files` searches filenames and UTF-8 file contents. It accepts a
  non-empty query, optional relative root, optional glob, bounded result limit,
  and an include-hidden flag. It skips binary files, symlinks that leave the
  workspace, and common generated directories by default. Results identify the
  relative path, match kind, line number for content matches, and a bounded
  excerpt.
- `create_directory` recursively creates directories and succeeds if the target
  already exists as a directory.
- `copy_file` copies one regular file to another. It creates no implicit parent
  directories and refuses overwrite unless `overwrite: true`.
- `move_file` moves one regular file. It first renames and falls back to
  copy-then-delete for cross-filesystem moves. It refuses overwrite unless
  `overwrite: true`.
- `delete_file` deletes one regular file. Recursive directory deletion is not
  supported.

Each tool returns concise structured text with bounded output and actionable
errors.

## Opening files and applications

`open_file` validates a workspace file and asks the OS to open it with its
default application. `open_application` accepts an explicit application name or
path plus an array of arguments. Arguments are passed directly, never joined
into a shell string.

Platform adapters use the conventional launcher:

- Linux: `xdg-open` for files; direct process spawn for applications.
- macOS: `open` for files; `open -a <application> --args ...` for applications.
- Windows: `cmd.exe /D /S /C start "" <file>` for default file opening and
  direct process spawn for applications, with platform-specific escaping kept
  inside the adapter.

Launch tools report that the OS accepted or rejected the launch. They do not
wait for the opened application to exit.

## Cross-platform command execution

`exec_command` keeps the existing 30-second default timeout, explicit timeout
validation, 64 KiB stdout/stderr limits, starting-directory behavior, and
nonzero-exit reporting.

- Linux and macOS execute through `/bin/sh -c` and retain process-group timeout
  cleanup.
- Windows executes through `cmd.exe /D /S /C` and uses a Windows job object (or
  the narrowest available equivalent) so timeout cleanup terminates descendant
  processes, not only the shell.

OS-specific implementation details stay behind small adapters. The public tool
schema and result format remain platform-neutral.

## UI

The permission picker visually matches the existing model picker and offers
Ask, Workspace, and Full access with short explanations. Full access has a clear
warning treatment but selecting it does not require an additional modal.

Approval cards appear in the assistant stream and show the tool, bounded
argument summary, and reason confirmation is required. They provide Allow once
and Deny buttons and expose an accessible status while waiting. Tool activity
events provide visible running/completed/failed states without persisting
internal tool protocol in conversation history.

Settings adds a default-permission selector. Only this Settings control changes
the default for future new chats.

## Error handling and cancellation

Invalid arguments fail before approval. A denied call is returned to the model
as a denial result. Launcher failures, filesystem races, command failures, and
approval transport failures use distinct actionable errors. Approval IDs are
single-use and scoped to the active turn. Stale, duplicate, or unknown approval
responses are rejected.

If the UI channel closes or the user leaves the active turn, pending approvals
are denied/cancelled and the unfinished turn is not saved. Runtime tool-call
limits continue to apply after denied calls.

## Testing and documentation

Every behavior is developed test-first. Unit and integration coverage includes:

- the full permission matrix and dynamic overwrite risk;
- authoritative saved-conversation modes and legacy Ask defaults;
- approval allow, deny, timeout/cancellation, duplicate response, and no partial
  persistence;
- path traversal, symlink escape, file-type, overwrite, cross-filesystem, UTF-8,
  binary, hidden-file, glob, and result-limit cases;
- platform adapter command construction under conditional compilation;
- desktop selector state, Settings default, approval cards, and IPC contracts;
- CLI confirmation behavior without real model services.

README and AGENTS documentation will describe the new tools, permission modes,
platform behavior, and the fact that Full access executes with the user's OS
authority.
