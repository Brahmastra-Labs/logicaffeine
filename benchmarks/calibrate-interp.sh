#!/usr/bin/env bash
# LOGICAFFEINE — interpreter (VM + JIT) runtime calibrator
#
# Sizes the LOGOS-interpreter-vs-JavaScript comparison. The LOGOS interpreter
# (largo run --interpret: bytecode VM with copy-and-patch JIT tier-up) and Node
# (V8) are peers — dynamically typed, bytecode VM + JIT — running the SAME naive
# algorithm from each benchmark's main.lg / main.js. We calibrate the SLOWER
# peer (the LOGOS interpreter): pinning it to ~250ms bounds total runtime so
# nothing explodes, and Node then runs faster but never below its ~30ms V8
# startup floor — both stay measurable.
#
# Same adaptive crawl as calibrate-c.sh: from N=1, count up; accelerate
# (multiply the step by ACCEL) when a target is slow to reach, then decelerate
# (divide the step) on overshoot to pin the exact largest N still under. Each
# run is timed out at OVERSHOOT × the target so an exploding benchmark (fib,
# ackermann) never hangs the sweep.
#
# `largo run --interpret N` reads N via args(), so the SAME main.lg the native
# build compiles is fed to the interpreter — no separate source. Only `largo`
# is built, once, in release: release matters because the debug shadow-oracle
# (which re-runs the tree-walker and asserts) is off, so this times the clean
# VM + JIT path.
#
# Output:
#   - A table to stdout (and logs/optimization/calibrate-interp.log).
#   - results/calibrated-interp-sizes.json — consumed by run-interp-vs-js.sh,
#     which picks each benchmark's N from it (CALIBRATION_TARGET=250).
#
# Usage: bash benchmarks/calibrate-interp.sh
#
# Environment knobs (mirror calibrate-c.sh):
#   ONLY=fib,sieve     Comma-separated subset to calibrate.
#   TARGETS=250        Target INTERPRETER runtimes in ms (ascending). Raise it
#                      (e.g. 500/1000) to push Node above its startup floor for
#                      a more compute-dominated comparison.
#   START=1            Smallest N to probe first.
#   BUDGET_S=1         Seconds of crawling toward a target before counting faster.
#   ACCEL=8            Step multiplier when too slow / divisor on overshoot.
#   NCAP=2000000000    Hard ceiling on N.
#   OVERSHOOT=2        A run is killed once it exceeds OVERSHOOT × the target.
#   TOL=0.01           A result within this of its target is flagged "ok".
#   OUT=results/calibrated-interp-sizes.json   Output path (relative to benchmarks/).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

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
OUT="${OUT:-results/calibrated-interp-sizes.json}"

# Per-benchmark hard ceiling on N, independent of the runtime target. Some
# benchmarks have a structural limit the calibration crawl cannot see by timing
# alone: ackermann(3, n) recurses to depth ~A(3,n) = 2^(n+3) - 3, so it trips the
# interpreter's MAX_CALL_DEPTH (2500) at n=9 (depth ~4093) even though n=9 is far
# under the ~250ms time target. Capping it at n=8 (depth ~2045) makes the
# calibrator pin the LARGEST depth-safe size and benchmark it, instead of
# crawling into a stack overflow and dropping the benchmark. Override per bench
# with MAXN_<bench>=N (empty/unset = no cap).
bench_maxn() {
    local override_var="MAXN_${1}"
    if [ -n "${!override_var:-}" ]; then echo "${!override_var}"; return; fi
    case "$1" in
        ackermann) echo 8 ;;
        *) echo "" ;;
    esac
}

IFS=',' read -ra TARGETS <<< "${TARGETS:-250}"

LOG_DIR="$SCRIPT_DIR/../logs/optimization"
mkdir -p "$LOG_DIR"
exec > >(tee "$LOG_DIR/calibrate-interp.log") 2>&1

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

mkdir -p "$RESULTS_DIR"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

