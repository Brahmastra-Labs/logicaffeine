# Versioning

This document describes the versioning strategy and release process for logicaffeine.

## Philosophy

Logicaffeine uses **lockstep versioning**: all crates in the workspace share the same version number. This simplifies dependency management and gives users a single version to track.

Lockstep is structural, not a convention: every member inherits its version from
`[workspace.package]` in the root `Cargo.toml` via `version.workspace = true`, and
internal dependencies resolve through `[workspace.dependencies]` entries in the same
file. The lockstep version therefore lives in exactly one file, and a member cannot
drift. The only exceptions are the out-of-band tools pinned at `0.0.0`
(`logicaffeine-wirebench`, `wiki-trace`).

Similar projects using lockstep: Dioxus, Bevy, Tokio, Axum.

## Semantic Versioning

All crates follow [Semantic Versioning 2.0.0](https://semver.org/):

```
MAJOR.MINOR.PATCH
```

| Component | When to Increment |
|-----------|-------------------|
| **MAJOR** | Breaking API changes |
| **MINOR** | New features (backwards compatible) |
| **PATCH** | Bug fixes only |

### Pre-1.0 Semantics

While below 1.0.0, we follow relaxed SemVer:

- `0.MINOR.0` — May contain breaking changes
- `0.MINOR.PATCH` — Bug fixes only, no breaking changes

This means `0.6.0` → `0.7.0` may break APIs, but `0.6.0` → `0.6.1` will not.

## Crate Hierarchy

Crates must be published in dependency order (leaves first). The publish workflow
(`.github/workflows/publish.yml`) derives both the publish set and its order from
`cargo metadata`, so there is no hand-maintained list to fall out of sync. The
current hierarchy, for orientation:

```
Tier 0 (no internal deps):
  logicaffeine-base
  logicaffeine-verify
  logicaffeine-forge
  logicaffeine-runtime

Tier 1 (depends on Tier 0):
  logicaffeine-data
  logicaffeine-kernel
  logicaffeine-lexicon

Tier 2 (depends on Tier 0-1):
  logicaffeine-system
  logicaffeine-proof

Tier 3 (depends on Tier 0-2):
  logicaffeine-language

Tier 4 (depends on Tier 0-3):
  logicaffeine-compile

Tier 4.5 (depends on Tier 0-4):
  logicaffeine-jit
  logicaffeine-lsp

Tier 5 (applications + validation):
  logicaffeine-cli
  logicaffeine-tv
```

## Changelogs

Each crate maintains its own `CHANGELOG.md` following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

### Format

```markdown
# Changelog

All notable changes to this crate will be documented in this file.

## [Unreleased]

## [0.6.0] - YYYY-MM-DD

### Added
- New feature description

### Changed
- Modified behavior description

### Fixed
- Bug fix description

### Removed
- Removed feature description
```

### Categories

Use these section headers (in order):

- **Added** — New features
- **Changed** — Changes to existing features
- **Deprecated** — Features marked for future removal
- **Removed** — Removed features
- **Fixed** — Bug fixes
- **Security** — Security-related fixes

### Guidelines

1. Write for users, not developers
2. Each entry should complete: "This release..."
3. Link to issues/PRs where helpful: `([#123](link))`
4. Keep entries concise (one line when possible)

## Release Process

### 1. Prepare Release

```bash
# Ensure all tests pass (full parity suite)
./scripts/run-all-tests-fast.sh

# Ensure no uncommitted changes
git status
```

### 2. Update Changelogs

For each crate with changes, and for the root `CHANGELOG.md`:

1. Move items from `[Unreleased]` to new version section
2. Add release date: `## [0.10.0] - 2026-07-15`
3. Create new empty `[Unreleased]` section

The root `CHANGELOG.md` provides a high-level summary across all crates.

### 3. Bump the Version

```bash
# Example: bump from 0.9.16 to 0.10.0
./scripts/bump-version.sh 0.9.16 0.10.0
```

The lockstep version lives in one file: the script edits the root `Cargo.toml`
(`[workspace.package] version` plus the `[workspace.dependencies]` internal
entries) and the VSCode extension's `package.json`, then runs
`cargo check --workspace` to validate the bump and refresh `Cargo.lock`.

### 4. Regenerate Benchmarks (bench box)

```bash
# On the dedicated bench box, with the box silenced
# (no other sessions, builds, or system timers competing for cores):
bash benchmarks/run.sh                 # full 11-language suite + interpreter + codec (hours)
bash benchmarks/run-solver-vs-z3.sh    # certified prover vs the field -> results/solvers.json
```

Benchmarks never run in CI — shared runners are too noisy for publishable
numbers. `run.sh` writes `results/latest.json`, `results/latest-interp.json`
and `results/latest-codec.json`, and archives `results/history/v<version>*.json`
(the version comes from the root `Cargo.toml`, so bump first). Commit the
refreshed `benchmarks/results/` — `deploy-frontend.yml` reads the checked-in
JSON at build time.

### 5. Update the Web Surfaces

- `apps/logicaffeine_web/src/ui/pages/roadmap.rs` — the version span
- `./scripts/generate-roadmap.sh` — regenerates
  `apps/logicaffeine_web/src/ui/pages/roadmap_history.json` from the changelog
  and prints a staging report; commit the result
- `apps/logicaffeine_web/src/ui/pages/news/data.rs` — the release news article
- Root `README.md` version badge

### 6. Publish Preflight

```bash
# Validates packaging of the whole publish set without touching the registry
cargo publish --workspace --dry-run

# Confirm the CARGO_REGISTRY_TOKEN repository secret is still valid
```

### 7. Commit, Tag and Push

```bash
git add -A
git commit -m "release 0.10.0 — <brief description>"
git tag v0.10.0
git push origin main --tags
```

The tag push triggers `publish.yml` (crates.io) and `release.yml` (GitHub release +
LSP binaries + VSCode extension + **10 `largo` binaries**: 5 platforms × lean/full,
the full flavor with Z3 statically linked). Benchmarks are regenerated on the
bench box before tagging (step 4); the frontend deploys from the checked-in
results when main goes green.

**Ordering note:** the installer scripts (`apps/logicaffeine_web/public/install.sh`
/ `install.ps1`, served at `logicaffeine.com/install.sh`) only update when
`deploy-frontend.yml` runs on main — so merge to main and let the frontend deploy
go green *before* announcing a release whose installers changed. The installers
themselves are version-independent (they resolve the latest tag at run time), so
routine releases need no installer edits.

Before the first tag with CLI binaries — and after any release.yml change — run
`release.yml` via `workflow_dispatch` on the branch to burn in all 10 `build-cli`
jobs (windows-full and darwin-x64-full are the slow/risky ones: Z3 builds from
vendored source, 30–60 min uncached).

### 8. Watch and Verify

`publish.yml` derives the publish set and dependency order from `cargo metadata`
and fails loudly on any error; re-runs are safe (crates already at the released
version are skipped). Afterwards verify:

- crates.io shows the new version for every published crate
- the GitHub release carries the LSP binaries and `.vsix`
- the GitHub release carries the 10 `largo-*` archives **and `SHA256SUMS`**
- the `e2e-install` jobs (linux/macos/windows, pinned to the tag) are green
- once the frontend deploy lands: `curl -fsSL https://logicaffeine.com/install.sh | sh`
  and `irm https://logicaffeine.com/install.ps1 | iex` install the new version;
  `largo --version` reports the tag (and `(full)` for `--full` installs); a lean
  `largo verify` prints the install-full hint
- `benchmarks/results/history/v<version>.json` was committed

## Unpublished Crates

The following crates are not published to crates.io:

| Crate | Reason |
|-------|--------|
| `logicaffeine-tests` | Internal test suite only (`publish = false`) |
| `logicaffeine-web` | Web application, not a library (`publish = false`) |
| `logicaffeine-synth` | Offline Z3 stencil tooling, dev-time only (`publish = false`) |
| `logicaffeine-wirebench` | Wire codec benchmark harness (`publish = false`) |
| `wiki-trace` | Internal tooling script (`publish = false`) |

The `logicaffeine-tests`, `logicaffeine-web` and `logicaffeine-synth` crates still
follow lockstep versioning for consistency. `logicaffeine-wirebench` and
`wiki-trace` are out-of-band utilities kept at `0.0.0`.

## Version History

The authoritative version history is the root [`CHANGELOG.md`](CHANGELOG.md) —
every released version with its date and cross-crate summary. Per-crate detail
lives in each crate's own `CHANGELOG.md`.
