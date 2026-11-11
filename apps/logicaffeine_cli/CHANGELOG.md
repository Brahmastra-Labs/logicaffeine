# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `largo lean check [--dual] <export>`: a whole lean4export stream checked by the kernel's core, the code the Lean
  Kernel Arena runs; exit 0 accepted, 1 rejected, 2 declined, the reason on stderr. `largo lean import`'s independent
  recheck asks that core to decide the export too.
- `largo emit rdu --cache DIR`: compile through the section cache in DIR, every section an earlier compile there
  passed reused and the bundle byte for byte an uncached one; stderr says how many sections were compiled and reused.

### Changed

- `largo emit rdu --tp` takes every width the author writes, heads in balanced runs where the width divides
  none; the bundle's program directories carry `report.json` and `certificate.whole.json`.

### Fixed

- The agent pack serves `docs/logic-mode.md` and `docs/imperative-mode.md` as the guides read, as `mcp::resources`
  holds every resource to its source: the logic guide's generic carries its noun's kinds in the restrictor, and the
  imperative guide reads a bare `pi` or `e` as the `Real` nearest it.

## [0.11.0] - 2026-11-02

### Added

- `largo emit rdu --model <checkpoint>` takes any GGUF file or Hugging Face checkpoint directory the author writes,
  each weight read as the checkpoint stores it; `--tp` is checked against every layer's heads and widths.
- `largo bringup <checkpoint> [--target CHIPS] [--ctx N] [-o DIR] [--run [--tokens N] [--prompt IDS | --golden FILE [--prompts N|all]]]`:
  the one program of a checkpoint — a GGUF file or a Hugging Face directory — brought up on every chip asked for —
  `vm`, `jit`, `c`, `rust`, `riscv-qemu`, `wasm`, `cuda`, `d5005`, `sn50`, `fabric`; prints the decode step's identity
  and each chip's verdict, the cut behind every artifact or the name of what the chip does not run. With `--run`, every
  chip that brought the model up then runs it here, and its verdict gains `ran:` — its tokens, its tokens a second and
  how many lead the reference's — and `the program it ran:`, or `did not run:` with the reason by name. `--prompts`
  runs a golden's first N prompts (`all`: every one), each from an empty state: `ran:` totals them and a `sequence`
  line a prompt gives its tokens and agreement.

- **`largo test` runs a project's `## Test` blocks.** The verb 0.10.1 reserved (hidden, exit 2) is real.
  - **Discovery:** every `## Test: <prose name>` in the entry's assembled import closure and in each `pages/<stem>.lg`.
  - **Isolation and report:** each test runs in a fresh sealed world on the tree-walker (virtual clock, in-memory filesystem, seeded randomness, no network), and the report is name-ordered and byte-identical across runs.
  - **Assertions:** `Expect` records a failure and continues, `Expect this to fail:` fails when the block succeeds, and `Expect output:` compares shown output byte for byte. A runaway test fails at a deterministic op budget (`This test may run up to N steps.`), and a `For all` property reports a shrunk counterexample.
  - **Flags:** `[FILTER]` (a substring of the test name); `--seed N` (default 0); `--format text|json`; `--json`, the claim inventory (each test classified example or finite-enumeration property, with status, vacuity flag, disclosed domain bound and counterexample); `--prove`, which checks finite-domain properties exhaustively and ranks them proven; `--live`, which also runs capability-granted tests; `--tiers all` (or a list of `interpreter`, `vm`), which re-runs each passing test on the bytecode VM and compares byte for byte, any other value refused with exit 2.
  - **`--browser`:** a mounted page test must also agree in headless Chrome (`CHROME_BIN`, else the first of `google-chrome`, `google-chrome-stable`, `chromium`, `chromium-browser`, `chrome` on `PATH`); no Chrome is a refused run, never a skip. The driver is `logicaffeine_cli::browser`, a DevTools client over `--remote-debugging-pipe` with trusted input, virtual time, held and stubbed fetches and a host-call trace; `LOGOS_BROWSER_TRACE=1` prints every frame.
  - **Exit status:** a failure, zero executed tests, a filter that matches nothing, a page whose tests do not compile, or more authored skips than `[test] skip_budget` in `Largo.toml` (default 0) exits 1. Stderr reports the tiers that ran (`LOGOS_TIERS_RAN:`).
  - Locks: `test_runner_lock`, `property_test_lock`, `inventory_lock`, `tiers_flag_lock`, `closure_lock`, `pages_in_test_lock`, `browser_tier_lock`, `browser_tier_cannot_skip_lock`, `testrun_page_tiers_lock`.
