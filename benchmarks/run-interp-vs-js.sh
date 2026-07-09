#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE LOGOS interpreter vs JavaScript (Node / V8)
#
# A peer-to-peer benchmark: the LOGOS interpreter (largo run --interpret — a
# bytecode VM with copy-and-patch JIT tier-up) against Node.js (V8). Both are
# dynamically typed, both run a bytecode VM + JIT, and both run the SAME naive
# algorithm from each benchmark's main.lg / main.js — so this is a clean engine
# head-to-head, not the apples-to-oranges trap of comparing against the LOGOS
# native compiler (which optimizes recursion the interpreter runs naively).
#
# Sizes come from calibrate-interp.sh (the N at which the LOGOS interpreter — the
# slower peer — runs for ~CALIBRATION_TARGET ms); run that first. Calibrating the
# slow side bounds runtime so nothing explodes; Node then runs faster but never
# below its ~30ms V8 startup floor, so both stay measurable.
#
# Ends with a worst-first ratio table (LOGOS-interp ÷ Node) and the geometric
# mean. Output teed to logs/optimization/interp-vs-js.log; results JSON to
# results/local-interp-vs-js.json. Each benchmark reports which LOGOS engine ran
# (vm+jit, or a tree-walker fallback) so a silent fallback is never mistaken for
# the JIT.
#
# Usage: bash benchmarks/run-interp-vs-js.sh
#
# Environment knobs:
#   ONLY=fib,sieve       Comma-separated subset of benchmarks to run.
#   RUNS=10              Timed runs per benchmark per engine.
#   WARMUP=2             Warmup runs per benchmark per engine.
#   SIZE_<bench>=N       Per-benchmark size override (e.g. SIZE_fib=31).
#   VERIFY=0             Skip the correctness phase.
#   CALIBRATION_TARGET=250   Which calibrated interpreter target to use.
#   OUT=results/local-interp-vs-js.json   Output JSON path (relative to benchmarks/).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw-interp-vs-js"

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

WARMUP="${WARMUP:-2}"
RUNS="${RUNS:-10}"
TIMEOUT="${TIMEOUT:-120}"
# Caps a whole hyperfine invocation (warmup+runs). LOGOS-interp/Node finish in a
# few seconds at the calibrated sizes; this bound mainly stops Python/Ruby — which
# are far slower at the interpreter-calibrated N — from burning minutes on the
# heaviest benchmarks before being dropped (they show where they fit, skip where
# too slow). Override for a standalone, more permissive run.
HYPERFINE_TIMEOUT="${HYPERFINE_TIMEOUT:-90}"
OUT="${OUT:-results/latest-interp.json}"

CALIBRATED_FILE="${CALIBRATED_FILE:-$RESULTS_DIR/calibrated-interp-sizes.json}"
# Sizes are calibrated so the FASTER of {interpreter, Node} reaches this many ms,
# which keeps BOTH off V8's ~30ms startup floor (see calibrate-interp.sh measure).
CALIBRATION_TARGET="${CALIBRATION_TARGET:-250}"
# Benchmarks excluded from the interpreter comparison (comma-separated). Empty by
# default: with MAX_CALL_DEPTH at 2500, ackermann(3,8) runs in the interpreter
# (depth ~2045), off V8's startup floor, so nothing is excluded. ackermann is
# held at its largest depth-safe size by the per-bench cap in bench_maxn() below
# (n=8; n=9 would recurse to depth ~4093 and overflow), so it is measured — slow
# but real — rather than skipped.
INTERP_SKIP="${INTERP_SKIP:-}"
interp_skip() { [ -n "$INTERP_SKIP" ] && echo ",$INTERP_SKIP," | grep -q ",$1,"; }

LOG_DIR="$SCRIPT_DIR/../logs/optimization"
mkdir -p "$LOG_DIR"
exec > >(tee "$LOG_DIR/interp-vs-js.log") 2>&1

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

