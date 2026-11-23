# Atlas documentation

Use the [README](../README.md) to start Atlas, [AGENTS.md](../AGENTS.md) for
contributor instructions, and [RULE.md](../RULE.md) for coding rules.
Architecture contracts below document implemented behavior unless marked otherwise.

## Guides

| Document | Purpose |
| --- | --- |
| [Atlas reference](guides/reference.md) | Detailed user configuration, features, storage, and logs |
| [Development](guides/development.md) | Toolchains, CI, verification commands, and test isolation |
| [Process demo](guides/process-manager-demo.md) | No-key demo and desktop walkthrough |
| [Windows releases](guides/windows-release.md) | Version tags, packaging, checksums, and publication |
| [UniKey on WSLg](platforms/wslg-unikey.md) | Reproduction notes and verified workaround |

## Architecture contracts

| Document | Scope |
| --- | --- |
| [Execution decisions](architecture/tool-execution-control.md) | Approved hard/lazy architecture, implemented; decisions and acceptance criteria |
| [Wakeup](architecture/wakeup.md) | Current execution protocol, scheduler, controls, durable results, and recovery |
| [Process sessions](architecture/process-sessions.md) | Commands, quotas, output/cursors, ownership, and OS cleanup |
| [File and computer tools](architecture/file-and-computer-tools.md) | Workspace helpers, outside search, permissions, input, and capture |
| [MCP](architecture/mcp.md) | Configuration, transports, OAuth, Settings checks, and cleanup |
| [Skills](architecture/skills.md) | Storage, imports, request snapshots, runtime tools, and Settings |
| [Delegated agents](architecture/delegated-agents.md) | Definitions, grants, runs, reports, and direct slash commands |
| [Model providers](architecture/model-providers.md) | Adapters, profiles, discovery, source identity, capacity, and usage |
| [Conversations](architecture/conversations.md) | Committed history, activity, memory, progress, and repair |
| [Session journals and logs](architecture/sessions.md) | Dated JSON sessions, durable events, context cursors, debug policy and correlation |
| [Desktop state](architecture/desktop-state.md) | Session/workspace ownership, built-in tasks, UI, and IME |
| [Live chat input](architecture/live-chat-input.md) | Queuing, supplements, steering, interruptible inference, and Stop |
| [Resource lifecycle rationale](architecture/tool-resource-lifecycle.md) | Original ownership decisions; execution/UI portions superseded as noted |

Keep detailed contracts in their owning document and link to them from other
pages. [VISION.md](../VISION.md) separates the current foundation from planned routing.

## Drafts and approval workflow

Keep project reference documentation and explicitly user-approved designs under
`docs/`. Document implemented behavior as implemented; distinguish it from future
proposals. Do not mark a design approved merely because the user asked to write
it, or because its wording looks complete.

Superpowers design drafts, plans, execution ledgers, research scratch, previews,
screenshots and test logs belong under `.superpowers/` and are Git-ignored. This
repository rule overrides skill defaults that would write to `docs/superpowers/`.
That old location and local `.worktrees/` are also ignored. Do not force-add these
artifacts or ignore runtime skills/agent definitions as workflow scratch.

When the user explicitly approves a design, promote a curated copy from
`.superpowers/specs/` to `docs/architecture/` (or a relevant docs section), record
its approval and implementation status, update `docs/README.md` and links, and
commit the reviewed document. Keep local plans/logs out of that promotion. Pending
drafts stay local until approved. Older workflow documents are preserved locally
under `.superpowers/archive/2026-11-09-docs/`; their prior versions also remain in
Git history. Do not infer approval from their presence in that archive.
