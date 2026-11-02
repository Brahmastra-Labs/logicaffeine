#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE Quick Benchmark Suite
#
# Runs every benchmark in the registry across the compiled and managed
# languages, Python and Ruby at configurable sizes and prints a full coverage
# matrix (benchmark x language) so gaps, failures, and timeouts are immediately
# visible. Every implementation is built by suite_build with the command
# lib/languages.tsv publishes and timed pinned to the performance cores, as in
# run.sh. Produces a results JSON in the same schema as run.sh, written to
# results/local.json by default so results/latest.json is never clobbered.
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
#   FORCE_BUILD=1        Rebuild every binary. By default a binary is reused
#                        when the same command and toolchain built it and it
#                        is newer than its source; LOGOS binaries additionally
#                        rebuild whenever largo itself was rebuilt.
#   OUT=results/local.json  Output JSON path (relative to benchmarks/).
#                           Set OUT=results/latest.json to preview results
#                           in the web frontend (revert before committing).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw-local"

# The benchmark list is the suite registry (benchmarks/lib/benchmarks.tsv), loaded here with the rest of it.
. "$SCRIPT_DIR/lib/suite.sh"
BIN_DIR="$SUITE_BIN"
GENERATED_DIR="$SUITE_GENERATED"

LOG_DIR="$SCRIPT_DIR/../logs/optimization"
mkdir -p "$LOG_DIR"
exec > >(tee "$LOG_DIR/quick-matrix.log") 2>&1

SIZE="${SIZE:-quick}"
WARMUP="${WARMUP:-3}"
RUNS="${RUNS:-10}"
TIMEOUT=120
HYPERFINE_TIMEOUT=300  # 5 min safety net per hyperfine invocation
SKIP_LANGS="${SKIP_LANGS:-}"
OUT="${OUT:-results/local.json}"

# The suite rows this script times, plus Python and Ruby, which the suite verifies (verify.sh) but never times at its
# sizes. A suite row's label, colour, tier, source, build and run come from lib/languages.tsv.
QUICK_IDS=(c cpp rust zig go java js python ruby nim logos_release)
declare -A INTERP_LABEL=([python]=Python [ruby]=Ruby) INTERP_COLOR=([python]="#3776ab" [ruby]="#cc342d")
declare -A INTERP_SOURCE=([python]=main.py [ruby]=main.rb)
declare -A INTERP_RUN=([python]="python3 {src} {n}" [ruby]="RUBY_THREAD_VM_STACK_SIZE=67108864 ruby {src} {n}")
declare -A INTERP_VERSION=([python]="python3 --version" [ruby]="ruby --version")

lang_label() { if [ -n "${INTERP_LABEL[$1]:-}" ]; then echo "${INTERP_LABEL[$1]}"; else echo "${LANG_LABEL[$1]}"; fi; }
lang_color() { if [ -n "${INTERP_COLOR[$1]:-}" ]; then echo "${INTERP_COLOR[$1]}"; else echo "${LANG_COLOR[$1]}"; fi; }
lang_tier() { if [ -n "${INTERP_LABEL[$1]:-}" ]; then echo interpreted; else echo "${LANG_TIER[$1]}"; fi; }
lang_src() { if [ -n "${INTERP_SOURCE[$1]:-}" ]; then echo "${INTERP_SOURCE[$1]}"; else echo "${LANG_SOURCE[$1]}"; fi; }

lang_tool() {
    case "$1" in
        c) echo gcc ;; cpp) echo g++ ;; rust) echo rustc ;; zig) echo zig ;;
        go) echo go ;; java) echo javac ;; js) echo node ;;
        python) echo python3 ;; ruby) echo ruby ;; nim) echo nim ;;
        logos_release) echo cargo ;;
    esac
}

# The command that runs benchmark $1 in language $2 at size $3.
run_cmd() {
    if [ -n "${INTERP_RUN[$2]:-}" ]; then
        local t="${INTERP_RUN[$2]}"
        t="${t//\{src\}/$PROGRAMS_DIR/$1/${INTERP_SOURCE[$2]}}"
        echo "${t//\{n\}/$3}"
    else
        suite_run_cmd "$1" "$2" "$3"
    fi
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
    echo "${BENCH_QUICK_SIZE[$1]}"
}

