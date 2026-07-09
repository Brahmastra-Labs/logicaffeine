#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE Quick Benchmark Suite
#
# Runs ALL 32 benchmarks across ALL languages at configurable sizes and
# prints a full coverage matrix (benchmark x language) so gaps, failures,
# and timeouts are immediately visible. Produces a results JSON in the same
# schema as run.sh, written to results/local.json by default so the
# CI-produced results/latest.json is never clobbered.
#
# Usage: bash benchmarks/run-quick.sh
#
# Environment knobs:
#   SIZE=quick|ref|max   Size profile (default: quick).
#                          quick = small sizes, full suite in minutes
#                          ref   = run.sh reference sizes
#                          max   = largest size in each sizes.txt
#   SIZE_<bench>=N       Per-benchmark size override (e.g. SIZE_fib=32).
#                        Sizes without an expected_<N>.txt are verified by
#                        cross-language output agreement (C is the reference).
#   ONLY=fib,sieve       Comma-separated subset of benchmarks to run.
#   RUNS=10              Timed runs per benchmark per language.
#   WARMUP=3             Warmup runs per benchmark per language.
#   SKIP_LANGS=zig,nim   Comma-separated language ids to skip.
#   FORCE_BUILD=1        Rebuild every binary. By default a binary newer than
#                        its source is reused; LOGOS binaries additionally
#                        rebuild whenever largo itself was rebuilt.
#   OUT=results/local.json  Output JSON path (relative to benchmarks/).
#                           Set OUT=results/latest.json to preview results
#                           in the web frontend (revert before committing).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="$SCRIPT_DIR/bin"
PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw-local"
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

LOG_DIR="$SCRIPT_DIR/../logs/optimization"
mkdir -p "$LOG_DIR"
exec > >(tee "$LOG_DIR/quick-matrix.log") 2>&1

SIZE="${SIZE:-quick}"
WARMUP="${WARMUP:-3}"
RUNS="${RUNS:-10}"
TIMEOUT=120
BUILD_TIMEOUT=60
HYPERFINE_TIMEOUT=300  # 5 min safety net per hyperfine invocation
SKIP_LANGS="${SKIP_LANGS:-}"
OUT="${OUT:-results/local.json}"

LANG_IDS=(c cpp rust zig go java js python ruby nim logos_release)

lang_label() {
    case "$1" in
        c) echo "C" ;; cpp) echo "C++" ;; rust) echo "Rust" ;; zig) echo "Zig" ;;
        go) echo "Go" ;; java) echo "Java" ;; js) echo "JavaScript" ;;
        python) echo "Python" ;; ruby) echo "Ruby" ;; nim) echo "Nim" ;;
        logos_release) echo "LOGOS (release)" ;;
    esac
}

lang_color() {
    case "$1" in
        c) echo "#555555" ;; cpp) echo "#f34b7d" ;; rust) echo "#dea584" ;; zig) echo "#f7a41d" ;;
        go) echo "#3fb950" ;; java) echo "#b07219" ;; js) echo "#f7df1e" ;;
        python) echo "#3776ab" ;; ruby) echo "#cc342d" ;; nim) echo "#ffe953" ;;
        logos_release) echo "#00d4ff" ;;
    esac
}

lang_tier() {
    case "$1" in
        c|cpp|rust|zig|logos_release) echo "systems" ;;
        go|java|js) echo "managed" ;;
        python|ruby) echo "interpreted" ;;
        nim) echo "transpiled" ;;
    esac
}

lang_tool() {
    case "$1" in
        c) echo gcc ;; cpp) echo g++ ;; rust) echo rustc ;; zig) echo zig ;;
        go) echo go ;; java) echo javac ;; js) echo node ;;
        python) echo python3 ;; ruby) echo ruby ;; nim) echo nim ;;
        logos_release) echo cargo ;;
    esac
}

