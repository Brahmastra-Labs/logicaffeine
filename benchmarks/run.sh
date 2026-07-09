#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE Cross-Language Benchmark Suite
#
# Builds, verifies, and benchmarks implementations across 11 languages.
# Produces results/latest.json consumed by the /benchmarks website page.
#
# Requirements: gcc, g++, rustc, zig, go, java/javac, node, python3, ruby, nim, hyperfine, jq, bc
#
# Usage: bash benchmarks/run.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="$SCRIPT_DIR/bin"
PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw"
GENERATED_DIR="$SCRIPT_DIR/generated"

BENCHMARKS=(
    # Recursion & Function Calls
    fib ackermann nqueens
    # Sorting
    bubble_sort mergesort quicksort counting_sort heap_sort
    # Floating Point
    nbody mandelbrot spectral_norm pi_leibniz
    # Integer Mathematics
    gcd collatz primes
    # Array Patterns
    sieve matrix_mult prefix_sum array_reverse array_fill
    # Hash Maps & Lookup
    collect two_sum histogram
    # Dynamic Programming
    knapsack coins
    # Combinatorial
    fannkuch
    # Memory & Allocation
    strings binary_trees
    # Loop Overhead & Control Flow
    loop_sum fib_iterative graph_bfs string_search
)

WARMUP="${BENCH_WARMUP:-2}"
RUNS="${BENCH_RUNS:-10}"
TIMEOUT="${BENCH_TIMEOUT:-120}"
BUILD_TIMEOUT="${BUILD_TIMEOUT:-60}"
HYPERFINE_TIMEOUT="${BENCH_HYPERFINE_TIMEOUT:-300}"  # 5 min per hyperfine invocation
SIZES_MODE="${BENCH_SIZES:-all}"  # "all" or "ref" (reference only)
SKIP_LANGS="${SKIP_LANGS:-}"      # comma-separated list of langs to skip (e.g., "zig,nim")
RUN_INTERP="${RUN_INTERP:-1}"     # 1 = also run the LOGOS-interpreter vs Node/V8 suite (Phase 6) -> results/latest-interp.json
RUN_CODEC="${RUN_CODEC:-1}"       # 1 = also run the wire-codec head-to-head (Phase 7) -> results/latest-codec.json

skip_lang() {
    local lang="$1"
    echo ",$SKIP_LANGS," | grep -q ",$lang,"
}

mkdir -p "$BIN_DIR" "$RAW_DIR" "$RESULTS_DIR/history" "$GENERATED_DIR"

# Set LOGOS_WORKSPACE so largo can find workspace crates from temp directories
export LOGOS_WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"

# Shared cargo target dir so LOGOS builds reuse compiled dependencies
LOGOS_TARGET_DIR="$SCRIPT_DIR/.logos-bench-target"
mkdir -p "$LOGOS_TARGET_DIR"
export CARGO_TARGET_DIR="$LOGOS_TARGET_DIR"

# NOTE: largo hardcodes target_dir = project_dir/target, not CARGO_TARGET_DIR.
# But the inner cargo build DOES use CARGO_TARGET_DIR, so deps are shared.
# Binaries end up at: $LOGOS_TMP/target/{release,debug}/build/target/{release,debug}/bench

# Color output helpers
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

# Portable timeout: use gtimeout (coreutils) or timeout (Linux), or shell fallback
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

