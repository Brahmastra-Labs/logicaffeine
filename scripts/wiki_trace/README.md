# wiki-trace

Internal workspace helper (`publish = false`, `0.0.0`) that runs free English text through the LOGOS English→FOL compiler and reports, per sentence, what the compiler produced and what work each failure implies. Strictly READ-ONLY: it *proposes*, it never edits source, lexicon, or tests.

## Role in the workspace

Member crate under `scripts/`. It depends on `logicaffeine-compile` (`ui_bridge::compile_for_ui` — the same path the web Studio uses: tokens, AST, three FOL renderings, readings, errors) and `logicaffeine-language` (`compile` for the typed `ParseError`, plus the `lexicon`). It drives the real compiler — no mock — to surface coverage gaps from corpus text (e.g. Wikipedia) and feed a gated improvement loop.

Both binaries read one input file (one sentence per line; blank lines skipped, trimmed, 1-indexed) and write into a per-article output dir. One shared library, two binaries.

## `wiki-trace` — verbose per-sentence dump

```text
wiki-trace <input.txt> [output_dir]
```

Default output: `wikis/traces/<input-stem>/`. Per sentence it calls `compile_for_ui` and renders tokens (offset spans + category), the Socratic error or the three FOL forms (unicode / simple / kripke), the AST tree, and ambiguity readings. Writes:

- `summary.txt` — one status line per sentence (`ok` / `PART` / `FAIL`) + FOL or error + a totals footer. PART = parsed but the AST holds `other` (unhandled `LogicExpr`) nodes.
- `traces.jsonl` — one JSON object per line: `{index, input, result}` (the full `CompileResult`).
- `sentences/NN.trace` — the verbose dump per sentence.

## `wiki-triage` — classify the work each sentence implies

```text
wiki-triage <input.txt> [output_dir]
```

Default output: `wikis/triage/<input-stem>/`. Per sentence it calls `classify`, which compiles twice (`compile_for_ui` for tokens/AST/FOL/readings, `compile` for the typed error) and sorts into six categories:

- `Clean` — parses to proper FOL.
- `ActionableLexiconGap` — an unknown lowercase content word absent from every lexicon DB, or an `UnknownQuantifier`/`UnknownModal` error; a candidate entry (lemma + suffix-guessed POS) is proposed.
- `ParserGap` — a structural failure (`ExpectedVerb`, `ExpectedContentWord`, `UnexpectedToken`, …).
- `SemanticLossy` — parses, but the AST collapsed or carries unhandled `other` nodes.
- `AmbiguityHuman` — scope / unresolved-pronoun conflicts, or ≥6 readings.
- `IsolateOutOfScope` — the failure point lies in quarantined noise.

Each record carries an `Outcome` (Ok/Partial/Fail), `Subsystem`, `Gate`, confidence, localization, and proposals. `Gate`: `Auto` = low-risk lexicon proposal a gated loop may apply; `Investigate` = a lead an agent implements; `Human` = never autonomous.

Pipeline inside `classify`:

1. **Quarantine** — flags noise spans: parentheticals `(...)`, citations `[...]`, quoted regions, abbreviations/acronyms (all-caps, interior cap like `PhD`, digit+cap like `N2pc`). A plain name (Mary) is not flagged; findings in these spans never reach the auto-fix path.
2. **Lexicon probe** — content tokens outside quarantine are tested by `is_actionable_gap` (morphology resolved); a function-word stoplist keeps `is`/`the`/`of` off the gap path. The lexer's unknown-word fallback is `Noun`, so the DB lookup (`lexical_status == Unknown`), not the POS tag, is the real discriminator.
3. **Metamorphic oracle** — for a failure, `metamorphic_variants` yields meaning-preserving paraphrases. v1 ships one high-precision rule, `pp_fronting_to_trailing` ("After the meeting, Mary left." → "Mary left after the meeting."). If the paraphrase parses, its FOL is **adopted** as the spec and a RED-test stub is proposed — the spec writes itself.
4. **Cluster** — groups non-clean records by `(category, subsystem, root-cause signature)`, sorted by size, so the loop fixes a class, not N duplicates.

Outputs:

- `verdict.json` — loop exit condition: `all_clean` flag, per-outcome and per-category counts, cluster count.
- `triage.jsonl` — one `TriageRecord` per line; `clusters.json` — the clusters.
- `worklist.md` — actionable items only (gate `auto`/`investigate`): clusters first, then Auto-eligible and Investigate items.
- `needs_human.md` — design decisions, ambiguity, semantic conflicts, isolated noise.
- `sentences/NN.trace` — classification header + the verbose trace.

## Key modules

- `src/lib.rs` — triage model (`TriageRecord`, `Category`, `Outcome`, `Gate`, `Subsystem`) and engine: `quarantine`, `classify`, `cluster`, lexicon probe, metamorphic oracle, `classify_error`, proposal builders.
- The `render` module (`src/render.rs`) — `render_trace` + `count_nodes`; the verbose dump shared by both binaries.
- `src/bin/trace.rs`, `src/bin/triage.rs` — the two binaries (triage also writes `worklist.md` / `needs_human.md`).

## Build / test

```bash
cargo test -p wiki-trace
cargo run -p wiki-trace --bin wiki-trace  -- article.txt
cargo run -p wiki-trace --bin wiki-triage -- article.txt
```

