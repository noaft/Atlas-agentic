# Local Conversations Design

## Summary

Atlas will persist multiple conversations in the current user's OS-local application data directory. A normal `atlas ask` command creates a new conversation. A user continues an existing conversation only by supplying its ID.

For every request, Atlas sends the agent's system prompt, a bounded window of prior user/assistant messages, and the new user input. After a successful final answer, Atlas saves the new user/assistant pair. Internal tool calls, tool results, and protocol messages are never stored as conversation messages.

The first successful turn also triggers one separate LLM request that creates a short conversation title from the first user input. Atlas stores that title in conversation metadata and never regenerates it automatically.

## Goals

- Persist conversation history across CLI processes.
- Support many conversations for one local user.
- Create a new conversation automatically when `ask` has no conversation ID.
- Continue a selected conversation when its ID is supplied.
- Provide create, read, update, and delete behavior through the CLI.
- Keep complete history on disk while bounding the history sent to the model.
- Keep tool protocol and tool results out of saved chat history.
- Generate a useful title once from the first user input.
- Keep conversation storage local, readable, recoverable, and provider-agnostic.
- Improve the default Atlas agent's coding behavior without duplicating runtime-injected tool definitions in its prompt.

## Non-Goals

- Synchronizing conversations between devices.
- Multiple operating-system users sharing one Atlas store.
- Semantic search, embeddings, or memory extraction.
- Automatically selecting a recent or active conversation.
- Automatically summarizing old messages.
- Editing individual saved messages.
- A desktop conversation UI in this phase.

## CLI Contract

### Start a new conversation

When no conversation ID is provided, `ask` always creates a new conversation:

```bash
atlas ask "help me understand this repo"
```

Atlas generates the ID internally. After the streamed answer finishes, the CLI prints the new conversation ID and generated title as a metadata line so the user can continue it later.

### Continue a conversation

```bash
atlas ask --conversation <id> "now explain the runtime"
```

Atlas loads the selected conversation, supplies its recent history to the agent, and appends the successful user/assistant turn.

### Manage conversations

```bash
atlas conversation list
atlas conversation show <id>
atlas conversation rename <id> "Atlas runtime work"
atlas conversation delete <id>
```

The commands map to CRUD behavior:

- Create: `ask` without `--conversation`.
- Read: `conversation list` and `conversation show`.
- Update: continuing a conversation and `conversation rename`.
- Delete: `conversation delete`.

`conversation list` is ordered by most recently updated first and shows at least the ID, title, creation time, update time, and message count.

## Storage Location and Layout

Atlas resolves the operating system's per-user application data directory rather than writing conversation data into a repository:

- Linux and WSL: the XDG data directory, normally `~/.local/share/atlas`.
- macOS: the user's Application Support directory.
- Windows: the user's local application data directory.

The implementation should use a platform directory library instead of hard-coding these paths.
For deterministic development and tests, `ATLAS_DATA_DIR` may override the resolved root. Normal users do not need to set it.

```text
<atlas-data>/
├── conversations.json
├── conversations.lock
├── conversations/
│   ├── <uuid>.jsonl
│   └── <uuid>.jsonl
└── trash/
    └── <uuid>.jsonl
```

`conversations.json` is the metadata index. Each conversation has one JSONL history file. Deleted history files move to `trash` so deletion is recoverable outside the CLI.

## Data Model

### Metadata index

The index has a format version and a collection of conversation records:

```json
{
  "version": 1,
  "conversations": [
    {
      "id": "0199...",
      "title": "Understand the Atlas runtime",
      "title_attempted": true,
      "created_at": "2026-10-04T12:00:00Z",
      "updated_at": "2026-10-04T12:03:00Z",
      "message_count": 4
    }
  ]
}
```

Conversation IDs are UUIDs. Timestamps are UTC RFC 3339 strings. New records begin with `Untitled conversation` and `title_attempted: false`.

### Conversation history

Each JSONL line is one message:

```json
{"role":"user","content":"help me understand this repo","created_at":"2026-10-04T12:00:00Z"}
{"role":"assistant","content":"Atlas is a Rust agent runtime...","created_at":"2026-10-04T12:00:10Z"}
```

Only `user` and `assistant` roles are valid in persisted history. Newlines inside content are JSON-escaped, preserving one record per line.

## Request and Save Flow

### New conversation

1. Resolve the user-local Atlas data directory.
2. Generate a UUID in memory and prepare temporary metadata with the fallback title.
3. Send the default agent the new user input with no previous history.
4. Allow the runtime to execute tools and stream the final answer.
5. If the request fails before a final answer, do not save the conversation.
6. Under the store lock, write the user and final assistant messages, create the metadata record, and atomically replace the metadata index.
7. Request an LLM-generated title using only the first user input.
8. Sanitize and store the title, or keep the fallback title if generation fails.
9. Mark `title_attempted: true` in either case, so automatic title generation happens only once.
10. Print the conversation ID and resulting title after the answer stream.

The main answer is completed and visible before title generation begins. A title failure never changes a successful answer into a failed `ask` command.

### Existing conversation

