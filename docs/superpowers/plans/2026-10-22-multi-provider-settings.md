# Multi-provider Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans for inline execution, or superpowers:subagent-driven-development only if the user selects delegation. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Configure DeepSeek, OpenAI, Anthropic, Gemini, and a custom OpenAI-compatible endpoint; discover models and display correctly attributed token usage.

**Architecture:** Resolve one provider profile per request and construct an adapter behind `ModelProvider`. Share Atlas prompt composition and SSE framing, while keeping native requests, usage normalization, and finish reasons in adapters. Extend the existing desktop settings and composer without changing runtime tool execution.

**Tech Stack:** Rust, serde, reqwest blocking HTTP, TOML descriptors, SHA-256 source identities, Tauri 2, TypeScript, Vitest.

**Spec:** [Approved design](../specs/2026-10-22-multi-provider-settings-design.md).

## Global Constraints

- Provider IDs: `deepseek`, `openai`, `anthropic`, `gemini`, `openai-compatible`.
- Settings version 2; load version 1 without writing; reject unsupported future versions.
- Resolution: `ATLAS_PROVIDER`, saved selected provider, then agent provider; provider environment fields override saved fields.
- Billing, account-wide usage, automatic routing, and multiple simultaneous profiles for one provider are outside this change.
- New providers require discovery selection or manual model entry; do not hard-code model names or capacities.
- Captures remain in memory. Unknown image token accounting explicitly disables automatic compaction for the affected request.
- Ring usage is the latest chat-model call, excluding title/summary usage and repeated tool continuations.
- Only a verified combined limit supplies the occupancy denominator; missing or zero limits remain unknown.
- Keep all frontend backend access in `atlas-api.ts`; do not broaden Tauri permissions.
- Preserve private settings, atomic publication, per-session ownership, execution leases, and failed-turn persistence rules.
- Tests use loopback fake servers, child-only environment setup, and no real API keys or external model calls.
- Finish implementation with one focused Git commit, following the repository's checks and staged-diff review. Keep task changes uncommitted until that final commit; the user's one-commit rule overrides per-task commit suggestions.

## Review Focus

- A URL changed only by trailing slash should keep identity; a changed host or path must invalidate old usage and limits (Tasks 1 and 7).
- Environment-selected provider changes must not borrow another profile's saved key/model or allow UI writes to override the environment (Tasks 1 and 6).
- A model alias returned as a dated model ID must retain usage for the model the user selected (Tasks 2–5 and 7).
- A provider can emit HTTP 200 and then an SSE error or premature EOF; this must fail without saving partial success (Tasks 2–5).
- Discovery of an unsaved profile must not persist its key, model, or draft, and stale responses must not overwrite newer selections (Tasks 6 and 7).

## File Map and Shared Interfaces

- `config/providers.toml`: endpoint/environment/protocol defaults, including configurable request output defaults of 4096 tokens for new native/compatible adapters. This is a request setting, never model capacity.
- `src/settings/providers.rs`: descriptors, typed provider IDs/profiles/capabilities, source identity, URL validation, environment resolution.
- `src/settings/migration.rs`: private version-1 DTO and migration; storage still lives in `src/settings.rs`.
- `src/models/prompt.rs`, `src/models/sse.rs`, `src/models/usage.rs`: neutral Atlas instructions, framed event reads, validated usage construction.
- `src/models/openai.rs`, `anthropic.rs`, `gemini.rs`, `compatible.rs`: protocol adapters; `deepseek.rs` retains its provider extensions.
- `src/models/discovery.rs`: provider dispatch and bounded discovery helpers.
- Existing `src/models/mod.rs`, `src/app.rs`, `src/settings.rs`: shared public boundaries and construction.
- `desktop/src-tauri/src/provider_settings.rs`: provider DTOs and settings/discovery helpers, called by `commands.rs`.
- Existing frontend `types.ts`, `atlas-api.ts`, `chat-state.ts`, `settings-view.ts`, `chat-view.ts`, `context-meter.ts`, `styles.css`: provider settings, discovery, and usage presentation.
- New Rust tests: `provider_settings_behavior.rs`, `provider_stream_behavior.rs`, `provider_discovery_behavior.rs`, `provider_adapters_behavior.rs`; reuse existing usage, compaction, CLI, desktop suites.

