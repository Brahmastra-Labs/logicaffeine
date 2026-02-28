#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE LOGOS vs C Benchmark
#
# Builds and benchmarks LOGOS (release) against C at the maximum sizes defined
# in each benchmark's sizes.txt. Large sizes eliminate process-startup noise
# from the comparison. Results are written to results/c_results.json in the
# same schema as results/latest.json.
#
# Usage: bash benchmarks/run-logos-vs-c.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="$SCRIPT_DIR/bin"
PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw-logos-vs-c"
GENERATED_DIR="$SCRIPT_DIR/generated"
ASM_DIR="$SCRIPT_DIR/asm"

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

WARMUP=5
RUNS=30
TIMEOUT=180
BUILD_TIMEOUT=60
HYPERFINE_TIMEOUT=600  # 10 min per hyperfine invocation

mkdir -p "$BIN_DIR" "$RAW_DIR" "$RESULTS_DIR/history" "$GENERATED_DIR" "$ASM_DIR"
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

# Last entry in sizes.txt; used as fallback by bench_size().
max_size() {
    local bench="$1"
    local sizes
    sizes=$(cat "$PROGRAMS_DIR/$bench/sizes.txt")
    echo "$sizes" | tr ' ' '\n' | tail -1
}

# Calibrated benchmark size targeting 100–2000ms C runtime.
# Eliminates process-startup noise that dominates at very small sizes.
# Falls back to max_size() for any unknown benchmark.
bench_size() {
    local bench="$1"
    case "$bench" in
        # Verified C medians in parentheses (measured on Apple Silicon)
        fib)             echo 40 ;;          # ~1271ms (exponential)
        ackermann)       echo 11 ;;          # ~570ms  (exponential)
        nqueens)         echo 14 ;;          # ~498ms  (super-exponential)
        bubble_sort)     echo 30000 ;;       # ~969ms  (O(n²))
        mergesort)       echo 3000000 ;;     # ~509ms  (O(n log n))
        quicksort)       echo 3000000 ;;     # ~577ms  (O(n log n))
        counting_sort)   echo 40000000 ;;    # ~675ms  (O(n))
        heap_sort)       echo 2500000 ;;     # ~726ms  (O(n log n))
        nbody)           echo 4000000 ;;     # ~521ms  (O(n))
        mandelbrot)      echo 2000 ;;        # ~558ms  (O(n²))
        spectral_norm)   echo 5000 ;;        # ~1445ms (O(n²))
        pi_leibniz)      echo 200000000 ;;   # ~200ms  (O(n))
        gcd)             echo 5000 ;;        # ~599ms  (O(n²))
        collatz)         echo 5000000 ;;     # ~1033ms (O(n))
        primes)          echo 3000000 ;;     # ~622ms  (O(n√n))
        sieve)           echo 100000000 ;;   # ~790ms  (O(n))
        matrix_mult)     echo 700 ;;         # ~613ms  (O(n³))
        prefix_sum)      echo 50000000 ;;    # ~718ms  (O(n))
        array_reverse)   echo 50000000 ;;    # ~483ms  (O(n))
        array_fill)      echo 50000000 ;;    # ~587ms  (O(n))
        histogram)       echo 100000000 ;;   # ~163ms  (O(n))
        knapsack)        echo 9000 ;;        # ~648ms  (O(n²))
        fannkuch)        echo 10 ;;          # ~191ms  (O(n·n!))
        binary_trees)    echo 30 ;;          # ~2ms (closed-form O(n))
        loop_sum)        echo 500000000 ;;   # ~1802ms (O(n))
        fib_iterative)   echo 500000000 ;;   # ~1881ms (O(n))
        # Hash-heavy: dynamic C hash tables scale with n
        collect)         echo 30000000 ;;    # ~620ms (O(n) hash insert+lookup)
        two_sum)         echo 60000000 ;;    # ~540ms (O(n) hash insert+lookup)
        # String/graph-heavy
        coins)           echo 10000000 ;;    # ~150ms (O(6n) DP)
        strings)         echo 5000000 ;;     # ~350ms (O(n) string concat)
        graph_bfs)       echo 3000000 ;;     # ~280ms (O(V+E) BFS)
        string_search)   echo 50000000 ;;    # ~145ms (O(5n) naive search)
        *)               max_size "$bench" ;;
    esac
}