mkdir -p "$RAW_DIR"
rm -rf "$RAW_DIR"/*

export LOGOS_WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
LOGOS_TARGET_DIR="$SCRIPT_DIR/.logos-bench-target"
mkdir -p "$LOGOS_TARGET_DIR"
export CARGO_TARGET_DIR="$LOGOS_TARGET_DIR"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

if command -v gtimeout &>/dev/null; then
    run_timeout() { gtimeout "$@"; }
elif command -v timeout &>/dev/null; then
    run_timeout() { timeout "$@"; }
else
    run_timeout() {
        local secs="$1"; shift
        "$@" &
        local pid=$!
        ( sleep "$secs" && kill "$pid" 2>/dev/null ) &
        local watchdog=$!
        wait "$pid" 2>/dev/null
        local rc=$?
        kill "$watchdog" 2>/dev/null
        wait "$watchdog" 2>/dev/null
        return $rc
    }
fi

min_size() { cat "$PROGRAMS_DIR/$1/sizes.txt" | tr ' ' '\n' | head -1; }

# Interpreter-scale fallback sizes — only used when neither SIZE_<bench> nor the
# calibrated file pins one. Kept small so the interpreter stays sub-second.
fallback_size() {
    case "$1" in
        fib) echo 31 ;; ackermann) echo 8 ;; nqueens) echo 9 ;;
        bubble_sort) echo 800 ;; mergesort) echo 50000 ;; quicksort) echo 50000 ;;
        counting_sort) echo 200000 ;; heap_sort) echo 50000 ;;
        nbody) echo 1000 ;; mandelbrot) echo 100 ;; spectral_norm) echo 100 ;;
        pi_leibniz) echo 1000000 ;;
        gcd) echo 300 ;; collatz) echo 100000 ;; primes) echo 10000 ;;
        sieve) echo 1000000 ;; matrix_mult) echo 50 ;; prefix_sum) echo 500000 ;;
        array_reverse) echo 500000 ;; array_fill) echo 500000 ;;
        collect) echo 200000 ;; two_sum) echo 1000 ;; histogram) echo 1000000 ;;
        knapsack) echo 300 ;; coins) echo 100000 ;;
        fannkuch) echo 7 ;;
        strings) echo 50000 ;; binary_trees) echo 16 ;;
        loop_sum) echo 1000000 ;; fib_iterative) echo 1000000 ;;
        graph_bfs) echo 1000 ;; string_search) echo 500000 ;;
        *) min_size "$1" ;;
    esac
}

# Per-benchmark hard ceiling on N (mirrors calibrate-interp.sh). ackermann(3, n)
# recurses to depth ~2^(n+3)-3, so n=9 (depth ~4093) overflows the interpreter's
# MAX_CALL_DEPTH (2500) while n=8 (depth ~2045) is safe. Clamping here keeps
# ackermann runnable — not skipped — even if the calibrated file is stale or a
# SIZE_ override is too large. Override per bench with MAXN_<bench>=N.
bench_maxn() {
    local override_var="MAXN_${1}"
    if [ -n "${!override_var:-}" ]; then echo "${!override_var}"; return; fi
    case "$1" in
        ackermann) echo 8 ;;
        *) echo "" ;;
    esac
}

bench_size() {
    local bench="$1"
    local sz="" maxn
    local override_var="SIZE_${bench}"
    if [ -n "${!override_var:-}" ]; then
        sz="${!override_var}"
    elif [ -f "$CALIBRATED_FILE" ] && sz=$(jq -r --arg b "$bench" --arg t "$CALIBRATION_TARGET" \
            '.benchmarks[$b][$t].n // empty' "$CALIBRATED_FILE" 2>/dev/null) && [ -n "$sz" ]; then
        :
    else
        sz="$(fallback_size "$bench")"
    fi
    maxn="$(bench_maxn "$bench")"
    if [ -n "$maxn" ] && [ -n "$sz" ] && [ "$sz" -gt "$maxn" ]; then sz="$maxn"; fi
    echo "$sz"
}

# ===========================================================================
# Phase 1: Build largo, set up interpret projects, check Node
# ===========================================================================
info "Phase 1: Building largo (release), setting up interpret projects, checking Node..."

if ! command -v node &>/dev/null; then fail "node not found on PATH"; exit 1; fi
ok "node $(node --version)"

cargo build -p logicaffeine-cli --release --manifest-path "$SCRIPT_DIR/../Cargo.toml" 2>/dev/null
LARGO="$LOGOS_TARGET_DIR/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$LOGOS_TARGET_DIR/release/largo"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/largo"
if [ ! -f "$LARGO" ]; then fail "Could not find largo binary"; exit 1; fi
ok "largo built"

TMP_BASE=$(mktemp -d)
trap 'rm -rf "$TMP_BASE"' EXIT
declare -A PROJ
setup_project() {
    local bench="$1"
    [ -f "$PROGRAMS_DIR/$bench/main.lg" ] || return 1
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

for bench in "${BENCHMARKS[@]}"; do
    [ -f "$PROGRAMS_DIR/$bench/main.lg" ] || { warn "  no main.lg for $bench"; continue; }
    [ -f "$PROGRAMS_DIR/$bench/main.js" ] || { warn "  no main.js for $bench"; continue; }
    setup_project "$bench"
done
ok "Phase 1 complete"

# Engine probe: which LOGOS interpreter engine runs each program (chosen at
# compile time, so a tiny N=1 run reports it regardless of runtime outcome).
declare -A ENGINE
detect_engine() {
    local bench="$1" proj="${PROJ[$bench]:-}"
    [ -n "$proj" ] || { ENGINE[$bench]="?"; return 0; }
    local line
    line=$( cd "$proj" && LOGOS_ENGINE_TRACE=1 run_timeout 30 "$LARGO" run --interpret 1 2>&1 >/dev/null \
            | grep -m1 'logos-engine:' ) || true
    ENGINE[$bench]="${line#logos-engine: }"
    if [ -z "${ENGINE[$bench]}" ]; then ENGINE[$bench]="?"; fi
    return 0
}

# ===========================================================================
# Phase 2: Verify — LOGOS interpreter and Node must agree at the chosen size
# ===========================================================================
# Benchmarks that fail verification are skipped (not fatal): a 32-way sweep
# shouldn't abort because one program can't be interpreted yet.
declare -A SKIP
# The size at which the interpreter verified (may be < the calibrated size after
# back-off, e.g. ackermann under MAX_CALL_DEPTH). Phase 3 benchmarks here.
declare -A VERIFIED_SIZE
if [ "${VERIFY:-1}" = "0" ]; then
    warn "Phase 2: skipped (VERIFY=0)"
else
info "Phase 2: Verifying LOGOS-interpreter / Node agreement at calibrated sizes..."

run_interp() { ( cd "${PROJ[$1]}" && run_timeout "$TIMEOUT" "$LARGO" run --interpret "$2" 2>/dev/null ) | tr -d '[:space:]'; }
run_js()     { run_timeout "$TIMEOUT" node "$PROGRAMS_DIR/$1/main.js" "$2" 2>/dev/null | tr -d '[:space:]'; }

for bench in "${BENCHMARKS[@]}"; do
    if interp_skip "$bench"; then warn "Excluding $bench from interpreter comparison (compiled-only)"; SKIP[$bench]=1; continue; fi
    [ -n "${PROJ[$bench]:-}" ] || { warn "No interpret project for $bench — skipping"; SKIP[$bench]=1; continue; }
    size="$(bench_size "$bench")"
    detect_engine "$bench"
    info "Verifying $bench (n=$size, interp engine: ${ENGINE[$bench]})..."

    # Back off on failure: if the interpreter can't run this size (e.g. ackermann
    # exceeds the shared MAX_CALL_DEPTH=1000 at the calibrated m), halve N and
    # retry until it agrees with the reference, or give up. The size that agrees
    # is recorded so Phase 3 benchmarks every engine there (same N apples-to-
    # apples). Only truly unsupported programs are skipped.
    vattempt=0
    while :; do
        js_out=$(run_js "$bench" "$size") || true
        interp_out=$(run_interp "$bench" "$size") || true
        expected_file="$PROGRAMS_DIR/$bench/expected_${size}.txt"
        if [ -f "$expected_file" ]; then
            reference=$(cat "$expected_file" | tr -d '[:space:]')
        else
            reference="$js_out"  # same algorithm, same N: Node is the reference
        fi
        if [ -n "$js_out" ] && [ "$js_out" = "$reference" ] && [ "$interp_out" = "$reference" ]; then
            ok "  agree at n=$size: $interp_out"
            VERIFIED_SIZE[$bench]="$size"
            break
        fi
        if [ "$vattempt" -lt 8 ] && [ "$size" -gt 1 ]; then
            vattempt=$((vattempt + 1))
            size=$(( size / 2 )); [ "$size" -lt 1 ] && size=1
            warn "  $bench disagreed/failed (interp='$interp_out' node='$js_out' ref='$reference') — backing off to n=$size"
            continue
        fi
        fail "  $bench: no size agrees (interp='$interp_out' node='$js_out') — skipping"
        SKIP[$bench]=1
        break
    done
done

SKIPPED="${!SKIP[*]}"
[ -n "$SKIPPED" ] && warn "Phase 2: skipped (disagreed/failed): $SKIPPED" || ok "Phase 2 complete: all agree"
fi

if [ "${VERIFY:-1}" = "0" ]; then
    for bench in "${BENCHMARKS[@]}"; do detect_engine "$bench"; done
fi

# ===========================================================================
# Phase 3: Benchmark — LOGOS interpreter vs Node, per-engine timeout isolation
# ===========================================================================
info "Phase 3: Benchmarking LOGOS interpreter vs Node ($WARMUP warmup, $RUNS runs)..."

PER_LANG_DIR="$RAW_DIR/per_lang"
mkdir -p "$PER_LANG_DIR"

merge_hyperfine_results() {
    local output_file="$1"; shift
    local files=("$@")
    if [ ${#files[@]} -eq 0 ]; then echo '{"results":[]}' > "$output_file"; return; fi
    jq -s '{ results: [.[].results[]] }' "${files[@]}" > "$output_file"
}

declare -A LANG_TIMEOUTS
# The size actually benchmarked per bench (may be smaller than the calibrated
# size if the interpreter timed out and we dropped N). Downstream assembly, the
# geomean, and the ratio table all read this so they line up with the raw files.
declare -A ACTUAL_SIZE

# try_bench LANG_ID LABEL CMD [EXTRA_HYPERFINE_FLAGS]
# Node passes --shell=none (its command is a bare invocation). The LOGOS
# interpreter needs the shell (cd into the project), so it omits the flag.
try_bench() {
    local lang_id="$1" label="$2" cmd="$3" extra="${4:-}"
    if [[ "${LANG_TIMEOUTS[${bench}_${lang_id}]:-}" == "1" ]]; then
        warn "  Skipping $label (timed out at smaller size)"; return
    fi
    local pf="$PER_LANG_DIR/${bench}_${size}_${lang_id}.json"
    local rc=0
    run_timeout "$HYPERFINE_TIMEOUT" hyperfine $extra \
        --warmup "$WARMUP" --runs "$RUNS" \
        --export-json "$pf" --time-unit millisecond \
        -n "$label" "$cmd" || rc=$?
    if [ "$rc" -eq 124 ] || [ "$rc" -eq 143 ]; then
        LANG_TIMEOUTS["${bench}_${lang_id}"]=1
        warn "  $label timed out for $bench at $size"
        echo "$HYPERFINE_TIMEOUT" > "$RAW_DIR/${bench}_${size}_${lang_id}.timeout"
    elif [ "$rc" -ne 0 ]; then
        warn "  $label benchmark failed for $bench at $size"
    fi
    [ -f "$pf" ] && MERGE_FILES+=("$pf")
    return 0
}

for bench in "${BENCHMARKS[@]}"; do
    interp_skip "$bench" && continue
    [ -n "${PROJ[$bench]:-}" ] || continue
    [ -n "${SKIP[$bench]:-}" ] && { warn "Skipping $bench (failed verification)"; continue; }
    # Start from the size Phase 2 verified at (after any back-off), else the
    # calibrated size.
    size="${VERIFIED_SIZE[$bench]:-$(bench_size "$bench")}"
    attempt=0
    while :; do
        info "Benchmarking $bench at n=$size (interp engine: ${ENGINE[$bench]:-?})..."
        MERGE_FILES=()
        # Clear any prior timeout marks for this bench so a retry actually runs.
        unset "LANG_TIMEOUTS[${bench}_logos_interp]" "LANG_TIMEOUTS[${bench}_logos_tiered]" "LANG_TIMEOUTS[${bench}_js]" \
              "LANG_TIMEOUTS[${bench}_python]" "LANG_TIMEOUTS[${bench}_ruby]"
        rm -f "$RAW_DIR/${bench}_${size}_logos_interp.timeout" "$RAW_DIR/${bench}_${size}_logos_tiered.timeout" "$RAW_DIR/${bench}_${size}_js.timeout"

        # The two LOGOS rows the page compares (HOTSWAP §12): `eager` is today's
        # full-upfront-optimizer headline (the A/B baseline); `tiered` starts at the
        # baseline and escalates by hotness, reclaiming the in-window optimizer cost.
        try_bench logos_interp "LOGOS (interpreted)" "cd \"${PROJ[$bench]}\" && LOGOS_TIER_PROFILE=eager \"$LARGO\" run --interpret $size"
        try_bench logos_tiered "LOGOS (tiered)" "cd \"${PROJ[$bench]}\" && LOGOS_TIER_PROFILE=tiered \"$LARGO\" run --interpret $size"
        try_bench js "JavaScript (Node)" "node $PROGRAMS_DIR/$bench/main.js $size" "--shell=none"
        # Python and Ruby are interpreted peers — show them in the second section
        # too. They run the SAME algorithm at the SAME N (same-N apples-to-apples);
        # per-engine timeout isolation drops them on a size they can't finish.
        [ -f "$PROGRAMS_DIR/$bench/main.py" ] && \
            try_bench python "Python" "python3 $PROGRAMS_DIR/$bench/main.py $size" "--shell=none"
        [ -f "$PROGRAMS_DIR/$bench/main.rb" ] && \
            try_bench ruby "Ruby" "env RUBY_THREAD_VM_STACK_SIZE=67108864 ruby $PROGRAMS_DIR/$bench/main.rb $size" "--shell=none"

        # The interpreter section must never time out: if the interpreter blew
        # past the limit, drop N by 4x and re-run BOTH engines so the comparison
        # stays same-N apples-to-apples at the smaller size.
        if [ "${LANG_TIMEOUTS[${bench}_logos_interp]:-}" = "1" ] && [ "$attempt" -lt 4 ]; then
            attempt=$((attempt + 1))
            new_size=$(( size / 4 ))
            [ "$new_size" -lt 1 ] && new_size=1
            warn "  LOGOS interpreter timed out at n=$size — retrying at n=$new_size"
            size="$new_size"
            continue
        fi
        break
    done
    ACTUAL_SIZE[$bench]="$size"

    if [ ${#MERGE_FILES[@]} -gt 0 ]; then
        merge_hyperfine_results "$RAW_DIR/${bench}_${size}.json" "${MERGE_FILES[@]}"
    else
        echo '{"results":[]}' > "$RAW_DIR/${bench}_${size}.json"
    fi
done

ok "Phase 3 complete"

# ===========================================================================
# Phase 4: Assemble results JSON
# ===========================================================================
info "Phase 4: Assembling $OUT..."

detect_cpu() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "$(uname -m)"
    elif [ -f /proc/cpuinfo ]; then
        grep -m1 'model name' /proc/cpuinfo 2>/dev/null | sed 's/.*: //' || echo "$(uname -m)"
    else uname -m; fi
}
detect_os() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        echo "$(sw_vers -productName 2>/dev/null || echo macOS) $(sw_vers -productVersion 2>/dev/null || echo '') $(uname -m)"
    elif [ -f /etc/os-release ]; then
        local pretty; pretty=$(grep -m1 PRETTY_NAME /etc/os-release 2>/dev/null | sed 's/PRETTY_NAME="\?\([^"]*\)"\?/\1/')
        echo "${pretty:-Linux} $(uname -m)"
    else echo "$(uname -s) $(uname -r) $(uname -m)"; fi
}

DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
COMMIT=$(cd "$SCRIPT_DIR/.." && git rev-parse --short HEAD 2>/dev/null || echo "unknown")
CPU=$(detect_cpu)
[ "${GITHUB_ACTIONS:-}" = "true" ] && CPU="$CPU (GitHub Actions)"
OS=$(detect_os)
if [ -n "${LOGOS_VERSION:-}" ]; then
    LOGOS_VER="$LOGOS_VERSION"
else
    LOGOS_VER=$(grep '^version' "$SCRIPT_DIR/../Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
fi
NODE_VER=$(node --version 2>/dev/null || echo "unknown")
PY_VER=$(python3 --version 2>/dev/null | sed 's/Python //' || echo "")
RB_VER=$(ruby --version 2>/dev/null | sed 's/ruby \([0-9.]*\).*/\1/' || echo "")

