#!/usr/bin/env bash
# Write the committed silicon artifacts the locks derive, then check them: the named test writes its file under
# LOGOS_REGEN=1 and compares. No suite runner sets LOGOS_REGEN, so a test run never changes a tracked file.
#
#   scripts/test/gen-silicon-goldens.sh ledger          benchmarks/silicon/tiny_transformer_fabric.json
#   scripts/test/gen-silicon-goldens.sh demo-manifest   benchmarks/silicon/programs/bench_silicon_manifest.txt
#   scripts/test/gen-silicon-goldens.sh wires           benchmarks/silicon/programs/bench_*.wire + bench_manifest.txt
#                                                       (WPE_PROFILE=stream: s200_*.wire + s200_manifest.txt)
#   scripts/test/gen-silicon-goldens.sh lsum-soc        benchmarks/silicon/nexys_seqprog_lsum.v
#
# The wires and the lsum SoC are board inputs: the committed ones are what the boards ran. A new one is proven only
# on the board (`scripts/logos-chip.py suite wpe --manifest benchmarks/silicon/programs/bench_manifest.txt`; for the
# SoC, rebuild nexys_seqprog_lsum.bit and run `scripts/validate-on-nexys-seqstate.sh`), so regenerate them when you
# re-validate on silicon, and record that run.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

regen() {
  LOGOS_REGEN=1 cargo nextest run -p logicaffeine-tests --run-ignored all --no-fail-fast --test "$1" -E "test(=$2)"
}

case "${1:-}" in
  ledger)        regen transformer_fabric_lock the_cycle_ledger_is_the_one_this_lock_derives ;;
  demo-manifest) regen wprog_all_benchmarks_lane silicon_demo_manifest ;;
  wires)         regen wprog_all_benchmarks_lane generate_benchmark_wires_for_the_fpga ;;
  lsum-soc)      regen fabric_seq_datapath_lock generate_lsum_state_plane_soc ;;
  *) echo "usage: $0 ledger|demo-manifest|wires|lsum-soc" >&2; exit 2 ;;
esac
