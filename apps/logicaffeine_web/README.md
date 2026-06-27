# logicaffeine-web

The browser IDE (**Studio**) and gamified curriculum (**Learn**) for LOGOS. A
single-page Dioxus/WASM app that runs the full LOGOS engine client-side — parsing,
streaming interpretation, proof checking, Rust code generation, and SystemVerilog
assertion synthesis all happen in-wasm with no server round-trip.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 4, WASM (Dioxus).
Embeds the engine crates and runs them client-side.

## Role in the workspace

This is the hosted web front-end: it consumes the Tier 0–3 engine crates as path
dependencies and compiles them to `wasm32` so the entire transpiler executes in the
browser. The crate is `publish = false` — it ships as the deployed site, not a library.

For the user-facing feature tour (the Studio modes, the Learn curriculum, XP/streaks,
spaced repetition), see **[../../new_docs/studio-and-learn.md](../../new_docs/studio-and-learn.md)**.
This README is the developer-facing view: tech stack, module layout, embedded crates,
and how to build and serve.

## Tech stack

- **Dioxus 0.7** (`web` + `router`), compiled to `wasm32`.
- `wasm-bindgen`, `web-sys`, `js-sys`, `wasm-bindgen-futures`, plus `gloo`
  (`gloo-timers`, `gloo-net`, `gloo-storage`).
- **CodeMirror 6** (editor) and **KaTeX 0.16** (LaTeX rendering), loaded from CDN in
  `index.html` and bridged to WASM via `window.CodeMirror` / `window.renderKaTeX`.
- LocalStorage (`gloo-storage`) backs progress, license, and theme state.
- `include_dir` embeds the curriculum assets into the binary at compile time;
  `serde`/`serde_json` parse the exercise JSON; `getrandom`/`rand` (js feature) seed
  generation; `bumpalo` arena and `async-recursion` support the engine and async flows.
- Release bundles are post-processed by binaryen's `wasm-opt` (see Develop & build).

## Module layout

`src/main.rs` is the WASM entry (`dioxus::launch(App)`); `src/lib.rs` re-exports `App`
and `AstNode` and declares the modules below.

**Learner-model modules** (`src/`):

| Module | Purpose |
|--------|---------|
| `content` | Loads the curriculum from embedded JSON (`include_dir!` of `assets/`); era → module → exercise hierarchy |
| `generator` | Fills exercise templates with lexicon words to build graded `Challenge`s |
| `game` | XP, streaks, combos, level progression, exercise flow |
| `grader` | Answer validation with whitespace/Unicode normalization |
| `progress` | Completed exercises, scores, review state |
| `srs` | SM-2 spaced-repetition scheduling |
| `achievements` | Achievement conditions and badge awards |
| `unlock` | Prerequisite state machine for module availability |
| `storage` | LocalStorage WASM bindings for persistence |
| `struggle` | Detects when a learner needs hints from attempt patterns |
| `learn_state` | Tab-focus and inactivity detection for the Learn page |
| `audio` | Sound-effect playback via JS interop |
| `sitemap` | SEO route enumeration |

**UI** (`src/ui/`): the Dioxus `App`, the `Route` enum (`router.rs`), `pages/`,
`components/`, `hooks/`, the theme system (`theme.rs`, `theme_state.rs`), responsive
helpers, and JSON-LD SEO schemas (`seo.rs`).

**Routes** (`src/ui/router.rs`): `/` Landing, `/studio`, `/learn`, `/benchmarks`,
`/guide`, `/crates`, `/registry` (+ `/registry/package/:name`), `/news` (+ `/news/:slug`),
`/roadmap`, `/roadmap-new`, `/pricing`, `/profile`, `/workspace/:subject`, `/success`,
`/privacy`, `/terms`, and a `/:..route` 404 catch-all.

**Studio** (`/studio`) is a four-mode playground (`StudioMode`):
- *Logic* — English → FOL with an AST tree and proof/tactic panel; also "Compile to SVA".
- *Code* — imperative LOGOS with a streaming REPL (`interpret_streaming`) and
  "Compile to Rust" (`generate_rust_code`).
- *Math* — theorems and types with interactive proofs and Rust extraction
  (`extract_math_rust` / `extract_logic_rust`).
- *Hardware* — English hardware spec → SystemVerilog Assertions and an in-browser,
  Z3-free proof (`codegen_sva::fol_to_sva::synthesize_sva_from_spec`, PSL + Rust monitor).

**Learn** (`/learn`) drives the gamified curriculum; **Benchmarks** (`/benchmarks`)
shows LOGOS vs other languages with a live optimization-toggle tree backed by
`logicaffeine_compile::optimization::{REGISTRY, OptimizationConfig}` that re-compiles
Rust in the browser.

The curriculum assets live under `assets/`, organized into four eras —
`01_first-steps`, `02_building-blocks`, `03_expanding-horizons`, `04_mastery` — each
holding numbered modules of exercise JSON, embedded via `include_dir!`.

## Embedded crates

All run client-side in WASM (`Cargo.toml` path deps):
`logicaffeine-base`, `logicaffeine-lexicon` (`dynamic-lexicon`), `logicaffeine-kernel`,
`logicaffeine-data`, `logicaffeine-system` (`persistence`), `logicaffeine-language`
(`dynamic-lexicon`), `logicaffeine-proof`, and `logicaffeine-compile`.

`compile` supplies `compile_for_ui`, `interpret_streaming`, `generate_rust_code`,
`extract_math_rust` / `extract_logic_rust`, the `codegen_sva` SVA pipeline, grid solving
(`SolvedGrid`), the `optimization` registry, and the re-exported `AstNode`.
`proof` supplies the `BackwardChainer`, `DerivationTree`, and tactic hints.

## Develop & build

Run **from the repository root** — `dx` resolves workspace `default-members` against the
current directory and panics otherwise:

```bash
# dev server with hot reload
dx serve -p logicaffeine-web

# optimized WASM bundle (workspace `wasm-release` profile)
dx build -p logicaffeine-web --release
```

The `wasm-release` profile inherits `release` with `debug = false`, `strip = true`,
`lto = true`, `codegen-units = 1`, `panic = "abort"`. Debug info is off so binaryen's
`wasm-opt` runs (it aborts on rustc's DWARF), while `opt-level` stays at 3 to keep the
in-wasm LOGOS engine fast.

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