Concrete shared types to establish before adapters:

```rust
enum ProviderId { DeepSeek, OpenAi, Anthropic, Gemini, OpenAiCompatible }
struct ModelSource { provider: ProviderId, source_id: String }
struct ProviderCapabilities {
    temperature: Option<bool>, top_p: Option<bool>, json_mode: Option<bool>,
    stream_usage: Option<bool>, image_input: Option<bool>,
}
struct ScopedModelLimits { source_id: String, limits: ModelLimits }
struct ProviderSettings {
    api_key: Option<String>, base_url: String, model: Option<String>,
    model_limits: BTreeMap<String, ScopedModelLimits>,
    capabilities: ProviderCapabilities, request_output_tokens: Option<u64>,
    allow_keyless: bool,
}
```

`ProviderId` serializes to the five exact IDs above. `ModelSource` serializes in camelCase and contains no URL/key. `EffectiveSettings` retains runtime workspace/history/access fields and adds provider, source, resolved profile fields and capabilities. `ModelInfo` adds source, optional `input_token_limit`, and capabilities; its existing `context_window` means combined capacity only. `ModelUsage` adds optional source with serde default for legacy sidecars. Add no required fields to old persisted activity JSON.

### Task 1: Versioned provider settings and source identity

**Files:** Create `config/providers.toml`, `src/settings/providers.rs`, `src/settings/migration.rs`, `tests/provider_settings_behavior.rs`; modify `src/settings.rs`, `tests/settings_behavior.rs`, and affected Rust fixture constructors.

**Interfaces:** Produce `provider_descriptors() -> Result<Vec<ProviderDescriptor>, String>`, `source_for(provider: ProviderId, base_url: &str) -> Result<ModelSource, String>`, and `AtlasSettings::resolve_for(&self, agent_provider: &str, agent_model: &str, environment: impl Fn(&str) -> Option<OsString>) -> Result<EffectiveSettings, String>`. Keep `resolve_from_env` and `resolve_with` wrappers for current callers using the bundled DeepSeek fallback until Task 6 switches runtime setup.

- [x] **1. Write failing settings tests.** Assert v1 load produces v2 in memory but leaves original bytes unchanged; an explicit save emits v2; private modes and atomic failure recovery still work. Assert a saved non-default v1 model survives, and the legacy default-model heuristic preserves agent fallback.
- [x] **2. Add failing resolution/identity tests.** `source_for(DeepSeek, "https://example.test/v1/") == source_for(DeepSeek, "https://example.test/v1")`; host/path/provider changes differ. Reject URLs with userinfo/query/fragment and unknown provider IDs. `OPENAI_API_KEY` cannot replace a DeepSeek key; `ATLAS_PROVIDER=openai` selects only OpenAI fields; unset model does not inherit a different provider's agent model. Future version load must not write. Keys must not appear in Debug output.
- [x] **3. Run `cargo test --test provider_settings_behavior`.** Expect new behavior failures before implementation, not test harness errors.
- [x] **4. Implement profiles, descriptors and migration.** Use `reqwest::Url` parsing, trailing-slash normalization and SHA-256 over provider plus normalized URL. New profile keys/models start unset. Descriptor environment prefixes are `DEEPSEEK`, `OPENAI`, `ANTHROPIC`, `GEMINI`, `ATLAS_COMPATIBLE`, each with `_API_KEY`, `_BASE_URL`, `_MODEL`. Existing DeepSeek endpoint/model defaults remain compatible. Only custom profiles may enable `allow_keyless`. Validate positive output bounds and endpoint-scoped overrides.
- [x] **5. Adapt consumers/fixtures mechanically to retain compilation.** Preserve settings atomic locks; migrate limits into the old endpoint's DeepSeek profile. Keep current DeepSeek runtime behavior until factory integration. Resolve the legacy default-vs-explicit model distinction once during migration.
- [x] **6. Run `cargo test --test provider_settings_behavior --test settings_behavior` and `cargo test`.** Expect all tests passing; review Task 1 diff and mark completed.

### Task 2: Neutral prompts, complete SSE events, and usage identity

