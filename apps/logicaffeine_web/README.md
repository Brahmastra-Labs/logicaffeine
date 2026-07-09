# logicaffeine-web

The browser IDE (**Studio**) and gamified curriculum (**Learn**) for LOGOS. A
single-page Dioxus/WASM app that runs the full LOGOS engine client-side — parsing,
streaming interpretation, proof checking, Rust code generation, and SystemVerilog
assertion synthesis all happen in-wasm with no server round-trip.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 4, WASM (Dioxus).
Embeds the engine crates and runs them client-side.

## Role in the workspace

This is the hosted web front-end: it consumes the Tier 0–3 engine crates as path
dependencies and compiles them to `wasm32` so the entire transpiler executes in the
browser. The crate is `publish = false` — it ships as the deployed site, not a library.

For the user-facing feature tour (the Studio modes, the Learn curriculum, XP/streaks,
spaced repetition), see **[studio-and-learn.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/studio-and-learn.md)**.
This README is the developer-facing view: tech stack, module layout, embedded crates,
and how to build and serve.

## Tech stack

- **Dioxus 0.7** (`web` + `router`), compiled to `wasm32`.
- `wasm-bindgen`, `web-sys`, `js-sys`, `wasm-bindgen-futures`, plus `gloo`
  (`gloo-timers`, `gloo-net`, `gloo-storage`).
- **KaTeX 0.16** (LaTeX rendering), lazy-loaded from CDN on the first math render and
  bridged to WASM via `window.renderKaTeX`; editors are styled native textareas.
- LocalStorage (`gloo-storage`) backs progress, license, and theme state.
- `include_dir` embeds the curriculum JSON (`assets/curriculum/` only — images stay out
  of the binary) at compile time; `serde`/`serde_json` parse the exercise JSON;
  `getrandom`/`rand` (js feature) seed generation; `bumpalo` arena and
  `async-recursion` support the engine and async flows.
- Heavy page data (benchmark results + program sources, legal HTML, the dynamic
  lexicon) is **fetched at runtime** from `/data/*`, staged from the repo's sources of
  truth by `scripts/stage-web-data.sh`; native builds compile the same bytes in, so
  tests and prerendering see identical data.
- Release bundles are post-processed by binaryen's `wasm-opt` (see Develop & build).

## Module layout

`src/main.rs` is the WASM entry (`dioxus::launch(App)`); `src/lib.rs` re-exports `App`
and `AstNode` and declares the modules below.

**Learner-model modules** (`src/`):

| Module | Purpose |
|--------|---------|
| `content` | Loads the curriculum from embedded JSON (`include_dir!` of `assets/curriculum/`); era → module → exercise hierarchy |
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

The `ui` module (`src/ui/`): the Dioxus `App`, the `Route` enum (`router.rs`), `pages/`,
`components/`, `hooks/`, the theme system (`theme.rs`, `theme_state.rs`), responsive
helpers, and JSON-LD SEO schemas (`seo.rs`).

**Routes** (`src/ui/router.rs`): `/` Landing, `/studio`, `/learn`, `/benchmarks`,
`/guide`, `/crates`, `/registry` (+ `/registry/package/:name`), `/news` (+ `/news/:slug`),
`/roadmap`, `/pricing`, `/profile`, `/workspace/:subject`, `/success`,
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

The curriculum assets live under `assets/curriculum/`, organized into four eras —
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

## Feature flags

The renderer is a per-target feature so `dx` can build both sides of the SSG
pipeline from one crate (a renderer in the base dependencies would drag
`dioxus-web` into the native prerender server and vice versa):

| Feature | Description |
|---------|-------------|
| `web` | The wasm client (`dioxus/web`) — dx auto-selects it for the browser build. Deliberately **not** fullstack: the client takes over the prerendered HTML with a fresh client-side render instead of hydrating (a hydrating client hard-fails on SPA-fallback URLs). |
| `server` | The native SSG prerender server (`dioxus/server` + `dioxus/fullstack`) — dx builds and spawns it for `--ssg --fullstack`; it answers `/api/static_routes` from `sitemap::prerender_routes()` and writes each route's HTML into `web/prerendered/`. |
| `split` | Code-splitting (`dioxus/wasm-split`): `#[component(lazy)]` route bodies move into a lazily-fetched wasm chunk. Deploy builds pass `--features split` together with dx's `--wasm-split`; without **both**, lazy components compile to direct calls. |

## Develop & build

Run **from the repository root** — `dx` resolves workspace `default-members` against the
current directory and panics otherwise:

```bash
# stage the /data runtime bundle (benchmark results, legal HTML, lexicon) —
# required before serving or building; validates every file or fails loudly
./scripts/stage-web-data.sh

# dev server with hot reload
dx serve -p logicaffeine-web

# optimized WASM bundle (workspace `wasm-release` profile)
dx build -p logicaffeine-web --release

# the full deploy artifact: split client (lazy route/engine chunks) + SSG
# prerender of every sitemap route, then lay the prerendered pages over
# public/ (dx writes the shell last)
dx build -p logicaffeine-web --release --ssg --fullstack --wasm-split --features split
./scripts/merge-ssg.sh
./scripts/verify-ssg.sh
./scripts/wasm-size-gate.sh
```

The `wasm-release` profile inherits `release` with `debug = false`, `strip = true`,
`lto = true`, `codegen-units = 1`, `panic = "abort"`. Debug info is off so binaryen's
`wasm-opt` runs (it aborts on rustc's DWARF), while `opt-level` stays at 3 to keep the
in-wasm LOGOS engine fast.

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
