# Conversation memory and three-block compaction

## Approved intent

Replace the fixed recent-message window with all available conversation context
until compaction is needed. Support automatic compaction at 90% and a manual
`/compact` command. Summarize older context in three separate blocks, retaining
at least the latest 10 original messages. Persist summaries for subsequent turns
and show exactly which original messages they replace. This is conversation-local
short-term memory, not cross-conversation personal memory.

## Current behavior

`ConversationService` selects the latest configured number of messages (40 by
default). Activity summaries use the same selected range. Runtime can issue
multiple model calls per turn as tools run. Provider usage reports the last
completed call; it cannot tell us the size of the next call before submission.
The desktop's model selection is global. Original messages and activity sidecars
are stored locally; there is no memory checkpoint today.

## Context shape

Before the first compaction, send all saved messages plus the current input,
system instructions, tool definitions, and applicable bounded activity context.
After compaction, send:

1. System instructions and runtime protocol.
2. Three chronological memory blocks, clearly labeled as historical data.
3. All original messages after the compacted boundary, including the latest 10.
4. Current input and current-turn tool results as required by the runtime.

Memory is not injected as new authoritative system instructions. Screenshots,
image bytes, attachments, raw activity secrets, and hidden model reasoning are
never persisted in memory. Summaries preserve user goals/preferences, decisions,
constraints, useful findings, changed paths, unresolved work, and relevant tool
outcomes. They distinguish user statements from assistant claims and uncertainties.

All original history remains on disk and visible. Memory changes the model's
working context, not the saved transcript. Compaction is lossy; viewing originals
remains possible.

## Trigger and capacity accounting

Automatic compaction is enabled by default with a configurable 90% threshold.
Check before each chat-model call, including tool continuations and repair calls.
Use the selected model's context capacity, not a hard-coded model-name table.
Reserve a configured output budget within the provider's known output limit.

Trigger when either the last chat call reported at least 90% occupancy or the
projected next request plus its output reserve reaches 90% of capacity. Provider
usage remains the actual last-call reading; preflight estimates are labeled as
estimates and never overwrite that reading.

Introduce a provider-neutral request-budget interface. Providers can supply an
exact tokenizer/counting implementation or a documented conservative estimator
covering the actual rendered prompt, protocol, tools, history, and images. Treat
unaccounted inputs as unknown rather than zero. A final safety check must reject
an oversized request instead of silently deleting recent messages. Summarization
requests also undergo budget checks. A context-length error may trigger one
bounded recovery attempt only when it is safe to retry without repeating tools.

If context capacity or safe budgeting is unavailable, explicitly report that
automatic compaction is unavailable. Permit an explicit capacity configuration;
never claim 90% is enforced using an invented capacity. Manual compaction still
requires a safe input/output budget. Switching to a smaller model reruns these
checks against that model before the next request.

## Building three summaries

Exclude the latest 10 messages, extending the retained tail backward if needed
to keep whole user/assistant exchanges. Ten means messages, not ten exchanges.
Divide the remaining chronological context into three approximately token-balanced
blocks at exchange boundaries. These are roughly thirds of older history, not
strict 30% slices of total capacity; the recent tail and prompt overhead also
consume space. If there are fewer than three older exchanges, use fewer blocks;
with no eligible older history, `/compact` reports that nothing can be compacted.

Run a dedicated no-tool summarization job with one bounded model request per block,
sequentially within that conversation. It uses the selected provider/model and a
summary prompt in local agent configuration, separate from ordinary chat and title
prompts. Configure an explicit per-block output-token ceiling, capped by the
provider's output limit and the aggregate memory budget. Initial target: 2,048
tokens per block, further reduced when required to fit. Request the limit through
the provider interface and reject output truncated at its generation limit.
Summary generation usage is tracked separately and cannot replace chat usage.

Large blocks are split into budget-safe subrequests and combined in bounded
passes; an individual oversized message may be split for summarization with its
source provenance preserved. Never submit an oversized block simply because it
is one third of history. Bound passes and fail clearly if a useful checkpoint
cannot fit; do not endlessly retry or truncate the memory silently.

On later compaction, combine the previous three summaries with eligible newer
messages into an ordered source sequence, repartition it, and produce three new
summaries. Existing summaries carry their original source ranges. Do not reload
all previously compacted originals for every summary request or append summaries
forever. Validate that the resulting memory plus retained tail/current input and
output reserve fits below the trigger before publishing it. If the protected tail
or current-turn data alone cannot fit, report the constraint without dropping it.

