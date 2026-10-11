# Native Ripgrep Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compile ripgrep's reusable Rust libraries into Atlas and expose a default `ripgrep` tool that can search workspace or approved machine paths without an installed `rg` executable.

**Architecture:** A focused `RipgrepTool` owns argument validation, path resolution, risk classification, ripgrep traversal, matching, and bounded rendering behind the existing provider-neutral `Tool` trait. A new `ReadOutsideWorkspace` risk distinguishes ordinary workspace reads from searches that may disclose machine data, while the runtime's existing approval flow remains unchanged.

**Tech Stack:** Rust 2021, `ignore`, `grep-regex`, `grep-searcher`, Serde, existing Atlas `Tool` and permission APIs.

**Spec:** `docs/superpowers/specs/2026-10-11-native-ripgrep-tool-design.md`

## Global Constraints

- The tool name is exactly `ripgrep`; keep the existing `search_files` tool.
- No external `rg` or `grep` process may be required or used by the tool.
- `pattern` is required; `path` defaults to `.`, relative to the workspace.
- Absolute paths are allowed, subject to OS-user permissions and Atlas authorization.
- Default filtering respects ignore files, skips hidden and binary files, and never follows symlinks.
- `limit` defaults to 50 and is constrained to 1 through 200; textual output is capped at 64 KiB.
- Ask and Workspace modes confirm `ReadOutsideWorkspace`; Full Access does not.
- The agent prefers native ripgrep because it is generally faster, but `grep` remains allowed.
- Follow strict red-green-refactor cycles and preserve unrelated worktree changes.
- The repository requires one focused implementation commit after all tasks and verification.

## Review Focus

- A workspace-relative root that resolves through a symlink outside the workspace must receive `ReadOutsideWorkspace`, not `ReadOnly`; Task 3 adds this authorization test.
- A single-file search root must be searched directly and render the same path/line format as a directory result; Task 2 adds this behavior test.
- A result crossing 64 KiB by one match must remain valid UTF-8 and include a truncation notice without exceeding the cap; Task 3 adds this boundary test.
- A FIFO or other unsupported special-file root must fail before opening it and must never block the runtime; Task 3 adds a Unix-only FIFO test.
- Empty `PATH` must not affect tool execution, proving there is no hidden dependency on an installed executable; Task 3 adds a child-process isolation test.

---

### Task 1: Outside-workspace read permission

**Files:**
- Modify: `src/permissions.rs`
- Modify: `tests/permission_behavior.rs`

**Interfaces:**
- Produces: `ToolRisk::ReadOutsideWorkspace` with `reason() -> "reads files outside the selected workspace and may send sensitive content to the configured model provider"`.
- Produces: `requires_confirmation(PermissionMode, ToolRisk)` behavior used by `RipgrepTool` in Task 3.

- [ ] **Step 1: Add failing permission matrix tests**

Extend `permission_matrix_matches_the_three_modes` with literal assertions that `ReadOutsideWorkspace` requires confirmation in Ask and Workspace modes, never in Full Access, and has the exact reason above.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test --test permission_behavior permission_matrix_matches_the_three_modes`

Expected: compilation fails because `ToolRisk::ReadOutsideWorkspace` does not exist.

- [ ] **Step 3: Implement the new risk**

Add `ReadOutsideWorkspace` to `ToolRisk`, its `reason()` match, and the Workspace confirmation match in `src/permissions.rs`. Ask mode already confirms every non-`ReadOnly` risk.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run: `cargo test --test permission_behavior permission_matrix_matches_the_three_modes`

Expected: PASS.

### Task 2: Native ripgrep contract and matching

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/tools/ripgrep.rs`
- Modify: `src/tools/mod.rs`
- Create: `tests/ripgrep_behavior.rs`

**Interfaces:**
- Consumes: existing `Tool`, `ToolDefinition`, `ToolResult`, and `ToolRisk` APIs.
- Produces: `pub struct RipgrepTool`.
- Produces: `pub fn RipgrepTool::new(workspace: impl AsRef<Path>) -> Self`.
- Produces: `Tool` methods `name`, `definition`, `risk`, and `execute` with the JSON schema from the spec.
- Produces privately: `fn resolve_root(&self, path: Option<&str>) -> Result<PathBuf, String>` shared by authorization and execution.