LANGUAGES=$(jq -n --arg node "$NODE_VER" --arg py "$PY_VER" --arg rb "$RB_VER" '[
  {"id":"logos_interp","label":"LOGOS (eager)","color":"#ff8c00","tier":"interpreted"},
  {"id":"logos_tiered","label":"LOGOS (tiered)","color":"#ffb84d","tier":"interpreted"},
  {"id":"js","label":("Node " + $node),"color":"#f7df1e","tier":"interpreted"},
  {"id":"python","label":(if $py == "" then "Python" else "Python " + $py end),"color":"#3776ab","tier":"interpreted"},
  {"id":"ruby","label":(if $rb == "" then "Ruby" else "Ruby " + $rb end),"color":"#cc342d","tier":"interpreted"}
]')

lang_id() {
    case "$1" in
        "LOGOS (interpreted)") echo logos_interp ;;
        "LOGOS (tiered)") echo logos_tiered ;;
        "JavaScript (Node)") echo js ;;
        "Python") echo python ;;
        "Ruby") echo ruby ;;
        *) echo "$1" | tr '[:upper:]' '[:lower:]' | tr ' ' '_' ;;
    esac
}

bench_name() {
    case "$1" in
        fib) echo "Recursive Fibonacci" ;; ackermann) echo "Ackermann Function" ;; nqueens) echo "N-Queens" ;;
        bubble_sort) echo "Bubble Sort" ;; mergesort) echo "Merge Sort" ;; quicksort) echo "Quicksort" ;;
        counting_sort) echo "Counting Sort" ;; heap_sort) echo "Heap Sort" ;;
        nbody) echo "N-Body Simulation" ;; mandelbrot) echo "Mandelbrot Set" ;;
        spectral_norm) echo "Spectral Norm" ;; pi_leibniz) echo "Pi (Leibniz Series)" ;;
        gcd) echo "GCD Sum" ;; collatz) echo "Collatz Conjecture" ;; primes) echo "Primes (Trial Division)" ;;
        sieve) echo "Sieve of Eratosthenes" ;; matrix_mult) echo "Matrix Multiply" ;;
        prefix_sum) echo "Prefix Sum" ;; array_reverse) echo "Array Reverse" ;; array_fill) echo "Array Fill & Sum" ;;
        collect) echo "Collection Operations" ;; two_sum) echo "Two Sum" ;; histogram) echo "Histogram" ;;
        knapsack) echo "0/1 Knapsack" ;; coins) echo "Coin Change" ;;
        fannkuch) echo "Fannkuch Redux" ;;
        strings) echo "String Assembly" ;; binary_trees) echo "Binary Trees" ;;
        loop_sum) echo "Loop Sum" ;; fib_iterative) echo "Iterative Fibonacci" ;;
        graph_bfs) echo "Graph BFS" ;; string_search) echo "Naive String Search" ;;
    esac
}

