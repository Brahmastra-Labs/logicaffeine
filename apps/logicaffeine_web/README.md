# logicaffeine-web

`logicaffeine-web` is the logicaffeine.com site: the Studio IDE, the Learn roadmap, the Syntax Guide, benchmarks, news and the package registry's browser, in one Dioxus app compiled to WebAssembly. The LOGOS engine runs inside the page, so parsing, interpretation, proof checking, Rust generation, SystemVerilog assertion synthesis and compiling a web page written in LGX (the LOGOS web framework) for a live preview all happen without a server round-trip. It ships as the deployed site, not as a library (`publish = false`).

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 6. The browser build is `wasm32`; the same crate also builds the native prerender server the static-site build drives (the `server` feature).

## Role in the workspace

The app consumes the engine crates of tiers 0–5 as workspace dependencies and compiles them to `wasm32`, so the whole transpiler executes in the browser.

For the user-facing tour (the Studio modes, the Learn roadmap, spaced repetition), see **[studio-and-learn.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/studio-and-learn.md)**. This README is the developer's view: tech stack, module layout, how to build and serve, and the embedded crates.

## Tech stack

- **Dioxus 0.7** (`web` + `router`), compiled to `wasm32`.
- `wasm-bindgen`, `web-sys`, `js-sys`, `wasm-bindgen-futures`, plus `gloo`
  (`gloo-timers`, `gloo-net`, `gloo-storage`).
- **KaTeX 0.16** (LaTeX rendering), lazy-loaded from a CDN on the first math render and
  bridged to WASM via `window.renderKaTeX`; editors are styled native textareas.
- LocalStorage (`gloo-storage`) backs progress and license state.
- `include_dir` embeds the curriculum JSON (`assets/curriculum/` only, so images stay out
  of the binary) at compile time; `serde`/`serde_json` parse the exercise JSON;
  `getrandom`/`rand` (js feature) seed generation; the `bumpalo` arena and
  `async-recursion` support the engine and async flows.
- Heavy page data (benchmark results and program sources, legal HTML, the dynamic
  lexicon) is **fetched at runtime** from `/data/*`, staged from its canonical files in the
  repository by `scripts/web/stage-web-data.sh`, which the crate's `build.rs` runs for every wasm
  build, so a `dx serve` or `dx build` can never ship without it. Native builds compile the same
  bytes in, so tests and prerendering see identical data.
- Release bundles are post-processed by binaryen's `wasm-opt` (see Develop & build).

## Module layout

`src/main.rs` launches `App` through `dioxus::LaunchBuilder`, as the wasm client under `web` or
as the prerender server under `server`; `src/lib.rs` re-exports `App` and `AstNode` and declares
the modules below.

**Learner-model modules** (`src/`):

| Module | Purpose |
|--------|---------|
| `content` | Loads the curriculum from embedded JSON (`include_dir!` of `assets/curriculum/`); era → module → exercise hierarchy |
| `generator` | Fills exercise templates with lexicon words to build graded `Challenge`s |
| `grader` | Answer validation with whitespace/Unicode normalization |
| `progress` | Finished lessons, the open lesson, exercise attempts and review state |
| `srs` | SM-2 spaced-repetition scheduling |
| `guide_review` | The Syntax Guide's SM-2 deck: recap self-marks and graded guide exercises feed the Review queue under a `guide/` key namespace |
| `storage` | LocalStorage WASM bindings for persistence |
| `struggle` | Detects when a learner needs hints from attempt patterns |
| `learn_state` | Learn page state: the roadmap journey, a lesson's question, and practice with its retries |
| `composer` | The symbol buttons beside a typed answer, and where a click types its symbol |
| `sitemap` | SEO route enumeration |

The `ui` module (`src/ui/`): the Dioxus `App`, the `Route` enum (`router.rs`), `pages/`,
`components/`, `hooks/`, responsive helpers, and JSON-LD SEO schemas (`seo.rs`).

**Routes** (`src/ui/router.rs`): `/` Landing, `/studio`, `/learn`, `/benchmarks`,
`/guide`, `/crates`, `/registry` (+ `/registry/package/:name`), `/news` (+ `/news/:slug`),
`/roadmap`, `/contact`, `/workspace/:subject`, `/privacy`, `/terms`, and a
`/:..route` 404 catch-all. `/pricing` redirects to `/contact` and `/profile` to `/learn`.

