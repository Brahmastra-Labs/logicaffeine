# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.10.0] - 2026-07-08


### Added
- Prebuilt binaries (5 platforms × lean/full flavors) with one-line installers (`logicaffeine.com/install.sh` / `install.ps1`), SHA-256-verified; the `full` flavor statically links Z3 (`verification-static` feature) and stamps `largo --version` with `(full)`.
- New verbs: `repl` (imperative replay session + English→FOL logic mode, meta-commands, rustyline on TTY / scriptable on pipes), `logic`, `prove` (`--trace`, `--json`), `sat` (shared `satcli` driver with `logos-sat`), `fmt` (`--check`), `emit rust|c|wasm|wasm-linked`, `doc`, `add`/`remove` (toml_edit, format-preserving), `clean`, `completions`; `test` reserved (hidden) for the future test framework.
- Global `-q/--quiet`, `-v/--verbose`, `--color` flags; cargo-palette help with Examples on every command; `error:`/`help:` styled errors with typed exit codes (`ui::CliError`).
- `largo build` streams cargo's colored output live behind `Compiling`/`Building`/`Finished` phase headers and classifies failures (`## Requires` resolution vs. generated-code compiler bug).
- Features: `verification` (system Z3) and `verification-static` (vendored static Z3) now actually wire `logicaffeine-verify` in.
- **REPL live syntax highlighting** — the prompt line paints as you type (keywords bold blue, strings green, numbers magenta, types cyan, meta-commands as a unit), driven by the language crate's `token_class` — the exact classifier the LSP uses, so terminal and editor can never disagree. Locked: stripping the ANSI escapes always yields the input byte-for-byte.
- **`:explain <word>`** — the REPL teaches from the same lesson table as the LSP's hover (`logicaffeine_language::teach`): one plain sentence, the runnable example indented below, and the socratic question that guides the next step. A word naming two constructs (`Set` the statement, `Set` the type) shows both; a near-miss suggests the closest lesson. ANSI styling never rewords (stripped output is locked byte-identical to the plain rendering).
- `largo check --deep` — after the in-memory compile, run rustc's analysis over the generated code (the IDE flycheck's exact pass) and print every finding translated to LOGOS with its `file:line`; exits nonzero on findings. The documented reproduction command for "what the editor's rustc diagnostics did".

### Changed
- Crate restructured: module-per-command `commands/`, `repl/` session modules, `ui.rs` output substrate; `cli.rs` is parser + dispatch only.

### Fixed
- `## Requires` dependencies were appended after `[profile.release]` in the generated Cargo.toml and silently ignored by cargo; they now land inside `[dependencies]` (and every component is charset-validated so no TOML structure can be smuggled into the manifest).
- The dead `phase37_cli.rs` integration tests (gated behind a feature that never existed) moved to `tests/project.rs` and run unconditionally.
- The wasm host shim wrote `args()` at a fixed low offset that the program's bump allocator grows through — argv silently corrupted under heap pressure. argv now lives on a shim-grown page the program can never address (the emitter never calls `memory.grow`). A trapping program also flushes its pre-trap output and fails loudly (exit 134) instead of swallowing everything.
- `largo run --emit <unknown>` fell through to the plain cargo build with the flag silently discarded; it now errors, and `--emit`/`--interpret` are mutually exclusive.
- The terminal REPL refreshed tab-completion by replaying the whole program a second time per line; completion now feeds from a zero-execution textual scan (`ReplSession::binding_names`).
- Assorted edge fixes: `logos:name@version` and doubled-`@` dependency specs are rejected; inline-table `dependencies = { … }` manifests are editable; `## Main`-prefixed and `## Theorem`-prefixed prose headers no longer match the real blocks; `largo doc` ignores `## ` lines inside fenced code; `.md` entry fallback works on interpret/emit/check/doc/prove (not just build); REPL `:save`/`:load` accept paths with spaces; `largo logic` on a TTY with no input errors immediately instead of hanging on stdin.
- `largo new`/`init` validate the project name (letter start; letters/digits/`-`/`_`) — path separators split the directory, quotes corrupted the generated wasm host shim, leading dashes were shell footguns. The shim additionally JSON-escapes the module filename, so standalone `largo emit wasm o'brien.lg` still emits valid JavaScript.
- The manifest `entry` is confined to the project (absolute paths and `..` escapes rejected), so a distributed Largo.toml cannot point commands at arbitrary files.
- `check --deep`'s flycheck workspace is keyed on the project *path*, not just the package name — two projects both named `hello` no longer share (or race on) one generated cache; finding line-reports clamp to char boundaries.
- `repl --load` of a headerless statement list loads it as the Main body instead of silently loading nothing; `doc --out <existing-file>` explains it needs a directory.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- `largo` CLI tool for LOGOS project management
- REPL mode for interactive logic exploration
- Project scaffolding (`largo new`)
- Package registry integration (`largo publish`, `largo install`)
- Build and test commands
- Rust code formatter for generated output
- Optional `verification` feature for Z3 integration
