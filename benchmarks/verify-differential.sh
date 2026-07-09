#!/usr/bin/env bash
# LOGICAFFEINE — differential-correctness gate over the benchmark corpus
#
# For every benchmark program, run it BOTH compiled (native binary) and
# interpreted (largo run --interpret) at a small size and assert the output is
# identical. The interpreter is the reference for LOGOS reference semantics;
# codegen must match it. This catches the class of bug where a codegen
# optimization silently changes a program's meaning (the unsound double-buffer
# swap, the aliased-SetIndex double-borrow) — which "passing against C" did not.
#
# Exits non-zero on ANY disagreement, so it can gate CI.
#
# Usage: bash benchmarks/verify-differential.sh
#   ONLY=fib,knapsack   Subset to check.
#   SIZE_<bench>=N      Per-benchmark size override (default: a small per-bench size).

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
PROGRAMS_DIR="$SCRIPT_DIR/programs"

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

# Small, interpreter-friendly sizes — correctness doesn't need scale, and the
# interpreter is slow. Exponential benchmarks (fib/ackermann/nqueens) stay tiny.
small_size() {
    case "$1" in
        fib) echo 12 ;; ackermann) echo 2 ;; nqueens) echo 6 ;;
        bubble_sort) echo 60 ;; mergesort) echo 200 ;; quicksort) echo 200 ;;
        counting_sort) echo 500 ;; heap_sort) echo 200 ;;
        nbody) echo 50 ;; mandelbrot) echo 20 ;; spectral_norm) echo 20 ;;
        pi_leibniz) echo 5000 ;;
        gcd) echo 50 ;; collatz) echo 2000 ;; primes) echo 500 ;;
        sieve) echo 5000 ;; matrix_mult) echo 12 ;; prefix_sum) echo 5000 ;;
        array_reverse) echo 5000 ;; array_fill) echo 5000 ;;
        collect) echo 2000 ;; two_sum) echo 200 ;; histogram) echo 5000 ;;
        knapsack) echo 20 ;; coins) echo 2000 ;;
        fannkuch) echo 6 ;;
        strings) echo 500 ;; binary_trees) echo 8 ;;
        loop_sum) echo 5000 ;; fib_iterative) echo 5000 ;;
        graph_bfs) echo 100 ;; string_search) echo 2000 ;;
        *) echo 100 ;;
    esac
}

bench_size() { local v="SIZE_${1}"; echo "${!v:-$(small_size "$1")}"; }

if [ -n "${ONLY:-}" ]; then
    IFS=',' read -ra BENCHMARKS <<< "$ONLY"
fi

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info(){ echo -e "${CYAN}[INFO]${NC} $*"; }
ok(){ echo -e "${GREEN}[OK]${NC} $*"; }
warn(){ echo -e "${YELLOW}[WARN]${NC} $*"; }
fail(){ echo -e "${RED}[FAIL]${NC} $*"; }

export LOGOS_WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
export CARGO_TARGET_DIR="$SCRIPT_DIR/.logos-bench-target"

info "Building largo (release)..."
cargo build -p logicaffeine-cli --release --manifest-path "$SCRIPT_DIR/../Cargo.toml" 2>/dev/null
LARGO="$CARGO_TARGET_DIR/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$CARGO_TARGET_DIR/release/largo"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/largo"
if [ ! -f "$LARGO" ]; then fail "no largo binary"; exit 1; fi
ok "largo built"

TMP_BASE=$(mktemp -d); trap 'rm -rf "$TMP_BASE"' EXIT
MISMATCH=0; CHECKED=0

for bench in "${BENCHMARKS[@]}"; do
    lg="$PROGRAMS_DIR/$bench/main.lg"
    [ -f "$lg" ] || { warn "$bench: no main.lg"; continue; }
    size="$(bench_size "$bench")"

    d="$TMP_BASE/$bench"; mkdir -p "$d/src"; cp "$lg" "$d/src/main.lg"
    printf '[package]\nname = "bench"\nversion = "0.1.0"\nentry = "src/main.lg"\n' > "$d/Largo.toml"

    interp=$( cd "$d" && timeout 120 "$LARGO" run --interpret "$size" 2>/dev/null | tr -d '[:space:]' )
    ( cd "$d" && CARGO_TARGET_DIR="$d/target" "$LARGO" build --release >/dev/null 2>&1 )
    bin=$(find "$d/target/release" -maxdepth 1 -name bench -type f 2>/dev/null | head -1)
    if [ -z "$bin" ]; then fail "$bench: compile failed"; MISMATCH=$((MISMATCH+1)); continue; fi
    compiled=$( timeout 120 "$bin" "$size" 2>/dev/null | tr -d '[:space:]' )

    CHECKED=$((CHECKED+1))
    if [ "$compiled" = "$interp" ]; then
        ok "$bench (n=$size): $compiled"
    else
        fail "$bench (n=$size): compiled='$compiled' interpreted='$interp'"
        MISMATCH=$((MISMATCH+1))
    fi
done

echo
if [ "$MISMATCH" -gt 0 ]; then
    fail "Differential gate FAILED: $MISMATCH/$CHECKED benchmarks disagree (compiled != interpreted)"
    exit 1
fi
ok "Differential gate PASSED: all $CHECKED benchmarks agree (compiled == interpreted)"