lang_src() {
    case "$1" in
        c) echo main.c ;; cpp) echo main.cpp ;; rust) echo main.rs ;; zig) echo main.zig ;;
        go) echo main.go ;; java) echo Main.java ;; js) echo main.js ;;
        python) echo main.py ;; ruby) echo main.rb ;; nim) echo main.nim ;;
        logos_release) echo main.lg ;;
    esac
}

# Build artifact for benchmark $1 in language $2 (empty = nothing to build).
bin_path() {
    local bench="$1" lid="$2"
    case "$lid" in
        c)    echo "$BIN_DIR/${bench}_c" ;;
        cpp)  echo "$BIN_DIR/${bench}_cpp" ;;
        rust) echo "$BIN_DIR/${bench}_rs" ;;
        zig)  echo "$BIN_DIR/${bench}_zig" ;;
        go)   echo "$BIN_DIR/${bench}_go" ;;
        java) echo "$BIN_DIR/java/$bench/Main.class" ;;
        nim)  echo "$BIN_DIR/${bench}_nim" ;;
        logos_release) echo "$BIN_DIR/${bench}_logos_release" ;;
        js|python|ruby) echo "" ;;
    esac
}

# A cached binary is reusable when it is newer than its source — and for
# LOGOS, newer than the largo compiler too, so iterating on the compiler
# invalidates exactly the LOGOS binaries and nothing else. FORCE_BUILD=1
# rebuilds everything.
build_cached() {
    local bench="$1" lid="$2"
    [ "${FORCE_BUILD:-}" = "1" ] && return 1
    local bin src
    bin="$(bin_path "$bench" "$lid")"
    [ -z "$bin" ] && return 0
    src="$PROGRAMS_DIR/$bench/$(lang_src "$lid")"
    [ -f "$bin" ] && [ "$bin" -nt "$src" ] || return 1
    if [ "$lid" = "logos_release" ]; then
        [ "$bin" -nt "$LARGO" ] || return 1
    fi
    return 0
}

# Command that runs benchmark $1 in language $2 (size appended by caller).
lang_cmd() {
    local bench="$1" lid="$2"
    case "$lid" in
        c)    echo "$BIN_DIR/${bench}_c" ;;
        cpp)  echo "$BIN_DIR/${bench}_cpp" ;;
        rust) echo "$BIN_DIR/${bench}_rs" ;;
        zig)  echo "$BIN_DIR/${bench}_zig" ;;
        go)   echo "$BIN_DIR/${bench}_go" ;;
        java) echo "java -cp $BIN_DIR/java/$bench Main" ;;
        js)   echo "node $PROGRAMS_DIR/$bench/main.js" ;;
        python) echo "python3 $PROGRAMS_DIR/$bench/main.py" ;;
        ruby) echo "RUBY_THREAD_VM_STACK_SIZE=67108864 ruby $PROGRAMS_DIR/$bench/main.rb" ;;
        nim)  echo "$BIN_DIR/${bench}_nim" ;;
        logos_release) echo "$BIN_DIR/${bench}_logos_release" ;;
    esac
}

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

# Reference sizes — mirrors ref_size() in run.sh.
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

max_size() {
    tr ' ' '\n' < "$PROGRAMS_DIR/$1/sizes.txt" | tail -1
}

# Size resolution: SIZE_<bench> override first, then the SIZE profile.
bench_size() {
    local bench="$1"
    local override_var="SIZE_${bench}"
    if [ -n "${!override_var:-}" ]; then
        echo "${!override_var}"
        return
    fi
    case "$SIZE" in
        quick) quick_size "$bench" ;;
        ref)   ref_size "$bench" ;;
        max)   max_size "$bench" ;;
        *)     fail "Unknown SIZE profile '$SIZE' (expected quick|ref|max)"; exit 1 ;;
    esac
}

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
            fail "Unknown benchmark in ONLY: '$want'"
            exit 1
        fi
    done
    BENCHMARKS=("${SELECTED[@]}")
