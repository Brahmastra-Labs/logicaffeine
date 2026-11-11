#!/usr/bin/env bash
#
# run-eda.sh — the EDA arenas (`eda_bench`): model checking on HWMCC'24 against ABC's pdr, bounded model checking on its
# designs the competition did not find safe against ABC's bmc3, rewriting on the EPFL suite against ABC's rewrite and
# resyn2, and exact model counting on the 2024 model counting competition's first track.
# Each run is limited and timed in CPU seconds; each arena writes benchmarks/results/eda/<arena>.json, every verdict
# scored against the competition's consensus and every rewritten circuit checked equal to its input by ABC's cec.
#
# Usage: bash benchmarks/eda/run-eda.sh [mc bmc rewrite count] — needs yosys-abc on the PATH and the corpora under
# external/ (HWMCC'24 AIGER with its results CSV, the EPFL benchmarks, MCC 2024 track 1). EDA_CORES (a comma list of
# CPUs, say the performance cores of a hybrid machine) runs one worker on each, every run pinned there, in place of
# EDA_JOBS workers wherever the scheduler puts them.
set -euo pipefail
cd "$(dirname "$0")/../.."
ARENAS="${*:-mc bmc rewrite count}"
JOBS="${EDA_JOBS:-8}"
TIMEOUT="${EDA_TIMEOUT:-60}"
cargo build --release -p logicaffeine-proof --example eda_bench --example mc_check --example rewrite_check --example count_check
BENCH="${CARGO_TARGET_DIR:-target}/release/examples/eda_bench"
for arena in $ARENAS; do
  if [ -n "${EDA_CORES:-}" ]; then
    "$BENCH" "$arena" --cores "$EDA_CORES" --timeout "$TIMEOUT"
  else
    "$BENCH" "$arena" --jobs "$JOBS" --timeout "$TIMEOUT"
  fi
done
