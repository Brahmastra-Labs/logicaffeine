# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **The CI workflows run on your own machine before they run on GitHub.** `scripts/run-ci-locally.py` executes
  `.github/workflows/test.yml` itself, and `release.yml` as the dispatch that releases a pushed tag: each job in its
  own Docker container built from its label's image under `scripts/test/ci-runner/` (ubuntu:24.04 or ubuntu:22.04 with
  the hosted runner image's packages, its uid-1001 `runner`
  account with passwordless sudo, its `/home/runner/work` paths and `CI`/`GITHUB_*`/`RUNNER_*` environment), every
  `run:` step verbatim, every `${{ }}` expression and `if:` evaluated with GitHub's rules, and every action replaced by
  a local equivalent (checkout into a fresh shallow clone, today's stable Rust at `--profile minimal` plus rustfmt and
  clippy in a toolchain copied fresh for each job, nextest, artifacts carried between jobs, a cache that restores
  nothing as a miss does, a checkout of the tag a dispatch names). Matrices expand with `include` and `exclude` as
  GitHub expands them, a dispatch reads the inputs it declares (`--input`), and `hashFiles` digests the workspace. A
  leg on a macOS, Windows or arm64 label, or one that needs what only such a leg makes, is named as not run, and a
  run that left one out does not call itself GREEN. The standard runner's
  size is applied (4 pinned cores, so `nproc` reads 4, and a 16 GB memory limit), each job's `timeout-minutes` is
  enforced, and the summary gives each job's time against its timeout and its disk use. No job holding a `write`
  permission runs, and no credential enters a container. `plan` refuses by name any action, runner label, key,
  trigger, context or expression function it cannot reproduce, and evaluates every value it can before a run, so the
  local run cannot drift from CI in silence; `run --tree .` runs the working tree with its uncommitted changes,
  `--job`/`--matrix`/`--from-run` repeat one leg, `--timeout-scale` stretches every timeout on a slower or busier
  machine (and says so in the summary), `exec` runs a command in a failed job's container (a passing job gives its
  workspace back), and a stopped run removes its containers. Locked by `ci_local_runner_lock`.

### Changed

- **The kernel's Lean checker spends 29% fewer instructions on Mathlib.** `logicaffeine-kernel`'s value
  engine, the Lean Kernel Arena entry's engine, checks the Arena's whole Mathlib export in 409.15 G instructions,
  from 577.15 G, every verdict unchanged: the Arena entries build with the new `unchecked-access`
  feature (the `dag` core reads its stores without bounds checks; every other build and every test keep them), cached
  values and types are found in the caller's frame, a neutral's arguments are stored contiguously and interned in
  one probe, and an environment remembers its two latest prunings in place of a pruning cache.
- **The Lean Kernel Arena entry credits the checkers it builds on, says how it is built, and runs the check LOGOS
  ships.** The kernel README's Acknowledgements, the Arena runbook and both entries' descriptions credit the Lean 4
  kernel, lean4lean, smalltt, sokonanoda, nanoda, mathgraph, nano-lean, lean4export and the Arena, each for what it
  gave; the core shares no three-line run with sokonanoda. The descriptions say the entry is the core LOGOS runs in a
  build tuned for the Arena's instruction count. Its run line is `dag::arena::run`, the check `largo lean check` and
  `cargo install logicaffeine-kernel`'s new `logos-kernel` binary run, so the Arena measures what LOGOS users run, and
  `largo lean import`'s independent recheck asks the core to decide the whole export too: a refusal of what the LOGOS
  kernel accepted stops the import as a disagreement between the kernels. Locked by `arena_entry_lock`.

### Fixed

- **The release workflow publishes notes longer than a GitHub release holds, and releases a tag pushed before it was
  fixed.** The 0.11.0 section of the changelog is 506,000 characters and GitHub refuses a release body over 125,000,
  so the release step failed after every binary had built. A section that long now ships every heading, each
  heading's first entries whole up to one cap (the largest under which the notes fit), and in place of the rest a
  link to the full section in the tag's CHANGELOG.md. A re-run of a tag's run executes the workflow file of that tag,
  so `release.yml` takes the tag to release on dispatch: `gh workflow run release.yml --ref main -f tag=vX.Y.Z`
  builds, publishes and install-tests the tag with the workflow of `main`, and publishes a draft already made for the
  tag rather than leaving it a draft. Locked by `release_assets_lock`.
- **A release holds the tag's binaries to the size budgets of the commit that dispatches it.** `release.yml` checks
  the `largo` and `logicaffeine-lsp` dist builds against `.config/size-budgets.toml` read from the workflow's own
  commit (a sparse checkout beside the tag's), so a budget changed after the tag applies to the tag's binaries, and
  `check-binary-sizes.sh --bin-dir` reads a relative directory from where it is called. The `[binaries]` budgets rise
  to v0.11.0's builds plus 3% (`largo` 55,599,919 B, `logicaffeine-lsp` 22,980,289 B): both had grown 17.5% past the
  2026-10-04 budgets, which only the release checked. Locked by `release_assets_lock`.
- **`largo run` and the optimizer finish on a recursive function that calls itself inside a loop.** The recursive
  inliner unfolds such a function eight levels deep, and the abstract interpreter proves each unfolded loop's entry
  facts with Fourier–Motzkin elimination, which can square its constraint set at every variable it eliminates: on
  `walk(n, base)` calling `walk(n - 1, acc)` four times in a loop, `largo run` and `compile_to_program_optimized`
  ran past 30 GB and never finished (0.10.1 as well). The optimizer's prover now eliminates under a budget of 4,096
  constraints a step and reads a proof past it as not found, which costs at most the optimization it would have
  licensed; the compile peaks at 19.6 MB. Locked by `compiler_stack_safety_lock` and the kernel's `lia` units.
- **The fast suite leaves every D5005 card co-simulation to `run-silicon-tests.sh`.** The card lock's tests that
  start Verilator take 600–3,400 s each; seven added since the on-demand ruling ran into the fast suite's 30-minute
  cap. `ondemand_exclusion_lock` now derives the co-simulations from the test file (each test that starts Verilator,
  directly or through the file's helpers) and holds the silicon selector to exactly them.
- **The per-push binary-size check installs the linker the test workflow links with.** `test.yml` links every Linux
  build with mold through clang, and the job that holds the shipped binaries to their budgets installed neither, so it
  failed at its first build script on every push. It installs both, and `workflow_linker_lock` holds every job that
  installs Rust to the packages its workflow's link line names.
- **A closure's captured integer and a popped value keep what they are on the collected tiers.** On the bytecode→C
  tier a closure that captured a loop variable crashed: its body never learned the kinds of what it captured, so `x +
  i` ran as a machine add while every caller read the result as an exact integer. A closure body's captured values are
  now settled with the parameters, from every `MakeClosure` that builds it. A popped value is the item it is
  (`resolve_pop` stops the program before any pop that could find none) rather than a boxed optional, so `length of`
  and every other operation take it, and a pop allocates nothing. A program that asks for its heap report
  (`LOGOS_HEAP_REPORT`) gets it even when it allocated nothing, and the RISC-V runtime calls the C library's allocator
  nowhere: the C runtime's fallback for a build without the collector (`lc_gc_libc_source`) stays out of it, and a
  RISC-V build where the collector cannot run refuses to compile. Locked by `collected_heap_lock`, its rows sized and
  shaped so each collects at least twice on every collected tier.
- **The published crates build on the Rust they declare, 1.88.** `logicaffeine-compile`'s one-time-pad ledger took
  its file lock with `File::lock`, which Rust 1.89 added, so building 0.11.0 on Rust 1.88 failed. The ledger locks
  through `fs2`, as the crate's other file locks do; CI's `msrv` job checks every published crate on 1.88.
- **The D5005 Quartus scripts keep to the job's own directories and signal only the processes they started.**
  `quartus.sh` kept its IP cache at `/mnt/build/d5005-quartus/ip-cache`, a path of one machine; its default is now
  `ip-cache` beside the job directories (`QUARTUS_IP_CACHE` still overrides it). `stall_watch.sh` finds a stalled
  step's processes by their group id (`pkill -g`, `pgrep -g`) instead of signalling a negative pid, the form
  `signal_blast_radius_lock` keeps out of the repository. Locked by `d5005_quartus_queue_lock`.
- **The release builds its Windows binaries.** A Windows runner's git checks text out with CRLF line endings, and
  the prelude's build script reads each module's source line for line against its spliced code, so every Windows
  build of v0.11.0 stopped in `logicaffeine-compile`'s build script ("its spliced code is not a sequence of its
  source's lines"). Each release job that checks out on Windows keeps the tree's own LF line endings first
  (`git config --global core.autocrlf false`). Locked by `release_assets_lock`.
- **The release's VSIX install gate launches VS Code on macOS.** VS Code 1.110 names its macOS executable for the
  product (`Code`), and the `@vscode/test-electron` the tag pins (2.5.2) launches `Contents/MacOS/Electron`, so the
  darwin-arm64 gate died at `spawn … Electron ENOENT` and held the release back. The gate installs
  `@vscode/test-electron` 3.1.0, which reads the executable's name from the app's `Info.plist`, before it runs.
  Locked by `release_assets_lock`.
- **`pi`, `π` and `e` are the `Real` constants wherever no exact type is declared, on every tier.** The quick
  guide's exact-functions row says a bare `pi` or `e` outside an exact context is a `Real`; every tier stopped instead
  with `Undefined variable: pi`, and `largo run` reported the generated Rust's failure as a compiler bug, so
  `Let area be pi * r * r.` never ran. Outside an exact context each is now the double nearest π or e, in every
  expression; a name the scope binds (a `Let`, a parameter, a closure's parameter, a loop's pattern, an arm's binding)
  is still that binding, and `pi(digits)` still the library's digits. Locked by `real_constants_every_tier_lock`.
- **A `Real` shows one text in an interpolation hole, a record, a map and a padded hole on both C backends.** The
  AST→C backend rendered a `Real` in a bare hole, a record's or map's text and a tuple's item with C's `%g`, six
  significant digits (`"{x}"` of π printed `3.14159`), handed a padded hole's double to `%s` or the centring helper
  (`{x:^12}` did not build), and held a `Map of Text to Real` or `Map of Int to Real` as an integer map (π showed as
  `3`, its text key as a pointer); the bytecode→C backend padded a `Real` with `%g` and refused to centre one. Each now
  renders the shortest decimal that reads back as the same double, as `Show` does. Locked by
  `real_text_every_tier_lock`.
- **A page that clicks its own element runs that click as a turn of its own, and a failing `When the page loads:`
  ends the program, on every tier.** A `Click` inside an action is the host's own click (`element.click()` in a
  browser, a click the in-process hosts deliver once the action's turn has committed), so the turn it starts commits
  or fails by itself: its failure is not the outer turn's, and the outer turn's write-back never overwrites what it
  wrote; a browser used to ignore the command. `When the page loads:` runs as the page mounts, not
  as a turn the page is handed, so its failure is the program's, as the browser's page main ends where its mount
  traps. Locked by `vm_turn_boundary_lock`.
- **A theorem whose premise presupposes what the other premises deny is a presupposition failure, not a proof.**
  `The king of France is bald.` presupposes that France has a king; beside `France has no king.`, accommodating that
  made the premises contradict each other, and the prover proved `France has a king.`, or any goal, from the
  contradiction. A presupposition is accommodated in its context, the other premises: when they are consistent and a
  kernel-checked contradiction shows they deny what a premise presupposes, `verify_theorem`, `prove_theorem_trace`,
  `theorem_certificate` and the Studio's theorem compile answer a presupposition failure naming the premise and what it
  presupposes. A premise is never its own context, and premises inconsistent without the presupposition are no failure
  of it. Locked by `phase142_vandersandt`.

## [0.11.0] - 2026-11-02

### Highlights
- **LGX: full-stack web applications in LOGOS, tested where they run.** Pages, components, stores and typed routes are written in the same English as the rest of a program; a page runs on the tree-walker, the bytecode VM and in the browser as WebAssembly, and `largo test` checks it on each. `docs/getting-started.md` and `docs/frontend.md` start there.
- **Tests, observation and replay are part of the language.** `## Test` blocks, `Expect that …`, `For all` properties and a sealed test world run under `largo test`, and a failing interpreted run writes a bundle `largo replay` re-runs exactly.
- **A standard library written in LOGOS**, multi-file programs with `Import`, and a checker that refuses at check time what could only fail at run time.
- **More ways to run a program.** C is a general backend beside new RISC-V and CUDA tiers, and the compiler lowers to Tenstorrent, FPGAs and sky130.
- **Language-model decode on FPGAs, compiled from LOGOS.** A tiny int8 transformer generates its golden trace token for token on two Artix-7 boards, alone and split across both, and the Intel PAC D5005's Qwen3.8 decode engine is written down from its arithmetic to its Verilog, each layer held bit for bit to the one above it.
- **Numbers past the reals.** Infinitesimals in any stated order, hyperreals with their roots, exponentials and logarithms, real numbers a program's own function defines, and supertasks that end in their exact state, bit-identical on every backend that holds the exact extension.
- **The kernel's integers are Lean's, and Lean 4 developments import**, re-checked by both kernels.
- **A toolchain for people and agents:** `largo mcp`, `largo check --json`, `largo explain`, and native HTTP serving with verified TLS.

**Upgrading from 0.10.1:** read Breaking changes below; every entry ends with its migration.

### Added

- **Benchmark runs on any machine, judged against the published run.** `benchmarks/compare-runs.py` says whether a
  run regressed: each suite against its yardstick (C, Node, postcard), only the LOGOS rows judged. On the same chip a
  row regresses when it is slower beyond the noise band and slower relative to the yardstick, so a machine that slowed
  down across the board reports drift; on another chip only the normalized ratios are compared, out-of-line rows are
  findings for that chip, and the chip's own speed against the baseline is reported; a changed codec size at the same
  commit regresses anywhere. `benchmarks/run-remote.sh <host> prepare|start|status|fetch` runs the full suite on
  another machine: it ships the tree, installs the rustc, Node and clang the published run recorded beside the
  machine's own toolchains, quiets the machine for the run (performance governor, sleep masked, package updaters
  paused) and gives every setting back when the run ends, then files the results under
  `benchmarks/results/machines/<chip>/<run>/` and judges them against the published run and that chip's previous run.
  Locked by `benchmark_compare_runs_lock` and `benchmark_remote_run_lock`.
- **LLM Heal: the Studio mends a reader's English.** A blue Heal button in Code and Logic sends the editor's text to
  `services/heal`, a Cloudflare worker that asks a model (NVIDIA's OpenAI-compatible endpoint, the key a wrangler
  secret, ten heals a minute an address, 8 KB a heal, allowed origins only) for the same text as clear, idiomatic
  LOGOS or logic sentences. The fairy flies to the first mended line, a card shows Before and After with Keep and
  Undo, and nothing lands in the editor until Keep. `pages/heal.rs` is the pure core (wire shapes, line diff, the
  flight's states, the pixelation schedule), held by unit tests, `studio_heal_lock` and the Studio e2e scenarios.
- **`readReals(path)`: numbers read at run time on every tier.** The `Seq of Real` a file of little-endian binary64
  values holds, the same bits on the tree-walker, the VM and its JIT, C, Rust, RISC-V and WebAssembly. A model's weights
  are data, not code: compiled in as statements a tiny model's cut was megabytes of code that C and RISC-V took minutes
  and tens of gigabytes over; read at run time they are a load. `Set W to readReals("w/W.f64").` fills a declared
  tensor. `weights_at_run_time_lock`.
- **Board B's own computer: an RV64 hart that compiles LOGOS programs from its flash.** `codegen_sva::hart` is an
  RV64IMAC + Zicsr + Zifencei core with M and U modes and PMP: an executable model (every RVC encoding as binutils
  decodes it, the riscv-tests suites, random programs equal to qemu-riscv64), its five-stage RTL with instruction and
  data caches in lockstep with the model, and a SoC (boot ROM, scratch RAM, a DDR2 controller with its calibrating
  SERDES PHY, the QSPI flash stream, the log UART and the terminal link) in lockstep with the board model
  `hart::soc::Board` register by register; the boot ROM refuses an empty flash and an image built for another
  bitstream. `codegen_sva::onboard` and examples `onboard_compile`/`onboard_image` build the compiler for the hart's
  Linux ABI (`scripts/hw/build-onboard-firmware.sh`) and its boot image; on the hart it compiles every library and
  silicon demo program to the host's bytes.
- **A scalar function lowers to a device region held to the VM.** `accel::region` lowers a function of the optimized
  program to a control-flow graph of checked cells whose device-lane run equals the checked run; `accel::c_model`
  generates the executable C device model a host links without an accelerator, equal to the Rust model at every lane
  shape.
- **The timing model has the D5005's measured delays.** `Delays::STRATIX10` (a carry bit, a DSP block and a RAM read
  in hundredths of a LUT level with its hop, from fit27's worst setup paths) and `STRATIX10_NS_PER_LEVEL`.
- **The Fabric Terminal: a LOGOS REPL on Board B's VGA screen, its programs run on the fabric engine.** One image
  holds the screen (80×30 text at 640×480, a small-font plane for side panels), a PS/2 keyboard and mouse port, a SLIP
  link to the host and the fabric engine with observer ports; `scripts/logos-terminal.py` edits, picks from a program
  library (`benchmarks/silicon/programs/terminal_library.txt`) and runs each program on the engine with its activity
  panel, per-block and per-cell counts and run totals drawn as the engine runs, `--check` running the whole library
  against the VM unattended. `fabric_terminal_lock` holds the protocol, the font, the emitted RTL, the screen frames
  pixel-exact against the reference picture, the activity counts against the engine clock and every library program
  on the board model ≡ VM.
- **A fitted D5005 image's timing netlist can be analysed in the Quartus queue.** `quartus.sh sta <job> <tree> <tcl>
  [ARG...]` runs a Tcl script in the Timing Analyzer on a fitted flat image or persona, the script given the project,
  its revision, the job's reports directory and the job's own arguments, its script kept beside its reports; it takes
  the queue, a log and its attempts as a build does and records no ledger row. `scripts/d5005/failing-paths.tcl` writes
  every endpoint a target period fails with its data delay, logic levels, skew and widest fan-out and the node driving
  it. On hybrid9e (fitted at 59 MHz) it found every one of the worst 200,000 endpoints at 10–12 ns of data delay whatever
  its logic levels: wire across the die, 70% of it through nets of 256 loads or more.
- **The timing model counts a net's loads.** `TimingGraph::fanouts` and `TimingGraph::wide`: each net's loads as the
  device builds them (a memory port the M20K or MLAB blocks it reaches, `memory_blocks`), what drives it, a register's
  `(* maxfan = N *)` counted as the copies Quartus makes of it, clocks left out. The 200 MHz stage checks of hybrid9's
  fast side hold each stage's count of named nets driving past 64 loads from logic or 512 from a register to a recorded
  ratchet (`HYBRID9_WIDE_NETS`) that only falls.
- **A co-simulated card run that stops moving ends saying so.** The co-simulation top ends a run in which no line of
  memory or of the host moves for a million compute cycles (`logos_cosim: stalled`), where the program's polls would
  have waited for weeks of simulated time.
- **A step that stops moving on the card says where.** A command the host gives up returns the last progress record it
  cut, and the core's progress record carries the engine's word, each context's in its 16 bits; the step program prints
  each context's word when a step stops moving.
- `sealBox(key, bytes)`, `openBox(key, box)`, `sealValueBox(key, value)` and `openValueBox(key, box)`: XChaCha20-Poly1305
  under a 24-byte nonce drawn from the system's entropy for each seal and carried in front of the box, so one key seals
  any number of boxes; a byte box and a value box never open as each other (`sealed_box_builtins`).
  `logicaffeine_system::aead` gains `hchacha20`, `xchacha20poly1305_seal`/`_open` and the box functions, checked against
  draft-irtf-cfrg-xchacha-03 and Wycheproof's 306 XChaCha20-Poly1305 cases; Wycheproof's ChaCha20-Poly1305,
  HMAC-SHA256 and PBKDF2-HMAC-SHA256 files are vendored and run natively and against `crypto.lg`
  (`aead_official_vectors`).
- `mldsa::sign_randomized`, FIPS 204's hedged signing; `mldsaSignCtxLogos` and `mldsaVerifyCtxLogos`, the LOGOS
  ML-DSA under a context string. The official vectors are vendored with the scripts that fetch them: NIST ACVP's
  ML-DSA-65 sigGen under contexts, sigVer and ML-KEM-768 key checks (`scripts/test/fetch-acvp-vectors.sh`), and
  Project Wycheproof's ML-DSA-65 and ML-KEM-768 files (`scripts/test/fetch-wycheproof-vectors.sh`).
- **The demos read their figures from ledgers, and the fusion book holds together.** `benchmarks/silicon/d5005/card_runs.json`
  records the D5005 card's decode runs from their committed logs (Qwen3.8-27B: one sequence at 1.012 s a step, four at
  3.14 tokens a second; Qwen3.5-0.8B at 0.160 s a step; every sequence token-exact to its f32 golden), and
  `d5005_card_runs_lock` re-derives every figure and, on demand, recomputes the identity of the program each run ran
  from the checkpoint. `fusion_map_lock` generates which fused symbols of the book's section 3.4 each chip's lowering of
  each model fires (`benchmarks/results/demo/fusion_map.json`: seven checkpoints on the CUDA lane and the D5005 engine,
  every refusal by name), and `layer_tour_lock` matches every English statement of a layer to the launch that computes
  it (`benchmarks/results/demo/layer_tour_qwen38.md`: Qwen3.8-27B's gated-delta layer, 26 statements in 10 launches).
  `sota_contract_lock` holds the book's index, streams and coverage counts to its ticket bodies; `frozen_reds_lock` pins
  every RED test handed to another agent, and `measured_fields_lock` refuses a board top that streams a constant where
  a measurement belongs. A ticket's RED tests can be staged behind the tests crate's `handed-reds` feature, compiled
  out of every gate. Section 12.20 of the book holds the demo tickets.
- **The D5005 engine folds each residual add and gated product into the matrix before it.** A residual add and a
  SwiGLU product are each one more operation on a matrix's sums as the streaming unit writes them back
  (`machine::Fold`: put, add, gated product; `machine::fold`, `Machine::folded`); `Streaming::with_folds` (`--folds
  <lanes>`) applies them in `logos_fold_lane_p` lanes, every value the vector job's bit for bit at every knob point, so
  Qwen3.8-27B's 128 residual adds and 64 gated products leave the vector side. The store is laid at the step's widest
  point, two slots sharing beats where no job holds both: a folded 27B token-row takes 2,249 beats where it took 3,584.
  A folded program refuses a build that writes its sums, and the other way, by its signature. `d5005_machine_lock`,
  `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_engine_card_lock`, `d5005_flow_lock`.
- **A Gated-DeltaNet memory can be flushed a window behind** (`StateFlush::Trailing { tokens }`). Each window of logged
  tokens is replayed into the dense state over the next window, a `1/tokens` share of every head's rows a token, from
  the rows the read-out streams anyway, so a flush costs no state read of its own: the dense state is
  `StateFlush::Every`'s a window later, and a read-out reads through up to `2·tokens − 1` logged tokens, a knob point
  held to its own bar on Qwen3.5-0.8B and Qwen3.8-27B. `numerics::gdn_fold_trailing` folds a query through the newer
  window and through the whole log in one walk; `model::Replay` names a token's share. The head lanes replay each row's
  share as they read it (`--recurrence-behind <tokens>`), and a program flushed a window behind refuses a build that
  replays whole logs, and the other way. A lane's flush of a head can be split by rows over 1, 2, 4 or 8 flush units
  (`--recurrence-flushes <n>`). `d5005_numerics_lock`, `d5005_engine_lock`, `d5005_machine_lock`,
  `d5005_roofline_lock`, `d5005_capability_lock`, `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_engine_card_lock`.
- **The D5005 head lanes read far ahead and run a job's rows back to back.** A lane asks up to `LANE_CREDIT_MAX` = 256
  lines ahead (`Streaming::with_recurrence_credit`, `--recurrence-credit <lines>`), each head's fold double-buffered and
  the flush's tables in three banks: a Qwen3.8-27B-shaped token-row behind a 96-clock memory takes 16,837 clocks where
  it took 46,997. The fast side runs a recurrence job's rows itself (`logos_gdn_rows`, its own rows table a context):
  the coordinator reads a row's log a row ahead into one of two copies, the lanes run on across rows, and a row past the
  first costs little more than its rows' stream; a sequence's next row starts once the last one's writes are on the
  card. `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_flow_lock`, `d5005_engine_card_lock`.
- **The D5005 head lanes make the gated norm.** A Gated-DeltaNet layer's gated norm folds into its recurrence job
  (`Job::Delta { norm }`, `machine::fold_delta_norms`, `Machine::delta_normed`), and `Streaming::with_delta_norm`
  (`--delta-norm`) norms and gates each head's read-outs in `logos_gdn_gate_p` as they leave the lane, inside the time
  the lane spends streaming the head's state: a vector job fewer a layer, where the card's profile put the gated norm at
  ~13 ms of a 27B token-row at 100 MHz. A normed build's core reads every normed job's norm word, and a normed build
  refuses memories rewritten every pass. `d5005_machine_lock`, `d5005_rtl_lock`, `d5005_engine_rtl_lock`,
  `d5005_engine_card_lock`, `d5005_flow_lock`.
- **The D5005 vector side multiplies and adds in one instruction, groups past the add's latency, and convolves on a
  path of its own.** `vector::Op::{MulAdd, MulAddScalar}` (the multiply, then the add, each rounded as the two
  instructions round) take the convolution from ten instructions to six, a convolution job 19% fewer clocks. A group of
  passes may run past the add's latency (`VECTOR_PASSES_MAX` = 32): at 16 passes the gated norm takes 22% and the head
  norms 24% fewer clocks. `Streaming::with_conv_path` (`--conv-path <lanes>`) runs every convolution job on
  `logos_conv_path`: the kernel read once a job into block RAM, each row's three ring rows and its x streamed through
  `logos_conv_lane_p` lanes (four products, the adds from +0 in tap order, the SiLU), ~3.3× fewer clocks on a 27B
  convolution at four lanes. A convolution waits on the card only for its own sequence's ring. A side whose convolutions
  run on the path holds eight vector registers (`CONV_PATH_VREGS`), every other job the machine lowers naming fewer, and
  faults a job that names a ninth; it recognizes a convolution job by a register of its descriptors' shape, and a vector
  side ends a memory store on its words' valids rather than their count, so the conv path's side keeps every stage
  within a 200 MHz clock. A side's registers can hold several passes of its deepest job (`Streaming::with_vector_depth`,
  `--vector-depth <n>`), a job of vectors that deep running its passes in groups. `d5005_numerics_lock`,
  `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_machine_lock`, `d5005_flow_lock`.
- **The D5005 array issues a held beat against two token-rows a clock.** `ColumnSet::with_token_pairs`
  (`--token-pairs`): each core keeps two token-rows' activation words, its processing elements twice or, with
  `--pe-pairs`, once in the doubled clock (`logos_matvec_pass_x2t`: one token-row at its middle edge, the other at the
  user clock's), two lanes and an octet lane a token-row, so a core's beat ring, control and share of its channel's feed
  serve 1,024 MACs a clock instead of 512; a step of `T` token-rows issues in `⌈T/2⌉` (`ColumnSet::issues`). The
  one-word lane sums its terms at their exact widths (30 and 23 bits where it carried 48) through one alignment
  shifter. `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_flow_lock`.
- **The D5005 token-row store is pipelined** (`logos_store_w512`, `STORE_READ_LATENCY` = 4). A build whose core reads
  the store for the host alone reads it through layers of 512 words, each its own block RAM read every clock and
  holding only the bank's word, every port registered beside the layers and the word gathered by registered ORs; every
  reader takes its word through `logos_landing` at the store's latency, and the vector unit's register file reads and
  writes through registers (`SIMD_P_READ_LATENCY` = 2). Quartus builds a 27B bank in 20,141 logic cells with no
  pass-through logic, where 71 layers each read under its enable took 151,220 and built every layer's read-during-write
  in the fabric. `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_flow_lock`.
- **Shallow memories ask for MLABs** (`rtl::ram::RAMSTYLE`). Quartus put every memory that asked for nothing in M20K
  blocks whatever its depth, a block per 40 bits of width: the memories of 32 words or fewer in a 27B build took 5,142
  blocks, 44% of the device, for 2.2 Mbit. Every engine memory names its style by its depth (`LOGOS_RAMSTYLE(words)`:
  MLABs to 32 words, M20K beyond; `LOGOS_RAMSTYLE_ASYNC` for a read with no register after it, its MLABs
  `no_rw_check`); an engine build's settings (`ofs::ENGINE_SETTINGS`) time an MLAB's read in the clock after its write;
  `scripts/d5005/ram_check.py` refuses an unstyled memory of 32 words or fewer and an MLAB read without a register, or
  transparently, without `no_rw_check`. `d5005_rtl_lock`, `d5005_flow_lock`.
- **The D5005 fast side's clock, worked from a fitted image's failing paths.** The compute side's paths between the
  core's, the card port's and the compute FIFO sides' registers are a multicycle of the compute clock
  (`ofs::compute_fifo_sides`): on the 55 MHz image the shell's compute-side endpoints failing 5 ns fall from 7,449 to 0.
  The compute clock's marks and release come through plain registers `MARK_LEAD` = 2 clocks ahead
  (`logos_compute_clock`'s `LEAD`), and the engine's fast ports reach their FIFOs through `PORT_PIPES` = 3 pipes each
  way. `TimingGraph::{fanouts, wide}` count a net's loads (a memory's as the blocks it reaches, `memory_blocks`; a
  register asking for a fan-out as its copies), and every 200 MHz stage check holds its stage's wide nets to a
  two-sided ratchet, each stage boxing the memory parts it builds itself. `quartus.sh sta <job> <tree> <tcl>` runs a
  Tcl script on a fitted image's timing netlist and records no row; `scripts/d5005/failing-paths.tcl` lists every
  endpoint a period fails with its widest net, and `scripts/d5005/failing_classes.py` groups them by it.
  `d5005_flow_lock`, `d5005_quartus_queue_lock`.
- **D5005 builds reuse the FIM's generated IP and stop a step that stands still.** `scripts/d5005/ip_cache.sh` keeps
  what an IP generation wrote, keyed by the hash of the IP sources and the Quartus version, once the design built from
  it has synthesized, and restores it into a tree of the same sources with every file checked, setting aside an entry
  that does not check. `scripts/d5005/stall_watch.sh` stops an IP generation or a synthesis whose processes take under
  thirty seconds of CPU and write nothing in three minutes, or that reaches its bound, and `quartus.sh` runs the step
  again. `d5005_quartus_queue_lock`.
- **A flat D5005 fit stopped after its retiming resumes at its finalize stage.** `quartus.sh resume` of a flat tree
  whose fit stopped once its plan and retime reports were newer than its synthesis's tries `quartus_fit --finalize`
  first, and runs the whole fit when the fitter refuses the retimed snapshot, as Quartus Pro 23.4 refuses one killed in
  its finalize stage ("Cannot load retimed database"). A refused attempt's output stays in finalize.log beside the job,
  the log taking one line with the fitter's reason, so the job is judged by the fit that follows.
  `d5005_quartus_queue_lock`.
- **Every D5005 program lists its jobs.** `ofs::job_listing` (`jobs.txt` beside the program; `d5005_afu
  --job-listing`) names each job at the words its job table opens it at, so a card profile reads job by job. A command
  the host gives up returns the last progress record it read, each context's word in it, and `run-cosim.sh` appends
  everything a simulation writes to `LOGOS_COSIM_LOG` as it runs. `d5005_engine_card_lock`, `d5005_flow_lock`.
- **A Hugging Face checkpoint is read in place.** `model::hf::HfCheckpoint` opens a directory as its maintainers
  publish it (`config.json` with a nested `text_config`, sharded safetensors and their index, the tokenizer files) and
  presents it as the GGUF llama.cpp's converter writes from it: the same architecture keys, tensor names, dims and
  values, so the spec, the binder and the reference read it unchanged. The converter's rearrangements are views over
  the stored bytes (Llama's rotation pairs, Gated DeltaNet's value-head order, gpt-oss's interleaved and transposed
  experts, latent attention's split key and value, expert stacks); its value steps (a norm's `1 + w`, a decay's
  `-exp`) are computed as it computes them, or in f32 as the checkpoint's forward does; every dtype but f32, f16 and
  bf16 (the 8-bit floats among them) is widened to f32 as it widens it; a key nobody reads, a missing shard and a dtype
  or tokenizer component the reader does not run are refused by name; and nothing is read whole (an 812 GB
  405B-shaped checkpoint opens and serves rows while the process reads a few megabytes). The tokenizer keys are the
  converter's too — its vocabulary, merges, special tokens, add flags and chat template, the SentencePiece path for a
  `tokenizer.model`, and the byte-pair pre-tokenizer named by the digest of its probe, encoded by `model::hf::HfTokenizer`,
  a port of the `tokenizers` library's encode (Oniguruma-semantics patterns over Unicode 16.0 categories, NFC over its
  normalizer's Unicode 9.0) that matches the library id for id; `tokenizer::Vocab::from_meta` reads them. Held to the
  pinned converter on tiny random twins of Llama 3, Qwen3, Qwen3-MoE, Qwen3.5 (with its draft block and vision tower),
  Gemma 3, gpt-oss and DeepSeek-V3, in bf16, f16 and five dtypes at once, and on the published Qwen3.5-0.8B,
  gemma-3-1b-it and Llama-3.2-1B-Instruct checkpoints: every key, value and tokenizer key, and the reference's logits.
- **The Samba book's sources are pinned and its numbers held to them.** `benchmarks/samba_papers.tsv` pins every
  bibliography entry of `benchmarks/PREPARE_FOR_SAMBA.md` (venue and year as published, a stable locator, the capture
  under `external/papers/` by `scripts/accel/samba-papers.py` with its SHA-256 and date), and `samba_papers_lock`
  finds every number the book attributes to a source in that source's text, a number the book derives being written
  out as its arithmetic. Re-reading at the source corrected the book: the gpt-oss-120b Copy bar is 740 tok/s as
  captured (an earlier excerpt said 743.4), the SN50 DeepSeek-R1 points are modeled, the Llama-3.3-70B bar is a
  per-user speed measured by SemiAnalysis, the speculative step times are the paper's figure values and their sums,
  SambaFlow's compile modes are cited to SambaNova's own deck and release notes, a hypersection claim no source bears
  out is gone, and the research-testbed access level now says what the testbed offers today.
- **Qwen3.8-Flash-Next is read from the checkpoint itself.** `model::ngram_hash` computes the row ids of its hashed
  n-gram embedding (the layer's splitmix64 multipliers, the heads' primes and offsets, the end-of-sequence segment
  reset, a cached context), held to transformers' module on sampled token histories and to the three integer tensors
  the published checkpoint stores. `scripts/accel/qwen4exp-headers.py` reads the checkpoint's tensor table and integer
  tensors by range request (about 2 MB of 360 GB), and section 7 of the Samba book is recomputed from it: every budget
  equals the tensor table, every tensor belongs to a component the book names, and the reading notes give each
  formula's file and line in transformers, vLLM and llama.cpp and every place they differ (the end-of-sequence token
  of a list, the indexer's tie order, the draft's reused selection, the configuration defaults the checkpoint leaves
  to code).
- **64-bit integers run exact on 32-bit lanes.** An RDU computes integers in INT32, and the n-gram hash, large indices
  and some samplers need 64 bits. `spatial::wide_int` lowers 64-bit add, subtract, multiply, and, or, exclusive-or,
  shifts and the remainder by any divisor below 2^32 to straight-line programs over 32-bit lane operations: 16-bit
  limbs so every limb product fits a lane, carries limb by limb, Barrett's reduction for the remainder. Each lowering
  carries a proof checked afresh: the arithmetic ones by their algebra (the program evaluated symbolically with an
  interval for every lane showing no operation wraps, its result an identity over the integers that the kernel checks
  by ring normalization and the second kernel re-checks where its exact evaluation fits, and for the remainder the
  bounds of Barrett's difference and the two cases of its correction), the bitwise ones and the shifts by a certified
  equivalence check of their bit-blasted circuits. The
  n-gram hash's row ids computed on 32-bit lanes equal `model::ngram_hash`'s.
- **The Samba book's scores and experiments are data.** `benchmarks/silicon/samba_cells.json` is the one home of
  every cell's score: its columns the measures SambaNova's papers define and the exam's own, each with its unit,
  direction, definition and source; its rows our side (simulated, modeled, measured, or not yet with its closers) and
  the published side under its citation, held by `samba_cells_lock` to the rules of a fair win (a modeled row never
  wins, a win carries its quality gates and beats every rival on its slowest run, a Plasticine cell is won only by a
  simulated row and an RDU cell only by a measured one). The schema waits on its approval before the first row.
  `benchmarks/silicon/samba_experiments.json` records every experiment of the climb, kept, rejected or of no effect,
  written from the lanes board's `EXPERIMENT SAMBA` lines, and opens with the dead ends of the GPU campaigns, each
  pointing at its record, so no dead end is walked twice.
- **Any model the author writes brings up on SambaNova's RDU from its checkpoint alone.** `largo emit rdu --model`
  takes a GGUF file or a Hugging Face directory (`model::open_checkpoint`) where it took Qwen3.5 and Qwen3.8 only: the
  deployment's kind programs are the decode step's distinct layer programs, found by the digest of each program up to
  its names and how it reads each weight (`spatial::streamed::layer_kinds`), each layer bound to its own tensors by the
  renaming its program aligns to; a weight the author declares Q8_0 folds as int8 values and block scales on the chip
  only where the checkpoint stores it so, and is read dense otherwise; each rotation's cache ships as a bundle constant,
  over the checkpoint's frequency factors where it reads them. Qwen3, Llama, a tied Llama, Qwen3-MoE and Qwen3.5 bring
  up on the SN50 with logits within the bf16 datapath's bound of the f64 reference (`rdu_bringup_lock`), a Llama from
  its Hugging Face directory with no GGUF anywhere and Qwen3.5 at widths of 2,880 and 7,168 with the reference's own
  first token. A host fills a bundle's tensors through `model::bind`: a weight through its view, a Q8_0 weight as the
  two planes the manifest names (`q8_planes`), a rotary table from the spec (`rotary_values`). Gemma 3 (windowed
  attention) and gpt-oss (windows, sinks, its biased and clamped mixture) bring up too. DeepSeek-V2's latent attention
  lowers at whole-tile widths — its 16-head, 576-wide step certifies on the SN50 — and its tiny checkpoints' 72-wide
  rows, Qwen3-MoE four times as wide and the Samba exam's open bring-up rows are frozen handed REDs, each naming what
  it needs.
- **A model's work shared between chips.** The state a decode step carries — every attention layer's cache, every
  gated-delta layer's memory and conv rings — is one contract every backend agrees on, read off the author's
  declarations (`spatial::handoff`) and stated in every RDU decode bundle's manifest. A prompt prefilled on the GPU
  target decodes on the SN50 bundle and the reverse, and a model split over two chips hands its hidden state on at the
  cut (`spatial_disaggregation_lock`); with fp32 elementwise on both sides every split holds the reference's bound.
- **Tensor parallelism at any width.** A model splits over chips its head counts do not divide — Qwen3.8-27B's 24
  query heads over a rack's 16 SN50s, one or two a chip, each chip with the KV heads its query heads read — and a chip
  with no share of a layer folds a zero partial; the program walk's fusions keep to one chip
  (`qwen38_tensor_parallel_lock`).
- **An unknown architecture names what it carries.** `largo emit rdu --model` refuses a checkpoint whose architecture
  name no arm reads by naming the families whose tensors it holds, or the nearest and its differences
  (`model::triage`).
- **Llama 4 from its spec.** Chunked-local attention (`within chunks of C`), a mixture scored by sigmoids whose weights
  scale its experts' inputs (`moe … by sigmoid … weighting the inputs`), the weightless QK norm and the attention
  temperature are language, spec, reference and author; a tiny Llama 4 prefills and decodes to the f64 reference on the
  VM (`model_conformance_lock`) and its decode step lowers whole on the SN50 (`spatial_nope_attention_lock`).
- **Qwen 2.5 and Mixtral read.** Their GGUFs and Hugging Face directories read as the converter writes them; Mixtral
  is read as its experts, never as a dense FFN (`model_hf_intake_lock`).
- **Every published spatial technique, mapped.** `benchmarks/samba_techniques.tsv` holds every compiler, mapper,
  runtime and hardware technique the Samba book's pinned papers describe, each quoted from its source, with what we have
  of it and the tickets that copy it (`samba_techniques_lock`).
- **DeepSeek-V3's routing, every tier.** `moe … by sigmoid biased by B within the top G of N groups scaled by S`
  computes DeepSeek-V3's, Kimi K2's and GLM-4.5's router — the selection bias that chooses but never weighs, the group
  limit, the clamped renormalization, the routed scale — on the VM, C and Rust; the author writes it from a spec, and
  the RDU lowering computes it in a whole decode step (`spatial_moe_routing_lock`).
- **Qwen1.5-MoE and Qwen2-MoE read; Qwen's gated shared expert written.** Their GGUFs (`qwen2moe`) and Hugging Face
  directories read as llama.cpp reads them — a softmax router whose kept weights stay as they are, beside a shared expert
  scaled by its sigmoid gate — and the author writes the gate, held to the f64 reference (`model_conformance_lock`). A
  checkpoint of an unknown architecture is triaged under every arm. The llama, qwen3moe, gpt-oss and llama4 arms no
  longer read a routed scale llama.cpp never applies; a Hugging Face configuration whose renormalization its GGUF cannot
  say is refused by name; a directory's SentencePiece model is read only where the converter reads one (Qwen2 and Qwen3).
  The f64 reference ranks a NaN router score below every number, as the chip's routing does, where it panicked comparing
  one.
- **Qwen3.5-MoE read and lowered.** Its GGUFs (`qwen35moe`) and Hugging Face directories read as llama.cpp reads them —
  Qwen3.5's hybrid with Qwen's mixture in every block, the kept softmax weights renormalized, beside a shared expert scaled
  by its sigmoid gate — and its decode step lowers whole on the SN50 (`spatial_moe_routing_lock`).
- **Qwen3-Next read.** Its GGUFs (`qwen3next`) and Hugging Face directories read as llama.cpp reads them, and every
  checkpoint now opens through a presentation layer that gives Qwen3-Next's value heads, grouped by key head as its
  converter writes them, Qwen3.5's tiled layout, so one Gated DeltaNet reads both. Its logits from a directory are
  transformers' own within a millionth (`model_hf_intake_lock`); on the SN50 it lowers whole from either source.
- **An attention scaled other than by 1/√hd is written.** Gemma 3 27B scores by 1/√168 over heads of 128; the author
  refused every scale but 1/√hd and now writes the attention `scaled by S`, held to the f64 reference on the dense Qwen3
  and Llama shapes (`model_conformance_lock`).
- **Qwen3.8-Flash-Next read, and its reference equals transformers.** Its Hugging Face directory reads as llama.cpp's
  converter writes it into a spec that carries hyper-connections, the per-layer n-gram embedding, QSA and Gated
  DeltaNet's sigmoid gate, and the f64 reference computes each: on a twin it equals transformers within a millionth
  at positions across an end of document and past QSA's budget, the prompt whole and a token at a time. The author
  and the RDU emitter refuse each part by name until they write it. Where the converter reads the configuration
  through transformers it loses QSA on the stack's layers; the reader keeps it.
- **Qwen 3.5 and Qwen 3.8 deployed through the generic path.** Every release, Qwen3.5-0.8B to 397B-A17B and Qwen3.8-27B
  and 2.4T-A95B, reads from its published configuration and lowers and certifies at its real widths on the SN50, one
  program per layer kind; Qwen 3.5's twin brings up from its directory. Qwen3.8 names Gated DeltaNet's gate activation
  (`output_gate_type`): swish is read as the SiLU the spec holds, any other refused by name. A Gated DeltaNet whose
  memory is within a tile but past one accumulator page of a row (Qwen 3.5's twin, four heads of 16) is blocked by
  head. The mixture's twin is the reference's at fp32 and on the bf16 datapath lands past its 2% bound (NUM-1).
  Qwen3.8-2.4T's own layout, its text model alone with its experts stored fused, reads as the converter reads it, and
  its twin brings up on the SN50.
- **A router row past one accumulator page: Kimi K2's 384 experts and the 512-expert mixtures' top 10 lower on the
  SN50.** A mixture whose router row is wider than an accumulator page is scored and routed in blocks of the row: a
  softmax from each block's maximum and sum, the router bias, sigmoid and selection a block at a time, each block's
  top K as candidates and the top K of the candidates; more picks than a page holds are stored and combined in pages.
  Every mixture option computes the VM's mixture at 300 and 512 experts, and Kimi K2 lowers at real width.
- **Every published configuration of a deployed release reads; Kimi K2 read.** Each release's published
  `config.json` reads without its weights (`HfCheckpoint::read_configuration`): the keys releases carry and twins never
  did (`auto_map`, training settings, transformers 4's serialized defaults, DeepSeek's `moe_layer_freq`, gpt-oss's own
  names) are read or refused by name. Kimi K2's tiktoken vocabulary reads without running its tokenizer code, its
  encode and every class of its pattern held to `tiktoken`, and the vocabulary held key for key to its published GGUF's;
  its twin brings up on the SN50.
- **FP8 checkpoints read in place.** A Hugging Face directory whose `quantization_config` is FP8 (DeepSeek-V3 and R1,
  Kimi K2, MiniMax-M2 and the FP8 releases of GLM-4.5 and Qwen3) reads as llama.cpp's converter writes it: each 8-bit
  float weight times its scale, broadcast over 128×128 blocks, bit for bit with the converter's own `dequant_simple`.
  Every other quantized format is refused by name.
- **GLM-4.5 read and deployed.** Its GGUFs (`glm4moe`) and Hugging Face directories read as llama.cpp reads them:
  NEOX rotation over half of each head, biased projections, per-head QK norms where the checkpoint carries them,
  dense lead blocks, then DeepSeek's mixture (sigmoid scores chosen with a routing bias, renormalized and scaled, a
  shared expert), the draft block after the stack read as the converter writes it. Its twin brings up on the SN50,
  and GLM-4.5, 4.6, 4.7 and 4.5-Air lower and certify (`deploy_kimi_glm_lock`). GLM-4's and GLM-4.5's vocabularies
  take the special tokens their converters name, and Phi-3's and GLM's rotation share is read from the rope
  parameters first, as the converters read it.
- **GLM-4 read and deployed.** Its GGUFs (`glm4`) and Hugging Face directories read as llama.cpp reads them: half of
  each head rotated by interleaved pairs, the projections biased where the checkpoint carries biases, norms on the
  attention's and the FFN's outputs, and one gate-and-up projection, which `model::layout` presents apart. Its twin
  brings up on the SN50 and both 0414 sizes (9B, 32B) lower and certify (`deploy_glm_lock`); GLM-4V's multimodal
  rotation is refused by name.
- **Phi-4 read and deployed.** Its GGUFs (`phi3`) and Hugging Face directories read as llama.cpp reads them: one
  query-key-value projection and one gate-and-up projection, which `model::layout` presents apart, NEOX rotation and
  no window. Its twin brings up on the SN50 and its 14B shape lowers and certifies (`deploy_phi_lock`); LongRoPE
  (Phi-3.5, Phi-4-mini) is refused by name.
- **Every Gemma 3 size lowers for the SN50.** Gemma 3 1B, 4B, 12B and 27B (its 1/√168 score scale) lower one program
  per layer kind, each certified (`deploy_gemma_lock`). Its twin brings up within 2.04% of the largest logit against the
  bf16 datapath's 2% bound, and within it on the SIMD path's fp32 witness: a handed RED on NUM-1 (`deploy_reds_lock`).
- **MiniMax-M2 read and deployed: all seven of SambaNova's published families deploy on the SN50.** Its GGUFs
  (`minimax-m2`) and Hugging Face directories read as llama.cpp reads them: the query and key each normed across every
  head at once (`Attention.qk_norm_whole`), half of each head rotated, sigmoid-scored experts chosen with a routing
  bias and renormalized. The author writes the norm, its twin brings up on the SN50 and M2's shape lowers and
  certifies (`deploy_minimax_lock`); the CUDA lane and the D5005 engine refuse the norm by name.
- **Six of SambaNova's published families deploy on the SN50.** Qwen 2.5, Llama 3, Mixtral, gpt-oss, DeepSeek-V3 and
  Llama 4: each family's tiny twin, written by transformers from the family's own configuration class, brings up from
  its Hugging Face directory with transformers, the f64 reference, the VM and the SN50 bundle agreeing on its first
  step, and every size the family publishes (Qwen 2.5 0.5B to 72B, Llama 3.2 1B to 3.1 405B, Mixtral 8x7B and 8x22B,
  gpt-oss-20b and 120b, DeepSeek-V3 at 671B's widths, Llama 4 Scout and Maverick) lowers for the SN50 one program per
  layer kind, every program certified (`deploy_qwen25_lock`, `deploy_llama3_lock`, `deploy_mixtral_lock`,
  `deploy_oss_lock`, `deploy_deepseek3_lock`, `deploy_llama4_lock`).
- **Soft caps, fused projections and values unlike the keys in width are written.** The attention sugar takes
  `capped at C` after its scale, each score `C·tanh(s/C)` before its softmax, and `Let Y be X capped at C` caps every
  element, on every tier (`tensor_softcap_lock`). The author writes Gemma 2's capped scores and logits, the fused
  query-key-value projection of Phi-3 and GLM-4 (each part, and each part of its bias, read through
  `View::FusedPart` under either rotation), and values narrower or wider than the keys, each on one chip and split over
  chips, held to the f64 reference (`model_conformance_lock`).
- **Soft caps lower onto the spatial chips.** The front end lifts an attention `capped at C` with the cap in its spec,
  apart from its scale, and `Let Y be X capped at C` as an op of its own. On the SN50 the sections form caps each
  scaled score before the mask, the flash fold caps each block's scores before its running maximum, and a capped tensor
  is one row section, each equal to the VM with scores hundreds of times past the cap included
  (`spatial_softcap_lock`); a Gemma 2 shaped decode step, its scores and logits capped, computes on the chip
  (`tensor_softcap_lock`). The CUDA lane and forward and the D5005 engine refuse a cap by name, the backward pass
  differentiates through it, and a census holds every file that reads an attention's spec to computing the cap or
  refusing it.
- **A Gated DeltaNet gated by a sigmoid.** Qwen3.8-Flash-Next's Gated DeltaNet gates its normed output by `σ(z)`
  where Qwen 3.5's gates by `silu(z)`: the sugar takes `with sigmoid gate Z`, the front end lifts the gate into the op,
  the SN50 computes it as the VM does, and the author writes it, a hybrid's prompt and step equal to the reference
  (`tensor_gated_delta_gate_lock`, `model_conformance_lock`). The CUDA lane and forward and the D5005 engine refuse it
  by name, and a census holds every reader of the op to its gate.
- **The published multimodal checkpoints read.** Llama 4 Scout and Maverick and Gemma 3 4B, 12B and 27B are published
  only as `…ForConditionalGeneration` checkpoints. The reader takes their text model under either prefix transformers
  has written, and its head beside it, the vision tower and its projector skipped as the converter skips them
  (`model_hf_intake_lock`); every published Gemma 3 size is held to its configuration (`deploy_gemma_lock`).
- **Gemma and Gemma 2 read and deployed.** Both read from GGUF and from their Hugging Face directories as llama.cpp's
  converters write them: Gemma 7B's heads wider than its width over its heads; Gemma 2's window on every other block,
  its scores and logits capped, its sandwich norms. The f64 reference equals transformers at f64 with the window and
  both caps binding on the twins (`model_hf_intake_lock`); each twin brings up on the SN50, and every published size
  (Gemma 2B and 7B, Gemma 2 2B, 9B and 27B) lowers and certifies at its real widths (`deploy_gemma_lock`). A
  configuration the GGUF cannot carry is refused by name, Gemma 3's `query_pre_attn_scalar` other than the scale
  llama.cpp fixes by depth among them.
- **Gated DeltaNet heads of any width lower on the spatial targets.** A head that is not whole tiles (16, 8, 48, 80)
  with a memory past one accumulator page of a row was refused; it is now widened to whole tiles by exact moves, stepped
  at that width with its own width's scale and norm, and moved back (`tensor_gdn_row_blocks_lock`).
- **An input read at two float widths composes.** On Wormhole a product reads its operands in bf16 and a sum in fp32;
  a program input both read was refused, and is now held at fp32, the product's read rounding it on the way in
  (`spatial_mixed_precision_lock`).
- **The flat mixture of experts moves only the chosen experts.** Widths that are not whole tiles no longer load every
  expert's weights into one site (`spatial_moe_options_lock`).
- **The RDU chip facts are their sources' figures.** Every published fact on the SN40L and SN50 opens its citation
  with the bibliography key of a pinned source and the figure as that source states it, and the figure is in the
  source's pinned text (`rdu_chip_facts_lock`; the SN50's product page and datasheet are now pinned). Capacities are
  read in powers of two, as the vendor counts (1040 PMUs of 512 KB are its 520 MB). The SN40L's HBM moves 1.6 TB/s as
  published, not the 1,843.2 GB/s a JEDEC model gave it, and the SN50's inherits it; the SN40L's peak is the 638
  TFLOPS its Hot Chips talk states; DDR and chip-to-chip rates and the PMU's capacity are facts; and a figure two
  sources state differently is kept beside the one the facts carry, with its source.
- **A work order for model bring-up on RDU-class spatial chips** (`benchmarks/PREPARE_FOR_SAMBA.md`). The fusion book
  leaves the spatial chips out of its SOTA contract; this book is their campaign ledger: SambaNova's own measures and
  published numbers as Copy bars (time per token, share of the roofline, bandwidth utilization, synchronization
  boundaries a token, socket scaling, speculative decoding), every access level from the papers to the vendor's lowest
  SDK with the adapter each needs, Plasticine's principles as the lowering must speak them, the 2019 Stanford Plasticine
  evaluation kit's interface (`get_model_info` modules, the traced Tungsten estimator), the model families a test can
  send with every feature each needs, Qwen3.8-Flash-Next (`qwen4_exp`) component by component with a Qwen 4 day-zero
  drill, an exam of 100 tests in 13 families, a hill-climbing loop with a ratchet, and 184 tickets in 18 file-disjoint
  streams. `samba_book_lock` holds its index, bodies, streams, dependencies, exam rows, citations, paths and counts.
- **One IR and one optimizer decide the fusions for every chip.** `tensor_graph` holds the front end, the one IR
  (`TensorGraph`), an evaluator equal to the VM bit for bit, the registry of every fusion class the book names
  (`ClassId`), the dataflow facts, the shared rule table with its numerics folds and its report, and the resolver a
  chip's link drives. Each chip answers every class through one `Shim` — the CUDA lane's (`LaneShim`, with the lane's
  kernel conditions and its own rules), the older forward's (`ForwardShim`, `cuda_stage`'s refusals word for word) and
  the spatial chips' (`SpatialShim`) — and a rule builds a class only where the chip links it whole. The shared table
  reproduces the lane's every fusion and launch on every tiny, wide and medium architecture under every numerics, and
  builds every fused class the book witnesses, each checked by the evaluator; the lane's link (`codegen_cuda::lane_link`)
  builds the lane's launches from the optimized graph. The spatial pipeline lowers the IR's ops through its own link
  (`spatial::link`), its layout, interval and Q8_0-view decisions taken once for the whole program; the older CUDA
  forward emits the graph the shared table fuses under its shim, every program byte-identical; the D5005 engine
  reads each merge it makes as the fused symbol the shared table built under its shim (`EngineShim`); and no backend
  keeps an op enum of its own or walks the source itself but those whose move is ticketed (`one_ir_lock`). Every rule
  of the table, and each chip's own, is proven with Z3: `tensor_graph::smt` states what a graph computes as terms over
  arrays, every op that is not an index map left open, and `logicaffeine_tv::tensor` discharges each rewrite on every
  witness and model cut, its proofs pinned by digest (`tensor_rule_proof_lock`); the book's map of fusions to chips is
  held to the shims' answers (`shim_lock`). Every speed
  on record has a floor five per cent under its best (`benchmarks/results/speed_floors.json`, `speed_floor_lock`).
- **The CUDA lane finishes a step's norms and rotations in the launches before them.** Three folds of the shared table
  link on the lane, each computing what its launches compute apart, bit for bit: the embedding's gather norms the row
  it gathers for the first layer (`embed_norm`); a step's query, key and value projections turn each head — its norm,
  its rotation, the key into its cache row — in the block that writes the head's last rows (`qkv_head_epilogue`); and
  an output or down projection, in Q8_0 or any block format, takes the residual add and the next norm, Gemma's
  post-norm too, into its last-arriving block (`gemv_norm_epilogue`). A mixture finishes the residual add and the next norm in its last step,
  the decode step's in its last down block and a chunk's or a batch's in its combine (`moe_deferred_finalize`); the
  attention quantizes its output in its own store (`attn_output_quant`), and a gated attention multiplies its output
  by its gate's sigmoid there too, each of a batch's sequences by its own row of the gate (`attn_output_gate`); the
  gated delta rule norms each head in its step kernel. A batch's sequences quantize their rows where they store them,
  each row set carrying its rows' quantized form into the table's (`row_write_quant`): no quantize launch follows a
  batch's attentions or delta rules, one launch fewer a layer. Each fold's norm is the norm launch's own reduction, played by whatever block runs it. Decode launches a
  token: Gemma-3-1B 238 → 133, Llama-3.2-1B 148 → 83, Qwen3-8B 328 → 183, Qwen3.5-0.8B 220 → 159, gpt-oss-20b
  196 → 99, DeepSeek-V2-Lite 327 → 274; Gemma's and Llama's layers are five launches. Each real model's decode census
  is held to a fixture (`tests/fixtures/lane_census/`). No lane kernel keeps an array in local memory: the
  attention's score runs mark which keys read a row by a mask, and the router keeps each pick in its own lane, the
  same bits. Each attention launch compiles only the path it takes past a row of one chunk and its run width: decode
  0.6–2.9% faster on the 3060, the batches of 64 2.7–3.6%. A fold norms its row in its last block only where that block
  is at least the norm's width; a narrower one played the norm's warps one after another, and DeepSeek-V2-Lite now
  decodes at 152.9 tokens a second (142.2) and gpt-oss-20b at 110.9 (106.1). `lane_compile_lock`, `cuda_lane_lock`.
- **Pointwise regions in one kernel.** The shared table's `elementwise_regions` takes any connected region of pointwise
  ops of one length and element type — a row-broadcast bias inside it, a member a later op reads written out beside
  the result — and the older CUDA forward runs it as one kernel, its products rounded as its kernels apart round them;
  the spatial chips run it as one section, every value it makes handed to its reader and never stored.
  `fusion_class_lock`, `cuda_elementwise_fusion_lock`, `cuda_fusion_census_lock`, `spatial_forward_lock`.
- **A chunk's last layer computes only its output row, its caches written whole.** `last_layer_rows` folds the tail of
  every attending architecture's last layer, token-sorted mixtures included, and keeps out every op writing a state the
  step carries (`Tensor::carried`, the front end's `carried_state`). `fusion_class_lock`.
- **A model is one LOGOS program, and every chip's artifact names the cut of it that it lowered.**
  `model::program` names each form of a model's program (`Cut`: the decode step, a prefill chunk, a batch, a layer
  of each, the embedding and head, and the split forms over tensor-parallel chips) and its `Identity` (the cut, the
  chips, the sha256 of the authored text), printed `LOGOS program: decode step, ctx 4096, sha256 <hex>`. The CUDA lane
  prints one such line before each form in its `.cu`. The D5005 engine's program is read off the decode step
  (`decode_engine::program::lower`: the author, the shared front end, the step graph, `lower_graph`), and its
  directory carries `program.lg` and `program.txt`; the old hand lowering stays only as the oracle
  (`hand_lowering`), byte-identical on Qwen3.8-27B's 11 program files. The SN50 bundle's manifest names the whole
  step and every embedding, layer-kind and head program, and `largo emit rdu --model <checkpoint.gguf>` reads its
  configuration from the checkpoint (`Qwen38Cfg::from_spec`, refused unless its decode step is the checkpoint's).
  `one_program_lock` recomputes every artifact's identity from the author, proves the SN50's 64 layer cuts are the
  decode step cut apart op for op, and keeps the set of program writers and the set of hand-written lanes from
  growing.
- **`largo bringup <checkpoint>`: one checkpoint on every chip, run and judged.** `logicaffeine_compile::bringup`
  is the chip interface (`Backend`, ten of them: the VM, its JIT, C, Rust, RISC-V, WebAssembly, the CUDA lane, the
  D5005, the SN50 and the fabric): each lowers the cuts it runs, writes its artifacts under `<out>/<chip>/` and names
  each one's cut, or refuses the model by name, each gap once with the layers it stands in. The command writes the
  model's program as `<out>/program.lg` and prints its identity and every chip's verdict; Qwen3.8-27B Q4_K_M comes up
  on the CUDA lane, the D5005 and the SN50 under one identity in 10.4 s. The checkpoint is a GGUF file or a Hugging Face
  directory as the Hub serves it (`config.json` beside its safetensors), and a directory writes the same program as the
  GGUF converted from it: `largo bringup` over Qwen/Qwen3.5-0.8B's own repository writes the decode step its Q8_0 GGUF
  does, sha256 `c67668ef959f…`, in about 6 s. A checkpoint the spec reader does not describe is refused in its words.
  With `--run`, each chip that brought the model up then runs it on this machine and prints `ran:` with its tokens,
  its tokens a second and how many lead the reference's (the model's f64 reference, or a golden given with
  `--golden`), or `did not run:` with the reason by name. A CPU tier builds one generation program and runs it once:
  every weight read once at run time with `readReals`, each sequence's caches, memories and rings declared afresh, and
  the decode step's own statements in a loop over the positions, the prompt fed a token a step; it is
  `<out>/<chip>/run/generation.lg`. The VM and the JIT run it in process, and C, Rust, RISC-V and WebAssembly build it
  once. CUDA is built by `nvcc` for this machine's GPU, the D5005 runs its engine's machine model when no card is here,
  the SN50's SDK is gated, and the fabric waits for the author's models to reach it. `--prompts N` (or `all`) runs a
  golden's first N prompts, each from an empty state and judged against its own continuation, a `sequence` line each.
  On every tiny architecture the VM generates the f64 reference's tokens. Qwen3.5-0.8B over all 32 prompts of its f32
  golden (440 prompt tokens, 499 generated, in English, code, arithmetic, French, Japanese and more): the GPU, the
  JIT, C and Rust each generate 499 of 499 of llama.cpp's f32 tokens, and the bytecode VM 32 of 32 over the golden's
  first two prompts. `bringup_lock`, `demo_bringup_run_lock`.
- **The bring-up scoreboard.** `scripts/accel/model_scoreboard.py` runs the compiled lane against llama.cpp b11635's
  `llama-bench` on one GPU in one boot: one warm-up and five timed runs a side, every run kept (llama.cpp's as its
  tokens over its own `samples_ns`), the box's load beside each row. `benchmarks/results/models/rtx3060_scoreboard.json`
  holds six families on the RTX 3060, and `model_scoreboard_lock` recomputes every mean and deviation, holds them to
  the ones llama-bench reports, and checks our side's compiled provenance and each program identity it ran.
  `model_lane --repeats=N` times N runs after a warm-up and prints their mean and sample deviation. The serving engines
  stand beside llama.cpp on the same protocol: vLLM (`scripts/accel/engine_bench_vllm.py`, FP8 weights) and ExLlamaV3
  (`scripts/accel/engine_bench_exl3.py`, 8.0 bits a weight), each in its own environment at batch 1, every run timed by
  the wall and by the engine's own clock, the checkpoint each ran hashed into the ledger.
  Every side also carries its agreement with the model's f32 golden (llama.cpp's run of the checkpoint on the CPU in
  `f32`): over the golden's prompts, the steps each generates its way before the first it does not, of the
  continuation's steps, and how far its gap between the golden's two top tokens at the first position sits from the
  golden's — ours from `model_lane`'s golden run, llama.cpp's from its own GPU run, each engine's from its bench's
  `--golden` run, every prompt generated for its own continuation's length. The batch rows run the lane's batch step
  beside vLLM at the same concurrency. Every side runs with the GPU to itself: it starts once no other process holds the
  card, a side another process joined runs again, and a card held past the wait is refused, each holder named in the
  log by pid, binary, memory and systemd unit. The lane it times is the executable its build reports writing, wherever the
  target directory is. llama.cpp's GPU run, the quality side, takes llama-bench's batches (2,048
  tokens in micro-batches of 512) and records them; at 4,096 its context did not initialize for DeepSeek-V2-Lite on 12 GB. A golden's JSON is
  valid UTF-8 where a continuation stops inside a character: each ill-formed sequence is written as U+FFFD.
- **The D5005 streaming unit quantizes its input blocks back to back.** Each weight block of a token-row's input was
  quantized to its last code before the next block's first beat was read: ~60 clocks a 256-element block and ~34 a
  32-element one, ~110–140 ms of a 16-row Qwen3.8-27B pass with the channels idle. `logos_act_blocks_p` takes a
  block's beats a clock apart or any clocks apart, the next block's first the clock after its last, and gives each
  block's codes, scale and place out in order a beat a clock behind its scale (`act_blocks_out`, its exact clocks);
  the streaming matvec reads every block of every token-row in one walk and writes each block's codes into the array's
  activations as they come. `d5005_rtl_lock`.
- **A vector job reads a weight it holds once a job.** A load of a tensor's span at stride 0 into a register no
  instruction and no other load of the job writes (a norm's weights, the convolution's taps, the gates' constants) is
  kept (`VectorJob::kept`, the load word's flag): the machine and the vector unit's side load it on the job's first
  pass alone and the register holds it for every pass after, where every head of every row read it again (a 20-head
  gated norm over four rows read its weight 80 times). `d5005_machine_lock`, `d5005_engine_rtl_lock`.
- **D5005 images can lay every streamed block in a slot.** With `--slots <n>` each block of a matrix laid in stripes
  takes a whole number of `<n>` bytes in the image, its block's bytes and then zeros (`ImageLayout::slot`), and the
  streaming unit's feeds gather each beat at a whole number of slots (`ColumnSet::with_slots`): at 16 bytes Qwen3.8-27B's
  Q4_K (144), Q5_K (176) and octets of Q8_0 (272) are whole and Q6_K's 210 is padded to 224, 1.6% of its streamed
  bytes. A program written with `--slots <n>` and a build carry the slot in their signature (`@16`), so neither runs
  on the other packed otherwise. `d5005_weight_image_lock`, `d5005_rtl_lock`, `d5005_engine_rtl_lock`,
  `d5005_flow_lock`.
- **The D5005 streaming unit can take Q8_0 eight blocks at a time.** A Q8_0 block is 32 elements, an eighth of the
  column's 256-element slot, so each issue of two digit passes carried one: Qwen3.8-27B's Q8_0 matrices, 7.9% of its
  bytes, took ~0.41 s of a 16-row pass's ~1.13 s of matvecs. With `--octets` a beat holds a row's eight blocks, the
  processing elements run them in one issue, and the array keeps each issue's sums in one of two buffers while the
  other's blocks leave for the lane a block a clock, every token-row's block 0 before any token-row's block 1, each the
  clock no add still holds its sum: every block is still a lone Q8_0 block to the lane, bit for bit, at a clock where it
  took an issue. A token-row's activation words drop to the K-quants' count (68 for the 27B where Q8_0 needed 544).
  The image lays Q8_0 rows in octets; a program written with `--octets` refuses a build without them by its signature
  (`Q8_0x8`). `d5005_rtl_lock`, `d5005_engine_rtl_lock`.
- **The D5005 decode engine runs Llama 3's rotations.** Llama 3.x scales each rotary frequency by a factor its
  checkpoint stores (`rope_freqs.weight`); the engine writes its rotary tables on the host, so the machine's image and
  the engine model divide each pair's angle by its factor, bit for bit what the lanes' tables hold, with no RTL change,
  and a checkpoint without factors keeps its tables byte for byte. Llama-3.2-1B, Llama-3.1-8B and Llama-3.1-70B are
  admitted (`benchmarks/silicon/d5005/capability.json`); a factored checkpoint without its table, or with a table
  that is not one factor a pair, is refused by name. `d5005_machine_lock`, `d5005_engine_lock`,
  `d5005_capability_lock`.
- **The D5005 foundry's tools: RTL proven, timed and pipelined before Quartus runs, and every build on record.**
  A certified equivalence engine (`logicaffeine_proof::{aig, sweep, seq_equiv, lrat}`), a cycle-ratio loop bound and
  network-simplex min-cost flow, a timing graph and a minimum-register auto-pipeliner in `codegen_sva` that cuts a
  module (or a pipeline afresh) at any clock and proves the cut, a Quartus ledger that records every build's
  resources, slack, hold delay and clock routing (`benchmarks/silicon/d5005/quartus_builds.json`), one queue for every
  Quartus run (`scripts/d5005/quartus.sh`, which waits out the license manager's start-up check before each attempt),
  the timing model calibrated against every recorded synthesis (`benchmarks/silicon/d5005/timing_calibration.json`),
  an incremental certified SAT sweeper (`logicaffeine_proof::fraig`), equivalence over units proven apart
  (`logicaffeine_proof::units`, the `rtl_equiv` example) and of valid-ready streams token for token
  (`logicaffeine_proof::streams`), the binary32 semantics family a hard FP block is identified by
  (`benchmarks/silicon/d5005/fp_discriminators.json`), and the persona that identifies it on the card
  (`codegen_sva::fp_oracle`).
- **The D5005 fast side can build its f32 multiply and add on the card's floating-point DSP block.** Soft, a
  `logos_fadd_p` is ~650 ALUTs and a `logos_fmul_p` ~390 and a DSP: hybrid4d's recurrence fold alone is 81k ALUTs of
  them, and a sixteen-row build with every unit on the fast side comes near the device. `Streaming::with_hard_float`
  takes the oracle's measured blocks (`codegen_sva::fp_hard::Block`) and the engine's `logos_fmul_p` and `logos_fadd_p`
  become the block's wrappers at the soft units' ports and latencies, correcting what the block computes otherwise
  (denormal operands and results flushed, every NaN the engine's), or the build is refused by name. A simulation
  carries the block as `sim/logos_fp_mac_model.v`, a stand-in of the measured semantics; the hardware takes the
  device's own. `d5005_flow_lock`, `d5005_engine_rtl_lock` (the whole fast-side step on a block of the engine's
  arithmetic and on one of IEEE 754's). The card's own block flushes a tiny product before it rounds, which no wrapper
  makes the engine's, so a build can put its adds alone on the block (`--hard-adds`), its multiplies soft. What the card
  measured is a receipt (`benchmarks/silicon/d5005/fp_hard_receipts.json`, `codegen_sva::fp_receipt`), recorded only
  from a build that passed, whose discriminator ledger regenerates the modules the oracle writes itself of the engine it
  compiled (the library it embeds held by the build's SHA-256, free to move on), on the image the card's run
  configured without a reconfiguration error, and read again from the inputs kept beside it: fp-oracle5's run says
  the block's add is the engine's own add three registers deep, which the engine's four-clock unit wraps with one
  register after it and nothing corrected; its multiply is no member of the family (the nearest class differs on
  1,086,916 of 33,555,299 steps); its multiply-add rounds twice. fp-oracle6, measuring the family with the multiply's
  XOR-signed NaN, names it: denormal operands and results flushed, tininess decided after rounding to 24 bits, the sign
  kept, every NaN the default quiet NaN signed by the operands' signs (the engine's own tininess differs on one step of
  33.5 million), which no wrapper makes the engine's multiply; every member's arithmetic is computed in software bit
  for bit (`Semantics::mul`, `Semantics::add`) and a wrapper is planned for any target (`fp_hard::plan_for`), so a build
  can take the card's multiply at its own semantics. `d5005_fp_receipt_lock`, `fp_hypotheses_lock`.
- **The D5005 fast side's deepest stages are split for a faster user clock.** Against the 0.55 ns a level fit27
  measured, the lane's sixteen-term unit sum (15 levels), the binary16 rounding's denormal shift (11), the exponential's
  nearest integer (13), the logarithm's integer conversions (14) and the activation code's placement (11) each took one
  stage; they now take two, their latencies (`LANE_VALUE_LATENCY`, `F32_TO_F16_LATENCY`, `RNE_P_LATENCY`, `LN_LATENCY`,
  `ACT_CODE_LATENCY`) carrying the extra clock to every unit that waits on them, the vector side's narrowing wait among
  them. `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_engine_card_lock`.
- **The D5005 recurrence unit takes a binary16 line a clock, and attention runs on the fast side.** At Qwen3.8-27B's
  widths the recurrence side spent 1,294 + ~95·count cycles a head on an ordinary token and 58,947 on a flushing one,
  ~127 ms a token-row at 100 MHz. `logos_gdn_read_w` and `logos_gdn_flush_w` take 16·W elements a clock with each
  lane's W terms through W chained adds, so every element still meets lane i mod 16 in the order numerics::fdot adds
  it; the flush holds a head and runs its dot and update passes beside each other; and the side reads the next head's
  log and folds its query while this head's rows come back. At W 2 it measures ~36 ms a token-row (`--recurrence-width
  2`). Attention left the divided core, where each key and value line was a request and a wait through the card port
  (~118 ms a token-row at 20 positions): `logos_attend_side` quantizes each query head and feeds `logos_attend_int8_p`,
  the int8 unit a line a clock, through the recurrence unit's port (`--attention`), and the core keeps no attention unit
  or quantizer. `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_flow_lock`, `d5005_engine_card_lock`.
- **A greedy head on the D5005 hybrid engine.** The head's read-out of Qwen3.8's 248,320-entry vocabulary is about a
  megabyte a token-row, so the token-row store held logits for a handful of rows. A greedy program
  (`ofs::write_greedy_program`, `d5005_afu --hybrid-program … --greedy`) asks the streaming unit, through the head's
  stream entry, for each token-row's arg-max instead: every beat its writer would store passes through `logos_greedy`,
  a four-level tree to the beat's first largest number and then each token-row's first largest so far, so the store
  keeps one beat a row (`StoreLayout::greedy`) holding `numerics::argmax`'s index and that logit, and the host reads that
  beat back. A build laid for greedy programs (`--greedy`) holds as many rows in the store as a row of logits did beats.
  `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_engine_card_lock`.
- **The decode engine's accelerator runs on the Intel PAC D5005.** Partially reconfigured into the card's OFS 2024.1-1
  FIM, the bring-up accelerator answers with its feature header and AFU ID, fills and checks each of its four DDR4-2400
  channels at one 72-byte line a cycle with every line read back equal, alone and all four at once, and moves a host
  buffer through a channel and back by DMA. The FIM's PF/VF mux routes PF0's VF1 to the PR region, but the DFL driver
  enables only as many VFs as ports it released, one, which is VF0, the FIM's loopback exerciser: so
  `scripts/d5005/afu-vf.sh attach`, run once the image is configured, moves the PF to the kernel's pci-pf-stub and
  enables every VF, binding VF1 to vfio-pci where the host has an IOMMU and leaving it to no driver where it has none,
  and `detach` puts the PF back on dfl-pci. `afu-vf.sh build` and `exec` build a host program against the backend the
  host takes and run it, and `afu-vf.sh clock` reads the user clock a GBS has fpgaconf set; `run-receipt.sh` and
  `run-load.sh` go through them. A host with no IOMMU (VT-d off) drives the VF through the physical backend
  `sw/logos_hal_pci.c`: BAR 0 mapped from sysfs, memory decoding and bus mastering on only while the program holds the
  function (its command word given back on close, at exit and on a signal), each buffer one huge page at the physical
  address the kernel's page map gives, a function behind an IOMMU that translates its addresses refused for VFIO, and a
  buffer whose frames cannot be read refused rather than handed to the card. A host's wait for a command is bounded by
  the command's progress, not its length: while a command runs, the core cuts a progress record every 2^16 of its
  cycles in which a line crossed a channel or the host channel or the engine's sequencer moved, so a step that takes
  minutes is waited for and one that stops is given up. `scripts/d5005/run-step.sh` decodes a model on the card's
  engine build. `d5005_flow_lock`, `d5005_engine_card_lock`.
- **The decode engine's Verilog is block RAM and shared units, fits the D5005, and is still the machine bit for bit.**
  Every table, the token-row store, the vector unit's register file, each column's activations and the attention
  unit's scores live in byte-lane memories (`spatial::decode_engine::rtl::ram`: the read address latched on the
  falling edge, writes on the rising edge under a byte mask). The line port queues its writes behind the card's
  `mem_wr_ready` and holds its reads until the queue drains. One streaming quantizer (`logos_act_stream`: sixteen code
  units fed a beat a cycle) serves every weight block and query head where a full-width quantizer stood for each
  block length; each vector lane's arithmetic is four multiplies and two adds every instruction shares; and a row of
  four function units (one exp, reciprocal, ln and reciprocal square root each, which sigmoid, silu, softplus and ln1p
  go through) serves the sixteen lanes a slice at a time, each unit idle unless an instruction calls it. The Verilog
  elaborates under a strict front end (pyslang, refusing implicit nets, unconnected ports, cut widths, sign compares
  and out-of-range selects). One engine runs every checkpoint it is the union of (`EngineParams::union`): Quartus
  Prime Pro 23.4 fits the one for Qwen3.8-27B and Qwen3.5-0.8B on the 1SX280 at 453,119 of 933,120 ALMs, 4,378 M20K
  and 2,385 DSP blocks, its single-cycle f32 units closing at 425 ns, inside the 640 ns its compute clock gives: the
  engine build's user clock of 100 MHz divided by 64 (`ofs::ENGINE_COMPUTE_DIVIDE`), the divider releasing its reset
  in its own clock. `d5005_rtl_lock`, `d5005_engine_rtl_lock` (the card's memory taking writes under random
  backpressure), `d5005_flow_lock`.
- **The D5005's memory as one map, and SECDED lines** (`spatial::decode_engine::{image, physical, ofs}`). The weight
  image is laid out from the machine's own layout (`image::image_of`): the tensors the machine reads in its order, a
  convolution kernel tap by tap as the machine stores it, so what the channels hold is the machine's memory byte for
  byte. Everything after the image — the rotary tables and every sequence's region — is dealt round the four channels
  in granules of the engine plan's state chunk from the first line past the image (`physical::PhysicalMap`), so each
  thing a job streams of a sequence — a conv ring's row, a value head's memory, a log's entries, an attention block's
  cache rows with their scales — is one stretch on each channel it touches. The card's EMIF runs with its controller's
  ECC off; a third line (`LocalMemoryLine::Data512Secded`) keeps each 64-bit word's SECDED(72,64) check byte in an ECC
  lane, word `w`'s in lane `7 − w`, so every codeword lies on 72 different DQ pins whether the controller's bus word
  runs beat by beat over all 72 pins or holds the data pins' beats beneath the ECC pins', and a failing pin flips one
  bit of each word, which the code corrects. The shell codes every line at each bank's port and corrects it in one
  registered stage behind it; the bring-up core injects flips and counts each channel's words corrected and lost, and
  the receipt proves the code on every channel. `d5005_physical_map_lock`, `d5005_weight_image_lock`, `d5005_flow_lock`.
- **The decode engine runs a model with its memory on the card's channels** (`spatial::decode_engine::{physical, ofs}`).
  The card port (`logos_card_port`) serves every logical line the engine asks for where the physical map puts it, one
  request at a time: a masked write is read and merged first, a write is ready only once its channel has written it,
  so a read after it reads it, and a line the SECDED code cannot correct faults rather than being coded afresh. The
  engine build of the accelerator (`AfuConfig::engine`) puts the engine behind the bring-up core, which configures it,
  writes its tables and store from host memory, runs a step with the channels handed to the card port, and reads its
  store back (`ENGINE_CONFIG`, `ENGINE_TABLE`, `ENGINE_RUN`, `STORE_READ`). `ofs::write_engine_program` writes what a
  host program runs a model with, and `sw/logos_step.c` decodes greedily from a prompt a token a step. Host programs
  run against the RTL under Verilator as under Icarus Verilog (a DPI-C backend beside the VPI one), and there the step
  program decodes the tiny model's tokens exactly as the machine does. Each channel's stream reader decides a burst from
  a registered chunk and room, and the engines' requests reach the platform's bridge through a two-deep queue.
  `d5005_card_port_lock`, `d5005_engine_card_lock`.
- **The D5005 accelerator built into the FIM itself** (`scripts/d5005/build-afu.sh --flat`). Beside the
  partial-reconfiguration persona, `--flat` builds the accelerator into the OFS d5005 2024.1-1 FIM from the FIM's own
  source (ofs-d5005, its ofs-common and the PIM's ofs-platform-afu-bbb at the release's tags, each archive
  SHA-256-pinned by `fetch-d5005-tools.sh`) through OFS's in-tree PIM build, the FIM's `afu_main` instantiating the
  accelerator in its port: one SOF holding both, loaded over JTAG with no partial reconfiguration, recorded by
  `record_afu_image.py` as a flat image and run by `run-receipt.sh` as it is. The FIM's static region instantiates
  its exercisers (`he_lb_top` on PF0 VF0) whatever its port holds, but the release's Stratix 10 `afu_main.tcl` loads
  their sources only when no PIM-wrapped AFU is configured, so a flat build's `sources.txt` names them itself, from the
  FIM that builds it (`QI:${OFS_ROOTDIR}/…/afu_main_std_exerciser_design_files.tcl`). Held by reading the whole
  project's settings with Quartus's assignment commands stood in for in Tcl (`fixtures/d5005_flow/
  quartus_assignments.tcl`): a flat build's project declares every module the static region instantiates, a persona's
  sources leave exactly `he_lb_top` undeclared. `d5005_flow_lock`.
- **The D5005 card-side scripts are in the repository** (`scripts/d5005/card/`), as dcdev-fpga runs them from
  `~/d5005/card`: a model decoded on a persona after a warm-up step (`card-step-warm.sh`), on a flat image loaded over
  JTAG (`card-step-flat.sh`), a job at a time with each job's cycles (`card-profile-flat.sh`, `logos_profile.c`), with the
  token-row store read at the last position (`card-probe-uclk.sh`, `logos_probe.c`), the port reset at the clock set
  (`logos_port_reset.c`) and the FME's partial-reconfiguration registers read (`fme_pr_regs.py`).
- **The D5005 crossing gate boxes a single-clock module instead of flattening it.** `cdc_check.py shell --single-clock
  <module>` first proves the module has one clock on its elaborated hierarchy (every clocked block one edge of one input,
  every child's clock wired to it, a latch, an event list or a derived clock refused) and then stands it in the shell as
  a box of that clock's domain, every value into it from that domain; an instance that sets its parameters is refused,
  the proof being made at its own. `build-afu.sh` boxes the engine, so an engine build's shell gates in a minute or two
  where flattening the engine through yosys ran for hours. `d5005_flow_lock`.
- **The streaming engine's matvec runs the machine's matvec jobs from the card's four channels, bit for bit.** The
  weight image lays every matrix a matvec job streams in stripes (`ImageLayout::stripe`): each channel's tile a row
  pair a core, block-major, so the column array's cores take the same block step together against one activation
  read. Each core issues a block against a token-row as passes — one per 4-bit code digit and per activation word,
  through one digit's processing elements — into a pipelined lane and a pipelined f32 add, every (row, token-row)
  adding its blocks in order. One array takes every format a checkpoint's matrices are in, the job's chosen; a channel's
  tile run walks into its image stream, its feed deals the blocks to its cores, and `logos_streaming_matvec` quantizes
  each token-row's input through a pipelined quantizer and writes the sums back into the store: every matvec job of a
  traced step is the machine's, its store bit for bit, and the array's clocks are its cycle model's exactly.
  `d5005_rtl_lock`, `d5005_engine_rtl_lock`, `d5005_weight_image_lock`.
- **The engine steps in the user clock under enables, and the hybrid engine runs its matvec jobs on the streaming
  matvec.** A partial-reconfiguration persona cannot put a clock of its own on a global network (the FIM's base compile
  owns them all), so the divided clock reached the engine's quarter-million registers on general routing and the
  router spent hours fixing hold. Now the engine's core (`logos_engine_core_ce`) runs in the user clock: every register
  steps where the divider marks the compute clock's rising edge (`fedge`), every memory (`logos_ram_ce_w<W>`) reads the
  user cycle after its falling edge (`fmid`), a transform of the engine's generated modules that keeps it step for step
  the engine the divided clock ran. The shell meets it through `logos_engine_bridge`: the shell's values taken two user
  cycles after the rising edge, the engine's held from two after the falling edge; the SDC times every path at its
  multiple. `EngineParams::streaming` adds the hybrid's user-clock side (`logos_engine_fast`): the streaming unit and
  the token-row store, owned by the unit while it runs a job. The core reads each matvec job's stream entry, three
  lines where the tensor's table entry points; the image holds the streamed matrices in stripes at each channel's
  start, the rest of the memory compacted after them with the entries (`Compaction`), and the card port reads that
  flat space unchanged. The shell gives a hybrid's every channel a fast port, the stream engine serving one side at a
  time. `cdc_check.py` boxes the core and the user-clock side as the clock map names them and proves the bridge's
  structure (`related`); `write_hybrid_program` writes the program and `logos_step.c` refuses one written for another
  streaming unit. The whole step is the machine's at three clock ratios, and the hybrid AFU decodes the tiny model as
  the machine does through the step program under Verilator. A simulator runs each engine build's twin
  (`sim/logos_engine_gated.v`, `simulation_twin`): the same engine with its core's clock gated to the user cycles the
  core steps or reads its memories in, which changes nothing the core does and spares the simulator the rest — the card
  cosimulations run three times faster, and the twin leaves every store, memory and cycle count the engine does.
  `d5005_engine_rtl_lock`, `d5005_flow_lock`, `d5005_engine_card_lock`.
- **The hybrid engine runs its lazily flushed memories on the card's channels in the user clock.** A Gated-DeltaNet
  layer's state is most of a token's memory traffic, and through the core's card port every line of it waited for the
  divided clock. `Streaming::with_recurrence` puts the recurrence unit beside the streaming unit: the core hands each
  token-row (`GdnJob`) to `logos_gdn_side`, which reads the log's count, appends the row's entry from the store in whole
  lines, folds every head's query back through the log, reads every row of the head's memory out and, when the log
  fills, replays and writes the rows back, the outputs into the store, and writes the count back, on pipelined f32 units
  a beat a clock. Its memory port, `logos_fast_port`, places every line where the card port does, gathers each channel's
  consecutive lines into one descriptor and returns reads in the order they were asked; the hybrid shell's fast ports
  now write too, a FIFO of lines and one of done tokens a channel, each write's done back to the side that wrote, and
  the card port's map crosses to the engine through the bridge. The core keeps no lazy row engine, log append or count
  of its own, and the build runs the hybrid build's programs unchanged (`build-afu.sh --hybrid <pairs> --recurrence`).
  Beside the card port, the fast port's reads and writes land where the map puts them on all four channels at three
  granules; the whole step, every recurrence job alone and the engine's twin are the machine's. `d5005_card_port_lock`,
  `d5005_engine_rtl_lock`, `d5005_flow_lock`.
- **The vector unit's functions run in four stages, so the engine's core can step twice as fast.** The function lanes
  (exponential, reciprocal, logarithm, reciprocal square root and the chains through them) were one path of some 570
  logic levels from a register bank and back, the core's longest by half, and every core path waits the same half engine
  cycle. `logos_lane_func` takes `STAGED`: the exponential, the add after it, the reciprocal, logarithm and square root,
  then ln1p's correction and the choice, a step apart, no stage deeper than one function's core. The vector unit issues
  a `Map`'s rows a slice a step into them one row after another and writes each row when its last slice is out, four
  steps more an instruction. The hybrid build with the recurrence unit runs its core at the user clock over 64.
  `d5005_rtl_lock`.
- **The hybrid engine runs its vector jobs in the user clock.** With its matvec jobs and its lazily flushed memories on
  the units in the user clock, a recurrent hybrid's core still ran every vector job — the norms, the gates, the
  convolution, the rotations, the cache rows, the residual adds, the float matrices — a beat and a row a core step: for
  the 27B more of a token's time than its streamed matrices. `Streaming::with_vectors` puts the vector unit's side beside
  the recurrence unit: the core hands each vector job over by its header's place (`VectorSideJob`) and passes its words,
  and `logos_vector_side` runs it as the core's vector-job states did, on its own copies of the core's tables, the
  token-row store's user-clock port and the recurrence unit's port onto the channels, its memory stores gathered a line
  at a time. Its vector unit, `logos_simd_p`, runs the vector unit's instruction set on the pipelined f32 units with
  every value the same, a row a clock through each instruction's units; its function units, `logos_lane_func_p`, run on
  pipelined reciprocal, reciprocal square root and logarithm units beside the pipelined exponential, whose
  integer-to-float step now takes two clocks of its own (`logos_i2f_p`) so no stage of it is deeper than the user
  clock's period. The core keeps no vector-job states of its own (`build-afu.sh --hybrid <pairs> --recurrence
  --vectors`). Every stage of the user-clock side fits the 100 MHz user clock on the 27B fit's delays: a binary16
  rounding takes two clocks (`logos_f32_to_f16_p`), the recurrence unit's flush and its log entries are staged around
  it, a dot's lanes each register their operands before the add, a rotary block's steps and lanes are chosen a clock ahead and its elements placed by enables, and
  the fast port places a line from a register, the next line of a run a clock ahead. `d5005_rtl_lock`,
  `d5005_engine_rtl_lock`, `d5005_flow_lock`, `d5005_engine_card_lock`.
- **A vector job's passes run side by side on the user clock's vector unit.** A pass of a vector job is one chain of
  dependent operations: a dot a row each add's latency, a reciprocal square root through a function lane about a
  hundred clocks deep, a scale. The side issued one instruction at a time, so a job over many heads paid every unit's
  latency a head; the 27B's Gated-DeltaNet output norms alone were 13 ms of a token-row's 84 on the card. `logos_simd_p`
  now runs an instruction over a group of up to `PASSES` passes (at most `VECTOR_PASSES_MAX`, 32), each pass's
  registers rows and scalars of its own: a dot's or a sum's passes take turns in the add's round a subgroup of the
  add's latency and one at a time, each with its own running sums, each subgroup's rows to their end before the next
  subgroup's; elementwise and function instructions walk the passes one after another; scalar operations stream a
  pass a clock, so the latency every instruction pays is the group's, not each pass's. Each pass's operations keep
  their order, so every value is the one the pass computes alone. A job runs
  in groups when its passes stand alone (`VectorJob::grouped`): every vector load and store one length, from a slot or
  a tensor, no ring row, no Hadamard or rotation, every register a pass reads loaded or written earlier in that pass,
  and no pass loading store elements an earlier pass of its row stores. Its job word carries the flag (bit 54), which
  the decoder holds to the job. With `Streaming::with_vector_passes` (`d5005_afu --vector-passes`, `build-afu.sh
  --vector-passes`) the vector unit's side loads as many passes as its unit holds rows for, each into rows of its own,
  runs the micro-program once over them, and stores each pass from its own rows and scalars. `d5005_rtl_lock` (random
  programs over every instruction a group runs, a group against one-pass units each holding a pass),
  `d5005_machine_lock` (the 27B's gated norms and head norms run in groups, no ring, rotation or Hadamard job does, a
  pass's values its own whatever its registers start as, a store an earlier pass makes into a later pass's load runs
  the job alone), `d5005_engine_rtl_lock` (every vector job in groups the machine's, at five, three and, on sixteen
  value heads, sixteen passes; a group in fewer cycles than its passes one after another, and a group of sixteen in
  fewer than groups of five). At the 27B's widths groups of sixteen take four rows' gated norm from 4,482 cycles to
  3,508 and their head norms from 715 to 542.
- **A D5005 head lane flushes a head over several flush units.** A lazily flushed head's flush, every sixteenth
  token, ran its rows' dot and update passes over the whole log on one unit: at the 27B's widths about 10,900 of the
  clocks a flushing head takes, against 512 for its read-out, and over half a lane's time. Each row's update is its
  own, so `Streaming::with_recurrence_flushes` (`d5005_afu --recurrence-flushes`, `build-afu.sh --recurrence-flushes`)
  deals a head's rows over 2, 4 or 8 flush units in order (`logos_gdn_flush_w`'s `ROWS`), each through the same passes,
  and asks their rows back one unit after another. Every row flushes as one unit flushes it. `d5005_rtl_lock` (every
  head read and flushed as the numerics do over two and four units, at 32 and 128 wide, in binary16 and f32, through
  quick, slow and late writes; a flushing lane in fewer clocks over two units than one and over four than two, a
  reading one in the same clocks), `d5005_engine_rtl_lock` (every recurrence job on head lanes over split flushes the
  machine's).
- **A D5005 matrix job folds its residual add or its gated product into its output.** On the card a 27B token-row's
  gated products cost 6.0 ms at 100 MHz and its residual adds 1.7 ms, each a vector job that reads the matrices'
  outputs back from the token-row store. The gated product also sets the store's size: its gate, its up projection, its
  output and the residual live together, 3,584 beats a token-row. A matrix job now carries a `Fold`: `Put` writes each
  sum, `Add` adds it into the element its slot holds, and `SwiGlu` multiplies it by that element's SiLU.
  `machine::fold` (`Machine::folded`) folds an add of a matrix's output into the matrix job before it, and a gated
  product into its up projection's job, the gate's sums written into the product's slot. A job folds only where its
  jobs stand next to each other and the slots it no longer writes are read by no job after. The fold rides the
  matrix's word (`[31:30]`) and its stream entry (beside the greedy flag). `Streaming::with_folds(lanes)`
  (`d5005_afu --folds`, `build-afu.sh --folds`) has the streaming unit read each output beat back, fold its sums over 1
  to 16 lanes of `logos_fold_lane_p` (the function unit's SiLU, the engine's add and product) and write it under its
  lanes. The build and its programs name the unit `+folds`, so `logos_step` refuses a mismatch. The 27B folds all 128
  adds and 64 gated products. An engine without the unit refuses a
  folded step (`EngineParams::folds`, `capability::Gap::Folds`). `d5005_machine_lock` (the folded machine computes
  the unfolded one's logits and memory bit for bit at every knob point past a lazy flush, in a batch, holding only its
  live slots; the 27B's folds and store), `d5005_rtl_lock` (the fold lane is `Fold::apply` over the vector unit's
  operands), `d5005_engine_rtl_lock` (every matrix job of a folded step and the whole folded step the machine's over
  4, 1 and 16 lanes), `d5005_capability_lock`.
- **A Gated-DeltaNet memory can be flushed a window behind (`StateFlush::Trailing`).** On the card a 27B token-row's
  recurrence is 26.3 ms at 100 MHz, more than half of it the flush every 16 tokens, during which a lane reads no row.
  Under a trailing flush of `T` tokens each window is replayed into the memory over the next window: the token whose
  log holds `T + 1 + j` tokens replays the first window into share `j` of every head's rows (rows `j·hd/T` up to
  `(j+1)·hd/T`), so a token's flush is a `1/T` share beside its read-out. The log holds the window being replayed and
  the one after it. A replayed row reads through the fold over the newer tokens alone, every other row through the
  whole log. The fold walks newest first, so one walk gives both (`numerics::gdn_fold_trailing`). The dense memory is a
  flush every `T`'s memory, a window later, bit for bit. `model::Replay` is the schedule the model, the machine and the
  twin share: which rows read which fold, which rows a token replays, and what the log drops. The machine's log is a
  ring of `Replay::capacity` slots, its count line holding the slot of its oldest token beside its count.
  `TRAILING_TOKENS` is 8, half of `FLUSH_TOKENS`, so the log is as long as a flush every 16 tokens' log.
  `EngineNumerics::all` gains the trailing points. The pricer and the twin charge each token's share of the state read
  and written back, and the log's second window. `EngineParams::delta` names how the memories are carried, so a
  trailing step runs only on a build whose head lanes replay a window behind (`capability::Gap::Recurrence`).
  At the precise point a trailing flush parts from the token-by-token memory by 1.06e-4 of the largest logit. A flush
  every 8 tokens parts by 9.6e-5.
  Locks:
  - `d5005_numerics_lock`: the one walk is both folds, at every split of logs up to two windows, at both multipliers.
  - `d5005_engine_lock`: every row is the token-by-token memory a window later, bit for bit. A trailing flush parts
    from that memory by at most twice what a flush every window does. Every knob point stays within its bar.
  - `d5005_machine_lock`: the machine is the model at every trailing point. A trailing log is a ring of two windows,
    and a flush every window's log fills from its first slot. The every-knob-point test is split by flush, so each
    part stays under the 30-minute backstop.
  - `d5005_roofline_lock`, `d5005_twin_lock`, `d5005_capability_lock`.
  - `one_ir_identity_lock`'s engine family is re-recorded with the trailing points (the knob points grew): 240 pins
    added, each its flush-every-window twin's words, and the 485 before byte for byte.
- **The recurrence's head lanes replay a window behind.** `lanes_trailing_verilog(window)` builds `logos_gdn_lanes` for a
  trailing flush:
  - The coordinator (`logos_gdn_side` at `WINDOW`) keeps the log as a ring of two windows, its count line's second
    word the slot of its oldest token. It appends the row's entry after the newest, reads the log into the lanes'
    copies oldest token first, and hands the lanes the share the row replays (`co_lo`).
  - Each lane is `logos_gdn_lane_t`. Its batches stay four heads; `logos_gdn_fold_m` (`TRAILING`) keeps each head's
    fold over the newer tokens beside its fold over the whole log, and a row below the share reads through the first.
  - Each share's rows go to `logos_gdn_flush_m` as the read-out takes them. That unit replays the log's oldest window
    over the shares of a batch's four heads at once in one of three banks, while one bank takes the next batch's log and
    another this batch's rows, so a lane loads and folds a batch while the one before streams and the one before that
    replays. Groups of other heads fill
    each group's wait for its coefficients, so a 128-wide head's share of sixteen rows takes 532 clocks, beside the
    read-out's 512, where a flush every sixteen tokens costs a lane 10,913 clocks a head on the token that flushes.
    A batch's shares replay while the lane streams the next batch's rows. At the 27B's shape (48 heads on two lanes)
    over a memory that answers a line every other clock, a trailing token costs a lane 33,711 clocks, against 45,633 a
    token for a flush every sixteen averaged over its window (28,919 reading, 296,339 flushing). Behind a memory that
    answers every ask a line a clock, a trailing row of a log of twelve tokens takes 21,428 clocks on two lanes.
  - The engine builds it for a trailing step on head lanes (`rtl::engine::recurrence_log`), and refuses one elsewhere.
    A build and its programs name the unit `+behind<T>`, so `logos_step` refuses a program carried the other way.
    `d5005_afu --recurrence-behind <t>` (and `build-afu.sh`) builds one; `--behind <t>` writes its programs.
  Locks:
  - `d5005_rtl_lock`: `logos_gdn_flush_m` is `gdn_flush` over every share; `logos_gdn_fold_m`'s newer fold is
    `gdn_fold_trailing`'s; head lanes replay each row's share, from every slot of the ring and through a drop, at
    32 and 128 wide, at a line and half a line a clock; a trailing token takes a lane fewer clocks than a token
    flushed every sixteen, averaged, at the 27B's shape.
  - `d5005_engine_rtl_lock`: every recurrence job and the whole step on head lanes flushed a window behind are the
    machine's; an engine names its flush, and flushes a window behind on head lanes alone.
  - `d5005_flow_lock`: a build whose recurrence replays a window behind elaborates against the PIM, warnings included.
  - `d5005_engine_card_lock`: the step program refuses a trailing program on a whole-log build and the other way.
  - `d5005_engine_lock`: the trailing point's own bars on Qwen3.5-0.8B and Qwen3.8-27B (`run-model-bars-tests.sh`).
- **The recurrence's head lanes read a line a clock behind the channels' latency.** A lane, and the coordinator that reads
  the log into the lanes' copies, kept at most 32 lines asked ahead of those they had taken. Behind a memory that
  answers 96 clocks after an ask, that is a line every three clocks. A lane also drained every line of a head before
  copying the next head's folds and asking its rows.
  - `Streaming::with_recurrence_credit(lines)` raises the credit of a lanes build to up to `rtl::gdn::LANE_CREDIT_MAX`
    (256) lines. Each line queue is the power of two past the credit. Builds that give no credit keep 32 and their own
    queues.
  - A lane copies each head's folds into one of two buffers and tags each line with its head's buffer, and on a trailing
    lane with its head's slot in the batch. It asks a head's rows while the head before is still on its way through the
    read-out, and copies the next head once every line of the head two back is through.
  - A trailing lane drains only at a batch's end on a token that replays, before replaying the batch's share.
  - `logos_gdn_flush_m` addresses its logs at their exact widths, so a log place past the last slot wraps in simulation
    as it does in block RAM.
  - Measured at the 27B's shape (48 value heads 128 wide on two lanes, binary16, a log of eight tokens) over a memory
    that takes every ask and answers a line a clock 96 clocks later: a row took 46,997 clocks at a credit of 32
    (3.8 clocks a lane's line) and takes 16,837 at 128 (1.37 clocks a lane's line, the coordinator's lines and the
    first batch's fold included). The memory's latency costs a trailing row 395 clocks with the two buffers and
    1,797 with a drain before each head.
  - `d5005_afu --recurrence-credit <lines>` and `build-afu.sh --recurrence-credit <lines>` build one.
  Locks:
  - `d5005_rtl_lock`: a row on two lanes takes its lines at a line a clock (the lanes' and the coordinator's, past
    the first batch's load and fold) behind a memory answering the next clock, and the memory's 96 clocks of latency
    cost it at most four latencies and one a batch end a trailing lane waits at. `logos_gdn_flush_m`'s batches run over
    all three banks, in turn and out of turn, and with all three started and waiting at once. Lanes reading 128 lines ahead read out and flush
    every head as the numerics do over memories answering at random: flushed every sixteen tokens and a window behind,
    binary16 and f32, narrow heads whose rows are a quarter of the credit, the queue wrapping; and behind a memory slower
    than the credit, every lane and the coordinator holding all 128 lines owed.
  - `d5005_engine_rtl_lock`: every recurrence job on two lanes reading 128 lines ahead, flushed a window behind, is the
    machine's.
  - `d5005_flow_lock`: a credited build gives its lanes the credit and elaborates against the PIM. A build that gives
    none leaves the lanes their own. A credit below 32, past 256, or on a build without head lanes is refused.
  - `d5005_engine_card_lock`: six sequences decode at once on two head lanes reading 128 lines ahead, flushed a window
    of four behind, each sequence's tokens the machine's.
  - `d5005_flow_lock`: the 200 MHz checks time the trailing recurrence at hybrid9's shape flushed a window of eight
    behind and reading 128 lines ahead: the trailing lane, the window's flush unit, the trailing fold, the coordinator's
    ring and the lanes. Their parameter resolver follows a value through the instantiating module's localparams (a
    number, a sized literal, or one operation of two known values), so the flush unit's share and slots resolve.
- **The recurrence runs a step's token-rows back to back.** A row took the coordinator's count, append and log read,
  then the first batch's load and fold, then its rows, then the last batch's replay, one row after another: at the
  27B's shape (48 value heads 128 wide on two lanes, binary16, a log of twelve flushed a window of eight behind) 3,061 +
  2,302 + 13,544 + 2,510 clocks a row.
  - The fast side gives a recurrence job's rows on itself (`logos_gdn_rows`: each row's region from a copy of the rows
    table, each vector's store beat from a copy of the slots table, both kept by the host's table writes), so the core
    hands a job over once. `GdnJob` names the job's rows and its vectors' slots.
  - `logos_gdn_lanes` takes the rows as descriptors. The coordinator reads a row's count, appends its entry and reads
    its log into the copy its parity names (each lane holds two) while the lanes run the row before, and writes the
    count back once every lane has taken the row. A row of the region of the row before (two positions of one
    sequence) waits until that row is done and its writes written. The coordinator's appending reads of the store come
    first; the lanes' query reads wait for them.
  - `logos_gdn_lane_t` takes up to two rows and runs them back to back: each stage (loader, fold, streamer, read-outs,
    flush writer) holds its row's fields; a line's tag carries whether its row reads through the newer fold and its
    place in the share; replayed batches queue for the flush writer with their row's region and share; a flush bank is
    busy from its batch's load until its replay's last line is queued. `logos_gdn_lane` takes a row once idle.
  - Measured (27B shape, near memory): a row past the first takes 13,837 clocks, down from 21,420 a row alone (12,288
    of them its rows' stream); flushed every sixteen, 14,428 against 16,543. 3.3 ms a token-row at 200 MHz over the
    48 Gated-DeltaNet layers, down from 5.1.
  Locks:
  - `d5005_rtl_lock`: a job's rows on four lanes one after another, each the numerics' alone (logging, flushing every
    sixteen, binary16 and f32, writes quick and late); a window behind, replaying rows beside rows that replay nothing,
    a window wrapping the ring, a window dropped, on four narrow lanes and two at the card's width; every lane test
    of one row on the new protocol; at the 27B's shape a row past the first within a quarter more than its rows'
    stream, a window behind and flushed every sixteen (a row taken only once every row before it is done fails it); six
    rows faster through the coordinator than the lanes (a copy of the log reused only once its row is done), eight heads
    a lane replaying behind slow writes (a flush bank loaded only once its replay is done), rows of different shares
    behind a memory answering 200 clocks late (each line in its own row's share).
  - `d5005_engine_rtl_lock`: every recurrence job and the whole step, the fast side looping the job's rows, are the
    machine's (single side, head lanes, a window behind, contexts); with the channels writing each line 64 to 127
    clocks after taking it, a sequence's next row starts only once the row before's writes are on the card.
  - `d5005_flow_lock`: the build elaborates against the PIM, a one-context build keeps no context machinery, and every fast-side module times within a 200 MHz clock.
- **The vector unit multiplies and adds in one instruction.** `vector::Op::MulAdd` (a vector addend) and
  `Op::MulAddScalar` (a scalar one) compute `c + a·b` with the product rounded and then the sum, so each is a `Mul` and
  an `Add` in one pass over the rows. Their word is opcode 15, the addend in the scalar field. `logos_simd` feeds each
  lane's multiply into its first add in the same step. `logos_simd_p` registers the multiply's result into the add's
  operands `MUL` clocks after the read, with the addend delayed beside it, and the sum is out by the elementwise
  write. The causal convolution is a multiply-add a tap: six instructions where it took ten, and a register fewer. At
  the 27B's widths a convolution job of four token-rows takes 5,888 core cycles on the fast side, down from 7,242.
  Locks:
  - `d5005_numerics_lock`: a multiply-add is the multiply then the add, bit for bit, at either multiplier, for every
    pairing of signed zeros, the values about the least normal, denormals, infinities, NaNs and the largest finite.
  - `d5005_rtl_lock`: both vector units run the multiply-adds as the interpreter does, a vector addend and a scalar
    one, in place and apart; the random differentials between the units draw them, alone and in groups of passes.
  - `d5005_machine_lock`: every instruction word round-trips, the multiply-adds and the scales included.
- **The D5005 token-row store is laid as short as a row can be.** Slots were laid first fit, the widest first. On the
  27B's folded step that left a row of 2,502 beats where no point of the step holds more than 2,249 at once. Where
  the widest first leaves a row longer than the step's widest point (`StoreLayout::widest`), `StoreLayout::of` now
  tries a fixed sequence of other orders (`STORE_ORDERS`) and takes the first that meets it, or the shortest. The 27B's
  folded row is 2,249 beats, 37% under the unfolded 3,584, so the same block RAM holds 1.6× the token-rows. A step that
  the widest first lays at its widest point is laid as before. `d5005_machine_lock`
  (`every_store_is_laid_at_its_steps_widest_point`: the tiny model at every knob point and the 27B at the card's points,
  folded and not, at one row and sixteen, greedy or not; no two slots live together overlap).
- **The D5005 vector side waits for the card only where a load reads what its job wrote, and streams a float matrix's
  weights once for every row.** A probe that times each of the vector side's states, run at Qwen3.8-27B's widths, found
  two costs.
  - **Every load waited.** Every load of every pass waited for every write before it to be on the card, slot and
    tensor loads too, which never read what a vector job writes. The convolution's next row, another sequence's, sat
    behind its last row's ring store. A ring load now waits only when its row's sequence stored a ring row earlier in the
    job. A sequence's next row still reads the ring row its last row wrote, and no other load waits.
  - **Float matrices streamed per group.** A float matrix's job (the Gated-DeltaNet gates' alpha and beta, 48 rows of
    5,120 f32 each) held `FM_ROWS` = 4 rows' inputs and streamed its weights once for each group of four. At 16
    token-rows a context that is four streams of about 35,000 clocks a job, about 8 ms a token-row at 100 MHz. The side
    now holds every row's input, a group of `FM_ROWS` rows to its lanes, and gives each weight line to the lanes once
    for each group. The weights stream once for every row, and each row's sums keep `numerics::fdot`'s order.
  - **Locks.** `d5005_engine_rtl_lock`:
    - `a_convolution_waits_for_the_card_only_for_its_own_sequences_ring`: the convolutions are the machine's, and take
      fewer cycles than with every load waiting, or with every ring load waiting once any ring row is stored;
    - `every_float_matrix_job_reads_its_weight_rows_once_for_every_row` (ported from once a group of rows);
    - the twenty-head float matrices over lane groups of three rows, at gaps and in bursts.
- **The D5005 step program times a step job by job.** With `LOGOS_PROFILE` naming the job list `d5005_afu
  --trace-store` writes, `sw/logos_step.c` runs the prompt's last position a job at a time, each job from its word to
  the next job's alone, and prints each job's cycles before the step's record (`scripts/d5005/card/card-profile-flat.sh`
  builds and runs it on the card). The separate `logos_profile.c` it replaces is gone. `d5005_engine_card_lock`.
- **A D5005 build names its vector unit's function units, and its emitter refuses a flag it does not know.**
  `d5005_afu --function-units <n>` and `build-afu.sh --function-units <n>` give the engine's vector side `<n>` function
  units (a divisor of sixteen; four when none is named). At Qwen3.8-27B's widths a token-row's vector jobs take 9.95 ms
  at 200 MHz on eight (10.50 on four, 9.67 on sixteen): eight takes the Maps' share for +27.5k ALUTs and +176 DSPs.
  `d5005_afu`'s build form now refuses a flag its usage does not name, where it passed over one (a misspelt flag built
  the default).
  Locks:
  - `d5005_flow_lock`: the emitter gives the vector side eight and sixteen units, four when none is named, and refuses
    three and a misspelt flag before emitting; a build's engine at eight; build-afu.sh offers exactly the emitter's
    build flags, each taking a value in both or in neither, and hands each to the emitter as given, the build's tree
    and line first.
  - `d5005_rtl_lock`: the user-clock vector unit runs every instruction as the engine's does at eight function units,
    and a group of passes as each pass alone at eight.

- **Reverse-mode differentiation of a tensor program, as a program.** `logicaffeine_compile::tensor_graph::vjp` writes the gradient of a scalar loss over a LOGOS tensor program as more LOGOS, so the VM, the fusion passes and every backend take the backward pass as they take the forward. Each op has a rule or a refusal that names it, and each rule is checked against central differences in f64.
- **LGX: full-stack web applications in LOGOS, tested where they run.** A `## Page /path` file is a page, a `## Route` is a typed HTTP endpoint, and one project serves itself in development, exports a static site and builds into a single binary; none of this existed in 0.10.1. A page lowers to ordinary LOGOS that the tree-walker, the bytecode VM and the browser's WebAssembly all run. `docs/frontend.md` teaches the framework in the order an app is built, `docs/web.md` teaches the server half, and `docs/lgx-reference.md` lists every construct.
  - **Pages and components.** `## Page /todos/:id (id: Int)` binds typed path segments (a parameter with no `:slot` is refused), and a page file with no header is routed by its name (`pages/app.lg` is `/`). `State:` is a record of `Let` fields (`Let x always be …` derives one), `Actions:` holds the only code that changes it, and `View:` is strict HTML: every element closes, `<script>` and `<style>` are refused, the full WHATWG character-reference table is read, holes are escaped, and the template language has `{x | f}` filters, `{if}`/`{else if}`/`{else}`, `{for each x in xs, keyed by k, at index i:}` (counting from 1), `{inspect}`/`{when}`, `{let}`, `{raw}`, `{extends Site}`/`{block}` layouts and `{children}`; another template language's spelling (`elif`, `endif`, `endfor`) is refused with the LGX one. `Style:` is CSS scoped to its component by a real tokenizer (`:global(…)` opts out), `Head:` merges a page's own head HTML over the defaults by tag identity, `## Directive` gives an attribute behaviour on the real element, and `## Error page 404` answers an unmatched URL. `## Component Name (props)` is placed as `<Name prop={x}/>`: props are read-only, a `fn(T) -> Unit` prop runs a parent's action as the parent's turn, every placement is its own instance, and a missing, extra or mistyped prop is refused. `Element`, `Event` and `View` are nominal types the checker keeps apart from `Int`, and every page ships design tokens, a dark mode and a favicon unless `Largo.toml` sets `[web] theme = false`. `lgx_constructs_lock`, `component_model_lock`, `lgx_template_lock`, `html_entity_table_lock`, `style_scope_lock`, `head_lock`, `layout_lock`, `directive_lock`, `nominal_dom_types_lock`, `compile_web_theme_lock`, `lgx_vm_parity_lock`.
  - **Turns, stores, triggers and events.** One event is one turn: the action runs, derived fields recompute and the page repaints once. A turn is a transaction: a failing action rolls back every field it touched and reaches `When an action fails (error: Text):` with the component or directive it came from, and the page keeps answering. `## Store Cart` shares state and actions across components for the page load, `## Store Cart preserved across reloads` for the tab, and `## Store Cart persisted as "cart"` in local storage through a derived JSON codec. `## Store S shared on "topic"` replicates a store to every page and tab on the topic through the relay: a plain field is a Lamport-ordered last-writer register, and a `Tally`, `ConvergentCount`, `SharedSet of T` or `SharedSequence of T` field is a CRDT its author reads as an `Int`, `Set of T` or `Seq of T`. `Publish m on "topic".` reaches every `When a message arrives on "topic" (m: T):` handler once, the sender's own page included. `When <Store>'s <field> changes:` wakes every component that uses the store once per commit, and a chain of wakes past 16 is refused by name; a component placing itself is refused past a depth of 32; `When this component appears:` runs after the paint that drew it, departures before arrivals; and timers (`Every 1 second as clock:`, `Start the timer clock.`) run on the page's own clock with absolute due times. The trigger vocabulary also covers the page loading, a field changing, the route changing, going offline and online, a resize, a scroll to the bottom and a dropped file. `.prevent`, `.stop`, `.once`, `.capture` and `.passive` work on every tier, key filters are a closed set, an unknown modifier is refused with the nearest spelling (`.prevent.passive` is refused naming both), several listeners on one element all answer in declaration order, and a departed component's listeners are retired with it. `store_lock`, `shared_store_crdt_lock`, `store_cascade_lock`, `store_watch_reach_lock`, `recursive_component_lock`, `appears_after_paint_lock`, `timer_lock`, `timer_period_lock`, `turn_rollback_fuzz`, `event_vocabulary_lock`, `event_modifier_catalogue_lock`, `event_behaviour_modifier_lock`, `event_propagation_lock`, `once_modifier_lock`, `one_element_several_listeners_lock`, `keyed_listener_lock`.
  - **Data, forms and navigation.** `Get "/api/todos" as Seq of Todo, then loaded.`, with `Post`, `Put`, `Patch`, `Delete` and a multipart `Upload`, hands the action a `Result of T and HttpError`: a status outside 200–299, a body that does not decode as `T` and a refused connection are each an `Err`, never retried implicitly, and a typed fetch completes in-process on the tree-walker and the VM as it does in a browser. `bind={field}` connects an input, textarea, select, checkbox or radio to a field both ways and keeps focus and caret across repaints; `required`, `pattern`, `min`/`max` and the type-implied shapes block a submit on every tier as the browser does (`on-invalid={action}` fires once per failing control, `novalidate` turns it off), and a boolean attribute is its presence. `Navigate to` a path the page's own route serves stays in the page and `When the route changes:` hears it, Back included; another page's path is a document load. `typed_fetch_lock`, `typed_fetch_completes_on_every_tier`, `forms_lock`, `progressive_form_lock`, `page_navigation_lock`.
  - **Typed routes, sessions and CSRF.** `## Route GET /posts/:id (id: Int) -> Text:` declares one path. The answer type picks the content type (`Text` as text, `View` as HTML, a record as JSON through a codec derived from its declaration, `Response` for status, headers and cookies), a segment typed `Int` that is not one is answered `400`, and parameters without a slot bind from the query string or a form body. `Serve on port 8080.` answers every request concurrently from the route table, a failing request answering `500`; `Serve on port 8080 with handle.` hands every request to one `Request -> Response` function; `LOGOS_PORT` or `PORT` overrides the port. `route`, `routeStatus`, `routeContentType` and `routeHeader` answer a request with no socket, so a server is tested with `Expect`. `## Before each request (r: Request) -> Option of Response:` answers in place of the route when it returns `some`, and `## Redirect /old -> /new.` answers `302` without shadowing a declared route. A `-> View` function or route is served as its markup on every tier, and a statements body under a `-> View` header runs. `## Session` declares the visitor's fields (`the session's user`), carried in one HMAC-SHA-256-signed cookie that is loaded before dispatch and written back only when a request changed it (`LOGOS_SESSION_SECRET` sets the key). With a session declared, a `POST`, `PUT`, `PATCH` or `DELETE` without the visitor's token is answered `403` before the route runs, a state-changing `<form>` without `_csrf` does not compile, and a page's own scripted requests carry the token, which `largo serve` and a built binary place in each page document for its visitor (`no-store`; a static file is served as it is). A `## To` function that reads or writes a binding `## Main` declares builds on AOT-Rust and C, where the bindings become one record lent to it and a binary serves the handler on the thread that owns the record, so a session site ships as a binary; a call that can reach a binding before its `Let` runs, and a task launched on such a function, are refused by name. `route_request_response_lock`, `server_surface_lock`, `http_serve_cross_tier`, `view_function_server_lock`, `session_lock`, `csrf_lock`, `csrf_automatic_lock`, `csrf_page_token_lock`, `aot_module_globals_lock`, `aot_program_record_lock`, `examples_lock::every_example_program_builds_into_a_binary`.
  - **The web standard library is LOGOS source.** `http.lg` builds responses (`plain`, `html`, `json`, `notFound`, `unauthorized`, `forbidden`, `redirectTo`, `permanentRedirectTo`, `bytesResponse`), adds headers through `withHeader`, which refuses an invalid name, a control byte, a hop-by-hop header and `content-type`, and writes cookies `HttpOnly; SameSite=Lax; Secure` by default, refusing a name or value that would split the header. `web.lg` escapes (`htmlEscape`, `safeUrl`), reads paths, queries, form bodies and multipart uploads, and carries the fetch wrappers pages call. `static.lg` is the one MIME table `largo serve` and `largo build` read, and `staticUnder` refuses any `..` segment before touching disk; a missing file answers `404`. `session.lg`, `forms.lg` and `ui.lg` hold signed sessions, field validation collected as a `Seq of FieldError`, and 1-based pagination and text filters (`docs/lgx-stdlib.md`). `json.lg` parses RFC 8259 with exact decimals, leniently (`jsonTryParse`) or refusing duplicate keys (`jsonParseStrict`), with typed readers that answer a `Result`. `tcp.lg` opens sockets and certificate-verified TLS connections (`tcpConnectTls`, `tcpUpgradeTls`), and `postgres.lg` speaks the PostgreSQL v3 protocol: cleartext, MD5 and SCRAM-SHA-256 authentication, parameterized and prepared statements, NULL-honest getters, transactions, statement timeouts, cancellation, and `pgConnectSecure`, which is TLS or a refusal. `http_reply_and_cookies_lock`, `http_headers_lock`, `http_static_lock`, `mime_single_source_lock`, `json_conformance_lock`, `tcp_tls_lock`, `postgres_aot_lock`, `lgx_stdlib_guide_lock`; `postgres_protocol` pins every frame byte for byte.
  - **One implementation on every tier.** What a `## Page` or `## Component` lowers to is the only implementation: the tree-walker runs it as the reference, the VM runs it, and the browser runs the WebAssembly it compiles to with a small host (`crates/logicaffeine_compile/assets/web/runtime.js`) that applies the page's patch stream and routes events. A view value is data (`__View`) on every tier, `one_lgx_implementation_lock` reads the source for any second implementation's entry points, and a view written in a form the framework retired is refused where it stands with the LGX spelling (`legacy_dialect_gone_lock`). `crates/logicaffeine_compile/assets/std/vdom.lg` keeps the page's tree between paints: a commit diffs the new tape against the screen with keyed matching and minimal moves (reversing five keyed rows is four moves), a paint that fails half-drawn sends nothing, and a pre-rendered page is adopted rather than redrawn, so focus, caret and text typed before the module loaded survive; markup a browser's parser would rewrite (`<tr>` outside `<tbody>`, a block inside `<p>`) is linted at build time. A server draws with the same walk onto the same tape (`html of`, `-> View` routes, error pages, `Head:`), so the markup a server sends and the page a browser paints come from one renderer, and requests served at the same time never share a component instance. A page's module carries only the reconciler paths it reaches: the one-button counter page ships 29,439 bytes gzipped (`vdom_size_lock`). A list, map or text access is a call into one shared helper outside loops and keeps its inline fast path inside them (one program's gzipped module came out 42% smaller at the same hot-loop speed: reads unchanged, writes +6%, map loops −22%, text loops −20%), and `wasm_site_size_lock` holds every site to a byte budget. Every heap block carries its class and a count, a turn's garbage is freed at the safepoint after it, and a repainting page costs 0 bytes per turn at 2,000 turns and at 20,000 (`wasm_memory_bound_lock`); the heap grows by `memory.grow` and fails with a named `OutOfMemory` rather than corrupting. The retired host calls `beginPaint` and `moveChildren` are refused by name on every tier, and a patch stream carrying their opcode is refused whole in both wire formats. `view_value_tier_lock`, `lowered_treewalker_tier_lock`, `vdom_lg_lock`, `retained_tree_lock`, `hydration_lock`, `ssr_is_the_tape_lock`, `ssr_requests_do_not_share_instances`, `runtime_js_lock`, `patch_opcode_parity_lock`, `patch_stream_lock`, `program_run_document_lock`, `lgx_entry_abi_lock`, `vdom_calls_lock`, `bundle_size_lock`, `wasm_heap_soak_lock`, `wasm_heap_class_lock`, `wasm_alloc_site_lock`, `wasm_heap_grow_lock`, `wasm_heap_fuzz_lock`, `retired_builtin_lock`.
  - **Tested where it runs.** A page's `## Test` starts from a freshly mounted page and fails if it asserts nothing. It clicks, types, submits, chooses, presses keys, focuses, hovers, raises browser events, reloads, advances the page clock, answers fetches (`Given that GET "/api/todos" answers 200 with "…".`) or serves them from the program's own routes, mounts one component on its own and replays a journal; it asks `the text of`, `the value of`, `the count of`, `is visible`, `is disabled`, `is checked`, `is focused`, `the title`, `the URL` and `the journal shows a then b`. A selector the engine does not evaluate (`ul > li`, `li:first-child`) is refused by name instead of matching nothing. `largo test` drives every page test through one driver (`page_host`) on the tree-walker and on the VM and reports a divergence as the first statement at which the documents differ, with its line; `largo test --browser` adds headless Chrome, driven over its DevTools pipe with a paused virtual clock, trusted input and recorders for clipboard, notifications and downloads. Known divergences sit in a shrink-only ledger: an unlisted divergence fails, and so does a listed one that has started to agree. Before anything runs, every name a view uses, every handler, `bind`, placed component and prop is resolved where it is written with a did-you-mean; `largo check` adds structural errors and accessibility warnings (an image without `alt`, a nameless button, an unlabelled control); and every refusal, lint, template error, CSS error and runtime failure carries `file:line:col` to `largo check --json`, the dev overlay, the terminal and the language server. `component_test_verbs_lock`, `page_query_lock`, `serve_fetches_lock`, `selector_subset_lock`, `page_trace_lock`, `lowered_vm_tier_lock`, `vm_session_lock`, `browser_tier_lock`, `browser_tier_cannot_skip_lock`, `testrun_page_tiers_lock`, `view_resolution_lock`, `lgx_check_lock`, `diagnostic_location_lock`, `runtime_failure_location_lock`, `vm_failure_site_lock`.
  - **Shipping a site.** `largo new <name> --web` scaffolds routes, `pages/` and `static/`, and `--site` scaffolds a site with no backend. `largo serve` serves a site, a full-stack site or a backend as the project requires, rebuilds and reloads on save, shows a page's compile error at its line in place of the page, and proxies what the pages do not answer to the backend. `largo build --static` writes a `dist/` of pre-rendered pages, content-addressed modules, `static/**`, `robots.txt`, `sitemap.xml`, `_headers`, a `_redirects` rewrite per `:param` page and `404.html`, deployable to any static host; beside a `Serve` entry, `largo build` embeds the pages and `static/` in the binary, answered before its routes on an exact path, so development and production answer the same bytes. Modules are cached immutably by content hash, every asset carries an `ETag` answered `304`, assets are compressed once at build time and negotiated on `Accept-Encoding` with `Vary`, a cache-busting query string resolves, every page is served as `text/html`, a project's own `favicon.svg` is linked, and a built binary answers `/_relay` itself whenever its pages use the relay. `LOGOS_HOST` sets the bind address, loopback by default. `largo expand <file>` prints the ordinary LOGOS a page or route lowers to, and `largo emit wasm-dom` writes a page's module, host and document. `real_website_lock` builds a four-page site and serves it both as files a static host maps by name and as one binary answering over a non-loopback address. `static_export_lock`, `error_page_ships_lock`, `dev_prod_parity_lock`, `serve_embed_assets_lock`, `asset_cache_lock`, `asset_compression_lock`, `bind_address_lock`, `site_metadata_lock`, `pages_in_test_lock`, `backend_relay_lock`, `largo_expand_lock`, `project_shape_lock`.
  - **Eighteen example projects and guides that cannot go stale.** `examples/` is new in this release: `counter`, `todo`, `tic_tac_toe`, `task_board`, `signup_form`, `fetch_dashboard`, `multi_page_site`, `modal_toast`, `stopwatch`, `shopping_cart`, `chat`, `data_table`, `file_drop_upload`, `infinite_scroll`, `marketing_site`, `login`, `guestbook` (server-rendered, no JavaScript) and `fullstack_todo`, each with a README stating its limits. `examples_lock` discovers every project and requires it to check, to test each action, handler, trigger and binding it declares, to pass on every tier, to serve its own markup and 404, to export its site and to build into a binary. `docs/lgx-reference.md` is generated from the compiler's catalogues and locked equal to them, and every browser snippet in the guides is mounted, rendered on both in-process tiers, compiled for the browser and has its tests run, so a guide cannot teach a second way of writing an app. `examples_lock`, `canonical_teaching_lock`, `lgx_reference_lock`, `lgx_stdlib_guide_lock`.
- **Tests are written in the language, and `largo test` runs them in a sealed world.** `## Test: <prose>` names a test by its sentence (a duplicate name is a parse error). Its body is ordinary LOGOS plus `Expect that <condition>.`, which records a failure and continues, rendering the condition back to English with the value of each named part; `Expect this to fail [with "…"]:`; `Expect output:`, byte-exact against an indented snapshot or `Expect output as in "<file>".`; `Advance time by <duration>.`; and `Given a file "<path>" containing "<text>".`. Leading sentences declare `Skip this test because "<reason>".` (the reason is mandatory), the real resources a test may use (`This test may use the real network.`, or the filesystem, environment or clock) and its step budget (`This test may run up to N steps.`; a runaway fails at the identical op every run). `For each of these cases:` followed by a pipe table runs one named sub-test per row, and `For all integers|naturals|booleans a and b [where …]:` samples 256 seeded cases and shrinks a counterexample. A test runs against a virtual clock, an in-memory filesystem, seeded randomness and no network unless it grants otherwise, and a test-only statement is refused outside a test. `largo test`, a reserved stub in 0.10.1, discovers every test in the entry's import closure and in `pages/`, reports in name order byte-identically across runs, and fails on a failure, on zero executed tests, on a filter that matches nothing, or on more authored skips than `[test] skip_budget` in `Largo.toml`. `--seed N` and `--format json` shape a run; `--json` prints a claim inventory classifying each test as an example or a property, with its vacuity flag, domain bound and counterexample; `--prove` exhausts finite domains (booleans up to 16 variables); `--live` admits capability-granted tests; and `--tiers all` re-runs each passing test's computation on the bytecode VM and compares byte for byte. A run ends in an earned rank: UNTRIED, FAILING, GREEN, HARDENED (a `For all` property held) or PROVEN (a property proved by exhaustion). `test_runner_lock`, `property_test_lock`, `inventory_lock`, `rank_lock`, `tiers_flag_lock`, `tiers_all_lock`, `closure_lock`, `hermeticity_redteam`, `failure_message_goldens`, `test_surface_catalog`.
- **Observation is part of the language.** `Note that …` (developer), `Warn that …` (operator) and `Record that …` (audit) emit structured events, with `with a and b` payloads and deterministic per-site sampling (`Note once in every N times that …`), on a channel separate from program output; `During <gerund>:` opens a span that closes on every exit path. The tree-walker, the VM (`Op::Obs`), compiled Rust (`logicaffeine_system::obs`) and WebAssembly (an `obs` import) emit the same stream, which serializes as `.logosobs`, one JSON object per line. A release build erases every observation statement before code generation, so the compiled program is byte-identical to the program without them. `obs_emission_lock`, `obs_span_balance_lock`, `obs_channel_separation_lock`, `obs_vm_emission_lock`, `obs_aot_emission_lock`, `obs_wasm_lock`, `obs_wire_lock`, `obs_zero_cost_codegen_lock`, `obs_grammar_catalog`, `obs_tier_differential`.
- **A failing run is replayable, and the debugger runs backward.** A failing `largo run --interpret` writes a self-contained `.replay` bundle under the project's target/replays directory (the source and its hash, the scheduler seed and decision trace, the arguments, the failure's kind, message and cursor) and prints `replay it: largo replay <path>`; `largo replay <bundle> [--verify]` needs no project and reports the failure reproduced byte-identically, no longer firing, or diverged. `largo dap` speaks the Debug Adapter Protocol over stdio (`launch` a `program` path or inline `source`, stack, scopes, variables, stepping, `evaluate`, `stepBack` and `reverseContinue`) by serializing the `debug_session::DebugSession` the Studio's debug drawer renders, and the VS Code extension adds a `logos` debug type that runs it. Reverse steps rest on the bytecode VM recording only what each op destroys onto a history tape and replaying the trace backward to the exact prior state (`Vm::run_recording`, `Vm::run_reverse`); spawned tasks, pipes, `Await` and `Select` reverse schedule-backward, and an op outside the reversible subset is declined before the run starts. `replay_lock`, `dap_lock`, `session_parity_lock`, `frame_eval_lock`, `reverse_op_lock`.
- **The standard library is written in LOGOS: 35 new modules, auto-imported when a program uses them.** Text: `strings` (95 functions over the UTF-8 byte view, among them split/join/trim/replace, a Two-Way `indexOf`, `wrapText`/`dedentText` and a rune codec), `chars`, `bytes`, `bits`, `regex` (literals, `.`, `*` and flat groups with Go-style submatch indices), `encoding` (hex, base64/32/85, ascii85, uuencode, quoted-printable, `hexDump`), `convert`, `json` and `unicode` (UCD category, mirrored, combining, digit and east-asian-width tables). Numbers: `math` (157 functions: trigonometric and hyperbolic, `erf`/`erfinv`, `gamma`, IEEE `ulp`/`nextafter`, `powMod`/`modInverse`/`modSqrt`, Miller–Rabin, arbitrary-precision `pi(digits)`, complex functions over `ComplexF`), `statistics`, `rng` (a seeded Lehmer generator, bit-identical on every tier, with ten distributions) and `operator`. Collections: `iter` (58 sequence operations, among them `mapSeq`/`filterSeq`/`reduceSeq`, bisection and set algebra), `collections` (Counter, `Deque`), `heap`, `sort` and `builder` (`StringBuilder`, `ByteReader`). Linear algebra: `blas` (levels 1–3), `lapack` (53 routines: Cholesky, LU, QR, SVD, symmetric and general eigenproblems, Schur, `expm`/`logm`/`sqrtm`, least squares, pseudo-inverse) and `clapack` over `ComplexF`. System and media: `calendar`, `compress` (CRC-32, Adler-32, zlib inflate of stored, fixed- and dynamic-Huffman blocks, a stored-block `zlibCompress`), `error`, `media` and `vision`; and the web modules above. `crypto.lg` adds SHA-256, HMAC and PBKDF2 (pinned to the FIPS 180-4, RFC 4231 and RFC 7677 vectors), ChaCha20 `encrypt`/`decrypt` (RFC 8439), a four-way Keccak that expands the ML-DSA matrix once per keygen, sign or verify, the Ding key exchange and Rainbow signatures; the tree-walker and the VM run `pbkdf2Sha256` natively, byte-identical to the LOGOS body the compiled tiers run. The library is held to a deduplicated superset of the C, Python, Rust, Go and Java standard libraries: `benchmarks/parity/PARITY_LEDGER.md` records 548 of 548 in-scope capabilities correct on every claimed tier and 38 of 520 performance-gated capabilities benchmarked faster, and `parity_catalog_lock` ratchets both counts upward. Every position a module takes or returns is 1-based, every `## To` name is unique across modules, and each compute module builds on AOT-Rust. `parity_catalog_lock`, `parity_ledger_lock`, `stdlib_position_base_lock`, `crypto_sha2_lock`, `mldsa_aot_lock`, `crypto_accel_lock`, `lapack_svd_lock`, `arbitrary_precision_math_lock`, `stdlib_no_name_collisions`, `parity_stdlib_aot_rust`.
- **A cancel never lands on the statement after it.** `pgCancelRequest` (and `pgCancelInFlight` with a query in flight) returned once the 16-byte CancelRequest was sent, so the statement a program sent next could reach the backend before the postmaster had signalled it and be the one cancelled: under load, `SELECT 41 + 1` after a cancel answered `ERROR 57014: canceling statement due to user request`. It now waits for the postmaster to close the cancel connection, as libpq's `PQcancel` does, so the signal reaches a backend waiting for its next command, which ignores it. `postgres_protocol`, `postgres_live`.
- **Programs span files and directories.** The `Import` sentence is new: `Import Geometry.`, `Import A and B.`, `Import Lexer, Parser, Ast.`, `Import G from "shapes/geometry.lg".`, `Import G as Geo.` and the selective `Import double as dbl from Util.` count anywhere in a module's prose before its first `##` header, an `Import` below that header is refused where it stands, and 0.10.1's Markdown link in the opening paragraph still counts. A bare name resolves beside the importer, then as a directory module (a `lexer` directory's `lexer.lg`, then its `mod.lg`), then under `src/`, then at the `Largo.toml` root, then in a declared `path` or `git` dependency, and a mistyped name gets a did-you-mean. The modules are assembled into one source every tier compiles, so imported functions reach the parser, the call graph and every backend (0.10.1 carried only imported types): functions are callable bare or as `Util::double(…)`, types as `Geometry::Point` (the bare and namespaced spellings are one type), identical definitions merge, a bare name two modules define differently is refused, each file sees only the modules it imports directly, and `## To private f` and `A private T` stay inside their module. Every pass reads a function header with one reader. A syntax error in an imported file names that file, a rustc error in generated code is reported at the LOGOS `file:line:col` with a caret, `largo check` warns on unused and duplicate imports and refuses a self-import, and `largo fmt` sorts and de-duplicates the import block. `project_module_lock`, `local_deps_lock`, `import_scan_lock`, `check_deep_multifile_lock`, `imported_record_fetch_lock`, `function_header_forms_lock`, `closure_lock`, `multimodule_encapsulation`, `multimodule_selective`, `import_hygiene`, `sourcemap_remap`.
- **The type checker runs on every program, and says where it fails.** `check_program` checks every statement kind with no wildcard arm: what `Show` prints and its recipient, every argument of a statement-form call, arity before argument types, a pushed element, a field or index assignment and both sides of a `Swap`; an `Int` argument widens into a `Real` parameter and never the reverse. A call to a name defined nowhere is refused at check time with the runtime's words (`Unknown function: <name>`), even inside a branch that never runs, and a type that neither the program, the language nor the declaration's own type parameters supply is refused at the statement that names it. Generic records and enums carry their arguments: `Box of Int` and `Box of Text` are different types, a program's own `Maybe` is not the built-in `Option`, constructions and variant payloads are checked against the substituted declaration, and `infer_instantiations` answers the type arguments at every generic site for the backends. A type error names the function its author wrote, or the construct a compiler-written function was lowered from (the `add` action of `## Component Counter`, a `## Route`), and a mismatch names both user types. 18 of the 32 programs in the ill-typed corpus are refused (the 14 still accepted are an exact-set ledger) while the gauntlet corpus stays accepted. `type_checker_rejects_lock`, `unknown_call_lock`, `undeclared_type_refusal_lock`, `type_error_location_lock`, `generics_all_tiers`.
- **New statements and forms.** `Fail with <reason>.` stops the program with its reason; `Assert that`, `Require that` and the new `Assume [that] …` take `, otherwise <reason>`, evaluated only on failure, and range contracts carry units (`Assume v is between 0.2 volts and 0.7 volts.`, folded to exact SI magnitudes at parse time); the reason reads identically on the tree-walker, the VM, AOT-Rust, C and WebAssembly. `Swap x and y.`; `Reversibly:` (alias `Uncompute:`), whose in-place additive and xor updates lower to reversible opcodes on the VM; `Close ch.` (buffered values stay receivable, then `nothing`); `Await h [into r].` or `Wait for h [into r].` joins a task; `Converge remote into local.` is the idempotent CRDT join, a bare `Int` field taking the maximum so a redelivered state never double-counts; `Send unreliably …`; and `On <event> from <source>:`. Tuple types `(Int, Text)` and named records `(quotient: Int, remainder: Int)` type parameters, returns and record fields, with `Let (q, r) be divide(a, b).` and `r's quotient`. `Let open be every t in todos where not t's done.` builds a new list and `Remove every t from todos where t's done.` removes in place; `x is none`, `is not equal to`, `is greater than or equal to`, `is approximately X within eps`, and a bare `x is "text"`, `x is total` or `c is Red` read as equality; `Increase n by 3.` works on a plain name; `a new Failure from Outcome` names a variant's enum; `Repeat for each t in xs`; `the count of` any collection; word-form `milliseconds`, `microseconds` and `nanoseconds`; possessives on calls and indexes (`handle(x)'s field`); and `Solve M times x equals v for x.` and `Minimize the quadratic Q with linear b over x.`. Every word-keyword is contextual: it names and reads back a variable, parameter, field, function or type (`Let at be 1. Show at.`). Every `##` header accepts an optional trailing `:`. `backend_failure_matrix_lock`, `analog_dimensioned_contract_lock`, `converge_crdt_lock`, `param_return_tier_coverage_lock`, `record_tuple_field_lock`, `list_where_lock`, `increase_decrease_plain_lock`, `variant_resolution_lock`, `count_of_collection_lock`, `header_trailing_colon_lock`, `op_backend_matrix_lock`, `error_fail_and_contracts`, `identifier_freedom`.
- **New values and builtins.** A floating complex value (`complex(1.5, 2)`, `conjugate`, `re`, `im`; an exact complex times a float promotes) and `Word8` (`word8`, `intOfWord8`). New builtins: `contains`, `makeDate`/`makeMoment`, `leading_zeros`/`trailing_zeros`, `readBytes`/`tryReadBytes`, `compressBytes`/`decompressBytes` (the smallest of deflate, lz4 and zstd, self-describing), `sealBytes`/`openBytes` and `sealFrame`/`openFrame` (compress, then ChaCha20-Poly1305, the tag verified before any byte is returned), `fromWireBytes`, and Reed–Solomon erasure coding (`reedSolomonEncode`/`reedSolomonRecover`, `fecProtect`/`fecRestore`) over the new `logicaffeine_base::gf256`. A program's own function of the same name still wins. `builtin_wire_id_lock`, `complex_seq_aot_lock`, `complexf_arith`, `fec_language`, `qa_secure_frames`.
- **The C tier is a general backend.** In 0.10.1 `largo emit c` was a benchmark subset with no concurrency, big integers, exact or dated values, bounds checks or `Show` for collections. `compile_to_c` now splices the auto-imported standard library and keeps only the functions a program reaches, and monomorphizes generics first: each call of a generic function becomes a call of one concrete copy per type-argument list, each applied generic type one concrete record or enum, and `some`/`none`/`Option of T` lower to core's own `Option`. C carries the exact numeric tower (`LInt` promotes to a big integer on provable overflow, in growing accumulators and in every `**`; `LDec` keeps `7 / 2` as `7/2`; complex and modular values), money, quantities, UUIDs, Words, every temporal value (literals, `Date ± Span`, `Moment ± Duration`, `Show`), tuples and named tuples (as values and as record fields), records of sequences and nested collections, insertion-ordered maps and sets that show as `{k: v}`, a map loop binding `(key, value)` in insertion order, closures as static C functions with exact higher-order calls, `ConvergentCount`/`Tally` fields, the offline network, policy checks, and `Fail with`/`, otherwise` reasons on stderr with exit status 1. A record, an interpolation hole, `format()` and a concatenation render through one exhaustive renderer (`Point { x: 10, y: 20 }`, a Real as its shortest round-trip decimal, an enum as `Just(42)`); indexing fails `index out of range` (a negative index counts from the end), an absent map key fails `missing map key`, an unmatched `Inspect` fails, and a zero divisor fails the task with the interpreter's words instead of SIGFPE. A binding that re-types or shadows a name is a C variable of its own, restored when its block closes; a function reads and writes the program's `## Main` bindings as file-scope statics; `Show x to f` hands `x` to `f`; and a LOGOS name that a C header also declares (`rename`, `send`, `time`) is mangled from the headers' own name list. A value C cannot type, and an expression kind it does not lower (a range as a value, a page query), refuses by name, and a temporal kind it cannot represent is an `#error` naming it. Where the interpreter promotes an `Int` past 64 bits on operands C cannot prove bounded, C's arithmetic still wraps; that divergence is ledgered in `crates/logicaffeine_tests/tests/gauntlet/c_known_divergences.txt`. `compile_to_c_via_bytecode` lowers the optimized bytecode the VM runs to C over one register file, including a real-socket TCP client whose errors read as the VM's. `c_numeric_tower_lock`, `c_bigint_power_lock`, `type_tier_coverage_lock`, `c_enum_option_lock`, `c_temporal_lock`, `c_binding_scope_lock`, `c_header_names_lock`, `c_crdt_lock`, `c_seq_of_tuple_lock`, `record_tuple_field_lock`, `higher_order_bignum_lock`, `exact_hof_call_site_lock`, `c_bytecode_lock`, `struct_show_cross_tier`, `c_collections_cross_tier`, `date_span_all_tiers`, `show_to_recipient_all_tiers`.
- **Host C, RISC-V, the GPU and the bytecode-to-C walker share one scheduler.** A concurrent program on any of them runs one C port of `logicaffeine_runtime`'s `process_step` (`crates/logicaffeine_compile/src/codegen_c/sched_core.c`): host C and RISC-V run tasks as coroutines, and the stepped tier (`compile_to_c_stepped`) runs them as step functions over heap frames, the lowering the CUDA tier and the walker take. Every concurrency operation is a scheduling point; a `Sleep` inside a task, a select on a closed pipe, a same-tick timer tie, a multi-ready select, a pipe's default capacity of 32, `Stop` and `Await` behave as on the interpreter; a failing task fails alone while the others run to quiescence and the first failure is reported; and each binary draws its choices from `LOGOS_SEED`. A receive from a closed, drained pipe, `Try to receive` and `Await … into` read `nothing`; task arguments cross whole, packed by the callee's parameter types; and a pipe carries its element type (`Pipe of Real`, `Pipe of Text`). `SchedulerKnobs` sets the policy, clock, default pipe capacity and seed, and `LOGOS_SCHED_WORKERS` above 1 runs the stepped host on a thread pool whose output does not depend on the worker count. A GPU thread cannot unwind, so a failure inside a runtime helper (a zero divisor, an index out of range) stops the whole GPU run, while a failing `Require` still fails only its task. `c_family_scheduler_conformance_lock` holds 36 programs at six seeds on host C, RISC-V under qemu, the stepped tier, the walker and the GPU against the tree-walker and the VM; `c_scheduler_port_lock`, `stepped_tier_lock`, `c_concurrency_knob_lock`, `c_bytecode_concurrency_lock`, `c_task_abort_lock`.
- **A RISC-V ahead-of-time tier.** `largo emit riscv` and `largo build --emit riscv` lower the register bytecode to RV64GC assembly for the SysV lp64d ABI with its C runtime (`logos_rt.c`), and print the gcc and qemu command. 115 of the 117 ISA ops lower (`Tcp` and `Serve` need host sockets and are refused by name). A collection changes in place while its register is its only holder and is copied first once it may be shared, the VM's value semantics, so a loop of pushes or indexed writes runs in linear memory. `riscv_aot_lock` classifies every op with an empty gap ledger and runs the corpus, the error corpus, the concurrent and network oracles and all 70 benchmark programs under `qemu-riscv64` against the VM, each at up to four sizes no larger than its smallest declared workload, every run bounded to 512 MB and 60 s (the benchmark half runs from `scripts/run-qemu-corpus-tests.sh`). `riscv_aot_lock::riscv_equals_vm_and_treewalker_over_the_corpus`, `riscv_aot_lock::a_change_reaches_only_the_holder_it_was_made_through`, `riscv_aot_lock::every_owned_mutation_runs_in_linear_memory`. The tier runs what the VM runs, as the VM runs it: only the functions a program can enter are lowered; `Option`, `Result` and declared enums match by constructor name; closures, tuples, sets and maps show, compare, iterate and copy as on the VM; `Merge`/`Converge` change the target's own record field by field; an index counts from the back when negative; every failure stops the program with the VM's own sentence, taken at compile time from the function the VM fails with; a function that finishes without a value answers `nothing`; and a list or counter in a record's field belongs to that record, so a change through one holder reaches no other (`riscv_aot_lock::riscv_runs_the_parity_corpus_as_the_vm_does`). None of it costs speed: `riscv_instruction_budget_lock` holds the instructions the emitted code of 64 programs executes to shrink-only budgets, every added check is proven away or taken on a cold branch, and the runtime keeps its frame information out of `.eh_frame`, so a binary's startup registers none.
- **A CUDA ahead-of-time tier.** `largo emit cuda` writes one `.cu` that runs a whole program on one GPU thread, with a host launcher, a `Makefile` and the C tier's oracle (`logos_oracle.c`). The runtime slice the program reaches is tree-shaken out of the C tier and lifted to the device, with a Dragon4 shortest-round-trip float formatter byte-identical to the host's, a device `vsnprintf` family, a host-injected fixed clock for `today`/`now`, the offline network, and a failure path that flushes device `printf` and traps. 112 of the 117 ops lift; concurrent programs schedule through the stepped tier; and a program whose C carries the C tier's own refusal is a typed refusal rather than an artifact `nvcc` cannot build. `cuda_aot_lock`, `cuda_float_lock`, `cuda_failwith_trap_lock`, `cuda_temporal_lock`, `cuda_net_lock` (the GPU halves skip without `nvcc` and a device).
- **The WebAssembly backend runs generics, records and collections.** `compile_to_wasm` (`largo emit wasm` and `wasm-linked`, and every page module) monomorphizes generics as the C tier does, so the generics corpus runs apart from one program (a generic function returning its `Option`'s payload). Every value shape displays as the VM displays it, kinds come from the program's declared types, and records and maps held in module bindings, records read from lists, `Result` payload records, nested collections and shared counters lower; record slots follow the declaration, sets and maps keep insertion order through removals, `For each c in <Text>` walks UTF-8, Text appends in amortized time, a `Return` inside a loop unwinds only its own function's loops, and a failing program stops with the interpreter's reason. A function nothing reaches is emitted as a trap stub instead of blocking the module, a refusal names its function, and single-read values stay on the operand stack. The host imports (`random_int`, `text_contains`, `logos_rt_fail_at`, `obs`, the temporal constructors) are an append-only catalog. The `$` currency format spec is still refused on this backend. `wasm_aot_lock::the_generics_corpus_runs_on_wasm`, `wasm_display_lock`, `wasm_declared_kind_lock`, `wasm_record_list_lock`, `wasm_result_payload_lock`, `wasm_collection_order_lock`, `wasm_text_append_lock`, `wasm_host_entries_lock`, `wasm_host_abi_lock`, `wasm_place_mutation_lock`.
- **AOT-Rust builds what the interpreter runs.** Generics stay Rust generics, each type parameter carrying the bounds every LOGOS value has, and a phantom type argument gets a turbofish from the checker's per-site solution. First-class tuples lower to `LogosTuple`, and a tuple may hold records (`Value::Boxed`). A record field may be named with any Rust keyword (escaped as `r#type`), and the compiler's own `File` and `HttpError` records are emitted only when a program names them. A binding keeps the exact value type its right-hand side produces, and `{x:$}` and `{x:.N}` print a decimal exactly; format specs give every kind the interpreter's text on the tree-walker, the VM, AOT-Rust and C. A `Tally` or `ConvergentCount` compares by its total. `length(x)`, `copy(x)`, `contains(h, n)`, `format(x)`, `readBytes`, `For each c in <Text>`, `Send m to peer` (through the mesh), a shift on a promoted `Int`, `sin(x) * 100`, an `Int` passed to a `Real` parameter and floating complex values build; a named element of a sequence literal is copied rather than moved; `Show x to f` lends `x`; a call statement lowers its arguments as a call expression does; two launches passing one pipe evaluate their arguments where they stand; dates add and subtract calendar spans; a value past 64 bits reaches every statement kind; an overflowing closure passed to a higher-order function promotes as on the VM; a `mutable` parameter builds for every type; and a function nothing calls still compiles. A compiled runtime error prints its LOGOS message alone and exits 1, and `Require` and `Assert` report their own words. `aot_specialized_recursion_lock`, `aot_list_literal_copies_lock`, `aot_exact_value_format_lock`, `aot_show_lends_lock`, `builtin_struct_emission_lock`, `aot_text_walk_lock`, `aot_record_field_move_lock`, `bignum_reaches_every_statement_lock`, `int_representation_lock`, `higher_order_bignum_lock`, `mvs_record_param_lock`, `semantic_probes_lock`, `tuple_of_records_all_tiers`, `keyword_field_names_all_tiers`, `counter_comparison_all_tiers`, `format_spec_all_tiers`, `format_builtin_all_tiers`.
- **The native tier runs functions over the program's own enums.** A node rides a frame slot as the VM's own reference-counted value: building one, an `Inspect` arm's test and bindings, and an enum parameter or return all run natively. Parameters and the fields read from them are borrowed and cost no refcount traffic; a node the function owns hands its count into a new node or the return value at its last use, or releases it on the edge where it dies; a side exit hands every node back to the VM. binary-trees at depth 16 takes 2.16 s on the JIT tier, against 5.04 s with the lane off (`LOGOS_JIT_NODES=0`). `jit_enum_lock`.
- **The native tier compiles the scalar builtins and the Word lanes.** 27 builtins tier to native code where 0.10.1 lowered `sqrt` alone: `abs`, `min` and `max` on `Int` (branchless), `count_ones`, `leading_zeros`, `trailing_zeros`, `floor`, `ceil` and `round` of a `Float` to an `Int`, `pow` with a constant exponent up to 32 (unrolled checked multiplies that deopt to the big-integer path), the `word32`/`word64`/`word16` family with its logical operations, shifts and rotations, and `chr` (a guard that deopts on an invalid scalar). Words ride unboxed in native registers and re-box at an exit or deopt, so same-width modular `+`, `-`, `*`, `xor`, `&` and `|` and indexed reads of a `Seq of Word32` tier, and a SHA-256 round and FNV-1a run natively, byte-identical to the tree-walker. An op whose instruction the CPU lacks (`popcnt`, `lzcnt`, `bmi1`, `sse4.1`) declines to the VM. `forge_builtin_compliance_ratchet` classifies all 234 builtins and holds the tiered floor at 27, and `forge_numeric_perf_lock` and `forge_fnv_hash_lock` require a hot native region to beat the VM by at least 3× on x86_64. `forge_sha256_correctness_lock`, `forge_sha256_compression_shape_lock`, `forge_chr_lane`, `forge_word_lane`, `forge_word_arith_lane`.
- Scalar swaps execute natively with their value kinds intact, including word widths and recovery after integer overflow. Collection swaps preserve copy-on-write values because register promotion requires a stable collection binding. `jit_scalar_swap_lock` and `jit_scalar_swap_edges_lock` pin the results.
- **A program can demand an optimization, and 36 new optimizations can each be switched off.** `## Require <opt>` fails compilation, naming the optimization, when it did not fire; `largo build` and `largo emit` (Rust, C, ISA, WebAssembly) and the verified path honor it through one decorator seam, and `largo run --interpret` refuses it up front. The registry grows from 40 to 76 optimizations, each with a keyword for `## No`, `## Require`, `## Tier` and `LOGOS_OPT_OFF`. Both pipelines gain sparse conditional constant propagation, integer induction strength reduction, register promotion of a loop's read-modify-write element, loop interchange, run-path memoization and comptime table-ization; AOT-Rust gains bounded-accumulator overflow-guard elision, range-aware helper specialization and div-mod fusion, an AVX2 reduction of transcendental accumulator loops, zero-copy text returns and builders, string-buffer reuse, big-integer clone elision and native bit intrinsics; closed forms extend to `Σ i²`, `Σ i³`, affine and `trunc(toReal(i))` sums, and shift-and-mask bit loops become `POPCNT`, `leading_zeros` and `trailing_zeros`. Against compiled C, Rust and Go in the parity catalog (`benchmarks/parity/catalog/superset.json`), `math.abs` moves from 0.41× to 1.05×, `math.divmod` from 0.73× to 1.05×, and `math.quorem` sits at 1.02×; `operator.sub`, `operator.mul` and `convert.to_real` fold to O(1). Every benchmark runs identically with and without optimization, and `phase_opt_trace` fires each optimization on a witness. `parity_opt_lattice_lock`, `sccp_lock`, `int_strength_lock`, `reg_promote_lock`, `loop_interchange_lock`, `memo_run_lock`, `comptime_tableize_lock`, `closed_form_poly_lock`, `closed_form_affine_lock`, `popcount_intrinsic_lock`, `simd_reduce_lock`, `perf_pass_guard_elision_lock`, `optimizer_soundness_lock`, `e2e_require_optimization`, `decorator_through_largo`.
- **`## Require` on a device tier.** `largo emit isa` refused a program that required `fastdiv` when fastdiv served it: the check every device tier makes for the exact extension compiled the program a second time through the `## Require` gate, without the range facts fastdiv needs. The check no longer enforces the program's requirements; the tier's own compile does.
- **An IR peephole for the backends that lower bytecode directly.** RISC-V, WebAssembly and the fabric lowering run `vm::peephole` first: block-local forward copy propagation, then dead-move elimination over an iterative liveness fixpoint, collapsing the five-move cascade of `Set a to b. Set b to t.` to two moves; 25 of the 35 benchmarks shrink. It is sound by construction: reads over-approximate and kills under-approximate each op's register effects, a copy dies when either side is mutated or reassigned, only `Move` is ever deleted, every register classifier is exhaustive over the ISA, and an enum `Inspect`'s arm targets are left in place. `peephole_soundness_lock` holds `program_outcome(peephole(P)) == program_outcome(P)` over the compute corpus and every benchmark, idempotence, and a 400-program copy-heavy fuzzer; `riscv_aot_lock::riscv_equals_vm_and_treewalker_over_the_corpus`, `wasm_aot_lock::wasm_equals_vm_and_treewalker_over_the_corpus`, `fabric_coverage_lock`.
- **The kernel's integers and classical logic are Lean's, defined rather than assumed.** `Int` is Lean's two-constructor inductive (`Int_ofNat | Int_negSucc`), and `add`, `sub`, `mul`, `div`, `mod` and `le` are checked definitions over it that bottom out in checked `Nat` arithmetic. The sixteen public integer laws (`add_comm`, `mul_assoc`, `mul_distrib_add`, `le_trans`, `le_total`, …; fifteen were axioms in 0.10.1, and `sub_elim` is new) are proof terms both kernels check with empty axiom closures, and an integer literal stays definitionally equal to its constructor term, so literals can be matched on while arithmetic on them stays compact. `propext` and `Classical_choice` are the only new axioms, the ones Lean assumes; `funext`, excluded middle, the classical choice layer and `dne` (an axiom in 0.10.1) are checked definitions. The prelude gains `HEq` and `cast`, `Setoid` and `Quotient`, Lean's four universe-polymorphic quotient primitives, and defined `Type 0` families `Option`, `Prod`, `Sum`, `Sigma`, `Subtype`, `Fin`, `List`, `Array`, fixed-width unsigned integers, `Char`, `String`, `Iff`, `Inhabited`, `Nonempty`, `OfNat` numerals and `Decidable` instances, and `Text_append`, `Text_beq`, `Text_isPrefixOf` and `Text_length` compute. The trusted base is inventoried, assumption by assumption, in `crates/logicaffeine_kernel/TCB.md`, held to the live prelude by `tcb_ledger_lock`, which also requires every constant Lean proves to stay a definition. `#print axioms` is kernel API (`axiom_dependencies`, `AxiomUsage`; `Print Axioms name.` in the vernacular), computed through statement and body as Lean computes it. `TheoremCertificate` and `EnvironmentCertificate` carry a proof with every declaration it needs, rebuilt through the kernel's checked gateways; `read_certificate` dispatches any certificate by schema, and the `recheck` example re-checks one. `tcb_ledger_lock`, `int_prelude_bridge`, `derived_integer_ring_laws`, `derived_integer_order`, `derived_integer_division`, `prelude_types_extra`, `axiom_footprint_complete`, `certified_quotient_audit`, `certificate_environment`.
- **Lean 4 proofs import into LOGOS, re-checked by two kernels.** `largo lean import --project DIR --target Name --out DIR` compiles a Lean project with the pinned Lean 4.14.0 and lean4export (or reads a `lean4export` NDJSON stream), checks every declaration of the target's closure with the LOGOS kernel under Lean's exact, non-cumulative conversion and again with the independent re-checker, binds the statement to the Lean source through a reference translator that shares no code with the importer, holds the assumptions to Lean's `propext`, `Quot.sound` and `Classical.choice`, and writes the closure as a literate `.lg` document that re-parses to the identical declarations. The `logos-lean-package/1` it publishes (a manifest with SHA-256 digests and a trusted-base disclosure, the document, an environment certificate, the Lean-to-LOGOS name correspondence and the source export) is replayed by a fresh process before it is renamed into `--out`, so a failed run leaves nothing there, and `largo lean replay DIR` re-verifies a package from its bytes alone. Every refusal carries a typed diagnostic code and exit status (0 verified, 1 invalid, 2 unsupported, 3 disallowed assumptions, 4 resource, 5 infrastructure, 64 usage); `unsafe` and `partial` declarations are quarantined, a safe declaration that references one is rejected, and `--arena-protocol` speaks the Lean Kernel Arena protocol. The library half is the kernel's new `lean` feature (`interface::lean`). `largo check --kernel` and `largo prove --kernel` check a kernel-vernacular document block by block, naming the first block that does not check. `lean_diagnostic_code_lock`, `lean_package_replay`, `lean_intake_contract`, `lean_source_emission`, `lean_import_audit`, `lean_cli_e2e`.
- **A Lean Kernel Arena checker.** `logicaffeine_kernel::dag` checks Lean 4 export streams directly over a hash-consed de Bruijn DAG with two independently written engines (substitution, and evaluation into closures with types as values), a complete decision procedure for universe-level `≤` and `≡`, inductive families elaborated by the kernel (an export's recursors are admitted only when identical to the derived ones), and `Nat` and `String` literal extensions enabled only after the environment's definitions are validated against Lean-elaborated reference terms. `arena_corpus` holds it to the expected verdict on each of the 221 cases of the Arena corpus at lean-kernel-arena `66c354d6`, a decline never counting as a pass (`arena_corpus_lock`); the heaviest inputs run with the ignored set from local exports. `crates/logicaffeine_kernel/arena/` holds the `logos-kernel` and `logos-dual` checker entries, prepared and not yet submitted, built under the root `[profile.arena]`. `arena_corpus_lock`, `arena_entry_lock`.
- **The kernel vernacular holds a Lean development.** New commands declare everything the kernel checks: `Theorem`, `Opaque`, `Axiom`, mutual and universe-polymorphic inductive blocks (`Inductive N.{u} (A : Sort u) : … := … with …`), `Quotient`, `Structure`, universe telescopes on any declaration, `Print Axioms`, `Protected`, namespaces with `Open … in` and `Export`, sections, `Notation` and the fixity commands, hygienic `Macro`/`MacroRules`, and doc comments. The term grammar spells every kernel term: binder groups with Lean's binder kinds (`{…}`, `⦃…⦄`, `[…]`), `Sort ℓ`, `max`/`imax`, `name.{ℓ}`, positional and pattern `match`, `fix … with … return`, `let`, and one spelling per literal kind (`5n`, `1.5f64`, `5s`, `2026-05-20`). The elaborator adds strict implicits, named arguments, `@f`, the `..` ellipsis, declared parameter defaults, auto-params discharged by `decide` and kernel-checked per call site, `OfNat` numerals and coercions; the kernel derives Lean's auxiliaries (`casesOn`, `recOn`, `noConfusion`, `brecOn`, `decEq`, constructor injectivity) as checked definitions, gives every checked definition its equation lemmas (`f_eq_def`, `f_eq_1 …`), compiles nested families and restores their recursors, and keeps Lean's attribute databases (reducibility, a simp-lemma registry that refuses rules that cannot rewrite, instance priorities, fallback and local instances, scopes that revert attributes and never declarations). The printer is the parser's exact inverse up to α, overlapping equations compile first-match-wins with any missing case named, and `Repl::execute_document` runs a whole document and stops at the first block that does not check. `term_parser_total_lock`, `vernacular_roundtrip`, `vernacular_theorem_axiom`, `derived_auxiliaries_extra`, `equation_registration`, `elaborator_extra`, `raw_match_patterns`.
- **A kernel term can carry how it was written without changing what it means.** `Term::Annotated` wraps a subterm with a binder kind (Lean's `BinderInfo`, for a `fun` or `Π` no declaration name records) or its provenance (a `file:line:column` position, or a Lean `mdata` entry). Every kernel checking door erases annotations once before it measures or checks anything, the context stores terms erased, `==` compares kernel meaning (`Term::identical` sees annotations), the term store interns an annotated term to its erased term's id, and certificates carry none. The elaborator inserts implicit arguments from a binder kind on an anonymous `fun`, and the interface prints `@[binder implicit] t`, `@[source "f.lean:1:0"] t` and `@[mdata "k" "v"] t` and reads each back. T00-005 and T00-009 of the conformance corpus are Native. `term_annotation_transparency_lock`, `lean_conformance_t00_terms`.
- **Lean-style proof steps, English arithmetic and proof footprints on the theorem surface.** A `Proof:` line takes `exact`, `exists`, `have` and `refine` with a real term argument (`use (n + 1)`, `exact (hpq hp)`, `?_` holes), `apply h`, `rfl`, `intro a b c`, `cases h with a b`, `rewrite h at 2` and `rewrite h at hp`, `simp only [h₁, h₂]`, `calc h₁ h₂`, `by_rule <Rule>`, `by_contra h`, `ring`, `linarith`, `induction n`, and `all_goals`/`any_goals`/`focus`; every step still ends in one kernel-certified derivation, `by_rule` certifies its node before it reports success, and `focus` is an error when its block leaves the goal open. `Prove: For all n, n plus 0 equals n.` states arithmetic in English and certifies through the same kernel path as the formal surface, `Proof: By <Rule>.` applies a kernel inference rule and certifies it, and `Proof: By induction on n.` inducts. `VerifiedProof::axiom_footprint()` is `#print axioms` for a certified proof, splitting what it rests on into the standard library's trusted declarations and what the proof introduced, and `largo prove --certificate` prints an independently re-checkable kernel certificate (proof term, goal, disclosed premises) for every kernel-proved `## Theorem`. The proof crate also gains exact linear algebra over ℚ with kernel-checked solves (`lapack_provable_lock` proves the standard library's `gesv` solve), dependency-free replay checkers for analog range, settled-solve and thermal-map certificates, XL linearization with a Rainbow reference, and reversible-gate certification. `proof_certificate_lock`, `lapack_provable_lock`, `T_tactics_extra`, `tactic_surface_extra`, `tactic_script`, `L_language_extra`, `axiom_footprint_complete`, `rainbow_logos`.
- **A toolchain built for agents.** `largo mcp` serves the toolchain over the Model Context Protocol (JSON-RPC 2.0 on stdio): six tools (`logos_check`, `logos_run`, `logos_logic`, `logos_prove`, `logos_emit`, `logos_opts`), a `write-logos` prompt and sixteen resources embedded in the binary, the error catalog among them rendered live. `logos_run` never executes inside the server: it re-spawns a worker under a wall clock and an optional step budget, and a program the requested engine cannot run is a typed refusal, never a silent run on another engine. Every project `largo new` writes carries an `AGENTS.md` (`largo init` adds it when absent), every `largo …` line of which re-parses against the real CLI. `largo check --json` and `largo build --json` print a `logos-check/1` document whose diagnostics are `logos-diagnostic/1` objects (a stable code, severity, file, span, message, explanation, suggestion, quickfix, docs link); every parse error has a stable kebab-case code catalogued in `docs/ERROR_CATALOG.md`; `largo fmt --check --json` prints `logos-fmt/1`; and prose stays on stderr with exit codes unchanged. `largo run` gains `--max-steps N` (the interpreter stops at the identical op every run and exits 122 with the partial output), `--timeout` (exit 124), `--seed N` and `--json` (`logos-run/1`). `largo explain <topic>` answers a diagnostic code, a construct or a primitive type offline, suggesting the nearest topic on a miss. `diagnostic_schema_lock`, `error_catalog_lock`, `mcp_e2e`, `agents_md`, `check_json`, `run_limits`, `explain`.
- **Every `largo` command reads one project model.** `project::shape` classifies a project as a site, a full-stack site, a backend or a program from two facts, whether it has `pages/` or `static/` and whether its entry's closure `Serve`s, and `serve`, `build`, `check`, `test`, `fmt`, `doctor` and `publish` read the same answer. Every command compiles the same import closure, imported modules and linked dependencies included (`check --deep`, `prove`, `doc`, `verify`, `run --interpret`, `emit`, `build --native-functions`). `path` dependencies and git dependencies pinned to a 40-hex `rev` (resolved at `.logos-deps/<name>-<rev>/` or an explicit `path`) link into every command, and `largo build --manifest` prints the linked closure as JSON without running cargo. A build's binary is its own: the generated crate builds under a per-project cargo identity and the artifact is copied to target/<mode>/<name>, so two projects sharing a `CARGO_TARGET_DIR` cannot overwrite each other. `largo check` checks every page on the tier it ships on and names each refusal `largo build` would stop on (`deploy-refusal` in `--json`); `largo fmt` formats `pages/`. `largo emit` reaches eleven targets (`rust`, `c`, `riscv`, `wasm`, `wasm-linked`, `wasm-dom`, `tenstorrent`, `cuda`, `analog`, `plasticine`, `rdu`), every target but `rust` compiling the full closure and reporting a refusal at `file:line:col`, and `largo build --emit isa` writes the Silicon ISA listing. `project_shape_lock`, `shape_agreement_lock`, `closure_lock`, `local_deps_lock`, `build_identity_lock`, `check_deploy_refusal_lock`, `pages_in_tooling_lock`, `diagnostic_surfaces_lock::emit_places_a_refusal_at_its_line`.
- **The editor speaks LGX, across modules.** The language server indexes components, pages, state fields, props and actions: go-to-definition, hover, rename and find-references work inside `View:` through one cursor rule; the outline names a route by method and path (`GET /api/todos`), a page by its path and an error page by its status; completion inside `View:` and `Head:` offers the page's own state, props, actions, components and template tags, and inside `Style:` the classes, ids and elements the view renders. Templates paint as HTML with LOGOS in the braces (the semantic-token legend gains `tag` and `attribute`, append-only), template lints and browser-build refusals are published at the construct with stable codes (`lgx-<kind>`, `browser-tier-refusal`, `copy-orphaned-copy` for the value-semantics copy lint), and findings on names the lowering mints are dropped. Across files, an unresolved name a project module exports gets an `Import <Module>.` quickfix, an unused import gets a removal, completion offers exactly the names a file's imports bring into scope, hover names the defining module, `private` definitions stay out of all four, and a standard-library hover carries the whole `## Note`. The server's codes, severities and quickfix notes are locked equal to the table `largo check --json` and `largo mcp` report from. The VS Code extension highlights `View:` and `Head:` as HTML and `Style:` as CSS, recognizes the LGX headers, ships `page`, `component`, `store`, `route`, `session` and `errorpage` snippets, and adds the `logos` debug type. `lgx_editor_lock`, `lgx_lint_span_lock`, `compiler_written_names_lock`, `diagnostic_location_editor_lock`, `stdlib_teach_lock`, `vscode_grammar_lock`, `locks`.
- **The Studio runs pages, and the Syntax Guide teaches by exercise.** A Studio file that declares a `## Page` opens in a fifth mode, Web, that compiles it to WebAssembly in the browser and renders it live in a sandboxed frame, rebuilt as typing pauses, with the build error in place of the page. The Studio ships 160 examples (89 in 0.10.1), the Code examples sorted into topic folders in learning order; its New file button creates an `untitled` file with a starter for the current mode; and Math mode runs a `## Theory` block theorem by theorem, each kernel-certified and reported on its own. The Syntax Guide grows from 25 to 43 sections, each with a problem-first hook, objectives, misconception callouts with a runnable counter-example, graded exercises with progressive hints and recall cards; its examples run in Logic, Code, Prove and Web modes, hide a locked answer until the reader predicts it, and the sidebar searches every section. `/review` interleaves the Guide's recall cards with Learn's on one spaced-repetition schedule, the benchmarks page adds a Tenstorrent cost-per-throughput tab (LOGOS-workload figures marked projected), and the site publishes `errors.json`, `examples.json` and `llms-full.txt` beside `llms.txt`. `studio_preview_debounce`, `e2e_studio_examples`, `code_examples_all_tiers`, `web_canonical_teaching_lock`, `guide_pedagogy`, `theory_development`, `tenstorrent_pareto_lock`.
- **`logicaffeine-system` serves HTTP and opens verified TLS natively.** The new `http` feature carries an HTTP/1.1 core (keep-alive and pipelining, `Content-Length` framing only with chunked bodies refused, an 8 MiB body cap and a 64 KiB header cap, no `Date` header so a response depends only on its handler, hop-by-hop headers stripped and CR, LF and NUL refused) whose refusals (`400`, `413`, `500`) answer byte-identically on the tree-walker, the VM and both native engines; a handler panic answers `500` and the server keeps serving. The production lifecycle reaches every tier through `LifecycleConfig::from_env`: `LOGOS_MAX_INFLIGHT`, `LOGOS_REQUEST_TIMEOUT_MS`, `LOGOS_READ_DEADLINE_MS`, `LOGOS_DRAIN_MS`, `LOGOS_MAX_BODY`, `LOGOS_CORRELATION_ID`, `LOGOS_READY_PATH` and `LOGOS_HEALTH_PATH` apply to the compiled `Serve` statement with no change to code generation, and `SIGTERM` or `SIGINT` stops advertising readiness, stops accepting, finishes in-flight work within the drain deadline and releases the port. Readiness and liveness are opt-in probe paths answered before admission (readiness `503` while draining, liveness `200`); the interpreted and bytecode `Serve` loops add bounded admission with a prompt `503`, the body cap and a read deadline, and enforce the per-request budget between statements. On Linux a thread-per-core io_uring engine (feature `io-uring`) is the default wherever a probe ring initializes (`LOGOS_HTTP_ENGINE=uring|tokio`, `LOGOS_HTTP_SHARDS`); it names itself on stderr, falls back to tokio on a failed bind, and falls back whenever admission, a request timeout, a probe path or a correlation id is configured, naming the variable. The `tcp` module implements `tcp.lg` for compiled programs over opaque handles with one error wording on every tier and a lock per connection, and TLS verifies against the bundled Mozilla roots plus a PEM named by `LOGOS_TLS_EXTRA_ROOT`. `testworld` is the sealed world a `## Test` runs in (a clock frozen at 2020-01-01, an in-memory filesystem, an empty environment, a seeded random stream, one capability granted at a time); `obs` is the observation channel of compiled programs; `time` and `random` compile for `wasm32`; and `LOGOS_HTTP_ACCESS_LOG` writes one line per request. `http_lifecycle_lock`, `http_lifecycle_bounds_lock`, `http_shared_handler_lock`, `http_serve_bench_lock`, `http_headers_lock`, `tcp_tls_lock`, `tcp_registry_concurrency_lock`, `random_int_lock`, `system_full_feature_lock`, `http_lifecycle_parity`, `http_serve_cross_tier`, `http_engine_fallback`.
- **Three more benchmark programs, and a CPU profile of the suite.** `analytics`, `vm` and `sha256` join the cross-language suite in all eleven languages (32 programs to 35), each with a title, description and declared complexity. `benchmarks/PROFILE.md` profiles every benchmark for LOGOS and C three ways (`scripts/bench/profile-benchmarks.sh`: perf, an instruction census, and strace with an n-versus-2n check). `benchmark_metadata_lock`.
- **Tenstorrent, and one spatial IR for every dataflow chip.** `largo emit tenstorrent` writes a TT-Metalium bundle that runs a program's compute body on one Tensix core (compute kernel with `Show` routed to DPRINT, host, CMake that requires `TT_METAL_HOME`, and the C oracle); channels, tasks, select, sleep and sockets are refused by name against a shrink-only ledger, and a recognized GEMM, elementwise, pipe, fan-out or select program lowers to multi-core Tensix kernels, every select arm resolved at compile time from the source's virtual times. Tensor programs lower once into `spatial::ir::SpatialProgram`: stages over bounded buffers joined by typed handshakes, placed on a described mesh (Wormhole, Blackhole, SN40L, SN50, RTX 3060) and scheduled as per-engine programs by target-parametric passes in a pinned order. `spatial::eval::eval_choreography` is the IR's executable meaning; every verified program carries a `ChoreographyCertificate` (a marked-graph liveness witness, buffer balance and high-water bounds, a SAT placement certificate per integer datapath, the numerics claim and a sharded program's all-reduce obligation) whose `recheck` trusts only the PR/RUP checker and integer arithmetic; the clock is the certified graph's maximum cycle mean; and work (matrix-unit calls, NOC transactions, bytes) is counted from the schedule. From the IR come Tensix reader, compute and writer kernels, a persistent CUDA kernel, synthesizable Verilog for the compiler's fabric with a matching cycle model, a SambaNova placement and Spatial source, every emitted construct the image of an IR object. Compilation is linear in the program: every per-section question reads an index built once, and a program is certified once, where it ships. A grid whose extents do not divide the tile count is admitted only where no evenly divided plan beats it on every predicted total, and a boundary site multicasts the rows it holds: at 640³ on Tenstorrent's Wormhole simulator the compiler's own plan moves 4,096,440 bytes against `ttnn.matmul`'s 4,097,000 and spends no more NOC transactions or Tensix instructions (`benchmarks/silicon/tenstorrent_gemm.json`; simulator counts, not silicon). The persistent CUDA GEMM fills shared rings with 16-byte `cp.async` copies, uses commit groups as FIFO pages, and proves before printing that no ring is filled past its depth (a schedule that would is a typed refusal): 17,994 GOPS at 2048³ bf16 on an RTX 3060, 1.20× the knob lattice's best rung in the same run (`benchmarks/results/cuda_spatial_persistent.json`), and its generated benchmark exits non-zero on any CUDA error. `tenstorrent_aot_lock`, `tenstorrent_op_coverage_lock`, `tt_matmul_lowering_lock`, `tt_concurrency_mesh_lock`, `tt_select_mesh_lock`, `spatial_ir_lock`, `spatial_one_ir_lock`, `spatial_backend_totality_lock`, `spatial_certificate_lock`, `spatial_clock_lock`, `spatial_work_model_lock`, `spatial_compose_lock`, `spatial_compile_index_lock`, `spatial_compile_scaling_timing_lock`, `spatial_ragged_admission_lock`, `tensix_boundary_page_lock`, `spatial_p6_ttsim_lock`, `cuda_spatial_persistent_lock`, `cuda_spatial_ring_lock`.
- **Every spatial target states its roofs.** `spatial::roofs` gives each chip preset the bandwidths and engine rates its programs are held against — Wormhole's 288 GB/s and 32-byte NOC links, Blackhole's 512 GB/s and 64-byte links, one Tensix matrix pass per cycle, the RTX 3060's tensor and shared-memory rates, the RDUs' published peaks, the Plasticine meshes' bandwidth and fp32 peak, our fabric's one element per cycle — each cited to the vendor document it is read from, derived with the arithmetic shown, or labeled modeled. `spatial_roofs_lock` cross-checks them against a second source wherever one exists, and `roofline_invariant_lock` holds every metered and every planned GEMM, and every Plasticine report row, to them: nothing moves fewer bytes than a GEMM's operands and result, and nothing outruns a roof. `roofs::roofline` names, for any planned program, the resource that bounds it. `Objective::Roofline` ranks plans by it.
- **The spatial planner prices a plan at what the compiler emits, on every target.** The closed form the plan search ranks by (`spatial::work::work_of_plan`) reads each tile's width, operand, buffer and result apart, and each site's movement engines off the program and its target, so on the RTX 3060, the RDUs, the sky130 fabric and every exact-numerics Tensix plan it counts the bytes the built choreography moves. `spatial_work_model_lock` holds it to the built choreography count for count on all six presets.
- **Each matrix engine states its sub-block API, and the planner counts its calls and operand loads by it** (`BlockApi`). The sky130 fabric's systolic array pays its fill and operand feed once per sub-block, so the planner gives it the sub-blocks that amortize them: under the fabric's cycle-exact twin a 32 × 32 × 16 GEMM takes 14,125 cycles where a per-tile count chose a plan taking 74,830, and `spatial_fabric_plan_lock` holds the planner's choice to the fastest candidate at every shape it times. On Tensix, where the unpacker re-loads one operand for every held tile, four plans take a sub-block column that ttsim measures at 14–32% fewer instructions.
- **Exact GEMMs on Tenstorrent read their operands once.** An exact plan could not split its reduction on Wormhole or Blackhole, which cannot reload an fp32 partial, so large GEMMs held K in one block and read their operands up to 22 times over. Where the partial sums are provably exact integers the reduction now accumulates in place and finishes in the output's own page; under ttsim a 2048 × 2048 × 1024 GEMM takes 5,966 device cycles where it took 22,895.
- `fuse_generic` folds a map into the map it follows when their accesses agree on the iteration space, an axis of
  extent one aside, rather than only when they are spelled alike.
- **Transformers written in LOGOS, and the GPU kernels under them.** Tensor sugar (`Let A be a 4 by 8 tensor of Real.`, `Let C be A matmul B.`, `softmax of X`, `rmsnorm of X with weight W`, `rope of Q with cos C and sin S from position P`, `attention of Q over K and V with H heads G groups up to P`, `gated delta of …`, `moe of X with router … choosing K of E`, `argmax of X`, row gathers and row writes) is shape-checked at parse time and desugars to ordinary sequences and loops that every tier runs; `tensor_recognize` lifts it back into typed operations the CUDA, Tenstorrent and spatial front ends share. Qwen3.8-27B's hybrid forward (48 Gated DeltaNet and 16 gated-attention layers) is generated as a LOGOS program and lowered to a CUDA generation program whose Gated DeltaNet kernel reads and writes each head's memory once per token; its precise lane reproduces llama.cpp's greedy continuation token for token on the CPU reference and on an A100, recorded at 41.29 tok/s against llama.cpp's 42.17 (`benchmarks/results/qwen38_27b.json`), and Qwen3-8B and Qwen3-30B-A3B forwards lower the same way over Q8_0 weights. Recognized matmuls lower to DP4A int8, WMMA, multistage `cp.async` or `mma.sync` kernels chosen from a legality-checked tiling lattice, with a knob only where a transform wins in one regime and loses in another, retargetable to the RTX 3060 (sm_86) or the B200 (sm_100); a recognized fused-row operation (RMSNorm, softmax, SwiGLU) packages as a SOL-ExecBench solution; and `codegen_sass` decodes, patches and schedules sm_86 machine code with control codes cross-checked against `nvdisasm`. `tensor_sugar_tier_lock`, `tensor_sugar_all_backends_lock`, `tensor_op_coverage_lock`, `tensor_recognize_module_lock`, `qwen38_golden_lock`, `qwen38_authored_lock`, `gated_delta_mechanism_lock`, `cuda_real_qwen8b_forward_lock`, `cuda_qwen30b_forward_lock`, `cuda_matrix_lock`, `cuda_gemm_lattice_lock`, `cuda_wmma_lock`, `cuda_knobs_lock`, `cuda_chip_lock`, `cuda_fused_row_lock`, `sol_solution_lock`, `sass_assembler_lock` (the GPU legs skip without the weights, `nvcc` and a device).
- **SambaNova RDU and Plasticine lanes, modeled.** `largo emit rdu` lowers a program onto the SN40L or SN50 descriptor (`--chip`) through a tensor, counted-loop or whole-program lane chosen by the program's shape, and writes a certified neutral placement, its certificate and a modeled report; `--format` adds Spatial source, a PyTorch module and weights manifest for SambaFlow, or a vLLM deployment plan, and `--bridge mock|sambaflow` hands the artifact to an SDK bridge that refuses anything whose certificate does not recheck. `--model qwen38` lowers the whole model as one certified program per layer kind plus embedding and head programs and a manifest binding each layer, and `--ctx`, `--batch N|auto`, `--tp N|auto` and `--prefill` make it a decode step with a KV cache, a batched decode, a tensor-parallel decode or a prefill chunk: Q8_0 weights stay quantized on the chip, activations and carried state are placed in SRAM with an off-chip plan for the rest, and executed layer by layer the bundle emits the VM's token. Every chip number is labeled published or modeled, sizing reports carry no speed claim, vendor configuration bytes wait on the SDK, nothing ran on an RDU, and each dial is refused by name where it does not apply; `examples/rdu_demo/` walks the three lanes. `largo emit plasticine` turns a counted-loop function or a tiled fp32 GEMM into Spatial source, a regression app checked against the VM, a checkerboard placement and a report row (PCU/PMU use, each metapipeline stage's initiation interval with the critical stage flagged, N-buffer depths, hop counts and static-link occupancy, the DRAM roofline and GFLOPS against the published fp32 peak), with mesh routing installed through `plasticine_place::install_mesh_router`. The public Plasticine toolchain runs in a container (`scripts/plasticine/`): Spatial's own `GEMM_0` passes through PIR mapping, plastiroute and Tungsten's cycle-accurate simulation, and the compiler's emitted loops calibrated the PCU model to within 5.2 % across the simulated apps (`benchmarks/silicon/plasticine_toolchain.json`, `benchmarks/silicon/plasticine_calibration.json`); every report row says `modeled`. `emit_rdu`, `emit_rdu_e2e`, `rdu_emit_matrix_lock`, `rdu_one_lane_lock`, `rdu_sdk_bridge_lock`, `rdu_fit_report_lock`, `spatial_chip_facts_lock`, `spatial_rdu_lock`, `qwen38_bundle_contract_lock`, `qwen38_decode_bundle_lock`, `qwen38_batched_decode_lock`, `qwen38_tensor_parallel_lock`, `qwen38_prefill_lock`, `plasticine_report_lock`, `metapipeline_lock`, `pmu_banking_lock`, `plasticine_sim_lock`.
- `scripts/plasticine/gate.sh` prints the gated app's own row of the toolchain's resource table before the table's tail.
- **LOGOS bytecode runs on FPGAs: a soft-core interpreter, programs as circuits, and a whole-program engine.** `largo build --emit isa` prints a program as the Silicon ISA listing, every instruction tagged with its reversibility class, and `vm::binisa` frames a compiled program as a versioned, checksummed `LGB1` stream whose opcode numbers are append-only. The soft-core interpreter executes the whole ISA (`codegen_sva::full_asm`) or its function-free subset (`codegen_sva::scalar_asm`) against a cycle-accurate model, and programs stream over a UART to it on a Nexys A7-100T built with the open yosys and nextpnr-xilinx flow, every `Show` compared with the VM. `codegen_sva::fabric_rtl` lowers a program to a fetch-free dataflow circuit computing the VM's result modulo 2^64 (115 ops realized, each with an equivalence witness), placed and routed onto a switchbox tile array under a SAT certificate that `PlacementCertificate::recheck` re-checks with the PR/RUP checker alone. The whole-program engine is one universal netlist that receives any fitting program's image and parameters over a UART and runs recursion, heap collections, binary64 floats, text and `Word32`: the core algorithms of all 35 benchmarks run ≡ VM on its model, the 31 chip-sized programs in `benchmarks/silicon/fpga/bench` lower ≡ VM, a call past the frame stack traps `0xE1`, and `scripts/logos-chip.py run` streams a `## Main` program to the engine on a Nexys A7-100T or Nexys Video and compares every `Show` with the VM. Each hardware lowering re-derives what a register holds from the op that writes it (`Op::for_each_written_reg`, `Op::for_each_read_reg`), a nested place mutation reads, clears, mutates and writes back as the compiler does, and `args()` answers `["logos"]` there as on every tier. Coprocessor and memory RTL is emitted and held to its software oracle under iverilog: the big-integer datapath, the G-Counter merge, the bounded pipe FIFO, the SplitMix64 draw, Keccak-f[1600] and the ML-KEM NTT butterfly, SECDED scratchpads, a weight-stationary GEMM tile, SWAR lanes, a SIMT engine, decoupled access-execute, a DRAM command scheduler with an independent legality checker, a DDR2 controller and PHY, a ComputeDRAM device model and a zoned-NAND scheduler. `isa_aot_lock`, `builtin_wire_id_lock`, `soft_core_op_support_lock`, `universal_completeness_lock`, `nexys_bytecode_over_wire_lock`, `fabric_coverage_lock`, `verified_placer_lock`, `pnr_certificate_lock`, `wprog_engine_soc_lock`, `wprog_completeness_lock`, `wprog_op_reachability_lock`, `wpe_depth_guard_lock`, `bigint_silicon_lock`, `crdt_silicon_lock`, `coproc_core_silicon_lock`, `weight_stationary_gemm_lock`, `memsched_lock`, `storage_lock`; `fpga_silicon_lock` runs on an attached board.
- The full soft core lowers `text_bytes`. Chip opcode 237 encodes a Text's codepoints into a fresh `Seq of Int` of their UTF-8 bytes, by codepoint range, identically in the encoder's cycle model and in `soft_core.v`. Every benchmark, `strings` included, now prints the VM's output on the width-64 soft core.
- The FPGA board hosts run from macOS as well as Linux, and from a machine with no Rust toolchain. `scripts/lib/ftdi_ports.py` finds each board's FT2232 channels by FTDI serial and USB interface on both, and the loader runs under `sudo -n` on Linux only. `scripts/logos-chip.py kit OUT TARGET PROGRAM[@ARGS]` exports a board kit: the host scripts, the target's image and build recipe, each program's stream with its VM golden, and the protocol table as `chip_host --table` prints it. Inside a kit, `run`, `suite`, `flash` and `detect` work unchanged and refuse what the kit does not carry. `logos-chip.py run core prog.lg` compiles a `## Main` program for the soft core and checks its output against the VM's. Every other board host and runner under `scripts/` finds its board the same way (`ftdi_ports.py port A|B`, `ftdi_ports.py flash A|B <bit>`; `--persist` writes and verifies the board's configuration flash, so it boots the image at power-on), so a runner no longer defaults to a `/dev/ttyUSBn` that names the other board after re-enumeration or flashes the Nexys Video with the Nexys A7's cable.
- **A program too big for an FPGA is refused at compile time, not in place-and-route.** When a spatial target names its part, the fabric lowering counts the block RAM, LUT-RAM, flip-flops and DSP slices the program's memories and arrays take and refuses it — naming the part, the resource and the largest memories — before emitting a line of RTL, instead of letting the overflow surface hours later in synthesis. A 768×1024 int8 weight is refused on the Artix-7 100T and held on the larger 200T.
- **The integer steps of an int8 model.** `X times M shifted right by S` requantizes (`(x · M) >> S`, an arithmetic
  shift, so a negative product rounds toward −∞), `X clamped between LO and HI` saturates, `relu of X` keeps what is
  not negative, and `lut attention of Q over K and V with H heads KV groups up to P with table E and scale M shifted
  right by S` is one query's attention over the first `P` cached rows with its softmax read from a table of exponent
  weights, its context divided toward −∞. Every operand is a tensor of `Int`. Each form desugars to the loops every tier
  runs; the recognizer lifts it to its spec; the spatial evaluator computes every all-integer body in checked `i64`, never
  through a float; the fabric prints it and the GPU runs it; `tensor_graph::vjp` refuses each as a step function with no
  gradient. `int_tensor_sugar_lock`, `int8_decode_lock`, `tensor_graph_vjp_lock`.
- **An int8 transformer decodes on an FPGA, compiled from LOGOS.** `spatial::int8_model` writes the tiny transformer's
  decode step as a LOGOS program, and the spatial fabric lowering prints it for the Nexys A7 (XC7A100T) and the Nexys
  Video (XC7A200T), `SpatialTarget::xc7a100t_fabric` and `xc7a200t_fabric`. `spatial::lower::fabric_board` makes it a
  board image: a host port over the board's serial line (the Video's through its FT245 FIFO), the autoregressive loop on
  the chip, and a core clock from a PLL at the fastest period the routed design's timing allows. Each board generates
  the golden trace token for token, each step in the cycles the twin predicts: Board A 27.7 µs a token
  (36,075 tokens/s), Board B 43.3 µs (23,073). The ledger records each image's bitstream by size and hash; the
  bitstreams stay out of the repository, and a checkout that holds them is held to every one.
  `benchmarks/silicon/decode_images.json`, `decode_images_lock`, `decode_board_lock`, `scripts/decode-host.py`.
- **The fabric engine** that runs it, every unit held to the evaluator in RTL and to the twin's cycles:
  - memories banked over `SiteSpec::mem_banks`, a tile run moving a word of banks a cycle;
  - a vector unit of `SiteSpec::vector_lanes` beside the matrix array, its lanes chosen per op and its folds a tree; a
    gather reads, in each lane, the element its own index names, through a FIFO read port of its own, so the LUT
    attention's table lookups take every lane; the table's page keeps a pool of its own, so each port copies only it;
  - a row that is not a whole word kept at a pitch of the word it moves in — a tensor of more than one row whose every
    walk keeps within a row, and a matmul's B page — so the tied head's sixteen rows of fourteen move a row a cycle;
    a walk keeps only the address terms that move it;
  - every Generic body pipelined a register a node, a floor division a stage per quotient bit, points issued on a
    modulo schedule;
  - the op table registered on the next state;
  - storage and arithmetic narrowed to the widths `spatial::value_range` proves, an interval analysis of the whole
    choreography;
  - reads with the latency `SiteSpec::local_mem_read_latency` states (block RAM's one cycle);
  - the PE array's weights starting at zero (`codegen_sva::systolic::WeightStart`), so a matmul shallower than the
    array multiplies the rows it does not load into zero rather than into a register's power-up unknown;
  - a sum a product reads kept in a register of its own (`(* keep *)`), out of the DSP slice's pre-adder: yosys 0.33
    folds such a sum into only the top slice of a product it splits over a cascade and ties the bottom slice's operand
    to `x`, and the bitstream multiplied a constant there.

  `fabric_banks_lock`, `fabric_lanes_lock`, `fabric_pipeline_lock`, `fabric_fold_lock`, `fabric_pool_lock`,
  `fabric_width_lock`, `fabric_integer_lock`, `fabric_cycle_breakdown_lock`, `value_range_lock`, and
  `fabric_postsynth_lock`, which simulates the synthesized netlist of the decode image at gate level;
  `fabric_pipeline_lock` synthesizes a product of a sum through the image flow's yosys and holds every DSP operand
  known.
- **A gather on the fabric reads the row its index names.** The fabric printer ignored a tile address's runtime index,
  so every `row tok of E` read the table's first row. `fabric_integer_lock`.
- **A decode step split over two boards.** `compile_program_to_chips` lowers a tensor-parallel program as one ordinary
  program a chip, lists the tensors that cross, and halts each chip where one crosses; `scripts/split-host.py` relays
  them between the boards over the host. Chip 0 on Board A takes 19.6 µs a step on chip, chip 1 on Board B 18.7.
  `benchmarks/silicon/decode_split_images.json`, `decode_tensor_parallel_lock`, `decode_split_images_lock`.
- **Every spatial program runs on the GPU.** `spatial::lower::cuda` prints a program the persistent kernel cannot hold
  as a general program, a block per site with every Generic body and fold, and one that fits one block flattened from
  its choreography; each equals the evaluator on the RTX 3060. The decode step's GPU receipt, the same boot as the
  boards: a hand-written warp 11.4 µs a token, LOGOS's one-block program 46.3 µs, its general program
  105.3 µs. `cuda_spatial_general_lock`, `decode_gpu_lock`, `benchmarks/silicon/decode_gpu.json`.
- **A result only the next statement reads is never stored.** `compile_program_to_spatial_returning` and
  `compile_program_to_chips_returning` lower a program whose caller declares what it reads besides its `Show`s and its
  carried state; a tensor nothing else reads is handed from its producer's compute to its consumer's, the two
  statements' sections one section and their maps one body (`spatial::passes::forward`). A join is made only where
  the store and the load address the same tiles, no other section reads the tensor, and the value stays within the
  interval the front end proves for it; a program whose results are not declared stores every tensor a statement names.
  The decode images are lowered returning their host's token: 1,507 → 1,240 cycles a step on Board A.
  `spatial_forward_lock`.
- `scripts/openxc7-patches/`, applied by `setup-openxc7.sh`: openXC7's nextpnr-xilinx wrote every RAM32M and RAM64M
  LUT-RAM's contents as zero, and its router permuted a LUT-RAM's read pins away from its write pins; it also misplaced
  DSP48E1 cascades and left a slice illegal after an annealing chain swap. Each decode image passes a gate on its FASM
  that every LUT-RAM bit and address pin is the netlist's.
- `scripts/openxc7-patches/nextpnr-xc7-hard-cell-timing.patch`: with `NEXTPNR_XC7_TIMINGS` naming prjxray-db's SDF
  directory, nextpnr-xilinx times the DSP48E1, block RAM, LUT-RAM and shift register ports its chipdb leaves untimed,
  from Vivado's models; every decode image's clock is chosen over that timing.
- `scripts/setup-openxc7.sh` builds nextpnr-xilinx at the commit its patches are written against, and generates the
  chipdbs of both Nexys boards where every flow reads them (`xilinx/<device>.bin`).
- **The decode receipts are built and recorded from the repository.** `scripts/xc7/` is the open 7-series flow of a
  fabric's board image: `synth-image.sh` (the synthesis `fabric_postsynth_lock` simulates), `route-image.sh`
  (nextpnr-xilinx with the hard cells timed from prjxray-db's SDF, then the bitstream), `sweep-image.sh` (seeds and
  placements side by side), the gates an image passes before it is recorded (`fasm_dram_check.py`: every LUT-RAM's
  contents and address pins; `netlist_x_check.py`: no DSP operand tied to `x`), and the recorders of
  `benchmarks/silicon/decode_images.json` and `decode_split_images.json`; the examples `decode_board_image` and
  `decode_split_images` print the images. `scripts/accel/decode-gpu-measure.sh` and `decode_gpu_ledger.py`, with the
  example `decode_gpu_programs`, measure and record `benchmarks/silicon/decode_gpu.json`. Run again over the recorded
  runs, they reproduce every ledger and bitstream byte for byte. `xc7_image_flow_lock`, `decode_gpu_lock`.
- The decode cost model's residency fills the fastest tier with what a token moves the most of per byte held, and the
  fewest chips that hold a model is the least such count. `spatial_decode_lock`.
- **A decode's tied head and rotation table are priced where they live.** For a checkpoint that ties its output head to its embedding table and for a llama-family RoPE frequency table, the batch-1 decode planner read bytes from the wrong memory — the head from the weights rather than the table, the table from nowhere. Both now stream from the tier that holds them; the only committed figure that moves is `benchmarks/silicon/decode/decode_chip.json`'s two Llama-3.1 rows, by ≤4.4e-6 relative.
- **The ZCU104 (Zynq UltraScale+ XCZU7EV) sized in the decode planner.** `benchmarks/silicon/decode/zcu104_sizing.json` prices every checkpoint under `models/` on the board from its own datasheets: the biggest we have that fits its 2 GB DDR4 is Llama-3.2-1B and the fastest is Qwen3.5-0.8B (about 7.6, 17 and 23 tok/s as the programmable logic reads the DDR at 6.4, 14.4 and 19.2 GB/s), while Qwen3-8B is refused past the board's memory. The board's facts — the XCZU7EV's 312 block RAMs, 96 UltraRAMs, 1,728 DSP slices and its 64-bit DDR4-2400 — are cited to DS891, UG573 and the ZCU104 board guide, and the PL-to-DDR read rate to the FPT'19 measurement on the sibling ZCU102. `scripts/accel/gguf_remote_header.py --ledger` writes a checkpoint's header ledger as the planner reads it, with the file's SHA-256.
- **The Stratix 10 SX 2800 in the part catalog.** `PartResources::sx2800()` is the Intel PAC D5005's FPGA (1SX280HN2F43E2VG): 933,120 ALMs and their 3,732,480 registers, 5,760 DSP blocks of two 18x19 multipliers each, 11,721 M20K blocks of 20 Kb and 23,796 MLABs of 640 bits, each figure quoted from the Stratix 10 device overview (S10-OVERVIEW 2020.09.28) or product table.
- **Qwen3.8-27B priced on one Intel PAC D5005** (`spatial::d5005`, `spatial::serving`). A card that holds its
  sequences' token-rows on chip and streams its weights through them once a pass is priced by `serving::PassShape`:
  the pass is affine in the batch and in the split between the card's DDR4 and host memory, over five rates — DDR4,
  the PCIe host link, the 4-bit integer array, the f32 vector units, and the block-scale lanes that turn each weight
  block's integer sums into f32 — at the batch and split that emit the most tokens per second, with drafted tokens at
  their measured acceptance, a frequency-ranked drafting head, a lazily flushed recurrent state, paged and
  Hadamard-rotated int8 KV, host memory as a fifth channel and a mix of per-sequence context limits (`ContextMix`).
  Each way a pass moves its bytes streams at its own share of the DDR4 peak, as the DDR4 ledger prices it on the
  shell's own controller timing: 95.15% on the weights' long read, 94.26% on a recurrent state read and written back
  512 lines at a time, 95.10% on a cache read with its row appended (`serving::Streams`, `d5005::DDR_STREAMS`).
  `benchmarks/silicon/d5005/roofline.json` climbs the trick ladder from the checkpoints' own headers, Q8_0 at 40 tok/s
  to Q4_K_M with every trick at 562 tok/s on 2k contexts (the array, DDR4 and PCIe binding together), and prices every
  format, the context sweep to 262,144 positions, and the shell's ECC-less 576-bit beats (`Beat::Full72`: 432 tok/s
  on the mixed contexts against 365). `d5005_roofline_lock`.
- **The decode engine's arithmetic, bit for bit** (`spatial::decode_engine::{numerics, model, lane}`): every number the
  engine computes, as exact integer operations and IEEE f32 operations with denormals flushed. Activations are
  quantized per weight block as llama.cpp's CPU path blocks them (256 for K-quants, 32 for the legacy formats) under an
  8-bit-significand scale, and each block's two scaled sums are taken exactly and rounded once; the knobs add two
  activation words, activations scaled per 32 elements, an f16 or f32 recurrent state, f16, int8 or Hadamard-rotated
  int8 KV, attention on the array, and the state flushed every 16 tokens through a log of decay exponents. On
  Qwen3.5-0.8B the fastest knob point parts from the f32 run by a mean top-logit gap of 0.0896 against llama.cpp's
  shipped 0.0853 over the short corpus and 0.1005 against 0.0783 over the long, each within the bar's two standard
  errors; with two activation words the rotation takes int8 KV's gap from 0.0796 to 0.0276. `d5005_numerics_lock`,
  `d5005_engine_lock`; the real-checkpoint bars, minutes a knob point, run on purpose through
  `scripts/run-model-bars-tests.sh`.
- **The engine's units in Verilog** (`spatial::decode_engine::rtl`), each held bit for bit to its numerics function
  under iverilog: the f32 multiply and add, the per-32 and per-block activation quantizers, a decoder for every GGUF
  block format the array reads, the processing element (two weight rows per Stratix 10 18×19 multiplier at 15-bit
  lanes through a signed high lane), the block-scale lane from the array's excess partial sums to the row's next f32,
  the matrix-vector step and the array's column that streams a matrix two rows a beat against held token-rows (each
  dot `numerics::dot`'s), the attention unit on the vector units and on the array (a head's scores, softmax and
  weighted sum over its f16 or int8 cache), the lazily flushed state row's read-out and flush,
  the vector units' exp, reciprocal, reciprocal square root, ln, ln1p, softplus, sigmoid and silu, the binary16
  state codec, the Gated-DeltaNet row engine, and the sixteen-lane vector unit running its instruction set's
  micro-programs — the norms, the softmax, the gated products, the Hadamard rotation, the rotary turn, a cache row's
  int8 codes, the causal convolution, the greedy token (`spatial::decode_engine::vector`). `d5005_rtl_lock`.
- **The decode engine's program and its cycle twin** (`spatial::decode_engine::{program, twin}`). A checkpoint's step
  is a program of unit commands — the matrix streams, the vector unit's norms, gates and products, the Gated-DeltaNet
  ring and memory, the cache's append and attend — lowered per knob point, and the engine model is its interpreter, so
  the model computes exactly what the sequencer issues. The twin asks every command for its work on each unit (the
  pricer's weights, traffic, array multiply-adds and lane cycles term for term, and the vector unit's own operations)
  and schedules the commands with the weights streaming ahead into the on-chip memory the batch's token-rows leave;
  `benchmarks/silicon/d5005/predicted.json` sets its tokens per second beside the roofline's at every knob point the
  real checkpoints are held to. A header ledger now reads as its checkpoint's architecture (`GgufHeader::spec`, through
  `model::spec::SpecSource`), so the 27B's architecture comes from the committed ledger. `d5005_program_lock`, `d5005_twin_lock`.
- **The decode engine's machine** (`spatial::decode_engine::machine`): a step as the jobs the sequencer issues over
  the card's memory, every byte where it lives. The memory holds the weight image in the order the program reads it,
  the rotary tables, and a region per sequence — each Gated-DeltaNet block's conv ring, memory and log, each attention
  block's keys and values — in the formats the knobs name; each command is one job, the vector unit's naming what each
  pass loads, the micro-program it runs and what it stores, a float matrix weight row by weight row against every
  token-row, every row the stream engines move whole lines. The machine computes what the engine model computes, bit
  for bit, at every knob point and in every block format the array reads, and a 27B sequence's region is what the
  serving pricer holds for it to within 2%. The sequencer reads the jobs as 64-bit words (`machine::{encode, decode}`).
  `d5005_machine_lock`.
- **The decode engine in Verilog runs a whole step as the machine does** (`spatial::decode_engine::rtl::engine`): the
  sequencer chaining a step's jobs over the token-row store and a port of whole 64-byte lines onto the card's memory;
  the vector unit, its passes loading registers a beat at a time and storing through f32, binary16, int8 and scale
  formats under lane and byte masks; the array's columns in every block format it reads, each weight block gathered
  from the lines it spans; the recurrence unit, `k·q` and a lazily flushed log's fold on the vector unit and the row
  engines streaming each head's state; and the attention unit, each cached row streamed as the unit asks for it, on the
  vector units or the array. The host writes the tables, the embeddings and the store through the engine's port.
  Under Verilator a whole step of the tiny qwen35 leaves the logits, every state and every cache exactly as the
  machine does — at the fastest point through a log's flush, over an eager f32 state and an f16 cache in two words,
  in Q4_K — and so does every job alone over every state, cache and block format. A rotation back scales by an exact
  power of two (`vector::Op::Scale`, `ldexp`): a product rounds a value just below the normals up into them.
  `d5005_engine_rtl_lock`; `scripts/d5005/fetch-verilator.sh` fetches the simulator.
- **The D5005 accelerator's OFS build tree.** `spatial::decode_engine::ofs::afu_sources` writes what the card's FIM PR flow takes: the PIM boundary (`ofs_plat_afu`: the host channel as Avalon-MM MMIO and host memory, each of the four DDR4 banks as Avalon-MM in its own EMIF clock with its 64 ECC lanes carried as data, the compute clock the user clock the AFU JSON asks for), a shell whose only paths between clocks are Gray-pointer dual-clock FIFOs, a stream engine on every channel, the OFS feature header and a command doorbell, and a bring-up core that fills and checks every channel at full rate and copies between host memory and a channel. `scripts/d5005/` holds the gates every image passes (`cdc_check.py`: every clock crossing a FIFO, read from yosys's netlist; `sv_check.py`: the accelerator elaborating against the FIM's own PIM with pyslang), the build (`build-afu.sh`, Quartus Prime Pro 23.4) and the recorder, which refuses an image whose fit, timing or gates failed. `d5005_flow_lock` runs the shell end to end across six clocks under iverilog and proves each gate refuses the defect it exists for. The host side comes with it: `sw/logos_hal.h`, the device a host program drives, its OPAE 2.12 backend for the card, and the bring-up receipt (`sw/logos_receipt.c`: the accelerator's identity, every channel filled and checked alone and all four at once with its bandwidth, a host buffer's round trip through a channel, as one JSON object). The same receipt runs unchanged against the RTL through a VPI backend that steps the simulation only while the program waits on an MMIO access, so a run's cycle counts are the same on every machine (`scripts/d5005/run-cosim.sh`); `run-receipt.sh` loads an image on the card, runs the receipt against OPAE and records it with `record_receipt.py`, which refuses a receipt that did not pass or ran on an image `afu_images.json` does not hold.
- **The D5005's DDR4 channels and its weight image** (`codegen_sva::ddr4_jedec`, `spatial::decode_engine::{ddr, image}`).
  Each of the card's four channels is clocked from its shell's own EMIF configuration (DDR4-2400 at 1200 MHz under a
  quarter-rate controller: CL 21 and CWL 16 plus a parity latency of 4, tRCD and tRP 17 clocks, tRAS 39, tFAW 26, tRFC
  420, tREFI 9,360; four bank groups of four banks in the controller's CS_R_B_C_BG order) and held to JESD79-4:
  `check_bus` refuses every bank, bank-group, bus-turnaround and refresh violation, and an in-order controller model
  opens rows 16 lines ahead of the data, so a long read pauses only to refresh.
  `benchmarks/silicon/d5005/ddr4_efficiency.json` prices a decode pass's streams on one channel: the weights at 95.15%
  of peak (73.1 GB/s for the card at 64 data bits a beat, 82.2 at all 72), a Gated-DeltaNet memory read and written
  back in place at 64.7% in chunks of 8 lines rising to 94.2% at 512, an attention cache read and appended to at 95.1%.
  The weight image lays a checkpoint's tensors over the channels in the order the engine's step reads them, whole rows
  a tile, the tiles dealt least loaded first, so no two channels ever differ by more than a tile and each streams its
  share of a pass as one run of lines; at all 72 bits every tile ends in a CRC-32C.
  `benchmarks/silicon/d5005/weight_image.json` lays out every Qwen3.8-27B checkpoint at both beats (Q4_K_M: 17.20 GB,
  67.2 M lines a channel). `d5005_ddr_model_lock`, `d5005_weight_image_lock`.
- **The D5005's image stream engine, DDR4-timed channels, and the weight image on the card**
  (`spatial::decode_engine::rtl::stream`, `codegen_sva::ddr4_jedec::verilog_model`, `spatial::decode_engine::ofs`). The
  stream engine takes one memory channel's tiles of the weight image and sends their rows on as 64-byte beats tagged
  with tensor, first row and byte offset, regathering 72-byte lines into beats (eight lines make nine) and checking
  every tile's CRC-32C as its lines pass, four tiles' reads outstanding so the channel's line rate holds across tiles:
  under iverilog on the shell's own channel path every beat equals its software twin's, and a bit flipped in a tile is
  caught on that tile alone. The co-simulation's channels can be DDR4-2400 channels timed by the controller model
  (`ChannelModel::Ddr4`): every line they schedule, every command they place and every read's return clock are the
  Rust controller's, and the bring-up receipt passes on them. The bring-up core copies between host memory and a channel
  at the channel's full line width (`HOST_TO_MEM_PACKED`, `MEM_TO_HOST_PACKED`), and `sw/logos_load.c` loads a weight
  image the image writer made (`d5005_weight_image -- --write`: `channel<k>.bin` and the tile table `tiles.bin`) and
  proves it on the card: read back against its files, every tile's check word holding, a damaged line found in its tile
  and the tile loaded again — against the RTL through `scripts/d5005/run-cosim.sh <afu> load`, on the card through
  `run-load.sh`, and the card's first day as one command, `bringup.sh` (the receipt, then the image with its
  self-test). The writer reads every image it writes back: Qwen3.8-27B Q4_K_M at 72-byte lines is 17.2 GB over 522,132
  tiles, every check word holding and every tensor's first and last rows the checkpoint's. `d5005_image_stream_lock`,
  `d5005_ddr_model_lock`, `d5005_flow_lock`, `d5005_weight_image_lock`.
- **The compiler emits its own physical design, signed off on sky130, and Caravel user projects.** A process is a value (`codegen_sva::pdk_target`: `SKY130A`, `GF180MCUC`, with cell library, metal stack, SRAM macros, shuttle user area and harness timing); a block's OpenLane configuration is derived with the measurement behind each number quoted at its field (`codegen_sva::flow_config`), a registered block's clock is the period its slow corner meets, and the emitter refuses any other or an unknown setting instead of building at a default. Hardened blocks (soft cores, fabric SoCs, systolic tiles, decision blocks, the DRAM executor, a SIMT engine, the PSRAM edge, a coprocessor core) are recorded signoff-clean with every sky130 corner closed in `benchmarks/silicon/sky130_signoff.json` and its sibling ledgers. The certified DRAM command player is a Wishbone Caravel user macro inside the unmodified `user_project_wrapper`; macro and wrapper are signoff-clean and the assembled submission passes all 19 `mpw_precheck` checks (`benchmarks/silicon/mpw_precheck.json`). The whole-program engine is prepared as a second Caravel user project: its heap, call-frame stack and configuration image live in an off-die APS6404L QSPI PSRAM behind write-back caches that answer a resident line exactly as block RAM does (a miss freezes the engine instead of changing it), the UART sits on GPIO 8/9 and the PSRAM on 10–15 at 20.8 MHz, with the PSRAM read closed against Caravel's characterized harness timing (`scripts/check-wpe-psram-timing.sh` closes the macro's own share on the routed netlist at the slow corner, `benchmarks/silicon/wpe_psram_timing.json`), a heap access past its region traps `0xE2`, the host refuses every table the profile cannot hold, and 24 of the 35 suite programs run on the shipped profile. Its macro and its wrapper harden signoff-clean at every corner, slew included, and pass the foundry's golden DRC deck (`benchmarks/silicon/caravel_macro_signoff.json`, `benchmarks/silicon/caravel_signoff_wpe.json`), and the assembled submission passes all 19 `mpw_precheck` checks (`benchmarks/silicon/mpw_precheck_wpe.json`). The Caravel chain (macro harden, wrapper harden, golden deck, submission, `mpw_precheck`) reads one project registry (`codegen_sva::caravel_project`), a macro is timed against the harness's characterized input delays and Caravel's own clock uncertainty and derate, takes power either through its own met4/met5 grid or fed by the wrapper at met4 (`--macro-grid own|fed`), carries its own measured hold margin, density, met1 penalty, slew-detection margin and synthesis strategy, repairs slew at every corner (again after antenna repair, legalizing each round's buffers before the next round measures) with 800 µm signal and clock wire limits or OpenROAD's own, and records into a signoff ledger that re-derives it. The antenna ECO writes the netlist again from the layout it leaves, so signoff STA times the cells that were drawn, and the recorder refuses `signoff_clean` to a run whose STA netlist and final layout differ by a single cell (`sta_netlist_missing_cells`, `sta_netlist_extra_cells`); its diode ties may rise by a via stack from the diode's own pin into the violating layer, a diode lands only where the island it shares below the gate's join stays under its own diffusion ratio (read from the LEF), and its clearance queries read a spatial index of the nets rather than walking all of them (a 342-gate jog pass on the engine took 21 hours). The OpenLane every block is hardened with is reproducible from the repository (`scripts/openlane-overlay/` over a pinned base, installed byte for byte), a golden verdict's KLayout reports are written beside the GDS they judge, a submission's own files and the wrapper's modified SDC live in `benchmarks/silicon/caravel/`, and assembled submissions live under the ignored `tools/caravel_submissions/<project>`. `SUBMISSION_OUT` assembles and prechecks a submission elsewhere, a staging copy a batch of re-signs lands with its ledgers, and the precheck ledger names the project's own directory, where it lands. `sky130_signoff_lock`, `sky130_deep_signoff_lock`, `signoff_ready_lock`, `signoff_period_lock`, `sky130_provenance_lock`, `flow_config_lock`, `caravel_signoff_lock`, `caravel_bringup_lock`, `mpw_precheck_lock`, `caravel_project_lock`, `wpe_offchip_memory_lock`, `wpe_caravel_lock`, `psram_edge_lock`, `wpe_floorplan_lock`, `openlane_overlay_lock`, `embedded_rtl_goldens_lock`.
- The post-route antenna diode ECO reaches an upper-layer run of the gate's net that passes beside a diode site. It stacks to the layer below the run, makes one straight jog there, and vias up. A straight stack needs the run directly over the diode's pin, which a vertical run at one x almost never is. On `coproc_core` the jog tied all four met4 violators that every site had refused, and OpenROAD's antenna check read zero.
- The antenna jog router refuses a bridge whose new metal joins its own net through a corner or a short stretch of edge. The merged polygon there is narrower than the width rule, which the shuttle's maximal-rule deck flags and Magic does not. On `gemm_mac_realbist` this moved one met4 bridge 3 µm and cleared both golden `m4.1` violations.
- `scripts/openlane-run.sh` forwards or refuses every setting it accepts. Eleven settings had reached nothing. `GRT_ANT_ITERS` and `GRT_ANT_MARGIN` now forward through the new emitter flags `--grt-antenna-iters` and `--grt-antenna-margin`. `GRT_REPAIR=0` selects `heuristic+jog`, and `FP_CORE_UTIL` reaches the config. `SYNTH_MAX_FANOUT`, `RT_MIN_SIGNAL_LAYER`, `OL_EXTRA` and resizer or hold-check opt-outs stop the run with their reason. A malformed `--eco-iters` is refused instead of ignored. `PDN_VPITCH` sets a block's vertical power-strap pitch through the new `--pdn-vpitch`. `HOLD_SLACK_MARGIN` sets the hold repair's margin through the new `--hold-slack-margin`, refused beside `--vehicle-macro`, whose hold margin is `--hold-margin`.
- The OpenLane overlay works to the SDC's fanout limit with every antenna diode counted as a load, as the liberty counts it. Clock-tree synthesis clusters within the limit and clones a clock buffer still over it. A diode-aware step splits an over-limit net by whole gates: each gate keeps the diodes placed for it, and each split buffer sits at its bin's centroid with its input guarded the way the heuristic diode pass guards a net's inputs; where no whole bin fits beside the buffers, the most critical gates stay on the driver. The post-route diode ECO adds a diode only where the net has room, and the steps leave it one load. The post-diode upsize re-measures slew on the global route the flow makes after antenna repair, and it never hands a held net to OpenROAD's single-net repair, which ignores don't-touch. The step's closing setup repair works every violating endpoint, not only the worst, while the design is within a clock period of its clock; on `soft_core_realbist` that took the step from +0.016 to +0.061 ns at the slow corner. A run missing by more needs a longer period, and its repair works the worst endpoint only: there the every-endpoint repair had rebuffered `qorsyn_k2_logos_p10` (−28.9 ns at a 10 ns clock) until OpenROAD's rebuffer walked a looped global route into a recursion that overflowed the stack. When the strongest buffer already drives a net held for its diodes, the post-diode upsize rebuffers it with the resizer's own repair and puts every diode back on its gate's net, or halves the longest arm of a detoured wire, and the step fails if a diode left its gate or a net went over the limit; on `coproc_core` that took the slow-corner slew violators from 18 pins to 0. The post-diode upsize then judges the route detailed routing follows for antennas — global routing's repair, the fanout step's splits of the nets its diodes take over the design limit, timing repaired with every dioded net held, and the slew rounds again, to the pass whose diodes the splits shed (at most three). Both diode-aware steps split through one file (`logos_diode_fanout_lib.tcl`), give each diode to the nearest pin of its net with the driver included, and fail, naming it, if an edit leaves a diode more than 100 um from every pin of its net; `coproc_core` then signed off clean on every gate, the golden deck included. Wherever the post-diode upsize runs, detailed routing follows the route it judged, also when global routing repairs no antenna; on `systolic_pipe16_realbist` the route made after it had carried 13 nets to 0.76–0.92 ns of slew at the slow corner. Each incremental global route the step makes continues its route under that route's arguments, since OpenROAD's `global_route` reads congestion tolerance and iterations from every call; a bare incremental re-route had refused the congestion the first route accepted and stopped `systolic_os_pipe8_realbist` with GRT-0232. Every block is re-signed on these steps, fanout zero, except `systolic_os_pipe8_realbist` and the whole-program engine's Caravel macro, wrapper and precheck, which keep their earlier records until their re-harden; each re-signed entry also records its ground's worst rise (`ground_bounce_worst_mv`) and its worst cell's supply drop plus ground rise (`supply_droop_worst_mv`) beside the supply's drop.
- **Faster signed clocks.** Hardened blocks are re-signed at the period every corner meets under delay-oriented synthesis, and the emitter builds each registered block under the strategy its period was measured with: the DRAM command player at 5.0 ns (from 10.5), its Wishbone macro at 4.8 ns (from 11), the HFT decision fabric at 4.4 ns (from 15) and the CGRA at 6.4 ns (from 18). The whole-program engine's float units, conversions and integer multiplier are staged one carry chain, leading-one search, barrel shift or compressor tree a stage, and its Caravel project is re-timed to 47.62 MHz (from 20.8 MHz), the period its slow corner meets: hardened at 18.5 ns and routed, it missed setup at ss_100C_1v60 by 1.13 ns on extracted parasitics, where the routing stage's repairs had closed on global-route estimates, so it runs at 21 ns. The PSRAM edge takes two engine cycles per SCLK half, so a read still lands inside the half period, and the UART divisor is 413. Its macro re-hardens at that clock.
- **The Whole-Program Engine runs on `user_clock2`.** Caravel's chip signoff times its core clock, `wb_clk_i`, at 25 ns, and the management SoC runs on it, so the engine's 21 ns clock is `user_clock2`, the PLL's 90° phase through a divider of its own. The engine's reset crosses into its clock through two flops, and the firmware resets it on logic-analyzer probe 0 after moving `user_clock2` from the pad clock to the PLL; the Wishbone acknowledge stays on `wb_clk_i`. A Caravel project names the harness clock its logic runs on, a project that drives the core clock faster than the chip signs it is refused, the macro is timed with the bus on `wb_clk_i` asynchronous to its own clock, and the wrapper is timed at both clocks (`usr_clk` from the scaffold's `usr_*` latency and transition), its ledger recording each.
- **The whole-program engine's stall comes from registers.** Each off-chip memory's cache took its hit from a tag
  lookup of the address the engine had registered, and that hit is the stall that freezes every register of the
  engine, so the lookup and the stall's spread across the die shared one cycle: at the slow corner it missed setup by
  1.13 ns at 16 ns (the frame cache) and by 1.71 ns at 18.5 ns (the config image's lanes, through `c_cellptr + ci`).
  The engine now registers each hit beside the address as it sets it, looked up in the lines as they will hold after
  that edge (a fill completing at the edge holds its line), and refreshes it for a held address; the cache's `busy`
  reads those registered hits, so the engine stalls in exactly the cycles it did. Every address a cache serves is set
  in one task beside its hit (`ci_set` for the cell index), and in simulation each cache counts any cycle a registered
  hit disagrees with its lines. `wpe_offchip_memory_lock`.
- **No wide single-bit select in the whole-program engine.** The FPU's incrementers carried `cin` to all 64 bits on
  one net (`{64{cin}}`), and the engine's signed-multiply overflow test replicated the product's sign 64 times, the
  met1-antenna shape `silicon_antenna_safety_lock` refuses. The incrementer now enters `cin` as its parallel-prefix
  scan's bit 0 and the overflow test reads `{hi, sign}` with two reductions; yosys's SAT proves each is the function
  it replaced over every input (`silicon_antenna_safety_lock::the_rewritten_selects_compute_what_they_replaced`).
- **The memory subsystem at 8.1 ns and the GEMM array at 9.4 ns.** `infinichip` re-signs at 8.1 ns (from 15): the SECDED codec computes every parity as one balanced reduction and decides an uncorrectable word beside the correction rather than after it, and each codec is registered at its die-side boundary, so a codeword's verdict leaves one edge after it arrives. `gemm_mac_realbist` re-signs at 9.4 ns (from 27) on the pipelined MAC array with its BIST signature leaving from a register; its power is re-measured on the new netlist.
- `fabric_horner_seq` is re-signed with its vertical power straps 38.4 µm apart (`ClosureSettings::pdn_vpitch_um`), half the 76.8 its record had: a standard cell midway between two straps sees the longest rail, and its worst cell's droop — the supply's drop plus the ground's rise at the cell — falls from 106.45 mV to 66.50 mV (supply 33.88, ground 34.06), under the 90 mV budget. Every other gate is zero, setup +0.98 ns at 41 ns, the golden deck clean.
- The output-stationary tile `systolic_os_pipe8_realbist` folds its self-test row by row: each row's eight accumulators fold into a registered row word beside the row, the row words into the two registered half-folds, and the MISR takes those, so the signature absorbs every accumulator two clocks late instead of one. The single fold of all 64 accumulators was the block's routing hotspot: placed at their centroid, it left global routing 4,697 GCells over at the centre of the die; with the row fold the block routes, and its slew closes with the post-diode upsize at a 28% margin. The array (`systolic_os_pipe8.v`) is unchanged. Re-signed on it: every gate zero, its max-fanout count among them (2,360 before), setup +1.86 ns at 20.5 ns, the golden deck clean. `pipelined_systolic_lock`.
- **Global routing's effort is a lever.** `ClosureSettings::grt_overflow_iters` (`GRT_OVERFLOW_ITERS`) and
  `ClosureSettings::grt_adjustment` (`GRT_ADJUSTMENT`, the share of every gcell's capacity the global route holds back)
  are emitted beside the per-layer adjustments only when chosen, so every block built without them keeps OpenLane's own
  50 iterations and 0.3. `emit_flow_config --grt-overflow-iters N --grt-adjustment A` refuses an iteration count that
  is not positive and an adjustment outside [0, 1); `openlane-run.sh` forwards `GRT_OVERFLOW_ITERS` and
  `GRT_ADJUSTMENT`; a Caravel project names its own (`CaravelProject::macro_grt_overflow_iters`,
  `macro_grt_adjustment_permille`), which `caravel-macro-harden.sh` passes on. Measured on the whole-program engine at
  16 ns: its global route stopped at GRT-0119 at 50, 150 and 400 iterations alike, and routed with no overflow at 150
  once the adjustment gave capacity back, and its route for detailed routing overflowed by one track in 72 gcells at a 0.2 adjustment once the fanout step had split its nets: the engine's project routes with 150 iterations and a 0.1 adjustment. `flow_config_lock`, `caravel_project_lock`, `flow_script_guard_lock`.
- Clock-tree synthesis no longer spins on a large core. TritonCTS splits each H-tree level between two branch points with an integer min-cost flow (LEMON's `NetworkSimplex<int>`) whose arc costs grow as distance^exponent; at its default exponent 4 a core more than about 1.35 mm across (width plus height) can push a reduced cost past `INT_MAX`, and the pivoting never ends: the whole-program engine's macro spun 22 hours at the tree's second level. The overlay's `cts.tcl` now passes TritonCTS the largest exponent up to 4 under which the flow's worst reduced cost (`2^30 + 9·D^e`, `D` the core's width plus height in TritonCTS's wire unit) stays a signed int (`logos_cts_kmeans.tcl`): small blocks keep 4 and build the same trees; the engine takes 3 and builds its tree in two minutes. `cts_kmeans_lock`.
- A Sky130 signoff entry records the OpenLane image its run flowed in (`openlane_image`, from the `openlane_image` file its launcher writes beside the run) when the launcher names one, and `record-sky130-signoff.py --verify` holds the entry to it. A run with no such file records no field, so every entry recorded before re-derives unchanged.
- **The comparative-synthesis bench** (`scripts/qor/`, `benchmarks/silicon/qor*`, `comparative_synthesis.json`): seven kernels — K1 trading decision, K2 Horner polynomial and K6 SIMT shader compiled from LOGOS, K3 GEMM tile, K4 Keccak-f[1600], K5 NTT butterfly and K7 DDR2 controller from its RTL generators — against Bambu, Calyx, Gemmini, Vortex, Spatial and LiteDRAM: a shared functional harness, Artix-7 place-and-route (yosys + nextpnr-xilinx, five seeds, a router ladder), Sky130 signoff in the bench's own OpenLane tree at each candidate's minimum period and at the kernel's iso period, gate-level power, and a DDR2 traffic gate held to JEDEC; every printed number re-derived from its tool reports by `comparative_synthesis_lock`, every run bound to the RTL it measured, and every row committed with its excerpts under its ledger's lock. K1 is also compiled for standard cells, K6's SIMT step also taken on ports, and Bambu also scheduled against its Nangate45 cell model for every kernel.
- LOGOS candidates that pipeline what the compiler emits: the trading decision one carry chain a stage, the Horner residual carry-save lowered, the NTT butterfly as an explicit add-tree datapath, the Keccak round in two halves, the SIMT lanes on one streamed multiply, and `ddr2_dfi_ctrl`, a bank-lookahead DDR2 controller on LiteDRAM's own interfaces.
- `scripts/openlane-run.sh` runs a flow in the bench's OpenLane tree (`OL_BENCH=1`) or on OpenLane as released (`OL_STOCK=1`), and a comparative block (`COMPARATIVE=1`); the overlay lays itself over the tree it is given. `scripts/opensta/` builds OpenSTA with the VCD-activity fix that makes gate-level power read the simulation's toggles.
- The comparative bench's Sky130 flows, attempts and controls alike, run OpenLane's image with its OpenROAD (41a51eaf) rebuilt from pinned sources with one fix (`scripts/openroad/`, `build-openroad.sh`): repair_timing walks a net's global route into a buffered tree, and on a route that closes a cycle the released walk never ends and overflows the stack; the fixed walk takes a route without a cycle exactly as before and one with a cycle once over a spanning tree. Every bench run records the image it ran.
- The bench runs a kernel's Sky130 flows side by side (`run-qor-sky130.py kernel|powers --jobs`, `BENCH_FLOWS`): a run is recorded only when the ledger holds the recorder's extract of that run, and `scripts/record-sky130-signoff.py` writes a ledger whole under one lock, so flows finishing at once lose no record. The bench image is also tagged with the machine's architecture, the name OpenLane's Makefile launches.
- The comparative bench's Sky130 verdicts gate on an anchored IR drop. `scripts/anchor-ir-drop.sh` replays a run's IR analysis in the run's own OpenLane tree and in the image the run names beside itself (`<run>/openlane_image`, the bench's image as `build-openroad.sh --name` prints it, which `scripts/openlane-run.sh` writes for a bench run and a control before anything reads the run), or OpenLane as released for a run in `tools/OpenLane` that names none; any other run is refused before a container starts. A bench run or a control whose replay fails records nothing (`openlane-run.sh` exits 5), and `ANCHOR_IR=0` is refused for them. A run's flow identity names the IR analysis, so a run anchored otherwise is run again. The runner starts a flow only with 50 GB free on the disk its trees are on, and drops a run it did not record down to its logs and reports.
- The bench runs several kernels whole at once under one flow limit (`run-qor-sky130.py kernels K3,K6,…`, each kernel with the controls named for it), and a kernel's power blocks for named candidates only (`powers K --candidates`). A third-party design whose memory template defaults its init file to one its tool never emits (Bambu's `array_a.mem` and `array.mem`) is staged with an empty stand-in under that name, marked in its stage row: OpenLane's yosys parses every template and would refuse the design for the missing default.
- The bench starts a candidate's Sky130 search at its synthesis estimate plus 15% (`run-qor-sky130.py --start-from`), treats the global router's second congestion verdict (GRT-0232) as congestion, and names a lint refusal by Verilator's first error. `record-qor-power.py` measures a routed netlist with tie cells (a pin with no net reads as such; the read ends with END, so a broken-off read is refused), and `all` passes over a power block whose run missed. `measure-silicon-power.py` mounts the targets of a checkout's top-level links for OpenSTA.
- **An analog lane whose tolerance is a compiler output.** `largo emit analog` lowers a float function whose inputs carry `Assume x is between A and B.` ranges onto analog tiles (weighted Kirchhoff sums, programmed gains, four-quadrant multipliers) and writes the tile netlist, a power-of-two gain schedule, a range certificate that `logicaffeine_proof::interval_replay` replays, an itemized error budget whose `recheck` re-derives its total, a placement, and a behavioral ngspice netlist with its simulator tolerances pinned. An integer region is claimed exact only when its budget proves the error below half a grid step; streamed reductions, OTA-C integrator loops, relaxation solves of `A·x = b` with an exact rational residual claim, a 3D stack with a certified thermal map and per-die calibration build on the same artifact, and `codegen_mx::partition` binds a function to the analog plane only when its certificate admits it, keeping its digital lowering beside it. `codegen_dram` classifies every op for DRAM compute-in-memory and composes bit-serial arithmetic, popcount and set algebra from `{And, Or, Not}`. Every refusal is a named error, every physical constant is sourced or labeled modeled, and no analog silicon exists. `analog_artifact_lock`, `analog_range_certificate_lock`, `analog_error_budget_lock`, `analog_int_lock`, `analog_relax_solve_lock`, `thermal_map_lock`, `mx_partition_lock`, `dram_op_coverage_lock`.
- **Every published crate builds from its own crates.io tarball, and ships only what it builds.** Every published manifest carries an anchored `include` allowlist, so tests, fixtures, fuzz corpora and examples stay out of the tarballs; each file a crate embeds lives in that crate (the Verilog the compiler emits beside its emitters, with the `benchmarks/silicon/` copies held as goldens of it; `largo`'s MCP resources rendered live or embedded from `apps/logicaffeine_cli/assets/agent/`). `packaging_lock` parses every published crate's module graph, through `#[path]` and macro token streams, and refuses any embed a registry build cannot resolve; `scripts/check-packaging.sh` builds every tarball against a local overlay of its siblings, builds each docs.rs feature set, refuses a `#[test]` in any unpacked crate and holds each `.crate` under 8 MiB, per push in CI and as the release preflight; and `publish.yml` publishes through cargo's own multi-package `cargo publish`, skipping only crates already on crates.io at their version. A new `msrv` job checks every published crate on the declared minimum toolchain. The released `largo` and `logicaffeine-lsp` binaries build with `[profile.dist]` (whole-program LTO, one codegen unit, stripped symbols, panics still unwind), which on x86_64 Linux takes `largo` from 53.5 MB to 37.2 MB and `logicaffeine-lsp` from 25.5 MB to 16.3 MB against the release profile; `.config/size-budgets.toml` holds a budget for every published crate, both binaries and the web app's WebAssembly, checked by `scripts/check-packaging.sh`, `scripts/check-binary-sizes.sh`, the release workflow and the frontend's deploy gate. A GitHub release's body is this CHANGELOG's section for its tag, every artifact the release copies is uploaded unconditionally by a job it depends on, the release's install matrix covers `linux-x64 --full` on the `ubuntu-22.04` glibc floor and `macos-full`, the one-line install command is byte-identical on every surface that shows it, and `scripts/bump-version.sh` anchors every rewrite to its own line and moves every surface that cannot inherit the version. `packaging_lock`, `size_budget_lock`, `dist_profile_lock`, `release_assets_lock`, `installer_docs_lock`, `bump_version_script_lock`, `version_lock`, `feature_forwarding_lock`.
- **Every test lives in one crate, and no exclusion can hide one.** The crate-local `tests/` directories (1,350 integration binaries with their helpers and fixtures) moved into `crates/logicaffeine_tests/tests/` under their own names, and every inline `#[cfg(test)]` body moved to `crates/logicaffeine_tests/unit/<crate>/<module path>.rs` behind a one-line `#[path]` stub, so unit tests keep their names and private access while no published crate ships test code; `test_home_lock` keeps any other package from growing a `tests/` directory and `unit_home_lock` refuses a `#[test]` in any `src/`. The CLI suites spawn `largo-under-test` and `logos-sat-under-test`, each one call to the product crate's library entry. `scripts/run-all-tests-fast.sh` drops six classes of test from the routine baseline (the SAT research campaign, the optimization-graph audit, the heavy on-demand locks, the kernel-pending specs, the qemu corpus and the Arena mathlib inputs), each class has a companion runner that runs exactly those tests, and CI reads the classes straight out of the script; `ondemand_exclusion_lock` parses the scripts and refuses a dropped clause no runner claims or two runners claim, a binary a runner claims that the baseline does not exclude, and a missing runner. The heavy runners carry their own contracts: the qemu runner requires its toolchain, and the research, Arena and audit runners cap their own memory. Tests that must not overlap run serially in their own nextest groups (the post-synthesis gate-level simulations of `wpe_postsynth_silicon_lock` run from `scripts/run-silicon-tests.sh`, the qemu corpus from `scripts/run-qemu-corpus-tests.sh`), the isolated `--features wasm-jit` pass reads one filterset (`scripts/test/wasm-jit-filter.txt`) that the fast runner and CI share, a missing tool is reported as `LOGOS_TOOL_SKIP` and fails under `LOGOS_REQUIRE_TOOLS`, CI drives a LOGOS page in headless Chrome and runs the RISC-V tier under `qemu-user`, and the forge JIT workflow's x86-64 macOS leg runs on `macos-15-intel`. Mutation-survivor and coverage-floor ratchets live in `.config/`. `test_home_lock`, `unit_home_lock`, `test_bin_shim_lock`, `ondemand_exclusion_lock`, `wasm_gated_suite_lock`, `cli_tool_preflight_lock`, `mutation_lock`, `changelog_citation_lock`, `browser_smoke`.
- **Every feature-gated test has a runner, and every on-demand runner builds only its own binaries.** `scripts/run-opt-in-feature-tests.sh` runs the suites behind the features the baseline leaves off — the kernel's `nbe` engine, the system's `io-uring` engine and file backend, wirebench's `capnproto`, `ffi-link-tests`, compile's `narrow-value` — one cargo invocation per feature. `feature_coverage_lock` reads every `cfg` gate that mentions a feature (on a test binary or anything in it, on a unit-test module or anything in it, through every module path from its root) and each runner invocation's feature set as `cargo tree` resolves it, and fails on a gate no invocation turns on; each opt-in invocation must reach a gate the baseline does not and keeps the baseline's exclusion classes, and no runner may name a binary its own features compile to nothing (the Arena mathlib runner builds `nbe`, so `dag_dual_agreement`'s whole-Mathlib case runs there). The five selector runners pass their binaries as `--test` from one `BINARIES` list and build at `test-opt`; `ondemand_exclusion_lock` holds each list to its selector and registers every `scripts/run-*-tests.sh`.
- Every test binary that reaches the GPU runs in nextest's `cuda-gpu` group: `decode_gpu_lock`, `spatial_forward_lock`
  and `spatial_section_accumulator_lock` join it.
- **Getting started, and documentation that runs.** `docs/getting-started.md` goes from installing `largo` to a tested program, a tested web page and a sentence turned into logic, with no Rust toolchain; `getting_started_lock` replays it and the README's Quick start command by command with nothing but `largo` on the `PATH`. Every crate README is its crate's rustdoc root and crates.io page and follows one shape, and its examples run; a `logos` block shown beside its Rust must match what the compiler emits; public prose is held to plain words; and docs.logicaffeine.com opens on a grouped, versioned landing page. `getting_started_lock`, `doc_examples`, `readme_lock`, `public_prose_lock`, `llms_lock`.
- **Exact device arithmetic for the accelerator backends** (`logicaffeine-compile`). The VM's 64-bit integer
  and binary64 operations are built from the lane words a device actually has. A multi-word integer core and an
  IEEE-754 softfloat are parameterized by word and multiplier width. Each fabric cell is reconciled with the VM's
  kernel through `eval_checked` and a named `Divergence`.
- **The bytecode→C walker is a full backend: every program the VM runs, it runs byte for byte**, including the VM's failure sentences, taken at generation time from the functions the VM calls; `today`/`now`, `args()`, `randomInt`/`randomFloat`, text bytes, words, the sandboxed file system, the offline network, value semantics without reference counts, and exact integers held on machine words until one outgrows them (the region moves into an exact twin at the first operation that does not fit, at 0.955-0.996x of unchecked code on collatz). The offload seam (`compile::compile_to_c_via_bytecode_offloaded`) lets an accelerator answer a function's calls at its entry and decline back to the host. Locked by `c_bytecode_lock` (the walker against the VM on stdout, stderr and exit status, fuzzing included) and `c_offload_seam_lock`; every harness now gives `today`/`now` one instant (`tests/support/fixed_clock.rs`).
- C bytecode maps preserve exact integer values across calls, copies and memoized recursion. Bounded counting proofs reject nested work and effectful updates, and WebAssembly distinguishes Nothing from integer zero without narrowing large integers.
- **One C core for convergent collections.** `codegen_c::CRDT_CORE` is the data crate's observed-remove set (add-wins and remove-wins) and observed-remove map ported to C once, for every C-family tier and RISC-V to lower onto. It is held to the data crate operation by operation under the sanitizers, leaks included. `crdt_core_differential_lock`.
- **`largo build --emit c` builds the program through C**, the same executable the Rust build makes, from the optimized program or (`--emit c-bytecode`) from its bytecode, with the flags the benchmark suite builds hand-written C with; `largo run --interpret --engine jit|vm|tree` picks the in-process engine.
- **Every benchmark is locked on every backend.** `benchmark_logos_tiers_lock` holds all 35 programs to the bytes the C, C++ and Rust reference ports agree on, through Rust, C from the program and from its bytecode, the JIT, the VM, the tree-walker, WebAssembly and RISC-V. The benchmarks page shows six of them beside the other languages, with each tier's generated code and the JIT's machine code.
- **Models as data, tokenized as llama.cpp tokenizes them.** `model::tokenizer` reads a checkpoint's vocabulary and splits and merges text exactly as llama.cpp does: every pre-tokenizer it ships, SentencePiece and byte-level BPE, the special-token order and end-of-turn choice its C++ library makes. It is held to llama.cpp on seeded text for every bundled vocabulary and every local checkpoint (`model_tokenizer_lock`). The `f64` reference forward reads Gated DeltaNet, gated attention with a multi-section rotation, sliding windows, attention sinks and soft-caps, YaRN, multi-head latent attention, softmax and sigmoid expert routing with groups, shared experts and dense lead layers from a checkpoint's header (`model_reference_lock`, `model_conformance_lock`), and `model::gguf::write` writes checkpoints (`model_gguf_lock`).
- **Generation compiled from the authored step.** `codegen_cuda::lane` compiles a model's decode step and its prefill, written in LOGOS from its spec, into one CUDA program. The program reads its quantized projections in place from the checkpoint, takes a prompt in chunks of up to 512 rows (a chunk of more than sixteen on the int8 tensor cores in every weight format, its expert mixtures expert by expert, DeepSeek's latent attention expanded — the latent's expansion halves reading their activations exactly, so the absorbed and expanded forms agree — and, in the decode step, every head in one block reading each cached row once), keeps every tensor a step computes in one arena by its lifetime, captures each step as a CUDA graph, and applies the residual-add, QK-norm, cache-row, convolution, gate, gate-and-up, shared-expert and quantization fusions where the dataflow allows. A wide chunk's projections of one activation run as one GEMM, its gated pairs paired with the activation in the GEMM's epilogue, its Q8_0 matrices streamed into the tensor cores' tiles, its `f32` projections in tiles and its Gated DeltaNet rule a 64-row chunk at a time: its attention on the f16 tensor cores up to 256-wide heads under the one word, and on the int8 tensor cores in two words under the two, every product of the operands' words taken, so it computes the attention of its operands in two words to `f32` rounding: a 512-token prompt prefills at 15,236 tokens a second on Llama-3.2-1B, 16,204 on Gemma-3-1B, 15,326 on Qwen3.5-0.8B and 2,419 on Qwen3-8B. Each fusion computes, bit for bit, what the same program computes with the fusion held back. A step's launches run over four streams, each waiting on another's launch by an event only where memory orders them, so Qwen3.5-0.8B decodes at 309 tokens a second. Dense, expert and hybrid recurrent models compile alike, Llama 3.2 included with no code of its own, and Qwen3-8B continues llama.cpp's golden prompt token for token (`lane_compile_lock`, `cuda_lane_lock`). The `activations` knob picks a projection's activation precision: `f32` with `f64` sums, a two-word int8 (the default, at the speed of the weight bytes), or llama.cpp's one int8 word. Each point meets its own bar on Qwen3.5 and Llama 3.2 over a 32-prompt corpus: the precise and two-word lanes follow llama.cpp's `f32` run further than its shipped build does, and the one-word lane's logits sit as close to the `f32` run as the shipped build's. A model past the device's memory runs whole: the program keeps on the device what fits the memory it finds free when it starts and reads the rest from mapped host memory, the matrices a step misses least moved first, so gpt-oss-20b's 12.1 GB meets every precision's bar on the RTX 3060's 12 GB, decoding at 112 tokens a second to llama.cpp's 99; a tied head reads its embedding table's upload rather than a copy of its own.
- **Every GGUF weight format in the compiled lane.** The generation lane reads Q2_K through Q6_K, Q4_0 through Q5_1, MXFP4, IQ4_NL and IQ4_XS matrices in place, decoding their blocks in the kernels exactly as the CPU dequantizer does, under every activation precision, in projections, embedding tables and expert mixtures (`lane_compile_lock`, `cuda_lane_lock`).
- **IQ4_NL and IQ4_XS checkpoints read.** `model::quant` dequantizes both bit for bit to ggml's
  `dequantize_row_iq4_nl` and `dequantize_row_iq4_xs` (`model_quant_lock` compiles the reference), and the decode
  engine reads their matrices on the array: each nibble's non-linear level as an excess code, IQ4_XS's 6-bit scale
  per 32 elements a unit multiplier.
- **Sinks, row biases and gpt-oss's mixture.** `attention … with sinks S`, `A plus B` with `B` one row over a block, and the `moe` sugar's per-weight biases and `using a swiglu clamped at L with slope A` run on every tier (`tensor_attention_sinks_lock`, `tensor_row_broadcast_lock`, `tensor_moe_options_lock`); the generation lane computes them, folds each projection's bias into its GEMV, and routes by an `f32` router.
- **DeepSeek-V2's latent attention, compiled from the checkpoint.** The tensor sugar gains a part of every head (`the first K of every N of X`, `the last K of every N of X`), joins (`A joined with B`, `across H heads`), a per-head product (`X matmul W across H heads`), attention whose values are narrower than its keys at a scale the program gives (`scaled by S`), and an expert mixture weighted by the softmax over every expert (`without renormalizing`), on every tier (`tensor_latent_attention_lock`, `tensor_moe_weighting_lock`). The model author writes multi-head latent attention absorbed — a cache row a position holding the normed latent and the shared rotated key, each head's query reaching the latent's space through its own rows of the expansion — with a low-rank query, shared experts and dense first layers, so DeepSeek-V2-Lite compiles from its GGUF into the generation lane with no code of its own and meets each activation precision's bar against llama.cpp over the 32-prompt corpus (`cuda_lane_lock`). The lane writes the cache row and each head's query in place from the projections that make them, and attends a row's span in chunks fixed by its length, a block a chunk in the decode step, so a cache's length is bounded by memory alone and a prompt continues alike token by token or in chunks.
- **Sliding windows and scaled tensors.** `attention … up to P within a window of W` (and the causal block and unbounded forms) keeps, for each query, its own position and the `W − 1` before it, and every tier loops over the window alone (`tensor_attention_window_lock`); `Let Y be X scaled by C` multiplies a tensor by a number on every tier and in the spatial IR (`tensor_scale_lock`, the probe corpus and backend matrix).
- **A norm names its epsilon.** `rmsnorm`, `layernorm`, `l2norm` and `gated delta` take `with epsilon E` after their operands (1e-6 when it is absent); every tier and accelerator emitter computes with it, and a spatial lowering refuses an epsilon it cannot carry (`tensor_norm_epsilon_lock`).
- **The CUDA tier's audit.** The CUDA forward refuses a statement it cannot lower by name instead of dropping it (`cuda_forward_refusal_lock`), every emitted program states truthfully whether it was compiled or written by hand (`cuda_provenance_lock`), and the fusion-parity ledger names each fusion llama.cpp, vLLM, SGLang, TensorRT-LLM and FlashInfer apply, pinned to their sources, beside ours (`fusion_parity_lock`, `cuda_fusion_census_lock`).
- **One proof of how a loop iterates, for every backend.** A `Repeat` over an Int range is a counted loop on the VM and the JIT (no list of the range is built), and one over a sequence nothing in the loop changes walks it in place on the VM and reads it through a borrow in Rust (`optimize::iteration`). A function over `Seq of Word32` runs natively, so SHA-256's compression runs on the JIT.
- **Infinitesimals, infinite integers and supertasks.** `Let dt be an infinitesimal.` declares the program's infinitesimal, with what is known of its reciprocal after `whose reciprocal is` — `even`, `odd`, `divisible by k`, `a multiple of k`, `congruent to r modulo k`, each optionally after `not`, joined by `and`; a contradiction, or facts that would split `⌈1/dt⌉` over more than 65,536 residue classes, is refused before the program runs. `x is infinitesimal`, `x is limited` and `x is infinitely close to y` (each with `not`) and `the standard part of x` read it. A loop whose clock steps by `dt` runs `⌈1/dt⌉` times and ends in its exact state: an update that is an affine map with roots of unity for eigenvalues ends in its closed form (a counter at `1/dt`, a running sum at `1/(2·dt^2) - 1/(2·dt)`, Thomson's lamp at `0 when ⌈1/dt⌉ is even, 1 when it is odd`), and any other update whose state is finite ends where the cycle it enters puts it; `At the limit, take the limit superior` (or `the limit inferior`, `the Cesàro mean`, `the limit`, `the standard part`) resolves what depends on the residue of `⌈1/dt⌉` for the supertasks after it in its block. `Repeat for every natural n:` searches every natural: decided exactly when its condition is linear in `n` with remainders (the first witness lies at a sign change or within one period after one), otherwise run on the first `K` naturals (`checking up to K cases`, ten million by default) and answered with its evidence, `false (provisional: checked 1,000 cases, 0 revisions; not proved)`, as is anything a function returns after an unsettled search. A supertask without an exact answer stops with its reason instead of running forever. The engine runs in the one seam every tier shares, and a program that declares no infinitesimal and searches no natural is handed on as parsed. `nonstandard_surface`, `supertask_strategies`, `tier_deadline_lock`.
- **Flows to exact solutions.** A loop stepped by `dt` that is an Euler step of a linear differential equation ends infinitely close to the equation's solution, and its standard part is that solution exactly: `Set x to x + x * dt.` until `t` reaches 1 leaves `e - (1/2)·e·dt + O(dt^2)`, whose standard part is `e`, and an oscillator run for two units of time stands at `cos(2)` and `-sin(2)`. `dt_flows`.
- **Nonlinear flows.** A loop stepped by `dt` that is an Euler step of a polynomial differential equation ends infinitely close to the solution, and its standard part is that solution, enclosed by validated Taylor integration so every printed digit is certified: `Set x to x + x * (1 - x) * dt.` from `1/2` until `t` reaches 1 leaves `0.731058578630004879251159241822`. A solution that leaves every bound before the loop's time ends unlimited. `dt_nonlinear_flows`.
- **Programs with infinitesimals compile to Rust.** `largo build` and `largo run` of a program using the infinitesimal, the hyperreals and infinite integers it generates, the exact number types or a supertask produce a native binary that prints what the interpreter prints, refuses in the interpreter's words, and stops where the interpreter stops on a condition that depends on the residue of `⌈1/dt⌉`. Every value the extension reaches lives in one compiled value model that keeps canonical forms, so a binding that holds an `Int` keeps integer division, big-integer promotion and its printing. Text joins an exact value as it prints (`Show "t = " + t.`), as it joins every other value.
- **Exact values by context.** Where a program declares `Rational`, `Algebraic`, `ExactReal` or `Hyperreal` — a binding, a parameter, a function's result, an assignment to such a variable, or an operation or a comparison with such an operand — `sqrt`, `exp` and `ln` are exact and `pi` (`π`) and `e` are the constants themselves: `Let r: Algebraic be sqrt(2).` is `√2` and `r * r` is `2`, `Let x: ExactReal be ln(exp(2)) + pi.` is `π + 2`. In an `Ordinal` context `omega` (`ω`) is the first infinite ordinal, so `omega + 1` is `ω + 1` and `1 + omega` is `ω`. Code that declares none of these keeps its meaning (`sqrt(2)` is a `Real`, `pi(digits)` the digits), and a function the program defines or a name it binds is never replaced. `exactSqrt`, `exactExp`, `exactLn`, `exactPi`, `exactE` and `exactOmega` name them directly. `exact_by_context`.
- **Exact contexts reach the infinitesimal and the exponent of `**`.** The program's infinitesimal and a standard part are exact operands, as a binding declared `Hyperreal` already was, so a quotient beside one is exact: `7 / 2 + dt` is `7/2 + dt`. An exact operand inside the exponent of `**` makes the exponent's own quotients exact, as it does anywhere else; an exponent with none still floors as an integer (`4 ** (3 / 2)` is `4`).
- **Both C backends run the exact extension.** A program the extension reaches (an infinitesimal, a declared exact type, a supertask's resolved loop, the exact constructors) compiles through `--emit c` and `--emit c-bytecode` and prints what the interpreter prints, failing in its words where it fails. Each of its values is a cell the collector owns around a handle into `logicaffeine-data`'s compiled value model, linked as one static library (the `logos_rt_value_*` C ABI, `liblogos_value_runtime.a`) built on demand from the same Rust the interpreter runs; the collector gives a handle back once its cell is unreached, and counts the bytes the runtime holds toward its next collection, so a loop that makes exact values runs in the memory it reaches. A list of exact values is one such value, and a value a program stores is a copy of its own, so changing a list never changes another's. A program the extension never reaches stays one self-contained C file. `largo build --emit c|c-bytecode` links the runtime, and `largo emit c -o` writes it beside the C with the command that links the two; the CUDA tier refuses such a program by name. A record field of an exact type now compiles on every backend (the checker had refused `a new Box with v 3` where the interpreter ran it). `exact_on_c_lock`, `largo_build_emit_c`.
- **Linked WebAssembly runs the exact extension.** A program the extension reaches compiles through `--emit wasm-linked` and prints what the interpreter prints, failing in its words where it fails: each of its values is a handle into the same `logos_rt_value_*` value runtime the C backends link, built for `wasm32` and linked into the module with the rest of the Rust runtime. A register that holds such a value holds every value as one, a value of another kind converted as it lands, so a counter that becomes exact, a parameter some call hands one, a loop over one and a record field declared one each compute through the runtime; a copy into a second place is a copy of the value, so changing a list never changes another's. A self-contained module (`--emit wasm`) has no runtime to hold one and refuses such a program by name, pointing at `wasm-linked`; a program the extension never reaches imports nothing of the runtime. The C bytecode walker, which refused them, now also reads an exact value as the index of a list of its own or as a range's bound (the `Int` it is, or the interpreter's refusal), and pushes onto a record's exact list in place, copying first what the record shares. `exact_on_wasm_lock`, `exact_on_c_lock`.
- **RISC-V runs the exact extension.** A program the extension reaches compiles through `largo emit riscv` and `largo build --emit riscv` and prints what the interpreter prints, failing in its words where it fails: each of its values is a cell of the shim's collector around a handle into the same `logos_rt_value_*` value runtime the C backends and linked WebAssembly link, built for `riscv64gc-unknown-linux-gnu` and linked statically beside the shim. Every operation over one is one call into the shim's adapter, each operand passed with its slot's shape, so a value of another kind is converted as it is read; which registers hold one is settled per exact web, the C walker's analysis lifted to `vm::tower::webs` and read by both. A parameter some call hands one, a global one is stored into and a field declared one hold one, and a value read as a machine integer (an index, a range bound) is narrowed with the interpreter's refusal. A program the extension never reaches links the shim alone. `largo emit riscv -o` writes the runtime beside the assembly as `liblogos_value_runtime_riscv64.a` with the command that links them; building it needs the target's standard library (`rustup target add riscv64gc-unknown-linux-gnu`). `exact_on_riscv_lock`.
- **A supertask whose state flows between jumps runs as the hybrid system it follows.** A loop whose clock steps by `dt`, whose body is Euler steps of a polynomial field and `If guard:` jumps (a ball that bounces, a thermostat that switches, a switch thrown at a set time), ends where the ideal hybrid system it follows ends, exactly: its flights are polynomials in time, each jump fires at the exact root where the flow crosses its guard, and a cycle of jumps that a scaling maps round to round is read in closed form, so a periodic run of a million seconds costs a few rounds and the infinitely many bounces of a Zeno tail end at the sum of their series, after which the ball rests while the jump fires again and again. A value the flow moves is infinitely close to what it shows; a count a jump keeps is exact. A guard the flow only touches, two jumps at once, a jump in the loop's last step, a condition comparing values infinitely close to each other, a count of infinitely many jumps, or more than a thousand jumps no scaling settles stops the program with its reason. The bouncing ball of Imamura's paper prints `0`, `0` and its Zeno time `4` on every tier. `supertask_hybrid`, `supertask_imamura_paper`.
- **A map or a tuple holding an exact value runs on every compiled backend.** A map stores an exact value under its key and keeps its own kind, and once it holds one it holds every value as one: the AST C backend holds `Map of Text to Hyperreal` in the generic `Map_str_<V>` family and `Map of Int to Hyperreal` in a new `Map_i64_<V>` one, the C walker and linked WebAssembly store and read them with a value of its own each way, and RISC-V keeps the map's value kind across later stores. The AST C backend had made the whole map an exact value (refused at generation) and failed to compile a tuple holding one (its typedef was never collected); the C walker and linked WebAssembly refused the store. A map keyed by an exact value is refused by name on the compiled backends. `exact_on_c_lock`, `exact_on_wasm_lock`, `exact_on_riscv_lock`.
- **Recursive functions over exact values compile.** Tail calls, accumulators, mutual recursion and doubly recursive functions over infinitesimals and the exact types run on AOT-Rust as on the interpreters, and a hyperreal like `1/(2·dt)` prints with its denominator in parentheses.
- **Infinitary quantifiers in Logic mode.** `Infinitely many primes are odd.` is `∃^∞x((Primes(x) ∧ Odd(x)))`, `Finitely many …` (or `Only finitely many …`) is `∃^<∞x`, and `All but finitely many dogs bark.` is `∀^∞x((Dogs(x) → …))`, in every position a determiner takes and in arithmetic sentences (`For all but finitely many n, n plus 0 equals n.`), printed in every output format. The proof engine reads them over the order of the naturals: `∃^∞x φ` is `∀b ∃x (x > b ∧ φ)`, `∀^∞x φ` is `∃b ∀x (x > b → φ)`, `∃^<∞x φ` is `∃b ∀x (x > b → ¬φ)`. `The light flashes infinitely often.` is `G(F(…))`, and `x is infinitely close to y` is the relation `InfinitelyClose(x, y)`. `logic_infinitary_quantifiers`.
- **The exact real tower.** `Algebraic` (real algebraic numbers, ordered exactly), `ExactReal` (computable reals: `π`, `e`, `exp`, `log`, `sin`, `cos` to any precision, equality decided on the algebraic, exponential-algebraic and π-polynomial forms and otherwise refused with `cannot separate … within 2^-k`), `Hyperreal`, `Hypernatural` and `Ordinal` (Cantor normal form below ε₀) are run-time values on the tree-walker and the VM, cross a task or a wire in a canonical encoding, and take `floor`, `ceil` and `round`. A binding may declare one (`Let t: Hyperreal be 0.`), and `/` in a declared `Algebraic`, `ExactReal` or `Hyperreal` is the field's division.
- **Several ordered infinitesimals.** `Let dx be an infinitesimal much smaller than dt.` declares `dx` smaller than every positive value made from `dt`, `dt^1000` among them, so the values of both are exact — `dx/dt`, `(dt + dx)/(dt - dx)`, their roots and rational powers, and `exp` and `ln` as series in `dx` whose coefficients are values of `dt` — and every comparison between them is decided. `much larger than` declares one above, and both bounds one between two already ordered; a bare `smaller than` or `larger than` decides too little and is refused before the program runs, and so is a value that combines infinitesimals whose order the program never states. `relative to dt` asks for the standard part, `is infinitesimal`, `is limited` and `is infinitely close to` from `dt`'s standpoint, the inner limit with `dt` held fixed (`the standard part of dt + dx relative to dt` is `dt`). A loop stepped by `dx` whose rates are values of `dt`, or by `dt` with rates of `dx`, ends in its exact state, and a supertask in each pass of another — an iterated limit over the same infinitesimal or a different one, nested to any depth, several in one pass, polynomial in what the passes change — is written as the map it is: `∫₀¹∫₀¹ (s + t) ds dt` over three infinitesimals is `1`. A value may depend on the residues of several reciprocals, as a dependence on the coarsest whose branches read the finer, and every limit rule reads them all. The integer part of a value infinite in several is an infinite integer of several on the classes its fraction needs — `⌊1/dt + 1/dx⌋` prints `1/dx + 1/dt` and its remainder by 2 reads both parities — and is refused by name where no residue decides that fraction, as in `⌊dt/dx⌋`; a supertask whose count is infinite in several, `⌈1/(dt·dx)⌉` steps of `dt·dx`, ends by its affine closed form, its cycle or `exp(N·log A)`. Bit-identical on every backend that holds the exact extension. `hyperreal_scales`, `supertask_strategies`.
- **The exponential and the logarithm of hyperreal numbers.** `exp(dt)` is `1 + dt + O(dt^2)` and `ln(1 + dt)` is `dt - (1/2)·dt^2 + O(dt^3)`: a function of a hyperreal is known by its series in `dt` (or in a root of `dt`) with exact coefficients, compared at the first order two series differ, and it takes roots and rational powers. A root of `dt` meets `π` as such a series (`π + √dt + O(dt)`), a flow's end state takes a root and a logarithm (`ln` of `(1 + dt)^(1/dt)` has standard part 1), and a flow may start from a root of `dt` or hold one in its rate. The exponential of an infinite value, beyond every power of `dt`, and the logarithm of a positive infinitesimal, which carries `ln(dt)`, are refused by name. Bit-identical on every backend that holds the exact extension. `hyperreal_functions`.
- **Roots of hyperreal numbers, exactly.** The hyperreals take roots: `sqrt(dt)` is `√dt`, an infinitesimal larger than every multiple of `dt`, and the square root of any positive hyperreal is exact — a real algebraic Puiseux series in `dt`, held by a polynomial it is a root of and the branch of its real roots it is. Field operations, order, equality, the standard part and the integer part are exact, and two routes to one value are equal. A value prints in one way: as a rational function of a root of `dt` when it is one (`1 + 2·√dt + dt`), as a radical when a power of it is (`√(1 + dt)`, `2·∛(1 + dt)`), and otherwise by its two leading terms and the order of the rest (`2√2 - (√2/16)·dt^2 + O(dt^3)`). A power to an exact exponent takes a root (`exactPower`): `2 ** half` is `√2` and `dt ** half` is `√dt`, `half` a `Rational` one half; a float written as the base keeps the float power. Bit-identical on every backend that holds the exact extension. `hyperreal_closure`.
- **Quadratic irrationals compute in their own field.** Two algebraic numbers of one quadratic field (`3√2 + 5√2`, `φ·φ`) add and multiply as `a + b·√d` rather than through a resultant: a loop of a thousand such sums runs in 70 ms where it took 11 s, on every tier.
- **A real number a function of the program defines.** `the real number whose digits in base k are f` and `the real number approximated by f` are `ExactReal`s that call `f` only as far as a comparison, a floor or a printed digit asks: `f(i)` is the digit worth `k^-i`, `f(n)` a rational within `1/2^n` of the number. A digit outside its base, or approximations no one number satisfies, stop the program naming the place; a function of another shape, one that reads a variable of the program and one that does more than answer are refused before the program runs. Every backend reads the function through its own call of it — a child interpreter on the tree-walker, a fresh machine on the VM, the compiled closure on the Rust tier, and on both C backends, RISC-V and linked WebAssembly a reader the value runtime calls back. Imamura's Proposition 3 and Corollary 4, one real holding a set of naturals that a loop reads members back from, run as he wrote them. A parallel block reads one on every tier; handed to a task or sent through a pipe — alone, in a list, or inside a hyperreal — it stops the program in the words a channel refuses a value with, since a task may run where the function is not. `defined_reals`, `supertask_imamura_paper`.
- **Imamura's Lemma 1 runs as he wrote it.** A loop that counts toward a value that can be infinite ends: its guard, `and`, `or` and `not` over comparisons of terms linear in the counter, is read where its truth can change, on each residue class of `⌈1/dt⌉` that point depends on. `While not ((n <= x and x < n + 1) or (0 - n <= x and x < 0 - n + 1)): Set n to n + 1.`, the floor function on the hyperreals, gives `1/dt` for `x = 1/dt + 1/2` and `1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) - 1/2 when it is odd` for `x = 1/(2·dt)`; a counter stepped by a constant toward an exact bound (`While n < x: Set n to n + 1.`) is counted the same way, and a loop that runs finitely often runs exactly as written. Proposition 6, the limit lemma, answers with its last guess and its evidence. `supertask_imamura_paper`.
- **A provisional answer carries one evidence.** A guess a function returned after its own search printed its evidence twice (`1 (provisional: …) (provisional: …)`), and a value made from two provisional answers held one inside the other. The evidence of a value made from answers is one now: settled when every answer is, a provisional answer's own beside a settled one, and otherwise the fewest cases any search checked with every revision any made. The wire refuses an answer encoded inside another.
- **A supertask inside a supertask.** The part of a pass that reads nothing the passes carry — an inner supertask over locals the pass binds afresh — is resolved once, ahead of the outer loop's end state, so the outer loop reaches the stage past every `ω·k`: a counter that counts to `1/dt` in each of `1/dt` passes ends at `1/dt^2`, and a pass whose value depends on the residue of `⌈1/dt⌉` is followed on each class (`0 when ⌈1/dt⌉ is even, 1/dt when it is odd`). Bit-identical on every backend. `supertask_strategies`.
- **A supertask whose body is a polynomial map ends exactly when the map's monomials close under it.** Such a map is the affine map of those monomials — `x ← x + t·t·dt, t ← t + dt` acts on `[x, t, t², 1]` — so `Σ t²·dt` over `⌈1/dt⌉` steps is `1/3 - (1/2)·dt + (1/6)·dt^2`, `Σ k²` is Faulhaber's cubic in `1/dt` and growth forced by `t²` ends at `2·e - 5`, where these printed the flow's certified digits. A map that reaches every power of its state, `x ← x + x·x·dt`, is still integrated as a flow. `supertask_strategies`.
- **A supertask that reads a value its pass computes runs after that value.** The part of a pass run once, ahead of the outer loop's end state, took an inner supertask that read a binding the pass leaves behind (`Let u be t * t`), so the program stopped with `Undefined variable: u`; such a supertask is now written as its map within the pass, or refused with the reason. `supertask_strategies`.
- **A loop that binds a name again keeps the two apart.** `Let` in a loop's pass binds a new name for the rest of the pass, while the guard and the statements before it read and change the program's own. The supertask engine read a pass by its names, so a pass that changed the program's `total` and then bound its own resolved the two as one (`1` for a total that ends at `1/dt`), and a pass that bound its clock again resolved as though it stepped the clock its guard reads (`1/dt` for a loop that never ends). Each such binding is now renamed apart before the engine reads the pass; a pass that binds its clock again, or binds a name in a form the renaming cannot follow, stops with the reason. `supertask_strategies`.
- **A supertask whose state grows quadratically is refused at once.** The cycle search follows a state to 2^16 bits (from 2^20): a step's work grows with the square of the state it multiplies, and a hyperreal state such as `x ← x + k·x²` took minutes to be refused where it now takes about a second, on every tier. A power of a root of a hyperreal past degree 4,096 is refused by name, its degree counted in that root; it panicked.

### Breaking changes

- `sealBytes`, `openBytes`, `sealFrame` and `openFrame` no longer compress: a frame sealed before this release opens to
  its compressed bytes, not its plaintext. `concurrency::pnp` frames carry a key slot and draw MAC keys from the top of
  each half (a frame of the old layout does not open), `PadLedgerStore` is one atomic `update`, `FileLedger` keeps a
  32-byte versioned record (an old 8-byte cursor file is refused), and `PnpSuite::with_recv_window` counts covers.
- `logicaffeine_system::mldsa::sign` returns `Result<Vec<u8>, SignError>`, `mlkem::encaps` and `mlkem::decaps` return
  `Result<_, KemError>`, and every LOGOS-facing kernel function (`ntt::mlkem_*`, `mlkem::mlkem_*_seq`,
  `mldsa::mldsa_*_seq`, `keccak::sha3_256`/`sha3_512`/`shake128`/`shake256`, `aead::chacha20_encrypt_seq`,
  `aead::poly1305_seq`) returns `Result<_, String>`. `concurrency::channel::pq_handshake_responder` and
  `pq_handshake_finish` return `Result<_, KemError>`. A LOGOS program that passes a crypto kernel or `crypto.lg` a
  value that is not a byte, a seed that is not 32 bytes, or a polynomial that is not 256 coefficients now stops with
  the reason; `byteEncode`/`byteDecode` take exactly one polynomial, and `compress`/`decompress` a width from 1 to 11.

- **New `##` header words turn some prose headings into code headers or typo errors.** In 0.10.1 an unknown `##` heading was literate prose. `Test`, `Component`, `Page`, `Route`, `Store`, `Directive` and `Redirect` are header words now: `## Test …` is always a test block, `## Component …` a component, `## Session` alone a session, and a header word without its shape (`## Page layout`, `## Store`, `## Route todos`) is a parse error. A heading whose first word is within two edits of any header word is refused as a probable typo, which now catches `## Tests`, `## Pages`, `## Routes`, `## Stores`, `## Components`, `## State`, `## Text …`, `## Next steps`, `## Best practices`, `## Rest`, `## Story` and `## Usage`; `## Error …` and `## Before …` headings without the LGX shape stay prose. `lgx_headers_lock::a_code_header_word_without_its_shape_is_loud`, `lgx_headers_lock::a_typo_of_a_new_header_is_loud_not_prose`, `lgx_headers_lock::prose_headings_that_share_a_first_word_stay_prose`. Migration: start the prose heading with a word three or more edits from every header word (`## Notes on pages`, `## About the tests`).
- **A binding lent to a `mutable` parameter may not be used again in the same call.** `h(a, a)` and `h(a, popLast(a))`, where `h` takes a `mutable` parameter, compiled in 0.10.1 and gave different answers on different tiers; they are refused where they are written, on every tier, naming the binding, the parameter it is lent to and the other use. `joined(a, popLast(a))`, which lends nothing to `joined`, is unchanged. `lend_exclusivity_lock`. Migration: evaluate the other use first (`Let last be popLast(a).` then `h(a, last).`), or pass `copy of a` as the other argument.
- **A function the program declares is called by its name, whatever word the name is.** In 0.10.1 the statement word won: `show(3).` ran the `Show` statement even when the program declared a function `show`. A declared function is now called by its name. `declared_function_call_statement_lock`. Migration: write the statement form (`Show 3.`) where the statement was meant, or rename the function.
- **The checker refuses programs 0.10.1 accepted.** A wrong arity or argument type, a call to a function defined nowhere (`Unknown function: <name>`, even in a branch that never runs), an unknown or mistyped construction field, a mistyped field or index assignment, a push of the wrong element type and a type nothing declares (`a new Todo` with no `Todo`) are refused at check or build time. A bare construction of a variant name two enums declare, which resolved in hash order, is refused, as are an `Inspect` whose arms name variants of two enums and a variant construction that omits a field. `type_checker_rejects_lock`, `unknown_call_lock`, `undeclared_type_refusal_lock`, `variant_resolution_lock`. Migration: fix the call, field or type; declare the type or `Import` it; name the variant's enum (`a new Failure from Outcome`); match one enum per `Inspect`; give every variant field.
- **Linked modules are scoped per file, and module loading is fail-closed.** A 0.10.1 module linked its dependencies from its opening paragraph (`[Geometry](file:./geo.md)`), and that form still works; a file that uses a name only a transitively linked module defines is now refused, and a module that resolves outside the project root, the `Largo.toml` root and the declared dependency roots, or cannot be canonicalized, is refused before it is read. `project_module_lock`, `multimodule_encapsulation`. Migration: link or `Import` the module directly, and declare out-of-tree code under `Largo.toml` `[dependencies]`.
- **Interpolation holes are read whole, and raw strings are verbatim.** A hole whose tokens did not all parse rendered its prefix in 0.10.1 (`"x{5 garbage here}y"` printed `x5y`, and `"{"a":1}"` printed `a`); it is refused, as is a width on a text literal. `r"…"` no longer interpolates: `r"{x}"` shows `{x}`, and `{{` stays doubled. `interpolation_hole_total_lock`. Migration: write literal braces as `{{` and `}}`, or use `r"…"`; use `"…"` where a raw string served as a template.
- **`the weekday of` answers ISO 8601: Monday is 1 and Sunday is 7.** It answered 0 = Sunday … 6 = Saturday on every tier, as did `LogosDate::weekday` and `LogosMoment::weekday` in compiled programs; only Sunday moves, from 0 to 7. `weekday_iso_convention_lock`. Migration: `(the weekday of d) mod 7` gives the old numbering.
- **More than 16 nested parentheses is `AstTooDeep`.** The parse recursion budget is an eighth of the AST depth limit (a quarter in 0.10.1, which sat at the measured stack cliff), and the logic-mode descent is guarded too, so a deep sentence is refused rather than overflowing the stack. `compiler_stack_safety_lock`. Migration: raise `LOGOS_MAX_AST_DEPTH`, or flatten the expression.
- **A compiled binary exits 1 with the LOGOS message on a runtime error.** A 0.10.1 binary printed a Rust panic report and exited 101. `semantic_probes_lock`. Migration: key scripts on a non-zero exit status.
- **AOT `text_from_bytes` refuses invalid UTF-8.** It returned U+FFFD where the tree-walker and the VM fail; a compiled program now fails with their sentence. `wasm_aot_lock`. Migration: validate first with `validUtf8`, or decode with `toValidUTF8Bytes`.
- **`Option` is a tagged value, `Word16` is its own width, and `args()` names the program.** `some x` and `none` evaluate to `Option` values on the tree-walker and the VM (0.10.1 used the bare payload and `nothing`), so `some 0` differs from `none`, `some none` is present, `a new Some with value 7` is `some 7`, and a bare record no longer satisfies a `Some` arm; `Show` and equality with `nothing` are unchanged. `word16(x)` is a `Word16` (a `Word32` in 0.10.1) that wraps at 16 bits, and comparing or dividing Words of different widths is an error. `args()` answers `["logos"]` on an in-process engine given no argument vector (0.10.1's interpreter answered `[]`). `option_tier_lock`, `word_narrow_all_tiers`. Migration: wrap a bare record with `some`; convert Word widths explicitly (`word32(intOfWord16(x))`); skip the first element of `args()`.
- **`largo emit c` output follows the interpreter.** A record shows as `Point { x: 10, y: 20 }` with its fields sorted (0.10.1: `Point(x: 10, y: 20)` in declaration order), a Real as its shortest round-trip decimal instead of `%g`, and maps and sets show; an absent map key, an out-of-range index and an unmatched `Inspect` fail with exit status 1 instead of answering `0` or `""` or reading past the buffer; a zero divisor fails with the interpreter's words instead of SIGFPE; more identifiers are mangled with a `logos_` prefix; and a construct C does not lower is an `#error` naming it instead of computing `0`. `struct_show_cross_tier`, `c_header_names_lock`. Migration: guard map reads with `contains`, and run a program C refuses on another tier.
- **A project with `pages/` or `static/` is a site.** If its entry never `Serve`s, a plain `largo build` exports `dist/` (deleting and rewriting it) instead of compiling the binary, and `largo check`, `largo fmt` and `largo test` read every `pages/<stem>.lg`, so a page that does not compile fails `largo check`. `project_shape_lock`. Migration: `largo run` still runs the entry, and `largo build --target <triple>`, `--lib`, `--emit` or `--native-functions` take the program path; or rename the directory.
- **`[dependencies]` are resolved, not ignored.** `check`, `build`, `run --interpret`, `doc`, `prove` and `verify` refuse a `git` dependency without a 40-hex `rev`, a `path` that does not exist and a table with none of `path`, `git` or `version`, and `largo add <name> --git <url>` still writes an entry with no `rev`. `local_deps_lock`. Migration: add `rev = "<40-hex commit>"` to the entry and vendor the checkout at `.logos-deps/<name>-<rev>/`, or set `path`.
- **The built binary moved to the project's target/<mode>/<name>.** 0.10.1 left it at target/<mode>/build/target/<mode>/<name>; the nested cargo artifact is now named `<name>-<digest>` and copied out. `build_identity_lock`. Migration: use the path `largo build` prints.
- **`largo verify`, `check`, `fmt --check`, `prove`, `test` and `run --interpret` refuse more.** `largo verify` and `largo build --verify` fail a program with a provable violation or an embedded refusal (they printed "Verification passed" unconditionally; see Fixed); `largo check` also fails on page errors, deploy refusals (`deploy-refusal`) and self-imports; `largo fmt --check` also flags unformatted `pages/` files and unsorted or duplicated import blocks; `largo prove` refuses a theorem that shares an axiom's name; `largo test`, which exited 2 as a reserved stub, runs tests and exits 1 when a project has none; and `largo run --interpret` refuses an import-scope violation and a `## Require`, and a failing run writes a `.replay` bundle under the project's target directory and prints two more stderr lines. `check_deploy_refusal_lock`, `pages_in_tooling_lock`. Migration: fix the program, run `largo fmt` once, rename the theorem, drop a `## Require` from an interpreted run, and keep the target directory in ignore lists (the scaffold's `.gitignore` already does).
- **Proof certificates from 0.10.1 are refused.** The prelude's version moved, so certificates produced by 0.10.1 are refused as incompatible rather than migrated, and a certificate with an unknown JSON field is refused. `certificate_environment`. Migration: re-run `largo prove --certificate`, or re-mint through `Certificate::new`.
- **Kernel integer division is Euclidean, `Prop` lives in `Type 0`, and instances resolve in Lean's order.** `div (-7) 3` is `-3` and `mod (-7) 3` is `2` (0.10.1: `-2` and `-1`), `div x 0` is `0` and `mod x 0` is `x` (they did not reduce), and `simp`'s constant folding and `decide` agree with both kernels; the type of `Prop`, and `Universe::Prop.succ()`, is `Type 0` (was `Type 1`); instance resolution tries higher priority first and, among equal priorities, the later declaration (0.10.1 took the first registered instance that resolved). `derived_integer_division`, `universe_levels`. Migration: restate a proof that computed a negative-operand quotient by truncation; compare an inferred sort semantically, not structurally; give the intended instance a higher priority with `add_instance_with_priority`.
- **Kernel registration refuses what it used to insert.** `Context::add_definition` checks its body and records an ill-typed one in `refused_declarations()`; a body naming a not-yet-declared global is a pending obligation invisible to lookup and reduction until the global exists; a name something outside its declaration unit depends on (the prelude's new `Option`, `Prod`, `List`, `Array`, `String`, `Char`, `Fin`, `HEq`, `Setoid` and `Quotient` included) can no longer be redeclared with a different meaning or extended with a constructor (`KernelError::StableMeaningViolation`); every checked definition also registers its `f_eq_def` and `f_eq_1 …` equation lemmas; and `TermParser::parse`, `parse_command` and `Repl::execute` refuse trailing input (`ParseError::TrailingInput`). `registration_soundness`, `stable_meaning`, `term_parser_total_lock`. Migration: register in dependency order, or call `Context::declare` for the `KernelError`; declare under a new name (an identical redeclaration is still accepted); send one command per `execute`, or a whole text through `Repl::execute_document`.
- **`linarith` no longer means `omega`.** `Proof: linarith.` reasons over the rationals, Fourier–Motzkin with a Farkas certificate, and declines a goal that needs integer tightening (`x < y, y < x + 1 ⊢ ⊥`), which is unsound over an ordered field. `omega_tactic`. Migration: write `omega` (or `arithmetic`, `integers`) for integer goals.
- **A theorem block is read as written.** A `Given:` below `Prove:` was silently left out of a 0.10.1 theorem's premises (`Premises: 0`), and two premises citable by one name were accepted; both are refused, whether the name is written twice (`Given (h):`) or collides with the positional `hp<n>` labels. `L_language_extra`. Migration: move every `Given:` above `Prove:` and give each premise its own label.
- **Seven `logicaffeine-proof` modules are gone.** `category_collapse`, `census`, `cofactor`, `res_width`, `orbit_stability`, `polycalc_gfp` and `polycalc_zm` are listed under Removed. Migration: pin `logicaffeine-proof = "=0.10.1"` for them.
- **Rust API: public enums gained variants, and some signatures changed.** None of these enums is `#[non_exhaustive]`, so an exhaustive `match` needs new arms: language `Stmt`, `Expr`, `TypeExpr`, `TokenType`, `BlockType`, `CalendarUnit` and `FieldType`; compile `LogosType` and `InferType` (whose `UserDefined` carries its type arguments), `TypeError`, `RuntimeValue`, `ListRepr`, `BuiltinId`, `vm::Op` (`Op::MagicDivU` also gains `divisor`), `vm::BoundaryType`, `vm::wasm::WasmLowerError` and `codegen_sva::sva_to_verify::BoundedExpr`; base `WordVal`; data `Value`; proof `InferenceRule`; forge `jit::MicroOp` (and `V_BINOP` gains a thirteenth family); runtime `RtPayload`; kernel `Term` (`Term::Annotated`), `KernelError`, `ParamKind`, `interface::Command` (`Command::Definition` gains `proof_goal`) and `interface::ParseError`; lsp `DefinitionKind` and `OwnedStmt`. Struct literals need new fields: `Stmt::RuntimeAssert` (`dim`, `reason`), `Stmt::SendMessage` (`unreliable`), `CompiledProgram` (`declared_types`, `observation_sites`), `TheoremCompileResult::discharged`, `TheoryCompileResult::axiom_names`, `analysis::Dependency::selection`, lsp `AnalysisResult` and `StdlibDoc::details`. Renamed or reshaped: `codegen::with_seeded_select` is `codegen::with_deterministic_replay`; `codegen_c::codegen_program_c` takes `policies: &PolicyRegistry`; `interpreter::needs_async` takes the interner; `ReCheckError::Unsupported` and `DoubleCheck::MainOnlyReCheckerIncomplete` carry an `Incomplete`; `arith::prove_int_eq` answers `None` unless `add`, `mul` and `sub` are the checked integer operations; `Flycheck::clear` returns `bool`; `CompileError::Parse`, `ParseError` and `HwError::ParseError` display the described diagnostic instead of a `Debug` dump; `Literal` compares `Float` by bit pattern and implements `Eq`; and `LogosMoment` displays as `YYYY-MM-DD HH:MM`. Migration: add the arms and fields (`..Default::default()` fills `CompiledProgram`), call `with_deterministic_replay`, pass `&PolicyRegistry::new()` or call `compile::compile_to_c`, read the `Incomplete`'s message and kind, build a proof context with `StandardLibrary::register`, and match on `ParseErrorKind::code()` rather than on error text.
- **The lexicon JSON moved into the lexicon crate.** The vocabulary database left the language crate's assets for `crates/logicaffeine_lexicon/assets/lexicon.json`, exported as `logicaffeine_lexicon::LEXICON_JSON`; its content is unchanged. Migration: read the new path or the constant.
- **Published tarballs carry no tests, examples or fixtures.** `cargo test` inside an unpacked `.crate` finds nothing, and a crate's examples (`cargo run --example solver_bench`) run from a git checkout. Migration: run tests and examples from a checkout (`cargo nextest run -p logicaffeine-tests --test <suite>`).
- **Web app links moved.** The Syntax Guide anchor `/guide#advanced-features` no longer exists (its content is under `#z3-verification`, `#assertions-trust` and `#modules`), and the Studio's Code examples moved into topic folders, so a shared `/studio?file=` link to examples/code/bubble-sort.logos now finds it at examples/code/algorithms/bubble-sort.logos. Migration: point shared links at the new anchors and paths.
- **`Assert that` is checked in every build of a compiled program.** AOT-Rust lowered it to `debug_assert!`, so a release binary ran past a false assertion that the tree-walker, the VM and a debug build stop at; it now stops there in every build, as `Require that` does. `phase50_security`. Migration: a check meant only for development belongs in a `## Test`.
- **`Pop` from an empty list stops the program.** In 0.10.1 it left the list as it was and bound `nothing` on the tree-walker, the VM and the C and RISC-V backends and a zero on the WASM tier, so a later step failed far from the pop or went on with a wrong value. Every tier now stops at the pop with one sentence naming the list as the program writes it, `Cannot pop from xs: it is empty` or `Cannot pop from b's items: it is empty`, after what the program printed. `pop_empty_list_lock`. Migration: test `length of xs` before popping.
- **`logicaffeine_system::temporal::LogosMoment::now()` is `logicaffeine_system::temporal::current_moment()`.** The temporal value types moved to `logicaffeine_data::temporal`, where the C value runtime reaches them; `logicaffeine-system` re-exports them at the same paths. Migration: call `current_moment()`.

### Changed

- **LOGOS is MIT, everywhere it ships.** `LICENSE.md` is the MIT License under Datacenter.Dev's copyright line, every
  manifest says MIT (the crates, the fuzz crates, the services, the VS Code extension and its lockfile), and every
  published crate now ships `LICENSE.md` in its package, as the extension's VSIX does. Verification takes no key (see
  Removed). The terms of service leave LOGOS, the site's own code included, to the MIT License where the two differ,
  and the footer says the site is released under it. The Caravel tape-out files carry MIT SPDX headers, with the
  scaffold's copyright line kept in `user_defines.v`. The README's license line names the code the tree carries from
  elsewhere under its authors' license: Berkeley SoftFloat, the RISC-V ISA tests, the Lean kernel arena's cases,
  OpenLane's flow scripts and two fonts. `license_lock`, `packaging_lock`, `legal_pages_lock`.
- **Every dual-clock FIFO of the D5005 shell says what its writer is.** `HOLDS`: the writer holds a word the FIFO
  refused until it is taken, and in simulation a refused word not offered again at the next step, unchanged, stops the
  run, naming the FIFO. `TOKENS`: the writer gives one-bit tokens as pulses whatever `wready` says, and the FIFO's write
  side counts those it has not taken (a channel's done FIFOs, and the host's write responses', which cannot wait and had
  only the FIFO's sixteen places). Neither: the doorbells' (a doorbell past a full FIFO is dropped and counted), the
  status records' (a progress record the host has not made room for gives way to a later one) and a hybrid channel's
  fast descriptors' (the channel multiplexer may offer another port's descriptor after one is refused, the refused one
  held at its port); every other FIFO's writer held its refused words through the shell's and the card port's
  simulations under the check. Synthesis never sees the check. `d5005_flow_lock` runs a token FIFO at five ratios of its
  clocks, every token taken where a plain FIFO given the same pulses loses some, and holds every FIFO the shell builds
  to its declaration.
- **No register of the D5005 engine clears asynchronously.** Stratix 10's Hyper-Retimer moves no register with an
  asynchronous clear, and the pipes between the engine's units (`logos_pipe`), there to be moved across the die, and its
  image streams cleared asynchronously. They clear with their clock, at the edges they did, their resets being registers
  of that clock, an image stream's queue of asked tiles kept in registers as before. `d5005_flow_lock` elaborates
  engines of every unit on the fast side, of contexts on head lanes, of token pairs and of the conv path, and finds no
  process triggered by more than its clock.
- **The D5005 array addresses its activation copies without an adder.** Each token-row of a core's activation copies
  sits at a power-of-two pitch of blocks, so a copy's read and write addresses are a token-row's place and a block's in
  bits of their own, `(t << NB_L) | hb`, where `t · NB_MAX + hb` put a multiplier and a carry chain between the step's
  next token-row and the copies' address registers: on hybrid9h's fit that adder was the user clock's critical chain in
  the Hyper-Retimer's report. The step's token-row, block and pass counter one on are registers beside them, so the
  step's end and its next token-row and block are compares and choices of registers and no carry chain lies in any
  copy's read address. `d5005_flow_lock`.
- **A NaN's wire bytes are canonical.** Every wire encoder writes a NaN as `0x7FF8_0000_0000_0000` and every other
  float with its exact bits, so no program can tell two NaNs apart through the wire, whatever host made them.
  Decoding is unchanged.
- **A D5005 build's compute side is constrained as the compute clock steps it.** Its constraints give every path between
  two of the shell's compute-side registers — the core's, the card port's and each dual-clock FIFO's compute side's
  (`ofs::compute_fifo_sides`, read from the shell's own connections) — the compute clock's user clocks; on hybrid9e's
  netlist the set holds 20,775 keepers and halves the shell's endpoints failing 5 ns. A lock holds every register it
  names to the user clock and the compute step, and every such register to the set.
- **The D5005 engine's broadcasts leave registers Quartus can copy beside their loads.** The column array reads each
  channel's copy of the activations at an address in a register of the copy's own; the engine's user-clock side takes
  its reset two registers on from the user clock's; a column core's word and first flags, the job's format copies, the
  channel multiplexer's holder and held write valid, the fast side's class flags and the vector side's widest registers
  ask for at most 256 loads a copy.
- **The Lean kernel decides structure η on a recursor's major before reducing the major, and is pinned to the Arena's
  new round.** Every `dag` engine expands the major of a structure's recursor into its constructor applied to its
  projections before it looks for the major's weak head normal form (for a structure never in `Prop`, and for a
  proposition whose own recursor eliminates into every universe, such as `And`), so a proof with no normal form is
  never reduced: the Lean Kernel Arena's `corner-cases/eta-struct-major-no-whnf` and `eta-struct-major-and-no-whnf`,
  past 300 s before, are decided in milliseconds; the official kernel, mathgraph and sokonanoda time out on both at
  the Arena's 60 s. Mathlib costs 577.25 G instructions on the Arena's engine against 576.75 G before. The entries,
  the vendored corpus and the scoreboard follow lean-kernel-arena `87512b60` (236 tests; `reachability`, rebuilt from
  its pinned Mathlib release, byte-identical to the Arena's), and the entries link as position-dependent executables,
  so the loader patches none of the checker's 1,451 table pointers at the start of every test. On that round the
  checker decides all 236 cases correctly, is the fastest checker on 219, and checks Mathlib in 577.15 G instructions
  against 776.43 G for the fastest other correct checker (`benchmarks/results/lean-kernel-arena/scoreboard.json`).
  `dag_struct_eta_major`, `arena_corpus_lock`, `arena_entry_lock`, `arena_scoreboard_lock`.
- **The D5005 vector unit's side runs a float matrix's job on a path of its own.** Each Gated-DeltaNet block's gate and
  decay projections are f32 matrices, a vector job a weight row a head: every (head, row) pass set its loads up and
  streamed both its weight row and the row's input before its Dot, ~1,000 clocks, so Qwen3.8-27B's 48-head projections
  took ~46 ms a token-row. The side now holds four rows' inputs and streams eight heads' weight rows a line of each in
  turn into 64 lanes, each lane adding a row's products in numerics::fdot's order, the trees after: ~4 ms a token-row.
  A line waits in a queue while its head's last term is still in the add, so a group of fewer heads than the add is
  deep, or lines the port returns back to back, still add each head's terms one after another.
  `d5005_engine_rtl_lock`.
- **A vectored D5005 engine whose memories flush lazily builds no vector unit or row engine in its core.** Every vector
  job runs on the vector unit's side and every lazily flushed token-row on the recurrence unit, so the core's vector
  unit (156k ALMs and 3,072 M20Ks in hybrid3b's fit) and its eager and lazy row engines (60k and 53k ALMs) were logic no
  state reached. The core now hands a recurrence row over from its header with only the hand-off's registers, and
  keeps neither the units nor their ports, pulses and states. `d5005_flow_lock`.
- **The D5005 engine computes the same from whatever its registers and memories power up holding.** A partial
  reconfiguration leaves every register no reset reaches, and every memory, holding what the bitstream gives it, not
  zeros. Qwen3.8-27B decoded the card's reference tokens on every run but the first after `fpgaconf`. That run read
  every vector job's stores out of one fixed row of the vector unit until the first lazily flushed recurrence job,
  because the core's read-port hand-off to the fold was a register only that job's states set and cleared. The first
  streamed matvec decoded its first blocks with power-up bits ORed in, because the column feed gathers each beat into a
  register by OR and only its fill count was reset. The vector unit's side left its line-gathering machine and its write request out of
  its reset, so a power-up value could hold its first job open on a read never asked for, or write a line of noise to
  memory. The hand-off is now a function of the core's state, and the gather register, the side's line machine and
  its write request clear with the reset, and so do the vector side's store-load and float-matrix landing valids: from
  noise they marked their landings full before any read, the side's first job took a word the store had not yet read,
  and a two-context decode's second context began on a wrong token. `scripts/d5005/run-cosim.sh` gains `LOGOS_SIM_SEED`,
  which builds and runs the co-simulation with every register and memory no reset reaches starting as noise of that
  seed, the card lock decodes on the plain, recurrent and vectored hybrid engines from two seeds of noise, and
  `d5005_flow_lock` walks every landing's valid back to registers that reset or are set every clock.
- **The D5005 accelerator makes no clock of its own.** A divided build ran its shell's compute side — the core, the card
  port and every FIFO's side toward them — on the user clock divided by a register, and a partial-reconfiguration
  region has no global clock network for a clock made inside it: the divided clock rode general routing to some 11,000
  registers and the router paid for its skew in delay added for hold: 70,067 ns on the first engine build, which failed
  routing on congestion, and 400,000 ns on a hybrid build. The divider now sits inside the shell and clocks nothing. Every register of the
  accelerator is on one of the platform's clocks, and the compute side steps in the user clock at `cce`, high in the
  user cycles that end with the divided clock's rising edges; each FIFO side steps at its own enable (`wce`, `rce`),
  its synchronizers too, so no register of the compute side moves but at the step. The constraints generate no clock;
  the engine's held values still have half a compute cycle less two into the compute side.
  `d5005_flow_lock`.
- **Bytecode C runs Takeuchi's function within 1.09x of AST C, down from 1.28x.** The bytecode→C walker checks a sum or
  difference with a constant by the overflow flag of the `add` or `sub` that computes it, the constant written in as an
  immediate, where it tested the other operand's range: that range ends within the constant of i64's ends, a bound no
  x86 compare takes as an immediate, so gcc held it in a callee-saved register for the whole function and spilled a live
  value around each of a recursion's calls. A product by a constant keeps the range test, so `3 * k + 1` stays one
  `lea`. Cycles on an idle core; `c_bytecode_perf_lock`'s ceiling is 1.30x.
- **The full-suite gate builds every dependency optimized.** `[profile.test-opt]` built only our own CPU-heavy libraries
  at opt-level 2, so a test that spends its time inside a dependency ran it unoptimized: `wasm_heap_soak_lock`'s million
  turns ran in `wasmi`'s unoptimized stack accessors for 1,515-1,800 s in the gate, once terminated at the 30-minute
  backstop, and now take 157 s alone. Every lock that races a dependency still wins (Z3, bincode). Overflow checks and
  debug assertions stay on. `dist_profile_lock`.
- **The D5005 card co-simulations run on demand, beside the silicon sims.** The eleven `d5005_engine_card_lock` tests
  that run the engine build under Verilator with its memory on the card's channels (a step, or the step program
  decoding the tiny model, including from power-up noise) take 600-3,400 s each alone and over an hour each in a full
  gate, where four reached their budgets. They leave `run-all-tests-fast.sh` for `scripts/run-silicon-tests.sh`; the
  card lock's fast tests stay in the gate. `ondemand_exclusion_lock` holds the two selectors to each other.
- **A full run records what every test cost.** The full nextest profile runs each test through
  `scripts/test/test-ledger.sh`, which runs it under GNU time and appends a line to the run's ledger beside its log:
  when it ran, its wall clock, its CPU with every process it waited for, its peak memory, and how it ended. The test's
  arguments, output, exit status and ending signal stay its own. `run-all-tests-fast.sh` prints a LOAD section from
  the ledger (`scripts/test/test-ledger-report.py`): the heaviest tests, the widest, the most crowded stretch and who
  crowded it, peak memory, and the tests a signal ended. In the last gate a single-threaded heavy ran 4 to 19 times
  slower than alone (a Z3 bit-blast 151 s alone, 1,227 s in the gate) beside tests that fan out inside, which nextest
  counts as one thread each; the ledger names them. Its first full run found 125 tests at least two cores wide, 15.5
  core-hours past one thread each. The full profile now declares the widest, the 28 that kept four or more cores busy
  over two minutes of CPU, as `threads-required` (`--widths` writes the block from a ledger). `test_ledger_lock`.
- **The D5005 vector side stores into the memory a beat a clock.** A store into the memory (a convolution's ring row,
  an attention block's cache row) took about seven clocks a beat: the beat read, its binary16 halves and int8 codes
  waited for, its line word made, then the next beat. The side now reads a beat a clock while the queue its beats wait
  in has room for every beat on its way, carries each beat's line, byte and lanes beside its halves and codes through
  the pipelined units, and gathers a beat a clock: the line takes the queue's head into an empty line or its own, and a
  full line goes to the write queue in the clock the next line's first beat comes. A store of n beats runs n clocks
  and a fill of about eight; the tiny model's convolutions went from 2,173 core cycles to the 1,940s. A pass reads
  the memory, and a job ends, only once the write queue is empty too. Every value stored, and where, is as before.
  `d5005_engine_rtl_lock` (`the_vector_side_stores_into_the_memory_a_beat_a_clock`: every memory-storing job of the
  step within its beats, an eighth of them and a fill a store).
- **The vector unit's RMSNorm holds 1/n as a constant.** `vector::rmsnorm` and `vector::gdn_out` load the reciprocal
  of the norm's width at the knob point's f32 unit with `Const`, where they loaded the width and took its reciprocal on
  a function lane every pass: the same value, one function instruction fewer a pass.
- **The D5005 200 MHz checks time every module of the fast side.** `d5005_flow_lock` boxed modules it never timed: the
  lanes, the rounding, the paired processing elements, the quantizers, the image stream, every f32 unit, the
  Gated-DeltaNet units and the vector unit. It now times each module of hybrid9's fast side at the parameters its
  instance gives it, followed up the hierarchy, in five checks by family (the fast side's control, the matvec units, the
  f32 units, the recurrence units, the vector unit), and a coverage check holds that every registered module is a stage
  or flattened into the nearest one. The attention unit at the 27B's widths is timed apart.
- **Certified unit propagation reads a clause without allocating.** The trusted RUP checker and the LRAT emitter
  collected each clause's unset literals into a fresh vector on every pass; one clause rule now answers a conflict, the
  unit it forces, or nothing, and stops at a clause's second distinct unset literal. Same verdicts; wide bit-vector
  equivalence certifies 15x faster (500 s to 32 s on `phase_hw_native_vs_z3`'s sixteen- and thirty-two-bit identities).
- **`Element`, `Event` and `View` are nominal types.** The parser used to rewrite the DOM handle types to `Int`, so `Let x: Element be 3.` type-checked and a `-> Element` function could `Return 0`. The three names are now distinct types in the static checker (`Let x: Element be 3.`, `Let e: Event be make()` where `make` returns an `Element`, and `Return 0` from a `-> Element` function are all rejected with the type names in the message) while their runtime representation stays the i64 handle every tier already moves — nothing about execution changes. Locked by `nominal_dom_types_lock`.
- **Datacenter.Dev holds the copyright.** Every legal-entity surface (the copyright line of `LICENSE.md` and of each copy, the web footer, the terms-of-service and privacy pages, and `llms.txt`) names Datacenter.Dev, which licenses LOGOS for itself and on behalf of its subsidiary Brahmastra Labs LLC. GitHub-org URLs (`github.com/Brahmastra-Labs/...`) are unchanged; they are functional addresses, not entity claims.
- **The roadmap tells the current story.** Its status dots sat 18.5 px beside the timeline (16.5 px on a phone) and 21 px above their titles, because the line and the dots were placed in two different containing blocks from two sets of numbers. Each card now draws its own dot and the stretch of line down to the next card from one set of CSS variables, so they meet at every width, and each stretch takes the colours of the statuses at its two ends instead of fixed percentages. Work in progress is marked with a hammer and planned work with a drafting compass in a dashed rim, in place of a clock and the AI sparkles glyph (`IconVariant::Clock`, which only the roadmap drew, is gone). The timeline gains one card, Full-Stack Web, with a typed route and the Rust it compiles to; the cards already there keep their words, and the Silicon card's 2.6× figure is now checked against `benchmarks/results/latest.json`. A Next release block opens the release history with every addition in `[Unreleased]`, baked from this CHANGELOG by the web crate's build script so the page never lags the work, and the page's structured data is generated from the cards instead of written by hand (it listed 9 items, one of them a card that did not exist). The timeline's list of nodes is pinned by a test, so adding one is always a decision. Locked by `crates/logicaffeine_tests/tests/roadmap_page.rs`, the `roadmap_data` and `changelog` unit tests, and a new browser suite for the site's pages, `scripts/web/test-site-e2e.sh` (Chrome and Firefox), which measures every dot against the line at four screen sizes.
- **The debugger's output is the Output column.** The drawer's green output pane repeated what the Output column shows, so it is gone: while the debugger is open, the Output column shows what the program has printed at the cursor — rewinding rewinds it — and closing the debugger gives the column back to the last Run. `Debugger::output` and `Debugger::error` read it from the recorded state without rebuilding a VM. Locked by `debug.rs` `output_and_error_read_what_the_snapshot_shows` and `the_debugger_output_is_what_the_program_printed`.
- **Hover cards are compact.** Their parts — a problem and its source, code, prose, rules — sit one 4 px gap apart inside 4 px padding, with no margins of their own. Locked by `the_hint_spaces_its_parts_evenly`.
- **Every Logic, Math and Hardware example reads cleanly.** Each opens with a titled comment saying what it shows (explanations moved out of Rust comments into the files), and each kind keeps one layout: Logic at the margin with a theorem as one block, Math without banner rules in 4-space steps, a spec one line per clause, Verilog one statement per line with spaced assignments, stored as raw multi-line strings. Every example still keeps its contract. Locked by `crates/logicaffeine_tests/tests/example_formats.rs`.
- **The Studio's file tree has room on the left, and its lines are one weight.** At a fractional zoom (90% in Firefox, 125% in Chrome) each 1 px line landed on a different fraction of a device pixel, so the lines read at different weights and the first looked heavier than the rest; folder branches were 2 px stubs. The tree's geometry is now one 18 px step (the user's choice, 2026-09-27: a branch as long as the gap between the lines), drawn by one tree component the Syntax Tree shares: its first line stands one step from the sidebar's edge, every level's line one step further in, and every branch runs exactly one step, from its parent's line to its own, 4 px before the row's first glyph, with a file's icon right after its branch. Every length snaps to whole device pixels and the tree pads its content onto the device-pixel grid wherever the layout puts it, so every line covers the same whole number of screen pixels (two at 80–150%, three at 200%). Locked by the web crate's `use_device_pixel_grid` unit tests and the browser scenarios `the_file_tree_reads_like_vs_code`, `the_file_tree_branches_connect` and `the_file_tree_lines_keep_one_weight_at_every_zoom`, run in Chrome and Firefox launched at real device scales.
- **Learn Logic is a roadmap.** `/learn` lays the curriculum out as a route of four eras, 18 modules and 77 lessons, and opens one lesson at a time: a short teaching, two examples and one question. Where a lesson teaches writing formulas the question is typed, with a button for every symbol the answer needs beside the keyboard; otherwise it is one multiple-choice question. A right answer, or the answer shown after a try, moves the learner to the next lesson, and any lesson opens from the roadmap. "More examples" and "More practice" hold the rest for anyone who wants it. Every formula a lesson shows and every typed key is what LOGOS prints for its sentence, and the practice carries one notation in symbols where 201 exercises printed raw LaTeX in 794 options. `learn_lessons`, `learn_symbol_pad`, `learn_page`, `learn_state_tests`, and the browser suite `scripts/web/test-learn-e2e.sh` in Chrome and Firefox.
- **The contact page replaces the pricing page.** `/contact` holds the install commands, a form that reaches the team and a four-question FAQ (LOGOS is free, it is under the MIT License, its source is on GitHub, and how to reach the team); no plan, price or commercial license appears on it. `/pricing` redirects to it, in the router and in `public/_redirects`, and the nav, the footer, the sitemap and the page meta call it Contact. `contact_page`.
- **The front page leads with agent instructions and a mini Studio.** Its heading reads "Code we can all read. Logic and mathematics built in. Bring your own agent." Copy agent instructions copies the onboarding the page shows a person and their coding agent, every command in it checked against the released largo's own help and this tree's CLI. Across the page, a LOGOS page sits in the Studio's editor beside its live preview, then a Logic, a Math and a Hardware example run by the Studio's runners. The preview's first paint is `largo build`'s, moved into `logicaffeine_compile::web_host::first_paint` with every built document unchanged, and `web_host::assemble_preview_painted` carries it into the frame.

- **Compiled programs are at least as fast as 0.10.1 again where 0.10.1 was faster.** An A/B of today's compiler against the published 0.10.1 and 0.10.0 compilers, same programs, same pinned cores, found AOT-Rust slower on spectral_norm, histogram and knapsack. A masked integer chain computes at the mask's width (`masknarrow`: a masked LCG in 32-bit wrapping arithmetic on AOT-Rust and both C backends), and an item access whose index is proven to count from the front reads through an accessor whose miss only fails, so spectral_norm's inner loop vectorizes its divides again (1.21x slower than 0.10.1, now 0.91x). A count kept in a sequence and a dynamic-programming row rolled from pass to pass add without a per-item overflow check under one comparison on the loop's limit, solved from how much the items can grow per pass (`optimize::item_growth`): histogram and knapsack. The range oracle bounds `&`, `|`, `^` and constant shifts. `mask_narrow_lock`, `item_growth_lock`, `optimization_guide_lock`.
- **A compiled program runs `Main` on the thread that started it.** `Main` needs a deep stack, and a compiled program got one by starting a 64 MiB thread and waiting for it; a second thread, even one only waiting, puts every `malloc` and `free` of the process on the C library's locked, atomic path. `Main` now runs on a 64 MiB stack mapped between two guard pages and switched to on the starting thread (`logicaffeine_system::stack`), and the process stays single-threaded: binary_trees at depth 20 runs in 3.03 s where it took 5.85 s, and no other benchmark moves. A run past the end of the stack writes out what the program had shown, reports `Stack overflow: maximum call depth exceeded`, the sentence every tier reports, and ends with exit status 1; it used to abort and lose the buffered output. `compiled_main_stack_lock`.
- **A program pays at startup only for the standard library it uses.** The build indexes the library once: each module's names and natives, the modules its code calls into, the lines its splice keeps and where each library function's section lies. Splicing a program, renaming the library around the program's own names, pruning it for C and deciding which of a program's functions are the library's read that index and copy only the modules the program names; no process re-reads the library's 950 KB to start. `largo run --interpret` of `Show 0.` retires 7 million instructions, where 0.10.0 retires 11 million. `cold_start_lock` holds what a start allocates to fractions of the library's size.
- **The range oracle counts a table's seed.** A counted loop that appends to a sequence learned the sequence's length only when the sequence was empty before it, so the coins and knapsack shape (`Push 1 to dp`, then `n` zeros) left `item (j + 1) of dp` checked in the hot loop. The loop adds its trips to whatever the sequence held, a lower bound that also holds when it runs zero times: coins runs 1.18x faster, 1.48x the speed of C. `oracle_seeded_build_length`.
- **`Repeat for t from a to b` compiles to a half-open range.** An inclusive Rust range carries an exhausted flag through every pass, and sha256's 64-round compression paid 40% more instructions than C for it. The loop walks `a..b + 1` wherever `b + 1` exists, a constant end below the largest Int or an end the range oracle bounds below it, and keeps the inclusive range otherwise: sha256 runs 1.22x faster, level with C. `repeat_range_lowering`.
- **A remainder of a value below twice its divisor is one subtraction.** For a constant `m > 0` and a value the range oracle bounds to `[0, 2m)`, `x % m` is `x - m` when `x >= m` and `x` otherwise, and a value below `m` is its own remainder (`modwrap`, on AOT-Rust and the C backend). A loop that carries a residue (`Set b to (a + b) % 1000000007`) has its remainder on the critical path: fib_iterative runs 4.3x faster, 4x the speed of C, and matrix_mult's inner loop vectorizes (3.2x); coins, prefix_sum, analytics and vm gain 3% to 66%. A sum of narrowed items now types as the Int it is, so the rule reaches a DP table's sum. `mod_wrap_lock`.
- **The bytecode tiers reduce such a remainder too.** The VM compiler takes the reduction from the oracle the AST emitters read: a value below its constant divisor is its own remainder, and one below twice it keeps its magic remainder marked `wrap`, which the VM and the C walker compute as a compare and a subtract while every other tier keeps the magic sequence. Through the C walker, `fib_iter_mod` runs as fast as through the AST C emitter (1.03x, from 3.33x). `mod_wrap_lock`, `c_bytecode_perf_lock`.
- **A two-armed assignment selects instead of branching.** `If c: Set x to a. Otherwise: Set x to b.`, with arms that cannot trap and at least one kept exact behind a guarded dual, computes both arms and lets `c` choose (`branchlessselect`, on AOT-Rust). Each arm's guard is tested on its own first, and the original branch, with its exact arithmetic, runs when one fails. collatz's parity step stops mispredicting: 1.26x faster, 1.35x the speed of C. `branchless_select_lock`.
- **A recursive enum boxes each variant that holds it, not each field.** `A Node with left Tree and right Tree` compiled to a box per field, so a tree of `n` leaves paid `2n - 2` allocations, a `Leaf` child included, and a pointer chase to reach every leaf. A variant that holds its enum now keeps all its fields in one box, and a variant with no fields is a plain value, the representation OCaml gives a datatype (`variantbox`, on AOT-Rust): one allocation per node. binary_trees runs 2.11x faster, 1.67x the speed of C. Values show, compare, copy and cross the wire exactly as before. `variant_box_lock`.
- **A merge's stores are proven in range.** The range oracle now carries the affine equalities among a function's integers and sequence lengths (Karr's domain): a push adds one to a length, a counted loop ends one past its bound, a merge advances `k` in step with `li` and `ri` so `k = li + ri - 1`, and a function that returns a sequence as long as its argument (proven over every `Return`) says so at its calls. From them it proves mergesort's `Set item k of result` in range, the one bounds check left in its merge loop: mergesort runs 1.18x faster, 1.09x the speed of C. A store the equalities do not bound keeps its check. `affine_equality_bounds_lock`.
- **A local `Seq of Bool` is one bit per item.** A sequence of flags one function uses only through item reads, item stores, pushes and its length compiles to a bit table (`bitpack`, on AOT-Rust), an eighth of the bytes of a `Vec<bool>`: the sieve's table of 10^8 flags is 12.5 MB where it was 100 MB, and the benchmark runs 2.8x faster, 2.7x the speed of C. A sequence used any other way keeps its bytes. `bit_pack_lock`.
- **A consumer of a tree applied to what a builder builds is one function that builds nothing.** `check(bottomUpTree(d))` built a tree of `2^(d+1) - 1` nodes to visit each once and count it. A function that only takes an enum apart, applied to what a function builds of it, now compiles to the builder's own recursion with each construction replaced by the consumer's arm for it (`foldfuse`, on the compiled tiers): no node is allocated, and binary_trees' fused check is a double recursion the compiler already computes in closed form. binary_trees at depth 20 runs in 0.6 ms where C takes 2.4 s, and the benchmark is now a disclosed collapse. A consumer or a builder that can fail, an arm that drops a subtree, a consumer that calls out and a tree used twice keep their trees, and every program prints what it printed. `fold_fusion_lock`.
- **A bubble sort sorts in `O(n log n)`.** A nest of passes of adjacent compare-exchanges over a sequence of Ints, with enough passes to sort its window, runs as one in-place heap sort of the window behind one check that the passes suffice and the window lies inside the sequence (`sortnest`, on the compiled tiers): any spelling of the exchange, ascending or descending, `While` or `Repeat` loops, an inner range that shrinks or stays whole. A sequence of Ints has one sorted arrangement, the one the nest leaves; a nest that sorts only part of its window keeps its loops, and one whose window runs off the sequence fails where it failed. bubble_sort at 30,000 items runs in 1.4 ms where C takes 1.28 s, and the benchmark is now a disclosed collapse, its note naming the heap sort. `sort_nest_lock`, `benchmark_collapse_lock`.
- **A search for a divisor up to each number's square root reads one sieve.** For every `i` of a counted range, `Let d be D0.` then `While d * d <= i` (or `d <= i / d`) of `If i % d equals 0: … Break.` and `Set d to d + 1` finds a divisor exactly when `i` has one from `D0` up to its root, which one sieve of the range marks before the loop (`divisorsieve`, on the compiled tiers), behind one check that the range ends by 2^28. Either comparison and either side of the test qualify, in a `While` or `Repeat` loop, in `Main` or a function; a search that reads its divisor afterwards, steps by more than one, stops short of the root or starts below 2 keeps its loop. primes to 3,000,000 runs in 5.8 ms where C takes 0.62 s, and the benchmark is now a disclosed collapse, its note naming the sieve. `divisor_sieve_lock`, `benchmark_collapse_lock`.
- **A pure iteration run from every start of a range stops where it meets a start already finished.** collatz walks each start down to 1 and counts its steps. What a walk adds from a value depends on the value alone, so the loop keeps each finished start's count in a table, and a walk that lands on a finished start adds its count and stops (`tracememo`, on the compiled tiers), behind one check that the range holds at most 2^24 starts. An iteration whose steps read anything that changes across starts, test the accumulator, show anything, carry a second accumulator or never shrink keeps its walk. collatz at 5,000,000 starts runs in 59 ms where C takes 0.84 s, and the benchmark is now a disclosed collapse. `trace_memo_lock`, `benchmark_collapse_lock`.
- **A map's membership test and the read of the same key under it are one lookup.** `If m contains k:` over a branch that reads `item k of m` asked the map for `k` twice, once for the test and once for the value. AOT-Rust now emits the test as `if let Some(v) = m.get(&k)` and each `item k of m` beneath it as `v` (`probefuse`). A branch that calls anything or can change `m` or `k`, a key that is any other expression, and a map of anything but `Int`, `Real` or `Bool` keep both lookups. collect retires 2.5% fewer instructions. `probe_fuse_lock`.
- **An Int map or set stays at most half full.** A linear-probing miss walks (1 + 1/(1 - α)²) / 2 slots, and in a table larger than the cache each is a cache miss; `Map of Int to Int` and the Int sets grew at three-quarters full and now grow at half, so two_sum's set lookups take 11% fewer cycles.
- **Mode B deterministic builds replay the interpreter's whole schedule.** A program built through `compile_to_rust_deterministic` or `compile_program_full_deterministic` (Mode B has no `largo` flag) changed only how a `Select` picked its winner in 0.10.1; it now runs every task, pipe, timer and select on `logicaffeine_system::replay`, a port of `logicaffeine_runtime`'s scheduler, with a launched task running before its parent resumes, every operation a scheduling point where the interpreter makes it one, and every choice drawn from one chooser seeded by `LOGOS_SEED`. A program that never touches the scheduler compiles as in the default mode, and a `Receive` from a closed, drained pipe stops the binary, since a compiled receive binds the pipe's element type. `replay_executor_port_lock` runs 18,000 generated programs on both schedulers and demands the same observations; `e2e_deterministic_replay` compares 24 built programs with the tree-walker and the VM at six seeds; `code_examples_all_tiers` builds the Studio's task-spawning examples this way.
- **The prelude follows a module's calls and renames what a program claims.** Auto-import covers 46 modules (`core` among them, so `Result` and `Option` arrive with no `Import`), selected to a fixpoint so a module brings the modules its bodies call, and spliced after the leading prose. "Declarer wins" is per name: a program's own `## To trim` is called at the program's sites while the rest of `strings.lg` still arrives and the library keeps calling its own renamed copy; a local value may share a library function's name (0.10.1 answered `Unknown function: split`); and the compiled tiers carry only the library functions a program reaches. Constructs whose lowering calls library functions the author never typed pull those functions in. `transitive_prelude_lock`, `user_name_shadows_stdlib_lock`, `stdlib_user_name_hygiene`.
- **SHA-3 and SHAKE are LOGOS source.** `sha3_256`, `sha3_512`, `shake128` and `shake256` were native bindings to a Rust Keccak; they are the FIPS-202 sponge over `keccakF` in `crypto.lg`, byte-identical, and every backend that compiles LOGOS runs them. `sha3_native`.
- **CRDT counters merge per replica.** `ConvergentCount` and `Tally` are per-replica state-based counters whose merge is idempotent, so a redelivered state never double-counts; `Let n: Tally be 5.` declares one, a counter shows and compares as its total, and a CRDT crosses a task boundary as its full state. `crdt_backend_lock`, `converge_crdt_lock`.
- **Kernel conversion follows Lean's algorithm.** Both kernels decide `a ≡ b` by lazy δ-reduction by definitional height: weak-head reduction without head δ, argument congruence first on equal heads, one δ step on the later-defined side, theorems last and opaque declarations never; K-like ι-reduction, unit and structure η, proof irrelevance and refutation of different constructors of closed data are conversion rules. Literal fast paths (the `Nat` order, arithmetic recurrences, `Nat.div` and `Nat.mod`, equality, in-bounds indexing) compute only after the kernel proves the program's defining equations over free variables, so a rebound constant loses the acceleration instead of computing a wrong value. `normalize` detects δ-cycles and has a 2^20-step budget (a fuel of 10,000 in 0.10.1), and `normalize_checked` says why it stopped. The independent re-checker gains its own lazy δ, open-context checking, universe polymorphism, K and quotients, and reports incompleteness as data; structural termination is a lexicographic measure over the inductive binders; and checking runs on stacks sized for its input. `k_recursor_conversion`, `lazy_theorem_delta_controls`, `lean_scale_nat_compare`, `recheck_differential`, `resource_stop_controls`, `G_recursor_extra`.
- **Proof automation proves more goals.** `omega` decides a ground `≤` or `<` by computation, a `≤` by chaining facts in scope and an `Int` equality through the ring oracle, and otherwise splits on `le_total` with no new axiom; the ring oracle keeps coefficients as big integers, decides closed goals at any literal width, and its proofs rest on no axiom now that the ring laws are proved in the prelude; order chains mix `=` and `≤`, `calc` E-matches its named `∀`-equality steps, and `formula` reads nested function terms. `omega_tactic`, `proof_order_axioms`, `symbolic_bigint_closed_proofs`, `arith_identity_certified`.
- **A `Proof:` line is a sketch.** In 0.10.1 a proof script that left goals open failed at its end; the obligations a `Proof:` line leaves are now discharged by the certified automation and reported (`Proved! [kernel-verified] (1 residual obligation discharged by automation)`), a step that fails is never swept, `Proof: Assume h.` over an implication certifies, and an inference-rule name written as a step is told it is a rule and shown `Proof: By <Rule>.`. `L_language_extra`.
- **Values compute faster.** `BigInt` multiplies by Karatsuba from 32 limbs and divides by Knuth's Algorithm D, the previous algorithms kept as the fuzz oracles each is checked against; `LogosInt` arithmetic borrows its big operands instead of cloning them; and appending to an unshared Text is in place. `perf_pass_bignum_inplace_lock`, `text_append_scaling_lock`.
- **The lexicon's word tables are binary-searched.** Every generated lookup is a sorted, duplicate-free key list where it was a linear `match`, differentially checked against the linear reference over more than 10,000 words, and `logicaffeine-language` reads the vocabulary from `logicaffeine-lexicon` through a build dependency. `word_table_lock`.
- **Errors read as sentences.** A rendered `ParseError` names its kind by its one-line headline instead of its `Debug` form, `CompileError::Parse` displays the explanation, and logic mode in `largo repl` explains a sentence it cannot parse with the Socratic message and a caret. `diagnostic_surfaces_lock::no_debug_dump_of_an_error_reaches_the_author`.
- **`logicaffeine-system`'s `full` feature includes `http`, and every native build links TLS.** `rustls` with the ring provider, `webpki-roots` and `tokio-rustls` are required native dependencies (none on `wasm32`), and only the Linux `io-uring` feature stays opt-in. `system_full_feature_lock`.
- **The language server no longer depends on the proof engine.** `logicaffeine-lsp` declared `logicaffeine-proof` and used none of it; a proof block's code lens runs `largo`.
- **The benchmark harness builds what it measures.** `benchmarks/sat/external-solvers.sh` fetches and builds every SAT competitor and proof checker at the pinned revisions `solvers.json` records, and a solver that fails to build stops the run. `benchmarks/run.sh` archives a run under `benchmarks/results/history/` only when the CHANGELOG's newest release is that version and `[Unreleased]` is empty, empties its binary directory and logs each build before measuring, stops naming every failed build, honors `SKIP_LANGS`, and takes `largo` only from `benchmarks/lib/largo.sh`. Every Zig port uses the truncated `@rem` that C's `%` computes. `solver_bench_provenance_lock`, `benchmark_metadata_lock`, `benchmark_fairness_lock`.
- **Every benchmark phase runs, and every published figure is drawn from the run.** `benchmarks/run.sh` runs each of its scripts through an interpreter, so no script's file mode can skip a phase (the interpreter-vs-V8 phase ran only while its script was executable). The README charts' label checks come from the registry, and the committed charts are exactly what `gen-readme-charts.py` draws from the committed results. `profile-benchmarks.sh` calls its assembler with the flags the assembler declares and replaces `latest-profile.json` only when the assembly succeeds. A raced SAT solver must decide a one-variable formula before it races, and CryptoMiniSat builds with a runpath relative to its binary, so moving the solver tree no longer leaves a column of errors. `benchmark_harness_fixture_lock`, `readme_benchmark_numbers_lock`, `benchmark_profile_assemble_lock`, `solver_bench_provenance_lock`.
- **Bit operations over arithmetic the compiler cannot bound run on machine words.** `available & (0 - available)` (nqueens' lowest set bit), a hash step `(h * 31 + x) & mask`, and every `&`, `|`, `xor` and shift over Int arithmetic whose range the Oracle cannot prove ran entirely in exact arithmetic. The compiled tiers compute such an operand with checked machine-word steps, one overflow check per step, and the exact form only where a step leaves the word, out of line so its big-integer code never crowds the loop: nqueens runs a third fewer instructions, level with 0.10.1 and 1.7x faster than C, and every value and every refusal is the reference's. The guarded form of an arithmetic chain moves its exact branch out of line too, needs no guard term for a leaf whose range the Oracle proves, and tests a guard two arms of a select share once. `bitwise_dual_lock`, `branchless_select_lock`.
- **The range analysis narrows a comparison by both sides' intervals.** A comparison bounded a name only when its other side was a constant; `j <= limit` with `limit` known to be at most 2^31 bounded nothing. A comparison that holds now caps a name on its smaller side by the other side's largest value and raises a name on its larger side to the other's smallest (an equality meets the two), so the version of the sieve's loop that holds under its guard steps its store index on machine words again. `abstract_interp` unit tests.
- **A loop whose bounds hint holds runs a copy that assumes it.** A loop reading `item i of xs` to a limit the compiler cannot compare with the list's length hoists the length and the hint's truth, and assumed the hint inside the loop behind that flag, which LLVM had to unswitch to read the loop unchecked; it did not for counting_sort's checksum loop. The innermost such loop is versioned on the flags (`hintversion`): the copy that assumes runs where every hint holds, the copy that assumes nothing everywhere else, so a loop that does index past the end fails where every other tier fails. A loop whose every access of its hinted lists the range oracle already proves keeps one copy, since both would be the same loop. counting_sort runs 14% fewer instructions, level with 0.10.1. `bounds_hint_version_lock`, `loop_split_knapsack`.
- **A sieve crosses off a small prime's multiples a word at a time.** A loop that stores one value into a bit-packed `Seq of Bool` at a fixed stride (`While j is at most limit: Set item (j + 1) of flags to true. Set j to j + i.`) set one bit per pass, and a stride shorter than a word came back to its word before the last `or` into it had landed, one store-to-load wait per bit. Where the range oracle proves every store in range and the stride at least 1, the loop is one call that writes each word once, the stride's bit pattern shifted to where the progression enters it (`stridefill`, on AOT-Rust), and the counter ends where the loop would leave it, by the loop's own last step. The call is out of line, so the loop around it carries no arithmetic of the progression: the sieve runs 1.6x faster, ahead of C in instructions and cycles. `bits_stride_fill_lock`.
- **Text grows by an Int's digits in place, and an Int map's probe runs inline.** `+ i` on Text wrote the digits into a buffer and copied them into the string through a library copy per number; it writes them straight into the string's spare room, two digits a division (strings: 12% fewer instructions, ahead of C). `LogosI64Map`, `LogosI64Set`, `LogosI32Map` and `LogosI32Set` take their inserts, lookups and membership tests inline and grow out of line, so the probe loop is the caller's (two_sum 23% fewer instructions, collect 6%).
- **An Int map or set slots a key by the top bits of its Fibonacci product.** `LogosI64Map`, `LogosI64Set`, `LogosI32Map` and `LogosI32Set` took the low bits after an xor-shift; the top bits of the key times 2^64/phi are the ones the multiply mixes from every bit of the key, in two instructions, the table the benchmarks hand-roll in C. collect and two_sum run 6–8% fewer instructions.
- **Every benchmark script builds and times a language the way the published run does, and every program does the work `main.c` does.** `suite_build` (`benchmarks/lib/suite.sh`) builds an implementation with the command `lib/languages.tsv` publishes and reuses a binary only when that command and toolchain built it; `run.sh`, `run-quick.sh`, `run-logos-vs-c.sh`, `calibrate-c.sh`, `run-logos-vs-interp.sh`, `verify.sh` and the stdlib parity gate build through it. `run-quick.sh` built C at `-O2`, Rust at `rustc -O` and Zig, Go and Nim without their host-CPU settings, and `run-logos-vs-c.sh` reused whatever C binary another script had left under its name as the `-O3` baseline; `run-logos-vs-c.sh` now caches a C baseline against the SHA-256 of the binary it timed, and `verify.sh` builds what it verifies rather than skipping what is missing. Every timing in every benchmark script runs pinned to the performance cores. collect and two_sum hand-roll the table `main.c` does (flat slots, Fibonacci hashing, linear probing) in every language, where C++, Rust, Zig, Go, Java, JavaScript, Nim, Python and Ruby used their own maps; strings formats numbers without `snprintf` in C or a `String` per number in Rust and counts spaces by byte in Rust and Go; JavaScript presizes its arrays and permutes in place; Java's nbody stores its bodies as C does; Nim builds with link-time optimization like every other compiled row; and a parity gate counterpart that fails to build stops the gate rather than dropping out of the comparison. The benchmark README states the flags each row builds with and what the LOGOS optimizer does beyond a literal translation, the compile-time labels state the full commands, every collapse note names the tiers its optimization fires on (fib memoizes on every tier, the C tiers included), and gcd declares its `O(n² log n)`. `benchmark_fairness_lock`, `benchmark_collapse_lock`, `benchmark_registry_lock`.
- **Licensor is now Datacenter.Dev.** Every legal-entity surface (the BSL parameters in both `LICENSE.md` copies, the web footer, the terms-of-service and privacy pages, and `llms.txt`) names Datacenter.Dev as the licensor; the legal pages reach it by email and are identical in both copies. GitHub-org URLs (`github.com/Brahmastra-Labs/...`) are unchanged. `legal_pages_lock`.
- **The repository root holds only the product.** The contributing and security policies live in `.github/`, the FAQ, versioning policy and challenge in `docs/project/`, mutation scope and coverage floors in `.config/`, and the workspace's members are `crates/*`; `.config/repo-shape.toml` defines a clean checkout. `repo_shape_lock`.
- **`scripts/` is organized by area.** The top of `scripts/` holds what people run (the suite runners, the release and packaging checks, the Tenstorrent ledger's three tools) and the FPGA and silicon tooling; every other script moved into the folder for its area (test, bench, accel, lean, wasm, web, release, dev), indexed in `scripts/README.md`, so its path changed. Records written before the move name a script at the top of `scripts/`; the file kept its name. `scripts_root_lock` resolves every path a script writes relative to its own location; `repo_shape_lock` holds the folders, the index and unique script names.
- **`scripts/generate-docs.sh` writes a handbook.** In 0.10.1 it printed a warning for every source path the workspace split had moved and wrote a source dump with broken Markdown; it now assembles the README's overview, Getting started, every guide, every crate README, the quick guide, the error catalog and the project documents into one book with a table of contents and links that resolve inside it (`--with-source` appends the core source files), and refuses by name an input that is missing. `generate_docs_paths_lock` runs it and holds the book whole and well formed.
- The WASM verification pass includes the word-copy regression suite in its generated test target list.

### Removed

- **The VS Code extension's license key.** `LOGOS: Set Verification License Key` and the prompt before Verify are gone;
  Verify runs `largo verify` with nothing added to its environment.
- **The verification license gate, the checkout page and the license validator.** LOGOS is MIT now, so `largo verify`
  and `largo build --verify` run Z3 for anyone with the full build: the `--license` flag, the `LOGOS_LICENSE` variable,
  `logicaffeine-verify`'s license module with its three license errors and its HTTP client, the `/success` page that
  turned a Stripe session into a key, the key the web app kept in local storage, and `services/license-validator` are
  gone. The docs, the agent pack, the crates page and `llms.txt` say "the full build" where they said Pro+ or
  license-gated.
- **`benchmarks/compare-vs-baseline.sh`.** It compared against the v0.9.17 snapshot with a fixed 2.5x floor and
  judged only LOGOS (Rust); `benchmarks/compare-runs.py` replaces it.
- **The web app's main-bundle size budget.** `scripts/web/wasm-size-gate.sh` failed every deploy build once the eager bundle passed 14 MiB — it carries the whole LOGOS engine while wasm-split is off, and grows with the language. The gate now reports the bundle's size and holds one budget, 60 MiB across all wasm artifacts.
- **The web app's theme picker.** The footer dropdown offered seven colourways but only the global chrome followed it — the Studio kept its own palette, so every choice but the default left the header and the IDE disagreeing. The picker, the choice it stored (`logicaffeine-theme` in `localStorage`, now ignored), the `data-theme` hook on the app root, the six icons only it drew, and the unused accent utility classes its stylesheet carried are gone. The palette — cyan and indigo on near-black, the former default — is now defined once, as tokens in the global stylesheet, so the site looks as it always did by default. Locked by `crates/logicaffeine_tests/tests/the_site_has_one_palette.rs` (no page renders a picker or a `data-theme`; every page defines each palette token exactly once) and the browser scenario `learn_page_ignores_a_stale_theme_choice`.
- **The Learn score and the profile page.** XP, levels, streaks, combos, achievements ("First Blood" among them), module unlocks, the sound effects and `/profile` are gone; `/profile` redirects to `/learn` (301). A progress blob saved by the scored page still loads, keeping its exercise history.

- **Seven research modules left `logicaffeine-proof`.** `category_collapse`, `census`, `cofactor`, `res_width`, `orbit_stability`, `polycalc_gfp` and `polycalc_zm`, published in 0.10.1 and reachable from no product code, are gone from the crate; `polycalc`'s `poly_mul_mono`, `close_perm_group`, `gf2_solve` and `apply_perm_to_mono` are public for code that builds on the GF(2) core. Migration: pin `logicaffeine-proof = "=0.10.1"`.
- **`LOGOS_BUG002_BAIL` and the native tier's cross-branch array-store stopgap.** The recursive in-place store shape the stopgap refused now tiers, correct under flow-sensitive list kinds, so `logicaffeine-jit` no longer reads the switch.

### Fixed

- **A D5005 channel reports every write it makes.** A channel's stream engine marks a write run done with a pulse as its
  last line leaves, into a done FIFO of four tokens back to the side that wrote it, and let the pulse fall when the FIFO
  looked full. Short runs end about as fast as the user clock takes their tokens at the clocks the card runs, 30 to 86
  MHz against the memories' 300, and the FIFO's far side hears of a token two of its steps late, so tokens were lost; a
  unit waiting for its last write (a convolution, a recurrence) then waited on a write the memory had made, and hybrid9h
  stalled at 60 MHz with one sequence and at 50 with eight. Each done FIFO counts the tokens it has not taken and takes
  them one at a time. `d5005_card_port_lock` runs a hybrid channel's writes from both sides at eight user clocks from 30
  to 200 MHz, every run reported done. `d5005_engine_card_lock` decodes over two contexts on hybrid9e's build with the
  user clock at the card's 50 MHz, the whole step at the ratio of the card's clocks.
- **The backward pass derives every clause of an attention.** Its rule scaled every score by `1/√hd` and attended the
  whole prefix whatever the attention said, so a scaled, windowed or sink-carrying attention's gradient was another
  function's, and values wider than the keys read past their rows. It now takes the attention's own scale, a sliding
  window or chunks, each head's sink (with the sink's own gradient), values of their own width and the soft cap on the
  scores, and the cap on a tensor's elements, each held to central differences (`tensor_graph_vjp_lock`).
- **A D5005 engine of contexts compares a ring load against its own context's rows.** The vector side read the region it
  compares a convolution's ring load against from context 0's rows table whatever context its job ran for, so a second
  context's ring load compared against another sequence's region and could skip the wait for the ring row its own
  sequence's previous row stored. It reads the table at its job's context, as every other read of it does.
  `d5005_flow_lock` holds every rows-table read of an engine of contexts to its job's context, and
  `d5005_engine_rtl_lock` runs two contexts' steps with every write landing 64 to 127 clocks late.
- **The D5005 conv path convolves behind the pipelined store as the machine does.** The path counted the beats of x on
  their way from the store as the landing's one bit, so behind the pipelined token-row store, three reads in flight, it
  asked past its queue and overwrote beats it had not yet taken: every convolution of a build with every unit on the
  fast side came out wrong. It counts every beat asked and not yet taken, whatever the store's read latency.
  `d5005_rtl_lock` runs the path behind stores of two to four clocks, `d5005_engine_rtl_lock` every convolution job
  behind the pipelined store.
- **The D5005 channel multiplexer no longer stalls on writes from several ports.** It gave a channel's write lock to
  the first port offering lines there, so a port could hold one channel for a later run's lines while its next line
  waited behind another port holding a second channel the same way (a mix of the card's ports hung on seeds 10, 11 and
  15). Each port's write lines now leave through one ordered queue, a channel's lock going to the port whose next line
  is the channel's; every fast port and the streaming unit's credits share one multiplexer, where one multiplexer
  nested in another lost the lines a port offered two channels at once; and the fast port's count of writes still out
  is defined from the first clock. `d5005_card_port_lock`, `d5005_flow_lock`, `d5005_engine_rtl_lock`.
- **The CUDA lane's projection that rotates its heads runs at its projection's speed.** Its epilogue chose the side it
  turns at run time from two by-value arguments, which put them in a local stack frame and every read of them in local
  memory: 41 µs a launch against the 11.5 µs of the projection and the rotation apart, and B=1 decode fell 15–20%
  (Gemma-3-1B 235 → 190 tokens a second, Qwen3-8B below llama.cpp). Each side is now its own call; Gemma-3-1B decodes
  at 228, Qwen3-8B at 40.1. The paired rotation (`lk_rope_parts`, DeepSeek-V2's) chose its side the same way and calls
  each side by itself too. `lane_compile_lock::the_rotation_kernels_keep_no_stack_frame`.
- **The certified-SAT ledger says what box and what build it ran on.** `benchmarks/results/solvers.json`'s metadata records
  the boot, the load average the run started under and the commit built (`-dirty` with uncommitted changes); re-run
  11-01. `proof_receipts_lock`.
- **A RISC-V program larger than a jump reaches links.** The stubs that stop a program (a zero divisor, a `nothing`
  read, code that never runs) sit after the whole program, and every check reached them with a branch or `j` the
  assembler widens no further than a `jal`'s ±1 MiB: a check early in a larger `main` could not be linked
  (`relocation truncated to fit: R_RISCV_JAL`), as Qwen3.5-0.8B's decode step at a 4096-token context could not. A
  check now jumps to its stub through `auipc`/`jalr` behind the inverted branch. `riscv_aot_lock`.
- **A clock pad's path to its global buffers is bound before routing.**
  `scripts/openxc7-patches/nextpnr-xc7-pad-clock-dedicated-route.patch`: in a congested image nextpnr-xilinx's router
  took a clock pad's net to its BUFGCTRL through an IO clock tile's regional-buffer mux that prjxray-db has no bits
  for, and the bitstream could not be assembled (the decision fabric's SoC routed on every seed and assembled on none).
  The clock pre-router binds an input buffer's net that feeds only global buffers over dedicated clock resources, as
  it binds a PLL's input. `xc7_image_flow_lock`.
- **A spatial join computes what its stored program computes.** A map folds into the map it follows only across an
  f32 page: across the SN50's bf16 pages the two stay two computes in the joined section, the page between them
  rounding as the program apart rounds, so the join is bit for bit the program with every result stored
  (`spatial_forward_lock`, `numerics_contract_lock`).
- **A loan something can see leaves `nothing` in the lent place on Rust, as on the VM and C.** A callee that reads
  the lent global, an argument after the lent one that names it, and a list, map or record whose item or field is
  lent see `nothing` while the call holds the value; a value-or-nothing is shown, compared, copied, passed, returned,
  joined into text and tested in a condition as the VM does, and every operation on one that holds `nothing` stops
  in the VM's words for that operation.
- **A copy a function makes of a list or map it was lent is its own on Rust.** The callee writes the caller's
  list in place, so a copy taken from it (`Let saved be xs`) no longer changes with the later writes; a write into an
  item of an item copies each shared level first, so a copy of the outer list made before or during a call keeps
  its items.
- **A slice of a program list a function reads compiles on Rust.**
- **A loan something can see leaves `nothing` in the lent place on C, as on the VM.** While a call holds what a
  `mutable` parameter was lent, the VM's place reads `nothing`: a callee that reads the lent global, an argument after
  the lent one that names it, and a list, map or record whose item or field is lent now see `nothing` on C too, and
  the parameter's value comes back when the call returns. A binding, parameter, result, field or list or map item a
  stored `nothing` reaches holds a value-or-nothing, and every operation on one that holds nothing fails in the VM's
  words for that operation. A loan nothing can see compiles as before.
- **An integer and a real compare on C as the numbers they are.** `is equal to`, `<`, `<=`, `>` and `>=` between an
  `Int` (a machine word or an exact integer past one) and a `Real` compare the mathematical values past 2^53, as the
  VM does: `9007199254740993 is equal to 9007199254740993.0` is false.
- **A function's names on C mean what the tree-walker resolves them to.** A `Let` inside a block of a function binds
  its name for that block only, so the program's binding of that name is read again after it; and a copy a function
  makes of one of the program's bindings stays its own when `## Main` later changes the binding in place.
- **A list of maps or of sets compiles to C, and two maps or two sets compare by their entries.** The C tier typed
  a list of maps or sets as a list of Ints, and `is equal to` between two maps or two sets did not build. A list of
  tables is a list of tables now, and two tables compare as the VM compares them: the same size, and every key of one
  found in the other with an equal value, whatever order the entries went in and whatever capacity either table grew
  to.
- **The accumulator-bound pass saturates its interval corners.** A loop accumulating a product or a sum of
  edge-of-range literals no longer overflows the optimizer; every accumulator stays guarded and prints what the VM
  prints.
- **A program compiles to the same bytecode in every process.** The type registry iterates its types in registration
  order and the abstract interpreter emits its hoisted guards in array order, so the bytecode's type tables and
  guards no longer follow a hash map's per-process seed.
- **Instantiating a quantifier terminates when the witness mentions the bound variable.** Induction whose step binds
  the goal's own variable (`∀n. P(n) → P(Succ n) ⊢ ∀n. P(n)`) overflowed the prover's stack: a binder was instantiated
  through the triangular substitution a unifier resolves, which re-reads its own replacement. Every binder
  instantiation in the tactics, the engine, crush, simp and the certifier is `unify::instantiate_expr` (once,
  capture-avoiding, a rebinding binder shadowing), and a cyclic substitution is refused naming its cycle.
- **Head lanes' guards are each held by a case that needs them.** A coordinator reusing a copy of the log before every
  lane is done with the row that wrote it is caught on rows of six batches a lane (as the 27B's); a flush bank loaded
  while it replays, under writes taken one clock in thirty-two; and rows replaying in twos around a row that replays
  nothing behind a far memory place each line in its own row's share.
- **A spatial program that joins a norm into the rotation after it computes the rotation.** `fuse_generic` folded the
  norm's map into one of the rotation's three reads of it (each element and its partner across the half of its head),
  deleted the norm's buffer, and left the other two reads on whatever buffer was renumbered into its place: on the
  RTX 3060, Wormhole and the SN50, a per-head norm's rotation came out wrong (0.124 for −0.0009). A consumer that reads
  an intermediate in more than one slot is no longer fused. Found by OPT-6's fused-equals-apart lock.
- **The older CUDA forward's fused kernels compute what its kernels compute apart, bit for bit.** A run of elementwise
  ops multiplied and then added in one kernel, and nvcc contracted the two into one fused multiply-add, an ulp from the
  run apart; a product the next step adds to is now rounded as its own kernel rounds it (`__fmul_rn`). And since it
  lowered the one IR the forward had added a sum into its norm before carrying it in a run, where it used to carry the
  run (`cuda_elementwise_fusion_lock`): it takes its runs first again, and adds into a norm only at the next stage.
- **A D5005 build keeps its shallow memories out of M20K blocks.** hybrid9c2's fit failed routing (109 of 1.92 M
  signals left after 3 h 37 min): its memories of 32 words or fewer held 5,142 M20K blocks, 44% of the device's, for
  2.2 Mbit, since Quartus Prime Pro builds a memory asking for nothing in a block for every 40 bits of its width at any
  depth; a column core's ring of two 5,280-bit beats alone took 132 blocks, so the blocks were 91% used and each core was
  spread over its 214. Every memory the engine declares now asks for the block RAM its depth fills (`rtl::ram`'s
  `LOGOS_RAMSTYLE`): MLABs to 32 words, M20K blocks beyond, and `logos_delay`'s ring is in MLABs to 32
  words whatever its width. A memory read without a register after it asks for MLABs checking no read-during-write
  (`LOGOS_RAMSTYLE_ASYNC`), which Quartus otherwise builds in flip-flops (a probe's 32 x 1,056-bit buffer: 33,797
  registers), and an engine's build sources `logos_settings.tcl`, which has the Timing Analyzer time an MLAB's write and
  a read of the same word, so a word read in the clock after its write is the one written. `scripts/d5005/ram_check.py`,
  a build gate, refuses a memory of 2 to 32 words asking for neither MLABs nor registers, and one in MLABs read without a
  register after it asking for no such check. `d5005_flow_lock` holds every build's shell to both; `d5005_rtl_lock`
  holds every memory `rtl::ram` builds and every delay to its depth's block RAM.
- **A folding D5005 build synthesizes.** The streaming unit's fold path declared `wire f_take, adv = ...`, one net
  with a value beside one without, which Quartus refuses ("all identifiers need to have initial value", error 17154)
  where iverilog, Verilator and slang take it: hybrid9c, the first folding build to reach synthesis, stopped there.
  `d5005_flow_lock` holds every build's RTL (the reference engine's, the hybrid's, hybrid9's and hybrid9's folding) to
  declaring a name with a value apart from names without.
- **A D5005 engine run is busy until every write it made is on the card.** The engine's busy was its sequencer's
  alone, so a run whose last job put lines into memory went idle with them still in the core's write queue; the shell
  then took its channels back, and the next run's first read waited behind them forever. The step program's profile
  mode, which runs a step's last position in pieces, hung so at layer 1's gate job, eleven writes of the convolution's
  ring left queued. The core's busy now holds until its write queue is empty (the card port takes a write once its
  channel has written it). `d5005_engine_rtl_lock`'s bench takes no write once a run is over, as the shell does, and
  runs the convolution's job alone on the reference and hybrid engines; the profile-mode card test is green again.
- **A flat D5005 build's IP generation no longer hangs the Quartus queue.** Three of `quartus.sh flat-syn`'s IP
  generations on 10-27 stopped at a Java generator asleep on a futex, writing nothing, where one takes nine minutes;
  another failed with its generator's connection to its own server on the host's loopback refused (the container runs
  on the host's network). The hang is a generator waiting with no timeout for the connection of a Tcl helper it
  started (`quartus_sh --ipc_sh`) that died of Quartus's start-up check: every thread of the generator parked, the
  helper gone, the whole generation asleep (twenty-two times on 10-27). `quartus.sh` now runs each generation under
  `scripts/d5005/stall_watch.sh`, which passes a command's output on and stops its whole process group once its
  processes take under thirty seconds of CPU and write nothing in three minutes (a hung generation's idle JVMs take
  about six and write nothing; a working one keeps cores busy or keeps logging) or at forty minutes in any case, so its
  loop of up to twenty attempts runs it again, as it runs one that fails. The synthesis that follows asks the IP
  catalog for its search paths through the same helpers and hung the same way (hybrid9c-syn4, forty-six minutes): it
  runs under the watch too, five minutes a window or four hours in all, and an attempt the watch stopped standing
  still goes again; one past its bound is a real failure. Its pass over the IP it finds generated logs each IP at about
  sixteen CPU-seconds in five minutes, which a watch of CPU alone took for a hang. `d5005_quartus_queue_lock`: a
  command standing still stopped at its window with no process left, one writing slowly left to finish with every
  line, one falling silent stopped, a working one left to finish with its own status, the bound.
- **A flat D5005 build restores the FIM's IP instead of generating it again.** Every flat build generated the FIM's IP
  in its own work tree, the flow's flakiest step: hybrid9c's generations on 10-27 died of the generator's refused
  loopback connection, of Quartus's start-up check, and hung twenty-two times. The IP
  sources are the FIM's, the same in every build. `scripts/d5005/ip_cache.sh` keeps what a generation writes outside
  the project directory, under a hash of the tree's IP sources (every `.ip`, `.qsys` and `_hw.tcl`) and the Quartus
  version, and `quartus.sh flat` and `flat-syn` restore it before they would generate, every file checked against the
  entry's manifest and an entry that does not check set aside; a generation's outputs are kept only once the design
  built from them has synthesized. The compile's own IP generation then finds every IP
  generated and runs no generator. A build whose IP sources differ generates and keeps its own (`QUARTUS_IP_CACHE`,
  default `/mnt/build/d5005-quartus/ip-cache`). `d5005_quartus_queue_lock`: the outputs kept and restored byte for
  byte, never the project's database or settings; the key; an entry that does not check; the flat flow's order and
  mounts.
- **ML-DSA-65 signs only a secret key FIPS 204's skDecode reads.** A short key panicked, a long one signed with its
  trailing bytes ignored, and an s₁ or s₂ coefficient outside [−η, η] signed anyway; each is now a `SignError`.
  KeyGen, Sign and Verify reduce each matrix–vector row (`reduce32`) before the inverse NTT, whose butterflies grow a
  coefficient 256-fold: a row of six products reaches 6(q − 1), and the unreduced sum overflowed 32 bits at that bound.
  The LOGOS ML-DSA in `crypto.lg` reduces its rows the same way (`mldsaReduceRow`).
- **ML-KEM-768 applies FIPS 203 §7's input checks.** `encaps` refuses an encapsulation key of the wrong length or with
  a 12-bit coefficient of q or more; `decaps` refuses a decapsulation key of the wrong length or whose H(ek) is not the
  hash of its ek, and a ciphertext of the wrong length. A short key or ciphertext had panicked, so a malformed peer
  `ek` or `ct` crashed the post-quantum channel's handshake, and `pq_handshake_responder`/`pq_handshake_finish` now
  return the `KemError`.
- **ML-KEM decapsulation no longer times its implicit rejection.** `decaps` compared the ciphertext with its
  re-encryption by an early-exit `==` and branched to K′ or K̄, the Fujisaki–Okamoto timing oracle; `mlkem768Decaps`
  in `crypto.lg` derived K̄ only on the reject path. Both now derive K′ and K̄ every time and choose by a mask over a
  full-length compare.
- **No division by q on a secret coefficient.** Compress, ByteDecode₁₂, the CBD samplers' reduction, the NTT outputs
  and ML-DSA's `freeze` divided by the modulus; wasm32 compiles such a `/` or `%` to `div`/`rem` at every opt-level,
  and x86-64 to `div`/`idiv` at opt-level 0 and `z`, whose latency follows the operand (the KyberSlash timing leak).
  Each is now a Barrett multiply or a masked add or subtract, checked exhaustively against its definition, and
  `pq_constant_time_lock` reads `mldsa.rs`, `mlkem.rs` and `ntt.rs` for any divide by a non-power-of-two runtime value
  and for a branch in the functions that reduce, compare or select secret data.
- **The post-quantum kernels take their arguments at their word.** The LOGOS-facing kernels reduced every item mod 256,
  so `[256]` signed, hashed and encapsulated as `[0]`; zero-padded a short seed or randomness to 32 bytes; zero-extended
  or truncated a polynomial of the wrong length; and panicked on others, which crashed the tree-walker and the VM. Each
  kernel now refuses a broken contract with its name and the reason, identically on every tier
  (`pq_kernel_contracts`): an encapsulation key that fails FIPS 203's checks makes `mlkemEncapsKem` return an empty
  sequence, a ciphertext of the wrong length makes `mlkemDecapsKem` return one, and `mldsaVerify` answers 0 for
  anything malformed. The FIPS-202 hashes, the ChaCha20 and Poly1305 wrappers and the LOGOS ML-KEM and ML-DSA in
  `crypto.lg` keep the same contract with the same reports (`pq_stdlib_contracts`), and so does the LOGOS AEAD:
  `chacha20Encrypt`, `poly1305Mac`, `aeadSeal` and `aeadOpen` had read the first 32 bytes of any key (a short one
  crashed on an index), took items that were not bytes, and let the block counter run past 32 bits. They take a
  32-byte key, a 12-byte nonce and a counter the data's blocks stay inside of; `aeadOpen` compares the tag without a
  branch per byte, and a sealed frame that holds a non-byte opens to nothing.
- **`pqAuthHandshake` signs under its context.** It took a `ctx` and never used it; both transcripts are now signed and
  verified under it. `mldsaVerifyLogos` refuses a signature with trailing bytes, which it had accepted.
- **A C-compiled program's `randomInt` and `randomFloat` are unpredictable.** The C tiers drew from SplitMix64, whose
  one word of state a single full-width draw gives back, so one output predicted every later one, a `randomSecret`
  session key included; the state was shared by every worker thread, and with no `/dev/urandom` it was seeded from
  the clock. The runtime now draws from ChaCha20 under a key from `getentropy`, one stream per thread, with fast key
  erasure, and stops when the system gives no entropy (`c_random_unpredictable_lock`, which pins the generator to
  RFC 8439's block function and its erased words to zero).
- **`sealBytes` and `sealFrame` no longer compress before they encrypt.** A frame as long as its compressed plaintext
  tells how compressible the plaintext was, and a frame that carries a secret beside data an attacker chooses gives
  the secret away a guess at a time (CRIME, BREACH): a 512-byte redundant plaintext sealed to 36 bytes and a random one
  to 529. A sealed frame is now its plaintext and its 16-byte tag. The four builtins take their keys, nonces, plaintexts
  and frames at their word, refusing an item that is not a byte rather than keeping its low eight bits.
- **SHA-256, HMAC-SHA256 and PBKDF2 take bytes.** `crypto.lg` packs four bytes into a word as b₀·2²⁴ + b₁·2¹⁶ + b₂·2⁸ + b₃,
  so an item past 255 carried into its neighbour and `[0, 256]` hashed as `[1, 0]`. `sha256`, `hmacSha256` and
  `pbkdf2Sha256` refuse a non-byte with their name and the item, and `pbkdf2Sha256` runs from 1 to 2³² − 1 iterations
  on every tier: the tree-walker and the VM accelerate it natively and had truncated the count to 32 bits, so 2³²
  iterations ran none, and a count of 0 ran one (`sha2_stdlib_contracts`).
- **Session signatures and CSRF tokens are checked byte for byte.** `verifySession` and `csrfValid` compared the MAC a
  visitor sent with text equality, which stops at the first byte that differs; they now use `sameTextInConstantTime`,
  whose steps do not depend on where the texts differ (`session_constant_time_lock`).
- **The package registry checks the OAuth state.** `/auth/callback` set an `oauth_state` cookie on the way to GitHub and
  never compared it on the way back, so a callback an attacker started signed the victim in to the attacker's account.
  It now refuses a callback whose state is missing or is not the cookie's, and spends the state once it is used. A
  verified sign-in hands its session token to the site in the URL's fragment, which a browser never sends to a server,
  where it had put it in the query string, and so in access logs, synced history and the landing page's `Referer`.
  The registry's tests apply its D1 migrations, so its package routes are tested against the schema they run on (three
  had failed on a missing table), and a sign-in runs end to end against a mocked GitHub.
- **A finished `--max-steps` run reports the steps it took.** The VM charged a fueled run's fuel only when it ran out,
  so `largo run --max-steps … --json` and the MCP step budget reported `steps_used: 0` for any program that finished
  on the VM. Each slice now charges what it ran: a budget of exactly the steps reported finishes and one less runs out
  (`fuel_budget`, `run_limits`).
- **A lifetime license is a Lifetime purchase.** The license service granted `lifetime` to any succeeded Stripe
  PaymentIntent, whatever it paid for; it now reads the plan from the price of the Checkout Session that took the
  payment, as it does for a subscription. `logicaffeine_verify` accepts the `pi_` key a lifetime buyer is given, which
  it had refused for its format.
- **Rainbow and Ding's exchange are marked for study.** `crypto.lg`'s Rainbow signatures (broken: Beullens, 2022,
  recovers a level-1 secret key from its public key in a weekend) and Ding's ring-LWE exchange (a reused secret leaks
  through its signal) say so where they are defined and in the guide.
- **The fabric board top measures its own settle.** `nexys_fabric_soc` streamed `localparam CYCLES = 16'd1` and read
  its results from the combinational array thousands of clocks after the inputs settled, so "one cycle" was never
  measured. It now primes and launches each vector, captures the array's output `L` clocks later for every `L` in its
  window under two primes, and streams the smallest `L` from which every capture holds the settled result (0 when it
  never settles); `fabric_perf_lock` proves under iverilog that a fabric slowed by 15, 35 and 95 ns reports 2, 4 and
  10 clocks. `scripts/fabric-host.py` checks the measured cycles and writes a JSON receipt.
- **The D5005 vector side's held float-matrix inputs and a head lane's copy of the log are block RAM.** Each was a
  memory declared inside a generate block, and Quartus built the vector side's four (1088 x 512 bits each at the 27B's
  widths) in 2.2 Mbit of flip-flops behind 1088-way read multiplexers, reporting nowhere that it had: hybrid7's flat image
  needed 2,120,891 ALMs of the device's 933,120. Both are now `logos_bram_w512` instances, read and written as before,
  which Quartus builds in M20K blocks. hybrid8's synthesis named 33 more memories Quartus built in registers for their
  asynchronous reads; the large ones now read as block RAM reads, each word straight into a register: the fold of
  several heads' banks, query, gates and log decays (about 85k registers a head lane), and the recurrence side's and a
  head lane's line queues, the line landing the clock it heads the queue registered beside the queue's word (about 65k
  a lane); a head lane copies a fold's out a clock after its address and starts its rows once the last beat is in. A build's gates now include `scripts/d5005/ram_check.py`: every memory of 64 Kbit
  or more in the elaborated shell is a block-RAM wrapper's or declared at its module's own scope, a large memory in a
  generate block refused by name; `record_afu_image.py` records no image whose RAM gate did not pass. `d5005_flow_lock`
  (the gate on its forms, the card's hybrid8 shape at the 27B's widths, the recorder).
- **The D5005 engine's delays of three clocks or more are block RAM, and a column core keeps one copy of its beat.**
  Quartus Prime Pro replaced no register chain with a RAM in hybrid8's synthesis, so every delay line was built as
  written: a column core's lane fields alone were 1,536 bits five clocks deep, 7,680 flip-flops in each of sixteen
  cores. `logos_delay` is now a chain of registers for one or two clocks and from three a ring of the next power of two
  of its depth in words, written a word a clock and read the depth less one behind, its only flip-flops the ring's write
  address: in M20K blocks from 40 bits wide (a block's widest word) and in MLABs below. Under iverilog the ring
  simulates 4x faster than the chain. The column core's lane fields and the pass's unit sums go through it; the
  systolic elements' activations are staggered once a column (`logos_column_act_stagger`, block k of each eight
  codes k clocks late) where every core delayed its own, and a block's four 4-bit digits are delayed in place of its two
  packed 19-bit operands and two activation bytes; the elements take a pass's codes from the beat held, registered a
  clock before its activations, so the core's second copy of the beat is gone, its lane fields from the beat a clock
  longer in their delay. Every latency is as it was. At hybrid8's shape a column core comes to about 20k registers
  where it was 39,212. `d5005_rtl_lock` (`logos_delay` at fifteen depths and five widths: every word back its depth in
  clocks later; one or two clocks in registers, three or more one memory read on the clock of the next power of two of
  the depth with only its write address in flip-flops, M20K or MLAB by width).
- **A D5005 channel's feed queues one decoded beat a core when it deals to two cores or more.** A pair's beat lands on
  its core no sooner than 2N blocks after the core's last, with one beat at most decoded on its way, so a second place
  never filled while its core took a beat a clock; a lone core, whose beats can come two blocks apart, keeps two. The
  queues are flip-flops (Quartus builds a memory read at its cores' constant addresses in registers), 5,280 a place at
  hybrid8's set: about 84k fewer across its sixteen cores. The stream rate lock's law holds as before at two and three
  pairs. `d5005_rtl_lock` (the places a feed's queue holds at one, two and three cores, counted as declared).
- **A D5005 column can take an octet's blocks four lanes a row a clock.** A Q8_0 octet left the issue's slots for the
  row's lane one block a clock, so its eight blocks cost the lanes eight clocks against the processing elements' two.
  Q8_0 and Q4_0 are 6.6% of the 27B's weights and about a quarter of its array time. `ColumnSet::with_octet_lanes`
  (`d5005_afu --octet-lanes`, `build-afu.sh --octet-lanes`) gives each row its full lane and lanes of one scale
  (`logos_lane_slim_p`: no unit multiplies, one product a block, the full lane's latency). A token-row's next blocks
  of its group go to them at once, and their values are added into the row's sum in block order through a chain of
  adds, each value delayed to meet its add. A sum's next group waits out the chain (`column_array_octet_lane_clocks`);
  a K-quant's jobs run as they did. `d5005_rtl_lock` (the slim lane equal to the lane model on every one-scale
  32-element format in both word counts; octet jobs on two and four lanes equal to `numerics::dot`; their exact
  clocks).
- **A D5005 one-word lane takes its units' products two to a DSP block.** Each of a lane's sixteen units multiplied its
  signed sum by its sub-block multiplier and its activation sum by its offset multiplier, each product a DSP block of
  its own: 34 blocks a lane, 68 of a column core's 198 in hybrid7b's synthesis. Built for one activation word a unit's
  signed sum is within 19 bits (every format's offset code within ±128, every one-word activation code within ±127: at
  most 260,096 in magnitude), so the lane's first stage sums a pair of units' products as one block's two 18×19
  multipliers, and its adds of the units' products take half the terms. Every value and latency unchanged.
  `d5005_rtl_lock` (every format's blocks of every byte pattern under activations all 127, all −127, a sign a unit and
  seeded, through the one-word lane, equal to the lane model).
- **A D5005 block-RAM memory whose depth is not a power of two layers is built as stacks that are.** The fitter builds
  a memory's depth as a stack of a power of two layers of 512 words. So the vector unit's registers at the 27B's widths
  (sixteen, three copies each, of 1,088 rows of 512 bits) took 2,048 rows' blocks, 64 a copy, and a token-row store bank
  of 57,344 beats took 65,536's. `rtl::ram`'s registered and held memories (`logos_bram_w*`, `logos_bram_re_w*`) 40 bits
  wide or wider, more than one layer deep in a number of layers that is no power of two, are now the stacks their number
  of layers names: the deepest the memory's own from address 0 and each other a `<name>_part` beside it, each memory at
  its own module's scope (as `ram_check.py` requires), the read the stack its address named. A register copy takes three
  layers' blocks where it took four, and a store bank 112 where it took 128. A memory narrower than 40 bits, of one
  layer or of a power of two layers is one memory as it was, and the falling-edge and enabled memories (the reference
  engine's and the core's) are one memory at every depth, their text as it was. `d5005_rtl_lock` (every form's memory
  at fourteen widths and depths under yosys, the stacks it is or the one memory, and `ram_check.py` passing it; both
  stacked forms reading and writing as the one memory of their depth over seeded runs across every stack's edges).
- **A D5005 column array can share each set of processing elements between two cores at twice their clock.** An array's
  cores run in lockstep against one activation word. `ColumnSet::with_pe_pairs` therefore gives each pair of cores one
  set of processing elements (`logos_matvec_pass_x2`) in a clock of twice the cores' (the array's `clk2`): core a's
  codes are taken with the word between the cores' edges and core b's at the next edge, and each core
  (`logos_column_core_bare_<set>`) gets its partials as its own elements gave them. The array's processing-element DSP
  blocks halve (128 of a core's 198) for the same passes, and every core's clocks are as they were
  (`column_array_clocks`, the octet schedules). `d5005_afu --pe-pairs` (`build-afu.sh --pe-pairs`) builds an AFU of
  paired cores. Its user side runs in the PIM's `uClk_usrDiv2` and the pairs in `uClk_usr`, twice it: one PLL's two
  outputs, their rising edges together. The AFU's JSON asks for the high clock at twice the low, and the clock map names
  `uclk2` and the pairs. `cdc_check.py` holds each pair inside the user side's box by its `clk`, and `cdc_check.py pair`
  proves a pair leaves `clk2` only through `clk`'s registers (gate `cdc_pair.txt`). Programs are the same as a build of
  lone cores takes. `d5005_rtl_lock` (the pair against two lone passes over seeded clocks at 1, 2 and 16 units; the
  paired array against `numerics::dot` in both word counts, a K-quant of one and two code digits, 32-element blocks and
  Q8_0 octets on four lanes, pairs within and across channels; its clocks the lone array's), `d5005_flow_lock` (the
  AFU's two clocks, its JSON, shell, map and cosimulation, a build without pairs as it was; the clock gate and the pair
  gate; PIM elaboration), `d5005_engine_rtl_lock` (every matvec job on paired cores the machine's),
  `d5005_engine_card_lock` (six sequences decoded at once on a paired AFU as the machine decodes each).
- **A D5005 vector unit's pair copies hold a head, not a whole register.** The user clock's vector unit keeps three
  block-RAM copies of each register: copy A for the row at hand, copies B and C for the rows a far Hadamard stage pairs
  a row with and the rows a rotary block reads. All three were as deep as the longest vector, 1,088 rows at the 27B's
  widths and 64 M20K blocks a copy. A Hadamard or rotary instruction pairs rows of its own operand, and the machine's
  Hadamard and rotary jobs take a head at a time. So copies B and C now hold the first `EngineParams::pair_rows` rows,
  the longest operand any Hadamard or rotary job loads: 16 at the 27B's 256-wide heads, one layer of blocks a copy.
  Writes past it are dropped. `d5005_rtl_lock` (every micro-program, rotary, cache-row, convolution and greedy program
  on the unit with pair copies as deep as the cases' widest pair operand, each register the interpreter's),
  `d5005_flow_lock` (the 0.8B engine's pair copies a head deep and given to its vector side).
- **The D5005 engine's numerics name the multiplier their products come from.** `numerics::FpUnit` is `Soft`, the
  engine's own multiply, or `Hard`, the card's floating-point DSP block as fp-oracle6's receipt names it
  (`FpUnit::CARD`: tininess decided after rounding to 24 bits with the exponent unbounded, the NaN signed by the
  operands' signs' exclusive or), which flushes a product just under the least normal that the engine rounds up into it.
  `EngineNumerics.fp_unit` makes it a knob point. Every function whose result takes a product (the transcendentals, the
  dots and trees' products, the norms, the rotations, softmax, both attentions, the Gated-DeltaNet steps, the int8 codes
  and cache rows) takes the unit, as do the vector unit's interpreter, the model and the machine. Every `Soft` point is
  the arithmetic it was: the tiny qwen35's logits at all 80 knob points in three formats digest bit for bit as before.
  `d5005_numerics_lock` (the card's member is its receipt's; the two units part only in that band, `(1 − 2^-24)·2^-126`
  among it, and in a NaN's sign; a dot at the card's unit takes the card's products; and no product of the engine's
  arithmetic goes past the unit's multiply, the census that found the machine's int8 cache read by the engine's own).
  Each real-checkpoint bar has its twin at `FpUnit::Hard` (`d5005_engine_lock`'s `*_on_the_cards_multiplier`: Qwen3.5-0.8B
  and Llama-3.2-1B at every knob point, Qwen3.8-27B at its four, short and long prompts), run by
  `scripts/run-model-bars-tests.sh`.
- **A D5005 build multiplies on the card's floating-point blocks at the knob point of the card's products.**
  `HardFloat.fp_unit` names the arithmetic a hard build's products compute. At `FpUnit::Hard` the product's wrapper is
  planned for the card's own (`fp_hard::plan_for` against `FpUnit::CARD`). At `Soft` it is planned for the engine's, so
  a block that decides a product tiny before the engine does is still refused. A program records its products
  (`fp hard` in `engine.txt`, `LOGOS_D5005_FP_HARD` in the build's header), and `logos_step` refuses a program written
  at the other multiply. `d5005_afu --hard-float` builds at the card's products, for programs written with
  `--hard-products`; `--hard-adds` keeps the engine's multiplies. Locks:
  - `d5005_flow_lock`: a build of the card's products plans its multiplies for the card's arithmetic;
  - `d5005_rtl_lock`: the card's multiply on its block is the card's unit, a clock at a time;
  - `d5005_engine_rtl_lock`: the whole step on the card's products runs as the machine runs it at that knob point;
  - `d5005_engine_card_lock`: a program written at the other multiply is refused.
- **A D5005 column's step moves a pointer, never a beat, and a job's format and its activations reach each module through
  a copy of its own.**
  - **The ring.** Each column core rings its decoded beats: `RING` places, the beat it issues at the head with one
    queued behind it, two for a lone core. The feed's decode register writes a core's next beat into its ring
    (`w_put`), and a step takes it by moving the head. Before, the core copied all 5,280 bits at the step's last issue:
    one combinational net to every bit of every core's beat, 20,560 sinks in hybrid7b's fit. The feed keeps no queue
    of its own, so the bits are the same, and so are the cycles: every steady array test keeps `column_array_clocks`
    exactly.
  - **The copies.** The streaming unit, the array, each feed and each core keep `(* dont_merge *) fmt_q`, the job's
    format a clock behind it, and read only that copy. Before, one register of the job's word reached 13,258 selects.
  - **The activations.** The array keeps its block RAM of every token-row's activation words beside each channel
    (`ach[k].act_ram`), every copy written alike a clock after the word is made. A channel's cores, and its pairs'
    processing elements, read only their channel's copy. Before, one memory's read reached every core of the array; a
    pair takes the word in the clock of twice the cores', half a core's clock after the read.
  - **Fewer levels.** The array's issue is a few levels from registers. It keeps `ready` (every core the step needs
    holds its beat), whether the block and the stripe are their stripe's and the matrix's last, whether the token-row is
    the step's last, and, a stripe ahead, the matrix's rows from the next stripe on and whether each core's pair is
    among the rows from the held stripe and from the next. Each is the value its wire had, the next clock's `ready`
    taken from this clock's beats offered, a step's needs a select of registers: 11 levels from the step's block to a
    core's beat before, and a 32-bit subtract and compare (6.8 ns in the foundry's carry-aware model) into `ready`,
    against the 5 ns of a 200 MHz clock. The streaming unit puts a clock's sums in row order a channel's block at a time, chosen by
    its turn alone (23 levels before). A core's octet lanes take their blocks' places from registers. The channel
    multiplexer takes every port through registers of its own: a port's descriptors through one queue of the port's
    own, one at a time in the order the port gives them whatever channel each is for, and its write lines through a
    register a channel, so its write lock starts at a register (14 levels before, from a recurrence lane port's run to
    the lock) and every ready a port sees is a register's. A register a channel between a port and the multiplexer
    would take the port's descriptor at once and let its next run reach another channel first; two ports' runs on two
    channels would then be taken in opposite orders and each port would wait on the other's (a four-lane engine hung
    so). The vector side's store length in the chunk at hand is a register (10 levels from its put table to a beat's
    lanes before). The vector unit writes a Map's row from a register of its own, ends a Map's row on a register, takes
    a turn's rows B and C for its writes from registers a clock behind them, and shifts a Scale's operand in a stage of
    its own (each 10 levels before at the card's parameters, four function units and pair copies sixteen rows deep). A
    recurrence lane
    asks its flush's out-stream on a compare of a register: the room its write queue and the asks already out leave,
    kept a clock ahead for each size of ask (10 levels before, a loop through the ask). A lane's rounding takes its
    sticky bits under `~(ones << (s − 1))`, the bits `(1 << (s − 1)) − 1` gave at every shift, with no 88-bit
    subtract's carry chain between the leading one and the sticky bit.
  - **Locks.** In `d5005_rtl_lock`: `a_step_moves_each_cores_ring_and_never_its_beats`,
    `a_jobs_format_reaches_each_module_of_the_matvec_through_a_copy_of_its_own`,
    `each_channels_cores_read_the_activations_from_a_copy_of_their_own` and
    `a_channel_queues_one_decoded_beat_a_core_when_it_deals_to_two_cores_or_more`; the array, feed and streaming
    harnesses put each core's beats into its ring; `every_port_of_the_channel_multiplexer_reaches_it_straight` (flow
    lock, by yosys's connections, in an engine of head lanes, of contexts, and of both); `d5005_engine_rtl_lock`'s bench
    stops a run busy with nothing moved (no line on the card port or a channel, no job word) for 200,000 cycles, where a
    hung run went on for days to its cycle cap, and holds every run's longest quiet stretch under a quarter of that;
    `four_head_lanes_flush_two_lines_an_ask_through_slow_write_ports_in_f32`. In `d5005_flow_lock`:
    `the_cards_fast_side_keeps_every_stage_within_a_200_mhz_clock`, every module of hybrid9's fast side within the 9 LUT
    delays of 5 ns in the timing model (its own logic, the streaming unit, the column array, a core, a feed, the vector
    side, a recurrence lane and their coordinator, the attention side and its query's quantizer).
  - **Faster checks.** The lock's yosys checks now elaborate only the modules they read (`stubbed`): the queue check
    went from 855 s to 4 s.
- **The D5005 vector side is as deep as the deepest job it cannot run a chunk at a time.**
  - **Chunked jobs.** A vector job whose rows cross nowhere runs a chunk of its rows at a time on the side's registers:
    one head, every load and store a vector of one length, every instruction elementwise or a scalar's
    (`machine::VectorJob::chunked`). Its job word's bit 53 says so, and the decoder refuses a word whose flag
    disagrees.
  - **The depth.** So the side's registers are as deep as the deepest job that is not chunked
    (`EngineParams::side_rows`): 320 rows at the 27B ∪ 0.8B, where the FFN product the reference core keeps is 1,088.
  - **Measured.** hybrid7c's vector side holds 222 M20Ks' worth of block bits against hybrid7b's 1,463, and its engine
    451k registers against 622k.
  - **Locks.**
    - `d5005_machine_lock`: a vector job runs a chunk at a time exactly when its rows cross nowhere.
    - `d5005_engine_rtl_lock`: a job deeper than the vector unit, and one whose last chunk is short, run a chunk at a
      time as the machine runs them.
    - `d5005_flow_lock`: the side is as deep as the deepest job it cannot run a chunk at a time.
    - `d5005_capability_lock`: the side's rows are judged.
- **The D5005 RAM gate counts a stacked memory's every stack as a block-RAM wrapper's.** A memory no power of two layers
  deep is built of its own stack and `_part` stacks (`rtl::ram`), and `scripts/d5005/ram_check.py` counted each part as
  its module's own memory, though a part is one of `rtl::ram`'s and its docstring said so. `d5005_flow_lock`
  (`the_ram_gate_counts_every_stack_of_a_block_ram_wrapper_as_the_wrappers`).
- **A D5005 engine can run two sequencer contexts over one set of units.** A pass was the sum of its matvec unit's
  time and its per-row units' (recurrence, vector and attention sides), since one sequencer ran one job at a time. With
  `Streaming::with_contexts(2)` (`d5005_afu --contexts`, `build-afu.sh --contexts`) the engine has a core a context,
  each over its own token-rows, rows table and bank of the store. The matvec class and the row class each serve one
  context at a time, granted at the engine's step to the context after the one last served and held until that
  context's core takes its bank back. One context's weights then stream while the other's rows run. The streaming unit
  shares the channels with the row class through the channel mux a descriptor at a time, through a credit a channel
  (`logos_read_credit`): its channels' feeds step together, so it takes a channel's lines only as its other channels let
  it, and it asks the mux only for the lines its credit's buffer of 512 can hold, in runs of at most 32 lines, each
  taken as it comes. Straight on the mux it deadlocked the row class's port, which waits on a line of one channel behind
  the unit's run there while the unit waits on that port's run in another (four sequences over two contexts stopped at
  their third position). The bring-up core takes each
  context's rows in ENGINE_CONFIG and a table write's or store read's context in its argument's top byte. The host deals
  sequence s to context s mod 2 as its row s / 2, and refuses a program written for other contexts (`contexts` in
  `engine.txt`, `LOGOS_D5005_CONTEXTS`). The machine remains the oracle, each context's step run alone.
  `d5005_engine_rtl_lock` (two contexts' steps at once equal to the machine's one after the other, each alone leaving
  the other's bank untouched, both at once in fewer cycles than the two alone), `d5005_flow_lock` (a core and a bank a
  context, the crossing unchanged, PIM elaboration, the unit's credit a channel by yosys's connections),
  `d5005_card_port_lock` (four image streams taken in lockstep beside a fast port deadlock the mux without credits, and
  through them finish every run with every line as it lies; a stream of 27B-sized tiles keeps the rate it keeps alone),
  `d5005_engine_card_lock` (four sequences decoded over two contexts as the machine decodes each; a program for other
  contexts refused).
- **The D5005 token-row store lays two slots over the same beats when no point of a step holds both.** Every slot had
  beats of its own, so the 27B's store took 8,016 beats a row (128,256 at sixteen rows, ~3,263 M20K blocks across the
  device, the busiest wide buses of hybrid7b's failed route among its users); the step never holds more than 3,584 a
  row at once. `StoreLayout` now places each slot, widest first, at the lowest beat no slot live beside it holds:
  a slot is live from a job that makes it afresh to the last job that reads it before it is made again, the step taken
  round with the host reading the logits after it, and a job that writes a slot in part leaves it live. The 27B's
  store at sixteen rows is 57,344 beats. The machine's store drops each slot once it is dead, so the engine's store,
  whose dead slots' beats another slot has taken, is held to the machine's on the slots live at each point.
  `d5005_machine_lock` (taken round the step twice, every slot a job reads holds over all its beats what its own last
  write left; a row of the store is the largest set of slots any point holds; before each job the machine holds
  exactly the slots made so far that are live).
- **Each D5005 channel's fast port crosses a register each way at the engine's edge.** hybrid7b's flat fit failed
  routing at 54% of the device: wide buses ran the length of the die, and nothing let the path from a channel's bank to
  the fast side be longer than one clock of 5 ns. `logos_pipe` (`physical::pipe_verilog`), a skid buffer whose input's
  ready is itself a register, sits on each channel's descriptors, lines, writes and write-done tokens
  (`engine|port_pipe[k]`), every stream valid/ready end to end, so a floorplan may stretch the path. `d5005_card_port_lock`
  (a source and a sink busy at random: every word in order, none lost or twice; both ready, a word a clock a clock late).
- **A D5005 build's units are floorplanned in regions of measured capacity.** The geometry probe pins a register to
  each tile of the device by its own Logic Lock region, so one fit measures every tile's ALMs, registers, M20Ks and DSP
  blocks. A unit's needs come from its synthesis, its memories counted as the fitter packs them: a stack of blocks a
  power of two deep, byte enables holding a byte a 10-bit lane, small `AUTO` memories in MLABs (hybrid7b's store and
  attention side: the fitter's own 1,024 and 45 M20Ks). Its wires come from the tree's Verilog, read through the
  module's own registers and multiplexers; each memory channel is wired to its bank's site, recorded in
  `benchmarks/silicon/d5005/emif_anchors.json`. The units are placed in the order given, each in a region of whole
  tiles grown round where its wires are shortest, or in the least rectangle that holds it (`d5005_unit_floorplan`);
  a unit is one instance or a group of several that share a region (`--group`).
  A synthesized flat image is held to the device by the same count (`d5005_budget`): its block memory bits understate
  the M20Ks a fit takes. `d5005_floorplan_lock`.
- **The Quartus queue fits a job against a floorplan, and synthesizes a flat image without fitting it.**
  `QUARTUS_FLOORPLAN=<tcl>` gives a fit a floorplan, named among its row's inputs: a fit job's is copied into the job and
  sourced by its settings; a flat image's resume's is copied beside the FIM's project and sourced once by its
  `d5005.qsf` once the queue is taken, and a resume given none is placed by none an earlier resume was given. A
  floorplan is refused where it would place no fit: by a synthesis, a persona, an image fitted since its synthesis, a
  tree with no FIM project and `quartus.sh flat`. `quartus.sh flat-syn <job> <tree>` runs a flat build to its
  synthesis's end and its budget and no further, its attempts never going on into a fit, so a floorplan is drawn from
  the image's own report and given to its `resume`. `d5005_quartus_queue_lock`.
- **A flat D5005 build fits only a design its synthesis says the device holds.** `quartus.sh flat` runs the FIM's
  compile to its synthesis's end (OFS's `build_top.sh -e`, its `-end synthesis`: the script clears an
  `ANALYSIS_AND_ELAB_ONLY` it is handed, which had let the compile fit), sums the report's estimates over its partitions
  (the ALMs needed, the DSP blocks, the block memory bits) and stops before the fitter, with its own exit status and
  what the design needs against the 1SX280's 933,120 ALMs, 5,760 DSP blocks and 240,046,080 block bits, when the device
  cannot hold it or the report gives no estimate; otherwise it fits, times and assembles, each once across attempts, as
  a resumed attempt does after the same check. hybrid8 synthesized at 1,056,208 ALMs needed. `d5005_quartus_queue_lock`.
- **A flat D5005 image is held to the device by the M20K blocks its memories take before its resume fits it, and at the
  end of a flat synthesis.** The block memory bits understate the blocks the fitter gives a memory (a stack a power of
  two deep, a byte-enabled block's 32 bits of 40): one store of 458,752 words of 512 bits is 97% of the device's bits
  and 13,312 M20K blocks, 113% of its 11,721. `quartus.sh resume` of a flat tree whose fit has not succeeded since its
  synthesis runs `d5005_budget` on the synthesis report on the host (`QUARTUS_BUDGET`, else cargo runs it); a design
  the device cannot hold is refused (5) before the job begins a log, takes the queue or runs a container, so its name
  stays free, and one it holds goes on with the verdict at the head of its log. `quartus.sh flat-syn` runs it once its
  synthesis has passed, its status and verdict the row's. The bits checked in the container stay the first gate, and
  `quartus.sh flat`'s only one. `d5005_quartus_queue_lock` (the 458,752 × 512 store refused before the queue, a design
  the device holds queued, a fitted tree not held again; the flat synthesis's row; the budget given; a budget that
  could not run).
- **The Quartus queue exits 0 for a persona's build or resume, and a flat image's resume, that passes.** Its last step,
  remembering the compile a job resumed itself from, was a test that failed whenever the job had not, so `set -e` ended
  a passing job with 1 once its row was recorded. `d5005_quartus_queue_lock` (a real persona build and resume through a
  Docker stand-in, each exiting 0 with its row passed and its fitter remembered).
- **The Quartus queue's hang watch reads a container's CPU time.** It asked `docker top <container> -eo time`, which
  Docker refuses without the process ID, so every container read as using no CPU and a job was judged by its log alone:
  a synthesis or fit job, whose log stays silent until its tool ends, was stopped as hung after twenty minutes and tried
  again. It now asks for the process ID beside the time. `d5005_quartus_queue_lock` (a fake Docker that refuses as
  Docker does).
- **A D5005 hybrid program for several sequences lays the token-row store for each of them.** The program writer laid
  the store for one row, so sequence 1's embedding was written over the start of the slot after the residual; the
  program's store now holds a row for each of the machine's sequences, and `engine.txt` gives its beats (`store`). The
  build's header gives its rows and store beats (`LOGOS_D5005_ROWS`, `LOGOS_D5005_STORE_BEATS`), and `logos_step`
  refuses a program past either before it touches the card. `d5005_engine_card_lock`.
- **`scripts/d5005/run-step.sh` loads an engine build's persona onto the card.** An engine build's AFU JSON fixes its user
  clock, and fpgaconf sets a fixed clock through the port's user-clock interface, a uio device only root opens: run as
  the user, fpgaconf failed to set it and the image never ran. fpgaconf now runs as root, and with
  `LOGOS_D5005_USERCLK_MHZ` the user clock is set to that frequency before the step program runs, for an image whose
  timing closes only below the clock it asks for. `d5005_flow_lock`.
- **A co-simulation's host program gets every argument it is given.** The simulation HAL kept the first sixteen
  `+arg=` plusargs and dropped the rest without a word, so the step program given a fourteen-token prompt decoded a
  thirteen-token one, and the recurrent hybrid's card co-simulation held a decode to the wrong prompt. `sim_start` now
  takes every `+arg=`, and Verilator's DPI backend reads the whole of `/proc/self/cmdline` (`sim_cmdline`).
  `d5005_engine_card_lock`.
- **A diode stays beside its gate through the post-diode upsize's legalizations.** Each slew-repair round adds cells and legalizes, and in a crowded region the legalizer carried a diode and the gate beside it apart a few tens of microns at a time until the diode was past every pin of its net: the whole-program engine's 18.5 ns harden left 293 diodes behind at step 24 and stopped, though no edit had moved one. Each legalization after an edit now holds every diode, and the instance whose pin is its net's nearest to it, where the last legalization left them, and moves the edit's cells around them: on the engine's step 24 no diode is left behind and the slew rounds converge in 11 with no net over the limit and 610 cells added, where setting parted diodes back on their pins left 86 behind and 48 nets over it after all 24 rounds. `fanout_within_limit_lock`.
- **A loop that grows a list no longer leaves the range oracle a length it never reaches.** The oracle's loop fixpoint
  widened each list's length every pass but judged convergence by its variables alone, so a loop that pushes onto a list
  was declared converged while the length still grew: after 200 pushes it recorded a length in [0, 10]. A loop condition
  was also recorded only at the state the loop is entered in, though it is read again after every iteration. Together
  they let the remainder in SHA3's padding loop, `(length of padded) % 136`, be reduced to `length of padded` on the AST
  C and Rust tiers since 10-14 and on the VM, and the loop never ended. A length still growing now keeps the loop
  unconverged, and a loop's condition is recorded at what the body leaves as well. `phase_exodia_oracle`,
  `mod_wrap_lock`. The loop-invariant libdivide rewrite had read a counter's entry-state range as the counter's and
  asked for a finite range where its proof needs only a lower bound; it now asks for the lower bound alone, as the VM's
  does. `phase_fast_div`.
- **A batch of re-signs can stage a block's record outside the repository.** `MACRO_LEDGER_OUT` files a Caravel macro's
  record in a staging copy of its ledger, which the batch lands with the wrapper, golden and precheck ledgers, but the
  block recorder took only a registered ledger's own path, so a staged macro harden ended at its record with its views
  unpublished. A copy outside the repository named as exactly one registered ledger now records as that ledger; any
  other path the registry does not name is still refused. `sky130_ledger_registry_lock`.
- **RED means red: `scripts/test/tdd.sh`.** The TDD loop was a rule in prose; it is now a verdict. `scripts/test/tdd.sh red|green rust <nextest args>` runs the selection under a new `tdd` nextest profile (no retries — a retry can turn a genuine RED into a flaky GREEN — no fail-fast, a JUnit report) and `scripts/lib/tdd-verdict.py` reads that report: RED holds only when every selected test compiled, ran and failed; one test that already passes, a skipped test, a filter that selects nothing, or a build that never ran a test is refused. Each failure prints the panic's own message (never its `panicked at` header), marks a `todo!()` stub as one, and says how many more lines the message has, so a RED is checked for failing for the right reason. `learn-e2e` and `junit` modes give the browser suite and any other JUnit producer the same verdict. The brakes the full baseline applies — nice/ionice, concurrency caps, refusing to overlap another suite or a place-and-route job — moved from `run-all-tests-fast.sh` into `scripts/lib/suite-guard.sh`, which both runners source. Locked by `tdd_verdict_lock` (18 tests over fixtures cut from real runs).
- **A LOGOS site can now actually be shipped.** The framework could build a website and could not deliver one. Five things closed that gap, each with a lock that crosses the boundary the old tests never did. **`LOGOS_HOST`** — every tier spelled `127.0.0.1` at its own bind site, so a `largo build --release` binary in a container, on Fly, Render or a k8s pod was unreachable from outside its own network namespace, while `docs/web.md` recommended exactly that deployment; ~600 web tests missed it because every one of them dials loopback from the same host. The address is now composed in one place (`env_bind_address`), defaults to loopback so running a program never publishes it, and brackets an IPv6 host. **`Head:`** — a fifth LGX section holding real head HTML, so a page owns its own title, description, social card and canonical URL instead of shipping `<title>` set to its file name; it merges over the built-in defaults by tag identity, so a page that declares nothing keeps the document it had. **`largo build --static`** — a `dist/` of pre-rendered pages, the content-addressed module, `static/**`, `robots.txt`, `sitemap.xml` and a `_headers` file, deployable to any static host; a site with no backend now builds at all, which it previously could not. **A declared `## Error page`** is published as `404.html` and served by the running binary alike. And **`largo test` now runs the tests written in `pages/`**, where every test that drives a page naturally lives — it had never looked there, so a `--web` project's frontend could be green and untested. `bind_address_lock`, `head_lock`, `site_metadata_lock`, `static_export_lock`, `error_page_ships_lock`, `pages_in_test_lock`, `dev_prod_parity_lock` and the crown `real_website_lock`, which builds a real four-page site and serves it both ways — as files a static host maps by name, and as one binary answering over a non-loopback address.
- **Shipped assets travel compressed.** There was no `Content-Encoding` anywhere in the serving stack, so the WebAssembly module — the largest thing a page downloads, and the whole reason a page can be written in LOGOS at all — went out raw on every first visit. It is compressed once at build time, so the saving cannot be skipped under load, and answered from a table of its own rather than by widening the row every response is built from. The three headers are one contract: negotiate on `Accept-Encoding`, declare `Content-Encoding`, and say `Vary: Accept-Encoding` so a shared cache never hands a compressed body to a client that cannot read one — with a distinct ETag per representation, which is what makes each variant revalidate as itself. A file too small to gain by it ships none.
- **Three more examples, and a lock that keeps them working.** `examples/tic_tac_toe` is a page with no server at all — state, actions, a derived winner and an action that takes the clicked cell as an argument, with its own `## Test` that plays a game and asserts it was won. `examples/guestbook` is the opposite end: a site with no JavaScript anywhere, server-rendered routes and an ordinary form POST, where a `method="post"` form without a `_csrf` field does not build and a forged token leaves the book unchanged. `examples/marketing_site` is four pages deployable to a CDN, with per-page titles and social cards, its own `404.html` and a generated sitemap. `examples_lock` sweeps the directory rather than listing it, so a new example is covered the moment it exists: every project must check, one with `pages/` must export a titled front page, one carrying a `## Test` must pass it, and each must explain itself.
- **A production lifecycle every tier actually gets, and an external way to control it.** The lifecycle state machine existed and was well tested, but only an in-process embedder could reach it: a deployed LOGOS binary had no more external control than `largo serve` did, because nothing in the repo handled a signal (`tokio` was compiled without its `signal` feature) and no lifecycle setting could be named from outside. Three seams close that, all in the shared crate so no tier can drift. `LifecycleConfig::from_env` is read by `ready_serve_lifecycle`, which every serve entry reaches — so the compiled `Serve` statement honors `LOGOS_MAX_INFLIGHT`, `LOGOS_REQUEST_TIMEOUT_MS`, `LOGOS_READ_DEADLINE_MS`, `LOGOS_DRAIN_MS`, `LOGOS_MAX_BODY`, `LOGOS_CORRELATION_ID`, `LOGOS_READY_PATH` and `LOGOS_HEALTH_PATH` with **no change to code generation at all**, the same trick `env_port_override` plays for the port. `shutdown_requested()` is a FUTURE rather than a spawned task, precisely so the `!Send` interpreted and bytecode accept loops can select on it: `SIGTERM` and `SIGINT` stop readiness being advertised, stop accepting, finish the in-flight work inside the drain deadline, and release the port for a successor. And readiness and liveness are opt-in probe paths answered BEFORE admission, so a monitor gets a reply during an overload rather than the overload — readiness turning `503` once draining while liveness stays `200`, because a draining server is healthy and killing it abandons exactly the requests it is finishing. Configure neither and nothing is reserved, which is why the handler still owns every route and the cross-tier byte-for-byte differential is unmoved. The interpreted and bytecode `Serve` loops gained the rest of the contract in the same pass: bounded admission with a prompt `503`, the configured body cap, and a read deadline that drops a client which connects and then dribbles. Locked by `http_lifecycle_parity` (four cases against the real `largo serve` binary), eight new unit laws over the environment seam and the probes, and the existing sixteen lifecycle and bounds tests unchanged. The interpreter's per-request budget is enforced from the INSIDE — at the same per-statement checkpoint the op budget already uses — because its handler borrows `&mut self` and runs inline on the one loop thread, so it cannot be raced against a timer the way the compiled tier's handler is moved to a blocking thread. The granularity is stated rather than glossed: the deadline is observed BETWEEN statements, so it bounds a runaway loop and not a runaway primitive, and a handler that loops forever is abandoned with a `503` while the very next request is still served.
- **The full-stack web surface, documented.** A new guide, [`docs/web.md`](docs/web.md), covers what the campaign built and nothing in the documentation set had described: serving over HTTP and the request/response shapes, headers and query parameters, the static-file server and its traversal guard, strict and typed JSON reading, the browser half (components, signals, the correlated fetch wrappers, routing), the pure-LOGOS PostgreSQL client and its parameterized-statement rule, the raw socket primitives everything stands on, and the SHA-256 family. Its examples are executable, not schematic: backend snippets compile through `compile_to_rust` under the existing doc-example lock, and browser snippets — which only the WASM tier lowers — are compiled through `compile_to_wasm` by a new `doc_browser_examples_compile_to_wasm` test over a second verified fence language, so a reactive example in the docs can no longer rot unnoticed. `docs/cli.md` gains the `serve` command and the `--web` scaffold, the complete ten-target `emit` list, the `LOGOS_PORT`/`PORT` seam, and a corrected `test` entry (it had been documented as "reserved for the future LOGOS test framework" since it shipped); `docs/imperative-mode.md`'s standard-library list, which named ten of thirty-eight modules, is now an accurate inventory.
- **`Real` reactive signals are a first-class value family.** `signalNewFloat`/`signalGetFloat`/`signalSetFloat` join the `Text` and `Seq` signal families, and the parser settles a signal's family once, at its declaration, from the initial value, an explicit `Let rate: Real be a signal 2` annotation, or a later fractional write. Integral writes into a `Real` signal store reals (`Set rate to 3` stores `3.0`, `Increase rate by 1` adds `1.0`), so a cell never changes family mid-life.
- **IR peephole (copy-propagation + dead-move elimination) for the flat backends — RISC-V, WASM, fabric.** The C tier hands its emitted code to gcc, which coalesces register-to-register moves and deletes dead copies; the backends that lower the bytecode straight to a final target had no such pass, so they shipped the VM compiler's move cascades verbatim. `Set a to b. Set b to t.` (the fib/hash swap idiom) lowers to a FIVE-move cascade through fresh temps every loop iteration; the new `vm::peephole` pass (block-local forward copy-propagation, then dead-move elimination over an iterative-liveness fixpoint) collapses it to the optimal two moves — three fewer ops in the hottest loop, per iteration. Measured: 25 of the 35 shipped benchmarks shrink. It is a semantics-preserving `CompiledProgram → CompiledProgram` transform, sound by construction: `reads` over-approximates and `kills` under-approximates the per-op register effects (so liveness never deletes a live value nor cuts a range short), copies are invalidated on both the key and value side by `clobbers = kills ∪ mutates` (the rule that keeps it correct under LOGOS's value-semantics collections — a `Move y = xs` fact dies the moment `xs` is mutated or reassigned), the in-place-mutated collection register is never a substitution site, only `Move` is ever deleted, and all four register classifiers are exhaustive over the 117-op ISA (a new op fails to compile until classified). Wired into `compile_to_riscv`, `compile_to_wasm`/`_observed`/`_linked`, and `whole_program_fabric_from_source`. Pinned by `peephole_soundness_lock` — `program_outcome(peephole(P)) == program_outcome(P)` over the compute corpus (raw AND optimized IR) + every benchmark, idempotence, a strict op+move-count drop on the swap idiom, a 400-program copy-heavy SplitMix64 fuzzer ≡ the VM, and a value-semantics aliasing torture — and confirmed ≡ VM over the whole corpus on all three backends (`riscv_equals_vm_and_treewalker_over_the_corpus`, `wasm_equals_vm_and_treewalker_over_the_corpus`, `fabric_coverage_lock`). One subtlety the locks caught and forbid: an enum `Inspect`'s scrutinee register is a typed anchor the flat backends' static kind-inference couples positionally (a `BindArm`'s payload type is resolved only when the guarding `TestArm` has the SAME target register), so the peephole leaves `TestArm`/`BindArm` targets untouched — rewriting one but not the other (they sit in different basic blocks) would push the still-≡VM program out of the backend's enum fragment.
- **Plasticine (open RDU) lane, modeled end to end.** `codegen_sva::plasticine_report` turns a counted-loop LOGOS function or a tiled fp32 GEMM into the artifact an RDU engineer reads — PCU/PMU used vs total, every metapipeline stage's initiation interval (Spatial's definition) with the critical stage flagged, N-buffer depths, hop counts and static-link occupancy, the DRAM roofline, GFLOPS and % of the ISCA'17 / SARA published fp32 peak — anchored to the VM at the modeled II (hazards one cycle below) and SAT-certified for ALU-only bodies. `compile_to_plasticine` + `largo emit plasticine [-o DIR]` emit Spatial source, the checkerboard placement and the report row; `benchmarks/silicon/plasticine_report.json` is generated only by `emit_plasticine_report` and pinned byte-equal by `plasticine_report_lock`. Every report row says `modeled`: the rows are a model, calibrated against the simulator below, and nothing ran on Plasticine silicon. **The public Plasticine toolchain runs in a container** and calibrates the model: Spatial's own `GEMM_0` passes end to end (PIR mapping, plastiroute, Tungsten cycle-accurate simulation at 132,068 cycles), recorded by `scripts/plasticine-gate-ledger.py` into `benchmarks/silicon/plasticine_toolchain.json` with committed log excerpts, and joined against our model in `benchmarks/silicon/plasticine_calibration.json` — 0.75 % apart on the same shape (`plasticine_sim_lock`, tolerance 15 %). **Our own emitted loops run through that chain too** (`largo emit plasticine` writes the Spatial regression app; `emit_plasticine_spatial_tests` writes the canonical set): five pass with the VM's answer, and Tungsten's cycle counts calibrated the PCU model — a loop-carried register costs the pipeline depth + 1 per iteration (7 on the six-stage PCU) regardless of the operations on the path (`UnitModel::plasticine_pcu`); across the simulated apps the model is never more than 5.2 % off. The 20x20 rows calibrate against the memory the chain actually simulates (`PlasticineTarget::tungsten_20x20_ddr4`, DDR4-2133 x4 as Tungsten configures it).
- **The Studio reads as one IDE.** It looked like several tools built at different times; it now works as one. Opening a file chooses its mode: the Logic / Code / Math / Hardware / Web tabs are gone, and one rule, `StudioMode::for_path`, reads the mode from the path for a deep link and a file-tree click alike. Every mode edits in one `CodeEditor` with the language server running in the page — `logicaffeine-lsp` builds for wasm with its new `server` feature (tower-lsp, tokio) off — so the Studio shows VS Code-style hovers, diagnostics as squiggles with the problem at the top of the hover, and every keyword coloured the way the lexer reads it, at any case. The debugger reports the program's own state: reference counts are the program's references rather than the rewind history's, and frames and the call stack carry function names from debug-only compiler information that a production compile never emits; it steps back through time as well as forward. The columns, the debugger drawer and Logic's proof panel resize from bars that follow the pointer frame by frame and keep their minimums at every window size. Hardware mode reads a design once, as its kind, and titles each panel for what it holds; an example that needs the compiled runtime says what it needs and points at `largo build` instead of pretending to run; a derivation the kernel refused is shown as uncertified, never as a proof. Every example keeps the promise its registry entry makes, checked in the Studio's own panels. Locked by `crates/logicaffeine_tests/tests/studio_file_is_the_mode.rs`, `tests/studio_example_contract.rs`, the `studio_layout`, `code_editor`, `problems` and `debug` unit tests, and the browser suite `scripts/web/test-studio-e2e.sh`, run in Chrome and in Firefox.
- **The Syntax Guide's examples are live editors.** Every example, every code exercise and every LOGOS or Logic snippet in the guide sits in the Studio's `CodeEditor` with the language server on: keywords coloured as the lexer reads them, a VS Code-style hover for the word under the pointer, and diagnostics as squiggles. Snippets are read-only with a Copy button, and First-Order Logic answers keep a plain box, since FOL is not LOGOS. Locked by `crates/logicaffeine_tests/tests/guide_language_server.rs` (every example presented as working opens without an error or warning), `tests/guide_render.rs`, and the browser suite `scripts/web/test-guide-e2e.sh`.
- **Every `[Unreleased]` reads as one release.** `[Unreleased]` becomes the next release's notes word for word, and it had drifted: this CHANGELOG carried two `### Fixed` headings, with five additions filed under the first. `changelog_shape_lock` now holds every CHANGELOG in the repository to `[Unreleased]` first and one heading per kind of change — Added, Changed, Deprecated, Removed, Fixed, Security, in that order. Released sections are left as they were written.
- **The Studio's editor behaves the way VS Code's does.** Enter keeps the line's indentation and deepens it after a line that opens a block — LOGOS's `:`, Math's `:=`, `with`, `Proof.` and `## Theorem:`, Verilog's `module …;`, `begin` and `case` — and a Verilog `end` steps back as Enter leaves it. Tab fills to the next stop or indents every line a selection touches, Shift-Tab outdents, every edit undoes like typing, and Escape then Tab leaves the editor. A 25 px status bar under a 2 px line counts the language service's errors and warnings and says where the caret is ("Ln 3, Col 4 (2 selected)"), how the editor indents ("Spaces: 4", two in Verilog) and which language it holds (LOGOS, Logic, Vernacular, Hardware, or Verilog, known from a module's first line). Locked by the `indent` and `status_bar` unit tests and the browser suite (`enter_and_tab_indent_as_the_language_does`, `the_status_bar_*`).
- **Math shows its symbols, and Verilog and Math are coloured by their own grammars.** Math files stay ASCII — the kernel reads `forall`, `fun`, `->` — and the Math editor sets them in Logos Math Mono, a subset of DejaVu Sans Mono renamed as its licence requires, whose contextual ligatures draw ∀ ∃ λ → ⇒ ≤ ≥ on whole words and whole operators only. Both of the editor's layers shape the same glyphs, so the caret, clicks and selection stay on the text, and a symbol is one character to the caret. Verilog is coloured by its own lexer (sized literals, `/* */`, `$tasks`, whole operators); Math from the kernel's vocabulary, with a test that fails if the kernel reads a keyword the editor does not colour; Logic's counting quantifiers ("at least three") read as one phrase. Locked by `crates/logicaffeine_tests/tests/math_font_ships.rs`, the `verilog` and `vernacular` unit tests, the HarfBuzz probes in `scripts/web/build-math-font.py`, and `math_shows_its_symbols_in_place_of_their_spellings`.
- **A use after a possible `Give` is refused again, whatever the optimizer folds.** `compile_to_rust_checked`, and the pre-check `compile_and_run` runs, checked ownership on the *optimized* program. Once `x` is a known constant, propagation turns `Give x to p.` into `Give 5 to p.` and `Show x.` into `Show 5.`, so `Let x be 5. If x > 0: Give x to p. Show x.` compiled while the editor, which checks the parse, underlined it. Every compile path now judges zone escapes and ownership on the freshly-parsed program, before observation elision, division resolution and the optimizer (`check_as_written` in `compile.rs`). The compiler and the editor now agree, and no optimization config changes a verdict. The escape check had the same defect: `compile_to_rust`, the checked compile and `largo`'s project path accepted an escape inside a branch the optimizer proves dead. Five more holes closed on the way:
  - **The parser leaked a move into a sibling branch.** A `Give` in an `If`'s then-branch reached its `Otherwise` branch, and one `Inspect` arm reached the next, so `If c: Give x to p. Otherwise: Show x.` was refused in every tier. This is also why `test_move_in_both_branches_then_use` had been passing for the wrong reason.
  - **A `Repeat` never merged.** A loop that may run no pass reported its body's move as certain.
  - **Control-flow heads went unread.** A maybe-moved value read by an `If` or `While` condition, a `Repeat`'s collection or an `Inspect`'s target went unnoticed.
  - **The refusal was garbled.** It read "If the a conditional branch branch executes"; it now says whether the value may have gone in a conditional branch or in a pass through the loop.
  - **`During` spans were skipped.** Both checkers now read a span's body in place.
  Locked by `ownership_judges_the_program_as_written.rs` (37 tests), five new cases in `a_move_is_certain_only_in_its_own_block.rs`, and the LSP's `the_editor_and_the_compiler_agree_on_ownership.rs`.
- **The Studio's Math and Logic pages hold still, and a failure nobody planned for no longer kills them.** Four defects, each locked where it lives. *The Context panel reshuffled on every Execute:* the kernel keeps inductives and definitions in `HashMap`s, and every fresh REPL drew a new hash seed, so the same program listed its environment in a new order each run. Every fresh kernel now starts from one cached standard library, so a program lists the same environment in the same order on every run (the `math_context_tests` unit tests). *A purple "Working…" button flashed* beside the buttons for the length of every run and made the row jump; it is gone. *A trap took the whole page down:* the web build aborts on panic, and in wasm an abort — or the stack running out, which `Check` on a term nested ~2,000 parentheses deep did in Math, and a deeply nested tactic script did in Logic — throws out of the module and left the Studio dead, every later press trapping again on the stack pointer the trap had left below zero. Math and Logic actions now run behind `ui::fault::contain`: a JavaScript `try`/`catch` around a call back into the module that restores the exported `__stack_pointer` before Rust runs again, with a panic hook taking the message on its way out; the fault becomes an orange note just left of the buttons (full message and source location in its tooltip) and the page keeps working (`src/ui/fault.rs` unit tests, `tests/fault_hook_chain.rs`, and `tests/wasm_fault_boundary.rs` in real wasm under node). *Compile on a long theorem crashed dioxus's DOM patcher* (`Cannot read properties of null (reading 'replaceWith')`): the interpreter counts the nodes an edit moves in a `u16`, and a code view drew every token as a sibling, so past 65,535 tokens the count wrapped; the highlighters now group tokens into runs of 4,096 (`tests/code_view_channel_lock.rs`). Browser scenarios for all four in `scripts/web/test-studio-e2e.mjs`.
- **docs.logicaffeine.com answers every address honestly.** The docs site shipped without a `404.html`, and Cloudflare Pages treats such a deployment as a single-page app: every address it had no file for came back as the landing page with a 200. So `/search.index/` — the directory rustdoc's search reads its shards from, never a page — was served, and crawled, as a duplicate of the home page. The same fallback hid more. `build.sh` hand-picked pieces of rustdoc's output and left out `help.html`, `settings.html`, `trait.impl/` and `type.impl/`, so every trait page's implementors script came back as that landing page. And `cargo doc --no-deps` does not order one crate's doc job after its dependencies', while rustdoc links another crate's items only if that crate's doc directory exists as rustdoc starts — so `logicaffeine_proof` shipped with no link into `logicaffeine_kernel` or `logicaffeine_base`, and `logicaffeine_language` none into `logicaffeine_base` or `logicaffeine_lexicon`. `build.sh` now hands the curated crates to a new `apps/logicaffeine_docs/site.py`. `site.py document` warms the build with a parallel `cargo check`, creates every curated crate's doc directory, then runs one `cargo doc --no-deps --lib --jobs 1 -p …` over exactly those crates — the search index names only shipped pages, and since cargo deletes a crate's doc directory just before documenting it, one rustdoc at a time is what makes every cross-crate type link on every build (a fixture that linked 6 times in 10 now links 20 in 20). `site.py assemble` ships rustdoc's output whole and adds a real `404.html` (`noindex`), a `robots.txt` and a `sitemap.xml` of every non-redirect page at its extensionless address. It refuses the build — leaving no `dist/` behind — when rustdoc documented a crate off the list, a shell file would overwrite rustdoc output, or any page references a file the site does not ship; only rustdoc's optional `async` implementor scripts and links inside docs it copies from foreign traits (`tracing`'s blanket impls) are rustdoc's to answer for. `docs_site_lock.rs` documents a fixture workspace with the real toolchain through `site.py` and checks the assembled site against a model of Pages' routing; `wrangler pages dev` over the real build agrees (`/search.index/` is a 404). The documented `RUSTDOCFLAGS=""` permissive build never worked (`${RUSTDOCFLAGS:-…}` replaces an empty value too); it does now. The build's own lint gate had been failing on 76 broken intra-doc links across `logicaffeine-system`, `-language`, `-proof` and `-compile`; they are fixed, including a `journal` module description that sat on `pub mod binisa;` and put binisa's links in the wrong scope.
- **The Learn Logic page, audited end to end.** A full audit found the page broken at its root: any `dx serve` or `dx build` without a hand-run staging step answered `/data/lexicon.json` with the SPA's index.html, and every module opened on "The word bank failed to load" — both collin-datacenter deployments included. The web crate's `build.rs` now stages the bundle for every wasm build and re-stages when a source changes or the output goes missing. Beneath that: lesson bold never closed (every `**` became an opening tag); exercises played in string order (`A_1.10` before `A_1.2`) in 16 of 18 modules; five cards showed stale exercise counts from a hand-kept copy of the curriculum; test mode graded answers before submission and paid XP on every re-click; a two-exercise module set a ten-question test; missed exercises never came back; ambiguity exercises printed their answers and took none; nothing persisted across a reload; the tab highlight drifted from the mode; sidebar links into a collapsed era did nothing; `/learn#module` deep links were erased by router boot; Space on an era header scrolled the page 787px; the page ignored six of the site's seven themes; the Basic Modal Logic preview did not compile; and every page carried a duplicate `id` from the inlined logo. The rules now live in pure, unit-tested state (`learn_state::{PracticeSession, TestSession}`); the curriculum is parsed once and reports a malformed file instead of dropping it; module examples, summaries and difficulty moved into each module's `meta.json`, every example compiling in Logic mode; the Fallacies and Argumentation module's religious examples were replaced. Locked RED-first by unit tests, SSR render tests, and a 29-scenario browser suite (`scripts/web/test-learn-e2e.{sh,mjs}`).
- **The deployed site's first paint, and what it does when the app is slow or never arrives.** Checked against a build made exactly as `deploy-frontend.yml` makes one, four defects in the SSG shell and merge, all on `main`, all unseen because no deploy has run since the Tests workflow went red. The takeover watchdog judged the app live by `getElementById('app-root')`, which the prerendered copy also carries, so it deleted the prerendered page 1.5 s after load whether or not the wasm ever arrived — a failed or slow boot left a blank screen; it now reads `#app`, where the wasm mounts. After 20 s it stopped watching for good, so an app that booted later rendered above a copy that never left; it now parks and wakes on the first change to `#app`. Every prerendered page opened with its `<title>` before `<!DOCTYPE html>` — the prerender server renders through dioxus's fallback template, which has no title of its own — so every page rendered in quirks mode and without `lang="en"`; `merge-ssg.sh` now gives each page the shell's prologue. And the server's hydration markers, written inside `<style>` as CSS text, dropped the first rule of 1107 style blocks — the palette among them — so every prerendered page painted light text on white; `merge-ssg.sh` strips them from raw-text elements. Separately, the web README documented a `--wasm-split` deploy build and no `dx`, which reproduces a different, broken site; it now carries the workflow's recipe. Locked by two new render-lock passes (with the wasm refused, every route keeps its styled prerendered page; with it held past the give-up window, the app still takes over), `verify-ssg.sh` prologue and marker checks, `crates/logicaffeine_tests/tests/ssg_merge.rs`, and `crates/logicaffeine_tests/tests/deploy_recipe.rs`, which holds the README's deploy block, every script's usage note and CI's `dx` rev to the workflow and the app's dioxus pin.
- **A record field named `who`, `what` or `whom` silently did not exist.** It was declared, it was absent, and the error blamed a read three lines below a declaration that had reported success. This is the `i`/`it` defect again: that fix taught the field-name acceptor about the pronoun token class, and the interrogatives have token types of their own, so the class closed and the shape stayed open. With the article consumed and the name unaccepted, neither branch of the field loop ran — no field pushed and no token consumed. The rule is now the position rather than a longer list of words: after an article in a `has:` block, an alphabetic token is the field name. Found by writing a guestbook, where `who` is the first field anybody reaches for.
- **Every page but the front page was served as a download.** Content types were derived from the path's extension, with `/` special-cased — and a page's canonical URL has no extension, so `/about`, `/blog` and `/docs/guide` all went out as `application/octet-stream`, which a browser offers as a file to save rather than a page to render. The front page is the one everybody tests, so nothing saw it: every existing assertion reads the response BODY, and the static-export locks read files off disk where a content type does not exist. A page is now HTML because it *is* one; guessing from the path stays where it belongs, on `static/**`, where the file name is genuinely all there is to go on. `.xml` joined the one shared MIME table so a generated `sitemap.xml` arrives as XML.
- **A cache-busting query string 404'd in production only.** The shipped asset table was gated on an empty query, so `/style.css?v=2` served all through `largo serve` and vanished the moment the site shipped — the sixth divergence of this shape the campaign has found, and the `?`-split written inside the generated lookup had been unreachable dead code behind the guard.
- **A project's own `favicon.svg` did nothing.** It was served and linked by nothing, so every page still pointed at the built-in icon. Invisible for `favicon.ico` alone, because browsers probe that path themselves.
- **An exclusion can no longer make a test dark.** `scripts/run-all-tests-fast.sh` drops two classes of test from the routine baseline — the SAT/coNP research campaign and, now, the heavy on-demand locks — and each class has a companion runner that runs exactly those tests on purpose. Both scripts carried a comment saying the two lists "MUST stay in sync", and a comment is not an invariant: drop a binary from the baseline, forget it in the runner, and the test runs NOWHERE, forever, while the suite still reports green. That is the worst failure a suite has, because it is a silence rather than a red. `crates/logicaffeine_tests/tests/ondemand_exclusion_lock.rs` closes it by construction — it parses both scripts and refuses any binary the baseline excludes that no runner picks up, any binary a runner claims that the baseline does not exclude, a missing or non-executable runner, and the deletion of the every-run companion that keeps the excluded marketing probe's floor pinned. Four tests, milliseconds, no cargo, so it runs in every baseline and catches the drift at the moment someone edits an exclusion.
- **Logic answers are graded by reading them.** The web app's grader never looked inside parentheses, and LOGOS writes every formula in them, so a wrong noun passed (`Dog(x)` where the key said `Bird(x)`), so did an extra predicate, and a correct answer with single parentheses failed against the double-parenthesised key — on the Learn, lesson and review pages and in the Syntax Guide. It now parses the notation LOGOS prints, compares bound variables by their binder, conjuncts in any order and names in any case, and treats a dictionary category beside its noun (`Animal(x)` beside `Bird(x)`) as optional, so the textbook answer and the formula LOGOS shows both pass. Locked by `grader::tests` and `learn_examples::the_grader_reads_every_answer_key_it_is_given`, which grades every generated answer key against a renamed-variable variant (must pass) and a renamed-predicate variant (must fail).
- **Guide and news prose no longer breaks mid-sentence on a phone.** Both renderers made each source line its own paragraph, so hard-wrapped text broke wherever the screen was narrower than the source. They now share one CommonMark-style block parser (`ui::markdown_blocks`); along the way three news paragraphs that began with a number stop disappearing, article code keeps its indentation, article tables render as tables, and on a phone an example card's header and an article's tags wrap instead of running off the edge. Locked by `markdown_blocks::tests`, `news_render`, `guide_render::every_line_of_guide_prose_reaches_the_page` and the browser scenario `every_example_header_fits_a_phone`.
- **Eleven post-synthesis silicon locks timed out on every full run, and not one of them was broken.** `wpe_postsynth_silicon_lock`'s gate-level sims each run a real `synth_xilinx` yosys flow and then simulate the synthesized ~90-RAMB netlist; their own `#[ignore]` reasons declare the cost as "~20-30min" and "~30-60min" EACH, and the file's header asks outright that they be run single-threaded so the heavy sims never overlap. Nothing enforced that. Unlike the `ttsim`, `cuda-gpu` and `check-deep` groups beside them they had no `max-threads = 1`, so the baseline's `--run-ignored all` started all eleven at once, each starved the others of CPU, and every one hit the profile's 30-minute backstop — eleven identical `TIMEOUT [1800.0xx s]` rows that read as eleven code regressions and were none. They now have a `wpe-postsynth` group enforcing the discipline the file already stated, and, like the research campaign, they are excluded from the routine baseline into `scripts/run-silicon-tests.sh`; the same file's FAST construction locks, which enforce the exact invariant each silicon fix establishes in milliseconds with no yosys, still run every push. The same run's other three lane failures are fixed the same way: `riscv_aot_lock` and `scratch_rv` cross-check every benchmark program against the tree-walker inside `qemu-riscv64` — pure emulated CPU with no I/O to overlap, measured at ~320s on a quiet box and blowing through both a 1200s and an 1800s cap under a saturated one — so they are serialized into a `qemu-corpus` group with a budget sized to that measurement, and `marketing_test_count_floor_is_live`, which shells out to a script that recompiles every test binary in a different profile and feature set, is excluded per its own `#[ignore]` reason while the non-ignored `marketing_test_count_floor` keeps pinning the same number every run.
- **The io_uring engine served with no lifecycle at all, and it is the default wherever it is compiled in.** No readiness, no admission, no request timeout, no drain, no correlation id — every guarantee the tokio engine enforces silently evaporated on that path. Its handler runs inline on the shard thread, so it genuinely cannot enforce a wall-clock budget, a probe path or admission shedding; rather than half-thread it, `choose_engine` now falls back to the tokio engine whenever one of those is configured, and names the environment variable that caused the fallback. The caps it does enforce (body size, read deadline) are properties of parsing and cost nothing.
- **The documentation generator had been reading a source layout that no longer exists.** Every one of the twenty-nine paths in `scripts/generate-docs.sh` predated the workspace split (`src/lexer.rs`, `src/parser/*`, `logos_core/*`); each printed `Warning: File not found` and the script exited 0, so the generated reference silently lost every section it could not find. All paths now resolve, a missing file is a hard failure instead of a warning, and `generate_docs_paths_lock` reads the paths straight out of the script and checks them — so a source file that moves fails in an ordinary test run rather than quietly emptying the docs.
- **A `Real` signal written by an event handler rendered its raw bit pattern.** A view showing `{price}` after a click handler ran `Set price to 2.5` printed `4612811918334230528` — the f64's bits read as an integer. WASM packs a `Real` signal's f64 into the shared i64 cell and reinterprets on the way out, but it decided which cells to reinterpret by inferring float-ness from a value's kind, and kinds are inferred one function at a time: the handler learned the cell was a `Real` and the view that read it never found out. The family is now named in the builtin the parser emits, exactly as `Text` and `Seq` signals have always worked, so every function agrees about the same cell with no cross-function inference at all. A signal declared `Real` from a whole-number start (`Let rate: Real be a signal 3`) now divides to `1.5` rather than truncating. Locked by three new cases in `dom_reconciliation_lock` covering both ends of the round trip.
- **The virtual machine and the tree-walker disagreed about an oversized request body.** A client declaring a `Content-Length` above the limit got `413 Payload Too Large` from the interpreted server and a flat `400 Bad Request` from the bytecode tier, because the VM's accept loop answered a canned 400 for every read error instead of applying the shared `read_error_response` law the tree-walker and both native engines use. The two tiers are locked byte-identical by `http_serve_cross_tier`, which had never sent a request of either shape; it now scripts both, and a named case asserts 413 for "too big" and 400 for "not a request" on each tier.
- **A missing static file is a 404, and it is now pinned.** `fileResponse` probes with the recoverable `read` before touching the aborting byte-reader, so an absent asset answers `notFound()` rather than taking the server down — behavior the standard library gained but no test held. `http_static_lock` locks it on every executing tier for both the direct call and the rooted server, and the todo example's README, which still documented the old crash as a known gap, has been corrected.
- **Eight `wasm-jit`-gated frontend test binaries had never run.** Both test runners filtered the isolated `--features wasm-jit` pass to a hand-listed three binaries, so `wasm_dom`'s wasm tier, the reconciliation lock, the full-stack crown lock, JSON-on-WASM and four more compiled nowhere. The filterset now lives in ONE file (`scripts/wasm-jit-filter.txt`) that `run-all-tests-fast.sh` and CI both read, and `wasm_gated_suite_lock` derives its required content from the gated sources — a new gated binary that is not listed fails the default pass instead of going dark.
- **WASM tier: functions with more than 64 basic blocks no longer trap.** The dispatch-loop emitter wrote every `i32.const` block index with the UNSIGNED LEB128 encoder; `i32.const` immediates are SIGNED, so indices 64..=127 decoded as negative numbers, the `br_table` fell to its default arm and the function's epilogue `unreachable` fired — the "cumulative size" trap that kept `jsonParseNumber` (and any large function) off the browser tier. Every block-index write now goes through one `cfg::set_next_block`/`goto_block` helper (nine sites across the whole-program emitter, the JIT region emitter and `emit_cond_jump`), the class of bug is forbidden by a source lint (`const_immediates_never_use_the_unsigned_encoder`), a synthetic size ramp is locked (`wasm_large_function_lock`), and the JSON number-parse test is no longer ignored.
- **Installed `largo build`/`largo run` now work outside the monorepo — the native AOT tier builds real projects.** The LOGOS→Rust codegen path staged the runtime crate *sources* (`logicaffeine_data`/`-system`/`-base`) from a workspace checkout and path-depended on them, so on an installed `largo` the default (native) `largo build`/`largo run` died with `Could not find workspace root` — only `largo run --interpret` and `--emit wasm` ran standalone, exactly the flow `install.sh` advertises (`largo new hello && cd hello && largo run`). The generated project's manifest now resolves the runtime from **crates.io at the compiler's own exact version** (`logicaffeine-data = "=X.Y.Z"`, `logicaffeine-system = { version = "=X.Y.Z", features = ["full"] }`) whenever no checkout is present, and skips the source-staging copy; inside a checkout the staged path-deps are preserved byte-for-byte. Both the single-file compile path (`logicaffeine_compile::compile::write_cargo_project`) and the multi-file project build (`apps/logicaffeine_cli` `project::build`) share one `RuntimeCrateSource` decision so the two manifests can't drift, locked by unit tests over the exact dependency lines. A clean-machine `curl … | sh` then `largo new hello && cd hello && largo run` now prints `Hello, world!` at full native speed.
- **Installer regression gates + docs consistency.** The release `e2e-install` matrix gained the exact combo that shipped broken in v0.10.0 — `linux-x64 --full` on the `ubuntu-22.04` glibc-2.35 floor, plus `macos-full` for parity — so a dynamic-libstdc++ regression fails the post-publish gate instead of reaching users. A new `installer_docs_lock` ratchet pins the one-line install command byte-identical across every surface (the READMEs, `docs/cli.md`, the web guide/news pages, the VSCode README + walkthrough, and the scripts themselves), holds the `sh -s -- --full` flavor form and the `~/.local/bin` install-location fact steady, and forbids a competing install host from drifting in.
- **Release notes are curated from the CHANGELOG, not auto-generated.** `release.yml` no longer uses GitHub's `generate_release_notes` (which listed raw PR titles + authors and a "New Contributors" section, mislabeling authorship on the v0.10.0 release); the `github-release` job now extracts this tag's CHANGELOG section and publishes it as the release body.
- **The editor's language server stopped flagging shipped examples.** Three runtime truths were wrong in the analysis. Ownership now follows value semantics, as every tier does, so only `Give`, and `give` as a call argument, move a value. `length(xs)` and `copy(xs)` lower to the same expressions as `length of xs` and `copy of xs`; the AOT backend had emitted calls to functions the generated crate does not have. A concise field named by an ordinal (`first: A.`) is a field again; it used to vanish, and `generics.logos` failed `largo build` because of it. Locked by `ownership_follows_value_semantics.rs`, `builtin_call_forms_lower_canonically.rs`, `phase34_generics.rs` and `phase33_enums.rs`.
- **A type mismatch names its sides the right way round:** what was expected, then what was found. Locked by `type_mismatch_names_its_sides.rs`.
- **No false "never used" hints.** The language server counts a name read inside interpolated text (`"{total}"`) as a use; 86 example variables had been called unused.
- **A language-server problem underlines the statement at fault.** Statements nested in blocks carry their own spans, and checker findings carry the trail of statements they came from, so a problem inside an `If` or a function underlines that statement instead of line 1 or a lone full stop, and a move is certain only in its own block. Locked by `a_finding_names_the_statement_at_fault.rs`, `block_statement_spans_lock.rs`, `a_statement_ends_at_its_full_stop.rs`, `a_move_is_certain_only_in_its_own_block.rs` and the LSP's `squiggle_anchor_tests`.
- **Logic reads more English the way English means it.** "He" prefers the referent that agrees in gender over the most recent one. A list of objects ("sees Mary, Bill and Sue") gives each the verb, inside the subject's scope and each object's own determiner. A verb phrase after "and" shares its subject ("Some man sees Mary and sees Bill"). A prepositional phrase on the verb ("in the park") stays inside its event. Gapping reads "…and Bill Susan" as Bill loving Susan. And an abandoned speculative parse no longer leaves a referent behind for a later pronoun: "Every X has a Y. It is Z." had read `Z = Z`. Locked by `a_pronoun_prefers_the_referent_that_agrees.rs`, `coordinated_objects_each_get_the_verb.rs`, `a_coordinated_verb_phrase_shares_its_subject.rs`, `a_pp_stays_inside_its_event.rs`, `a_gapped_remnant_of_two_names_is_subject_and_object.rs` and `parser_backtracks_rewind_the_drs_lock.rs`.
- **The prover proves what coordinated sentences say, and refuses what they do not at once.** A goal may be any conjunct of a rule's conclusion; the ⊥ search remembers the fact sets it failed on and skips case splits that cannot succeed (over premises that have a model, and over a closed Horn set), so a refusal that took 194 s is immediate; an existential goal may be witnessed from its context; and a group subject distributes over a distributive verb ("John and Mary left" proves "John left"), but not over a collective one ("met") or one that may be collective ("carried"). The engine no longer drops self-named guards (`dog(Dog)`) that the certifier keeps, which had strengthened rules the premises did not state. Locked by `logicaffeine_proof`'s `a_rule_with_a_conjunctive_conclusion.rs` and `the_certifier_reads_premises_as_the_engine_does.rs`, and `a_group_subject_distributes.rs`.
- **"The butler did it" proves again: a definite description presupposes its noun, and a tensed fact is itself.** "The butler" reaches the prover as the lowercase constant `butler`, and once the engine stopped dropping self-named guards, a rule guarded by the noun (`∀x(butler(x) ∧ do(x) → seen(x))`) no longer applied to the butler: `phase60_proof_engine`'s butler theorem found no proof. What a problem's descriptions presuppose, `butler(butler)`, is now a premise the engine searches with and a hypothesis the certifier cites, read from one symbol walk (`symbols::SymbolCollector`, which also types the kernel's symbols). A proper name (`Butler`) presupposes nothing, nor does a noun the problem also uses as a relation. A description in the goal or its local context presupposes like one in the premises, and a goal's presupposition lasts only for its own proof. Two faults underneath surfaced with it. The kernel has one namespace, so the noun `butler` and the butler it names could not both be declared, and `butler(butler)` applied the predicate to itself; a predicate that shares its name with an individual is now filed under a name of its own, for the kernel only (`symbols::KernelNames`). And the engine's structural equality had no case for a tensed proposition, so `Past(see(butler))` was not equal to itself and never met its negation: a rule concluding a tensed fact could not be refuted through one of its conjuncts. Equality now matches every expression variant exhaustively, and each is equal to itself: tensed, modal, counterfactual, event, match, term and hole. A new variant does not compile until it says when it is equal. Locked by `logicaffeine_proof`'s `a_definite_description_presupposes_its_noun.rs` and `a_tensed_fact_is_the_same_fact_everywhere.rs`, and `engine::tests`.

- **A rational is the VM's exact number on every backend that compiles a program.** The VM holds a whole rational as the integer it equals: `/` of two whole values truncates; `%`, `//`, the bitwise operators, an index, a range bound and a slice bound take it as that integer; `** -1` refuses in an integer's words; and a fraction refuses `%` and the bitwise operators in its own words. The compiled backends — C, C bytecode, RISC-V, linked WebAssembly, and Rust in debug and release — now keep these rules wherever they hold a rational:
  - a list of rationals: shown, indexed from either end, sliced, walked, searched, grown, popped, filtered, stored into, sorted, nested and passed through a function;
  - a variable that held an integer before it gained a fraction (`Set t to t + r`), which joins as a rational;
  - a record's rational field;
  - an integer handed to a rational parameter or returned as a rational result.

  They printed heap handles or pointer bits, segfaulted, or did not build. A record built without its rational field holds nothing on the VM, which a compiled rational cannot hold, and every compiled backend refuses that construction by name. `rational_number_lock`.
- **A value popped from a list computes as the VM's on every backend.** C bytecode refused, or printed the box's bits for, a sum, product, quotient, order test or accumulation over a popped value; it reads the number now, and stops with the VM's sentence over `nothing`. Linked WebAssembly refused a shown product or sum of reals as big-integer arithmetic. `popped_value_lock`.
- **Linked WebAssembly:**
  - `x += r` takes `+`'s rule, so an integer variable that gains a rational holds the exact value.
  - An integer argument to a rational parameter converts, in a call and in a launched task. A big integer was passed as the raw handle.
  - An integer, a fraction or a value of the exact extension read where a machine integer is taken (an index, a store, a range or slice bound) refuses in the VM's words for that place.
- **The exact extension:**
  - A slice of a list of its values runs on every backend that holds them: AST C, C bytecode, RISC-V, linked WebAssembly and Rust.
  - Its values in a record are read back as they were stored on RISC-V.
- **A linked WebAssembly module carries no debug information.** `--emit wasm-linked` linked the runtime's DWARF into every module — the whole runtime's, around the few functions `--gc-sections` keeps — so a program of eight functions was a 113 MB module that every run read. The link strips it (`--strip-debug`; the `name` section stays): that program is 272 KB, emitted in 1.9 s rather than 7.4 s and run in 26 ms rather than 310 ms. `emit_wasm`, `lent_value_lock`.
- **The tree-walker hands a task its arguments as the VM does.** Each crosses to the task as a copy, and a value that cannot cross — a function, a real number a function of the program defines — stops the program in the VM's words instead of running there alone. `defined_reals`, `tier_parity::semantic`.
- **A specialized function keeps every argument it reads.** The optimizer specializes a function on a constant argument by folding the constant in and dropping the parameter; it dropped it even where the parameter was read inside a form the substitution carries through unchanged — a closure (`Let f be (y: Int) -> y * k.`) or a loop the supertask engine had rewritten — so the program failed with `k` undefined on the VM, C and RISC-V, or its Rust did not build. The constant now reaches every read, or the function is not specialized; a loop is unrolled on the same condition.
- **A full test run no longer fills its disk.** `run-all-tests-fast.sh` and `run-all-tests.sh` refuse to start when the disk holding their `TMPDIR` or their target directory has less than 150 GB free (`scripts/test/disk-headroom.sh`). A run writes tens of gigabytes of test scratch; one that filled its disk failed every later test for a reason not its own, and every other build on that disk with it.
- **A full test run no longer starts beside an FPGA route or a Quartus build.** The fast runner refused to start beside place-and-route, synthesis and emulation, but knew nextpnr only as `nextpnr`, a name no build gives its executable (each architecture installs its own: `nextpnr-xilinx`, `nextpnr-ice40`, `nextpnr-ecp5`, `nextpnr-himbaechel`), and knew no Quartus executable at all, so xc7 routes and a D5005 FIM compile ran unseen. Both full runners now ask one check, `scripts/test/heavy-jobs.sh`, which matches each tool by its family of executable names, never by a command line; `run-all-tests.sh` had no such check. `heavy_jobs_lock`.
- **`min` and `max` of reals answer alike on every tier.** Of two equal reals (`0` and `-0.0`) the answer's sign was whatever the optimizer chose for `f64::min`/`fmin`, so `min(0, -0.0)` printed `0` on the interpreter and `-0` in a compiled program built with optimizations, and the AST C backend let a NaN win when it stood second. `min` now answers the one with the negative sign and `max` the one with the positive sign, and a NaN gives way to the other operand (IEEE 754-2019 `minimumNumber`/`maximumNumber`), on the interpreters, AOT-Rust, both C generators, RISC-V and WASM. `exact_value_model_parity`, `minmax/signed_zeros_and_a_nan`.
- **An exact context reaches comparisons, arguments and powers.** `If r is less than 1/2:` with `r: Rational` compared `r` with `0`, the `1/2` floored; an argument at a parameter declared `Rational` (`tenth(1/3)`) floored too, and so did the base of `**` in a `Rational` context (`(1/2) ** 2` was `0`). A comparison with an exact operand, an argument at a parameter declared exact and the base of `**` in an exact context are now exact contexts themselves. `exact_by_context`.
- **A JIT-compiled loop that can return from inside reports a finished loop as finished.** The register-allocating backend moved a constant write out of a loop whenever no other op in the loop wrote its slot, so the write took effect on every path through the loop. A region's return flag is such a write, made only on the path that returns: once a function like `find`, whose loop says `If n * n = m: Return true.`, was hot, a call whose loop ran out answered as though it had returned, with `false` or `0` in place of the code after the loop. A constant now moves to the loop's pre-header only when its slot is dead at the loop head, counting the slots the VM reads back after the region as read at every exit. `jit_region_in_loop_return_lock`.
- **A condition on an infinite integer's residue reads the same on every run path.** `If n % 2 = 0:` with `n` the reciprocal of an infinitesimal stopped under `largo run` with `` `&` requires integer, boolean, or Set operands ``: the optimizer reads a power-of-two divisibility test as a bit mask, which an infinite integer has none of. The test keeps its remainder wherever the exact extension reaches, so every path says the condition depends on the residue of `⌈1/dt⌉` until a fact about the reciprocal settles it. `nonstandard_surface`.
- **Every path that runs or emits a program runs the same program.** `largo build`, `largo run`, `largo emit rust`, the Studio's interpreter and Rust view, the asynchronous interpreter, the VM's IO runner, `largo verify` and the VM's compiled-native functions now resolve a program before running it, as the tree-walker, the VM and the C backends do: `Let x: Rational be 7 / 2.` printed `3` on them and a supertask never ended. A census lock holds every future seam to it. The first Futamura projection's residual keeps declared types, type arguments and initial fields.
- **A compiled sieve no longer writes outside its flags.** A loop lowering that consumes a counter's `Let` (the fill of a list of flags, a copy, a counted range) declared the counter again in Rust without its type, so the arithmetic after it read the counter as no number at all and compiled raw: in the sieve, `i * i` overflowed past 2^63 for a limit above 3,037,000,499, wrapped negative, and the unchecked store it started wrote outside the flags (a segmentation fault where every other tier answers). Every counter a lowering declares states and keeps its Int type, so the arithmetic on it is checked like any other Int's. `loop_counter_exit_lock`.
- **A call's Int result is exact beside other arithmetic.** A call to one of the program's own `-> Int` functions was typed as nothing in particular, so `f(x) + 9223372036854775807` compiled to a raw machine add: a panic in a debug build and a silently wrapped sum in a release one, where every other tier answers exactly. A call is typed by its declaration, and the arithmetic around it is exact. `bitwise_dual_lock`.
- **An Int parameter takes its argument as a machine word.** Inside an exact chain (`next(x + 10)` under a bit operation or a `Show`), the argument of a call to an `Int` parameter was computed as an exact integer and handed to a machine-word parameter: generated Rust that did not compile. A parameter declared as a machine word is a storage sink, and its argument narrows into it. `bitwise_dual_lock`.
- **A determiner reads the same in every construction it heads.** `Many dogs will bark.` read as `∀x(Dogs(x) ∧ …)`, `No dog stopped barking.` and `No dog does bark.` lost their negation (the second read as `∃x(Dog(x) ∧ Bark(x))`), `Most dogs do not bark.` and `Three dogs do not bark.` read as `∃`, `Any dog chases a cat.` conjoined its restrictor, `Not any dogs will run.` read as `¬∀`, and `Every book was read by no students.` read as `∃y(Students(y) ∧ …)`. Every construction now takes its quantifier and its restrictor's join from one table. `logic_determiner_readings` holds 13 determiners across 11 subject and 5 object constructions.
- **A counter read through an item's index compiles.** `Set total to total + item (v + 1) of arr` with `v` a counter stepped by one: the wide-accumulator optimization missed the read inside the index and widened `v` to a 128-bit integer, and once the item read became `v` itself the generated Rust added it to a 64-bit total and did not build (0.10.1 built it). Every read now counts at any depth. `wide_accumulator_lock`.
- **A loop leaves a counter it reuses where the loop left it.** The compiled tiers lower a counted loop to a `for` over a range and wrote the counter's exit value back only when a later statement of the same block mentioned it. A counter the loop reuses (`Set j to 1`) belongs to an enclosing scope, so a read after an enclosing loop got the value it had before the inner loop: interpreted, a bubble sort's `j` ended at 2, compiled at 0. The range loop, a tiled nest, a copy or a slice, and a run continued from an earlier loop now write back any counter their block does not declare. A sequence copied into itself by a counted push loop compiles; it emitted Rust that read the sequence before declaring it. `loop_counter_exit_lock`.
- **Machine words mean the same on both C backends as on the VM.** A parity lock had been comparing nothing on its isolated rows (see below); run for real, bytecode C failed to wrap a word sum in a loop, took a word's remainder signed, refused a big integer handed to `word8(...)`, word map keys, word sets and an aligned word in a text, ran an ordering of two word widths, and dropped every statement after `"w=" + w`; AST C read that join as pointer arithmetic. `c_word_lock`.
- **A tier-parity sweep can no longer agree with itself.** A suite that declared no `tier_parity_worker` re-executed its test binary into nothing for every isolated backend, the reference included, so every such row ended without a verdict and matched every other: `c_word_lock` and `c_set_iteration_lock` compared nothing. `tier_parity::sweep` and `run_isolated` now refuse a binary that lists no worker, naming the declaration to add.
- **A remainder is no longer taken as non-negative.** `x % 8` keeps the sign of `x` (`-7 % 8` is `-7`), but the compiled tiers' non-negativity proof counted any remainder by a positive literal as `>= 0`, so `r % 4` and `r / 2` of such a remainder became `r & 3` and `r >> 1`: wrong for a negative `r` on AOT-Rust and both C backends. `strength_reduce_pow2_lock`.
- **Bit operations on a big integer mean what the reference says on AOT-Rust.** `&` of a non-negative big integer and a low mask takes its low bits, and any other big operand of `&`, `|`, `^`, `<<` or `>>` is refused in the reference's words; a promoted operand under `&` no longer leaves generated Rust that does not build. `exact_bitwise_lock`.
- **A RISC-V program holds only the memory it can still reach.** The RISC-V runtime allocated every value and never freed one, so binary_trees at depth 16 reached 1.18 GB under qemu and failed in the corpus differential's 512 MB address space. Every block now comes from the collector the C tiers use, emitted ahead of the RISC-V runtime, and the task scheduler hands it each stack it switches to; a record, a tuple and an enum value are one block, their cells after the header. The collector takes its 64 KiB blocks from the system 64 at a time, each mapping cut to a block boundary, so a heap's address space is its blocks' (glibc padded each block to its alignment, nearly doubling it), and reserves its marking stack when it starts, so a collection allocates nothing until marking outgrows it. binary_trees at 16 holds at most 157 MB and prints the VM's output. `riscv_heap_lock`, `riscv_aot_lock::riscv_matches_the_vm_on_the_benchmark_corpus`.
- **`largo test --browser` waits for a page's handler as long as the page runs it.** A click or a typed key returns only once the page has handled it, and the harness gave it the 20 s any protocol command gets, so a handler that did real work timed out on a loaded machine (`browser_navigation_lock`, under the full suite). Input and the harness actions that fire it now wait up to 300 s; every other command keeps 20 s.
- **A counted range means the same on the C backend as on the VM.** `Repeat for t from a to b` counted with `t <= b; t++`, so a range ending at the largest Int ran forever, and a body that set `t` changed the count (`Set t to t + 10` showed `11` once where the VM shows `11`, `12`, `13`). The count is hidden and stops at its end, and each pass binds `t` afresh; a range used as a value stops at its end too. On AOT-Rust such a body did not build: the loop's binding is `mut` when the body sets it. `repeat_range_lowering`.

- Machine words run on AST C as on the VM. `word8` through `word64` values wrap `+ − ×` at their width, divide, order and print unsigned, shift by a count modulo the width, and convert back through `intOfWord8`–`intOfWord64`. Mixing a word with an `Int`, or two widths, is refused with the VM's words.

- AST C keeps a machine word a word inside a collection. A `Seq`, `Set` or `Map` of words holds each as its bits and reads it back as the word it was, so SHA-256 over a `Seq of Word32` matches the VM again. A list, set or map of `Word64` shows its elements unsigned, and a loop over one binds each element as a word.

- A loop over a `Set` on AST C visits each element once, in the order it was first added, as the interpreter does.

- AST C lowers `min` and `max` over unbounded integers exactly, and holds a remainder by a divisor that fits a machine word in an `int64_t`. A countdown loop that steps its counter mid-body runs on a machine-word counter on C, as one that steps last does.

- `x is none` and `x is nothing` compile to one absence test on the VM and every backend that reads its bytecode, so WebAssembly no longer refuses a comparison against `none`. A WebAssembly variant prints its payload from the standalone enum layout.

- Standalone WebAssembly roots a batch function's live handles only where a safepoint can run, and exact integer arithmetic outside a loop calls one shared helper per operation. Modules with exact integers shrink.

- The CUDA device lift keeps a runtime function reachable when only a macro calls it, a documented macro included, and routes allocations through its shims without splitting a multibyte character.

- `largo opts` reports a program a backend rejects at its mistake, with the file, line and caret every other command prints.

- **A program that shows a line at a time no longer pays a system call per line.** `std::io::Stdout` is line-buffered whatever it is connected to, so the compiled Rust made one `write(2)` per `Show` and `largo run --interpret` printed its lines one call each; 100,000 shown lines were 100,000 writes. Both now buffer 64 KiB on a pipe or a file and write a line at a time to a terminal, the convention the C tier's stdio already kept. Output keeps its place: it is written before a failure's reason, before input is read, before a program's own Rust writes, and at exit. `show_buffering_lock` counts the writes with `strace`.
- **A Real shows the same on every WebAssembly host as on every other tier.** The browser runtime and the node shim formatted a float to six fixed decimals: `1.0 / 3.0` showed as `0.333333`, `0.0000001` as `0`, `10^21` as `1e+21` and `-0.0` as `0`. Both now carry one definition, `web_host::FMT_F64_JS`, the display `fmt_f64` defines: the shortest digits that read back to the same bits, never scientific notation, and an exact tie spelled as Rust spells it. `wasm_float_display_lock` holds both hosts to `fmt_f64` over 120,024 doubles and a compiled module to the VM.
- **`Simultaneously:` binds every task's value on the Rust tiers.** A block of three or more tasks spawned threads and threw their results away, so any `Let` among them did not compile, and a `Let` beside another kind of task bound the whole pair. Every block is now one balanced tree of `rayon::join`s on the pool: each `Let` binds its own value, any other task binds nothing, and a literal task states `i64`. `e2e_concurrency` pins it.
- **An `Int` that outgrows a machine word is exact on WebAssembly**, as on the VM, the tree-walker, Rust and C: the module carries a tagged word (the integer while it fits, a handle to the host's integer when not), the hosts compute on held integers with the reference tier's arithmetic, and a program whose integers all fit a word compiles to the module it did before. The exactness decision is one, shared by every compiled tier (`analysis::exactness`): it follows a value through a global and a swap, and AST C asks it too, where it printed a wrapped word for recursions, doubling sums, Fibonacci-like cycles and program bindings a function grows.
- WebAssembly record fields preserve unbounded integers across reads, writes, copies, calls and display; counted integer cells reclaim host values and storage.
- Nested mutable calls in WebAssembly preserve integer, float and heap values across temporary ownership clears. C exact floor division releases its internal quotient and remainder storage.
- **An `Int` bound to integer literals is an `i64` in the generated Rust.** A binding or loop counter whose type only literals decided took rustc's `i32` default: `Repeat for i from 1 to 50000: Set x to x + i * i.` summed wrong in a release build and failed in a debug one, and a counted loop ending at 2147483647 did not compile. The emitter states the type the analysis inferred: such a `Let` declares `: i64` and such a range spells its end `_i64`. `int_representation_lock` pins it.
- **Statement operands keep their evaluated values.** Later mutable calls cannot change an earlier index or value. Nested mutation keys run once from the root outward, and optimization preserves mutable argument places and their effects on subsequent reads. `statement_snapshot_lock` pins the behavior across the interpreter, VM and optimized VM.
- **Mutable arguments return to their original place.** The VM and interpreter preserve nested records and collection slots while a mutable argument is lent, then write the final value back without changing saved aliases. Binary ISA version 2 carries separate return-slot and ownership-proof metadata; regenerate version 1 artifacts. Hot native calls also return mutable scalar and list values through nested calls and precise deoptimization, preserving aliases.
- **Expression values survive later mutable calls.** The VM preserves earlier operands, map keys, and closure callees while subsequent operands replace their original bindings.
- **Every backend answers the same on what an optimization must not touch.** A function that changes a program variable, directly or through a callee, is pure on no path: one definition (`optimize::purity`) gates memoization, compile-time evaluation, the symmetry rewrite and the Rust tier's pure-function set, so a recursion that counts its calls prints the same count everywhere. Equality saturation no longer folds a variable across a call that lends it to a `mutable` parameter. `purity_lock`, `in_out_lock`.
- **The C tiers and WebAssembly keep a `mutable` parameter the caller's variable, and C evaluates operands in LOGOS order.** Both C backends pass in-out parameters by reference and store them back, and a WebAssembly function returns them after its result; a lent element or field is stored back through the keys it was lent with; an in-out integer that outgrows a machine word stays exact; an operand that writes what a sibling reads runs in source order. `in_out_lock`, `c_operand_order_lock`, `in_out_unrolling_lock`.
- The VM compiler checks AST depth before recursive analysis, returning a diagnostic for deeply nested programmatically constructed expressions instead of overflowing the host stack.
- Certified SAT checks a model obtained by root unit propagation before starting symmetry discovery. Every original clause must accept that model; unresolved or conflicting formulas retain the certified search path.
- **The optimizer decides a branch after a loop only from what holds after it.** The range oracle's dead-branch elimination (`Opt::Oracle`) gave a loop counter its pre-loop value narrowed by the exit condition: after `While i is less than n:` with `n` read at run time, `If i equals 0` took its first branch on every engine `largo run --interpret` drives, a literal bound left the counter an empty range so `If i is less than 10` ran as well, and an exit by `Break` counted as an exit by the condition. A variable a loop writes is now unknown after it, narrowed by the negated condition only when no `Break` of the loop's own can end it, and the facts the backends read keep every value the iteration that broke out wrote. The generated Rust gave a counted `While` a `Break` can end its bound as the counter's value after it (`for i in 0..100` then `let mut i = 100;`); a loop a `Break` of its own can end is no longer a counted loop for any pass.
- WebAssembly preserves Word32 and Word64 values through copies, record fields, indexed writes, appends, and value parameters.
- Specializing a function preserves writes through mutable arguments, including parameters whose positions change and mutable temporary arguments.
- Optimized functions retain writes through mutable arguments, including forwarded arguments and text values.
- Repeated parsing, interpretation and compilation release the vectors and strings owned by their AST nodes. Arena reset drops each value while retaining storage for the next evaluation.
- Loop tiling preserves which bounds failure occurs first when rows have different lengths. Optimizer fact scopes stay on their creating thread and end with their compile. Covered by `statement_tier_totality_lock` and the optimizer facts units.
- RISC-V structural equality preserves the VM’s identity rules for lists and its field comparisons for records, tuples and enums, including NaN. Numeric elements compare exactly across Int, Rational and Float layouts, including subnormal and non-finite Float values. Declaration discovery retains names such as `First`. Locked by `riscv_equality_lock`.
- **An integer becomes a Real by one rounding, on every tier.** `BigInt::to_f64` rounded once per 64-bit limb and the C runtime's `lbig_to_f64` once per base-10^9 limb, so an integer past 2^64 meeting a Real (`n + 0.5`, `sqrt(n)`) missed the nearest float for 1.5% of random 64–200-bit integers on the tree-walker and the VM and for 39% on C, on different integers. Both now take the top 64 significant bits with a sticky bit and round once: the nearest binary64, ties to even, infinite once the rounding reaches 2^1024. `c_bigint_to_f64_lock` holds the C runtime to `BigInt::to_f64` over random 64–4096-bit integers of both signs and exact halfway cases.
- **`largo verify` verifies the program, and the CLI's `verification` feature builds.** In 0.10.1 `largo verify` read the entry into a discarded variable, asked Z3 whether `true` is valid and printed "Verification passed"; it now runs the Z3 static-verification pass, escape analysis and the type checker over the assembled project (`compile_to_rust_verified`) and fails on an embedded refusal, and `largo build --verify` takes the same path. `logicaffeine-cli`'s `verification` feature forwards to `logicaffeine-compile` and `logicaffeine-proof`, so a CLI built with it compiles. `feature_forwarding_lock`.
- **An installed `largo` builds natively outside a checkout.** The generated project path-depended on runtime crate sources staged from a workspace checkout, so an installed `largo build` or `largo run` died with `Could not find workspace root`; outside a checkout the generated manifest takes `logicaffeine-data` and `logicaffeine-system` from crates.io at the compiler's own exact version, and the single-file and project builds share that one decision (`RuntimeCrateSource`), down to the Linux section that gives a compiled server the io_uring engine. Inside a checkout the staged path dependencies stay, the workspace `Cargo.lock` is seeded, and hot-swap builds use their own target directory, so an inherited `CARGO_TARGET_DIR` no longer leaves the native tier unloaded.
- **The published crates build from crates.io, at the Rust they declare.** `logicaffeine-lexicon`'s `dynamic-lexicon` feature embedded a file from the language crate's directory, outside its own package, so the feature docs.rs builds could not compile from the registry; the vocabulary lives in the lexicon crate. `rust-version` declared 1.75 while published code needed 1.82 and the locked dependency graph 1.88; it is 1.88, the oldest toolchain that builds every published crate, and CI checks it. `packaging_lock`.
- **Optimizer passes forget what a statement and its callees write.** Common-subexpression elimination, loop-invariant code motion, constant propagation, the supercompiler, SCCP and abstract interpretation read one exhaustive write model (`optimize::writes`): a pre-loop `item p of xs` is not reused inside a loop that advances `p`; a binding a called function assigns is not folded to its pre-call value (`Let counter be 0. bump(). Show counter.` showed `0`); a later `Let` rebinding a name, a `Set` inside a match arm, a `Pop … into` and a lend to a `mutable` parameter count as writes (`Let x be 1. Let x be length of args() + 3. Return x.` returned 1); and a collection length a loop grows is widened rather than folded as empty. `optimizer_write_effects_lock`, `cse_loop_soundness_lock`, `optimizer_soundness_lock`, `oracle_loop_length_soundness`, `supercompile_store_lock`, `bootstrap_scope_lock`.
- **The optimizer reads the names the source reads.** An inlined callee no longer captures a same-named local of its call site (`## To peek -> Int: Return x.` inlined beside `Let x be 7` answered 7 on `largo run --interpret`), a helper that writes a program binding is not hoisted out of the expression that reads it, loop peeling and unswitching keep a block whose names later statements read, constant propagation forgets a name's literal where a binding shadows it (`Let i be 5. Repeat for i from 1 to 2: Show i.` showed `5 5 5`), dead-branch elimination keeps a known branch's bindings inside the branch, `s + 0` on a Text keeps its `"0"`, and an array read from the end stays a runtime sequence. Every inliner and splicer asks one module, `optimize::capture`. `optimizer_name_capture_lock`.
- **`## No <opt>` reaches every tier.** `largo build` and `largo emit` compiled with a bare environment configuration and dropped a file's `## No`, and the C, VM, WebAssembly and ISA seams read the all-on default (`## No fastdiv` still produced `MAGICDIVU`); every seam installs the file's decorators. Output is unchanged; speed can move. `decorator_through_largo`.
- **`Launch a task to f with x and y` passes two arguments, and a launched task's failure fails the run.** The launch statement read `with ch and 42` as the one argument `ch and 42` and refused a function whose name lexes as another word class; both launch forms read the target and the arguments as `Call` does. On the tree-walker a launched task's error vanished and the run succeeded; the run's one error sink is shared with every task it launches, the first failure is kept and the other tasks still run to quiescence, as on the VM. `launch_arguments_lock`, `launched_task_failure_lock`.
- **A `Select` that names one pipe in two arms no longer takes that pipe's next value.** A woken select's registration on the waking pipe outlived it and resumed the task inside an unrelated `Receive`, and `Close` woke such a select twice; the scheduler withdraws every registration, on the tree-walker and the VM alike. `select_same_pipe_twice_lock`.
- **A record field is named by its position.** A field named `who`, `what` or `whom`, an ordinal (`first`, `second`), `with a Int and b Int` (which declared a field named `Int`) and the concise `a: Int.` dropped the field from the registry without a word, so the compiled tiers refused a program the tree-walker ran; the word before `:` or the `,` of `name, which is Type` is the name, the first word of each field in a `with` list is the name, and an article is itself the name exactly when skipping it leaves no field. A field typed `Result of Int and E`, `Pair of A and B` or `Triple …` keeps every parameter. `record_field_name_position_lock`, `wasm_result_payload_lock`.
- **A program's own variant wins over a standard-library variant of the same name, however it is spelled.** The prelude counted a variant only after an article, so an `Outcome` written `Failure (message: Text).` did not claim `Failure` and `a new Failure` built the library's `Severity`; every indented line of an `is either:` block names a variant, and a mismatch names both user types (`expected Severity, found Outcome`).
- **`Float` is `Real`, and a numeral the surface cannot value stays symbolic.** `Let x: Float be 3.14.`, `Let r: Float be 1.` and `Let rate: Real be 3.` were refused; they are accepted, and a whole-number initializer to a `Real` slot is stored as a real (`Show rate / 2.` shows `1.5`). In logic mode `taller than 99999999999999999999` and `99999999999999999999 dollars` parsed as 0; the lexeme and its unit are kept. `state_field_type_lock`, `float_real_unify`.
- **The lexer reads camelCase and multibyte words.** A camelCase identifier ending in `-ly` (`bOnly`) no longer lexes as an adverb, and a doubled multibyte letter no longer underflows comparative and superlative stemming. `call_statement_word_class_lock`.
- **The tree-walker and the VM agree on files, recursion, words and variants.** `Write … to file`, `Mount … at` and `Read … from file` run on the VM against the tree-walker's sandboxed filesystem (0.10.1: "VFS not initialized"), and `file.lg`'s `read` and `write` run on both engines instead of answering `nothing`; the VM reclaims scratch registers at every statement boundary, so a long straight-line program no longer exhausts its register frame; the tree-walker recurses as deep as the VM instead of overflowing near 50 calls, and resolves a definition written below `## Main`; `word16(65535) + word16(3)` is 2 on both, as in a built binary; a variant's named fields bind by declaration however the `with` clause orders them; an unmatched `Inspect` names the value's variant and type in the same words on both; the VM falls back to exact division for a big-integer dividend instead of panicking; a function whose only use of an outer name is `Set x to …` reaches the program's binding on the VM; and a `Sleep` with no reactor sleeps on the thread. `phase_vm_error_parity`, `file_io_cross_tier`, `vm_register_recycling_lock`, `treewalker_recursion_headroom_lock`, `word_narrow_all_tiers`, `inspect_unhandled_message_lock`, `bignum_strength_reduction_lock`, `vm_global_promotion_lock`.
- **Nested writes land where the sentence says, and a record copy owns its lists.** `Push 5 to a's b's c`, `Pop … from item i of grid` and `Set a's b's n to …` failed on the tree-walker with an "identifier required" error, `Set item I of r's f to v` silently changed nothing on the VM, and a push to a list field of a copied record reached the other copy; writes go through any field or item path, copying only the shared collections on the path, and a `mutable` argument naming a place is lent and given back. `vm_place_mutation_lock`, `mvs_record_param_lock`, `correctness_place_lvalues`.
- **`SharedMap from K to V` builds, runs and converges.** In 0.10.1 it compiled to an `ORMap<String, i64>` that did not build (an `i64` has no merge) and the interpreter had no map CRDT; it now has add-wins keys and Lamport-ordered last-writer values, index reads and writes, `contains`, `Remove` and key-ordered iteration, converging identically on the tree-walker, the VM and a compiled binary. `shared_map_all_tiers`, `crdt_backend_lock`.
- **ML-KEM and ML-DSA run on the tree-walker and the VM.** `crypto.lg`'s `## To native` kernels (ML-KEM NTT, sampling and serialization; ML-KEM and ML-DSA keypair, encapsulation, decapsulation, signing and verification) returned `Nothing` on the interpreters while a built binary was correct; one table of native kernels serves the tree-walker, the VM and AOT-Rust with append-only codes that travel in bytecode, and C, RISC-V, WebAssembly and the soft core refuse a reachable kernel call by name. `native_kernel_lock`, `native_kernel_reach_lock`.
- **A WebAssembly function with more than 64 basic blocks no longer traps.** The dispatch loop wrote block indices with the unsigned LEB128 encoder where `i32.const` is signed, so indices 64 to 127 decoded negative and a large function fell into its epilogue's `unreachable`; every block-index write goes through one helper, and a source lint forbids the unsigned encoder for constant immediates. `wasm_large_function_lock`.
- **The native tier keeps lists sound.** List typing is flow-sensitive per instruction, so a register the bytecode compiler reuses no longer plants a list from an `Int` (a quicksort SIGABRT); a store into a list that may be a caller's copies it first, as the VM does; every reachable instruction has a deopt row, so an `Int` overflow inside a list-parameter function resumes instead of panicking; aliases of one native list re-box to one shared value; a list parameter's length is re-derived at a region call's entry, where a hot `length of xs` read a stale slot; and a region that reads a list and can return early declines to the VM. `jit_register_kind_reuse_lock`, `jit_list_search_differential_lock`, `jit_forge_pin_length_gauntlet`, `jit_forge_random_fuzz`.
- **The kernel refuses closed proofs of `False` that 0.10.1 accepted.** A raw `Context::add_definition` inserted its body unchecked (`evil : False := Zero`); redefining a definition a theorem was proved about, adding a constructor after exhaustive elimination, or a constructor over a definition's name left the dependent certified for a statement it no longer stated; a definitional alias (`Definition Negate := Bad -> False`) walked around strict positivity; a failed literate tactic proof registered a false theorem, and a document ran on past a false block; the term parser checked a prefix (`Check Nat -> Nat garbage` checked `Nat -> Nat`); `Float` literal equality was IEEE `==`, not reflexive at NaN and equating `0.0` with `-0.0`; and a term past the native stack aborted the process. Each is refused, compared by bit pattern or stopped at a depth limit. `registration_soundness`, `stable_meaning`, `constructor_blind_positivity`, `literate_proof_soundness`, `term_parser_total_lock`, `W_perf_extra`, `term_display_depth`.
- **Proof automation no longer calls true goals false.** `decide` divided by truncation and got stuck at a zero divisor; it divides Euclidean-style, totally and at arbitrary precision as both kernels do, and a goal evaluated false is reported false rather than undecided. Integer literals past `i64` are numbers in every proof layer (`9223372036854775808 + 1 = 9223372036854775809` is provable), a theorem may mention a free variable (registered as a kernel local typed by its use, so the re-checker no longer disagrees over a valid proof), a development's definitions certify in any collection order, and a tactic cannot report a proof the trust door refuses. `definition_registration_lock`, `T_tactics_extra`.
- **Wire decoding is bounded.** Every decompressor stops at 256 MiB, lz4 at its claimed size, and an untrusted field count no longer preallocates beyond the remaining input. `never_panic_lock`.
- **Every web app route renders in production.** The deploy's split WebAssembly build corrupted the framework's edit stream and blanked every route but `/studio`; the deploy ships one eager bundle (22.0 MB, 12.6 MB in 0.10.1, budgeted in `.config/size-budgets.toml`), the shell keeps the pre-rendered page visible whenever the live app is not painting, and each deploy renders every route and refuses a page with an empty `#main`. A deeply nested term no longer overflows the browser's WebAssembly stack, and the landing page shows the Socratic diagnostic for a compile error. `ssr_render`.
- **A specialized clone reads its caller's arguments in the caller's scope.** The partial evaluator classified each argument of a call against the callee's parameter bindings, so a caller variable spelled like a callee parameter that the same call binds to a literal took that literal: `randBeta`'s `randGamma(seed + 7919, beta, 1.0)` drew its second gamma at shape `1.0`, and `randBeta(42, 2.0, 3.0)` printed 618 on the C tier, on AOT-Rust and on `largo run --interpret` where the tree-walker prints 201. `optimizer_name_capture_lock` holds the shapes on every optimized tier.
- **The C tier runs `json.lg`, and an integer past `i64` stays exact through every slot it reaches.** A value the C tier carries as the tagged `LInt` was handed to `int64_t` parameters and stored in `int64_t` fields, and the program did not build; one whole-program pass decides which function returns, parameters, record fields and variant fields are `LInt`, and emission declares each body's `LInt` locals from the same computation, closures included. An empty `Seq` of a user enum is built by its own sequence family, `Set item … of` replaces an element of a sequence of records and writes through a record field's sequence, and `jsonValid`, `jsonParse` and `jsonSerialize` print on C what the tree-walker prints. `c_json_shapes_lock`.
- **`logicaffeine-tests --features capnproto` builds.** The Cap'n Proto head-to-heads build the competitor's messages with the `capnp` runtime crate, which the feature now brings in beside the wirebench schema.
- **An index names the same item on every tier, and so does every slice and every loop the optimizer copies in one step.** One rule, `logicaffeine_data::logos_offset`: 1 is the first item, a negative index counts from the back, and 0 or anything past either end names nothing. AOT-Rust's fast paths for copy, fill, drain, reverse and swap loops, fixed-size arrays and tiled nests indexed around it, so an index the interpreter refused could read another item, stop with Rust's own message, or leave a loop counter where the loop never leaves it. They now go through the rule, and an index the compiler proves names an item still compiles to an unchecked or plain indexed access. `statement_tier_totality_lock`.
- **A failure reads the same on every tier.** `logicaffeine_data::failure` holds the sentence for every runtime failure the engines report (division and modulo by zero, an index or key that names nothing, quantities, money, modular arithmetic, UUIDs, timestamps, time zones, `decimal(…)`), and the tree-walker, the VM, generated Rust and the WebAssembly Node host all report it. A compiled program stopped by a failure prints its reason alone and exits 1 under either panic strategy. `statement_tier_totality_lock`.
- **`decimal(…)` means one thing everywhere.** It reads Text, widens an Int or a big integer exactly, keeps a Decimal, and refuses a Real, a Bool or a Char in one sentence. AOT-Rust read any argument through its text, so `decimal(2.5)` succeeded compiled and failed interpreted, and a big integer did the reverse. `statement_tier_totality_lock`.
- **A sequence stored in 32-bit slots keeps every value the program puts in it.** AOT-Rust narrows a `Seq of Int` to `i32` slots when it proves every stored value fits. A sequence exchanged by `Swap` with one holding wider values, or written inside a zone or another statement the proof did not classify, kept the narrow slots and lost the high bits; sequences a `Swap` exchanges now narrow together or not at all, and every statement that names the sequence is judged. `statement_tier_totality_lock`.
- **Two specializations of a function never share a name.** The partial evaluator named a clone by its constant arguments, so `f(15.0)` and `f(15)`, `f(-0.0)` and `f(0.0)`, or a clone and a function the program declares could collide and run one body for both calls; every name is now distinct. `statement_tier_totality_lock`.
- **Every `largo` command that runs or emits a program checks it the same way first.** `largo run --interpret` and every `largo emit` target pass the program through one front end, so a program that does not parse, or reaches a module it never imported, is reported once in the same words before any backend sees it; a diagnostic names its file relative to the directory `largo` runs in. `largo run --interpret` runs the program on the reactor `Connect`, `Serve` and a timed `Sleep` need, and announces a replay recording only to a terminal, so a piped run's stderr is the program's reason alone.
- **`items lo through hi of xs` is a slice whatever word starts it.** `items` followed by anything but a number or a parenthesis was read as a variable named `items`; a single word before `through`, or `length of …`, now starts a slice as a number does. `statement_tier_totality_lock`.
- **The AST→C backend (`compile_to_c`, `largo emit c`) runs the programs it compiles as the VM does, byte for byte on stdout, stderr and exit status, without copying or allocating per call or per read.** Values keep their semantics without copies on every call: a caller copies a list, map or set only for a parameter the callee changes while the caller still reads the argument (sharing classes and liveness), so `Set arr to siftDown(arr, …)` hands the buffer over; heap_sort at 2.5M and quicksort at 3M run in 21–25 MB where they crashed at 4 GB. Texts append in place and join in one allocation; `item k of` a text reads by UTF-8 character, from the end for a negative `k`, allocating nothing. Integers stay exact: tuple and named-record returns carry an exact component, function values and lifted closures are exact, and a fold kept in range by `& c` or `% c` runs on machine words where the operands' ranges prove it and with a checked step and a cold exact fallback where they do not, never wrapping silently. Maps and sets probe by mask, keep order and every entry through `Remove` (now also on maps and on record fields), and take exact keys; `Swap` works for every kind; an `Inspect` with no arm fails in the VM's words; a list a counted loop fills is sized once before the loop. Locked by `c_ast_vm_parity_lock` (VM reference pinned, C built with the release flags, memory and allocation caps for the scaling rows) and a corpus lock that no generated C holds a placeholder or an `#error`.
- **A loop nest runs in tiles only where no run can tell the order apart, and a loop's index checks are lifted only under a guard every tier evaluates alike.** AOT-Rust tiled every triple nest shaped like a matrix product, so a nest that folded into one variable or stored a running count printed a different answer than every other tier; and a loop over a row of a flattened matrix (`item (i * n + j + 1) of c`) asserted its index range in Rust of its own, failing with a sentence no other tier prints. The optimizer now decides whether a reorder is observable (Fourier–Motzkin over the nest's accesses, the Wolf–Lam condition) and runs such a loop as `If P: <the fast clone> Otherwise: <the loop as written>`, where P bounds every name it reads first, then checks every access's corner indices (and, for a reorder, every divisor) once, and cannot itself fail. An index that leaves its sequence fails where the VM fails; the tiled matrix product and the unchecked row loops keep their speed; a Real product stays bit for bit, since each item's own updates keep their order. `## No tile` turns tiling off and `## No oraclehints` the bounds versioning. `statement_tier_totality_lock`.
- **A list written with items of different kinds no longer prints wrong values on C.** Both C backends held such a list in its first item's element type: `[sqrt(2.0), 1, 7]` printed the integers' bits as reals on the bytecode lowering and divided `7` by `2` as reals on the syntax-tree one, `[true, 1]` printed `[true, true]`, and a text after an integer crashed. Each now refuses such a list by name at generation (the forms of one integer stay one kind, and a list holding exact values is the value runtime's own). `c_mixed_list_lock`.
- **Linked WebAssembly compares rationals and adds onto one in place.** `--emit wasm-linked` refused `q < 1 / 2` on a `Rational` ("comparison of rational values") and `Set q to q + 1 / 3` ("arithmetic on a non-numeric value"), so a loop bounded by a rational, or summing one, never compiled. A comparison over a rational now orders both sides exactly through the runtime (an integer side widened, never through a float), and an in-place sum of any number the linked runtime holds lowers as the sum it names. `wasm_linked_rational_lock`.
- **A list lent to a `mutable` parameter no longer changes the copies others hold, on the C bytecode walker.** The walker changed a lent value in place however many places held it, so after `Set xs to v` (a global), `Let b be v`, `a new Box with items v` or `[v, [9]]`, a later `Push 7 to v` showed up in every one of them; the VM, the tree-walker and the AST C kept each its own. A lent value now starts as the function's own (the call site isolates it) and is copied before it changes once another place shares it; its final value still goes back to the caller. `lent_value_lock`.
- **A `mutable` parameter hands its value back to the caller on RISC-V.** `Call grow with a` left `a` as it was however `grow` changed it (`[0]` where the VM prints `[0, 1, 2, 3]`): a function now hands each `mutable` parameter's final value back beside its result, and the call stores it where the VM keeps it for the copy-out. `lent_value_lock` runs on RISC-V too.
- **RISC-V shows nested values and reads date and span literals.** `Show` of a list of lists, a tuple, or a map of lists was refused ("Show of this composite value"); every value of a shape the shim models now prints as the VM prints it. A date, moment or span literal (`2026-01-01`, `2 weeks`) was refused ("LoadConst of non-scalar constant"). `lent_value_lock`, `exact_on_riscv_lock`.
- **A C map whose values no fixed family holds stops on a missing key.** `item "zz" of m` on a `Map of Text to Seq of Int` printed an empty list on the AST C backend, where every tier stops; it now stops as the fixed families do. `map_missing_key_traps_lock`.
- **An exact value a function stores into Main's binding is the binding's value on every compiled backend.** `Set x to dt` inside a function, where Main had set `x` to `3`, left both C backends reading `x` as the integer it first was: `x + 1` printed a pointer plus one. The reach analysis now gives a name a function assigns without binding it to the program's root binding, the C walker types a global any region stores an exact value into as one whatever its first store held, and linked WebAssembly, which refused the store, re-plans such a global as one and converts a machine value stored into it (it had read the width difference as an ownership clear and stored a null). Linked WebAssembly also copies an exact list for `copy of` (refused). `exact_on_c_lock`, `exact_on_wasm_lock`, `exact_on_riscv_lock`.
- **Every device tier refuses a program the exact extension reaches, by name.** The Tenstorrent tier and the Silicon ISA emitted an artifact for one, a kernel calling a runtime a Tensix core cannot reach and an instruction listing of builtins the chip has none of; the analog, Plasticine and RDU tiers, the soft core and the fabric refused it for some op they do not lower. Each now says `the <tier> does not run the exact extension`, as the CUDA tier did, and passes a program the extension never reaches. `exact_on_devices_lock`.
- **An exact quotient of integers is a fraction on the AST C backend wherever it stands.** `half(0 - 1 / 3)` with `half` taking a `Rational` emitted `half((-1LL / 3LL))`, which did not compile, and an exact division inside other arithmetic computed an integer quotient; it is now the fraction it denotes, and a big integer joins one exactly. A list literal bound to a `Seq of Rational` holds its items as fractions (`[1 / 2, 3]` emitted a list of machine integers the declaration could not take). `c_ast_vm_parity_lock`.

### Security

- **ML-DSA-65 refuses a context longer than 255 bytes.** `mldsa::sign` and `verify` wrote the context's length as
  one byte and never checked it, so a context of 256·k + n bytes was encoded as the n-byte context C[..n] over the
  message C[n..] ‖ M. A 256-byte context aliased the empty one: a service that reserved the empty context signed under
  it for any caller, and a signature on (c, M) verified for (c ‖ M[..256], M[256..]). FIPS 204 returns ⊥ there; `sign`
  now returns `SignError::ContextTooLong`, `verify` answers false, and `mldsaSign` stops with `mldsaSign(): the context
  is 256 bytes; FIPS 204 allows at most 255` on every tier. Affected `logicaffeine-system` 0.10.0 and 0.10.1.
  Wycheproof's `mldsa_65_verify_test` (210 cases) and ACVP's sigGen and sigVer under contexts now run against the
  kernel in `pq_official_vectors`. Reported by Bhabani Sankar Das ([@007bsd](https://github.com/007bsd)) under
  coordinated disclosure; thank you.
- **The one-time pad authenticates.** A PNP receiver keyed each cover's one-time MAC from the pad at the offset the
  frame claimed, so whoever knew one cover's plaintext knew that stretch of keystream and could withhold the cover and
  send one placed over it, keyed by keystream it knew: a forgery that authenticated at every position. Keystream and
  MAC keys now come from opposite ends of each half, a frame names its key slot under its tag, and the receiver keys
  the MAC only from the key region. `Connect … with pad` and `Listen … with pad` kept their cursor in memory, so every
  run, and every activation in one run, encrypted from the bottom of the pad again — a two-time pad; each session now
  keeps a durable ledger beside its pad (`<pad>.<role>.ledger`), which reserves under a file lock and records before a
  frame exists, syncs its directory after a rename, refuses a ledger it cannot read rather than starting over, and
  keeps the receive half's high-water key slot, so a cover opened before a restart is refused after it.
  Affected `logicaffeine-compile` 0.10.0 and 0.10.1; a pad such a version used for more than one run, or whose traffic
  an attacker may know any plaintext of, should be retired. Found in an internal audit.
- **A PQ channel frame opens once.** `PqSuite` opened any authenticated frame as often as it arrived, though
  `ActiveSession::open` promised to refuse replays; it now keeps an RFC 4303 anti-replay window of 1,024 sequence
  numbers, checked and set under one lock after the tag verifies, and refuses to seal a 2^64th frame.
  Affected `logicaffeine-compile` 0.10.0 and 0.10.1. Found in an internal audit.

## [0.10.1] - 2026-07-11

### Fixed
- **Prebuilt `largo-full` Linux binaries now run on stock glibc.** The Z3-static (`--full`) binaries are built with gcc-13 (required for Z3 4.16's C++20 `std::format`), whose libstdc++ tags symbols `GLIBCXX_3.4.31` — absent on stock ubuntu-22.04 and older — so the dynamically-linked full binary died at startup with `GLIBCXX_3.4.31 not found` (v0.10.0's `largo-full-linux-x64` and `largo-full-linux-arm64`). The full Linux builds now statically bundle libstdc++ (a library-search dir holding only `libstdc++.a` forces the linker's `-lstdc++`, emitted by z3-sys, to resolve to the static archive), so the binary self-contains its C++ runtime and runs on any glibc; a release-time assertion (`Assert no dynamic libz3 / libstdc++`) locks it so a dynamic libstdc++ can never regress back in. The lean binaries and all published crates were unaffected.

## [0.10.0] - 2026-07-08

### Fixed
- **Web app: query parameters survive router boot — studio deep links work.** The Dioxus router's startup normalization re-serializes the matched route and replaces the browser URL, destroying any query string the route type doesn't model — so `/studio?file=…` share links and refreshes fell back to the default example, and the same wipe silently broke Stripe checkout success (`?session_id=` license activation), the registry GitHub OAuth callback (`?token=&login=`), and `/news?tag=` filters. All four routes now declare their parameters as typed query segments (`#[route("/studio?:file")]` etc.) and the pages receive them as props; the raw `location.search` scraping is gone and forbidden forever by a source ratchet (`query_scraping_is_forbidden`). Round-trip locks pin every parameterized route through parse→serialize→parse, `studio_file_url` builds share links from the route type itself so writers can't drift, and the studio keeps the address bar on the canonical `?file=` URL of the open file. Sitemap hardening rode along: an enum-completeness ratchet (every static route must be sitemapped or explicitly excluded), boot-normalization round-trip and hygiene locks over every published URL, a robots.txt↔sitemap agreement check, and a `lastmod`-freshness lock that immediately caught the shipped sitemap lagging the 0.10.0 release article.


### Added
- **Prebuilt `largo` binaries + one-line installers** — `release.yml` gains a `build-cli` matrix: 5 platforms (linux/macos × x64/arm64, windows x64) × two flavors — lean, and **full** with Z3 built from the z3-sys vendored source and statically linked (`verification-static`). Linux builds pin `ubuntu-22.04` for the glibc 2.35 floor; arm64 builds run on native ARM runners (no C++ cross-compilation); artifacts are packaged inside the build job (upload-artifact drops the exec bit — also fixed for the LSP tarballs) and the release carries a `SHA256SUMS`. `curl -fsSL https://logicaffeine.com/install.sh | sh` (POSIX sh, dash-clean; `--full`, `--version`, `--to`) and `irm https://logicaffeine.com/install.ps1 | iex` (TLS 1.2, `Unblock-File`, user-PATH) install atomically to `~/.local/bin` / `%LOCALAPPDATA%\Programs\largo` after verifying the checksum — no GitHub API (the latest tag resolves via the `releases/latest` redirect), no sudo, no rc-file edits. Both flavors install as `largo`; `largo --version` reports `(full)`. The installers are static assets of the web app (`apps/logicaffeine_web/public/`), tested offline by `scripts/test-installers.sh` (fake release layout + local HTTP server + corrupted-tarball/404/PATH cases, under sh/dash/bash --posix), gated in CI by `installer.yml`, and tag pushes run pinned `e2e-install` jobs against the real release.
- **`largo` widened to the whole engine** — the CLI grows from 10 subcommands to the full front door: **`repl`** (interactive session: imperative statements against a persistent replay session — the exact `run --interpret` engine, so REPL semantics cannot drift from program semantics — plus English→FOL logic mode with discourse anaphora, `:format`/`:readings`/`:vars`/`:save`-as-runnable-program meta-commands, rustyline editing with keyword completion on a TTY, a plain scriptable line loop on a pipe), **`logic`** (English → FOL: `--format unicode|latex|ascii|kripke`, `--all-readings` scope+forest enumeration, `--discourse`, `--pragmatic`; byte-identical to the library API by test), **`prove`** (`## Theory` developments and English `## Theorem` blocks, kernel-certified, `--trace` derivation trees, `--json`), **`sat`** (the certified DIMACS solver as a verb — the driver extracted to `logicaffeine_proof::satcli` and shared verbatim with `logos-sat`; DRAT/DPR/SR export, competition exit codes 10/20), **`fmt`** (`--check`; rules lifted to `logicaffeine_language::source_format`, the LSP formatting provider delegates to the same function), **`emit rust|c|wasm|wasm-linked`** (`-o`, standalone-file mode), **`doc`** (markdown from a project's `##` blocks), **`add`/`remove`** (format-preserving `Largo.toml` edits via toml_edit), **`clean`**, **`completions`** (5 shells), and a reserved hidden `test` verb for the future test framework. New engine seams backing it: `logicaffeine_compile::repl::ReplSession` (replay session with output high-water mark and error rollback), `Interpreter::global_bindings`, `Session::set_format`.
- **`largo` UX overhaul** — global `-q/--quiet`, `-v/--verbose`, `--color auto|always|never` (NO_COLOR honored; all output routes through anstream so engine ANSI never leaks into pipes); cargo-palette styled help with an Examples section on every command (enforced by a CommandFactory meta-test); errors render as `error:` + an actionable `help:` hint with typed exit codes (`ui::CliError`). `largo build` now **streams cargo's own colored output live** (stdout inherited, stderr teed with a bounded tail) behind cargo-style `Compiling`/`Building`/`Finished` phase headers, passes `--color`/`-q`/`-v` through, and classifies failures: a `## Requires` resolution problem is framed as user-actionable, rustc errors in generated code as "a LOGOS compiler bug, not an error in your program". The CLI crate itself was restructured (module-per-command `commands/`, `ui.rs` output substrate) and its long-dead `phase37` integration tests were resurrected into `crates/logicaffeine_tests/tests/` where they actually run.
- **EXODIA native execution tier** — a register bytecode VM (the live sync/WASM engine, corpus-certified against the tree-walking interpreter as a shadow oracle) with a copy-and-patch JIT on top. `Int` fast paths are bit-identical to the kernel by the wrapping-`i64` spec, pinned by an edge-grid differential. The tier-up seam compiles hot **functions** per call with argument guards and kind-inference (params `Int`, comparisons `Bool`), and region-tiers hot **Main loops** (OSR-lite) with incoming-dead analysis and per-entry guards; anything outside the integer/float subset fails closed to bytecode — the deopt contract.
- **`logicaffeine-forge`** — new crate: the copy-and-patch JIT's executable-memory layer and stencil runtime (native only). `JitPage` allocates page-aligned memory, copies machine code in, flips it to executable, and returns a callable function pointer; compiling a function at runtime is `memcpy(stencil bytes)` + patch relocations. Includes the build-time-baked stencil runtime, the J1 micro-op compiler, and the **EXODIA contiguous register-allocating region/function x86-64 codegen tier** (`regalloc.rs`, `x64asm.rs`) that replaces per-stencil-piece dispatch. Platform-correct W^X for macOS/aarch64 (`MAP_JIT` + per-thread write-protect toggling + `sys_icache_invalidate`), other Unix (`mmap` → `mprotect` + aarch64 I-cache flush), and Windows (`VirtualAlloc` → `VirtualProtect` → `FlushInstructionCache`). Build-time stencil extraction via a `build.rs` over `object` with a relocation whitelist and tail-call/leaf-purity gates.
- **`logicaffeine-jit`** — new crate: the LOGOS native tier. `ForgeTier` translates VM bytecode (`Op`) into the forge's `MicroOp` subset for both whole functions (`ChainFn`) and hot loop regions (`RegionChain`), compiles each to a native stencil chain or the register-allocating backend, and hands the unit back. `install()` makes the tier process-wide — every live VM constructor picks it up, and `largo` installs it at startup. Native-only (`#![cfg(not(target_arch = "wasm32"))]`); WASM builds it to nothing and the browser runs pure bytecode.
- **`logicaffeine-synth`** — new crate: EXODIA Phase 2, the Forge's offline proof tooling. Z3 specifications for the integer micro-operations over 64-bit bitvectors, satisfiability and algebraic-property gates, and a three-way witness harness that runs Z3-chosen inputs through the real compiled stencil. Development/CI-time only; never on the production runtime path.
- **`logicaffeine-runtime`** — new crate: the deterministic concurrency runtime for the interpreter and VM — task scheduler, FIFO channels, `Select`, a logical-clock timer wheel, and the seed/trace machinery. A run is a deterministic function of `(program, seed)` and replays bit-for-bit from `(program, trace)` through a single `Chooser::decide` choke point. Pure `std`, WASM-safe, tokio-free, and by charter never linked into AOT-compiled binaries (the compiled path uses `logicaffeine-system`).
- **`logicaffeine-tv`** — new crate: SMT translation validation. Proves the emitted Rust is observationally equivalent to the LOGOS source per compile by symbolically executing both into the `logicaffeine-verify` domain and discharging the equivalence with Z3. `check_encoder_sound` cross-validates the LOGOS encoder against the tree-walking interpreter as the load-bearing trust anchor (rung 3–4 translation validation).
- **Verified arithmetic and proof advances** — proof-producing arithmetic with certificates and worked examples (`logicaffeine-proof::arith`), modal translation and independent verification, a CDCL core plus an incremental grid solver with grounding and trust-tiers, and label/PP convergence.
- **Strict whole-input parsing** — `compile()` now rejects parses that strand tokens (`TrailingTokens`) instead of silently dropping meaning; a dropped `until AWREADY` clause is a wrong assertion, not a style issue. The parser gains the coverage the strictness demands: noun-noun compound heads (`stop bit`, `grant signal`) with ambiguity-preserving gates, possessive heads over `Ambiguous` noun/verb words (`its value`), trailing temporal operators inside `If`-consequents (`until`/`release`/`weak-until`, `within N cycles`), postposed `when`-clauses and sentence-final temporal anchors, and quantified/cardinal objects under modals and under `never`. Lexer: letter-hyphen-letter compounds, attributive participial adjectives, `-ing` prepositions. A per-token lexical-ambiguity forest enumerates the strict-parse combinations.
- **Hardware-spec parsing and synthesis (PR #36)** — bounded delay synthesis (`within N cycles` → SVA `##[0:N]` via a `BoundedEventually(u32)` operator preserved through Kripke lowering), signal extraction for counting quantifiers, copula temporal adverbs in consequent clauses, `while` as a duration subordinator, and `exactly N` tokenization. Probe corpus 87/102 → 101/102.
- **The socratic teaching LSP — the editor teaches you to code.** One teaching brain (`logicaffeine_language::teach`) now feeds every surface: a lesson table whose TYPE requires a plain one-sentence explanation, a runnable example, and a socratic question or tip for every taught keyword (24), every `##` block type (all 19), and the built-in types — LSP hover and completion docs render it through one shared markdown renderer, and `largo repl` teaches the identical lessons via the new `:explain <word>`. All 39 socratic error explanations were rewritten to the contract *what happened → why → a guiding question → the concrete next step* (token names humanized — "a comma (',')", never `Comma`; byte offsets removed — the span carries the location) and committed as a golden so every future wording change is a reviewable diff, with a mechanical teaches-or-exempted lock behind it. Diagnostics with stable codes now carry `codeDescription` links into LOGOS_QUICKGUIDE.md (a compile-forced fourth field on the decision table; anchors verified against the guide's real headings). The stdlib became genuinely literate: all 158 prelude definitions (crypto suite included) carry `## Note` docs that surface in hover/completion/signature help — while the runtime prelude strips them byte-exactly, so nothing changes at execution. The whole surface is ratcheted: a teaching-corpus golden (hover markdown verbatim, completion text, signature docs, lens + quickfix titles, docs links), a quickguide↔teach parity lock (every guide row maps to a lesson or records why prose suffices, both directions), a stdlib docs lock, and a quickfix-parity lock proving every promised fix against a real broken program — which found that `undefined-variable` has NO producing program (typo'd names pass silently today; recorded as an explicit gap for the unresolved-reference work).
- **LSP: cross-file references and rename, a warnings layer, interpolation painting, range formatting.** References and rename now cross files (open buffers answer from LIVE text — a stale disk index can never produce a wrong edit; unopened files answer from the workspace reference map; local `Let`s stay private). New `shadowed-variable` warning (a second `Let x` in the same block, related-info pointing at the first) and `unused-function` hint (only in programs WITH a `## Main` — a library's functions are its API), both under a severity table locked live by producing programs. `"Hello {name}!"` interiors paint as CODE (braces as operators, `name` exactly as it paints outside the string). Range formatting un-exempted: the structural whole-document format filtered to intersecting lines.
- **The teaching ratchets caught five live bugs on their first runs**: sentence-level parse recovery DEGRADED specific diagnoses to generic ones (`x is 5.` taught "say equals" in the full parse, then recovery replaced it with "expected a statement" — the specific error now wins, and the imperative dispatch diagnoses it directly); the use-after-move quickfix could offer and INSERT `a copy of .` (punctuation read from the diagnostic span — it now names the variable from the message and edits the variable's own token); spelling suggestions fired noise on coded diagnostics ("Did you mean 'a'?" for a moved `x`); token-extracted function definitions (any document with a theorem block) dropped their parenthesized parameters from hover/completion/signature help; and the root `assets/std` copies had silently drifted from the canonical compile-tree stdlib (now byte-locked in both trees).
- **The AST depth gate: no LOGOS program can stack-overflow the compiler anymore, on any surface.** A 5,000-term expression chain (or parenthesis tower, or block pyramid) used to SIGABRT the CLI, REPL, LSP, and web Studio alike — every downstream walker recurses on tree depth. The language crate now enforces "parsed ⇒ bounded" at the single `parse_program` choke point (`ast_depth`: an iterative, wildcard-free walker — a new AST variant with children fails compilation until classified — plus parse-time recursion guards), so deep programs get a graceful `AstTooDeep` diagnostic that teaches BOTH fixes: split into `Let` bindings, or copy the suggested `LOGOS_MAX_AST_DEPTH=<n>` override from the message. The limit is environment-tunable (default 128, sized for the smallest standard stacks and proven in 2 MiB-thread tests) and **the override genuinely works**: `largo` runs on a worker thread whose stack is sized FROM the limit, so a potato keeps its safe default and a big machine raises one knob and compiles the 5,000-term chain clean. Long-line error excerpts window around the caret instead of flooding 20 KB of generated source; `largo check` renders parse failures socratically instead of as a Debug struct. Full corpus unaffected (language 268 / compile 1045 / LSP 266 all green).
- **LSP: end-to-end harness + ratchet locks.** The language server gains its first integration tests — a wire-level harness that drives the real `tower_lsp::Server` loop over in-memory pipes (framing, request correlation, `publishDiagnostics`), plus ratchet locks in the `readme_lock` idiom: every `TokenType` must classify inside the advertised semantic-token legend, every `ParseErrorKind` must pass a new total `decision_for` severity/code/quickfix table (a new error kind does not compile until it decides all three), the capability set is pinned in both directions with reasons for what's deliberately absent, and the token legend is locked append-only. Two real bugs fell out immediately: the signature-help trigger `with` was multi-character (LSP clients only send single characters — it could never fire; now `␠`/`,`), and parse-path `use-after-move` diagnostics neither spoke the ownership house voice nor linked their cause — they now say "Cannot use 'x' after giving it away" and carry `DiagnosticRelatedInformation` pointing at the `Give` statement, same as the ownership-checker path.
- **VSCode extension: real editor front-end.** The 46-line shell became a full client: platform/arch-aware resolution of the bundled server binary (explicit `logicaffeine.lsp.path` wins and fails loudly if missing; `bin/logicaffeine-lsp-<platform>-<arch>` next, win32-arm64 falling back to the x64 binary under emulation; PATH last — every cell unit-tested via an injected-fs resolver), an output channel + `logicaffeine.trace.server`, a language-status item showing server state and which binary is running, actionable spawn-failure notifications, and restart/show-log commands. The server's code lenses stopped dead-ending: `logicaffeine.run` (task running `largo run` with an `interpret|debug|release` mode setting from the file's `Largo.toml` root), `logicaffeine.verify` (project `largo verify`, license via SecretStorage — never settings.json — injected as `LOGOS_LICENSE`, honest "preview" copy), and `logicaffeine.prove`/`checkProof` (`largo prove <file>`, optional `--trace`), plus context menus, an editor run button, and keybindings. Integration suite (test-electron under xvfb in CI, against a debug server build) locks activation, the diagnostics round trip, task spawning from the fixture project, and that every contributed command is registered.
- **VSCode extension: highlighting overhaul.** The TextMate grammar was rewritten against the real surface (`LOGOS_QUICKGUIDE.md` + the lexer): `#` line comments (declared in language-configuration too — they never were), decomposed `##` headers (`## To` names color as function definitions, `## A Point has:` as type definitions), `Let`/`Set` binding captures with `mutable` as a storage modifier, parameter/type annotations, `a new Type` construction, `When Variant` match arms, call sites and UFCS method calls, the multiword English operators (`is at least`, `is divisible by`, `combined with`, …), string interpolation holes with format specs, duration/date/time/money literals, and `## Note`/`## Example` bodies rendered as documentation-comment prose so English recedes while code speaks (mirroring the server's semantic classification). A `semanticTokenScopes` contribution maps all 13 server token types to theme-safe scopes, and a markdown injection grammar highlights ```` ```logos ```` fences in literate `.md`. Locked by a quickguide coverage **ratchet** (every non-`(proposed)` backticked surface form must highlight or be allowlisted with a reason — currently zero allowlisted — failing in both drift directions, the `guide_examples.rs` pattern) plus tmgrammar snapshot tests over an imperative/theorem/CRDT/prose corpus.
- **LSP/tooling audit: highlighting corrected, the formatter made safe and structural, one classifier everywhere.** A new committed corpus golden (every token of a representative program rendered `lexeme→class+modifiers`) caught three live classification bugs — `Int` painting as a variable, struct names painting as properties (span collision from registry-path field definitions), `mutable` painting as a type — all fixed and now diff-reviewable forever. The canonical formatter was silently corrupting multiline strings (trailing-space strip, tab conversion, and line-blanking INSIDE `\"\"\"` literals, live in both `largo fmt` and the LSP); it is now string-and-prose-protected AND structure-aware (4 spaces per lexed nesting level), locked by token-stream-equivalence + idempotence property tests. Token classification moved into the language crate (`token_class`) so the LSP and the REPL share one brain — and `largo repl` gained live ANSI syntax highlighting from it. A server-side quickguide ratchet (every canonical form must paint through the real pipeline, crashers named individually) found that a bare `## Theorem:` header PANICKED analysis — the parser is now total (EOF-normalized streams, clamped `peek`), with server-side panic containment behind it.
- **VSCode extension: marketplace-ready release pipeline.** Marketplace assets landed (icon, `.lg` file icon, snippet library, a four-step "Get started with LOGOS" walkthrough, the README that becomes the marketplace page — all locked into the VSIX by the packaging test), and `release.yml` grew the real pipeline: per-platform VSIXs (one server binary each, plus a binary-free universal fallback the Marketplace serves to untargeted platforms), an **install gate** that installs the actual packaged VSIX into a real VSCode on Linux/macOS/Windows and asserts the bundled server answers diagnostics before anything publishes, and fail-loud `vsce`/`ovsx` publish jobs (missing `VSCE_PAT`/`OVSX_PAT` abort — never a silent skip; re-runs tolerate "already exists") — the publish job is wired but deliberately held disabled for 0.10.0, so the VSIXs ship as GitHub-release artifacts only; marketplace listings follow. Trust hardening: untrusted workspaces cannot redirect which binaries execute; virtual workspaces get highlighting only.
- **LSP: capability completion + gates.** Document highlights (WRITE on bindings/mutations), selection ranges walking English structure (word → sentence → block → document), on-type formatting (typing `.` applies the `largo fmt` rules to the closed sentence), call hierarchy over indexed call sites, and pull diagnostics with cross-engine result ids — the advertised surface is now 20 providers, every one pinned by the capability lock. A live `logicaffeine.flycheck.enable` setting toggles the rustc pass with retraction, and latency locks put a loud ceiling over whole-document analysis.
- **LSP: the rustc flycheck — "English with a borrow checker".** The dormant rustc→LOGOS diagnostic bridge is live end-to-end: codegen now RECORDS the sourcemap it always had an API for (`codegen_program_mapped` ties every generated line to its LOGOS statement span and tracks variable ownership roles — previously `SourceMapBuilder` was never called and every translated error had no span), a new `rustc_check` compiles with the optimizer off (spans stay 1:1) into a persistent per-workspace cache and runs `cargo check --message-format=json`, and the LSP triggers it on save: rustc findings arrive translated to English on real user-source spans under a distinct `logicaffeine (rustc)` source. Newer saves win by generation guard, edits clear findings, overlaps with interactive errors deduplicate, and no cargo = silent degrade — all mock-proven over the real server loop. `largo check --deep` is the CLI twin. The first fixture immediately proved the point: rustc caught `Give a to b.` where `b` is an Int (generated `b(a)`) — a nonsense program the parser and both local checkers accepted.
- **LSP: workspace symbols + cross-file navigation.** The server now indexes every `.lg`/`.md` file under the workspace folders in the background (bounded scan, refreshed on save/watched changes): `workspace/symbol` searches definitions across files never opened, and goto-definition falls back to the workspace index when the current document doesn't define the name — landing in the defining file directly.
- **LSP: socratic diagnostics depth.** The parser gained a statement-span side-table (`Parser::stmt_spans`) and the compile crate a `check_program_collect` that reports every failing top-level statement (fail-fast `check_program` untouched) — so the typechecker finally surfaces in the editor: return-type mismatches, arity, field-not-found (message lists the fields that DO exist, filled from the registry), not-a-function, infinite-type, dimension — each anchored on its own statement's span with a stable diagnostic code. Block recovery went sentence-level: each independently broken sentence reports (excise-and-reparse, structure-guarded) while the good statements around it stay parsed and navigable — plus an EOF-termination fix for mid-stream block slices the parser could previously walk off. Unused `Let`s get a faded UNNECESSARY hint with a remove-statement quickfix.
- **LSP: resolution-aware semantic highlighting.** English's grammar is the syntax — verbs paint as functions, nouns as types, adjectives as modifiers — and now a `SymbolIndex` overlay upgrades identifiers to what they *resolve* to (parameter/function/type/field/variant/variable) at declaration and reference alike, with `declaration` firing only at the definition site (references were previously painted as declarations), `readonly` on immutable `Let`s, a new `modification` bit on write sites, and a new `defaultLibrary` bit on stdlib prelude names; `## Note`/`## Example` prose recedes to comment color. Range and full/delta semantic-token requests landed (single-splice deltas against cached result ids). A real resolution bug fell out: verbs resolved by lexicon lemma ("Greet") instead of surface form ("greet"), and lexically ambiguous words ("name") resolved not at all — English-word identifiers now support goto-def/references/rename.
- **LSP: async snapshot architecture + incremental sync.** Documents became immutable `Arc` snapshots (holding a map guard across `.await` — a latent deadlock in `publish_diagnostics` — is now unrepresentable), text sync moved FULL → INCREMENTAL with UTF-16-correct range edits, and a new per-document scheduler debounces analysis (~150 ms) behind a generation counter with the pipeline on `spawn_blocking`: a 51-keystroke burst now runs one analysis over the final text instead of 51, locked by an e2e coalescing test over the real server loop.
- **Web app: full SSG prerendering.** The deploy now runs `dx build --ssg --fullstack`: dx builds the wasm client (`--features web`) and a native prerender server (`--features server`), then writes real per-route HTML for every route in `sitemap::prerender_routes()` — landing, pricing, guide, crates, learn, benchmarks (full tables from the native-side data), roadmap, privacy/terms, news index + **every news article**, and shell+meta pages for studio/registry/profile. Each page carries its own `<title>`, description, Open Graph/Twitter card, canonical link, and JSON-LD (emitted by `PageHead` via `document::Meta`/`Link` on the server render), so crawlers, AI bots (GPTBot/ClaudeBot/PerplexityBot execute no JS), and link unfurlers finally see real content — and first paint is prerendered HTML instead of a blank shell while the wasm loads. The client is deliberately **not** hydrating (a hydrate-enabled client hard-crashes on SPA-fallback URLs like `/registry/package/:name` that cannot be enumerated at build time): the wasm mounts into a separate `#app` root while the prerendered copy stays visible in `#main`, and a MutationObserver in the shell removes the copy only once the app has committed real, visible content (judged on DOM truth — non-empty rendered text), so the swap is atomic: no cleared-DOM gap, no unstyled flash, and a still-loading route can never blank the page. The prerender writes into `web/prerendered/` and `scripts/merge-ssg.sh` lays it over `public/` after dx finishes (the client build rewrites the shell `index.html` last and would clobber the prerendered `/`). Gates: `tests/ssr_render.rs` renders all 46 routes on the native target (any browser API in a render path fails there, plus per-page content markers and a JSON-LD check), `scripts/verify-ssg.sh` proves every sitemap route produced its file with exactly one og:title (dx silently skips routes that panic), `scripts/test-ssg-smoke.sh` drives headless Chrome through a prerendered route and an SPA-fallback route (content present, takeover completed — `#main` gone, no duplication), and `scripts/test-ssg-screenshots.sh` loads EVERY sitemap route in headless Chrome after wasm boot — per-page content markers, zero page errors, completed takeover, a pixel-level blank-viewport detector (sharp-edge density, catching styled-but-empty pages that DOM checks miss), and a screenshot per page for human review. The smoke test caught the wasm-split blank-page regression the moment a local Chrome could run it. Two real render-path panics were fixed by the new tests: `LicenseState::new` and `ThemeState::new` read localStorage un-gated at App root.
- **Web app: wasm code-splitting — first visit ~3.9 MB → ~683 KB gzipped.** The dx `--wasm-split` pipeline ships behind the new `split` cargo feature: all 13 heavy route pages are `#[component(lazy)]`, and the two engine-driving landing showcases (the hero mini-studio, the Hello-World editor) were extracted into lazy components so the entire LOGOS engine leaves the eager bundle. A first-time visitor downloads a **1.1 MB main bundle plus a ~1 MB landing chunk (~683 KB gzipped total)** for a fully interactive landing; the engine chunks (Studio ~10 MB, benchmarks/learn/guide ~5–6 MB each) stream in only when their page opens, once per browser (immutable-cached). Getting there took fixing dioxus itself: its 0.7.9 split runtime crashed pure-CSR resolution three ways (single-split routes hit `ReplaceWith` on an unregistered node — "Cannot read properties of undefined (reading 'listening')"; double-lazy routes hung; NotFound never committed), rooted in two core-suspense bugs (writer-less background renders reaching a real writer) and two wasm-split-cli symbol-resolution bugs (LTO-folded names and mangled-vs-demangled name matching, which corrupted main-module data segments). All four are fixed on our dioxus fork (`Brahmastra-Labs/dioxus`, pinned by rev in `[patch.crates-io]` and the `deploy-frontend.yml` dx install) and submitted upstream; the deploy unpins once the fixes land in a released dioxus. The CSR takeover, size gate (eager ≤ 2 MB / total ≤ 60 MB), and the 46-route screenshot suite all gate the split build. `wasm-release` carries `debug = true` (the splitter needs the DWARF symbol table; `wasm-opt` strips it afterward).
- **Web app: runtime data bundle (`/data/*`).** The wasm binary no longer carries its heavyweight data. Benchmark results (latest/solvers/latest-codec/latest-interp JSON), all 32×11 benchmark program sources, privacy/terms HTML, and the dynamic lexicon are fetched at runtime from `/data/*`, staged from the repo's sources of truth by `scripts/stage-web-data.sh` (self-validating — a missing or empty file fails the build, the same guarantee `include_str!` gave at compile time; wired into the deploy before `dx build`, gitignored so it can never drift). Native builds compile the identical bytes in, so tests and the SSG prerender see the same data through shared accessors; sources are now looked up **by benchmark id** (`bench_sources_for`) instead of positional indexing, locked by a new `sources_cover_every_benchmark` test. The Benchmarks page gains a loading/error shell with retry; the lexicon loads once behind a `LexiconGate` (it was being re-parsed from 279 KB of JSON on *every render* of the Learn/lesson/review pages).
- **SAT benchmark: our own proof size, plus CaDiCaL + CryptoMiniSat.** The `/benchmarks` "Our solver vs the field" section now shows the size of *our* certified proof beside every winning family, the way Kissat and SaDiCaL already showed theirs — answering "why doesn't Logos show its proof size?". For the pigeonhole class it is the **SR proof**; for the algebraic families (Tseitin GF(2), mod-p, mod-6) it is the **compact GF/ℤ-ring linear-dependency certificate** the Gaussian route actually returns (O(#equations)), *not* its exponential clausal-DRAT expansion. It is measured by streaming the artifact through a capped byte counter in memory (`proof_emit::SizeSink` + the new streaming `proof_emit::write_sr`) — never materialized, never written to disk, and bounded so a pathological proof cannot exhaust RAM. Two solvers join the field on the byte-identical DIMACS: **CaDiCaL** (Biere's mainline DRAT reference) and **CryptoMiniSat** (native GF(2) Gaussian, run in its default DRAT-emitting config). Both wall on pigeonhole exactly as the resolution bound predicts.
- **New crush families.** `families::mod_m_tseitin_expander` — the composite-modulus Tseitin obstruction over `ℤ/m` (mod-6), decided by the **ring route** (`ℤ/6 ≅ GF(2) × GF(3)` via CRT) that resolution walls on and a pure GF(2) parity engine is blind to. Plus a **pigeonhole-variants** family (functional / onto / weak PHP), each refuted by the generic prover's matching/symmetry route while Z3/Kissat/CaDiCaL/CryptoMiniSat all wall.
- **A proof size for every family, per its own certificate kind** — the multi-route prover shows the certificate the route actually produces: SR proof (PHP), GF/ℤ-ring linear certificate (parity/mod-p/mod-6), the O(1) **counting certificate** `pigeons > holes` (pigeonhole variants), the compact **Hall witness** (mutilated chessboard), and the plain-CDCL **DRAT** on the random control's UNSAT instance. Emitting a *clausal* proof for the structural families would be the exponential blowup the competitors wall on, so the compact re-checkable certificate is what we measure.

- **Documentation set promoted** — the root `README.md` becomes the product front page (mission, quick start, a compiling merge-sort walkthrough, eight feature pillars, the full 8-chart benchmark section, the tiered workspace map) and `docs/` lands eight code-grounded guides (imperative mode, logic mode, execution tiers, proof & verification, concurrency, Studio & Learn, the `largo` CLI, architecture). Every ```logos example and every English→FOL pair in the set is extracted and checked against the live compiler by the `doc_examples` harness, so the documentation cannot silently rot; all crate-README cross-links repointed from the staging paths.

### Changed
- **Interpreter performance** — release-bench: tree-walker 70ms → VM 16.6ms (4.2×) → tiered 6.0ms (11.7×). The VM+JIT geomean moves from 3.34× to ≈1.0× versus Node/V8 on the interpreter benchmark suite, with 13 of 32 programs faster than V8; Main loops that never reached the JIT now run native.
- **Run-path optimizer** — magic-reciprocal division/modulo, run-path recursion inlining, and loop-invariant pointer/length plus constant hoisting on the interpreted execution path.
- **Workspace and CI** — workspace members gain `forge`, `jit`, `tv`; `default-members` excludes the Z3-backed crates (`verify`, `tv`) so a plain `cargo test` needs no Z3. `publish.yml` now derives the publish set and dependency order from `cargo metadata` and fails loudly on any publish error (re-runs skip already-published versions); `test.yml` enables `logicaffeine-tests/verification` explicitly; new `forge.yml` runs a 4-OS runtime matrix and a 7-target cross-check.
- **Z3 bindings upgraded to `z3` 0.20 (`z3-sys` 0.11, vendored Z3 4.16).** The verification layer (`logicaffeine-verify`, `logicaffeine-synth`) migrated from the lifetime-threaded `Context` model to z3 0.20's implicit thread-local context — a mechanical ~700-line change (verdicts unchanged, every verification test green against system Z3 and the vendored build). The static `largo-full` flavor now builds the vendored **Z3 4.16**, which compiles cleanly under macOS libc++; the older vendored 4.12 did not (a two-phase-lookup error), which had blocked the macOS `-full` binaries.
- **Benchmark artifacts** — generated `.s`/`.ll` asm and LLVM-IR dumps (~620 MB) moved behind `.gitignore`; they are build outputs of `run-logos-vs-c.sh` and regenerable.
- **Lockstep publishing** — `logicaffeine-forge`, `logicaffeine-jit` and `logicaffeine-runtime` join the published lockstep version line. Lockstep is now structural: every member inherits `version.workspace = true` and internal dependencies resolve through `[workspace.dependencies]`, so the version lives in exactly one file and `scripts/bump-version.sh` edits only the root manifest.
- **CDCL engine — per-conflict allocation eliminated (~30% faster on random 3-SAT).** The general solver's hot conflict path matched the SOTA lineage in *algorithms* (two-watched literals + blocking literals, 1UIP, recursive minimization, VSIDS heap, LBD/glue deletion, Glucose+Luby adaptive restarts, phase saving) but still allocated per conflict the way a reference implementation does not. `analyze` and `lit_redundant` cloned every reason clause (now indexed in place — copying each `Lit` ends the borrow before the mutation); learned-clause `minimize` allocated a fresh `HashSet`+`HashMap` each conflict (now the shared `seen` array plus solver-resident, sparsely-reset memo/touch buffers); and LBD was a per-conflict `Vec`+sort (now generation-stamped level counting). Byte-identical search — same conflicts, same propagations, same verdicts (every CDCL brute-force and verdict-invariance test unchanged) — just no per-conflict heap traffic: `random_3sat` batch throughput ~75 ms → ~50 ms per instance. Speeds up the whole VM/Studio path, which runs the same engine.
- **CDCL engine — implicit binary clauses in the watch lists.** Each `Watcher` now carries a `binary` flag; when a binary clause's blocking literal is not already true, the clause is decided unit-or-conflict directly from the watcher with **no clause dereference** (the Kissat/CaDiCaL trick), kept exact by watch (re)creation on every strengthening + `reduce_db` rebuild. Zero regression (all 938 proof-crate unit tests green); it pays off on binary-heavy and long solves — short random 3-SAT, being mostly the original ternary clauses, sees it as neutral.
- **Certified-prover cascade — the expensive cuts are escalation-only.** `prove_unsat`'s costly certified rungs (Lyapunov collapse, Nullstellensatz, Polynomial Calculus, the recursive certified symmetry break) ran on every call — a ~50 ms tax on microsecond equivalence/BMC obligations that had quietly inverted the native-vs-Z3 speed locks. The cheap structural recognizers stay upfront; the complete CDCL search then runs under a conflict budget, and only an instance that exhausts it — the exponential families the rungs exist for — escalates, finishing on a fresh solver so the refutation certificate stays self-contained. Verdicts unchanged; `native_is_faster_than_z3` and `optimizer_beats_z3` hold again and the hard symmetric families still refute through the escalation path.

### Fixed
- **`## Requires` dependencies were silently dropped or mis-scoped** — both Cargo.toml generators appended user-declared crates *after* a section header (`[profile.release]` in the CLI's build path, `[target.'cfg(linux)'.dependencies]` in `compile_to_dir`), so the dependency landed in the wrong TOML table: cargo ignored it entirely on the `largo build` path, and treated it as Linux-only on the compile-to-dir path. User dependencies are now written inside `[dependencies]` before any later section; caught by the new build-passthrough e2e (a nonexistent `## Requires` crate must fail the build with a dependency-resolution framing).
- **3sg stem recovery** — `strip_s` derived the stem from surface heuristics, making `planes` the 3sg of `plan` and sinking gerund subjects (`Flying planes can be dangerous.`). The inverse is now defined by the forward rule: a stem is accepted only if it is a known base verb whose `third_person_of` regenerates the surface form exactly.
- **Word-class regressions from lexicon growth** — presupposition triggers read through `Ambiguous` tokens again (restoring Van der Sandt projection under negation) via one shared `consume_presup_trigger`; `strip_s` resolves sibilant `-es` and `-ies` stems through `is_base_verb`; a temporal-adverb reading is blocked immediately after a determiner (generic structs regain their first field); focus constructions accept copular predication (`Only dogs are red.` → `Only(D, Red(Dogs))`).
- **Exercise generation** — challenge generation rejection-samples (`MAX_DRAWS=32`), so one unparseable word in a pool can no longer sink an exercise (456/456 generate); `debug.rs` and `equivalence.rs` doc examples are now compiling doctests.
- **VSCode extension: the shipped VSIX could not activate.** `release.yml` packaged with `vsce package --no-dependencies` on top of a plain-`tsc` build, so `vscode-languageclient` was never in the package — the released extension failed at import — while ~42 MB of staged LSP binaries rode along unused. The extension is now esbuild-bundled (`dist/extension.js`, minified, self-contained; `--no-dependencies` is thereby correct), with `.vscodeignore` trimming the package to 6 files (~85 KB + binaries), a committed `package-lock.json` (`npm ci` in CI), eslint + typecheck, and a new `extension` CI job (lint/typecheck/unit under Node, integration under xvfb against a debug LSP build). Two new packaging locks make the old failure unshippable: `vsix_contents.test.ts` asserts the bundle is in the package and nothing references `node_modules/`, and `version_lockstep.test.ts` pins the extension version to the workspace version.
- **Web bundle down 15.40 MB → 11.41 MB (−26%)** — `include_dir!` was embedding the entire `assets/` directory into the wasm, including 1.98 MB of images (the largest single data segment in the binary at 12.6%); it now embeds `assets/curriculum/` only, and the images deploy as static files. **This also fixes the live 404 of the social-share image**: every `og:image` pointed at `/assets/OG-photo.png`, which was inside the wasm rather than on the CDN — share cards never showed an image in production. The OG card itself was recompressed from a 2224×1180/1.4 MB PNG to a proper 1200×630/312 KB card (unfurlers time out on megabyte images), with `og:image:width/height/alt` now emitted. The dead CodeMirror CDN module (never called — the editors are styled textareas) is deleted from `index.html`, and KaTeX now lazy-loads on the first math render instead of on every page (which also fixes math renders being silently dropped when the CDN was slower than the first paint).
- **Live `<title>` concatenation** — the deployed site's title read "LOGICAFFEINE | Debug Your ThoughtsLOGOS Studio": dx splices the `Dioxus.toml` title into `index.html`'s existing `<title>`. The template now uses the `{app_title}` placeholder and per-page titles come from `PageHead`.
- **Stale SEO surfaces** — `public/sitemap.xml` listed 15 URLs (3 of ~33 news articles, no `/benchmarks`); it is now generated from `sitemap::prerender_routes()` (46 URLs) and held byte-equal to the generator by a test (`regenerate_shipped_sitemap` refreshes it). `llms.txt` claims updated to current reality (Dioxus 0.7, four Studio modes incl. Hardware, the Z3-free in-browser certified prover, the 2.544× C geomean headline, `/benchmarks` methodology section). `/success` is now `noindex`. Hashed bundle assets get `Cache-Control: immutable` via `_headers` (repeat visits stop re-downloading the wasm; rules deliberately match only the content-hashed filenames since Cloudflare Pages *joins* duplicate headers across matching rules).
- **`cert_farkas` kernel reconstruction on doubled constants** — the proof-producing arithmetic normalizer had no proof path for like monomials recombining to coefficient 1, and a merge that cancelled an entire prefix concluded with an unproven `add 0 x` residue the kernel rejected — so a correct Farkas refutation of `x+1 ≤ y ∧ y+1 ≤ x+1` failed to certify. Both holes close (locked by new `arith::tests`), and the long-ignored `probe_double_constant_le_via_auto` now runs un-ignored.

## [0.9.16] - 2026-04-06

### Added
- **IEEE 1800-2023 SVA upgrade** — 4 sprints (22-25) upgrading from IEEE 1800-2017 to 1800-2023 compliance. 3 new `SvaExpr` variants: `ArrayMap` (IEEE 7.12 array `.map()` with iterator/index arguments), `TypeThis` (IEEE 6.23 `type(this)` construct), `RealConst` (IEEE 5.7.2 real literals). `RandVarType` enum replacing `width: u32` on `RandVar` to support `rand real` / `rand const real` checker variables (IEEE 17.7). Triple-quoted string support in action blocks (IEEE 5.9). New system task recognition: `$timeunit`, `$timeprecision`, `$stacktrace` (IEEE 20.4.1, 20.17). 4-state operator truth table errata audit (IEEE 11.4).
- **`VerifyType::Real`** — Z3 Real sort threaded through the entire verification pipeline: `ir.rs`, `equivalence.rs`, `kinduction.rs`, `smtlib.rs`, `solver.rs`, `type_infer.rs`. `BoundedSort::Real` in SVA-to-verify translation for checker variable quantification.
- **95 new SVA tests** — `phase_hw_sva_2023.rs` (1,160 lines) covering all 4 IEEE 1800-2023 upgrade sprints: triple-quoted strings, new system tasks, array map methods, `type(this)`, `rand real` / `rand const real`, and semantic audit cross-feature composition.
- **Hardware parser gap fixes** — `shall` modal gate, `always`/`never` after copula, `after`/`when` subordinators, `and`-conjunction in conditionals, `bit` as noun, `request`/`grant` disambiguation, counting quantifiers, HAB SVA synthesis patterns.
- **SVA synthesizer coverage expansion** — new FOL-to-SVA synthesis patterns for counting quantifiers, HAB temporal patterns, and additional property templates.
- **Cirq v2 implementation plan** — complete rewrite of the quantum backend plan: 17 sprints, 314 tests planned across 13 test files, mirroring the proven SVA backend architecture.

### Changed
- **CI pipeline** — benchmark workflow no longer double-triggers deploy workflows. Benchmark result commits use `[skip ci]` to prevent cascading re-runs.

## [0.9.15] - 2026-04-04

### Added
- **IEEE 1800-2017 SVA expansion** — 40+ new `SvaExpr` variants covering property connectives (`not`, `implies`, `iff`), LTL temporal operators (`always`, `s_always`, `eventually`, `s_eventually`, `until`), sequence composition (`and`, `or`), abort operators (`accept_on`, `reject_on`, `sync_accept_on`, `sync_reject_on`), assertion directives, local variables, endpoint methods, bitwise/reduction operators, part selects, concatenation, complex data types, and let/checker declarations.
- **Vacuity analysis** — new `sva_vacuity.rs` module implementing IEEE 16.14.8 compliant vacuity checking with 33 rules for nonvacuous evaluation tracking, dead assertion detection, and coverage gap identification.
- **Bounded verification IR** — `SequenceMatch` struct with length-tracking for proper composition semantics, `BoundedExpr::Apply` for system function encoding, local variable bindings, and queue timestep support for `const'()` freezing.
- **System function support** — `$onehot0`, `$onehot`, `$countones`, `$isunknown`, `$sampled`, `$bits`, `$clog2`, `$countbits`, `$isunbounded` with proper bounded verification IR encoding.
- **Benchmark specifications** — engineering specs for FVEval NL2SVA (300 cases), VERT (20,000 cases), and AssertionBench (101 designs) targeting 100% IEEE 1800-2017 SVA coverage.
- **SVA coverage analysis** — gap analysis documenting 60 remaining features across 21 planned sprints with IEEE section references.
- **SVA test expansion** — new `phase_hw_sva_coverage.rs` (6,200+ lines) plus expansions to the IEEE 1800, roundtrip, surface, and translate test suites.

### Changed
- **SVA translation pipeline** — unified delay convention (`None` = unbounded `$`, `Some(n)` = bounded), proper `throughout`/`within` desugaring with length-matching semantics, and sequence instance resolution with parameter substitution.

## [0.9.14] - 2026-04-03

### Added
- **SVA synthesis codegen** — `codegen_sva/synthesize.rs` and `codegen_sva/z3_synth.rs`: Z3-guided synthesis of SVA properties with realizability checking.
- **Z3 equivalence test expansion** — 217 new lines of bitvector and semantic equivalence tests in `phase_hw_z3_equiv.rs`.

### Changed
- **Verification engine strengthening** — deeper passes across the model-checking stack introduced in 0.9.13 (≈5,500 lines): IC3/PDR generalization, k-induction with auxiliary-invariant strengthening, Craig interpolation, CEGAR predicate abstraction, liveness-to-safety reduction, multi-clock and compositional (assume-guarantee) reasoning, and certificate proof-witness embedding.
- **Solver infrastructure** — Z3 encoding with bitvector operation support and improved counterexample extraction.
- **Verification IR** — extended with node types for multi-clock and compositional reasoning.

## [0.9.13] - 2026-04-03

### Added
- **Hardware kernel types** — first-class `Bit`, `BitVec`, and `Circuit` types in the formal kernel prelude, enabling kernel-level hardware reasoning.
- **Bitvector decision procedures** — delta reduction support for bitvector operations in the kernel reduction engine.
- **Advanced model checking** — IC3, k-induction, and Craig interpolation strategies for unbounded property verification.
- **Compositional verification** — assume-guarantee reasoning with contract-based decomposition for modular hardware verification.
- **Clock domain crossing analysis** — CDC verification module detecting synchronization issues across clock boundaries.
- **Power isolation verification** — power domain analysis ensuring correct isolation and retention behavior.
- **Security property verification** — information flow and non-interference checking for hardware security properties.
- **Multi-clock verification** — formal reasoning about synchronization across multiple clock domains.
- **Synthesis oracle** — Z3-guided synthesis from formal properties to circuit implementations via the kernel type theory.
- **Test generation** — automatic test case generation from formal specifications.
- **Incremental verification** — delta-based re-verification avoiding redundant work when specs change.
- **Parameterized verification** — verification of parameterized designs across all instantiations.
- **Synthesis-compiler checks** — correctness checks for the SVA synthesis compiler (`verified_compiler`).
- **SMT-LIB2 dialect support** — direct SMT-LIB2 output for interoperability with external solvers.
- **Verilog extraction** — Verilog module extraction from kernel proof terms.
- **RISC-V protocol templates** — pre-verified SVA property templates for RISC-V bus protocols.
- **Liveness checking** — liveness property verification with fairness constraints and ranking functions.
- **Abstraction refinement** — CEGAR-style abstraction with automatic predicate discovery.
- **Certificate generation** — independently checkable verification certificates for proof portability.
- **Verification strategy selection** — automatic strategy selection based on property classification.
- **Type inference for properties** — automatic type inference for verification expressions.
- **Automata-based reasoning** — Buchi and omega-automata for temporal property verification.
- **27 new hardware-verification test files** — coverage across the new verification domains.

### Changed
- **Pricing removed** — replaced public pricing tiers with contact-based commercial licensing. Free for individuals, teams under 25, and education.
- **Verify crate publishing** — logicaffeine-verify now publishes to crates.io with fixed cascade.
- **Equivalence checking** — expanded semantic equivalence with new analysis passes.
- **Test documentation** — README expansion covering 60+ test files.

## [0.9.12] - 2026-04-02

### Added
- **FOL-to-SVA synthesis** — pattern-matching translation from first-order logic to SystemVerilog Assertions via Kripke-lowered structures, mapping quantified temporal patterns to `s_eventually`, `nexttime`, and temporal implications.
- **Coverage analysis** — signal, property, edge, and temporal coverage metrics measuring how well SVA properties cover the specification knowledge graph.
- **Sufficiency analysis** — pre-verification checks detecting lonely signals, unconstrained outputs, and missing handshake patterns.
- **Spec health checking** — self-consistency analysis of English specifications through the FOL-to-VerifyExpr-to-Z3 pipeline, detecting contradictions, vacuity, and redundancy.
- **Property decomposition** — hierarchical decomposition of conjunctive properties into independently verifiable sub-properties.
- **CEGAR synthesis refinement** — counterexample-guided refinement loop classifying SVA divergence as too-strong or too-weak with transformation strategies.
- **RTL extraction** — Verilog declaration parser extracting module structure (ports, signals, parameters, clock detection).
- **RTL knowledge graph** — converts RTL modules to hardware knowledge graphs and links spec KGs to RTL KGs via signal name matching.
- **Waveform rendering** — renders Z3 counterexamples to VCD (Value Change Dump) format for waveform viewer inspection.
- **Invariant discovery** — automatic candidate invariant generation from KG structure (mutex, handshake, pipeline patterns) with Z3 verification.
- **Protocol templates** — pre-verified parameterizable SVA properties for standard protocols (AXI4, APB, handshake) with English specification mappings.
- **Consistency checking** — multi-property consistency verification with minimal unsatisfiable subset extraction and vacuity/redundancy detection.
- **Hardware verification test suites** — 19 new test files covering coverage, decomposition, CEGAR, RTL extraction, RTL KG, protocols, spec health, sufficiency, synthesis refinement, waveform, invariants, consistency, signal bridge, SVA IEEE1800, ontology, and advanced/e2e Z3 verification.

### Changed
- **Hardware pipeline** — expanded end-to-end pipeline with coverage, sufficiency, decomposition, and CEGAR integration.
- **SVA model** — extended with coverage and decomposition support types.
- **FOL-to-verify translation** — mapping extended to support the new analysis modules.
- **SVA-to-verify pipeline** — translation with consistency and invariant-checking hooks.
- **Equivalence checking** — extended for consistency and cross-property analysis.
- **Knowledge graph** — expanded entity and relation extraction for RTL-spec linking.
- **Parser improvements** — expanded verb recognition and clause parsing for hardware specification sentences.
- **Lexicon expansion** — additional vocabulary for hardware verification domains.

## [0.9.11] - 2026-03-31

### Added
- **Bitvector theory** — `BitVector(n)` and `Array(idx, elem)` types in the verification IR, with full `BitVecOp` enum (bitwise, shift, arithmetic, comparison) and Z3 encoding support.
- **Bounded model checking** — `verify_temporal()` method for unrolling transition relations and checking temporal properties at each step with counterexample generation.
- **Equivalence checking** — new `equivalence.rs` module in `logicaffeine_verify` for structural and semantic equivalence of verification expressions.
- **SVA model expansion** — `Repetition`, `SAlways`, `Stable`, `Changed`, `DisableIff`, `Nexttime`, and `IfElse` expression variants in the SVA model, with full parsing and emission support.
- **SVA surface tests** — new `phase_hw_sva_surface.rs` test suite (606 lines) for SVA parsing and rendering coverage.
- **Z3 equivalence tests** — `phase_hw_z3_equiv.rs` expanded by 512 lines of bitvector and equivalence verification tests.
- **Knowledge graph expansion** — entity and relation extraction with 122 new lines in `knowledge_graph.rs`.
- **Kripke model updates** — extended Kripke structure support for temporal reasoning (48 new lines).

### Changed
- **Parser improvements** — expanded quantifier handling, clause parsing, and verb recognition for hardware specification sentences.
- **FOL-to-verify translation** — mapping from first-order logic to verification IR, supporting bitvector and array expressions.
- **SVA-to-verify pipeline** — translation from SVA model to verification IR with support for new temporal operators.
- **Verification solver** — extended Z3 encoding for bitvector operations, array select/store, and temporal unrolling.

## [0.9.10] - 2026-03-30

### Added
- **Hardware verification pipeline** — new `codegen_sva` module for generating SystemVerilog Assertions (SVA) from FOL specifications, with complete lexicon-to-assertion translation.
- **Knowledge graph semantics** — `knowledge_graph.rs` module for extracting structured knowledge graphs from parsed specifications, enabling semantic analysis and equivalence checking.
- **Hardware verification test suites** — 15 new test files (`phase_hw_*.rs`) covering SVA codegen, roundtrip, temporal logic, Z3 equivalence, Futamura projection application, and end-to-end pipeline testing.
- **PE infrastructure improvements** — expanded Futamura projection test suite from 513 to 543 tests with additional coverage.

### Changed
- **Proof engine** — updates to `oracle.rs` and `unify.rs` for improved reasoning.
- **Language infrastructure** — lexicon expansion, parser changes, and semantic analysis updates across the language crate.
- **LSP hover** — expanded hover information in `hover.rs`.

## [0.9.9] - 2026-03-29

### Added
- **PE Map/CCopy support** — all three PE variants (pe_source, pe_bti, pe_mini) now handle `VMap` in `exprToVal`/`valToExpr` and `CCopy` expression evaluation, enabling Map types and copy semantics through partial evaluation.
- **Expression embedding analysis** — new `exprEmbeds()` and `argsStrictlyEmbed()` predicates in pe_source and pe_bti for finer-grained memoization decisions during specialization.
- **Extended key generation** — pe_mini now generates unique memoization keys for 15 additional CExpr variants (CIndex, CLen, CMapGet, CNewSeq, CNewSet, CFieldAccess, CNew, CRange, CSlice, CCopy, CContains, CUnion, CIntersection, etc.).
- **Decompilation infrastructure** — `discover_entry_points()`, `fix_decompiled_types()`, and `generate_block_wrapper()` helpers in `logicaffeine-compile`'s `compile.rs` for automated PE code generation.
- **P3 surface-level language coverage** — 20 new tests verifying all language features through the Third Futamura Projection (structs, maps, seqs, options, variants, control flow, recursion, copy semantics).

### Changed
- **Repeat statement encoding** — simplified by extracting common logic into `encode_stmt_list_src`, removing 77 lines of explicit while-loop lowering.
- **Float literal formatting** — ensures float values always include a decimal point in encoded output.

### Verified
- **All 3 Futamura Projections** — 6,035 tests pass (512 Futamura-specific + full e2e suite), 0 failures, 0 regressions. Jones optimality confirmed. P1 specialization, P2 compiler generation, P3 compiler-generator generation all validated with cross-projection equivalence checks.

## [0.9.8] - 2026-03-19

### Fixed
- **FFI map keys/values type mismatch** — `logos_map_*_keys()` and `logos_map_*_values()` stored raw `Vec<T>` in the handle registry but downstream `logos_seq_*_len()`/`logos_seq_*_free()` cast them to `LogosSeq<T>` (now `Rc<RefCell<Vec<T>>>`). Fixed by wrapping in `LogosSeq::from_vec()` before registration.

## [0.9.7] - 2026-03-19

### Fixed
- **FFI serde for LogosSeq/LogosMap** — `Serialize`/`Deserialize` impls for `LogosSeq<T>` and `LogosMap<K, V>`, delegating through the `Rc<RefCell<…>>` wrappers to the inner `Vec`/`FxHashMap`. The FFI codegen emits `serde_json` round-trips on these types, but the wrappers had lacked serde impls since the reference-semantics change in 0.9.4.

## [0.9.6] - 2026-03-19

### Fixed
- **Studio file browser** — broken since 0.9.0. Dioxus 0.7 stopped serving `assets/` directly; moved `opfs-worker.js` and `style.css` to `public/assets/` so the OPFS web worker and stylesheet load correctly. Studio sidebar now shows files again.
- **CI deploy** — `robots.txt`, `sitemap.xml`, `_redirects` moved from `assets/` to `public/`, eliminating the manual copy step in `deploy-frontend.yml`.

### Added
- **Local Vec optimization** — escape analysis (`collect_escaping_collection_vars`) identifies collection variables that never leave the function boundary. These are stored as `Vec<T>` with zero-overhead indexing instead of `LogosSeq<T>` (Rc<RefCell<Vec<T>>>).
- **Borrow parameter optimization** — readonly `Seq<T>` parameters emit `&[T]` borrows; mutable-only parameters emit `&mut [T]`. Borrow types propagate through aliases.
- **Function return type tracking** — codegen context tracks function return types for call-site type inference.

### Changed
- **Peephole patterns** — slice/push/extend/drain patterns updated to handle both `Vec<T>` and `LogosSeq<T>` sources.
- **Test infrastructure** — offline-first `cargo build`/`cargo run` (tries `--offline` first, falls back to online). Increased `RUST_MIN_STACK` for deeply recursive AST walks.

## [0.9.5] - 2026-03-17

### Fixed
- **LogosMap/LogosSeq escape-block APIs** — `LogosMap::values()`/`keys()` returning owned `Vec`, and `LogosSeq: From<Vec<T>>`, for escape-block interop.
- **Showable blanket impl** — `&T: Showable` where `T: Showable`, fixing nested inspect arms with ref-bound pattern variables.
- **Escape block updates** — binary search, map access, and list construction escape tests updated for the `LogosSeq`/`LogosMap` APIs.

## [0.9.4] - 2026-03-16

### Added
- **PE BTI source** (`optimize/pe_bti_source.logos`, 1215 LOC) — binding-time improved partial evaluator source for Futamura Projection 2.
- **PE mini source** (`optimize/pe_mini_source.logos`, 785 LOC) — minimal clean-room partial evaluator for Futamura Projection 3.
- **Decompile source** (`optimize/decompile_source.logos`, 645 LOC) — source-level decompiler for PE output.
- **Reference semantics tests** (`e2e_ref_semantics.rs`, 191 lines) — end-to-end tests for Rc-based reference semantics.
- **~220 new partial-evaluation tests** — genuine self-application, Sprint I/J infrastructure, PE mini/BTI verification.

### Changed
- **Self-interpreter** (`compile.rs`) — expanded by 974 lines covering value handling, environment management, and PE integration.
- **PE source** (`pe_source.logos`) — expanded by 948 lines with specResults memoization, makeKey collision fixes, MSG wiring, BTA SCC support, and partially-static data handling.
- **Partial evaluator** (`partial_eval.rs`) — 264 lines of improvements: staticEnv copy propagation, CInspect field binding, mixed-arg CCall inlining, extractReturn sentinel fix.
- **Supercompiler** (`supercompile.rs`) — 81 lines of driving/generalization changes.
- **BTA** (`bta.rs`) — 61 lines: SCC wiring, isStatic expansion for CNew/CRange/CCopy.
- **Codegen peephole** (`peephole.rs`) — 152 lines of new optimization patterns.
- **Codegen stmt/expr** — 239 lines of emission improvements across `stmt.rs` and `expr.rs`.
- **Data types** (`types.rs`) — 260 lines: LogosSeq/LogosMap reference-semantic wrappers with interior mutability.
- **Indexing** (`indexing.rs`) — 58 lines: new indexing operations for reference-semantic collections.

### Fixed
- **Peephole mutability** — `vec_fill`, `vec_with_capacity`, sibling collection, and buffer reuse patterns now emit `let mut` when the LOGOS source declares `mutable`.
- **Conditional/unconditional swap** — LogosSeq swap codegen uses `__swap_tmp` inside `borrow_mut()` scope instead of `.swap()`, matching the codegen spec.

## [0.9.3] - 2026-03-06

### Added
- **Futamura projections** — all three projections implemented and verified with 276 tests (`phase_futamura.rs`):
  - **Projection 1**: `pe(interpreter, program) = compiled` — partial evaluation specializes the self-interpreter on a source program to produce a compiled executable.
  - **Projection 2**: `pe(pe, interpreter) = compiler` — partial evaluation applied to itself with the interpreter produces a standalone compiler.
  - **Projection 3**: `pe(pe, pe) = compiler_generator` — partial evaluation applied to itself twice produces a compiler generator.
- **Self-interpreter** — complete LOGOS interpreter written in LOGOS itself (`compile.rs`), supporting all core value types (int, float, text, bool, nothing, seq, map, set, error, crdt), arithmetic, comparisons, control flow, functions, recursion, and data structures.
- **Binding-time analysis** (`optimize/bta.rs`, 758 LOC) — classifies expressions as static or dynamic for partial evaluation, with 33 tests (`phase_bta.rs`).
- **Partial evaluator** (`optimize/partial_eval.rs`, 960 LOC) — online partial evaluation with environment-based specialization, function unfolding, and residual code generation, with 35 tests (`phase_partial_eval.rs`).
- **Supercompiler** (`optimize/supercompile.rs`, 878 LOC) — driving, generalization, and homeomorphic embedding for aggressive program specialization, with 59 tests (`phase_supercompile.rs`).
- **PE source language** (`optimize/pe_source.logos`, 556 LOC) — the partial evaluator's own source in LOGOS, enabling self-application.
- **Abstract interpretation** (`optimize/abstract_interp.rs`, 668 LOC) — forward abstract interpretation framework with interval and sign domains, with 16 tests (`phase_abstract_interp.rs`).
- **Deforestation** (`optimize/deforest.rs`, 539 LOC) — eliminates intermediate data structures in compositions, with 8 tests (`phase_deforestation.rs`).
- **Effect analysis** (`optimize/effects.rs`, 698 LOC) — tracks computational effects (pure, IO, mutation, divergence) for optimization safety, with 19 tests (`phase_effects.rs`).
- **Global value numbering** (`optimize/gvn.rs`, 483 LOC) — hash-consing based redundant computation elimination.
- **Loop-invariant code motion** (`optimize/licm.rs`, 391 LOC) — hoists loop-invariant expressions out of loops.
- **Compile-time function evaluation** (`optimize/ctfe.rs`, 443 LOC) — evaluates pure functions at compile time.
- **Closed-form optimization** (`optimize/closed_form.rs`, 354 LOC) — replaces simple loops with closed-form expressions.
- **Polyhedral optimization** tests (`phase_polyhedral.rs`) — 6 tests for polyhedral loop transformations.
- **Auto-parallelization** tests (`phase_autoparallel.rs`) — 6 tests for automatic parallelization detection.
- **Mountain climb** tests (`phase_mountain_climb.rs`) — 27 tests for optimization composition.
- **~550 new tests** across 10 new test files and expanded existing test files.

### Changed
- **Optimizer pipeline** — extended with constant propagation, expanded DCE, and new optimization passes. Pipeline: fold → propagate → dce with expanded pattern coverage.
- **Codegen peephole optimizations** — extended with new patterns including `(j+1)-1 → j` index simplification.
- **DCE** — expanded from basic dead-code elimination to include unused variable removal and dead-store elimination (282 lines added).
- **Constant folding** — extended with additional algebraic simplifications (90 lines added).
- **Constant propagation** — added safety guards for index/slice contexts (49 lines added).

### Fixed
- **Self-interpreter error casing** — `valToText` returned `"error: {msg}"` (lowercase) instead of `"Error: {msg}"` (capital E).
- **PE substring false positives** — renamed `CEscapeExpr`/`CEscapeStmt` to `CEscExpr`/`CEscStmt` to eliminate false substring matches with `peExpr`/`peStmt` in Futamura projection tests.
- **Codegen boilerplate false positives** — pre-computed stack size literal (`67_108_864`) and used function pointer (`_logos_main`) to avoid `* 1` and `||` matches in optimizer test assertions.
- **FFI snapshot refresh** — regenerated 5 FFI codegen snapshots to match current output.

## [0.9.2] - 2026-02-28

### Fixed
- **Benchmark timeout isolation** — each language now runs in its own hyperfine invocation with an independent timeout. Previously, all 11 languages ran in a single hyperfine call; when one language (e.g., Python on ackermann n=11) exceeded the timeout, `run_timeout` killed the entire process and remaining languages (JS, Ruby, Nim, LOGOS) were never measured. Per-language results are merged into the same JSON format so downstream assembly is unchanged.
- **Smart timeout skipping** — if a language times out at size N, it is automatically skipped for all larger sizes of that benchmark, avoiding wasted CI minutes on languages known to be too slow.
- **Trimmed benchmark sizes** — removed noise sizes from all 32 `sizes.txt` files where C runtime is <10ms (process startup dominates measurement). Keeps 3-5 meaningful sizes per benchmark for real scaling data.
- Applied to all three benchmark scripts: `run.sh`, `run-quick.sh`, `run-logos-vs-c.sh`.

## [0.9.1] - 2026-02-28

### Fixed
- **Benchmark CI completely broken** — all hyperfine calls used `--timeout` flag which doesn't exist in any version of hyperfine, causing every benchmark to fail with zero data and empty geometric mean results. Removed the invalid flag; the `run_timeout` wrapper already handles timeouts via the system `timeout` command.

## [0.9.0] - 2026-02-27

### Added
- **Bidirectional type checker** — Robinson unification-based inference (`analysis/check.rs`, `analysis/unify.rs`). Eliminates `Unknown` types for field access, empty collections, option literals, pipe receives, inspect arm bindings, and closure calls.
- **Call graph analysis** (`analysis/callgraph.rs`) — whole-program call graph with Kosaraju SCC detection for readonly and purity analysis.
- **Liveness analysis** (`analysis/liveness.rs`) — backward dataflow computing per-statement live-after sets, enabling last-use move optimization.
- **Read-only parameter inference** (`analysis/readonly.rs`) — fixed-point iteration over the call graph identifies `Seq<T>` parameters never mutated, emitting `&[T]` borrows instead of clones.
- **Bitwise operators** — `x xor y`, `x shifted left by y`, `x shifted right by y`.
- **Break statement** — `Break.` exits the innermost while loop.
- **Unary NOT** — `not x` for logical and bitwise negation.
- **Generic function type parameters** — polymorphic type variable declarations on functions.
- **Triple-quote strings** — `"""multi-line"""` with automatic indentation stripping.
- **Scientific notation** — `4.84e+00`, `2.5e-2` in numeric literals.
- **io_uring VFS** (`logicaffeine_system`) — Linux kernel-async file I/O via dedicated worker thread.
- **~392 new tests** across 7 new test files and expanded suites covering type checker, bitwise ops, break, codegen optimization, math builtins, string interpolation, and optimizer features.

### Changed
- **Codegen architecture** — monolithic `codegen.rs` (8,300 lines) split into 13 modules (`context`, `detection`, `expr`, `stmt`, `peephole`, `program`, `ffi`, `marshal`, `policy`, `bindings`, `tce`, `types`). Public API preserved via re-exports.
- **C backend architecture** — `codegen_c.rs` (2,000 lines) split into 4 modules (`emit`, `runtime`, `types`).
- **Compilation pipeline** — `check_program()` now runs between analysis and codegen, producing a `TypeEnv` for optimization decisions.
- **FxHashMap/FxHashSet** — generated code uses `rustc-hash` for faster integer-key hashing.
- **15 codegen optimizations** — last-use clone elimination, liveness-based move, sentinel exit detection, dead post-loop counter elimination, HashMap `.get()` for comparisons, string byte comparison, self-append via `write!`, flattened string concatenation, read-only `&[T]` borrows, `Vec::with_capacity`, `assert_unchecked` for proven bounds, raised inline threshold, power-of-2 modulo strength reduction, `target-cpu=native`.
- **Benchmark suite** — expanded from ~6 to 30+ programs with multi-language implementations and correctness verification.
- **C backend** — extended with interpolated strings, bitwise ops, shifts, enums, slices, sets, and multiple map type variants.

## [0.8.19] - 2026-02-15

### Added
- **Swap pattern regression test** — codegen and E2E tests verifying `.swap()` fires for the bubble_sort benchmark pattern (nested while loops, inferred Vec type via `new Seq of Int`).

### Changed
- **Benchmark warm-ups and runs increased** — runtime benchmarks now use 5 warm-ups and 20 runs (up from 3/10) to reduce variance between versions.
- **Zig 0.15 upgrade** — all 6 Zig benchmark programs updated from Zig 0.13 to 0.15 API (`std.io.getStdOut()` → `std.fs.File.stdout().writer(&buf)`, ArrayList now unmanaged). CI updated from Zig 0.13.0 to 0.15.2.

## [0.8.18] - 2026-02-15

### Fixed
- **Constant propagation string safety** — propagation no longer substitutes `Literal::Text` (String) values. String is non-Copy in Rust, so substituting string literals into multiple use sites created independent `String::from(...)` allocations, hiding use-after-move errors (E0382) from rustc.
- **Constant propagation zone scoping** — zone-scoped `Let` bindings are no longer registered in the propagation environment. Previously, propagating a zone-scoped variable outward replaced `Expr::Identifier` with `Expr::Literal`, which the escape checker treated as safe — hiding zone escape violations (E0597).

## [0.8.17] - 2026-02-15

### Added
- **C codegen backend** — `compile_to_c()` produces self-contained C files with embedded runtime (Seq, Map, Set, string helpers, IO). Compiles with `gcc -O2`. Supports integers, floats, booleans, strings, collections, control flow, functions.
- **Constant propagation** optimizer pass — forward substitution of immutable constants, chained with fold and DCE. Safety: skips Index/Slice expressions to preserve swap/vec-fill pattern detection.
- **638 new E2E tests** across 21 new test files:
  - 16 Rust codegen mirror files (181 tests) — every interpreter-only feature now also tested through the Rust codegen pipeline
  - `e2e_codegen_gaps.rs` (64 tests) — floats, modulo, options, nothing, collection type combos, struct/enum patterns, control flow, functions, escape blocks, strings
  - `e2e_codegen_optimization.rs` (15 tests) — TCO, constant propagation, DCE, vec-fill, swap, fold, index simplification
  - `e2e_interpreter_gaps.rs` (60 tests) — interpreter counterparts for gap coverage
  - `e2e_interpreter_optimization.rs` (14 tests) — interpreter counterparts for optimization correctness
  - `phase_codegen_c.rs` (214 tests) — C-backend codegen coverage

### Fixed
- **For-range guard for complex expressions** — `While i is at most length of items` no longer produces `_` in generated Rust. Added `is_simple_expr()` guard.
- **For-range post-loop value for empty loops** — empty loops now correctly keep the counter at its start value using `max(start, limit)`.
- **Vec-fill pattern relaxed mutability** — `Let items be a new Seq of Bool` (without explicit `mutable`) now matches the vec-fill optimization.
- **C codegen missing Set variants** — `SetI64` and `SetStr` added to `c_type_str()`.
- **Interpreter float comparison** — `apply_comparison` now handles Float-Float, Int-Float, and Float-Int comparisons.

## [0.8.16] - 2026-02-15

### Fixed
- **For-range loop regression**: `RangeInclusive` (`..=`) has a known per-iteration overhead in Rust due to internal bookkeeping for edge cases. In O(n^2) inner loops like bubble sort, this compounded to a 41.4% regression vs the `while` loop it replaced. All inclusive ranges now emit exclusive form: `for i in 1..=n` becomes `for i in 1..(n + 1)`. For literal limits, the addition is computed at compile time (e.g. `for i in 1..6` instead of `for i in 1..=5`).

## [0.8.15] - 2026-02-15

### Added
- **TIER 1 codegen optimizations** — five peephole-level improvements targeting array-heavy benchmark performance
  - **For-range loop emission**: `Let i be 1. While i <= n: ... Set i to i + 1` compiles to `for i in 1..(n + 1)` instead of `while (i <= n)`, enabling LLVM trip count recognition, unrolling, and vectorization
  - **Iterator-based loops**: `Repeat for x in items` emits `.iter().copied()` instead of `.clone()` for Copy-type collections (`Vec<i64>`, `Vec<f64>`, `Vec<bool>`), eliminating full-collection copies
  - **Direct array indexing for list literals**: `Let items be [10, 20, 30]` now registers element type, enabling direct `arr[(idx-1) as usize]` instead of `LogosIndex` trait dispatch
  - **Vec fill exclusive bound**: `While i < n: Push 0 to items` now optimizes to `vec![0; n]` (previously only `<=` was matched)
  - **Swap pattern equality comparisons**: `If a equals b: swap` and `If a is not b: swap` now optimize to `arr.swap()`
- 24 new optimizer tests across all 5 TIER 1 items (codegen assertions + E2E correctness)

## [0.8.14] - 2026-02-15

### Added
- **TIER 0 optimizer bedrock** — deep expression recursion, unreachable-after-return DCE, algebraic simplification
  - Constant folding now recurses into all 26 Expr variants (function call args, list literals, index expressions, struct constructors, Option/Maybe Some, Contains, Closure bodies)
  - Statements after `Return` are eliminated as dead code
  - Algebraic identity/annihilator rules for both int and float: `x + 0 → x`, `x * 1 → x`, `x * 0 → 0`, `x - 0 → x`, `x / 1 → x`
- **Maybe syntax** — `Maybe Int` and `Maybe of Int` as dual syntax for `Option of Int`
  - Supports direct Haskell-style `Maybe T` (no "of" required) and consistent `Maybe of T`
  - Works in all positions: variable annotations, return types, nested generics
- 33 new optimizer and type expression tests

### Fixed
- Publish workflow: skip already-published crates instead of failing

## [0.8.13] - 2026-02-14

### Added
- **Accumulator introduction** — converts `f(n-1) + k` and `n * f(n-1)` into zero-overhead loops
- Automatic memoization: pure multi-branch recursive functions get thread-local HashMap cache
- Mutual tail call optimization: pairs like `isEven`/`isOdd` merged into single loop with tag dispatch
- Purity analysis: fixed-point dataflow to identify side-effect-free functions
- 14 new optimizer tests (accumulator, memoization, mutual TCO — codegen + E2E correctness)

## [0.8.12] - 2026-02-14

### Added
- **Closures and interpreter expansion** — closure support (`e2e_closures.rs` and parser changes) and a buildout of the tree-walking interpreter (`interpreter.rs`), alongside the initial bytecode VM design plan (`VM_PLAN.md`).

## [0.8.11] - 2026-02-14

### Added
- **Peephole vec-fill pattern** — `vec![val; count]` instead of push loop
- Peephole optimizer: swap pattern (`arr.swap()` instead of temp variable assignments)
- Copy-type elision: skip `.clone()` on Vec/HashMap indexing for primitive types
- HashMap equality optimization: `map.get()` instead of `map[key].clone()` for comparisons

### Changed
- Release profile: `opt-level = 3`, `codegen-units = 1`, `panic = "abort"`, `strip = true`
- `#[inline]` on Value arithmetic, LogosDate/LogosMoment accessors, parseInt/parseFloat
- Variable type tracking threaded through all expression codegen paths

## [0.8.10] - 2026-02-14

### Changed
- **Direct collection indexing** — codegen for known Vec/HashMap types avoids trait dispatch
- `#[inline(always)]` on all Showable, LogosContains, LogosIndex trait impls
- `get_unchecked` after validated bounds in Vec indexing (removes redundant bounds check)
- LTO enabled in release profile for generated projects

## [0.8.9] - 2026-02-14

### Fixed
- **Benchmark CI `actions: write` permission** — granted the benchmark workflow `actions: write` so it can trigger the frontend deploy.

## [0.8.8] - 2026-02-14

### Fixed
- **Benchmark CI dirty-checkout** — `latest.json` is saved to `/tmp` before `git checkout main`, fixing the checkout failure when the working tree held uncommitted benchmark results.

## [0.8.7] - 2026-02-14

### Fixed
- **Frontend deploy ordering** — the deploy triggers after benchmarks commit fresh results, and the deploy checkout always uses the latest `main` HEAD (`deploy-frontend.yml`).
- **Benchmark size and stability** — dropped the two largest sizes per benchmark, fixed a JS ackermann stack overflow, and worked around a `jq` "argument list too long" via a temp file.

## [0.8.6] - 2026-02-13

### Fixed
- **FFI test CI fixes** — refreshed the FFI codegen snapshots, fixed the Zig benchmark builds, and fixed a Ruby stack overflow in the benchmark suite.

## [0.8.5] - 2026-02-13

### Fixed
- **CI mold linker config** — removed the mold linker setting from `.cargo/config.toml`, which broke CI, and began tracking benchmark `latest.json` in-repo so the frontend bakes current results.

## [0.8.4] - 2026-02-13

### Added
- **Multi-language benchmark suite** — benchmark programs in 10 languages (C, C++, Rust, Go, Zig, Nim, Python, Ruby, JS, Java) with `benchmarks/run.sh`, a benchmark CI workflow, and the interactive benchmarks web page.
- **Interpreter map keys** — map key support in the tree-walking interpreter (`phase_interpreter_map_keys.rs`).
- **Standard library and interpreter-benchmark coverage** — `phase38_stdlib.rs` and `phase_benchmark_interp.rs`, plus an expanded `e2e_feature_matrix.rs`.
- **nextest configuration** — `cargo-nextest` runner config for the test suite.

## [0.8.3] - 2026-02-12

### Fixed
- **FFI C-linkage test gating** — gated behind the `ffi-link-tests` feature for CI compatibility
- Platform-aware linker flags for C ABI tests (macOS + Linux)

## [0.8.2] - 2026-02-12

### Added

- **Optimizer infrastructure** — constant folding and dead code elimination
- Interpreter mode (`largo run --interpret`) for sub-second development feedback
- Map insertion syntax (`Set X at KEY to VALUE`)

### Changed

- FFI/C export safety: thread-local error cache, panic boundaries, null handle checks, dynamic `logos_version()`
- `LogosHandle` from `*const c_void` to `*mut c_void`
- Text/String excluded from C ABI value types

### Fixed

- UTF-8 string indexing in interpreter (`.len()` → `.chars().count()`)
- Rust keyword escaping in generated FFI code

## [0.8.1] - 2026-02-12

### Changed

- **Version bump** — republished the workspace at 0.8.1; no functional change (the `@types/node` VSCode devDependency fix landed under the 0.8.0 tag).

## [0.8.0] - 2026-02-10

### Added

- **LSP server** — definitions, references, hover, completions, rename, semantic tokens, diagnostics
- VSCode extension with bundled LSP binaries for 5 platforms
- FFI/C export system for cross-language interop
- CI/CD workflows for release, publish, and deployment

### Changed

- Compiler improvements and bug fixes

## [0.7.0] - 2026-02-01

### Added

- **E2E test suite expansion** — temporal, escape-hatch, FFI, and concurrency test coverage.

### Changed

- **Concurrency and async hardening** — expanded and fixed the concurrency/async runtime; the escape-analysis and concurrency modules predate this release.
- **Web platform** — studio IDE, mobile responsiveness, homepage redesign.

### Fixed

- 10 compiler bugs across codegen, parser, and map handling.
- Async and concurrency correctness issues.

## [0.6.0] - 2026-01-17

Initial crates.io release with lockstep versioning.

### Changed

- Synchronized all crate versions to 0.6.0 (lockstep versioning)
- Added CHANGELOG.md to every crate
- Added VERSIONING.md with release process documentation

## [0.5.5] - 2026-01-01

First public release.

### Added

**Compiler**
- Z3 SMT solver integration for static verification (`logos_verification` crate)
- Refinement type syntax with `where` clauses
- DRS (Discourse Representation Structures) for donkey anaphora
- Event adjective analysis ("Olga is a beautiful dancer")
- Escape analysis for memory safety
- Diagnostic system with source mapping

**Runtime (`logos_core`)**
- Standard library: `env`, `file`, `random`, `time` modules
- CRDT support: GCounter, LWW registers
- Memory zones for region-based allocation

**Tooling**
- CLI tool (`largo`) for project management
- Package registry with publish/download
- GitHub Actions for CI/CD deployment
- Rust code formatter for generated output

**Web Platform**
- Learning platform with interactive curriculum
- Vocabulary reference component
- User profile page
- Universal navigation

**Tests**
- End-to-end test suite (collections, functions, structs, enums, etc.)
- Phase 41: Event adjectives
- Phase 42: DRS
- Phase 50: Security policy analysis
- Phase 85: Memory zones
- Grand challenge: Mergesort example

### Core Features (v0.5.5)

**Logic Mode** - English → First-Order Logic
- Quantifiers: universal, existential, negative, cardinal
- Modal operators: necessity, possibility, deontic
- Temporal logic: tense, aspect
- Wh-questions, relative clauses, reflexives, reciprocals
- Scope ambiguity resolution
- Parse forests for structural ambiguity

**Imperative Mode** - English → Rust
- Variables, mutation, control flow
- Functions with typed parameters
- Structs and enums with pattern matching
- Collections with 1-based indexing
- Generics (`Seq of Int`, `Box of [T]`)
