# Versioning

This document describes the versioning strategy and release process for logicaffeine.

## Philosophy

Logicaffeine uses **lockstep versioning**: all crates in the workspace share the same version number. This simplifies dependency management and gives users a single version to track.

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

Crates must be published in dependency order (leaves first):

```
Tier 0 (no internal deps):
  logicaffeine-base
  logicaffeine-data

Tier 1 (depends on Tier 0):
  logicaffeine-kernel
  logicaffeine-lexicon

Tier 2 (depends on Tier 0-1):
  logicaffeine-system

Tier 3 (depends on Tier 0-2):
  logicaffeine-language
  logicaffeine-proof

Tier 4 (depends on Tier 0-3):
  logicaffeine-compile

Tier 5 (applications):
  logicaffeine-cli
  logicaffeine-web

Optional (excluded from workspace):
  logicaffeine-verify
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
# Ensure all tests pass
cargo test -- --skip e2e

# Ensure no uncommitted changes
git status

# Update version in all Cargo.toml files
# (manual or script - see below)
```

### 2. Update Changelogs

For each crate with changes:

1. Move items from `[Unreleased]` to new version section
2. Add release date: `## [0.7.0] - 2026-01-17`
3. Create new empty `[Unreleased]` section

### 3. Update Root CHANGELOG

The root `CHANGELOG.md` provides a high-level summary across all crates. Update it with the most significant changes.

### 4. Commit and Tag

```bash
git add -A
git commit -m "Release v0.7.0"
git tag v0.7.0
```

### 5. Publish to crates.io

Publish in dependency order (Tier 0 first, then Tier 1, etc.):

```bash
# Tier 0
cargo publish -p logicaffeine-base
cargo publish -p logicaffeine-data

# Tier 1
cargo publish -p logicaffeine-kernel
cargo publish -p logicaffeine-lexicon

# Tier 2
cargo publish -p logicaffeine-system

# Tier 3
cargo publish -p logicaffeine-language
cargo publish -p logicaffeine-proof

# Tier 4
cargo publish -p logicaffeine-compile

# Tier 5 (apps)
cargo publish -p logicaffeine-cli
# logicaffeine-web is typically not published (web app)
```

Note: Wait for each crate to appear on crates.io before publishing dependents.

### 6. Push

```bash
git push origin main --tags
```

## Version Bump Script

To bump all crates simultaneously, use:

```bash
# Example: bump to 0.7.0
./scripts/bump-version.sh 0.7.0
```

This script updates all `Cargo.toml` files and inter-crate dependencies.

## Unpublished Crates

The following crates are not published to crates.io:

| Crate | Reason |
|-------|--------|
| `logicaffeine-tests` | Internal test suite only |
| `logicaffeine-web` | Web application, not a library |
| `logicaffeine-verify` | Requires Z3, optional feature |

These crates still follow lockstep versioning for consistency.

## Version History

| Version | Date | Notes |
|---------|------|-------|
| 0.8.12 | 2026-02-14 | Optimizer updates |
| 0.8.11 | 2026-02-14 | Peephole optimizer: vec fill, swap, copy elision, release profile |
| 0.8.10 | 2026-02-14 | Codegen optimizations: direct indexing, inlining, LTO |
| 0.8.9 | 2026-02-14 | Auto-deploy frontend after benchmarks, CI fixes |
| 0.8.8 | 2026-02-14 | (skipped — missing actions:write permission) |
| 0.8.7 | 2026-02-14 | (skipped — CI fix landed after tag) |
| 0.8.6 | 2026-02-13 | Benchmarks, interpreter improvements, CI fixes |
| 0.8.3 | 2026-02-12 | FFI test CI compatibility, platform-aware C linkage |
| 0.8.2 | 2026-02-12 | Optimizer, interpreter mode, FFI safety |
| 0.8.1 | 2026-02-12 | VSCode extension fix |
| 0.8.0 | 2026-02-10 | LSP, VSCode extension, FFI/C exports, CI/CD |
| 0.7.0 | 2026-02-01 | Escape analysis, concurrency, web platform |
| 0.6.0 | 2026-01-17 | Initial crates.io release |
| 0.5.5 | 2026-01-01 | First public release (pre-crates.io) |
