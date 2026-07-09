#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE LOGOS compiled vs interpreted Benchmark
#
# The compiled-vs-interpreted twin of run-logos-vs-c.sh: it measures how much
# faster compiled LOGOS is than the interpreter (the bytecode VM with the
# copy-and-patch JIT). Both sides run the SAME main.lg at the SAME calibrated
# size, so the only variable is the engine:
#
#   compiled    largo build --release  ->  native LLVM binary  (bin/<bench>_logos_release)
#   interpreted largo run --interpret N ->  bytecode VM + JIT tier-up, in-process
#
# main.lg reads its size from args(); `largo run --interpret N` forwards N, so a
# single source runs both ways. Sizes come from calibrate-interp.sh (the N at
# which the INTERPRETER runs for ~CALIBRATION_TARGET ms) — run that first.
#
# Ends with a worst-first speedup table (compiled vs interpreted) and the
# geometric-mean speedup. Output is teed to logs/optimization/logos-vs-interp.log;
# results JSON goes to results/local-logos-vs-interp.json (same schema family as
# results/latest.json). Each benchmark reports which interpreter engine actually
# ran (vm+jit, or a tree-walker fallback) so a silent fallback is never mistaken
# for the JIT.
#
# Usage: bash benchmarks/run-logos-vs-interp.sh
#
# Environment knobs:
#   ONLY=fib,sieve       Comma-separated subset of benchmarks to run.
#   RUNS=10              Timed runs per benchmark per engine.
#   WARMUP=2             Warmup runs per benchmark per engine.
#   SIZE_<bench>=N       Per-benchmark size override (e.g. SIZE_fib=30).
#   VERIFY=0             Skip the correctness phase.
#   CALIBRATION_TARGET=250   Which calibrated interpreter target to use.
#   OUT=results/local-logos-vs-interp.json   Output JSON path (relative to benchmarks/).
#   FORCE_BUILD=1        Rebuild every native binary even if cached.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="$SCRIPT_DIR/bin"
PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw-logos-vs-interp"
GENERATED_DIR="$SCRIPT_DIR/generated"

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
TIMEOUT=300
HYPERFINE_TIMEOUT=600
OUT="${OUT:-results/local-logos-vs-interp.json}"

# Per-benchmark sizes come from calibrate-interp.sh (the N at which the COMPILED
# binary runs for ~CALIBRATION_TARGET ms — sizing the fast side keeps it above
# hyperfine's timing floor while the interpreter runs at target × its slowdown).
# SIZE_<bench> overrides; the fallback table below is a conservative safety net
# for when calibration has not been run — calibrate-interp.sh first is recommended.
CALIBRATED_FILE="${CALIBRATED_FILE:-$RESULTS_DIR/calibrated-interp-sizes.json}"
CALIBRATION_TARGET="${CALIBRATION_TARGET:-10}"

LOG_DIR="$SCRIPT_DIR/../logs/optimization"
mkdir -p "$LOG_DIR"
exec > >(tee "$LOG_DIR/logos-vs-interp.log") 2>&1

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

mkdir -p "$BIN_DIR" "$RAW_DIR" "$GENERATED_DIR"
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

# Smallest size in sizes.txt — the interpreter-friendly fallback when nothing
# else pins a size.
min_size() {
    cat "$PROGRAMS_DIR/$1/sizes.txt" | tr ' ' '\n' | head -1
}

# Interpreter-scale fallback sizes — only used when neither SIZE_<bench> nor the
# calibrated file pins one. Kept small so the (much slower) interpreter stays
# sub-second; the real workflow calibrates first.
fallback_size() {
    case "$1" in
        fib) echo 28 ;; ackermann) echo 7 ;; nqueens) echo 9 ;;
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

bench_size() {
    local bench="$1"
    local override_var="SIZE_${bench}"
    if [ -n "${!override_var:-}" ]; then
        echo "${!override_var}"; return
    fi
    if [ -f "$CALIBRATED_FILE" ]; then
        local calibrated
        calibrated=$(jq -r --arg b "$bench" --arg t "$CALIBRATION_TARGET" \
            '.benchmarks[$b][$t].n // empty' "$CALIBRATED_FILE" 2>/dev/null)
        if [ -n "$calibrated" ]; then echo "$calibrated"; return; fi
    fi
    fallback_size "$bench"
}

# ===========================================================================
# Phase 1: Build largo, compile native binaries, set up interpret projects
# ===========================================================================
info "Phase 1: Building largo (release), native LOGOS binaries, interpret projects..."

