#!/usr/bin/env bash
#
# run-all-tests-fast.sh — the full baseline test run, fast.
#
# Covers the exact same surface as run-all-tests.sh (every crate, the Z3-backed
# verification tests, the #[ignore]d tests, unit tests, integration tests, and
# doctests) but schedules individual tests across all cores with cargo-nextest
# instead of running ~420 test binaries one at a time.
#
# Two passes:
#   1. cargo nextest run  — every unit + integration test (nextest cannot run
#      doctests, hence pass 2)
#   2. cargo test --doc   — every doctest, mirroring the baseline's inline run
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
OVERLAP_DOCTESTS=1
for arg in "$@"; do
  case "$arg" in
    --no-ignored) RUN_IGNORED=0 ;;
    --sequential) OVERLAP_DOCTESTS=0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# --- environment -------------------------------------------------------------
# Z3 header for the `verification` feature (Linux apt path; override if needed).
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
# Make cargo available in non-login/background shells.
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"
if ! cargo nextest --version >/dev/null 2>&1; then
  echo "ERROR: cargo-nextest not found. Install with:" >&2
  echo '  curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C "$HOME/.cargo/bin"' >&2
  exit 1
fi

# --- one suite at a time -----------------------------------------------------
# Never overlap with another test run (shared target dir, shared machine). Detect the
# actual cargo / cargo-nextest PROCESS by executable name (`pgrep -x`), NOT by a
# command-line substring. The old `pgrep -f 'cargo(-nextest)? (test|nextest)'` also
# matched any innocent process whose *command line mentioned* those words — a monitoring
# `pgrep`, a launcher, this very check — so the script refused to start against its own
# watcher shells and wedged. `cargo-nextest` stays alive for a whole nextest run (build +
# run), so detecting it covers the common case; a plain `cargo test` is caught by reading
# the cargo process's own argv. Our own PID and shell are never `cargo`, so no self-match.
overlapping=""
if pgrep -x cargo-nextest >/dev/null 2>&1; then
  overlapping="$(pgrep -x -a cargo-nextest 2>/dev/null)"
else
  for _pid in $(pgrep -x cargo 2>/dev/null); do
    [ "$_pid" = "$$" ] && continue
    _args="$(tr '\0' ' ' < "/proc/$_pid/cmdline" 2>/dev/null)"
    case " $_args " in
      *" test "*|*" nextest "*) overlapping="$_args"; break ;;
    esac
  done
fi
if [ -n "$overlapping" ]; then
  echo "ERROR: another cargo test / nextest run appears to be in progress:" >&2
  echo "$overlapping" >&2
  echo "Refusing to start a second suite. Wait for it to finish." >&2
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
    echo " command : cargo nextest run --workspace --features verification --profile full --cargo-profile test-opt --run-ignored all"
    echo "         + cargo nextest run -p logicaffeine-compile --features wasm-jit --cargo-profile test-opt -E 'binary(wasm_jit_differential) + binary(wasm_aot_lock) + binary(wasm_aot_unit) + test(wasm_jit)'"
    echo "         + cargo test --workspace --features verification --profile test-opt --doc --no-fail-fast -- --include-ignored"
  else
    echo " command : cargo nextest run --workspace --features verification --profile full --cargo-profile test-opt   [--no-ignored]"
    echo "         + cargo nextest run -p logicaffeine-compile --features wasm-jit --cargo-profile test-opt -E 'binary(wasm_jit_differential) + binary(wasm_aot_lock) + binary(wasm_aot_unit) + test(wasm_jit)'"
    echo "         + cargo test --workspace --features verification --profile test-opt --doc --no-fail-fast"
  fi
  echo "════════════════════════════════════════════════════════════════════"
  echo
} | tee "$LOG_FILE"

START_EPOCH="$(date +%s)"