# Merge multiple single-language hyperfine JSONs into one combined result.
# Output matches hyperfine's native multi-command format so downstream
# Phase 5 assembly code works unchanged.
merge_hyperfine_results() {
    local output_file="$1"
    shift
    local files=("$@")
    if [ ${#files[@]} -eq 0 ]; then
        echo '{"results":[]}' > "$output_file"
        return
    fi
    jq -s '{ results: [.[].results[]] }' "${files[@]}" > "$output_file"
}

# ===========================================================================
# Phase 1: Build Everything
# ===========================================================================
info "Phase 1: Building all implementations..."

# Build largo (the LOGOS CLI)
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

    # C
    if [ -f "$PROGRAMS_DIR/$bench/main.c" ]; then
        gcc -O3 -march=native -flto -o "$BIN_DIR/${bench}_c" "$PROGRAMS_DIR/$bench/main.c" -lm 2>/dev/null && \
            ok "  C" || warn "  C build failed"
    fi

    # C++
    if [ -f "$PROGRAMS_DIR/$bench/main.cpp" ]; then
        g++ -O3 -march=native -flto -std=c++17 -o "$BIN_DIR/${bench}_cpp" "$PROGRAMS_DIR/$bench/main.cpp" 2>/dev/null && \
            ok "  C++" || warn "  C++ build failed"
    fi

    # Rust
    if [ -f "$PROGRAMS_DIR/$bench/main.rs" ]; then
        rustc --edition 2021 -C opt-level=3 -C lto=fat -C codegen-units=1 -C target-cpu=native -o "$BIN_DIR/${bench}_rs" "$PROGRAMS_DIR/$bench/main.rs" 2>/dev/null && \
            ok "  Rust" || warn "  Rust build failed"
    fi

    # Zig (timeout to avoid hangs on macOS)
    if [ -f "$PROGRAMS_DIR/$bench/main.zig" ] && command -v zig &>/dev/null && ! skip_lang zig; then
        run_timeout "$BUILD_TIMEOUT" zig build-exe -O ReleaseFast -mcpu native --name "${bench}_zig" "$PROGRAMS_DIR/$bench/main.zig" 2>/dev/null && \
            mv "${bench}_zig" "$BIN_DIR/" 2>/dev/null && \
            ok "  Zig" || warn "  Zig build failed or timed out"
    fi

    # Go
    if [ -f "$PROGRAMS_DIR/$bench/main.go" ]; then
        go build -o "$BIN_DIR/${bench}_go" "$PROGRAMS_DIR/$bench/main.go" 2>/dev/null && \
            ok "  Go" || warn "  Go build failed"
    fi

    # Java
    if [ -f "$PROGRAMS_DIR/$bench/Main.java" ]; then
        mkdir -p "$BIN_DIR/java/$bench"
        javac -d "$BIN_DIR/java/$bench" "$PROGRAMS_DIR/$bench/Main.java" 2>/dev/null && \
            ok "  Java" || warn "  Java build failed"
    fi

    # Nim
    if [ -f "$PROGRAMS_DIR/$bench/main.nim" ] && command -v nim &>/dev/null; then
        nim c -d:release --passC:"-O3 -march=native" --hints:off -o:"$BIN_DIR/${bench}_nim" "$PROGRAMS_DIR/$bench/main.nim" 2>/dev/null && \
            ok "  Nim" || warn "  Nim build failed"
    fi

    # LOGOS (release)
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
        # Build UNSTRIPPED so the recorded as-built size is a fair, apples-to-apples
        # comparison with the other toolchains. `largo build --release` strips by default
        # (great for shipping), but that would make LOGOS's as-built the already-stripped
        # size — flattering it next to rustc/gcc, which leave their symbol table on the
        # as-built artifact. Overriding strip keeps the symbols so `as_built` is comparable;
        # measure-sizes.sh strips a copy for the shipped (stripped) number. The symbol table
        # lives in non-loaded ELF sections, so runtime and peak RSS are unaffected.
        (cd "$LOGOS_TMP" && CARGO_PROFILE_RELEASE_STRIP=false "$LARGO" build --release 2>/dev/null) && {
            # Find binary: CARGO_TARGET_DIR redirects output, so check there first
            LOGOS_BIN=""
            if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -f "$CARGO_TARGET_DIR/release/bench" ]; then
                LOGOS_BIN="$CARGO_TARGET_DIR/release/bench"
            elif [[ "$(uname -s)" == "Darwin" ]]; then
                LOGOS_BIN=$(find "$LOGOS_TMP/target/release" -type f -perm +111 -name "bench" 2>/dev/null | head -1)
            else
                LOGOS_BIN=$(find "$LOGOS_TMP/target/release" -type f -executable -name "bench" 2>/dev/null | head -1)
            fi
            if [ -n "$LOGOS_BIN" ]; then
                rm -f "$BIN_DIR/${bench}_logos_release"
                cp "$LOGOS_BIN" "$BIN_DIR/${bench}_logos_release"
                ok "  LOGOS (release)"
            else
                warn "  LOGOS (release) binary not found"
            fi
            # Copy generated Rust (non-fatal)
            GENERATED_RS=$(find "$LOGOS_TMP" -name "main.rs" -path "*/build/src/*" 2>/dev/null | head -1)
            if [ -n "$GENERATED_RS" ]; then
                cp "$GENERATED_RS" "$GENERATED_DIR/$bench.rs" 2>/dev/null || true
            fi
        } || warn "  LOGOS (release) build failed"

        rm -rf "$LOGOS_TMP"
    fi
done

ok "Phase 1 complete"

# ===========================================================================
# Phase 2: Verify Correctness
# ===========================================================================
info "Phase 2: Verifying correctness at reference sizes..."

ref_size() {
    case "$1" in
        fib) echo 30 ;; ackermann) echo 10 ;; nqueens) echo 11 ;;
        bubble_sort) echo 2000 ;; mergesort) echo 10000 ;; quicksort) echo 10000 ;;
        counting_sort) echo 100000 ;; heap_sort) echo 10000 ;;
        nbody) echo 10000 ;; mandelbrot) echo 500 ;; spectral_norm) echo 1000 ;; pi_leibniz) echo 10000000 ;;
        gcd) echo 2000 ;; collatz) echo 1000000 ;; primes) echo 100000 ;;
        sieve) echo 1000000 ;; matrix_mult) echo 200 ;; prefix_sum) echo 1000000 ;;
        array_reverse) echo 1000000 ;; array_fill) echo 5000000 ;;
        collect) echo 50000 ;; two_sum) echo 10000 ;; histogram) echo 1000000 ;;
        knapsack) echo 1000 ;; coins) echo 10000 ;;
        fannkuch) echo 9 ;;
        strings) echo 50000 ;; binary_trees) echo 16 ;;
        loop_sum) echo 100000000 ;; fib_iterative) echo 100000000 ;;
        graph_bfs) echo 10000 ;; string_search) echo 100000 ;;
    esac
}

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
    size="$(ref_size "$bench")"
    expected_file="$PROGRAMS_DIR/$bench/expected_${size}.txt"
    if [ ! -f "$expected_file" ]; then
        warn "No expected output for $bench at size $size"
        continue
    fi
    expected=$(cat "$expected_file")
    info "Verifying $bench (n=$size, expected=$expected)..."

    [ -f "$BIN_DIR/${bench}_c" ]   && verify "$bench" "C" "$BIN_DIR/${bench}_c" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_cpp" ] && verify "$bench" "C++" "$BIN_DIR/${bench}_cpp" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_rs" ]  && verify "$bench" "Rust" "$BIN_DIR/${bench}_rs" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_zig" ] && verify "$bench" "Zig" "$BIN_DIR/${bench}_zig" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_go" ]  && verify "$bench" "Go" "$BIN_DIR/${bench}_go" "$size" "$expected"
    [ -d "$BIN_DIR/java/$bench" ]  && verify "$bench" "Java" "java -cp $BIN_DIR/java/$bench Main" "$size" "$expected"
    [ -f "$PROGRAMS_DIR/$bench/main.js" ]  && verify "$bench" "JavaScript" "node $PROGRAMS_DIR/$bench/main.js" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_nim" ] && verify "$bench" "Nim" "$BIN_DIR/${bench}_nim" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_logos_release" ] && verify "$bench" "LOGOS (release)" "$BIN_DIR/${bench}_logos_release" "$size" "$expected"
done

if [ "$ERRORS" -gt 0 ]; then
    fail "Phase 2 failed: $ERRORS verification errors"
    exit 1
fi
ok "Phase 2 complete: all implementations verified"

# ===========================================================================
# Phase 3: Benchmark Runtime (Scaling) — per-language timeout isolation
# ===========================================================================
info "Phase 3: Benchmarking runtime performance..."

PER_LANG_DIR="$RAW_DIR/per_lang"
mkdir -p "$PER_LANG_DIR"

# Track which languages have timed out per benchmark.
# Key: "${bench}_${lang_id}", Value: "1" if timed out.
# A language that times out at size N is automatically skipped for all
# larger sizes of that benchmark, avoiding wasted CI minutes.
declare -A LANG_TIMEOUTS

