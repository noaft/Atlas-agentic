# Atlas skills

Approved direction: add a Skills Settings page, Personal/Project installation, GitHub-link preview/import, and model-visible skill metadata with lazy `load_skill` instructions. Built-in marketplace search is deferred. Personal means shared across local projects; no bundled skills are needed for v1.

Use standard YAML-frontmatter SKILL.md folders (name, description; optional title, falling back to name). Personal skills live under the Atlas config directory's skills folder; project skills live at <starting workspace>/.agents/skills. IDs include scope, so duplicate names are unambiguous. Disabled or malformed skills are excluded from the model catalog. Surface bounded validation issues in Settings without breaking ordinary chat.

The Rust loader adds one read-only load_skill tool when enabled skills exist, with catalog metadata in its definition. Full skill text is returned on demand. Supporting references can be loaded through a bounded resource argument on that same tool. Skill instructions never grant permissions or run scripts during import/load. Existing runtime budget checks and authorization still govern execution.

GitHub import supports public HTTPS repository and folder URLs. Preview resolves the reference to an immutable commit, lists matching skills and shows instructions/source before installation. Install downloads the selected whole folder at that pinned commit, validates relative paths, symlinks, bounds and metadata, and atomically publishes it. No git hooks, package installers, credential reuse, or script execution. Remember source/reference/revision for explicit update previews. Duplicate installation is rejected; explicit updates preserve enabled state.

Settings operations are immediate, independent of model Save/Discard, with errors, busy states, retained input/preview, and stale-result ownership. Project actions derive saved workspace authority from conversation metadata; drafts use a validated starting path/default. All Tauri calls go through atlas-api.ts and blocking work uses workers.

Approaches considered: invoke the external skills CLI (extra Node/runtime and destination coupling), or a native GitHub importer (chosen for existing Rust HTTP/runtime boundaries and explicit per-scope storage). Deferred marketplace API search can later supply source URLs to the same importer.