- **`largo serve` serves what the project is.** One decision, `project::shape`, classifies a project from two facts — whether it has `pages/` or `static/`, and whether its entry's closure `Serve`s — and chooses the plan:
  - **Site** (`pages/` or `static/`, no `Serve`): served from memory, an unmatched path answered 404 or by the site's `## Error page 404`.
  - **Full-stack site:** also runs the entry's `Serve` as a child on a loopback `LOGOS_PORT`, tied to the dev server's stdin, and proxies whatever the site does not answer; static assets shadow routes on an exact path, as in a built binary.
  - **Backend:** interpreted, binding its own port. **Any other program:** served as its WebAssembly bundle.
  - **Watching:** a site always watches the backend's closure, `pages/` and `static/`. A page or asset edit rebuilds in memory and reloads the browser; a backend edit restarts the child and waits for its port first; a broken page serves its error overlay over the last good bundle and prints the same `file:line:col`; a backend that dies on boot has its stderr served as a 502. `/__dev` lists every page, its URL, module size and failure. `--watch` live-reloads a program bundle.
  - **Production lifecycle:** it announces the bound port on stdout, honors the lifecycle environment names shared with the compiled tier, answers the opt-in readiness and liveness probes (`LOGOS_READY_PATH`, `LOGOS_HEALTH_PATH`), sheds with 503 under bounded backpressure, and drains on `SIGTERM`/`SIGINT`, finishing in-flight requests before releasing the port.
  - **Flags:** `--port` for a site or bundle (default 8080, falling forward through the next 15 ports), `--host` (default `127.0.0.1`; `0.0.0.0` prints the LAN address), `--open`, `--timeout`, `--seed`, and trailing program arguments.
  - Tests: `project_shape_lock`, `shape_agreement_lock`, `cli_relay_lock`, `crates/logicaffeine_tests/tests/serve_fullstack_watch.rs`; `crates/logicaffeine_tests/tests/http_lifecycle_parity.rs` drives the real binary.
- **A site ships as one binary or as `dist/`.**
  - **Full-stack binary:** beside a `Serve` entry, `largo build` compiles every `pages/<stem>.lg` to its module, shim and document, and embeds them with every `static/**` file as a generated `logos_assets` table answered before the program's own routes. When a page uses the relay, the table carries `USES_RELAY` and the binary answers `/_relay`. A program with no site lowers exactly as before.
  - **Static export:** `largo build --static` (and a plain `largo build` of a site) replaces `dist/` with the pre-rendered pages, the modules, `static/**`, a generated `robots.txt` when the project has none, `sitemap.xml` when `[package] site` in `Largo.toml` names the URL, each `## Error page <status>` as `<status>.html`, `_headers` carrying the caching rules, and a `_redirects` rewrite for each `:param` page.
  - **Page URLs:** a page serves at its `## Page /path` in development, in the binary and in `dist/`; the file name is the default and `pages/app.lg` is `/`. Two pages claiming one URL refuse the build, and `## Page /posts/:id` answers every path its pattern matches.
  - **Caching:** modules are content-addressed (`<stem>.<hash>.<ext>`, `Cache-Control: public, max-age=31536000, immutable`), everything else is `no-cache`, every asset carries an `ETag`, and `If-None-Match` is answered 304.
  - **Pre-render:** each page ships its first paint in the body; a page the server walk cannot draw still ships its `Head:` and is named in a build warning. `[web] theme = false` in `Largo.toml` drops the built-in design-token stylesheet.
  - Locks: `serve_embed_assets_lock`, `dev_prod_parity_lock`, `pages_routing_lock`, `page_pattern_routing_lock`, `asset_cache_lock`, `prerender_is_the_first_paint_lock`, `static_export_lock`, `error_page_ships_lock`, `backend_relay_lock`, `cli_web_theme_lock`.
