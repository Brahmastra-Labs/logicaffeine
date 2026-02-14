#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE Cross-Language Benchmark Suite
#
# Builds, verifies, and benchmarks implementations across 10 languages + 3 LOGOS variants.
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

BENCHMARKS=(fib sieve collect strings bubble_sort ackermann)

WARMUP="${BENCH_WARMUP:-3}"
RUNS="${BENCH_RUNS:-10}"
TIMEOUT="${BENCH_TIMEOUT:-120}"
BUILD_TIMEOUT="${BUILD_TIMEOUT:-60}"
SIZES_MODE="${BENCH_SIZES:-all}"  # "all" or "ref" (reference only)
SKIP_LANGS="${SKIP_LANGS:-}"      # comma-separated list of langs to skip (e.g., "zig,nim")

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

# ===========================================================================
# Phase 1: Build Everything
# ===========================================================================
info "Phase 1: Building all implementations..."

# Build largo (the LOGOS CLI)
info "Building largo..."
cargo build -p logicaffeine-cli --release --manifest-path "$SCRIPT_DIR/../Cargo.toml" 2>/dev/null
LARGO="$SCRIPT_DIR/../target/release/logicaffeine-cli"
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
        gcc -O2 -o "$BIN_DIR/${bench}_c" "$PROGRAMS_DIR/$bench/main.c" -lm 2>/dev/null && \
            ok "  C" || warn "  C build failed"
    fi

    # C++
    if [ -f "$PROGRAMS_DIR/$bench/main.cpp" ]; then
        g++ -O2 -std=c++17 -o "$BIN_DIR/${bench}_cpp" "$PROGRAMS_DIR/$bench/main.cpp" 2>/dev/null && \
            ok "  C++" || warn "  C++ build failed"
    fi

    # Rust
    if [ -f "$PROGRAMS_DIR/$bench/main.rs" ]; then
        rustc --edition 2021 -O -o "$BIN_DIR/${bench}_rs" "$PROGRAMS_DIR/$bench/main.rs" 2>/dev/null && \
            ok "  Rust" || warn "  Rust build failed"
    fi

    # Zig (timeout to avoid hangs on macOS)
    if [ -f "$PROGRAMS_DIR/$bench/main.zig" ] && command -v zig &>/dev/null && ! skip_lang zig; then
        run_timeout "$BUILD_TIMEOUT" zig build-exe -O ReleaseFast --name "${bench}_zig" "$PROGRAMS_DIR/$bench/main.zig" 2>/dev/null && \
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
        nim c -d:release --hints:off -o:"$BIN_DIR/${bench}_nim" "$PROGRAMS_DIR/$bench/main.nim" 2>/dev/null && \
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
        (cd "$LOGOS_TMP" && "$LARGO" build --release 2>/dev/null) && {
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

        # LOGOS (debug)
        (cd "$LOGOS_TMP" && "$LARGO" build 2>/dev/null) && {
            LOGOS_BIN=""
            if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -f "$CARGO_TARGET_DIR/debug/bench" ]; then
                LOGOS_BIN="$CARGO_TARGET_DIR/debug/bench"
            elif [[ "$(uname -s)" == "Darwin" ]]; then
                LOGOS_BIN=$(find "$LOGOS_TMP/target/debug" -type f -perm +111 -name "bench" 2>/dev/null | head -1)
            else
                LOGOS_BIN=$(find "$LOGOS_TMP/target/debug" -type f -executable -name "bench" 2>/dev/null | head -1)
            fi
            if [ -n "$LOGOS_BIN" ]; then
                cp "$LOGOS_BIN" "$BIN_DIR/${bench}_logos_debug"
                ok "  LOGOS (debug)"
            fi
        } || warn "  LOGOS (debug) build failed"

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
        fib) echo 35 ;; sieve) echo 1000000 ;; collect) echo 100000 ;;
        strings) echo 100000 ;; bubble_sort) echo 5000 ;; ackermann) echo 10 ;;
    esac
}

