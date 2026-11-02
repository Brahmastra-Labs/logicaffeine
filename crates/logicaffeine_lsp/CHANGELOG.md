# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Added

- A `server` feature, on by default, holding the Language Server Protocol transport (tower-lsp, tokio, env_logger). Without it the crate is the analysis alone — hover, diagnostics, completion and the rest — and builds for wasm32, which the browser Studio links directly. `workspace::url_path` / `workspace::path_url` convert between `file://` URLs and paths, and answer `None` where there is no filesystem.

- **The editor understands an LGX page.**
  - **Symbol index:** components, pages, their state fields, props and actions enter it (`DefinitionKind::{Component, StateField, Action}`, `OwnedStmt::ComponentDef`) with the spans the parser recorded, and the names a `View:` mentions resolve as references once every definition exists.
  - **Navigation:** go-to-definition, hover, rename and find-references work inside a template through one cursor rule, `document::name_at`. Workspace symbols list components as classes, actions as methods and state fields as properties.
  - **Outline:** a route is named by method and path (`GET /api/todos`), a page by its path, a redirect as `from -> to`, an error page by its status, and components, stores and directives by name, each with its own symbol kind.
  - **Completion:** inside `View:` and `Head:` it offers the page's own state, props, actions, components and the template tags (`template::TAGS`) instead of the whole prelude; inside `Style:` it offers the classes, ids and elements the same block's view renders, plus `:global(…)`.
  - **Painting:** templates paint as HTML with LOGOS in the braces. The semantic-token legend gains `tag` and `attribute` (13 → 15 types, append-only); component elements paint as types and their attributes as parameters, `on-…` directives as keywords with `.modifier`s as modifiers, and holes and filters resolve through the index. A `Style:` body is left to the CSS grammar.
  - **Diagnostics:** template lints are published at their element as `lgx-<kind>` (`lgx-image-without-alt`, `lgx-form-without-csrf`); a construct the browser build refuses is an ERROR coded `browser-tier-refusal` at that construct; escape and ownership findings on names the LGX lowering mints are dropped.
  - Locks: `lgx_editor_lock`, `lgx_lint_span_lock`, `compiler_written_names_lock`, `diagnostic_location_editor_lock`.
- **Cross-module editing.** For programs that `Import` their own modules:
  - **Quickfixes:** an unresolved name a project module exports gets an `Import <Module>.` quickfix, one per exporting module, and an unreferenced import gets "Remove unused import".
  - **Completion:** after `Import `, completion offers module names; in code it offers exactly the names the file's imports bring into scope, labelled `from <Module>`, so a selective `Import double from Util.` offers only `double`.
  - **Hover:** names the module that defines a cross-file symbol.
  - **`private` definitions:** the marker is blanked before analysis, so `## To private f` is indexed as `f`, and a private definition stays out of workspace exports, completion and auto-import.
  - Tests: `crates/logicaffeine_tests/unit/logicaffeine_lsp/code_actions/tests.rs`, `crates/logicaffeine_tests/unit/logicaffeine_lsp/completion/tests.rs`, `crates/logicaffeine_tests/unit/logicaffeine_lsp/hover/tests.rs`.
- **The value-semantics copy lint is a diagnostic.** `logicaffeine_language::lint`'s finding is published as a WARNING coded `copy-orphaned-copy`.

### Breaking changes
- **The library API changed.** `index::DefinitionKind` gained `Component`, `StateField` and `Action`, and `pipeline::OwnedStmt` gained `ComponentDef`; neither enum is `#[non_exhaustive]`. `pipeline::AnalysisResult` gained `lgx_findings`, `copy_findings` and `refusals`, `stdlib_docs::StdlibDoc` gained `details`, and `Flycheck::clear` returns `bool`. Migration: add the match arms and struct fields; `Flycheck::clear`'s result says whether anything was dropped.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.
- A hover no longer repeats the diagnostics at its position: the editor shows the problem first in its own hover, above the word's lesson (`a_hover_leaves_problems_to_the_editor`).

