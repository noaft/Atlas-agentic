# Atlas Agentic Repository Guide

## What This Repository Is

Atlas Agentic is a Rust project for a desktop-first AI agent system.

The current product includes a Rust CLI and a Tauri desktop app. Both load the
default Atlas agent from `agents/atlas.toml`, share model/settings/runtime code,
and persist multiple local conversations. Atlas is a general assistant for
conversation and OS tasks as well as coding. The desktop app adds an automatic
Atlas home, optional native project-folder selection, per-conversation starting
folder persistence, grouped history, and local Settings.

The planned direction is a multi-agent system with a router that selects the best agent for each user task. The router chooses an agent; it does not perform the task itself.

Read `RULE.md` before changing agent, provider, runtime, router, or tool architecture.

## General Repository Structure

```text
.
├── agents/                 Local agent definitions
│   └── atlas.toml          Default Atlas agent configuration
├── src/
│   ├── main.rs             CLI entrypoint and command execution
│   ├── lib.rs              CLI command parsing and rendering
│   ├── app.rs              Shared runtime and default-tool construction
│   ├── runtime.rs          Agent and tool orchestration
│   ├── agent_output.rs     Final-answer and tool-call parsing
│   ├── agents/             Agent traits and local agent implementation
│   ├── conversations/      Local conversation types, storage, and orchestration
│   │   ├── activity.rs     Bounded activity records, redaction, and model summaries
│   │   ├── types.rs        UUIDs, messages, workspace metadata, and context policy
│   │   ├── store.rs        Locked message/activity JSONL and atomic metadata updates
│   │   ├── service.rs      Ask, activity capture, workspace persistence, and titles
│   │   └── title.rs        Title sanitization
│   ├── tasks/             Authoritative built-in task catalog and validated dispatch
│   ├── settings.rs         Versioned private settings and environment precedence
│   ├── workspace.rs        Automatic Atlas home provisioning and validation
│   ├── models/             Native/compatible adapters, discovery, usage, and budgets
│   └── tools/              Tool trait, registry, and built-in tools
├── desktop/
│   ├── public/             Atlas Compass browser assets
│   ├── src/                Workspace chat state, adapter, chat and Settings UI
│   └── src-tauri/          Workspace/settings bridge, permissions, icons, and config
├── tests/                  Integration and behavior tests
├── Cargo.toml              Rust package configuration
├── README.md               User-facing project documentation
├── VISION.md               Product direction
└── RULE.md                 Coding and architecture rules
```

The built-in runtime tools are:

- `current_time`: show the current date and time from the user's machine, including its local UTC offset
- `pwd`: show the current workspace path
- `ls`: list files and directories inside the workspace
- `read_file`: safely read up to 64 KiB from a UTF-8 file inside the workspace
- `ripgrep`: search file contents with compiled-in ripgrep matching, including
  approved paths outside the workspace
- `search_files`: search workspace filenames and UTF-8 contents
- `create_file`: safely create a new file inside the workspace
- `create_directory`: create directories inside the workspace
- `copy_file`, `move_file`, `delete_file`: manage files inside the workspace
- `open_file`: open a workspace file with the OS default application
- `open_application`: launch an application
- `exec_command`: execute an unrestricted platform shell command from the workspace
- `mouse_move`: move the pointer with absolute or relative coordinates
- `mouse_click`: click left, middle, right, back, or forward in the active application
- `mouse_scroll`: scroll horizontally or vertically in the active application
- `type_text`: type bounded Unicode text into the active application
- `key_press`: press named or Unicode keys with optional modifiers
- `get_ui_state`: read pointer, primary-display, and active-window metadata
- `capture_screen`: capture the primary display as an in-memory PNG for model vision

The displayed workspace is a starting directory, not a shell sandbox.
`ripgrep` is compiled into Atlas and does not require an `rg` executable on
`PATH`. It defaults to the workspace, respects ignore files, skips hidden and
binary files and symlinks, and accepts absolute roots. A canonical root outside
the workspace requires approval in Ask and Workspace modes; Full access skips
that Atlas confirmation but remains subject to OS-user permissions.
`exec_command` is available on Linux, WSL, macOS, and Windows and is intentionally
unrestricted. It supports shell operators, pipes, redirects, and paths outside
the starting folder, and it runs with the same permissions as the Atlas process.
The structured file helpers remain workspace-scoped. The command's default
timeout is 30 seconds, with stdout and stderr each truncated after 64 KiB.