cargo build -p logicaffeine-cli --release --manifest-path "$SCRIPT_DIR/../Cargo.toml" 2>/dev/null
LARGO="$LOGOS_TARGET_DIR/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$LOGOS_TARGET_DIR/release/largo"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/logicaffeine-cli"
[ -f "$LARGO" ] || LARGO="$SCRIPT_DIR/../target/release/largo"
if [ ! -f "$LARGO" ]; then fail "Could not find largo binary"; exit 1; fi
ok "largo built"

logos_cached() {
    [ "${FORCE_BUILD:-}" = "1" ] && return 1
    [ -f "$BIN_DIR/${1}_logos_release" ] && \
        [ "$BIN_DIR/${1}_logos_release" -nt "$PROGRAMS_DIR/$1/main.lg" ] && \
        [ "$BIN_DIR/${1}_logos_release" -nt "$LARGO" ] || return 1
    return 0
}

# Persistent interpret projects for the whole run (one per benchmark), each
# feeding the same main.lg the native build compiles.
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
    info "Building $bench..."
    [ -f "$PROGRAMS_DIR/$bench/main.lg" ] || { warn "  no main.lg — skipping"; continue; }

    setup_project "$bench"

    if logos_cached "$bench"; then
        ok "  LOGOS (compiled) (cached)"
    else
        LOGOS_TMP=$(mktemp -d)
        mkdir -p "$LOGOS_TMP/src"
        cp "$PROGRAMS_DIR/$bench/main.lg" "$LOGOS_TMP/src/main.lg"
        cat > "$LOGOS_TMP/Largo.toml" << 'TOML'
[package]
name = "bench"
version = "0.1.0"
entry = "src/main.lg"
TOML
        (cd "$LOGOS_TMP" && "$LARGO" build --release 2>/dev/null) && {
            LOGOS_BIN=""
            if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -f "$CARGO_TARGET_DIR/release/bench" ]; then
                LOGOS_BIN="$CARGO_TARGET_DIR/release/bench"
            elif [[ "$(uname -s)" == "Darwin" ]]; then
                LOGOS_BIN=$(find "$LOGOS_TMP/target/release" -type f -perm +111 -name "bench" 2>/dev/null | head -1)
            else
                LOGOS_BIN=$(find "$LOGOS_TMP/target/release" -type f -executable -name "bench" 2>/dev/null | head -1)
            fi
            [ -n "$LOGOS_BIN" ] && rm -f "$BIN_DIR/${bench}_logos_release" && cp "$LOGOS_BIN" "$BIN_DIR/${bench}_logos_release" && ok "  LOGOS (compiled)" || true
            GENERATED_RS=$(find "$LOGOS_TMP" -name "main.rs" -path "*/build/src/*" 2>/dev/null | head -1)
            [ -n "$GENERATED_RS" ] && cp "$GENERATED_RS" "$GENERATED_DIR/$bench.rs" 2>/dev/null || true
        } || true
        rm -rf "$LOGOS_TMP"
    fi
done

ok "Phase 1 complete"

# Engine probe: which interpreter engine actually runs each program. The engine
# is chosen at compile time (VM-accept vs needs-async vs VM-reject), so a tiny
# N=1 run reports it regardless of runtime outcome.
declare -A ENGINE
detect_engine() {
    local bench="$1" proj="${PROJ[$bench]:-}"
    [ -n "$proj" ] || { ENGINE[$bench]="?"; return; }
    local line
    line=$( cd "$proj" && LOGOS_ENGINE_TRACE=1 run_timeout 30 "$LARGO" run --interpret 1 2>&1 >/dev/null \
            | grep -m1 'logos-engine:' ) || true
    ENGINE[$bench]="${line#logos-engine: }"
    if [ -z "${ENGINE[$bench]}" ]; then ENGINE[$bench]="?"; fi
    return 0
}

# ===========================================================================
# Phase 2: Verify — compiled and interpreted must agree at the calibrated size
# ===========================================================================
ERRORS=0
if [ "${VERIFY:-1}" = "0" ]; then
    warn "Phase 2: skipped (VERIFY=0)"
else
info "Phase 2: Verifying compiled/interpreted agreement at calibrated sizes..."

run_compiled() { run_timeout "$TIMEOUT" "$BIN_DIR/${1}_logos_release" "$2" 2>/dev/null | tr -d '[:space:]'; }
run_interp()   { ( cd "${PROJ[$1]}" && run_timeout "$TIMEOUT" "$LARGO" run --interpret "$2" 2>/dev/null ) | tr -d '[:space:]'; }

