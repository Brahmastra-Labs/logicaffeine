#!/usr/bin/env bash
# LOGICAFFEINE — per-benchmark interpreter tier/timing census
#
# A fast diagnostic sweep the orchestrator runs after each optimization wave. For
# every benchmark in programs/* it runs the release `largo run --interpret` once
# and reports three things side by side:
#
#   - wall   : wall-clock time of the run (via /usr/bin/time), in ms.
#   - engine : which interpreter engine ran (LOGOS_ENGINE_TRACE=1 ->
#              `logos-engine: vm+jit`, or a tree-walker fallback). A silent
#              fallback to the tree-walker is never mistaken for the JIT.
#   - tiers  : region tier-up outcomes (LOGOS_RDIAG=1) — a count of `adapt OK`
#              (regions that tiered to native) vs `RDIAG-BAIL` (regions the JIT
#              refused), plus the single most common bail reason. This is the
#              "does the hot loop actually JIT?" signal that drives the campaign.
#
# This is diagnosis, NOT the authoritative geomean benchmark (the orchestrator
# owns that on a quiet box). It does a single timed run per bench, no warmup, no
# statistics — it is meant to be cheap and quick to read between waves.
#
# Usage:
#   bash benchmarks/census.sh                 # every bench at the default size
#   ONLY=fib,histogram bash benchmarks/census.sh
#   SIZE=200000 bash benchmarks/census.sh     # one fixed size for all benches
#   SIZE_fib=35 bash benchmarks/census.sh     # per-bench size override
#   CALIBRATED=1 bash benchmarks/census.sh    # size from calibrated-interp-sizes.json
#
# Environment knobs:
#   ONLY=fib,sieve   Comma-separated subset of benchmarks (default: all).
#   SIZE=N           Default N for every bench (default: 5000 — small + quick).
#   SIZE_<bench>=N   Per-benchmark size override (highest precedence).
#   CALIBRATED=1     Pull each bench's N from the calibrated file (heavier).
#   CALIBRATION_TARGET=250   Which calibrated target to read when CALIBRATED=1.
#   TIMEOUT=30       Per-run wall-clock timeout, seconds.
#   RDIAG_HEAD=0     If >0, also print that many raw RDIAG lines per bench.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
CALIBRATED_FILE="${CALIBRATED_FILE:-$RESULTS_DIR/calibrated-interp-sizes.json}"
CALIBRATION_TARGET="${CALIBRATION_TARGET:-250}"

DEFAULT_SIZE="${SIZE:-5000}"
TIMEOUT="${TIMEOUT:-30}"
RDIAG_HEAD="${RDIAG_HEAD:-0}"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info() { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail() { echo -e "${RED}[FAIL]${NC} $*"; }

if command -v gtimeout &>/dev/null; then RUN_TO() { gtimeout "$@"; }
elif command -v timeout &>/dev/null; then RUN_TO() { timeout "$@"; }
else RUN_TO() { shift; "$@"; }; fi

# Mirror the per-bench structural ceiling enforced by run-interp-vs-js.sh so the
# census never drives ackermann past MAX_CALL_DEPTH (depth ~2^(n+3)-3 > 2500 at
# n=9). A diagnostic run that stack-overflows reports nothing useful.
bench_maxn() {
    local override_var="MAXN_${1}"
    if [ -n "${!override_var:-}" ]; then echo "${!override_var}"; return; fi
    case "$1" in
        ackermann) echo 8 ;;
        *) echo "" ;;
    esac
}

bench_size() {
    local bench="$1" sz="" maxn
    local override_var="SIZE_${bench}"
    if [ -n "${!override_var:-}" ]; then
        sz="${!override_var}"
    elif [ "${CALIBRATED:-0}" = "1" ] && [ -f "$CALIBRATED_FILE" ]; then
        sz=$(jq -r --arg b "$bench" --arg t "$CALIBRATION_TARGET" \
            '.benchmarks[$b][$t].n // empty' "$CALIBRATED_FILE" 2>/dev/null) || sz=""
        [ -n "$sz" ] || sz="$DEFAULT_SIZE"
    else
        sz="$DEFAULT_SIZE"
    fi
    maxn="$(bench_maxn "$bench")"
    if [ -n "$maxn" ] && [ -n "$sz" ] && [ "$sz" -gt "$maxn" ]; then sz="$maxn"; fi
    echo "$sz"
}

LARGO="$SCRIPT_DIR/.logos-bench-target/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/.logos-bench-target/release/largo"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/largo"
if [ ! -f "$LARGO" ]; then
    fail "largo release binary not found — build it first: cargo build -p logicaffeine-cli --release"
    exit 1
fi
export LOGOS_WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"

