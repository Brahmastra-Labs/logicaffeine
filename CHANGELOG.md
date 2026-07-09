# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.10.0] - 2026-07-08

### Fixed
- **Web app: query parameters survive router boot — studio deep links work.** The Dioxus router's startup normalization re-serializes the matched route and replaces the browser URL, destroying any query string the route type doesn't model — so `/studio?file=…` share links and refreshes fell back to the default example, and the same wipe silently broke Stripe checkout success (`?session_id=` license activation), the registry GitHub OAuth callback (`?token=&login=`), and `/news?tag=` filters. All four routes now declare their parameters as typed query segments (`#[route("/studio?:file")]` etc.) and the pages receive them as props; the raw `location.search` scraping is gone and forbidden forever by a source ratchet (`query_scraping_is_forbidden`). Round-trip locks pin every parameterized route through parse→serialize→parse, `studio_file_url` builds share links from the route type itself so writers can't drift, and the studio keeps the address bar on the canonical `?file=` URL of the open file. Sitemap hardening rode along: an enum-completeness ratchet (every static route must be sitemapped or explicitly excluded), boot-normalization round-trip and hygiene locks over every published URL, a robots.txt↔sitemap agreement check, and a `lastmod`-freshness lock that immediately caught the shipped sitemap lagging the 0.10.0 release article.


