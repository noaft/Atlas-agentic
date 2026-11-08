# Precise file edits and process sessions

User approved implementing the recommended apply_patch and process-session tools. Continue inline with tests and focused local commits, without pushing. Preserve the completed MCP feature.

## Decisions

- `apply_patch` uses structured JSON `{path, edits:[{old_text,new_text}]}` for one existing UTF-8 workspace file. Each old_text must be nonempty and match exactly once, in sequential edit order. No creation/deletion/moves in this tool; use existing helpers. Bounds: 64 edits and 1 MiB source/result/input replacement bytes. Validate all edits before publishing. Destructive risk preserves existing overwrite-confirmation semantics. Authorization snapshots full preimage and binds it to execution; changed files fail. Open anchored parent, reject symlink/special-file leaves, lock source inode with bounded lease, re-resolve workspace/parent identity and verify source path identity/content at execution/publication, stage private same-directory file, preserve Unix mode, atomically replace. No persistent lock files, no shell execution. Return hashes and replacement count rather than file contents.
- Keep synchronous exec_command behavior for existing callers. Opt in using `yield_time_ms` (0–10000). Session commands return completed result immediately if done, otherwise opaque session_id. `timeout_ms` is bounded 1–600000; defaults remain 30s synchronously, 10min for sessions. `read_process` returns only unread stdout/stderr and status, with optional wait_ms 0–10000. `stop_process` stops only a session owned by this runtime, with Command risk; read is ReadOnly. All validation occurs before launch.
- Sessions are isolated to the owning runtime request, shared across that request's tool calls. They are not saved, not OS PID handles, and cannot access another runtime/chat. On all request exits/reset/drop, signal stop and join workers, terminate owned Unix groups or Windows Job Objects. Preserve legacy timeout/pipe cleanup. Session completion also terminates quiet group descendants before reaping the Unix leader; retain its PID using WNOWAIT. Windows preserves literal approved command text via cmd.exe raw outer quoting. No cross-turn terminal or new desktop permissions/UI in this release.
- Reuse the existing platform shell executors; add an optional cancellation/output observer. Shared manager keeps at most 8 active sessions and 32 retained records, with per-stream unread buffers of 64 KiB. Overflow is explicit. Incremental UTF-8 output preserves incomplete character suffixes until later reads. Poll never replays consumed bytes. Terminal failures are reported; timeouts and stops must not imply the command had no effects. No automatic retries or stdin/PTY input.

## Alternatives

Full shell-only editing and synchronous commands remain possible but provide weaker structured control. Persistent per-conversation terminals require separate lifecycle, restart and UI decisions; defer those rather than imply persistence the current per-request runtime cannot provide. Standard textual multi-file patches would add parsing and partial-publication complexity; start with explicit, exact single-file replacements.

## Evidence

Tests cover stale approval preimages, ambiguous/failed edits without writes, UTF-8/CRLF/mode/path safety, concurrent modification, session isolation, incremental output, stop/timeout and descendants, overflow, resource limits, request-scope cleanup, and existing synchronous behavior. Run repository fmt/test/clippy, desktop checks and build. Commit each completed feature independently after checks.
