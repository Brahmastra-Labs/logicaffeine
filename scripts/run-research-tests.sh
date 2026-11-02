#!/usr/bin/env bash
#
# run-research-tests.sh — the SAT/coNP "P-vs-NP" research campaign, run on purpose.
#
# These campaign probes exercise only product code (the research instruments they grew up
# beside live in the private research package, which runs its own suite). They are EXPLORATORY,
# not correctness locks: exhaustive orbit/ladder enumeration whose runtime grows with n, from
# minutes to HOURS. They are deliberately EXCLUDED from scripts/run-all-tests-fast.sh (the routine
# baseline) so a normal run stays semi-quick and never hangs — but they are not deleted,
# and this script runs exactly them, with --run-ignored, when you actually want the
# research numbers. Expect this to run for a long time.
#
# The binary list MUST stay in sync with RESEARCH_EXCLUDE in run-all-tests-fast.sh.
#
# Usage: ./scripts/run-research-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/test/in-memory-scope.sh" 100G "$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")" "$@"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

# The research campaign binaries — identical set to RESEARCH_EXCLUDE in run-all-tests-fast.sh.
RESEARCH_ONLY='binary(/^(cofactor_mirror|cofactor_family_counting|rigid_residue_census|sat_census|frege_generator_ladder|martin_lof_omega_kernel|reflection_mirror|orbit_stability_kernel|uniform_transfer_theorem|no_randomness_at_infinity|no_finite_randomness_infinity|family_emergence|ef_class_probe)$/)'
# The test binaries that selector can reach, each passed to cargo as `--test`, so the run builds only
# them: `-p logicaffeine-tests` alone builds every one of the crate's ~2,300 test binaries whatever `-E`
# selects. ondemand_exclusion_lock holds this list to the selector.
BINARIES='cofactor_mirror cofactor_family_counting rigid_residue_census sat_census frege_generator_ladder martin_lof_omega_kernel reflection_mirror orbit_stability_kernel uniform_transfer_theorem no_randomness_at_infinity no_finite_randomness_infinity family_emergence ef_class_probe'
TEST_ARGS=()
for bin in $BINARIES; do TEST_ARGS+=(--test "$bin"); done

echo "Running the P-vs-NP / SAT-coNP research campaign (--run-ignored all). This is SLOW."
# The probes run for minutes to hours, so they run under the `research` profile (.config/nextest.toml): a
# 6 h per-test runaway guard in place of `full`'s 30 min, inside a 100G memory scope.
exec cargo nextest run -p logicaffeine-tests --profile research --cargo-profile test-opt --run-ignored all \
  "${TEST_ARGS[@]}" -E "$RESEARCH_ONLY" "$@"