Runtime permission modes are `ask`, `workspace`, and `fullAccess`. Ask permits
read-only tools automatically. Workspace also permits non-destructive structured
workspace changes. Outside-workspace searches, destructive actions, launches,
and commands still require an approval in Workspace mode. Desktop observation
and mouse/keyboard control require approval in both Ask and Workspace modes
because they can expose or change sensitive state. Full access skips Atlas
confirmations but never
elevates beyond the Atlas process's OS-user permissions. The Settings default
applies to new drafts; a saved conversation stores and restores its own mode.

Computer input targets the application active at execution time. Windows may
reject input into a higher-integrity process, macOS requires user-granted
Accessibility access, and Linux currently requires a usable X11 display.
Wayland-only sessions return an actionable error. Keep these capabilities in
the Rust backend; do not broaden frontend Tauri shell, process, accessibility,
screen-capture, or filesystem permissions.

`capture_screen` uses `ComputerObserve`. Ask and Workspace modes require
approval because visible desktop data may be sent to the configured model
provider. Captures are primary-display-only, remain in memory for the current
turn, and are never written to disk or conversation history. The DeepSeek
adapter rejects images over 8192 pixels per side or 32 MiB. Windows uses native
capture, macOS requires Screen Recording permission, and Linux requires X11.
Linux development/runtime environments need `libxcb`, `libxrandr`, and `dbus`
(Debian/Ubuntu packages: `libxcb1 libxrandr2 libdbus-1-3`).

## Model providers

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

Desktop Settings preserves per-provider drafts until Save/Cancel and supports
transient model discovery without persisting credentials or selecting a model.
Manual model IDs remain usable after discovery errors. Provider catalog responses
carry source identity per model and use generation tokens to reject stale results.
A model change must validate the current source before saving. Keep optional
request capabilities explicit; omit unsupported/unknown sampling fields. Output
request defaults are configurable and are never interpreted as model capacity.

Provider source identity hashes provider ID plus normalized base URL, excluding
credentials. Scope model-limit overrides and new usage readings to that source.
Usage preserves the requested model ID when an API returns a dated alias target.
Legacy source-less readings remain readable but are not shown as attributed usage.
Input-only limits are separate from combined context limits; never add independent
input/output maxima to invent combined capacity. Keep unknown capacities unknown.

## Configure and Run

Show CLI help without configuring a model:

```bash
cargo run -- help
```

Set the DeepSeek API key in the environment or a local `.env` file:

```bash
export DEEPSEEK_API_KEY="your_api_key_here"
```

Optional model configuration:

```bash
export DEEPSEEK_BASE_URL="https://api.deepseek.com"
export DEEPSEEK_MODEL="deepseek-flash"
```

The same fields can be saved from the desktop Settings screen. User-local
`settings.json` is plaintext with private filesystem permissions, not keychain
storage. Environment variables override saved fields without being copied into
the file. `ATLAS_CONFIG_DIR` overrides the settings directory for tests.

Run a request. Responses stream by default:

```bash
cargo run -- ask "hello atlas"
```

The existing `--stream` form remains available as an alias:

```bash
cargo run -- ask --stream "hello atlas"
```

Every `ask` without `--conversation` starts a new conversation. After the streamed
answer, Atlas prints its ID and title. Use that ID explicitly to continue:

```bash
cargo run -- ask --conversation <id> "now explain the runtime"
cargo run -- conversation list
cargo run -- conversation show <id>
cargo run -- conversation rename <id> "Atlas runtime work"
cargo run -- conversation delete <id>
```

`--stream` and `--conversation` accept either order before the prompt. Quote
multiword titles. Help and all conversation management commands parse and run
without a model API key or agent configuration. List is newest-update-first and
shows IDs, titles, creation/update times, and message counts; show includes saved
roles, content, and UTC timestamps.

