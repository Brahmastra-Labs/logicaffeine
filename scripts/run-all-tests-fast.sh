#!/usr/bin/env bash
#
# run-all-tests-fast.sh — the full baseline test run, fast.
#
# Covers the exact same surface as run-all-tests.sh (every crate, the Z3-backed
# verification tests, the #[ignore]d tests, unit tests, integration tests, and
# doctests) but schedules individual tests across all cores with cargo-nextest
# instead of running ~420 test binaries one at a time.
#
# Three passes:
#   1. cargo nextest run  — every unit + integration test (nextest cannot run
#      doctests, hence pass 2)
#   2. cargo test --doc   — every doctest, mirroring the baseline's inline run
#   3. scripts/lean/conformance-verify-verdicts.sh — the Lean-parity scoreboard checked
#      against reality. lean_parity_matrix.rs proves every corpus case is
#      CLASSIFIED; nothing proved a case recorded `Native` actually PASSES. This
#      pass runs all 16 corpus binaries and joins the real results against
#      crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/*.json, so the scoreboard cannot lie in either
#      direction. Cheap: pass 1 already built those binaries.
#
# Output is streamed to logs/test-run-fast-<timestamp>.log and a
# logs/latest-fast.log symlink always points at the most recent run. A summary
# — wall-clock time, totals, and any failures — is printed at the end.
#
# Verify parity against a baseline run with:
#   ./scripts/compare-test-runs.sh logs/latest.log logs/latest-fast.log
#
# Usage: ./scripts/run-all-tests-fast.sh [--no-ignored]
#
#   --no-ignored   Skip #[ignore]d tests (drops the multi-minute fuzz/bench
#                  monsters — fastest iteration loop, NOT baseline-parity).
#
set -uo pipefail

RUN_IGNORED=1
# Doctests run AFTER the nextest passes, not overlapped with them. The overlap was worth ~88s of a
# ~74-minute run (2%), and it cost a phantom 11-test doctest failure on every run: `cargo test --doc`
# builds the workspace lib for rustdoc into the SAME target/test-opt that the nextest passes build
# their binaries into, and whichever finishes second can replace an rlib the first's rustdoc phase is
# still resolving — every one of those 11 was the identical `extern location for logicaffeine_compile
# does not exist: …rlib`, and the same doctests pass alone. cargo's build LOCK serializes the builds;
# it does not stop the eviction. `--overlap-doctests` opts back in for anyone who wants the 2%.
OVERLAP_DOCTESTS=0
# `--exclude <crate>` (repeatable) leaves a crate OUT of both workspace passes — for a baseline of everything
# else while one crate's test target is broken mid-refactor (a target that does not compile aborts the whole
# nextest pass, so without this the other crates get no verdict at all). The exclusion is printed in the
# summary so the log can never pass as a full-workspace baseline.
EXCLUDE_CRATES=()
while [ $# -gt 0 ]; do
  case "$1" in
    --no-ignored) RUN_IGNORED=0 ;;
    --sequential) OVERLAP_DOCTESTS=0 ;;
    --overlap-doctests) OVERLAP_DOCTESTS=1 ;;
    --exclude) [ -n "${2:-}" ] || { echo "--exclude needs a crate name" >&2; exit 2; }; EXCLUDE_CRATES+=("$2"); shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done
EXCLUDE_ARGS=()
for c in "${EXCLUDE_CRATES[@]}"; do EXCLUDE_ARGS+=(--exclude "$c"); done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# --- environment -------------------------------------------------------------
# Z3 header for the `verification` feature (Linux apt path; override if needed).
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
# This box carries the pinned Lean 4.14.0 toolchain: a `largo lean` project-mode test that cannot find it
# must FAIL, never fall back to asserting the refusal path (competition rule R2). It also carries headless
# Chrome: the browser page tier (`browser_*` in crates/logicaffeine_tests/tests) FAILS without it rather than
# skipping. And it carries `reference-kernels/`, the Lean exports the Arena heavy cases and the NbE
# differential read: a clone without them (CI) records each as a counted skip, and this run fails instead. And
# it carries `eda-tools`, the machine-local EDA tree under `tools/` (OpenLane with the hardened designs, the
# Caravel checkouts) the silicon evidence locks re-derive from, on the same terms. And it carries `private-guard`,
# the private-material guard installed in the git directory (`private_names_lock` scans the tracked tree with it).
# The list is comma-separated (`tool_required_by`); `browser_tier_cannot_skip_lock`, `arena_corpus_lock`,
# `eda_tools_seam_lock` and `private_names_lock` read it.
export LOGOS_REQUIRE_TOOLS="${LOGOS_REQUIRE_TOOLS:-lean,chrome,reference-kernels,eda-tools,private-guard}"
# Make cargo available in non-login/background shells.
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

