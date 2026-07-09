#!/usr/bin/env bash
#
# fetch-sat.sh — download a curated, competition-style SAT corpus (the "targets").
#
# Pulls classic SATLIB benchmark families that span the same axes the SAT Competition main track
# scores on: real application/circuit instances, hard crafted UNSAT, and uniform-random 3-SAT at
# the phase transition. Each family lands flattened under instances/sat/<category>/*.cnf. Tarballs
# are cached so re-runs are offline. A family whose download fails is skipped (the run still works
# on whatever is present), mirroring run-satbench.sh's resilient provenance discipline.
#
# Provenance: SATLIB (Hoos & Stützle), https://www.cs.ubc.ca/~hoos/SATLIB/benchm.html
#
# Usage: bash benchmarks/arena/fetch-sat.sh
set -uo pipefail
cd "$(dirname "$0")"

BASE="https://www.cs.ubc.ca/~hoos/SATLIB/Benchmarks/SAT"
DEST="instances/sat"
CACHE="cache"
mkdir -p "$DEST" "$CACHE"

# category  →  tarball path under $BASE  (category encodes the family's competition flavor)
SETS=(
  "app_ssa|DIMACS/SSA/ssa.tar.gz"                       # circuit single-stuck-at fault (application)
  "app_bf|DIMACS/BF/bf.tar.gz"                          # circuit (application)
  "app_hanoi|DIMACS/HANOI/hanoi.tar.gz"                 # Towers of Hanoi planning (application)
  "app_logistics|PLANNING/logistics.tar.gz"            # logistics planning (application)
  "app_blocksworld|PLANNING/BlocksWorld/blocksworld.tar.gz"  # blocksworld planning (application)
  "crafted_aim|DIMACS/AIM/aim.tar.gz"                   # crafted SAT+UNSAT
  "crafted_dubois|DIMACS/DUBOIS/dubois.tar.gz"          # hard crafted UNSAT
  "crafted_pret|DIMACS/PRET/pret.tar.gz"                # hard crafted UNSAT
  "crafted_parity|DIMACS/PARITY/parity.tar.gz"          # parity-learning (crafted)
  "random_jnh|DIMACS/JNH/jnh.tar.gz"                    # random
  "random_uf250|RND3SAT/uf250-1065.tar.gz"             # random 3-SAT at threshold (SAT)
  "random_uuf250|RND3SAT/uuf250-1065.tar.gz"           # random 3-SAT at threshold (UNSAT)
)

for entry in "${SETS[@]}"; do
  name="${entry%%|*}"; rel="${entry#*|}"
  tgz="$CACHE/$name.tar.gz"
  if [ ! -s "$tgz" ]; then
    echo "fetching $name …"
    curl -sSL -m 180 -o "$tgz" "$BASE/$rel" || { echo "  skip $name (download failed)"; rm -f "$tgz"; continue; }
  fi
  out="$DEST/$name"
  mkdir -p "$out"
  tar xzf "$tgz" -C "$out" 2>/dev/null || { echo "  skip $name (extract failed)"; continue; }
  # Flatten: hoist every .cnf to the category dir, drop the now-empty nesting.
  find "$out" -mindepth 2 -name '*.cnf' -exec mv -f -t "$out" {} + 2>/dev/null
  find "$out" -mindepth 1 -type d -empty -delete 2>/dev/null
  echo "  $name: $(find "$out" -maxdepth 1 -name '*.cnf' | wc -l | tr -d ' ') instances"
done

echo "total: $(find "$DEST" -name '*.cnf' | wc -l | tr -d ' ') instances under $DEST"