## Persistence and exact boundaries

Add a versioned per-conversation memory sidecar containing:

- Checkpoint ID, creation time, summarizer model, and policy version.
- Covered original-message prefix length (exclusive end index).
- Hash of that original prefix to detect stale or modified history.
- Up to three ordered blocks, each with text and original-message source range.
- Summary usage and whether it was manually or automatically triggered.

For a boundary of 80, summaries represent message indices `[0, 80)`; the next
request loads original messages from index 80 onward. UI presents one-based ranges
so users see “Messages 1–80 summarized.” Covered activity is summarized once with
its source block; only activity belonging to retained messages is added afterward.

Generate without holding a global storage lock. Before publishing, acquire the
existing store lock, verify the source prefix and expected checkpoint version,
and atomically replace the memory sidecar. Publish all blocks together only after
all requests and validation succeed. Failure leaves the previous checkpoint and
original history intact. A successful memory checkpoint can survive a subsequent
failed chat answer; it does not create an ordinary successful chat turn.

Serialize ask/compact operations for the same conversation, including across app
processes, with a per-conversation execution lease. Other conversations remain
independent. Rename keeps memory; delete moves its sidecar to trash with history;
legacy conversations without memory load normally. Missing or invalid checkpoints
must not cause gaps: rebuild from originals or report an actionable error.

## Desktop and CLI behavior

Intercept standalone `/compact` in the desktop before normal chat submission.
Do not send it to the conversational model, save it as a user message, or create
an empty saved conversation. Expose the same service via
`atlas conversation compact <id>` for CLI users.

The active chat shows “Compacting context · block 1 of 3” while the job runs.
Its composer is busy to prevent competing operations; other chats remain usable.
Automatic compaction resumes the pending request after a successful checkpoint.
Manual compaction saves memory and displays its result without generating a chat
answer. Cancellation/failure retains the previous checkpoint and shows a retryable
error without an automatic retry loop.

Place a persistent divider after the covered transcript range:
“Earlier context summarized · View memory.” Its details show the three blocks,
message ranges, time, and model. Reopening restores this state. The context-ring
tooltip distinguishes the last actual chat usage from a newly prepared/compacted
context; it must not invent a fresh percentage after a summary-only operation.
Show compaction-unavailable status when capacity/budgeting is missing.

The old history-limit setting no longer silently discards messages in this mode.
Preserve settings-file compatibility, migrate the UI away from presenting 40 as
the active policy, and document treatment of `ATLAS_HISTORY_MESSAGES`. Explicit
legacy environment limits should be honored as an opt-in legacy mode with a
visible status; compaction mode must not pretend the truncated history is complete.

## Architecture boundaries

Use a pure context planner for ranges, budgets, and checkpoint validation; a
compaction service for the bounded summary job; and store helpers for atomic
sidecar lifecycle. Runtime calls a context-preparation boundary before every
model invocation so growing tool results are counted without replaying tools.
Keep provider request rendering/counting/output limits inside provider adapters.
Keep summarization prompts/policy in local configuration. Tauri only bridges
operations/events; desktop communicates through `atlas-api.ts`.

No new autonomous agent type or router is needed: the summary job is a dedicated
no-tool task through the existing agent/provider abstractions.

## Verification requirements

Use fake models and loopback responses only. Cover full history beyond 40,
three-block ordering/coverage, a protected 10-message tail, repeated compactions,
short histories, actual usage versus preflight estimates, output reserves,
unknown capacities, smaller model selection, oversized messages and tool results,
summary output truncation, bounded recursive reduction, and no-tool summarization.

Test checkpoint atomicity, crash/failure/cancellation, prefix/version conflicts,
legacy loading, delete/trash, duplicate/gap prevention, same-conversation leases,
and independent concurrent chats. UI tests cover `/compact` interception,
progress/error states, automatic continuation, restored memory details, and
unchanged actual chat usage after summary-only calls. Run repository formatting,
core/native/frontend tests, Clippy, frontend build, and Tauri debug build before
claiming the implementation complete.

## Scope and next gate

This document specifies the approved three-summary direction. Implementation has
not begun. Review this spec, then write and review the implementation plan before
changing runtime behavior.