# Run a single language through hyperfine with its own timeout.
# If it times out, mark it so larger sizes are skipped.
try_bench() {
    local lang_id="$1" label="$2" cmd="$3"

    if skip_lang "$lang_id"; then
        return
    fi

    if [[ "${LANG_TIMEOUTS[${bench}_${lang_id}]:-}" == "1" ]]; then
        warn "  Skipping $label (timed out at smaller size)"
        return
    fi

    local pf="$PER_LANG_DIR/${bench}_${size}_${lang_id}.json"
    local rc=0
    run_timeout "$HYPERFINE_TIMEOUT" hyperfine \
        --warmup "$WARMUP" --runs "$RUNS" \
        --export-json "$pf" --time-unit millisecond \
        -n "$label" "$cmd" || rc=$?

    if [ "$rc" -eq 124 ] || [ "$rc" -eq 143 ]; then
        LANG_TIMEOUTS["${bench}_${lang_id}"]=1
        warn "  $label timed out for $bench at $size — skipping larger sizes"
        echo "$HYPERFINE_TIMEOUT" > "$RAW_DIR/${bench}_${size}_${lang_id}.timeout"
    elif [ "$rc" -ne 0 ]; then
        warn "  $label benchmark failed for $bench at $size"
    fi

    [ -f "$pf" ] && MERGE_FILES+=("$pf")
}

# Peak resident memory (kB) for one command, measured ONCE (memory is a property
# of the program + size, not of run-to-run variance, so no warmup/runs needed).
# Uses GNU `/usr/bin/time -v`; echoes an integer kB, or the JSON `null` if time is
# unavailable or the command fails.
measure_memory() {
    local cmd="$1"
    [ -x /usr/bin/time ] || { echo null; return; }
    local errf; errf=$(mktemp)
    if /usr/bin/time -v $cmd >/dev/null 2>"$errf"; then
        local kb
        kb=$(grep -oP 'Maximum resident set size \(kbytes\): \K[0-9]+' "$errf")
        rm -f "$errf"
        if [ -n "$kb" ]; then echo "$kb"; else echo null; fi
    else
        rm -f "$errf"
        echo null
    fi
}

