# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Breaking changes

- Verification takes no key. The `license` module (`LicensePlan`, `LicenseValidator`) is gone, with
  `VerificationErrorKind::LicenseRequired`, `LicenseInvalid` and `LicenseInsufficientPlan`, their constructors
  (`VerificationError::license_required`, `license_invalid`, `insufficient_plan`) and the `ureq` and `dirs`
  dependencies that served them.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.

- **The published crate carries only its source.** An anchored `include` allowlist ships `src/`, the README and the CHANGELOG. The `#[cfg(test)]` bodies of `ir`, `license` and `solver` moved to `crates/logicaffeine_tests/unit/logicaffeine_verify/` behind one-line `#[path]` stubs, so the tarball carries no test code. The declared `rust-version` is 1.88 (0.10.1 declared 1.75). The API is unchanged.

## [0.10.0] - 2026-07-08

### Added
- `static-link-z3` feature: builds Z3 from the source vendored in z3-sys and links it statically — the substrate of the release `largo-full` binaries. Default stays dynamic against the system libz3.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial release (excluded from workspace, requires Z3 installation).

### Added

- Z3 SMT solver integration for static verification
- License gating via Stripe subscription validation
- Socratic error messages for proof failures
- Support for Pro, Premium, Lifetime, and Enterprise plans
- Integration with `logicaffeine-proof` as oracle fallback
