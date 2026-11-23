# Conversation, memory, and progress contracts

Implemented contracts for `src/conversations/`, diagnostics, and runtime protocol
repair. For active-turn input and explicit Stop, also read [Live chat input](live-chat-input.md).

## Diagnostics

Backend diagnostics initialize at CLI/desktop startup and write JSON lines to
`logs/atlas.jsonl` under the same data root (`ATLAS_DATA_DIR` applies).
`ATLAS_LOG_DIR` overrides only the log directory; `ATLAS_LOG_STDERR=1` also emits
entries to stderr. Keep files bounded at 5 MiB with three backups, private Unix
creation modes, and process-safe rotation. Logging failures must not block app
use; bound log-lock acquisition and omit response-derived parser-error values.
Correlate worker operations and nested model calls with a request ID; log
startup, operation outcomes/durations, provider/model metadata, and tool
names/outcomes. Never log prompts, response bodies, tool arguments/output,
screenshots, or credentials. Register saved credentials for redaction before
provider setup; omit endpoint URLs from diagnostics. Failed requests belong in
these diagnostic logs even though failed turns remain absent from chat history.

## Storage and turn persistence

Conversation storage is resolved using a platform directory library: Linux/WSL
uses `$XDG_DATA_HOME/atlas` (normally `~/.local/share/atlas`), macOS uses
`~/Library/Application Support/atlas`, and Windows uses `%LOCALAPPDATA%\atlas`.
`ATLAS_DATA_DIR` overrides the root for tests and development. The root contains
`conversations.json`, `conversations.lock`, `conversations/<uuid>.jsonl`, optional
`activities/<uuid>.jsonl` sidecars, and `trash/`. Delete moves history and any
activity and memory sidecars to `trash/` and reports the history recovery location; recovery
is manual, with no CLI restore command.

`ConversationService` coordinates the store and runtime. Successful ordinary
history stores the original User, ordered applied supplemental Users, and final
Assistant atomically. Explicit user Stop additionally saves the authoritative
displayed partial answer with an interrupted Activity marker and skips title
inference. Generic failure, channel loss, and scope teardown still save no turn.
See [Live chat input](live-chat-input.md).

## Progress messages

Ordinary `tool_call` responses may include an optional top-level `message` string
of 1–1000 Unicode characters after trimming. Runtime-injected protocol instructions
explain this field; the agent prompt guides when to provide next-step updates.
Place it after `type`, before `tool` and `args`, for early streaming. No separate
progress tool or model call is needed. Runtime emits transient cumulative
`progressPreview` events and clears them on every model-call exit. Never execute
or persist previews, expose ordinary arguments as commentary, or add message text
to diagnostic logs. Only complete protocol-valid messages on recognized tool
requests with a valid authorization context emit committed `Progress` events,
before any required user approval or execution. These messages express intent,
not approval or successful completion. Existing tool calls without a message stay
valid. Keep preview text separate from the last committed update in each owning
session; rejection restores that update, and streamed/final answer text replaces
it. Ignore late callbacks. Save committed updates as activity only on successful
turns, include them in bounded future context/compaction as progress rather than
completed work, and never create separate ordinary chat messages. Preserve legacy
saved progress records. The CLI sends committed updates to stderr.

## Activity and compaction

Save sanitized, bounded runtime status and tool activity from successful or
explicitly stopped turns in the separate sidecar; other failed turns save neither. Never persist screenshot pixels, image data,
or tool attachments. New activity batches also store elapsed runtime/model work
duration; legacy batches without it remain valid. Raw activity details remain
local, while later model turns receive only a separately capped summary from the
same uncompacted turns selected for message history. Keep all original history
on disk. The bundled agent enables `[memory]`: full history before compaction,
then one model message per persisted summary block followed by original messages
from the checkpoint's exclusive boundary onward. Never silently apply the old
40-message cap in this mode. Explicit `ATLAS_HISTORY_MESSAGES` opts into legacy
bounded history; third-party agents without memory configuration retain legacy
behavior. The environment limit must be positive.

### Compaction and checkpoints

