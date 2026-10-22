# Multi-provider settings, model discovery, and usage

Status: approved and implemented. See the implementation plan for verification and
compatibility decisions.

## Intent and scope

The user wants Atlas settings to support popular providers beyond DeepSeek,
including listing models and showing model usage. The user approved the proposed
scope: DeepSeek, OpenAI, Anthropic, Gemini, and custom OpenAI-compatible endpoints.
Interpret usage as reported input/output tokens and context occupancy. Billing,
account-wide usage, automatic routing, and multiple simultaneous profiles for
one provider are outside this change.

Success means a user can configure each provider independently, select a provider
and model, run ordinary chat and tools, and see correctly attributed usage.
Existing DeepSeek settings and conversations continue to load. Provider changes
affect subsequent requests; running requests retain their starting configuration.

## Current implementation

- `src/settings.rs` stores only `deepseek` credentials, URL, and model.
- `src/models/mod.rs` has a shared `ModelProvider` trait but a DeepSeek-only factory.
- `src/models/deepseek.rs` implements discovery, streaming, images, summaries,
  and a conservative byte-based context estimate. Its request format includes
  provider-specific fields.
- Runtime tools use Atlas's JSON response protocol. Native provider tool calling
  is not required to extend provider support.
- Desktop DTOs and settings drafts expose one credential set. Model switching
  writes `saved.deepseek.model`.
- Usage and model-limit overrides identify models only by name. Identical names
  on different endpoints can incorrectly share a displayed reading or limit.

## Approaches considered

1. Change only the DeepSeek base URL. Small, but cannot express native Anthropic
   and Gemini protocols or safely isolate DeepSeek-specific request fields.
2. Native adapters behind `ModelProvider`, with a shared settings and discovery
   boundary. Recommended: preserves runtime behavior and gives each protocol
   explicit request, stream, image, and usage handling.
3. Add a gateway dependency for every provider. Adds a service dependency and
   credential boundary the user did not request.

Use approach 2. Share protocol-neutral prompt construction and HTTP/SSE utilities;
keep provider payloads, authentication, and finish-reason mapping in adapters.

## Settings and configuration

Introduce settings version 2 with optional `selectedProvider` and a `providers`
map keyed by validated IDs: `deepseek`, `openai`, `anthropic`, `gemini`, and
`openai-compatible`. Each profile stores API key, base URL, optional selected
model, model-limit overrides, and explicit optional request capability settings.
The compatible profile additionally permits keyless endpoints and opt-in JSON
mode/stream usage support. Its initial defaults omit optional extension fields.

Keep endpoint defaults and environment variable names in a bundled provider
configuration file, loaded through typed descriptors. Do not hard-code a current
model catalog or new default model names. New provider profiles initially require
discovery selection or manual model entry before chat. Discovery itself requires
no selected model. Custom endpoints require an explicit base URL.

Resolution order is explicit `ATLAS_PROVIDER`, saved selected provider, then the
agent definition's provider. For that provider, resolve API key, base URL, and
model from its environment variables over saved fields. Fall back to the agent's
model only when its provider matches the resolved provider. Keep all existing
`DEEPSEEK_*` variables. Use `OPENAI_*`, `ANTHROPIC_*`, `GEMINI_*`, and
`ATLAS_COMPATIBLE_*` for their respective profiles, documenting exact names.
Never let another provider's environment key or model override the active one.

Model selection must distinguish unset from explicitly saved values; remove the
current equality-to-default heuristic. Preserve the version-1 DeepSeek fallback
behavior during migration, including the bundled agent default and explicit
environment override. Migrate version-1 model limits into the DeepSeek profile.
Load migration in memory; publish version 2 only during an explicit successful
settings update. Reject unsupported future versions without overwriting files.

Keep atomic updates, private modes, locks, and redacted Debug output. Settings
views expose key source and configuration status, never stored key values.
Blank key input preserves a key; explicit clear affects only the named profile.
Partial updates preserve inactive profiles and global workspace/history/access
settings. Validate URLs as HTTP(S), rejecting embedded credentials, query strings,
and fragments; HTTP remains available for local endpoints and loopback tests.

## Provider adapters and runtime

