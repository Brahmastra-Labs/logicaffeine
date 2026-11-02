# The `largo` CLI

One binary should take you from an empty directory to a running program, a deployed site or a
checked proof, without a second tool to learn. `largo` is the LOGOS build tool and package manager:
Cargo for English, and the front door to the whole engine. It scaffolds projects, compiles LOGOS to
Rust (and on to a native binary) or directly to WASM, and runs programs through the interpreter. It
also hosts the interactive REPL, translates English to First-Order Logic, proves theorems
kernel-certified, solves DIMACS SAT instances, formats source, performs static verification, and
talks to the package registry.

This page is the reference: how to install `largo`, every subcommand and its flags, the
`Largo.toml` manifest, and the environment variables `largo` and a running program read.

## Installing

One line on Linux and macOS (x64 + arm64):

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

Windows (x64):

```powershell
irm https://logicaffeine.com/install.ps1 | iex
```

The installer downloads the prebuilt binary for your platform from the GitHub release, verifies
its SHA-256 against the release's `SHA256SUMS`, and installs it atomically as `largo` in
`~/.local/bin` (Windows: `%LOCALAPPDATA%\Programs\largo`). It never uses sudo and never edits
shell configuration; if the directory isn't on your `PATH`, it prints the exact line to add.

| Flag | Effect |
|------|--------|
| `--full` | Install the full build: Z3 static verification bundled (`largo verify` works out of the box). ~30 MB larger. |
| `--version vX.Y.Z` | Install an exact release instead of the latest. |
| `--to DIR` | Install directory override (also: `LARGO_INSTALL_DIR`). |

Both flavors install as `largo`; `largo --version` reports `X.Y.Z (full)` for the full build.
Prebuilt platforms: `linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, `win32-x64`.
Re-running the installer upgrades in place. Uninstall by deleting the binary.

`largo run --interpret` and `largo run --emit wasm` (which runs its module through Node.js) need no
Rust toolchain, while a plain `largo build` or `largo run` of a program compiles through cargo and
needs Rust; `largo doctor` reports what your machine has.

### From source

```bash
cargo install logicaffeine-cli           # needs Rust 1.88 or newer
# or, in the repo:
cargo build -p logicaffeine-cli          # debug build → target/debug/largo
cargo build -p logicaffeine-cli --release
```

## Commands

The full subcommand set (from the `Commands` enum in `cli.rs`):

**Project**

| Command | Purpose |
|---------|---------|
| `largo new <name> [--site\|--web]` | Scaffold a new project in a new `<name>/` directory — a program, a site, or a full-stack site |
| `largo init [--name <name>]` | Scaffold a project in the current directory |
| `largo add <spec> [--path <dir>\|--git <url>]` | Add a dependency to `Largo.toml` (format-preserving edit) |
| `largo remove <name>` | Remove a dependency from `Largo.toml` |
| `largo clean [--all]` | Delete `target/` (`--all` also clears the `.logos-native/` cache) |

**Build & run**

| Command | Purpose |
|---------|---------|
| `largo build [flags]` | Compile LOGOS → Rust, then `cargo build` the result; a site exports to `dist/`; `--emit` builds a module or assembly directly; `--manifest` describes the linked build |
| `largo run [flags] [-- args…]` | Build and run — compiled, in-process (`--interpret`), or as WASM; `--max-steps` / `--timeout` / `--seed` / `--json` bound and reproduce runs |
| `largo check [--deep] [--json] [--kernel <file>]` | Parse + type-check the entry and every page without producing a binary |
| `largo serve [--watch] [--port <n>] [--open] [--host <addr>] [--timeout <dur>] [--seed <n>] [-- args…]` | Serve the project over HTTP — a site, a full-stack site, or a backend, decided by the project itself |
| `largo emit <target> [file] [-o path]` | Print/write a compiled translation without building a binary (every target: [emit](#emit)) |
| `largo fmt [paths…] [--check] [--json]` | Format LOGOS sources (same rules as the language server) and sort each import block |
| `largo opts <file> [--json]` | Report which optimizations fired for a file, and which enabled or preempted which |
| `largo replay <bundle> [--verify]` | Re-run a recorded `.replay` bundle and report reproduced, fixed, or diverged |
| `largo bringup <checkpoint> [--target <chips>] [--ctx <n>] -o <dir>` | Bring a model up on every chip from its one program: the CUDA lane, the D5005 decode engine, the SN50 |
| `largo expand <file>` | Show what an LGX page became: the functions its components, actions and routes lower to |

**Agents**

| Command | Purpose |
|---------|---------|
| `largo explain <topic> [--json]` | Look up a diagnostic code or a construct, offline |
| `largo mcp` | Serve the whole toolchain over the Model Context Protocol (stdio) |
| `largo dap` | Speak the Debug Adapter Protocol, including real reverse execution |

**Logic, proof & SAT**

| Command | Purpose |
|---------|---------|
| `largo logic "<sentence>" [flags]` | English → First-Order Logic from the terminal |
| `largo prove [file] [--trace\|--json\|--certificate]` | Prove `## Theory` / `## Theorem` blocks, kernel-certified; `--kernel <file>` checks a kernel-vernacular document |
| `largo sat <file.cnf> [--proof <path>] [--stats]` | The certified SAT engine on DIMACS CNF (exit 10/20) |
| `largo lean import --project <dir> --target <Name> --out <dir>` | Lean 4 → LOGOS: import a Lean declaration as a checked, independently re-verified LOGOS package |
| `largo lean replay <package>` | Re-verify a published Lean → LOGOS package in a fresh process |
| `largo verify` | Run Z3 static verification only (the full build) |

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

**Testing**

| Command | Purpose |
|---------|---------|
| `largo test [filter] [flags]` | Run the project's `## Test` blocks — the entry's and every page's — each in a fresh isolated world |

Every command takes the global flags `-q/--quiet`, `-v/--verbose` (repeatable), and
`--color auto|always|never` (`NO_COLOR` respected).

### `new` / `init`

`new` creates `<name>/` containing a `Largo.toml`, a `src/main.lg` entry point with a hello-world
example, a `.gitignore`, and an `AGENTS.md` that teaches AI agents the check/run loop and the
classic LOGOS mistakes. `init` writes the manifest into an existing directory, plus `src/main.lg`
and `AGENTS.md` where they are absent, and refuses a directory that already holds a `Largo.toml`
(the package name defaults to the directory name, or `--name`).

