#!/usr/bin/env bash
# LOGICAFFEINE Solver-vs-the-field Benchmark
#
# Times our pure-Rust certified prover against Z3 (SMT), Kissat (CDCL champion), SaDiCaL (the
# reference PR/SDCL solver), CaDiCaL (Biere's mainline DRAT reference) and CryptoMiniSat (native
# GF(2) Gaussian) on the families where structure beats brute force — pigeonhole (+ functional/onto/
# weak variants), mutilated chessboard, Tseitin GF(2) parity, mod-p (GF(3/5/7)), mod-6 ℤ/6 ring, plus
# a random-3SAT control — and writes results/solvers.json, baked into the /benchmarks website page.
# External solvers run as subprocesses on the byte-identical DIMACS, each emitting a clausal proof to
# disk whose size is recorded; OUR proof size is the certified artifact (SR proof / compact GF-ring
# certificate) measured in memory. A solver that cannot finish in its timeout is recorded `timeout`
# (the wall); a missing binary is simply omitted.
#
# Requirements: Z3 (Linux: /usr/include/z3.h) + the `verification` feature; python3 (sanity check).
# Optional external solvers via env (defaults to the /tmp build locations):
#   KISSAT_BIN  (default /tmp/kissat/build/kissat)
#   SADICAL_BIN (default /tmp/sadical/sadical/sadical)
#   CADICAL_BIN (default /tmp/cadical/build/cadical)
#   CMS_BIN     (default /tmp/cryptominisat/build/cryptominisat5)
#
# Usage: bash benchmarks/run-solver-vs-z3.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
export SOLVERS_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

OUT="$SCRIPT_DIR/results/solvers.json"
mkdir -p "$SCRIPT_DIR/logs"
LOG="$SCRIPT_DIR/logs/solver-vs-z3-$(date +%Y%m%d-%H%M%S).log"

echo "Building + running solver_bench (release, verification) … progress → $LOG"
cargo run --release -p logicaffeine-proof --example solver_bench --features verification \
  2> "$LOG" > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
echo "Wrote $OUT"

# Sanity: valid JSON with a non-empty families array.
python3 -c "import json; d=json.load(open('$OUT')); assert d['families'], 'no families'; print('families:', [f['id'] for f in d['families']])"

echo "Progress log: $LOG"
