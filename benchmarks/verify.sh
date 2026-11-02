#!/usr/bin/env bash
# Cross-language verification: every benchmark, at every size in its sizes.txt, in every implementation the suite
# registers (benchmarks/lib/languages.tsv, built by suite_build — a binary run.sh or another script already built
# with the same command is reused) plus Python and Ruby up to the benchmark's slow_size, each compared byte for byte
# with expected_<n>.txt. A size an implementation cannot run (lib/limits.tsv) is counted as skipped, never as passed;
# an implementation that does not build fails. Exits non-zero if any implementation disagrees.
#
# Usage: bash benchmarks/verify.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

. "$SCRIPT_DIR/lib/suite.sh"
largo_build || exit 1

PASS=0
FAIL=0
SKIP=0

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# check <bench> <label> <size> <expected-file> <command…>
check() {
    local bench="$1" label="$2" size="$3" expected="$4" got
    shift 4
    got=$(mktemp)
    if timeout 300 "$@" > "$got" 2>/dev/null && cmp -s "$got" "$expected"; then
        PASS=$((PASS + 1))
    else
        echo -e "${RED}FAIL${NC}: $bench/$label @ n=$size — got [$(head -c 80 "$got" | od -An -c | tr -s ' ')] expected [$(head -c 80 "$expected" | od -An -c | tr -s ' ')]"
        FAIL=$((FAIL + 1))
    fi
    rm -f "$got"
}

echo "Cross-language benchmark verification"
echo "======================================"

declare -A BUILT=()
for bench in "${BENCHMARKS[@]}"; do
    echo "--- $bench ---"
    for id in "${LANG_IDS[@]}"; do
        suite_has "$bench" "$id" || continue
        log=$(mktemp)
        if suite_build "$bench" "$id" > "$log" 2>&1; then
            BUILT["$bench $id"]=1
        else
            echo -e "${RED}FAIL${NC}: $bench/${LANG_LABEL[$id]} does not build:"; sed 's/^/    /' "$log"
            FAIL=$((FAIL + 1))
        fi
        rm -f "$log"
    done
    for size in $(tr -s ' \n' '\n' < "programs/$bench/sizes.txt" | sed '/^$/d'); do
        expected="programs/$bench/expected_${size}.txt"
        if [ ! -f "$expected" ]; then
            echo -e "${RED}FAIL${NC}: $bench has no expected output at n=$size"
            FAIL=$((FAIL + 1))
            continue
        fi
        for id in "${LANG_IDS[@]}"; do
            suite_has "$bench" "$id" || continue
            if suite_over_limit "$bench" "$id" "$size"; then SKIP=$((SKIP + 1)); continue; fi
            [ -n "${BUILT["$bench $id"]:-}" ] || continue
            check "$bench" "${LANG_LABEL[$id]}" "$size" "$expected" bash -c "$(suite_run_cmd "$bench" "$id" "$size")"
        done
        for pair in "python:python3:main.py" "ruby:ruby:main.rb"; do
            IFS=: read -r label interp file <<< "$pair"
            [ -f "programs/$bench/$file" ] || continue
            if [ "$size" -le "${BENCH_SLOW_SIZE[$bench]}" ]; then
                check "$bench" "$label" "$size" "$expected" "$interp" "programs/$bench/$file" "$size"
            else
                SKIP=$((SKIP + 1))
            fi
        done
    done
done

echo "======================================"
echo -e "Results: ${GREEN}${PASS} passed${NC}, ${RED}${FAIL} failed${NC}, ${YELLOW}${SKIP} skipped${NC}"
[ "$FAIL" -eq 0 ]
