# Atlas documentation

This directory contains references for implemented features and designs explicitly
approved by the user. Each design states its implementation status; approval does
not imply that it is already available in the product.

| Document | Status |
| --- | --- |
| [Atlas reference](guides/reference.md) | Detailed configuration and current-feature guide |
| [Tool/resource lifecycle](architecture/tool-resource-lifecycle.md) | Approved for implementation 2026-11-09; desktop process scopes implemented |
| [Demo and Tauri walkthrough](guides/process-manager-demo.md) | Local demo without an API key and UI instructions |
| [Wakeup](architecture/wakeup.md) | Implemented hard/lazy protocol, scheduler, mailbox, and durable bounded results; verified |
| [Tool execution control: hard/lazy](architecture/tool-execution-control.md) | Approved 2026-11-13; tool-only implementation verified; replaces job/task, tracks every tool, and stores durable results; user-message queuing deferred |
| [Delegated agents](architecture/delegated-agents.md) | Approved for implementation 2026-11-13; runtime and desktop child monitoring implemented |
| [Process sessions](architecture/process-sessions.md) | Current command lifetime and ownership reference, 2026-11-09 |
| [UniKey on WSLg](platforms/wslg-unikey.md) | Reproduction notes and verified workaround |

## Drafts and approval workflow

- Design drafts: `.superpowers/specs/`.
- Plans, ledgers, logs, and previews: their corresponding `.superpowers/` directories.
- These files stay local and Git-ignored, and are absent from fresh clones.
- A request to write documentation authorizes drafting, not design approval.
- After approval, move the agreed version into the appropriate `docs/` section,
  state its status, repair links, and update the table above. Commit only approved
  documents; keep plans/logs local.
- Never use `git add -f` to add Superpowers drafts or artifacts to the repository.

Earlier workflow documents were archived unchanged locally in
`.superpowers/archive/2026-11-09-docs/` with a SHA-256 manifest. Older versions also
remain in Git history. The archive is historical storage, not a list of approved designs.