interp_size() {
    case "$1" in
        fib) echo 35 ;; sieve) echo 10000 ;; collect) echo 1000 ;;
        strings) echo 1000 ;; bubble_sort) echo 1000 ;; ackermann) echo 3 ;;
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

verify_interp() {
    local bench="$1" name="$2" expected="$3"
    local interp_file="$PROGRAMS_DIR/$bench/interp.lg"
    [ ! -f "$interp_file" ] && return

    local INTERP_TMP
    INTERP_TMP=$(mktemp -d)
    mkdir -p "$INTERP_TMP/src"
    cp "$interp_file" "$INTERP_TMP/src/main.lg"
    cat > "$INTERP_TMP/Largo.toml" << 'TOML'
[package]
name = "bench_interp"
version = "0.1.0"
entry = "src/main.lg"
TOML
    local output
    output=$(run_timeout "$TIMEOUT" bash -c "cd '$INTERP_TMP' && '$LARGO' run --interpret" 2>/dev/null | tr -d '[:space:]') || true
    expected=$(echo "$expected" | tr -d '[:space:]')
    if [ "$output" = "$expected" ]; then
        ok "  $name: $output"
    else
        warn "  $name: got '$output', expected '$expected' (interpreter may not support this benchmark)"
    fi
    rm -rf "$INTERP_TMP"
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
    [ -f "$PROGRAMS_DIR/$bench/main.py" ]  && verify "$bench" "Python" "python3 $PROGRAMS_DIR/$bench/main.py" "$size" "$expected"
    [ -f "$PROGRAMS_DIR/$bench/main.rb" ]  && verify "$bench" "Ruby" "RUBY_THREAD_VM_STACK_SIZE=67108864 ruby $PROGRAMS_DIR/$bench/main.rb" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_nim" ] && verify "$bench" "Nim" "$BIN_DIR/${bench}_nim" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_logos_release" ] && verify "$bench" "LOGOS (release)" "$BIN_DIR/${bench}_logos_release" "$size" "$expected"
    [ -f "$BIN_DIR/${bench}_logos_debug" ]   && verify "$bench" "LOGOS (debug)" "$BIN_DIR/${bench}_logos_debug" "$size" "$expected"

    # Verify interpreter (uses hardcoded smaller size, expected from corresponding file)
    interp_size="$(interp_size "$bench")"
    interp_expected_file="$PROGRAMS_DIR/$bench/expected_${interp_size}.txt"
    if [ -f "$interp_expected_file" ] && [ -f "$PROGRAMS_DIR/$bench/interp.lg" ]; then
        interp_expected=$(cat "$interp_expected_file")
        verify_interp "$bench" "LOGOS (interpreted)" "$interp_expected"
    fi
done

if [ "$ERRORS" -gt 0 ]; then
    fail "Phase 2 failed: $ERRORS verification errors"
    exit 1
fi
ok "Phase 2 complete: all implementations verified"

# ===========================================================================
# Phase 3: Benchmark Runtime (Scaling) + Interpreter
# ===========================================================================
info "Phase 3: Benchmarking runtime performance..."