extract_name() { echo "$1" | jq -r '.command' 2>/dev/null; }

assemble_benchmark() {
    local bench="$1"
    local size="${ACTUAL_SIZE[$bench]:-$(bench_size "$bench")}"
    local logos_src=""
    [ -f "$PROGRAMS_DIR/$bench/main.lg" ] && logos_src=$(cat "$PROGRAMS_DIR/$bench/main.lg")
    local js_src=""
    [ -f "$PROGRAMS_DIR/$bench/main.js" ] && js_src=$(cat "$PROGRAMS_DIR/$bench/main.js")

    local scaling="{}"
    local raw_file="$RAW_DIR/${bench}_${size}.json"
    if [ -f "$raw_file" ]; then
        local size_data="{}"
        local results
        results=$(jq -c '.results[]' "$raw_file" 2>/dev/null) || true
        while IFS= read -r result; do
            [ -z "$result" ] && continue
            local name lid mean_s median_s stddev_s min_s max_s user_s system_s
            name=$(extract_name "$result")
            lid=$(lang_id "$name")
            mean_s=$(echo "$result" | jq '.mean')
            median_s=$(echo "$result" | jq '.median')
            stddev_s=$(echo "$result" | jq '.stddev')
            min_s=$(echo "$result" | jq '.min')
            max_s=$(echo "$result" | jq '.max')
            user_s=$(echo "$result" | jq '.user // null')
            system_s=$(echo "$result" | jq '.system // null')
            local mean_ms median_ms stddev_ms min_ms max_ms cv user_ms system_ms
            mean_ms=$(echo "$mean_s * 1000" | bc -l 2>/dev/null || echo "0")
            median_ms=$(echo "$median_s * 1000" | bc -l 2>/dev/null || echo "0")
            stddev_ms=$(echo "$stddev_s * 1000" | bc -l 2>/dev/null || echo "0")
            min_ms=$(echo "$min_s * 1000" | bc -l 2>/dev/null || echo "0")
            max_ms=$(echo "$max_s * 1000" | bc -l 2>/dev/null || echo "0")
            if [ "$user_s" != "null" ] && [ -n "$user_s" ]; then
                user_ms=$(echo "$user_s * 1000" | bc -l 2>/dev/null || echo "null")
            else user_ms="null"; fi
            if [ "$system_s" != "null" ] && [ -n "$system_s" ]; then
                system_ms=$(echo "$system_s * 1000" | bc -l 2>/dev/null || echo "null")
            else system_ms="null"; fi
            if [ "$(echo "$mean_ms > 0" | bc -l 2>/dev/null)" = "1" ]; then
                cv=$(echo "$stddev_ms / $mean_ms" | bc -l 2>/dev/null || echo "0")
            else cv="0"; fi
            size_data=$(echo "$size_data" | jq \
                --arg lid "$lid" \
                --argjson mean "$mean_ms" --argjson median "$median_ms" --argjson stddev "$stddev_ms" \
                --argjson min "$min_ms" --argjson max "$max_ms" --argjson cv "$cv" \
                --argjson runs "$RUNS" --argjson user "$user_ms" --argjson sys "$system_ms" \
                '.[$lid] = {mean_ms: $mean, median_ms: $median, stddev_ms: $stddev, min_ms: $min, max_ms: $max, cv: $cv, runs: $runs, user_ms: $user, system_ms: $sys}')
        done <<< "$results"
        scaling=$(echo "$scaling" | jq --arg size "$size" --argjson data "$size_data" '.[$size] = $data')
    fi

    jq -n \
        --arg id "$bench" --arg name "$(bench_name "$bench")" \
        --arg ref "$size" --arg engine "${ENGINE[$bench]:-?}" \
        --arg logos_src "$logos_src" --arg js_src "$js_src" \
        --arg sizes_str "$(cat "$PROGRAMS_DIR/$bench/sizes.txt")" \
        --argjson scaling "$scaling" \
        '{
            id: $id, name: $name, reference_size: $ref,
            interpreter_engine: $engine,
            sizes: ($sizes_str | split(" ")),
            logos_source: $logos_src, js_source: $js_src,
            scaling: $scaling, compilation: {}, timeouts: {}
        }'
}

