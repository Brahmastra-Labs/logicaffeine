#!/usr/bin/env bash
#
# run-opt-audit-tests.sh — the exhaustive optimization-graph completeness audit, run on purpose.
#
# `phase_opt_completeness::exhaustive_edge_completeness_audit` certifies the optimization graph by a 2^k
# opt-disable sweep over the benchmark programs (~10 min single-threaded, ~2.75 GB RSS). It passes on the
# merits, but under concurrent baseline load it exceeds the 30-min backstop, so scripts/run-all-tests-fast.sh
# excludes it (OPT_AUDIT_EXCLUDE) and it runs here, alone. Its fast companion `phase_opt_graph` locks the baked
# graph every run; re-run this audit after adding or changing an optimization. The selector MUST stay in sync
# with OPT_AUDIT_EXCLUDE (ondemand_exclusion_lock enforces it).
#
# Usage: ./scripts/run-opt-audit-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/test/in-memory-scope.sh" 32G "$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")" "$@"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

OPT_AUDIT_ONLY='test(=exhaustive_edge_completeness_audit)'
BINARIES='phase_opt_completeness'
TEST_ARGS=()
for bin in $BINARIES; do TEST_ARGS+=(--test "$bin"); done

echo "Running the optimization-graph completeness audit (--run-ignored all), alone, in a 32G memory scope."
exec cargo nextest run -p logicaffeine-tests --profile full --cargo-profile test-opt --run-ignored all \
  --test-threads 1 "${TEST_ARGS[@]}" -E "$OPT_AUDIT_ONLY" "$@"
