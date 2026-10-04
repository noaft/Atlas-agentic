# Local Conversations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add user-local, multi-conversation history with one-time LLM titles, bounded role-preserving context, CRUD commands, and a stronger command-oriented default Atlas prompt.

**Architecture:** A provider-neutral `conversations` module owns message types, JSONL persistence, metadata, locking, and workflow orchestration. Runtime and agent requests accept typed prior messages; the CLI selects create-versus-continue behavior and exposes management commands. The existing provider boundary translates history to DeepSeek messages, while title behavior stays configured in the local agent TOML.

**Tech Stack:** Rust, Serde/serde_json, `directories` for OS-local paths, `uuid` for IDs, `chrono` for RFC 3339 UTC timestamps, `fs2` for file locking, existing blocking DeepSeek provider.

**Spec:** `docs/superpowers/specs/2026-10-04-local-conversations-design.md`

## Global Constraints

- `atlas ask <prompt>` always creates a new conversation unless `--conversation <uuid>` is supplied.
- Persist only final `user` and `assistant` messages; never persist tool calls, tool results, or partial output.
- Store data under the current user's OS application-data directory, never in the repository.
- Allow `ATLAS_DATA_DIR` to override the user-data root for deterministic development and tests.
- Store one JSONL history per UUID conversation plus an atomically replaced version-1 metadata index.
- Keep complete disk history; send at most 40 saved messages by default, overridden by positive `ATLAS_HISTORY_MESSAGES`.
- Preserve message roles and chronological order; do not flatten history into text.
- Generate a title once after the first successful answer, using only the first user input and no tools.
- Title failure must not fail or remove a successful conversation; retain `Untitled conversation` and set `title_attempted`.
- Stream final answers by default and keep tool protocol hidden.
- Keep title and default-agent behavior in `agents/atlas.toml`; do not hard-code tool names or schemas there.
- Every task follows red-green-refactor, runs formatting/tests, and creates one focused commit without unrelated files.

## Review Focus

- A malicious or malformed conversation ID such as `../../file` must be rejected before any path is constructed (Task 2 test).
- An odd history limit that cuts a turn must not send an orphan assistant message at the start of model context (Task 1 test).
- Two writers targeting the same store must serialize history/index mutations instead of interleaving JSONL records (Task 2 test).
- Multiline and Unicode message/title content must round-trip and truncate on Unicode characters, not bytes (Tasks 2 and 4 tests).
- A malformed final JSONL line or corrupt index must return a path-and-line-aware error without silently discarding stored content (Task 2 test).

---

### Task 1: Conversation Types and Context Selection

**Files:**
- Create: `src/conversations/mod.rs`
- Create: `src/conversations/types.rs`
- Create: `tests/conversation_context_behavior.rs`
- Modify: `src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Produces: `ConversationId::new()`, `ConversationId::parse(&str)`, `MessageRole::{User, Assistant}`, `ConversationMessage { role, content, created_at }`, `ConversationMetadata`, `Conversation`, `DEFAULT_HISTORY_MESSAGES: usize = 40`, `history_limit_from_env() -> Result<usize, String>`, and `select_recent_history(&[ConversationMessage], usize) -> Vec<ConversationMessage>`.
- Consumes: `chrono::DateTime<Utc>` and `uuid::Uuid`; no provider or filesystem types.

- [ ] **Step 1: Write failing domain tests**

Add tests proving UUID generation/parsing, rejection of path-like IDs, default and overridden history limits, rejection of zero/non-numeric limits, chronological selection of at most 40 messages, and removal of a leading orphan `Assistant` when an odd limit cuts a turn.

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test --test conversation_context_behavior`

Expected: compilation fails because `atlas_agentic::conversations` does not exist.

- [ ] **Step 3: Add domain dependencies and exact types**

Add `chrono` with Serde support and `uuid` with v4 support. Implement the interfaces above. `ConversationId::parse` must round-trip through `Uuid::parse_str`; filesystem code receives only a validated `ConversationId`.

- [ ] **Step 4: Implement bounded context selection**

Select the newest `limit` saved messages, preserve chronological order, and drop a leading assistant message when selection begins mid-turn. The new input remains outside this function.

- [ ] **Step 5: Verify GREEN**

