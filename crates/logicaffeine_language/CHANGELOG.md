# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.10.0] - 2026-07-08


### Added
- `source_format` module: the canonical LOGOS source formatter (`format_source`/`format_line` — leading tabs → 4 spaces, trailing whitespace stripped, CRLF→LF, final-newline preserved, idempotent). One rule set shared by the LSP's formatting provider and `largo fmt`.
- `Session::set_format` — mid-session output-format switching for the logic REPL; affects rendering only, discourse state carries across (a switched session renders exactly as one created with that format).
- Strict whole-input parsing: `compile()` rejects parses that strand tokens (`TrailingTokens`) instead of silently dropping meaning. New coverage: noun-noun compound heads, possessive heads over `Ambiguous` noun/verb words, trailing temporal operators in `If`-consequents (`until`/`release`/`weak-until`, `within N cycles`), postposed `when`-clauses and sentence-final temporal anchors, and quantified/cardinal objects under modals and under `never`. Lexer: letter-hyphen-letter compounds, attributive participial adjectives, `-ing` prepositions. A per-token lexical-ambiguity forest enumerates the strict-parse combinations.
- Hardware-spec coverage: bounded delay synthesis (`within N cycles` → `BoundedEventually`), signal extraction for counting quantifiers, copula temporal adverbs in consequent clauses, and `while` as a duration subordinator.
- `Parser::stmt_spans()` — a side-table with one source span per top-level statement `parse_program` returned (period/dedent inclusive, 1:1 with the statement list). The seam that lets downstream consumers (typechecker diagnostics, ownership cause-links, the rustc sourcemap) anchor findings without a span field on every `Stmt` variant.
- **The AST depth gate ("parsed ⇒ bounded")** — `ast_depth` module: any program `parse_program` returns has nesting depth within the effective limit, so every downstream recursive walker (optimizer, codegen, tree-walker, VM compiler) is bounded by construction — a 10,000-term operator chain, parenthesis tower, or block pyramid now gets a graceful `AstTooDeep` diagnostic (with the split-into-`Let`s fix and the env override spelled out) instead of stack-overflowing every surface at once. Iterative wildcard-free walker (a new AST variant with children fails compilation here until classified) + parse-time recursion guards on `parse_statement`/`parse_primary_expr`/`parse_indented_block`. Default 128 levels, sized for the tightest standard stacks and measured empirically in 2 MiB-thread tests; `LOGOS_MAX_AST_DEPTH` tunes it per environment — potato or supercomputer, no rebuild.
- `token_class` — the single token-classification truth (verbs=function, nouns=type, adjectives=modifier, …) as a plain enum + wildcard-free `classify`. Every highlighting surface derives from it (the LSP maps to its legend, the REPL to ANSI), so they can never disagree — and a new `TokenType` variant now fails to compile HERE, next to the enum, until its class is decided.
- `teach` — the single teaching truth: a `ConstructDoc` lesson (one plain sentence + runnable example + socratic question/tip, all REQUIRED BY THE TYPE) for 24 statement keywords, all 19 `##` block types, and the 14 built-in type names; `doc_for` is wildcard-free (a new `TokenType` doesn't compile until someone decides its lesson), `doc_for_block` is total. LSP hover, completion documentation, and the CLI REPL's new `:explain` all derive from this table — plus the literate-doc extractors (`module_doc`, `doc_for_header_at`, `extract_literate_docs`) that turn a `## Note` above a definition into its documentation, `## Definition`-body types included. Ratcheted by `tests/teach_lock.rs` (completeness incl. every example must lex, lookup↔table parity both directions, guide anchors resolve against LOGOS_QUICKGUIDE.md's real headings).

### Changed
- **Every socratic explanation rewritten to actually teach**: *what happened → why → a guiding question → the concrete next step*, in plain words. Token names render through the new `describe_token` ("a comma (',')", never the `Comma` debug name), pronouns humanize ("'she'", not `Female Singular`), and byte positions are gone from the prose — the span carries the location; the caret excerpt and editor ranges show it. The full set is committed as a golden in the LSP crate (`socratic_explanations.txt`) so every future wording change is a reviewable diff.
- The imperative statement dispatch diagnoses `x is 5.` as `IsValueEquality` (say `equals`) instead of a generic `ExpectedStatement` — the specific lesson was previously only reachable on the logic-mode path.
- **The formatter is now structure-aware and provably safe.** `format_source` reindents code to 4 spaces per LEXED nesting level (depth from the lexer's own Indent/Dedent reading — a double-tab first indent canonicalizes to 4 spaces, not 8), leaves `## Note`/`## Example` prose entirely to the author (markdown hard-breaks and nested-list indents survive), and — the bug fix — never touches a byte inside a multiline string (`format_line` was stripping trailing spaces, converting tabs, and blanking whitespace-only lines INSIDE `"""` literals: silent string corruption in both `largo fmt` and the LSP). Locked by `formatter_locks.rs`: token-stream equivalence (`tokens(format(x)) == tokens(x)`, string bytes included) + idempotence over the corpus.
- **The parser is total.** `Parser::new` EOF-terminates any fragment or block slice that arrives without one, and `peek()` clamps to the terminator instead of indexing out of bounds — a bare `## Theorem: Socrates` (header, no body) used to PANIC analysis, found by the new quickguide highlighting ratchet.
- The `UseAfterMove` socratic explanation now speaks the ownership house voice — "Cannot use 'x' after giving it away", with the transfer explained and both remedies named (`Show` to lend, `a copy of` to keep) — matching the ownership checker and the rustc diagnostic bridge instead of the mechanical "this value has been moved".

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