- [ ] **Step 1: Add ripgrep dependencies**

Add semver-compatible current releases from the official ripgrep project: `grep-regex = "0.1"`, `grep-searcher = "0.1"`, and `ignore = "0.4"`. Run `cargo check` once to resolve `Cargo.lock`; do not add PCRE2 or a bundled executable.

- [ ] **Step 2: Write failing public-contract and core-search tests**

Create `tests/ripgrep_behavior.rs` with tests named:

- `definition_exposes_the_bounded_native_search_contract`
- `searches_regex_recursively_in_deterministic_order`
- `searches_a_single_file_root`
- `supports_fixed_strings_case_and_glob_controls`
- `rejects_unknown_fields_invalid_patterns_globs_limits_and_paths`

Use temporary real files. Assert the exact tool name, required `pattern`, schema defaults and limits, literal `path:line:text` output, `No matches.`, fixed-string behavior for regex metacharacters, case toggling, include/exclude globs, and actionable error fragments.

- [ ] **Step 3: Run the new test target and verify RED**

Run: `cargo test --test ripgrep_behavior`

Expected: compilation fails because `RipgrepTool` is not exported.

- [ ] **Step 4: Implement the schema, parser, matcher, and deterministic search**

In `src/tools/ripgrep.rs`, deserialize a private `RawArgs` with `#[serde(rename_all = "camelCase", deny_unknown_fields)]`, validate all exact bounds, and compile with `grep_regex::RegexMatcherBuilder`. Use `regex::escape` behavior supplied by the matcher builder's fixed-string/literal configuration rather than hand-written escaping.

Use `ignore::WalkBuilder` with filename sorting and an `ignore::overrides::OverrideBuilder` for the optional glob. Search each regular file using `grep_searcher::SearcherBuilder` with line numbers and binary quit detection. Return records in deterministic path order. Export `RipgrepTool` from `src/tools/mod.rs`.

- [ ] **Step 5: Run the new test target and verify GREEN**

Run: `cargo test --test ripgrep_behavior`

Expected: all Task 2 tests pass.

### Task 3: Filtering, bounds, and path authorization

**Files:**
- Modify: `src/tools/ripgrep.rs`
- Modify: `tests/ripgrep_behavior.rs`
- Modify: `tests/permission_behavior.rs`

**Interfaces:**
- Consumes: `ToolRisk::ReadOutsideWorkspace` from Task 1 and `RipgrepTool` from Task 2.
- Produces: dynamic `RipgrepTool::risk(&Value) -> Result<ToolRisk, String>` classification using the canonical root.
- Produces: bounded results no longer than 65,536 UTF-8 bytes and no more than 200 matches.

- [ ] **Step 1: Write failing filtering and safety tests**

Add tests named:

- `respects_hidden_ignore_binary_and_symlink_defaults`
- `include_flags_can_search_hidden_and_ignored_text`
- `bounds_match_count_and_utf8_output_bytes`
- `classifies_canonical_inside_and_outside_roots`
- `relative_symlink_root_is_classified_outside`
- `rejects_a_special_file_root_without_blocking`
- `works_with_an_empty_path_environment`