- **`largo new <name> --site` and `--web`.** Both write `pages/app.lg` (a `## Page /` placing a `## Component` with a passing `## Test`), `static/theme.css` and `static/style.css`; `--site` has no entry program, and `--web` adds a `## Route` backend ending `Serve on port 8080.` The two flags conflict, and a plain `largo new` is unchanged. Tests: `crates/logicaffeine_tests/tests/web_scaffold.rs`, `cli_web_theme_lock`.
- **`largo run` bounds and reproduces a run.** `--max-steps N` stops the interpreter at the identical op on every run and exits 122 with the partial output delivered; `--timeout 500ms|10s|2m` stops the interpreter or the compiled child and exits 124; `--seed N` sets `LOGOS_SEED` for the run and any child; `--json` prints one `logos-run/1` envelope (`outcome` `ok|runtime-error|budget-exceeded|timeout`, `lines`, `error`, `steps_used`). `--max-steps` and `--json` are refused without `--interpret`. `crates/logicaffeine_tests/tests/run_limits.rs`.
- **A failing interpreted run leaves a replayable bundle.** `largo run --interpret` writes `<entry>-<unix-seconds>.replay` under the project's target/replays directory — a self-contained record of the source, the scheduler seed and decision trace, the arguments, and the failure's kind, message and cursor — and prints `replay it: largo replay <path>`. No bundle is written on budget exhaustion or when the deterministic engine does not reproduce the failure. `largo replay <bundle> [--verify]` needs no project and reports reproduced byte-identically, no longer fires, or diverged; under `--verify`, diverged fails the command. `replay_lock`.
- **`largo explain <topic> [--json]`, offline.** It looks a topic up in the diagnostic catalog (every code `check --json` and the MCP tools emit, one entry per variant sharing it), then the teach lessons and primitive types; a miss is refused with the nearest real topic. `--json` prints `logos-explain/1` (`kind` `error-code` or `construct`). `crates/logicaffeine_tests/tests/explain.rs`.
- **`largo expand <file>` prints what an LGX page lowered to.** For each component, page and route, with its imports assembled: the render function, one function per action, the mount entry, the server HTML builder, the dispatcher and the `route(method, path)` walk, and derived `toJson_<T>`, each with the calls it makes. A component that synthesized nothing is reported as refused, and a parse error at `file:line:col`. `largo_expand_lock`.
- **`largo dap`: a Debug Adapter Protocol server with reverse execution.** DAP over stdio with `Content-Length` framing. `launch` takes a `program` path or inline `source` and stops on entry; `stackTrace`, `scopes`, `variables`, `next`, `stepIn`, `stepOut`, `continue`, `evaluate`, and `stepBack`/`reverseContinue` over the recorded history (`supportsStepBack: true`). `setBreakpoints` acknowledges lines without mapping them to operations yet. It serializes `logicaffeine_compile::debug_session::DebugSession`, the facade the Studio's debug drawer renders; `dap::DapServer` is public. Locks: `dap_lock`, `session_parity_lock`.
- **`largo mcp`: the toolchain over the Model Context Protocol.** JSON-RPC 2.0 on stdio, one object per line; `2025-06-18`, `2025-03-26` and `2024-11-05` are negotiated.
  - **Tools:** `logos_check` (a check that finds errors is a successful call), `logos_run`, `logos_logic`, `logos_prove`, `logos_emit` (`rust`, `c`, `isa`, `wasm`) and `logos_opts`; the prompt `write-logos`.
  - **Sandboxed runs:** `logos_run` never executes inside the server. It re-spawns `largo mcp --worker <tw|vm>` under a wall clock (`timeout_ms`, default 5000, at most 60000) and an optional `step_budget`; a program the requested engine cannot run is a typed refusal (`refused: true`, `engine_ran: null`), never a substitution.
  - **Resources:** sixteen ship inside the binary: `logos://quickguide`, thirteen `logos://docs/*` guides, `logos://errors` and `logos://examples`. The error catalog is rendered live from `logicaffeine_language::diagnostic`; the rest is embedded from `apps/logicaffeine_cli/assets/agent/`, regenerated by `scripts/release/gen-agent-artifacts.sh`.
  - Tests: `crates/logicaffeine_tests/tests/mcp_protocol.rs`, `crates/logicaffeine_tests/tests/mcp_tools.rs`, `crates/logicaffeine_tests/tests/mcp_e2e.rs`.
