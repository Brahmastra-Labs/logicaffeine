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

echo "Cross-language benchmark verification"
echo "======================================"
echo ""

for bench in fib sieve collect strings bubble_sort ackermann; do
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

        # Java
        [ -d "bin/java/$bench" ] && check "$bench" "Java" "$size" "$expected" java -cp "bin/java/$bench" Main "$size" || skip

        # JavaScript
        [ -f "programs/$bench/main.js" ] && check "$bench" "JavaScript" "$size" "$expected" node "programs/$bench/main.js" "$size" || skip

        # Python (skip very large sizes — too slow)
        if [ -f "programs/$bench/main.py" ]; then
            case "$bench" in
                fib)
                    if [ "$size" -le 35 ]; then
                        check "$bench" "Python" "$size" "$expected" python3 "programs/$bench/main.py" "$size"
                    else skip; fi ;;
                bubble_sort)
                    if [ "$size" -le 5000 ]; then
                        check "$bench" "Python" "$size" "$expected" python3 "programs/$bench/main.py" "$size"
                    else skip; fi ;;
                ackermann)
                    if [ "$size" -le 10 ]; then
                        check "$bench" "Python" "$size" "$expected" python3 "programs/$bench/main.py" "$size"
                    else skip; fi ;;
                *)
                    check "$bench" "Python" "$size" "$expected" python3 "programs/$bench/main.py" "$size" ;;
            esac
        fi

        # Ruby (skip very large sizes — too slow)
        if [ -f "programs/$bench/main.rb" ]; then
            case "$bench" in
                fib)
                    if [ "$size" -le 35 ]; then
                        check "$bench" "Ruby" "$size" "$expected" ruby "programs/$bench/main.rb" "$size"
                    else skip; fi ;;
                bubble_sort)
                    if [ "$size" -le 5000 ]; then
                        check "$bench" "Ruby" "$size" "$expected" ruby "programs/$bench/main.rb" "$size"
                    else skip; fi ;;
                ackermann)
                    if [ "$size" -le 10 ]; then
                        check "$bench" "Ruby" "$size" "$expected" ruby "programs/$bench/main.rb" "$size"
                    else skip; fi ;;
                *)
                    check "$bench" "Ruby" "$size" "$expected" ruby "programs/$bench/main.rb" "$size" ;;
            esac
        fi

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