BENCHMARKS=()
for d in "$PROGRAMS_DIR"/*/; do
    b="$(basename "$d")"
    [ -f "$d/main.lg" ] && BENCHMARKS+=("$b")
done

if [ -n "${ONLY:-}" ]; then
    SELECTED=()
    IFS=',' read -ra WANTED <<< "$ONLY"
    for want in "${WANTED[@]}"; do
        found=false
        for b in "${BENCHMARKS[@]}"; do [ "$b" = "$want" ] && found=true && break; done
        if [ "$found" = true ]; then SELECTED+=("$want")
        else fail "Unknown benchmark in ONLY: '$want'"; exit 1; fi
    done
    BENCHMARKS=("${SELECTED[@]}")
fi

# One temp Largo project per bench, identical to the run/calibrate harness setup.
TMP_BASE=$(mktemp -d)
trap 'rm -rf "$TMP_BASE"' EXIT
setup_project() {
    local bench="$1" d="$TMP_BASE/$bench"
    mkdir -p "$d/src"
    cp "$PROGRAMS_DIR/$bench/main.lg" "$d/src/main.lg"
    cat > "$d/Largo.toml" << 'TOML'
[package]
name = "bench"
version = "0.1.0"
entry = "src/main.lg"
TOML
    echo "$d"
}

# milliseconds of one interpreter run (wall clock), or a sentinel:
#   __OVER__  hit the timeout    __ERR__  exited non-zero (e.g. stack overflow)
run_wall_ms() {
    local proj="$1" n="$2" rc=0 start end
    start=$(date +%s.%N)
    ( cd "$proj" && RUN_TO "$TIMEOUT" "$LARGO" run --interpret "$n" >/dev/null 2>&1 ) || rc=$?
    end=$(date +%s.%N)
    if [ "$rc" -eq 124 ] || [ "$rc" -eq 143 ]; then echo "__OVER__"; return; fi
    if [ "$rc" -ne 0 ]; then echo "__ERR__"; return; fi
    awk -v s="$start" -v e="$end" 'BEGIN{ printf "%.1f", (e - s) * 1000 }'
}

detect_engine() {
    local proj="$1" line
    line=$( cd "$proj" && LOGOS_ENGINE_TRACE=1 RUN_TO "$TIMEOUT" "$LARGO" run --interpret 1 2>&1 >/dev/null \
            | grep -m1 'logos-engine:' ) || true
    line="${line#logos-engine: }"
    [ -n "$line" ] && echo "$line" || echo "?"
}

echo
info "Per-benchmark interpreter census (single timed run, no warmup/stats)"
info "largo: $LARGO"
[ "${CALIBRATED:-0}" = "1" ] && info "sizes: calibrated @ ${CALIBRATION_TARGET}ms" \
                             || info "sizes: SIZE=${DEFAULT_SIZE} (override per bench with SIZE_<bench>)"
echo
printf "%-14s %9s %10s %7s %7s  %-s\n" "benchmark" "n" "wall(ms)" "tierOK" "bail" "engine / top bail"
printf '%.0s-' $(seq 1 92); echo

declare -A T_OK T_BAIL ENGINE WALL TOPBAIL
for bench in "${BENCHMARKS[@]}"; do
    proj="$(setup_project "$bench")"
    n="$(bench_size "$bench")"

    ENGINE[$bench]="$(detect_engine "$proj")"
    WALL[$bench]="$(run_wall_ms "$proj" "$n")"

    rdiag=$( cd "$proj" && LOGOS_RDIAG=1 RUN_TO "$TIMEOUT" "$LARGO" run --interpret "$n" 2>&1 >/dev/null ) || true
    T_OK[$bench]=$(printf '%s\n' "$rdiag" | grep -c 'adapt OK' || true)
    T_BAIL[$bench]=$(printf '%s\n' "$rdiag" | grep -c 'RDIAG-BAIL' || true)
    # Most common bail reason (the token after `RDIAG-BAIL head_pc=N`, head_pc
    # stripped so reasons aggregate across loops).
    TOPBAIL[$bench]=$(printf '%s\n' "$rdiag" | grep 'RDIAG-BAIL' \
        | sed -E 's/.*RDIAG-BAIL (head_pc=[0-9]+ )?//; s/ (reg|g|head_pc|op|kind|slot)=.*//' \
        | sort | uniq -c | sort -rn | head -1 | sed -E 's/^ *[0-9]+ //' || true)

    note="${ENGINE[$bench]}"
    [ "${T_BAIL[$bench]}" -gt 0 ] && [ -n "${TOPBAIL[$bench]}" ] && note="$note | ${TOPBAIL[$bench]}"
    printf "%-14s %9s %10s %7s %7s  %-s\n" \
        "$bench" "$n" "${WALL[$bench]}" "${T_OK[$bench]}" "${T_BAIL[$bench]}" "$note"
    if [ "$RDIAG_HEAD" -gt 0 ]; then
        printf '%s\n' "$rdiag" | grep 'RDIAG' | head -"$RDIAG_HEAD" | sed 's/^/    /'
    fi
done

echo
total_ok=0; total_bail=0; tw=0
for bench in "${BENCHMARKS[@]}"; do
    total_ok=$(( total_ok + ${T_OK[$bench]:-0} ))
    total_bail=$(( total_bail + ${T_BAIL[$bench]:-0} ))
    [ "${ENGINE[$bench]}" != "vm+jit" ] && [ "${ENGINE[$bench]}" != "?" ] && tw=$(( tw + 1 ))
done
info "Totals: ${total_ok} regions tiered (adapt OK), ${total_bail} regions bailed (RDIAG-BAIL)"
[ "$tw" -gt 0 ] && warn "$tw benchmark(s) did NOT run on vm+jit (tree-walker fallback) — check the engine column"
info "wall=__OVER__ timed out (TIMEOUT=${TIMEOUT}s); wall=__ERR__ exited non-zero (e.g. stack overflow)"
ok "Census complete"
