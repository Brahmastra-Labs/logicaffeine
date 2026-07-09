#!/usr/bin/env bash
#
# run-research-tests.sh — the SAT/coNP "P-vs-NP" research campaign, run on purpose.
#
# These logicaffeine-proof test binaries are EXPLORATORY probes, not correctness locks:
# exhaustive orbit/census/ladder enumeration whose runtime grows with n, from minutes to
# HOURS. They are deliberately EXCLUDED from scripts/run-all-tests-fast.sh (the routine
# baseline) so a normal run stays semi-quick and never hangs — but they are not deleted,
# and this script runs exactly them, with --run-ignored, when you actually want the
# research numbers. Expect this to run for a long time.
#
# The binary list MUST stay in sync with RESEARCH_EXCLUDE in run-all-tests-fast.sh.
#
# Usage: ./scripts/run-research-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

# The research campaign binaries — identical set to RESEARCH_EXCLUDE in run-all-tests-fast.sh.
RESEARCH_ONLY='binary(/^(cofactor_mirror|cofactor_climb|cofactor_lens|cofactor_family_counting|rigid_residue_census|sat_census|frege_generator_ladder|martin_lof_omega_kernel|reflection_mirror|orbit_stability_kernel|ultimate_symmetry_finder|hardness_retreat|hardness_witness_ladder|uniform_transfer_theorem|no_randomness_at_infinity|no_finite_randomness_infinity|family_emergence|pvnp_gunsight|ef_class_probe)$/)'

echo "Running the P-vs-NP / SAT-coNP research campaign (--run-ignored all). This is SLOW."
# The `full` profile's 30-min terminate-after still applies as a per-test safety cap; pass
# --profile default and a larger cap on the command line if a probe legitimately needs longer.
exec cargo nextest run -p logicaffeine-proof --profile full --run-ignored all \
  -E "$RESEARCH_ONLY" "$@"
