#!/usr/bin/env bash
# Diagnostic: dump the forge register-class histogram (LOGOS_DUMP_CLASS) for one
# or more benchmarks at their CALIBRATED interpreter workload size. Sets up the
# minimal Largo project the harness needs (raw main.lg errors "Largo.toml not
# found"), pulls N from benchmarks/results/calibrated-interp-sizes.json, and runs
# the CLASSIC regalloc path (LOGOS_REGALLOC_PRECISE=0) so the float-class +
# placement + linear-scan-spill simulation line is emitted. Read-only; no timing,
# safe to run while a build/test holds its token but NOT during a benchmark A/B
# (it executes the workload and would perturb timing).
#
# Usage: scripts/diag-class.sh nbody mandelbrot pi_leibniz spectral_norm
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LARGO="$ROOT/target/release/largo"
SIZES="$ROOT/benchmarks/results/calibrated-interp-sizes.json"
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
for bench in "$@"; do
  lg="$ROOT/benchmarks/programs/$bench/main.lg"
  [ -f "$lg" ] || { echo "$bench: no main.lg"; continue; }
  N=$(python3 -c "import json,sys; d=json.load(open('$SIZES'))['benchmarks']; print(d.get('$bench',{}).get('250',{}).get('n',''))" 2>/dev/null)
  [ -n "$N" ] || { echo "$bench: no calibrated N"; continue; }
  d="$TMP/$bench"; mkdir -p "$d/src"; cp "$lg" "$d/src/main.lg"
  printf '[package]\nname = "bench"\nversion = "0.1.0"\nentry = "src/main.lg"\n' > "$d/Largo.toml"
  echo "=== $bench (N=$N) ==="
  ( cd "$d" && LOGOS_REGALLOC_PRECISE=0 LOGOS_DUMP_CLASS=1 "$LARGO" run --interpret "$N" 2>&1 >/dev/null </dev/null ) \
    | grep 'DUMP-CLASS'
done