fi

# ===========================================================================
# Phase 0: Preflight — toolchains and source coverage
# ===========================================================================
info "Phase 0: Preflight checks..."

if ! command -v hyperfine &>/dev/null; then
    fail "hyperfine not found — run: bash benchmarks/setup-local.sh"
    exit 1
fi

declare -A LANG_AVAILABLE
echo
printf "  %-12s %-10s %s\n" "Language" "Status" "Toolchain"
printf "  %-12s %-10s %s\n" "--------" "------" "---------"
for lid in "${LANG_IDS[@]}"; do
    tool="$(lang_tool "$lid")"
    if skip_lang "$lid"; then
        printf "  %-12s ${YELLOW}%-10s${NC} %s\n" "$(lang_label "$lid")" "SKIPPED" "via SKIP_LANGS"
    elif [ "$lid" = "java" ] && ! command -v java &>/dev/null; then
        printf "  %-12s ${RED}%-10s${NC} %s\n" "$(lang_label "$lid")" "MISSING" "java runtime not on PATH"
    elif command -v "$tool" &>/dev/null; then
        LANG_AVAILABLE[$lid]=1
        printf "  %-12s ${GREEN}%-10s${NC} %s\n" "$(lang_label "$lid")" "ok" "$tool"
    else
        printf "  %-12s ${RED}%-10s${NC} %s\n" "$(lang_label "$lid")" "MISSING" "$tool not on PATH — run benchmarks/setup-local.sh"
    fi
done
echo

MISSING_SOURCES=0
for bench in "${BENCHMARKS[@]}"; do
    for lid in "${LANG_IDS[@]}"; do
        src="$PROGRAMS_DIR/$bench/$(lang_src "$lid")"
        if [ ! -f "$src" ]; then
            warn "Missing source: $src"
            MISSING_SOURCES=$((MISSING_SOURCES + 1))
        fi
    done
done
if [ "$MISSING_SOURCES" -eq 0 ]; then
    ok "Source coverage complete: ${#BENCHMARKS[@]} benchmarks x ${#LANG_IDS[@]} languages"
else
    warn "$MISSING_SOURCES missing source files (marked in the matrix)"
fi

info "Profile: SIZE=$SIZE  RUNS=$RUNS  WARMUP=$WARMUP  OUT=$OUT"

# STATUS[bench|lang]: notool skip nofile build_fail verify_fail timeout ok
declare -A STATUS
declare -A MEAN_MS
declare -A HAS_RESULT

for bench in "${BENCHMARKS[@]}"; do
    for lid in "${LANG_IDS[@]}"; do
        if skip_lang "$lid"; then
            STATUS["$bench|$lid"]=skip
        elif [ -z "${LANG_AVAILABLE[$lid]:-}" ]; then
            STATUS["$bench|$lid"]=notool
        elif [ ! -f "$PROGRAMS_DIR/$bench/$(lang_src "$lid")" ]; then
            STATUS["$bench|$lid"]=nofile
        else
            STATUS["$bench|$lid"]=pending
        fi
    done
done

runnable() {
    [ "${STATUS[$1|$2]:-}" = "pending" ]
}

# ===========================================================================
# Phase 1: Build Everything
# ===========================================================================
info "Phase 1: Building all implementations..."

LARGO=""
if [ -n "${LANG_AVAILABLE[logos_release]:-}" ]; then
    info "Building largo..."
    cargo build -p logicaffeine-cli --release --manifest-path "$SCRIPT_DIR/../Cargo.toml" 2>/dev/null
    for candidate in \
        "$LOGOS_TARGET_DIR/release/logicaffeine-cli" \
        "$LOGOS_TARGET_DIR/release/largo" \
        "$SCRIPT_DIR/../target/release/logicaffeine-cli" \
        "$SCRIPT_DIR/../target/release/largo"; do
        [ -f "$candidate" ] && LARGO="$candidate" && break
    done
    if [ -z "$LARGO" ]; then
        fail "Could not find largo binary"
        exit 1
    fi
    ok "largo built"