for bench in "${BENCHMARKS[@]}"; do
    if [ "$SIZES_MODE" = "ref" ]; then
        sizes="$(ref_size "$bench")"
    else
        sizes=$(cat "$PROGRAMS_DIR/$bench/sizes.txt")
    fi
    for size in $sizes; do
        info "Benchmarking $bench at n=$size..."
        MERGE_FILES=()

        [ -f "$BIN_DIR/${bench}_c" ]   && try_bench c "C" "$BIN_DIR/${bench}_c $size"
        [ -f "$BIN_DIR/${bench}_cpp" ] && try_bench cpp "C++" "$BIN_DIR/${bench}_cpp $size"
        [ -f "$BIN_DIR/${bench}_rs" ]  && try_bench rust "Rust" "$BIN_DIR/${bench}_rs $size"
        [ -f "$BIN_DIR/${bench}_zig" ] && try_bench zig "Zig" "$BIN_DIR/${bench}_zig $size"
        [ -f "$BIN_DIR/${bench}_go" ]  && try_bench go "Go" "$BIN_DIR/${bench}_go $size"
        [ -d "$BIN_DIR/java/$bench" ]  && try_bench java "Java" "java -cp $BIN_DIR/java/$bench Main $size"
        # Skip JS for ackermann at n>10 (Node.js stack overflow)
        if [ "$bench" != "ackermann" ] || [ "$size" -le 10 ]; then
            [ -f "$PROGRAMS_DIR/$bench/main.js" ]  && try_bench js "JavaScript" "node $PROGRAMS_DIR/$bench/main.js $size"
        fi
        [ -f "$BIN_DIR/${bench}_nim" ] && try_bench nim "Nim" "$BIN_DIR/${bench}_nim $size"
        [ -f "$BIN_DIR/${bench}_logos_release" ] && try_bench logos_release "LOGOS (release)" "$BIN_DIR/${bench}_logos_release $size"

        # Merge per-language results into the combined file Phase 5 expects
        if [ ${#MERGE_FILES[@]} -gt 0 ]; then
            merge_hyperfine_results "$RAW_DIR/${bench}_${size}.json" "${MERGE_FILES[@]}"
        else
            echo '{"results":[]}' > "$RAW_DIR/${bench}_${size}.json"
        fi

        # Collect per-language timeout markers into a single size-level marker
        has_timeout=false
        for tf in "$RAW_DIR/${bench}_${size}_"*.timeout; do
            [ -f "$tf" ] && has_timeout=true && break
        done
        if [ "$has_timeout" = true ]; then
            echo "$HYPERFINE_TIMEOUT" > "$RAW_DIR/${bench}_${size}.timeout"
        fi
    done
done

ok "Phase 3 complete"

# ===========================================================================
# Phase 3.5: Peak memory (RSS) — measured ONCE per (benchmark, language, size).
# One-shot (no warmup/runs), so cheap; gate with MEASURE_MEM=0 to skip. Multiple
# sizes let the frontend fit an empirical SPACE big-O exponent like it does for time.
# ===========================================================================
if [ "${MEASURE_MEM:-1}" = "1" ] && [ -x /usr/bin/time ]; then
    info "Phase 3.5: Measuring peak memory (GNU time -v)..."
    for bench in "${BENCHMARKS[@]}"; do
        if [ "$SIZES_MODE" = "ref" ]; then
            mem_sizes="$(ref_size "$bench")"
        else
            mem_sizes=$(cat "$PROGRAMS_DIR/$bench/sizes.txt")
        fi
        by_size="{}"
        for size in $mem_sizes; do
            langs="{}"
            add_mem() {  # lang_id  command
                local kb; kb=$(measure_memory "$2")
                langs=$(echo "$langs" | jq --arg l "$1" --argjson kb "$kb" '.[$l]=$kb')
            }
            [ -f "$BIN_DIR/${bench}_c" ]   && add_mem c    "$BIN_DIR/${bench}_c $size"
            [ -f "$BIN_DIR/${bench}_cpp" ] && add_mem cpp  "$BIN_DIR/${bench}_cpp $size"
            [ -f "$BIN_DIR/${bench}_rs" ]  && add_mem rust "$BIN_DIR/${bench}_rs $size"
            [ -f "$BIN_DIR/${bench}_zig" ] && add_mem zig  "$BIN_DIR/${bench}_zig $size"
            [ -f "$BIN_DIR/${bench}_go" ]  && add_mem go   "$BIN_DIR/${bench}_go $size"
            [ -d "$BIN_DIR/java/$bench" ]  && add_mem java "java -cp $BIN_DIR/java/$bench Main $size"
            [ -f "$PROGRAMS_DIR/$bench/main.js" ] && add_mem js "node $PROGRAMS_DIR/$bench/main.js $size"
            [ -f "$BIN_DIR/${bench}_nim" ] && add_mem nim  "$BIN_DIR/${bench}_nim $size"
            [ -f "$BIN_DIR/${bench}_logos_release" ] && add_mem logos_release "$BIN_DIR/${bench}_logos_release $size"
            by_size=$(echo "$by_size" | jq --arg s "$size" --argjson l "$langs" '.[$s]=$l')
        done
        echo "{\"method\":\"gnu_time_v_maxrss_kb\",\"by_size\":$by_size}" > "$RAW_DIR/${bench}_mem.json"
    done
    ok "Phase 3.5 complete"
else
    info "Phase 3.5: memory measurement skipped (MEASURE_MEM=0 or /usr/bin/time absent)"
fi

# ===========================================================================
# Phase 3.6: Binary size — compiled-artifact footprint per language
# ===========================================================================
if [ "${MEASURE_SIZE:-1}" = "1" ]; then
    info "Phase 3.6: Measuring binary sizes..."
    bash "$SCRIPT_DIR/measure-sizes.sh" --bins-only "${BENCHMARKS[@]}" \
        && ok "Phase 3.6 complete" \
        || warn "Phase 3.6: binary-size measurement failed (non-fatal)"
else
    info "Phase 3.6: binary-size measurement skipped (MEASURE_SIZE=0)"
fi

# ===========================================================================
# Phase 4: Benchmark Compilation Time — per-compiler timeout isolation
# ===========================================================================
info "Phase 4: Benchmarking compilation times..."

declare -A COMPILE_TIMEOUTS

try_compile_bench() {
    local lang_id="$1" label="$2" cmd="$3"

    if [[ "${COMPILE_TIMEOUTS[${bench}_${lang_id}]:-}" == "1" ]]; then
        return
    fi

    local pf="$PER_LANG_DIR/compile_${bench}_${lang_id}.json"
    local rc=0
    run_timeout "$HYPERFINE_TIMEOUT" hyperfine \
        --warmup 1 --runs 5 \
        --export-json "$pf" --time-unit millisecond \
        -n "$label" "$cmd" || rc=$?

    if [ "$rc" -eq 124 ] || [ "$rc" -eq 143 ]; then
        COMPILE_TIMEOUTS["${bench}_${lang_id}"]=1
        warn "  $label compile timed out"
    elif [ "$rc" -ne 0 ]; then
        warn "  $label compile benchmark failed"
    fi

    [ -f "$pf" ] && COMPILE_MERGE_FILES+=("$pf")
}

for bench in "${BENCHMARKS[@]}"; do
    info "Compilation benchmark: $bench"
    COMPILE_MERGE_FILES=()

    [ -f "$PROGRAMS_DIR/$bench/main.c" ] && \
        try_compile_bench "gcc_-o3" "gcc -O3 -march=native -flto" "gcc -O3 -march=native -flto -o /dev/null $PROGRAMS_DIR/$bench/main.c -lm"
    [ -f "$PROGRAMS_DIR/$bench/main.cpp" ] && \
        try_compile_bench "g++_-o3" "g++ -O3 -march=native -flto" "g++ -O3 -march=native -flto -std=c++17 -o /dev/null $PROGRAMS_DIR/$bench/main.cpp"
    [ -f "$PROGRAMS_DIR/$bench/main.rs" ] && \
        try_compile_bench "rustc_-o3" "rustc -O3 -C lto=fat -C target-cpu=native" "rustc --edition 2021 -C opt-level=3 -C lto=fat -C codegen-units=1 -C target-cpu=native -o /tmp/bench_rustc_out $PROGRAMS_DIR/$bench/main.rs && rm -f /tmp/bench_rustc_out"
    [ -f "$PROGRAMS_DIR/$bench/main.go" ] && \
        try_compile_bench "go_build" "go build" "go build -o /dev/null $PROGRAMS_DIR/$bench/main.go"
    [ -f "$PROGRAMS_DIR/$bench/Main.java" ] && \
        try_compile_bench "javac" "javac" "javac -d /tmp $PROGRAMS_DIR/$bench/Main.java"
    command -v nim &>/dev/null && [ -f "$PROGRAMS_DIR/$bench/main.nim" ] && \
        try_compile_bench "nim_c" "nim c -d:release -march=native" "nim c -d:release --passC:'-O3 -march=native' --hints:off -o:/dev/null $PROGRAMS_DIR/$bench/main.nim"
    command -v zig &>/dev/null && [ -f "$PROGRAMS_DIR/$bench/main.zig" ] && \
        try_compile_bench "zig_build-exe" "zig build-exe -O ReleaseFast -mcpu native" "cd /tmp && rm -rf /tmp/zig-bench-cache && zig build-exe -O ReleaseFast -mcpu native --cache-dir /tmp/zig-bench-cache --name bench_zig_out $PROGRAMS_DIR/$bench/main.zig && rm -f /tmp/bench_zig_out"

    # LOGOS compilation (largo build + largo build --release)
    if [ -f "$PROGRAMS_DIR/$bench/main.lg" ]; then
        LOGOS_COMPILE_TMP=$(mktemp -d)
        mkdir -p "$LOGOS_COMPILE_TMP/src"
        cp "$PROGRAMS_DIR/$bench/main.lg" "$LOGOS_COMPILE_TMP/src/main.lg"
        cat > "$LOGOS_COMPILE_TMP/Largo.toml" << 'TOML'
[package]
name = "bench"
version = "0.1.0"
entry = "src/main.lg"
TOML
        try_compile_bench "largo_build" "largo build" "cd '$LOGOS_COMPILE_TMP' && '$LARGO' build"
        try_compile_bench "largo_build_--release" "largo build --release" "cd '$LOGOS_COMPILE_TMP' && '$LARGO' build --release"
    fi

    # Merge per-compiler results
    if [ ${#COMPILE_MERGE_FILES[@]} -gt 0 ]; then
        merge_hyperfine_results "$RAW_DIR/compile_${bench}.json" "${COMPILE_MERGE_FILES[@]}"
    else
        echo '{"results":[]}' > "$RAW_DIR/compile_${bench}.json"
    fi

    # Clean up LOGOS compile temp dir if it was created
    [ -n "${LOGOS_COMPILE_TMP:-}" ] && rm -rf "$LOGOS_COMPILE_TMP"
    unset LOGOS_COMPILE_TMP
done

ok "Phase 4 complete"

# ===========================================================================
# Phase 5: Assemble Results JSON
# ===========================================================================
info "Phase 5: Assembling results JSON..."

# B1: CPU detection
detect_cpu() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "$(uname -m)"
    elif [ -f /proc/cpuinfo ]; then
        grep -m1 'model name' /proc/cpuinfo 2>/dev/null | sed 's/.*: //' || echo "$(uname -m)"
    else
        uname -m
    fi
}

# B2: OS detection
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

# B3: LOGOS version
if [ -n "${LOGOS_VERSION:-}" ]; then
    LOGOS_VER="$LOGOS_VERSION"
else
    LOGOS_VER=$(grep '^version' "$SCRIPT_DIR/../Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
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
        nim)    nim --version 2>/dev/null | head -1 || echo "unknown" ;;
    esac
}

# Build the JSON using jq
VERSIONS=$(jq -n \
    --arg c "$(get_version c)" \
    --arg cpp "$(get_version cpp)" \
    --arg rust "$(get_version rust)" \
    --arg zig "$(get_version zig)" \
    --arg go "$(get_version go)" \
    --arg java "$(get_version java)" \
    --arg node "$(get_version node)" \
    --arg nim "$(get_version nim)" \
    '{c: $c, cpp: $cpp, rust: $rust, zig: $zig, go: $go, java: $java, node: $node, nim: $nim}')

LANGUAGES='[
  {"id":"c","label":"C","color":"#555555","tier":"systems"},
  {"id":"cpp","label":"C++","color":"#f34b7d","tier":"systems"},
  {"id":"rust","label":"Rust","color":"#dea584","tier":"systems"},
  {"id":"zig","label":"Zig","color":"#f7a41d","tier":"systems"},
  {"id":"logos_release","label":"LOGOS","color":"#00d4ff","tier":"systems"},
  {"id":"go","label":"Go","color":"#3fb950","tier":"managed"},
  {"id":"java","label":"Java","color":"#b07219","tier":"managed"},
  {"id":"js","label":"JavaScript","color":"#f7df1e","tier":"managed"},
  {"id":"nim","label":"Nim","color":"#ffe953","tier":"transpiled"}
]'

# Map display names to JSON IDs
lang_id() {
    case "$1" in
        "C")                echo c ;;
        "C++")              echo cpp ;;
        "Rust")             echo rust ;;
        "Zig")              echo zig ;;
        "Go")               echo go ;;
        "Java")             echo java ;;
        "JavaScript")       echo js ;;
        "Nim")              echo nim ;;
        "LOGOS (release)")  echo logos_release ;;
        "gcc -O3 -march=native -flto") echo "gcc_-o3" ;;
        "g++ -O3 -march=native -flto") echo "g++_-o3" ;;
        "rustc -O3 -C lto=fat -C target-cpu=native") echo "rustc_-o3" ;;
        "go build")         echo "go_build" ;;
        "javac")            echo "javac" ;;
        "nim c -d:release -march=native") echo "nim_c" ;;
        "zig build-exe -O ReleaseFast -mcpu native") echo "zig_build-exe" ;;
        "largo build")      echo "largo_build" ;;
        "largo build --release") echo "largo_build_--release" ;;
        *)                  echo "$1" | tr '[:upper:]' '[:lower:]' | tr ' ' '_' ;;
    esac
}

