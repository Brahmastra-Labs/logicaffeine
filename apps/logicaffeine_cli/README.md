# logicaffeine-cli

`largo` is the LOGOS build tool — the single front door to the whole engine. The `logicaffeine-cli` crate ships one binary — **`largo`** — that scaffolds projects, compiles LOGOS `.lg` source into a Cargo binary (or straight to `.wasm` with no toolchain), runs programs natively or on the interpreter, hosts an interactive REPL (imperative statements *and* English→FOL logic mode), translates English to first-order logic, proves theorems kernel-certified, solves DIMACS SAT instances with exportable DRAT proofs, formats sources, generates docs, edits manifest dependencies, audits the optimizer, runs optional Z3 verification, and publishes to a registry.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 4. Binary: `largo`. Depends on base, kernel, language, compile, proof (and jit, native-only); verify behind the `verification` feature.

## Installing

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

Windows: `powershell -ExecutionPolicy Bypass -c "irm https://logicaffeine.com/install.ps1 | iex"`. Prebuilt for Linux/macOS (x64 + arm64) and Windows x64, SHA-256-verified against the release's `SHA256SUMS`, installed to `~/.local/bin` with no sudo and no shell-config edits. `--full` installs the flavor with Z3 verification statically linked (`largo --version` then reports `(full)`). From source: `cargo install logicaffeine-cli`.

## Role in the workspace