- **The server's diagnostic codes are the CLI's.** Every code, severity, quickguide anchor and quickfix note is locked equal to `logicaffeine_language::diagnostic`, the table `largo check --json` and `largo mcp` report from. `locks::lsp_codes_agree_with_canonical_table` in `crates/logicaffeine_tests/tests/locks.rs`.
- **Stdlib hover carries the whole note.** `StdlibDoc::details` renders the rest of a `## Note` under its summary, and an enum variant (`Ok`, `Err`) hovers with its own line and its enum's note. `stdlib_teach_lock`.
- **The published crate carries only its source.** An anchored `include` allowlist ships `src/`, the README and the CHANGELOG; the end-to-end, golden and ratchet suites moved to `crates/logicaffeine_tests/tests/` and the unit-test bodies to `crates/logicaffeine_tests/unit/logicaffeine_lsp/`, so no test code ships. The released `logicaffeine-lsp` binary is built with the new `[profile.dist]` (whole-program LTO, one codegen unit, stripped symbols, panics still unwind): 16.3 MB, down from 25.5 MB on x86_64 Linux, so a backtrace from it is unsymbolized. The declared `rust-version` is 1.88 (0.10.1 declared 1.75).

### Removed
- **The unused `logicaffeine-proof` dependency.** A proof block's code lens issues `logicaffeine.checkProof`, which the editor extension runs through `largo`.

### Fixed

- The editor and the compiler reach the same ownership and escape verdict (`the_editor_and_the_compiler_agree_on_ownership.rs`).
  - A value given in one branch and shown in the other is no longer underlined.
  - A zone escape or a possible move inside a `During` span is.
  - A value a loop may have given away is described as given away "in a pass through the loop" in the maybe-move finding, the hover and the inlay hint.
- A name read inside interpolated text (`"{total}"`) counts as a use (`index::index_interpolation_reads`), so it is no longer reported as never used.
- A problem underlines the statement at fault: findings resolve their statement trail through the parser's nested spans (`pipeline::statement_span`), instead of anchoring to line 1 or a lone full stop.

- **Semantic tokens stay protocol-legal.** A multi-line `"""…"""` string is emitted one token per line, spans are sorted, and deltas saturate; a token crossing a newline desynchronized every later token. `crates/logicaffeine_tests/tests/semantic_tokens_e2e.rs`.
- **Position math is total.** An offset inside a multi-byte character snaps to its boundary instead of panicking, and a column past a line's end clamps to that line.
- **Stale rustc findings retract on edit.** An edit that clears flycheck findings republishes the interactive diagnostics at once; they used to linger at pre-edit positions until the debounced analysis.
- **An empty document gets keyword completions.** A cursor before the first token got none.

## [0.10.0] - 2026-07-08

This per-crate changelog begins here; the crate has shipped since 0.8.0 (see the root CHANGELOG and news for earlier history).

### Changed
- The formatting provider delegates to `logicaffeine_language::source_format::format_line` — the same canonical rules `largo fmt` applies — instead of carrying its own copy.
- Signature-help trigger characters are `␠` (space) and `,` — the previous `with` trigger was multi-character, which LSP clients never send, so it could not fire.
- **Async architecture.** Documents are now immutable snapshots (`DashMap<Url, Arc<DocumentState>>`) — handlers clone the `Arc` and drop the map guard immediately, which makes holding a guard across an `.await` (a latent same-shard deadlock in the previous `publish_diagnostics`) structurally impossible. Text sync switched from FULL to **INCREMENTAL**: a new `scheduler` module applies UTF-16 range edits to the live text (`document::apply_content_change`, surrogate-pair and CRLF correct) and debounces analysis ~150 ms behind a per-document generation counter, so a typing burst coalesces into one pipeline pass over the final text instead of one per keystroke — analysis itself moved off the async runtime onto `spawn_blocking`, and stale results are dropped by the generation guard. Pinned by e2e tests: a 51-edit burst publishes a handful of diagnostics sets ending at the final version, never 51.

### Fixed
- Formatting a file whose final line lacks a trailing newline produced an edit range one byte short — duplicating the last character on apply, and panicking on a multibyte final character. The final line's range now ends exactly at EOF.

### Added
- Language Server Protocol implementation for `.logos` files (`tower-lsp` + `tokio`), with full document re-analysis on change and a per-document symbol index.
- 14 capabilities: diagnostics, hover, semantic tokens, go-to-definition, completion, signature help, code actions, rename, folding ranges, inlay hints, code lens, and formatting. See the root CHANGELOG for cross-crate context.

