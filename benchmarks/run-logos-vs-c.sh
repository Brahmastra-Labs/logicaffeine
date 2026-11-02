#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE LOGOS vs C Benchmark
#
# Builds and benchmarks LOGOS (release) against C at calibrated sizes
# (100-2000ms C runtime) so process-startup noise is irrelevant. Ends with a
# worst-first ratio table — the top rows are the optimization targets.
#
# Both sides build through suite_build, with the commands lib/languages.tsv
# publishes, and time pinned to the performance cores, as in run.sh. Built for
# the optimization loop: C baselines are measured ONCE and cached in
# results/c-baselines.json against the C binary's digest (C never changes
# between compiler iterations), so a typical run only re-measures LOGOS. Output is teed to
# logs/optimization/logos-vs-c.log. Results JSON goes to
# results/local-logos-vs-c.json in the same schema as results/latest.json.
#
# Usage: bash benchmarks/run-logos-vs-c.sh
#
# Environment knobs:
#   ONLY=fib,sieve       Comma-separated subset of benchmarks to run.
#   RUNS=10              Timed runs per benchmark per language.
#   WARMUP=2             Warmup runs per benchmark per language.
#   SIZE_<bench>=N       Per-benchmark size override (e.g. SIZE_fib=42).
#                        Sizes without an expected_<N>.txt are verified by
#                        C/LOGOS output agreement.
#   VERIFY=0             Skip the correctness phase (saves one full-size run
#                        per side when iterating hard on speed).
#   DUMPS=1              Also emit assembly + LLVM IR for both sides into
#                        asm/ (3 extra cargo builds per LOGOS benchmark —
#                        slow; use with ONLY= for the bench being studied).
#   FORCE_BASELINE=1     Re-measure cached C baselines.
#   OUT=results/local-logos-vs-c.json  Output JSON path (relative to benchmarks/).
#   FORCE_BUILD=1        Rebuild every binary. By default a binary is reused
#                        when the same command and toolchain built it and it
#                        is newer than its source; LOGOS binaries additionally
#                        rebuild whenever largo itself was rebuilt.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw-logos-vs-c"
ASM_DIR="$SCRIPT_DIR/asm"

# The benchmark list is the suite registry (benchmarks/lib/benchmarks.tsv), loaded here with the rest of it.
. "$SCRIPT_DIR/lib/suite.sh"
BIN_DIR="$SUITE_BIN"
GENERATED_DIR="$SUITE_GENERATED"

WARMUP="${WARMUP:-2}"
RUNS="${RUNS:-10}"
TIMEOUT=180
HYPERFINE_TIMEOUT=600  # 10 min per hyperfine invocation
OUT="${OUT:-results/local-logos-vs-c.json}"
BASELINES_FILE="$RESULTS_DIR/c-baselines.json"

LOG_DIR="$SCRIPT_DIR/../logs/optimization"
mkdir -p "$LOG_DIR"
exec > >(tee "$LOG_DIR/logos-vs-c.log") 2>&1

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

# A benchmark's size: SIZE_<bench> when set, else its registry dev_size, where C runs long enough (hundreds of
# milliseconds) that process startup is noise.
bench_size() {
    local override="SIZE_$1"
    echo "${!override:-${BENCH_DEV_SIZE[$1]}}"
}

# ===========================================================================
# Phase 1: Build
# ===========================================================================
info "Phase 1: Building LOGOS (release) and C..."

info "Building largo${LARGO_FEATURES:+ (features: $LARGO_FEATURES)}..."
. "$SCRIPT_DIR/lib/largo.sh"
largo_build "${LARGO_FEATURES:-}" || exit 1
ok "largo built"

# DUMPS=1: the assembly and LLVM IR of both sides, for reading what the optimizer made of a benchmark. C through clang
# at the C row's optimization and float flags (gcc's -flto output is LTO bytecode, not assembly); LOGOS through a
# second `largo build --release` whose Rust is re-emitted by `cargo rustc --emit`.
dump_c() {
    local bench="$1" flags="-O3 -march=native -ffp-contract=off"
    clang -S $flags -fno-asynchronous-unwind-tables \
        -o "$ASM_DIR/${bench}_c.s" "$PROGRAMS_DIR/$bench/main.c" 2>/dev/null && ok "  C asm" || true
    clang -S -emit-llvm $flags \
        -o "$ASM_DIR/${bench}_c.ll" "$PROGRAMS_DIR/$bench/main.c" 2>/dev/null && ok "  C llvm-ir" || true
}