Run: `cargo fmt --check && cargo test --test conversation_context_behavior && cargo test`

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/conversations/mod.rs src/conversations/types.rs tests/conversation_context_behavior.rs
git commit -m "feat: add conversation domain types"
```

### Task 2: User-Local JSONL Conversation Store

**Files:**
- Create: `src/conversations/store.rs`
- Create: `tests/conversation_store_behavior.rs`
- Modify: `src/conversations/mod.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: Task 1 domain types.
- Produces: `ConversationStore::from_user_data() -> Result<Self, String>`, `ConversationStore::at(PathBuf) -> Self`, `create_turn(&self, user: &str, assistant: &str) -> Result<ConversationMetadata, String>`, `append_turn(&self, id: &ConversationId, user: &str, assistant: &str) -> Result<ConversationMetadata, String>`, `load(&self, id: &ConversationId) -> Result<Conversation, String>`, `list(&self) -> Result<Vec<ConversationMetadata>, String>`, `rename(&self, id: &ConversationId, title: &str) -> Result<ConversationMetadata, String>`, `record_title_attempt(&self, id: &ConversationId, title: Option<&str>) -> Result<ConversationMetadata, String>`, and `delete(&self, id: &ConversationId) -> Result<PathBuf, String>`.

- [ ] **Step 1: Write failing storage layout and round-trip tests**

Using `tempfile`, assert that `create_turn` creates `conversations.json` version 1 plus `<id>.jsonl`, persists exactly one user and one assistant line, preserves multiline/Unicode content, and returns fallback title `Untitled conversation` with `title_attempted == false`.

- [ ] **Step 2: Write failing CRUD and ordering tests**

Assert append updates message count/time, load returns messages in order, list sorts newest first, rename changes only the title, `record_title_attempt` sets the flag and optional sanitized title, and delete moves history into `trash` before removing metadata.

- [ ] **Step 3: Write failing safety and recovery tests**

Assert invalid IDs cannot form paths, absent IDs return `conversation not found: <id>`, concurrent appends produce valid non-interleaved JSONL, corrupt index errors name `conversations.json`, malformed JSONL errors name its path and one-based line, and stale metadata is reconciled from valid history before the next mutation.

- [ ] **Step 4: Run storage tests and verify RED**

Run: `cargo test --test conversation_store_behavior`

Expected: compilation fails because `ConversationStore` is missing.

- [ ] **Step 5: Add path and locking dependencies**

Add `directories` and `fs2`. `from_user_data` uses `ATLAS_DATA_DIR` when set, otherwise `directories::BaseDirs::data_local_dir().join("atlas")`; unit tests use `at(tempdir)`.

- [ ] **Step 6: Implement locked JSONL and atomic index writes**

Hold `<root>/conversations.lock` for every mutation. Serialize each message to one JSON line, flush history before updating metadata, then write/flush a sibling temporary index and rename it over `conversations.json`. Parse every line strictly and reconcile count/latest timestamp rather than silently dropping records.

- [ ] **Step 7: Verify GREEN**

