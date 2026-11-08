# MCP Onboarding Implementation Plan

> Use superpowers:executing-plans inline and test-driven-development. User authorized execution. Preserve existing work; commit this completed feature locally.

**Goal:** Add MCP servers and explicitly inspect available tools from simple Settings controls.
**Spec:** docs/superpowers/specs/2026-11-08-mcp-onboarding-design.md
**Architecture:** Rust config store owns bounded append/lease/publication. Existing MCP registry owns discovery and permissions. A Settings-only worker/channel owns approval and cancellation. Frontend APIs stay in atlas-api.ts.

## Tasks

1. [x] Config store: src/mcp/settings.rs and config path helper; tests tests/mcp_settings_behavior.rs. Typed load/fingerprint/add-only; preserve original bytes, validate before publishing; no connection construction. RED then GREEN config tests.
2. [x] Core inspection: src/mcp/inspection.rs; registry authorization/execution reuse, cancellation before/after calls, bounded timeout; focused fake server and denied-authorizer tests.
3. [x] Tauri integration: mcp_commands.rs + small inspection-worker module; reuse backend workspace resolver and saved permission authority. Settings operation IDs/channels/approval registry; no global chat cancellation. Unit tests for routing, scoped cancellation, replay rejection and late completion.
4. [x] Frontend: mcp-types.ts, atlas-api.ts, chat-state.ts, settings-view.ts, mcp-settings.ts and tests. Stable add form, honest auth/test states, bounded tool list disclosure and in-panel approval. Shared test API doubles updated. RED then GREEN UI ownership/draft tests.
5. [x] Docs and complete verification: README/AGENTS; cargo formats/tests/clippy, Tauri formats/tests/clippy, frontend tests/build and Tauri debug build. Real-browser UI checks and independent review. Review staged diff and commit; do not push.

## Review focus

- No network/process during load/save/authorization validation.
- No hidden overwrite, malformed-document replacement or credential identity mutation.
- Settings approvals cannot authorize other operations or chats.
- Saved workspace/mode authority and stale callback invalidation.
- Drafts/focus remain intact during independent OAuth/check updates.

## Progress

- Baseline clean on main; created feat/mcp-settings-onboarding. Scope excludes the separately proposed Stop/diff/history features.

- Config, inspection and UI RED/GREEN checks recorded during implementation. Additional regression checks cover concurrent writers, FIFO/symlink rejection, cancellation before worker reservation, cancellation between initialize/discovery pages, stale refresh after Save, draining checks after context invalidation, and changing chats while Settings is hidden.
- Independent read-only review completed. Fixed all three findings: retained Cancelling until worker exit, rejected pre-save refresh completions with a mutation epoch, and stopped discovery at RPC boundaries after cancellation. Follow-up review found no important issues, including the bounded early-cancellation registry.
- Verification: Rust core 453 tests and desktop 57 tests passed. cargo fmt --check, cargo test, cargo clippy --all-targets --all-features -- -D warnings; desktop Rust fmt/test/clippy; npm test (297 passing), npm run build and Tauri debug --no-bundle build all passed. Existing screenshots dependency future-compatibility notice remains.
- Browser exercised the production panel with a fixture backend: add HTTP connection, preserve draft across refresh, explicit approval and successful tool disclosure, desktop and 390px viewport with no horizontal overflow. Screenshots: /tmp/atlas-mcp-add.png, /tmp/atlas-mcp-verified.png, /tmp/atlas-mcp-mobile.png. Actual stdio fixtures verify discovery, cancellation and process cleanup; no live user MCP config or provider credentials were changed. Native Tauri package built; browser checks do not claim native end-to-end OAuth acceptance.
