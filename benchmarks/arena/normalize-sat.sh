#!/usr/bin/env bash
#
# normalize-sat.sh — make the fetched corpus strict, competition-grade DIMACS.
#
# The SAT Competition only ever feeds strict, well-formed DIMACS. Some SATLIB files are not:
#   * uf*/uuf* random instances carry a trailing `%` / `0` marker after the last clause;
#   * a few crafted files (e.g. dubois100) are corrupt — clauses missing their `0` terminator,
#     so the header clause-count disagrees with reality and a lenient parser silently merges
#     clauses into the wrong formula (a fake verdict).
#
# This pass strips the `%` trailer, then keeps a file ONLY if its declared clause count equals the
# actual number of `0` terminators (exactly the strictness `kissat` enforces). Mismatches are moved
# to instances/sat/_malformed/ so the run set is clean and every engine sees an identical, valid
# formula. Idempotent.
#
# Usage: bash benchmarks/arena/normalize-sat.sh
set -uo pipefail
cd "$(dirname "$0")"

ROOT="instances/sat"
QUAR="$ROOT/_malformed"
kept=0
quarantined=0

# Print the file up to (but excluding) the first line whose first non-blank char is `%`.
strip_pct() { awk '{ s=$0; sub(/^[ \t]+/,"",s); if (substr(s,1,1)=="%") exit; print }' "$1"; }

while IFS= read -r -d '' f; do
  case "$f" in "$QUAR"/*) continue;; esac
  header="$(grep -m1 '^p cnf' "$f" || true)"
  if [ -z "$header" ]; then
    cat=$(basename "$(dirname "$f")"); mkdir -p "$QUAR/$cat"; mv -f "$f" "$QUAR/$cat/"; quarantined=$((quarantined+1)); continue
  fi
  declared=$(echo "$header" | awk '{print $4}')
  tmp="$(mktemp)"
  strip_pct "$f" > "$tmp"
  zeros=$(grep -v '^c' "$tmp" | grep -v '^p cnf' | tr -s ' \t' '\n' | grep -c '^0$' || true)
  if [ "$zeros" = "$declared" ]; then
    mv -f "$tmp" "$f"; kept=$((kept+1))
  else
    rm -f "$tmp"
    cat=$(basename "$(dirname "$f")"); mkdir -p "$QUAR/$cat"; mv -f "$f" "$QUAR/$cat/"; quarantined=$((quarantined+1))
  fi
done < <(find "$ROOT" -name '*.cnf' -print0)

echo "normalized: $kept kept, $quarantined quarantined → $QUAR"
echo "clean run set: $(find "$ROOT" -name '*.cnf' -not -path "$QUAR/*" | wc -l | tr -d ' ') instances"