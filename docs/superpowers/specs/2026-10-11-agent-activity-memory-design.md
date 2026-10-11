# Agent Activity Memory Design

## Summary

Atlas currently shows transient tool start and finish events in the desktop UI,
but only successful user and final-assistant messages survive a restart. Later
turns therefore know what the participants said but not what Atlas inspected,
changed, or attempted while producing an answer.

This change adds a durable, structured activity log for successful turns. The
desktop displays that log as an expandable timeline, and later model requests
receive a compact, bounded summary of recent activity. The feature records
observable runtime actions, not private model reasoning.

## Goals

- Let users see what Atlas did while producing each answer.
- Preserve useful tool history across application restarts.
- Give later turns enough action context to avoid repeating work or claiming
  ignorance of prior operations.
- Keep ordinary conversation history readable and backward compatible.
- Bound context use and avoid persisting screenshots or obvious credentials.
- Use the same storage and runtime behavior for the CLI and desktop, while
  initially adding the activity presentation only to the desktop UI.

## Non-goals

- Exposing chain-of-thought, hidden reasoning, or provider-internal tokens.
- Replaying the complete provider transcript in future requests.
- Persisting screenshot pixels, image data URLs, or other binary attachments.
- Turning tool activity into ordinary user or assistant chat messages.
- Adding a general tracing, telemetry, or analytics system.
- Adding a CLI activity viewer in this iteration.

## User Experience

While a request is running, the desktop shows a status row before the first
tool request, followed by one row per tool operation. Examples include:

```text
Thinking…
Searching files for "ToolEvent"
search_files succeeded
Found 14 matching files
Preparing answer…
```

The runtime status is descriptive UI state, not model reasoning. Tool rows show
a concise label and state: waiting for approval, running, succeeded, failed, or
denied. A row can be expanded to show its bounded, redacted arguments and
result. Once the answer completes, the activity belongs to that assistant turn.
Reopening the conversation restores the same timeline.

The existing chat transcript remains focused on user and assistant messages.
Conversations created before this feature simply show no historical activity.

## Domain Model

Activity is represented independently from `ConversationMessage`:

```rust
ActivityRecord {
    turn_id: String,
    sequence: u32,
    kind: ActivityKind,
    tool: Option<String>,
    arguments: Option<String>,
    outcome: Option<ActivityOutcome>,
    summary: String,
    details: Option<String>,
    created_at: DateTime<Utc>,
}
```

`ActivityKind` initially supports runtime status, tool call, and tool result.
`ActivityOutcome` supports succeeded, failed, and denied. The schema is
versioned at the file-envelope level so fields and activity kinds can evolve.

`turn_id` groups activity with a successful user/assistant pair without
changing the existing message JSONL schema. `sequence` provides stable ordering
inside a turn. IDs are locally generated and have no provider meaning.

Runtime status records use approved phrases such as `Thinking` and `Preparing
answer`; they never contain hidden reasoning supplied by a model. A tool result
contains a short human-readable summary and optional bounded details.

## Storage

Each conversation may have a sidecar activity file:

```text
activities/<conversation-id>.jsonl
```

The existing `conversations/<conversation-id>.jsonl` message history remains
unchanged. This avoids reinterpreting old message lines and keeps conversation
rendering independent from internal activity.

For a successful turn, `ConversationService` accumulates activity in memory.
The store commits the user message, assistant message, activity batch, and
metadata while holding the existing conversation lock. Temporary files and
atomic publication are used where replacement is required. If activity cannot
be persisted, the turn commit fails rather than silently creating conversation
memory that disagrees with the visible answer.

For a new conversation, no message or activity file is created until the final
answer succeeds. For an existing conversation, a failed overall request leaves
both durable message history and durable activity unchanged. Failed and denied
tool attempts are saved when the overall turn later succeeds because they are
part of the action history that produced the answer.

Deleting a conversation moves both its message history and activity sidecar to
the trash area. The command continues to report a recoverable location. Missing
activity files are normal and read as an empty activity list.

## Capture and Sanitization

The runtime emits richer structured events around each tool request:

- tool requested, including the structured arguments;
- approval requested or denial;
- tool started;
- tool finished, including success and the textual result;
- turn-level runtime status.

Persistence applies these rules before data reaches the store:

- Serialize arguments deterministically and cap their stored length.
- Cap result details per record and append an explicit truncation marker.
- Cap the total stored activity detail per turn.
- Redact values whose object keys indicate credentials, including `api_key`,
  `apiKey`, `token`, `password`, `secret`, `authorization`, and close
  case-insensitive variants.
- For `capture_screen`, save only operation metadata, dimensions when
  available, and success or failure. Never save bytes, base64, or data URLs.
- Do not persist tool attachments.

Redaction is defense in depth, not a guarantee that arbitrary command output
contains no sensitive text. Consequently, raw stored details are local-only
and are not automatically included in later provider requests.

