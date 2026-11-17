# Skill storage and runtime contracts

Implemented contracts for `src/skills/` and the Skills Settings UI.
See the [Skills user guide](../guides/reference.md#skills) for installation and use.

## Storage and imports

Skills are source-independent YAML-frontmatter `SKILL.md` folders. Project skills
live under the validated workspace's `.agents/skills/`. New Personal skills use
absolute XDG_CONFIG_HOME/agents/skills or ~/.config/agents/skills on all platforms,
matching the Universal layout. Also discover ~/.agents/skills (canonical global
CLI installs), then the legacy Atlas config skills folder. Primary wins duplicate
names; operations on an existing fallback skill resolve its actual root. Explicit
ATLAS_CONFIG_DIR and custom SettingsStore instances remain isolated. Never move
or delete existing skills automatically.

Keep personal:name/project:name identities distinct. No remote source is required.
Create validates name/title/summary/instructions and supporting UTF-8 file maps,
never overwrites, and produces one folder per skill. `create_skill` is available
even when the catalog is empty; only save when the user explicitly asks. Project
creation is ModifyWorkspace; Personal is ModifyOutsideWorkspace and requires
approval in Ask and Workspace modes. No implicit script/dependency execution.

Local imports accept a single SKILL.md (only that file) or whole skill folders /
bounded collections. Preview full instructions/resources, then bind installation
to a whole-package fingerprint. Edits preserve resources/enabled state, reject
renames, and bind their token to both content and the selected storage root so a
new primary copy cannot redirect a legacy editor. Editing a remote skill clears
remote provenance. Retain safe staging, leases, recovery, and existing bounds.

## Request snapshots and runtime tools

The `load_skill` tool exposes enabled metadata first, then frozen full instructions,
base folder, relative file list, and UTF-8 resources on demand. Invalid/disabled
skills stay out of the request catalog; valid others remain usable. Running
requests keep their snapshots; newly saved skills are available next request.
Instructions cannot grant permissions or override user/system guidance.

Preload concise enabled metadata, not full instructions; descriptions should aim
for roughly 30 tokens describing when to use the skill. Runtime catalog bounds
are 128 enabled skills, 64 KiB metadata and 32 MiB package snapshots. Preserve
omission notices and frozen resources. Ratings remain a later feature.
`preview_skill` and `install_skill` are available even with an empty catalog.
Preview uses ExternalTool approval and retains up to 8 pinned packages / 8 MiB
in memory, returning at most 64 KiB of instructions and file metadata. Install
requires a token from that runtime's successful preview, reuses its exact bytes,
and never redownloads, overwrites, executes scripts or installs dependencies.
A successful new preview replaces old tokens; tokens do not survive a request.
The runtime owns a tool request-scope guard that clears transient import state on
entry and every exit, including errors, without resetting frozen skill catalogs.
Project installation is ModifyWorkspace; Personal is ModifyOutsideWorkspace.
Never discover credentials from development .env.local for either operation.

## Settings UI

Skills Settings keeps one Add skill action and scope selection at the top. The
add panel opens on demand with one source picker (GitHub/folder/file), showing
only the selected source's controls. A separate Installed skills library shows
scoped search, installed/enabled counts and focus-preserving on/off switches.
View/edit/update actions live in each skill's collapsed Details; refresh and
other installation methods live under Library options. Group catalog issues in
a collapsed warning without dropping diagnostics or weakening symlink checks.
Closing the add panel preserves drafts/previews and does not cancel operations.
Pending reads must respect a user collapsing Details; background refresh must
preserve expanded details and editor focus/drafts.
Keep the Create form removed; runtime creation remains available. Single installs
track the selected source, preserve other candidates, and merge verified backend
results immediately even if a subsequent refresh fails. Install all uses a bounded
backend batch over reviewed pinned sources; reuse commit/tree metadata, retain
per-source results, and never implicitly overwrite existing skills. Verify staged
and published package bytes under the storage lease before returning success.
Installed instructions toggle closed without discarding cached documents.
Search skills through the Rust BM25 index over metadata and instructions, within
the selected scope; debounce frontend input and reject stale results. Edit drafts and input
focus survive navigation/background refreshes. Capture
workspace/context per operation; saved conversations ignore conflicting frontend
paths. Filesystem/network work runs on workers without broader frontend access.
Folders installed by other tools using the shared locations require no
Atlas-specific manifest.
