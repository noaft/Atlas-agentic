# Native Ripgrep Tool Design

## Goal

Give Atlas a native `ripgrep` runtime tool for fast recursive text search on
Linux, macOS, and Windows. The tool must be compiled into Atlas and work when
the standalone `rg` executable is not installed.

The default Atlas agent should prefer this tool for file-content searches
because ripgrep is generally faster than traditional `grep`. This preference
does not prohibit `grep`: the agent may still use it through `exec_command`
when it is appropriate or explicitly requested.

## Product Behavior

- Register `ripgrep` in the default tool set for the CLI and desktop app.
- Search from the conversation workspace by default.
- Accept an absolute path or workspace-relative path so an agent can search any
  location readable by the Atlas OS process.
- Require Atlas approval before a search whose canonical root is outside the
  conversation workspace in both Ask and Workspace modes. Full Access skips
  Atlas confirmation without elevating OS permissions.
- Keep the existing workspace-scoped `search_files` tool for compatibility and
  for combined filename/content searches.
- Update the default agent guidance to prefer the native `ripgrep` tool for
  content search while allowing `grep` when useful.

## Tool Contract

The tool is named `ripgrep` and accepts:

- `pattern`: required nonempty regular expression.
- `path`: optional file or directory, defaulting to `.` relative to the
  workspace. Absolute paths are allowed.
- `glob`: optional ripgrep-style include or exclude glob.
- `fixedStrings`: optional boolean, default `false`, that treats `pattern` as
  literal text.
- `caseSensitive`: optional boolean, default `true`.
- `includeHidden`: optional boolean, default `false`.
- `includeIgnored`: optional boolean, default `false`, that disables ignore
  file filtering.
- `limit`: optional result count from 1 through 200, default 50.

Unknown fields are rejected. A regular expression is compiled before any file
walk begins. Normal output contains one `path:line:text` record per matching
line in deterministic path order. Paths under the workspace are rendered
relative to it; other paths are rendered as absolute paths. Long matching
lines are truncated safely on Unicode boundaries. No matches return
`No matches.`.

The total textual result is capped at 64 KiB as a second bound after the match
count. Reaching either bound returns the matches collected so far with a
truncation notice. Binary files are skipped instead of emitting arbitrary
bytes.

## Search Architecture

Atlas will use ripgrep's reusable Rust crates instead of spawning a child
process or bundling a platform-specific executable:

- `ignore` performs recursive traversal and applies `.gitignore`, `.ignore`,
  and `.rgignore` filtering.
- `grep-regex` compiles the search expression using ripgrep's regex engine.
- `grep-searcher` searches files, detects binary data, and reports matching
  lines and line numbers.

These dependencies stay behind a new `src/tools/ripgrep.rs` implementation.
Their types do not leak into the shared `Tool` trait, runtime, provider, or
agent interfaces. The implementation is synchronous like the other current
tools and stops walking as soon as its result bound is reached.

The walker follows ripgrep's safe defaults: it respects ignore files, excludes
hidden entries, skips binary data, and never follows symbolic links. Explicit
schema flags may include hidden or ignored files, but this first version does
not expose binary search or symlink following.

The constructor stores a canonical workspace path. Argument parsing resolves a
relative `path` against that workspace and canonicalizes the search root. Both
authorization and execution use the same resolver. Missing, inaccessible, or
unsupported roots fail with a path-specific error.

## Permission and Privacy Model

Add a provider-neutral `ToolRisk::ReadOutsideWorkspace` risk. Its approval
reason states that the tool reads files outside the selected workspace and may
return sensitive content to the configured model provider.

The `ripgrep` tool computes risk from the canonical search root:

- a root equal to or contained by the canonical workspace is `ReadOnly`;
- every other root is `ReadOutsideWorkspace`.

Ask mode continues to allow only `ReadOnly` without confirmation. Workspace
mode allows workspace reads but requires confirmation for
`ReadOutsideWorkspace`. Full Access skips Atlas confirmation. All modes remain
limited to files readable by the OS user running Atlas.

Symbolic links are not followed during traversal. Canonicalizing the initial
root before classification prevents a workspace-relative symlink from silently
turning into an outside-workspace search.

## Error Handling

- Empty patterns, unknown fields, invalid regular expressions, malformed
  globs, invalid limits, and missing paths fail before traversal.
- An unreadable search root fails the call with its path and underlying error.
- Unreadable descendants do not discard successful matches from readable
  files. The result includes a bounded warning count without leaking unlimited
  OS error text.
- A search stops cleanly when the match or output cap is reached.
- The tool never falls back to shelling out to `rg` or `grep`, so behavior does
  not depend on `PATH`, shell quoting, or external program installation.
- Panics from dependency APIs are not converted into normal tool behavior;
  Atlas validates inputs and uses fallible APIs at its boundary.

## Testing

Follow test-driven development and add focused coverage for:

- tool name, description, schema defaults, unknown-field rejection, and
  default registration;
- regex, fixed-string, and case-sensitive/case-insensitive matching;
- recursive deterministic results with line numbers;
- glob, hidden-file, ignore-file, binary-file, and symlink behavior;
- result-count and 64 KiB output bounds;
- invalid pattern, glob, limit, and path errors;
- workspace-relative and absolute roots;
- canonical inside/outside classification, including a relative symlink to an
  outside directory;
- confirmation behavior for `ReadOutsideWorkspace` in Ask, Workspace, and Full
  Access modes; and
- successful operation when no external `rg` executable is available.

Run the complete core, desktop Rust, frontend, and debug desktop build commands
listed in `AGENTS.md` before completion.

## Documentation

Update `README.md` and `AGENTS.md` to list `ripgrep`, explain that it is built
into Atlas, describe its default filtering, and document approval for roots
outside the workspace. Update the prior prompt documentation so it no longer
implies that Atlas requires a system-installed `rg` command.

Primary references:

- Ripgrep repository and performance overview:
  <https://github.com/BurntSushi/ripgrep>
- Ripgrep core crate guidance:
  <https://github.com/BurntSushi/ripgrep/blob/master/crates/core/README.md>
- Ripgrep filtering behavior:
  <https://github.com/BurntSushi/ripgrep/blob/master/GUIDE.md>

## Out of Scope

- Replacing or removing `search_files`
- Installing or bundling the standalone `rg` executable
- Full compatibility with every ripgrep command-line flag
- Searching binary file contents
- Following symbolic links
- Compressed-file search
- PCRE2 expressions or multiline matching
- Filename-only listing or replacement operations
