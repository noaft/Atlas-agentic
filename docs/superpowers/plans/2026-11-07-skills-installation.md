# Skills Installation Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Preloaded compact skill catalog, safe runtime installation, and installation-first Skills Settings.

**Architecture:** Standard skill folders and frozen runtime catalogs remain authoritative. Two tools share request-local reviewed package tokens and use existing GitHub downloader and SkillStore publication. Existing frontend APIs remain the only desktop bridge.

**Tech Stack:** Rust, serde, existing GitHub HTTP adapter, TypeScript DOM, Vitest.

**Spec:** docs/superpowers/specs/2026-11-07-skills-installation-design.md

## Global Constraints

No overwrite, script execution, implicit credentials or new frontend permissions. Personal/Project remain distinct. Preserve 64 KiB metadata/32 MiB snapshots; enabled cap 128. Preview batch <=8, package bytes <=8 MiB, output <=64 KiB. Ratings deferred. Work in current feature checkout; do not commit/push or create worktrees without a user request. User explicitly authorized execution, so proceed inline without another approval round.

## Review Focus

- Forged/expired preview tokens cannot choose new package bytes.
- Denied preview/install has no network/filesystem side effect.
- Installation retains captured workspace and scope during navigation.
- Installation failure preserves reviewed candidates and editor drafts.
- Metadata remains visible without importing full instructions into model definitions.

### Task 1: Installation-first Skills UI

Files: desktop/src/skills-settings.ts, desktop/src/skills-settings.test.ts, desktop/src/styles.css.
Interfaces: existing SkillsApi unchanged.
- [x] Update hierarchy test and add enabled-count/empty-state tests; run npm --prefix desktop test -- src/skills-settings.test.ts and observe intended failures.
- [x] Place visible add/import controls and destination first; put installed list/search/counts in a separate section. Keep asynchronous/editor ownership behavior.
- [x] Run frontend skill regressions, then full desktop suite.

### Task 2: Runtime preview/install tools

Files: src/tools/import_skill.rs (new), src/tools/mod.rs, src/app.rs, tests/app_runtime_behavior.rs.
Interfaces: register_skill_import_tools(&mut ToolRegistry, SkillStore). preview_skill {url}; install_skill {token,scope}. Shared frozen packages keyed by UUID; private source adapter for network-only test doubles.
- [x] Add unit tests using real temporary SkillStore and fake external acquisition only, covering preview/result tokens, exact installed bytes, scope risks, invalid args, limits, stale tokens and duplicate publication; run cargo test import_skill and observe missing-feature failure.
- [x] Implement bounded preview fetch/cache and scope-aware install; register tools even with empty catalog.
- [x] Verify denied runtime requests do not acquire data or create directories; run focused tests and full cargo test.

### Task 3: Catalog bounds, guidance and documentation

Files: src/skills/store.rs, src/skills/mod.rs, src/tools/load_skill.rs, src/tools/create_skill.rs, tests/skills_behavior.rs, agents/atlas.toml, README.md, AGENTS.md.
- [x] Adjust catalog regression for 100 admitted skills / 129 boundary and metadata-only output; watch 100-skill test fail.
- [x] Centralize enabled cap at 128, keep byte bounds and lazy instruction behavior. Encourage short descriptions; explain preview-before-install in tool/prompt guidance.
- [x] Document runtime tools and top-first installation UI, correcting obsolete installed-first rules under explicit user instructions.
- [x] Run Rust/desktop/Tauri formatting, tests, clippy and debug build; inspect diff and run independent review, address actionable findings. Record command outcomes below.

## Progress and rulings

- Scope: this delivery implements the revised Skills request. The larger MCP redesign has not been approved in detail and is not included.
- Storage: retain standard folders and immutable in-memory runtime metadata; no persistent index/database without measured need.
- Baseline: current branch feat/ai-stock-comedy-studio; initial tracked working tree clean. Earlier proposal is retained as an untracked conversation artifact if present.

- Review ruling: per-runtime state alone did not enforce per-request token expiry when Runtime is reused. Added default no-op Tool reset hook and a registry request-scope guard; tokens clear on entry and success/error exits. Two-ask regression reproduced the issue before the fix. Same-request success, actual approval denial and interrupted-model cleanup are covered.
- Review minor: existing GitHub preview can fetch up to its own bounded 64 candidates before the runtime's narrower eight-candidate rejection. Kept existing acquisition behavior; runtime retained packages/output remain bounded and the UI supports larger reviewed batches.
- Baseline Rust suite passed. UI red: two intended hierarchy/count failures. UI green: 28 focused tests and all 271 desktop tests. Catalog red: 100 skills produced 64 entries; green: 100 entries, 128 cap. Token expiry red reproduced cross-ask installation; fix verified in focused tests.
- Final verification after lifecycle fix: cargo test (442 passed), desktop Tauri cargo test (54 passed), npm --prefix desktop test (271 passed), both cargo fmt --check commands, both all-targets/all-features Clippy commands with -D warnings, frontend production build and Tauri debug/no-bundle build all exit 0. git diff --check passed.
- Independent final review: P2 request expiry resolved; focused reviewer confirmation 9 import-tool tests passed. No remaining important findings. No live provider/download credentials were used. Interactive desktop visual inspection was not performed.
- Delivery: all three tasks complete; changes were subsequently authorized for a local commit by the user; no publication.
