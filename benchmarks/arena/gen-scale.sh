#!/usr/bin/env bash
#
# gen-scale.sh — generate a random-3SAT scaling ladder at the hardness threshold.
#
# Random 3-SAT at clause/variable ratio ≈ 4.267 is the canonical raw-CDCL stress test: hardness
# climbs steeply with n, with no exploitable structure, so it isolates pure search + propagation
# efficiency (the levers Kissat wins on at scale). We generate the SAME DIMACS for every engine.
#
# Usage: bash benchmarks/arena/gen-scale.sh ["250 350 450 550"] [count]
set -uo pipefail
cd "$(dirname "$0")"
SIZES="${1:-300 350 400}"
COUNT="${2:-4}"
RATIO=4.267
# Write straight into the arena's `sat` tree so each rung shows as its own category in arena_bench.
DEST="instances/sat"
for n in $SIZES; do rm -rf "$DEST/scale_n${n}"; done
mkdir -p "$DEST"

for n in $SIZES; do
  outdir="$DEST/scale_n${n}"
  mkdir -p "$outdir"
  python3 - "$n" "$COUNT" "$RATIO" "$outdir" <<'PY'
import sys, random
n, count, ratio, outdir = int(sys.argv[1]), int(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
m = round(ratio * n)
for i in range(count):
    random.seed(n * 100000 + i)
    lines = [f"p cnf {n} {m}"]
    for _ in range(m):
        vs = random.sample(range(1, n + 1), 3)
        lits = [v if random.random() < 0.5 else -v for v in vs]
        lines.append(f"{lits[0]} {lits[1]} {lits[2]} 0")
    open(f"{outdir}/rand3_n{n}_{i:02d}.cnf", "w").write("\n".join(lines) + "\n")
PY
  echo "n=$n: $COUNT instances (m=$(python3 -c "print(round($RATIO*$n))")) → $outdir"
done
echo "scale ladder: $(for n in $SIZES; do find "$DEST/scale_n${n}" -name '*.cnf'; done | wc -l | tr -d ' ') instances written into the arena tree"
