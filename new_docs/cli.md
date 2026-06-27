# The `largo` CLI

`largo` is the LOGOS build tool and package manager — Cargo for English. It scaffolds projects,
compiles LOGOS to Rust (and on to a native binary), runs programs directly through the interpreter,
performs static verification, and talks to the package registry.

Source of truth: [`apps/logicaffeine_cli/src/cli.rs`](../apps/logicaffeine_cli/src/cli.rs) (the
command surface) and [`apps/logicaffeine_cli/src/project/`](../apps/logicaffeine_cli/src/project/)
(manifest, build, registry, credentials).

## Installing

`largo` is the binary produced by the `logicaffeine-cli` crate:

```bash
cargo build -p logicaffeine-cli          # debug build → target/debug/largo
cargo build -p logicaffeine-cli --release
```

## Commands

The full subcommand set (from the `Commands` enum in `cli.rs`):

| Command | Purpose |
|---------|---------|
| `largo new <name>` | Scaffold a new project in a new `<name>/` directory |
| `largo init [--name <name>]` | Scaffold a project in the current directory |
| `largo build [flags]` | Compile LOGOS → Rust, then `cargo build` the result |
| `largo run [flags] [-- args…]` | Build and run (or interpret) the project |
| `largo check` | Parse + type-check without producing a binary |
| `largo verify [--license <key>]` | Run Z3 static verification only (Pro+) |
| `largo opts <file> [--json]` | Report which optimizations actually fired for a file |
| `largo publish [flags]` | Package and upload to the registry |
| `largo login [--registry <url>] [--token <key>]` | Store a registry token |
| `largo logout [--registry <url>]` | Remove stored credentials |

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

### `run`

`largo build` followed by executing the binary; the program's exit code is propagated.

| Flag | Effect |
|------|--------|
| `-r`, `--release` | Optimized build before running |
| `-i`, `--interpret` | Skip Rust compilation; execute via the tree-walking interpreter for sub-second feedback |
| `-- <args…>` | Arguments passed through to the program |

### `check`

Parses and type-checks without the full build pipeline — fast validation during development.

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

The default registry is `https://registry.logicaffeine.com`.

---
[Docs index](README.md) · [Root README](../NEW_README.md) · [Changelog](../CHANGELOG.md)