**Files:** Create `src/models/prompt.rs`, `src/models/sse.rs`, `src/models/usage.rs`, `tests/provider_stream_behavior.rs`; modify `src/models/mod.rs`, `src/models/deepseek.rs`, `tests/token_usage_behavior.rs`, `src/conversations/activity.rs`, affected fixtures.

**Interfaces:** Produce `render_model_instructions(request: &ModelRequest) -> String` in the neutral module; `SseEvent { event: Option<String>, data: String }` and `read_sse(reader: &mut dyn BufRead, on_event: &mut dyn FnMut(SseEvent) -> Result<bool, String>) -> Result<(), String>` (callback false stops; adapters validate terminal markers); `checked_usage(source: ModelSource, model: &str, input: u64, output: u64, total: Option<u64>) -> Option<ModelUsage>`. Preserve `ModelProvider`'s existing method signatures.

- [x] **1. Write failing stream tests.** Decode chunk-fragmented multi-line `data`, CRLF, named events and comments; dispatch at blank boundaries; ignore ping/unknown events; reject incomplete framing at EOF. Keep provider-specific terminal validation outside the shared reader.
- [x] **2. Write failing usage tests.** `checked_usage(source, "alias", 1200, 300, Some(1500))` preserves requested model/source; mismatched total and integer overflow return None. Deserialize old activity usage without source successfully; no new field contains a key or URL.
- [x] **3. Run `cargo test --test provider_stream_behavior --test token_usage_behavior`.** Confirm new assertions fail.
- [x] **4. Implement shared modules and extend model types.** Move prompt rendering without altering DeepSeek tool/memory instructions. Update DeepSeek stream parsing to framed SSE, source-aware usage and required terminal completion; preserve its wire payload, image bounds and summary thinking switch. Legacy usage gets `source: None`.
- [x] **5. Run `cargo test --test provider_stream_behavior --test token_usage_behavior --test summary_provider_behavior --test conversation_activity_behavior` and DeepSeek unit tests.** Expect unchanged tool ordering and passing terminal/error tests; review Task 2 diff.

### Task 3: OpenAI Responses and compatible Chat Completions

**Files:** Create `src/models/openai.rs`, `src/models/compatible.rs`, `tests/provider_adapters_behavior.rs`; modify `src/models/mod.rs`.

**Interfaces:** Each adapter exports `new(config: ResolvedProviderConfig) -> Self`, implements `ModelProvider`, and uses Task 2 prompt/SSE/usage helpers. `ResolvedProviderConfig` is the provider/source/key/URL/model/capability/output-bound/limit subset of `EffectiveSettings`; produce it in `src/models/mod.rs` with a fallible conversion that requires a selected model.

- [x] **1. Write failing loopback tests for both adapters.** Check auth and exact endpoint paths; OpenAI payload has `store: false`, instructions and text/image inputs; custom chat payload has messages and omits DeepSeek `thinking`, unknown sampling, JSON and stream-usage options. Keyless custom calls omit Authorization. Explicit capability options enable only their supported fields.
- [x] **2. Add streaming/summary tests.** OpenAI emits only output-text deltas, consumes completed usage and rejects failed/incomplete responses; compatible consumes final usage and completion markers. HTTP 200 followed by error/EOF fails. An API-returned dated model retains requested alias in usage. Bounded summaries reject empty/truncated/refusal results. Verify default/explicit output bound, PNG serialization and unknown image-accounting behavior.
- [x] **3. Run `cargo test --test provider_adapters_behavior`.** Confirm tests fail on absent adapters.
- [x] **4. Implement adapters.** Use native `/responses` for OpenAI and `/chat/completions` for compatible profiles. Apply normalized optional parameters, known output bounds and checked usage. Map summary success/output exhaustion to `stop`/`length`; do not retry a generation via another protocol. Sanitize errors; do not echo bodies or auth values.
- [x] **5. Run `cargo test --test provider_adapters_behavior --test provider_stream_behavior` and `cargo test`.** Expect passing, with no external HTTP; review Task 3 diff.

### Task 4: Anthropic and Gemini native adapters

**Files:** Create `src/models/anthropic.rs`, `src/models/gemini.rs`; modify `src/models/mod.rs`, `tests/provider_adapters_behavior.rs`.