`largo` is the user-facing front of the LOGOS pipeline. All command logic lives in the library crate `logicaffeine_cli`; the binary (`src/main.rs`) is a thin wrapper around `run_cli` that renders errors in the `error:`/`help:` style and maps them to exit codes (`0` ok, `1` failure, `2` usage; `sat` uses the competition's `10`/`20`). On native targets `main` installs the copy-and-patch JIT process-wide before dispatch, so every interpreted program gets hot-function / hot-loop tier-up; `LOGOS_NO_JIT=1` skips it (VM bytecode interpreter only), and the JIT is never linked on `wasm32`.

A project is a directory with a `Largo.toml` manifest — `[package]` (name, version, description, authors, `entry`, defaulting to `src/main.lg`) and `[dependencies]` keyed by version, `path`, or `git`. `build` generates a Cargo crate under `target/<mode>/build`, wires the runtime deps (`logicaffeine-data`, `logicaffeine-system` with `full`, `tokio`), and applies an aggressive release profile (LTO, `opt-level = 3`, one codegen unit, `panic = abort`, strip, plus `target-cpu=native` for non-cross release builds). During the cargo phase, cargo's own colored progress streams through live; failures are classified (a `## Requires` dependency problem vs. a compiler bug in the generated Rust) and framed accordingly.

For the full command / flag / environment reference, see **[docs/cli.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/cli.md)**.

## Commands (overview)

The clap subcommands (the `Commands` enum in `src/cli.rs`), dispatched by `run_cli` to handlers in `src/commands/`:

| Command | Purpose |
|---------|---------|
| `new <name>` / `init` | Scaffold a project (`Largo.toml`, `src/main.lg`, `.gitignore`) |
| `build` | Compile `.lg` → Rust, then `cargo build` (live streamed); `--emit wasm` for the direct backend |
| `run [args…]` | Build and execute; `-i/--interpret` for the sub-second tree-walker path |
| `check` | Parse and compile to Rust without producing a binary |
| `repl` | Interactive session: imperative statements + English→FOL logic mode (`:help` inside) |
| `logic [sentence]` | English → first-order logic (`--format unicode\|latex\|ascii\|kripke`, `--all-readings`, `--discourse`) |
| `prove [file]` | Kernel-certified proving of `## Theory` developments and `## Theorem` blocks (`--trace`, `--json`) |
| `sat <file.cnf>` | The certified SAT solver; `--proof` exports DRAT/DPR/SR; exit `10`/`20` |
| `fmt [paths…]` | Format sources with the canonical rules (the LSP's); `--check` for CI |
| `emit <rust\|c\|wasm\|wasm-linked>` | Print or write the generated code |
| `doc [--out DIR]` | Generate markdown documentation from a project's `##` blocks |
| `add <spec>` / `remove <name>` | Edit `Largo.toml` dependencies, format-preserving (toml_edit) |
| `clean [--all]` | Remove `target/` (and `.logos-native/` with `--all`) |
| `opts <file>` | Report which optimizations actually FIRED for a `.lg` file |
| `verify` | Run Z3 static verification only (Pro+ license; `verification` feature) |
| `completions <shell>` | Shell completion scripts (bash/zsh/fish/powershell/elvish) |
| `doctor` | Diagnose the environment: toolchain, wasm32 target, node for `--emit wasm`, verification flavor, registry + credentials, manifest health (offline-capable) |
| `publish` / `login` / `logout` | Registry packaging, upload, and credentials |

Global flags on every command: `-q/--quiet`, `-v/--verbose`, `--color auto|always|never` (NO_COLOR respected). `largo test` is reserved for the future LOGOS test framework. Flags and environment variables (`LOGOS_NO_JIT`, `LOGOS_LICENSE`, `LOGOS_TOKEN`, `LOGOS_CREDENTIALS_PATH`) are documented in [docs/cli.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/cli.md). Default registry: `https://registry.logicaffeine.com`.

## Crate structure / public API

```text
src/
├── main.rs        # `largo` binary: JIT install + run_cli wrapper
├── lib.rs         # public API surface
├── cli.rs         # Cli / Commands parser + dispatch
├── ui.rs          # CliError, exit codes, color state, clap palette, phase headers
├── commands/      # one module per verb (build, run, logic, prove, sat, fmt, …)
├── repl/          # the interactive session: loop, meta-commands, multiline, editor
├── compile.rs     # re-export of logicaffeine_compile::compile::*
└── project/
    ├── manifest.rs    # Largo.toml: Manifest, Package, DependencySpec
    ├── build.rs       # build orchestration: BuildConfig → BuildResult (+ cargo failure classifier)
    ├── registry.rs    # RegistryClient + tarball packaging
    └── credentials.rs # token storage / lookup
```

Public API (`logicaffeine_cli`):

- `run_cli()` — parse argv and dispatch (also `cli::{Cli, Commands}`).
- `ui::{CliError, render_error, ColorMode}` + the exit-code constants — the error/exit contract shared with the binary.
- `project::{BuildConfig, BuildResult, BuildError, CargoFailure, CargoFailureKind, classify_cargo_failure, build, run, find_project_root}` — build orchestration.
- `project::{Manifest, ManifestError}` — `Largo.toml` parse / serialize.
- `project::{RegistryClient, create_tarball, is_git_dirty}` — registry client and publishing.
- `project::{Credentials, get_registry_token}` — credential storage and token lookup.
- `project::{Loader, ModuleSource}` — module loader re-exported from the compile crate.
- `compile::*` (including `compile_project`), plus re-exports `interface` (kernel) and `analysis` (compile) for external tooling.

## Feature flags

| Feature | Effect |
|---------|--------|
| `default` | Empty (`default = []`). |
| `verification` | `largo verify` against the system Z3 (dynamic link); pulls the optional `logicaffeine-verify` dependency. |
| `verification-static` | `verification` with Z3 built from the z3-sys vendored source and statically linked — the release `largo-full` flavor (needs cmake + a C++ toolchain + libclang). |

Lean builds keep `verify` as a stub that points at the `--full` installer. `largo --version` reports `(full)` when verification is compiled in.

## Dependencies

Internal (Tiers 0–3): `logicaffeine-base`, `logicaffeine-kernel`, `logicaffeine-language`, `logicaffeine-compile`, `logicaffeine-proof` (sat/prove); `logicaffeine-verify` (optional, `verification`); plus `logicaffeine-jit` under `cfg(not(target_arch = "wasm32"))` for the native JIT tier.

External: `clap` + `clap_complete` (arg parsing, completions), `anstream` + `anstyle` (color discipline: auto-strips on pipes, honors NO_COLOR), `rustyline` (REPL line editing), `toml` + `toml_edit` + `serde` (manifest read/edit), `ureq` + `serde_json` (registry HTTP/JSON), `flate2` + `tar` (publish tarballs), `dirs` (config-dir resolution), `futures` (`block_on` for interpret/repl), `rand`. Dev: `tempfile`.

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