```bash
largo new my_project
cd my_project
largo run
```

Two flags scaffold a website instead, laid out by convention — there are no extra manifest keys
to learn. `pages/` holds the pages (`## Page /path`, one WebAssembly bundle each); `static/` is served
verbatim at root-relative paths. The front page, `pages/app.lg`, is a `## Page /` that places one
`## Component` and carries a passing `## Test`.

- `--site` is a site and nothing else: `pages/` and `static/`, no backend, and no entry program — a
  page mounts itself, and the manifest carries no `entry`. `largo build` exports it to `dist/` for
  any static host.
- `--web` is a full-stack site: the same pages and assets, plus a backend in `src/main.lg` — one
  `## Route` per path, ending in `Serve on port 8080.`

```bash
largo new mysite --site
cd mysite
largo serve
```

### `build`

Compiles the LOGOS source to Rust, then invokes `cargo build` on the generated crate, and publishes
the binary at `target/debug/<name>` (or `target/release/<name>`) — the file it reports and the file
`largo run` runs. A full-stack site's binary carries its pages and `static/` inside it, served before
its routes, so it deploys as one file.

A **site** — pages and assets with no `Serve` backend — has no program to compile, so a plain
`largo build` exports it, exactly as `--static` does, and says so.

| Flag | Effect |
|------|--------|
| `-r`, `--release` | Optimized build |
| `--verify` | Run Z3 static verification after compilation (the full build) |
| `--lib` | Generate a library (`lib.rs`, `crate-type = ["cdylib"]`) instead of a binary |
| `--target <triple>` | Cross-compile; `wasm` is shorthand for `wasm32-unknown-unknown` |
| `--native-functions` | Pre-build every `is exported for native` function — the entry's and every imported module's — into a cached cdylib under `.logos-native/`; a function outside the native scalar subset is skipped and keeps running on the VM and JIT. `largo run --interpret` loads the bundle |
| `--emit c` | Build the same executable through C instead of Rust: the program lowered to C and compiled by `$CC` (default `cc`), published at the same `target/debug/<name>` or `target/release/<name>`, the generated source kept in `target/<mode>/build-c/main.c`. Debug is `-O0 -g`; `--release` is `-O3 -march=native -flto -ffp-contract=off`, the flags the benchmark suite builds its hand-written C with, and strips like the Rust release build unless `CARGO_PROFILE_RELEASE_STRIP=false`. A program the exact extension reaches also links that extension's value runtime, a static library built once from the same Rust the interpreter runs. `-v` prints the compiler command |
| `--emit c-bytecode` | As `--emit c`, lowering the VM's bytecode to C instead of the optimized syntax tree |
| `--emit wasm` | Compile straight to a self-contained `target/<name>.wasm` (plus its `<name>.mjs` Node.js host shim) via the built-in backend: no rustc, cargo, or wasm-bindgen in the loop; milliseconds. A program the exact extension reaches is refused by name: its values need the runtime `wasm-linked` links |
| `--emit wasm-linked` | As `--emit wasm`, but links the real Rust runtime via `rust-lld`, so overflowing integer arithmetic computes the exact big number instead of wrapping and a program the exact extension reaches runs on that extension's value runtime (needs the Rust toolchain + a wasm32 build of the runtime) |
| `--emit isa` | Compile the entry to LOGOS Silicon ISA assembly — the register VM's opcode set as a chip ISA, each instruction annotated with its reversibility contract — written beside the entry as `<entry>.isa`. No rustc |
| `--emit riscv` | Compile the entry to RV64GC assembly, written beside the entry as `<entry>.s` with the `logos_rt.c` runtime shim; assemble and link with a RISC-V GCC and run under qemu. A program the exact extension reaches also links that extension's value runtime, built for RISC-V (`riscv64gc-unknown-linux-gnu`) and written beside the assembly as `liblogos_value_runtime_riscv64.a`; the command that links them is printed. `risc-v` is accepted too |
| `--manifest` | Print the build manifest as JSON instead of building: the package identity, the feature set, every resolved `path`/`git` dependency (a git one with its `url` and `rev`), the transitive closure (every linked `.lg` file tagged with the dependency it belongs to), and the registry state (never accessed: a local closure reaches only the filesystem). No cargo, no rustc |
| `--static` | Export the site to `dist/` as plain files — pre-rendered pages (`/about` as `about/index.html`), the content-addressed modules, `static/**`, `404.html` for an `## Error page 404`, `robots.txt`, a `sitemap.xml` when `Largo.toml` declares `site`, `_headers` with the binary's cache rules, and `_redirects` for parameterized pages. No cargo, no rustc, no backend |
| `--json` | One `logos-check/1` envelope on stdout: a compile error (in the entry or a page) as a diagnostic at its `file:line:col`, a cargo failure as one `build-failure` diagnostic |

### `run`

`largo build` followed by executing the binary; the program's exit code is propagated.

| Flag | Effect |
|------|--------|
| `-r`, `--release` | Optimized build before running |
| `-i`, `--interpret` | Skip Rust compilation and run in-process for sub-second feedback: the bytecode VM with its JIT tiers, or the tree-walking interpreter for a concurrent or asynchronous program and for any construct the VM compiler does not take. A program declaring `## Require <opt>` is refused on this path; the compiled path (`largo run`, `largo build`) is the one that checks it |
| `--engine jit` \| `vm` \| `tree` | The in-process engine (`--interpret` only): `jit`, the default, is the bytecode VM with JIT tier-up where the platform has one; `vm` the bytecode VM alone; `tree` the tree-walking interpreter. Every engine runs the same optimized program |
| `--emit c` \| `c-bytecode` | Build through C (as `largo build --emit c`), then run the binary |
| `--emit wasm` \| `wasm-linked` | Compile directly to `.wasm` (built-in backend) and run it through the emitted Node.js host shim — compile-and-run in one step |
| `--max-steps <N>` | Stop the run after N engine steps — deterministic (the identical op on every run, JIT tiering suspended so nothing escapes the count). Exhaustion exits **122** with the partial output delivered. The safety rail for running generated code. Needs `--interpret`: the compiled path refuses it |
| `--timeout <dur>` | Kill the run (`500ms`, `10s`, `2m`) after the wall clock expires; works on both the interpreter and the compiled child. Expiry exits **124** (the GNU timeout convention) |
| `--seed <N>` | Set `LOGOS_SEED` for this run (and any compiled child): concurrent scheduling reproduces byte-for-byte |
| `--json` | Print one `logos-run/1` envelope on stdout: `outcome` (`ok` \| `runtime-error` \| `budget-exceeded` \| `timeout`), `lines`, `error`, `steps_used`. Needs `--interpret`: the compiled path refuses it |
| `-- <args…>` | Arguments passed through to the program |