**Studio** (`/studio`) is a playground with one mode per `StudioMode`:
- *Logic* — English → FOL with an AST tree and proof/tactic panel; also "Compile to SVA".
- *Code* — imperative LOGOS with a streaming REPL (`interpret_streaming_with_vfs`, file
  I/O through an in-browser virtual file system) and "Compile to Rust" (`generate_rust_code`).
- *Math* — theorems and types with interactive proofs and Rust extraction
  (`extract_math_rust` / `extract_logic_rust`).
- *Hardware* — English hardware spec → SystemVerilog Assertions and an in-browser,
  Z3-free proof (`codegen_sva::fol_to_sva::synthesize_sva_from_spec`, PSL + Rust monitor).
- *Web* — an LGX page (`## Page`) compiled to WebAssembly in the browser
  (`web_host::assemble_preview`) and rendered live in a sandboxed frame beside the
  editor, rebuilt as the source changes.

The Studio's file browser holds the shipped examples (`src/ui/examples.rs`); a deep
link `/studio?file=<path>` opens one.

**Syntax Guide** (`/guide`) teaches the language in sections grouped into parts
(`src/ui/pages/guide/content/`). Each example runs in place in its `ExampleMode`: Logic,
Imperative, Theorem (the kernel's verdict via `compile_theorem_for_ui`), or Web (the
Studio's live page preview). "Now you try" exercises are graded in place: a program by its
exact output, a page by its own `## Test` blocks. Exercise results and recall self-marks
feed the `guide_review` deck.

**Learn** (`/learn`) walks the curriculum as a roadmap of short lessons; **Benchmarks** (`/benchmarks`)
shows LOGOS against other languages with a live optimization-toggle tree, backed by
`logicaffeine_compile::optimization::{REGISTRY, OptimizationConfig}`, that re-compiles
Rust in the browser, and in its other tabs the SAT solver with the EDA engines on it (model
checking, bounded model checking, rewriting, counting, cryptanalysis) and the wire codec against their counterparts.

The curriculum assets live under `assets/curriculum/`, one directory per era
(`01_first-steps`, `02_building-blocks`, `03_expanding-horizons`, `04_mastery`), each
holding numbered modules of exercise JSON, embedded via `include_dir!`.

## Develop & build

Run **from the repository root**: `dx` resolves workspace `default-members` against the
current directory and panics otherwise.

```bash
# dev server with hot reload (build.rs stages the /data bundle — benchmark results,
# legal HTML, the lexicon — on the first build and whenever a source changes;
# ./scripts/web/stage-web-data.sh runs the same staging by hand)
dx serve -p logicaffeine-web

# optimized WASM bundle (workspace `wasm-release` profile)
dx build -p logicaffeine-web --release

# the full deploy artifact (.github/workflows/deploy-frontend.yml): the eager
# client + SSG prerender of every sitemap route, then lay the prerendered
# pages over public/ (dx writes the shell last), built with the pinned dx
cargo install --git https://github.com/Brahmastra-Labs/dioxus --rev fde227fac7f9974fc61a3d126053b462a122136f dioxus-cli --locked
rm -rf target/dx
dx build -p logicaffeine-web --release --ssg --fullstack
./scripts/web/merge-ssg.sh
./scripts/web/gen-prefetch-manifest.sh   # public/chunks.json → shell warms every lazy chunk in the background
./scripts/web/wasm-size-gate.sh          # the main bundle against .config/size-budgets.toml [web]
./scripts/web/verify-ssg.sh
# the browser gates (headless Chrome + `npm install --no-save puppeteer-core@23`)
CHROME_BIN=$(command -v google-chrome) ./scripts/web/test-ssg-render.sh
CHROME_BIN=$(command -v google-chrome) ./scripts/web/test-ssg-smoke.sh
CHROME_BIN=$(command -v google-chrome) ./scripts/web/test-ssg-screenshots.sh
```

`dx` must be the pinned fork (`cargo install --git https://github.com/Brahmastra-Labs/dioxus
--rev fde227fac7f9974fc61a3d126053b462a122136f dioxus-cli --locked`), the same rev the app
crates are patched to; the crates.io `dx` builds a different client.

The `wasm-release` profile inherits `release` with `debug = true`, `strip = true`,
`lto = true`, `codegen-units = 1`, `panic = "abort"`. Debug info is on because dx's
wasm-split pass reads the DWARF symbol table; binaryen's `wasm-opt` then runs at level
`z` with `debug = false` (`Dioxus.toml` `[web.wasm_opt]`) and strips it, so the served
binary carries no debug sections. `opt-level` stays at 3 to keep the in-wasm LOGOS
engine fast.

## Feature flags

The renderer is a per-target feature so `dx` can build both sides of the SSG
pipeline from one crate (a renderer in the base dependencies would drag
`dioxus-web` into the native prerender server and vice versa):

| Feature | Default | Enables |
|---------|---------|---------|
| `web` | off | The wasm client (`dioxus/web`); dx selects it for the browser build. Deliberately **not** fullstack: the client takes over the prerendered HTML with a fresh client-side render instead of hydrating, because a hydrating client hard-fails on SPA-fallback URLs. |
| `server` | off | The native SSG prerender server (`dioxus/server` + `dioxus/fullstack`), which dx builds and spawns for `--ssg --fullstack`. It answers `/api/static_routes` from `sitemap::prerender_routes()` and writes each route's HTML into `web/prerendered/`. |
| `split` | off | Code-splitting (`dioxus/wasm-split`): `#[component(lazy)]` route bodies move into a lazily-fetched wasm chunk when dx's `--wasm-split` is passed too; without **both**, lazy components compile to direct calls. The deploy build leaves it off, because the fork's splitter miscompiles the DOM-edit stream, so the site ships one eager bundle. |

## Dependencies

These engine crates run client-side in WASM (`Cargo.toml` workspace deps):
`logicaffeine-base`, `logicaffeine-lexicon` (`dynamic-lexicon`), `logicaffeine-kernel`,
`logicaffeine-data`, `logicaffeine-system` (`persistence`), `logicaffeine-language`
(`dynamic-lexicon`), `logicaffeine-proof`, and `logicaffeine-compile`.

`compile` supplies `compile_for_ui`, `interpret_streaming`, `generate_rust_code`,
`extract_math_rust` / `extract_logic_rust`, the `codegen_sva` SVA pipeline, grid solving
(`SolvedGrid`), the `optimization` registry, and the re-exported `AstNode`.
`proof` supplies the `BackwardChainer`, `DerivationTree`, and tactic hints. The external
crates are the ones listed under Tech stack.


## Testing the Learn page

Three layers, each run RED-first through `scripts/test/tdd.sh` (which demands that the selected
tests fail before a fix and pass after it):

```bash
# unit + SSR: the session rules, curriculum loading, lesson markup, and what /learn renders
scripts/test/tdd.sh green rust -p logicaffeine-web

# the browser layer: keys, clicks, scrolling, focus, storage — against a built site
dx build -p logicaffeine-web
scripts/test/tdd.sh green learn-e2e target/dx/logicaffeine-web/debug/web/public
```

`scripts/web/test-learn-e2e.sh --list` names every browser scenario and the defect
it locks.

## Testing the Syntax Guide

Every LOGOS example, exercise and quoted snippet on `/guide` sits in the Studio's editor with the
language server on. Two layers lock it:

```bash
# what the language server says about the guide's code, and what /guide renders
cargo nextest run -p logicaffeine-tests --features web-tests --test guide_language_server --test guide_render

# the browser layer: hovers, squiggles, editors that fit their code, callout corners —
# against a built site or a running dev server, in Chrome (CHROME_BIN) or Firefox (FIREFOX_BIN)
./scripts/web/test-guide-e2e.sh target/dx/logicaffeine-web/debug/web/public --junit guide-e2e.xml
./scripts/web/test-guide-e2e.sh http://127.0.0.1:8080 --firefox
scripts/test/tdd.sh green junit guide-e2e.xml
```

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/apps/logicaffeine_web/CHANGELOG.md)
