#!/usr/bin/env bash
#
# run-all-tests.sh — the full baseline test run.
#
# Runs the literally-everything suite: every crate (--workspace), the Z3-backed
# test files (--features verification), the e2e surface, and the #[ignore]d
# tests (--include-ignored). Inline unit tests and doctests come along too.
#
# Output is streamed to logs/test-run-<timestamp>.log (kept on disk, gitignored)
# and a logs/latest.log symlink always points at the most recent run. A summary
# — wall-clock time, totals, and any failures — is printed at the end and
# appended to the log.
#
# Usage: ./scripts/run-all-tests.sh
#
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# --- environment -------------------------------------------------------------
# Z3 header for the `verification` feature (Linux apt path; override if needed).
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
# Make cargo available in non-login/background shells.
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

# --- log file ----------------------------------------------------------------
LOG_DIR="$REPO_ROOT/logs"
mkdir -p "$LOG_DIR"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
LOG_FILE="$LOG_DIR/test-run-$TIMESTAMP.log"
ln -sfn "$(basename "$LOG_FILE")" "$LOG_DIR/latest.log"

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
  echo " logicaffeine — full test baseline"
  echo "════════════════════════════════════════════════════════════════════"
  echo " started : $(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo " host    : $(hostname)"
  echo " branch  : $BRANCH @ $COMMIT"
  echo " rustc   : $(rustc --version 2>/dev/null)"
  echo " z3      : $(z3 --version 2>/dev/null)"
  echo " command : cargo test --workspace --features verification --no-fail-fast -- --include-ignored"
  echo "════════════════════════════════════════════════════════════════════"
  echo
} | tee "$LOG_FILE"

# --- run ---------------------------------------------------------------------
START_EPOCH="$(date +%s)"

cargo test --workspace --features verification --no-fail-fast -- --include-ignored \
  2>&1 | tee -a "$LOG_FILE"
EXIT_CODE="${PIPESTATUS[0]}"

END_EPOCH="$(date +%s)"
ELAPSED=$(( END_EPOCH - START_EPOCH ))
H=$(( ELAPSED / 3600 )); M=$(( (ELAPSED % 3600) / 60 )); S=$(( ELAPSED % 60 ))

# --- summary -----------------------------------------------------------------
# Aggregate every "test result:" line (one per test binary + doctests).
read -r PASSED FAILED IGNORED MEASURED FILTERED < <(
  awk '
    /test result:/ {
      for (i = 1; i <= NF; i++) {
        if ($i == "passed;")   passed   += $(i-1)
        if ($i == "failed;")   failed   += $(i-1)
        if ($i == "ignored;")  ignored  += $(i-1)
        if ($i == "measured;") measured += $(i-1)
        if ($i == "filtered")  filtered += $(i-1)
      }
    }
    END { printf "%d %d %d %d %d", passed, failed, ignored, measured, filtered }
  ' "$LOG_FILE"
)
TOTAL=$(( PASSED + FAILED ))

{
  echo
  echo "════════════════════════════════════════════════════════════════════"
  echo " SUMMARY"
  echo "════════════════════════════════════════════════════════════════════"
  printf  " duration : %02dh %02dm %02ds (%ds)\n" "$H" "$M" "$S" "$ELAPSED"
  printf  " ran      : %s tests\n" "$TOTAL"
  printf  " passed   : %s\n" "$PASSED"
  printf  " failed   : %s\n" "$FAILED"
  printf  " ignored  : %s (now run via --include-ignored unless 0)\n" "$IGNORED"
  printf  " measured : %s (benches)\n" "$MEASURED"
  if (( FAILED > 0 )); then
    echo
    echo " FAILED TESTS:"
    grep -E '^\s*test .* \.\.\. FAILED$|^\s+[A-Za-z0-9_:]+::.*$' "$LOG_FILE" \
      | grep -iE 'FAILED|^\s+[A-Za-z0-9_]+::' | sort -u | sed 's/^/   /' | head -100
  fi
  echo
  if (( EXIT_CODE == 0 )); then
    echo " RESULT   : ✅ GREEN — all tests passed"
  else
    echo " RESULT   : ❌ RED — exit code $EXIT_CODE"
  fi
  echo " log      : $LOG_FILE"
  echo "════════════════════════════════════════════════════════════════════"
  echo "EXIT: $EXIT_CODE"
} | tee -a "$LOG_FILE"

exit "$EXIT_CODE"