A failing `--interpret` run also writes a `.replay` bundle under `target/replays/` (see
[`replay`](#replay)).

### `serve`

Serves the project over HTTP. What stands up is decided from the project itself, so there is no mode
flag to get wrong:

- a **site** — `pages/` or `static/`, no `Serve` backend — gets the dev server: largo owns the public
  port and serves the built pages and assets from memory. A request nothing in the site answers gets
  a `404` (the site's own `## Error page 404` when it declares one), exactly as a static host
  answers it;
- a **full-stack site** — the same, plus the entry's `Serve` backend running as a child process;
  every request the pages and assets do not answer is proxied to it;
- a **backend** — a `Serve` program alone — is interpreted directly and binds its own port;
- any other program that compiles to WebAssembly is served as a bundle (`--watch` rebuilds it on
  save); one that does not is run by the interpreter.

Assets win over routes on an exact path match, in development and in a release binary alike — one
rule, so what you see locally is what ships. A site always watches: editing a page or an asset
rebuilds it in memory and reloads the browser, and editing the backend restarts the child. A page
that does not compile serves its error overlay in the browser and prints the same error, at its
`file:line:col`, in the terminal. `/__dev` lists every page, its size, and what is failing.

A site or a bundle takes its public port from `--port` (default 8080) and falls forward through the
next 15 ports when it is taken. `--host 0.0.0.0` serves a site to the whole local network (a phone on
the same wifi can open it, and the address it types is printed); the default is this machine only, and
a bundle always binds `127.0.0.1`. `--open` opens the served URL in the default browser. A backend
takes none of the three: the port its `Serve` statement names decides, overridden by `LOGOS_PORT` or
`PORT`, and `LOGOS_HOST` picks the interface (see [Environment variables](#environment-variables)).
`--timeout <dur>`, `--seed <n>` and the arguments after `--` go to a program the interpreter runs — a
backend, or a program that does not compile to WebAssembly. A full-stack site's backend child is
spawned on a largo-chosen port without them.

```bash
largo serve
largo serve --port 3000 --open
largo serve --host 0.0.0.0
```

### `check`

Parses and type-checks without the full build pipeline, for fast validation during development. It
checks the entry program and every page in `pages/`, each on the tier it ships on, and reports every
error at the author's own `file:line:col` — never at a line of the combined source an `Import`
spliced together. Accessibility and template warnings (an `<img>` with no `alt`, a button that
announces nothing) and value-semantics copy warnings (a local copied from a field, mutated, and never
written back) are printed the same way and do not fail the check. A construct `largo build` would
refuse in the entry's binary fails the check as a `deploy-refusal`, so a program cannot check clean
and then not build; a program that touches the DOM ships as WebAssembly and is exempt. After a clean
compile without `--deep`, the entry's imports are linted: an unused, duplicate or self import is reported as
`unused-import`, `duplicate-import` or `self-import`.
`--deep` also runs rustc's analysis over the generated code of the entry's whole assembled closure,
imported modules included (the same pass the IDE's flycheck uses), and translates its findings back
to LOGOS terms at the file they came from. A site with no entry program has nothing for it to check.

`--kernel <file>` checks a LOGOS kernel-vernacular literate document (the form `largo lean import`
emits) through the kernel's own document door instead of the project: every block in order, stopping
at the first that does not check and naming it. With `--json` it prints one report instead: each
block's output, or `failed_block` and its `error`. `largo prove --kernel <file>` is the same check.

`--json` prints one `logos-check/1` envelope on stdout — `ok` plus `diagnostics`, each carrying
a stable kebab-case `code` (see [ERROR_CATALOG.md](ERROR_CATALOG.md)), byte and line/column
spans, a one-line `message`, the full Socratic `explanation`, and a `suggestion` when a close
known word exists. The human prose keeps flowing to stderr; exit codes are unchanged. This is
the same diagnostic object the MCP `logos_check` tool returns. A page's errors and its warnings are
in the envelope too, each naming the page's file. `largo build --json` speaks the
same envelope (a cargo-phase failure reports as one `build-failure` diagnostic).

```bash
largo check --json
largo check --json | jq -r '.diagnostics[0].code'
largo check --kernel Demo.andSwap.lg
```

### `opts`

Compiles a `.lg` file with every optimization on and the firing trace recording, on each path a
program can take — the compiled path (`largo build`), the run-path optimizer and the VM compile
(`largo run --interpret`) — then lists the optimizations that *genuinely changed* the program (not
merely the ones enabled), grouped by category, each with the paths it fired on. One that replaced the
program's algorithm rather than speeding it up — memoization, a closed form, a symmetry cut, a map
turned into an array, work done at compile time — is marked as such. The dependencies follow (one
fired only because another was on), then the blockers (one took precedence and skipped another),
both from the compiled path. `--json` emits `{"fired": […], "paths": {"compiled": […], "run": […],
"vm": […]}, "changes_algorithm": […], "blockers": [[winner, loser], …], "dependencies": [[dependent,
dep], …]}`.

```bash
largo opts src/main.lg
largo opts src/main.lg --json
```

### `verify`

Runs Z3 static verification of the project's logical constraints without building (the full build
carries Z3). The subject is the entry and
every module it imports, through escape analysis, the Z3 pass and the type checker; `pages/` are not
verified (`largo check` compiles them), and a program the compiler refuses is reported as unverifiable
rather than passed. See [Proof & verification](proof-and-verification.md).

> The verifier ships behind a build-time gate: a default `largo` build answers `verify` (and
> `build --verify`) with a notice that verification has to be compiled in. It pulls in the
> Z3-backed [`logicaffeine_verify`](../crates/logicaffeine_verify/README.md) crate, kept out of the
> default build so the common path needs no Z3 toolchain.

### `publish` / `login` / `logout`

`publish` packages the project as a tarball and uploads it. Pre-flight checks confirm an auth token
is available (`largo login`, or `LOGOS_TOKEN`), the entry point exists, and the git working directory
is clean (override with `--allow-dirty`). `--dry-run` validates without uploading; `--registry <url>`
overrides the default.

`login` stores an API token (interactive prompt, or `--token`); `logout` removes it. Tokens live in
`logos/credentials.toml` under the platform's configuration directory (`~/.config/logos/credentials.toml`
on Linux), or wherever `LOGOS_CREDENTIALS_PATH` points.

### `emit`

Prints or writes a compiled translation of the program without building a binary. Without a file
argument it uses the current project's entry; with one, it works on any standalone `.lg`/`.md`
source. The source is compiled together with the modules it imports. What `-o` names depends on
the target:

- for the text targets, the output file (without it they print to stdout);
- for the wasm targets, the module path, with the host files beside it (without it a standalone
  file's module is written beside the file and a project's at `target/<name>.wasm`);
- for `tenstorrent`, `cuda`, `analog`, `plasticine` and `rdu`, the bundle directory (without it
  they print their primary artifact).

| Target | What it produces |
|--------|------------------|
| `rust` | The generated Rust program — what `largo build` hands to cargo |
| `c` | The C translation: one self-contained C file with its runtime embedded. A program the exact extension reaches links that extension's value runtime; with `-o`, its archive is written beside the C as `liblogos_value_runtime.a`, and the command that links the two is printed |
| `riscv` | RV64GC assembly for the RISC-V ahead-of-time tier; with `-o`, the `logos_rt.c` runtime shim is written beside it, and for a program the exact extension reaches, that extension's value runtime built for RISC-V as `liblogos_value_runtime_riscv64.a`, with the command that links them |
| `wasm` | A self-contained module plus a Node.js host shim (`.mjs`), no toolchain needed |
| `wasm-linked` | A module linked against the real Rust runtime (`rust-lld`): big integers, exact numbers, and the exact extension's values |
| `wasm-dom` | A DOM app plus a browser host (`.html` + `.js`) that wires the DOM imports and events |
| `tenstorrent` | A single-core TT-Metalium bundle: `logos_compute_kernel.cpp`, `logos_tt_host.cpp`, `CMakeLists.txt` and `logos_oracle.c`; bare, the kernel |
| `cuda` | A single-file CUDA program: `logos_cuda.cu` (the kernel and its host launcher), a `Makefile` and `logos_oracle.c`; bare, the `.cu` |
| `analog` | A mixed-signal bundle for a modeled FPAA: `netlist.sp` (a behavioral ngspice netlist), `schedule.json` (the gain schedule), `budget.json` (the itemized error budget) and `cert.json` (the per-net range certificate); bare, the netlist |
| `plasticine` | An open reconfigurable-dataflow bundle for the first function, a counted loop: `Logos_<fn>.scala` (Spatial source), `Logos_<fn>_test.scala` (the same loop as a regression-harness app checking the VM's answer), `placement.json` and `report.json` (the modeled report row); bare, the Spatial source |
| `rdu` | The SambaNova RDU: a certified neutral placement, SDK-gated for the vendor config — `neutral.json`, `placement.txt`, `certificate.json`, `report.json`, plus the `--format`'s files; bare, the placement text (or the format's artifact) with the lane and chip on stderr |

The `tenstorrent` and `cuda` bundles carry `logos_oracle.c`: the program's C-tier translation that
the device code is derived from, buildable on the host as the reference answer.

`rdu` picks its lane from the program's shape (`tensor`, `loop` or `whole-program`) and prints it
first, so a refusal names the lane that refused. Its dials, each refused by name on any other
target:

| Flag | Effect |
|------|--------|
| `--chip <chip>` | The RDU generation: `sn40l` or `sn50` (the default) |
| `--format <format>` | The artifact printed bare or added to the bundle: `placement` (the default), `spatial` (`spatial.scala`), `sambaflow` (`model.py`, `compile.py` and `manifest.json` — a PyTorch module and its SambaFlow compile scaffold), `vllm` (`vllm_plan.json`, the Qwen3.8 deployment plan for the chip's vLLM backend) or `report` (the modeled report). The `sambaflow` and `vllm` artifacts are written before the lowering, since neither derives from it |
| `--bridge <name>` | Hand the certified artifact to an SDK bridge — `mock` or `sambaflow` — adding `vendor_config.json` (or `.bin`) and `bridge.json`, which records the certificate digest |
| `--model <model>` | Lower a whole model instead of a file: `qwen38` (Qwen3.8-27B, every layer), `qwen38-tiny`, `qwen38-mini` (the tiny shape with heads that split over two or four chips), or a Qwen3.5/3.8 checkpoint's `.gguf`, whose program is read from the checkpoint. One certified program per layer kind (a directory each holding `neutral.json`, `placement.txt` and `certificate.json`) plus the head, the model constants, `program.lg` (the model's program the bundle deploys) and a `manifest.json` binding every layer and naming the cut of the program each part was lowered from (`LOGOS program: <cut>, sha256 <hex>`). Needs `-o DIR`, takes no file |
| `--ctx <n>` | With `--model`: the decode deployment with a KV cache of `n` positions — an embedding program, layer kinds whose attention appends to and reads the cache, and a head that emits the next token. Without it the bundle is the authored forward |
| `--batch <n>` | With `--model --ctx`: `n` sequences per decode step, every weight read once for all of them; `auto` takes the most sequences whose per-step working set stays in a chip's SRAM. The default is one |
| `--tp <n>` | With `--model --ctx`: tensor-parallel over `n` chips, each with its own heads, FFN columns and weight slice; `n` must split the model's heads (Qwen3.8-27B: 1, 2, 4 or 8). `auto` takes the widest width the heads admit within the chip's scale-up domain. The default is one chip |
| `--prefill <n>` | With `--model --ctx`: prefill a chunk of `n` prompt tokens of one sequence per step, into the decode bundle's state; `n` must not exceed the context, and a prefill bundle is one sequence on one chip |

```bash
largo emit rust                       # generated Rust on stdout
largo emit c standalone.lg            # the C translation of one file
largo emit wasm-linked -o dist/app.wasm
largo emit wasm-dom pages/app.lg -o dist/app.wasm
largo emit cuda -o cuda_out/
largo emit rdu --chip sn40l kernel.lg -o rdu_out/
largo emit rdu --model qwen38 --ctx 4096 --tp 8 -o qwen38_sn50_tp8/
```

### `bringup`

Brings a model up on every chip from its one program. The checkpoint is a GGUF file or a Hugging Face directory
(`config.json` beside its safetensors, the repository as the Hub serves it); its spec is read from its own metadata, and
its computation is written once, as a LOGOS program (`program.lg` in the output directory). A directory and the GGUF
converted from it write the same program. Each backend asked for (`--target`,
comma-separated; every one when absent) lowers a cut of that program:

| Chip | What it writes under `<dir>/<chip>/` | What `--run` runs it on |
|---|---|---|
| `vm` | `decode.bytecode`, the decode step as the bytecode the VM runs | this process, on the bytecode VM |
| `jit` | `decode.bytecode`, the same bytecode | this process, the VM with its native tier |
| `c` | `decode.c`, the decode step as one C program | its generation program built by `cc` (or `$CC`) once and run |
| `rust` | `decode.rs`, the decode step as one Rust program | its generation program built in release into one staged project and run |
| `riscv-qemu` | `decode.s`, the decode step as RISC-V assembly | its generation program linked by `riscv64-linux-gnu-gcc` once and run under `qemu-riscv64` |
| `wasm` | `decode.wasm` and `decode.mjs`, its host | its generation program's module under `node` |
| `cuda` | `lane.cu`, the decode step and its prefill chunks as one fused CUDA program, and `lane.side`, the constants it reads | the lane built by `nvcc` for this machine's GPU |
| `d5005` | the decode engine's program: `commands.txt`, `words.hex` (the sequencer's words), `program.lg` and `program.txt` | the engine's machine model, which runs the card's jobs over the card's memory image |
| `sn50` | the certified decode bundle: a program per layer kind, the embedding and the head, `manifest.json` and `program.lg` | nothing: the SN50 runs under SambaNova's gated SDK |
| `fabric` | nothing yet: the fabric does not lower a model the author writes yet | — |

Every artifact names the cut of the program it was lowered from, `LOGOS program: <cut>, sha256 <hex>`, so artifacts from
different chips can be checked to be the same program. A backend that does not run the model refuses it by name
(the D5005 engine has no unit for experts). `--ctx` sets the positions the decode step's caches hold (4096 unless
given).

`--run` then runs the model on every chip that brought it up. A CPU tier builds one generation program, once, and runs
it once: every weight declared and read once at run time from a file of its values (`readReals`), each sequence's
state (the caches, memories and rings the step carries) declared afresh, and the decode step's own statements in a loop
over the positions — the prompt fed a token a step, then each token the step generates. The program it ran is
`<dir>/<chip>/run/generation.lg`. The CUDA lane is built once and runs each sequence; the D5005 runs its engine's
machine model. Each chip's verdict gains a line, `ran:` with its tokens, its tokens a second (prompt to last token)
and how many of them lead the reference's before the first that does not, or `did not run:` with the reason by name —
a tool or a device this machine lacks, or the work a run waits for. The reference is the model's f64 reference
generating from the prompt (`--prompt`, token ids from 0, `5,17,2,30` unless given; `--tokens`, 8 unless given), or a
golden (`--golden`): its first prompt, and its continuation as the tokens to agree with. `--prompts N` (or `all`) runs
the golden's first N prompts instead, each from an empty state and judged against its own continuation: the `ran:`
line totals them, and a `sequence` line a prompt gives its tokens and agreement. `--tokens` then caps every
sequence.

```bash
largo bringup models/qwen3.8-27b/Qwen3.8-27B-Q4_K_M.gguf -o qwen38/
largo bringup llama.gguf --target cuda --ctx 1024 -o llama/
largo bringup qwen3.5-0.8b.gguf --target cuda --run --golden benchmarks/models/qwen3.5-0.8b/golden-f32.json -o qwen35/
largo bringup models/qwen3.5-0.8b/hf --target vm,sn50 -o qwen35-hf/
largo bringup qwen3.5-0.8b.gguf --target jit,c --run --golden benchmarks/models/qwen3.5-0.8b/golden-f32.json --prompts all -o qwen35/
```

### `expand`

Shows what an LGX page became. A `## Component` is not run as written: it lowers to a render
function, an action function per action, a writer per bound input, a mount entry, a server-side HTML
builder and a dispatcher; a `## Route` lowers to a function plus the synthesized
`route(method, path)` walk; a serialized record gains derived `toJson_` and `fromJson_` functions.
When a page behaves in a way its markup does not explain, the lowering is the answer — this prints
it, with the calls each synthesized function makes. The file is expanded with every module it imports; a page that does not parse, or that the
browser tier refuses, is reported at its `file:line:col`.

```bash
largo expand pages/app.lg
```

### `replay`

A failing `largo run --interpret` writes a self-contained bundle to
`target/replays/<entry>-<unix-time>.replay`: the embedded source, the scheduler seed and decision
trace, the input transcript, and the failure coordinate. Capture re-executes the program on the
deterministic engine and writes nothing when that engine does not reproduce the failure.
`largo replay` re-executes it deterministically and reports one of three verdicts — the recorded failure
still reproduces byte-identically, it no longer fires (fixed), or the run diverged (the program or the
bundle changed since recording). Reproduced and fixed are both coherent; `--verify` turns divergence into
a non-zero exit. A bundle is portable and committable: checked into a project's test corpus, it is a
permanent reproduction of the bug it recorded.

```bash
largo replay target/replays/main-1720000000.replay
largo replay bug.replay --verify
```

### `fmt`

Applies the canonical style (4-space indentation, no tabs, no trailing whitespace) — the exact
rules the language server's formatting provider uses, so the CLI and the editor can never disagree.
Each file's import block is sorted and deduplicated first. Without paths it formats every `.lg` and
`.md` file under `src/` and `pages/` plus the manifest entry; with paths, exactly those files.
`--check` writes nothing and exits 1 if anything would change (the CI gate). `--check --json` prints
one `logos-fmt/1` envelope (`clean` + `unformatted`) instead of the plain file list; `--json` without
`--check` is refused.

### `add` / `remove`

Format-preserving `Largo.toml` edits (comments and layout survive). `add` accepts `name` (any
version), `name@version`, or `logos:name` (the registry URI form); `--path <dir>` and `--git <url>`
record local and git dependencies. `--git` records the URL alone, and a git dependency builds only
once its exact `rev` is added (see [the manifest](#the-largotoml-manifest)). `remove` deletes the named
`[dependencies]` entry and leaves the rest of the manifest byte-identical.

### `clean`

Deletes the project's `target/` directory. `--all` also removes the `.logos-native/`
compiled-function bundle cache produced by `largo build --native-functions`.

### `repl`

The interactive session, two modes in one. `logos>` runs imperative statements against a
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
never counts. Without a file it proves the project entry; either way the source is assembled with
the modules it imports, so a theorem in an imported module is proved too. A name that is both a
theorem and an axiom is refused, so an assumption cannot pass as a proved fact.

- `--trace` renders the derivation tree under each proved theorem.
- `--json` emits `file`, `axioms`, `theorems` (each `name`, `verified`, `error`) and `all_verified`,
  plus the claim `inventory` — every theorem classified proved or unproved, every admitted axiom
  reported as an assumption — and its `summary`.
- `--certificate` emits the independently re-checkable certificate of every proved English
  `## Theorem`: the kernel proof term, the goal and the disclosed premises, each re-checked here by
  type-checking the term (`rechecks`, `two_kernel_agreement`), with the kernel's `prelude_version`. A
  third party re-checks it with the kernel alone, no prover. A `## Theory` theorem, whose proof path
  exposes no kernel term, and an unproved goal report `certificate: null` with the reason.
- `--kernel <file>` checks a kernel-vernacular document instead (see [`check`](#check)).

The three output flags are exclusive, and any failed theorem exits 1.

```bash
largo prove socrates.lg --trace
largo prove tarski.lg --json
largo prove socrates.lg --certificate
largo prove --kernel Demo.andSwap.lg
```

### `sat`

The SAT Competition interface as a largo verb: solves a DIMACS CNF with the certified engine,
printing `s SATISFIABLE` with a `v` model or `s UNSATISFIABLE`, with competition exit codes
(10 = SAT, 20 = UNSAT, 1 = error). `--proof <path>` exports the UNSAT certificate (DRAT; DPR/SR
for the symmetry routes) for external checkers like drat-trim; `--stats` prints solver statistics
to stderr.

### `lean`

Lean in, LOGOS out, independently checked. `import` takes a Lean declaration and produces a LOGOS
package that proves the same statement; `replay` re-verifies a package from its bytes alone.

```bash
largo lean import --project ./proofs --target Demo.andSwap --out ./Demo.andSwap   # a Lean project
largo lean import Demo.ndjson --target Demo.andSwap --out ./out                    # a lean4export stream
largo lean import Demo.ndjson --arena-protocol                                     # Lean Kernel Arena protocol
largo lean replay ./Demo.andSwap
```

`--project` compiles the project in isolation with the pinned Lean 4.14.0 (`--lean <path>` names the
executable, `lean` by default, whose elan shim selects the toolchain) and exports the target with the
pinned `lean4export` (revision `20c4a64a`, export format 3.1.0), located by `--exporter <dir>`, else
`$LOGOS_LEAN4EXPORT`, else `~/.logos/lean/lean4export`. The declaration's closure is then checked by
the LOGOS kernel and rechecked by the independent second kernel. It is held to the assumption policy
by Lean's own `#print axioms` rule (`propext`, `Quot.sound` and `Classical.choice` are allowed;
`sorryAx` is refused, including through lemmas), and emitted as one literate `<Target>.lg` document
that is re-parsed and re-checked in a fresh context. The target's statement is compared with an
independent reference translation of the Lean type. The package (`manifest.json`, `<Target>.lg`,
`certificate.json`, `correspondence.json`, `source/export.ndjson`) is published atomically, and
only after a separate process has replayed it; a failed run leaves nothing at `--out`.

stdout is one `logos-lean-result/1` document reporting seven stages: `lean_accepted`,
`translated`, `primary_check`, `independent_recheck`, `assumption_policy`, `emission_reparsed` and
`fresh_replay`. The outcome is `verified` only when all seven passed, so only `--project` can reach
it: an export given directly was never seen being accepted by Lean. stderr carries the diagnostics
as NDJSON, each with a typed code. Exit codes: `0` verified · `1` invalid source · `2` unsupported
feature · `3` disallowed assumptions · `4` resource limit · `5` infrastructure failure (a defect of
LOGOS's own artifact, never blamed on the input) · `64` usage. `--timeout-secs` (default 1800)
bounds the whole run and `--max-export-bytes` (default 1 GiB) the export, both checked before the
work they bound.

`replay` prints the package's receipt document and exits `0` when every stage replays, `1` at a
failed stage, `2` when the receipt is otherwise incomplete, and `5` when the package cannot be read.

`--arena-protocol` is the Lean Kernel Arena benchmark protocol byte for byte: `ACCEPT` and exit
0/1/2, nothing else. It takes an export and no `--project`, `--target` or `--out`; under it,
`--trace-stages` reports every checking stage as JSON on stderr and `--audit-declarations` runs the
declaration audit.

### `doc`

Renders a markdown reference from the literate structure of the entry and every module it imports —
`## To` signatures, type definitions, notes, examples, and formal blocks, in source order (`## Main`
and `## Requires` blocks and `Import` lines are omitted). Output is `<name>.md` in `target/doc` or
`--out <dir>`.

### `doctor`

Diagnoses the environment largo runs in: the Rust toolchain (needed by `build`/`run`), the wasm32
target, node (for `--emit wasm`), the verification flavor, registry reachability and credentials,
and update freshness. Inside a project it also reports the project's health: the package name and
version and the project's shape (a program, a backend, a site, or a full-stack site), or the first
line of the error when the shape cannot be decided. Degradations are warnings; only a broken
project fails. Works offline.

### `completions`

Writes a completion script for bash, zsh, fish, powershell, or elvish to stdout — source it from
your shell's configuration for tab completion on every largo command and flag.

```bash
largo completions zsh > ~/.zfunc/_largo
```

### `explain`

Looks a topic up offline, in two namespaces: diagnostic codes first (every `code` from
`check --json`, `run` failures, and the MCP tools — the full list lives in
[ERROR_CATALOG.md](ERROR_CATALOG.md)), then language constructs from the teach lessons.
No project or network needed; `--json` emits a `logos-explain/1` document.

```bash
largo explain zero-index
largo explain use-after-move
largo explain Let
largo explain type-mismatch --json
```

### `mcp`

Serves the whole toolchain over the Model Context Protocol — JSON-RPC 2.0 on stdio, one JSON
object per line. Built for AI agents: register once and the agent gets the tools below plus the
learning corpus as resources.

```bash
claude mcp add logos -- largo mcp
```

| Tool | What it does |
|------|--------------|
| `logos_check` | Parse + type-check → `{ok, diagnostics}` with stable codes, spans, Socratic explanations |
| `logos_run` | Execute in a sandboxed subprocess: deterministic with `seed`, always bounded by `timeout_ms` (default 5000, at most 60000); `step_budget` stops the engine at a deterministic op (exit code 122, partial output kept); `engine` is `tw` (the tree-walker, the default) or `vm`, strictly — a program the chosen engine cannot run is a typed refusal (`refused: true`), never rerouted, and `engine_ran` names what ran; `args` are the program arguments |
| `logos_logic` | English → FOL (`mode`: default / pragmatic / kripke / all_scopes / discourse; `format`: unicode / ascii / latex) |
| `logos_prove` | Prove every `## Theory` / `## Theorem` in the source, kernel-certified → `{axioms, theorems, all_verified}` |
| `logos_emit` | Generated code: `target` rust, c, isa, or wasm (base64) |
| `logos_opts` | Which optimizations fired, per tier → `{aot, run, vm}` |

Resources (embedded in the binary — an installed `largo` teaches the language offline):
`logos://quickguide`; `logos://docs/<guide>` for every guide in this directory, with the error
catalog at `logos://docs/error-catalog`; and two JSON documents, `logos://errors` (the catalog as
`logos-errors/1`) and `logos://examples` (the example corpus with runtime-verified expected outputs,
`logos-examples/1`). One prompt, `write-logos`, orients an agent before its first program. Every tool
accepts inline `source` or a `file` path, and a check that *finds* errors is a successful call: read
`ok`, not `isError`.

### `dap`

Speaks the Debug Adapter Protocol on stdio with `Content-Length` framing: `launch`, breakpoints,
`stackTrace` / `scopes` / `variables`, the stepping verbs, `evaluate`, and real reverse execution
(`stepBack` / `reverseContinue`, advertised as `supportsStepBack`). It is a thin serializer over the same
`DebugSession` the Studio's debugger drawer renders, so an editor and the browser see the same session. Point
an editor's debug adapter at `largo dap`; at a terminal, `largo repl` is the interactive tool.

`launch` takes a `program` (a `.lg` file path) or an inline `source` (which wins when both are given)
and stops on entry. `setBreakpoints` acknowledges each requested line as verified, but lines are not
mapped to program ops, so a line breakpoint does not stop `continue`. The VS Code extension
contributes the `logos` debug type, whose launch configuration runs `largo dap` with `program`
defaulting to the open file.

### `test`

Runs the project's `## Test` blocks — the entry's, every imported module's, and every page's — each in a
fresh, isolated world on the interpreter tier, and prints a deterministic report ordered by test name
(the text report ends in the run's fitness rank). A positional filter keeps the tests whose name
contains it; a page's tests are named `pages/<stem>.lg › <test>`.

- `Expect that` failures record and continue, so one run reports every failure.
- A runaway test fails at a deterministic operation budget: a timeout that cannot flake.
- A test that mounts a page drives the page's lowered LOGOS on the tree-walker and on the bytecode VM,
  which must render and trace alike.
- A skip is loud, needs a reason, and fails the run unless budgeted by `[test] skip_budget` in `Largo.toml`;
  a test skipped for want of `--live` does not count against the budget.
- `--seed <n>` sets the suite seed (default `0`), and `--format json` emits the report as JSON. `--json`
  emits the claim inventory (every test classified by kind, example or finite-enumeration property, with
  its status, disclosed domain bounds and any concrete counterexample), even when the run fails.
- `--prove` checks finite-domain `For all` properties exhaustively instead of sampling them.
- `--tiers all` (or a comma-separated list of `interpreter` and `vm`) also runs each passing test's
  computation on the bytecode VM and fails on any byte of disagreement; any other tier name is refused
  (exit 2), since this flag compares only those two. `--browser` makes a mounted page's tests agree with a
  real headless Chrome as well (`CHROME_BIN`, else the first Chrome or Chromium on `PATH`). With no Chrome
  found, it refuses the run rather than skipping the tier.
- `--live` also runs tests granted a real capability (`This test may use the real filesystem.`), which
  leave the hermetic guarantee.

The run exits 0 only when every test passed and the skips fit the budget. It exits 1 on any failure, on an
unbudgeted skip, on a page that declares tests and does not compile, and when no test ran at all (no
`## Test` in the project, or a filter that matched none). stderr carries one machine-readable line naming
the tiers the run exercised — `LOGOS_TIERS_RAN: interpreter=<n> vm=off`, or with `--tiers all`
`vm=on vm_agreed=<n> vm_refused=<n> vm_diverged=<n>`, plus `page_tiers=…` when a mounted test ran.

```bash
largo test
largo test withdraw
largo test --seed 7 --format json
```

## The `Largo.toml` manifest

Defined by the `Manifest`/`Package` structs in
[`project/manifest.rs`](../apps/logicaffeine_cli/src/project/manifest.rs):

```toml
[package]
name = "my_project"
version = "0.1.0"                     # default "0.1.0"
description = "..."                   # optional
authors = ["You <you@example.com>"]   # optional
entry = "src/main.lg"                 # the default; a site may omit it
site = "https://acme.example"         # optional: the address a site is served at

[dependencies]
std  = "logos:std"                    # simple form: a version/URI string
math = { path = "./math" }            # detailed form: { version, path, git, rev, branch, tag }
geo  = { git = "https://example.com/geo.git", rev = "3f2a9c1e5b7d4f60a8c2e1b9d7f5a3c1e9b7d5f3" }

[test]
skip_budget = 0                       # loud skips the suite may carry

[web]
theme = true                          # false: pages ship without the built-in theme
```

- `entry` is a path inside the project (an `.md` twin is tried when the `.lg` is absent). A project
  with `pages/` or `static/` whose manifest names no `entry` is a site with no entry program.
- `site` is what makes an absolute `sitemap.xml` possible: `largo build --static` writes one only when
  it is set.
- A `path` dependency resolves against the project root. A `git` dependency must pin `rev` to a full
  40-character hex commit; a `branch` or `tag` is recorded but never resolved from, and a git
  dependency without an exact `rev` is refused as an irreproducible build. Resolution reads a checkout
  already on disk — the dependency's `path` when given, else `.logos-deps/<name>-<rev>/` — and never
  the network. A version string or `logos:` URI is not linked into the local closure.
- `Import <Name>.` brings in a linked dependency by its `[dependencies]` key; `largo build --manifest`
  prints the resolved closure.
- `[test] skip_budget` is how many authored `Skip this test because …` skips `largo test` tolerates
  (default 0).
- `[web] theme = false` drops the built-in design-token stylesheet from every built page, for a
  project that brings its whole design.

## Environment variables

Read by `largo` itself:

| Variable | Effect |
|----------|--------|
| `LOGOS_NO_JIT` | Skip installing the JIT, so `run --interpret` runs on the bytecode VM alone (the JIT exists on x86-64 Linux and macOS) |
| `LOGOS_TOKEN` | Registry auth token; wins over the credentials file (alternative to `largo login`) |
| `LOGOS_CREDENTIALS_PATH` | Override the credentials file location |
| `LOGOS_MAX_AST_DEPTH` | Raise/lower the AST nesting-depth limit (default 128, at least 16), read by the parser; `largo` also sizes its worker stack from it. The default is sized for the smallest standard stacks (worker threads, browser wasm); machines with deep stacks can raise it for heavily generated code, constrained embedders can lower it |
| `CHROME_BIN` | The Chrome or Chromium binary `largo test --browser` drives; else the first of `google-chrome`, `google-chrome-stable`, `chromium`, `chromium-browser`, `chrome` on `PATH` |
| `LOGOS_BROWSER_TRACE` | Set (and not `0`): print every DevTools frame `largo test --browser` exchanges with Chrome to stderr, with each reply's round-trip time |
| `LOGOS_LEAN4EXPORT` | The pinned `lean4export` checkout `largo lean import --project` uses when `--exporter` is not given |
| `LOGOS_STATS` | Set: `largo sat` prints solver statistics, as `--stats` does |
| `LOGOS_MCP_STEP_BUDGET` | Set by `largo mcp` on its sandbox worker from `logos_run`'s `step_budget`; not a user setting |

Read by the running program — the interpreter, the VM and a compiled binary alike
(`logicaffeine_system`):

| Variable | Effect |
|----------|--------|
| `LOGOS_SEED` | Scheduler seed for deterministic concurrent runs (`largo run --seed` and `largo serve --seed` set it for you); a failing run's `.replay` bundle records it |
| `LOGOS_PORT` | The port a `Serve` statement binds, overriding the port written in the program. `PORT` is honored too, so a platform that assigns one works unchanged; `LOGOS_PORT` wins when both are set |
| `LOGOS_HOST` | The interface a `Serve` statement binds; default `127.0.0.1`, `0.0.0.0` for a container. An IPv6 address is bracketed for you |
| `LOGOS_TLS_EXTRA_ROOT` | A PEM file whose certificate authority TLS client connections trust beside the bundled Mozilla roots — how a private CA's server is reached |
| `LOGOS_HTTP_ENGINE` | `uring` or `tokio`. By default a server uses the io_uring engine where the build includes it (Linux) and a probe ring initializes, and tokio otherwise |
| `LOGOS_HTTP_SHARDS` | The io_uring engine's shard count (default: the available parallelism, at most 1024) |
| `LOGOS_HTTP_ACCESS_LOG` | Write one access line per request to stderr |
| `LOGOS_MAX_INFLIGHT` | How many requests may be served at once; one arriving past the limit is refused immediately rather than queued |
| `LOGOS_REQUEST_TIMEOUT_MS` | The wall-clock budget for one handler call |
| `LOGOS_READ_DEADLINE_MS` | How long a client has to finish sending a request before it is dropped |
| `LOGOS_DRAIN_MS` | How long a graceful stop waits for in-flight work before giving up |
| `LOGOS_MAX_BODY` | The largest request body accepted |
| `LOGOS_READY_PATH` | A path that answers a readiness probe. Unset by default, so no path is reserved |
| `LOGOS_HEALTH_PATH` | A path that answers a liveness probe. Unset by default |
| `LOGOS_CORRELATION_ID` | `1`, `true`, `yes` or `on`: stamp every response with a correlation id. Off by default |

The io_uring engine runs a handler inline on its shard thread, so it cannot enforce a request budget,
answer a probe path, shed by admission or stamp a correlation id: setting `LOGOS_REQUEST_TIMEOUT_MS`,
`LOGOS_MAX_INFLIGHT`, `LOGOS_READY_PATH`, `LOGOS_HEALTH_PATH` or `LOGOS_CORRELATION_ID` serves on tokio
instead, and says so on stderr. The body and read-deadline caps hold on both engines.

The default registry is `https://registry.logicaffeine.com`.

## Where this is tested

The command surface is the clap `Commands` enum in
[`apps/logicaffeine_cli/src/cli.rs`](../apps/logicaffeine_cli/src/cli.rs); the manifest, build,
registry and credentials code is in
[`apps/logicaffeine_cli/src/project/`](../apps/logicaffeine_cli/src/project/). `cli_docs_lock`
parses that enum and the `EmitTarget` enum and holds this guide to them: every subcommand has a row
in the command table and a `###` section, no section names a subcommand the binary lacks, and the
`emit` table names every target. `installer_docs_lock` holds the install commands and the install
location on this page to the same canonical strings the installer scripts carry.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
