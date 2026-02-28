#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE Quick Benchmark Suite
#
# Runs ALL 32 benchmarks at their reference size with enough runs for
# statistically accurate results. Produces results/latest.json identical
# in format to run.sh output. Skips debug builds, compilation benchmarks,
# and multi-size scaling sweeps.
#
# Usage: bash benchmarks/run-quick.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="$SCRIPT_DIR/bin"
PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw"
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

# Quick mode: reference sizes, enough runs for statistical accuracy
WARMUP=3
RUNS=10
TIMEOUT=120
BUILD_TIMEOUT=60
HYPERFINE_TIMEOUT=300  # 5 min safety net per hyperfine invocation (quick mode)
SKIP_LANGS="${SKIP_LANGS:-}"

skip_lang() {
    local lang="$1"
    echo ",$SKIP_LANGS," | grep -q ",$lang,"
}

mkdir -p "$BIN_DIR" "$RAW_DIR" "$RESULTS_DIR/history" "$GENERATED_DIR"
rm -rf "$RAW_DIR"/*

export LOGOS_WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
LOGOS_TARGET_DIR="$SCRIPT_DIR/.logos-bench-target"
mkdir -p "$LOGOS_TARGET_DIR"
export CARGO_TARGET_DIR="$LOGOS_TARGET_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

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

# Quick size: middle ground between smallest and reference.
# Big enough for real measurement (~5-50ms compiled, ~1-2s interpreted),
# small enough to keep the full suite under ~15 minutes.
quick_size() {
    case "$1" in
        fib) echo 25 ;; ackermann) echo 8 ;; nqueens) echo 10 ;;
        bubble_sort) echo 1000 ;; mergesort) echo 5000 ;; quicksort) echo 5000 ;;
        counting_sort) echo 50000 ;; heap_sort) echo 5000 ;;
        nbody) echo 5000 ;; mandelbrot) echo 200 ;; spectral_norm) echo 500 ;; pi_leibniz) echo 1000000 ;;
        gcd) echo 1000 ;; collatz) echo 100000 ;; primes) echo 50000 ;;
        sieve) echo 100000 ;; matrix_mult) echo 100 ;; prefix_sum) echo 100000 ;;
        array_reverse) echo 100000 ;; array_fill) echo 1000000 ;;
        collect) echo 10000 ;; two_sum) echo 5000 ;; histogram) echo 100000 ;;
        knapsack) echo 500 ;; coins) echo 5000 ;;
        fannkuch) echo 8 ;;
        strings) echo 10000 ;; binary_trees) echo 14 ;;
        loop_sum) echo 10000000 ;; fib_iterative) echo 10000000 ;;
        graph_bfs) echo 5000 ;; string_search) echo 50000 ;;
    esac
}

# ===========================================================================
# Phase 1: Build Everything
# ===========================================================================
info "Phase 1: Building all implementations..."

info "Building largo..."
cargo build -p logicaffeine-cli --release --manifest-path "$SCRIPT_DIR/../Cargo.toml" 2>/dev/null
LARGO="$LOGOS_TARGET_DIR/release/logicaffeine-cli"
if [ ! -f "$LARGO" ]; then
    LARGO="$LOGOS_TARGET_DIR/release/largo"
fi
if [ ! -f "$LARGO" ]; then
    LARGO="$SCRIPT_DIR/../target/release/logicaffeine-cli"
fi
if [ ! -f "$LARGO" ]; then
    LARGO="$SCRIPT_DIR/../target/release/largo"
fi
if [ ! -f "$LARGO" ]; then
    fail "Could not find largo binary"
    exit 1
fi
ok "largo built"

for bench in "${BENCHMARKS[@]}"; do
    info "Building $bench..."

    [ -f "$PROGRAMS_DIR/$bench/main.c" ] && \
        gcc -O2 -o "$BIN_DIR/${bench}_c" "$PROGRAMS_DIR/$bench/main.c" -lm 2>/dev/null && \
        ok "  C" || true

    [ -f "$PROGRAMS_DIR/$bench/main.cpp" ] && \
        g++ -O2 -std=c++17 -o "$BIN_DIR/${bench}_cpp" "$PROGRAMS_DIR/$bench/main.cpp" -lm 2>/dev/null && \
        ok "  C++" || true

    [ -f "$PROGRAMS_DIR/$bench/main.rs" ] && \
        rustc --edition 2021 -O -o "$BIN_DIR/${bench}_rs" "$PROGRAMS_DIR/$bench/main.rs" 2>/dev/null && \
        ok "  Rust" || true

    if [ -f "$PROGRAMS_DIR/$bench/main.zig" ] && command -v zig &>/dev/null && ! skip_lang zig; then
        run_timeout "$BUILD_TIMEOUT" zig build-exe -O ReleaseFast --name "${bench}_zig" "$PROGRAMS_DIR/$bench/main.zig" 2>/dev/null && \
            mv "${bench}_zig" "$BIN_DIR/" 2>/dev/null && ok "  Zig" || true
    fi

    [ -f "$PROGRAMS_DIR/$bench/main.go" ] && \
        go build -o "$BIN_DIR/${bench}_go" "$PROGRAMS_DIR/$bench/main.go" 2>/dev/null && \
        ok "  Go" || true

    if [ -f "$PROGRAMS_DIR/$bench/Main.java" ]; then
        mkdir -p "$BIN_DIR/java/$bench"
        javac -d "$BIN_DIR/java/$bench" "$PROGRAMS_DIR/$bench/Main.java" 2>/dev/null && ok "  Java" || true
    fi

    if [ -f "$PROGRAMS_DIR/$bench/main.nim" ] && command -v nim &>/dev/null; then
        nim c -d:release --hints:off -o:"$BIN_DIR/${bench}_nim" "$PROGRAMS_DIR/$bench/main.nim" 2>/dev/null && ok "  Nim" || true
    fi

    if [ -f "$PROGRAMS_DIR/$bench/main.lg" ]; then
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
            [ -n "$LOGOS_BIN" ] && cp "$LOGOS_BIN" "$BIN_DIR/${bench}_logos_release" && ok "  LOGOS (release)" || true
            GENERATED_RS=$(find "$LOGOS_TMP" -name "main.rs" -path "*/build/src/*" 2>/dev/null | head -1)
            [ -n "$GENERATED_RS" ] && cp "$GENERATED_RS" "$GENERATED_DIR/$bench.rs" 2>/dev/null || true
        } || true
        rm -rf "$LOGOS_TMP"
    fi
done

ok "Phase 1 complete"

# ===========================================================================
# Phase 2: Verify + Benchmark (combined — quick mode)
# ===========================================================================
info "Phase 2: Verifying and benchmarking at quick sizes..."

ERRORS=0

verify() {
    local bench="$1" name="$2" cmd="$3" size="$4" expected="$5"
    local output
    output=$(run_timeout "$TIMEOUT" bash -c "$cmd $size" 2>/dev/null | tr -d '[:space:]') || true
    expected=$(echo "$expected" | tr -d '[:space:]')
    if [ "$output" = "$expected" ]; then
        ok "  $name: $output"
    else
        fail "  $name: got '$output', expected '$expected'"
        ERRORS=$((ERRORS + 1))
    fi
}

for bench in "${BENCHMARKS[@]}"; do
    size="$(quick_size "$bench")"
    expected_file="$PROGRAMS_DIR/$bench/expected_${size}.txt"
    if [ ! -f "$expected_file" ]; then
        warn "No expected output for $bench at size $size"
        continue
    fi
    expected=$(cat "$expected_file")
    info "Verifying $bench (n=$size)..."

    [ -f "$BIN_DIR/${bench}_c" ]   && verify "$bench" "C" "$BIN_DIR/${bench}_c" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_cpp" ] && verify "$bench" "C++" "$BIN_DIR/${bench}_cpp" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_rs" ]  && verify "$bench" "Rust" "$BIN_DIR/${bench}_rs" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_zig" ] && verify "$bench" "Zig" "$BIN_DIR/${bench}_zig" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_go" ]  && verify "$bench" "Go" "$BIN_DIR/${bench}_go" "$size" "$expected"
    [ -d "$BIN_DIR/java/$bench" ]  && verify "$bench" "Java" "java -cp $BIN_DIR/java/$bench Main" "$size" "$expected"
    [ -f "$PROGRAMS_DIR/$bench/main.js" ]  && verify "$bench" "JavaScript" "node $PROGRAMS_DIR/$bench/main.js" "$size" "$expected"
    [ -f "$PROGRAMS_DIR/$bench/main.py" ]  && verify "$bench" "Python" "python3 $PROGRAMS_DIR/$bench/main.py" "$size" "$expected"
    [ -f "$PROGRAMS_DIR/$bench/main.rb" ]  && verify "$bench" "Ruby" "RUBY_THREAD_VM_STACK_SIZE=67108864 ruby $PROGRAMS_DIR/$bench/main.rb" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_nim" ] && verify "$bench" "Nim" "$BIN_DIR/${bench}_nim" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_logos_release" ] && verify "$bench" "LOGOS (release)" "$BIN_DIR/${bench}_logos_release" "$size" "$expected"
done

if [ "$ERRORS" -gt 0 ]; then
    fail "Verification failed: $ERRORS errors"
    exit 1
fi
ok "Phase 2 complete: all verified"

# ===========================================================================
# Phase 3: Quick Benchmark (smallest size, 3 runs)
# ===========================================================================
info "Phase 3: Quick benchmarking..."

for bench in "${BENCHMARKS[@]}"; do
    size="$(quick_size "$bench")"
    info "Benchmarking $bench at n=$size..."

    HYPERFINE_ARGS=(
        --warmup "$WARMUP"
        --runs "$RUNS"
        --timeout "$TIMEOUT"
        --export-json "$RAW_DIR/${bench}_${size}.json"
        --time-unit millisecond
    )

    [ -f "$BIN_DIR/${bench}_c" ]   && HYPERFINE_ARGS+=(-n "C" "$BIN_DIR/${bench}_c $size")
    [ -f "$BIN_DIR/${bench}_cpp" ] && HYPERFINE_ARGS+=(-n "C++" "$BIN_DIR/${bench}_cpp $size")
    [ -f "$BIN_DIR/${bench}_rs" ]  && HYPERFINE_ARGS+=(-n "Rust" "$BIN_DIR/${bench}_rs $size")
    [ -f "$BIN_DIR/${bench}_zig" ] && HYPERFINE_ARGS+=(-n "Zig" "$BIN_DIR/${bench}_zig $size")
    [ -f "$BIN_DIR/${bench}_go" ]  && HYPERFINE_ARGS+=(-n "Go" "$BIN_DIR/${bench}_go $size")
    [ -d "$BIN_DIR/java/$bench" ]  && HYPERFINE_ARGS+=(-n "Java" "java -cp $BIN_DIR/java/$bench Main $size")
    [ -f "$PROGRAMS_DIR/$bench/main.js" ]  && HYPERFINE_ARGS+=(-n "JavaScript" "node $PROGRAMS_DIR/$bench/main.js $size")
    [ -f "$PROGRAMS_DIR/$bench/main.py" ]  && HYPERFINE_ARGS+=(-n "Python" "python3 $PROGRAMS_DIR/$bench/main.py $size")
    [ -f "$PROGRAMS_DIR/$bench/main.rb" ]  && HYPERFINE_ARGS+=(-n "Ruby" "RUBY_THREAD_VM_STACK_SIZE=67108864 ruby $PROGRAMS_DIR/$bench/main.rb $size")
    [ -f "$BIN_DIR/${bench}_nim" ] && HYPERFINE_ARGS+=(-n "Nim" "$BIN_DIR/${bench}_nim $size")
    [ -f "$BIN_DIR/${bench}_logos_release" ] && HYPERFINE_ARGS+=(-n "LOGOS (release)" "$BIN_DIR/${bench}_logos_release $size")

    rc=0
    run_timeout "$HYPERFINE_TIMEOUT" hyperfine "${HYPERFINE_ARGS[@]}" || rc=$?
    if [ "$rc" -eq 124 ] || [ "$rc" -eq 143 ]; then
        warn "hyperfine timed out for $bench at $size (>${HYPERFINE_TIMEOUT}s)"
        echo "$HYPERFINE_TIMEOUT" > "$RAW_DIR/${bench}_${size}.timeout"
    elif [ "$rc" -ne 0 ]; then
        warn "hyperfine failed for $bench at $size"
    fi

    # Detect per-command timeouts (hyperfine --timeout killed individual commands)
    if [ -f "$RAW_DIR/${bench}_${size}.json" ] && \
       jq -e '[.results[] | select(.mean == null)] | length > 0' "$RAW_DIR/${bench}_${size}.json" &>/dev/null; then
        warn "per-command timeout detected for $bench at $size"
        [ ! -f "$RAW_DIR/${bench}_${size}.timeout" ] && echo "$TIMEOUT" > "$RAW_DIR/${bench}_${size}.timeout"
        jq '.results = [.results[] | select(.mean != null)]' "$RAW_DIR/${bench}_${size}.json" > "$RAW_DIR/${bench}_${size}.json.tmp" && \
            mv "$RAW_DIR/${bench}_${size}.json.tmp" "$RAW_DIR/${bench}_${size}.json"
    fi
done

ok "Phase 3 complete"

# ===========================================================================
# Phase 4: Assemble Results JSON (same format as run.sh)
# ===========================================================================
info "Phase 4: Assembling results JSON..."

detect_cpu() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "$(uname -m)"
    elif [ -f /proc/cpuinfo ]; then
        grep -m1 'model name' /proc/cpuinfo 2>/dev/null | sed 's/.*: //' || echo "$(uname -m)"
    else
        uname -m
    fi
}

detect_os() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        local name version
        name=$(sw_vers -productName 2>/dev/null || echo "macOS")
        version=$(sw_vers -productVersion 2>/dev/null || echo "")
        echo "$name $version $(uname -m)"
    elif [ -f /etc/os-release ]; then
        local pretty
        pretty=$(grep -m1 PRETTY_NAME /etc/os-release 2>/dev/null | sed 's/PRETTY_NAME="\?\([^"]*\)"\?/\1/')
        echo "${pretty:-Linux} $(uname -m)"
    else
        echo "$(uname -s) $(uname -r) $(uname -m)"
    fi
}

DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
COMMIT=$(cd "$SCRIPT_DIR/.." && git rev-parse --short HEAD 2>/dev/null || echo "unknown")

CPU=$(detect_cpu)
if [ "${GITHUB_ACTIONS:-}" = "true" ]; then
    CPU="$CPU (GitHub Actions)"
fi

OS=$(detect_os)

if [ -n "${LOGOS_VERSION:-}" ]; then
    LOGOS_VER="$LOGOS_VERSION"
else
    LOGOS_VER=$(grep '^version' "$SCRIPT_DIR/../apps/logicaffeine_cli/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
fi

get_version() {
    case "$1" in
        c)      gcc --version 2>/dev/null | head -1 || echo "unknown" ;;
        cpp)    g++ --version 2>/dev/null | head -1 || echo "unknown" ;;
        rust)   rustc --version 2>/dev/null || echo "unknown" ;;
        zig)    zig version 2>/dev/null || echo "unknown" ;;
        go)     go version 2>/dev/null || echo "unknown" ;;
        java)   java --version 2>/dev/null | head -1 || echo "unknown" ;;
        node)   node --version 2>/dev/null || echo "unknown" ;;
        python) python3 --version 2>/dev/null || echo "unknown" ;;
        ruby)   ruby --version 2>/dev/null || echo "unknown" ;;
        nim)    nim --version 2>/dev/null | head -1 || echo "unknown" ;;
    esac
}

VERSIONS=$(jq -n \
    --arg c "$(get_version c)" \
    --arg cpp "$(get_version cpp)" \
    --arg rust "$(get_version rust)" \
    --arg zig "$(get_version zig)" \
    --arg go "$(get_version go)" \
    --arg java "$(get_version java)" \
    --arg node "$(get_version node)" \
    --arg python "$(get_version python)" \
    --arg ruby "$(get_version ruby)" \
    --arg nim "$(get_version nim)" \
    '{c: $c, cpp: $cpp, rust: $rust, zig: $zig, go: $go, java: $java, node: $node, python: $python, ruby: $ruby, nim: $nim}')

LANGUAGES='[
  {"id":"c","label":"C","color":"#555555","tier":"systems"},
  {"id":"cpp","label":"C++","color":"#f34b7d","tier":"systems"},
  {"id":"rust","label":"Rust","color":"#dea584","tier":"systems"},
  {"id":"zig","label":"Zig","color":"#f7a41d","tier":"systems"},
  {"id":"logos_release","label":"LOGOS","color":"#00d4ff","tier":"systems"},
  {"id":"go","label":"Go","color":"#00ADD8","tier":"managed"},
  {"id":"java","label":"Java","color":"#b07219","tier":"managed"},
  {"id":"js","label":"JavaScript","color":"#f7df1e","tier":"managed"},
  {"id":"python","label":"Python","color":"#3776ab","tier":"interpreted"},
  {"id":"ruby","label":"Ruby","color":"#cc342d","tier":"interpreted"},
  {"id":"nim","label":"Nim","color":"#ffe953","tier":"transpiled"}
]'

lang_id() {
    case "$1" in
        "C") echo c ;; "C++") echo cpp ;; "Rust") echo rust ;; "Zig") echo zig ;;
        "Go") echo go ;; "Java") echo java ;; "JavaScript") echo js ;;
        "Python") echo python ;; "Ruby") echo ruby ;; "Nim") echo nim ;;
        "LOGOS (release)") echo logos_release ;;
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

bench_desc() {
    case "$1" in
        fib) echo "Naive recursive fibonacci. Measures function call overhead and recursion depth." ;;
        ackermann) echo "Ackermann(3, m). Measures extreme recursion depth and stack frame overhead." ;;
        nqueens) echo "N-Queens backtracking. Measures recursive constraint solving." ;;
        bubble_sort) echo "O(n^2) bubble sort. Measures nested loops, indexed array mutation, and swap patterns." ;;
        mergesort) echo "Top-down merge sort. Measures allocation-heavy divide-and-conquer." ;;
        quicksort) echo "Lomuto-partition quicksort. Measures in-place swap-heavy recursion." ;;
        counting_sort) echo "Non-comparison O(n+k) sort. Measures pure array indexing throughput." ;;
        heap_sort) echo "Heap sort with sift-down. Measures logarithmic array jumps." ;;
        nbody) echo "5-body gravitational simulation. Measures FP struct arrays and sqrt." ;;
        mandelbrot) echo "Mandelbrot set escape iteration. Measures FP branching and convergence." ;;
        spectral_norm) echo "Spectral norm power method. Measures FP dot products and array throughput." ;;
        pi_leibniz) echo "Leibniz series for pi. Measures pure FP loop overhead." ;;
        gcd) echo "GCD sum via Euclidean algorithm. Measures modulo-heavy tight loops." ;;
        collatz) echo "Collatz step counting. Measures unpredictable branching." ;;
        primes) echo "Trial division prime counting. Measures nested loops with early exit." ;;
        sieve) echo "Classic prime sieve. Measures indexed array mutation and tight loops." ;;
        matrix_mult) echo "O(n^3) matrix multiply. Measures cache locality and triple-nested loops." ;;
        prefix_sum) echo "Sequential prefix sum scan. Measures read-modify-write bandwidth." ;;
        array_reverse) echo "Two-pointer in-place reversal. Measures strided cache access." ;;
        array_fill) echo "Array push and sum. Measures raw memory bandwidth and allocation." ;;
        collect) echo "Hash map insert and lookup. Measures hash computation and cache behavior." ;;
        two_sum) echo "Interleaved hash insert+lookup. Measures hash table under mixed workload." ;;
        histogram) echo "Array-indexed frequency counting. Measures random array access." ;;
        knapsack) echo "0/1 knapsack DP. Measures 2D table fills and conditional max." ;;
        coins) echo "Coin change DP. Measures 1D DP with inner-loop additions." ;;
        fannkuch) echo "Fannkuch permutation benchmark. Measures tight array reversal loops." ;;
        strings) echo "String concatenation and assembly. Measures allocator throughput and GC pressure." ;;
        binary_trees) echo "Recursive tree creation and checksum. Measures allocation pressure." ;;
        loop_sum) echo "Pure loop accumulation. Measures raw loop overhead with minimal body." ;;
        fib_iterative) echo "Iterative fibonacci mod. Measures loop + data dependency chain." ;;
        graph_bfs) echo "BFS on generated graph. Measures queue operations and random access." ;;
        string_search) echo "Naive O(nm) string search. Measures character-level access and inner loop." ;;
    esac
}

extract_name() {
    echo "$1" | jq -r '.command' 2>/dev/null
}

assemble_benchmark() {
    local bench="$1"
    local size="$(quick_size "$bench")"
    local logos_src=""
    local gen_rust=""

    [ -f "$PROGRAMS_DIR/$bench/main.lg" ] && logos_src=$(cat "$PROGRAMS_DIR/$bench/main.lg")
    [ -f "$GENERATED_DIR/$bench.rs" ] && gen_rust=$(cat "$GENERATED_DIR/$bench.rs")

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
            else
                user_ms="null"
            fi
            if [ "$system_s" != "null" ] && [ -n "$system_s" ]; then
                system_ms=$(echo "$system_s * 1000" | bc -l 2>/dev/null || echo "null")
            else
                system_ms="null"
            fi
            if [ "$(echo "$mean_ms > 0" | bc -l 2>/dev/null)" = "1" ]; then
                cv=$(echo "$stddev_ms / $mean_ms" | bc -l 2>/dev/null || echo "0")
            else
                cv="0"
            fi
            size_data=$(echo "$size_data" | jq \
                --arg lid "$lid" \
                --argjson mean "$mean_ms" \
                --argjson median "$median_ms" \
                --argjson stddev "$stddev_ms" \
                --argjson min "$min_ms" \
                --argjson max "$max_ms" \
                --argjson cv "$cv" \
                --argjson runs "$RUNS" \
                --argjson user "$user_ms" \
                --argjson sys "$system_ms" \
                '.[$lid] = {mean_ms: $mean, median_ms: $median, stddev_ms: $stddev, min_ms: $min, max_ms: $max, cv: $cv, runs: $runs, user_ms: $user, system_ms: $sys}')
        done <<< "$results"
        scaling=$(echo "$scaling" | jq --arg size "$size" --argjson data "$size_data" '.[$size] = $data')
    fi

    # Collect timeout markers
    local timeouts="{}"
    if [ -f "$RAW_DIR/${bench}_${size}.timeout" ]; then
        local timeout_secs
        timeout_secs=$(cat "$RAW_DIR/${bench}_${size}.timeout")
        local timeout_ms
        timeout_ms=$(echo "$timeout_secs * 1000" | bc -l 2>/dev/null || echo "600000")
        timeouts=$(echo "$timeouts" | jq --arg size "$size" --argjson ms "$timeout_ms" '.[$size] = $ms')
    fi

    jq -n \
        --arg id "$bench" \
        --arg name "$(bench_name "$bench")" \
        --arg desc "$(bench_desc "$bench")" \
        --arg ref "$size" \
        --arg logos_src "$logos_src" \
        --arg gen_rust "$gen_rust" \
        --arg sizes_str "$(cat "$PROGRAMS_DIR/$bench/sizes.txt")" \
        --argjson scaling "$scaling" \
        --argjson timeouts "$timeouts" \
        '{
            id: $id,
            name: $name,
            description: $desc,
            reference_size: $ref,
            sizes: ($sizes_str | split(" ")),
            logos_source: $logos_src,
            generated_rust: $gen_rust,
            scaling: $scaling,
            compilation: {},
            timeouts: $timeouts
        }'
}

BENCHMARKS_JSON="["
first=true
for bench in "${BENCHMARKS[@]}"; do
    if [ "$first" = true ]; then first=false; else BENCHMARKS_JSON+=","; fi
    BENCHMARKS_JSON+=$(assemble_benchmark "$bench")
done
BENCHMARKS_JSON+="]"

# Geometric mean speedup vs C
compute_geometric_mean() {
    local benchmarks_json="$1"
    local all_langs
    all_langs=$(echo "$benchmarks_json" | jq -r '
        [.[] | .scaling[.reference_size] // {} | keys[]] | unique | .[]
    ' 2>/dev/null)
    local geo_mean="{}"
    for lang in $all_langs; do
        local log_sum=0 count=0
        for bench in "${BENCHMARKS[@]}"; do
            local ref="$(quick_size "$bench")"
            local c_mean lang_mean
            c_mean=$(echo "$benchmarks_json" | jq -r --arg ref "$ref" --arg bench "$bench" \
                '.[] | select(.id == $bench) | .scaling[$ref].c.mean_ms // 0')
            lang_mean=$(echo "$benchmarks_json" | jq -r --arg ref "$ref" --arg bench "$bench" --arg lang "$lang" \
                '.[] | select(.id == $bench) | .scaling[$ref][$lang].mean_ms // 0')
            if [ "$(echo "$c_mean > 0" | bc -l 2>/dev/null)" = "1" ] && \
               [ "$(echo "$lang_mean > 0" | bc -l 2>/dev/null)" = "1" ]; then
                local ratio log_ratio
                ratio=$(echo "$c_mean / $lang_mean" | bc -l 2>/dev/null || echo "0")
                if [ "$(echo "$ratio > 0" | bc -l 2>/dev/null)" = "1" ]; then
                    log_ratio=$(echo "l($ratio)" | bc -l 2>/dev/null || echo "0")
                    log_sum=$(echo "$log_sum + $log_ratio" | bc -l 2>/dev/null || echo "0")
                    count=$((count + 1))
                fi
            fi
        done
        if [ "$count" -gt 0 ]; then
            local avg_log geo
            avg_log=$(echo "$log_sum / $count" | bc -l 2>/dev/null || echo "0")
            geo=$(echo "e($avg_log)" | bc -l 2>/dev/null || echo "0")
            geo=$(printf "%.3f" "$geo")
            geo_mean=$(echo "$geo_mean" | jq --arg lang "$lang" --argjson val "$geo" '.[$lang] = $val')
        fi
    done
    echo "$geo_mean"
}

GEO_MEAN=$(compute_geometric_mean "$BENCHMARKS_JSON")

BENCHMARKS_JSON_FILE=$(mktemp)
printf '%s\n' "$BENCHMARKS_JSON" > "$BENCHMARKS_JSON_FILE"

jq -n \
    --arg date "$DATE" \
    --arg commit "$COMMIT" \
    --arg cpu "$CPU" \
    --arg os "$OS" \
    --arg logos_version "$LOGOS_VER" \
    --argjson warmup "$WARMUP" \
    --argjson runs "$RUNS" \
    --argjson versions "$VERSIONS" \
    --argjson languages "$LANGUAGES" \
    --slurpfile benchmarks "$BENCHMARKS_JSON_FILE" \
    --argjson geo_mean "$GEO_MEAN" \
    '{
        schema_version: 1,
        metadata: {
            date: $date,
            commit: $commit,
            logos_version: $logos_version,
            cpu: $cpu,
            os: $os,
            warmup: $warmup,
            runs: $runs,
            versions: $versions
        },
        languages: $languages,
        benchmarks: $benchmarks[0],
        summary: {
            geometric_mean_speedup_vs_c: $geo_mean
        }
    }' > "$RESULTS_DIR/latest.json"

rm -f "$BENCHMARKS_JSON_FILE"

ok "Results written to $RESULTS_DIR/latest.json"
info "Quick benchmark suite complete!"