- **Every scaffolded project carries `AGENTS.md`.** `largo new` writes it from `apps/logicaffeine_cli/templates/AGENTS.md` — the check/run/test/serve loop, the classic mistakes and the MCP registration line — and `largo init` adds it when absent. `crates/logicaffeine_tests/tests/agents_md.rs` re-parses every `largo …` line in it against the real CLI and every LOGOS fence as LOGOS.
- **`largo lean import` and `largo lean replay`.**
  - **Import:** `import --project DIR --target Name --out DIR` compiles a Lean project in isolation with the pinned Lean 4.14.0 and exports the target with the pinned lean4export (`--exporter`, else `$LOGOS_LEAN4EXPORT`). The export is checked by the LOGOS kernel, rechecked by the independent second kernel, held to `#print axioms` policy, and emitted as a literate `<Target>.lg` package; `import <export.ndjson>` starts from an existing export.
  - **Publication:** the package is replayed by a fresh process before it is renamed into `--out`, so a failed run leaves nothing there. `replay DIR` re-verifies a published package.
  - **Output:** stdout is one `logos-lean-result/1` document and stderr is NDJSON diagnostics; exit codes are 0 verified, 1 invalid source, 2 unsupported, 3 disallowed assumptions, 4 resource limit, 5 infrastructure, 64 usage. `--arena-protocol` speaks the Lean Kernel Arena protocol.
  - Tests: `crates/logicaffeine_tests/tests/lean_cli_e2e.rs`.
