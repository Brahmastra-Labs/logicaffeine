#!/usr/bin/env bash
#
# select-solvable.sh — keep the instances a reference solver decides within a CPU budget.
#
# A competition main track is calibrated for a 5000-second budget, so at an arena budget of a minute most of it times
# out for every engine and ranks nothing. This runs the reference (Kissat by default) on each instance of the given
# categories under `prlimit --cpu=<secs>`, one pinned core per job, and hard-links each instance it decides — with
# the verdict the category name expects, `_sat` or `_unsat` — into `<category>_k<secs>`. The arena then compares
# engines on what the reference can do in that budget, at the same budget or a multiple of it.
#
# Usage: bash benchmarks/arena/select-solvable.sh <secs> <category>...
# Env:   REF_BIN (default external/solvers/kissat/build/kissat), CORES (default "12 14 16 18")
set -uo pipefail
cd "$(dirname "$0")"

SECS="${1:?usage: select-solvable.sh <secs> <category>...}"
shift
REF="${REF_BIN:-../../external/solvers/kissat/build/kissat}"
read -r -a CORES <<< "${CORES:-12 14 16 18}"

decide() {
  local core=$1 file=$2 expect=$3 dest=$4
  local verdict
  verdict=$(taskset -c "$core" prlimit --cpu="$SECS" "$REF" -q "$file" 2>/dev/null | awk '/^s SATISFIABLE/{print "SAT"} /^s UNSATISFIABLE/{print "UNSAT"}')
  if [ "$verdict" = "$expect" ]; then
    ln -f "$file" "$dest/$(basename "$file")"
  elif [ -n "$verdict" ]; then
    echo "  DISAGREEMENT: $file decided $verdict, expected $expect"
  fi
}

for category in "$@"; do
  src="instances/sat/$category"
  expect=""
  case "$category" in *_sat) expect=SAT;; *_unsat) expect=UNSAT;; esac
  [ -n "$expect" ] || { echo "  skip $category (no _sat/_unsat suffix to check against)"; continue; }
  dest="instances/sat/${category}_k${SECS}"
  mkdir -p "$dest"
  i=0
  for file in "$src"/*.cnf; do
    core=${CORES[$((i % ${#CORES[@]}))]}
    decide "$core" "$file" "$expect" "$dest" &
    i=$((i + 1))
    if [ $((i % ${#CORES[@]})) -eq 0 ]; then
      wait
    fi
  done
  wait
  echo "  ${category}_k${SECS}: $(find "$dest" -name '*.cnf' | wc -l | tr -d ' ') of $i decided within ${SECS}s"
done
