# Contributing to Logicaffeine

In this project the tests are the spec. When a test and the code disagree, the code is wrong, so
every change starts as a failing test (red), becomes a passing one (green), and is cleaned up with the
suite still green. The documentation is held to the same bar: the examples in the guides are
compiled and run by the suite, and a guide that drifts from the code fails a test.

## Getting Started

### Prerequisites

- Rust 1.88 or newer (the workspace `rust-version`)
- [cargo-nextest](https://nexte.st), which every test command below runs on
- For the full suite and the verification features: the Z3 SMT solver and its C header (`z3` + `libz3-dev` on Debian/Ubuntu, `brew install z3` on macOS), with `Z3_SYS_Z3_HEADER` pointing at `z3.h` (the environment for each platform is in `crates/logicaffeine_verify/README.md`)

### Building

```bash
# Clone the repository
git clone https://github.com/Brahmastra-Labs/logicaffeine.git
cd logicaffeine

# Build the default members (no Z3 needed)
cargo build
```

### Running Tests

Every integration suite is its own test binary in one package, `logicaffeine-tests`. Name the suite
you want with `--test`: without it, cargo builds every test binary in the package before it runs any
of them, which costs a long build and a lot of disk. For the same reason there is no bare
`cargo test` line here.

```bash
# The full suite: every crate, the Z3 verification tests, the ignored heavy tests and the doctests
./scripts/run-all-tests-fast.sh

# The same, skipping the multi-minute ignored tests (the fast iteration loop)
./scripts/run-all-tests-fast.sh --no-ignored

# One suite: name its file with --test
cargo nextest run -p logicaffeine-tests --test phase1_garden_path

# One test by name within a suite
cargo nextest run -p logicaffeine-tests --test phase1_garden_path -E 'test(<name>)'

# One crate's unit tests
cargo nextest run -p logicaffeine-kernel

# A suite with its output shown
cargo nextest run -p logicaffeine-tests --test phase1_garden_path --no-capture
```

The full-suite runner refuses to start while another suite is running, keeps a timestamped log of
the run, and ends with a summary of the duration and the pass/fail counts. Some classes of test are
deliberately left out of it (research probes, gate-level silicon simulations and a few other
on-demand sets), and each has its own runner; `scripts/README.md` lists them.

### Running CI on Your Machine

`scripts/run-ci-locally.py` runs a workflow itself (`.github/workflows/test.yml` by default),
job by job, each in a Docker container shaped like the GitHub runner its label names (ubuntu-24.04 or
ubuntu-22.04): its packages, its `runner` account and paths, its 4 cores and 16 GB for the standard
labels. Every `run:` step runs as written, every action through a local equivalent, and anything it
cannot reproduce faithfully is refused by name rather than skipped. A leg this machine cannot host
(macOS, Windows, arm64) is named as not run, never counted as passed. A workflow that is green here is
green on GitHub before a runner minute is spent.

```bash
# What each trigger runs, or what the runner cannot reproduce
python3 scripts/run-ci-locally.py plan

# Every job on the working tree, uncommitted changes included (a few hundred GB of state)
CI_LOCAL_STATE=/path/on/a/big/disk python3 scripts/run-ci-locally.py run --tree .

# One leg again, reusing the archive and outputs of an earlier run
python3 scripts/run-ci-locally.py run --tree . --job test --matrix partition=2 --from-run <run>

# A command in a failed job's container, toolchain and workspace
python3 scripts/run-ci-locally.py exec --run <run> --job "test (2)" -- cargo nextest run --archive-file ...

# The release workflow as the dispatch that releases a pushed tag (nothing is published)
python3 scripts/run-ci-locally.py run --workflow .github/workflows/release.yml --event workflow_dispatch --input tag=v0.11.0
```

Logs land in `logs/ci-local/<run>/`, one file per step, with `summary.txt` giving each job's result,
time against its timeout and disk use. A job that passes gives its workspace back; a failing job's is
kept for `exec` (`--keep-workspaces` keeps them all).

## Development Workflow

Development is **test-driven (TDD)**:

1. **RED** - Write a failing test first
2. **GREEN** - Write minimal code to make it pass
3. **REFACTOR** - Clean up while keeping tests green

### Important Rules

- **Never modify failing tests** - Tests define the spec. If a test fails, fix the implementation, not the test.
- **Keep changes focused** - Don't add unrelated improvements in the same PR.
- **No over-engineering** - Only add what's needed for the current task.

## Code Style

- AST nodes live in bumpalo arenas
- Identifiers and words are interned as `Symbol`s (`logicaffeine_base::intern`)
- Parser backtracking goes through `ParserGuard`, an RAII guard that rolls the parser back
- Read-only walks over logical expressions implement `logicaffeine_language::visitor::Visitor`

## Changing docs

The guides are code-grounded, and the suite reads them. What runs:

- **In the root `README.md`, every guide in `docs/` and every crate README:** each fence tagged
  `logos` compiles. A `logos` fence followed by an `output` fence runs on the tree-walker and must
  print exactly that output; one followed by a `rust` fence must match the Rust the compiler emits.
  Every `## Test` in a `logos` fence passes, and in an untagged fence every `"English" → FOL` line,
  and every quoted sentence followed by its numbered readings, is re-derived by the compiler
  (`doc_examples`).
- **The first-run path:** `docs/getting-started.md` and the README's Quick start are replayed command
  by command, with nothing but `largo` on the `PATH` (`getting_started_lock`).
- **Pages:** each fence tagged `logos-dom` in those files, the quick guide and the example READMEs
  is parsed, mounted and rendered on the tree-walker and the VM, compiled for the browser, and its
  `## Test` blocks run (`canonical_teaching_lock`). A fence that holds LOGOS without one of the two
  LOGOS tags fails, because nothing would run it.
- **Crate READMEs** are each crate's rustdoc root, so a fence tagged `rust` there is a doctest.
  Tag every fence (`readme_lock` allows `rust`, `rust,no_run`, `bash`, `sh`, `text`, `logos` and
  `toml`).

Beyond what runs:

- **No hard counts in prose.** A number nothing computes goes stale the day something is added.
- **Every guide is indexed.** A new `docs/*.md` is linked from `docs/README.md` and from the root
  README's Documentation section; a new example directory, from `examples/README.md`
  (`docs_index_lock`).
- **Public paths only.** Cite files a reader of this repository can open, and nothing else
  (`public_prose_lock`).
- **Regenerate what the docs feed.** After editing a guide in `docs/`, `LOGOS_QUICKGUIDE.md` or the
  Syntax Guide, run `./scripts/release/gen-agent-artifacts.sh`: it rewrites `llms-full.txt`,
  `examples.json`, the error catalog and the agent pack `largo` serves, each byte-locked by a test.
  A new guide also takes its place in the handbook: add it to `PARTS` in
  `scripts/release/docs/handbook.py`, and `./scripts/generate-docs.sh` assembles the one-file book
  (`generate_docs_paths_lock` fails on a guide the book leaves out).

The voice, in brief:

- Address the reader as *you* and the project as *we*.
- Say why before what, and show real input and real output.
- Be concrete: a measured number or a named test beats an adjective.
- Say plainly what is not done yet.
- Skip the fluff words (`powerful`, `seamless`, `robust`, `blazing`), ALL-CAPS emphasis and internal codenames.

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes following TDD
4. Ensure all tests pass (`./scripts/run-all-tests-fast.sh`), and the CI workflow
   (`python3 scripts/run-ci-locally.py run --tree .`)
5. Submit a pull request

### PR Guidelines

- Include a clear description of what changes were made and why
- Reference any related issues
- Keep commits focused and atomic
- Update documentation if needed (see [Changing docs](#changing-docs))

## Project Structure

```text
logicaffeine/
├── crates/
│   ├── logicaffeine_base/       # Arenas, interning, spans, numeric/quantity/money/time/UUID value types, hashes
│   ├── logicaffeine_lexicon/    # English vocabulary tables
│   ├── logicaffeine_kernel/     # Calculus of Constructions kernel (no lexicon); TCB.md inventories its assumptions
│   ├── logicaffeine_data/       # Runtime values, CRDTs, wire codec (IO-free)
│   ├── logicaffeine_system/     # Platform IO, networking, HTTP, relay, persistence
│   ├── logicaffeine_language/   # English → FOL: lexer, parser, AST, transpiler
│   ├── logicaffeine_proof/      # Proof engine: solvers, tactics, certificates
│   ├── logicaffeine_compile/    # Interpreter, VM, codegen (Rust, C, WASM, RISC-V, CUDA, …); the stdlib in assets/std/
│   ├── logicaffeine_forge/      # EXODIA, the copy-and-patch JIT (native)
│   ├── logicaffeine_jit/        # Wires the forge JIT into the VM (native)
│   ├── logicaffeine_runtime/    # Deterministic concurrency runtime
│   ├── logicaffeine_lsp/        # Language Server Protocol
│   ├── logicaffeine_verify/     # Z3 static verification (verification feature)
│   ├── logicaffeine_tv/         # SMT translation validation
│   ├── logicaffeine_synth/      # Offline Z3 stencil proofs (unpublished)
│   ├── logicaffeine_wirebench/  # Wire-codec benchmark harness (unpublished)
│   └── logicaffeine_tests/      # Every test: integration suites in tests/, unit-test bodies in unit/
├── apps/
│   ├── logicaffeine_cli/        # largo — the LOGOS build tool
│   ├── logicaffeine_web/        # Dioxus web IDE: Studio, Learn, Syntax Guide
│   └── logicaffeine_docs/       # Build script for the rustdoc site
├── docs/                        # The guides; docs/project/ holds the FAQ, the challenge rules and versioning
├── examples/                    # Example LOGOS projects, each with a README
├── editors/vscode/              # VS Code extension
├── benchmarks/                  # Cross-language benchmark suite + results
├── services/                    # Cloudflare Workers: the package registry, the lead form and the heal worker
├── scripts/                     # Entry points and the FPGA/silicon tooling at the top; the rest by area, indexed in scripts/README.md
├── .config/                     # nextest profiles, repo shape, size budgets, mutation scope, coverage floors
├── .github/                     # CI workflows, issue and PR templates, this guide
└── LOGOS_QUICKGUIDE.md          # Every LOGOS construct on one page
```

## Tests

Every test lives in `crates/logicaffeine_tests`. Integration suites are `crates/logicaffeine_tests/tests/*.rs`, one test binary each. A crate's white-box unit tests have their bodies in `crates/logicaffeine_tests/unit/<crate dir>/<module path>.rs`, reached through a one-line `#[cfg(test)] #[path = "…"] mod tests;` stub in the crate. They keep private access and still run under `cargo nextest run -p <crate>`; `test_home_lock` and `unit_home_lock` keep tests out of every other `tests/` directory and every `src/`. The logic-mode suites are phase-organized by linguistic and logical complexity (garden paths, polarity, tense/aspect, wh-movement, and on through the higher phases), alongside the execution-tier, concurrency, wire-codec, and proof suites.

## Questions?

- Open an issue for bugs or feature requests
- Check existing issues before creating new ones

## License

By contributing, you agree that your contributions will be licensed under the project's MIT License ([LICENSE.md](../LICENSE.md)).
