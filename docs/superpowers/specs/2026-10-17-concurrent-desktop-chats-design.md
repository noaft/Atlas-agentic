# Concurrent desktop chats

## Intent

Users must be free to navigate Atlas while an agent works. The user explicitly
confirmed that separate chats must also accept requests at the same time.
Success means a request continues in its own chat when the user opens Settings,
starts a new chat, or visits another conversation, without mixing messages,
approvals, workspaces, errors, or completion results between chats.

## Current behavior and cause

The desktop already runs each `ask` in `spawn_blocking`. The agent and its tools
execute sequentially within that worker; the model call does not occupy the
frontend JavaScript thread.

The frontend nevertheless has one set of messages, one status, one pending
approval, and a global `askToken`. Both `ChatStore` and the view prohibit most
navigation during streaming. Leaving during an approval cancels all pending
approvals. Simply removing the disabled attributes would let late callbacks
update a different conversation or disappear after token invalidation.

Streaming also reparses the entire assistant answer on every delta, and tool
events rebuild large portions of the view. Background execution alone does not
address that frontend work.

## Approaches considered

1. Remove navigation restrictions while retaining one global request. This is
   smaller but does not meet the confirmed requirement for concurrent chats.
2. Keep request state per frontend chat and reuse the existing independent
   backend workers. Recommended: it fits the current IPC channel per request
   and preserves the existing persistence contract.
3. Build a durable backend job service. This could survive window reloads, but
   adds job persistence, reconnection, and lifecycle interfaces beyond this task.

## Chat ownership and lifecycle

Introduce a frontend chat session with a stable local identity independent of
the persisted conversation ID. It owns messages, draft text, workspace,
permission mode, request status, errors, live activity, and pending approval.
Global state owns saved conversation metadata, Settings, model discovery, and
the selected session/screen. The existing view-facing active-chat fields may be
derived from the selected session to limit unrelated changes.

Each request captures its owning session and a request generation. Every IPC
event and completion updates that owner, regardless of the visible screen.
Late callbacks from an older request cannot update a newer request. Selecting a
saved chat reuses its live session before loading persisted history. A pending
load cannot overwrite newer navigation or live request state.

New chat remains an unsaved frontend draft. A first request gets an in-memory
sidebar entry while working, with a provisional label from its input. On success
the same session gains the backend conversation ID, and its sidebar entry is
replaced by the saved conversation without duplicates. First-request failure
keeps a retryable local session; it creates no conversation on disk. Retain
unsent drafts when switching between sessions during this app lifetime.

Allow one request per chat at a time and concurrent requests in different chats.
The backend must reject overlapping requests for the same saved conversation,
with a guard released on success and failure. Do not hold conversation-store
locks during model calls or while awaiting approval.

## User interaction

New chat, conversation selection, Settings, typing, scrolling, and fullscreen
remain usable during agent work. A running chat shows Working in the sidebar;
an approval shows Needs approval. A failed background request remains visible
as failed, with its error and retry draft available when selected. Completion
updates history without forcing navigation or stealing focus.

Sending again, deleting, changing the workspace, or changing tool-access mode
for the same running chat remains unavailable until its request finishes.
These operations must not prevent use of another chat. Renaming may remain
unavailable for the running chat to avoid racing first-turn title generation.

Opening Settings during a request does not interrupt it. Saving global Settings
affects later requests; a running worker retains the settings and starting
folder resolved at its start. Late settings responses must not replace another
session's workspace, draft, or messages. Preserve Settings input focus and edits
while background events arrive.

## Approvals

An approval belongs to its request's session, using the existing unique approval
ID. Navigation never calls global `cancelToolApprovals`. The owning chat shows
the approval card, and its sidebar entry indicates that attention is needed.
Switching back restores the same approval. Approving or denying one request
must not resolve another. Keep the existing two-minute timeout; expiry and
request failure clear only the owner's pending approval.

The task does not add tool privileges or change shared-desktop targeting:
computer tools still act on the active OS application at execution time under
the existing permission rules.

## Rendering and native work

Coalesce visible streaming renders to animation frames. Update the active
answer and activity without rebuilding unrelated navigation or Settings.
Hidden text deltas update their session without rerendering the visible chat;
sidebar status changes are published when status actually changes. Preserve
user scroll position unless already following the bottom of the chat.

Move blocking store/settings command work off the Tauri event thread where it
can delay navigation. Keep all frontend native calls in `atlas-api.ts`, retain
the narrow Tauri capabilities, and reuse per-request IPC channels.

## Scope limits

Requests live for the current desktop app process. Reload/restart recovery,
durable jobs, stop/cancel execution, multi-agent routing, and new providers are
outside this change. The CLI's execution model stays as it is.

## Verification

Use fake APIs and deferred promises to prove two requests can overlap, switch
back with live messages intact, complete in either order, and fail independently.
Cover first-turn draft-to-saved identity, background approval ownership and
expiry, drafts retained across navigation, Settings during streaming, stale
loads, and a second send rejected for the same chat.

View tests must cover enabled navigation, sidebar work/approval/error states,
focus preservation, bounded streaming renders, Settings inputs unaffected by
background deltas, and scrolling while text streams. Bridge tests must prove
same-conversation exclusion, separate-conversation concurrency, guard cleanup,
and independent approvals without using real providers or keys.

Run core and desktop formatting/tests, frontend tests/build, relevant Clippy
checks, and the Tauri debug build. Update README.md and AGENTS.md to explain the
new lifetime and concurrency behavior. Commit implementation as a focused change.