fi

build_lang() {
    local bench="$1" lid="$2"
    case "$lid" in
        c)
            gcc -O2 -o "$BIN_DIR/${bench}_c" "$PROGRAMS_DIR/$bench/main.c" -lm 2>/dev/null ;;
        cpp)
            g++ -O2 -std=c++17 -o "$BIN_DIR/${bench}_cpp" "$PROGRAMS_DIR/$bench/main.cpp" -lm 2>/dev/null ;;
        rust)
            rustc --edition 2021 -O -o "$BIN_DIR/${bench}_rs" "$PROGRAMS_DIR/$bench/main.rs" 2>/dev/null ;;
        zig)
            run_timeout "$BUILD_TIMEOUT" zig build-exe -O ReleaseFast --name "${bench}_zig" "$PROGRAMS_DIR/$bench/main.zig" 2>/dev/null && \
                mv "${bench}_zig" "$BIN_DIR/" 2>/dev/null ;;
        go)
            go build -o "$BIN_DIR/${bench}_go" "$PROGRAMS_DIR/$bench/main.go" 2>/dev/null ;;
        java)
            mkdir -p "$BIN_DIR/java/$bench"
            javac -d "$BIN_DIR/java/$bench" "$PROGRAMS_DIR/$bench/Main.java" 2>/dev/null ;;
        nim)
            nim c -d:release --hints:off -o:"$BIN_DIR/${bench}_nim" "$PROGRAMS_DIR/$bench/main.nim" 2>/dev/null ;;
        js|python|ruby)
            return 0 ;;
        logos_release)
            local logos_tmp rc=1
            logos_tmp=$(mktemp -d)
            mkdir -p "$logos_tmp/src"
            cp "$PROGRAMS_DIR/$bench/main.lg" "$logos_tmp/src/main.lg"
            cat > "$logos_tmp/Largo.toml" << 'TOML'
[package]
name = "bench"
version = "0.1.0"
entry = "src/main.lg"
TOML
            if (cd "$logos_tmp" && "$LARGO" build --release 2>/dev/null); then
                local logos_bin=""
                if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -f "$CARGO_TARGET_DIR/release/bench" ]; then
                    logos_bin="$CARGO_TARGET_DIR/release/bench"
                elif [[ "$(uname -s)" == "Darwin" ]]; then
                    logos_bin=$(find "$logos_tmp/target/release" -type f -perm +111 -name "bench" 2>/dev/null | head -1)
                else
                    logos_bin=$(find "$logos_tmp/target/release" -type f -executable -name "bench" 2>/dev/null | head -1)
                fi
                if [ -n "$logos_bin" ]; then
                    cp "$logos_bin" "$BIN_DIR/${bench}_logos_release"
                    rc=0
                fi
                local generated_rs
                generated_rs=$(find "$logos_tmp" -name "main.rs" -path "*/build/src/*" 2>/dev/null | head -1)
                [ -n "$generated_rs" ] && cp "$generated_rs" "$GENERATED_DIR/$bench.rs" 2>/dev/null || true
            fi
            rm -rf "$logos_tmp"
            return $rc ;;
    esac
}

for bench in "${BENCHMARKS[@]}"; do
    info "Building $bench..."
    for lid in "${LANG_IDS[@]}"; do
        runnable "$bench" "$lid" || continue
        if build_cached "$bench" "$lid"; then
            ok "  $(lang_label "$lid") (cached)"
        elif build_lang "$bench" "$lid"; then
            ok "  $(lang_label "$lid")"
        else
            STATUS["$bench|$lid"]=build_fail
            fail "  $(lang_label "$lid") build failed"
        fi
    done
done

ok "Phase 1 complete"

# ===========================================================================
# Phase 2: Verify — expected file when present, else cross-language agreement
# ===========================================================================
info "Phase 2: Verifying correctness..."