dump_logos() {
    local bench="$1" proj project_dir file
    proj=$(mktemp -d)
    _logos_scaffold "$bench" "$proj"
    if (cd "$proj" && "$LARGO" -q build --release 2>/dev/null); then
        project_dir="$proj/target/release/build"
        sed -i.bak 's/strip = true/strip = false/' "$project_dir/Cargo.toml" 2>/dev/null || true
        for kind in asm:s llvm-ir:ll; do
            (cd "$project_dir" && cargo rustc --release -- --emit="${kind%%:*}" 2>/dev/null) || continue
            file=$(find "$project_dir/target/release/deps" "${CARGO_TARGET_DIR:-/nonexistent}/release/deps" \
                -name "bench-*.${kind##*:}" 2>/dev/null | head -1)
            [ -n "$file" ] && cp "$file" "$ASM_DIR/${bench}_logos.${kind##*:}" && ok "  LOGOS ${kind%%:*}" || true
        done
    fi
    rm -rf "$proj"
}

mkdir -p "$RAW_DIR/build"
for bench in "${BENCHMARKS[@]}"; do
    info "Building $bench..."
    for id in c logos_release; do
        suite_has "$bench" "$id" || continue
        log="$RAW_DIR/build/${bench}_${id}.log"
        if suite_build "$bench" "$id" > "$log" 2>&1; then
            if [ "$SUITE_BUILD_REUSED" = 1 ]; then ok "  ${LANG_LABEL[$id]} (cached)"; else ok "  ${LANG_LABEL[$id]}"; fi
        else
            fail "  ${LANG_LABEL[$id]} build failed ($log)"
            continue
        fi
        [ "${DUMPS:-}" = "1" ] || continue
        case "$id" in
            c) command -v clang &>/dev/null && { [ "$SUITE_BUILD_REUSED" = 1 ] && [ -f "$ASM_DIR/${bench}_c.s" ] && [ -f "$ASM_DIR/${bench}_c.ll" ] || dump_c "$bench"; } ;;
            logos_release) [ "$SUITE_BUILD_REUSED" = 1 ] && [ -f "$ASM_DIR/${bench}_logos.s" ] && [ -f "$ASM_DIR/${bench}_logos.ll" ] || dump_logos "$bench" ;;
        esac
    done
done

ok "Phase 1 complete"

# ===========================================================================
# Phase 2: Verify Correctness
# ===========================================================================
ERRORS=0

if [ "${VERIFY:-1}" = "0" ]; then
    warn "Phase 2: skipped (VERIFY=0)"
else
info "Phase 2: Verifying correctness at calibrated sizes..."

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
    info "Verifying $bench (n=$size)..."
    if [ -f "$expected_file" ]; then
        expected=$(cat "$expected_file")
        [ -f "$BIN_DIR/${bench}_c" ]             && verify "$bench" "C" "$BIN_DIR/${bench}_c" "$size" "$expected"
        [ -f "$BIN_DIR/${bench}_logos_release" ] && verify "$bench" "LOGOS (release)" "$BIN_DIR/${bench}_logos_release" "$size" "$expected"
    elif [ -f "$BIN_DIR/${bench}_c" ] && [ -f "$BIN_DIR/${bench}_logos_release" ]; then
        # No expected file at this size — verify by C/LOGOS output agreement.
        c_output=$(run_timeout "$TIMEOUT" bash -c "$BIN_DIR/${bench}_c $size" 2>/dev/null | tr -d '[:space:]') || true
        if [ -z "$c_output" ]; then
            fail "  C: no output at size $size"
            ERRORS=$((ERRORS + 1))
            continue
        fi
        ok "  C: $c_output (reference)"
        verify "$bench" "LOGOS (release)" "$BIN_DIR/${bench}_logos_release" "$size" "$c_output"
    else
        warn "No expected output for $bench at size $size and missing a binary — skipping verification"
    fi
