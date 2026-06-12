# wikis/ — English→FOL improvement loop

This directory is the workspace for an **autonomous improvement loop** over the LOGOS
English→First-Order-Logic compiler. The goal: take real Wikipedia prose, find what the
compiler can't yet handle, classify the work, fix the safest items, and repeat until a
whole article compiles to proper FOL — then move to the next article.

This README is written **for the agent/loop**. Read it before operating.

## TL;DR commands

```bash
# Fetch a fresh random article AND triage it (the loop's input step):
./scripts/next-wiki.sh                       # → wikis/<slug>.txt + wikis/triage/<slug>/

# Triage an article you already have:
./scripts/triage-wiki.sh wikis/<slug>.txt    # → wikis/triage/<slug>/

# Verbose per-sentence traces (tokens/AST/FOL/readings), no classification:
./scripts/trace-wiki.sh   wikis/<slug>.txt   # → wikis/traces/<slug>/
```

Articles are one **sentence per line**. `wikis/<slug>.txt` are inputs; `wikis/traces/` and
`wikis/triage/` are generated outputs (safe to delete and regenerate).

## What the triage harness produces — `wikis/triage/<slug>/`

| File | For | Contents |
|------|-----|----------|
| `verdict.json` | the loop | **Exit condition.** `all_clean: true` ⇒ the article fully compiles. Plus per-outcome and per-category counts. |
| `worklist.md` | you | Ranked, clustered, actionable items (gate `auto`/`investigate`) with localization, oracles, and proposals. |
| `needs_human.md` | a human | Design decisions, genuine ambiguity, semantic conflicts, and isolated noise. **Never auto-act on these.** |
| `triage.jsonl` | tooling | One structured record per sentence (schema below). |
| `clusters.json` | the loop | Root-cause clusters — fix the **class**, not N duplicates. |
| `sentences/NN.trace` | debugging | Classification header + full token/AST/FOL/readings dump per sentence. |

### Record schema (`triage.jsonl`)

```
{ index, input, outcome: ok|partial|fail,
  category:  clean | actionable_lexicon_gap | parser_gap | semantic_lossy | ambiguity_human | isolate_out_of_scope,
  subsystem: lexicon | parser | semantics | none,
  gate:      auto | investigate | human,
  confidence: 0.0–1.0,
  fol, localization: { error_kind, error_span, offending_text, suspect_words[], isolated_spans[] },
  oracle?:   { transform, variant_sentence, expected_fol, equivalence },
  evidence:  { reading_count, ast_nodes, unhandled_ast_nodes, lexicon_misses[], dropped_content_words[] },
  proposal?: { red_test?, lexicon_entry? } }
```

## Categories → what work it implies

- **`actionable_lexicon_gap`** (subsystem `lexicon`) — a content word the lexicon lacks (the
  lexer fell back to `Adjective`/`Other` and the word is in no lexicon DB). Fix = add a lexicon
  entry. A `proposal.lexicon_entry` is included (POS is a suffix-based **guess** — verify it).
- **`parser_gap`** (subsystem `parser`) — a grammatical construction the parser doesn't handle
  (e.g. fronted prepositional phrases). If an equivalent paraphrase parses, `oracle.expected_fol`
  is the spec ("the spec writes itself") and a `proposal.red_test` is included.
- **`semantic_lossy`** (subsystem `semantics`) — parsed, but the FOL is collapsed/incomplete.
- **`ambiguity_human`** — genuine ambiguity or a semantic conflict. **Human decides.**
- **`isolate_out_of_scope`** — abbreviations/acronyms, parentheticals, quoted strings, citations.
  Quarantined on purpose; do **not** try to resolve/expand them.
- **`clean`** — compiles to proper FOL. Nothing to do.

## Gates — what the loop may do (HEAVY GUARDRAILS)

- **`auto`** — low blast radius with a concrete, verifiable proposal (a lexicon entry). A gated
  loop *may* apply it — but only behind a **full-green test suite** check, and the POS guess must
  be confirmed. Currently only `actionable_lexicon_gap`.
- **`investigate`** — there is a lead (an oracle paraphrase, a known parser construction) but an
  agent must implement it with judgment. Parser/semantic work lives here. **Never auto-applied.**
- **`human`** — design decisions and isolated noise. The loop must not act.

### Non-negotiable rules for the loop

1. **The harness is read-only.** It proposes; it never edits source, lexicon, or tests. You apply
   changes, and only the gated ones.
2. **Tests are added, never edited.** Use `proposal.red_test` to seed a NEW failing test. Do not
   modify an existing test to make it pass (see repo `CLAUDE.md` rule #4).
3. **Green before, green after.** Start from a fully green suite and return to one. If a change
   doesn't keep the suite green, revert it.
4. **Fix clusters, not lines.** Use `clusters.json`; one fix should clear a whole class.
5. **Don't touch `isolate_out_of_scope`.** Abbreviations/quotes/parens are deferred by design.
6. **Parser changes stay human-supervised in v1.** Higher blast radius than lexicon data.

## The loop

```
1. ./scripts/next-wiki.sh                      # fetch a fresh page + triage it
2. read wikis/triage/<slug>/verdict.json
3. if all_clean: record the win; go to 1 (next page)
4. else:
     pick the top safe item from worklist.md (prefer auto/lexicon, then investigate)
     implement it via TDD (RED test from proposal/oracle → GREEN), keep the full suite green
     ./scripts/triage-wiki.sh wikis/<slug>.txt  # re-triage the SAME page
     go to 3
```

Each random page is an unseen stress test; over many pages the compiler converges on real
English. Expect dense academic prose to be lexicon-dominated first; once vocabulary is filled,
parser/semantic issues re-surface on re-triage.