for bench in "${BENCHMARKS[@]}"; do
    if [ "$SIZES_MODE" = "ref" ]; then
        sizes="$(ref_size "$bench")"
    else
        sizes=$(cat "$PROGRAMS_DIR/$bench/sizes.txt")
    fi
    for size in $sizes; do
        info "Benchmarking $bench at n=$size..."

        # Build hyperfine command dynamically based on available binaries
        HYPERFINE_ARGS=(
            --warmup "$WARMUP"
            --runs "$RUNS"
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
        [ -f "$BIN_DIR/${bench}_logos_debug" ]   && HYPERFINE_ARGS+=(-n "LOGOS (debug)" "$BIN_DIR/${bench}_logos_debug $size")

        hyperfine "${HYPERFINE_ARGS[@]}" || warn "hyperfine failed for $bench at $size"
    done

    # Interpreter benchmark (fixed size, separate run)
    if [ -f "$PROGRAMS_DIR/$bench/interp.lg" ]; then
        interp_size="$(interp_size "$bench")"
        info "Benchmarking $bench interpreter at n=$interp_size..."

        INTERP_TMP=$(mktemp -d)
        mkdir -p "$INTERP_TMP/src"
        cp "$PROGRAMS_DIR/$bench/interp.lg" "$INTERP_TMP/src/main.lg"
        cat > "$INTERP_TMP/Largo.toml" << 'TOML'
[package]
name = "bench_interp"
version = "0.1.0"
entry = "src/main.lg"
TOML
        hyperfine \
            --warmup "$WARMUP" \
            --runs "$RUNS" \
            --export-json "$RAW_DIR/${bench}_interp.json" \
            --time-unit millisecond \
            -n "LOGOS (interpreted)" \
            "cd '$INTERP_TMP' && '$LARGO' run --interpret" \
            || warn "interpreter benchmark failed for $bench"

        rm -rf "$INTERP_TMP"
    fi
done

ok "Phase 3 complete"

# ===========================================================================
# Phase 4: Benchmark Compilation Time
# ===========================================================================
info "Phase 4: Benchmarking compilation times..."

for bench in "${BENCHMARKS[@]}"; do
    info "Compilation benchmark: $bench"
    COMPILE_ARGS=(
        --warmup 1
        --runs 5
        --export-json "$RAW_DIR/compile_${bench}.json"
        --time-unit millisecond
    )

    [ -f "$PROGRAMS_DIR/$bench/main.c" ] && \
        COMPILE_ARGS+=(-n "gcc -O2" "gcc -O2 -o /dev/null $PROGRAMS_DIR/$bench/main.c -lm")
    [ -f "$PROGRAMS_DIR/$bench/main.cpp" ] && \
        COMPILE_ARGS+=(-n "g++ -O2" "g++ -O2 -std=c++17 -o /dev/null $PROGRAMS_DIR/$bench/main.cpp")
    [ -f "$PROGRAMS_DIR/$bench/main.rs" ] && \
        COMPILE_ARGS+=(-n "rustc -O" "rustc --edition 2021 -O $PROGRAMS_DIR/$bench/main.rs -o /dev/null")
    [ -f "$PROGRAMS_DIR/$bench/main.go" ] && \
        COMPILE_ARGS+=(-n "go build" "go build -o /dev/null $PROGRAMS_DIR/$bench/main.go")
    [ -f "$PROGRAMS_DIR/$bench/Main.java" ] && \
        COMPILE_ARGS+=(-n "javac" "javac -d /tmp $PROGRAMS_DIR/$bench/Main.java")
    command -v nim &>/dev/null && [ -f "$PROGRAMS_DIR/$bench/main.nim" ] && \
        COMPILE_ARGS+=(-n "nim c" "nim c -d:release --hints:off -o:/dev/null $PROGRAMS_DIR/$bench/main.nim")

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
        COMPILE_ARGS+=(-n "largo build" "cd '$LOGOS_COMPILE_TMP' && '$LARGO' build")
        COMPILE_ARGS+=(-n "largo build --release" "cd '$LOGOS_COMPILE_TMP' && '$LARGO' build --release")
    fi

    hyperfine "${COMPILE_ARGS[@]}" || warn "compile benchmark failed for $bench"

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

# Build the JSON using jq
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
  {"id":"go","label":"Go","color":"#00ADD8","tier":"managed"},
  {"id":"java","label":"Java","color":"#b07219","tier":"managed"},
  {"id":"js","label":"JavaScript","color":"#f7df1e","tier":"managed"},
  {"id":"python","label":"Python","color":"#3776ab","tier":"interpreted"},
  {"id":"ruby","label":"Ruby","color":"#cc342d","tier":"interpreted"},
  {"id":"nim","label":"Nim","color":"#ffe953","tier":"transpiled"},
  {"id":"logos_release","label":"LOGOS (release)","color":"#00d4ff","tier":"logos"},
  {"id":"logos_debug","label":"LOGOS (debug)","color":"#818cf8","tier":"logos"},
  {"id":"logos_interp","label":"LOGOS (interpreted)","color":"#c084fc","tier":"logos"}
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
        "Python")           echo python ;;
        "Ruby")             echo ruby ;;
        "Nim")              echo nim ;;
        "LOGOS (release)")  echo logos_release ;;
        "LOGOS (debug)")    echo logos_debug ;;
        "LOGOS (interpreted)")  echo logos_interp ;;
        "gcc -O2")          echo "gcc_-o2" ;;
        "g++ -O2")          echo "g++_-o2" ;;
        "rustc -O")         echo "rustc_-o" ;;
        "go build")         echo "go_build" ;;
        "javac")            echo "javac" ;;
        "nim c")            echo "nim_c" ;;
        "largo build")      echo "largo_build" ;;
        "largo build --release") echo "largo_build_--release" ;;
        *)                  echo "$1" | tr '[:upper:]' '[:lower:]' | tr ' ' '_' ;;
    esac
}

