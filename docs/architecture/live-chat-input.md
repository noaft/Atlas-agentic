# Live chat input and stopping responses

Status: implemented for desktop chat. Tool wakeup policy remains as described in
[Tool execution control](tool-execution-control.md).

## User behavior

The composer stays editable while Atlas is working. Sending while the agent is
waiting for a tool, approval, or another runtime boundary supplements the active
turn immediately. Sending during model inference puts the message in that chat's
FIFO queue, displayed above the input. On the next native idle-model event,
queued messages supplement the active turn automatically, without waiting for
its final answer. If that turn has already sealed, unapplied messages fall back
to the next turn. There is no send-mode selector.

Each queued message has a **Steer** action. It removes only that message from the
queue and submits it to the active turn. If inference is in flight, Steer aborts
that HTTP request and regenerates using the accepted instruction; it does not
wait for the old answer to finish. Existing managed tools keep running so the
agent can inspect or cancel them according to the new instruction. Completed
side effects are preserved and never replayed just because generation restarted.
Other queued messages and the current composer draft are unchanged. If the turn
is stopping or cannot accept steering, the message stays queued.

Send and Stop share one primary button: while a turn is active, an empty composer
shows Stop; typing switches the same button to Queue message during inference,
or Add to active turn while waiting. Enter sends text and never implicitly stops
a response. Composition input remains protected.

Queue entries can be removed before dispatch. Stop and request errors pause the
queue; Resume explicitly starts it again. Navigating to another chat or Settings
neither moves messages between chats nor prevents a normally completing chat from
advancing its queue. Pending queues are session-local memory, limited to 16
messages per chat; reload/restart does not restore them.

**Stop** freezes the currently displayed answer immediately and requests native
cancellation. The UI remains in Stopping until the request worker exits. Inference I/O is interrupted; an already running tool may still finish within
its own bounds before cancellation is observed.
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
Channel data uses `requestId` and `responseEpoch` in camelCase. A shared JSON
fixture verifies native serialization and frontend delivery together, including
buffered input before request identity arrives and the Stop snapshot epoch.
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

## Interruptible inference and phase reporting

Runtime emits request-scoped `modelState` events around each inference call. The
native bridge forwards them to the owning chat; the frontend initially assumes
inference is busy until it receives a phase event. Tool rows are not used to infer
whether the model is generating. Events from stale requests or other chats cannot
change routing.

`Agent::process_request_with_control` and `ModelProvider::stream_with_control`
carry an interruption predicate. LocalAgent forwards it to every supported
provider. Pending user input and explicit Stop interrupt inference; only Stop
cancels the whole turn. On steering, runtime drains accepted input, resets obsolete
answer text, preserves tool continuations, and starts one fresh model request.
It does not treat interruption as malformed output or dispatch a stale action.

Production inference uses an async HTTP request driven by a current-thread Tokio
runtime on the existing worker. A control check at most 25 ms apart can drop a
pending send or response-body future, including before headers and during a
silent stream. The transport exits before the next inference starts; there is no
detached reader or overlapping local inference. Redirects and automatic retries
are disabled. Native providers retain their 120-second request timeout; DeepSeek
retains its previous no-total-timeout setting. SSE frames remain bounded to
64 MiB and preserve chunked UTF-8, CRLF, multiline data and final EOF frames.
Legacy custom trait implementations retain their existing cooperative fallback;
they must implement the controlled entrypoint for interruptible I/O.

Closing a local HTTP request cannot guarantee when a remote server stops its
computation. Usage for an aborted request may be unavailable; no usage is invented.

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
As soon as native acceptance arrives, the frontend clears the superseded answer
and places the active reply below the accepted user instruction, so new decisions
appear there during the same turn. Duplicate acceptance events do not clear new
text. The frontend reconciles to the same persisted order when a turn finishes. Activity points
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