BENCHMARKS_JSON="["
first=true
for bench in "${BENCHMARKS[@]}"; do
    [ -n "${SKIP[$bench]:-}" ] && continue
    if [ "$first" = true ]; then first=false; else BENCHMARKS_JSON+=","; fi
    BENCHMARKS_JSON+=$(assemble_benchmark "$bench")
done
BENCHMARKS_JSON+="]"

# Geometric mean of a LOGOS engine ÷ Node (>1 = LOGOS is slower). `$1` is the engine
# id (`logos_interp` = eager, default; `logos_tiered`).
compute_geometric_mean() {
    local engine="${1:-logos_interp}"
    local log_sum=0 count=0
    for bench in "${BENCHMARKS[@]}"; do
        local ref="${ACTUAL_SIZE[$bench]:-$(bench_size "$bench")}"
        local l_file="$PER_LANG_DIR/${bench}_${ref}_${engine}.json"
        local j_file="$PER_LANG_DIR/${bench}_${ref}_js.json"
        [ -f "$l_file" ] && [ -f "$j_file" ] || continue
        local l_ms j_ms
        l_ms=$(jq -r '.results[0].mean * 1000' "$l_file")
        j_ms=$(jq -r '.results[0].mean * 1000' "$j_file")
        if [ "$(echo "$l_ms > 0 && $j_ms > 0" | bc -l 2>/dev/null)" = "1" ]; then
            local ratio; ratio=$(echo "$l_ms / $j_ms" | bc -l)
            if [ "$(echo "$ratio > 0" | bc -l)" = "1" ]; then
                log_sum=$(echo "$log_sum + l($ratio)" | bc -l)
                count=$((count + 1))
            fi
        fi
    done
    if [ "$count" -gt 0 ]; then
        printf "%.3f" "$(echo "e($log_sum / $count)" | bc -l)"
    else echo "0"; fi
}

