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

# The benchmark list is the suite registry (benchmarks/lib/benchmarks.tsv), loaded here with the rest of it.
. "$SCRIPT_DIR/lib/suite.sh"

# Small, interpreter-friendly sizes — correctness doesn't need scale, and the
# interpreter is slow. Exponential benchmarks (fib/ackermann/nqueens) stay tiny.
small_size() {
    echo "${BENCH_SMALL_SIZE[$1]}"
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
. "$SCRIPT_DIR/lib/largo.sh"
largo_build || exit 1
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
