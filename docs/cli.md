# The `largo` CLI

`largo` is the LOGOS build tool and package manager — Cargo for English, and the front door to the
whole engine. It scaffolds projects, compiles LOGOS to Rust (and on to a native binary) or directly
to WASM, runs programs through the interpreter, hosts the interactive REPL, translates English to
First-Order Logic, proves theorems kernel-certified, solves DIMACS SAT instances, formats source,
performs static verification, and talks to the package registry.

Source of truth: [`apps/logicaffeine_cli/src/cli.rs`](../apps/logicaffeine_cli/src/cli.rs) (the
command surface) and [`apps/logicaffeine_cli/src/project/`](../apps/logicaffeine_cli/src/project/)
(manifest, build, registry, credentials).

## Installing

One line on Linux and macOS (x64 + arm64):

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

Windows (x64):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://logicaffeine.com/install.ps1 | iex"
```

The installer downloads the prebuilt binary for your platform from the GitHub release, verifies
its SHA-256 against the release's `SHA256SUMS`, and installs it atomically as `largo` in
`~/.local/bin` (Windows: `%LOCALAPPDATA%\Programs\largo`). It never uses sudo and never edits
shell configuration — if the directory isn't on your `PATH`, it prints the exact line to add.

| Flag | Effect |
|------|--------|
| `--full` | Install the full build: Z3 static verification bundled (`largo verify` works out of the box; still license-gated at runtime). ~30 MB larger. |
| `--version vX.Y.Z` | Install an exact release instead of the latest. |
| `--to DIR` | Install directory override (also: `LARGO_INSTALL_DIR`). |

Both flavors install as `largo`; `largo --version` reports `X.Y.Z (full)` for the full build.
Prebuilt platforms: `linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, `win32-x64`.
Re-running the installer upgrades in place. Uninstall by deleting the binary.

### From source

```bash
cargo install logicaffeine-cli           # needs a Rust toolchain
# or, in the repo:
cargo build -p logicaffeine-cli          # debug build → target/debug/largo
cargo build -p logicaffeine-cli --release
```

## Commands

The full subcommand set (from the `Commands` enum in `cli.rs`):

**Project**

| Command | Purpose |
|---------|---------|
| `largo new <name>` | Scaffold a new project in a new `<name>/` directory |
| `largo init [--name <name>]` | Scaffold a project in the current directory |
| `largo add <spec> [--path\|--git]` | Add a dependency to `Largo.toml` (format-preserving edit) |
| `largo remove <name>` | Remove a dependency from `Largo.toml` |
| `largo clean [--all]` | Delete `target/` (`--all` also clears the `.logos-native/` cache) |

**Build & run**

| Command | Purpose |
|---------|---------|
| `largo build [flags]` | Compile LOGOS → Rust, then `cargo build` the result (or `--emit wasm`) |
| `largo run [flags] [-- args…]` | Build and run — compiled, interpreted, or as WASM |
| `largo check [--deep]` | Parse + type-check without producing a binary |
| `largo emit rust\|c\|wasm\|wasm-linked [file] [-o path]` | Print/write the compiled translation without building a binary |
| `largo fmt [paths…] [--check]` | Format LOGOS sources (same rules as the language server) |
| `largo opts <file> [--json]` | Report which optimizations actually fired for a file |

**Logic, proof & SAT**

| Command | Purpose |
|---------|---------|
| `largo logic "<sentence>" [flags]` | English → First-Order Logic from the terminal |
| `largo prove [file] [--trace\|--json]` | Prove `## Theory` / `## Theorem` blocks, kernel-certified |
| `largo sat <file.cnf> [--proof <path>] [--stats]` | The certified SAT engine on DIMACS CNF (exit 10/20) |
| `largo verify [--license <key>]` | Run Z3 static verification only (Pro+) |

**Interactive & docs**

| Command | Purpose |
|---------|---------|
| `largo repl [--logic] [--format <f>] [--load <file>]` | The interactive session (imperative + logic modes) |
| `largo doc [--out <dir>]` | Generate markdown docs from the project's `##` blocks |

**Registry**

| Command | Purpose |
|---------|---------|
| `largo publish [flags]` | Package and upload to the registry |
| `largo login [--registry <url>] [--token <key>]` | Store a registry token |
| `largo logout [--registry <url>]` | Remove stored credentials |

**Environment**

| Command | Purpose |
|---------|---------|
| `largo doctor [--registry <url>]` | Diagnose the toolchain, wasm32 target, node, registry, project health |
| `largo completions <shell>` | Shell completions (bash, zsh, fish, powershell, elvish) |