# Benchmark descriptions
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

# Theoretical (time, space) complexity per benchmark — the naive algorithm each
# language runs, tab-separated. The frontend pairs this DECLARED complexity with
# the EMPIRICAL exponent it fits from the multi-size scaling / memory curves.
bench_complexity() {
    case "$1" in
        fib)           printf 'O(2^n)\tO(n)' ;;
        ackermann)     printf 'O(A(m,n))\tO(m)' ;;
        nqueens)       printf 'O(n!)\tO(n)' ;;
        bubble_sort)   printf 'O(n^2)\tO(1)' ;;
        mergesort)     printf 'O(n log n)\tO(n)' ;;
        quicksort)     printf 'O(n log n)\tO(log n)' ;;
        counting_sort) printf 'O(n+k)\tO(k)' ;;
        heap_sort)     printf 'O(n log n)\tO(1)' ;;
        nbody)         printf 'O(k*n^2)\tO(n)' ;;
        mandelbrot)    printf 'O(k*n^2)\tO(1)' ;;
        spectral_norm) printf 'O(n^2)\tO(n)' ;;
        pi_leibniz)    printf 'O(n)\tO(1)' ;;
        gcd)           printf 'O(log n)\tO(1)' ;;
        collatz)       printf 'O(n log n)\tO(1)' ;;
        primes)        printf 'O(n log log n)\tO(n)' ;;
        sieve)         printf 'O(n log log n)\tO(n)' ;;
        matrix_mult)   printf 'O(n^3)\tO(n^2)' ;;
        prefix_sum)    printf 'O(n)\tO(n)' ;;
        array_reverse) printf 'O(n)\tO(n)' ;;
        array_fill)    printf 'O(n)\tO(n)' ;;
        collect)       printf 'O(n)\tO(n)' ;;
        two_sum)       printf 'O(n)\tO(n)' ;;
        histogram)     printf 'O(n)\tO(k)' ;;
        knapsack)      printf 'O(n*W)\tO(W)' ;;
        coins)         printf 'O(n*k)\tO(n)' ;;
        fannkuch)      printf 'O(n*n!)\tO(n)' ;;
        strings)       printf 'O(n)\tO(n)' ;;
        binary_trees)  printf 'O(2^n)\tO(n)' ;;
        loop_sum)      printf 'O(n)\tO(1)' ;;
        fib_iterative) printf 'O(n)\tO(1)' ;;
        graph_bfs)     printf 'O(V+E)\tO(V)' ;;
        string_search) printf 'O(n*m)\tO(1)' ;;
        *)             printf '?\t?' ;;
    esac
}