Retain DeepSeek's existing wire behavior. Add native OpenAI Responses,
Anthropic Messages, and Gemini generateContent adapters, plus a separate
OpenAI-compatible Chat Completions adapter. A compatible endpoint supports the
documented Chat Completions contract; arbitrary services and non-chat models are
not implicitly supported. No automatic protocol retry that could duplicate work.

Extract memory/activity/tool instruction rendering into a neutral model module.
All adapters send the same Atlas runtime tool contract, historical memory blocks,
and selected messages. They normalize text into existing runtime parsing rather
than executing provider-native tools. Preserve one no-tool title request and
separately bounded summary calls. Disable remote response storage for OpenAI.

Each adapter implements non-streaming completion, incremental text streaming,
discovery, optional usage, image validation/serialization, request estimates, and
summary finish-reason normalization. Shared SSE framing must support event names,
multi-line data, blank boundaries, chunk splits, comments, and explicit error
events. Unexpected EOF or protocol errors fail the turn rather than publishing a
partial successful answer. Never forward reasoning text as visible answer text.

Do not blindly send the bundled agent's temperature and top-p to every model.
Provider metadata capabilities, or explicit profile capability settings when
metadata is absent, govern optional sampling/JSON/reasoning fields. Unknown
capabilities omit optional sampling fields. DeepSeek retains current behavior.
Profile capability options are optional booleans for temperature, top-p, JSON
mode, stream usage, and image input. Known metadata governs supported features;
explicit false can disable one, while true cannot override a known rejection.
For APIs requiring an output bound on ordinary calls, use the request's explicit
bound, otherwise the profile's optional request output bound, otherwise a bounded
default from the provider descriptor configuration. Clamp to any known output
limit and validate positive values. This default is an output request setting,
never inferred model capacity. Summary requests always use their explicit bound.
Summary calls require a known bounded output parameter; normalize successful
completion to `stop`, output exhaustion to `length`, and reject refusals, empty
content, and other finishes through the existing compaction checks. Keep
DeepSeek's thinking-disable behavior in its adapter.

Image support is adapter-specific. Validate model capabilities when reported;
unsupported or unknown custom image capabilities return an actionable error.
Captures remain in memory. Unknown image token accounting explicitly disables
automatic compaction for the affected request, as it does today.

## Model discovery and limits

Discovery operates on a resolved provider profile independently of runtime
construction. Handle provider pagination with bounded pages, deduplicate IDs,
reject empty IDs, and retain display name and optional capabilities/limits.
For Gemini retain the resource name required for requests and filter models
whose declared generation methods do not support chat generation.

Keep model option text to model names. Retain a configured/manual model when
discovery is unavailable, and show discovery errors with Retry. An empty model
list does not erase selection or block ordinary chat with a valid manual model.

Represent input-token limits separately from combined context limits. Only an
explicit combined limit can be the context ring's denominator or enable the
existing combined context-budget policy. Never add input/output limits together
to invent a capacity. Preserve provider-reported output limits and show
input-only limits in model details. Users can supply explicit combined context
and output overrides to enable budgeting where metadata is insufficient.
Missing and zero values remain unknown; never infer capacity from model names.

Limits are scoped to provider endpoint and model. Store profile overrides with
their normalized endpoint identity so changing a URL cannot silently reuse the
old endpoint's limits. A stable identity hashes provider ID and normalized base
URL, excluding secrets. Use the same identity in discovery results and usage.

## Usage normalization and persistence

Extend `ModelUsage` with optional provider/source identity for backward-compatible
sidecar loading. New readings always include source identity and requested model
ID. Preserve any returned model ID separately if needed for diagnostics; aliases
must not cause the reading to disappear for the requested selection.

Normalize only complete, nonnegative integer counts with checked arithmetic:

- DeepSeek/Chat Completions: prompt and completion counts; cached/reasoning
  breakdowns are already included and must not be added again.
- OpenAI Responses: reported input, output, and total counts; reasoning and
  cached tokens remain subsets.
- Anthropic: input includes uncached, cache-read, and cache-creation input
  categories once; output follows the final cumulative output reading. Merge
  stream updates rather than summing cumulative events.
- Gemini: prompt input already includes cached content; output includes candidate
  and thought tokens once. Include any separate tool-use prompt count in input
  when present. Check the normalized sum against the reported total.

