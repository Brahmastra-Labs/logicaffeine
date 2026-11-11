#!/usr/bin/env bash
#
# fetch-satcomp.sh — download a sample of a SAT Competition main track, each instance with its known verdict.
#
# The main track is the competition's ground truth for "competitive at scale": industrial and crafted families of
# 10^4–10^7 clauses where the SATLIB corpus tops out near 3000 variables. The Global Benchmark Database (Iser &
# Jabs, GBD) serves every instance by its content hash and answers queries over its features, so the sample is
# drawn the same way every run: within the clause cap, the instances GBD records satisfiable and those it records
# unsatisfiable, each list in hash order, every k-th kept until `PER_RESULT` of each. The instances land in
# instances/sat/satcomp<year>_{sat,unsat}/<hash>.cnf, the category naming the expected verdict for the arena's
# cross-check; the compressed downloads are cached so re-runs are offline.
#
# Provenance: GBD, https://benchmark-database.de (SAT Competition benchmarks, https://satcompetition.github.io)
#
# Usage: bash benchmarks/arena/fetch-satcomp.sh [year=2024] [per_result=40] [max_clauses=500000]
set -uo pipefail
cd "$(dirname "$0")"

YEAR="${1:-2024}"
PER_RESULT="${2:-40}"
MAX_CLAUSES="${3:-500000}"
GBD="https://benchmark-database.de"
CACHE="cache/satcomp$YEAR"
mkdir -p "$CACHE"

for result in sat unsat; do
  dest="instances/sat/satcomp${YEAR}_${result}"
  mkdir -p "$dest"
  query="track%3Dmain_${YEAR}%20and%20result%3D${result}%20and%20clauses%20%3C%20${MAX_CLAUSES}"
  list="$CACHE/${result}.list"
  if [ ! -s "$list" ]; then
    curl -sSL -m 120 "$GBD/getinstances?query=$query&context=cnf" | grep -o '[0-9a-f]\{32\}' | sort -u > "$list" \
      || { echo "  skip $result (query failed)"; rm -f "$list"; continue; }
  fi
  total=$(wc -l < "$list" | tr -d ' ')
  [ "$total" -gt 0 ] || { echo "  $result: GBD lists no instance"; continue; }
  step=$(( total / PER_RESULT )); [ "$step" -ge 1 ] || step=1
  kept=0
  while read -r hash; do
    xz="$CACHE/$hash.cnf.xz"
    if [ ! -s "$xz" ]; then
      curl -sSL -m 600 -o "$xz" "$GBD/file/$hash" || { echo "  skip $hash (download failed)"; rm -f "$xz"; continue; }
    fi
    xz -dc "$xz" > "$dest/$hash.cnf" 2>/dev/null || { echo "  skip $hash (decompress failed)"; rm -f "$dest/$hash.cnf"; continue; }
    kept=$((kept + 1))
  done < <(awk -v step="$step" -v cap="$PER_RESULT" 'NR % step == 0 && n < cap { print; n++ }' "$list")
  echo "  satcomp${YEAR}_${result}: $kept of $total instances under $MAX_CLAUSES clauses"
done
echo "normalize with: bash benchmarks/arena/normalize-sat.sh"
