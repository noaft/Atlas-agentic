# Windows releases from version tags

GitHub Actions builds Atlas's Windows x64 NSIS installer when a stable `vX.Y.Z`
tag is pushed. The workflow is `.github/workflows/release-windows.yml`. It uses
GitHub-hosted Windows build machines and the repository's automatic
`GITHUB_TOKEN`; no personal access token or model API key is required.

## First release

The current app version is `0.1.1`. Commit and push the workflow and application
changes before pushing a matching tag:

```bash
git push origin main
node scripts/check-release-version.mjs v0.1.1
git tag -a v0.1.1 -m "Atlas 0.1.1"
git push origin v0.1.1
```

The workflow must be present in the tagged commit. Open **Actions → Windows
release** to follow the build; the installer appears under **Releases** after
all checks, packaging and asset uploads succeed. Merely creating a local tag
does not start GitHub Actions. GitHub Actions must be enabled in repository
Settings; repository or organization policy must permit the publish job's
`contents: write` permission.

Ordinary pushes, merges and pull requests run CI tests, linting and frontend
validation. They do not invoke Tauri app builds, package installers or upload app
binaries. Only a pushed version tag starts the Windows app build and subsequent
GitHub Release publication.
Native test output is uncaptured so a process crash retains its
diagnostics. Failed core targets do not prevent the remaining core targets from
running; a sequential unit-test rerun provides additional diagnostics without
clearing the original failure.

## Later versions

Before creating the next tag, update the version in:

- `Cargo.toml`
- `desktop/src-tauri/Cargo.toml`
- `desktop/src-tauri/tauri.conf.json`
- `desktop/package.json`

Refresh the lock files without upgrading dependencies:

```bash
npm --prefix desktop install --package-lock-only --ignore-scripts
cargo check
cargo check --manifest-path desktop/src-tauri/Cargo.toml
node scripts/check-release-version.mjs v0.1.2
```

Commit the manifests and locks together. Then push that commit and a matching
annotated tag as above. The validation helper checks both npm version entries,
both Cargo manifests, and Atlas's local package entries in both Cargo lock files.
A stale lock version or mismatched tag stops the build before publishing.
Only stable tags such as `v0.1.2` are supported; prerelease and malformed tags fail
validation. Do not move or reuse published version tags.

## Build and publication

The Windows job installs Node from `.node-version` and Rust from
`rust-toolchain.toml`, runs release-helper tests, frontend tests and Rust core/
desktop tests, then invokes the locked Tauri CLI with
`desktop/src-tauri/tauri.release.conf.json`. This release-only overlay enables
NSIS packaging, the app icons and per-user installation. It retains the base
config's bundled `agents/atlas.toml`; development builds keep their current
configuration.

The helper stages exactly one nonempty `*-setup.exe` and its `.sha256` checksum.
It does not include raw build directories, credentials or user settings. The
build job has read-only repository permissions. A separate Ubuntu job downloads
the run's tested artifact, checks its checksum and the tag's current commit, and uses `gh release create
--verify-tag` to upload both assets and publish the Release. GitHub CLI holds the
release as a draft until all asset uploads succeed.

An existing release is never overwritten. An interrupted upload can leave a
draft; inspect that draft and deliberately remove it before rerunning publication
for the same tag. Build artifacts remain available from the Actions run for seven
days even if publication fails. The installer is currently unsigned; code signing
can be configured separately when a signing certificate is available.

## Local checks

```bash
node --test scripts/release.test.mjs
node scripts/check-release-version.mjs v0.1.1
```

On Windows, build the installer locally with:

```bash
npm ci --prefix desktop
npm --prefix desktop run tauri -- build --config src-tauri/tauri.release.conf.json --target x86_64-pc-windows-msvc --ci -- --locked
```

A Linux `--debug --no-bundle` build can validate the merged configuration and
application compilation, but it does not verify Windows packaging or installer
execution. The hosted Windows job provides the actual platform build.
