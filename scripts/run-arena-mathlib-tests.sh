#!/usr/bin/env bash
#
# run-arena-mathlib-tests.sh — the Arena mathlib cases, run on purpose.
#
# `arena_corpus`, `dag_dual_agreement`, `dag_dual_corpus`, `dag_sem_corpus` and `dag_sem_probe` share one case table
# (crates/logicaffeine_tests/tests/arena_corpus/cases.rs). Its `case_mathlib` checks the whole Mathlib
# export under reference-kernels/: measured at ~85 minutes for one check, and `dag_sem_probe` checks it
# twice (budgets 0 and 1). That does not fit the routine baseline's 30-minute per-test backstop, so the
# baseline excludes them (`ARENA_MATHLIB_EXCLUDE` in run-all-tests-fast.sh) and they run here, one at a
# time (the `arena-mathlib` nextest group), each with a hang backstop of hours.
#
# The test set MUST stay in sync with ARENA_MATHLIB_EXCLUDE in run-all-tests-fast.sh
# (enforced by crates/logicaffeine_tests/tests/ondemand_exclusion_lock.rs).
#
# Needs the large inputs under reference-kernels/arena-inputs-<round>/, at the paths the vendored manifest
# (crates/logicaffeine_tests/tests/fixtures/arena_corpus/manifest.json) records (scripts/lean/arena-crown/build-arena-inputs.py).
#
# Usage: ./scripts/run-arena-mathlib-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/test/in-memory-scope.sh" 100G "$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")" "$@"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

# The runner requires the inputs it exists to check: with reference-kernels named in LOGOS_REQUIRE_TOOLS, a
# missing tree fails every case loudly instead of passing it as a skip (arena_corpus_lock holds this line).
export LOGOS_REQUIRE_TOOLS="${LOGOS_REQUIRE_TOOLS:+$LOGOS_REQUIRE_TOOLS,}reference-kernels"

ARENA_MATHLIB_ONLY='binary(/^(arena_corpus|dag_dual_agreement|dag_dual_corpus|dag_sem_corpus|dag_sem_probe)$/) and test(=case_mathlib)'
# The test binaries that selector can reach, each passed to cargo as `--test`, so the run builds only
# them: `-p logicaffeine-tests` alone builds every one of the crate's ~2,300 test binaries whatever `-E`
# selects. ondemand_exclusion_lock holds this list to the selector.
BINARIES='arena_corpus dag_dual_agreement dag_dual_corpus dag_sem_corpus dag_sem_probe'
TEST_ARGS=()
for bin in $BINARIES; do TEST_ARGS+=(--test "$bin"); done
# `dag_dual_agreement` is compiled only with the kernel's `nbe` feature (it decides every definitional
# equality with both conversion engines), so the run turns it on; for the other four binaries it changes
# nothing, since the NbE engine answers only when a checker asks for it. feature_coverage_lock refuses a
# runner that names a binary its own features compile to nothing.

echo "Running the Arena mathlib cases (five whole-Mathlib checks, one at a time)."
echo "This is SLOW: expect several hours, and give it a quiet box."
exec cargo nextest run -p logicaffeine-tests --features nbe --profile full --cargo-profile test-opt --run-ignored all \
  "${TEST_ARGS[@]}" -E "$ARENA_MATHLIB_ONLY" "$@"
