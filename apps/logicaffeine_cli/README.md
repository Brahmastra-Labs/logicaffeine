# logicaffeine-cli

`largo` is the LOGOS build tool and package manager. The `logicaffeine-cli` crate ships one binary — **`largo`** — that scaffolds projects, compiles LOGOS `.lg` source (English → first-order logic → Rust) into a Cargo binary, runs it natively or on the tree-walking interpreter, type-checks, audits the optimizer, runs optional Z3 verification, and publishes to a registry.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 4. Binary: `largo`. Depends on base, kernel, language, compile (and jit, native-only).

## Role in the workspace

`largo` is the user-facing front of the LOGOS pipeline. All command logic lives in the library crate `logicaffeine_cli`; the binary (`src/main.rs`) is a thin wrapper around `run_cli` that maps errors to exit codes (`0` ok, `1` error to stderr). On native targets `main` installs the copy-and-patch JIT process-wide before dispatch, so every interpreted program gets hot-function / hot-loop tier-up; `LOGOS_NO_JIT=1` skips it (VM bytecode interpreter only), and the JIT is never linked on `wasm32`.

A project is a directory with a `Largo.toml` manifest — `[package]` (name, version, description, authors, `entry`, defaulting to `src/main.lg`) and `[dependencies]` keyed by version, `path`, or `git`. `build` generates a Cargo crate under `target/<mode>/build`, wires the runtime deps (`logicaffeine-data`, `logicaffeine-system` with `full`, `tokio`), and applies an aggressive release profile (LTO, `opt-level = 3`, one codegen unit, `panic = abort`, strip, plus `target-cpu=native` for non-cross release builds).

For the full command / flag / environment reference, see **[new_docs/cli.md](../../new_docs/cli.md)**.

## Commands (overview)

Ten clap subcommands (the `Commands` enum in `src/cli.rs`), dispatched by `run_cli`:

| Command | Purpose |
|---------|---------|
| `new <name>` | Scaffold a project in a new directory (`Largo.toml`, `src/main.lg`, `.gitignore`) |
| `init [--name <n>]` | Same, in the current directory; name defaults to the folder name |
| `build` | Compile `.lg` → Rust, then `cargo build` the generated crate |
| `run [args…]` | Build and execute (or `-i/--interpret` on the tree-walker); exit code propagates |
| `check` | Parse and compile to Rust without producing a binary |
| `verify` | Run Z3 static verification only (Pro+ license) |
| `opts <file>` | Report which optimizations actually FIRED for a `.lg` file |
| `publish` | Package the project as a tarball and upload it to the registry |
| `login` | Store a registry API token (`~/.config/logos/credentials.toml`) |
| `logout` | Remove a stored registry token |

Flags and environment variables (`LOGOS_NO_JIT`, `LOGOS_LICENSE`, `LOGOS_TOKEN`, `LOGOS_CREDENTIALS_PATH`) are documented in [new_docs/cli.md](../../new_docs/cli.md). Default registry: `https://registry.logicaffeine.com`.

## Crate structure / public API

```
src/
├── main.rs        # `largo` binary: JIT install + run_cli wrapper
├── lib.rs         # public API surface
├── cli.rs         # Cli / Commands parser + per-command handlers
├── compile.rs     # re-export of logicaffeine_compile::compile::*
└── project/
    ├── manifest.rs    # Largo.toml: Manifest, Package, DependencySpec
    ├── build.rs       # build orchestration: BuildConfig → BuildResult
    ├── registry.rs    # RegistryClient + tarball packaging
    └── credentials.rs # token storage / lookup
```

Public API (`logicaffeine_cli`):

- `run_cli()` — parse argv and dispatch (also `cli::{Cli, Commands}`).
- `project::{BuildConfig, BuildResult, BuildError, build, run, find_project_root}` — build orchestration.
- `project::{Manifest, ManifestError}` — `Largo.toml` parse / serialize.
- `project::{RegistryClient, create_tarball, is_git_dirty}` — registry client and publishing.
- `project::{Credentials, get_registry_token}` — credential storage and token lookup.
- `project::{Loader, ModuleSource}` — module loader re-exported from the compile crate.
- `compile::*` (including `compile_project`), plus re-exports `interface` (kernel) and `analysis` (compile) for external tooling.

## Feature flags

| Feature | Effect |
|---------|--------|
| `default` | Empty (`default = []`); no extra features are declared. |

The manifest declares only an empty `default`. `src/cli.rs` carries a `#[cfg(feature = "verification")]` gate for the Z3 path (`use logicaffeine_verify::{LicenseValidator, Verifier}`), but this crate's `Cargo.toml` does **not** declare a `verification` feature or depend on `logicaffeine-verify`. As configured, the `#[cfg(not(feature = "verification"))]` branch is active, so `verify` and `build --verify` return "Verification requires the 'verification' feature." Enabling the real Z3 path requires wiring the feature and the optional `logicaffeine-verify` dependency into this manifest.

## Dependencies

Internal (Tiers 0–3): `logicaffeine-base`, `logicaffeine-kernel`, `logicaffeine-language`, `logicaffeine-compile`; plus `logicaffeine-jit` under `cfg(not(target_arch = "wasm32"))` for the native JIT tier.

External: `clap` (derive arg parsing), `ureq` + `serde_json` (registry HTTP/JSON), `flate2` + `tar` (publish tarballs), `toml` + `serde` (manifest), `dirs` (config-dir resolution), `futures` (`block_on` for `run --interpret`), `rand`. Dev: `tempfile`.

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