The exact caps are named constants near the sanitizer and covered by boundary
tests. They are implementation limits, not user settings in this iteration.

## Later-Turn Model Context

When continuing a conversation, `ConversationService` selects activity for the
same recent turns selected by the existing message-history policy. It renders a
compact context block such as:

```text
Recent Atlas activity (facts from earlier successful turns):
- Searched workspace files for "ToolEvent"; found 14 matches.
- Read desktop/src/types.ts successfully.
- Ran `npm test`; command succeeded.
```

Only summaries are included. Stored argument details and raw result details are
not sent automatically. The block has a separate total character cap and keeps
the newest complete entries when truncation is necessary.

Activity context is carried as an explicit field through the runtime, agent,
and `ModelRequest` boundaries. Providers render it as a separate system-context
section before conversation messages. It is not impersonated as a user or
assistant message. Empty activity produces no additional model content.

The active turn continues to use full in-memory tool results for tool-loop
continuations as it does today. Durable summaries affect only later user turns.

## Runtime and Event Flow

The current `ToolEvent` contract is extended rather than adding a second event
channel. Events contain enough information for both persistence and the live
UI, while serialization and redaction helpers create the durable form.

```text
Runtime executes tool
  -> structured ToolEvent
  -> ConversationService accumulates sanitized records
  -> desktop channel renders live state
  -> successful final answer commits messages + activity
  -> later request selects activity summaries
  -> ModelRequest carries separate activity context
```

The runtime emits a `Thinking` status when a model request begins and a
`Preparing answer` status after tool processing when another model request is
made. These states communicate progress only. The model is not asked to reveal
or manufacture reasoning text.

## Desktop API and State

Conversation DTOs gain activity grouped by turn. The Tauri chat event contract
gains runtime-status and detailed tool-event payloads. Sensitive filtering and
size limits happen in Rust; the frontend does not receive unsanitized values.

`ChatStore` maintains live activity for the active request. When the request
completes, it replaces transient entries with the canonical persisted activity
returned by the backend. Loading a saved conversation reconstructs activity
groups and associates them with assistant turns using `turn_id`.

The chat view renders a collapsed `Activity` disclosure below the associated
assistant response. While streaming, the current disclosure remains visible and
updates in place. Status and tool outcome remain accessible through text and
ARIA labels rather than relying on color alone. Long details use preformatted,
wrapping text and remain bounded by the backend.

Approval controls stay in their current prominent location. The corresponding
activity row shows that Atlas is waiting, then changes to denied or running when
the user decides.

## Error Handling

- Malformed or unsupported activity records produce a path- and line-specific
  storage error rather than being silently ignored.
- A missing sidecar is treated as an old conversation with no activity.
- A live desktop channel failure still aborts the request through the existing
  fallible sink behavior.
- Tool failures and denials become normal activity outcomes and do not by
  themselves fail persistence.
- If the final response fails, accumulated activity remains transient and is
  discarded.
- If the coordinated store update fails, the error reports any recoverable
  paths and avoids publishing metadata that claims a completed turn.

## Compatibility and Documentation

No migration is required for existing conversation histories or metadata. The
activity sidecar is optional. The versioned format allows a future reader to
reject incompatible versions clearly.

`README.md` and `AGENTS.md` will be updated to describe activity persistence,
later-turn summary context, local raw details, screenshot exclusion, and the
successful-turn commit rule. The existing statement that internal tool results
are never saved must be revised to distinguish the user-visible message history
from the separate activity log.

## Testing and Verification

Focused Rust tests will cover:

- activity serialization and backward-compatible missing-sidecar behavior;
- ordered storage and loading of successful-turn activity;
- no durable activity after failed turns;
- preservation of failed and denied tools after a successful final answer;
- redaction and per-record/per-turn truncation boundaries;
- screenshot payload exclusion;
- recent-turn selection and activity-context character caps;
- provider request placement without fake user or assistant messages;
- deletion and trash behavior for message and activity files;
- enriched runtime event ordering and data;
- desktop DTO conversion and persisted activity loading.

Frontend tests will cover:

- live status and tool-row transitions;
- approval waiting, denial, success, and failure states;
- expandable bounded details;
- association of restored activity with assistant turns;
- accessibility labels and stale-request protection.

Before completion, run the repository-required Rust formatting and tests, both
Rust clippy suites, Tauri tests and formatting, frontend tests and build, and the
debug no-bundle Tauri build. Review the staged diff and commit only files that
belong to this feature.

## Acceptance Criteria

- A user can watch Atlas progress and tool operations in the desktop chat.
- Completed activity reappears after restarting and reopening a conversation.
- A later turn receives concise action summaries and can use them to avoid
  repeating prior work.
- Ordinary conversation history contains only user and final-assistant
  messages.
- No chain-of-thought or screenshot pixels are persisted or displayed.
- Raw details are bounded, redacted, local-only, and excluded from automatic
  later model context.
- Old conversations remain readable without migration.
- Failed overall turns leave no durable message or activity records.
