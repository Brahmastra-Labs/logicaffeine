# Studio & Learn

The [`logicaffeine_web`](../apps/logicaffeine_web/README.md) app is a Dioxus + WebAssembly single-page
site that runs the **entire LOGOS engine in the browser** — parser, interpreter, VM, proof engine,
and code generation all execute client-side. No server round-trip.

Source of truth: [`apps/logicaffeine_web/src/`](../apps/logicaffeine_web/src/) — the router
([`ui/router.rs`](../apps/logicaffeine_web/src/ui/router.rs)), the pages
([`ui/pages/`](../apps/logicaffeine_web/src/ui/pages/)), and the learner model (the modules listed
below).

## Studio — the playground

[`/studio`](https://logicaffeine.com/studio) is a multi-mode editor
([`ui/pages/studio.rs`](../apps/logicaffeine_web/src/ui/pages/studio.rs)):

- **Logic mode** — parse English to First-Order Logic with an AST view. The output toggles between
  the FOL interpretation, generated Rust, and synthesized SVA (`LogicView`).
- **Code mode** — write imperative LOGOS with a streaming REPL; the output toggles between
  interpreter output and the generated Rust (`CodeOutputMode`). Execution uses the engine's
  [`interpret_streaming`](../crates/logicaffeine_compile/src/ui_bridge.rs) API, which emits output
  lines incrementally and can route file I/O through an in-browser virtual file system.
- **Math mode** — define theorems and types and prove them interactively, with tactic hints from the
  [proof engine](proof-and-verification.md).
- **Hardware mode** — write an English hardware spec; the engine synthesizes SystemVerilog
  Assertions (SVA), PSL, and a Rust runtime monitor
  ([`codegen_sva/`](../crates/logicaffeine_compile/src/codegen_sva/)), then proves the properties
  in-browser and renders a **counterexample waveform** when one fails. Logic mode's "Compile to SVA"
  reuses the same synthesis. See [Proof & verification](proof-and-verification.md).

## Learn Logic — the curriculum

[`/learn`](https://logicaffeine.com/learn) is a gamified course. The content is embedded at compile
time from [`apps/logicaffeine_web/assets/`](../apps/logicaffeine_web/assets/) (via `include_dir`) and
organized into four **eras**:

```
01_first-steps  →  02_building-blocks  →  03_expanding-horizons  →  04_mastery
```

Each era holds modules; each module holds sections (prose, definitions, examples, symbol glossaries)
and exercises. Exercise types (`ExerciseType` in
[`content.rs`](../apps/logicaffeine_web/src/content.rs)):

- **Translation** — translate English to FOL (graded structurally).
- **MultipleChoice** — pick the correct reading.
- **Ambiguity** — identify the distinct readings of an ambiguous sentence.

The learner model is built from focused modules under
[`apps/logicaffeine_web/src/`](../apps/logicaffeine_web/src/):

| Module | Role |
|--------|------|
| [`srs.rs`](../apps/logicaffeine_web/src/srs.rs) | SM-2 spaced repetition (`ResponseQuality`, ease factor, intervals) |
| [`grader.rs`](../apps/logicaffeine_web/src/grader.rs) | Structural AST-similarity grading with partial credit |
| [`progress.rs`](../apps/logicaffeine_web/src/progress.rs) | Per-learner progress + review scheduling |
| [`struggle.rs`](../apps/logicaffeine_web/src/struggle.rs) | Struggle detection → adaptive hints |
| [`achievements.rs`](../apps/logicaffeine_web/src/achievements.rs) | XP, streaks, rewards |
| [`unlock.rs`](../apps/logicaffeine_web/src/unlock.rs) | Prerequisite-gated unlocks |
| [`game.rs`](../apps/logicaffeine_web/src/game.rs) | Session/game state |

Grading normalizes whitespace and Unicode and compares FOL *structurally*, so an answer that is
formally equivalent is accepted regardless of cosmetic differences.

## The rest of the site

The router defines these routes (from `ui/router.rs`):

| Route | Page |
|-------|------|
| `/` | Landing |
| `/studio` | Studio playground |
| `/learn` | Learn Logic curriculum |
| `/benchmarks` | Live performance benchmarks ([Execution & performance](execution-and-performance.md)) |
| `/registry`, `/registry/package/:name` | Package registry browser |
| `/guide` | Documentation & tutorials |
| `/crates` | Crate documentation browser |
| `/news`, `/news/:slug` | Release announcements |
| `/roadmap` | Roadmap |
| `/pricing` | Plans & licensing |
| `/profile` | User settings & progress |
| `/workspace/:subject` | Subject workspace |

Persistence (progress, license, theme) uses browser storage; file I/O in Studio uses an OPFS /
IndexedDB-backed virtual file system.

## Running it locally

```bash
dx serve -p logicaffeine-web    # from the repo root
```

## See also

- The app's own developer README → [`apps/logicaffeine_web/README.md`](../apps/logicaffeine_web/README.md)
- What the engine does → [Imperative mode](imperative-mode.md), [Logic mode](logic-mode.md)

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
