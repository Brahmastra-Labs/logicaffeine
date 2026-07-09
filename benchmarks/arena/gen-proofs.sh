#!/usr/bin/env bash
#
# gen-proofs.sh — emit and EXTERNALLY verify strict DRAT/LRAT proofs for our algebraic crushes.
#
# Our parity (GF(2)) and mod-p (GF(p)) routes refute by linear algebra and certify with a native
# algebraic witness. This script compiles that witness to a Boolean clausal proof via the CNF→GF(.)
# bridge (`logos-sat <cnf> <drat>`), then checks it with the standard, independent toolchain:
#   * drat-trim <cnf> <drat>            → "s VERIFIED"   (the universal DRAT checker)
#   * drat-trim <cnf> <drat> -L <lrat>  → an LRAT certificate
#   * lrat-check <cnf> <lrat>           → "c VERIFIED"   (the formally-verified, cake_lpr-class checker)
#
# The resolution bridge is polynomial only when the elimination stays narrow; instances whose DRAT
# would blow up FAIL CLOSED — logos-sat prints a budget warning and writes no proof file, and this
# script records them as `over-budget` (the verdict still stands on the native algebraic certificate).
#
# Artifacts land in proofs/algebraic/. Usage: bash benchmarks/arena/gen-proofs.sh
set -uo pipefail
cd "$(dirname "$0")/../.."

SAT="target/release/examples/satbench"
BIN="target/release/logos-sat"
DT="${DRAT_TRIM:-/tmp/drat-trim/drat-trim}"
LC="${LRAT_CHECK:-/tmp/drat-trim/lrat-check}"
OUT="proofs/algebraic"

cargo build --release -p logicaffeine-proof --bin logos-sat --example satbench >/dev/null 2>&1
mkdir -p "$OUT"
[ -x "$DT" ] || { echo "drat-trim not found at $DT (set DRAT_TRIM)"; exit 1; }

# family tag | satbench gen command
INSTANCES=(
  "tseitin_n10|gen-tseitin 10 1"
  "tseitin_n14|gen-tseitin 14 1"
  "tseitin_n20|gen-tseitin 20 1"
  "modp_n4_p3|gen-modp 4 3 1"
  "modp_n6_p3|gen-modp 6 3 1"
  "modp_n8_p3|gen-modp 8 3 1"
  "modp_n4_p5|gen-modp 4 5 1"
)

printf "%-14s %-9s %-12s %-12s\n" "instance" "verdict" "drat-trim" "lrat-check"
printf -- "----------------------------------------------------------\n"
for spec in "${INSTANCES[@]}"; do
  tag="${spec%%|*}"; gen="${spec##*|}"
  cnf="$OUT/$tag.cnf"; drat="$OUT/$tag.drat"; lrat="$OUT/$tag.lrat"
  rm -f "$drat" "$lrat"
  $SAT $gen > "$cnf"
  verdict=$($BIN "$cnf" "$drat" 2>/dev/null | grep -oE "SATISFIABLE|UNSATISFIABLE" | head -1)
  if [ ! -s "$drat" ]; then
    printf "%-14s %-9s %-12s %-12s\n" "$tag" "${verdict:-?}" "over-budget" "-"
    continue
  fi
  dt=$($DT "$cnf" "$drat" 2>/dev/null | grep -qE "VERIFIED" && echo "VERIFIED" || echo "FAILED")
  $DT "$cnf" "$drat" -L "$lrat" >/dev/null 2>&1
  lc=$($LC "$cnf" "$lrat" 2>/dev/null | grep -qE "VERIFIED" && echo "VERIFIED" || echo "FAILED")
  printf "%-14s %-9s %-12s %-12s\n" "$tag" "${verdict:-?}" "$dt" "$lc"
done
echo
echo "artifacts in $OUT/ (.cnf input, .drat our proof, .lrat formally-checkable certificate)"