**Interfaces:** Same adapter constructor/config and `ModelProvider` methods as Task 3. Native provider response structs remain private to each adapter.

- [x] **1. Add failing Anthropic tests.** Verify `/messages`, `x-api-key`, configured API version, required output bound, separate system instructions, historical messages, images, visible text deltas, and message stop/error events. Usage input 100 uncached + 200 cache read + 50 cache write becomes 350; cumulative output updates 10 then 30 yield 30, total 380.
- [x] **2. Add failing Gemini tests.** Verify `x-goog-api-key` rather than key query parameters, model resource path, systemInstruction, model/user roles, inline PNG, generationConfig and SSE generation. Prompt 100 including cache 20, tool-use input 5, candidates 30 and thoughts 10 produces input 105/output 40/total 145. Duplicate cumulative usage snapshots must not double counts; absent/inconsistent metadata remains unavailable.
- [x] **3. Add native failure/compaction tests.** Cover blocked/refused candidates, no candidates/text, summary truncation, event errors, EOF without success, alias attribution, unsupported capabilities, zero/negative/overflow counts and image validation. Ensure no partial successful answer or memory checkpoint is published.
- [x] **4. Run `cargo test --test provider_adapters_behavior`.** Confirm newly added tests fail before implementing.
- [x] **5. Implement both adapters.** Normalize native finish reasons and provider usage at the adapter boundary. Keep thought/reasoning blocks out of text. For text-only budgets serialize the actual request and return the same explicitly estimated conservative byte upper bound strategy as existing DeepSeek; image requests return unknown estimate. Store API version and request defaults in descriptors rather than runtime logic.
- [x] **6. Run `cargo test --test provider_adapters_behavior --test compaction_behavior --test summary_provider_behavior` and `cargo test`.** Expect all passing; review Task 4 diff.

### Task 5: Model discovery and capacity semantics

**Files:** Create `src/models/discovery.rs`, `tests/provider_discovery_behavior.rs`; modify `src/models/mod.rs`, all adapters, `src/models/budget.rs` only if necessary to retain separate metadata.

**Interfaces:** Produce `discover_models(profile: &ResolvedProviderProfile) -> Result<Vec<ModelInfo>, String>`, independent of a model selection. `ResolvedProviderProfile` contains resolved endpoint/auth/source/capabilities without requiring a model; `ResolvedProviderConfig` from Task 3 adds selected model. Discovery returns source-tagged `ModelInfo` with separate input-only and combined limits. `ModelProvider::model_limits()` still returns `Option<ModelLimits>` only for verified combined context plus output bound or a matching explicit override.

- [x] **1. Write failing discovery tests for each protocol.** Check authentication, no-model discovery, duplicate/blank IDs, display-name fallback, positive optional metadata, pagination and model list errors. Use max 100 pages and fail on repeated cursors or an exhausted bound. Gemini filters declared unsupported generation methods and preserves `models/...` resource IDs. Empty results remain valid.
- [x] **2. Pin capacity semantics.** Gemini metadata `inputTokenLimit=100000`, `outputTokenLimit=8000` yields input-only 100000, output 8000, combined None. Anthropic max-input metadata also never implies a combined capacity. A matching explicit combined override yields budget limits; wrong source or zero metadata does not. No name-based capacity lookup.
- [x] **3. Run `cargo test --test provider_discovery_behavior`.** Confirm failures before implementation.
- [x] **4. Implement discovery dispatch and metadata mapping.** Consume native list formats; keep OpenAI non-chat models discoverable by name when capabilities are unknown, and return generation errors clearly if manually selected. Do not invent compatibility for every listed model. Cache per-runtime limits without requiring selected-model construction for discovery.
- [x] **5. Run `cargo test --test provider_discovery_behavior --test context_planning_behavior --test compaction_behavior` and `cargo test`.** Expect all passing; review Task 5 diff.

### Task 6: Runtime factory, native settings DTOs, and transient discovery

**Files:** Create `desktop/src-tauri/src/provider_settings.rs`; modify `src/models/mod.rs`, `src/app.rs`, `src/main.rs` where settings resolve, `desktop/src-tauri/src/commands.rs`, `desktop/src-tauri/src/lib.rs`, `tests/models_behavior.rs`, `tests/app_runtime_behavior.rs`, `tests/cli_binary.rs`.

