#!/usr/bin/env bash
#
# run-opt-in-feature-tests.sh — the suites behind the features the baseline leaves off, run on purpose.
#
# scripts/run-all-tests-fast.sh builds the workspace with `--features verification,logicaffeine-tests/web-tests`
# (and pass 1b with `wasm-jit`). A test compiled only under some OTHER feature is not filtered out of that
# baseline — it does not exist in it, and nothing reports it missing. Each feature below changes what the
# shipped build is, so it stays off the baseline, and its suites run here instead, each feature in a cargo
# invocation of its own: one invocation with all of them would unify the features and build none of them
# the way it is used.
#
#   • logicaffeine-kernel/nbe — the dag core's closure-based conversion engine, opt-in while it is under
#     construction: the NbE checker, the question-for-question differential against the classic engine,
#     and the dual-engine agreement over every vendored Arena case.
#   • logicaffeine-system/io-uring — the io_uring HTTP engine and file backend (Linux): the engine battery,
#     the engine bench, the relay upgrade through the io_uring engine, and the system crate's unit tests
#     built with the feature.
#   • logicaffeine-wirebench/capnproto — the head-to-heads against Cap'n Proto on its own axes (the wirebench
#     build script runs `capnp`).
#   • logicaffeine-tests/ffi-link-tests — phase_ffi_requires' end-to-end runs, which build a LOGOS program
#     against real crates.io crates (they need the network). CI turns this one on in its suite as well.
#   • logicaffeine-compile/narrow-value — the 8-byte NaN-boxed VM register cell; the compile crate's unit
#     tests built with it (the VM↔tree-walker differential holds under either representation).
#   • logicaffeine-kernel/shadow — the dag core re-decides every declaration the `Term` kernel's checked
#     doors decide and panics on a disagreement; the controls of that decision, built with it.
#
# The baseline's exclusion classes hold here as well: a test run-all-tests-fast.sh leaves to an on-demand runner
# runs in that runner, under whatever feature it needs (the Arena runner turns on `nbe` for dag_dual_agreement's
# whole-Mathlib case), and never in this pass. Every invocation filters by BASELINE_FILTER, built from the
# gate's own `*_EXCLUDE` assignments as CI builds its filter, so a class added there is dropped here too.
#
# feature_coverage_lock reads this script's invocations beside the baseline's: a feature-gated test that no
# invocation here or there turns on is red, so a new opt-in suite cannot go unrun.
#
# Usage: ./scripts/run-opt-in-feature-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

command -v capnp >/dev/null 2>&1 || echo "WARNING: capnp not found on PATH — the capnproto build fails." >&2
# The NbE differential checks Lean's whole `Init` export under reference-kernels/: required here, so a missing
# tree fails loudly instead of passing as a skip (arena_corpus_lock holds this line).
export LOGOS_REQUIRE_TOOLS="${LOGOS_REQUIRE_TOOLS:+$LOGOS_REQUIRE_TOOLS,}reference-kernels"

BASELINE_FILTER=""
while IFS= read -r assignment; do
  eval "$assignment"
  class="${assignment%%=*}"
  BASELINE_FILTER="${BASELINE_FILTER:+$BASELINE_FILTER and }${!class}"
done < <(grep -E "^[A-Z_]+_EXCLUDE='" scripts/run-all-tests-fast.sh)
[ -n "$BASELINE_FILTER" ] || { echo "no exclusion class read from scripts/run-all-tests-fast.sh" >&2; exit 2; }

STATUS=0
section() {
  echo
  echo "──────────────────────────────────────────────────────────────────────"
  echo " $1"
  echo "──────────────────────────────────────────────────────────────────────"
}

section "logicaffeine-kernel/nbe"
cargo nextest run -p logicaffeine-tests --features nbe --profile full --cargo-profile test-opt --run-ignored all \
  --test dag_nbe_checker --test dag_nbe_differential --test dag_dual_agreement -E "$BASELINE_FILTER" "$@" || STATUS=1

section "logicaffeine-system/io-uring"
cargo nextest run -p logicaffeine-tests --features io-uring --profile full --cargo-profile test-opt --run-ignored all \
  --test http_engine_uring --test http_engine_bench --test backend_relay_lock -E "$BASELINE_FILTER" "$@" || STATUS=1
cargo nextest run -p logicaffeine-system --features full,io-uring --profile full --cargo-profile test-opt --run-ignored all \
  --lib -E "$BASELINE_FILTER" "$@" || STATUS=1

section "logicaffeine-wirebench/capnproto"
cargo nextest run -p logicaffeine-tests --features capnproto --profile full --cargo-profile test-opt --run-ignored all \
  --test bulk_cursor --test codec_report --test superiority -E "$BASELINE_FILTER" "$@" || STATUS=1

section "logicaffeine-tests/ffi-link-tests"
cargo nextest run -p logicaffeine-tests --features ffi-link-tests --profile full --cargo-profile test-opt --run-ignored all \
  --test phase_ffi_requires -E "$BASELINE_FILTER" "$@" || STATUS=1

section "logicaffeine-compile/narrow-value"
cargo nextest run -p logicaffeine-compile --features narrow-value --profile full --cargo-profile test-opt --run-ignored all \
  --lib -E "$BASELINE_FILTER" "$@" || STATUS=1

section "logicaffeine-kernel/shadow"
cargo nextest run -p logicaffeine-kernel --features shadow --profile full --cargo-profile test-opt --run-ignored all \
  --lib -E "$BASELINE_FILTER" dag::mirror::shadow_controls "$@" || STATUS=1

echo
echo "EXIT: $STATUS"
exit "$STATUS"
