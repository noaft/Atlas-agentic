# Atlas Agentic Repository Guide

## Project

Atlas is a desktop-first general AI assistant with a Rust CLI and Tauri app.
Both use `agents/atlas.toml`, shared runtime/settings/providers, and local history.
Reusable child agents and direct `/name task` dispatch are implemented;
automatic routing is planned. A future router selects an agent; the selected agent performs the task.

Read [RULE.md](RULE.md) before changing agent, provider, runtime, router, or tool
architecture. Read the relevant contracts below before editing that subsystem.

## Repository map

| Path | Responsibility |
| --- | --- |
| `agents/`, `config/` | Bundled agent, provider defaults, and runtime prompts |
| `src/main.rs`, `src/lib.rs`, `src/app.rs` | CLI and shared runtime construction |
| `src/runtime.rs`, `src/runtime/`, `src/agent_output.rs` | Orchestration and protocol |
| `src/agents/`, `src/agent_catalog.rs`, `src/agent_runs.rs`, `src/delegation.rs` | Agents, definitions, owned child runs |
| `src/executions/`, `src/tools/`, `src/process_limits.rs` | Managed tools, processes, and quotas |
| `src/models/`, `src/settings.rs` | Providers, budgets, profiles, and settings |
| `src/conversations/`, `src/diagnostics.rs` | History, activity, memory, and private logs |
| `src/skills/`, `src/mcp/`, `src/tasks/`, `src/workspace.rs` | Skills, MCP/OAuth, built-in tasks, starting folders |
| `desktop/src/`, `desktop/src-tauri/` | Frontend and native bridge |
| `tests/`, `examples/` | Behavior tests and no-key demos |

## Read by change area

These documents contain required implementation constraints, not optional background.
Read only the areas relevant to the change; cross-cutting work may need several.

| Change area | Required reference |
| --- | --- |
| Tool execution, wakeups, results, cancellation | [Execution decisions](docs/architecture/tool-execution-control.md), [Wakeup protocol](docs/architecture/wakeup.md) |
| Shell commands, quotas, resource ownership | [Process sessions](docs/architecture/process-sessions.md) |
| File helpers, search, computer input/capture | [File and computer tools](docs/architecture/file-and-computer-tools.md) |
| MCP config, transports, OAuth, Settings checks | [MCP contracts](docs/architecture/mcp.md) |
| Skill discovery, creation, imports, Settings | [Skill contracts](docs/architecture/skills.md) |
| Child agents, grants, reports, slash dispatch | [Delegated agents](docs/architecture/delegated-agents.md) |
| Providers, profiles, discovery, limits, usage | [Model provider contracts](docs/architecture/model-providers.md) |
| History, memory, compaction, progress, repair, logs | [Conversation contracts](docs/architecture/conversations.md) |
| Desktop state, task dispatch, workspace, IME | [Desktop contracts](docs/architecture/desktop-state.md) |
| Queuing, steering, interruptible inference, Stop | [Live chat input](docs/architecture/live-chat-input.md) |
| Native Vietnamese input on WSLg | [UniKey reproduction and workaround](docs/platforms/wslg-unikey.md) |
| CI, checks, toolchains | [Development guide](docs/guides/development.md) |
| Windows tags, packaging, publication | [Windows release guide](docs/guides/windows-release.md) |

## Shared invariants

- Keep provider payloads and behavior behind `ModelProvider`; runtime controls tools.
  Configure prompts/defaults outside implementation code. Never infer model
  catalogs or capacities from names; unknown limits stay unknown.
- Native saved conversation metadata determines workspace and permission mode.
  Model arguments or conflicting frontend paths never choose another owner.
- Modes are `ask`, `workspace`, and `fullAccess`. Ask allows ordinary read-only
  tools; Workspace also allows non-destructive structured workspace changes.
  Commands, destructive actions, launches, outside-workspace searches, computer
  observation/control, and external MCP discovery/calls require approval in both.
  Full access skips Atlas confirmations without elevating OS-user permissions.
- The starting folder is not a shell sandbox. Structured file helpers remain
  workspace-scoped; `exec_command` is unrestricted and ripgrep supports approved
  outside roots. Do not broaden frontend Tauri native permissions.
- Every tool call is tracked. Preserve authorization snapshots, grants, owner
  scopes, quotas, bounded output, and atomic/private storage. Never replay actions
  automatically after failure, timeout, protocol repair, compaction, or restart.
- Cancellation is cooperative: show Cancelling/Stopping until worker exit.
  Preserve Unix process groups and Windows kill-on-close Jobs. Only explicitly
  requested background services may survive final; no model calls after final/restart.
- Screenshots remain in-memory for the active turn, never history/logs/artifacts.
  Keep credentials, prompts, raw responses, tool arguments/output, and image data
  out of diagnostics. MCP OAuth credentials stay in the OS credential store.
- Frontend backend access goes through `desktop/src/atlas-api.ts`. Preserve
  session-owned drafts, focus, scroll, approvals, and stale-callback rejection.
  Blocking native work runs on workers. Tool/agent activity stays inline;
  no separate permanent Processes panel or automatic process-output polling.
- Child agents receive only explicitly granted tools from the effective registry;
  skills never grant permissions. Save new reusable agents/skills only when requested.
- Use fake providers in tests, isolated workspaces/data roots, and child-only
  environment changes. Never use real API keys or external model services in tests.

## Verification

Use `rust-toolchain.toml` and `.node-version`. Before every commit:

```bash
cargo fmt --check
cargo test
```

Run additional checks appropriate to changed code:

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm --prefix desktop test
npm --prefix desktop run build
npm --prefix desktop run tauri -- build --debug --no-bundle
```

See the [development guide](docs/guides/development.md) for CI and test isolation.
Report platform verification separately; compilation/browser tests do not prove
native Windows/macOS behavior. Review documentation links after moving content.

## Documentation and commits

- Keep `README.md` for quick start, `RULE.md` for coding rules, and `VISION.md` for
  direction. Put detailed contracts in `docs/` and link them from this guide and
  [the documentation index](docs/README.md). Update the owning reference when
  behavior changes; avoid copying its full contract into multiple documents.
- Distinguish implemented behavior, approved designs, and proposals. Writing a
  design does not approve it. Follow [the documentation workflow](docs/README.md#drafts-and-approval-workflow).
- Superpowers drafts, plans, ledgers, previews, screenshots, and test logs belong
  in Git-ignored `.superpowers/`, overriding skill defaults. Never force-add them.
  Runtime skills/agent definitions are not workflow scratch.
- After verifying each feature, fix, or requested documentation change, create a
  focused local Git commit unless the user asks otherwise. Commit independently
  completed features separately and include the hash in the handoff. This does
  not authorize push, merge, release, or deployment.
- Before committing, check `git status --short` and review the staged diff. Include
  only task files; never commit `.env`, credentials, `target/`, or unrelated work.
