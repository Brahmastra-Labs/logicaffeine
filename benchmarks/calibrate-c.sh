#!/usr/bin/env bash
# LOGICAFFEINE — C runtime calibrator
#
# For every benchmark: start at N=1 and count up, timing the C binary at each
# step. Record the largest N whose runtime is still UNDER each target (100ms,
# 500ms, 1000ms) — always the closest N *under*, never over.
#
# The step adapts in two modes, so it is both fast and exact:
#   - Counting up, too slow: if BUDGET_S seconds pass without reaching the next
#     target, "count faster" — multiply the step by ACCEL. This ramps from N=1
#     to billions in seconds, so each benchmark discovers its own scale (no
#     growth rate to guess).
#   - Overshot the target: "count slower" — back up to the last under-N and
#     divide the step by ACCEL. Repeating this shrinks the step back to 1, which
#     pins the exact largest N still under the target.
# Accelerate to find the scale; decelerate to nail the boundary.
#
# Every run is timed out at OVERSHOOT × the current target (e.g. 200ms while
# seeking 100ms). A run that blows past that is, by definition, well over the
# target — so it is killed immediately and counts as "over". This both keeps an
# exploding benchmark (fib, ackermann, fannkuch) from ever hanging the sweep and
# makes the fast-counting overshoots cost almost nothing to detect.
#
# Output:
#   - A table to stdout (and logs/optimization/calibrate-c.log).
#   - results/calibrated-sizes.json — consumed by run-logos-vs-c.sh, which
#     picks each benchmark's N from it (CALIBRATION_TARGET=100|500|1000).
#
# Usage: bash benchmarks/calibrate-c.sh
#
# Environment knobs:
#   ONLY=fib,sieve     Comma-separated subset to calibrate.
#   TARGETS=100,500,1000   Target C runtimes in ms (ascending).
#   START=1            Smallest N to probe first.
#   BUDGET_S=1         Seconds of crawling toward a target before counting faster.
#   ACCEL=8            Step is multiplied by this when too slow, divided by it on
#                      overshoot.
#   NCAP=2000000000    Hard ceiling on N (keeps int-typed programs in range).
#   OVERSHOOT=2        A run is killed once it exceeds OVERSHOOT × the target
#                      time; that run counts as "over". 2 = "more than double".
#   TOL=0.01           A result within this of its target is flagged "ok".
#   FORCE_BUILD=1      Rebuild every C binary even if cached.
#   OUT=results/calibrated-sizes.json   Output path (relative to benchmarks/).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="$SCRIPT_DIR/bin"
PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"

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

START="${START:-1}"
BUDGET_S="${BUDGET_S:-1}"
ACCEL="${ACCEL:-8}"
NCAP="${NCAP:-2000000000}"
OVERSHOOT="${OVERSHOOT:-2}"
TOL="${TOL:-0.01}"
OUT="${OUT:-results/calibrated-sizes.json}"

IFS=',' read -ra TARGETS <<< "${TARGETS:-100,500,1000}"

LOG_DIR="$SCRIPT_DIR/../logs/optimization"
mkdir -p "$LOG_DIR"
exec > >(tee "$LOG_DIR/calibrate-c.log") 2>&1

if [ -n "${ONLY:-}" ]; then
    SELECTED=()
    IFS=',' read -ra WANTED <<< "$ONLY"
    for want in "${WANTED[@]}"; do
        found=false
        for b in "${BENCHMARKS[@]}"; do
            [ "$b" = "$want" ] && found=true && break
        done
        if [ "$found" = true ]; then
            SELECTED+=("$want")
        else
            echo "Unknown benchmark in ONLY: '$want'" >&2
            exit 1
        fi
    done
    BENCHMARKS=("${SELECTED[@]}")
fi

mkdir -p "$BIN_DIR" "$RESULTS_DIR"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

# within TOL of the target (relative)
within_tol() { awk -v t="$1" -v T="$2" -v tol="$TOL" 'BEGIN{exit !((t>T?t-T:T-t)/T <= tol)}'; }

# --- measure BIN N TIMEOUT_S -> milliseconds for one run, or a sentinel:
#   __OVER__  the run hit the timeout (so it is well past the target)
#   __ERR__   the binary failed at this N (e.g. too small) — keep crawling
measure() {
    local bin="$1" n="$2" to="$3" rc=0 start end
    start=$(date +%s.%N)
    timeout "$to" "$bin" "$n" >/dev/null 2>&1 || rc=$?
    end=$(date +%s.%N)
    if [ "$rc" -eq 124 ]; then echo "__OVER__"; return; fi
    if [ "$rc" -ne 0 ]; then echo "__ERR__"; return; fi
    awk -v s="$start" -v e="$end" 'BEGIN{ printf "%.3f", (e - s) * 1000 }'
}

