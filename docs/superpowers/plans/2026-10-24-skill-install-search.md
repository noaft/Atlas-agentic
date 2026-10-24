# Skill install and search plan

- [x] Reproduce selected-item status, missing visible result after refresh failure, and instruction closing with failing tests.
- [x] Verify package bytes during publication; add end-to-end download/store/load regressions.
- [x] Add bounded backend batch installs with shared repository metadata and independent outcomes; bridge and UI tests.
- [x] Add backend BM25 installed-skill search and typed search bridge with ranking/context regressions.
- [x] Verify Rust/native/frontend checks, document behavior and commit focused changes.

Verification: reproduced three frontend failures before fixes (closing instructions, selected-only progress, and keeping successful installs visible when refresh fails). Backend regressions cover independent batch failures, one metadata fetch per repository revision, duplicate rejection, installed resource bytes, fresh runtime skill loading, and BM25 scope/tokenization/ranking. 432 root Rust tests, 54 native tests, and 270 frontend tests passed (756 total). Root/native formatting and Clippy with all targets/features passed; frontend and Tauri debug builds passed.
