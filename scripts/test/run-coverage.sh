#!/usr/bin/env bash
#
# run-coverage.sh — measure workspace code coverage with cargo-llvm-cov.
# REPORT-ONLY: it prints per-crate region/line coverage and
# writes a machine-readable summary; it does NOT gate. scripts/test/check-coverage-floors.sh turns
# the numbers into a monotone ratchet.
#
# Covers the same surface as run-all-tests-fast.sh, in three accumulating passes so the
# whole suite lands in ONE merged figure:
#   1. cargo llvm-cov --no-report nextest   — unit + integration (research campaign excluded)
#   2. cargo llvm-cov --no-report nextest   — the isolated wasm-jit tier + AOT parity lock
#   3. cargo llvm-cov --no-report --doc      — doctests
# then `cargo llvm-cov report` merges the three profraw sets.
#
# It also runs an ISOLATION pass: language/compile measured with ONLY their own unit
# tests, so the "weakly-tested-in-isolation" gap is a distinct, visible number instead of
# being masked by the integration-tier logicaffeine_tests sink.
#
# Branch + MC/DC (the trusted-core, SQLite-MC/DC analogue) are nightly-experimental; pass
# --mcdc to add them on logicaffeine-kernel + logicaffeine-proof via the nightly toolchain.
#
# Usage: ./scripts/test/run-coverage.sh [--mcdc]
set -uo pipefail

WANT_MCDC=0
for arg in "$@"; do
  case "$arg" in
    --mcdc) WANT_MCDC=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

# --- environment -------------------------------------------------------------
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"
if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "ERROR: cargo-llvm-cov not found. Install with:" >&2
  echo "  cargo install cargo-llvm-cov" >&2
  exit 1
fi

# --- one suite at a time (mirror run-all-tests-fast.sh; NEVER overlap a test run) -------
# Detect a real cargo / cargo-nextest PROCESS by executable name (`pgrep -x`), never by a
# command-line substring (which self-matches this script's own argv).
overlapping=""
if pgrep -x cargo-nextest >/dev/null 2>&1; then
  overlapping="$(pgrep -x -a cargo-nextest 2>/dev/null)"
else
  for _pid in $(pgrep -x cargo 2>/dev/null); do
    [ "$_pid" = "$$" ] && continue
    _args="$(tr '\0' ' ' < "/proc/$_pid/cmdline" 2>/dev/null)"
    case " $_args " in
      *" test "*|*" nextest "*|*" llvm-cov "*) overlapping="$_args"; break ;;
    esac
  done
fi
if [ -n "$overlapping" ]; then
  echo "ERROR: a cargo test / nextest / llvm-cov run appears to be in progress:" >&2
  echo "$overlapping" >&2
  echo "Refusing to start (shared target dir + machine). Wait for it to finish." >&2
  exit 1
fi

# --- config kept in sync with run-all-tests-fast.sh --------------------------
# The SAT/coNP research campaign is exploratory, not a correctness lock, and would drag
# the certifier's real number down or hide gaps — exclude it from BOTH the run and the
# coverage denominator. Keep this list identical to RESEARCH_EXCLUDE in run-all-tests-fast.sh.
RESEARCH_EXCLUDE='not binary(/^(cofactor_mirror|cofactor_family_counting|rigid_residue_census|sat_census|frege_generator_ladder|martin_lof_omega_kernel|reflection_mirror|orbit_stability_kernel|uniform_transfer_theorem|no_randomness_at_infinity|no_finite_randomness_infinity|family_emergence|ef_class_probe)$/)'
RESEARCH_FILE_RE='cofactor|census|ladder|pvnp|ef_class|reflection_mirror|orbit_stability|ultimate_symmetry|hardness_|uniform_transfer|randomness_at_infinity|family_emergence'

# --- log ---------------------------------------------------------------------
LOG_DIR="$REPO_ROOT/logs"; mkdir -p "$LOG_DIR"
COV_DIR="$REPO_ROOT/.config/coverage"; mkdir -p "$COV_DIR"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
LOG_FILE="$LOG_DIR/coverage-$TIMESTAMP.log"
ln -sfn "$(basename "$LOG_FILE")" "$LOG_DIR/latest-coverage.log"