Conversation storage is resolved using a platform directory library: Linux/WSL
uses `$XDG_DATA_HOME/atlas` (normally `~/.local/share/atlas`), macOS uses
`~/Library/Application Support/atlas`, and Windows uses `%LOCALAPPDATA%\atlas`.
`ATLAS_DATA_DIR` overrides the root for tests and development. The root contains
`conversations.json`, `conversations.lock`, `conversations/<uuid>.jsonl`, optional
`activities/<uuid>.jsonl` sidecars, and `trash/`. Delete moves history and any
activity and memory sidecars to `trash/` and reports the history recovery location; recovery
is manual, with no CLI restore command.

`ConversationService` coordinates the store and runtime. Save only successful
user/final-assistant pairs to ordinary message history. Save sanitized, bounded
runtime status and tool activity from successful turns in the separate sidecar;
failed overall turns save neither. Never persist screenshot pixels, image data,
or tool attachments. New activity batches also store elapsed runtime/model work
duration; legacy batches without it remain valid. Raw activity details remain
local, while later model turns receive only a separately capped summary from the
same uncompacted turns selected for message history. Keep all original history
on disk. The bundled agent enables `[memory]`: full history before compaction,
then one model message per persisted summary block followed by original messages
from the checkpoint's exclusive boundary onward. Never silently apply the old
40-message cap in this mode. Explicit `ATLAS_HISTORY_MESSAGES` opts into legacy
bounded history; third-party agents without memory configuration retain legacy
behavior. The environment limit must be positive.

Compaction is token-based: automatic at 90%; manual `/compact` chooses one block
below 30%, two below 60%, and three from 60% upward. Preserve at least the latest
10 original messages as whole exchanges. Recompact prior summaries with eligible
newer messages and replace them, never accumulate endless summary blocks. Each
block is a separate no-tool request using `conversation_summary_prompt`, with an
explicit output limit. Split oversized inputs under bounded request/pass limits;
reject empty or truncated results. Keep summary usage separate from chat usage.

Save `memory/<uuid>.json` atomically with version, policy version, checkpoint ID,
original-prefix SHA-256 fingerprint, covered ranges, model, trigger, occupancy
estimate and summary usage. Validate against originals before loading/publishing;
advance the boundary only with a fully successful fitting checkpoint. A checkpoint
can survive a subsequent failed ordinary answer. Keep private filesystem modes,
trash memory with history, and hold per-conversation execution leases for
ask/compact/delete. Do not hold the global index lock during model calls.

Prepare and budget-check context before every runtime model call, including tool
continuations and repair calls; never replay tools during compaction. DeepSeek
currently uses a conservative byte estimate, not an exact tokenizer; distinguish
that from actual usage. Unknown model capacity or unsupported image accounting
makes auto-compaction explicitly unavailable. Optional per-model context/output
limits live in `conversation.modelLimits`; never infer capacities from model
names. Known oversized requests fail without discarding the protected tail.
Selecting the desktop Compact task must not create a chat message or persist an unsaved draft;
progress, checkpoint restoration, cancellation and memory details belong to the
owning session while other chats continue independently.
Compaction progress/results/failures use the persistent status card. Keep failure
metadata and retry task arguments per session; retries dispatch that saved task
without consuming a newer draft. Preserve open diagnostic details during unrelated
UI updates. Automatic compaction failures keep their chat prompt available for a
normal resend; do not silently launch another summary job.
DeepSeek summary requests explicitly disable thinking to reserve the bounded output
budget for summary content. Keep this provider-specific switch in the adapter,
not shared runtime logic. Distinguish empty output, output-limit truncation, and
unexpected finish reasons; none may publish partial memory or echo response text.


