# Shared task registry and slash menu

> Interaction correction approved after implementation: selecting a task by
> clicking its menu row or pressing Enter runs that task immediately. Rows explain
> the action and progress/results remain visible in the owning chat. Menu selection
> passes the descriptor directly, without inserting or parsing a draft command.
> Send always sends literal text to the agent, including `/compact`, bare `/`, and
> unknown slash text. Escape/outside click/Tab dismiss without running a task;
> Enter after dismissal sends the text normally. This supersedes the original
> insert-then-run, Tab selection, and text-command interception requirements below.
> Backend registry/validation, ownership, cancellation, and automatic compaction
> requirements remain in effect. The original implementation plan records the
> earlier behavior; this correction is a bounded update approved in chat.

## Intent and scope

The user selected a shared task registry and slash menu first, leaving automatic
model routing for later. Typing `/` should help users discover and choose a task;
new tasks should not require another hard-coded composer branch. `/compact` is
the first real registered task. Do not add placeholder commands or a model-based
router in this change.

The existing desktop special-cases `/compact` in `ChatStore.send()`. Compaction
already has a shared Rust implementation used by manual and automatic paths.
Replace the frontend command-specific decision with registry-based dispatch while
preserving its progress, cancellation, memory state, and per-chat ownership.

## Authoritative registry

Add a small provider-independent Rust task registry, with a typed built-in handler
binding. A descriptor contains:

- Stable task ID: `conversation.compact`.
- Slash name: `compact`, displayed as `/compact`.
- Title and plain-language description.
- Argument schema: an empty object for compact; additional arguments rejected.
- Preconditions: a saved conversation and model configuration for compact.
- Whether cancellation is supported.

Descriptions describe when a task is appropriate, making this catalog useful for
future model routing. This release does not send the catalog to a model or let a
model invoke tasks. Keep metadata, schema validation, and handler binding together
in the registry; the frontend must not maintain a second list of task names.
Reject duplicate IDs/slash names at registration. Descriptors expose no executable
paths or arbitrary code. A frontend-supplied ID is always checked by Rust.

Built-in registration in a dedicated `src/tasks/` module is sufficient. User-defined
task loading, plugins, aliases, and arbitrary shell handlers are out of scope.
Adding a future built-in requires one descriptor, validated argument handling,
a handler, and its tests; the generic menu discovers its descriptor automatically.

## Dispatch

Expose a read-only `list_tasks` Tauri command, available without an API key, and a
`run_task` command with a task ID, validated arguments, saved conversation ID when
needed, and an event channel. Desktop access stays inside `atlas-api.ts`.
Return a typed result tagged with task ID and outcome kind; compact wraps the
existing compaction result. Keep task failures distinct from ordinary chat turns.

The backend resolves the ID, validates arguments and preconditions, obtains the
existing per-conversation request ownership, and calls the registered handler.
It derives workspace authority from saved metadata and retains existing provider,
permission, execution-lease, and cancellation behavior. Do not create messages,
titles, or an empty saved chat when running a task. Failure preserves the draft
and previously saved memory, with a retryable error in the owning chat.

Move the existing CLI compact entry through the shared task dispatch where it
already has an explicit ID/command. Preserve `atlas conversation compact <id>`.
Do not add a second CLI command grammar in this change.

Automatic 90% compaction continues to call the same underlying compaction handler
inside its current runtime context. It must not invoke Tauri, redispatch through
the frontend, start a nested ask, or reacquire a lease already held by that turn.
Use the same stable task ID in task metadata/progress where appropriate, while
retaining the existing compaction events and callback signatures unless a typed
wrapper is required at the dispatch boundary.

## Composer behavior

Open a compact menu above the composer when `/` begins the draft's first token
and the cursor is in that token. Filter case-insensitively by command name, title,
and description as the user types. Only registered tasks appear.

Match the existing UI: rounded surface, soft shadow, lavender active/hover row,
a command/title line and short description, bounded height and scrolling. Keep
the popup inside the viewport. The textarea keeps typing focus, with accessible
listbox/active-descendant semantics and an announced results count.

- Up/Down moves the active option; Home/End selects the first/last result.
- Enter or Tab selects an option and inserts its canonical command. Selection
  closes the menu and does not execute the task. A subsequent Enter or Send runs
  the command. Prevent reopening for an unchanged accepted/dismissed draft.
- Pointer selection has the same insert-only behavior and returns typing focus.
- Escape dismisses the menu without clearing the draft. Outside click closes it.
- Shift+Enter and IME composition preserve normal text-entry behavior.
- Disabled tasks remain discoverable with an explanation, such as “Start a saved
  conversation first” or “Configure a model first,” and cannot be executed.

A fully typed registered command can run with Enter after dismissing the menu or
with Send. Bare `/`, unknown commands, and invalid arguments never reach the chat
model: explain the issue, preserve the draft, and provide matching task suggestions.
Unknown input must never select and execute a loosely matching task automatically.

Recognize commands only at the start of the draft, using a single slash-name token
with command-name characters. Ordinary prose containing `/compact`, quoted/code
examples, URLs, and multi-segment absolute paths such as `/home/user/file.rs` remain
normal messages. The parser rejects trailing arguments for compact rather than
silently ignoring them. UI autocomplete and backend dispatch both validate the
command/arguments; frontend validation is not execution authority. Command
matching is case-insensitive; insertion uses the canonical registered name.

## Catalog and session state

Load the catalog independently of model discovery during initialization. Catalog
failure cannot block normal chat: show a menu error/retry state and preserve input.
Do not silently fall back to a duplicated hard-coded command catalog. A retry
refreshes the catalog without losing drafts or active selection.

Keep catalog data global, but draft query, menu selection/dismissal, task progress,
errors, and request ownership tied to the correct chat/composer. Switching chats
must close the old popup and rebuild suggestions from the new draft; a background
task must not insert text or change the foreground chat. Busy rules remain per
conversation, allowing other chats to send messages.

Command execution goes through one frontend task dispatcher rather than another
`if (prompt === '/compact')` branch. Adapt the typed compact outcome to existing
memory/progress state through an explicit result handler. Future task outcomes
can add handlers without changing slash parsing or catalog discovery.

## Verification

Rust tests cover descriptor discovery without model setup, duplicate registration,
unknown IDs, invalid arguments, saved-chat requirements, and compact dispatch to
the existing service. Verify one lease/operation per conversation and no nested
lease on automatic compaction. Preserve CLI parsing and compact behavior.

Desktop tests cover `/` discovery, filtering, keyboard and pointer selection,
insert-only Enter/Tab, subsequent execution, Escape/outside dismissal, empty and
unknown commands, invalid compact arguments, IME/Shift+Enter, ordinary slash text
and paths, disabled explanations, catalog failure/retry, and no extra chat history.
Test background execution, chat switching, cancellation, and the existing memory
viewer/usage behavior after registry dispatch.

Use fake providers and loopback services. Run formatting, all Rust/native/frontend
tests, both Clippy commands, frontend build, and debug Tauri build before completing
implementation. Document how future built-in tasks register and how users select
a slash command. Create one focused implementation commit and use the established
local merge workflow after verification.

## Review gate

This document records the selected scope. Review and approve it before writing
the implementation plan. Automatic model routing remains a separate future task.
