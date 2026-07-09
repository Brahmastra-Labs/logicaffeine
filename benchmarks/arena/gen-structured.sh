#!/usr/bin/env bash
#
# gen-structured.sh — generate the structured families our specialists crush (and resolution-based
# solvers blow up on), so the arena measures BOTH speeds of the solver in one place:
#   * pigeonhole (PHP)        → Hall matching refutation (vs exponential resolution: Haken 1985)
#   * clique-colouring        → covering-measure collapse
#   * Tseitin expander parity → GF(2) Gaussian 0=1 refutation (vs exponential resolution)
#   * mod-p counting (Count_p) → GF(p) Gaussian refutation. The mod-p obstruction GF(2) is BLIND to:
#                                resolution (kissat/cadical) AND Z3 both need 2^Ω(n); we refute in µs.
# These are the families on the /benchmarks page; kissat/cadical hit the resolution wall on them.
#
# Usage: bash benchmarks/arena/gen-structured.sh
set -uo pipefail
cd "$(dirname "$0")/../.."
SAT="target/release/examples/satbench"
[ -x "$SAT" ] || cargo build --release -p logicaffeine-proof --example satbench >/dev/null 2>&1
DEST="benchmarks/arena/instances/sat"

mkdir -p "$DEST/struct_php" "$DEST/struct_clique" "$DEST/struct_tseitin" "$DEST/struct_modp" \
         "$DEST/struct_chessboard" "$DEST/struct_ordering"
for n in 18 22 26 30; do "$SAT" gen-php "$n" > "$DEST/struct_php/php$n.cnf"; done
for nk in "10 9" "12 11" "14 13"; do set -- $nk; "$SAT" gen-clique "$1" "$2" > "$DEST/struct_clique/clique$1_$2.cnf"; done
for n in 80 100 120; do "$SAT" gen-tseitin "$n" 1 > "$DEST/struct_tseitin/tseitin$n.cnf"; done
# Mutilated chessboard (Hall-matching infeasibility) and the linear ordering principle GT(n) — both
# resolution-hard. NOTE: today these are HONEST measuring families, not yet our crushes: our covering
# recognizer does not fire on the domino encoding (→ CDCL), and GT(n) blows up our CDCL while the
# field linearizes it. They sit on the board to track the gap (chessboard Hall recognizer; CDCL/GTn).
for n in 10 12 14 16; do "$SAT" gen-chessboard "$n" > "$DEST/struct_chessboard/chess$n.cnf"; done
for n in 10 15 20 25; do "$SAT" gen-ordering "$n" > "$DEST/struct_ordering/gt$n.cnf"; done
# mod-p counting: UNSAT obstructions past the resolution wall (n≥60 p=3, n≥40 p=5 time out the field).
for spec in "60 3" "80 3" "100 3" "40 5" "60 5"; do
  set -- $spec; "$SAT" gen-modp "$1" "$2" 1 > "$DEST/struct_modp/modp_n$1_p$2.cnf"
done
# a satisfiable mod-p control: our GF(p) route returns a model, cross-checked against the field.
"$SAT" gen-modp-sat 60 3 1 > "$DEST/struct_modp/modp_sat_n60_p3.cnf"

echo "structured families written:"
for d in struct_php struct_clique struct_tseitin struct_modp struct_chessboard struct_ordering; do
  echo "  $d: $(find "$DEST/$d" -name '*.cnf' | wc -l | tr -d ' ')"
done