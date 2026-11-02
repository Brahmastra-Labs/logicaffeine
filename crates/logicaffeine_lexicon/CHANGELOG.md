# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Added
- **`LEXICON_JSON`: the vocabulary database belongs to this crate.** `crates/logicaffeine_lexicon/assets/lexicon.json` is exported as a `&'static str`; `logicaffeine-language` compiles its word tables from it through a build-dependency, and the `dynamic-lexicon` `runtime` module parses it. The vocabulary is unchanged from 0.10.1: the file is byte-identical to the one the language crate carried, and no verb, noun or adjective was added or removed. `packaging_lock`.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.

### Fixed
- **The `dynamic-lexicon` feature builds from the published crate.** `runtime` embedded the lexicon from the language crate's directory, a path outside this package, so the feature, which docs.rs builds, could not compile from crates.io. The package now carries the file itself, and its manifest's `include` allowlist ships only the sources, the lexicon, the README and the CHANGELOG.

## [0.10.0] - 2026-07-08

Synced to workspace version 0.10.0. See root CHANGELOG for full history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Compile-time lexicon lookup for English vocabulary
- Vendler verb classification (activity, accomplishment, achievement, state)
- Noun animacy and count/mass distinctions
- Adjective types (intersective, subsective, non-subsective, privative)
- Thematic role definitions (agent, patient, theme, etc.)
- Optional `dynamic-lexicon` feature for runtime vocabulary extension
