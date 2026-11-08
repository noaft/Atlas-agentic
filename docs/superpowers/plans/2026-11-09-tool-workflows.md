# File patch and process sessions implementation plan

> Execute inline using executing-plans and test-driven-development. User has authorized execution and autonomous implementation. One independent review at completion.

**Goal:** Exact workspace editing and bounded per-request process sessions.
**Architecture:** Anchored single-file patch tool with preimage authorization; shared request-owned process manager wrapping existing Unix/Windows executors. No provider-specific logic or frontend native permissions.
**Spec:** docs/superpowers/specs/2026-11-09-tool-workflows-design.md

## Tasks

- [x] Patch: tests/apply_patch_behavior.rs RED; src/tools/apply_patch.rs implements typed validation, unique replacements, snapshot authorization, inode lease and atomic publication; expose/register in tools/mod.rs/app.rs. GREEN tests, docs, fmt/test/clippy then feature commit.
- [ ] Sessions: tests/process_session_behavior.rs RED; src/tools/process_sessions.rs handles manager/control/bounded output/lifecycle. Existing exec_command.rs and exec_command_windows.rs accept yield/timeout and optional observer; reuse platform cleanup. Register read_process/stop_process sharing an ExecCommandTool manager. Tests include synchronous compatibility, incremental output/UTF-8, runtime isolation, reset/drop cleanup, stop/timeout, validation and limits. GREEN and platform checks.
- [ ] Integration: actual runtime tool continuation test; descriptions in agent config, README and AGENTS. Full Rust and desktop fmt/test/clippy, frontend test/build, Tauri debug build. Independent review, resolve important issues, focused session-feature commit. No push/merge.

## Review focus

- Patch failures do not partially update the file; preimage cannot change while approval waits.
- No path escape/symlink/special-file writes or permission broadening.
- No process/session crosses runtime ownership; all exit paths clean up.
- Output flood, split UTF-8, descendants retaining pipes and stop races remain bounded.
- Original command results and confirmation modes remain compatible.

## Ledger

- Clean starting checkout e0aa753; feature branch created in existing workspace, preserving the user's established inline workflow. Separate worktree and repeated design-approval prompts omitted under the user's prior autonomous-work direction.
- Ruling: session lifetime is one runtime request, including all tool continuations. Cross-turn processes would require durable conversation ownership not present in current runtime; document this limit explicitly.

- Patch RED: missing ApplyPatchTool; overlap regression also observed failing before correction. GREEN: all 459 Rust tests, fmt and clippy passed; desktop Rust tests/clippy passed. Exact UTF-8 edits, ambiguous/overlapping edits, bounded inputs, stale approval and source inode publication tested.
