#!/usr/bin/env bash
#
# compare-test-runs.sh — prove two test-run logs executed the same tests.
#
# Extracts the full set of executed test identities (binary::test_name, with
# doctests as doctest:crate::name) from each log and diffs them. Handles both
# log formats:
#   - cargo test   (the baseline run-all-tests.sh log)
#   - cargo nextest + cargo test --doc  (the run-all-tests-fast.sh log)
# Each log is parsed with BOTH extractors and the results unioned, so a log
# containing both formats (the fast log) is fully covered.
#
# Exit 0 only when the two sets are identical.
#
# Usage: ./scripts/compare-test-runs.sh <baseline-log> <fast-log>
#        ./scripts/compare-test-runs.sh logs/latest.log logs/latest-fast.log
#
set -uo pipefail

if (( $# != 2 )); then
  echo "usage: $0 <baseline-log> <fast-log>" >&2
  exit 2
fi
LOG_A="$1"; LOG_B="$2"
[[ -r "$LOG_A" ]] || { echo "ERROR: cannot read $LOG_A" >&2; exit 2; }
[[ -r "$LOG_B" ]] || { echo "ERROR: cannot read $LOG_B" >&2; exit 2; }

# --- extractor: cargo test format ---------------------------------------------
# Tracks the current binary from "Running ... (target/.../deps/<bin>-<hash>)"
# context lines and "Doc-tests <crate>" headers, then emits one identity per
# "test <name> ... <status>" line.
extract_cargo() {
  awk '
    function norm(s) { gsub(/-/, "_", s); return s }
    /^[[:space:]]*Running .*\(.*\/deps\/[^)]*\)/ {
      path = $0
      sub(/^.*\/deps\//, "", path); sub(/\).*$/, "", path)
      sub(/-[0-9a-f]+$/, "", path)
      ctx = norm(path)
      next
    }
    /^[[:space:]]*Doc-tests / {
      crate = $0
      sub(/^[[:space:]]*Doc-tests[[:space:]]+/, "", crate)
      ctx = "doctest:" norm(crate)
      next
    }
    /^test .* \.\.\. (ok|FAILED|ignored)/ {
      if (ctx == "") next
      name = $0
      sub(/^test /, "", name)
      sub(/ \.\.\. (ok|FAILED|ignored).*$/, "", name)
      sub(/ - should panic$/, "", name)
      print ctx "::" name
    }
  ' "$1"
}

# --- extractor: nextest format -------------------------------------------------
# One identity per "STATUS [ time] <binary-id> <test name>" line. Binary ids:
#   crate-name::testfile   → testfile          (integration test)
#   crate-name::bin/name   → name              (bin-target unit tests)
#   crate-name             → crate_name        (lib unit tests)
extract_nextest() {
  awk '
    function norm(s) { gsub(/-/, "_", s); return s }
    /^[[:space:]]*(PASS|FAIL|LEAK|SLOW|TIMEOUT|ABORT|TRY [0-9]+ [A-Z]+) \[[^]]*\] [^ ]+ / {
      line = $0
      sub(/^[[:space:]]*/, "", line)
      sub(/^TRY [0-9]+ /, "", line)
      sub(/^[A-Z]+ \[[^]]*\] /, "", line)
      sub(/^\([^)]*\) /, "", line)
      id = line
      sub(/ .*$/, "", id)
      name = line
      sub(/^[^ ]+ /, "", name)
      if (id ~ /::/) {
        sub(/^.*::/, "", id)
        sub(/^(bin|build)\//, "", id)
        id = norm(id)
      } else {
        id = norm(id)
      }
      print id "::" name
    }
  ' "$1"
}

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

{ extract_cargo "$LOG_A"; extract_nextest "$LOG_A"; } | sort -u > "$TMP_DIR/a.txt"
{ extract_cargo "$LOG_B"; extract_nextest "$LOG_B"; } | sort -u > "$TMP_DIR/b.txt"

COUNT_A="$(wc -l < "$TMP_DIR/a.txt")"
COUNT_B="$(wc -l < "$TMP_DIR/b.txt")"
comm -23 "$TMP_DIR/a.txt" "$TMP_DIR/b.txt" > "$TMP_DIR/missing.txt"   # in A, not in B
comm -13 "$TMP_DIR/a.txt" "$TMP_DIR/b.txt" > "$TMP_DIR/extra.txt"     # in B, not in A
MISSING="$(wc -l < "$TMP_DIR/missing.txt")"
EXTRA="$(wc -l < "$TMP_DIR/extra.txt")"

DOC_A="$(grep -c '^doctest:' "$TMP_DIR/a.txt" || true)"
DOC_B="$(grep -c '^doctest:' "$TMP_DIR/b.txt" || true)"

echo "════════════════════════════════════════════════════════════════════"
echo " TEST-RUN PARITY"
echo "════════════════════════════════════════════════════════════════════"
echo " A (baseline) : $LOG_A"
echo "                $COUNT_A tests executed ($DOC_A doctests)"
echo " B (fast)     : $LOG_B"
echo "                $COUNT_B tests executed ($DOC_B doctests)"
echo "────────────────────────────────────────────────────────────────────"
echo " missing from B (ran in A only) : $MISSING"
echo " extra in B     (ran in B only) : $EXTRA"

if (( MISSING > 0 )); then
  echo
  echo " MISSING FROM B:"
  sed 's/^/   /' "$TMP_DIR/missing.txt"
fi
if (( EXTRA > 0 )); then
  echo
  echo " EXTRA IN B:"
  sed 's/^/   /' "$TMP_DIR/extra.txt"
fi

echo
if (( MISSING == 0 && EXTRA == 0 && COUNT_A > 0 )); then
  echo " RESULT : ✅ PARITY — both runs executed exactly the same $COUNT_A tests"
  echo "════════════════════════════════════════════════════════════════════"
  exit 0
else
  echo " RESULT : ❌ MISMATCH — the runs did not execute the same test set"
  echo "════════════════════════════════════════════════════════════════════"
  exit 1
fi