ERRORS=0

for bench in "${BENCHMARKS[@]}"; do
    size="$(bench_size "$bench")"
    expected=""
    expected_source=""
    expected_file="$PROGRAMS_DIR/$bench/expected_${size}.txt"
    if [ -f "$expected_file" ]; then
        expected=$(tr -d '[:space:]' < "$expected_file")
        expected_source="expected_${size}.txt"
    fi
    info "Verifying $bench (n=$size)..."

    for lid in "${LANG_IDS[@]}"; do
        runnable "$bench" "$lid" || continue
        cmd="$(lang_cmd "$bench" "$lid")"
        output=$(run_timeout "$TIMEOUT" bash -c "$cmd $size" 2>/dev/null | tr -d '[:space:]') || true
        if [ -z "$expected" ]; then
            if [ -n "$output" ]; then
                expected="$output"
                expected_source="$(lang_label "$lid") output (cross-language agreement)"
                ok "  $(lang_label "$lid"): $output (reference)"
            else
                STATUS["$bench|$lid"]=verify_fail
                fail "  $(lang_label "$lid"): no output"
                ERRORS=$((ERRORS + 1))
            fi
            continue
        fi
        if [ "$output" = "$expected" ]; then
            ok "  $(lang_label "$lid"): $output"
        else
            STATUS["$bench|$lid"]=verify_fail
            fail "  $(lang_label "$lid"): got '$output', expected '$expected' (ref: $expected_source)"
            ERRORS=$((ERRORS + 1))
        fi
    done
done

if [ "$ERRORS" -gt 0 ]; then
    warn "Phase 2: $ERRORS verification failures — those entries are excluded from benchmarking"
else
    ok "Phase 2 complete: all verified"
fi

# ===========================================================================
# Phase 3: Benchmark — per-language timeout isolation
# ===========================================================================
info "Phase 3: Benchmarking..."

PER_LANG_DIR="$RAW_DIR/per_lang"
mkdir -p "$PER_LANG_DIR"

# Merge multiple single-language hyperfine JSONs into one combined result.
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

try_bench() {
    local lid="$1"
    local label cmd pf rc=0
    label="$(lang_label "$lid")"
    cmd="$(lang_cmd "$bench" "$lid") $size"
    pf="$PER_LANG_DIR/${bench}_${size}_${lid}.json"

    run_timeout "$HYPERFINE_TIMEOUT" hyperfine \
        --warmup "$WARMUP" --runs "$RUNS" \
        --export-json "$pf" --time-unit millisecond \
        -n "$label" "$cmd" || rc=$?

    if [ "$rc" -eq 124 ] || [ "$rc" -eq 143 ]; then
        STATUS["$bench|$lid"]=timeout
        warn "  $label timed out for $bench at $size"
        echo "$HYPERFINE_TIMEOUT" > "$RAW_DIR/${bench}_${size}_${lid}.timeout"
        rm -f "$pf"
        return
    elif [ "$rc" -ne 0 ]; then
        STATUS["$bench|$lid"]=run_fail
        warn "  $label benchmark failed for $bench at $size"
        rm -f "$pf"
        return
    fi

    if [ -f "$pf" ]; then
        STATUS["$bench|$lid"]=ok
        MEAN_MS["$bench|$lid"]=$(jq -r '.results[0].mean * 1000' "$pf")
        HAS_RESULT[$lid]=1
        MERGE_FILES+=("$pf")
    fi
}

