# Local skills implementation plan

**Goal:** Make skills local folder resources first, support creation/edit/local import and Universal shared locations.
**Spec:** ../specs/2026-10-24-local-skills-design.md
**Architecture:** General source-optional package/store, shared root fallback, bounded local previews; native desktop bridge; create_skill runtime tool and permission risk; primary local UI.

- [x] Store: optional provenance, shared-root resolution/fallback, create/edit and conflict checks, local file/folder preview/import with fingerprints; focused filesystem regressions.
- [x] Runtime: create_skill with validated inputs and scope-dependent permission mode tests; registered without requiring an existing skill.
- [x] Desktop: typed worker commands, native file/folder pickers, captured saved workspace authority and API adapter tests.
- [x] UI: Create and Import local first; edit document, optional GitHub details; stable/stale async/drafts tests.
- [x] Verify full Rust/desktop/frontend checks, browser workflow, independent review, docs and focused commit.

Shared DTOs: SkillDraftInput{name,title?,description,instructions,files?:record<string,string>}; LocalSkillPreview{name,title,description,content,files,path,fingerprint}; SkillDocument gains fingerprint. SkillPackage.source becomes optional; GitHub preview source stays required.
Store methods create(input,scope), update_document(id,content,fingerprint), local::preview(path)->Vec<LocalSkillPreview>, local::download(path,fingerprint)->SkillPackage. create/update preserve normal package validation/locks. SkillStore::for_settings(settingsStore,workspace)->Result<Self> selects shared/default+legacy or explicit isolated config. Existing SkillStore::new explicit roots remains isolated for tests.

Verification: 430 root Rust tests, 54 native desktop tests, and 264 frontend tests passed (748 total); root/native formatting and all-target/all-feature Clippy passed; frontend and Tauri debug builds passed. Real browser checks covered create, edit, local import, draft preservation, and mobile layout. Official skills CLI 1.7.4 verified Project folder interoperability and revealed the canonical global ~/.agents/skills fallback. Independent review fixes preserve unsaved editors, bind edits to their selected root, and keep snapshot reads tied to one resolved root.