**Interfaces:** Factory dispatches `provider_from_definition_with_settings` by resolved provider; `build_runtime_with_settings` uses one resolved snapshot. Native `ProviderSettingsView` exposes ID/name/source/key-source/saved and effective model/base URL/override flags/capabilities/scoped limits. `SettingsViewDto` adds selected provider and profile views. `ProviderSettingsPatch` explicitly targets one provider with optional key, clear flag, URL, optional model, capabilities, bounds and overrides. `SaveSettingsInput` carries selected provider, profile patches and existing globals. `list_models(request: ModelDiscoveryInput) -> Result<ModelCatalogDto, String>` accepts saved provider or transient profile patch and returns source plus models. `set_model(provider: ProviderId, source_id: String, model: String)` rejects stale endpoints and environment-controlled selection.

- [x] **1. Add failing backend tests.** All five factories dispatch with proper config; missing model blocks chat but permits discovery. Profile save/clear preserves other keys. Environment provider/model fields remain protected with exact variable names. A transient discovery patch never writes settings. Stale source/model writes fail before mutation. Serialized DTOs contain no saved/transient key.
- [x] **2. Add CLI/session regression tests.** Existing DeepSeek CLI tests pass unchanged in behavior; child-only `ATLAS_PROVIDER` and selected profile exercise a fake provider. Missing-agent/config errors remain useful. A background request retains its starting provider after settings change; no added global lock spans HTTP.
- [x] **3. Run `cargo test --test models_behavior --test app_runtime_behavior --test cli_binary` and focused native tests.** Confirm new assertions fail.
- [x] **4. Implement factory and DTO/helpers.** Resolve agent fallback before building a provider, retain workspace ownership and one execution lease, run settings/discovery work on blocking workers, derive active source on the backend. Route commands to the new helper module without moving unrelated conversation command logic. Update existing fixtures for new settings/usage fields.
- [x] **5. Run `cargo test` and `cargo test --manifest-path desktop/src-tauri/Cargo.toml`.** Expect passing; review Task 6 diff.

### Task 7: Provider settings UI, race-safe catalogs, and context usage

**Files:** Modify `desktop/src/types.ts`, `atlas-api.ts`, `chat-state.ts`, `settings-view.ts`, `chat-view.ts`, `context-meter.ts`, `styles.css`, and corresponding `.test.ts` files; add `desktop/src/provider-settings.test.ts`.

**Interfaces:** Mirror Task 6 DTOs in TypeScript. `AtlasApi.listModels(input: ModelDiscoveryInput): Promise<ModelCatalog>` and `setModel(provider: ProviderId, sourceId: string, model: string): Promise<SettingsView>`. `SettingsDraft` holds selected provider and a draft map; draft API keys stay frontend-only until an authorized save/transient discovery request. `ChatStore.selectSettingsProvider(provider: ProviderId): void`, `refreshSettingsModels(): Promise<void>` and provider-aware `selectModel(model: string): Promise<void>` own request-generation tokens. `ContextReading` adds optional provider/model labels and input-only limit for details.

- [x] **1. Write failing settings/adapter tests.** Provider switching preserves each draft, blank keys keep saved keys, clear is profile-local, Cancel discards changes, unknown models can be entered manually, and Refresh uses the transient profile without saving. Verify provider/key/base URL/model controls and environment hints are accessible; missing key does not disable keyless discovery.
- [x] **2. Write failing races/session tests.** Slow provider-A discovery/save results cannot replace provider-B state. Closing Settings invalidates pending draft discovery. Settings changes during chat preserve callbacks, draft text, approvals and workspace of the owning session; background renders retain input focus/scroll.
- [x] **3. Write failing usage/composer tests.** Same model on different source shows unknown; trailing-slash-equivalent source matches; endpoint changes invalidate readings/overrides; legacy source-less usage loads but displays unknown; only latest saved turn is restored. An aliased requested model matches its reading. Input 1200/output 300/combined 6000 shows 25%; missing combined limit shows em dash while exposing 1500 tokens and any input-only limit. Details work on hover/focus/tap; model option text contains names only.
- [x] **4. Run `npm --prefix desktop test`.** Confirm new tests fail before implementation.
- [x] **5. Implement typed bridge and draft/catalog state.** Invalidate request tokens on provider/source/Settings ownership changes. Preserve manual configured models on discovery error; allow ordinary chat when discovery fails. Settings saves only edited profiles plus global fields. Send always remains ordinary chat; preserve task-menu semantics.
- [x] **6. Implement provider Settings controls and usage display.** Show provider beside composer model selection, route provider changes through Settings, and compare model plus backend source identity before displaying usage. Keep existing narrow permissions and animation-frame streaming updates.
- [x] **7. Run `npm --prefix desktop test` and `npm --prefix desktop run build`.** Expect all passing; review Task 7 diff.