{
  echo "════════════════════════════════════════════════════════════════════"
  echo " logicaffeine — code coverage (cargo-llvm-cov, report-only)"
  echo "════════════════════════════════════════════════════════════════════"
  echo " started : $(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo " llvm-cov: $(cargo llvm-cov --version 2>/dev/null)"
  echo "════════════════════════════════════════════════════════════════════"
} | tee "$LOG_FILE"

START_EPOCH="$(date +%s)"

# --- clean any stale profraw so the three passes accumulate cleanly ----------
cargo llvm-cov clean --workspace 2>&1 | tee -a "$LOG_FILE"

# --- pass 1: unit + integration (research campaign excluded) -----------------
echo "── PASS 1/3 — unit + integration (nextest) ──" | tee -a "$LOG_FILE"
cargo llvm-cov --no-report nextest \
  --workspace --features verification --profile full \
  -E "$RESEARCH_EXCLUDE" 2>&1 | tee -a "$LOG_FILE"

# --- pass 2: isolated wasm-jit tier + AOT parity lock ------------------------
echo "── PASS 2/3 — wasm-jit tier + AOT backend (isolated) ──" | tee -a "$LOG_FILE"
# shellcheck disable=SC2046
cargo llvm-cov --no-report nextest -p logicaffeine-compile -p logicaffeine-tests --features wasm-jit \
  --lib $("$REPO_ROOT/scripts/test/wasm-jit-targets.sh") \
  -E "$(cat "$REPO_ROOT/scripts/test/wasm-jit-filter.txt")" \
  2>&1 | tee -a "$LOG_FILE"

# --- pass 3: doctests --------------------------------------------------------
echo "── PASS 3/3 — doctests ──" | tee -a "$LOG_FILE"
cargo llvm-cov --no-report --doc \
  --workspace --features verification 2>&1 | tee -a "$LOG_FILE"

# --- merged report -----------------------------------------------------------
# Human-readable per-crate/per-file table (research campaign filtered from the denominator).
echo "── MERGED COVERAGE (all three passes) ──" | tee -a "$LOG_FILE"
cargo llvm-cov report \
  --ignore-filename-regex "$RESEARCH_FILE_RE" 2>&1 | tee -a "$LOG_FILE"

# Machine-readable summary for the ratchet (scripts/test/check-coverage-floors.sh reads crate-summary.json).
cargo llvm-cov report --json \
  --ignore-filename-regex "$RESEARCH_FILE_RE" \
  --output-path "$COV_DIR/summary.json" 2>&1 | tee -a "$LOG_FILE"
echo " wrote: $COV_DIR/summary.json" | tee -a "$LOG_FILE"

# --- isolation pass: language/compile with ONLY their own unit tests ---------
# Surfaces the "under-tested in isolation" gap as a DISTINCT number (a separate profraw
# set + report), not masked by the integration-tier logicaffeine_tests coverage.
echo "── ISOLATION — language/compile measured by their OWN unit tests only ──" | tee -a "$LOG_FILE"
for crate in logicaffeine-language logicaffeine-compile; do
  echo "  · $crate (isolation)" | tee -a "$LOG_FILE"
  cargo llvm-cov --json --summary-only \
    -p "$crate" nextest -E "package($crate)" \
    --output-path "$COV_DIR/isolation-$crate.json" 2>&1 | tee -a "$LOG_FILE"
done

# --- optional: branch + MC/DC on the trusted core (nightly) ------------------
if (( WANT_MCDC )); then
  echo "── MC/DC + branch on the trusted core (nightly) ──" | tee -a "$LOG_FILE"
  if rustup toolchain list 2>/dev/null | grep -q nightly; then
    cargo +nightly llvm-cov --mcdc --branch \
      -p logicaffeine-kernel -p logicaffeine-proof \
      --ignore-filename-regex "$RESEARCH_FILE_RE" report 2>&1 | tee -a "$LOG_FILE"
  else
    echo "  (nightly toolchain not installed — skipping MC/DC)" | tee -a "$LOG_FILE"
  fi
fi

END_EPOCH="$(date +%s)"; ELAPSED=$(( END_EPOCH - START_EPOCH ))
{
  echo "════════════════════════════════════════════════════════════════════"
  printf " duration : %dm %ds\n" $(( ELAPSED / 60 )) $(( ELAPSED % 60 ))
  echo " summary  : $COV_DIR/summary.json"
  echo " log      : $LOG_FILE"
  echo "════════════════════════════════════════════════════════════════════"
} | tee -a "$LOG_FILE"
