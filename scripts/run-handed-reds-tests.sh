#!/usr/bin/env bash
#
# run-handed-reds-tests.sh — the RED tests handed out with their tickets, run on purpose.
#
# A ticket of benchmarks/FUSIONS_WE_NEED.md can go out with the RED tests its work must turn green, written and run
# RED before the ticket is posted (section 1.1). Each such suite opens with `#![cfg(feature = "handed-reds")]`, so
# the baseline never compiles it; the agent that takes the ticket deletes that one line as the work lands, and the
# suite joins the baseline. Until then the suite runs here, and only here, one suite at a time: the run is green while
# every suite is still red, and red the day one passes behind its gate — its work has landed, and the line must go.
#
# BINARIES is every suite that still carries the line, and nothing else: the script refuses to run when the two
# disagree, so a landed suite leaves this list as it joins the baseline. feature_coverage_lock reads the invocation
# below: a handed RED that no runner compiles would never run anywhere.
#
# Usage: ./scripts/run-handed-reds-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

BINARIES='deploy_reds_lock fabric_author_model_lock fabric_build_recipes_lock fabric_settle_probe_lock model_hf_config_reds_lock rdu_bringup_reds_lock samba_exam_bringup_reds_lock samba_exam_correctness_reds_lock samba_exam_moe_reds_lock spatial_disaggregation_reds_lock spatial_mixed_precision_reds_lock'

gated=$(grep -l '^#!\[cfg(feature = "handed-reds")\]' crates/logicaffeine_tests/tests/*.rs | xargs -n1 basename | sed 's/\.rs$//' | sort | tr '\n' ' ')
listed=$(echo $BINARIES | tr ' ' '\n' | sort | tr '\n' ' ')
if [ "$gated" != "$listed" ]; then
  echo "BINARIES is not the set of suites behind handed-reds:" >&2
  echo "  gated:  $gated" >&2
  echo "  listed: $listed" >&2
  exit 2
fi

STATUS=0
for bin in $BINARIES; do
  TEST_ARGS=(--test "$bin")
  cargo nextest run -p logicaffeine-tests --features handed-reds --no-fail-fast "${TEST_ARGS[@]}" "$@"
  if [ $? -eq 0 ]; then
    echo "LANDED: $bin passes behind handed-reds — delete its #![cfg] line so it joins the baseline, and its name here." >&2
    STATUS=1
  fi
done
echo
echo "EXIT: $STATUS"
exit "$STATUS"
