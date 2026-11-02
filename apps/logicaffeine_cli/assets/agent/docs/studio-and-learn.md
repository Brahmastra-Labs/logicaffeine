# Studio & Learn

The quickest way to find out whether English can be a programming language is to try it, and that
should not start with installing a toolchain. The
[`logicaffeine_web`](../apps/logicaffeine_web/README.md) app is a Dioxus + WebAssembly single-page
site that runs the **entire LOGOS engine in the browser**: parser, interpreter, VM, proof engine,
and code generation all execute client-side, with no server round-trip.

After this guide you will know what each Studio mode does, how the Learn Logic course grades and
schedules its exercises, and what the Syntax Guide teaches.

## Studio — the playground

[`/studio`](https://logicaffeine.com/studio) is a multi-mode editor
([`ui/pages/studio.rs`](../apps/logicaffeine_web/src/ui/pages/studio.rs)):

- **Logic mode** — parse English to First-Order Logic with an AST view. The output toggles between
  the FOL interpretation and generated Rust (`LogicView`). A proof panel under the input runs the
  [proof engine](proof-and-verification.md) on the sentence — `Auto` backward chaining or a named
  tactic — and shows the derivation tree, or a hint when the proof fails.
- **Code mode** — write imperative LOGOS with a streaming REPL; the output toggles between
  interpreter output and the generated Rust (`CodeOutputMode`). Execution uses the engine's
  [`interpret_streaming_with_vfs`](../crates/logicaffeine_compile/src/ui_bridge.rs) API, which emits
  output lines incrementally and routes file I/O through an in-browser virtual file system.
- **Math mode** — kernel vernacular: definitions, inductive types and theorems, each statement run
  through the kernel's REPL. A `## Theory` development goes to a multi-theorem driver that certifies
  every theorem on its own and reports each. The output toggles between the REPL and the definitions
  extracted to Rust.
- **Hardware mode** — write an English hardware spec; the engine synthesizes SystemVerilog
  Assertions (SVA), PSL, and a Rust runtime monitor
  ([`codegen_sva/`](../crates/logicaffeine_compile/src/codegen_sva/)), then proves the properties
  in-browser, kernel-certified with no Z3 in the bundle, and renders a **counterexample
  waveform** when one fails. See [Proof & verification](proof-and-verification.md).
- **Web mode** — write a page (`## Page /`) and see it run: the source compiles to WebAssembly in
  the browser ([`web_host::assemble_preview`](../crates/logicaffeine_compile/src/web_host.rs)) and
  loads into a sandboxed frame beside the editor, rebuilt as you type; a build error shows the
  offending line in the frame. A `.logos` file that declares a page opens here. The guide's web
  examples use the same preview. See [Frontend](frontend.md).

The file browser holds the shipped examples
([`ui/examples.rs`](../apps/logicaffeine_web/src/ui/examples.rs)), the Code examples in topic folders
sorted in learning order, `basics/` first. **New file** creates `untitled.<ext>` (then
`untitled-1.<ext>`, …) with a starter for the current mode and opens it. A deep link
`/studio?file=<path>` opens one file.

## Learn Logic — the roadmap

[`/learn`](https://logicaffeine.com/learn) walks the curriculum one lesson at a time. The content is
embedded at compile time from [`apps/logicaffeine_web/assets/curriculum/`](../apps/logicaffeine_web/assets/curriculum/)
(via `include_dir`) and organized into four **eras**:

```
01_first-steps  →  02_building-blocks  →  03_expanding-horizons  →  04_mastery
```

Each era holds modules, and each module holds lessons (`sec_*.json`) and practice exercises
(`ex_*.json`). The page draws the eras, modules and lessons as a roadmap and opens one lesson at a
time: a short teaching, two examples, and one question. Where the lesson teaches writing formulas the
question is typed, with a symbol button for every symbol the answer needs beside the keyboard;
otherwise it is one multiple-choice question. Answering it, or asking to see the answer after a try,
moves the learner on. Any lesson can be opened from the roadmap, and "More examples" and "More
practice" stay there for anyone who wants them. Progress is kept in the browser.

Every formula a lesson shows, and every typed answer key, is exactly what LOGOS prints for its
sentence: `crates/logicaffeine_tests/tests/learn_lessons.rs` re-derives each one with the compiler.
The symbol pad holds every symbol a typed key needs and nothing the grader cannot read
(`learn_symbol_pad.rs`), and `scripts/web/test-learn-e2e.sh` drives lessons end to end in Chrome and
Firefox.

Practice exercise types (`ExerciseType` in
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
| [`progress.rs`](../apps/logicaffeine_web/src/progress.rs) | The lessons finished, the lesson left open, and every exercise's attempts and review schedule |
| [`struggle.rs`](../apps/logicaffeine_web/src/struggle.rs) | Struggle detection → adaptive hints |
| [`learn_state.rs`](../apps/logicaffeine_web/src/learn_state.rs) | The roadmap journey, a lesson's question, and practice with its retries |
| [`composer.rs`](../apps/logicaffeine_web/src/composer.rs) | The symbol buttons and where a click types its symbol |
| [`guide_review.rs`](../apps/logicaffeine_web/src/guide_review.rs) | The Syntax Guide's recall cards and exercise results on the same SM-2 schedule, under a `guide/` key namespace |

Grading normalizes whitespace and Unicode and compares FOL *structurally*, so an answer that is
formally equivalent is accepted regardless of cosmetic differences.

## The Syntax Guide

[`/guide`](https://logicaffeine.com/guide) teaches the language in sections grouped into parts
([`ui/pages/guide/content/`](../apps/logicaffeine_web/src/ui/pages/guide/content/)). A section opens
on a problem the reader cannot yet solve and the objectives it covers, confronts common
misconceptions each with a runnable counter-example, and closes on recall questions. Its examples
run in place in one of four modes (`ExampleMode`): Logic compiles English to FOL, Imperative runs a
program, Theorem shows the kernel's verdict, and Web shows a page live in the same sandboxed preview
as the Studio's Web mode. An example whose runtime exists only in a compiled binary shows a
"Compiled — run with largo" badge instead of a Run button, and a predict-first example hides its
output until the reader commits to an answer.

"Now you try" exercises are graded in place — a program by its exact output, a page by its own
`## Test` blocks, FOL structurally by the Learn grader — with hints disclosed one at a time. Exercise
results and recall self-marks feed the SM-2 schedule (`guide_review.rs`). The search box at the top
covers every section's prose, objectives, misconceptions, recall questions and example code, and a
read mark per section is kept in the browser's local storage.

## The rest of the site

The router defines these routes (from `ui/router.rs`):

| Route | Page |
|-------|------|
| `/` | Landing |
| `/studio` | Studio playground |
| `/learn` | Learn Logic roadmap (`/profile` redirects here) |
| `/benchmarks` | Benchmark results, read from the checked-in `benchmarks/results/` ([Execution & performance](execution-and-performance.md)) |
| `/registry`, `/registry/package/:name` | Package registry browser |
| `/guide` | The Syntax Guide |
| `/crates` | Crate documentation browser |
| `/news`, `/news/:slug` | Release announcements |
| `/roadmap` | Roadmap |
| `/contact` | Contact the team (`/pricing` redirects here) |
| `/privacy`, `/terms` | Privacy policy, terms of service |
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

## Where this is tested

The app is in [`apps/logicaffeine_web/src/`](../apps/logicaffeine_web/src/): the router
([`ui/router.rs`](../apps/logicaffeine_web/src/ui/router.rs)), the pages
([`ui/pages/`](../apps/logicaffeine_web/src/ui/pages/)), and the learner-model modules listed
above.

- `web_canonical_teaching_lock` mounts every page among the Syntax Guide's and the Studio's
  examples, renders it on the tree-walker and the VM, compiles it for the browser and runs its tests.
- `guide_examples` runs every Syntax Guide example on the entry point its mode uses.
- `studio_math_document_door` reads every Studio Math example through the kernel's document reader.
- `guide_coverage` and `guide_pedagogy` hold the Syntax Guide's structure and teaching shape.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