# `{time, space}` JSON for the benchmark's declared complexity.
bench_complexity_json() {
    local ct time space
    ct=$(bench_complexity "$1")
    time="${ct%%$'\t'*}"
    space="${ct##*$'\t'}"
    jq -n --arg t "$time" --arg s "$space" '{time: $t, space: $s}'
}

bench_desc() {
    case "$1" in
        fib) echo "Naive recursive Fibonacci." ;;
        ackermann) echo "Ackermann(3, m) — deep non-tail recursion." ;;
        nqueens) echo "N-Queens backtracking search." ;;
        bubble_sort) echo "O(n^2) in-place bubble sort." ;;
        mergesort) echo "Top-down merge sort." ;;
        quicksort) echo "Lomuto-partition quicksort." ;;
        counting_sort) echo "Non-comparison O(n+k) counting sort." ;;
        heap_sort) echo "Binary-heap sort with sift-down." ;;
        nbody) echo "5-body gravitational simulation." ;;
        mandelbrot) echo "Mandelbrot escape-iteration set." ;;
        spectral_norm) echo "Spectral-norm power method." ;;
        pi_leibniz) echo "Leibniz series for pi." ;;
        gcd) echo "Euclidean GCD summed over a range." ;;
        collatz) echo "Collatz stopping-time counting." ;;
        primes) echo "Trial-division prime counting." ;;
        sieve) echo "Sieve of Eratosthenes." ;;
        matrix_mult) echo "O(n^3) dense matrix multiply." ;;
        prefix_sum) echo "Sequential prefix-sum scan." ;;
        array_reverse) echo "Two-pointer in-place array reversal." ;;
        array_fill) echo "Array fill, then sum." ;;
        collect) echo "Hash-map insert and lookup." ;;
        two_sum) echo "Interleaved hash insert and lookup (two-sum)." ;;
        histogram) echo "Array-indexed frequency counting." ;;
        knapsack) echo "0/1 knapsack dynamic programming." ;;
        coins) echo "Coin-change dynamic programming." ;;
        fannkuch) echo "Fannkuch-redux permutation flips." ;;
        strings) echo "Repeated string concatenation and assembly." ;;
        binary_trees) echo "Recursive binary-tree allocation and checksum." ;;
        loop_sum) echo "Tight accumulation loop." ;;
        fib_iterative) echo "Iterative Fibonacci modulo." ;;
        graph_bfs) echo "Breadth-first search over a generated graph." ;;
        string_search) echo "Naive O(nm) substring search." ;;
    esac
}

# B4: Parse hyperfine results — use .command as the name (hyperfine 1.18+ with -n sets command = display name)
extract_name() {
    local result="$1"
    # hyperfine 1.18+ with -n "Name" stores the display name in .command
    echo "$result" | jq -r '.command' 2>/dev/null
}