Run: `cargo fmt --check && cargo test --test conversation_store_behavior && cargo test`

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/conversations/mod.rs src/conversations/store.rs tests/conversation_store_behavior.rs
git commit -m "feat: add local conversation store"
```

### Task 3: Role-Preserving Model and Runtime History

**Files:**
- Modify: `src/models/mod.rs`
- Modify: `src/models/deepseek.rs`
- Modify: `src/agents/mod.rs`
- Modify: `src/agents/local.rs`
- Modify: `src/runtime.rs`
- Modify: `tests/models_behavior.rs`
- Modify: `tests/tool_call_behavior.rs`

**Interfaces:**
- Consumes: `ConversationMessage` and `MessageRole` from Task 1.
- Produces: `ModelRequest.history: &[ConversationMessage]`; history-aware `Agent::process`/`process_stream`; `Runtime::ask_stream_with_history(prompt, history, on_delta)` while retaining `ask_stream` as the empty-history convenience method.

- [ ] **Step 1: Write failing provider request tests**

Capture a fake provider request and assert the exact history roles/content and newest user input remain separate. Add a DeepSeek request-construction test asserting message order `system, user, assistant, user` for one saved turn plus new input.

- [ ] **Step 2: Write failing runtime tool-loop history tests**

Assert the same saved history is supplied on the initial model call and every internal tool-continuation call, while tool results remain internal and final content still streams incrementally.

- [ ] **Step 3: Run focused tests and verify RED**

Run: `cargo test --test models_behavior && cargo test --test tool_call_behavior`

Expected: compilation fails because `ModelRequest` and runtime do not accept history.

- [ ] **Step 4: Thread typed history through boundaries**

Add the exact interfaces above. DeepSeek maps `User` to `"user"` and `Assistant` to `"assistant"`; it inserts history after the system message and before the current input. Keep provider-specific message structs inside `deepseek.rs`.

- [ ] **Step 5: Verify GREEN and streaming regression safety**

Run: `cargo fmt --check && cargo test --test models_behavior && cargo test --test tool_call_behavior && cargo test`

Expected: all tests pass, including existing streaming/tool protocol tests.

- [ ] **Step 6: Commit**

```bash
git add src/models/mod.rs src/models/deepseek.rs src/agents/mod.rs src/agents/local.rs src/runtime.rs tests/models_behavior.rs tests/tool_call_behavior.rs
git commit -m "feat: pass conversation history to agents"
```

### Task 4: One-Time LLM Titles and Enhanced Default Agent

**Files:**
- Create: `src/conversations/title.rs`
- Modify: `src/conversations/mod.rs`
- Modify: `src/agents/mod.rs`
- Modify: `src/agents/local.rs`
- Modify: `agents/atlas.toml`
- Modify: `tests/agents_behavior.rs`
- Create: `tests/conversation_title_behavior.rs`

**Interfaces:**
- Consumes: existing `ModelProvider` and Task 1 types.
- Produces: required `LocalAgentDefinition.conversation_title_prompt: String`, `Agent::generate_conversation_title(first_input: &str) -> Result<String, String>`, and `sanitize_title(raw: &str) -> Option<String>`.

- [ ] **Step 1: Write failing title-generation tests**

Assert `LocalAgent` sends the configured title prompt as the system prompt, only the first input as user input, empty history, and no tools. Assert provider errors propagate from this method for the workflow to downgrade later.

- [ ] **Step 2: Write failing title sanitization tests**

Assert whitespace/line breaks collapse, surrounding single/double quotes are removed, blank output returns `None`, and a Unicode title is limited to 60 characters without splitting a character.

- [ ] **Step 3: Write the failing default-agent prompt test**

Load `agents/atlas.toml` and assert its system prompt directs Atlas to inspect before repository claims, use runtime-provided tools, execute relevant coding/debugging/testing commands, preserve unrelated changes, verify results, and report blockers. Assert it does not contain hard-coded tool protocol JSON or a list of tool names.

- [ ] **Step 4: Run focused tests and verify RED**

Run: `cargo test --test agents_behavior && cargo test --test conversation_title_behavior`

Expected: tests fail because the title prompt/method/sanitizer and enhanced guidance are absent.

- [ ] **Step 5: Implement title generation and prompt config**

Add the required TOML field and agent method. The title request uses `ModelProvider::complete`, title system prompt, first input, no history, and no tools. Implement sanitizer with Unicode-character truncation.

- [ ] **Step 6: Enhance `agents/atlas.toml`**

Add the approved command-oriented guidance without embedding tool names, schemas, or call protocol; add the dedicated title prompt requesting plain text no longer than 60 characters.

- [ ] **Step 7: Verify GREEN**

Run: `cargo fmt --check && cargo test --test agents_behavior && cargo test --test conversation_title_behavior && cargo test`

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add agents/atlas.toml src/agents/mod.rs src/agents/local.rs src/conversations/mod.rs src/conversations/title.rs tests/agents_behavior.rs tests/conversation_title_behavior.rs
git commit -m "feat: enhance Atlas and generate conversation titles"
```

### Task 5: Conversation Ask Workflow

**Files:**
- Create: `src/conversations/service.rs`
- Modify: `src/conversations/mod.rs`
- Modify: `src/runtime.rs`
- Create: `tests/conversation_service_behavior.rs`

**Interfaces:**
- Consumes: `ConversationStore`, history selection, `Runtime::ask_stream_with_history`, `Runtime::generate_conversation_title`, and title sanitizer.
- Produces: `ConversationService::new(&Runtime, &ConversationStore, history_limit)`, `ask(prompt, Option<&ConversationId>, on_delta) -> Result<ConversationAskResult, String>`, and `ConversationAskResult { answer, metadata, created }`.

- [ ] **Step 1: Write failing new-conversation workflow tests**

Assert no ID invokes the runtime with empty history, streams the answer, saves exactly the final user/assistant pair only after success, attempts a title once using the first input, and returns generated metadata with `created == true`.

- [ ] **Step 2: Write failing failure/title fallback tests**

Assert model/tool failure creates no conversation; storage failure returns an error after the model result; title failure retains the saved answer, fallback title, and `title_attempted == true`; a successful title is sanitized once and never requested on continuation.

- [ ] **Step 3: Write failing continuation tests**

Assert an ID loads and selects recent history, preserves roles/order, appends only after a final response, updates metadata, returns `created == false`, and never stores streamed partials or tool protocol/results.

- [ ] **Step 4: Run service tests and verify RED**

Run: `cargo test --test conversation_service_behavior`

Expected: compilation fails because `ConversationService` and runtime title delegation are missing.

- [ ] **Step 5: Implement workflow orchestration**

