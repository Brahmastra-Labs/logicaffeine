# Versioning

How Logicaffeine numbers its releases, how a release's changes are written down, and the runbook
that ships one.

## Philosophy

Logicaffeine uses **lockstep versioning**: all crates in the workspace share the same version number. This simplifies dependency management and gives users a single version to track.

Lockstep is structural, not a convention: every member inherits its version from
`[workspace.package]` in the root `Cargo.toml` via `version.workspace = true`, and
internal dependencies resolve through `[workspace.dependencies]` entries in the same
file. The lockstep version therefore lives in exactly one file, and a member cannot
drift. The only exceptions are the out-of-band tools pinned at `0.0.0`
(`logicaffeine-wirebench`, `wiki-trace`).

Dioxus and Bevy version their workspaces the same way.

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
(`.github/workflows/publish.yml`) hands the whole set to one `cargo publish`: every member without
a `publish` key that crates.io does not yet hold at its version. Cargo orders the set by dependency
itself, so there is no hand-maintained list to fall out of sync. A crate's tier is the length of its
longest chain of internal dependencies (`tier_map_lock` holds this list to the manifests). The
current hierarchy, for orientation:

```
Tier 0 (no internal deps):
  logicaffeine-base
  logicaffeine-forge
  logicaffeine-lexicon
  logicaffeine-runtime
  logicaffeine-verify

Tier 1 (depends on Tier 0):
  logicaffeine-data
  logicaffeine-kernel

Tier 2 (depends on Tier 0-1):
  logicaffeine-proof
  logicaffeine-system

Tier 3 (depends on Tier 0-2):
  logicaffeine-language

Tier 4 (depends on Tier 0-3):
  logicaffeine-compile

Tier 5 (depends on Tier 0-4):
  logicaffeine-jit
  logicaffeine-lsp
  logicaffeine-tv

Tier 6 (depends on Tier 0-5):
  logicaffeine-cli
```

## Changelogs

The root `CHANGELOG.md` carries each release's cross-crate entry, and each crate keeps its own
`CHANGELOG.md` with the detail for that crate. Both follow
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

### Format

```markdown
# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.12.0] - YYYY-MM-DD

### Added
- **A bold sentence that says what a user can now do.** The specifics: what it does, how to call
  it, and the tests that hold it (`example_lock`).

### Breaking changes
- **A bold sentence that says what now behaves differently.** What the previous release did, what
  this one does, and the tests that hold it. Migration: the exact change a user makes.

### Fixed
- **A bold sentence that says what was wrong, stated as what is right now.** The specifics.
```

### Categories

Use these section headers, in this order, and only the ones a release needs:

- **Highlights** — a short lead: the few things a reader should know about the release first
- **Added** — new features
- **Breaking changes** — anything that makes code, a project or a certificate that worked on the
  previous release fail or behave differently; every bullet ends with a `Migration:` sentence
- **Changed** — changes to existing behavior that break nothing
- **Deprecated** — features marked for future removal
- **Removed** — removed features
- **Fixed** — bug fixes
- **Security** — security-related fixes

### Guidelines

1. Write for users, not developers.
2. Open each bullet with a bold headline sentence that says what changed. Then give the specifics a
   user needs (what it does, how to use it, what to migrate) and name the tests that hold it. A
   bullet runs as long as those specifics need, and no longer.
3. A breaking change says what the previous release did and what this one does, and ends with
   `Migration:` and the exact change a user makes.
4. The release entry covers every change since the previous tag, not only the latest work.
5. Cite only what the public tree carries: from 0.11.0 on, every backticked lock names a suite in
   `crates/logicaffeine_tests/tests/` and every backticked path exists (`changelog_citation_lock`).
6. Link to issues/PRs where helpful: `([#123](link))`.

### What the release gate checks

`scripts/release/check-changelogs.py` compares each published crate's shipped code (`src/`,
`build.rs`, `assets/`, `stencils/`, `templates/` and the manifest's `[features]`) with the newest
release crates.io holds. A crate whose shipped code changed needs at least one bullet (a line
starting `- `) in its pending section: `## [<workspace version>]` once rolled, `## [Unreleased]`
before. The roll's `Synced to workspace version` line does not count. `publish.yml` runs the check
before anything uploads, and `changelog_release_gate_lock` holds both the order and the check.

## Release Process

### 1. Prepare Release

```bash
# Ensure all tests pass (the full suite)
./scripts/run-all-tests-fast.sh

# Ensure no uncommitted changes
git status
```

### 2. Update Changelogs

For each crate with changes, and for the root `CHANGELOG.md`:

1. Move items from `[Unreleased]` to new version section
2. Add release date: `## [0.11.0] - 2026-10-04`
3. Create new empty `[Unreleased]` section

The root `CHANGELOG.md` provides a high-level summary across all crates.

### 3. Bump the Version

```bash
# Example: bump from 0.10.1 to 0.11.0
./scripts/bump-version.sh 0.10.1 0.11.0
```