# Memory-capped qemu-riscv64 (scripts/test/toolwrap) resolves ahead of /usr/bin. qemu-user maps
# the guest's allocations into its OWN host process with no guest kernel in between, so a
# runaway RISC-V program becomes a runaway HOST process — measured at ~126 GB RSS on this
# 125 GiB box, which OOM-kills the entire user slice: every SSH session and the VS Code
# server, mid-run. nextest's slow-timeout cannot catch it (memory has no timeout; qemu goes
# 2 GB -> 126 GB well inside the 900s warning). The wrapper caps RLIMIT_AS so the guest gets
# a clean malloc failure in seconds and the test reports a real failure instead of the
# machine falling over. Override with LOGICAFFEINE_QEMU_MEM_MB (0 disables); see the header
# of scripts/test/toolwrap/qemu-riscv64.
export PATH="$REPO_ROOT/scripts/test/toolwrap:$PATH"


# --- load governor, one suite at a time, one heavy job at a time ------------
# The brakes are shared with scripts/test/tdd.sh; each one's history and rationale lives in
# scripts/lib/suite-guard.sh, and the heavy jobs are the ones scripts/test/heavy-jobs.sh names.
source "$SCRIPT_DIR/lib/suite-guard.sh"
suite_guard_brakes
suite_guard_require_nextest || exit 1
suite_guard_refuse_overlap || exit 1
suite_guard_refuse_heavy "$REPO_ROOT/scripts/test/heavy-jobs.sh" || exit 1

# --- and room on its disks ---------------------------------------------------
# A full run writes tens of gigabytes of test scratch into $TMPDIR (the cargo projects, e2e caches and
# per-process lane directories the tests build) and rebuilds into its target directory. A run that fills
# its disk fails every test after the first "No space left on device" for a reason that is not the
# test's own, and every other build on that disk with it, so a run without room is refused before it
# builds anything.
if ! "$REPO_ROOT/scripts/test/disk-headroom.sh" 150 "${TMPDIR:-/tmp}" "${CARGO_TARGET_DIR:-$REPO_ROOT/target}"; then
  echo "Refusing to start: free space on that disk first." >&2
  exit 1
fi

# --- log file ----------------------------------------------------------------
LOG_DIR="$REPO_ROOT/logs"
mkdir -p "$LOG_DIR"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
LOG_FILE="$LOG_DIR/test-run-fast-$TIMESTAMP.log"
ln -sfn "$(basename "$LOG_FILE")" "$LOG_DIR/latest-fast.log"