# timeout in seconds for the run while seeking target T (ms): OVERSHOOT × T,
# floored so process startup never trips it.
target_timeout() {
    awk -v T="$1" -v f="$OVERSHOOT" 'BEGIN{ s=T*f/1000; if (s<0.05) s=0.05; printf "%.3f", s }'
}

# --- build C (mirror run-logos-vs-c.sh: gcc -O2 ... -lm) ----------------------
c_cached() {
    [ "${FORCE_BUILD:-}" = "1" ] && return 1
    [ -f "$BIN_DIR/${1}_c" ] && [ "$BIN_DIR/${1}_c" -nt "$PROGRAMS_DIR/$1/main.c" ]
}
build_c() {
    local bench="$1"
    if c_cached "$bench"; then ok "  C (cached)"; return 0; fi
    if [ ! -f "$PROGRAMS_DIR/$bench/main.c" ]; then warn "  no main.c for $bench"; return 1; fi
    if gcc -O2 -o "$BIN_DIR/${bench}_c" "$PROGRAMS_DIR/$bench/main.c" -lm 2>/dev/null; then
        ok "  C built"; return 0
    fi
    fail "  C build failed for $bench"; return 1
}

# --- core: crawl one benchmark ----------------------------------------------
declare -A RES_N RES_T RES_STATUS

record() {  # bench target n t status
    RES_N["$1:$2"]="$3"; RES_T["$1:$2"]="$4"; RES_STATUS["$1:$2"]="$5"
}

close_target() {  # bench ti-target prev_n prev_t fallback_n fallback_t
    local bench="$1" target="$2" pn="$3" pt="$4" fn="$5" ft="$6"
    if [ -n "$pn" ]; then
        record "$bench" "$target" "$pn" "$pt" \
            "$(within_tol "$pt" "$target" && echo ok || echo under)"
    else
        record "$bench" "$target" "$fn" "$ft" over_at_min
    fi
    ok "  $bench @ ${target}ms -> n=${RES_N["$bench:$target"]} (${RES_T["$bench:$target"]%.*}ms, ${RES_STATUS["$bench:$target"]})"
}

calibrate() {
    local bench="$1"
    local bin="$BIN_DIR/${bench}_c"
    local ti=0 prev_n="" prev_t="" n="$START" step=1
    local phase_start=$SECONDS

    while [ "$ti" -lt "${#TARGETS[@]}" ] && [ "$n" -ge 1 ]; do
        local target="${TARGETS[$ti]}"
        local t; t=$(measure "$bin" "$n" "$(target_timeout "$target")")

        if [ "$t" = "__ERR__" ]; then
            n=$((n + step)); continue
        fi

        # Over the target? (timeout counts as over.) Two responses:
        #   step > 1  -> overshot while counting fast: back up and count slower.
        #   step == 1 -> this is the boundary: prev_n is the closest under.
        if [ "$t" = "__OVER__" ] || awk -v t="$t" -v T="$target" 'BEGIN{exit !(t>=T)}'; then
            if [ "$step" -gt 1 ] && [ -n "$prev_n" ]; then
                step=$((step / ACCEL)); [ "$step" -lt 1 ] && step=1
                n=$((prev_n + step))
                phase_start=$SECONDS
                continue
            fi
            close_target "$bench" "$target" "$prev_n" "$prev_t" "$n" "$t"
            ti=$((ti + 1))
            step=1; phase_start=$SECONDS
            [ -n "$prev_n" ] && n=$((prev_n + 1)) || n=$((n + 1))
            continue
        fi

        # Under the target — remember it, then advance, counting faster if the
        # climb toward this target is dragging.
        prev_n="$n"; prev_t="$t"
        if [ "$n" -ge "$NCAP" ]; then
            break          # reached the ceiling and still under — unreachable
        fi
        if [ $((SECONDS - phase_start)) -ge "$BUDGET_S" ]; then
            step=$((step * ACCEL)); phase_start=$SECONDS
            info "    $bench: counting faster — step=$step (n=$n, ${t%.*}ms)"
        fi
        n=$((n + step))
        [ "$n" -gt "$NCAP" ] && n="$NCAP"   # probe the ceiling, never leap past it
    done

    # Ran out of room (NCAP) before reaching a remaining target.
    local rem
    for rem in "${TARGETS[@]:$ti}"; do
        record "$bench" "$rem" "$prev_n" "$prev_t" unreached
        warn "  $bench @ ${rem}ms: unreached (max ${prev_t%.*}ms at n=$prev_n)"
    done
}