# within TOL of the target (relative)
within_tol() { awk -v t="$1" -v T="$2" -v tol="$TOL" 'BEGIN{exit !((t>T?t-T:T-t)/T <= tol)}'; }

# --- measure PROJECT N TIMEOUT_S -> milliseconds for one interpreter run, or a
#   sentinel:
#   __OVER__  the run hit the timeout (so it is well past the target)
#   __ERR__   the program failed at this N (e.g. too small) — keep crawling
measure() {
    local proj="$1" n="$2" to="$3" bench="${4:-}" rc=0 start end
    # First run the interpreter: this both detects whether the program runs at
    # this N (errors -> keep crawling, or bail after many) and bounds runaway
    # programs (timeout -> __OVER__).
    start=$(date +%s.%N)
    ( cd "$proj" && timeout "$to" "$LARGO" run --interpret "$n" >/dev/null 2>&1 ) || rc=$?
    end=$(date +%s.%N)
    if [ "$rc" -eq 124 ]; then echo "__OVER__"; return; fi
    if [ "$rc" -ne 0 ]; then echo "__ERR__"; return; fi
    local i_ms; i_ms=$(awk -v s="$start" -v e="$end" 'BEGIN{ printf "%.3f", (e - s) * 1000 }')
    # Calibrate on NODE so V8 stays off its ~30ms startup floor. The native
    # interpreter has a ~4ms floor, so it never needs floor protection — only
    # Node does. (A single, stable metric, unlike a min-of-both crawl.)
    local js="$PROGRAMS_DIR/$bench/main.js"
    if [ -n "$bench" ] && [ -f "$js" ]; then
        local nrc=0 ns ne
        ns=$(date +%s.%N)
        timeout "$to" node "$js" "$n" >/dev/null 2>&1 || nrc=$?
        ne=$(date +%s.%N)
        if [ "$nrc" -eq 124 ]; then echo "__OVER__"; return; fi
        if [ "$nrc" -eq 0 ]; then
            awk -v s="$ns" -v e="$ne" 'BEGIN{ printf "%.3f", (e - s) * 1000 }'
            return
        fi
    fi
    # No usable Node reference: fall back to the interpreter's own time.
    echo "$i_ms"
}

# timeout in seconds for the run while seeking target T (ms): OVERSHOOT × T,
# floored so interpreter startup never trips it.
target_timeout() {
    awk -v T="$1" -v f="$OVERSHOOT" 'BEGIN{ s=T*f/1000; if (s<0.10) s=0.10; printf "%.3f", s }'
}

# --- cap_runtime_ms PROJECT N BENCH -> interpreter wall time (ms) at a per-bench
# ceiling, or __BADRUN__ if it does not run cleanly there. Unlike measure(), this
# is a CORRECTNESS gate, not a time gate: the cap is a structural ceiling (e.g.
# ackermann's recursion depth), so a slow-but-correct run must still be pinned and
# benchmarked. It uses a generous timeout (MAXN_TIMEOUT_S, default 60s) and, when
# an expected_<n>.txt exists, requires the interpreter's output to match it — so a
# silent stack overflow ("maximum call depth exceeded", which prints to stderr and
# yields no stdout) is caught as __BADRUN__ even though the process may exit 0.
cap_runtime_ms() {
    local proj="$1" n="$2" bench="${3:-}" rc=0 start end out
    local to="${MAXN_TIMEOUT_S:-60}"
    start=$(date +%s.%N)
    out=$( cd "$proj" && timeout "$to" "$LARGO" run --interpret "$n" 2>/dev/null ) || rc=$?
    end=$(date +%s.%N)
    out=$(printf '%s' "$out" | tr -d '[:space:]')
    if [ "$rc" -ne 0 ] || [ -z "$out" ]; then echo "__BADRUN__"; return; fi
    local exp="$PROGRAMS_DIR/$bench/expected_${n}.txt"
    if [ -n "$bench" ] && [ -f "$exp" ]; then
        local want; want=$(cat "$exp" | tr -d '[:space:]')
        [ "$out" = "$want" ] || { echo "__BADRUN__"; return; }
    fi
    awk -v s="$start" -v e="$end" 'BEGIN{ printf "%.3f", (e - s) * 1000 }'
}