# --- provenance header -------------------------------------------------------
# Branch read from .git/HEAD directly (no git invocation needed).
BRANCH="unknown"; COMMIT="unknown"
if [[ -f "$REPO_ROOT/.git/HEAD" ]]; then
  HEAD_REF="$(cut -d' ' -f2 "$REPO_ROOT/.git/HEAD" 2>/dev/null)"
  if [[ "$HEAD_REF" == refs/* ]]; then
    BRANCH="${HEAD_REF#refs/heads/}"
    [[ -f "$REPO_ROOT/.git/$HEAD_REF" ]] && COMMIT="$(cut -c1-12 "$REPO_ROOT/.git/$HEAD_REF")"
  else
    BRANCH="(detached)"; COMMIT="$(printf '%s' "$HEAD_REF" | cut -c1-12)"
  fi
fi

{
  echo "════════════════════════════════════════════════════════════════════"
  echo " logicaffeine — full test baseline (FAST / nextest)"
  echo "════════════════════════════════════════════════════════════════════"
  echo " started : $(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo " host    : $(hostname)"
  echo " branch  : $BRANCH @ $COMMIT"
  echo " rustc   : $(rustc --version 2>/dev/null)"
  echo " nextest : $(cargo nextest --version 2>/dev/null | head -1)"
  echo " z3      : $(z3 --version 2>/dev/null)"
  if (( RUN_IGNORED )); then
    echo " command : cargo nextest run --workspace --features verification,logicaffeine-tests/web-tests --profile full --cargo-profile test-opt --run-ignored all"
    echo "         + cargo nextest run -p logicaffeine-compile --features wasm-jit --cargo-profile test-opt --lib \$(scripts/test/wasm-jit-targets.sh) -E \"\$(cat scripts/test/wasm-jit-filter.txt)\""
    echo "         + cargo test --workspace --features verification,logicaffeine-tests/web-tests --profile test-opt --doc --no-fail-fast -- --include-ignored"
    echo "         + scripts/lean/conformance-verify-verdicts.sh   (Lean-parity verdicts vs reality)"
  else
    echo " command : cargo nextest run --workspace --features verification,logicaffeine-tests/web-tests --profile full --cargo-profile test-opt   [--no-ignored]"
    echo "         + cargo nextest run -p logicaffeine-compile --features wasm-jit --cargo-profile test-opt --lib \$(scripts/test/wasm-jit-targets.sh) -E \"\$(cat scripts/test/wasm-jit-filter.txt)\""
    echo "         + cargo test --workspace --features verification,logicaffeine-tests/web-tests --profile test-opt --doc --no-fail-fast"
    echo "         + scripts/lean/conformance-verify-verdicts.sh   (Lean-parity verdicts vs reality)"
  fi
  echo "════════════════════════════════════════════════════════════════════"
  echo
} | tee "$LOG_FILE"

START_EPOCH="$(date +%s)"

# --- tracked writes -----------------------------------------------------------
# A test compares against a committed file; it never rewrites one. The run snapshots every tracked file that
# differs from HEAD now, and again after its last pass: any tracked file the run changed turns it RED.
TRACKED_WRITES="$SCRIPT_DIR/lib/tracked-writes.sh"
TRACKED_BEFORE="${LOG_FILE%.log}.tracked-before"
bash "$TRACKED_WRITES" snapshot "$REPO_ROOT" "$TRACKED_BEFORE"

# --- pass 2 (doctests): launched FIRST, in the background, to OVERLAP pass 1 ----
# Doctests are pure compile-bound work nextest cannot schedule; running them
# serially after nextest wastes the cores the nextest tail leaves idle. On this
# box (dedicated to test runs) we start them up front and let them run concurrently
# with pass 1 — the doctest snippet-compile overlaps the nextest RUN, hiding ~the doctest
# time. They are COLLECTED before pass 1b, whose different feature set shares the same
# target dir and would otherwise evict artifacts this run still needs (see there).
# `--sequential` restores the strictly-ordered passes.
DOCTEST_LOG="$LOG_DIR/doctest-$TIMESTAMP.log"
DOCTEST_IGNORED_ARGS=()
(( RUN_IGNORED )) && DOCTEST_IGNORED_ARGS+=(--include-ignored)
# --- preconditions of the #[ignore]d heavies ---------------------------------------
# ntt_logos's AOT races time what a user gets from `largo build --release`, so they drive the
# RELEASE `largo` binary at target/release/largo and fail loud when it is absent. A run that
# includes the ignored tests builds it first, here, so the races measure the tree rather than
# whatever binary a previous build happened to leave behind. Built before the overlapped doctests
# start: both take the target directory's build lock.
if (( RUN_IGNORED )); then
  {
    echo "──────────────────────────────────────────────────────────────────────"
    echo " PRECONDITIONS — the release largo the ignored AOT races drive"
    echo "──────────────────────────────────────────────────────────────────────"
    cargo build -p logicaffeine-cli --release 2>&1
    echo "release largo build exit: $?"
  } | tee -a "$LOG_FILE"
fi
# The ttsim locks measure through `scripts/tt-matmul-measure.sh`, which emits each bundle with a
# `cargo run --features codegen --example …`. Left to the test, that is a cold build of the compile
# crate inside a test under the whole suite's load, and it outlasts the per-test backstop; built here,
# every measurement starts from a warm example.
if [[ -d "${TT_METAL_HOME:-$HOME/tt-metal}/build_Release" ]]; then
  {
    echo "──────────────────────────────────────────────────────────────────────"
    echo " PRECONDITIONS — the bundle emitters the ttsim locks run"
    echo "──────────────────────────────────────────────────────────────────────"
    cargo build -p logicaffeine-compile --features codegen --example emit_spatial_tensix --example emit_tt_tiling 2>&1
    echo "ttsim emitter build exit: $?"
  } | tee -a "$LOG_FILE"
fi

run_doctests() {
  cargo test --workspace "${EXCLUDE_ARGS[@]}" --features verification,logicaffeine-tests/web-tests --profile test-opt --doc --no-fail-fast -- "${DOCTEST_IGNORED_ARGS[@]}" \
    > "$DOCTEST_LOG" 2>&1
}
DOCTEST_BG=""
if (( OVERLAP_DOCTESTS )); then
  run_doctests &
  DOCTEST_BG=$!
fi

# --- pass 1: unit + integration tests (nextest) -------------------------------
{
  echo "──────────────────────────────────────────────────────────────────────"
  echo " PASS 1/2 — cargo nextest run (unit + integration, ignored included)"
  echo "──────────────────────────────────────────────────────────────────────"
} | tee -a "$LOG_FILE"

# The SAT/coNP "P-vs-NP" campaign probes that exercise only product code (the proof engine's
# research instruments themselves live outside the product, in the private research package)
# are EXPLORATORY, not correctness locks: they run minutes-to-HOURS (exhaustive orbit/ladder
# enumeration at growing n) and hung the baseline for 4h+ under --run-ignored. They are
# excluded from the routine run and live in scripts/run-research-tests.sh instead — keep
# the two lists in sync. The profile.full 30-min terminate-after backstops any straggler
# not named here, so a newly-added probe can never silently hang the baseline again.
#
# One #[ignore]d heavy needs the whole box and blows the 30-min terminate-after under concurrent baseline
# load, so it has its own class and runner:
#   • OPT_AUDIT_EXCLUDE — exhaustive_edge_completeness_audit, the exponential optimization-graph completeness
#     certification (2^k opt-disable sweep; ~7 min alone). Its fast companion `phase_opt_graph` locks the
#     baked graph every run. It runs via ./scripts/run-opt-audit-tests.sh.
# ondemand_exclusion_lock keeps it in sync with its runner.
RESEARCH_EXCLUDE='not binary(/^(cofactor_mirror|cofactor_family_counting|rigid_residue_census|sat_census|frege_generator_ladder|martin_lof_omega_kernel|reflection_mirror|orbit_stability_kernel|uniform_transfer_theorem|no_randomness_at_infinity|no_finite_randomness_infinity|family_emergence|ef_class_probe)$/)'
OPT_AUDIT_EXCLUDE='not (test(=exhaustive_edge_completeness_audit))'

# --- heavy ON-DEMAND locks, excluded like the research campaign above -----------
# These PASS on their merits; they simply cannot fit inside a routine baseline, and running
# them here produced failures that looked like code regressions and were not.
#
#   • wpe_postsynth_silicon_lock's 11 `postsynth_*` sims — each runs a real `synth_xilinx`
#     yosys flow and then gate-level simulates the synthesized netlist. Their own #[ignore]
#     reasons declare "~20-30min" / "~30-60min" EACH, and the file's header states the design
#     outright: the slow sims are "the on-demand gold standard", while its FAST construction
#     locks "run in EVERY CI (no yosys/iverilog, milliseconds)" and "keep the pit-of-success
#     closed on every push". Run under `--run-ignored all` they were neither serialized nor
#     affordable: all 11 overlapped, starved each other, and every one hit the 30-min backstop.
#     The fast companions — ramb_model_round_trips_*, heap_stream_depth_is_a_power_of_two_*,
#     wide_state_memories_are_lo_hi_split_* — still run here every time and enforce the exact
#     invariant each silicon fix establishes. The sims run on purpose:
#     ./scripts/run-silicon-tests.sh
#   • fabric_postsynth_lock's `postsynth_*` sim — the S1 decode board as synthesis builds it, stepped and generating
#     on its serial line gate by gate: about an hour. Its fast companions in the fabric locks hold the RTL every time.
#   • d5005_engine_card_lock's co-simulations — the engine build's RTL under Verilator running a step, or the step
#     program decoding the tiny model, with its memory on the card's channels: 600–3,400 s each alone, over an hour
#     each in GATE-3E-4, where four reached their budgets (Tristen, 2026-10-23: on-demand, beside the silicon sims).
#     The card lock's fast tests (the refusal, the host's arguments, the OPAE build and link) still run here, and the
#     engine's RTL locks hold every unit the co-simulations exercise.
#   • hart_soc_lock::the_soc_boots_a_program_from_its_flash — the board's whole computer in RTL booting a program from
#     its flash in lockstep with the board model: 12M cycles of iverilog with the commit trace, 36 min on a loaded box,
#     past the full profile's 30-minute cap (on-demand, beside the silicon sims). The SoC's register, refusal and
#     device tests still run here.
#   • H_honesty_extra::every_verdict_agrees_with_reality — the same shape, one layer out: it shells
#     out to scripts/lean/conformance-verify-verdicts.sh, which builds and runs all 16 corpus
#     binaries. PASS 3 below runs that script DIRECTLY, so under --run-ignored all this test would
#     nest a second full corpus run inside the one that just happened. Its fast companions still
#     run here every time — the_verdict_gate_catches_every_planted_lie (the join fed every lie it
#     exists to catch, milliseconds, no cargo) and
#     the_gates_binary_discovery_agrees_with_an_independent_resolver.
HEAVY_ONDEMAND_EXCLUDE='not ((binary(/^(wpe_postsynth_silicon_lock|fabric_postsynth_lock)$/) and test(/^postsynth_/)) or (binary(hart_soc_lock) and test(=the_soc_boots_a_program_from_its_flash)) or (binary(d5005_engine_card_lock) and test(/^(the_engine_runs_a_step_with_its_memory_on_the_card|the_step_program_profiles_its_prompts_last_position_a_job_at_a_time_and_decodes_as_the_machine|a_faulting_step_is_reported_by_the_engines_run|the_step_program_decodes_on_data_lines_as_the_machine_does|the_step_program_decodes_on_secded_lines_as_the_machine_does|the_step_program_decodes_through_a_flip_in_every_secded_line_as_the_machine_does|the_step_program_decodes_on_the_hybrid_engine_as_the_machine_does|the_step_program_decodes_on_the_recurrent_hybrid_engine_as_the_machine_does|the_step_program_decodes_on_the_vectored_hybrid_engine_as_the_machine_does|the_step_program_decodes_on_the_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding|the_step_program_decodes_on_the_recurrent_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding|the_step_program_decodes_on_the_vectored_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding|the_step_program_decodes_three_sequences_at_once_on_the_vectored_hybrid_engine_as_the_machine_decodes_each|the_greedy_step_program_decodes_six_sequences_at_once_on_four_pairs_a_channel_as_the_machine_decodes_each|the_greedy_step_program_decodes_six_sequences_at_once_with_the_recurrence_at_a_line_a_clock_as_the_machine_decodes_each|the_greedy_step_program_decodes_six_sequences_at_once_with_every_unit_on_the_fast_side_as_the_machine_decodes_each|the_greedy_step_program_decodes_six_sequences_at_once_with_its_recurrence_on_four_head_lanes_as_the_machine_decodes_each|the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_as_the_machine_decodes_each|the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_at_the_cards_clock|the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_at_the_small_models_widths|the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_whatever_it_powers_up_holding)$/))) and not test(every_verdict_agrees_with_reality)'

# Kernel NORTH-STAR specs for a capability the importer does not have yet. The two lazy_conversion
# app-lam controls require the whole app-lam original to convert and publish without expanding its
# captured source. Production sharing cannot publish it at any allowance (charges are per captured
# position, and helper telescopes reach 3,999 > MAX_TERM_DEPTH 1,600); only the test-only certified
# normal form can, and it spends 518,659 of a 100,000 budget. Closing it is a fused evaluator promoted
# into the production importer. They are `#[ignore]`d with a `kernel-pending:` reason, run on purpose
# via ./scripts/run-kernel-pending-tests.sh, and are un-ignored when that lands. ondemand_exclusion_lock
# keeps this list in sync with that runner.
KERNEL_PENDING_EXCLUDE='not (package(logicaffeine-kernel) and test(/::(original_app_lam_converts_without_expanding_its_captured_source|the_entire_original_app_lam_imports_and_publishes_a_checked_definition)$/))'

# --- the qemu benchmark-corpus differentials, excluded like the silicon sims ------
# `riscv_aot_lock::riscv_matches_the_vm_on_the_benchmark_corpus` and `riscv_qemu_corpus_differential::qa_execution_differential`
# cross-check the WHOLE benchmark corpus against the tree-walker inside `qemu-riscv64`. MEASURED scale:
# 70 programs (35 dirs x main.lg + interp.lg) x 4 sizes = 280 assemble + emulate + VM cycles per run, each
# invoking `riscv64-linux-gnu-gcc` and then `qemu-riscv64`. Emulated execution is pure CPU with no I/O to
# overlap, so it does NOT speed up on a quiet box: it ran the full 6000s `qemu-corpus` budget to completion
# WITHOUT finishing, on an idle machine, holding 100% of one core the whole way (verified by /proc sampling
# — computing, not hung). Raising the budget again without measuring is how a 30-minute failure became a
# 100-minute one; the honest reading is that ~2h of emulation cannot fit a ~75-minute baseline.
#
# Same shape as the gate-level silicon sims: work that passes on its merits but does not belong in a routine
# run. It runs on purpose via ./scripts/run-qemu-corpus-tests.sh. NOT a coverage loss — `riscv_aot_lock`'s
# STATIC half (the `BENCHMARK_FLOOR` compile-coverage ratchet, which can only rise) is a separate assertion
# reached before the toolchain check, and every other test in both binaries still runs here.
QEMU_CORPUS_EXCLUDE='not (test(/^(riscv_matches_the_vm_on_the_benchmark_corpus|qa_execution_differential)$/))'

# The Arena mathlib cases. Five binaries include the shared case table (`tests/arena_corpus/cases.rs`):
# `arena_corpus`, `dag_dual_agreement`, `dag_dual_corpus`, `dag_sem_corpus` and `dag_sem_probe`. Its
# `case_mathlib` checks the whole Mathlib export from reference-kernels/: measured at ~85 minutes for one
# check, and `dag_sem_probe` checks it twice (budgets 0 and 1). Under `--run-ignored all` each reaches the
# 30-minute backstop and fails without having answered, so the baseline can never be green with them in
# it. They run on purpose, one at a time with an hours-long budget, via
# ./scripts/run-arena-mathlib-tests.sh. Every other case of all five binaries still runs here, including
# the heavy ones that fit.
ARENA_MATHLIB_EXCLUDE='not (binary(/^(arena_corpus|dag_dual_agreement|dag_dual_corpus|dag_sem_corpus|dag_sem_probe)$/) and test(=case_mathlib))'

# The decode engine's real-checkpoint accuracy bars: `d5005_engine_lock`'s `qwen3_*` and `llama3_*` tests walk a corpus of prompts
# through the engine model at every knob point it is held to, against llama.cpp's runs of the same checkpoint —
# about seven minutes a knob point on a loaded box, past the baseline's backstop. They are receipts, not locks: the
# arithmetic they rest on is held bit for bit by d5005_numerics_lock, d5005_rtl_lock and d5005_engine_lock's
# tiny-model bars, which run here every time. They run on purpose via ./scripts/run-model-bars-tests.sh.
MODEL_BARS_EXCLUDE='not (binary(d5005_engine_lock) and test(/^(qwen3|llama3)_/))'

# The suites behind the features this baseline leaves off — the kernel's `nbe` engine and `shadow` mode, the
# system's `io-uring` engine, wirebench's `capnproto`, `ffi-link-tests`, compile's `narrow-value` — need no
# exclusion: with the feature off they are not built at all, which is exactly why nothing here would notice
# them missing. They run on purpose via ./scripts/run-opt-in-feature-tests.sh, one invocation per feature, and
# feature_coverage_lock holds every feature-gated test to an invocation of that script or this one that turns
# its gate on. The RED suites handed out with the fusion book's tickets sit behind `handed-reds` until each lands,
# and run on purpose via ./scripts/run-handed-reds-tests.sh.

NEXTEST_IGNORED_ARGS=()
(( RUN_IGNORED )) && NEXTEST_IGNORED_ARGS+=(--run-ignored all)
# Every test of this pass runs through the full profile's wrapper (scripts/test/test-ledger.sh), which appends what it
# cost to the ledger beside this log: wall clock, CPU with every process it waited for, peak memory, how it ended. The
# LOAD section after the run reads the run's load from it test by test.
LEDGER_FILE="${LOG_FILE%.log}.ledger.tsv"
printf 'run_id\tbinary_id\ttest\tstart\tend\twall_s\tuser_s\tsys_s\tmaxrss_kb\toutcome\n' > "$LEDGER_FILE"
export LOGOS_TEST_LEDGER="$LEDGER_FILE"
cargo nextest run --workspace "${EXCLUDE_ARGS[@]}" --features verification,logicaffeine-tests/web-tests --profile full --cargo-profile test-opt "${NEXTEST_IGNORED_ARGS[@]}" -E "$RESEARCH_EXCLUDE and $OPT_AUDIT_EXCLUDE and $HEAVY_ONDEMAND_EXCLUDE and $KERNEL_PENDING_EXCLUDE and $QEMU_CORPUS_EXCLUDE and $ARENA_MATHLIB_EXCLUDE and $MODEL_BARS_EXCLUDE" \
  2>&1 | tee -a "$LOG_FILE"
NEXTEST_EXIT="${PIPESTATUS[0]}"
PASS1_EPOCH="$(date +%s)"
unset LOGOS_TEST_LEDGER
{
  echo
  echo "──────────────────────────────────────────────────────────────────────"
  echo " LOAD — what pass 1's tests cost ($LEDGER_FILE)"
  echo "──────────────────────────────────────────────────────────────────────"
  python3 "$SCRIPT_DIR/test/test-ledger-report.py" "$LEDGER_FILE" 2>&1
} | tee -a "$LOG_FILE"

# --- collect the overlapped doctests BEFORE pass 1b ---------------------------
# The doctest run builds `--features verification` into target/test-opt; pass 1b builds the
# SAME crate with `--features wasm-jit` into the SAME dir. Different feature sets mean
# different metadata hashes, and cargo is free to replace an rlib that the doctest run's
# rustdoc phase is still resolving — which it did: every doctest "failure" in the 22:25 run
# was one error, `extern location for logicaffeine_compile does not exist: …rlib`, and the
# same doctests passed alone. cargo's build LOCK serializes the two builds; it does not stop
# the second from evicting the first's artifacts. So the doctests are collected here, after
# the long pass they were overlapped with and before the pass whose feature set conflicts.
# The overlap that motivated the design (doctest compile vs the pass-1 RUN) is unchanged.
if [ -n "$DOCTEST_BG" ]; then
  wait "$DOCTEST_BG"
  DOCTEST_EXIT=$?
  DOCTEST_COLLECTED=1
fi

# --- pass 1b: WASM (isolated) -------------------------------------------------
# Run the WASM tests in their own pass scoped to logicaffeine-compile, so `wasmi`
# (pulled by the `wasm-jit` feature) never enters the rest of the workspace's
# dependency graph. Enabling `wasm-jit` workspace-wide changed feature unification
# and broke an unrelated async-closure lifetime in `interp_networking`; isolating
# it keeps these linked into the suite without perturbing other crates. Covers BOTH
# the browser WASM-JIT tier (`wasm_jit_differential` + the lib `vm::wasm::func`
# `wasm_jit_*` tests) AND the direct AOT backend (`wasm_aot_lock` — the
# WASM==VM==Treewalker parity lock with its exhaustive op catalog — and
# `wasm_aot_unit`). The AOT lock MUST run here so a backend regression or a newly
# unclassified VM op is caught by the standard suite, never only by a manual run.
# The filterset is NOT written here: it lives in `scripts/test/wasm-jit-filter.txt`, the
# ONE source of truth this pass and CI's step both read, and
# `crates/logicaffeine_tests/tests/wasm_gated_suite_lock.rs` derives the expected
# content from the gated test sources — so a new `wasm-jit`-gated binary that is not
# listed fails the DEFAULT pass instead of silently never running (eight binaries,
# 56+ tests, were dark for months under the old hand-list).
{
  echo
  echo "──────────────────────────────────────────────────────────────────────"
  echo " PASS 1b — WASM JIT tier + AOT backend + parity lock (logicaffeine-compile lib + logicaffeine-tests, --features wasm-jit)"
  echo "──────────────────────────────────────────────────────────────────────"
} | tee -a "$LOG_FILE"
# `--lib` + the `--test` flags derived from the filter file build ONLY this pass's binaries
# (an unrelated compile-crate test file mid-edit cannot take the frontend pass down).
# shellcheck disable=SC2046
cargo nextest run -p logicaffeine-compile -p logicaffeine-tests --features wasm-jit --cargo-profile test-opt \
  --lib $("$REPO_ROOT/scripts/test/wasm-jit-targets.sh") \
  -E "$(cat "$REPO_ROOT/scripts/test/wasm-jit-filter.txt")" \
  2>&1 | tee -a "$LOG_FILE"
WASMJIT_EXIT="${PIPESTATUS[0]}"

# --- pass 2: doctests (collect the overlapped run, or run now if --sequential) -
{
  echo
  echo "──────────────────────────────────────────────────────────────────────"
  echo " PASS 2/2 — cargo test --doc (doctests${DOCTEST_BG:+, overlapped with pass 1})"
  echo "──────────────────────────────────────────────────────────────────────"
} | tee -a "$LOG_FILE"

if [ -n "${DOCTEST_COLLECTED:-}" ]; then
  : # already collected above, before pass 1b — $DOCTEST_EXIT is set
elif [ -n "$DOCTEST_BG" ]; then
  wait "$DOCTEST_BG"
  DOCTEST_EXIT=$?
else
  run_doctests
  DOCTEST_EXIT=$?
fi
cat "$DOCTEST_LOG" 2>/dev/null | tee -a "$LOG_FILE"

# --- pass 3: THE SCOREBOARD MUST NOT BE ABLE TO LIE ---------------------------
# `lean_parity_matrix.rs` proves all 248 corpus cases are classified, that no two
# share a decider, and that per-tier TIER_FLOORS never falls — but it reads the verdict JSONs as
# TEXT and never runs a corpus test. So `"status": "Native"` was an unverified ASSERTION about a
# capability, and the ratchet sat on a number nobody had measured. When it was first checked by
# hand it was wrong in BOTH directions at once: cases recorded Native whose test FAILS (a
# capability we claim and do not have — the thing that loses a head-to-head against Lean), and
# cases recorded Gap whose test PASSES (free wins nobody counted, and floors left too low).
#
# This pass is that join, run for real: every corpus binary, nextest's machine-readable output,
# joined against crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/*.json BY TEST NAME, with SIGABRT/SIGSEGV/timeout treated
# as NOT PASSING (a `grep FAIL` misses an abort, and that already cost this campaign a day).
#
# It is cheap HERE specifically: pass 1 already built these exact binaries with this exact profile
# and feature set, so this is a run, not a build. And it is deliberately NOT a committed results
# file read by a Rust lock — such a file is stale the instant anyone edits kernel source, nothing
# in the repo can tell, and the lock goes green anyway. Results are produced in the same invocation
# that reports them, always. crates/logicaffeine_tests/tests/H_honesty_extra.rs makes DELETING this
# pass a red test, which is the only staleness left that can actually hurt.
# ⚠ SECTION BOUNDARY — CLOSED IN WAVE 3, BOTH HALVES. Kept because the measurement is the reason
# the two files are joined by a test, and the next pass added here inherits the hazard.
#
# scripts/lib/nextest-summary.sh slices this log into passes. Its terminator did not match
# " PASS 3", so the PASS 2/2 (doctest) section ran to EOF and swallowed everything this pass
# printed. Measured over hand-cut logs, rather than reasoned about:
#
#   * a `^error: could not compile …` here changed NOTHING. `build_failed_in` is consulted only for
#     PASS 1/2 and PASS 1b, which have a `<FAILED TO BUILD>` sentinel row; the doctest row has none
#     and never called it. (Wave 1 named this section's three consumed shapes but not this
#     function; it was safe for a different reason than the one given.)
#   * a `test result:` line here was counted AS DOCTESTS: a planted
#     `test result: FAILED. 0 passed; 99 failed;` produced ` doctests : 7 passed, 99 failed`.
#   * a `FAIL [` row here was listed under FAILED TESTS and then tripped the RECONCILIATION
#     warning, because the nextest Summary lines never counted it.
#
# The terminator now reads `^ PASS [123]|^ PASS 1b`, which closes the first two; the failure-row
# scrape is now SCOPED to the two nextest passes' own sections, which closes the third — it read
# the whole log while its reconciliation total came from two passes, so it warned against itself.
# Both are pinned by H_honesty_extra::a_later_pass_cannot_be_counted_as_doctests, against the real
# summarizer over a log in which this pass deliberately speaks the doctest counter's language.
#
# Renaming this pass to " PASS 2b" would have closed it from this side too, but the log would then
# disagree with this script's own header — and renaming the OTHER passes is not an option:
# `summary_after " PASS 1/2 "`, `" PASS 1b "` and `" PASS 2/2 "` are exact markers that
# gate_summary_lock pins against fixtures cut from real runs. A pass 4 must be added to the
# summarizer's terminator in the same change;
# H_honesty_extra::the_slicer_recognises_every_pass_marker_the_gate_prints joins the two files so
# that forgetting is red rather than silent.
#
# Independently of the boundary, this pass must not print per-test rows at all: its
# `--status-level none --final-status-level none --message-format libtest-json-plus` sends them to
# a JSON file. H_honesty_extra::the_verdict_gate_never_writes_test_rows_into_the_shared_log makes
# removing a flag a RED test. Two independent guards, because this one cost a day once.
{
  echo
  echo "──────────────────────────────────────────────────────────────────────"
  echo " PASS 3 — Lean-parity verdicts vs reality (scripts/lean/conformance-verify-verdicts.sh)"
  echo "──────────────────────────────────────────────────────────────────────"
} | tee -a "$LOG_FILE"
# `--features verification,logicaffeine-tests/web-tests` mirrors pass 1 EXACTLY. It is not that the corpus needs that
# feature — it needs none — but a different feature set is a different metadata hash, and this
# pass would rebuild from scratch what pass 1 just built. Standalone runs of the gate default to no
# features and are cheaper for it.
# The corpus it scores ships in the tree (crates/logicaffeine_tests/tests/fixtures/lean_conformance/), so the gate runs on every
# checkout; a gate that is not there fails the run.
VERDICT_GATE="$SCRIPT_DIR/lean/conformance-verify-verdicts.sh"
if [[ ! -x "$VERDICT_GATE" ]]; then
  echo " FATAL: the Lean-parity gate $VERDICT_GATE is not here" | tee -a "$LOG_FILE"
  VERDICTS_EXIT=1
else
  "$VERDICT_GATE" --features verification,logicaffeine-tests/web-tests 2>&1 | tee -a "$LOG_FILE"
  VERDICTS_EXIT="${PIPESTATUS[0]}"
fi

END_EPOCH="$(date +%s)"
ELAPSED=$(( END_EPOCH - START_EPOCH ))
P1=$(( PASS1_EPOCH - START_EPOCH ))
P2=$(( END_EPOCH - PASS1_EPOCH ))
H=$(( ELAPSED / 3600 )); M=$(( (ELAPSED % 3600) / 60 )); S=$(( ELAPSED % 60 ))

TRACKED_AFTER="${LOG_FILE%.log}.tracked-after"
bash "$TRACKED_WRITES" snapshot "$REPO_ROOT" "$TRACKED_AFTER"
TRACKED_CHANGED="$(bash "$TRACKED_WRITES" compare "$TRACKED_BEFORE" "$TRACKED_AFTER")"
TRACKED_EXIT=$?

EXIT_CODE=0
(( NEXTEST_EXIT != 0 )) && EXIT_CODE="$NEXTEST_EXIT"
(( WASMJIT_EXIT != 0 && EXIT_CODE == 0 )) && EXIT_CODE="$WASMJIT_EXIT"
(( DOCTEST_EXIT != 0 && EXIT_CODE == 0 )) && EXIT_CODE="$DOCTEST_EXIT"
# A scoreboard that lies is a RED baseline. Exit 2 from the gate means it could not COLLECT (the
# corpus did not build) — a different fact from "the scoreboard is lying", and reported as such.
(( VERDICTS_EXIT != 0 && EXIT_CODE == 0 )) && EXIT_CODE="$VERDICTS_EXIT"
(( TRACKED_EXIT == 1 && EXIT_CODE == 0 )) && EXIT_CODE=1

# --- summary -----------------------------------------------------------------
# One summarizer for every pass, shared with the gate_summary_lock: it reports the MAIN
# nextest pass and the wasm-jit pass BY NAME (never "the last Summary line in the file",
# which is the tiny wasm-jit pass), sums the doctests, and lists TIMEOUT rows beside FAIL
# rows so a terminate-after hang can never vanish from this block.
SUMMARIZER="$SCRIPT_DIR/lib/nextest-summary.sh"
SUMMARY_BODY="$(bash "$SUMMARIZER" "$LOG_FILE" 2>/dev/null)"

{
  echo
  echo "════════════════════════════════════════════════════════════════════"
  echo " SUMMARY (FAST)"
  echo "════════════════════════════════════════════════════════════════════"
  printf  " duration : %02dh %02dm %02ds (%ds total — nextest %ds, doctests %ds)\n" \
    "$H" "$M" "$S" "$ELAPSED" "$P1" "$P2"
  if (( ${#EXCLUDE_CRATES[@]} )); then
    echo " EXCLUDED : ${EXCLUDE_CRATES[*]} — NOT a full-workspace baseline (--exclude was passed)"
  fi
  printf '%s\n' "$SUMMARY_BODY"
  echo " ledger   : $LEDGER_FILE (what each test of pass 1 cost; the LOAD section above reads it)"
  if (( VERDICTS_EXIT == 0 )); then
    echo " verdicts : ✅ every Lean-parity verdict matches what its test actually did"
  elif (( VERDICTS_EXIT == 2 )); then
    echo " verdicts : ⚠️  COULD NOT COLLECT — the corpus did not build, so the scoreboard went unchecked"
  else
    echo " verdicts : ❌ MISMATCH — see PASS 3 (a Native case whose test is red, or a Gap that passes)"
  fi
  if (( TRACKED_EXIT == 0 )); then
    echo " tracked  : ✅ the run wrote no tracked file"
  elif (( TRACKED_EXIT == 2 )); then
    echo " tracked  : ⚠️  $TRACKED_CHANGED"
  else
    echo " tracked  : ❌ the run rewrote $(printf '%s\n' "$TRACKED_CHANGED" | grep -c .) tracked file(s) — a test must compare, never record:"
    printf '%s\n' "$TRACKED_CHANGED" | sed 's/^/            /'
  fi
  echo
  if (( EXIT_CODE == 0 )); then
    echo " RESULT   : ✅ GREEN — all tests passed"
  else
    echo " RESULT   : ❌ RED — exit code $EXIT_CODE (nextest: $NEXTEST_EXIT, doctests: $DOCTEST_EXIT, verdicts: $VERDICTS_EXIT, tracked writes: $TRACKED_EXIT)"
  fi
  echo " log      : $LOG_FILE"
  echo " parity   : ./scripts/compare-test-runs.sh logs/latest.log $LOG_FILE"
  echo "════════════════════════════════════════════════════════════════════"
  echo "EXIT: $EXIT_CODE"
} | tee -a "$LOG_FILE"

exit "$EXIT_CODE"
