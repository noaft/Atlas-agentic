# Development and verification

Use the pinned Rust and Node toolchains. Start the app using the
[quick start](../../README.md#run-the-desktop-app); follow [coding rules](../../RULE.md).

## CI and toolchains

Use the pinned Rust toolchain in `rust-toolchain.toml` (including rustfmt/clippy)
and Node version in `.node-version`. CI independently validates workflows,
frontend tests/build, Rust core, and the native desktop bridge tests/lints.
Windows core/desktop tests run on ordinary pushes and pull requests before a
release tag, with uncaptured output for native crash diagnosis. Native Tauri app
builds, NSIS packaging and installer uploads run only in the version-tag release
workflow; ordinary CI does not build or upload desktop app binaries. Failed Windows
core targets do not suppress the other targets; a sequential unit-test rerun is
diagnostic only and never clears the original failure. Keep action
references pinned to reviewed commit SHAs and preserve locked dependency checks.
Frontend CI also rejects high/critical npm advisories; use compatible reviewed
dependency updates rather than forcing unrelated major upgrades.

## Checks

Format and test the repository:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm --prefix desktop test
npm --prefix desktop run build
npm --prefix desktop run tauri -- build --debug --no-bundle
```

## Test isolation

CLI binary tests isolate their workspace and `ATLAS_DATA_DIR`, use child-only
environment configuration, and serve fake DeepSeek responses over loopback TCP.
Do not use real API keys or external model services in tests.
