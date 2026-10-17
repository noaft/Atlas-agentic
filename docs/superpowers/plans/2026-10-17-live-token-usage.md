# Live Token Usage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Populate the approved context ring with the latest chat-model call's real input plus output tokens, including all provider-counted prompt content.

**Architecture:** Add a typed response carrying optional model usage while preserving existing string-returning provider/agent APIs through default methods. Forward usage through runtime events, save the latest reading in successful activity sidecars, and restore it into the owning frontend session. The ring uses a reading only for the selected model and displays input/output details.

**Tech Stack:** Rust, serde, reqwest SSE, Tauri channels, TypeScript, Vitest.

**Spec:** User-approved ring UI and explicit instruction to show the latest call's full token usage. No separate spec file; this wires metadata into that existing flow.

## Global Constraints

- Provider-reported prompt tokens already include system prompts, history, tools, and current input. Never add them again.
- Use latest chat call usage, not cumulative billing consumption. Title generation cannot replace the chat reading.
- Preserve absent usage as unknown; cached/reasoning details are subsets and are not additional tokens.
- Save usage only with successful turns; preserve legacy sidecar compatibility and screenshot privacy.
- Keep background requests and model selections isolated; never show an old model's usage as the selected model's reading.
- Use fake providers or loopback HTTP in tests. One focused implementation commit and the established local merge workflow.

## Review Focus

- Usage-only terminal SSE chunk with empty choices or no trailing newline (Task 1).
- UTF-8 split across network reads and interrupted streams (Task 1).
- Tool continuation or repair request replaces, rather than sums, prior usage (Task 2).
- Newest turn with missing usage must not resurrect older counts (Task 2).
- Concurrent chats, model changes, history reloads, and failed turns must not mix readings (Task 3).

### Task 1: Capture provider usage

**Files:** `src/models/mod.rs`, `src/models/deepseek.rs`, provider tests.

**Interfaces:** `ModelUsage { model: String, input_tokens: u64, output_tokens: u64, total_tokens: u64 }`; `ModelResponse { content: String, usage: Option<ModelUsage> }`; `ModelProvider::stream_with_usage(request, on_delta) -> Result<ModelResponse, String>` defaults to wrapping existing streaming with unknown usage.

- [x] Add failing loopback tests for real streaming usage, empty terminal choices, missing/null/invalid usage, UTF-8, and final unterminated data line.
- [x] Run tests to establish failures, then implement line-based UTF-8-safe SSE parsing. Request `stream_options.include_usage` only for streaming. Validate usage sums without failing an otherwise valid answer for missing/invalid metadata.
- [x] Run core tests and formatting; expected all pass.

### Task 2: Route and persist latest usage

**Files:** agent traits/local agent, runtime, permissions, conversation service/activity, Tauri commands, affected existing fixtures, integration tests.

**Interfaces:** Add a richer agent streaming method with the existing context/image arguments and default wrapping behavior. Add `ToolEvent::Usage { usage: Option<ModelUsage> }`; add optional `usage` to activity turns and ask results, and corresponding camelCase desktop DTOs/events.

- [x] Add failing end-to-end fake-provider tests for tool-call replacement, latest missing usage, persistence/reload, legacy sidecars, and title exclusion.
- [x] Forward usage after each successful model call, including unknown, without turning it into activity summary text. Preserve typed string APIs and fake providers. Copy latest usage into the successful activity batch and ask result.
- [x] Test core and desktop bridge, including wire serialization. Expected all pass.

### Task 3: Feed the ring and finish

**Files:** desktop types, chat state, context meter, view, their tests; README.md and AGENTS.md.

- [x] Add failing tests proving live usage updates per session, successful completion/reload retains counts, missing usage clears stale counts, and model mismatch shows unknown.
- [x] Keep `contextUsage` owned by the chat; handle the usage event and restore newest activity usage. Feed total/input/output to the existing ring and label it as the last model call.
- [x] Run formatting, all core/desktop/frontend tests, both Clippy commands, frontend build, and Tauri debug build. Update docs, request independent review, fix significant findings with regressions, review staged diff, and commit.
