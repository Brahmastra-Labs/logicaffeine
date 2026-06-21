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
for arg in "$@"; do
  case "$arg" in
    --no-ignored) RUN_IGNORED=0 ;;
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
# Never overlap with another test run (shared target dir, shared machine).
if pgrep -f 'cargo(-nextest)? (test|nextest)' >/dev/null 2>&1; then
  echo "ERROR: another cargo test / nextest run appears to be in progress:" >&2
  pgrep -af 'cargo(-nextest)? (test|nextest)' >&2
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
    echo " command : cargo nextest run --workspace --features verification --profile full --run-ignored all"
    echo "         + cargo test --workspace --features verification --doc --no-fail-fast -- --include-ignored"
  else
    echo " command : cargo nextest run --workspace --features verification --profile full   [--no-ignored]"
    echo "         + cargo test --workspace --features verification --doc --no-fail-fast"
  fi
  echo "════════════════════════════════════════════════════════════════════"
  echo
} | tee "$LOG_FILE"

START_EPOCH="$(date +%s)"

# --- pass 1: unit + integration tests (nextest) -------------------------------
{
  echo "──────────────────────────────────────────────────────────────────────"
  echo " PASS 1/2 — cargo nextest run (unit + integration, ignored included)"
  echo "──────────────────────────────────────────────────────────────────────"
} | tee -a "$LOG_FILE"

NEXTEST_IGNORED_ARGS=()
(( RUN_IGNORED )) && NEXTEST_IGNORED_ARGS+=(--run-ignored all)
cargo nextest run --workspace --features verification --profile full "${NEXTEST_IGNORED_ARGS[@]}" \
  2>&1 | tee -a "$LOG_FILE"
NEXTEST_EXIT="${PIPESTATUS[0]}"
PASS1_EPOCH="$(date +%s)"

# --- pass 2: doctests ----------------------------------------------------------
{
  echo
  echo "──────────────────────────────────────────────────────────────────────"
  echo " PASS 2/2 — cargo test --doc (doctests)"
  echo "──────────────────────────────────────────────────────────────────────"
} | tee -a "$LOG_FILE"

DOCTEST_IGNORED_ARGS=()
(( RUN_IGNORED )) && DOCTEST_IGNORED_ARGS+=(--include-ignored)
cargo test --workspace --features verification --doc --no-fail-fast -- "${DOCTEST_IGNORED_ARGS[@]}" \
  2>&1 | tee -a "$LOG_FILE"
DOCTEST_EXIT="${PIPESTATUS[0]}"

END_EPOCH="$(date +%s)"
ELAPSED=$(( END_EPOCH - START_EPOCH ))
P1=$(( PASS1_EPOCH - START_EPOCH ))
P2=$(( END_EPOCH - PASS1_EPOCH ))
H=$(( ELAPSED / 3600 )); M=$(( (ELAPSED % 3600) / 60 )); S=$(( ELAPSED % 60 ))

EXIT_CODE=0
(( NEXTEST_EXIT != 0 )) && EXIT_CODE="$NEXTEST_EXIT"
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