Every command takes the global flags `-q/--quiet`, `-v/--verbose` (repeatable), and
`--color auto|always|never` (`NO_COLOR` respected). `largo test` is reserved for the future LOGOS
test framework.

### `new` / `init`

`new` creates `<name>/` containing a `Largo.toml`, a `src/main.lg` entry point with a hello-world
example, and a `.gitignore`. `init` does the same in an existing directory (the package name
defaults to the directory name, or `--name`).

```bash
largo new my_project
cd my_project
largo run
```

### `build`

Compiles the LOGOS source to Rust, then invokes `cargo build` on the generated crate.

| Flag | Effect |
|------|--------|
| `-r`, `--release` | Optimized build |
| `--verify` | Run Z3 static verification after compilation (needs a Pro+ license) |
| `--license <key>` | License key for verification (or set `LOGOS_LICENSE`) |
| `--lib` | Generate a library (`lib.rs`, `crate-type = ["cdylib"]`) instead of a binary |
| `--target <triple>` | Cross-compile; `wasm` is shorthand for `wasm32-unknown-unknown` |
| `--native-functions` | Pre-build every `is exported for native` function into a cached cdylib under `.logos-native/` |
| `--emit wasm` | Compile DIRECTLY to a self-contained `target/<name>.wasm` via the built-in backend — no rustc, cargo, or wasm-bindgen in the loop; milliseconds |
| `--emit wasm-linked` | As `--emit wasm`, but links the real `logicaffeine_base::BigInt` runtime via `rust-lld`, so overflowing integer arithmetic computes the exact big number instead of wrapping (needs the Rust toolchain + a wasm32 `base` build) |

### `run`

`largo build` followed by executing the binary; the program's exit code is propagated.

| Flag | Effect |
|------|--------|
| `-r`, `--release` | Optimized build before running |
| `-i`, `--interpret` | Skip Rust compilation; execute via the tree-walking interpreter for sub-second feedback |
| `--emit wasm` \| `wasm-linked` | Compile directly to `.wasm` (built-in backend) and run it through the emitted Node.js host shim — compile-and-run in one step |
| `-- <args…>` | Arguments passed through to the program |

### `check`

Parses and type-checks without the full build pipeline — fast validation during development.
`--deep` also runs rustc's analysis over the generated code (the same pass the IDE's flycheck
uses) and translates its findings back to LOGOS terms.

### `opts`

Compiles a `.lg` file on the AOT, run-path, and VM-compile paths with the optimization firing trace
on, then lists the optimizations that *genuinely changed* the program (not merely the ones enabled).
`--json` emits the fired-optimization keyword list as JSON.

```bash
largo opts src/main.lg
largo opts src/main.lg --json
```

### `verify`

Runs Z3 static verification of the project's logical constraints without building, gated on a Pro+
license (via `--license` or the `LOGOS_LICENSE` environment variable). See
[Proof & verification](proof-and-verification.md).

> The verifier ships behind a build-time gate: a default `largo` build answers `verify` (and
> `build --verify`) with a notice that verification has to be compiled in, because it pulls in the
> Z3-backed [`logicaffeine_verify`](../crates/logicaffeine_verify/README.md) crate (kept out of the
> default build so the common path needs no Z3 toolchain).

### `publish` / `login` / `logout`

`publish` packages the project as a tarball and uploads it. Pre-flight checks confirm the entry
point exists, the git working directory is clean (override with `--allow-dirty`), and the auth token
is valid. `--dry-run` validates without uploading; `--registry <url>` overrides the default.

`login` stores an API token (interactive prompt, or `--token`); `logout` removes it. Tokens live in
`~/.config/logos/credentials.toml`.

### `emit`

Prints the generated Rust or C translation of the program, or writes a self-contained `.wasm`
module (built-in backend, no rustc) with its Node.js host shim. Without a file argument it uses the
current project's entry; with one, it works on any standalone `.lg`/`.md` source. `-o` writes to a
path instead of stdout (rust/c) or the default module path (wasm).

```bash
largo emit rust                       # generated Rust on stdout
largo emit c standalone.lg            # the C translation of one file
largo emit wasm-linked -o dist/app.wasm
```

### `fmt`

Applies the canonical style (4-space indentation, no tabs, no trailing whitespace) — the exact
rules the language server's formatting provider uses, so the CLI and the editor can never disagree.
Without paths it formats the whole project; with paths, exactly those files. `--check` writes
nothing and exits 1 if anything would change (the CI gate).

### `add` / `remove`

