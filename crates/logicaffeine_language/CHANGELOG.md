# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.9.17] - 2026-06-11

### Added
- Strict whole-input parsing: `compile()` rejects parses that strand tokens (`TrailingTokens`) instead of silently dropping meaning. New coverage: noun-noun compound heads, possessive heads over `Ambiguous` noun/verb words, trailing temporal operators in `If`-consequents (`until`/`release`/`weak-until`, `within N cycles`), postposed `when`-clauses and sentence-final temporal anchors, and quantified/cardinal objects under modals and under `never`. Lexer: letter-hyphen-letter compounds, attributive participial adjectives, `-ing` prepositions. A per-token lexical-ambiguity forest enumerates the strict-parse combinations.
- Hardware-spec coverage: bounded delay synthesis (`within N cycles` → `BoundedEventually`), signal extraction for counting quantifiers, copula temporal adverbs in consequent clauses, and `while` as a duration subordinator.

### Fixed
- 3sg stem recovery derived from the generative rule: `strip_s` accepts a stem only if it is a known base verb whose `third_person_of` regenerates the surface form exactly, so `planes` is no longer the 3sg of `plan`.
- Presupposition triggers read through `Ambiguous` tokens (Van der Sandt projection under negation); `strip_s` resolves sibilant `-es` and `-ies` stems via `is_base_verb`; a temporal-adverb reading is blocked right after a determiner; focus constructions accept copular predication (`Only dogs are red.`).

See the root CHANGELOG for the cross-crate history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- English to first-order logic transpilation pipeline
- Lexer with tokenization for natural language input
- Parser with arena-allocated AST nodes
- Quantifier scope resolution (universal, existential, negation)
- Modal operators (necessity, possibility, deontic)
- Temporal logic (tense, aspect)
- Wh-questions and relative clause handling
- Reflexives and reciprocals
- Parse forest generation for structural ambiguity
- Optional `dynamic-lexicon` feature for runtime vocabulary