- **Kernel documents and re-checkable proof certificates.** `largo check --kernel FILE` and `largo prove --kernel FILE [--json]` check a kernel-vernacular literate document block by block through `Repl::execute_document`, naming the first block that does not check. `largo prove --certificate` prints a JSON certificate (proof term, goal, disclosed premises) for every kernel-proved English `## Theorem`, each re-checked against the kernel, and `--json` gains `inventory` and `summary`. Locks: `proof_certificate_lock`, `inventory_lock`.
- **Machine-readable envelopes.** `largo check --json` and `largo build --json` print one `logos-check/1` document whose diagnostics are `logos-diagnostic/1` objects (stable kebab-case code, severity, file, byte and line/column span, message, Socratic explanation, suggestion, quickfix, docs, related); a cargo-phase failure is one `build-failure` diagnostic carrying the raw output. `largo fmt --check --json` prints `logos-fmt/1` (`clean`, `unformatted`). Prose stays on stderr and exit codes are unchanged. Tests: `diagnostic_schema_lock`, `crates/logicaffeine_tests/tests/check_json.rs`.
- **Local `path` and exactly-pinned `git` dependencies link into every command.** `check`, `build`, `run --interpret`, `doc`, `prove`, `verify` and `emit` resolve `[dependencies]` into the assembled closure. A git dependency carries a 40-hex `rev` and resolves against `.logos-deps/<name>-<rev>/` or an explicit `path`; `branch` and `tag` are informational. `largo build --manifest` prints the linked closure as JSON without running cargo. `local_deps_lock`.
- **`largo emit` reaches eleven targets.** `riscv`, `wasm-dom`, `tenstorrent`, `cuda`, `analog`, `plasticine` and `rdu` join `rust`, `c`, `wasm` and `wasm-linked`; bare, each prints its primary artifact.
  - **`riscv`:** RV64GC assembly, with `logos_rt.c` beside `-o`. **`wasm-dom`:** a DOM app's `.wasm`, `.mjs`, `.js` and themed `.html`.
  - **`tenstorrent`:** a single-Tensix TT-Metalium bundle (`logos_compute_kernel.cpp`, `logos_tt_host.cpp`, `CMakeLists.txt`, `logos_oracle.c`). **`cuda`:** a single-file CUDA program (`logos_cuda.cu`, `Makefile`, `logos_oracle.c`). Only these two write the C oracle.
  - **`analog`:** `netlist.sp`, `schedule.json`, `budget.json` and `cert.json`.
  - **`plasticine`:** for a counted-loop function, `Logos_<fn>.scala` (Spatial source), `placement.json` (checkerboard placement and metapipeline II model) and `report.json` (the modeled report row); anything else is refused with a typed `plasticine backend:` error.
  - **`rdu`:** a SambaNova RDU bundle (`neutral.json`, `placement.txt`, `certificate.json`, `report.json`) for `--chip sn40l|sn50` (default `sn50`), with `--format placement|spatial|sambaflow|vllm|report` and `--bridge mock|sambaflow`; `--model qwen38|qwen38-tiny|qwen38-mini` (with `-o DIR`) lowers a whole model as one certified program per layer kind plus a manifest, and `--ctx N`, `--batch N|auto`, `--tp N|auto` and `--prefill N` turn it into a decode, batched, tensor-parallel or prefill bundle. Each dial is refused by name without its prerequisite, and every RDU flag is refused on another target (exit 2). `rdu_emit_matrix_lock` records which program × format artifacts are producible.
  - **Refusals:** every target but `rust` compiles the full closure and reports a refusal at `file:line:col`. `largo build --emit` also accepts `isa` (the Silicon ISA listing, `<entry>.isa`) and `riscv`/`risc-v` (`<entry>.s` and `logos_rt.c`, with the gcc and qemu command printed).
  - Tests: `crates/logicaffeine_tests/tests/emit_rdu.rs`, `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`, `crates/logicaffeine_tests/tests/emit_plasticine.rs`, `crates/logicaffeine_tests/tests/wasm_dom_emit.rs`, `qwen38_bundle_contract_lock`, `diagnostic_surfaces_lock::emit_places_a_refusal_at_its_line`.
