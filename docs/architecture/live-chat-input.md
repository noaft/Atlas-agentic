# Live chat input and stopping responses

Status: implemented for desktop chat. Tool wakeup policy remains as described in
[Tool execution control](tool-execution-control.md).

## User behavior

The composer stays editable while Atlas is working. A busy chat offers two send
modes:

- **Wait until finished** puts the message in that chat's FIFO queue. After a
  normal completion, Atlas starts the next queued message as a new turn.
- **Supplement active turn** delivers a subsequent user instruction to the active turn.
  Atlas applies it at the next safe runtime boundary and reconsiders any pending
  answer or proposed action. It preserves work already completed.

Queue entries can be removed before dispatch. Stop and request errors pause the
queue; Resume explicitly starts it again. Navigating to another chat or Settings
neither moves messages between chats nor prevents a normally completing chat from
advancing its queue. Pending queues are session-local memory, limited to 16
messages per chat; reload/restart does not restore them.

**Stop** freezes the currently displayed answer immediately and requests native
cancellation. The UI remains in Stopping until the request worker exits. Existing
provider/tool I/O may finish within its own bounds before cancellation is observed.
Stop cannot undo a completed side effect. The retained answer is marked Stopped,
including after reopening the conversation; an empty stopped answer is valid.

## Native ownership and inbox

The app owns a shared `TurnManager`. Each active native process/chat scope has
one request token and one `TurnControl`; a lease removes it on exit. Desktop
commands resolve the window, client epoch, and saved/draft process context before
accepting input or Stop. A request token alone grants no access to another chat.

`requestStarted` publishes the active token. The adapter exposes
`submit_chat_input(client, context, requestId, inputId, content)` and
`stop_chat_request(client, context, requestId, responseEpoch?, displayedBytes?)`.
All frontend calls go through `atlas-api.ts`; there are no new frontend OS
permissions. Active input is limited to 32 messages, 64 KiB per message and
256 KiB total per request. Repeating an input ID with identical bytes is
idempotent; reusing it for different text fails.

A receipt identifies acceptance and whether the consumer is finished. A receipt
transport error has an unknown outcome: acceptance events and the final result's
`additionalInputs` reconcile the same ID without duplicating it. Messages that
were not applied stay visible for recovery; stopped/failed turns leave recovered
messages paused. The final seal is atomic with inbox acceptance, so a late input
cannot silently disappear into a completed turn or another request.

## Safe runtime boundaries

The runtime is the inbox's single consumer. It drains accepted input before a
model call, after the response/context preparation, after authorization (including
an interrupted approval), and immediately before executing an action. A new
instruction supersedes a stale proposed action; it does not replay earlier tools.
Desktop approval waits withdraw the current approval when input or Stop arrives.
Other chats' approvals remain intact.

An execution wait is interruptible by pending input. The wakeup scheduler still
owns tool deadlines and notifications; submitting input does not fabricate tool
completion or start parallel inference. The continuation carries the original
request and ordered typed user messages with shared guidance from
`config/live-input-prompt.md`. Skills, model budgets, tool authorization and native
execution ownership retain their existing authority.

## Stop and the displayed prefix

Frontend Stop captures the response epoch and the UTF-8 byte length currently
shown before awaiting native context resolution. Native output records sent final
text by epoch, resetting on `responseReset`. The first Stop cursor wins; late
chunks cannot extend the retained response. The backend resolves the cursor
against its own text and validates the epoch, byte range and UTF-8 boundary. It
never trusts client-supplied answer text.

This transient display buffer is bounded to 64 epochs and 16 MiB. An unavailable
or invalid prefix fails closed rather than saving a different answer. Generic
channel failure, window/client teardown or scope deletion invalidates the turn;
these paths do not persist a stopped response, even if Stop was previously
requested. They are distinct from explicit user Stop.

## History and context

On success, storage atomically writes the original User message, applied
supplemental User messages in receipt order, and the final Assistant message.
The frontend reconciles to that same order when a turn finishes. Activity points
to the final assistant index; tool events and wakeup logs stay out of ordinary
message history.

Explicit user Stop is the exception to success-only persistence: it stores the
same applied-input sequence and the authoritative partial Assistant answer with
an `interrupted` Activity marker. It skips title inference. Unapplied inputs are
not silently stored as instructions the agent read. Provider failures and generic
teardown still save no ordinary turn.

Future context and compaction explicitly label interrupted assistant text as
partial, including empty answers and legacy history paths. History selection
keeps the original request with its supplements, and compaction groups complete
exchanges through the assistant boundary. These context annotations operate on
clones; they do not rewrite stored messages or invalidate source fingerprints.

## Verification

Behavior tests cover single-consumer delivery, idempotence, bounds, final-seal
races, interruptible execution waits, stale-action suppression and approval
withdrawal. Native tests cover owner/request isolation, explicit Stop versus
teardown, and display-prefix forwarding. Frontend tests cover FIFO pause/resume,
receipt races, navigation, delayed callbacks, stopped prefixes and matching
local/reloaded history order. Tests use fake model responses and require no model
credentials.
