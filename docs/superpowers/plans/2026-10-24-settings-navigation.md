# Settings navigation implementation plan

**Goal:** Implement the approved interactive mockup as real, separate Settings pages with a category sidebar and Back to chat navigation.

**Design:** Clicking Settings replaces the conversation sidebar with Back to chat and Models & providers, MCP connections, Workspace, Permissions. Only the selected category is visible. Existing pastel colors gain consistent cards, spacing and button styles. Category switching and Back to chat retain drafts; Discard explicitly resets all edits. Save changes stays in Settings; the previous programmatic save default remains compatible. MCP connections remain immediate operations with their own status, outside the model-settings save flow.

**Architecture:** Add a global frontend Settings section and navigation methods in the existing ChatStore. Render the category sidebar from typed descriptors. Retain mounted category content during navigation and preserve the MCP panel across updates. Do not change native permissions, backend APIs, model/provider behavior or chat ownership. On narrow windows Settings navigation stays accessible above content; chat retains its existing collapsible sidebar.

## Tasks

- [x] Add failing state tests for category navigation, Back draft retention, explicit Discard, late model discovery, and saving without leaving Settings; implement the state methods and dirty comparison.
- [x] Add failing view tests; implement category sidebar, breadcrumb, accessible Back navigation and restoration of the existing chat/composer.
- [x] Split and style the Settings content, retaining real provider controls and connection state; test category isolation, field identity, saves and responsive navigation.
- [x] Run frontend tests/build, required Rust formatting/tests and relevant desktop checks; review, document and create one focused commit.

## Review focus

Do not erase masked credential drafts or per-provider edits during navigation, persist unsaved changes, replay discovery results owned by a departed screen, destroy an active OAuth panel, or hide navigation when the chat sidebar was collapsed. Preserve active chat identity, composer draft, background approvals, scroll and settings inputs. Empty and unknown model limits remain unknown; all model changes still follow current source validation.

## Validation

- Frontend: 237 tests pass; TypeScript and Vite production build pass.
- Rust: 385 root tests and 52 desktop backend tests pass; both formatting and strict Clippy checks pass.
- Native packaging: Tauri debug build without bundling passes.
- Chromium with a local mock API: desktop (1280 × 900) and narrow (390 × 844) layouts inspected; no horizontal page overflow. Verified category navigation, retained private drafts across Back/reopen, permission arrow keys, and native validation revealing hidden fields.
- Independent review resolved retained-draft model conflicts, keyboard focus loss, and hidden-category validation. No backend API or permission changes.