for bench in "${BENCHMARKS[@]}"; do
    size="$(bench_size "$bench")"
    info "Benchmarking $bench at n=$size..."
    MERGE_FILES=()

    for lid in "${LANG_IDS[@]}"; do
        runnable "$bench" "$lid" || continue
        try_bench "$lid"
    done

    if [ ${#MERGE_FILES[@]} -gt 0 ]; then
        merge_hyperfine_results "$RAW_DIR/${bench}_${size}.json" "${MERGE_FILES[@]}"
    else
        echo '{"results":[]}' > "$RAW_DIR/${bench}_${size}.json"
    fi

    has_timeout=false
    for tf in "$RAW_DIR/${bench}_${size}_"*.timeout; do
        [ -f "$tf" ] && has_timeout=true && break
    done
    if [ "$has_timeout" = true ]; then
        echo "$HYPERFINE_TIMEOUT" > "$RAW_DIR/${bench}_${size}.timeout"
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

# Only languages that actually produced at least one result this run.
LANGUAGES="[]"
for lid in "${LANG_IDS[@]}"; do
    [ "${HAS_RESULT[$lid]:-}" = "1" ] || continue
    LANGUAGES=$(echo "$LANGUAGES" | jq \
        --arg id "$lid" \
        --arg label "$(lang_label "$lid" | sed 's/ (release)//')" \
        --arg color "$(lang_color "$lid")" \
        --arg tier "$(lang_tier "$lid")" \
        '. + [{id: $id, label: $label, color: $color, tier: $tier}]')
done

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
    local size="$(bench_size "$bench")"
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

mkdir -p "$(dirname "$OUT")"

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
    }' > "$OUT"

rm -f "$BENCHMARKS_JSON_FILE"

ok "Results written to $OUT"

# ===========================================================================
# Phase 5: Coverage Matrix
# ===========================================================================
echo
info "Coverage matrix (mean ms; SIZE=$SIZE):"
echo

matrix_cell() {
    case "${STATUS[$1|$2]:-}" in
        ok)          printf "%10.1f" "${MEAN_MS[$1|$2]}" ;;
        timeout)     printf "%10s" "T/O" ;;
        verify_fail) printf "%10s" "FAIL" ;;
        run_fail)    printf "%10s" "ERR" ;;
        build_fail)  printf "%10s" "BUILD" ;;
        nofile)      printf "%10s" "NOSRC" ;;
        notool|skip) printf "%10s" "-" ;;
        *)           printf "%10s" "?" ;;
    esac
}

printf "%-14s %12s" "benchmark" "n"
for lid in "${LANG_IDS[@]}"; do
    case "$lid" in
        c) h="C" ;; cpp) h="C++" ;; rust) h="Rust" ;; zig) h="Zig" ;;
        go) h="Go" ;; java) h="Java" ;; js) h="JS" ;;
        python) h="Py" ;; ruby) h="Rb" ;; nim) h="Nim" ;;
        logos_release) h="LOGOS" ;;
    esac
    printf "%10s" "$h"
done
echo
printf '%.0s-' $(seq 1 $((26 + 10 * ${#LANG_IDS[@]}))); echo

for bench in "${BENCHMARKS[@]}"; do
    printf "%-14s %12s" "$bench" "$(bench_size "$bench")"
    for lid in "${LANG_IDS[@]}"; do
        matrix_cell "$bench" "$lid"
    done
    echo
done

echo
echo "Legend: T/O=timed out  FAIL=wrong output  ERR=run failed  BUILD=compile failed  NOSRC=missing source  -=toolchain missing/skipped"
echo
info "Geometric mean speedup vs C (>1 means faster than C):"
echo "$GEO_MEAN" | jq -r 'to_entries | sort_by(-.value) | .[] | "  \(.key): \(.value)x"'

TOTAL_BAD=0
for key in "${!STATUS[@]}"; do
    case "${STATUS[$key]}" in
        verify_fail|run_fail|build_fail|nofile) TOTAL_BAD=$((TOTAL_BAD + 1)) ;;
    esac
done

echo
if [ "$TOTAL_BAD" -gt 0 ]; then
    fail "Quick benchmark suite finished with $TOTAL_BAD broken benchmark/language pairs (see matrix)"
    exit 1
fi
ok "Quick benchmark suite complete!"
