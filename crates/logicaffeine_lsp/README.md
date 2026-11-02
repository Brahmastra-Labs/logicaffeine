# logicaffeine-lsp

`logicaffeine-lsp` is the LOGOS language server: point any LSP-capable editor at its binary and you get live diagnostics, completion, hover, navigation and refactoring for `.lg` files, literate `.md` modules, and pages written in LGX, the LOGOS web framework. It carries no second grammar of its own. It drives the compiler's pipeline (lexer → MWE → discovery → parser → escape/ownership analysis), so the editor and the compiler read a program the same way.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 5. It builds one binary, `logicaffeine-lsp`: a `tower-lsp` + `tokio` server that speaks JSON-RPC over stdin/stdout.

## Role in the workspace

The server reuses the toolchain rather than reimplementing it (see [architecture](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/architecture.md)). `logicaffeine_language` supplies the lexer/MWE/parser/discovery pass and the type and policy registries, `logicaffeine_compile` (`codegen` feature) supplies `EscapeChecker`/`OwnershipChecker`, and `logicaffeine_base` supplies arenas, interning and spans. A proof block's code lens issues `logicaffeine.checkProof`, which the editor extension runs through `largo`.

State is a map of immutable snapshots (`DashMap<Url, Arc<DocumentState>>`). A request handler clones the `Arc` out and drops the map guard immediately, so no guard is ever held across an `.await`.

Text sync is incremental (`TextDocumentSyncKind::INCREMENTAL`). The `scheduler` module holds each document's live text, applies UTF-16 range edits, and debounces analysis (about 150 ms) behind a per-document generation counter. A typing burst coalesces into one pipeline pass over the final text, and a stale result is dropped by the generation guard rather than cancelled with locks. Each completed analysis rebuilds the document's `SymbolIndex` (definitions, references, block and statement spans) and swaps it in as a fresh snapshot.

The per-document index is authoritative for open files. Behind it, a background `workspace` index over every `.lg`/`.md` file under the workspace folders answers `workspace/symbol`, cross-file goto-definition, cross-file references and rename. It also carries the module knowledge behind auto-import, import-scoped completion and "Defined in module …" hover.

To use the server, point any LSP client at the `logicaffeine-lsp` binary for `.lg` files. Completion triggers on `.` `:` `'` and signature help on `␠` (space) and `,`: all single characters, since LSP clients send only single-character triggers. The VS Code extension lives in [editors/vscode/logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/tree/main/editors/vscode/logicaffeine).

## LSP capabilities

What `initialize` advertises, each capability backed by a handler in `LogicAffeineServer`:

- **Diagnostics** — parse errors rendered as Socratic explanations, and typechecker findings anchored on real statement spans (`type-mismatch`, `arity-mismatch`, `field-not-found` listing the fields that do exist, `not-a-function`, `infinite-type`). Escape and ownership errors (`use-after-move`, `double-move`, `maybe-moved`, `escape-return`, `escape-assignment`, `zero-index`, `is-value-equality`, `undefined-variable`) carry `DiagnosticRelatedInformation` pointing at the exact causing statement. Recovery is sentence-level: every broken sentence reports, and good code stays analyzed. Unused variables get hints tagged `UNNECESSARY`, with a remove quickfix. The same channel carries the LGX template checker's findings (`lgx-<kind>`, e.g. `lgx-image-without-alt`, at the element), the value-semantics copy lint (`copy-<kind>`, e.g. `copy-orphaned-copy`), and the browser build's refusals (`browser-tier-refusal`, an error at the construct a page cannot run in the browser).
- **rustc flycheck** — on save, the document compiles through the AOT backend's mapped codegen and `cargo check` runs over it in a persistent per-workspace cache. Every rustc finding comes back translated to English on a user-source span, published under the `logicaffeine (rustc)` source. A newer save always wins (generation guard), edits clear findings, and findings overlapping interactive errors are deduplicated. A machine without cargo degrades quietly to interactive-only diagnostics.
- **Semantic tokens** — resolution-aware. The base layer classifies by part of speech (verbs as functions, nouns as types, adjectives as modifiers: the grammar *is* the syntax). A `SymbolIndex` overlay then upgrades identifiers to what they resolve to (parameter, function, type, field, variant, variable). Modifiers follow the binding: `declaration` only at the definition site, `readonly` on immutable `Let`s, `modification` on write targets (`Set`/`Increase`/`Push … to`), and `defaultLibrary` on stdlib prelude names. `## Note`/`## Example` prose recedes to comment; inside a `View:` template, element and attribute names paint as `tag`/`attribute`, which the VS Code manifest maps onto HTML scopes. Full, **range**, and **full/delta** requests are served (single-splice edits against a cached result id); the legend is `TOKEN_TYPES`/`TOKEN_MODIFIERS` in `semantic_tokens`, with UTF-16 offsets, append-only.
- **Hover** — keyword docs, block-header descriptions, identifier type and ownership info, a name inside a template hole resolved to its definition, and "*Defined in module …*" for a name another project module defines.
- **Document symbols** — nested outline from block headers and definitions.
- **Go to definition** and **find references** (honors `includeDeclaration`).
- **Completion** — context-aware: statement keywords after `.`/newline, expressions after `be`, types after `:`, struct fields after `'s`, enum variants after `Inspect`, and identifiers in scope otherwise, with stdlib prelude names after the locals. Inside a `View:` template it offers only the page's state, props and actions, the components it can place, and the template tags; inside a `Style:` body, the selectors its component renders. After `Import` it offers the project's module names; in code, the names the file's imports bring into scope (a selective import narrows to its names). Trigger characters `.` `:` `'`.
- **Signature help** — parameter hints inside `Call`; trigger characters `␠` (space) `,`.
- **Code actions** — quick fixes (`QUICKFIX`): spelling suggestions (`suggest::find_similar`), `is` → `equals`, `a copy of …` for move and escape errors, `0` → `1` for zero-index, and the nearest definition for an undefined variable. Auto-import offers an `Import <Module>.` for an unresolved name another module exports, one action per candidate module, and an unused `Import` gets a removal fix.
- **Rename** — with `prepareRename` and new-name validation (rejects whitespace and reserved keywords).
- **Folding ranges** — block headers and indent/dedent regions.
- **Inlay hints** — inferred type annotations for untyped `Let` bindings, plus ownership-state markers (`moved`/`maybe moved`/`borrowed`) on non-owned variables.
- **Code lens** — `Run` over `## Main`, `Verify` + `Prove` over `## Theorem`, `Check Proof` over `## Proof`; each command carries the document URI and block name (no reference counts; `resolve_provider: false`).
- **Formatting** — the whole document through the canonical formatter `logicaffeine_language::source_format::format_source`: the `largo fmt` rules, a structural reindent at 4 spaces per nesting level with string and prose interiors untouched. **Range** formatting applies the edits on the requested lines; **on-type**, typing `.` normalizes the sentence's line with the same rules.
- **Document highlights** — every occurrence of the symbol under the cursor: `WRITE` on bindings and mutation sites, `READ` elsewhere.
- **Selection ranges** — expand-selection follows English structure: word → sentence → block → document.
- **Call hierarchy** — incoming and outgoing calls over indexed `f(…)`/`Call f` sites. Calls from `## Main` don't appear as incoming edges, since Main is the program, not a function.
- **Pull diagnostics** — `textDocument/diagnostic` with result-id reuse (an unedited document answers `Unchanged`); push publishing stays for older clients.
- **Workspace symbols** — case-insensitive substring search over every indexed file in the workspace.