### Added (the socratic teaching layer)
- **One teaching brain.** Hover, completion documentation, and the REPL's new `:explain` all derive from `logicaffeine_language::teach` — a lesson table whose TYPE requires a plain one-sentence `what`, a runnable example, and a socratic question or tip for every entry. Hover's hand-written keyword strings and completion's separately-worded details are deleted; `teach_md` is the single markdown renderer, so the hover card and the completion docs panel can never phrase a lesson differently. Ten newly-taught keywords (Pop, Add, Remove, Break, Assert, Trust, Increase, Decrease, Spawn, Merge) get hovers that never existed.
- **Socratic explanations v2.** All 39 `ParseErrorKind` explanations rewritten to the contract *what happened → why → a guiding question → the concrete next step*, committed as a reviewable golden (`tests/goldens/socratic_explanations.txt`) with a mechanical teaches-or-exempted lock. Token names humanize through `describe_token` ("a comma (',')", never `Comma`), and byte offsets are gone from the prose — the span already carries the location (`explanations_never_recite_byte_positions`).
- **Docs links on diagnostics.** `decision_for` grew a compile-forced fourth decision: every error kind chooses a LOGOS_QUICKGUIDE.md anchor (`codeDescription` opens the lesson) or records why not; anchors verify against the live guide's headings. Analysis-path codes (`use-after-move`, escapes, `unused-variable`) link too.
- **Literate docs live.** A `## Note` block directly above a `## To`/`## A` definition becomes that definition's documentation — in hover, completion docs, and signature help (which finally teaches: `SignatureInformation.documentation` from the same prose). Every one of the 158 stdlib prelude definitions got an authored `## Note` (`stdlib_docs` registry; `md5` in the editor teaches exactly what its source says), ratcheted by `stdlib_teach_lock` — and the runtime prelude strips Notes, byte-identical to the pre-documentation join.
- **The teaching-corpus golden** (`tests/goldens/teaching_corpus.md`): hover markdown verbatim, completion teaching text, signature-help documentation, lens titles, quickfix titles, and docs links over one representative program — every wording change on the teaching surface is a reviewable diff. Parity ratchets close the loop: every quickguide construct row maps to a lesson or records why prose suffices (both directions), and every quickfix the decision table promises must be produced by a real broken program (`quickfix_parity` — which found and now permanently documents that `undefined-variable` has NO producing program yet: typo'd names pass silently, recorded in `NOT_YET_PRODUCIBLE` until unresolved-reference detection lands).

### Added (SOTA completions)
- **Cross-file references and rename.** The workspace index now carries a per-file reference map; references/rename merge the current document, other OPEN buffers (live text always wins over the disk index), and unopened files. Local `Let`s stay private — only API-shape symbols cross files.
- **A warnings layer with a locked severity contract.** `shadowed-variable` (WARNING — a second `Let x` in the same block, with related-info pointing at the first; each re-`Let` now anchors its own binding token) and `unused-function` (HINT + faded, only in programs WITH a `## Main` — a library's functions are its API). `ANALYSIS_DECISIONS` records code→severity→reason, locked both directions by a battery of producing programs.
- **Interpolation interiors paint as code.** `"Hello {name}!"` renders the string segments as string, the braces as operators, and `name` exactly as it paints outside the string (re-lexed with the real lexer, resolved through the document's index; `{{` escapes stay string).
- **Range formatting** — the whole-document structural format filtered to intersecting lines (sound because `format_source` is line-count preserving); the capability-lock exemption retired through its own sanctioned path.

### Fixed (found by the new ratchets, live)
- The sentence-level parse recovery DEGRADED specific teachings to generic ones: the full parse diagnosed `IsValueEquality` for `x is 5.`, the per-sentence reparse replaced it with `ExpectedStatement`. The full parse's same-line error now wins. En route, the imperative statement dispatch learned to diagnose `x is <literal>.` itself, spanning the whole comparison.
- The use-after-move quickfix could read stray punctuation out of the diagnostic range: it offered — and would have INSERTED — `a copy of .`. The fix now names the variable from the message and retargets the edit to the variable's own token (double-move too).
- Spelling suggestions fired on coded diagnostics and one-character words: a moved `x` drew "Did you mean 'a'?". Both gated off.
- Function definitions recovered from tokens (any document with a theorem block takes this path) dropped their parenthesized parameters — `To double() -> Int` instead of `To double(n: Int) -> Int` in hover, completion, and signature help. The extraction now reads `(n: Int)` groups like the prose `with` form, and `extract_params_from_signature` walks every prepositional group.
- Repeated `Let x` definitions all anchored on the FIRST occurrence's span; each now claims its own binding token.
- End-to-end test harness (`tests/harness/`): drives the real `tower_lsp::Server` loop over in-memory duplex pipes at the wire level (`Content-Length` framing, request/response correlation, server-initiated `publishDiagnostics`), so protocol behavior is tested exactly as an editor sees it.
- Ratchet locks (`tests/locks.rs`): every `TokenType` variant must classify inside the advertised semantic-token legend (wildcard-free guard breaks the build on new variants); every `ParseErrorKind` must pass the decision table; the advertised capability set is pinned in both directions with a reason recorded for each capability deliberately not offered; the legend is locked append-only.
- `diagnostics::decision_for` — the total severity/code/quickfix decision table every `ParseErrorKind` flows through. A new error kind does not compile until it explicitly decides its severity, its stable kebab-case diagnostic code, and whether a mechanical quickfix is safe (declining records the reason).
- Parse-path `use-after-move` diagnostics now carry `DiagnosticRelatedInformation` pointing at the `Give` statement that moved the value, matching the ownership-checker path.
- `semantic_tokens::classify_token` is public: the classifier is now directly testable and reusable.

### Added (highlighting)
- **Resolution-aware semantic tokens.** On top of the part-of-speech base layer, a `SymbolIndex` overlay now classifies identifiers by what they resolve to — parameter, function, type, field, variant, or variable — at the declaration and at every reference. Modifiers carry the facts: `declaration` fires only where the token IS the definition (references stop being painted as declarations, fixing the old always-on `ProperName` behavior), `readonly` marks immutable `Let`s, the new `modification` bit marks write sites (`Set` targets incl. keyed/indexed forms, `Increase`/`Decrease`, `Push`/`Add`/`Append … to`, `Remove`/`Pop … from`), and the new `defaultLibrary` bit marks stdlib prelude names (`md5`, `uuidV3`, …). `## Note`/`## Example` prose reclassifies as comment so documentation recedes while code speaks. The legend grew append-only (4 modifiers).
- `textDocument/semanticTokens/range` and `full/delta` — ranged requests encode only overlapping tokens (first token absolute), and deltas answer a known result id with the minimal single-splice edit over the integer stream.
- Symbol resolution now sees English-word identifiers: `resolve_token_name` reads the SURFACE form for `Verb` tokens (the lexicon lemma "Greet" never matched a definition named "greet") and handles lexically `Ambiguous` words ("name"), so goto-definition, references, and rename work for identifiers that happen to be English words.

### Added (diagnostics depth)
- **The typechecker reports in the editor, with real spans.** The parser gained a statement-span side-table (`Parser::stmt_spans`, one span per top-level statement, 1:1 with the returned list), and a new `check_program_collect` in the compile crate reports EVERY failing top-level statement instead of bailing at the first (strict `check_program` unchanged). The LSP maps each finding onto its statement span, so return-type mismatches, arity errors, field-not-found (with the `available` fields filled from the registry — the socratic message lists what DOES exist), not-a-function, infinite types, and dimension errors now anchor where they happened. New stable codes: `type-mismatch` (now also for detailed mismatches), `infinite-type`, `arity-mismatch`, `field-not-found`, `not-a-function`.
- **Sentence-level parse recovery.** When a block fails to parse, the failing `Period`-delimited sentence is excised and the block reparsed (bounded, structure-guarded), so every independently broken sentence reports its own diagnostic and the good statements around it stay parsed, indexed, and navigable. Also fixed en route: mid-stream block slices had no EOF token, so the parser could walk off the end of a recovered block.
- **Statement-precise cause links.** Ownership and escape checkers grew `check_program_collect` variants (compile crate) that report EVERY finding with its top-level statement index and — for moves — the index of the statement that CAUSED it; the LSP maps both onto `Parser::stmt_spans`, so "was given away here" points at the exact `Give` statement machine-verifiably, and one use-after-move no longer hides the next (the variable resets to owned after reporting). The fallback heuristic was fixed too: the cause search now requires the variable in OBJECT position (`Give y to x` never blames the Give that *received* x), takes the most recent move before the use site, and the primary range approximates the USE (last occurrence), not the binding.
- **Unused-variable hints.** A `Let` nothing reads gets a HINT tagged UNNECESSARY (editors fade it) anchored on the binding, with a "Remove unused 'x'" quickfix that deletes the whole statement through its newline. Resolution-based (a reference must resolve to that definition), Variables only — parameters and fields are never flagged.

### Added (rustc flycheck)
- **"English with a borrow checker."** On save, the document compiles through the AOT backend's new MAPPED codegen and `cargo check` runs over the generated Rust in a persistent per-workspace cache directory (warm, incremental); every rustc finding is translated back to English and published on a real user-source span under the `logicaffeine (rustc)` source. The behavior contract is mock-proven over the real server loop: a newer save always beats a slower older one (generation guard), edits clear findings outright (stale positions never lie), findings overlapping interactive errors are deduplicated (rustc's value is what the local checkers MISS), and a machine without cargo degrades silently to interactive-only diagnostics. The check engine is a `FlycheckRunner` trait — `CargoFlycheck` in production, injectable mocks in tests — and an untranslatable build failure surfaces as "likely a LOGOS compiler bug", never a silent all-clear.

### Added (highlighting audit)
- **Semantic corpus golden** (`tests/semantic_corpus.rs`): a representative program rendered token-by-token (`lexeme→class+modifiers`) and committed — highlighting changes now show up as a reviewable diff. The audit it enabled fixed three real classification bugs: `Int` in annotations painted as a *variable* (primitives now resolve as types through the registry), struct names painted as *properties* (registry-path field definitions reused the struct header's span and collided in the declaration map), and `mutable` painted as a *type* (it lexes as a noun; it is a storage modifier).
- **Quickguide highlighting ratchet**: every canonical surface form in `LOGOS_QUICKGUIDE.md` must produce semantic tokens through the REAL pipeline (zero allowlisted), and a form that PANICS analysis is named individually — which is how the bare-`## Theorem:` parser crash was found. Analysis panics are additionally contained server-side (a poisoned document degrades to a stale snapshot instead of dying).
- The classification brain moved to `logicaffeine_language::token_class` (this crate maps class → legend index + modifiers) — the terminal REPL now paints with the identical classifier.
- Whole-document formatting now flows through `format_source` (structural reindent + multiline-string/prose protection — the per-line path could corrupt string literals), and on-type formatting refuses to touch a line inside an open `"""` literal.

### Added (configuration + gates)
- `logicaffeine.flycheck.enable` (default on): the on-save rustc pass honors live configuration — disabling retracts published findings across open documents and stops consulting the toolchain; re-enabling brings the pass back on the next save. Proven over the real server loop with a mock runner.
- Latency locks (`tests/perf_gate.rs`): full analysis and semantic-token encoding of a ~2k-line document must stay under generous catastrophe ceilings (5s/2s, ~25× today's numbers) — the loud tripwire that schedules the planned block-level reparse cache if a pass ever goes quadratic.

### Added (capability completion)
- **Document highlights** — cursor-hold highlighting of every occurrence, WRITE kind on bindings and mutation sites (the same write analysis the semantic tokens use), READ elsewhere.
- **Selection ranges** — expand-selection walks English structure: word → sentence → block → document, from the statement/block span tables the index already keeps.
- **On-type formatting** — typing `.` (the sentence terminator) normalizes that line with the exact `largo fmt` rules (`source_format::format_line`), so canonical form applies as you write, one sentence at a time.
- **Call hierarchy** — `f(…)` and `Call f` sites are indexed with their enclosing function; incoming and outgoing calls group per function with every call range (`## Main` callers are deliberately absent — the program is not a function).
- **Pull diagnostics** — `textDocument/diagnostic` returns full reports carrying a result id spanning BOTH engines (interactive version + flycheck state) and answers a known id with `Unchanged`; push publishing remains for older clients.

### Added (workspace)
- **Workspace-wide symbols + cross-file goto-definition.** `initialize` captures the workspace folders (with `root_uri` fallback) and a background scan analyzes every `.lg`/`.md` under them (bounded: 2000 files, 512 KB each, `target`/`node_modules`/dot-dirs skipped, truncation logged loudly) into a workspace index of functions/types/variants/fields/theorems. `workspace/symbol` answers case-insensitive substring queries across files never opened, and goto-definition falls back to the index when the current document doesn't define the name — jumping straight into the defining file. The index refreshes on save and on watched-file changes, and open-document state stays authoritative.
