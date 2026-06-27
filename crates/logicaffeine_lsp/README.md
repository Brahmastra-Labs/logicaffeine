# logicaffeine-lsp

A Language Server Protocol server for LOGOS. Builds the `logicaffeine-lsp` binary — a `tower-lsp` + `tokio` server that speaks JSON-RPC over stdin/stdout and drives the LOGOS pipeline (lexer → MWE → discovery → parser → escape/ownership analysis) to give editors live diagnostics, completion, hover, navigation, and refactoring for `.logos` source.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 4 — depends on logicaffeine_base, logicaffeine_language, logicaffeine_compile, logicaffeine_proof. Binary: `logicaffeine-lsp`.

## Role in the workspace

The server reuses the toolchain rather than reimplementing it (see [architecture](../../new_docs/architecture.md)): `logicaffeine_language` supplies the lexer/MWE/parser/discovery pass and type & policy registries, `logicaffeine_compile` (codegen feature) supplies `EscapeChecker`/`OwnershipChecker`, `logicaffeine_base` supplies arenas/interning/spans, and `logicaffeine_proof` backs the proof-related code-lens commands.

State lives in a `DashMap<Url, DocumentState>` for lock-free concurrent access. Text sync is **full** (`TextDocumentSyncKind::FULL`): each `did_change` carries the whole document and re-runs the pipeline, rebuilding the per-document `SymbolIndex` (definitions, references, block/statement spans) from the fresh parse. Indexing and references are **per-document** — there is no cross-file or workspace-wide symbol resolution.

Editor integration: point any LSP client at the `logicaffeine-lsp` binary and associate it with the `logos` filetype. The server reports completion trigger characters `.` `:` `'` and signature trigger characters `with` `,`. A VSCode extension lives at `editors/vscode/logicaffeine/`.

## LSP capabilities

`initialize` advertises 14 providers, each backed by a handler in `LogicAffeineServer`:

- **Diagnostics** — parse errors rendered as Socratic explanations, plus escape/ownership errors (`use-after-move`, `double-move`, `maybe-moved`, `escape-return`, `escape-assignment`, `zero-index`, `is-value-equality`, `undefined-variable`, `type-mismatch`), with `DiagnosticRelatedInformation` pointing at the cause (the `Give`/`Zone` site).
- **Semantic tokens** — full-document, delta-encoded on the wire, UTF-16 offsets; 13 token types and `declaration`/`readonly` modifiers (range requests not offered).
- **Hover** — keyword docs, block-header descriptions, identifier type/ownership info.
- **Document symbols** — nested outline from block headers and definitions.
- **Go to definition** and **find references** (honors `includeDeclaration`).
- **Completion** — context-aware: statement keywords after `.`/newline, expressions after `be`, types after `:`, struct fields after `'s`, enum variants after `Inspect`, identifiers in scope otherwise; trigger chars `.` `:` `'`.
- **Signature help** — parameter hints inside `Call`; trigger chars `with` `,`.
- **Code actions** — diagnostic-driven quick fixes only (`QUICKFIX`): spelling suggestions (`suggest::find_similar`), `is` → `equals`, `a copy of …` for move/escape errors, `0` → `1` for zero-index, nearest definition for an undefined variable.
- **Rename** — with `prepareRename` and new-name validation (rejects whitespace and reserved keywords).
- **Folding ranges** — block headers and indent/dedent regions.
- **Inlay hints** — inferred type annotations for untyped `Let` bindings, plus ownership-state markers (`moved`/`maybe moved`/`borrowed`) on non-owned variables.
- **Code lens** — `Run` over `## Main`, `Verify` + `Prove` over `## Theorem`, `Check Proof` over `## Proof`; each command carries the document URI and block name (no reference counts; `resolve_provider: false`).
- **Formatting** — tabs → 4 spaces, normalized leading whitespace, trailing-whitespace removal.

## Public API / binary

The crate exposes one server type, `server::LogicAffeineServer`, which implements `tower_lsp::LanguageServer`. The `logicaffeine-lsp` binary (`src/main.rs`) wires it up:

```rust
let (service, socket) = LspService::new(LogicAffeineServer::new);
Server::new(tokio::io::stdin(), tokio::io::stdout(), socket).serve(service).await;
```

It communicates over stdin/stdout; logging goes through `env_logger` to stderr (`RUST_LOG=debug logicaffeine-lsp`). Install with `cargo install logicaffeine-lsp` or build from the workspace with `cargo build --release -p logicaffeine-lsp`. Feature modules (`diagnostics`, `hover`, `completion`, `semantic_tokens`, `index`, `pipeline`, …) are public for reuse and testing; tests are inline `#[cfg(test)]` units across the source.

## Dependencies

- **Internal:** `logicaffeine-base`, `logicaffeine-language`, `logicaffeine-compile` (with `codegen`), `logicaffeine-proof` — all pinned to the workspace version.
- **External:** `tower-lsp` 0.20 (protocol + async trait), `tokio` 1 (`full`, async runtime + stdio), `dashmap` 6 (concurrent document map), `serde`/`serde_json` (command arguments), `log`/`env_logger` (stderr logging).

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