# --- pass 2 (doctests): launched FIRST, in the background, to OVERLAP pass 1 ----
# Doctests are pure compile-bound work nextest cannot schedule; running them
# serially after nextest wastes the cores the nextest tail leaves idle. On this
# box (dedicated to test runs) we start them up front and let them run concurrently
# with passes 1/1b — cargo's build lock serializes the BUILDS safely, then the
# doctest snippet-compile overlaps the nextest RUN, hiding ~the doctest time.
# `--sequential` restores the strictly-ordered passes.
DOCTEST_LOG="$LOG_DIR/doctest-$TIMESTAMP.log"
DOCTEST_IGNORED_ARGS=()
(( RUN_IGNORED )) && DOCTEST_IGNORED_ARGS+=(--include-ignored)
run_doctests() {
  cargo test --workspace --features verification --profile test-opt --doc --no-fail-fast -- "${DOCTEST_IGNORED_ARGS[@]}" \
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

# The SAT/coNP "P-vs-NP" research campaign in logicaffeine-proof is EXPLORATORY, not a
# correctness lock: its probes run minutes-to-HOURS (exhaustive orbit/census/ladder
# enumeration at growing n) and hung the baseline for 4h+ under --run-ignored. They are
# excluded from the routine run and live in scripts/run-research-tests.sh instead — keep
# the two lists in sync. The profile.full 30-min terminate-after backstops any straggler
# not named here, so a newly-added probe can never silently hang the baseline again.
RESEARCH_EXCLUDE='not binary(/^(cofactor_mirror|cofactor_climb|cofactor_lens|cofactor_family_counting|rigid_residue_census|sat_census|frege_generator_ladder|martin_lof_omega_kernel|reflection_mirror|orbit_stability_kernel|ultimate_symmetry_finder|hardness_retreat|hardness_witness_ladder|uniform_transfer_theorem|no_randomness_at_infinity|no_finite_randomness_infinity|family_emergence|pvnp_gunsight|ef_class_probe)$/)'

NEXTEST_IGNORED_ARGS=()
(( RUN_IGNORED )) && NEXTEST_IGNORED_ARGS+=(--run-ignored all)
cargo nextest run --workspace --features verification --profile full --cargo-profile test-opt "${NEXTEST_IGNORED_ARGS[@]}" -E "$RESEARCH_EXCLUDE" \
  2>&1 | tee -a "$LOG_FILE"
NEXTEST_EXIT="${PIPESTATUS[0]}"
PASS1_EPOCH="$(date +%s)"

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
{
  echo
  echo "──────────────────────────────────────────────────────────────────────"
  echo " PASS 1b — WASM JIT tier + AOT backend + parity lock (logicaffeine-compile, --features wasm-jit)"
  echo "──────────────────────────────────────────────────────────────────────"
} | tee -a "$LOG_FILE"
cargo nextest run -p logicaffeine-compile --features wasm-jit --cargo-profile test-opt \
  -E 'binary(wasm_jit_differential) + binary(wasm_aot_lock) + binary(wasm_aot_unit) + test(wasm_jit)' \
  2>&1 | tee -a "$LOG_FILE"
WASMJIT_EXIT="${PIPESTATUS[0]}"

# --- pass 2: doctests (collect the overlapped run, or run now if --sequential) -
{
  echo
  echo "──────────────────────────────────────────────────────────────────────"
  echo " PASS 2/2 — cargo test --doc (doctests${DOCTEST_BG:+, overlapped with pass 1})"
  echo "──────────────────────────────────────────────────────────────────────"
} | tee -a "$LOG_FILE"

if [ -n "$DOCTEST_BG" ]; then
  wait "$DOCTEST_BG"
  DOCTEST_EXIT=$?
else
  run_doctests
  DOCTEST_EXIT=$?
fi
cat "$DOCTEST_LOG" 2>/dev/null | tee -a "$LOG_FILE"

END_EPOCH="$(date +%s)"
ELAPSED=$(( END_EPOCH - START_EPOCH ))
P1=$(( PASS1_EPOCH - START_EPOCH ))
P2=$(( END_EPOCH - PASS1_EPOCH ))
H=$(( ELAPSED / 3600 )); M=$(( (ELAPSED % 3600) / 60 )); S=$(( ELAPSED % 60 ))

EXIT_CODE=0
(( NEXTEST_EXIT != 0 )) && EXIT_CODE="$NEXTEST_EXIT"
(( WASMJIT_EXIT != 0 && EXIT_CODE == 0 )) && EXIT_CODE="$WASMJIT_EXIT"
(( DOCTEST_EXIT != 0 && EXIT_CODE == 0 )) && EXIT_CODE="$DOCTEST_EXIT"

# --- summary -----------------------------------------------------------------
# nextest totals from its final "Summary [ ...s] N tests run: ..." line.
NEXTEST_SUMMARY="$(grep -E '^[[:space:]]*Summary \[' "$LOG_FILE" | tail -1 | sed 's/^[[:space:]]*//')"
# Doctest totals from the cargo-format "test result:" lines, counted only after
# the PASS 2/2 marker — nextest's captured output of failed tests can contain
# nested "test result:" lines that must not be counted.
read -r DOC_PASSED DOC_FAILED DOC_IGNORED < <(
  awk '
    /^ PASS 2\/2 / { in_doc = 1 }
    in_doc && /test result:/ {
      for (i = 1; i <= NF; i++) {
        if ($i == "passed;")  passed  += $(i-1)
        if ($i == "failed;")  failed  += $(i-1)
        if ($i == "ignored;") ignored += $(i-1)
      }
    }
    END { printf "%d %d %d", passed, failed, ignored }
  ' "$LOG_FILE"
)

{
  echo
  echo "════════════════════════════════════════════════════════════════════"
  echo " SUMMARY (FAST)"
  echo "════════════════════════════════════════════════════════════════════"
  printf  " duration : %02dh %02dm %02ds (%ds total — nextest %ds, doctests %ds)\n" \
    "$H" "$M" "$S" "$ELAPSED" "$P1" "$P2"
  echo " nextest  : ${NEXTEST_SUMMARY:-<no summary line found>}"
  printf  " doctests : %s passed, %s failed, %s ignored\n" "$DOC_PASSED" "$DOC_FAILED" "$DOC_IGNORED"
  if (( NEXTEST_EXIT != 0 )); then
    echo
    echo " FAILED TESTS (nextest):"
    grep -E '^[[:space:]]*FAIL ' "$LOG_FILE" | sed 's/^[[:space:]]*/   /' | sort -u | head -100
  fi
  if (( DOC_FAILED > 0 )); then
    echo
    echo " FAILED DOCTESTS:"
    awk '/^ PASS 2\/2 /{in_doc=1} in_doc && /^test .* \.\.\. FAILED$/' "$LOG_FILE" \
      | sed 's/^/   /' | sort -u | head -100
  fi
  echo
  if (( EXIT_CODE == 0 )); then
    echo " RESULT   : ✅ GREEN — all tests passed"
  else
    echo " RESULT   : ❌ RED — exit code $EXIT_CODE (nextest: $NEXTEST_EXIT, doctests: $DOCTEST_EXIT)"
  fi
  echo " log      : $LOG_FILE"
  echo " parity   : ./scripts/compare-test-runs.sh logs/latest.log $LOG_FILE"
  echo "════════════════════════════════════════════════════════════════════"
  echo "EXIT: $EXIT_CODE"
} | tee -a "$LOG_FILE"

exit "$EXIT_CODE"
