# File and computer tool contracts

Implemented backend contracts. Read before changing structured file helpers, search,
computer input, or screen capture. User examples are in the [reference](../guides/reference.md).

## Directory listing, file reads, and atomic patches

`ls` defaults to 100 entries per page (1–1000), accepts `offset` up to 100000,
and bounds text/JSON output to 64 KiB. `format: "json"` uses the same
`{data,truncated,next_cursor}` envelope as `read_file`, with typed file/directory/
symlink/other entries. Merge its cursor with the same directory `path`. Child
symlinks are classified without following them. Pagination reflects the current
directory rather than a snapshot. Beyond the offset cap, a truncated JSON page
sets `data.pagination_limit_reached` and has no next cursor; narrow the directory
or search instead. Retain only the smallest offset+limit+1 names while scanning.

### File reads

`read_file` keeps plain text by default. Optional `start_line`/`end_line` select
inclusive 1-based lines; `line_numbers` adds labels. `format: "json"` returns
`{data,truncated,next_cursor}` with content and source byte positions. Merge a
returned cursor with `path` to resume: byte cursors are unnumbered and preserve
remaining range lines through `line_count`. Cursors assume unchanged file bytes.
Output is bounded to 64 KiB including JSON/labels. Line-start scans are bounded
to 32 MiB; use `byte_offset` beyond that. Byte offsets must be UTF-8 boundaries.

### Atomic patches

`apply_patch` accepts one relative path and 1–64 sequential `{old_text,new_text}`
edits. Source, result and aggregate replacement input are bounded to 1 MiB. Old
text must match exactly once, including overlapping occurrences. Preserve bytes
outside edits and Unix permission bits. Reject symlink/special-file targets and
unsafe parents. Capture the full preimage at authorization and verify its content
and inode under a bounded source-file lease before same-directory atomic replace;
re-resolve the original workspace path and parent identity at execution/publication,
and reject moved/replaced parents rather than writing through a detached handle;
stale approvals must not overwrite newer files. Patch overwrites use Destructive
risk in Ask and Workspace modes. No shell execution or partial edit publication.

## Search outside the workspace

`ripgrep` is compiled into Atlas and does not require an `rg` executable on
`PATH`. It defaults to the workspace, respects ignore files, skips hidden and
binary files and symlinks, and accepts absolute roots. A canonical root outside
the workspace requires approval in Ask and Workspace modes; Full access skips
that Atlas confirmation but remains subject to OS-user permissions.

## Permissions and computer access

Runtime permission modes are `ask`, `workspace`, and `fullAccess`. Ask permits
read-only tools automatically. Workspace also permits non-destructive structured
workspace changes. Outside-workspace searches, destructive actions, launches,
and commands still require an approval in Workspace mode. Desktop observation
and mouse/keyboard control require approval in both Ask and Workspace modes
because they can expose or change sensitive state. Full access skips Atlas
confirmations but never
elevates beyond the Atlas process's OS-user permissions. The Settings default
applies to new drafts; a saved conversation stores and restores its own mode.

### Computer input

Computer input targets the application active at execution time. Windows may
reject input into a higher-integrity process, macOS requires user-granted
Accessibility access, and Linux currently requires a usable X11 display.
Wayland-only sessions return an actionable error. Keep these capabilities in
the Rust backend; do not broaden frontend Tauri shell, process, accessibility,
screen-capture, or filesystem permissions.

### Screen capture

`capture_screen` uses `ComputerObserve`. Ask and Workspace modes require
approval because visible desktop data may be sent to the configured model
provider. Captures are primary-display-only, remain in memory for the current
turn, and are never written to disk or conversation history. The DeepSeek
adapter rejects images over 8192 pixels per side or 32 MiB. Windows uses native
capture, macOS requires Screen Recording permission, and Linux requires X11.
Linux development/runtime environments need `libxcb`, `libxrandr`, and `dbus`
(Debian/Ubuntu packages: `libxcb1 libxrandr2 libdbus-1-3`).
Building the OAuth credential-store integration also requires `libdbus-1-dev`
and `pkg-config` on Debian/Ubuntu; runtime sign-in requires an unlocked desktop
Secret Service session.