# ===========================================================================
# Phase 1: Build
# ===========================================================================
info "Phase 1: Building LOGOS (release) and C..."

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

    # Assembly + LLVM IR dumps for C (requires clang)
    if [ -f "$PROGRAMS_DIR/$bench/main.c" ] && command -v clang &>/dev/null; then
        clang -S -O2 -fno-asynchronous-unwind-tables \
            -o "$ASM_DIR/${bench}_c.s" "$PROGRAMS_DIR/$bench/main.c" -lm 2>/dev/null && \
            ok "  C asm" || true
        clang -S -emit-llvm -O2 \
            -o "$ASM_DIR/${bench}_c.ll" "$PROGRAMS_DIR/$bench/main.c" -lm 2>/dev/null && \
            ok "  C llvm-ir" || true
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
            [ -n "$LOGOS_BIN" ] && rm -f "$BIN_DIR/${bench}_logos_release" && cp "$LOGOS_BIN" "$BIN_DIR/${bench}_logos_release" && ok "  LOGOS (release)" || true
            GENERATED_RS=$(find "$LOGOS_TMP" -name "main.rs" -path "*/build/src/*" 2>/dev/null | head -1)
            [ -n "$GENERATED_RS" ] && cp "$GENERATED_RS" "$GENERATED_DIR/$bench.rs" 2>/dev/null || true

            # Assembly + LLVM IR dumps for Logos/Rust
            RUST_PROJECT=$(find "$LOGOS_TMP" -name "Cargo.toml" -path "*/build/Cargo.toml" 2>/dev/null | head -1)
            if [ -n "$RUST_PROJECT" ]; then
                RUST_PROJECT_DIR=$(dirname "$RUST_PROJECT")
                # Disable strip so symbols appear in assembly
                sed -i.bak 's/strip = true/strip = false/' "$RUST_PROJECT" 2>/dev/null || true

                # Emit assembly
                (cd "$RUST_PROJECT_DIR" && \
                    cargo rustc --release -- --emit=asm 2>/dev/null) && {
                    ASM_FILE=$(find "$RUST_PROJECT_DIR/target/release/deps" -name "bench-*.s" 2>/dev/null | head -1)
                    if [ -z "$ASM_FILE" ] && [ -n "${CARGO_TARGET_DIR:-}" ]; then
                        ASM_FILE=$(find "$CARGO_TARGET_DIR/release/deps" -name "bench-*.s" 2>/dev/null | head -1)
                    fi
                    [ -n "$ASM_FILE" ] && cp "$ASM_FILE" "$ASM_DIR/${bench}_logos.s" && ok "  LOGOS asm" || true
                } || true

                # Emit LLVM IR
                (cd "$RUST_PROJECT_DIR" && \
                    cargo rustc --release -- --emit=llvm-ir 2>/dev/null) && {
                    LL_FILE=$(find "$RUST_PROJECT_DIR/target/release/deps" -name "bench-*.ll" 2>/dev/null | head -1)
                    if [ -z "$LL_FILE" ] && [ -n "${CARGO_TARGET_DIR:-}" ]; then
                        LL_FILE=$(find "$CARGO_TARGET_DIR/release/deps" -name "bench-*.ll" 2>/dev/null | head -1)
                    fi
                    [ -n "$LL_FILE" ] && cp "$LL_FILE" "$ASM_DIR/${bench}_logos.ll" && ok "  LOGOS llvm-ir" || true
                } || true
            fi
        } || true
        rm -rf "$LOGOS_TMP"
    fi
done

ok "Phase 1 complete"

# ===========================================================================
# Phase 2: Verify Correctness
# ===========================================================================
info "Phase 2: Verifying correctness at calibrated sizes..."

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
    size="$(bench_size "$bench")"
    expected_file="$PROGRAMS_DIR/$bench/expected_${size}.txt"
    if [ ! -f "$expected_file" ]; then
        warn "No expected output for $bench at size $size — skipping verification"
        continue
    fi
    expected=$(cat "$expected_file")
    info "Verifying $bench (n=$size)..."

    [ -f "$BIN_DIR/${bench}_c" ]             && verify "$bench" "C" "$BIN_DIR/${bench}_c" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_logos_release" ] && verify "$bench" "LOGOS (release)" "$BIN_DIR/${bench}_logos_release" "$size" "$expected"
done

if [ "$ERRORS" -gt 0 ]; then
    fail "Verification failed: $ERRORS errors"
    exit 1
fi
ok "Phase 2 complete: all verified"

# ===========================================================================
# Phase 3: Benchmark
# ===========================================================================
info "Phase 3: Benchmarking LOGOS (release) vs C at calibrated sizes ($WARMUP warmup, $RUNS runs)..."

for bench in "${BENCHMARKS[@]}"; do
    size="$(bench_size "$bench")"

    # Skip benchmarks where neither binary exists
    if [ ! -f "$BIN_DIR/${bench}_c" ] && [ ! -f "$BIN_DIR/${bench}_logos_release" ]; then
        warn "No binaries for $bench — skipping"
        continue
    fi

    info "Benchmarking $bench at n=$size..."

    HYPERFINE_ARGS=(
        --warmup "$WARMUP"
        --runs "$RUNS"
        --timeout "$TIMEOUT"
        --export-json "$RAW_DIR/${bench}_${size}.json"
        --time-unit millisecond
    )

    [ -f "$BIN_DIR/${bench}_c" ]             && HYPERFINE_ARGS+=(-n "C" "$BIN_DIR/${bench}_c $size")
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
# Phase 4: Assemble c_results.json
# ===========================================================================
info "Phase 4: Assembling results/c_results.json..."

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

VERSIONS=$(jq -n \
    --arg c "$(gcc --version 2>/dev/null | head -1 || echo "unknown")" \
    '{c: $c}')

LANGUAGES='[
  {"id":"c","label":"C","color":"#555555","tier":"systems"},
  {"id":"logos_release","label":"LOGOS","color":"#00d4ff","tier":"systems"}
]'

lang_id() {
    case "$1" in
        "C") echo c ;;
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
            local ref="$(bench_size "$bench")"
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
    }' > "$RESULTS_DIR/c_results.json"

rm -f "$BENCHMARKS_JSON_FILE"

ok "Results written to $RESULTS_DIR/c_results.json"
info "LOGOS vs C benchmark complete!"
