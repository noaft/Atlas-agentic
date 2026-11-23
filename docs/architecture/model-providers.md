# Model provider and settings contracts

Implemented contracts for `src/models/`, provider profiles, model discovery,
and usage display. See [configuration examples](../guides/reference.md#settings-and-access).

## Adapters and profile migration

Atlas supports provider IDs `deepseek`, `openai`, `anthropic`, `gemini`, and
`openai-compatible`. `config/providers.toml` holds endpoint, environment-prefix,
API-version, and default output-request settings; do not infer model catalogs or
capacities from names. Native OpenAI uses Responses with remote storage disabled,
Anthropic uses Messages, Gemini uses generateContent, and custom compatible
endpoints use Chat Completions. Keep their payloads, finish reasons, image formats,
and usage normalization inside `src/models/`, behind `ModelProvider`.
`prompt.rs` renders shared Atlas tool/memory instructions; `sse.rs` frames streams.
Provider-native tools do not execute outside the existing Atlas runtime protocol.

Settings version 2 adds a selected provider and independent profiles. Retain the
legacy DeepSeek fields as a compatibility boundary; targeted DeepSeek updates
synchronize them. Loading version 1 migrates only in memory, binding old limits
to the original endpoint; explicit settings updates publish version 2 atomically.
Never overwrite unsupported future versions. `ATLAS_PROVIDER` overrides the saved
selection, then the agent supplies a fallback. Provider prefixes `DEEPSEEK`,
`OPENAI`, `ANTHROPIC`, `GEMINI`, and `ATLAS_COMPATIBLE` support `_API_KEY`,
`_BASE_URL`, and `_MODEL`; one provider's variables never configure another.
Only compatible endpoints can opt into keyless requests. API keys must not appear
in settings views, Debug output, provider errors, or conversation artifacts.

## Settings and model discovery

Desktop Settings uses separate Models & providers, MCP connections, Skills, Workspace,
and Permissions pages. Category navigation and Back to chat preserve per-provider
drafts in memory; Save changes stays on the selected page and Discard resets all
Settings edits. Preserve mounted fields, keyboard focus, category scroll positions,
and the asynchronous MCP and Skills panels during navigation and background chat updates.
Desktop Settings supports
transient model discovery without persisting credentials or selecting a model.
Manual model IDs remain usable after discovery errors. Provider catalog responses
carry source identity per model and use generation tokens to reject stale results.
A model change must validate the current source before saving. Keep optional
request capabilities explicit; omit unsupported/unknown sampling fields. Output
request defaults are configurable and are never interpreted as model capacity.

## Source identity and capacity

Provider source identity hashes provider ID plus normalized base URL, excluding
credentials. Scope model-limit overrides and new usage readings to that source.
Usage preserves the requested model ID when an API returns a dated alias target.
Legacy source-less readings remain readable but are not shown as attributed usage.
Input-only limits are separate from combined context limits; never add independent
input/output maxima to invent combined capacity. Keep unknown capacities unknown.

## Preflight token counting

Before each chat inference, including tool/wakeup continuations and protocol repair,
context preparation counts the current provider payload. Native OpenAI uses
`POST /responses/input_tokens`, Anthropic uses `POST /messages/count_tokens`, and
Gemini uses `POST /models/{model}:countTokens` with `generateContentRequest`.
Count projections reuse inference payload builders: memory, selected history,
system instructions, Atlas tool catalog, activity, current continuation/steering
and images are included. Atlas tools remain runtime-protocol text, not native
provider tools. OpenAI text-format configuration is forwarded when present.
Anthropic counts remain marked estimated; count results are not billed chat usage.

These count requests use the configured provider endpoint and credentials; no new
service or tokenizer dependency is required. A single-entry in-memory cache stores
only a SHA-256 payload digest and numeric result, never raw prompts or images.
Changed input invalidates the cache. Count HTTP errors, malformed/missing counts,
and oversized responses fail visibly before inference; there is no silent fallback
for native providers. Requests have a 120-second timeout, a 64 KiB response bound,
no redirects or automatic retries, and cooperative cancellation during send/body
reads. Stop and steering interrupt preflight; steering recounts the updated input
before starting inference without replaying tools.

DeepSeek and custom OpenAI-compatible endpoints retain their explicitly estimated
byte budgets; image accounting there remains unsupported. No count API is assumed
for an arbitrary compatible endpoint. Counting still runs when model capacity is
unknown, but cannot establish that the request fits until limits are configured.
Known combined capacity reserves output before inference; counts cannot predict
future generated output. Raw tool results retain existing bounded reads and are
counted when included in the next request, not predicted before executing a tool.
Provider count APIs are also used by existing summary-budget probes.

Official API references: [OpenAI](https://developers.openai.com/api/docs/guides/token-counting),
[Anthropic](https://platform.claude.com/docs/en/build-with-claude/token-counting),
[Gemini](https://aistudio.google.com/api-reference/tokens/count-tokens),
[DeepSeek offline counting](https://api-docs.deepseek.com/quick_start/token_usage/).

## Usage display

Model discovery preserves optional provider-reported `contextWindow` and
`maxOutputTokens` limits in the shared model catalog. Model options contain names
only; a compact context ring beside Send exposes exact used/max token details
on hover, focus, or tap. The ring uses the latest chat-model call's reported
input plus output usage, never the sum of repeated calls or the title-generation
request. Input already includes system instructions, selected history, tools,
and the current request; cached and reasoning breakdowns are subsets, not extra
tokens. Carry optional usage through typed model responses and runtime events,
and save it with successful activity turns. Restore only the newest saved turn's
reading, and show an em dash when usage is absent or belongs to another selected
model. Maximum context includes input and output. Missing or zero limits remain unknown; do not infer limits from
model names or use hard-coded capacities.