done

if [ "$ERRORS" -gt 0 ]; then
    fail "Verification failed: $ERRORS errors"
    exit 1
fi
ok "Phase 2 complete: all verified"
fi

# ===========================================================================
# Phase 3: Benchmark — per-language timeout isolation
# ===========================================================================
info "Phase 3: Benchmarking LOGOS (release) vs C at calibrated sizes ($WARMUP warmup, $RUNS runs)..."

PER_LANG_DIR="$RAW_DIR/per_lang"
mkdir -p "$PER_LANG_DIR"

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

declare -A LANG_TIMEOUTS

try_bench() {
    local lang_id="$1" label="$2" cmd="$3"

    if [[ "${LANG_TIMEOUTS[${bench}_${lang_id}]:-}" == "1" ]]; then
        warn "  Skipping $label (timed out at smaller size)"
        return
    fi

    local pf="$PER_LANG_DIR/${bench}_${size}_${lang_id}.json"
    local rc=0
    run_timeout "$HYPERFINE_TIMEOUT" taskset -c "$(suite_cpus)" hyperfine \
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

# C baselines are static between compiler iterations: measured once, stored
# as raw hyperfine result objects in results/c-baselines.json beside the
# SHA-256 of the binary they timed, and re-measured whenever the C binary is
# not that one (main.c, the build command or the compiler changed) or
# FORCE_BASELINE=1.
[ -f "$BASELINES_FILE" ] || echo '{"meta":{},"baselines":{}}' > "$BASELINES_FILE"

c_digest() {
    sha256sum "$BIN_DIR/${1}_c" | cut -d' ' -f1
}

c_baseline_get() {
    jq -ce --arg b "$1" --arg s "$2" --arg d "$(c_digest "$1")" \
        'select(.binaries[$b] == $d) | .baselines[$b][$s] // empty' "$BASELINES_FILE" 2>/dev/null
}

c_baseline_put() {
    local bench="$1" size="$2" pf="$3" tmp digest
    digest=$(c_digest "$bench")
    tmp=$(mktemp)
    jq --arg b "$bench" --arg s "$size" --slurpfile r "$pf" --arg d "$digest" \
        --arg gcc "$(suite_toolchain c)" \
        --arg date "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
        'if .binaries[$b] != $d then .baselines[$b] = {} else . end
         | .binaries[$b] = $d
         | .baselines[$b][$s] = $r[0].results[0]
         | .meta = {gcc: $gcc, updated: $date}' \
        "$BASELINES_FILE" > "$tmp" && mv "$tmp" "$BASELINES_FILE"
}

for bench in "${BENCHMARKS[@]}"; do
    size="$(bench_size "$bench")"

    # Skip benchmarks where neither binary exists
    if [ ! -f "$BIN_DIR/${bench}_c" ] && [ ! -f "$BIN_DIR/${bench}_logos_release" ]; then
        warn "No binaries for $bench — skipping"
        continue
    fi

    info "Benchmarking $bench at n=$size..."
    MERGE_FILES=()

    if [ -f "$BIN_DIR/${bench}_c" ]; then
        baseline=""
        if [ "${FORCE_BASELINE:-}" != "1" ]; then
            baseline=$(c_baseline_get "$bench" "$size") || baseline=""
        fi
        if [ -n "$baseline" ]; then
            pf="$PER_LANG_DIR/${bench}_${size}_c.json"
            printf '{"results":[%s]}' "$baseline" > "$pf"
            MERGE_FILES+=("$pf")
            ok "  C baseline (cached): $(echo "$baseline" | jq -r '(.mean * 1000 * 10 | round) / 10') ms"
        else
            try_bench c "C" "$BIN_DIR/${bench}_c $size"
            [ -f "$PER_LANG_DIR/${bench}_${size}_c.json" ] && \
                c_baseline_put "$bench" "$size" "$PER_LANG_DIR/${bench}_${size}_c.json"
        fi
    fi
    [ -f "$BIN_DIR/${bench}_logos_release" ] && try_bench logos_release "LOGOS (release)" "$BIN_DIR/${bench}_logos_release $size"

    # Merge per-language results
    if [ ${#MERGE_FILES[@]} -gt 0 ]; then
        merge_hyperfine_results "$RAW_DIR/${bench}_${size}.json" "${MERGE_FILES[@]}"
    else
        echo '{"results":[]}' > "$RAW_DIR/${bench}_${size}.json"
    fi

    # Collect per-language timeout markers
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
# Phase 4: Assemble results JSON
# ===========================================================================
info "Phase 4: Assembling $OUT..."

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
    echo "${BENCH_NAME[$1]}"
}

bench_desc() {
    echo "${BENCH_DESC[$1]}"
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
# Phase 5: Ratio table — worst-first, the top rows are optimization targets
# ===========================================================================
echo
info "LOGOS vs C (ratio > 1.00 means LOGOS is slower than C):"
echo

ROWS=""
for bench in "${BENCHMARKS[@]}"; do
    size="$(bench_size "$bench")"
    c_file="$PER_LANG_DIR/${bench}_${size}_c.json"
    l_file="$PER_LANG_DIR/${bench}_${size}_logos_release.json"
    c_ms="" l_ms=""
    [ -f "$c_file" ] && c_ms=$(jq -r '.results[0].mean * 1000' "$c_file")
    [ -f "$l_file" ] && l_ms=$(jq -r '.results[0].mean * 1000' "$l_file")
    if [ -n "$c_ms" ] && [ -n "$l_ms" ]; then
        ratio=$(echo "$l_ms / $c_ms" | bc -l)
        ROWS+=$(printf "%s|%s|%s|%s|%s\n" "$ratio" "$bench" "$size" "$c_ms" "$l_ms")$'\n'
    else
        [ -z "$c_ms" ] && warn "  $bench: no C result"
        [ -z "$l_ms" ] && warn "  $bench: no LOGOS result"
    fi
done

printf "%-14s %12s %12s %12s %10s\n" "benchmark" "n" "C ms" "LOGOS ms" "ratio"
printf '%.0s-' $(seq 1 64); echo
echo -n "$ROWS" | sort -t'|' -k1 -gr | while IFS='|' read -r ratio bench size c_ms l_ms; do
    [ -z "$bench" ] && continue
    printf "%-14s %12s %12.1f %12.1f %9.2fx\n" "$bench" "$size" "$c_ms" "$l_ms" "$ratio"
done

WINS=0
LOSSES=0
while IFS='|' read -r ratio bench size c_ms l_ms; do
    [ -z "$bench" ] && continue
    if [ "$(echo "$ratio > 1.0" | bc -l)" = "1" ]; then
        LOSSES=$((LOSSES + 1))
    else
        WINS=$((WINS + 1))
    fi
done <<< "$ROWS"

echo
fail "Here's where C beats LOGOS the worst:"
echo -n "$ROWS" | sort -t'|' -k1 -gr | head -3 | while IFS='|' read -r ratio bench size c_ms l_ms; do
    [ -z "$bench" ] && continue
    if [ "$(echo "$ratio > 1.0" | bc -l)" = "1" ]; then
        printf "  %-14s LOGOS is %.2fx slower (C %.1fms vs LOGOS %.1fms at n=%s)\n" \
            "$bench" "$ratio" "$c_ms" "$l_ms" "$size"
        echo "    -> ONLY=$bench DUMPS=1 bash benchmarks/run-logos-vs-c.sh  # then diff asm/${bench}_c.s asm/${bench}_logos.s"
    fi
done

LOGOS_GEO=$(echo "$GEO_MEAN" | jq -r '.logos_release // empty')
echo
info "LOGOS wins $WINS / $((WINS + LOSSES)) benchmarks against C"
if [ -n "$LOGOS_GEO" ]; then
    info "Geometric mean: LOGOS runs at ${LOGOS_GEO}x the speed of C"
fi
info "Full log: logs/optimization/logos-vs-c.log"
info "LOGOS vs C benchmark complete!"
