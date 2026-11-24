# Session journals and diagnostic logs

Implemented session tracing is separate from committed conversation history.
`src/sessions/` owns journal storage, event recording and context provenance;
`src/diagnostics.rs` owns technical logs. Neither layer executes or replays tools.

The single-file layout below was approved on 2026-11-24 and is implemented.

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
  .locks/<session-uuid>.lock
  YYYY/MM/DD/
    HHMMSSffffff_<session-uuid>.json
logs/
  atlas.jsonl
```

Year/month/day and time come from the first submission, not completion or the
latest continuation. Zero-padded time prefixes sort sessions chronologically;
UUIDs disambiguate equal timestamps. The default clock is the operating system's
local timezone; `utc` is configurable. The original timestamp and offset are
persisted and never recalculated when opening the session on another day or after
a timezone/configuration change. Each session is one ordinary JSON document
with a top-level `version: 2`, `metadata` object and ordered `events` array.
The date directory contains session JSON files directly, with no per-session
subdirectory or per-event files. The versioned index locates existing sessions
without searching the directory tree. `SessionJournal::path()` returns the JSON
file; `SessionStore::find()` returns that file, or an existing legacy journal
folder until conversion.

### Compatibility with earlier journals

Explicitly opening an earlier `HHMMSSffffff_<session-uuid>/` journal validates
its metadata and every numbered event under both its old lease and the new stable
lease. Atlas publishes the complete JSON replacement before touching the source.
It then atomically moves the old folder into root-level `.legacy/` staging and
removes it. Interrupted or failed staging leaves both copies; on the next open,
legacy events must be an exact prefix of the published JSON before cleanup can
continue. Divergence or corruption fails visibly and retains both sources.
Interrupted recursive cleanup can leave a private staging folder; it does not
block subsequent opens or become a new journal. No inference or tools are replayed.
Do not run old and new Atlas versions against the same data root concurrently.

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

Write each event immediately by atomically replacing the complete JSON document
through a private temporary file, updating metadata and events together. By default
sync the file (and parent directory on Unix) to disk. This rewrites accumulated
session data on each event; it trades write cost for one directly readable JSON.
An unfinished temporary file is never a published session update. Stable exclusive
leases under `.locks/` remain held across file replacements and archive rollback;
cloned journal handles serialize updates through shared state. Opening validates
contiguous event sequences, record identities and schema, rebuilds sequence/cursor
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
Deleting a saved conversation moves its JSON file to `trash/<uuid>.session`
alongside existing history/activity/memory recovery files, removes index aliases,
and restores those files if index publication fails. An unopened legacy journal
is archived as its original folder. If both formats remain after interrupted
conversion, deletion fails visibly until an explicit session open validates and
finishes conversion, so neither trace is silently left behind. No automatic
restore exists.
Failed drafts remain inspectable in the journal even after their in-memory UI ends.

## Context provenance and cursor

Model input continues to come from the committed conversation/memory projection
plus the current continuation. Do not send arbitrary journal/debug records back to
the model. Before a chat inference, `SessionContext` records the selected source:
conversation ID, original-message start, selected history length/fingerprint,
memory checkpoint/ranges, bounded current input and its full SHA-256 digest,
tool catalog names/digest, image count, model and output reserve.

The session document's `metadata.contextCursor` stores a logical context cursor:
`historyMessageStart`,
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
sync, maximum index/event/whole-session bytes, and per-field text bound.
`max_session_bytes` defaults to 64 MiB and must be at least `max_event_bytes`;
complete replacement policy files must include this field. Reject oversized
sessions and indexes before publication, retaining their previous readable files.
A rejected session update latches the journal failure and blocks subsequent work;
raising the limit allows a later explicit open, without replaying actions.
Text truncation
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