### Task 8: End-to-end validation, documentation, and focused commit

**Files:** Modify `README.md`, `AGENTS.md`; any fix stays in the task-owned files above. Mark this plan's completed steps only as verified.

- [x] **1. Check spec coverage against final diff.** Ensure every provider supports ordinary and streamed chat, Atlas tools, titles, bounded summaries, discovery, usage and image handling. Confirm legacy settings/history load, no external tests, no unauthorized persistence/permissions, and no source-less new readings.
- [x] **2. Update README/AGENTS.** Document exact environment names and precedence, version-1 migration, native protocols, keyless/custom options and capability switches, manual selection/discovery errors, request output bounds, source-scoped overrides, input-only versus combined capacity, and latest-call usage semantics. Keep README concise. Product direction is unchanged.
- [x] **3. Run Rust checks.** `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets --all-features -- -D warnings`. All must exit zero; investigate actual regressions rather than hiding failures.
- [x] **4. Run native desktop checks.** `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check`; `cargo test --manifest-path desktop/src-tauri/Cargo.toml`; `cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`. Expect zero exits.
- [x] **5. Run frontend and packaging checks.** `npm --prefix desktop test`; `npm --prefix desktop run build`; `npm --prefix desktop run tauri -- build --debug --no-bundle`. Expect zero exits. Install lockfile dependencies via `npm ci --prefix desktop` only if necessary. Record environmental blockers separately.
- [x] **6. Review the complete change inline.** Check source/identity propagation, URL/auth handling, streaming termination, summary/output budgets, profile patches and request ownership. Independent review requires explicit user delegation; do not spawn an agent automatically.
- [x] **7. Check `git status --short`, stage only task files, run `git diff --cached --check`, and review the staged diff.** Never stage `.env`, secrets, build outputs or unrelated changes. Commit as `feat: support multiple model providers` after required verification succeeds.
- [ ] **8. Report shipped behavior and evidence.** Include commit, tests/build results, and material limits such as custom Chat Completions compatibility or unknown combined capacity; do not claim live-provider verification from fake-server tests.

## Execution record

Implemented inline on `feat/multi-provider` after approval. Focused RED→GREEN
regressions cover settings migration/isolation, native protocols, framed streams,
source attribution, agent fallback, null API error fields, reported output bounds,
known capability restrictions, changed-endpoint limits and discarded discoveries.
Fresh review found six Important issues and no Critical/Minor issues; all six
were fixed in one regression-tested pass.

Verified: `cargo fmt --check`, `cargo test`, strict root Clippy, native format,
47 native tests, strict native Clippy, 206 frontend tests, frontend production
build, and `tauri build --debug --no-bundle`. Existing `screenshots` dependency
reports a future Rust compatibility notice; current checks succeed.

Compatibility decisions:

- Keep legacy DeepSeek fields synchronized with targeted profile updates to
  preserve callers and settings fixtures; this retains a compatibility boundary.
- Reuse `EffectiveSettings` as the resolved snapshot rather than duplicating
  profile/config types. Discovery permits no model; generation validates it.
- Retain catalog arrays, carrying per-model source and request-generation tokens;
  stale/empty results are protected by request ownership rather than an envelope.
- Accept a terminal SSE frame without a final newline for existing endpoints;
  adapter completion checks still reject incomplete answers.
- Exercise providers with loopback fixtures rather than real credentials; live
  accounts/model-specific behavior and platform capture/input remain unverified.

Verification and staged review are complete. The focused implementation commit
is ready for the local integration choice; no remote changes were made.