The symlink test is `#[cfg(unix)]`; create an outside directory and a workspace symlink to it, then assert `ReadOutsideWorkspace`. For the output boundary, use a multibyte match that crosses 65,536 bytes and assert `message.len() <= 65_536`, no Unicode replacement character, and a truncation notice. The Unix-only special-file test creates a FIFO with `libc::mkfifo`, calls the tool from a thread, and requires a prompt error result over a bounded channel receive. For empty `PATH`, spawn the current test executable with only a child-helper environment marker and `PATH=""`, then execute `RipgrepTool` in the exact helper test.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test --test ripgrep_behavior`

Expected: new filtering, risk, or bound assertions fail because Task 2 implements only the core path.

- [ ] **Step 3: Implement default filtering and explicit overrides**

Configure `WalkBuilder` so hidden entries and symbolic links stay excluded, standard ignore files are active by default, and `includeHidden`/`includeIgnored` toggle only their documented filters. Count descendant traversal errors and append one bounded warning summary while preserving collected matches.

- [ ] **Step 4: Implement canonical path risk classification**

Canonicalize the workspace and requested root through `resolve_root`. Return `ReadOnly` only when `root == workspace || root.starts_with(&workspace)`; otherwise return `ReadOutsideWorkspace`. Re-resolve at execution so a removed or retargeted root fails rather than using stale authority.

- [ ] **Step 5: Implement result bounds**

Stop matching and walking at `limit`. Render each excerpt without splitting a Unicode scalar value, reserve room for `\n[results truncated]`, and guarantee the final `String` is at most 65,536 bytes.

- [ ] **Step 6: Run ripgrep and permission tests and verify GREEN**

Run: `cargo test --test ripgrep_behavior`

Run: `cargo test --test permission_behavior`

Expected: both test targets pass with no failures.

### Task 4: Default registration and agent guidance

**Files:**
- Modify: `src/app.rs`
- Modify: `agents/atlas.toml`
- Modify: `tests/agents_behavior.rs`

**Interfaces:**
- Consumes: `RipgrepTool::new(workspace_root)` from Task 2.
- Produces: `ripgrep` in every default runtime's injected tool definitions.
- Produces: agent guidance that native ripgrep is generally faster and preferred for content search while `grep` remains permitted.

- [ ] **Step 1: Write failing registration and prompt tests**

Add `"ripgrep"` to the exact expected default-tool list in `src/app.rs` tests. Replace the obsolete `"do not use grep"` prompt assertion with assertions for `"generally faster"`, `"prefer"`, and explicit language that grep remains allowed.

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test app::tests::default_tools_include_the_workspace_file_family_and_machine_clock`

Run: `cargo test --test agents_behavior default_atlas_prompt_guides_general_computer_work_without_embedding_tool_protocol`

Expected: registration lacks `ripgrep`, and the current prompt forbids grep and describes shell `rg` rather than the native tool.

- [ ] **Step 3: Register the tool and update the prompt**

Import `RipgrepTool` in `src/app.rs` and construct it in `build_default_tools`. Update `agents/atlas.toml` without embedding JSON protocol or hard-coding its schema; remove the instruction that forbids grep and the implication that ripgrep must run through shell access.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the two commands from Step 2.

Expected: both pass.

### Task 5: Documentation, full verification, and traceable commit

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify if needed for product direction: `VISION.md`
- Review: every file changed by Tasks 1-4

**Interfaces:**
- Consumes: the completed tool contract and permission behavior.
- Produces: accurate user/developer documentation and one focused implementation commit.

- [ ] **Step 1: Update documentation**

List `ripgrep` beside `search_files` in README and AGENTS. State that it is compiled into Atlas, searches content using ripgrep's default filtering, accepts absolute paths, requests approval outside the workspace in Ask and Workspace modes, and does not require `rg` on `PATH`. Correct the earlier README wording that says Atlas shells out to `rg`. Update VISION only if its current capability description would otherwise become false.

- [ ] **Step 2: Update documentation coverage**

Add `ripgrep` to `readme_documents_the_complete_default_toolset_and_permission_modes` in `tests/agents_behavior.rs`. Run that focused test and verify it passes.

- [ ] **Step 3: Run core verification**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all commands exit 0 with no test failures or Clippy warnings.

- [ ] **Step 4: Run desktop verification**

Run:

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm --prefix desktop test
npm --prefix desktop run build
npm --prefix desktop run tauri -- build --debug --no-bundle
```

Expected: every command exits 0. If a platform dependency prevents the Tauri build, report the exact blocker and keep all earlier evidence.

- [ ] **Step 5: Review and commit only task files**

Run `git status --short`, review `git diff`, stage only the implementation and documentation files from this plan, review `git diff --cached`, and verify no secrets, `.env`, `target/`, or unrelated changes are staged.

Commit once with:

```bash
git commit -m "feat: add native ripgrep search tool"
```
