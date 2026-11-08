# File patch and process sessions implementation plan

> Execute inline using executing-plans and test-driven-development. User has authorized execution and autonomous implementation. One independent review at completion.

**Goal:** Exact workspace editing and bounded per-request process sessions.
**Architecture:** Anchored single-file patch tool with preimage authorization; shared request-owned process manager wrapping existing Unix/Windows executors. No provider-specific logic or frontend native permissions.
**Spec:** docs/superpowers/specs/2026-11-09-tool-workflows-design.md

## Tasks

- [x] Patch: tests/apply_patch_behavior.rs RED; src/tools/apply_patch.rs implements typed validation, unique replacements, snapshot authorization, inode lease and atomic publication; expose/register in tools/mod.rs/app.rs. GREEN tests, docs, fmt/test/clippy then feature commit.
- [x] Sessions: tests/process_session_behavior.rs RED; src/tools/process_sessions.rs handles manager/control/bounded output/lifecycle. Existing exec_command.rs and exec_command_windows.rs accept yield/timeout and optional observer; reuse platform cleanup. Register read_process/stop_process sharing an ExecCommandTool manager. Tests include synchronous compatibility, incremental output/UTF-8, runtime isolation, reset/drop cleanup, stop/timeout, validation and limits. GREEN and platform checks.
- [x] Integration: actual runtime tool continuation test; descriptions in agent config, README and AGENTS. Full Rust and desktop fmt/test/clippy, frontend test/build, Tauri debug build. Independent review, resolve important issues, focused session-feature commit. No push/merge.

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

- Patch commit: 03af1bf. Independent review reproduced a moved-parent stale approval; regression observed RED and then GREEN after original path/parent identity revalidation. Under-lock reads use the original file handle. Fix committed separately as ff35a21.
- Sessions RED: missing register_process_tools. GREEN: real shell start/poll/stop, UTF-8 split reads, output overflow, eight-session admission limit, retained-record bound, quiet redirected descendants, continuous-output stop, runtime isolation and cleanup on final/provider error. Existing synchronous shell tests remain green.
- Review findings resolved: Unix normal completion now terminates remaining owned group before reaping the leader, using waitid WNOWAIT to prevent PID reuse; decoder retains incomplete trailing UTF-8 even after invalid bytes. Regression tests observed failing before fixes. Reviewer independently reran all three reproduction harnesses successfully; follow-up found no important issues.
- Ruling: preserve runtime-injected tool names/protocol, keep general workflow guidance in agents/atlas.toml. Existing prompt-contract test caught an overly specific draft; guidance now describes capabilities and lifecycle without naming shell/file tools.
- Verification: core cargo fmt/test/clippy passed (472 tests); desktop Rust fmt/test/clippy passed (57 tests); frontend tests passed (297), frontend production build and Tauri debug --no-bundle build passed. Existing screenshots future-compatibility notice remains. No model requests or live credentials used.
- Windows: native cargo check --tests --locked completed against freshly timestamped copied source. Updated executable tests could not complete because Windows Application Control blocked the test binary (OS error4551); do not bypass policy or claim native execution acceptance. Windows-only tests compile, including quoted PowerShell child stop and unchanged patch contents when Windows prevents parent rename. cmd.exe raw_arg construction follows the installed official Rust std example and received independent source review. Existing unrelated Windows conditional-import/builder warnings remain. macOS native execution was not available.
- User-facing session limitation: one agent request only, no persistent terminal or service across messages. No frontend permissions, stdin/PTY, daemon, push or merge added.
