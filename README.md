# Atlas Agentic

A desktop-first AI assistant for conversation, files, applications, computer
control, and coding. The Tauri desktop app and Rust CLI share the same agent,
tools, and local conversation history. DeepSeek is the current model provider;
multi-agent routing is planned.

## Quick start

Install Rust and Node.js/npm, plus the platform dependencies in the
[Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/).
Linux also needs `libxcb`, `libxrandr`, and `dbus`
(Debian/Ubuntu: `libxcb1 libxrandr2 libdbus-1-3`).

From the repository root:

```bash
npm ci --prefix desktop
npm --prefix desktop run tauri dev
```

Open **Settings** to add your DeepSeek API key and choose a model. New chats use
an automatically created `Atlas` folder in your home directory; choose another
folder per chat or set a default in Settings.

Chats can run independently, with their own drafts, activity, and approvals.
History is saved after successful replies; unsaved drafts and unfinished requests
do not survive an app restart.

## CLI

Set your API key in the environment or a local `.env` file (see
[.env.example](.env.example)), then send a request:

```bash
export DEEPSEEK_API_KEY="your_api_key_here"
cargo run -- ask "hello atlas"
```

Replies stream by default. Each request starts a new conversation unless you
pass the ID printed after a previous answer:

```bash
cargo run -- ask --conversation <id> "continue our discussion"
cargo run -- conversation list
cargo run -- conversation show <id>
cargo run -- conversation rename <id> "My project"
cargo run -- conversation compact <id>
cargo run -- conversation delete <id>
```

`cargo run -- help` and conversation management (except compaction) work without
an API key. The CLI uses your current directory as its starting folder.

## Settings and access

Desktop Settings supports the API key, base URL, model, model limits, default
folder, and tool-access mode. Environment variables override saved settings:
`DEEPSEEK_API_KEY`, `DEEPSEEK_BASE_URL`, and `DEEPSEEK_MODEL`.
The default agent is configured in [agents/atlas.toml](agents/atlas.toml).
Saved API keys are plaintext in the user-local `settings.json`, protected by
private filesystem permissions where supported. Never commit credentials.

| Mode | Tools allowed without confirmation |
| --- | --- |
| **Ask** | Ordinary workspace reads |
| **Workspace** | Reads and non-destructive structured workspace changes |
| **Full access** | All registered tools, within your OS-user permissions |

Ask and Workspace require approval for commands, application launches, desktop
observation/control, and searches outside the workspace. The starting folder is
not a shell sandbox: approved shell commands can access paths outside it.
Computer control requires X11 on Linux and Accessibility permission on macOS;
macOS screen capture also requires Screen Recording permission. Screenshots stay
in memory for the current turn and may be sent to the model provider.

Built-in tools:

| Purpose | Tools |
| --- | --- |
| Inspect and search | `current_time`, `pwd`, `ls`, `read_file`, `ripgrep`, `search_files` |
| Manage files | `create_file`, `create_directory`, `copy_file`, `move_file`, `delete_file` |
| Open and execute | `open_file`, `open_application`, `exec_command` |
| Control the desktop | `mouse_move`, `mouse_click`, `mouse_scroll`, `type_text`, `key_press` |
| Observe the desktop | `get_ui_state`, `capture_screen` |

## History and memory

Conversations are stored locally outside the repository. Atlas keeps original
history and compacts older context when usable model limits are available. Set
missing context/output limits in Settings. Summary requests consume model tokens.

For manual compaction, type `/` in the desktop composer and select **/compact**,
or use the CLI command above. **Send always sends ordinary chat text**, including
literal `/compact`; selecting the task runs compaction. Recent exchanges remain
intact. Deleted conversations move to local trash for manual recovery.

## Development

Run the core checks:

```bash
cargo fmt --check
cargo test
```

Build a desktop debug executable:

```bash
npm --prefix desktop run tauri -- build --debug --no-bundle
```

See [AGENTS.md](AGENTS.md) for repository structure, storage paths, configuration
details, and the full verification commands; [RULE.md](RULE.md) for architecture
rules; and [VISION.md](VISION.md) for product direction.