# Benchmark descriptions
bench_name() {
    case "$1" in
        fib) echo "Recursive Fibonacci" ;; sieve) echo "Sieve of Eratosthenes" ;;
        collect) echo "Collection Operations" ;; strings) echo "String Assembly" ;;
        bubble_sort) echo "Bubble Sort" ;; ackermann) echo "Ackermann Function" ;;
    esac
}

bench_desc() {
    case "$1" in
        fib) echo "Naive recursive fibonacci. Measures function call overhead and recursion depth." ;;
        sieve) echo "Classic prime sieve. Measures indexed array mutation, tight loops, and conditional branching." ;;
        collect) echo "Hash map insert and lookup. Measures hash computation, allocation pressure, and cache behavior." ;;
        strings) echo "String concatenation and assembly. Measures allocator throughput and GC pressure." ;;
        bubble_sort) echo "O(n^2) bubble sort. Measures nested loops, indexed array mutation, and swap patterns." ;;
        ackermann) echo "Ackermann(3, m). Measures extreme recursion depth and stack frame overhead." ;;
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
                local mean_s median_s stddev_s min_s max_s
                mean_s=$(echo "$result" | jq '.mean')
                median_s=$(echo "$result" | jq '.median')
                stddev_s=$(echo "$result" | jq '.stddev')
                min_s=$(echo "$result" | jq '.min')
                max_s=$(echo "$result" | jq '.max')
                local mean_ms median_ms stddev_ms min_ms max_ms cv
                mean_ms=$(echo "$mean_s * 1000" | bc -l 2>/dev/null || echo "0")
                median_ms=$(echo "$median_s * 1000" | bc -l 2>/dev/null || echo "0")
                stddev_ms=$(echo "$stddev_s * 1000" | bc -l 2>/dev/null || echo "0")
                min_ms=$(echo "$min_s * 1000" | bc -l 2>/dev/null || echo "0")
                max_ms=$(echo "$max_s * 1000" | bc -l 2>/dev/null || echo "0")
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
                    '.[$lid] = {mean_ms: $mean, median_ms: $median, stddev_ms: $stddev, min_ms: $min, max_ms: $max, cv: $cv, runs: $runs}')
            done <<< "$results"
            scaling=$(echo "$scaling" | jq --arg size "$size" --argjson data "$size_data" '.[$size] = $data')
        fi
    done

    # B7: Merge interpreter results into scaling at the interpreter's reference size
    local interp_file="$RAW_DIR/${bench}_interp.json"
    if [ -f "$interp_file" ]; then
        local interp_size="$(interp_size "$bench")"
        local results
        results=$(jq -c '.results[]' "$interp_file" 2>/dev/null) || true
        while IFS= read -r result; do
            [ -z "$result" ] && continue
            local mean_s median_s stddev_s min_s max_s
            mean_s=$(echo "$result" | jq '.mean')
            median_s=$(echo "$result" | jq '.median')
            stddev_s=$(echo "$result" | jq '.stddev')
            min_s=$(echo "$result" | jq '.min')
            max_s=$(echo "$result" | jq '.max')
            local mean_ms median_ms stddev_ms min_ms max_ms cv
            mean_ms=$(echo "$mean_s * 1000" | bc -l 2>/dev/null || echo "0")
            median_ms=$(echo "$median_s * 1000" | bc -l 2>/dev/null || echo "0")
            stddev_ms=$(echo "$stddev_s * 1000" | bc -l 2>/dev/null || echo "0")
            min_ms=$(echo "$min_s * 1000" | bc -l 2>/dev/null || echo "0")
            max_ms=$(echo "$max_s * 1000" | bc -l 2>/dev/null || echo "0")
            if [ "$(echo "$mean_ms > 0" | bc -l 2>/dev/null)" = "1" ]; then
                cv=$(echo "$stddev_ms / $mean_ms" | bc -l 2>/dev/null || echo "0")
            else
                cv="0"
            fi
            # Add to the reference size entry (or create it if this size isn't in sizes.txt)
            local existing_data
            existing_data=$(echo "$scaling" | jq --arg size "$interp_size" '.[$size] // {}')
            existing_data=$(echo "$existing_data" | jq \
                --argjson mean "$mean_ms" \
                --argjson median "$median_ms" \
                --argjson stddev "$stddev_ms" \
                --argjson min "$min_ms" \
                --argjson max "$max_ms" \
                --argjson cv "$cv" \
                --argjson runs "$RUNS" \
                '.logos_interp = {mean_ms: $mean, median_ms: $median, stddev_ms: $stddev, min_ms: $min, max_ms: $max, cv: $cv, runs: $runs}')
            scaling=$(echo "$scaling" | jq --arg size "$interp_size" --argjson data "$existing_data" '.[$size] = $data')
        done <<< "$results"
    fi

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

    # Output benchmark JSON
    jq -n \
        --arg id "$bench" \
        --arg name "$(bench_name "$bench")" \
        --arg desc "$(bench_desc "$bench")" \
        --arg ref "$(ref_size "$bench")" \
        --arg logos_src "$logos_src" \
        --arg gen_rust "$gen_rust" \
        --arg sizes_str "$(cat "$PROGRAMS_DIR/$bench/sizes.txt")" \
        --argjson scaling "$scaling" \
        --argjson compilation "$compilation" \
        '{
            id: $id,
            name: $name,
            description: $desc,
            reference_size: $ref,
            sizes: ($sizes_str | split(" ")),
            logos_source: $logos_src,
            generated_rust: $gen_rust,
            scaling: $scaling,
            compilation: $compilation
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
            local ref="$(ref_size "$bench")"
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

# Final assembly
jq -n \
    --arg date "$DATE" \
    --arg commit "$COMMIT" \
    --arg cpu "$CPU" \
    --arg os "$OS" \
    --arg logos_version "$LOGOS_VER" \
    --argjson versions "$VERSIONS" \
    --argjson languages "$LANGUAGES" \
    --argjson benchmarks "$BENCHMARKS_JSON" \
    --argjson geo_mean "$GEO_MEAN" \
    '{
        schema_version: 1,
        metadata: {
            date: $date,
            commit: $commit,
            logos_version: $logos_version,
            cpu: $cpu,
            os: $os,
            versions: $versions
        },
        languages: $languages,
        benchmarks: $benchmarks,
        summary: {
            geometric_mean_speedup_vs_c: $geo_mean
        }
    }' > "$RESULTS_DIR/latest.json"

ok "Phase 5 complete: results written to $RESULTS_DIR/latest.json"

# Archive with version
cp "$RESULTS_DIR/latest.json" "$RESULTS_DIR/history/v${LOGOS_VER}.json"

info "Benchmark suite complete!"