for bench in "${BENCHMARKS[@]}"; do
    [ -f "$BIN_DIR/${bench}_logos_release" ] || { warn "No compiled binary for $bench — skipping"; continue; }
    [ -n "${PROJ[$bench]:-}" ] || { warn "No interpret project for $bench — skipping"; continue; }
    size="$(bench_size "$bench")"
    detect_engine "$bench"
    info "Verifying $bench (n=$size, interp engine: ${ENGINE[$bench]})..."

    compiled_out=$(run_compiled "$bench" "$size") || true
    interp_out=$(run_interp "$bench" "$size") || true

    expected_file="$PROGRAMS_DIR/$bench/expected_${size}.txt"
    reference=""
    if [ -f "$expected_file" ]; then
        reference=$(cat "$expected_file" | tr -d '[:space:]')
    else
        reference="$compiled_out"  # same source, same N: compiled is the reference
    fi

    if [ -z "$compiled_out" ]; then
        fail "  compiled: no output at n=$size"; ERRORS=$((ERRORS + 1)); continue
    fi
    if [ "$compiled_out" != "$reference" ]; then
        fail "  compiled: got '$compiled_out', expected '$reference'"; ERRORS=$((ERRORS + 1))
    fi
    if [ "$interp_out" = "$reference" ]; then
        ok "  agree: $interp_out"
    else
        fail "  interpreted: got '$interp_out', expected '$reference'"; ERRORS=$((ERRORS + 1))
    fi
done

if [ "$ERRORS" -gt 0 ]; then
    fail "Verification failed: $ERRORS errors"
    exit 1
fi
ok "Phase 2 complete: all agree"
fi

# Engine probe even when verification is skipped (cheap; needed for the table).
if [ "${VERIFY:-1}" = "0" ]; then
    for bench in "${BENCHMARKS[@]}"; do detect_engine "$bench"; done
fi

# ===========================================================================
# Phase 3: Benchmark — compiled vs interpreted, per-engine timeout isolation
# ===========================================================================
info "Phase 3: Benchmarking compiled vs interpreted ($WARMUP warmup, $RUNS runs)..."

PER_LANG_DIR="$RAW_DIR/per_lang"
mkdir -p "$PER_LANG_DIR"

merge_hyperfine_results() {
    local output_file="$1"; shift
    local files=("$@")
    if [ ${#files[@]} -eq 0 ]; then echo '{"results":[]}' > "$output_file"; return; fi
    jq -s '{ results: [.[].results[]] }' "${files[@]}" > "$output_file"
}

declare -A LANG_TIMEOUTS

# try_bench LANG_ID LABEL CMD [EXTRA_HYPERFINE_FLAGS]
# The compiled side passes --shell=none: its command is a bare binary, so this
# strips shell-startup noise that otherwise dominates sub-millisecond runs. The
# interpreted side needs the shell (cd into the project), so it omits the flag.
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
    [ -f "$BIN_DIR/${bench}_logos_release" ] || { warn "No compiled binary for $bench — skipping"; continue; }
    [ -n "${PROJ[$bench]:-}" ] || continue
    size="$(bench_size "$bench")"
    info "Benchmarking $bench at n=$size (interp engine: ${ENGINE[$bench]:-?})..."
    MERGE_FILES=()

    try_bench logos_release "LOGOS (compiled)" "$BIN_DIR/${bench}_logos_release $size" "--shell=none"
    try_bench interp "LOGOS (interpreted)" "cd \"${PROJ[$bench]}\" && \"$LARGO\" run --interpret $size"

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

LANGUAGES='[
  {"id":"logos_release","label":"LOGOS (compiled)","color":"#00d4ff","tier":"systems"},
  {"id":"interp","label":"LOGOS (interpreted)","color":"#ff8c00","tier":"interpreted"}
]'

lang_id() {
    case "$1" in
        "LOGOS (compiled)") echo logos_release ;;
        "LOGOS (interpreted)") echo interp ;;
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
    local size="$(bench_size "$bench")"
    local logos_src="" gen_rust=""
    [ -f "$PROGRAMS_DIR/$bench/main.lg" ] && logos_src=$(cat "$PROGRAMS_DIR/$bench/main.lg")
    [ -f "$GENERATED_DIR/$bench.rs" ]     && gen_rust=$(cat "$GENERATED_DIR/$bench.rs")

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
        --arg logos_src "$logos_src" --arg gen_rust "$gen_rust" \
        --arg sizes_str "$(cat "$PROGRAMS_DIR/$bench/sizes.txt")" \
        --argjson scaling "$scaling" \
        '{
            id: $id, name: $name, reference_size: $ref,
            interpreter_engine: $engine,
            sizes: ($sizes_str | split(" ")),
            logos_source: $logos_src, generated_rust: $gen_rust,
            scaling: $scaling, compilation: {}, timeouts: {}
        }'
}

