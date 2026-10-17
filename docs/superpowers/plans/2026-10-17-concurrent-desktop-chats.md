# Concurrent Desktop Chats Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Keep Atlas navigable while separate chats run requests concurrently.

**Architecture:** Give each frontend chat a stable session identity and its own live request state. Keep global settings and saved metadata separate, route IPC callbacks to their session, and reuse existing Rust workers with same-conversation exclusion. Render visible streaming updates once per frame without replacing Settings or stealing scroll/focus.

**Tech Stack:** Rust, Tauri v2, TypeScript, Vitest/jsdom.

**Spec:** `docs/superpowers/specs/2026-10-17-concurrent-desktop-chats-design.md`

## Global Constraints

- New chat remains an unsaved frontend draft until a successful first request.
- Allow one request per chat at a time and concurrent requests in different chats.
- Navigation never calls global `cancelToolApprovals`; preserve the two-minute timeout.
- Running workers retain their starting settings and workspace.
- Keep native calls in `atlas-api.ts` and retain narrow Tauri capabilities.
- No durable jobs, cancellation feature, router, providers, or CLI behavior changes.

## Review Focus

- A first request fails after navigation: its local chat must remain reachable and retryable (Task 1).
- A load or Settings save completes after navigation: it must not overwrite a newer chat (Task 1).
- Approval resolution races request completion: it must not clear another chat's approval (Task 1).
- Background output arrives during Settings input or active scrolling: preserve input identity, focus, edits, and scroll (Task 2).
- An execution fails or a tool waits for approval: release request guards on failure without blocking independent chats (Task 3).

### Task 1: Own requests by chat session

**Files:** Modify `desktop/src/types.ts`, `desktop/src/chat-state.ts`, `desktop/src/chat-state.test.ts`, `desktop/src/chat-composer.ts`.

**Interfaces:** Produce `ChatSessionSummary` and `ChatState.activeSessionId`, `ChatState.sessions`; expose `ChatStore.selectSession(id: string): Promise<void>` for local and saved sessions. Keep `selectConversation(id)` and existing active-chat fields for view compatibility.

- [x] Write failing tests for two deferred requests completing out of order, switching back while streaming, first-request failure/retry, independent approvals without global cancellation, stale loads, draft retention, and Settings saves during navigation.
- [x] Run `npm --prefix desktop test -- src/chat-state.test.ts`; observe failures caused by navigation guards or missing session behavior.
- [x] Implement session snapshots owned by a map, a stable active identity, and per-request generation. Publish selected session fields plus global summaries; never mutate active session state from background callbacks. Snapshot settings and workspace at request start. Route approval resolution to the captured owner. Invalidate pending loads when sending or navigating.
- [x] Run the state suite and update old tests that intentionally asserted global locking/cancellation to assert the approved behavior instead. Expected: all state tests pass.

### Task 2: Keep navigation and rendering responsive

**Files:** Modify `desktop/src/chat-view.ts`, `desktop/src/chat-view.test.ts`, `desktop/src/styles.css`; keep composer input behavior compatible.

**Interfaces:** Consume Task 1's summaries and `selectSession`. Preserve synchronous initial/navigation rendering; schedule streaming output patches with `requestAnimationFrame` and cancel queued frames at unmount/navigation.

- [x] Add failing view tests for enabled navigation/Settings while streaming, reachable local running sessions and approval/error badges, independent request completion, Settings input node/focus preservation, frame batching, and scroll position preservation.
- [x] Run `npm --prefix desktop test -- src/chat-view.test.ts`; observe the expected disabled-button and rendering failures.
- [x] Render local sessions alongside saved chats without duplicates. Keep only active request mutations restricted. Restore draft text on session changes; late send completion cannot replace another session's composer. Patch streaming answer/activity and sidebar statuses without reconstructing unrelated DOM. Reuse Settings DOM when only background session data changes. Follow bottom only if already near it.
- [x] Run frontend tests and `npm --prefix desktop run build`. Expected: tests pass and TypeScript/Vite build succeeds.

### Task 3: Protect native request concurrency and finish

**Files:** Create `desktop/src-tauri/src/requests.rs`; modify `desktop/src-tauri/src/lib.rs`, `desktop/src-tauri/src/commands.rs`, `README.md`, `AGENTS.md`.

**Interfaces:** `ActiveRequests::acquire(self: &Arc<Self>, id: Option<&ConversationId>) -> Result<RequestGuard, String>` rejects a duplicate saved ID until `RequestGuard` drops. Add it to `DesktopState` and acquire inside request orchestration before resolving workspace/history. Wrap blocking persistence/settings commands with an async `spawn_blocking` helper without changing invoke names or payloads.

- [x] Add failing bridge tests for duplicate saved request exclusion, independent saved requests, failure cleanup, and two pending approvals resolved independently; use fake runtimes, no external providers.
- [x] Run the desktop Rust tests; observe expected failures before guard integration.
- [x] Implement the request guard and background command wrappers. Reuse existing store locks only for short persistence operations; keep guards out of model/provider interfaces.
- [x] Run core and desktop formatting/tests and Clippy, frontend tests/build, and Tauri debug build. Expected: all commands succeed; report any real environment blocker precisely.
- [x] Update README.md and AGENTS.md with session lifetime, concurrent chats, approval ownership, and per-chat restrictions.
- [x] Review the entire change with a fresh reviewer; fix significant findings with failing regression tests first. Review staged diff and create one focused implementation commit after required core formatting/tests pass.
