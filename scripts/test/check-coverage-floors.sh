#!/usr/bin/env bash
#
# check-coverage-floors.sh — the coverage RATCHET.
# Fails if any crate's measured region coverage has dropped below its committed floor.
# MONOTONE: a human RAISES a floor as coverage improves; never lower one to go green.
#
# Reads:  .config/coverage/crate-summary.json   (measured, written by scripts/test/run-coverage.sh)
#         .config/coverage/thresholds.toml ([floors] table, committed)
#
# Implemented as a SCRIPT (not a compiled test) so it can never break the build; it runs
# as a CI gate after the coverage job. Exit 0 = all floors met (or no data yet), 1 = a
# crate regressed below its floor.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

SUMMARY=".config/coverage/crate-summary.json"
THRESHOLDS=".config/coverage/thresholds.toml"

if [ ! -f "$SUMMARY" ]; then
  echo "coverage ratchet: no $SUMMARY yet (run scripts/test/run-coverage.sh) — inert, passing."
  exit 0
fi
command -v jq >/dev/null || { echo "ERROR: jq required"; exit 2; }

# Extract the committed floors from the [floors] table of thresholds.toml:
#   logicaffeine_X = NN.N   (up to the next [section]).
declare -A FLOOR
while IFS= read -r line; do
  key="${line%%=*}"; val="${line#*=}"
  key="$(echo "$key" | tr -d ' ')"; val="$(echo "$val" | sed 's/#.*//' | tr -d ' ')"
  [ -n "$key" ] && [ -n "$val" ] && FLOOR["$key"]="$val"
done < <(awk '/^\[floors\]/{f=1;next} /^\[/{f=0} f && /=/ && !/^[[:space:]]*#/{print}' "$THRESHOLDS")

if [ "${#FLOOR[@]}" -eq 0 ]; then
  echo "coverage ratchet: no [floors] committed yet — inert, passing."
  exit 0
fi

VIOLATIONS=0
printf "%-24s %10s %10s   %s\n" "crate" "measured" "floor" "status"
printf "%-24s %10s %10s   %s\n" "-----" "--------" "-----" "------"
for crate in $(printf '%s\n' "${!FLOOR[@]}" | sort); do
  floor="${FLOOR[$crate]}"
  measured="$(jq -r --arg c "$crate" '.[] | select(.crate == $c) | .region_pct' "$SUMMARY")"
  if [ -z "$measured" ] || [ "$measured" = "null" ]; then
    printf "%-24s %10s %10s   %s\n" "$crate" "n/a" "$floor" "no data (skipped)"
    continue
  fi
  # Compare measured >= floor (float) via awk.
  if awk "BEGIN{exit !($measured + 0.001 >= $floor)}"; then
    printf "%-24s %9s%% %9s%%   OK\n" "$crate" "$measured" "$floor"
  else
    printf "%-24s %9s%% %9s%%   ❌ REGRESSED\n" "$crate" "$measured" "$floor"
    VIOLATIONS=$((VIOLATIONS + 1))
  fi
done

echo
if [ "$VIOLATIONS" -gt 0 ]; then
  echo "COVERAGE RATCHET FAILED: $VIOLATIONS crate(s) below their committed floor."
  echo "⚠️ DO NOT lower the floor to go green — add tests. ⚠️"
  exit 1
fi
echo "coverage ratchet: all ${#FLOOR[@]} crate floors met."
exit 0
