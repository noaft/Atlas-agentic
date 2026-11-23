# Session journals and diagnostic logs

Implemented session tracing is separate from committed conversation history.
`src/sessions/` owns journal storage, event recording and context provenance;
`src/diagnostics.rs` owns technical logs. Neither layer executes or replays tools.

## Identity and chronological storage

The first submitted ask reaching `ConversationService` creates a durable session
before inference, even if the turn later fails. Merely opening New chat creates
no journal. The journal reserves the UUID used by its eventual saved conversation.
Desktop draft requests use a hash of the native process-scope identity to find the
same journal on retry; arbitrary frontend paths do not select journal ownership.
Continuations resolve by saved conversation ID. Existing conversations acquire a
journal lazily, anchored to their original creation time; original history is not
rewritten or moved.

Relative to the platform data root (`ATLAS_DATA_DIR` overrides it), defaults are:

```text
sessions/
  index.json
  index.lock
  YYYY/MM/DD/
    HHMMSSffffff_<session-uuid>/
      session.json
      session.lock
      events/
        00000000000000000001.json
        00000000000000000002.json
logs/
  atlas.jsonl
```

Year/month/day and time come from the first submission, not completion or the
latest continuation. Zero-padded time prefixes sort sessions chronologically;
UUIDs disambiguate equal timestamps. The default clock is the operating system's
local timezone; `utc` is configurable. The original timestamp and offset are
persisted and never recalculated when opening the session on another day or after
a timezone/configuration change. Each event is an independent JSON document,
not an append to a JSON array. The versioned index locates existing sessions
without searching the directory tree.

## Durable steps and recovery

Each record carries a schema version, session ID, turn ID, monotonic sequence,
UTC timestamp, event kind, bounded data and truncation marker. Record turn start
and outcome, descriptive turn workspace/permission/model context, streamed
assistant deltas and authoritative final text, accepted supplemental inputs,
progress, tool calls/approvals/start/results, execution wakeups, model-call start/end,
usage, context counts/selections and protocol repair. Title generation has its own
start/end markers; its usage is not chat usage. Preview events retain presence and
length only, never transient preview text. Screenshots and image bytes are never
persisted; context records include only their count. Native delegated workers, including direct `/agent-name` dispatch, append their
model/tool events and streamed output to the same session with `agentRunId`;
child lifecycle records also identify nested parent runs. The native journal
handle is carried explicitly into each worker, without granting a tool or changing
permissions. Child context selections use `child_context_prepared` and the frozen
originating cursor, so parallel child activity cannot move the session's root
context cursor or accidentally reference a later root preparation.

Write each event immediately, under an exclusive session lease, through a private
temporary file and atomic publication. By default sync each event and metadata to
disk. An unfinished temporary file is never an event. Opening a journal validates
contiguous event filenames, record identities and schema, rebuilds sequence/cursor
from published events, and closes an unsealed prior writer with `turn_recovered`.
This records an interrupted writer, not proof that a side effect failed. A child
completion may be appended during cooperative cleanup after a failed parent turn;
it does not reopen the turn or permit further inference/actions. Never
resume inference or replay actions automatically. Corrupt records fail visibly.

A journal failure blocks subsequent inference and tool/control mutations at checked
runtime boundaries. If conversation history has already committed, retain successful
metadata and return a `persistenceWarning` instead of presenting the turn as failed.
Desktop retains the saved conversation identity and displays the warning; CLI prints
it to stderr. This prevents an automatic retry of already committed work. A failed
journal may remain incomplete and is explicitly unconfirmed on later inspection.

The journal is a trace, not the ordinary chat projection. Failed turns remain in
the journal but do not enter User/Assistant history or future model instructions.
Successful/stopped turns retain existing history/activity persistence semantics.
Deleting a saved conversation moves its entire journal to `trash/<uuid>.session`
alongside existing history/activity/memory recovery files, removes index aliases,
and restores those files if index publication fails. No automatic restore exists.
Failed drafts remain inspectable in the journal even after their in-memory UI ends.

## Context provenance and cursor

Model input continues to come from the committed conversation/memory projection
plus the current continuation. Do not send arbitrary journal/debug records back to
the model. Before a chat inference, `SessionContext` records the selected source:
conversation ID, original-message start, selected history length/fingerprint,
memory checkpoint/ranges, bounded current input and its full SHA-256 digest,
tool catalog names/digest, image count, model and output reserve.

`session.json` stores a logical context cursor: `historyMessageStart`,
`checkpointId`, and `preparedEvent` identifying the latest published context
selection. It is rebuilt from events, not an unchecked byte offset. This records
which original history was selected, including bounded-history mode. Compaction
still owns its existing checkpoint boundary; this change does not implement a new
compaction algorithm or automatic continuation from journal events. Saved native
conversation metadata remains authoritative for workspace and permissions.

## Configuration and debug correlation

Defaults live in `config/observability.toml`. `ATLAS_OBSERVABILITY_CONFIG` selects
a complete replacement TOML file; reject invalid values and unknown fields.
Session policy controls the relative directory, `local`/`utc` timezone, per-event
sync, maximum index/event bytes, and per-field text bound. Reject an oversized
index before publication, retaining the previous readable index. Text truncation
is explicit; registered credentials and sensitive structured/embedded JSON fields
are redacted. Journal content remains private local session data.

Diagnostics keep one rotating `logs/atlas.jsonl` stream, with the existing
`ATLAS_LOG_DIR` and `ATLAS_LOG_STDERR` overrides. Policy controls DEBUG/INFO/WARN/ERROR
threshold, file size, backup count, field length and bounded lock waits. DEBUG is
the default. Session/turn IDs join existing request IDs; journal step metadata also
includes sequence, kind and available model/call/tool identifiers and usage.
Failures and incomplete persistence are logged while the session trace is active.
Never copy prompts, raw responses, tool arguments/output or images into diagnostics.
Logging failures remain non-fatal; durable journal failures follow the rules above.