The lockstep version lives in one file. The script edits the root `Cargo.toml`
(`[workspace.package] version` plus the `[workspace.dependencies]` internal entries), then
rewrites every surface that can't inherit it:

- the VSCode extension's `package.json` and `package-lock.json`;
- the README version badge, the roadmap hero span, and the `install.ps1` example pin.

It finishes with `cargo check --workspace` to validate the bump and refresh `Cargo.lock`. Every one
of those surfaces is pinned: `version_lock.rs` holds them to the workspace version, so a missed
surface goes red in the suite instead of shipping stale. `bump_version_script_lock.rs` runs the
script against a scratch tree to hold it to moving the lockstep version and nothing else.

### 4. Regenerate Benchmarks (bench box)

```bash
# On the dedicated bench box, with the box silenced
# (no other sessions, builds, or system timers competing for cores):
bash benchmarks/run.sh                 # full 11-language suite + interpreter + codec (hours)
bash benchmarks/run-solver-vs-z3.sh    # certified prover vs the field -> results/solvers.json
```

Benchmarks never run in CI: shared runners are too noisy for publishable numbers. `run.sh` writes
`results/latest.json`, `results/latest-interp.json` and `results/latest-codec.json`, and archives
`results/history/v<version>*.json` (the version comes from the root `Cargo.toml`, so bump first).
Commit the refreshed `benchmarks/results/`, because `deploy-frontend.yml` reads the checked-in JSON
at build time.

### 5. Update the Web Surfaces

The roadmap's version span and the README badge already moved with the bump (step 3).

- `./scripts/generate-roadmap.sh` — regenerates
  `apps/logicaffeine_web/src/ui/pages/roadmap_history.json` from the changelog
  and prints a staging report; commit the result
- `apps/logicaffeine_web/src/ui/pages/news/data.rs` — the release news article, dated the tag day
- `apps/logicaffeine_web/src/sitemap.rs` — `CURRENT_DATE` set to the same day, which every
  `<lastmod>` carries (`sitemap_lastmod_is_current` refuses an article dated after it). Then
  regenerate the shipped sitemap:
  `cargo nextest run -p logicaffeine-web -E 'test(regenerate_shipped_sitemap)' --run-ignored all`
  (`shipped_sitemap_is_current` holds `public/sitemap.xml` byte-equal to the generator)

### 6. Publish Preflight

The two checks `publish.yml` runs before it uploads anything, run locally first:

```bash
# Every published crate built from exactly the tarball crates.io will receive, every docs.rs
# feature set, and every tarball inside the size budget
./scripts/check-packaging.sh

# Every crate whose shipped code changed since its crates.io release says what changed
python3 scripts/release/check-changelogs.py

# Confirm the CARGO_REGISTRY_TOKEN repository secret is still valid
```

### 7. Commit, Tag and Push

```bash
git pull --rebase origin main
git add -A
git commit -m "release 0.11.0 — <brief description>"
git tag v0.11.0
git push origin main --tags
```

The tag push triggers `publish.yml` (crates.io) and `release.yml` (GitHub release +
LSP binaries + VSCode extension + **10 `largo` binaries**: 5 platforms × lean/full,
the full flavor with Z3 statically linked). Benchmarks are regenerated on the
bench box before tagging (step 4); the frontend deploys from the checked-in
results when main goes green.

**Ordering note:** the installer scripts (`apps/logicaffeine_web/public/install.sh`
/ `install.ps1`, served at `logicaffeine.com/install.sh`) only update when
`deploy-frontend.yml` runs on main. Merge to main and let the frontend deploy
go green *before* announcing a release whose installers changed. The installers
themselves are version-independent (they resolve the latest tag at run time), so
routine releases need no installer edits.

Before the first tag with CLI binaries, and after any `release.yml` change, run
`release.yml` via `workflow_dispatch` on the branch to burn in all 10 `build-cli`
jobs. The windows-full and darwin-x64-full jobs are the slow, risky ones: Z3 builds
from vendored source, 30–60 min uncached.

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
| `logicaffeine-tests` | Every test in the workspace (`publish = false`) |
| `logicaffeine-web` | Web application, not a library (`publish = false`) |
| `logicaffeine-synth` | Offline Z3 stencil tooling, dev-time only (`publish = false`) |
| `logicaffeine-wirebench` | Wire codec benchmark harness (`publish = false`) |
| `wiki-trace` | Internal tooling script (`publish = false`) |

The `logicaffeine-tests`, `logicaffeine-web` and `logicaffeine-synth` crates still
follow lockstep versioning for consistency. `logicaffeine-wirebench` and
`wiki-trace` are out-of-band utilities kept at `0.0.0`.

## Version History

The authoritative version history is the root [`CHANGELOG.md`](../../CHANGELOG.md) —
every released version with its date and cross-crate summary. Per-crate detail
lives in each crate's own `CHANGELOG.md`.

---
[Docs index](../README.md) · [Root README](../../README.md) · [Changelog](../../CHANGELOG.md)
