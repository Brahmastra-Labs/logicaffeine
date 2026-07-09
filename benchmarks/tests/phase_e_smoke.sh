#!/usr/bin/env bash
# Smoke test for the Phase E memory + big-O harness functions in run.sh. Extracts
# the pure functions under test (run.sh is a pipeline script, not a sourceable
# library) and asserts their behavior without running the multi-minute benchmark.
set -euo pipefail

RUN="$(cd "$(dirname "$0")/.." && pwd)/run.sh"
eval "$(sed -n '/^measure_memory()/,/^}/p; /^bench_complexity()/,/^}/p; /^bench_complexity_json()/,/^}/p' "$RUN")"

fail() { echo "FAIL: $*"; exit 1; }

# measure_memory returns a positive integer kB for a real command.
kb=$(measure_memory "/bin/echo hi")
[[ "$kb" =~ ^[0-9]+$ ]] && [ "$kb" -gt 0 ] || fail "measure_memory not a positive int: '$kb'"

# ...and the JSON `null` for a command that cannot run (graceful, never crashes).
nn=$(measure_memory "/nonexistent/binary-xyz")
[ "$nn" = "null" ] || fail "measure_memory should be 'null' on failure, got '$nn'"

# Every benchmark has a declared (time, space) complexity — no '?' fallbacks.
for b in fib ackermann nqueens bubble_sort mergesort quicksort counting_sort heap_sort \
         nbody mandelbrot spectral_norm pi_leibniz gcd collatz primes sieve matrix_mult \
         prefix_sum array_reverse array_fill collect two_sum histogram knapsack coins \
         fannkuch strings binary_trees loop_sum fib_iterative graph_bfs string_search; do
    ct=$(bench_complexity "$b")
    [ "$ct" != $'?\t?' ] || fail "$b has no declared complexity"
done

# The complexity JSON is well-formed with both fields.
bench_complexity_json mergesort | jq -e '.time and .space' >/dev/null || fail "complexity JSON malformed"

echo "PHASE E SMOKE: OK"
