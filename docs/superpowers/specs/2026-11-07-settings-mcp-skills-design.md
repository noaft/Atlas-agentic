# Settings, MCP and skill discovery — proposed design

Status: earlier proposal; Skills portions are superseded by `2026-11-07-skills-installation-design.md` after the user chose preloaded descriptions and installation-first UI. MCP portions remain pending. Original status: design for user review; implementation has not started. The user selected approach 2 and requested the design before implementation. Detailed design approval remains pending. A read-only independent builder review has been incorporated.

## Intent

Make Atlas easier to configure and make installed skills easier for the agent to discover without expanding every model request with the entire skill catalog. Keep the desktop-first Rust/Tauri architecture and runtime authorization boundaries.

## Current behavior and constraints

- `desktop/src/settings-view.ts` mixes basic provider setup with model limits and uses shared Save/Discard for Models, Workspace and Permissions. MCP and Skills apply immediately.
- `desktop/src/mcp-settings.ts` lists configured servers and manages OAuth. Its Connected label reflects credential availability, not a live server test. Adding a server requires editing the user-local configuration externally.
- `desktop/src/skills-settings.ts` supports installed skills, scoped BM25 search, instruction viewing/editing, and reviewed local/GitHub imports. These operations must retain draft, focus, scope and workspace ownership behavior.
- `src/tools/load_skill.rs` freezes enabled packages for a request and embeds all catalog metadata plus an ID enum in its tool definition. Full instructions and UTF-8 supporting resources are loaded on demand.
- `src/skills/search.rs` provides local BM25 for Settings. It searches mutable storage and includes disabled skills, so it must not be wired directly into runtime discovery.
- The current runtime snapshot caps enabled skills at 64, metadata at 64 KiB, and package bytes at 32 MiB. Settings can list more installations. Do not silently remove these bounds; enabled-but-omitted installations are not available in that runtime snapshot. Preserve omission notices.
- Saved conversations determine their own workspace. Settings jobs run on backend workers. Configuration inspection never starts servers or authorization. MCP discovery/calls retain existing runtime approval, timeouts, framing and cleanup.

## Approaches

1. UI improvements only. Lowest migration cost; leaves all skill metadata in each model request and discovery entirely dependent on that catalog.
2. Search, then load (recommended). Add local runtime skill discovery over the same frozen snapshot as loading. Compact stable tool definitions; the model requests relevant metadata before loading instructions. Costs one tool round trip when discovery is necessary.
3. Automatically inject skills selected from the user prompt. Avoids the explicit search round trip but risks wrong selection, premature instruction injection and opaque behavior. Requires a separate selection policy; defer.

The user selected approach 2. Scope below describes the proposed detailed design.

## Skill discovery and loading

Use one immutable request-owned snapshot/index for both `search_skills` and `load_skill`.

1. The model sees compact descriptions of the two tools, not the full catalog or a large enum.
2. `search_skills` accepts a query of at most 512 Unicode characters (at most 32 indexed query terms), optional Personal/Project scope, result limit (default 5, maximum 10), and offset (0–64). Return at most 64 KiB serialized metadata per response. Empty query explicitly browses admitted enabled metadata in deterministic pages; pagination applies to filtered results as well as browsing. Return total matches and next offset; do not silently truncate a page without advancing consistently.
3. Return ID, title, description and scope, plus result/page metadata. Search returns no full instructions, resource bodies, local paths, source revisions or credentials. Disabled/invalid skills never enter the runtime index.
4. Reuse BM25 scoring as a pure indexing/ranking component. Keep Settings search behavior: scoped, includes disabled skills, no remote requests. Runtime search indexes the frozen instructions locally while exposing only metadata results.
5. Exact qualified IDs and exact names rank first; ambiguous unqualified names return both scope identities. No stemming/translation or semantic-search claims: lexical search can miss cross-language matches. The model may reformulate or browse.
6. `load_skill` retains its existing arguments and validates membership against the same snapshot. A user-named qualified ID may be loaded directly. Resource loading remains relative, registered, UTF-8-only and bounded.
7. Editing/disabling/installing during a running request affects the next request, never an existing snapshot. Search and load cannot disagree because of live disk changes.
8. Skill discovery/loading remains read-only and does not execute scripts, grant permission, select a new model or add routing.
9. Construct the snapshot once and register search/load together. A valid subset remains usable when other packages are invalid. Do not independently build or silently swallow a failed half of the tool pair. Keep creation available when the snapshot is empty.
10. Tool guidance asks the model to search for ordinary tasks that might benefit from a skill, as well as explicit skill mentions. Search does not run unconditionally for every greeting or direct answer. Ranking is a suggestion; results are not instructions until loaded.

No embedding dependency, extra model call for selection, persistent usage ranking, per-chat pinning, automatic execution or new creation form in this iteration.

## Skills Settings

- Keep Installed skills first, with explicit Personal/Project scope, current project folder, search and enabled/disabled counts.
- Explain Enabled as available for agent discovery; it does not mean currently loaded. Explain that running requests retain their existing snapshot.
- Keep cards compact: title/description/status and primary enable control; instructions, resources, provenance and edit actions in an expandable detail area.
- Keep instruction viewing separate from editing; preserve cached documents, unsaved editors, selection and focus during background refreshes.
- Preserve the collapsed Import from source section. Local and GitHub flows remain preview-first with pinned fingerprints and per-source outcomes.
- Empty states distinguish no installed skills, no enabled skills, no search matches and failed loading, with appropriate next actions.
- Show a clear immediate-application notice. Save/Discard for model settings never claims to control skill changes.