BENCHMARKS_JSON="["
first=true
for bench in "${BENCHMARKS[@]}"; do
    if [ "$first" = true ]; then first=false; else BENCHMARKS_JSON+=","; fi
    BENCHMARKS_JSON+=$(assemble_benchmark "$bench")
done
BENCHMARKS_JSON+="]"

# Geometric mean: speedup of compiled over interpreted (interp_ms / compiled_ms).
compute_geometric_mean() {
    local log_sum=0 count=0
    for bench in "${BENCHMARKS[@]}"; do
        local ref="$(bench_size "$bench")"
        local c_file="$PER_LANG_DIR/${bench}_${ref}_logos_release.json"
        local i_file="$PER_LANG_DIR/${bench}_${ref}_interp.json"
        [ -f "$c_file" ] && [ -f "$i_file" ] || continue
        local c_ms i_ms
        c_ms=$(jq -r '.results[0].mean * 1000' "$c_file")
        i_ms=$(jq -r '.results[0].mean * 1000' "$i_file")
        if [ "$(echo "$c_ms > 0 && $i_ms > 0" | bc -l 2>/dev/null)" = "1" ]; then
            local ratio; ratio=$(echo "$i_ms / $c_ms" | bc -l)
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

GEO=$(compute_geometric_mean)

BENCHMARKS_JSON_FILE=$(mktemp)
printf '%s\n' "$BENCHMARKS_JSON" > "$BENCHMARKS_JSON_FILE"
mkdir -p "$(dirname "$OUT")"

jq -n \
    --arg date "$DATE" --arg commit "$COMMIT" --arg cpu "$CPU" --arg os "$OS" \
    --arg logos_version "$LOGOS_VER" \
    --argjson warmup "$WARMUP" --argjson runs "$RUNS" \
    --argjson languages "$LANGUAGES" \
    --slurpfile benchmarks "$BENCHMARKS_JSON_FILE" \
    --argjson geo "${GEO:-0}" \
    '{
        schema_version: 1,
        metadata: {
            date: $date, commit: $commit, logos_version: $logos_version,
            cpu: $cpu, os: $os, warmup: $warmup, runs: $runs
        },
        languages: $languages,
        benchmarks: $benchmarks[0],
        summary: { geometric_mean_speedup_compiled_vs_interp: $geo }
    }' > "$OUT"

rm -f "$BENCHMARKS_JSON_FILE"
ok "Results written to $OUT"

# ===========================================================================
# Phase 5: Speedup table — how much faster compiled is than interpreted
# ===========================================================================
echo
info "Compiled vs interpreted (speedup = interpreted ms / compiled ms):"
echo

ROWS=""
for bench in "${BENCHMARKS[@]}"; do
    size="$(bench_size "$bench")"
    c_file="$PER_LANG_DIR/${bench}_${size}_logos_release.json"
    i_file="$PER_LANG_DIR/${bench}_${size}_interp.json"
    c_ms="" i_ms=""
    [ -f "$c_file" ] && c_ms=$(jq -r '.results[0].mean * 1000' "$c_file")
    [ -f "$i_file" ] && i_ms=$(jq -r '.results[0].mean * 1000' "$i_file")
    if [ -n "$c_ms" ] && [ -n "$i_ms" ] && [ "$(echo "$c_ms > 0" | bc -l)" = "1" ]; then
        speedup=$(echo "$i_ms / $c_ms" | bc -l)
        ROWS+=$(printf "%s|%s|%s|%s|%s|%s\n" "$speedup" "$bench" "$size" "$c_ms" "$i_ms" "${ENGINE[$bench]:-?}")$'\n'
    else
        [ -z "$c_ms" ] && warn "  $bench: no compiled result"
        [ -z "$i_ms" ] && warn "  $bench: no interpreted result"
    fi
done

printf "%-14s %10s %12s %14s %9s  %s\n" "benchmark" "n" "compiled ms" "interpreted ms" "speedup" "engine"
printf '%.0s-' $(seq 1 78); echo
echo -n "$ROWS" | sort -t'|' -k1 -gr | while IFS='|' read -r speedup bench size c_ms i_ms engine; do
    [ -z "$bench" ] && continue
    printf "%-14s %10s %12.2f %14.2f %8.1fx  %s\n" "$bench" "$size" "$c_ms" "$i_ms" "$speedup" "$engine"
done

echo
if [ "$(echo "${GEO:-0} > 0" | bc -l)" = "1" ]; then
    info "Geometric mean: compiled LOGOS runs ${GEO}x faster than the interpreter"
fi
info "Full log: logs/optimization/logos-vs-interp.log"
info "LOGOS compiled vs interpreted benchmark complete!"
