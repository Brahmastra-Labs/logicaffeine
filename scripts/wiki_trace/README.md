# wiki-trace

Point the LOGOS English→FOL compiler at a real article and some sentences will fail; the useful question is *why* each one failed and what fixing it would take. `wiki-trace` runs free English text through the compiler and reports, sentence by sentence, what it produced and the work each failure implies: a lexicon gap, a parser gap, a lossy parse, or an ambiguity a human has to settle. It is read-only by design: it proposes, and never edits source, lexicon, or tests.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 5. It is an internal tool under `scripts/` (`publish = false`, `0.0.0`): one library and two binaries.

## Role in the workspace

It depends on `logicaffeine-compile` (`ui_bridge::compile_for_ui`, the same path the web Studio uses: tokens, AST, three FOL renderings, readings, errors) and `logicaffeine-language` (`compile` for the typed `ParseError`, plus the `lexicon`). It drives the real compiler, with no mock, to surface coverage gaps from corpus text such as Wikipedia and feed a gated improvement loop.

Both binaries read one input file (one sentence per line; lines trimmed, blank lines skipped, numbered from 1) and write into a per-article output directory.

## Public API

- `src/lib.rs` — the triage model (`TriageRecord`, `Category`, `Outcome`, `Gate`, `Subsystem`, `Cluster`) and the engine's entry points: `quarantine`, `classify`, `cluster`, and `count_nodes`, which counts an AST's nodes and its unhandled `other` nodes. The lexicon probe, the metamorphic oracle, `classify_error` and the proposal builders are private to `classify`.
- The `render` module (`src/render.rs`) — `render_trace`, the verbose per-sentence dump both binaries share.
- `src/bin/trace.rs`, `src/bin/triage.rs` — the two binaries (triage also writes `worklist.md` / `needs_human.md`).

## `wiki-trace` — verbose per-sentence dump

```text
wiki-trace <input.txt> [output_dir]
```

Default output: `wikis/traces/<input-stem>/`. Per sentence it calls `compile_for_ui` and renders tokens (offset spans and category), the Socratic error or the three FOL forms (unicode / simple / kripke), the AST tree, and ambiguity readings. It writes:

- `summary.txt` — one status line per sentence (`ok` / `PART` / `FAIL`) with the FOL or the error, and a totals footer. `PART` means the sentence parsed but the AST holds `other` (unhandled `LogicExpr`) nodes.
- `traces.jsonl` — one JSON object per line: `{index, input, result}` (the full `CompileResult`).
- `sentences/NN.trace` — the verbose dump per sentence.

## `wiki-triage` — classify the work each sentence implies

```text
wiki-triage <input.txt> [output_dir]
```

Default output: `wikis/triage/<input-stem>/`. Per sentence it calls `classify`, which compiles twice (`compile_for_ui` for tokens, AST, FOL and readings; `compile` for the typed error) and sorts the sentence into a category:

- `Clean` — parses to proper FOL.
- `ActionableLexiconGap` — an unknown lowercase content word absent from every lexicon DB, or an `UnknownQuantifier`/`UnknownModal` error; a candidate entry (lemma plus a suffix-guessed POS) is proposed.
- `ParserGap` — a structural failure (`ExpectedVerb`, `ExpectedContentWord`, `UnexpectedToken`, …).
- `SemanticLossy` — parses, but the AST collapsed or carries unhandled `other` nodes.
- `AmbiguityHuman` — scope or unresolved-pronoun conflicts, or ≥6 readings.
- `IsolateOutOfScope` — the failure point lies in quarantined noise.

Each record carries an `Outcome` (Ok/Partial/Fail), `Subsystem`, `Gate`, confidence, localization, and proposals. `Gate` says who may act: `Auto` is a low-risk lexicon proposal a gated loop may apply, `Investigate` is a lead an agent implements, and `Human` is never acted on autonomously.

The pipeline inside `classify`:

1. **Quarantine** — flags noise spans: parentheticals `(...)`, citations `[...]`, quoted regions, abbreviations and acronyms (all-caps, an interior capital like `PhD`, digit+capital like `N2pc`). A plain name (Mary) is not flagged, and findings in these spans never reach the auto-fix path.
2. **Lexicon probe** — content tokens outside quarantine are tested by `is_actionable_gap` (morphology resolved); a function-word stoplist keeps `is`/`the`/`of` off the gap path. The lexer's unknown-word fallback is `Noun`, so the DB lookup (`lexical_status == Unknown`), not the POS tag, is the real discriminator.
3. **Metamorphic oracle** — for a failure, `metamorphic_variants` yields meaning-preserving paraphrases. One high-precision rule ships, `pp_fronting_to_trailing` ("After the meeting, Mary left." → "Mary left after the meeting."). If the paraphrase parses, its FOL is **adopted** as the spec and a RED-test stub is proposed, so the spec writes itself.
4. **Cluster** — groups non-clean records by `(category, subsystem, root-cause signature)`, sorted by size, so the loop fixes a class rather than N duplicates.

Outputs:

- `verdict.json` — the loop's exit condition: the `all_clean` flag, per-outcome and per-category counts, and the cluster count.
- `triage.jsonl` — one `TriageRecord` per line; `clusters.json` — the clusters.
- `worklist.md` — actionable items only (gate `auto`/`investigate`): clusters first, then Auto-eligible and Investigate items.
- `needs_human.md` — design decisions, ambiguity, semantic conflicts, isolated noise.
- `sentences/NN.trace` — a classification header plus the verbose trace.

## Build and test

```bash
cargo nextest run -p wiki-trace                               # unit tests
cargo nextest run -p logicaffeine-tests --test classify       # the classifier suite
cargo run -p wiki-trace --bin wiki-trace  -- article.txt
cargo run -p wiki-trace --bin wiki-triage -- article.txt
```

The unit tests' body lives in `crates/logicaffeine_tests/unit/wiki_trace/tests.rs`, reached through the `#[cfg(test)]` stub in `src/lib.rs`.

## Dependencies

- **Internal:** `logicaffeine-compile` (`ui_bridge`), `logicaffeine-language` (`compile`, the lexicon, the typed errors).
- **External:** `serde`, `serde_json` (the JSON and JSONL outputs).

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/scripts/wiki_trace/CHANGELOG.md)