## MCP Settings

- Distinguish Configured, Sign-in required/Authorized/Storage error and Test not run/Testing/Verified/Failed. Authorization does not imply a working transport.
- Show the actual user configuration path supplied by Rust. Provide backend-owned setup/configuration actions; do not add frontend filesystem/shell permissions.
- Add a server editor for remote HTTP and local stdio: name, URL or command/argument list, timeout and optional OAuth fields. Keep server arguments as an explicit list, never a shell command string. Advanced OAuth fields remain collapsed.
- Validate drafts without network access or launching processes. Explicit Save writes user-local config atomically under a lease and rejects stale whole-document fingerprints; preserve unrelated servers and UTF-8 comments with a TOML document editor. The current parser rejects unknown fields: unsupported/malformed documents remain untouched with an actionable error rather than claiming compatibility. Use bounded reads, private file creation and an explicit reject-symlinks write policy, with same-directory atomic publication.
- Use explicit per-editor Save/Cancel. Existing connections keep their original configuration until a new request; communicate this in the UI. Capture the old server configuration under the configuration lease before an identity-changing edit/removal; cancel and invalidate its pending authorization before publishing the new document. Do not cancel by name after overwriting config. Failed credential-store invalidation leaves configuration unchanged and surfaces recovery controls. Preserve shared credential identities: editing/removing a server must not silently delete another server's grant. Guard asynchronous publication with captured config fingerprints and the existing invalidation-generation rules.
- Test and inspect tools is a separate deliberate action. Surface the server/process effect before execution, reuse the existing authorization path in Ask/Workspace, and never probe automatically on page entry, Refresh or Save. Cancel/denial creates no connection or subprocess.
- Prefer a narrow typed backend discovery service shared with the runtime, not executing arbitrary tool names from the frontend. Test only initialize/list tools; never invoke a discovered business tool. Give each Settings test its own operation ID and event channel, capture validated workspace and permission mode at dispatch, and render its approval in the owning MCP panel. Reuse `AuthorizationRequest`, risk classification and approval resolution; no transport starts before approval. Scoped cancel/owner teardown/channel loss cancels only that operation, never all chat approvals. Share the validated MCP execution path rather than bypassing it through a direct transport command.
- Display bounded discovered tool names/descriptions and last test time in memory. Identify the configuration fingerprint and workspace used; invalidate results on config/workspace change and discard late callbacks. A test connection is short-lived and closes on success/failure.
- Disconnect must be labeled as sign-out/removing saved authorization; explain that it does not delete the server or revoke a provider-side grant. Separate Remove configuration from sign-out. Never put tokens or raw protocol/server stderr in UI diagnostics.

## General Settings

- Keep existing navigation and mounted controls. Prioritize Provider → credentials → model; collapse endpoint overrides, capacities and optional capabilities into advanced areas as appropriate for each provider.
- Make manual model ID an explicit alternative to discovered choices and explain failed discovery without blocking manual IDs.
- Keep required custom endpoint fields visible for compatible providers.
- Clearly label Save changes as applying to model/workspace/permission drafts, while MCP/Skills use their own explicit actions.
- Preserve environment override explanations, provider-specific drafts, source validation, category scroll and concurrent chat behavior.

## Delivery sequence

1. Runtime skill discovery and snapshot sharing; focused Rust tests, then existing skill/runtime regressions. Update prompt/tool descriptions and repository docs.
2. Skills management UX; tests for empty/status states and preservation of search/editor/workspace ownership.
3. MCP honest authorization/setup states and contextual errors; retain OAuth cancellation/recovery tests.
4. MCP configuration editor with atomic validated persistence and conflict tests.
5. Explicit MCP test/tool inspection through existing approval semantics, configuration/workspace ownership and cleanup tests.
6. General model Settings simplification; keyboard/navigation and unsaved-draft regressions.

Each slice must be reviewable and keep the existing app usable. Do not replace the transport, OAuth, storage or chat architecture wholesale.

## Acceptance and verification

- Runtime tool definition size no longer grows with every installed skill's description/ID enum. All skills admitted to the bounded snapshot remain discoverable through bounded browsing; omitted installations are reported accurately.
- Search→load remains consistent after on-disk modifications; disabled skills, unsafe paths and invalid arguments are rejected. Tests cover duplicate names/scopes, exact-ID ranking, limits, pagination and empty results.
- Settings retains existing BM25 results including disabled skills. The runtime snapshot and Settings storage views intentionally have different eligibility.
- MCP never reports a live test based only on credentials. Loading, refreshing, saving and cancelling authorization do not trigger discovery or business calls.
- Tests cover config validation/conflicts, preservation of other servers, pending OAuth identity changes, approval denial, failed/late tests and owned process cleanup.
- Frontend tests cover focus/draft/scroll preservation, active session ownership and background operations. API additions stay in `atlas-api.ts`; native work stays in Rust workers.
- Run the relevant focused tests per slice, then repository Rust format/test/clippy and desktop test/build checks, including Tauri checks/build when platform prerequisites are available. Report any unavailable checks explicitly.

## Approval needed

Confirm the detailed design and delivery scope before implementation. Approach 2 has already been selected. This document is a design proposal, not a claim that the features exist or have passed tests.