Keep filesystem behavior in the store and agent/tool behavior in runtime. Add `Runtime::generate_conversation_title` as a default-agent delegation. Catch only title-generation failure; request, tool, and storage errors remain explicit.

- [ ] **Step 6: Verify GREEN**

Run: `cargo fmt --check && cargo test --test conversation_service_behavior && cargo test`

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/runtime.rs src/conversations/mod.rs src/conversations/service.rs tests/conversation_service_behavior.rs
git commit -m "feat: orchestrate persistent conversation asks"
```

### Task 6: CLI CRUD Integration and Documentation

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `tests/cli_behavior.rs`
- Modify: `tests/cli_binary.rs`
- Modify: `README.md`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: `ConversationService`, `ConversationStore::from_user_data`, Task 1 IDs/limits, and existing runtime construction.
- Produces: `AppResult::Ask { prompt, conversation_id }`, `ConversationList`, `ConversationShow { id }`, `ConversationRename { id, title }`, and `ConversationDelete { id }` command variants.

- [ ] **Step 1: Write failing CLI parser tests**

Assert exact parsing for new ask, `ask --conversation <uuid> <prompt>`, `--stream` and `--conversation` in either order before the prompt, list/show/rename/delete, and clear errors for missing ID, missing prompt/title, invalid UUID, unknown flags before the prompt, and extra management-command arguments.

- [ ] **Step 2: Write failing binary behavior tests**

Use `ATLAS_DATA_DIR` with a temporary directory. Assert conversation management commands do not require `DEEPSEEK_API_KEY`; list/show/rename/delete render stable human-readable output; a new ask prints the conversation ID/title metadata after the streamed answer; continuation uses the supplied ID.

- [ ] **Step 3: Run CLI tests and verify RED**

Run: `cargo test --test cli_behavior && cargo test --test cli_binary`

Expected: tests fail because the new variants and command handling are absent.

- [ ] **Step 4: Implement CLI parsing and command routing**

Parse before constructing a provider so help and local management commands work without an API key. Construct the model/runtime only for `Ask`. Resolve `ATLAS_HISTORY_MESSAGES`, open the user-local store, and route asks through `ConversationService`.

- [ ] **Step 5: Render management and ask metadata output**

Keep streamed answer content on stdout. After completion print one metadata line containing conversation ID and title. Render list newest-first and show saved role/content/timestamps. Report recoverable deletion location without exposing unrelated paths.

- [ ] **Step 6: Update repository documentation**

Document all commands, automatic-new behavior, explicit continuation, OS-local storage, recoverable delete, one-time LLM titles, `ATLAS_HISTORY_MESSAGES`, source-tree additions, and the enhanced default agent.

- [ ] **Step 7: Run complete verification**

Run: `cargo fmt --check && cargo test && cargo clippy --all-targets --all-features -- -D warnings && git diff --check && git status --short`

Expected: all commands exit 0; status contains only this task's intended files before commit.

- [ ] **Step 8: Commit**

```bash
git add src/lib.rs src/main.rs tests/cli_behavior.rs tests/cli_binary.rs README.md AGENTS.md
git commit -m "feat: add conversation management CLI"
```

### Task 7: Whole-Feature Review and Acceptance

**Files:**
- Modify only files required by verified review findings.

**Interfaces:**
- Consumes: all prior task interfaces and the approved spec.
- Produces: a reviewed, clean branch whose CLI, persistence, title, history, streaming, and tool behavior satisfy the spec together.

- [ ] **Step 1: Run end-to-end acceptance tests in an isolated data directory**

Run the deterministic service and CLI-binary suites with their fake providers/temporary `ATLAS_DATA_DIR`. Confirm they create two conversations, continue one by ID, list/show/rename/delete it, and prove a no-ID ask creates a third conversation rather than reusing either existing one.

- [ ] **Step 2: Request whole-feature code review**

Review against the spec with focus on data loss, protocol leakage, path traversal, concurrency, context ordering, title retries, and CLI commands that accidentally require an API key.

- [ ] **Step 3: Fix Critical/Important findings test-first**

For each accepted finding, add a focused failing regression, run it to verify RED, implement the minimal correction, and rerun its owning test suite.

- [ ] **Step 4: Run final verification**

Run: `cargo fmt --check && cargo test --all-targets && cargo clippy --all-targets --all-features -- -D warnings && git diff --check && git status --short`

Expected: all commands exit 0 and the worktree is clean after any review-fix commit.

- [ ] **Step 5: Commit review fixes if needed**

```bash
git add <only-reviewed-fix-files>
git commit -m "fix: address conversation workflow review"
```

Skip this commit when review requires no changes.
