#!/usr/bin/env bash
#
# run-kernel-pending-tests.sh — the kernel NORTH-STAR specs for an importer capability not built yet,
# run on purpose.
#
# These are `#[ignore]`d with a `kernel-pending:` reason (so `cargo nextest run -p logicaffeine-kernel`
# stays green) and EXCLUDED from scripts/run-all-tests-fast.sh's `--run-ignored all` pass (see
# KERNEL_PENDING_EXCLUDE):
#   • lazy_conversion::runtime_controls::original_app_lam_converts_without_expanding_its_captured_source
#   • lazy_conversion::runtime_import_controls::the_entire_original_app_lam_imports_and_publishes_a_checked_definition
# Both need the app-lam original converted and published without expanding its captured source. The
# production sharing path cannot publish it at any allowance, and the test-only certified normal form
# spends 518,659 of a 100,000 budget; the capability is a fused evaluator promoted into the production
# importer.
#
# Each is EXPECTED to fail today, and the run's verdict is that each fails with exactly its gap: the
# runner exits 0 iff every spec FAILS and its panic carries its signature in KERNEL_PENDING_GAPS
# (scripts/test/kernel-pending-verdict.py reads nextest's JUnit). A spec that PASSES turns the run red
# with the instruction to un-ignore it — the capability landed; a spec that fails some other way turns
# it red with its panic; a build that fails keeps cargo's exit. The test set MUST stay in sync with
# KERNEL_PENDING_EXCLUDE in run-all-tests-fast.sh, and the table with the selector
# (ondemand_exclusion_lock enforces both).
#
# Usage: ./scripts/run-kernel-pending-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

KERNEL_PENDING_ONLY='package(logicaffeine-kernel) and test(/::(original_app_lam_converts_without_expanding_its_captured_source|the_entire_original_app_lam_imports_and_publishes_a_checked_definition)$/)'
# Each spec and its gap: a stable substring of the panic that names the capability the spec waits for.
KERNEL_PENDING_GAPS='original_app_lam_converts_without_expanding_its_captured_source|lazy conversion trace allowance exhausted
the_entire_original_app_lam_imports_and_publishes_a_checked_definition|shared term materialization budget'

# nextest writes the `kernel-pending` profile's JUnit under the workspace's target/nextest/, whatever
# CARGO_TARGET_DIR says; a stale report is removed first so a run that never reaches a verdict cannot read one.
JUNIT="$REPO_ROOT/target/nextest/kernel-pending/junit.xml"
rm -f "$JUNIT"

echo "Running the kernel PENDING specs (--run-ignored all)."
echo "Each must FAIL with its own gap until the importer publishes the app-lam original through a certified normal form."
cargo nextest run -p logicaffeine-kernel --lib --profile kernel-pending --cargo-profile test-opt --run-ignored all \
  -E "$KERNEL_PENDING_ONLY" "$@"
python3 "$SCRIPT_DIR/test/kernel-pending-verdict.py" "$JUNIT" "$KERNEL_PENDING_GAPS" "$?"