Format-preserving `Largo.toml` edits (comments and layout survive). `add` accepts `name` (any
version), `name@version`, or `logos:name` (the registry URI form); `--path` and `--git` record
local and git dependencies. `remove` deletes the named `[dependencies]` entry and leaves the rest
of the manifest byte-identical.

### `clean`

Deletes the project's `target/` directory. `--all` also removes the `.logos-native/`
compiled-function bundle cache produced by `largo build --native-functions`.

### `repl`

The interactive session — two modes in one. `logos>` runs imperative statements against a
persistent interpreter session (the exact `run --interpret` engine, so REPL semantics cannot drift
from program semantics); `logic>` is English → FOL with discourse-aware anaphora. `:help` lists the
meta-commands (`:mode`, `:format`, `:readings`, `:vars`, `:explain <word>`, `:save` — which writes
the session back out as a runnable program). Line editing and keyword completion on a TTY; a plain
scriptable line loop on a pipe.

```bash
largo repl
largo repl --logic --format latex
printf 'Let x be 5.\nShow x.\n' | largo repl
```

### `logic`

Logic mode from the terminal: compiles an English sentence (inline, `--file`, or piped stdin) to
First-Order Logic, printing bare FOL on stdout so it pipes cleanly. `--format
unicode|latex|ascii|kripke` selects the rendering, `--all-readings` enumerates every quantifier
scope and parse-forest reading (numbered), `--pragmatic` enriches with scalar implicature, and
`--discourse` treats each input line as one sentence of a discourse with shared anaphora context.

```bash
largo logic "Every woman loves a man." --all-readings
printf 'A farmer owns a donkey.\nHe feeds it.' | largo logic --discourse
```

### `prove`

Proves the theorems in a LOGOS source file: `## Theory` developments (formal Axiom/Theorem
declarations, proved in citation order) and English `## Theorem` blocks (Given/Prove/Proof) run
through the proof engine, and every ✓ is certified by the type-theory kernel — a mere derivation
never counts. `--trace` renders the derivation tree under each proved theorem; `--json` emits
machine-readable results.

### `sat`

The SAT Competition interface as a largo verb: solves a DIMACS CNF with the certified engine,
printing `s SATISFIABLE` with a `v` model or `s UNSATISFIABLE`, with competition exit codes
(10 = SAT, 20 = UNSAT, 1 = error). `--proof <path>` exports the UNSAT certificate (DRAT; DPR/SR
for the symmetry routes) for external checkers like drat-trim; `--stats` prints solver statistics
to stderr.

### `doc`

Renders a markdown reference from the literate structure of the entry file — `## To` signatures,
type definitions, notes, examples, and formal blocks, in source order (`## Main` is omitted).
Output goes to `target/doc` or `--out <dir>`.

### `doctor`

Diagnoses the environment largo runs in: the Rust toolchain (needed by `build`/`run`), the wasm32
target, node (for `--emit wasm`), the verification flavor, registry reachability and credentials,
update freshness, and — inside a project — manifest health. Degradations are warnings; only a
broken project fails. Works offline.

### `completions`

Writes a completion script for bash, zsh, fish, powershell, or elvish to stdout — source it from
your shell's configuration for tab completion on every largo command and flag.

```bash
largo completions zsh > ~/.zfunc/_largo
```

## The `Largo.toml` manifest

Defined by the `Manifest`/`Package` structs in
[`project/manifest.rs`](../apps/logicaffeine_cli/src/project/manifest.rs):

```toml
[package]
name = "my_project"
version = "0.1.0"
description = "..."          # optional
authors = ["You <you@example.com>"]   # optional
entry = "src/main.lg"        # entry point

[dependencies]
std  = "logos:std"                    # simple form: a version/URI string
math = { path = "./math" }            # detailed form: { version, path, git }
```

## Environment variables

Read by the CLI (grep of `apps/logicaffeine_cli/src/`):

| Variable | Effect |
|----------|--------|
| `LOGOS_NO_JIT` | Skip the JIT; run on the bytecode VM only |
| `LOGOS_LICENSE` | License key for `verify` / `build --verify` |
| `LOGOS_TOKEN` | Registry auth token (alternative to `largo login`) |
| `LOGOS_CREDENTIALS_PATH` | Override the credentials file location |
| `LOGOS_MAX_AST_DEPTH` | Raise/lower the AST nesting-depth limit (default 128). The default is sized for the smallest standard stacks (worker threads, browser wasm); machines with deep stacks can raise it for heavily generated code, constrained embedders can lower it |

The default registry is `https://registry.logicaffeine.com`.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