# --- one temp Largo project per benchmark, feeding the SAME main.lg the native
# build uses. `largo run --interpret N` reads N via args(), so the program is
# identical to the one Node runs.
TMP_BASE=$(mktemp -d)
trap 'rm -rf "$TMP_BASE"' EXIT
declare -A PROJ
setup_project() {
    local bench="$1"
    if [ ! -f "$PROGRAMS_DIR/$bench/main.lg" ]; then warn "  no main.lg for $bench"; return 1; fi
    local d="$TMP_BASE/$bench"
    mkdir -p "$d/src"
    cp "$PROGRAMS_DIR/$bench/main.lg" "$d/src/main.lg"
    cat > "$d/Largo.toml" << 'TOML'
[package]
name = "bench"
version = "0.1.0"
entry = "src/main.lg"
TOML
    PROJ[$bench]="$d"
    return 0
}

# --- core: crawl one benchmark (identical adaptive logic to calibrate-c.sh) ---
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
    local proj="${PROJ[$bench]}"
    local ti=0 prev_n="" prev_t="" n="$START" step=1 consec_err=0
    local phase_start=$SECONDS
    local maxerr="${MAXERR:-40}"
    local maxn; maxn="$(bench_maxn "$bench")"

    while [ "$ti" -lt "${#TARGETS[@]}" ] && [ "$n" -le "$NCAP" ] && [ "$n" -ge 1 ]; do
        local target="${TARGETS[$ti]}"
        # Per-bench ceiling: a structural limit (e.g. ackermann's recursion depth
        # vs MAX_CALL_DEPTH), not a timing one. At the cap, verify the interpreter
        # RUNS correctly there — then pin it for every remaining target even if it
        # is slower than the time target. The cap is the largest depth-safe size;
        # benchmarking it (slow but real) is the goal, never dropping it.
        if [ -n "$maxn" ] && [ "$n" -ge "$maxn" ]; then
            local ct; ct=$(cap_runtime_ms "$proj" "$maxn" "$bench")
            if [ "$ct" = "__BADRUN__" ]; then
                if [ -n "$prev_n" ]; then
                    warn "  $bench: capped n=$maxn does not run cleanly — pinning largest good n=$prev_n"
                    for rem in "${TARGETS[@]:$ti}"; do
                        record "$bench" "$rem" "$prev_n" "$prev_t" "ceiling"
                        ok "  $bench @ ${rem}ms -> n=$prev_n (${prev_t%.*}ms, ceiling)"
                    done
                else
                    fail "  $bench: capped n=$maxn does not run — skipping"
                    RES_N["$bench:$target"]=""
                fi
                return
            fi
            warn "  $bench: at MAXN cap n=$maxn (depth-bounded) — pinning even if slow (${ct%.*}ms interp)"
            for rem in "${TARGETS[@]:$ti}"; do
                record "$bench" "$rem" "$maxn" "$ct" "capped"
                ok "  $bench @ ${rem}ms -> n=$maxn (${ct%.*}ms, capped)"
            done
            return
        fi
        local t; t=$(measure "$proj" "$n" "$(target_timeout "$target")" "$bench")

        if [ "$t" = "__ERR__" ]; then
            # A handful of errors at tiny N is normal (program needs N >= k);
            # but if it keeps failing, the benchmark hit a ceiling.
            consec_err=$((consec_err + 1))
            if [ "$consec_err" -ge "$maxerr" ]; then
                if [ -n "$prev_n" ]; then
                    # It ran fine up to prev_n, then a depth/size limit kicks in
                    # (e.g. ackermann(3,m) vs MAX_CALL_DEPTH). Use the largest N
                    # that DID run for every remaining target rather than dropping
                    # the benchmark — that is its honest interpreted ceiling.
                    warn "  $bench: hits a depth/size ceiling past n=$prev_n — pinning n=$prev_n"
                    for rem in "${TARGETS[@]:$ti}"; do
                        record "$bench" "$rem" "$prev_n" "$prev_t" "ceiling"
                        ok "  $bench @ ${rem}ms -> n=$prev_n (${prev_t%.*}ms, ceiling)"
                    done
                    return
                fi
                fail "  $bench: interpreter errored at $maxerr consecutive sizes (n up to $n) — skipping"
                RES_N["$bench:$target"]=""; return
            fi
            n=$((n + step)); continue
        fi
        consec_err=0

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

        prev_n="$n"; prev_t="$t"
        if [ $((SECONDS - phase_start)) -ge "$BUDGET_S" ]; then
            step=$((step * ACCEL)); phase_start=$SECONDS
            info "    $bench: counting faster — step=$step (n=$n, ${t%.*}ms)"
        fi
        n=$((n + step))
        # Never leap past a per-bench ceiling: clamp so the cap is probed exactly.
        [ -n "$maxn" ] && [ "$n" -gt "$maxn" ] && n="$maxn"
    done

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