# Parse hyperfine results into scaling data
assemble_benchmark() {
    local bench="$1"
    local sizes
    if [ "$SIZES_MODE" = "ref" ]; then
        sizes="$(ref_size "$bench")"
    else
        sizes=$(cat "$PROGRAMS_DIR/$bench/sizes.txt")
    fi
    local logos_src=""
    local gen_rust=""

    [ -f "$PROGRAMS_DIR/$bench/main.lg" ] && logos_src=$(cat "$PROGRAMS_DIR/$bench/main.lg")
    [ -f "$GENERATED_DIR/$bench.rs" ] && gen_rust=$(cat "$GENERATED_DIR/$bench.rs")

    # Per-program optimization graph (fired + blockers + dependencies), from one
    # all-on evaluation, so the website's toggle tree pops in instantly. Never
    # affects timing — this is metadata baked alongside the source.
    local opt_graph='{"fired":[],"blockers":[],"dependencies":[]}'
    if [ -f "$PROGRAMS_DIR/$bench/main.lg" ]; then
        opt_graph=$("$LARGO" opts "$PROGRAMS_DIR/$bench/main.lg" --json 2>/dev/null \
            || echo '{"fired":[],"blockers":[],"dependencies":[]}')
    fi

    # Build scaling object
    local scaling="{}"
    for size in $sizes; do
        local raw_file="$RAW_DIR/${bench}_${size}.json"
        if [ -f "$raw_file" ]; then
            local size_data="{}"
            local results
            results=$(jq -c '.results[]' "$raw_file" 2>/dev/null) || continue
            while IFS= read -r result; do
                [ -z "$result" ] && continue
                local name
                name=$(extract_name "$result")
                local lid
                lid=$(lang_id "$name")
                local mean_s median_s stddev_s min_s max_s user_s system_s
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
    done

    # Build compilation object
    local compilation="{}"
    local compile_file="$RAW_DIR/compile_${bench}.json"
    if [ -f "$compile_file" ]; then
        local results
        results=$(jq -c '.results[]' "$compile_file" 2>/dev/null) || true
        while IFS= read -r result; do
            [ -z "$result" ] && continue
            local name
            name=$(extract_name "$result")
            local mean_s stddev_s
            mean_s=$(echo "$result" | jq '.mean')
            stddev_s=$(echo "$result" | jq '.stddev')
            local mean_ms stddev_ms
            mean_ms=$(echo "$mean_s * 1000" | bc -l 2>/dev/null || echo "0")
            stddev_ms=$(echo "$stddev_s * 1000" | bc -l 2>/dev/null || echo "0")
            local key
            key=$(lang_id "$name")
            compilation=$(echo "$compilation" | jq \
                --arg key "$key" \
                --argjson mean "$mean_ms" \
                --argjson stddev "$stddev_ms" \
                '.[$key] = {mean_ms: $mean, stddev_ms: $stddev}')
        done <<< "$results"
    fi

    # Collect timeout markers (size → timeout_ms)
    local timeouts="{}"
    for size in $sizes; do
        if [ -f "$RAW_DIR/${bench}_${size}.timeout" ]; then
            local timeout_secs
            timeout_secs=$(cat "$RAW_DIR/${bench}_${size}.timeout")
            local timeout_ms
            timeout_ms=$(echo "$timeout_secs * 1000" | bc -l 2>/dev/null || echo "600000")
            timeouts=$(echo "$timeouts" | jq --arg size "$size" --argjson ms "$timeout_ms" '.[$size] = $ms')
        fi
    done
    if [ -f "$RAW_DIR/compile_${bench}.timeout" ]; then
        local timeout_secs
        timeout_secs=$(cat "$RAW_DIR/compile_${bench}.timeout")
        local timeout_ms
        timeout_ms=$(echo "$timeout_secs * 1000" | bc -l 2>/dev/null || echo "600000")
        timeouts=$(echo "$timeouts" | jq --arg size "compile" --argjson ms "$timeout_ms" '.[$size] = $ms')
    fi

    # Reference size for the headline + geomean: the largest benchmarked size
    # that actually has both C and LOGOS data (falls back to largest with LOGOS,
    # then largest overall, then the hardcoded ref_size). This keeps every
    # benchmark in the comparison even as sizes.txt evolves.
    local emit_ref
    emit_ref=$(echo "$scaling" | jq -r '
        ([to_entries[] | select((.value.c != null) and (.value.logos_release != null)) | (.key|tonumber)]) as $both
        | ([to_entries[] | select(.value.logos_release != null) | (.key|tonumber)]) as $lg
        | ([keys[] | tonumber]) as $any
        | (if ($both|length)>0 then ($both|max) elif ($lg|length)>0 then ($lg|max) elif ($any|length)>0 then ($any|max) else null end)
        | if . == null then empty else tostring end' 2>/dev/null)
    [ -z "$emit_ref" ] && emit_ref="$(ref_size "$bench")"

    # Output benchmark JSON
    jq -n \
        --arg id "$bench" \
        --arg name "$(bench_name "$bench")" \
        --arg desc "$(bench_desc "$bench")" \
        --arg ref "$emit_ref" \
        --arg logos_src "$logos_src" \
        --arg gen_rust "$gen_rust" \
        --arg sizes_str "$(cat "$PROGRAMS_DIR/$bench/sizes.txt")" \
        --argjson scaling "$scaling" \
        --argjson compilation "$compilation" \
        --argjson timeouts "$timeouts" \
        --argjson memory "$(cat "$RAW_DIR/${bench}_mem.json" 2>/dev/null || echo null)" \
        --argjson binary_sizes "$(cat "$RAW_DIR/${bench}_sizes.json" 2>/dev/null || echo null)" \
        --argjson complexity "$(bench_complexity_json "$bench")" \
        --argjson opt_graph "$opt_graph" \
        '{
            id: $id,
            name: $name,
            description: $desc,
            reference_size: $ref,
            sizes: ($sizes_str | split(" ")),
            logos_source: $logos_src,
            generated_rust: $gen_rust,
            scaling: $scaling,
            compilation: $compilation,
            timeouts: $timeouts,
            memory: $memory,
            binary_sizes: $binary_sizes,
            complexity: $complexity,
            fired: $opt_graph.fired,
            blockers: $opt_graph.blockers,
            dependencies: $opt_graph.dependencies
        }'
}

# Assemble all benchmarks
BENCHMARKS_JSON="["
first=true
for bench in "${BENCHMARKS[@]}"; do
    if [ "$first" = true ]; then
        first=false
    else
        BENCHMARKS_JSON+=","
    fi
    BENCHMARKS_JSON+=$(assemble_benchmark "$bench")
done
BENCHMARKS_JSON+="]"