Absent or inconsistent usage becomes unavailable, never zero or an estimate.
Stream snapshots replace prior snapshots rather than accumulating them. The
ring continues to use the latest chat-model call only, excluding title/summary
usage and repeated tool continuations. Preserve separate summary usage.

Display a reading only when source identity and requested model match the active
selection. Legacy readings without identity still load but display unknown when
their source cannot be established. Restore only the newest saved turn's usage;
never search older turns to fill missing readings. Do not persist credentials,
provider response bodies, images, or transient discovery data in sidecars.

## Desktop behavior

Settings adds a provider selector, per-provider credential/base URL/model fields,
Refresh models, manual model entry, and optional model limits. Preserve unsaved
drafts per provider while switching the settings selector. Saving validates and
atomically updates edited profiles and selected provider. Cancel discards all
draft edits. API-key source and environment override hints name the active
provider's exact variable. Environment-controlled fields remain protected.

The composer shows the active provider beside the existing model selector.
Choosing another provider goes through Settings; choosing a model changes only
that provider's selection. Keep all backend access in `atlas-api.ts`.
Discovery and save responses carry source identity/request generation so stale
responses cannot replace a newer provider's catalog or draft. Failed refresh
does not overwrite chat input. Existing sessions, concurrent requests, folder
authority, approvals, task state, focus, and scroll behavior remain intact.

The context ring exposes input, output, total, combined maximum when known, and
provider/model attribution on hover, focus, or tap. Without a verified combined
maximum, show an em dash for occupancy while retaining available token details.

## Errors and testing

Validate provider IDs, URL, key requirements, model, and limit overrides before
requests. Report provider and operation with sanitized HTTP/protocol errors.
Never echo authorization values, image payloads, prompts, or full provider error
bodies. Provider errors do not save failed ordinary turns or partial checkpoints.

Write focused failing tests before product behavior changes. Use loopback fake
servers and child-only environment setup; no real keys or external model calls.
Cover migration, future-version rejection, credential isolation, precedence,
keyless compatibility, malformed URLs, provider factory dispatch, model-less
discovery, pagination, limit semantics, images, summary completion/truncation,
SSE fragmentation/errors/EOF, cumulative and cache/reasoning usage, and missing
or overflowing counts. Add desktop tests for profile drafts, manual models,
stale catalog responses, provider-aware model changes and usage restoration,
and concurrent requests retaining their starting settings.

Run the repository's Rust and desktop format/test/lint/build checks, including
the Tauri debug build. Report environmental blockers separately from failing
behavior. Update README and AGENTS for configuration, protocols, migration,
model discovery, and usage semantics. Product direction does not change.
Finish implementation with one focused commit after reviewing staged changes.

## Official protocol references checked 2026-10-22

- [OpenAI text generation](https://developers.openai.com/api/docs/guides/text)
- [OpenAI models](https://developers.openai.com/api/reference/resources/models/methods/list)
- [OpenAI token counting and output usage](https://developers.openai.com/api/docs/guides/token-counting)
- [Anthropic streaming and cumulative usage](https://platform.claude.com/docs/en/build-with-claude/streaming)
- [Anthropic model metadata](https://platform.claude.com/docs/en/api/models/list)
- [Gemini content and usage metadata](https://ai.google.dev/api/generate-content)
- [Gemini model limits](https://ai.google.dev/api/models)
- [DeepSeek model discovery](https://api-docs.deepseek.com/api/list-models/)

## Implementation outcome

Implemented inline after spec and plan approval. A fresh read-only review found
six important issues; regression tests reproduced them and verified the fixes.
Rust, native desktop and frontend suites, format checks, strict Clippy checks,
frontend production build and Tauri debug build pass. Tests use fake loopback
providers; live-provider and platform capture/input behavior were not exercised.

Implementation preserves the existing `EffectiveSettings` snapshot instead of
adding duplicate resolved-profile/config types. Generation alone requires a
model; discovery accepts an unselected model. The catalog bridge retains arrays,
with per-model source identities and request-generation tokens protecting empty
and stale results. Legacy DeepSeek fields remain synchronized at the settings
boundary. A terminal SSE frame may omit its final newline; incomplete answers
still fail. These choices retain compatibility while adding adapter boundaries.