GEO=$(compute_geometric_mean logos_interp)
GEO_TIERED=$(compute_geometric_mean logos_tiered)

BENCHMARKS_JSON_FILE=$(mktemp)
printf '%s\n' "$BENCHMARKS_JSON" > "$BENCHMARKS_JSON_FILE"
mkdir -p "$(dirname "$OUT")"

# Engine footprint — the largo VM+JIT binary vs node, plus the browser WASM bundle.
INTERP_SIZES_JSON=null
if [ -x "$SCRIPT_DIR/measure-sizes.sh" ]; then
    if bash "$SCRIPT_DIR/measure-sizes.sh" --engines-only >/dev/null 2>&1; then
        INTERP_SIZES_JSON="$(cat "$RESULTS_DIR/raw/interpreter_sizes.json" 2>/dev/null || echo null)"
    else
        warn "engine-size measurement failed (non-fatal)"
    fi
fi

jq -n \
    --arg date "$DATE" --arg commit "$COMMIT" --arg cpu "$CPU" --arg os "$OS" \
    --arg logos_version "$LOGOS_VER" --arg node "$NODE_VER" \
    --argjson warmup "$WARMUP" --argjson runs "$RUNS" \
    --argjson languages "$LANGUAGES" \
    --slurpfile benchmarks "$BENCHMARKS_JSON_FILE" \
    --argjson geo "${GEO:-0}" \
    --argjson geo_tiered "${GEO_TIERED:-0}" \
    --argjson interpreter_sizes "$INTERP_SIZES_JSON" \
    '{
        schema_version: 1,
        metadata: {
            date: $date, commit: $commit, logos_version: $logos_version, node: $node,
            cpu: $cpu, os: $os, warmup: $warmup, runs: $runs
        },
        languages: $languages,
        benchmarks: $benchmarks[0],
        interpreter_sizes: $interpreter_sizes,
        summary: {
            geometric_mean_logos_interp_over_node: $geo,
            geometric_mean_logos_tiered_over_node: $geo_tiered
        }
    }' > "$OUT"