# B6: Compute geometric mean speedup vs C
info "Computing geometric mean speedup vs C..."
compute_geometric_mean() {
    local benchmarks_json="$1"

    # Collect all language IDs that appear in any benchmark at the reference size
    local all_langs
    all_langs=$(echo "$benchmarks_json" | jq -r '
        [.[] | .scaling[.reference_size] // {} | keys[]] | unique | .[]
    ' 2>/dev/null)

    local geo_mean="{}"

    for lang in $all_langs; do
        # Collect c_mean / lang_mean ratios for each benchmark where both C and this lang exist
        local log_sum=0
        local count=0
        for bench in "${BENCHMARKS[@]}"; do
            # Use each benchmark's emitted reference_size (largest size with data),
            # not the hardcoded ref_size(), so all benchmarks count.
            local ref
            ref=$(echo "$benchmarks_json" | jq -r --arg bench "$bench" \
                '.[] | select(.id == $bench) | .reference_size // empty')
            [ -z "$ref" ] && ref="$(ref_size "$bench")"
            local c_mean lang_mean
            c_mean=$(echo "$benchmarks_json" | jq -r --arg ref "$ref" --arg bench "$bench" \
                '.[] | select(.id == $bench) | .scaling[$ref].c.mean_ms // 0')
            lang_mean=$(echo "$benchmarks_json" | jq -r --arg ref "$ref" --arg bench "$bench" --arg lang "$lang" \
                '.[] | select(.id == $bench) | .scaling[$ref][$lang].mean_ms // 0')

            if [ "$(echo "$c_mean > 0" | bc -l 2>/dev/null)" = "1" ] && \
               [ "$(echo "$lang_mean > 0" | bc -l 2>/dev/null)" = "1" ]; then
                local ratio
                ratio=$(echo "$c_mean / $lang_mean" | bc -l 2>/dev/null || echo "0")
                if [ "$(echo "$ratio > 0" | bc -l 2>/dev/null)" = "1" ]; then
                    local log_ratio
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
            # Round to 3 decimal places
            geo=$(printf "%.3f" "$geo")
            geo_mean=$(echo "$geo_mean" | jq --arg lang "$lang" --argjson val "$geo" '.[$lang] = $val')
        fi
    done

    echo "$geo_mean"
}

GEO_MEAN=$(compute_geometric_mean "$BENCHMARKS_JSON")
info "Geometric means: $GEO_MEAN"

# Final assembly — write large JSON to temp file to avoid "Argument list too long"
BENCHMARKS_JSON_FILE=$(mktemp)
printf '%s\n' "$BENCHMARKS_JSON" > "$BENCHMARKS_JSON_FILE"

# Keep only languages that actually produced results this run, so skipped
# languages (SKIP_LANGS) never appear as data-less gaps in the frontend.
PRESENT_LANGS=$(jq -c '[.[] | .scaling // {} | .[] | keys[]] | unique' "$BENCHMARKS_JSON_FILE")
LANGUAGES=$(echo "$LANGUAGES" | jq --argjson present "$PRESENT_LANGS" \
    '[.[] | select(.id as $id | $present | index($id))]')

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

ok "Phase 5 complete: results written to $RESULTS_DIR/latest.json"

# Archive with version
cp "$RESULTS_DIR/latest.json" "$RESULTS_DIR/history/v${LOGOS_VER}.json"

# ===========================================================================
# Phase 6: LOGOS interpreter vs Node/V8 (peer-to-peer, calibrated sizes)
# ===========================================================================
# The interpreter (bytecode VM + JIT) gets its own comparison vs Node's V8. We
# calibrate so the FASTER of the two engines reaches INTERP_CALIBRATION_TARGET ms
# (calibrate-interp.sh times both), which keeps BOTH off V8's ~30ms startup floor.
# run-interp-vs-js.sh then also measures the cold-start floor. The output
# (results/latest-interp.json) feeds the interpreter section of the page.
INTERP_CALIBRATION_TARGET="${INTERP_CALIBRATION_TARGET:-250}"
if [ "$RUN_INTERP" = "1" ] && [ -x "$SCRIPT_DIR/run-interp-vs-js.sh" ]; then
    info "Phase 6: LOGOS interpreter vs Node/V8 (calibrating to ${INTERP_CALIBRATION_TARGET}ms, off V8's floor)..."
    if [ -x "$SCRIPT_DIR/calibrate-interp.sh" ]; then
        # OVERSHOOT must accommodate the SLOWER engine at the calibration size:
        # since we crawl on the faster engine, the slower one can be several times
        # the target (up to ~12x on V8-fast string kernels). A tight timeout would
        # cut the crawl short and leave Node on its floor.
        TARGETS="$INTERP_CALIBRATION_TARGET" OVERSHOOT="${INTERP_CAL_OVERSHOOT:-24}" \
            bash "$SCRIPT_DIR/calibrate-interp.sh" || warn "interp calibration failed — using existing/fallback sizes"
    fi
    OUT="results/latest-interp.json" CALIBRATION_TARGET="$INTERP_CALIBRATION_TARGET" LOGOS_VERSION="$LOGOS_VER" \
        bash "$SCRIPT_DIR/run-interp-vs-js.sh" || warn "Phase 6 (interpreter vs Node) failed — latest-interp.json may be stale"
    [ -f "$RESULTS_DIR/latest-interp.json" ] && \
        cp "$RESULTS_DIR/latest-interp.json" "$RESULTS_DIR/history/v${LOGOS_VER}-interp.json"
    ok "Phase 6 complete"
else
    info "Phase 6 skipped (RUN_INTERP=$RUN_INTERP)"
fi

# ===========================================================================
# Phase 7: Wire codec vs industry serializers (size + enc/dec + random access)
# ===========================================================================
# The LOGOS wire codec gets its own head-to-head vs bincode/postcard/MessagePack/
# CBOR/JSON, plus Apache Arrow and — when protoc + capnp are on PATH — Protobuf and
# Cap'n Proto. The wirebench harness emits results/latest-codec.json (the same data
# its stdout table prints), which feeds the Serialization section of the page. We
# pass the machine identity computed in Phase 5 so the metadata matches latest.json.
if [ "$RUN_CODEC" = "1" ]; then
    if command -v protoc >/dev/null 2>&1 && command -v capnp >/dev/null 2>&1; then
        CODEC_FEATURES="--features heavy"   # arrow + protobuf + Cap'n Proto
        info "Phase 7: Wire codec head-to-head (protoc + capnp present -> heavy)..."
    else
        CODEC_FEATURES="--features arrow-bench"   # pure-Rust competitors + Arrow (no toolchain)
        warn "Phase 7: protoc/capnp absent -> core + Arrow only (install them for the full chart)"
    fi
    WIREBENCH_JSON="$RESULTS_DIR/latest-codec.json" \
    WIREBENCH_ITERS="${WIREBENCH_ITERS:-20000}" \
    WIREBENCH_DATE="$DATE" WIREBENCH_COMMIT="$COMMIT" \
    WIREBENCH_CPU="$CPU" WIREBENCH_OS="$OS" LOGOS_VERSION="$LOGOS_VER" \
        cargo run --release -p logicaffeine-wirebench $CODEC_FEATURES \
        || warn "Phase 7 (wire codec) failed — latest-codec.json may be stale"
    [ -f "$RESULTS_DIR/latest-codec.json" ] && \
        cp "$RESULTS_DIR/latest-codec.json" "$RESULTS_DIR/history/v${LOGOS_VER}-codec.json"
    ok "Phase 7 complete"
else
    info "Phase 7 skipped (RUN_CODEC=$RUN_CODEC)"
fi

# ===========================================================================
# Phase 8: Regenerate README benchmark charts (SVG)
# ===========================================================================
# Turn the freshly-assembled result JSON into the static SVGs the root README
# embeds. Reads results/{latest,latest-interp,latest-codec}.json plus the
# separately-produced results/solvers.json (from run-solver-vs-z3.sh).
if command -v python3 >/dev/null 2>&1; then
    info "Phase 8: Regenerating README charts (SVG)..."
    python3 "$SCRIPT_DIR/gen-readme-charts.py" \
        && ok "Phase 8 complete: results/charts/*.svg" \
        || warn "Phase 8 (README charts) failed — results/charts/*.svg may be stale"
else
    info "Phase 8 skipped (python3 not found)"
fi

info "Benchmark suite complete!"
