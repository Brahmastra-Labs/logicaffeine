# logicaffeine-cli

`largo` is the LOGOS build tool, and it works the way Cargo does for Rust: one `Largo.toml` manifest and one binary that takes a project from `largo new` to a package in the registry. Running a program, translating English to first-order logic, checking a proof, solving a SAT instance and serving a website are all subcommands of that one tool, so there is one front door to learn rather than one per job. Rust is optional: only the compiled build goes through cargo, while the interpreter, the logic and proof commands and `--emit wasm` run without a Rust toolchain.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 6. The crate ships one binary, `largo`, over the `logicaffeine_cli` library; the JIT links only on x86-64 System V (Linux and macOS), and Z3 verification sits behind the `verification` feature.

## Getting started

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

Windows, in PowerShell: `irm https://logicaffeine.com/install.ps1 | iex`. The installer fetches the prebuilt binary for Linux or macOS (x64 and arm64) or Windows x64 and checks its SHA-256 against the release's `SHA256SUMS`. On Linux and macOS it installs to `~/.local/bin`, with no sudo and no shell-config edits. `curl -fsSL https://logicaffeine.com/install.sh | sh -s -- --full` installs the flavor with Z3 verification statically linked, which `largo --version` reports as `(full)`. From source: `cargo install logicaffeine-cli`.

Then make a project and run it:

```bash
largo new hello && cd hello
largo run --interpret   # runs in-process, no Rust toolchain needed
largo run               # compiles to Rust and builds with cargo, so it needs Rust
largo doctor            # reports what this machine has: cargo, the wasm32 target, node, verification, the registry
```

`largo build` needs Rust for the same reason `largo run` does; `largo build --emit wasm` does not.

## Role in the workspace

`largo` is the user-facing end of the LOGOS pipeline. Every command lives in the library crate `logicaffeine_cli`, and the binary (`src/main.rs`) is one call to `largo_main()`. That function runs `run_cli` on a worker thread whose stack is sized from the AST depth limit, renders errors in the `error:`/`help:` style, and exits `0` on success, `1` on failure and `2` on a usage error. A command may set its own code: `sat` uses the SAT Competition's `10`/`20`, `run` exits `122` when its `--max-steps` budget runs out and `124` when its `--timeout` expires, and `lean import` exits `0`–`5` by outcome and `64` for usage.

On x86-64 System V, `largo_main` installs EXODIA, the copy-and-patch JIT, process-wide before dispatch, so the bytecode VM tiers hot functions and hot loops up to native code. `LOGOS_NO_JIT=1` skips the install and leaves the VM on its own; on Windows, aarch64 and `wasm32` the JIT is not linked.

