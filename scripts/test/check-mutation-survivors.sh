#!/usr/bin/env bash
#
# check-mutation-survivors.sh — the mutation GATE.
# Fails if a cargo-mutants run produced a surviving ("MISSED") mutant that is NOT in the
# committed baseline .config/mutants/known-survivors.txt (where every entry is a PROVABLY EQUIVALENT
# mutant or a justified deferral). A new, unexplained survivor = a P0 test gap: write a test
# that kills it, or (if truly equivalent) add it to the baseline with a justification.
#
# Implemented as a SCRIPT (not a compiled test) so it can never break the build; runs as a CI
# gate after a mutation run.
#
# Usage: ./scripts/test/check-mutation-survivors.sh <cargo-mutants-log>
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

LOG="${1:-}"
BASELINE=".config/mutants/known-survivors.txt"

if [ -z "$LOG" ] || [ ! -f "$LOG" ]; then
  echo "usage: $0 <cargo-mutants-log>   (the stdout of a run-mutation-core.sh run)" >&2
  exit 2
fi
[ -f "$BASELINE" ] || { echo "ERROR: $BASELINE missing"; exit 2; }

# Normalize a survivor line to "file.rs:line:col: <mutation>" (strip timing + path prefix).
norm() { sed -E 's/ in [0-9].*$//; s#^.*/([a-z_]+\.rs)#\1#'; }

# Committed baseline survivor keys (first field before whitespace-run), comment lines skipped.
declare -A KNOWN
while IFS= read -r line; do
  key="$(echo "$line" | awk '{print $1}')"
  KNOWN["$key"]=1
done < <(grep -aE "^[a-z_]+\.rs:[0-9]+:[0-9]+" "$BASELINE")

NEW=0
while IFS= read -r survivor; do
  s="$(echo "$survivor" | norm)"
  key="$(echo "$s" | awk -F: '{print $1":"$2":"$3}')"   # file:line:col
  if [ -n "${KNOWN[$key]:-}" ]; then
    :  # a known equivalent/deferred survivor — fine
  else
    echo "  ❌ NEW SURVIVOR (not in baseline): $s"
    NEW=$((NEW + 1))
  fi
done < <(grep -aE "^MISSED|MISSED " "$LOG" | grep -aoE "[a-z_]+\.rs:[0-9]+:[0-9]+: .*")

echo
if [ "$NEW" -gt 0 ]; then
  echo "MUTATION GATE FAILED: $NEW new surviving mutant(s) not explained by $BASELINE."
  echo "→ Write a test that kills each, OR (if provably equivalent) add it to $BASELINE with a"
  echo "  justification. ⚠️ Do not add a real gap to the baseline to go green. ⚠️"
  exit 1
fi
echo "mutation gate: no new survivors — every surviving mutant is a documented equivalent."
exit 0