1. Validate the supplied conversation ID.
2. Load its metadata and JSONL history.
3. Select the most recent configured number of messages.
4. Send the agent system prompt, selected messages in their original roles, and the new user input.
5. Execute tools and stream the final answer normally.
6. After success, append the user and final assistant messages under the store lock.
7. Update `updated_at` and `message_count`, then atomically replace the metadata index.

If agent execution fails, neither the user input nor a partial assistant response is saved.

## Model Request Shape

The model-provider boundary will support typed conversation messages rather than flattening history into one text prompt. The effective request is:

```text
system:    agent system prompt
user:      oldest selected user message
assistant: oldest selected assistant message
...
user:      newest input
```

Runtime-only tool continuations remain internal to the current request. They are available while the tool loop runs but are not included in persisted conversation history.

Core conversation types remain provider-agnostic. DeepSeek converts them into its provider-specific chat message representation inside the provider module.

## Context Window Policy

Atlas stores the complete conversation but sends only recent history to the model. The default is the most recent 40 messages, equivalent to at most 20 complete user/assistant turns.

`ATLAS_HISTORY_MESSAGES` overrides the default with a positive integer. Atlas preserves chronological order after selecting the recent window. The new user input is added after the selected saved messages and does not count against the saved-message limit.

This first version uses a message-count limit because Atlas does not yet have a provider-neutral tokenizer. Token-budget selection or old-history summarization can be added later without changing the on-disk format.

## Title Generation

The local agent definition gains a dedicated conversation-title system prompt. Keeping this prompt in agent configuration follows the repository rule against hard-coding agent behavior in runtime code.

After the first successful turn, the local agent makes one non-streaming provider request with:

- the configured title system prompt;
- only the first user input;
- no conversation history;
- no tools.

Atlas trims whitespace, collapses line breaks, removes surrounding quote characters, and limits the stored title to 60 Unicode characters. Empty or invalid output falls back to `Untitled conversation`.

The title request occurs only when `title_attempted` is false. Manual rename updates the title but does not trigger or re-enable automatic title generation.

## Default Agent Prompt

`agents/atlas.toml` will be enhanced so the default Atlas agent:

- inspects the workspace before making repository-specific claims;
- uses runtime-provided tools when they are needed to complete the task;
- executes relevant commands for coding, debugging, testing, and verification;
- preserves unrelated user changes and never invents command results;
- reports completed work, verification evidence, and remaining blockers clearly.

The prompt must not list tool names, schemas, or tool-call protocol details. Those remain injected automatically from `ToolRegistry` through `ModelRequest.tools`.

## Components and Boundaries

### Conversation domain module

A new conversation module owns provider-independent types and storage behavior:

- `ConversationId`
- `ConversationMessage`
- `ConversationMetadata`
- `ConversationStore`

The store handles directory creation, index loading, JSONL loading/appending, listing, renaming, and recoverable deletion. It does not call models or execute agents.

### CLI

The CLI parses conversation commands and the optional `--conversation` value. It coordinates storage with the runtime but does not implement storage parsing itself.

### Runtime and agents

The runtime accepts typed prior messages for an ask operation and passes them through the agent/provider boundary. It still owns tool-loop coordination.

The agent boundary exposes title generation. `LocalAgent` uses its configured title prompt and existing model provider, keeping title behavior out of the CLI and storage modules.

### Model providers

`ModelRequest` carries typed historical messages plus the current input. Providers translate those messages into their native request format. No conversation storage code enters a provider.

## Consistency and Recovery

- Mutations acquire a user-store lock to prevent concurrent index replacement and interleaved history appends.
- Metadata updates write a temporary file, flush it, and rename it over `conversations.json`.
- History appends write complete JSON lines and flush before metadata is advanced.
- A malformed index or history line returns a clear error containing the affected path and line when applicable.
- Atlas never silently discards malformed stored data.
- If history is appended but metadata replacement fails, the next successful load reconciles `message_count` and `updated_at` from the history file before writing further changes.
- Deletion moves the history file to `trash` before removing its metadata entry.
- Missing IDs and IDs absent from the index return explicit `conversation not found` errors.

## Testing

Tests will use temporary data directories and fake model providers. Coverage includes:

- `ask` without an ID creates a UUID conversation.
- Failed asks do not create or append conversation data.
- A successful turn saves exactly one user and one final assistant message.
- Tool calls and tool outputs are not persisted.
- Continuing by ID supplies old messages in role-preserving chronological order.
- Context selection keeps the most recent configured messages and always includes the new input.
- Title generation receives only the first input, occurs once, and does not block saving on failure.
- Title sanitization and fallback behavior.
- Listing order and displayed metadata.
- Showing, renaming, and recoverably deleting a conversation.
- Invalid IDs, missing IDs, malformed indexes, and malformed JSONL lines.
- Atomic metadata replacement and reconciliation after a partial metadata failure.
- CLI parsing for every new command and option.
- Existing default streaming and tool-loop behavior remains intact.
- The enhanced default-agent prompt contains the approved command-oriented guidance without hard-coding runtime tool definitions.

Before completion, the implementation must pass:

```bash
cargo fmt --check
cargo test
```

## Documentation

Implementation will update `README.md` and `AGENTS.md` with:

- conversation commands;
- the user-local storage behavior;
- the context-window environment variable;
- the one-time LLM title behavior;
- any resulting source-tree additions.