rm -f "$BENCHMARKS_JSON_FILE"
ok "Results written to $OUT"

# Cold-start floor (serverless / CLI metric) — merges a "startup" block into $OUT.
if [ -x "$SCRIPT_DIR/measure-startup.sh" ]; then
    info "Measuring cold-start floor (interpreter vs Node/Python/Ruby)..."
    OUT="$OUT" bash "$SCRIPT_DIR/measure-startup.sh" || warn "startup measurement failed (non-fatal)"
fi

# ===========================================================================
# Phase 5: Ratio table — LOGOS interpreter ÷ Node (worst-first)
# ===========================================================================
echo
info "LOGOS interpreter vs Node (ratio = LOGOS-interp ms / Node ms; >1 means LOGOS is slower):"
echo

ROWS=""
for bench in "${BENCHMARKS[@]}"; do
    [ -n "${SKIP[$bench]:-}" ] && continue
    size="${ACTUAL_SIZE[$bench]:-$(bench_size "$bench")}"
    l_file="$PER_LANG_DIR/${bench}_${size}_logos_interp.json"
    j_file="$PER_LANG_DIR/${bench}_${size}_js.json"
    l_ms="" j_ms=""
    [ -f "$l_file" ] && l_ms=$(jq -r '.results[0].mean * 1000' "$l_file")
    [ -f "$j_file" ] && j_ms=$(jq -r '.results[0].mean * 1000' "$j_file")
    if [ -n "$l_ms" ] && [ -n "$j_ms" ] && [ "$(echo "$j_ms > 0" | bc -l)" = "1" ]; then
        ratio=$(echo "$l_ms / $j_ms" | bc -l)
        ROWS+=$(printf "%s|%s|%s|%s|%s|%s\n" "$ratio" "$bench" "$size" "$l_ms" "$j_ms" "${ENGINE[$bench]:-?}")$'\n'
    else
        [ -z "$l_ms" ] && warn "  $bench: no LOGOS interp result"
        [ -z "$j_ms" ] && warn "  $bench: no Node result"
    fi
done

printf "%-14s %10s %14s %10s %9s  %s\n" "benchmark" "n" "LOGOS-interp ms" "Node ms" "ratio" "engine"
printf '%.0s-' $(seq 1 76); echo
echo -n "$ROWS" | sort -t'|' -k1 -gr | while IFS='|' read -r ratio bench size l_ms j_ms engine; do
    [ -z "$bench" ] && continue
    printf "%-14s %10s %14.1f %10.1f %8.2fx  %s\n" "$bench" "$size" "$l_ms" "$j_ms" "$ratio" "$engine"
done

echo
if [ "$(echo "${GEO:-0} > 0" | bc -l)" = "1" ]; then
    info "Geometric mean: the LOGOS interpreter is ${GEO}x Node's runtime (>1 = slower than Node)"
fi
warn "Note: Node carries a ~20-40ms V8 startup floor; on benchmarks where Node lands near it, the ratio understates LOGOS's compute gap. Raise the calibration target for a more compute-dominated comparison."
info "Full log: logs/optimization/interp-vs-js.log"
info "LOGOS interpreter vs Node benchmark complete!"