# --- run ---------------------------------------------------------------------
detect_cpu() {
    if [ -f /proc/cpuinfo ]; then
        grep -m1 'model name' /proc/cpuinfo 2>/dev/null | sed 's/.*: //' || uname -m
    else uname -m; fi
}
detect_os() {
    if [ -f /etc/os-release ]; then
        local pretty; pretty=$(grep -m1 PRETTY_NAME /etc/os-release 2>/dev/null | sed 's/PRETTY_NAME="\?\([^"]*\)"\?/\1/')
        echo "${pretty:-Linux} $(uname -m)"
    else echo "$(uname -s) $(uname -r) $(uname -m)"; fi
}

info "Phase 1: building C binaries..."
RUNNABLE=()
for bench in "${BENCHMARKS[@]}"; do
    info "Building $bench..."
    build_c "$bench" && RUNNABLE+=("$bench")
done
ok "Phase 1 complete"

info "Phase 2: crawling C runtimes to targets: ${TARGETS[*]} ms (BUDGET_S=$BUDGET_S, ACCEL=$ACCEL)..."
for bench in "${RUNNABLE[@]}"; do
    info "Calibrating $bench..."
    calibrate "$bench"
done
ok "Phase 2 complete"

# --- Phase 3: assemble JSON --------------------------------------------------
info "Phase 3: writing $OUT..."
TARGETS_JSON=$(printf '%s\n' "${TARGETS[@]}" | jq -R 'tonumber' | jq -s '.')
BENCH_JSON="{}"
for bench in "${RUNNABLE[@]}"; do
    entry="{}"
    for target in "${TARGETS[@]}"; do
        n="${RES_N["$bench:$target"]:-}"
        t="${RES_T["$bench:$target"]:-}"
        status="${RES_STATUS["$bench:$target"]:-error}"
        [ -z "$n" ] && continue
        entry=$(jq -n --argjson cur "$entry" --arg target "$target" \
            --argjson n "$n" --argjson cms "${t:-0}" --argjson tms "$target" --arg status "$status" \
            '$cur + {($target): {n: $n, c_ms: $cms, target_ms: $tms, status: $status}}')
    done
    BENCH_JSON=$(jq -n --argjson cur "$BENCH_JSON" --arg b "$bench" --argjson e "$entry" '$cur + {($b): $e}')
done

mkdir -p "$(dirname "$OUT")"
jq -n \
    --arg date "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
    --arg cpu "$(detect_cpu)" --arg os "$(detect_os)" \
    --arg gcc "$(gcc --version 2>/dev/null | head -1 || echo unknown)" \
    --argjson targets "$TARGETS_JSON" --argjson tol "$TOL" \
    --argjson budget_s "$BUDGET_S" --argjson accel "$ACCEL" --argjson overshoot "$OVERSHOOT" \
    --argjson benchmarks "$BENCH_JSON" \
    '{
        meta: { date: $date, cpu: $cpu, os: $os, gcc: $gcc,
                targets_ms: $targets, tol: $tol,
                budget_s: $budget_s, accel: $accel, overshoot: $overshoot },
        benchmarks: $benchmarks
    }' > "$OUT"
ok "Wrote $OUT"

# --- Phase 4: table ----------------------------------------------------------
echo
info "Calibrated C sizes (largest n still under each target runtime):"
echo
header=$(printf "%-14s" "benchmark")
for target in "${TARGETS[@]}"; do header+=$(printf " %16s" "n@${target}ms (ms)"); done
echo "$header"
printf '%.0s-' $(seq 1 $((14 + 17 * ${#TARGETS[@]}))); echo
for bench in "${RUNNABLE[@]}"; do
    row=$(printf "%-14s" "$bench")
    for target in "${TARGETS[@]}"; do
        n="${RES_N["$bench:$target"]:-}"; t="${RES_T["$bench:$target"]:-}"
        if [ -n "$n" ]; then cell=$(printf "%s (%.0f)" "$n" "${t:-0}"); else cell="—"; fi
        row+=$(printf " %16s" "$cell")
    done
    echo "$row"
done

echo
info "Full log: logs/optimization/calibrate-c.log"
info "Feed run-logos-vs-c.sh with: CALIBRATION_TARGET=500 bash benchmarks/run-logos-vs-c.sh"
ok "Calibration complete!"