### Added
- **Prebuilt `largo` binaries + one-line installers** — `release.yml` gains a `build-cli` matrix: 5 platforms (linux/macos × x64/arm64, windows x64) × two flavors — lean, and **full** with Z3 built from the z3-sys vendored source and statically linked (`verification-static`). Linux builds pin `ubuntu-22.04` for the glibc 2.35 floor; arm64 builds run on native ARM runners (no C++ cross-compilation); artifacts are packaged inside the build job (upload-artifact drops the exec bit — also fixed for the LSP tarballs) and the release carries a `SHA256SUMS`. `curl -fsSL https://logicaffeine.com/install.sh | sh` (POSIX sh, dash-clean; `--full`, `--version`, `--to`) and `irm https://logicaffeine.com/install.ps1 | iex` (TLS 1.2, `Unblock-File`, user-PATH) install atomically to `~/.local/bin` / `%LOCALAPPDATA%\Programs\largo` after verifying the checksum — no GitHub API (the latest tag resolves via the `releases/latest` redirect), no sudo, no rc-file edits. Both flavors install as `largo`; `largo --version` reports `(full)`. The installers are static assets of the web app (`apps/logicaffeine_web/public/`), tested offline by `scripts/test-installers.sh` (fake release layout + local HTTP server + corrupted-tarball/404/PATH cases, under sh/dash/bash --posix), gated in CI by `installer.yml`, and tag pushes run pinned `e2e-install` jobs against the real release.
- **`largo` widened to the whole engine** — the CLI grows from 10 subcommands to the full front door: **`repl`** (interactive session: imperative statements against a persistent replay session — the exact `run --interpret` engine, so REPL semantics cannot drift from program semantics — plus English→FOL logic mode with discourse anaphora, `:format`/`:readings`/`:vars`/`:save`-as-runnable-program meta-commands, rustyline editing with keyword completion on a TTY, a plain scriptable line loop on a pipe), **`logic`** (English → FOL: `--format unicode|latex|ascii|kripke`, `--all-readings` scope+forest enumeration, `--discourse`, `--pragmatic`; byte-identical to the library API by test), **`prove`** (`## Theory` developments and English `## Theorem` blocks, kernel-certified, `--trace` derivation trees, `--json`), **`sat`** (the certified DIMACS solver as a verb — the driver extracted to `logicaffeine_proof::satcli` and shared verbatim with `logos-sat`; DRAT/DPR/SR export, competition exit codes 10/20), **`fmt`** (`--check`; rules lifted to `logicaffeine_language::source_format`, the LSP formatting provider delegates to the same function), **`emit rust|c|wasm|wasm-linked`** (`-o`, standalone-file mode), **`doc`** (markdown from a project's `##` blocks), **`add`/`remove`** (format-preserving `Largo.toml` edits via toml_edit), **`clean`**, **`completions`** (5 shells), and a reserved hidden `test` verb for the future test framework. New engine seams backing it: `logicaffeine_compile::repl::ReplSession` (replay session with output high-water mark and error rollback), `Interpreter::global_bindings`, `Session::set_format`.
- **`largo` UX overhaul** — global `-q/--quiet`, `-v/--verbose`, `--color auto|always|never` (NO_COLOR honored; all output routes through anstream so engine ANSI never leaks into pipes); cargo-palette styled help with an Examples section on every command (enforced by a CommandFactory meta-test); errors render as `error:` + an actionable `help:` hint with typed exit codes (`ui::CliError`). `largo build` now **streams cargo's own colored output live** (stdout inherited, stderr teed with a bounded tail) behind cargo-style `Compiling`/`Building`/`Finished` phase headers, passes `--color`/`-q`/`-v` through, and classifies failures: a `## Requires` resolution problem is framed as user-actionable, rustc errors in generated code as "a LOGOS compiler bug, not an error in your program". The CLI crate itself was restructured (module-per-command `commands/`, `ui.rs` output substrate) and its long-dead `phase37` integration tests were resurrected into `apps/logicaffeine_cli/tests/` where they actually run.
- **EXODIA native execution tier** — a register bytecode VM (the live sync/WASM engine, corpus-certified against the tree-walking interpreter as a shadow oracle) with a copy-and-patch JIT on top. `Int` fast paths are bit-identical to the kernel by the wrapping-`i64` spec, pinned by an edge-grid differential. The tier-up seam compiles hot **functions** per call with argument guards and kind-inference (params `Int`, comparisons `Bool`), and region-tiers hot **Main loops** (OSR-lite) with incoming-dead analysis and per-entry guards; anything outside the integer/float subset fails closed to bytecode — the deopt contract.
- **`logicaffeine-forge`** — new crate: the copy-and-patch JIT's executable-memory layer and stencil runtime (native only). `JitPage` allocates page-aligned memory, copies machine code in, flips it to executable, and returns a callable function pointer; compiling a function at runtime is `memcpy(stencil bytes)` + patch relocations. Includes the build-time-baked stencil runtime, the J1 micro-op compiler, and the **EXODIA contiguous register-allocating region/function x86-64 codegen tier** (`regalloc.rs`, `x64asm.rs`) that replaces per-stencil-piece dispatch. Platform-correct W^X for macOS/aarch64 (`MAP_JIT` + per-thread write-protect toggling + `sys_icache_invalidate`), other Unix (`mmap` → `mprotect` + aarch64 I-cache flush), and Windows (`VirtualAlloc` → `VirtualProtect` → `FlushInstructionCache`). Build-time stencil extraction via a `build.rs` over `object` with a relocation whitelist and tail-call/leaf-purity gates.
- **`logicaffeine-jit`** — new crate: the LOGOS native tier. `ForgeTier` translates VM bytecode (`Op`) into the forge's `MicroOp` subset for both whole functions (`ChainFn`) and hot loop regions (`RegionChain`), compiles each to a native stencil chain or the register-allocating backend, and hands the unit back. `install()` makes the tier process-wide — every live VM constructor picks it up, and `largo` installs it at startup. Native-only (`#![cfg(not(target_arch = "wasm32"))]`); WASM builds it to nothing and the browser runs pure bytecode.
- **`logicaffeine-synth`** — new crate: EXODIA Phase 2, the Forge's offline proof tooling. Z3 specifications for the integer micro-operations over 64-bit bitvectors, satisfiability and algebraic-property gates, and a three-way witness harness that runs Z3-chosen inputs through the real compiled stencil. Development/CI-time only; never on the production runtime path.
- **`logicaffeine-runtime`** — new crate: the deterministic concurrency runtime for the interpreter and VM — task scheduler, FIFO channels, `Select`, a logical-clock timer wheel, and the seed/trace machinery. A run is a deterministic function of `(program, seed)` and replays bit-for-bit from `(program, trace)` through a single `Chooser::decide` choke point. Pure `std`, WASM-safe, tokio-free, and by charter never linked into AOT-compiled binaries (the compiled path uses `logicaffeine-system`).
- **`logicaffeine-tv`** — new crate: SMT translation validation. Proves the emitted Rust is observationally equivalent to the LOGOS source per compile by symbolically executing both into the `logicaffeine-verify` domain and discharging the equivalence with Z3. `check_encoder_sound` cross-validates the LOGOS encoder against the tree-walking interpreter as the load-bearing trust anchor (rung 3–4 translation validation).
- **Verified arithmetic and proof advances** — proof-producing arithmetic with certificates and worked examples (`logicaffeine-proof::arith`), modal translation and independent verification, a CDCL core plus an incremental grid solver with grounding and trust-tiers, and label/PP convergence.
- **Strict whole-input parsing** — `compile()` now rejects parses that strand tokens (`TrailingTokens`) instead of silently dropping meaning; a dropped `until AWREADY` clause is a wrong assertion, not a style issue. The parser gains the coverage the strictness demands: noun-noun compound heads (`stop bit`, `grant signal`) with ambiguity-preserving gates, possessive heads over `Ambiguous` noun/verb words (`its value`), trailing temporal operators inside `If`-consequents (`until`/`release`/`weak-until`, `within N cycles`), postposed `when`-clauses and sentence-final temporal anchors, and quantified/cardinal objects under modals and under `never`. Lexer: letter-hyphen-letter compounds, attributive participial adjectives, `-ing` prepositions. A per-token lexical-ambiguity forest enumerates the strict-parse combinations.
- **Hardware-spec parsing and synthesis (PR #36)** — bounded delay synthesis (`within N cycles` → SVA `##[0:N]` via a `BoundedEventually(u32)` operator preserved through Kripke lowering), signal extraction for counting quantifiers, copula temporal adverbs in consequent clauses, `while` as a duration subordinator, and `exactly N` tokenization. Probe corpus 87/102 → 101/102.
- **The socratic teaching LSP — the editor teaches you to code.** One teaching brain (`logicaffeine_language::teach`) now feeds every surface: a lesson table whose TYPE requires a plain one-sentence explanation, a runnable example, and a socratic question or tip for every taught keyword (24), every `##` block type (all 19), and the built-in types — LSP hover and completion docs render it through one shared markdown renderer, and `largo repl` teaches the identical lessons via the new `:explain <word>`. All 39 socratic error explanations were rewritten to the contract *what happened → why → a guiding question → the concrete next step* (token names humanized — "a comma (',')", never `Comma`; byte offsets removed — the span carries the location) and committed as a golden so every future wording change is a reviewable diff, with a mechanical teaches-or-exempted lock behind it. Diagnostics with stable codes now carry `codeDescription` links into LOGOS_QUICKGUIDE.md (a compile-forced fourth field on the decision table; anchors verified against the guide's real headings). The stdlib became genuinely literate: all 158 prelude definitions (crypto suite included) carry `## Note` docs that surface in hover/completion/signature help — while the runtime prelude strips them byte-exactly, so nothing changes at execution. The whole surface is ratcheted: a teaching-corpus golden (hover markdown verbatim, completion text, signature docs, lens + quickfix titles, docs links), a quickguide↔teach parity lock (every guide row maps to a lesson or records why prose suffices, both directions), a stdlib docs lock, and a quickfix-parity lock proving every promised fix against a real broken program — which found that `undefined-variable` has NO producing program (typo'd names pass silently today; recorded as an explicit gap for the unresolved-reference work).
- **LSP: cross-file references and rename, a warnings layer, interpolation painting, range formatting.** References and rename now cross files (open buffers answer from LIVE text — a stale disk index can never produce a wrong edit; unopened files answer from the workspace reference map; local `Let`s stay private). New `shadowed-variable` warning (a second `Let x` in the same block, related-info pointing at the first) and `unused-function` hint (only in programs WITH a `## Main` — a library's functions are its API), both under a severity table locked live by producing programs. `"Hello {name}!"` interiors paint as CODE (braces as operators, `name` exactly as it paints outside the string). Range formatting un-exempted: the structural whole-document format filtered to intersecting lines.
- **The teaching ratchets caught five live bugs on their first runs**: sentence-level parse recovery DEGRADED specific diagnoses to generic ones (`x is 5.` taught "say equals" in the full parse, then recovery replaced it with "expected a statement" — the specific error now wins, and the imperative dispatch diagnoses it directly); the use-after-move quickfix could offer and INSERT `a copy of .` (punctuation read from the diagnostic span — it now names the variable from the message and edits the variable's own token); spelling suggestions fired noise on coded diagnostics ("Did you mean 'a'?" for a moved `x`); token-extracted function definitions (any document with a theorem block) dropped their parenthesized parameters from hover/completion/signature help; and the root `assets/std` copies had silently drifted from the canonical compile-tree stdlib (now byte-locked in both trees).
- **The AST depth gate: no LOGOS program can stack-overflow the compiler anymore, on any surface.** A 5,000-term expression chain (or parenthesis tower, or block pyramid) used to SIGABRT the CLI, REPL, LSP, and web Studio alike — every downstream walker recurses on tree depth. The language crate now enforces "parsed ⇒ bounded" at the single `parse_program` choke point (`ast_depth`: an iterative, wildcard-free walker — a new AST variant with children fails compilation until classified — plus parse-time recursion guards), so deep programs get a graceful `AstTooDeep` diagnostic that teaches BOTH fixes: split into `Let` bindings, or copy the suggested `LOGOS_MAX_AST_DEPTH=<n>` override from the message. The limit is environment-tunable (default 128, sized for the smallest standard stacks and proven in 2 MiB-thread tests) and **the override genuinely works**: `largo` runs on a worker thread whose stack is sized FROM the limit, so a potato keeps its safe default and a big machine raises one knob and compiles the 5,000-term chain clean. Long-line error excerpts window around the caret instead of flooding 20 KB of generated source; `largo check` renders parse failures socratically instead of as a Debug struct. Full corpus unaffected (language 268 / compile 1045 / LSP 266 all green).
- **LSP: end-to-end harness + ratchet locks.** The language server gains its first integration tests — a wire-level harness that drives the real `tower_lsp::Server` loop over in-memory pipes (framing, request correlation, `publishDiagnostics`), plus ratchet locks in the `readme_lock` idiom: every `TokenType` must classify inside the advertised semantic-token legend, every `ParseErrorKind` must pass a new total `decision_for` severity/code/quickfix table (a new error kind does not compile until it decides all three), the capability set is pinned in both directions with reasons for what's deliberately absent, and the token legend is locked append-only. Two real bugs fell out immediately: the signature-help trigger `with` was multi-character (LSP clients only send single characters — it could never fire; now `␠`/`,`), and parse-path `use-after-move` diagnostics neither spoke the ownership house voice nor linked their cause — they now say "Cannot use 'x' after giving it away" and carry `DiagnosticRelatedInformation` pointing at the `Give` statement, same as the ownership-checker path.
- **VSCode extension: real editor front-end.** The 46-line shell became a full client: platform/arch-aware resolution of the bundled server binary (explicit `logicaffeine.lsp.path` wins and fails loudly if missing; `bin/logicaffeine-lsp-<platform>-<arch>` next, win32-arm64 falling back to the x64 binary under emulation; PATH last — every cell unit-tested via an injected-fs resolver), an output channel + `logicaffeine.trace.server`, a language-status item showing server state and which binary is running, actionable spawn-failure notifications, and restart/show-log commands. The server's code lenses stopped dead-ending: `logicaffeine.run` (task running `largo run` with an `interpret|debug|release` mode setting from the file's `Largo.toml` root), `logicaffeine.verify` (project `largo verify`, license via SecretStorage — never settings.json — injected as `LOGOS_LICENSE`, honest "preview" copy), and `logicaffeine.prove`/`checkProof` (`largo prove <file>`, optional `--trace`), plus context menus, an editor run button, and keybindings. Integration suite (test-electron under xvfb in CI, against a debug server build) locks activation, the diagnostics round trip, task spawning from the fixture project, and that every contributed command is registered.
- **VSCode extension: highlighting overhaul.** The TextMate grammar was rewritten against the real surface (`LOGOS_QUICKGUIDE.md` + the lexer): `#` line comments (declared in language-configuration too — they never were), decomposed `##` headers (`## To` names color as function definitions, `## A Point has:` as type definitions), `Let`/`Set` binding captures with `mutable` as a storage modifier, parameter/type annotations, `a new Type` construction, `When Variant` match arms, call sites and UFCS method calls, the multiword English operators (`is at least`, `is divisible by`, `combined with`, …), string interpolation holes with format specs, duration/date/time/money literals, and `## Note`/`## Example` bodies rendered as documentation-comment prose so English recedes while code speaks (mirroring the server's semantic classification). A `semanticTokenScopes` contribution maps all 13 server token types to theme-safe scopes, and a markdown injection grammar highlights ```` ```logos ```` fences in literate `.md`. Locked by a quickguide coverage **ratchet** (every non-`(proposed)` backticked surface form must highlight or be allowlisted with a reason — currently zero allowlisted — failing in both drift directions, the `guide_examples.rs` pattern) plus tmgrammar snapshot tests over an imperative/theorem/CRDT/prose corpus.
- **LSP/tooling audit: highlighting corrected, the formatter made safe and structural, one classifier everywhere.** A new committed corpus golden (every token of a representative program rendered `lexeme→class+modifiers`) caught three live classification bugs — `Int` painting as a variable, struct names painting as properties (span collision from registry-path field definitions), `mutable` painting as a type — all fixed and now diff-reviewable forever. The canonical formatter was silently corrupting multiline strings (trailing-space strip, tab conversion, and line-blanking INSIDE `\"\"\"` literals, live in both `largo fmt` and the LSP); it is now string-and-prose-protected AND structure-aware (4 spaces per lexed nesting level), locked by token-stream-equivalence + idempotence property tests. Token classification moved into the language crate (`token_class`) so the LSP and the REPL share one brain — and `largo repl` gained live ANSI syntax highlighting from it. A server-side quickguide ratchet (every canonical form must paint through the real pipeline, crashers named individually) found that a bare `## Theorem:` header PANICKED analysis — the parser is now total (EOF-normalized streams, clamped `peek`), with server-side panic containment behind it.
- **VSCode extension: marketplace-ready release pipeline.** Marketplace assets landed (icon, `.lg` file icon, snippet library, a four-step "Get started with LOGOS" walkthrough, the README that becomes the marketplace page — all locked into the VSIX by the packaging test), and `release.yml` grew the real pipeline: per-platform VSIXs (one server binary each, plus a binary-free universal fallback the Marketplace serves to untargeted platforms), an **install gate** that installs the actual packaged VSIX into a real VSCode on Linux/macOS/Windows and asserts the bundled server answers diagnostics before anything publishes, and fail-loud `vsce`/`ovsx` publish jobs (missing `VSCE_PAT`/`OVSX_PAT` abort — never a silent skip; re-runs tolerate "already exists") — the publish job is wired but deliberately held disabled for 0.10.0, so the VSIXs ship as GitHub-release artifacts only; marketplace listings follow. Trust hardening: untrusted workspaces cannot redirect which binaries execute; virtual workspaces get highlighting only.
- **LSP: capability completion + gates.** Document highlights (WRITE on bindings/mutations), selection ranges walking English structure (word → sentence → block → document), on-type formatting (typing `.` applies the `largo fmt` rules to the closed sentence), call hierarchy over indexed call sites, and pull diagnostics with cross-engine result ids — the advertised surface is now 20 providers, every one pinned by the capability lock. A live `logicaffeine.flycheck.enable` setting toggles the rustc pass with retraction, and latency locks put a loud ceiling over whole-document analysis.
- **LSP: the rustc flycheck — "English with a borrow checker".** The dormant rustc→LOGOS diagnostic bridge is live end-to-end: codegen now RECORDS the sourcemap it always had an API for (`codegen_program_mapped` ties every generated line to its LOGOS statement span and tracks variable ownership roles — previously `SourceMapBuilder` was never called and every translated error had no span), a new `rustc_check` compiles with the optimizer off (spans stay 1:1) into a persistent per-workspace cache and runs `cargo check --message-format=json`, and the LSP triggers it on save: rustc findings arrive translated to English on real user-source spans under a distinct `logicaffeine (rustc)` source. Newer saves win by generation guard, edits clear findings, overlaps with interactive errors deduplicate, and no cargo = silent degrade — all mock-proven over the real server loop. `largo check --deep` is the CLI twin. The first fixture immediately proved the point: rustc caught `Give a to b.` where `b` is an Int (generated `b(a)`) — a nonsense program the parser and both local checkers accepted.
- **LSP: workspace symbols + cross-file navigation.** The server now indexes every `.lg`/`.md` file under the workspace folders in the background (bounded scan, refreshed on save/watched changes): `workspace/symbol` searches definitions across files never opened, and goto-definition falls back to the workspace index when the current document doesn't define the name — landing in the defining file directly.
- **LSP: socratic diagnostics depth.** The parser gained a statement-span side-table (`Parser::stmt_spans`) and the compile crate a `check_program_collect` that reports every failing top-level statement (fail-fast `check_program` untouched) — so the typechecker finally surfaces in the editor: return-type mismatches, arity, field-not-found (message lists the fields that DO exist, filled from the registry), not-a-function, infinite-type, dimension — each anchored on its own statement's span with a stable diagnostic code. Block recovery went sentence-level: each independently broken sentence reports (excise-and-reparse, structure-guarded) while the good statements around it stay parsed and navigable — plus an EOF-termination fix for mid-stream block slices the parser could previously walk off. Unused `Let`s get a faded UNNECESSARY hint with a remove-statement quickfix.
- **LSP: resolution-aware semantic highlighting.** English's grammar is the syntax — verbs paint as functions, nouns as types, adjectives as modifiers — and now a `SymbolIndex` overlay upgrades identifiers to what they *resolve* to (parameter/function/type/field/variant/variable) at declaration and reference alike, with `declaration` firing only at the definition site (references were previously painted as declarations), `readonly` on immutable `Let`s, a new `modification` bit on write sites, and a new `defaultLibrary` bit on stdlib prelude names; `## Note`/`## Example` prose recedes to comment color. Range and full/delta semantic-token requests landed (single-splice deltas against cached result ids). A real resolution bug fell out: verbs resolved by lexicon lemma ("Greet") instead of surface form ("greet"), and lexically ambiguous words ("name") resolved not at all — English-word identifiers now support goto-def/references/rename.
- **LSP: async snapshot architecture + incremental sync.** Documents became immutable `Arc` snapshots (holding a map guard across `.await` — a latent deadlock in `publish_diagnostics` — is now unrepresentable), text sync moved FULL → INCREMENTAL with UTF-16-correct range edits, and a new per-document scheduler debounces analysis (~150 ms) behind a generation counter with the pipeline on `spawn_blocking`: a 51-keystroke burst now runs one analysis over the final text instead of 51, locked by an e2e coalescing test over the real server loop.
- **Web app: full SSG prerendering.** The deploy now runs `dx build --ssg --fullstack`: dx builds the wasm client (`--features web`) and a native prerender server (`--features server`), then writes real per-route HTML for every route in `sitemap::prerender_routes()` — landing, pricing, guide, crates, learn, benchmarks (full tables from the native-side data), roadmap, privacy/terms, news index + **every news article**, and shell+meta pages for studio/registry/profile. Each page carries its own `<title>`, description, Open Graph/Twitter card, canonical link, and JSON-LD (emitted by `PageHead` via `document::Meta`/`Link` on the server render), so crawlers, AI bots (GPTBot/ClaudeBot/PerplexityBot execute no JS), and link unfurlers finally see real content — and first paint is prerendered HTML instead of a blank shell while the wasm loads. The client is deliberately **not** hydrating (a hydrate-enabled client hard-crashes on SPA-fallback URLs like `/registry/package/:name` that cannot be enumerated at build time): the wasm mounts into a separate `#app` root while the prerendered copy stays visible in `#main`, and a MutationObserver in the shell removes the copy only once the app has committed real, visible content (judged on DOM truth — non-empty rendered text), so the swap is atomic: no cleared-DOM gap, no unstyled flash, and a still-loading route can never blank the page. The prerender writes into `web/prerendered/` and `scripts/merge-ssg.sh` lays it over `public/` after dx finishes (the client build rewrites the shell `index.html` last and would clobber the prerendered `/`). Gates: `tests/ssr_render.rs` renders all 46 routes on the native target (any browser API in a render path fails there, plus per-page content markers and a JSON-LD check), `scripts/verify-ssg.sh` proves every sitemap route produced its file with exactly one og:title (dx silently skips routes that panic), `scripts/test-ssg-smoke.sh` drives headless Chrome through a prerendered route and an SPA-fallback route (content present, takeover completed — `#main` gone, no duplication), and `scripts/test-ssg-screenshots.sh` loads EVERY sitemap route in headless Chrome after wasm boot — per-page content markers, zero page errors, completed takeover, a pixel-level blank-viewport detector (sharp-edge density, catching styled-but-empty pages that DOM checks miss), and a screenshot per page for human review. The smoke test caught the wasm-split blank-page regression the moment a local Chrome could run it. Two real render-path panics were fixed by the new tests: `LicenseState::new` and `ThemeState::new` read localStorage un-gated at App root.
- **Web app: wasm code-splitting — first visit ~3.9 MB → ~683 KB gzipped.** The dx `--wasm-split` pipeline ships behind the new `split` cargo feature: all 13 heavy route pages are `#[component(lazy)]`, and the two engine-driving landing showcases (the hero mini-studio, the Hello-World editor) were extracted into lazy components so the entire LOGOS engine leaves the eager bundle. A first-time visitor downloads a **1.1 MB main bundle plus a ~1 MB landing chunk (~683 KB gzipped total)** for a fully interactive landing; the engine chunks (Studio ~10 MB, benchmarks/learn/guide ~5–6 MB each) stream in only when their page opens, once per browser (immutable-cached). Getting there took fixing dioxus itself: its 0.7.9 split runtime crashed pure-CSR resolution three ways (single-split routes hit `ReplaceWith` on an unregistered node — "Cannot read properties of undefined (reading 'listening')"; double-lazy routes hung; NotFound never committed), rooted in two core-suspense bugs (writer-less background renders reaching a real writer) and two wasm-split-cli symbol-resolution bugs (LTO-folded names and mangled-vs-demangled name matching, which corrupted main-module data segments). All four are fixed on our dioxus fork (`Brahmastra-Labs/dioxus`, pinned by rev in `[patch.crates-io]` and the `deploy-frontend.yml` dx install) and submitted upstream; the deploy unpins once the fixes land in a released dioxus. The CSR takeover, size gate (eager ≤ 2 MB / total ≤ 60 MB), and the 46-route screenshot suite all gate the split build. `wasm-release` carries `debug = true` (the splitter needs the DWARF symbol table; `wasm-opt` strips it afterward).
- **Web app: runtime data bundle (`/data/*`).** The wasm binary no longer carries its heavyweight data. Benchmark results (latest/solvers/latest-codec/latest-interp JSON), all 32×11 benchmark program sources, privacy/terms HTML, and the dynamic lexicon are fetched at runtime from `/data/*`, staged from the repo's sources of truth by `scripts/stage-web-data.sh` (self-validating — a missing or empty file fails the build, the same guarantee `include_str!` gave at compile time; wired into the deploy before `dx build`, gitignored so it can never drift). Native builds compile the identical bytes in, so tests and the SSG prerender see the same data through shared accessors; sources are now looked up **by benchmark id** (`bench_sources_for`) instead of positional indexing, locked by a new `sources_cover_every_benchmark` test. The Benchmarks page gains a loading/error shell with retry; the lexicon loads once behind a `LexiconGate` (it was being re-parsed from 279 KB of JSON on *every render* of the Learn/lesson/review pages).
- **SAT benchmark: our own proof size, plus CaDiCaL + CryptoMiniSat.** The `/benchmarks` "Our solver vs the field" section now shows the size of *our* certified proof beside every winning family, the way Kissat and SaDiCaL already showed theirs — answering "why doesn't Logos show its proof size?". For the pigeonhole class it is the **SR proof**; for the algebraic families (Tseitin GF(2), mod-p, mod-6) it is the **compact GF/ℤ-ring linear-dependency certificate** the Gaussian route actually returns (O(#equations)), *not* its exponential clausal-DRAT expansion. It is measured by streaming the artifact through a capped byte counter in memory (`proof_emit::SizeSink` + the new streaming `proof_emit::write_sr`) — never materialized, never written to disk, and bounded so a pathological proof cannot exhaust RAM. Two solvers join the field on the byte-identical DIMACS: **CaDiCaL** (Biere's mainline DRAT reference) and **CryptoMiniSat** (native GF(2) Gaussian, run in its default DRAT-emitting config). Both wall on pigeonhole exactly as the resolution bound predicts.
- **New crush families.** `families::mod_m_tseitin_expander` — the composite-modulus Tseitin obstruction over `ℤ/m` (mod-6), decided by the **ring route** (`ℤ/6 ≅ GF(2) × GF(3)` via CRT) that resolution walls on and a pure GF(2) parity engine is blind to. Plus a **pigeonhole-variants** family (functional / onto / weak PHP), each refuted by the generic prover's matching/symmetry route while Z3/Kissat/CaDiCaL/CryptoMiniSat all wall.
- **A proof size for every family, per its own certificate kind** — the multi-route prover shows the certificate the route actually produces: SR proof (PHP), GF/ℤ-ring linear certificate (parity/mod-p/mod-6), the O(1) **counting certificate** `pigeons > holes` (pigeonhole variants), the compact **Hall witness** (mutilated chessboard), and the plain-CDCL **DRAT** on the random control's UNSAT instance. Emitting a *clausal* proof for the structural families would be the exponential blowup the competitors wall on, so the compact re-checkable certificate is what we measure.

- **Documentation set promoted** — the root `README.md` becomes the product front page (mission, quick start, a compiling merge-sort walkthrough, eight feature pillars, the full 8-chart benchmark section, the tiered workspace map) and `docs/` lands eight code-grounded guides (imperative mode, logic mode, execution tiers, proof & verification, concurrency, Studio & Learn, the `largo` CLI, architecture). Every ```logos example and every English→FOL pair in the set is extracted and checked against the live compiler by the `doc_examples` harness, so the documentation cannot silently rot; all crate-README cross-links repointed from the staging paths.

### Changed
- **Interpreter performance** — release-bench: tree-walker 70ms → VM 16.6ms (4.2×) → tiered 6.0ms (11.7×). The VM+JIT geomean moves from 3.34× to ≈1.0× versus Node/V8 on the interpreter benchmark suite, with 13 of 32 programs faster than V8; Main loops that never reached the JIT now run native.
- **Run-path optimizer** — magic-reciprocal division/modulo, run-path recursion inlining, and loop-invariant pointer/length plus constant hoisting on the interpreted execution path.
- **Workspace and CI** — workspace members gain `forge`, `jit`, `tv`; `default-members` excludes the Z3-backed crates (`verify`, `tv`) so a plain `cargo test` needs no Z3. `publish.yml` now derives the publish set and dependency order from `cargo metadata` and fails loudly on any publish error (re-runs skip already-published versions); `test.yml` enables `logicaffeine-tests/verification` explicitly; new `forge.yml` runs a 4-OS runtime matrix and a 7-target cross-check.
- **Benchmark artifacts** — generated `.s`/`.ll` asm and LLVM-IR dumps (~620 MB) moved behind `.gitignore`; they are build outputs of `run-logos-vs-c.sh` and regenerable.
- **Lockstep publishing** — `logicaffeine-forge`, `logicaffeine-jit` and `logicaffeine-runtime` join the published lockstep version line. Lockstep is now structural: every member inherits `version.workspace = true` and internal dependencies resolve through `[workspace.dependencies]`, so the version lives in exactly one file and `scripts/bump-version.sh` edits only the root manifest.
- **CDCL engine — per-conflict allocation eliminated (~30% faster on random 3-SAT).** The general solver's hot conflict path matched the SOTA lineage in *algorithms* (two-watched literals + blocking literals, 1UIP, recursive minimization, VSIDS heap, LBD/glue deletion, Glucose+Luby adaptive restarts, phase saving) but still allocated per conflict the way a reference implementation does not. `analyze` and `lit_redundant` cloned every reason clause (now indexed in place — copying each `Lit` ends the borrow before the mutation); learned-clause `minimize` allocated a fresh `HashSet`+`HashMap` each conflict (now the shared `seen` array plus solver-resident, sparsely-reset memo/touch buffers); and LBD was a per-conflict `Vec`+sort (now generation-stamped level counting). Byte-identical search — same conflicts, same propagations, same verdicts (every CDCL brute-force and verdict-invariance test unchanged) — just no per-conflict heap traffic: `random_3sat` batch throughput ~75 ms → ~50 ms per instance. Speeds up the whole VM/Studio path, which runs the same engine.
- **CDCL engine — implicit binary clauses in the watch lists.** Each `Watcher` now carries a `binary` flag; when a binary clause's blocking literal is not already true, the clause is decided unit-or-conflict directly from the watcher with **no clause dereference** (the Kissat/CaDiCaL trick), kept exact by watch (re)creation on every strengthening + `reduce_db` rebuild. Zero regression (all 938 proof-crate unit tests green); it pays off on binary-heavy and long solves — short random 3-SAT, being mostly the original ternary clauses, sees it as neutral.
- **Certified-prover cascade — the expensive cuts are escalation-only.** `prove_unsat`'s costly certified rungs (Lyapunov collapse, Nullstellensatz, Polynomial Calculus, the recursive certified symmetry break) ran on every call — a ~50 ms tax on microsecond equivalence/BMC obligations that had quietly inverted the native-vs-Z3 speed locks. The cheap structural recognizers stay upfront; the complete CDCL search then runs under a conflict budget, and only an instance that exhausts it — the exponential families the rungs exist for — escalates, finishing on a fresh solver so the refutation certificate stays self-contained. Verdicts unchanged; `native_is_faster_than_z3` and `optimizer_beats_z3` hold again and the hard symmetric families still refute through the escalation path.

### Fixed
- **`## Requires` dependencies were silently dropped or mis-scoped** — both Cargo.toml generators appended user-declared crates *after* a section header (`[profile.release]` in the CLI's build path, `[target.'cfg(linux)'.dependencies]` in `compile_to_dir`), so the dependency landed in the wrong TOML table: cargo ignored it entirely on the `largo build` path, and treated it as Linux-only on the compile-to-dir path. User dependencies are now written inside `[dependencies]` before any later section; caught by the new build-passthrough e2e (a nonexistent `## Requires` crate must fail the build with a dependency-resolution framing).
- **3sg stem recovery** — `strip_s` derived the stem from surface heuristics, making `planes` the 3sg of `plan` and sinking gerund subjects (`Flying planes can be dangerous.`). The inverse is now defined by the forward rule: a stem is accepted only if it is a known base verb whose `third_person_of` regenerates the surface form exactly.
- **Word-class regressions from lexicon growth** — presupposition triggers read through `Ambiguous` tokens again (restoring Van der Sandt projection under negation) via one shared `consume_presup_trigger`; `strip_s` resolves sibilant `-es` and `-ies` stems through `is_base_verb`; a temporal-adverb reading is blocked immediately after a determiner (generic structs regain their first field); focus constructions accept copular predication (`Only dogs are red.` → `Only(D, Red(Dogs))`).
- **Exercise generation** — challenge generation rejection-samples (`MAX_DRAWS=32`), so one unparseable word in a pool can no longer sink an exercise (456/456 generate); `debug.rs` and `equivalence.rs` doc examples are now compiling doctests.
- **VSCode extension: the shipped VSIX could not activate.** `release.yml` packaged with `vsce package --no-dependencies` on top of a plain-`tsc` build, so `vscode-languageclient` was never in the package — the released extension failed at import — while ~42 MB of staged LSP binaries rode along unused. The extension is now esbuild-bundled (`dist/extension.js`, minified, self-contained; `--no-dependencies` is thereby correct), with `.vscodeignore` trimming the package to 6 files (~85 KB + binaries), a committed `package-lock.json` (`npm ci` in CI), eslint + typecheck, and a new `extension` CI job (lint/typecheck/unit under Node, integration under xvfb against a debug LSP build). Two new packaging locks make the old failure unshippable: `vsix_contents.test.ts` asserts the bundle is in the package and nothing references `node_modules/`, and `version_lockstep.test.ts` pins the extension version to the workspace version.
- **Web bundle down 15.40 MB → 11.41 MB (−26%)** — `include_dir!` was embedding the entire `assets/` directory into the wasm, including 1.98 MB of images (the largest single data segment in the binary at 12.6%); it now embeds `assets/curriculum/` only, and the images deploy as static files. **This also fixes the live 404 of the social-share image**: every `og:image` pointed at `/assets/OG-photo.png`, which was inside the wasm rather than on the CDN — share cards never showed an image in production. The OG card itself was recompressed from a 2224×1180/1.4 MB PNG to a proper 1200×630/312 KB card (unfurlers time out on megabyte images), with `og:image:width/height/alt` now emitted. The dead CodeMirror CDN module (never called — the editors are styled textareas) is deleted from `index.html`, and KaTeX now lazy-loads on the first math render instead of on every page (which also fixes math renders being silently dropped when the CDN was slower than the first paint).
- **Live `<title>` concatenation** — the deployed site's title read "LOGICAFFEINE | Debug Your ThoughtsLOGOS Studio": dx splices the `Dioxus.toml` title into `index.html`'s existing `<title>`. The template now uses the `{app_title}` placeholder and per-page titles come from `PageHead`.
- **Stale SEO surfaces** — `public/sitemap.xml` listed 15 URLs (3 of ~33 news articles, no `/benchmarks`); it is now generated from `sitemap::prerender_routes()` (46 URLs) and held byte-equal to the generator by a test (`regenerate_shipped_sitemap` refreshes it). `llms.txt` claims updated to current reality (Dioxus 0.7, four Studio modes incl. Hardware, the Z3-free in-browser certified prover, the 2.544× C geomean headline, `/benchmarks` methodology section). `/success` is now `noindex`. Hashed bundle assets get `Cache-Control: immutable` via `_headers` (repeat visits stop re-downloading the wasm; rules deliberately match only the content-hashed filenames since Cloudflare Pages *joins* duplicate headers across matching rules).
- **`cert_farkas` kernel reconstruction on doubled constants** — the proof-producing arithmetic normalizer had no proof path for like monomials recombining to coefficient 1, and a merge that cancelled an entire prefix concluded with an unproven `add 0 x` residue the kernel rejected — so a correct Farkas refutation of `x+1 ≤ y ∧ y+1 ≤ x+1` failed to certify. Both holes close (locked by new `arith::tests`), and the long-ignored `probe_double_constant_le_via_auto` now runs un-ignored.

## [0.9.16] - 2026-04-06

### Added
- **IEEE 1800-2023 SVA upgrade** — 4 sprints (22-25) upgrading from IEEE 1800-2017 to 1800-2023 compliance. 3 new `SvaExpr` variants: `ArrayMap` (IEEE 7.12 array `.map()` with iterator/index arguments), `TypeThis` (IEEE 6.23 `type(this)` construct), `RealConst` (IEEE 5.7.2 real literals). `RandVarType` enum replacing `width: u32` on `RandVar` to support `rand real` / `rand const real` checker variables (IEEE 17.7). Triple-quoted string support in action blocks (IEEE 5.9). New system task recognition: `$timeunit`, `$timeprecision`, `$stacktrace` (IEEE 20.4.1, 20.17). 4-state operator truth table errata audit (IEEE 11.4).
- **`VerifyType::Real`** — Z3 Real sort threaded through the entire verification pipeline: `ir.rs`, `equivalence.rs`, `kinduction.rs`, `smtlib.rs`, `solver.rs`, `type_infer.rs`. `BoundedSort::Real` in SVA-to-verify translation for checker variable quantification.
- **95 new SVA tests** — `phase_hw_sva_2023.rs` (1,160 lines) covering all 4 IEEE 1800-2023 upgrade sprints: triple-quoted strings, new system tasks, array map methods, `type(this)`, `rand real` / `rand const real`, and semantic audit cross-feature composition.
- **Hardware parser gap fixes** — `shall` modal gate, `always`/`never` after copula, `after`/`when` subordinators, `and`-conjunction in conditionals, `bit` as noun, `request`/`grant` disambiguation, counting quantifiers, HAB SVA synthesis patterns.
- **SVA synthesizer coverage expansion** — new FOL-to-SVA synthesis patterns for counting quantifiers, HAB temporal patterns, and additional property templates.
- **Cirq v2 implementation plan** — complete rewrite of the quantum backend plan: 17 sprints, 314 tests planned across 13 test files, mirroring the proven SVA backend architecture.

### Changed
- **CI pipeline** — benchmark workflow no longer double-triggers deploy workflows. Benchmark result commits use `[skip ci]` to prevent cascading re-runs.

## [0.9.15] - 2026-04-04

### Added
- **IEEE 1800-2017 SVA expansion** — 40+ new `SvaExpr` variants covering property connectives (`not`, `implies`, `iff`), LTL temporal operators (`always`, `s_always`, `eventually`, `s_eventually`, `until`), sequence composition (`and`, `or`), abort operators (`accept_on`, `reject_on`, `sync_accept_on`, `sync_reject_on`), assertion directives, local variables, endpoint methods, bitwise/reduction operators, part selects, concatenation, complex data types, and let/checker declarations.
- **Vacuity analysis** — new `sva_vacuity.rs` module implementing IEEE 16.14.8 compliant vacuity checking with 33 rules for nonvacuous evaluation tracking, dead assertion detection, and coverage gap identification.
- **Bounded verification IR** — `SequenceMatch` struct with length-tracking for proper composition semantics, `BoundedExpr::Apply` for system function encoding, local variable bindings, and queue timestep support for `const'()` freezing.
- **System function support** — `$onehot0`, `$onehot`, `$countones`, `$isunknown`, `$sampled`, `$bits`, `$clog2`, `$countbits`, `$isunbounded` with proper bounded verification IR encoding.
- **Benchmark specifications** — engineering specs for FVEval NL2SVA (300 cases), VERT (20,000 cases), and AssertionBench (101 designs) targeting 100% IEEE 1800-2017 SVA coverage.
- **SVA coverage analysis** — gap analysis documenting 60 remaining features across 21 planned sprints with IEEE section references.
- **SVA test expansion** — new `phase_hw_sva_coverage.rs` (6,200+ lines) plus expansions to the IEEE 1800, roundtrip, surface, and translate test suites.

### Changed
- **SVA translation pipeline** — unified delay convention (`None` = unbounded `$`, `Some(n)` = bounded), proper `throughout`/`within` desugaring with length-matching semantics, and sequence instance resolution with parameter substitution.

## [0.9.14] - 2026-04-03

### Added
- **SVA synthesis codegen** — `codegen_sva/synthesize.rs` and `codegen_sva/z3_synth.rs`: Z3-guided synthesis of SVA properties with realizability checking.
- **Z3 equivalence test expansion** — 217 new lines of bitvector and semantic equivalence tests in `phase_hw_z3_equiv.rs`.

### Changed
- **Verification engine strengthening** — deeper passes across the model-checking stack introduced in 0.9.13 (≈5,500 lines): IC3/PDR generalization, k-induction with auxiliary-invariant strengthening, Craig interpolation, CEGAR predicate abstraction, liveness-to-safety reduction, multi-clock and compositional (assume-guarantee) reasoning, and certificate proof-witness embedding.
- **Solver infrastructure** — Z3 encoding with bitvector operation support and improved counterexample extraction.
- **Verification IR** — extended with node types for multi-clock and compositional reasoning.

## [0.9.13] - 2026-04-03

### Added
- **Hardware kernel types** — first-class `Bit`, `BitVec`, and `Circuit` types in the formal kernel prelude, enabling kernel-level hardware reasoning.
- **Bitvector decision procedures** — delta reduction support for bitvector operations in the kernel reduction engine.
- **Advanced model checking** — IC3, k-induction, and Craig interpolation strategies for unbounded property verification.
- **Compositional verification** — assume-guarantee reasoning with contract-based decomposition for modular hardware verification.
- **Clock domain crossing analysis** — CDC verification module detecting synchronization issues across clock boundaries.
- **Power isolation verification** — power domain analysis ensuring correct isolation and retention behavior.
- **Security property verification** — information flow and non-interference checking for hardware security properties.
- **Multi-clock verification** — formal reasoning about synchronization across multiple clock domains.
- **Synthesis oracle** — Z3-guided synthesis from formal properties to circuit implementations via the kernel type theory.
- **Test generation** — automatic test case generation from formal specifications.
- **Incremental verification** — delta-based re-verification avoiding redundant work when specs change.
- **Parameterized verification** — verification of parameterized designs across all instantiations.
- **Synthesis-compiler checks** — correctness checks for the SVA synthesis compiler (`verified_compiler`).
- **SMT-LIB2 dialect support** — direct SMT-LIB2 output for interoperability with external solvers.
- **Verilog extraction** — Verilog module extraction from kernel proof terms.
- **RISC-V protocol templates** — pre-verified SVA property templates for RISC-V bus protocols.
- **Liveness checking** — liveness property verification with fairness constraints and ranking functions.
- **Abstraction refinement** — CEGAR-style abstraction with automatic predicate discovery.
- **Certificate generation** — independently checkable verification certificates for proof portability.
- **Verification strategy selection** — automatic strategy selection based on property classification.
- **Type inference for properties** — automatic type inference for verification expressions.
- **Automata-based reasoning** — Buchi and omega-automata for temporal property verification.
- **27 new hardware-verification test files** — coverage across the new verification domains.

### Changed
- **Pricing removed** — replaced public pricing tiers with contact-based commercial licensing. Free for individuals, teams under 25, and education.
- **Verify crate publishing** — logicaffeine-verify now publishes to crates.io with fixed cascade.
- **Equivalence checking** — expanded semantic equivalence with new analysis passes.
- **Test documentation** — README expansion covering 60+ test files.

## [0.9.12] - 2026-04-02

### Added
- **FOL-to-SVA synthesis** — pattern-matching translation from first-order logic to SystemVerilog Assertions via Kripke-lowered structures, mapping quantified temporal patterns to `s_eventually`, `nexttime`, and temporal implications.
- **Coverage analysis** — signal, property, edge, and temporal coverage metrics measuring how well SVA properties cover the specification knowledge graph.
- **Sufficiency analysis** — pre-verification checks detecting lonely signals, unconstrained outputs, and missing handshake patterns.
- **Spec health checking** — self-consistency analysis of English specifications through the FOL-to-VerifyExpr-to-Z3 pipeline, detecting contradictions, vacuity, and redundancy.
- **Property decomposition** — hierarchical decomposition of conjunctive properties into independently verifiable sub-properties.
- **CEGAR synthesis refinement** — counterexample-guided refinement loop classifying SVA divergence as too-strong or too-weak with transformation strategies.
- **RTL extraction** — Verilog declaration parser extracting module structure (ports, signals, parameters, clock detection).
- **RTL knowledge graph** — converts RTL modules to hardware knowledge graphs and links spec KGs to RTL KGs via signal name matching.
- **Waveform rendering** — renders Z3 counterexamples to VCD (Value Change Dump) format for waveform viewer inspection.
- **Invariant discovery** — automatic candidate invariant generation from KG structure (mutex, handshake, pipeline patterns) with Z3 verification.
- **Protocol templates** — pre-verified parameterizable SVA properties for standard protocols (AXI4, APB, handshake) with English specification mappings.
- **Consistency checking** — multi-property consistency verification with minimal unsatisfiable subset extraction and vacuity/redundancy detection.
- **Hardware verification test suites** — 19 new test files covering coverage, decomposition, CEGAR, RTL extraction, RTL KG, protocols, spec health, sufficiency, synthesis refinement, waveform, invariants, consistency, signal bridge, SVA IEEE1800, ontology, and advanced/e2e Z3 verification.

### Changed
- **Hardware pipeline** — expanded end-to-end pipeline with coverage, sufficiency, decomposition, and CEGAR integration.
- **SVA model** — extended with coverage and decomposition support types.
- **FOL-to-verify translation** — mapping extended to support the new analysis modules.
- **SVA-to-verify pipeline** — translation with consistency and invariant-checking hooks.
- **Equivalence checking** — extended for consistency and cross-property analysis.
- **Knowledge graph** — expanded entity and relation extraction for RTL-spec linking.
- **Parser improvements** — expanded verb recognition and clause parsing for hardware specification sentences.
- **Lexicon expansion** — additional vocabulary for hardware verification domains.

## [0.9.11] - 2026-03-31

### Added
- **Bitvector theory** — `BitVector(n)` and `Array(idx, elem)` types in the verification IR, with full `BitVecOp` enum (bitwise, shift, arithmetic, comparison) and Z3 encoding support.
- **Bounded model checking** — `verify_temporal()` method for unrolling transition relations and checking temporal properties at each step with counterexample generation.
- **Equivalence checking** — new `equivalence.rs` module in `logicaffeine_verify` for structural and semantic equivalence of verification expressions.
- **SVA model expansion** — `Repetition`, `SAlways`, `Stable`, `Changed`, `DisableIff`, `Nexttime`, and `IfElse` expression variants in the SVA model, with full parsing and emission support.
- **SVA surface tests** — new `phase_hw_sva_surface.rs` test suite (606 lines) for SVA parsing and rendering coverage.
- **Z3 equivalence tests** — `phase_hw_z3_equiv.rs` expanded by 512 lines of bitvector and equivalence verification tests.
- **Knowledge graph expansion** — entity and relation extraction with 122 new lines in `knowledge_graph.rs`.
- **Kripke model updates** — extended Kripke structure support for temporal reasoning (48 new lines).

### Changed
- **Parser improvements** — expanded quantifier handling, clause parsing, and verb recognition for hardware specification sentences.
- **FOL-to-verify translation** — mapping from first-order logic to verification IR, supporting bitvector and array expressions.
- **SVA-to-verify pipeline** — translation from SVA model to verification IR with support for new temporal operators.
- **Verification solver** — extended Z3 encoding for bitvector operations, array select/store, and temporal unrolling.

## [0.9.10] - 2026-03-30

### Added
- **Hardware verification pipeline** — new `codegen_sva` module for generating SystemVerilog Assertions (SVA) from FOL specifications, with complete lexicon-to-assertion translation.
- **Knowledge graph semantics** — `knowledge_graph.rs` module for extracting structured knowledge graphs from parsed specifications, enabling semantic analysis and equivalence checking.
- **Hardware verification test suites** — 15 new test files (`phase_hw_*.rs`) covering SVA codegen, roundtrip, temporal logic, Z3 equivalence, Futamura projection application, and end-to-end pipeline testing.
- **PE infrastructure improvements** — expanded Futamura projection test suite from 513 to 543 tests with additional coverage.

### Changed
- **Proof engine** — updates to `oracle.rs` and `unify.rs` for improved reasoning.
- **Language infrastructure** — lexicon expansion, parser changes, and semantic analysis updates across the language crate.
- **LSP hover** — expanded hover information in `hover.rs`.

## [0.9.9] - 2026-03-29

### Added
- **PE Map/CCopy support** — all three PE variants (pe_source, pe_bti, pe_mini) now handle `VMap` in `exprToVal`/`valToExpr` and `CCopy` expression evaluation, enabling Map types and copy semantics through partial evaluation.
- **Expression embedding analysis** — new `exprEmbeds()` and `argsStrictlyEmbed()` predicates in pe_source and pe_bti for finer-grained memoization decisions during specialization.
- **Extended key generation** — pe_mini now generates unique memoization keys for 15 additional CExpr variants (CIndex, CLen, CMapGet, CNewSeq, CNewSet, CFieldAccess, CNew, CRange, CSlice, CCopy, CContains, CUnion, CIntersection, etc.).
- **Decompilation infrastructure** — `discover_entry_points()`, `fix_decompiled_types()`, and `generate_block_wrapper()` helpers in `logicaffeine-compile`'s `compile.rs` for automated PE code generation.
- **P3 surface-level language coverage** — 20 new tests verifying all language features through the Third Futamura Projection (structs, maps, seqs, options, variants, control flow, recursion, copy semantics).

### Changed
- **Repeat statement encoding** — simplified by extracting common logic into `encode_stmt_list_src`, removing 77 lines of explicit while-loop lowering.
- **Float literal formatting** — ensures float values always include a decimal point in encoded output.

### Verified
- **All 3 Futamura Projections** — 6,035 tests pass (512 Futamura-specific + full e2e suite), 0 failures, 0 regressions. Jones optimality confirmed. P1 specialization, P2 compiler generation, P3 compiler-generator generation all validated with cross-projection equivalence checks.

## [0.9.8] - 2026-03-19

### Fixed
- **FFI map keys/values type mismatch** — `logos_map_*_keys()` and `logos_map_*_values()` stored raw `Vec<T>` in the handle registry but downstream `logos_seq_*_len()`/`logos_seq_*_free()` cast them to `LogosSeq<T>` (now `Rc<RefCell<Vec<T>>>`). Fixed by wrapping in `LogosSeq::from_vec()` before registration.

## [0.9.7] - 2026-03-19

### Fixed
- **FFI serde for LogosSeq/LogosMap** — `Serialize`/`Deserialize` impls for `LogosSeq<T>` and `LogosMap<K, V>`, delegating through the `Rc<RefCell<…>>` wrappers to the inner `Vec`/`FxHashMap`. The FFI codegen emits `serde_json` round-trips on these types, but the wrappers had lacked serde impls since the reference-semantics change in 0.9.4.

## [0.9.6] - 2026-03-19

### Fixed
- **Studio file browser** — broken since 0.9.0. Dioxus 0.7 stopped serving `assets/` directly; moved `opfs-worker.js` and `style.css` to `public/assets/` so the OPFS web worker and stylesheet load correctly. Studio sidebar now shows files again.
- **CI deploy** — `robots.txt`, `sitemap.xml`, `_redirects` moved from `assets/` to `public/`, eliminating the manual copy step in `deploy-frontend.yml`.

### Added
- **Local Vec optimization** — escape analysis (`collect_escaping_collection_vars`) identifies collection variables that never leave the function boundary. These are stored as `Vec<T>` with zero-overhead indexing instead of `LogosSeq<T>` (Rc<RefCell<Vec<T>>>).
- **Borrow parameter optimization** — readonly `Seq<T>` parameters emit `&[T]` borrows; mutable-only parameters emit `&mut [T]`. Borrow types propagate through aliases.
- **Function return type tracking** — codegen context tracks function return types for call-site type inference.

### Changed
- **Peephole patterns** — slice/push/extend/drain patterns updated to handle both `Vec<T>` and `LogosSeq<T>` sources.
- **Test infrastructure** — offline-first `cargo build`/`cargo run` (tries `--offline` first, falls back to online). Increased `RUST_MIN_STACK` for deeply recursive AST walks.

## [0.9.5] - 2026-03-17

### Fixed
- **LogosMap/LogosSeq escape-block APIs** — `LogosMap::values()`/`keys()` returning owned `Vec`, and `LogosSeq: From<Vec<T>>`, for escape-block interop.
- **Showable blanket impl** — `&T: Showable` where `T: Showable`, fixing nested inspect arms with ref-bound pattern variables.
- **Escape block updates** — binary search, map access, and list construction escape tests updated for the `LogosSeq`/`LogosMap` APIs.

## [0.9.4] - 2026-03-16

### Added
- **PE BTI source** (`optimize/pe_bti_source.logos`, 1215 LOC) — binding-time improved partial evaluator source for Futamura Projection 2.
- **PE mini source** (`optimize/pe_mini_source.logos`, 785 LOC) — minimal clean-room partial evaluator for Futamura Projection 3.
- **Decompile source** (`optimize/decompile_source.logos`, 645 LOC) — source-level decompiler for PE output.
- **Reference semantics tests** (`e2e_ref_semantics.rs`, 191 lines) — end-to-end tests for Rc-based reference semantics.
- **~220 new partial-evaluation tests** — genuine self-application, Sprint I/J infrastructure, PE mini/BTI verification.

### Changed
- **Self-interpreter** (`compile.rs`) — expanded by 974 lines covering value handling, environment management, and PE integration.
- **PE source** (`pe_source.logos`) — expanded by 948 lines with specResults memoization, makeKey collision fixes, MSG wiring, BTA SCC support, and partially-static data handling.
- **Partial evaluator** (`partial_eval.rs`) — 264 lines of improvements: staticEnv copy propagation, CInspect field binding, mixed-arg CCall inlining, extractReturn sentinel fix.
- **Supercompiler** (`supercompile.rs`) — 81 lines of driving/generalization changes.
- **BTA** (`bta.rs`) — 61 lines: SCC wiring, isStatic expansion for CNew/CRange/CCopy.
- **Codegen peephole** (`peephole.rs`) — 152 lines of new optimization patterns.
- **Codegen stmt/expr** — 239 lines of emission improvements across `stmt.rs` and `expr.rs`.
- **Data types** (`types.rs`) — 260 lines: LogosSeq/LogosMap reference-semantic wrappers with interior mutability.
- **Indexing** (`indexing.rs`) — 58 lines: new indexing operations for reference-semantic collections.

### Fixed
- **Peephole mutability** — `vec_fill`, `vec_with_capacity`, sibling collection, and buffer reuse patterns now emit `let mut` when the LOGOS source declares `mutable`.
- **Conditional/unconditional swap** — LogosSeq swap codegen uses `__swap_tmp` inside `borrow_mut()` scope instead of `.swap()`, matching the codegen spec.

## [0.9.3] - 2026-03-06

### Added
- **Futamura projections** — all three projections implemented and verified with 276 tests (`phase_futamura.rs`):
  - **Projection 1**: `pe(interpreter, program) = compiled` — partial evaluation specializes the self-interpreter on a source program to produce a compiled executable.
  - **Projection 2**: `pe(pe, interpreter) = compiler` — partial evaluation applied to itself with the interpreter produces a standalone compiler.
  - **Projection 3**: `pe(pe, pe) = compiler_generator` — partial evaluation applied to itself twice produces a compiler generator.
- **Self-interpreter** — complete LOGOS interpreter written in LOGOS itself (`compile.rs`), supporting all core value types (int, float, text, bool, nothing, seq, map, set, error, crdt), arithmetic, comparisons, control flow, functions, recursion, and data structures.
- **Binding-time analysis** (`optimize/bta.rs`, 758 LOC) — classifies expressions as static or dynamic for partial evaluation, with 33 tests (`phase_bta.rs`).
- **Partial evaluator** (`optimize/partial_eval.rs`, 960 LOC) — online partial evaluation with environment-based specialization, function unfolding, and residual code generation, with 35 tests (`phase_partial_eval.rs`).
- **Supercompiler** (`optimize/supercompile.rs`, 878 LOC) — driving, generalization, and homeomorphic embedding for aggressive program specialization, with 59 tests (`phase_supercompile.rs`).
- **PE source language** (`optimize/pe_source.logos`, 556 LOC) — the partial evaluator's own source in LOGOS, enabling self-application.
- **Abstract interpretation** (`optimize/abstract_interp.rs`, 668 LOC) — forward abstract interpretation framework with interval and sign domains, with 16 tests (`phase_abstract_interp.rs`).
- **Deforestation** (`optimize/deforest.rs`, 539 LOC) — eliminates intermediate data structures in compositions, with 8 tests (`phase_deforestation.rs`).
- **Effect analysis** (`optimize/effects.rs`, 698 LOC) — tracks computational effects (pure, IO, mutation, divergence) for optimization safety, with 19 tests (`phase_effects.rs`).
- **Global value numbering** (`optimize/gvn.rs`, 483 LOC) — hash-consing based redundant computation elimination.
- **Loop-invariant code motion** (`optimize/licm.rs`, 391 LOC) — hoists loop-invariant expressions out of loops.
- **Compile-time function evaluation** (`optimize/ctfe.rs`, 443 LOC) — evaluates pure functions at compile time.
- **Closed-form optimization** (`optimize/closed_form.rs`, 354 LOC) — replaces simple loops with closed-form expressions.
- **Polyhedral optimization** tests (`phase_polyhedral.rs`) — 6 tests for polyhedral loop transformations.
- **Auto-parallelization** tests (`phase_autoparallel.rs`) — 6 tests for automatic parallelization detection.
- **Mountain climb** tests (`phase_mountain_climb.rs`) — 27 tests for optimization composition.
- **~550 new tests** across 10 new test files and expanded existing test files.

### Changed
- **Optimizer pipeline** — extended with constant propagation, expanded DCE, and new optimization passes. Pipeline: fold → propagate → dce with expanded pattern coverage.
- **Codegen peephole optimizations** — extended with new patterns including `(j+1)-1 → j` index simplification.
- **DCE** — expanded from basic dead-code elimination to include unused variable removal and dead-store elimination (282 lines added).
- **Constant folding** — extended with additional algebraic simplifications (90 lines added).
- **Constant propagation** — added safety guards for index/slice contexts (49 lines added).

### Fixed
- **Self-interpreter error casing** — `valToText` returned `"error: {msg}"` (lowercase) instead of `"Error: {msg}"` (capital E).
- **PE substring false positives** — renamed `CEscapeExpr`/`CEscapeStmt` to `CEscExpr`/`CEscStmt` to eliminate false substring matches with `peExpr`/`peStmt` in Futamura projection tests.
- **Codegen boilerplate false positives** — pre-computed stack size literal (`67_108_864`) and used function pointer (`_logos_main`) to avoid `* 1` and `||` matches in optimizer test assertions.
- **FFI snapshot refresh** — regenerated 5 FFI codegen snapshots to match current output.

## [0.9.2] - 2026-02-28

### Fixed
- **Benchmark timeout isolation** — each language now runs in its own hyperfine invocation with an independent timeout. Previously, all 11 languages ran in a single hyperfine call; when one language (e.g., Python on ackermann n=11) exceeded the timeout, `run_timeout` killed the entire process and remaining languages (JS, Ruby, Nim, LOGOS) were never measured. Per-language results are merged into the same JSON format so downstream assembly is unchanged.
- **Smart timeout skipping** — if a language times out at size N, it is automatically skipped for all larger sizes of that benchmark, avoiding wasted CI minutes on languages known to be too slow.
- **Trimmed benchmark sizes** — removed noise sizes from all 32 `sizes.txt` files where C runtime is <10ms (process startup dominates measurement). Keeps 3-5 meaningful sizes per benchmark for real scaling data.
- Applied to all three benchmark scripts: `run.sh`, `run-quick.sh`, `run-logos-vs-c.sh`.

## [0.9.1] - 2026-02-28

### Fixed
- **Benchmark CI completely broken** — all hyperfine calls used `--timeout` flag which doesn't exist in any version of hyperfine, causing every benchmark to fail with zero data and empty geometric mean results. Removed the invalid flag; the `run_timeout` wrapper already handles timeouts via the system `timeout` command.

## [0.9.0] - 2026-02-27

### Added
- **Bidirectional type checker** — Robinson unification-based inference (`analysis/check.rs`, `analysis/unify.rs`). Eliminates `Unknown` types for field access, empty collections, option literals, pipe receives, inspect arm bindings, and closure calls.
- **Call graph analysis** (`analysis/callgraph.rs`) — whole-program call graph with Kosaraju SCC detection for readonly and purity analysis.
- **Liveness analysis** (`analysis/liveness.rs`) — backward dataflow computing per-statement live-after sets, enabling last-use move optimization.
- **Read-only parameter inference** (`analysis/readonly.rs`) — fixed-point iteration over the call graph identifies `Seq<T>` parameters never mutated, emitting `&[T]` borrows instead of clones.
- **Bitwise operators** — `x xor y`, `x shifted left by y`, `x shifted right by y`.
- **Break statement** — `Break.` exits the innermost while loop.
- **Unary NOT** — `not x` for logical and bitwise negation.
- **Generic function type parameters** — polymorphic type variable declarations on functions.
- **Triple-quote strings** — `"""multi-line"""` with automatic indentation stripping.
- **Scientific notation** — `4.84e+00`, `2.5e-2` in numeric literals.
- **io_uring VFS** (`logicaffeine_system`) — Linux kernel-async file I/O via dedicated worker thread.
- **~392 new tests** across 7 new test files and expanded suites covering type checker, bitwise ops, break, codegen optimization, math builtins, string interpolation, and optimizer features.

### Changed
- **Codegen architecture** — monolithic `codegen.rs` (8,300 lines) split into 13 modules (`context`, `detection`, `expr`, `stmt`, `peephole`, `program`, `ffi`, `marshal`, `policy`, `bindings`, `tce`, `types`). Public API preserved via re-exports.
- **C backend architecture** — `codegen_c.rs` (2,000 lines) split into 4 modules (`emit`, `runtime`, `types`).
- **Compilation pipeline** — `check_program()` now runs between analysis and codegen, producing a `TypeEnv` for optimization decisions.
- **FxHashMap/FxHashSet** — generated code uses `rustc-hash` for faster integer-key hashing.
- **15 codegen optimizations** — last-use clone elimination, liveness-based move, sentinel exit detection, dead post-loop counter elimination, HashMap `.get()` for comparisons, string byte comparison, self-append via `write!`, flattened string concatenation, read-only `&[T]` borrows, `Vec::with_capacity`, `assert_unchecked` for proven bounds, raised inline threshold, power-of-2 modulo strength reduction, `target-cpu=native`.
- **Benchmark suite** — expanded from ~6 to 30+ programs with multi-language implementations and correctness verification.
- **C backend** — extended with interpolated strings, bitwise ops, shifts, enums, slices, sets, and multiple map type variants.

## [0.8.19] - 2026-02-15

### Added
- **Swap pattern regression test** — codegen and E2E tests verifying `.swap()` fires for the bubble_sort benchmark pattern (nested while loops, inferred Vec type via `new Seq of Int`).

### Changed
- **Benchmark warm-ups and runs increased** — runtime benchmarks now use 5 warm-ups and 20 runs (up from 3/10) to reduce variance between versions.
- **Zig 0.15 upgrade** — all 6 Zig benchmark programs updated from Zig 0.13 to 0.15 API (`std.io.getStdOut()` → `std.fs.File.stdout().writer(&buf)`, ArrayList now unmanaged). CI updated from Zig 0.13.0 to 0.15.2.

## [0.8.18] - 2026-02-15

### Fixed
- **Constant propagation string safety** — propagation no longer substitutes `Literal::Text` (String) values. String is non-Copy in Rust, so substituting string literals into multiple use sites created independent `String::from(...)` allocations, hiding use-after-move errors (E0382) from rustc.
- **Constant propagation zone scoping** — zone-scoped `Let` bindings are no longer registered in the propagation environment. Previously, propagating a zone-scoped variable outward replaced `Expr::Identifier` with `Expr::Literal`, which the escape checker treated as safe — hiding zone escape violations (E0597).

## [0.8.17] - 2026-02-15

### Added
- **C codegen backend** — `compile_to_c()` produces self-contained C files with embedded runtime (Seq, Map, Set, string helpers, IO). Compiles with `gcc -O2`. Supports integers, floats, booleans, strings, collections, control flow, functions.
- **Constant propagation** optimizer pass — forward substitution of immutable constants, chained with fold and DCE. Safety: skips Index/Slice expressions to preserve swap/vec-fill pattern detection.
- **638 new E2E tests** across 21 new test files:
  - 16 Rust codegen mirror files (181 tests) — every interpreter-only feature now also tested through the Rust codegen pipeline
  - `e2e_codegen_gaps.rs` (64 tests) — floats, modulo, options, nothing, collection type combos, struct/enum patterns, control flow, functions, escape blocks, strings
  - `e2e_codegen_optimization.rs` (15 tests) — TCO, constant propagation, DCE, vec-fill, swap, fold, index simplification
  - `e2e_interpreter_gaps.rs` (60 tests) — interpreter counterparts for gap coverage
  - `e2e_interpreter_optimization.rs` (14 tests) — interpreter counterparts for optimization correctness
  - `phase_codegen_c.rs` (214 tests) — C-backend codegen coverage

### Fixed
- **For-range guard for complex expressions** — `While i is at most length of items` no longer produces `_` in generated Rust. Added `is_simple_expr()` guard.
- **For-range post-loop value for empty loops** — empty loops now correctly keep the counter at its start value using `max(start, limit)`.
- **Vec-fill pattern relaxed mutability** — `Let items be a new Seq of Bool` (without explicit `mutable`) now matches the vec-fill optimization.
- **C codegen missing Set variants** — `SetI64` and `SetStr` added to `c_type_str()`.
- **Interpreter float comparison** — `apply_comparison` now handles Float-Float, Int-Float, and Float-Int comparisons.

## [0.8.16] - 2026-02-15

### Fixed
- **For-range loop regression**: `RangeInclusive` (`..=`) has a known per-iteration overhead in Rust due to internal bookkeeping for edge cases. In O(n^2) inner loops like bubble sort, this compounded to a 41.4% regression vs the `while` loop it replaced. All inclusive ranges now emit exclusive form: `for i in 1..=n` becomes `for i in 1..(n + 1)`. For literal limits, the addition is computed at compile time (e.g. `for i in 1..6` instead of `for i in 1..=5`).

## [0.8.15] - 2026-02-15

### Added
- **TIER 1 codegen optimizations** — five peephole-level improvements targeting array-heavy benchmark performance
  - **For-range loop emission**: `Let i be 1. While i <= n: ... Set i to i + 1` compiles to `for i in 1..(n + 1)` instead of `while (i <= n)`, enabling LLVM trip count recognition, unrolling, and vectorization
  - **Iterator-based loops**: `Repeat for x in items` emits `.iter().copied()` instead of `.clone()` for Copy-type collections (`Vec<i64>`, `Vec<f64>`, `Vec<bool>`), eliminating full-collection copies
  - **Direct array indexing for list literals**: `Let items be [10, 20, 30]` now registers element type, enabling direct `arr[(idx-1) as usize]` instead of `LogosIndex` trait dispatch
  - **Vec fill exclusive bound**: `While i < n: Push 0 to items` now optimizes to `vec![0; n]` (previously only `<=` was matched)
  - **Swap pattern equality comparisons**: `If a equals b: swap` and `If a is not b: swap` now optimize to `arr.swap()`
- 24 new optimizer tests across all 5 TIER 1 items (codegen assertions + E2E correctness)

## [0.8.14] - 2026-02-15

### Added
- **TIER 0 optimizer bedrock** — deep expression recursion, unreachable-after-return DCE, algebraic simplification
  - Constant folding now recurses into all 26 Expr variants (function call args, list literals, index expressions, struct constructors, Option/Maybe Some, Contains, Closure bodies)
  - Statements after `Return` are eliminated as dead code
  - Algebraic identity/annihilator rules for both int and float: `x + 0 → x`, `x * 1 → x`, `x * 0 → 0`, `x - 0 → x`, `x / 1 → x`
- **Maybe syntax** — `Maybe Int` and `Maybe of Int` as dual syntax for `Option of Int`
  - Supports direct Haskell-style `Maybe T` (no "of" required) and consistent `Maybe of T`
  - Works in all positions: variable annotations, return types, nested generics
- 33 new optimizer and type expression tests

### Fixed
- Publish workflow: skip already-published crates instead of failing

## [0.8.13] - 2026-02-14

### Added
- **Accumulator introduction** — converts `f(n-1) + k` and `n * f(n-1)` into zero-overhead loops
- Automatic memoization: pure multi-branch recursive functions get thread-local HashMap cache
- Mutual tail call optimization: pairs like `isEven`/`isOdd` merged into single loop with tag dispatch
- Purity analysis: fixed-point dataflow to identify side-effect-free functions
- 14 new optimizer tests (accumulator, memoization, mutual TCO — codegen + E2E correctness)

## [0.8.12] - 2026-02-14

### Added
- **Closures and interpreter expansion** — closure support (`e2e_closures.rs` and parser changes) and a buildout of the tree-walking interpreter (`interpreter.rs`), alongside the initial bytecode VM design plan (`VM_PLAN.md`).

## [0.8.11] - 2026-02-14

### Added
- **Peephole vec-fill pattern** — `vec![val; count]` instead of push loop
- Peephole optimizer: swap pattern (`arr.swap()` instead of temp variable assignments)
- Copy-type elision: skip `.clone()` on Vec/HashMap indexing for primitive types
- HashMap equality optimization: `map.get()` instead of `map[key].clone()` for comparisons

### Changed
- Release profile: `opt-level = 3`, `codegen-units = 1`, `panic = "abort"`, `strip = true`
- `#[inline]` on Value arithmetic, LogosDate/LogosMoment accessors, parseInt/parseFloat
- Variable type tracking threaded through all expression codegen paths

## [0.8.10] - 2026-02-14

### Changed
- **Direct collection indexing** — codegen for known Vec/HashMap types avoids trait dispatch
- `#[inline(always)]` on all Showable, LogosContains, LogosIndex trait impls
- `get_unchecked` after validated bounds in Vec indexing (removes redundant bounds check)
- LTO enabled in release profile for generated projects

## [0.8.9] - 2026-02-14

### Fixed
- **Benchmark CI `actions: write` permission** — granted the benchmark workflow `actions: write` so it can trigger the frontend deploy.

## [0.8.8] - 2026-02-14

### Fixed
- **Benchmark CI dirty-checkout** — `latest.json` is saved to `/tmp` before `git checkout main`, fixing the checkout failure when the working tree held uncommitted benchmark results.

## [0.8.7] - 2026-02-14

### Fixed
- **Frontend deploy ordering** — the deploy triggers after benchmarks commit fresh results, and the deploy checkout always uses the latest `main` HEAD (`deploy-frontend.yml`).
- **Benchmark size and stability** — dropped the two largest sizes per benchmark, fixed a JS ackermann stack overflow, and worked around a `jq` "argument list too long" via a temp file.

## [0.8.6] - 2026-02-13

### Fixed
- **FFI test CI fixes** — refreshed the FFI codegen snapshots, fixed the Zig benchmark builds, and fixed a Ruby stack overflow in the benchmark suite.

## [0.8.5] - 2026-02-13

### Fixed
- **CI mold linker config** — removed the mold linker setting from `.cargo/config.toml`, which broke CI, and began tracking benchmark `latest.json` in-repo so the frontend bakes current results.

## [0.8.4] - 2026-02-13

### Added
- **Multi-language benchmark suite** — benchmark programs in 10 languages (C, C++, Rust, Go, Zig, Nim, Python, Ruby, JS, Java) with `benchmarks/run.sh`, a benchmark CI workflow, and the interactive benchmarks web page.
- **Interpreter map keys** — map key support in the tree-walking interpreter (`phase_interpreter_map_keys.rs`).
- **Standard library and interpreter-benchmark coverage** — `phase38_stdlib.rs` and `phase_benchmark_interp.rs`, plus an expanded `e2e_feature_matrix.rs`.
- **nextest configuration** — `cargo-nextest` runner config for the test suite.

## [0.8.3] - 2026-02-12

### Fixed
- **FFI C-linkage test gating** — gated behind the `ffi-link-tests` feature for CI compatibility
- Platform-aware linker flags for C ABI tests (macOS + Linux)

## [0.8.2] - 2026-02-12

### Added

- **Optimizer infrastructure** — constant folding and dead code elimination
- Interpreter mode (`largo run --interpret`) for sub-second development feedback
- Map insertion syntax (`Set X at KEY to VALUE`)

### Changed

- FFI/C export safety: thread-local error cache, panic boundaries, null handle checks, dynamic `logos_version()`
- `LogosHandle` from `*const c_void` to `*mut c_void`
- Text/String excluded from C ABI value types

### Fixed

- UTF-8 string indexing in interpreter (`.len()` → `.chars().count()`)
- Rust keyword escaping in generated FFI code

## [0.8.1] - 2026-02-12

### Changed

- **Version bump** — republished the workspace at 0.8.1; no functional change (the `@types/node` VSCode devDependency fix landed under the 0.8.0 tag).

## [0.8.0] - 2026-02-10

### Added

- **LSP server** — definitions, references, hover, completions, rename, semantic tokens, diagnostics
- VSCode extension with bundled LSP binaries for 5 platforms
- FFI/C export system for cross-language interop
- CI/CD workflows for release, publish, and deployment

### Changed

- Compiler improvements and bug fixes

## [0.7.0] - 2026-02-01

### Added

- **E2E test suite expansion** — temporal, escape-hatch, FFI, and concurrency test coverage.

### Changed

- **Concurrency and async hardening** — expanded and fixed the concurrency/async runtime; the escape-analysis and concurrency modules predate this release.
- **Web platform** — studio IDE, mobile responsiveness, homepage redesign.

### Fixed

- 10 compiler bugs across codegen, parser, and map handling.
- Async and concurrency correctness issues.

## [0.6.0] - 2026-01-17

Initial crates.io release with lockstep versioning.

### Changed

- Synchronized all crate versions to 0.6.0 (lockstep versioning)
- Added CHANGELOG.md to every crate
- Added VERSIONING.md with release process documentation

## [0.5.5] - 2026-01-01

First public release.

### Added

**Compiler**
- Z3 SMT solver integration for static verification (`logos_verification` crate)
- Refinement type syntax with `where` clauses
- DRS (Discourse Representation Structures) for donkey anaphora
- Event adjective analysis ("Olga is a beautiful dancer")
- Escape analysis for memory safety
- Diagnostic system with source mapping

**Runtime (`logos_core`)**
- Standard library: `env`, `file`, `random`, `time` modules
- CRDT support: GCounter, LWW registers
- Memory zones for region-based allocation

**Tooling**
- CLI tool (`largo`) for project management
- Package registry with publish/download
- GitHub Actions for CI/CD deployment
- Rust code formatter for generated output

**Web Platform**
- Learning platform with interactive curriculum
- Vocabulary reference component
- User profile page
- Universal navigation

**Tests**
- End-to-end test suite (collections, functions, structs, enums, etc.)
- Phase 41: Event adjectives
- Phase 42: DRS
- Phase 50: Security policy analysis
- Phase 85: Memory zones
- Grand challenge: Mergesort example

### Core Features (v0.5.5)

**Logic Mode** - English → First-Order Logic
- Quantifiers: universal, existential, negative, cardinal
- Modal operators: necessity, possibility, deontic
- Temporal logic: tense, aspect
- Wh-questions, relative clauses, reflexives, reciprocals
- Scope ambiguity resolution
- Parse forests for structural ambiguity

**Imperative Mode** - English → Rust
- Variables, mutation, control flow
- Functions with typed parameters
- Structs and enums with pattern matching
- Collections with 1-based indexing
- Generics (`Seq of Int`, `Box of [T]`)
