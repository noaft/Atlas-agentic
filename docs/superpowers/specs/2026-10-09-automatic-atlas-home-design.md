# Automatic Atlas Home and General OS Assistant Design

## Purpose

Atlas should open ready for an ordinary conversation or computer task without requiring the user to choose a project folder first. It should behave as a general personal computer assistant rather than a coding-only companion. Its shell tool should be able to operate anywhere the current OS user can access.

The automatic Atlas folder is a useful starting directory and an organizational label. It is not a sandbox for `exec_command`.

## Scope

This change applies to the Tauri desktop app and the shared default Atlas agent definition. The desktop app will create and use an `Atlas` directory inside the user's home directory when no custom default workspace is configured. A user may still choose a different folder for a draft, a saved conversation, or the default workspace setting.

The CLI will continue to start tools in the terminal's current directory. That behavior is useful and expected when a request originates from a shell.

The change will not add a command approval system, elevate privileges, bypass OS permissions, or add native Windows shell execution. `exec_command` will retain its current Linux, WSL, and macOS availability and unrestricted behavior.

## Workspace Semantics

For a new desktop conversation, workspace resolution will use this order:

1. A folder explicitly chosen for the draft.
2. A valid custom default workspace saved in Settings.
3. The automatic home-directory folder named `Atlas`.

For a saved conversation, its persisted workspace remains authoritative. A conflicting path supplied by the frontend must still be ignored.

For a legacy conversation without a workspace, Atlas will use the current resolved default: the saved custom default when present, otherwise the automatic Atlas folder. The legacy conversation will receive that workspace only when its next request succeeds. A failed request must leave its messages and metadata unchanged.

The automatic Atlas folder will be resolved through the platform home-directory API rather than a hard-coded path or environment-specific string. Atlas will create the directory and its missing parents when first needed. It will then canonicalize and validate the result using the existing workspace validation rules. Failure to resolve the home directory, create the folder, or validate it will produce a clear error before runtime construction or conversation mutation.

An invalid or deleted custom default will continue to produce an error instead of silently falling back. This keeps an explicit user choice authoritative and makes stale configuration visible. Clearing the custom default in Settings restores the automatic Atlas folder.

## Tool Access

The automatic or selected workspace determines the initial working directory for runtime tools and the folder used to group conversations.

`exec_command` remains intentionally unrestricted. It may use absolute paths, change directories, invoke installed applications, and access files outside the conversation workspace. Commands run with the same permissions as the Atlas process, so normal OS access controls still apply.

The structured `pwd`, `ls`, `read_file`, and `create_file` tools will keep their existing workspace boundaries. This preserves the repository's file-tool safety rules. When a task requires a path outside that boundary, the default agent may use `exec_command` on supported Unix-like systems.

The default agent prompt will describe Atlas as a general local assistant for conversation, planning, files, applications, shell tasks, coding, and project work. It will tell the agent to inspect relevant context, use tools when needed, preserve unrelated data, verify material actions, and report blockers honestly. Tool schemas and execution rules remain runtime-injected.

## Backend Design

A focused workspace helper will resolve and provision the automatic Atlas folder. It will accept an injectable home-directory source for deterministic tests while production uses the platform directory library already present in the project.

The desktop state will resolve a workspace before building the runtime:

- New conversations use the three-level precedence described above.
- Assigned conversations use only their persisted workspace.
- Unassigned legacy conversations use the resolved default.

Conversation persistence will gain an atomic append operation that can assign a workspace only when the conversation is currently unassigned. Under the existing store lock, it will re-read the conversation, preserve an existing assignment, append the successful user/assistant pair, update timestamps and counts, and publish the index and history consistently. This prevents an old conversation from receiving workspace metadata when the model request or streaming sink fails.

The Settings view returned to the desktop will expose the effective default workspace and whether it is automatic or explicitly configured. The automatic folder itself will not be written into `settings.json`; only user overrides remain persisted. This makes clearing an override reliably return to the platform-specific Atlas home.

## Desktop Experience

After initialization, a new chat will show the resolved default workspace and enable the composer immediately. Pressing **New chat** will reset to the resolved default without opening the native folder picker.

The workspace card remains visible. Its action opens the native picker so the user can attach the draft or saved conversation to a specific project folder. Selecting a different folder for a saved conversation continues to update metadata without rewriting its history.

The empty-chat content will use general-assistant language such as "How can I help?" It will explain that Atlas can converse, organize files, run commands, help with applications, and work on code. The brand subtitle and composer help text will no longer claim that Atlas is limited to coding or confined to the selected folder. They will accurately state that the displayed folder is the shell's starting location.

Settings will label the fallback as **Atlas home (automatic)**. When a custom default is active, the user can change it or reset it to Atlas home. The existing private local storage and environment-override behavior remains unchanged.

When an unassigned legacy conversation is opened, the frontend will show the resolved default as its active starting folder and allow sending. After a successful response, returned metadata will contain the assigned workspace, moving the conversation into the appropriate workspace group. Until then, on-disk data remains unchanged.

## Failure Handling

Workspace resolution and creation happen before model or tool construction. Errors include enough path context to explain whether Atlas could not locate the home directory, create the Atlas folder, or access an explicitly configured folder.

The existing successful-turn rule remains intact: partial streamed output, provider failure, title-generation failure, or frontend channel failure must never save an incomplete user/assistant pair. Automatic workspace assignment follows the same boundary.

Shell commands continue to report their exit status, timeout, captured output, and truncation flags. The assistant must not claim access that the OS denied or claim success when a command failed.

## Testing

Rust tests will cover:

- platform-independent automatic Atlas path resolution with an injected home directory;
- creation and canonical validation of a missing Atlas folder;
- explicit draft and configured-default precedence over the automatic folder;
- automatic fallback for new chats and legacy unassigned conversations;
- persisted conversation workspace authority over frontend input;
- atomic assignment plus append after a successful legacy continuation;
- no metadata or history mutation after provider or streaming failure;
- clear failures for unavailable homes, failed directory creation, and stale configured defaults;
- unrestricted `exec_command` access to an absolute path outside its starting workspace.

Desktop tests will cover:

- initialization and **New chat** selecting the effective default without opening the picker;
- immediate composer availability and sending without manual folder selection;
- optional folder changes for drafts and saved conversations;
- legacy conversations using the effective default until successful assignment;
- automatic and custom Settings labels and reset behavior;
- general-assistant welcome, brand, placeholder, and tool-access copy.

The final verification will run the repository's Rust formatting, test, and Clippy commands; desktop Rust formatting, tests, and Clippy commands; frontend tests and production build; and the debug Tauri build without bundling.

## Documentation

`README.md` and `AGENTS.md` will describe the automatic Atlas home, optional project-folder selection, workspace precedence, legacy-conversation behavior, and the distinction between a shell starting directory and OS access. `VISION.md` will be updated because the product positioning changes from a coding-oriented workspace companion to a general desktop-first assistant that also supports software work.