Compaction is token-based: automatic at 90%; manual `/compact` chooses one block
below 30%, two below 60%, and three from 60% upward. Preserve at least the latest
10 original messages as whole exchanges. Recompact prior summaries with eligible
newer messages and replace them, never accumulate endless summary blocks. Each
block is a separate no-tool request using `conversation_summary_prompt`, with an
explicit output limit. Split oversized inputs under bounded request/pass limits;
reject empty or truncated results. Keep summary usage separate from chat usage.

Save `memory/<uuid>.json` atomically with version, policy version, checkpoint ID,
original-prefix SHA-256 fingerprint, covered ranges, model, trigger, occupancy
estimate and summary usage. Validate against originals before loading/publishing;
advance the boundary only with a fully successful fitting checkpoint. A checkpoint
can survive a subsequent failed ordinary answer. Keep private filesystem modes,
trash memory with history, and hold per-conversation execution leases for
ask/compact/delete. Do not hold the global index lock during model calls.

Prepare and budget-check context before every runtime model call, including tool
continuations and repair calls; never replay tools during compaction. Native
OpenAI, Anthropic and Gemini use provider token-count APIs before inference;
see [preflight token counting](model-providers.md#preflight-token-counting).
DeepSeek and compatible endpoints use conservative byte estimates, not exact
tokenizers; distinguish preflight counts/estimates from actual usage. Unknown
model capacity or unsupported image accounting
makes auto-compaction explicitly unavailable. Optional per-model context/output
limits live in `conversation.modelLimits`; never infer capacities from model
names. Known oversized requests fail without discarding the protected tail.
Selecting the desktop Compact task must not create a chat message or persist an unsaved draft;
progress, checkpoint restoration, cancellation and memory details belong to the
owning session while other chats continue independently.
Compaction progress/results/failures use the persistent status card. Keep failure
metadata and retry task arguments per session; retries dispatch that saved task
without consuming a newer draft. Preserve open diagnostic details during unrelated
UI updates. Automatic compaction failures keep their chat prompt available for a
normal resend; do not silently launch another summary job.
DeepSeek summary requests explicitly disable thinking to reserve the bounded output
budget for summary content. Keep this provider-specific switch in the adapter,
not shared runtime logic. Distinguish empty output, output-limit truncation, and
unexpected finish reasons; none may publish partial memory or echo response text.

## Titles and protocol repair

After the first successful answer, the agent makes one non-streaming, no-tool title
request using only the first input and `conversation_title_prompt` from
`agents/atlas.toml`. Sanitize generated titles to at most 60 Unicode characters;
failure leaves `Untitled conversation`. Continuing and manual rename never
regenerate the title. The default agent prompt guides workspace inspection,
command execution, verification, preservation of unrelated changes, and honest
reporting of results and blockers. `max_tool_calls` is an optional positive
per-turn agent limit; omit it for no Atlas-imposed limit. Tool schemas and protocol remain runtime-injected.

### Protocol repair

Malformed tool-protocol replies go back to the model with the validation error,
latest invalid reply, and a stable pending request. Continue repair by default;
optional positive `max_protocol_repairs` caps consecutive repairs and resets after
a valid reply. Never nest repair envelopes, lose prior tool results, or replay
executed tools during repair. Prepare/budget context for every retry. Providers
may explicitly classify incomplete generation outcomes as repairable through the
provider/agent boundary; never match provider error strings in shared runtime
logic. DeepSeek distinguishes empty/reasoning-only output, length truncation,
missing completion markers, resource interruptions, aborted output, filtering,
and unsupported finish reasons. Only known repairable generation outcomes enter
the same repair loop and cap. Authentication, transport, filtering, unknown errors,
and context failures still propagate normally. Log safe stream-completion metadata
without answer/reasoning text or arbitrary finish-reason values. Keep invalid reply bodies out of
backend logs and ordinary conversation history; log sanitized `protocol.repair`
metadata instead. If a malformed response already emitted text, send a
`responseReset` event before retrying: clear only that session's pending assistant
text in the desktop and explicitly mark the restarted answer in CLI output.