- **`logicaffeine_cli::largo_main()` is the whole `largo` process.** It installs the native tier, runs the stack-sized worker, renders errors and sets the exit code. `apps/logicaffeine_cli/src/main.rs` is exactly this call, and so is the test crate's `largo-under-test`, so the CLI suites spawn the shipped program (`test_bin_shim_lock`). The library also exposes `browser`, `dap::DapServer`, `mcp::McpServer`, `project::shape`, `project::deps`, and the manifest's `ResolvedDep`, `DepSource`, `DependencyError`, `TestSection` and `WebSection`.
- **`largo build --emit c` and `--emit c-bytecode` build the same executable through C**: the program lowered to C (from the optimized syntax tree, or from the VM's bytecode) and compiled by `$CC` (default `cc`), published in the selected mode’s target directory, with the generated C source retained in its build-c subdirectory. Debug is `-O0 -g`; `--release` is `-O3 -march=native -flto`, the flags the benchmark suite builds its hand-written C with; `-v` prints the compiler command. `largo run --emit c` (or `c-bytecode`) builds and runs (`largo_build_emit_c`).
- **`largo run --interpret --engine jit|vm|tree`** picks the in-process engine: the VM with JIT tier-up (the default), the VM alone, or the tree-walker, each over the same optimized program.
- **`largo build --emit c|c-bytecode` links the exact extension's value runtime** for a program the extension reaches (`codegen_c::value_runtime`), announcing its build; `largo emit c -o` writes the runtime beside the C as `liblogos_value_runtime.a` and prints the command that links the two, and on stdout notes that `-o` writes it. `build_c::c_compiler_args` takes the program's link inputs. `largo_build_emit_c`.
- **`largo emit wasm-linked` and `largo build|run --emit wasm-linked` run the exact extension**, the module linking its value runtime; the build announces the module as `[wasm-linked, Rust runtime, …]`. `largo emit wasm` refuses such a program by name, pointing at `wasm-linked`. `exact_on_wasm_lock`.
- **`largo emit riscv` and `largo build --emit riscv` run the exact extension**: the runtime shim written beside the assembly carries the extension's adapter, and `-o` writes the value runtime built for RISC-V as `liblogos_value_runtime_riscv64.a` with the command that links the three.

### Breaking changes
- **A project with `pages/` or `static/` is a site.** If its entry never `Serve`s, a plain `largo build` exports `dist/` (deleting and rewriting it) instead of compiling a binary; if it does `Serve`, those files are embedded and answered before the program's own routes on an exact path, and `pages/*.lg` compile to WebAssembly. `largo check`, `largo fmt --check` and `largo test` also read every `pages/<stem>.lg`. Migration: `largo run` still runs the entry, and `largo build --target <triple>`, `--lib`, `--emit` or `--native-functions` take the program path; rename a directory not meant to be served.
- **`[dependencies]` are resolved, not ignored.** `check`, `build`, `run --interpret`, `doc`, `prove`, `verify` and `emit` refuse a `git` dependency without a 40-hex `rev`, a `path` that does not exist, and a table with none of `path`, `git` or `version`; registry versions and `logos:` specs are still skipped. Migration: add `rev = "<40-hex commit>"` and vendor the checkout at `.logos-deps/<name>-<rev>/` (or set `path`). `largo add <name> --git <url>` still writes the entry without a `rev`; add it by hand.
- **The built binary moved.** It was target/<mode>/build/target/<mode>/<name>; it is target/<mode>/<name>, the file `largo run` runs. The generated crate builds under a per-project cargo identity (`<name>-<digest>`) and the artifact is read from cargo's JSON messages, so two projects sharing a `CARGO_TARGET_DIR` no longer overwrite each other. Migration: use the path `largo build` prints. `build_identity_lock`.
- **`largo verify` and `largo build --verify` can fail.** In 0.10.1 they verified nothing and always passed; see Fixed. Migration: fix the finding the verifier names.
- **`largo check` fails on a program `largo build` would refuse.** See Fixed. Migration: fix the construct each `deploy-refusal` names.
- **`largo prove` refuses a theorem that shares an axiom's name.** An axiom is an assumption and cannot masquerade as a proved theorem. Migration: rename the theorem. The new `--json` keys are additive.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.
- `largo emit rdu --model <checkpoint.gguf>` reads the configuration from the checkpoint's spec and writes the
  program the bundle was lowered from as `program.lg`.
- **`largo build`'s first paint comes from `logicaffeine_compile::web_host::first_paint`.** The render moved out of `web_bundle.rs` unchanged, so the web app's live preview paints a page the way a built page does. Every document `largo build` ships is byte-identical to before (`prerender_golden_lock`).

- `largo run --interpret` writes the program's lines through one buffered stream instead of one `println!` each.
- **Every command compiles the same closure.** `check --deep`, `prove`, `doc`, `verify`, `run --interpret`, `emit` and `build --native-functions` see imported modules and linked dependencies exactly as `build` does, so an imported module's `## Test`s run and its theorems are proved; `doc` omits `Import` lines. Locks: `closure_lock`, `check_deep_multifile_lock`.
- **`largo check` and `largo fmt` read the whole project.** `check` checks every page in `pages/` on the tier it ships on (a site with no entry checks just its pages), reports LGX findings and value-copy warnings, warns on an unused or duplicate `Import` and refuses a self-import. `fmt` formats `pages/` and sorts and de-duplicates the import block. Locks: `pages_in_tooling_lock`; `crates/logicaffeine_tests/tests/import_hygiene.rs`.
- **A site needs no `entry`.** Commands that need a program say so, and a declared but missing `entry` is an error. `largo doctor` prints the project's shape.
- **Release binaries are built with the new `[profile.dist]`.** Whole-program LTO, one codegen unit, stripped symbols, panics still unwind: `largo` is 37.2 MB, down from 53.5 MB on x86_64 Linux, and a backtrace from it is unsymbolized.
- **The published crate carries only what builds it.** An anchored `include` allowlist ships `src/`, `assets/`, `templates/`, the README and the CHANGELOG; the integration suites moved to `crates/logicaffeine_tests/tests/` and the unit-test bodies to `crates/logicaffeine_tests/unit/logicaffeine_cli/`. `largo` now depends on `logicaffeine-system` and on the kernel's `lean` feature. The declared `rust-version` is 1.88 (0.10.1 declared 1.75), so `cargo install logicaffeine-cli` needs Rust 1.88.

### Fixed

- `largo test --browser` reads a page only after every task due by the end of an `Advance time` has run. Chrome pauses its clock at the end of an advance and holds the page's timer queues, so a timer's follow-up due at that instant waited for the next advance, and under load a timer due then did too. An advance now ends by letting the held work run, and `Advance time by 0 seconds.` runs what is due now instead of waiting for a budget Chrome never reports spent.

- `largo opts` renders a backend's parse error at its source location instead of printing its debug form.

- `largo build` takes the release panic strategy from the compiler: a program that serves builds with `panic = "unwind"`, so a failed request answers `500` instead of ending the server.
- **`largo verify` verifies the program.** 0.10.1 read the entry into a discarded variable, asked Z3 whether `true` is valid and printed success. It now runs `compile_to_rust_verified` (escape analysis, the Z3 pass, the type checker) over the assembled project and fails on an embedded `compile_error!`; `largo build --verify` takes the same path. The `verification` feature forwards to `logicaffeine-compile/verification` and `logicaffeine-proof/verification`, which that path needs; `feature_forwarding_lock` keeps every feature forwarded to the workspace dependencies that declare it. `crates/logicaffeine_tests/unit/logicaffeine_cli/commands/verify/tests.rs`.
- **`largo check` fails on a program `largo build` would refuse.** It names each `compile_error!` refusal the native tier compiles into the binary (`deploy-refusal` in `--json`); a program that touches the DOM is not read against the native tier. `check_deploy_refusal_lock`.
- **An installed `largo build`/`largo run` works outside the monorepo.** The multi-file project build resolves its runtime dependencies through `logicaffeine_compile::compile::RuntimeCrateSource`: an installed `largo` depends on the crates.io runtime at its own exact version and skips staging local sources, so the native tier compiles `largo new` projects on a clean install. It used to die with `Could not find workspace root`, leaving only `--interpret` and `--emit wasm`. The manifest ends with the Linux section every generated project shares (`RuntimeCrateSource::linux_engine_section`), so a built `Serve` binary has the io_uring engine on Linux. `served_address_lock::the_compiled_tier_announces_the_port_it_bound_on_either_engine`.
- **Logic mode in `largo repl` explains a sentence it cannot parse.** It prints the Socratic message and a caret excerpt instead of the parser's internal error value. `diagnostic_surfaces_lock::no_debug_dump_of_an_error_reaches_the_author`.
- **Every `largo` command that runs or emits a program checks it the same way first.** `largo run --interpret` and every `largo emit` target pass the program through one front end, so a program that does not parse, or reaches a module it never imported, is reported once in the same words before any backend sees it; a diagnostic names its file relative to the directory `largo` runs in. `largo run --interpret` runs the program on the reactor `Connect`, `Serve` and a timed `Sleep` need, and announces a replay recording only to a terminal, so a piped run's stderr is the program's reason alone.
- **`largo test --browser` waits for a handler as long as the page runs it.** A trusted input event, and a harness action that fires one, returns only once the page has handled it, and was bounded by the 20 s each protocol command gets, so a click whose handler did real work timed out under load (`browser_navigation_lock`, in the full suite). Those wait within `browser::PAGE_WORK_TIMEOUT` (300 s); every other command keeps 20 s.

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
