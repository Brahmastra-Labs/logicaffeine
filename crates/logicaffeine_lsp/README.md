# logicaffeine-lsp

A Language Server Protocol server for LOGOS. Builds the `logicaffeine-lsp` binary — a `tower-lsp` + `tokio` server that speaks JSON-RPC over stdin/stdout and drives the LOGOS pipeline (lexer → MWE → discovery → parser → escape/ownership analysis) to give editors live diagnostics, completion, hover, navigation, and refactoring for `.logos` source.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 4 — depends on logicaffeine_base, logicaffeine_language, logicaffeine_compile, logicaffeine_proof. Binary: `logicaffeine-lsp`.

## Role in the workspace

The server reuses the toolchain rather than reimplementing it (see [architecture](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/architecture.md)): `logicaffeine_language` supplies the lexer/MWE/parser/discovery pass and type & policy registries, `logicaffeine_compile` (codegen feature) supplies `EscapeChecker`/`OwnershipChecker`, `logicaffeine_base` supplies arenas/interning/spans, and `logicaffeine_proof` backs the proof-related code-lens commands.

State is a map of immutable snapshots (`DashMap<Url, Arc<DocumentState>>`): request handlers clone the `Arc` out and drop the map guard immediately, so a guard can never be held across an `.await`. Text sync is **incremental** (`TextDocumentSyncKind::INCREMENTAL`): the `scheduler` module holds the live text per document, applies UTF-16 range edits, and debounces analysis (~150 ms) behind a per-document generation counter — a typing burst coalesces into one pipeline pass over the final text, and stale results are dropped by the generation guard rather than cancelled with locks. Each completed analysis rebuilds the per-document `SymbolIndex` (definitions, references, block/statement spans) and is swapped in as a fresh snapshot. Per-document indexing is authoritative for open files; a background `workspace` index over every `.lg`/`.md` under the workspace folders adds `workspace/symbol` and cross-file goto-definition (references/rename remain per-document).

Editor integration: point any LSP client at the `logicaffeine-lsp` binary and associate it with the `logos` filetype. The server reports completion trigger characters `.` `:` `'` and signature trigger characters `␠` (space) `,` — all single characters, since LSP clients only send single-character triggers. A VSCode extension lives at `editors/vscode/logicaffeine/`.

## LSP capabilities

`initialize` advertises 20 providers, each backed by a handler in `LogicAffeineServer`:

- **Diagnostics** — parse errors rendered as Socratic explanations, typechecker findings anchored on real statement spans (`type-mismatch`, `arity-mismatch`, `field-not-found` listing the fields that DO exist, `not-a-function`, `infinite-type`), escape/ownership errors (`use-after-move`, `double-move`, `maybe-moved`, `escape-return`, `escape-assignment`, `zero-index`, `is-value-equality`, `undefined-variable`) with `DiagnosticRelatedInformation` pointing at the exact causing statement, sentence-level recovery (every broken sentence reports; good code stays analyzed), and unused-variable hints (UNNECESSARY-tagged, with a remove quickfix).
- **rustc flycheck** — on save, the document compiles through the AOT backend's mapped codegen and runs `cargo check` in a persistent per-workspace cache; every rustc finding comes back translated to English on a user-source span, published under the `logicaffeine (rustc)` source. A newer save always wins (generation guard), edits clear findings, findings overlapping interactive errors are deduplicated, and a machine without cargo degrades silently to interactive-only diagnostics.
- **Semantic tokens** — resolution-aware: the base layer classifies by part of speech (verbs=function, nouns=type, adjectives=modifier — the grammar IS the syntax), and a `SymbolIndex` overlay upgrades identifiers to what they resolve to (parameter/function/type/field/variant/variable) with `declaration` only at the definition site, `readonly` on immutable `Let`s, `modification` on write targets (`Set`/`Increase`/`Push … to`), and `defaultLibrary` on stdlib prelude names; `## Note`/`## Example` prose recedes to comment. Full, **range**, and **full/delta** requests (single-splice edits against a cached result id); 13 token types, 4 modifiers, UTF-16 offsets, append-only legend.
- **Hover** — keyword docs, block-header descriptions, identifier type/ownership info.
- **Document symbols** — nested outline from block headers and definitions.
- **Go to definition** and **find references** (honors `includeDeclaration`).
- **Completion** — context-aware: statement keywords after `.`/newline, expressions after `be`, types after `:`, struct fields after `'s`, enum variants after `Inspect`, identifiers in scope otherwise; trigger chars `.` `:` `'`.
- **Signature help** — parameter hints inside `Call`; trigger chars `␠` (space) `,`.
- **Code actions** — diagnostic-driven quick fixes only (`QUICKFIX`): spelling suggestions (`suggest::find_similar`), `is` → `equals`, `a copy of …` for move/escape errors, `0` → `1` for zero-index, nearest definition for an undefined variable.
- **Rename** — with `prepareRename` and new-name validation (rejects whitespace and reserved keywords).
- **Folding ranges** — block headers and indent/dedent regions.
- **Inlay hints** — inferred type annotations for untyped `Let` bindings, plus ownership-state markers (`moved`/`maybe moved`/`borrowed`) on non-owned variables.
- **Code lens** — `Run` over `## Main`, `Verify` + `Prove` over `## Theorem`, `Check Proof` over `## Proof`; each command carries the document URI and block name (no reference counts; `resolve_provider: false`).
- **Formatting** — tabs → 4 spaces, normalized leading whitespace, trailing-whitespace removal; **on-type**: typing `.` normalizes the sentence's line with the same `largo fmt` rules.
- **Document highlights** — every occurrence of the symbol under the cursor, WRITE on bindings and mutation sites, READ elsewhere.
- **Selection ranges** — expand-selection follows English structure: word → sentence → block → document.
- **Call hierarchy** — incoming/outgoing calls over indexed `f(…)`/`Call f` sites (calls from `## Main` don't appear as incoming edges — Main is the program, not a function).
- **Pull diagnostics** — `textDocument/diagnostic` with result-id reuse (an unedited document answers `Unchanged`); push publishing stays for older clients.
- **Workspace symbols** — case-insensitive substring search over every indexed file in the workspace.

## Public API / binary

The crate exposes one server type, `server::LogicAffeineServer`, which implements `tower_lsp::LanguageServer`. The `logicaffeine-lsp` binary (`src/main.rs`) wires it up:

```text
let (service, socket) = LspService::new(LogicAffeineServer::new);
Server::new(tokio::io::stdin(), tokio::io::stdout(), socket).serve(service).await;
```

It communicates over stdin/stdout; logging goes through `env_logger` to stderr (`RUST_LOG=debug logicaffeine-lsp`). Install with `cargo install logicaffeine-lsp` or build from the workspace with `cargo build --release -p logicaffeine-lsp`.

Every feature module is public for reuse and testing (tests are inline `#[cfg(test)]` units across the source, plus end-to-end tests in `tests/` that drive the real server loop over in-memory pipes and ratchet locks that pin the classification/decision/capability invariants):

- **Pipeline & state** — `pipeline` (lexer→parse→analysis driver), `state` (the snapshot store) + `scheduler` (live text, incremental edits, debounce + generation guard) + `document` (one analyzed snapshot, plus `apply_content_change`), `index` + `line_index` (per-document `SymbolIndex` and UTF-16 line/offset mapping), `workspace` (the background cross-file index behind `workspace/symbol`, cross-file goto-definition, and cross-file references/rename — open buffers answer for themselves, the disk index answers for everything else), `flycheck` (the on-save rustc pass: `FlycheckRunner` seam, generation-guarded staleness, `CargoFlycheck` shelling to `cargo check` through the compile crate's mapped codegen).
- **Request handlers** — `diagnostics` (including `decision_for`, the total severity/code/quickfix table every `ParseErrorKind` must pass through), `hover`, `completion`, `definition`, `references`, `document_symbols`, `document_highlights`, `selection_ranges`, `call_hierarchy`, `semantic_tokens` (including the public `classify_token` classifier), `signature_help`, `code_actions`, `rename`, `folding`, `inlay_hints`, `code_lens`, `formatting` — one per LSP capability.
- **Teaching** — `teach_md` renders the shared lesson table (`logicaffeine_language::teach`) to markdown: one renderer feeds hover AND completion documentation (they can never phrase a lesson differently), and `guide_url` is the single seam for the quickguide "read more" links. `stdlib_docs` is the stdlib teaching registry: every prelude definition's literate `## Note` documentation, read once from the raw embedded module sources — hover, completion, and signature help fall back here, so `md5` teaches in the editor exactly what its Note says in the source (ratcheted by `tests/stdlib_teach_lock.rs`).
- **Server** — `server` ties them together as `LogicAffeineServer`.

## Dependencies

- **Internal:** `logicaffeine-base`, `logicaffeine-language`, `logicaffeine-compile` (with `codegen`), `logicaffeine-proof` — all pinned to the workspace version.
- **External:** `tower-lsp` 0.20 (protocol + async trait), `tokio` 1 (`full`, async runtime + stdio), `dashmap` 6 (concurrent document map), `serde`/`serde_json` (command arguments), `log`/`env_logger` (stderr logging).

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
