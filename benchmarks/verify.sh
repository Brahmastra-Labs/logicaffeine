#!/usr/bin/env bash
# Comprehensive cross-language verification.
# Tests every benchmark at every listed size across all available languages.
# Exits with non-zero if any implementation disagrees with expected output.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PASS=0
FAIL=0
SKIP=0
TOTAL=0

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

check() {
    local bench="$1" lang="$2" size="$3" expected="$4"
    shift 4
    TOTAL=$((TOTAL + 1))
    local output
    output=$("$@" 2>/dev/null | tr -d '[:space:]') || output="ERROR"
    if [ "$output" = "$expected" ]; then
        PASS=$((PASS + 1))
    else
        echo -e "${RED}FAIL${NC}: $bench/$lang @ n=$size — got '$output', expected '$expected'"
        FAIL=$((FAIL + 1))
    fi
}

skip() {
    SKIP=$((SKIP + 1))
}

BENCHMARKS=(
    fib ackermann nqueens
    bubble_sort mergesort quicksort counting_sort heap_sort
    nbody mandelbrot spectral_norm pi_leibniz
    gcd collatz primes
    sieve matrix_mult prefix_sum array_reverse array_fill
    collect two_sum histogram
    knapsack coins
    fannkuch
    strings binary_trees
    loop_sum fib_iterative graph_bfs string_search
)

# Max sizes for slow languages (Python, Ruby) per benchmark
max_slow_size() {
    case "$1" in
        fib) echo 35 ;; ackermann) echo 10 ;; nqueens) echo 11 ;;
        bubble_sort) echo 5000 ;; mergesort) echo 10000 ;; quicksort) echo 10000 ;;
        counting_sort) echo 100000 ;; heap_sort) echo 10000 ;;
        nbody) echo 5000 ;; mandelbrot) echo 200 ;; spectral_norm) echo 500 ;; pi_leibniz) echo 1000000 ;;
        gcd) echo 2000 ;; collatz) echo 1000000 ;; primes) echo 50000 ;;
        sieve) echo 100000 ;; matrix_mult) echo 100 ;; prefix_sum) echo 1000000 ;;
        array_reverse) echo 1000000 ;; array_fill) echo 1000000 ;;
        collect) echo 50000 ;; two_sum) echo 10000 ;; histogram) echo 1000000 ;;
        knapsack) echo 500 ;; coins) echo 10000 ;;
        fannkuch) echo 9 ;;
        strings) echo 50000 ;; binary_trees) echo 14 ;;
        loop_sum) echo 10000000 ;; fib_iterative) echo 50000000 ;;
        graph_bfs) echo 10000 ;; string_search) echo 100000 ;;
        *) echo 999999999 ;;
    esac
}

echo "Cross-language benchmark verification"
echo "======================================"
echo ""

for bench in "${BENCHMARKS[@]}"; do
    sizes=$(cat "programs/$bench/sizes.txt")
    echo "--- $bench ---"

    for size in $sizes; do
        expected_file="programs/$bench/expected_${size}.txt"
        if [ ! -f "$expected_file" ]; then
            echo "  SKIP: no expected output for $bench @ n=$size"
            continue
        fi
        expected=$(cat "$expected_file" | tr -d '[:space:]')

        # C
        [ -f "bin/${bench}_c" ] && check "$bench" "C" "$size" "$expected" "./bin/${bench}_c" "$size" || skip

        # C++
        [ -f "bin/${bench}_cpp" ] && check "$bench" "C++" "$size" "$expected" "./bin/${bench}_cpp" "$size" || skip

        # Rust
        [ -f "bin/${bench}_rs" ] && check "$bench" "Rust" "$size" "$expected" "./bin/${bench}_rs" "$size" || skip

        # Go
        [ -f "bin/${bench}_go" ] && check "$bench" "Go" "$size" "$expected" "./bin/${bench}_go" "$size" || skip

        # Zig
        [ -f "bin/${bench}_zig" ] && check "$bench" "Zig" "$size" "$expected" "./bin/${bench}_zig" "$size" || skip

        # Java
        [ -d "bin/java/$bench" ] && check "$bench" "Java" "$size" "$expected" java -cp "bin/java/$bench" Main "$size" || skip

        # JavaScript (skip ackermann at large sizes — stack overflow)
        if [ -f "programs/$bench/main.js" ]; then
            if [ "$bench" = "ackermann" ] && [ "$size" -gt 10 ]; then
                skip
            else
                check "$bench" "JavaScript" "$size" "$expected" node "programs/$bench/main.js" "$size"
            fi
        fi

        # Python (skip very large sizes — too slow)
        if [ -f "programs/$bench/main.py" ]; then
            max=$(max_slow_size "$bench")
            if [ "$size" -le "$max" ]; then
                check "$bench" "Python" "$size" "$expected" python3 "programs/$bench/main.py" "$size"
            else
                skip
            fi
        fi

        # Ruby (skip very large sizes — too slow)
        if [ -f "programs/$bench/main.rb" ]; then
            max=$(max_slow_size "$bench")
            if [ "$size" -le "$max" ]; then
                check "$bench" "Ruby" "$size" "$expected" ruby "programs/$bench/main.rb" "$size"
            else
                skip
            fi
        fi

        # Nim
        [ -f "bin/${bench}_nim" ] && check "$bench" "Nim" "$size" "$expected" "./bin/${bench}_nim" "$size" || skip

        # LOGOS (release)
        [ -f "bin/${bench}_logos_release" ] && check "$bench" "LOGOS (release)" "$size" "$expected" "./bin/${bench}_logos_release" "$size" || skip

        # LOGOS (debug)
        [ -f "bin/${bench}_logos_debug" ] && check "$bench" "LOGOS (debug)" "$size" "$expected" "./bin/${bench}_logos_debug" "$size" || skip
    done
    echo ""
done

echo "======================================"
echo -e "Results: ${GREEN}${PASS} passed${NC}, ${RED}${FAIL} failed${NC}, ${YELLOW}${SKIP} skipped${NC} (${TOTAL} total)"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