## Public API

The crate exposes one server type, `server::LogicAffeineServer`, which implements `tower_lsp::LanguageServer`. The `logicaffeine-lsp` binary (`src/main.rs`) wires it up:

```text
let (service, socket) = LspService::new(LogicAffeineServer::new);
Server::new(tokio::io::stdin(), tokio::io::stdout(), socket).serve(service).await;
```

It communicates over stdin/stdout; logging goes through `env_logger` to stderr (`RUST_LOG=debug logicaffeine-lsp`). Install with `cargo install logicaffeine-lsp`, or build from the workspace with `cargo build --release -p logicaffeine-lsp`.

Every feature module is public for reuse and testing. Unit tests sit behind `#[cfg(test)]` stubs across the source, with their bodies in `crates/logicaffeine_tests/unit/logicaffeine_lsp/`. End-to-end tests in `crates/logicaffeine_tests/tests/` drive the real server loop over in-memory pipes, and the locks there pin the classification, decision and capability invariants.

- **Pipeline & state** — `pipeline` (the lexer→parse→analysis driver); `state` (the snapshot store) with `scheduler` (live text, incremental edits, debounce and generation guard) and `document` (one analyzed snapshot, plus `apply_content_change`); `index` + `line_index` (the per-document `SymbolIndex` and UTF-16 line/offset mapping). `workspace` is the background cross-file index behind `workspace/symbol`, cross-file goto-definition, and cross-file references and rename: open buffers answer for themselves, and the disk index answers for everything else. `flycheck` is the on-save rustc pass: the `FlycheckRunner` seam, generation-guarded staleness, and `CargoFlycheck`, which shells out to `cargo check` through the compile crate's mapped codegen.
- **Request handlers** — one per LSP capability. `diagnostics` includes `decision_for`, the total severity/code/quickfix table every `ParseErrorKind` must pass through; the rest are `hover`, `completion`, `definition`, `references`, `document_symbols`, `document_highlights`, `selection_ranges`, `call_hierarchy`, `semantic_tokens` (with the public `classify_token` classifier), `signature_help`, `code_actions`, `rename`, `folding`, `inlay_hints`, `code_lens`, `formatting`. `template` is the LGX template vocabulary (tag phrases, `{for each}` clause words, filter pipes) that completion and the `View:` painter share.
- **Teaching** — `teach_md` renders the shared lesson table (`logicaffeine_language::teach`) to markdown. One renderer feeds both hover and completion documentation, so the two cannot phrase a lesson differently, and `guide_url` builds every quickguide "read more" link. `stdlib_docs` is the stdlib teaching registry: every prelude definition's literate `## Note` documentation, read once from the raw embedded module sources. Hover, completion, and signature help fall back to it, so `md5` teaches in the editor exactly what its Note says in the source (locked by `crates/logicaffeine_tests/tests/stdlib_teach_lock.rs`).
- **Server** — `server` ties them together as `LogicAffeineServer`.

## Feature flags

| Feature | Default | Gates |
|---------|---------|-------|
| `server` | Yes | The Language Server Protocol transport — `tower-lsp`, `tokio` and `env_logger` — with the `server` module and the `logicaffeine-lsp` binary. Without it the crate is the analysis alone (hover, diagnostics, completion, …), which the browser Studio links directly: the workspace takes it with `default-features = false`. |

## Dependencies

- **Internal:** `logicaffeine-base`, `logicaffeine-language`, `logicaffeine-compile` (with `codegen`), all pinned to the workspace version.
- **External:** `tower-lsp` 0.20 (protocol and async trait), `tokio` 1 (`full`: the async runtime and stdio), `dashmap` 6 (the concurrent document map), `serde`/`serde_json` (command arguments), `log`/`env_logger` (stderr logging). Dev: `tempfile`.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_lsp/CHANGELOG.md)
