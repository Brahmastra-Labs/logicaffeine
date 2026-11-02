#!/usr/bin/env bash
#
# run-mutation-core.sh — drive cargo-mutants over the CORRECTNESS CORE.
# A surviving mutant is a proof that "if the suite passes you
# cannot break the code" is false at that point, so this run drives Tier-A survivors to
# zero. Scope + tiers live in .config/mutants.toml; this script drives execution.
#
# Two modes:
#   (default)   full Tier-A run, sharded across cores — nightly / on the bench box.
#   --in-diff   mutate ONLY the lines changed vs origin/main that fall in the core globs
#               — the fast per-PR gate.
#
# Per-mutant tests run under the tight `[profile.mutants]` nextest profile (2-min cap) so an
# infinite-loop mutant is killed fast. Uses a SCRATCH target dir so it never collides with
# the primary target/ (the bench box already carries hundreds of GB there).
#
# Usage: ./scripts/test/run-mutation-core.sh [--in-diff] [--shard K/N]
set -uo pipefail

MODE="full"; SHARD=""
while [ $# -gt 0 ]; do
  case "$1" in
    --in-diff) MODE="in-diff" ;;
    --shard) SHARD="$2"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"
if ! cargo mutants --version >/dev/null 2>&1; then
  echo "ERROR: cargo-mutants not found. Install with:" >&2
  echo "  cargo install cargo-mutants" >&2
  exit 1
fi

# --- refuse to overlap a test run (shared machine); detect by exe name, never -f ------
if pgrep -x cargo-nextest >/dev/null 2>&1; then
  echo "ERROR: a nextest run is in progress — refusing to start mutation (shared machine)." >&2
  pgrep -x -a cargo-nextest >&2
  exit 1
fi

# --- scratch target dir so we never fill/contend the primary target/ -----------------
SCRATCH_TARGET="${MUTANTS_TARGET_DIR:-$REPO_ROOT/target-mutants}"
export CARGO_TARGET_DIR="$SCRATCH_TARGET"

LOG_DIR="$REPO_ROOT/logs"; mkdir -p "$LOG_DIR"
mkdir -p "$REPO_ROOT/mutants"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
LOG_FILE="$LOG_DIR/mutation-$TIMESTAMP.log"
ln -sfn "$(basename "$LOG_FILE")" "$LOG_DIR/latest-mutation.log"

{
  echo "════════════════════════════════════════════════════════════════════"
  echo " logicaffeine — mutation testing (correctness core)"
  echo "════════════════════════════════════════════════════════════════════"
  echo " started : $(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo " mode    : $MODE ${SHARD:+(shard $SHARD)}"
  echo " mutants : $(cargo mutants --version 2>/dev/null)"
  echo " target  : $SCRATCH_TARGET"
  echo "════════════════════════════════════════════════════════════════════"
} | tee "$LOG_FILE"

# Common args: nextest test tool + our tight mutants profile + the verification feature the
# proof/kernel correctness tests need. .config/mutants.toml supplies examine/exclude globs + timeouts.
#
# --config: cargo-mutants 27.x reads `.cargo/mutants.toml` by DEFAULT, NOT
#   `.config/mutants.toml` — so without this the examine/exclude globs are ignored and it mutates the WHOLE
#   workspace (~77k mutants that never finish). Point it at the checked-in config explicitly.
# --in-place: mutate the working tree in place (reverted after each mutant) instead of copying the
#   whole tree per run — each copy is ~150 GB with build artifacts and nine of them filled the disk.
#   Safe here because the script already refuses to start while another test/mutation run is live.
# The test scope (.config/mutants/test-scope.txt): the Tier-A packages' unit tests plus the integration
#   suites that moved from them into logicaffeine-tests — never the whole test crate per mutant.
COMMON=(--config "$REPO_ROOT/.config/mutants.toml" --in-place --test-tool nextest -- --profile mutants --features verification -E "$(cat "$REPO_ROOT/.config/mutants/test-scope.txt")")
[ -n "$SHARD" ] && COMMON=(--shard "$SHARD" "${COMMON[@]}")

if [ "$MODE" = "in-diff" ]; then
  DIFF_FILE="$(mktemp)"
  git diff origin/main...HEAD > "$DIFF_FILE" 2>/dev/null || git diff > "$DIFF_FILE"
  echo " diff    : $(wc -l < "$DIFF_FILE") lines" | tee -a "$LOG_FILE"
  cargo mutants --in-diff "$DIFF_FILE" "${COMMON[@]}" 2>&1 | tee -a "$LOG_FILE"
  MUT_EXIT="${PIPESTATUS[0]}"
  rm -f "$DIFF_FILE"
else
  cargo mutants "${COMMON[@]}" 2>&1 | tee -a "$LOG_FILE"
  MUT_EXIT="${PIPESTATUS[0]}"
fi

# cargo-mutants writes mutants.out/ (outcomes.json + the caught/missed breakdown). Surface
# the survivor list for mutation_lock.rs / .config/mutants/known-survivors.txt to consume.
OUTCOMES="$REPO_ROOT/mutants.out/outcomes.json"
{
  echo "════════════════════════════════════════════════════════════════════"
  echo " RESULT"
  echo "════════════════════════════════════════════════════════════════════"
  if [ -f "$OUTCOMES" ]; then
    echo " outcomes: $OUTCOMES"
    echo " (feed survivors into .config/mutants/known-survivors.txt; Tier-A target = 0 survivors)"
  fi
  echo " exit    : $MUT_EXIT   log: $LOG_FILE"
  echo "════════════════════════════════════════════════════════════════════"
} | tee -a "$LOG_FILE"

exit "$MUT_EXIT"