# Reference sizes — mirrors ref_size() in run.sh.
ref_size() {
    echo "${BENCH_REF[$1]}"
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
for lid in "${QUICK_IDS[@]}"; do
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
    for lid in "${QUICK_IDS[@]}"; do
        src="$PROGRAMS_DIR/$bench/$(lang_src "$lid")"
        if [ ! -f "$src" ]; then
            warn "Missing source: $src"
            MISSING_SOURCES=$((MISSING_SOURCES + 1))
        fi
    done
done
if [ "$MISSING_SOURCES" -eq 0 ]; then
    ok "Source coverage complete: ${#BENCHMARKS[@]} benchmarks x ${#QUICK_IDS[@]} languages"
else
    warn "$MISSING_SOURCES missing source files (marked in the matrix)"
fi

info "Profile: SIZE=$SIZE  RUNS=$RUNS  WARMUP=$WARMUP  OUT=$OUT"

# STATUS[bench|lang]: notool skip nofile build_fail verify_fail timeout ok
declare -A STATUS
declare -A MEAN_MS
declare -A HAS_RESULT

for bench in "${BENCHMARKS[@]}"; do
    for lid in "${QUICK_IDS[@]}"; do
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
    . "$SCRIPT_DIR/lib/largo.sh"
    largo_build || exit 1
    ok "largo built"
fi

mkdir -p "$RAW_DIR/build"
for bench in "${BENCHMARKS[@]}"; do
    info "Building $bench..."
    for lid in "${QUICK_IDS[@]}"; do
        runnable "$bench" "$lid" || continue
        [ -n "${INTERP_RUN[$lid]:-}" ] && continue
        log="$RAW_DIR/build/${bench}_${lid}.log"
        if suite_build "$bench" "$lid" > "$log" 2>&1; then
            if [ "$SUITE_BUILD_REUSED" = 1 ]; then ok "  $(lang_label "$lid") (cached)"; else ok "  $(lang_label "$lid")"; fi
        else
            STATUS["$bench|$lid"]=build_fail
            fail "  $(lang_label "$lid") build failed ($log)"
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

    for lid in "${QUICK_IDS[@]}"; do
        runnable "$bench" "$lid" || continue
        cmd="$(run_cmd "$bench" "$lid" "$size")"
        output=$(run_timeout "$TIMEOUT" bash -c "$cmd" 2>/dev/null | tr -d '[:space:]') || true
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
    cmd="$(run_cmd "$bench" "$lid" "$size")"
    pf="$PER_LANG_DIR/${bench}_${size}_${lid}.json"

    run_timeout "$HYPERFINE_TIMEOUT" taskset -c "$(suite_cpus)" hyperfine \
        --warmup "$WARMUP" --runs "$RUNS" \
        --export-json "$pf" --time-unit millisecond \
        -n "$lid" "$cmd" || rc=$?

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

    for lid in "${QUICK_IDS[@]}"; do
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

VERSIONS="{}"
for lid in "${QUICK_IDS[@]}"; do
    [ "$lid" = logos_release ] && continue
    if [ -n "${INTERP_VERSION[$lid]:-}" ]; then
        version=$(${INTERP_VERSION[$lid]} 2>/dev/null | head -1 || echo unknown)
    else
        version=$(suite_toolchain "$lid")
    fi
    VERSIONS=$(jq -c --arg k "$lid" --arg v "${version:-unknown}" '.[$k]=$v' <<< "$VERSIONS")
done

# Only languages that actually produced at least one result this run.
LANGUAGES="[]"
for lid in "${QUICK_IDS[@]}"; do
    [ "${HAS_RESULT[$lid]:-}" = "1" ] || continue
    LANGUAGES=$(echo "$LANGUAGES" | jq \
        --arg id "$lid" \
        --arg label "$(lang_label "$lid")" \
        --arg color "$(lang_color "$lid")" \
        --arg tier "$(lang_tier "$lid")" \
        '. + [{id: $id, label: $label, color: $color, tier: $tier}]')
done

# A hyperfine result is named by its language id.
lang_id() {
    echo "$1"
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
for lid in "${QUICK_IDS[@]}"; do
    case "$lid" in
        c) h="C" ;; cpp) h="C++" ;; rust) h="Rust" ;; zig) h="Zig" ;;
        go) h="Go" ;; java) h="Java" ;; js) h="JS" ;;
        python) h="Py" ;; ruby) h="Rb" ;; nim) h="Nim" ;;
        logos_release) h="LOGOS" ;;
    esac
    printf "%10s" "$h"
done
echo
printf '%.0s-' $(seq 1 $((26 + 10 * ${#QUICK_IDS[@]}))); echo

for bench in "${BENCHMARKS[@]}"; do
    printf "%-14s %12s" "$bench" "$(bench_size "$bench")"
    for lid in "${QUICK_IDS[@]}"; do
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
