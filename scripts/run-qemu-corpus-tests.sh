#!/usr/bin/env bash
#
# run-qemu-corpus-tests.sh — the RISC-V benchmark-corpus differentials, run on purpose.
#
# These two tests cross-check the WHOLE benchmark corpus against the VM inside `qemu-riscv64`:
#
#   logicaffeine-tests::riscv_aot_lock                   riscv_matches_the_vm_on_the_benchmark_corpus
#   logicaffeine-tests::riscv_qemu_corpus_differential   qa_execution_differential
#
# MEASURED scale: 70 programs (35 directories under benchmarks/programs/, each contributing `main.lg` and
# `interp.lg`), each at the sizes no larger than its smallest declared workload
# (crates/logicaffeine_tests/tests/support/riscv_runner.rs): 274 runs and 344 runs. Each program is
# assembled once with `riscv64-linux-gnu-gcc`; each run executes under `qemu-riscv64` inside a 512 MB
# address space and a 60 s limit, beside the VM oracle. Measured 134 s and 150 s at the test profile.
#
# WHY THEY ARE NOT IN THE ROUTINE BASELINE. Emulated execution is pure CPU with no I/O to overlap, so it
# neither degrades gracefully under load nor speeds up on a quiet box; the corpus runs here, serialized,
# the way the gate-level silicon sims run from scripts/run-silicon-tests.sh. They are excluded by
# `QEMU_CORPUS_EXCLUDE` in run-all-tests-fast.sh.
#
# NOT a coverage loss. `riscv_aot_lock`'s STATIC half — the `BENCHMARK_FLOOR` compile-coverage ratchet,
# which can only rise and needs no toolchain — is a separate assertion reached before the
# `have_toolchain()` check, and every other test in both binaries still runs in the baseline. What moves
# here is only the execution differential.
#
# The test set MUST stay in sync with QEMU_CORPUS_EXCLUDE in run-all-tests-fast.sh
# (enforced by crates/logicaffeine_tests/tests/ondemand_exclusion_lock.rs).
#
# Requires `riscv64-linux-gnu-gcc` and `qemu-riscv64`; the runner names both in LOGOS_REQUIRE_TOOLS, so a
# missing one fails the run instead of passing it as a skip.
#
# Usage: ./scripts/run-qemu-corpus-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

# Memory-capped qemu-riscv64 (scripts/test/toolwrap) resolves ahead of /usr/bin. qemu-user maps
# the guest's allocations into its OWN host process with no guest kernel in between, so a
# runaway RISC-V program becomes a runaway HOST process — measured at ~126 GB RSS on this
# 125 GiB box, which OOM-kills the entire user slice: every SSH session and the VS Code
# server, mid-run. nextest's slow-timeout cannot catch it (memory has no timeout; qemu goes
# 2 GB -> 126 GB well inside the 900s warning). The wrapper caps RLIMIT_AS so the guest gets
# a clean malloc failure in seconds and the test reports a real failure instead of the
# machine falling over. Override with LOGICAFFEINE_QEMU_MEM_MB (0 disables); see the header
# of scripts/test/toolwrap/qemu-riscv64.
export PATH="$REPO_ROOT/scripts/test/toolwrap:$PATH"


# The runner requires the tools it exists to exercise: with them named in LOGOS_REQUIRE_TOOLS, a missing one
# fails the run loudly instead of passing it as a skip.
export LOGOS_REQUIRE_TOOLS="${LOGOS_REQUIRE_TOOLS:+$LOGOS_REQUIRE_TOOLS,}riscv64-linux-gnu-gcc,qemu-riscv64"
for tool in riscv64-linux-gnu-gcc qemu-riscv64; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "ERROR: $tool not found on PATH — the execution half requires it, and the run will fail." >&2
  }
done

QEMU_CORPUS_ONLY='test(/^(riscv_matches_the_vm_on_the_benchmark_corpus|qa_execution_differential)$/)'
# The test binaries that selector can reach, each passed to cargo as `--test`, so the run builds only
# them: `-p logicaffeine-tests` alone builds every one of the crate's ~2,300 test binaries whatever `-E`
# selects. ondemand_exclusion_lock holds this list to the selector.
BINARIES='riscv_aot_lock riscv_qemu_corpus_differential'
TEST_ARGS=()
for bin in $BINARIES; do TEST_ARGS+=(--test "$bin"); done

echo "Running the RISC-V benchmark-corpus differentials (274 + 344 emulated runs beside the VM oracle)."
echo "Measured at about two and a half minutes each; they run one at a time."
exec cargo nextest run -p logicaffeine-tests --profile full --cargo-profile test-opt --run-ignored all \
  "${TEST_ARGS[@]}" -E "$QEMU_CORPUS_ONLY" "$@"