A project is a directory with a `Largo.toml` manifest: `[package]` (name, version, description, authors, an `entry` that defaults to `src/main.lg`, and `site`, a website's public address), `[dependencies]` keyed by version, `path` or `git`, and optional `[test]` and `[web]` sections. `build` generates a Cargo crate under `target/<mode>/build` and publishes the binary at `target/<mode>/<name>`. The generated crate depends on the runtime (`logicaffeine-data`, `logicaffeine-system` with `full` and `http`, and `tokio`): path dependencies on staged sources inside a checkout, and the exact crates.io release of this `largo` everywhere else. Release builds get LTO, `opt-level = 3`, one codegen unit, `panic = "abort"` and stripping, plus `target-cpu=native` unless cross-compiling. Cargo's own colored progress streams through live, and a failure is classified as a `## Requires` dependency problem or a compiler bug in the generated Rust, then framed to match.

Every flag and environment variable is documented in **[docs/cli.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/cli.md)**.

## Commands

The subcommands of the clap `Commands` enum in `src/cli.rs`, each dispatched by `run_cli` to a handler in `src/commands/`:

| Command | Purpose |
|---------|---------|
| `new <name>` / `init` | Scaffold a project (`Largo.toml`, `src/main.lg`, `.gitignore`); `--site` scaffolds a static site, `--web` a full-stack one |
| `build` | Compile `.lg` to Rust, then `cargo build` it with cargo's output streamed live; `--emit c` builds the same executable through C and the system C compiler; `--emit wasm` uses the direct backend instead; a site exports to `dist/` |
| `run [args…]` | Build and execute; `-i/--interpret` runs the program in-process with no Rust build |
| `serve` | Serve the project over HTTP (a site, a full-stack site, or a backend) with `--watch` live reload |
| `check` | Parse and type-check the entry and every page without producing a binary; `--json` for agents |
| `test [filter]` | Run the project's `## Test` blocks, each in an isolated world, with a deterministic report |
| `replay <bundle>` | Re-run a recorded `.replay` bundle: reproduced, fixed, or diverged |
| `bringup <checkpoint>` | Bring a model (a GGUF file or a Hugging Face directory) up on every chip from its one program: the CPU tiers, the CUDA lane, the D5005 decode engine, the SN50 and the fabric, each artifact naming the cut of the program it was lowered from; `--run` runs it on every chip here — a prompt, or every prompt of a golden with `--prompts` — and reports tokens, speed and agreement |
| `expand <file>` | Show the functions an LGX page's components, actions and routes lower to (LGX is the LOGOS web framework) |
| `repl` | Interactive session: imperative statements plus English→FOL logic mode (`:help` inside) |
| `dap` | Debug a program over the Debug Adapter Protocol on stdio, reverse execution included (`stepBack`, `reverseContinue`); it serializes the same `DebugSession` the Studio's debugger renders |
| `explain <topic>` | Look up a diagnostic code or a construct, offline |
| `mcp` | Serve the toolchain to AI agents over the Model Context Protocol (stdio) |
| `logic [sentence]` | English → first-order logic (`--format unicode\|latex\|ascii\|kripke`, `--all-readings`, `--discourse`) |
| `prove [file]` | Kernel-certified proving of `## Theory` developments and `## Theorem` blocks (`--trace`, `--json`) |
| `sat <file.cnf>` | The certified SAT solver; `--proof` writes a DRAT (or DPR/SR) refutation; exits `10`/`20` |
| `lean import` / `lean replay` | Import a Lean 4 declaration as a checked LOGOS package, or re-verify a published one |
| `fmt [paths…]` | Format sources with the canonical rules the language server uses; `--check` for CI |
| `emit <target>` | Print or write a compiled translation without building a binary (every target: [docs/cli.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/cli.md#emit)) |
| `doc [--out DIR]` | Generate markdown documentation from a project's `##` blocks |
| `add <spec>` / `remove <name>` | Edit `Largo.toml` dependencies, preserving its formatting (`toml_edit`) |
| `clean [--all]` | Remove `target/` (and `.logos-native/` with `--all`) |
| `opts <file>` | Report which optimizations fired for a `.lg` file |
| `verify` | Run Z3 static verification only (Pro license or above; `verification` feature) |
| `completions <shell>` | Shell completion scripts (bash, zsh, fish, powershell, elvish) |
| `doctor` | Diagnose the environment: whether `largo` is current, cargo, the wasm32 target, node for `--emit wasm`, the verification flavor, the registry and stored credentials, and manifest health; works offline |
| `publish` / `login` / `logout` | Registry packaging, upload, and credentials |

Global flags on every command: `-q/--quiet`, `-v/--verbose`, `--color auto|always|never` (`NO_COLOR` is respected). The environment variables (`LOGOS_NO_JIT`, `LOGOS_TOKEN`, `LOGOS_CREDENTIALS_PATH`) are documented in [docs/cli.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/cli.md). Default registry: `https://registry.logicaffeine.com`.

## Public API

The binary is a thin shell over the `logicaffeine_cli` library, which exposes:

- `largo_main()` — the whole `largo` process: the native tier, the sized worker, error rendering, the exit code. `run_cli()` parses argv and dispatches (also `cli::{Cli, Commands}`).
- `ui::{CliError, render_error, ColorMode}` and the exit-code constants — the error and exit contract shared with the binary.
- `project::{BuildConfig, BuildResult, BuildError, CargoFailure, CargoFailureKind, classify_cargo_failure, build, run, find_project_root}` — build orchestration.
- `project::{Manifest, ManifestError}` — `Largo.toml` parse and serialize.
- `project::{RegistryClient, create_tarball, is_git_dirty}` — registry client and publishing.
- `project::{Credentials, get_registry_token}` — credential storage and token lookup.
- `project::{Loader, ModuleSource}` — the module loader, re-exported from the compile crate.
- `project::shape` / `project::deps` — the project-shape classifier and the dependency resolver.
- `compile::*` (including `compile_project`), plus the re-exports `interface` (kernel) and `analysis` (compile) for external tooling.
- `dap::DapServer` — the pure DAP transition behind `largo dap` (`handle`: one request in, responses and events out).
- `mcp::{McpServer, cmd_mcp}` — the MCP server behind `largo mcp`. Its pure `handle_line` JSON-RPC transition serves the check/run/logic/prove/emit/opts tools and the embedded learning-corpus resources; agents register it with `claude mcp add logos -- largo mcp`.
- `browser::{Browser, Page, PageBundle, Snapshot, ChromeLauncher}` — the headless-Chrome page tier behind `largo test --browser`. It is a DevTools-protocol client over `--remote-debugging-pipe` that serves a built page from a loopback HTTP server, then drives it with trusted input events, virtual time, stubbed fetches, a host-call trace and canonical document snapshots. `ChromeLauncher` adapts it to `page_host::RemotePageLauncher`, the seam that `largo test --browser` and the tree-walker/VM/Chrome corpus lock drive.

## Source layout

```text
src/
├── main.rs        # `largo` binary: exactly `largo_main()`
├── lib.rs         # public API surface, `largo_main`, the web-serving helpers
├── cli.rs         # Cli / Commands parser + dispatch
├── ui.rs          # CliError, exit codes, color state, clap palette, phase headers
├── diagnostics.rs # every diagnostic placed in the author's file: terminal, `--json`, dev-server overlay
├── closure.rs     # the entry's assembled import closure (and the files behind it) every command compiles
├── commands/      # one module per verb (build, run, logic, prove, sat, fmt, …)
├── repl/          # the interactive session: loop, meta-commands, multiline, editor
├── mcp/           # `largo mcp`: the Model Context Protocol server (JSON-RPC/stdio, tools, resources, run sandbox)
├── dap/           # `largo dap`: the Debug Adapter Protocol server over `DebugSession`
├── browser/       # the headless-Chrome page tier behind `largo test --browser`
├── compile.rs     # re-export of logicaffeine_compile::compile::*
└── project/
    ├── manifest.rs    # Largo.toml: Manifest, Package, DependencySpec
    ├── build.rs       # build orchestration: BuildConfig → BuildResult (+ cargo failure classifier)
    ├── deps.rs        # `[dependencies]` path and git (exact rev) entries → the loader's dependency roots
    ├── shape.rs       # project shape: program, backend, site, or full-stack site
    ├── registry.rs    # RegistryClient + tarball packaging
    └── credentials.rs # token storage / lookup
assets/agent/          # the agent pack `largo mcp` embeds as resources (quick guide, docs, examples)
templates/AGENTS.md    # the agent guide `largo new` writes into a project
```

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `verification` | off | `largo verify` against the system Z3 (dynamic link): the optional `logicaffeine-verify` dependency plus `verification` in `logicaffeine-compile` and `logicaffeine-proof`. |
| `verification-static` | off | `verification` with Z3 built from its vendored source and statically linked (`logicaffeine-verify/static-link-z3`): the release `largo-full` flavor. Needs cmake, a C++ toolchain and libclang. |

Without either feature, `largo verify` is a stub that points at the `--full` installer. `largo --version` reports `(full)` when verification is compiled in.

## Dependencies

- **Internal:** `logicaffeine-base`, `logicaffeine-kernel` (`lean`), `logicaffeine-language`, `logicaffeine-compile` (`wasm-jit`), `logicaffeine-proof` (sat/prove), `logicaffeine-system`; `logicaffeine-verify` (optional, `verification`); plus `logicaffeine-jit` under `cfg(all(target_arch = "x86_64", not(target_os = "windows")))` for the native JIT tier.
- **Command line:** `clap` + `clap_complete` (argument parsing, completions), `anstream` + `anstyle` (color that strips itself on pipes and honors `NO_COLOR`), `rustyline` (REPL line editing), `futures` (`block_on` for interpret and the REPL), `rand`.
- **Projects and the registry:** `toml` + `toml_edit` + `serde` (manifest read and edit), `ureq` + `serde_json` (registry HTTP and JSON), `flate2` + `tar` (publish tarballs), `dirs` (config-dir resolution).
- **Headless Chrome:** `base64` (CDP fetch bodies) and `libc` on Unix (the Chrome pipe).
- **Dev:** `tempfile`, `libc`.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/apps/logicaffeine_cli/CHANGELOG.md)