info "Phase 1: building largo (release) and setting up projects..."
cargo build -p logicaffeine-cli --release --manifest-path "$SCRIPT_DIR/../Cargo.toml" 2>/dev/null
LARGO="$SCRIPT_DIR/.logos-bench-target/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/largo"
if [ ! -f "$LARGO" ]; then fail "Could not find largo binary"; exit 1; fi
ok "largo built"

RUNNABLE=()
for bench in "${BENCHMARKS[@]}"; do
    setup_project "$bench" && RUNNABLE+=("$bench")
done
ok "Phase 1 complete"

info "Phase 2: crawling interpreter runtimes to targets: ${TARGETS[*]} ms (BUDGET_S=$BUDGET_S, ACCEL=$ACCEL)..."
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
        # interp_ms must be valid JSON. For sentinel timings (__OVER__ from an
        # over_at_min bench, __ERR__) or any non-numeric value, record 0 so a
        # single overshooting benchmark can't poison the whole file's assembly.
        case "$t" in
            ''|*[!0-9.]*) t=0 ;;
        esac
        entry=$(jq -n --argjson cur "$entry" --arg target "$target" \
            --argjson n "$n" --argjson ims "${t:-0}" --argjson tms "$target" --arg status "$status" \
            '$cur + {($target): {n: $n, interp_ms: $ims, target_ms: $tms, status: $status}}')
    done
    BENCH_JSON=$(jq -n --argjson cur "$BENCH_JSON" --arg b "$bench" --argjson e "$entry" '$cur + {($b): $e}')
done

mkdir -p "$(dirname "$OUT")"
jq -n \
    --arg date "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
    --arg cpu "$(detect_cpu)" --arg os "$(detect_os)" \
    --argjson targets "$TARGETS_JSON" --argjson tol "$TOL" \
    --argjson budget_s "$BUDGET_S" --argjson accel "$ACCEL" --argjson overshoot "$OVERSHOOT" \
    --argjson benchmarks "$BENCH_JSON" \
    '{
        meta: { date: $date, cpu: $cpu, os: $os,
                calibrated_side: "interpreter (VM + copy-and-patch JIT)",
                targets_ms: $targets, tol: $tol,
                budget_s: $budget_s, accel: $accel, overshoot: $overshoot },
        benchmarks: $benchmarks
    }' > "$OUT"
ok "Wrote $OUT"

# --- Phase 4: table ----------------------------------------------------------
echo
info "Calibrated interpreter sizes (largest n still under each target runtime):"
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
info "Full log: logs/optimization/calibrate-interp.log"
info "Feed run-interp-vs-js.sh with: CALIBRATION_TARGET=${TARGETS[0]} bash benchmarks/run-interp-vs-js.sh"
ok "Calibration complete!"