`src/tasks/mod.rs` is the authoritative built-in task registry, initially
`conversation.compact` (`/compact`, empty-object arguments). Keep descriptors,
argument validation and typed handlers together; reject duplicate IDs/commands.
Tauri `list_tasks` needs no model configuration, and `run_task` validates requests
before provider setup, obtains native ownership once, derives the saved workspace,
and delegates to the service's existing execution lease. CLI compact shares the
registry; automatic compaction retains its direct runtime path without nested leases.
All frontend task access stays in `atlas-api.ts`. The composer discovers tasks
from the backend catalog and filters command/title/description. Clicking a task
or pressing Enter on the highlighted row runs the selected descriptor directly;
never infer task execution or arguments from the text submitted through Send.
Send always calls ordinary chat, including literal `/compact`, bare `/`, unknown
slash text, paths and prose. Escape/outside click/Tab close the menu and preserve
input; Tab does not run a task. Explain each task and show its progress/result in
the owning chat. Keep unavailable tasks visible with reasons. Catalog
failure/retry cannot block ordinary messages or overwrite drafts. Keep task/menu
ownership per session. Do not add frontend fallback task names or automatic model
routing in this release. Future handlers need registry validation/dispatch tests,
argument support and typed outcome adapters as needed.


After the first successful answer, the agent makes one non-streaming, no-tool title
request using only the first input and `conversation_title_prompt` from
`agents/atlas.toml`. Sanitize generated titles to at most 60 Unicode characters;
failure leaves `Untitled conversation`. Continuing and manual rename never
regenerate the title. The default agent prompt guides workspace inspection,
command execution, verification, preservation of unrelated changes, and honest
reporting of results and blockers. `max_tool_calls` is an optional positive
per-turn agent limit; omit it for no Atlas-imposed limit. Tool schemas and
protocol remain runtime-injected.

Format and test the repository:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm --prefix desktop test
npm --prefix desktop run build
npm --prefix desktop run tauri -- build --debug --no-bundle
```

Install desktop dependencies and launch the Tauri app from the repository root:

```bash
npm ci --prefix desktop
npm --prefix desktop run tauri dev
```

The desktop UI calls Rust only through `desktop/src/atlas-api.ts`. Keep **New
chat** as an unsaved frontend draft; only a successful first `ask` may create
local conversation data or persist its validated starting folder. New chats use
an explicitly chosen draft folder, then a custom Settings default, then the
automatic `Atlas` folder in the user's home directory. Saved continuations derive
workspace authority from conversation metadata and must ignore a conflicting
frontend path. Legacy unassigned conversations use the current default and gain
that assignment only with their next successful saved turn. The CLI continues to
use its process's current directory.

Desktop chat sessions have stable frontend identities separate from saved
conversation IDs. Keep request callbacks, live messages, drafts, tool activity,
errors, and approvals owned by their session, even when another chat or Settings
is visible. Independent chats may run concurrently, but only one request may
run per saved conversation in the desktop backend. Navigation must not cancel
all pending approvals. First requests remain in-memory sidebar sessions until
successful persistence supplies an ID; failures stay retryable without writing
conversation data. Session state and unfinished requests do not survive an app
reload or restart. Running requests retain their starting settings and folder.
Keep Settings inputs and user scroll position stable during background updates,
and coalesce visible streaming renders to animation frames. Blocking native
storage/settings operations run on background workers rather than the Tauri
event thread. Do not broaden frontend permissions to achieve concurrency.

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

Keep native platform access narrow. Folder selection uses
`@tauri-apps/plugin-dialog` with `dialog:allow-open`, and the fullscreen
control uses only `core:window:allow-set-fullscreen`; do not add shell,
process, or unrestricted filesystem permissions to the frontend.

CLI binary tests isolate their workspace and `ATLAS_DATA_DIR`, use child-only
environment configuration, and serve fake DeepSeek responses over loopback TCP.
Do not use real API keys or external model services in tests.

## Commit Traceability Rule

After completing a requested code or documentation change, create one focused Git commit so the work can be traced, unless the user explicitly asks not to commit.

Before committing:

- Run `cargo fmt --check`.
- Run `cargo test`.
- Check `git status --short`.
- Review the staged diff.
- Commit only files that belong to the current task.
- Never commit `.env`, API keys, secrets, `target/`, or unrelated changes.

Use a short commit message that describes the completed change, such as:

```text
feat: send tool results back to agents
fix: handle streamed tool responses
docs: simplify agent repository guide
```
