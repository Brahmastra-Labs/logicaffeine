#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE Cross-Language Benchmark Suite
#
# Builds, verifies and times every program in benchmarks/programs/ in every implementation benchmarks/lib/ registers
# (C, C++, Rust, Zig, Go, Java, JavaScript, Nim, and LOGOS three ways: through Rust, through C, and on the JIT), and
# writes results/latest.json, the data the /benchmarks page renders.
#
#   Phase 0   the machine: refuse to measure a busy box (lib/suite.sh quiet_box_gate)
#   Phase 1   build everything; a failed build stops the run by name
#   Phase 2   verify: every implementation prints expected_<n>.txt byte for byte, at every timed size
#   Phase 3   time: hyperfine on the performance cores, each benchmark after a fresh quiet-box check
#   Phase 3.5 peak memory · 3.6 binary size · 4 compile time
#   Phase 5   assemble results/latest.json (lib/assemble.py)
#   Phase 6   the LOGOS interpreter against Node/V8 · 7 the wire codec · 8 the README charts
#
# Requirements: gcc, g++, rustc, zig, go, java/javac, node, python3, ruby, nim, hyperfine, jq, taskset, GNU time
#
# Usage: bash benchmarks/run.sh
#
# Knobs: BENCH_WARMUP (2) BENCH_RUNS (10) BENCH_TIMEOUT (120 s, one verification run) BENCH_HYPERFINE_TIMEOUT (300 s,
# one timing) BENCH_SIZES=all|ref|quick BENCH_ONLY=fib,nbody SKIP_LANGS=zig,nim BENCH_RESULTS_DIR (write somewhere other
# than results/; such a run is never archived and does not redraw the charts) BENCH_WORK_DIR (where its binaries and
# generated sources go, instead of bin/ and generated/) BENCH_TARGET_DIR (largo's cargo target) BENCH_LOG_DIR (the run
# log's directory, logs/bench by default) RUN_INTERP=1 RUN_CODEC=1 MEASURE_MEM=1
# MEASURE_SIZE=1 MEASURE_COMPILE=1 BENCH_MEMORY_MAX (48G, or off), and the machine gate's BENCH_QUIET=off BENCH_MAX_LOAD
# BENCH_QUIET_WAIT BENCH_CPUS BENCH_GOVERNOR_OK (lib/suite.sh).

set -euo pipefail

# The whole run lives in one memory-capped cgroup: a program that runs away is killed inside it (exit 137) instead of
# pushing a shared machine into the kernel's OOM killer.
BENCH_MEMORY_MAX="${BENCH_MEMORY_MAX:-48G}"
if [ -z "${BENCH_IN_SCOPE:-}" ] && [ "$BENCH_MEMORY_MAX" != off ] && command -v systemd-run >/dev/null 2>&1; then
    exec systemd-run --user --scope --quiet -p MemoryMax="$BENCH_MEMORY_MAX" -p MemorySwapMax=0 \
        env BENCH_IN_SCOPE=1 BENCH_MEMORY_MAX="$BENCH_MEMORY_MAX" bash "$0" "$@"
fi
[ -n "${BENCH_IN_SCOPE:-}" ] || BENCH_MEMORY_MAX=off

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CANONICAL_RESULTS="$SCRIPT_DIR/results"
RESULTS_DIR="$(realpath -m "${BENCH_RESULTS_DIR:-$CANONICAL_RESULTS}")"
cd "$SCRIPT_DIR"

. "$SCRIPT_DIR/lib/suite.sh"

BIN_DIR="$SUITE_BIN"
PROGRAMS_DIR="$SUITE_PROGRAMS"
RAW_DIR="$RESULTS_DIR/raw"
PER_LANG_DIR="$RAW_DIR/per_lang"

WARMUP="${BENCH_WARMUP:-2}"
RUNS="${BENCH_RUNS:-10}"
TIMEOUT="${BENCH_TIMEOUT:-120}"
HYPERFINE_TIMEOUT="${BENCH_HYPERFINE_TIMEOUT:-300}"
SKIP_LANGS="${SKIP_LANGS:-}"
RUN_INTERP="${RUN_INTERP:-1}"
RUN_CODEC="${RUN_CODEC:-1}"

LOG_DIR="${BENCH_LOG_DIR:-$REPO_ROOT/logs/bench}"
mkdir -p "$RESULTS_DIR/history" "$LOG_DIR"
RUN_LOG="$LOG_DIR/run-$(date -u +%Y%m%d-%H%M%S).log"
exec > >(tee -a "$RUN_LOG") 2>&1

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

skip_lang() { [[ ",$SKIP_LANGS," == *",$1,"* ]]; }

# Every LOGOS tier measures the product default; a tuning variable in the caller's environment would not.
suite_clean_logos_env
export LOGOS_WORKSPACE="$REPO_ROOT"
suite_check_paths
info "Run log: $RUN_LOG"

# ===========================================================================
# Phase 0: The machine
# ===========================================================================
info "Phase 0: Checking the machine (timed CPUs $(suite_cpus), $(suite_cpus_source); governor $(suite_governors))..."
LOAD_START=$(cut -d' ' -f1 "$BENCH_PROC_ROOT/loadavg")
quiet_box_wait || { fail "Phase 0: the machine never went quiet — stop the competing work, or run with BENCH_QUIET=off (marked noisy)"; exit 1; }
GATE_AT_START="$QUIET_STATE"
ok "Phase 0 complete: $GATE_AT_START"

# ===========================================================================
# Phase 1: Build everything
# ===========================================================================
info "Phase 1: Building all implementations..."

# bin/ outlives a run, and the development scripts build into it. Every binary Phases 2-4 verify and time is one this
# run built, through suite_build with the command languages.tsv publishes: bin/ starts empty, and a failed build stops
# the run by name. A language this box cannot build is skipped on purpose with SKIP_LANGS, never dropped silently.
rm -rf "$BIN_DIR" "$RAW_DIR"
mkdir -p "$BIN_DIR" "$PER_LANG_DIR" "$RAW_DIR/build" "$SUITE_GENERATED"
BUILD_ERRORS=()

info "Building largo..."
largo_build || exit 1
suite_check_paths
ok "largo built ($LARGO)"

for bench in "${BENCHMARKS[@]}"; do
    info "Building $bench..."
    for id in "${LANG_IDS[@]}"; do
        skip_lang "$id" && continue
        suite_has "$bench" "$id" || continue
        [ "${LANG_BUILD[$id]}" = "-" ] && continue
        log="$RAW_DIR/build/${bench}_${id}.log"
        if suite_build "$bench" "$id" > "$log" 2>&1; then
            ok "  ${LANG_LABEL[$id]}"
        else
            fail "  ${LANG_LABEL[$id]} build failed ($log)"
            BUILD_ERRORS+=("$bench/${LANG_LABEL[$id]}")
        fi
    done
done

if [ "${#BUILD_ERRORS[@]}" -gt 0 ]; then
    fail "Phase 1 failed: ${#BUILD_ERRORS[@]} build(s) failed — fix them, or skip a language on purpose with SKIP_LANGS:"
    printf '  %s\n' "${BUILD_ERRORS[@]}" >&2
    rm -rf "$BIN_DIR"
    exit 1
fi
ok "Phase 1 complete"

# ===========================================================================
# Phase 2: Verify — every implementation, every timed size, byte for byte
# ===========================================================================
info "Phase 2: Verifying every implementation at every timed size..."

# A language that times out at size n is not verified or timed at any larger size of that benchmark.
declare -A LANG_TIMEOUTS=()
timed_out_by() {  # bench id size: the row ran out of time at this size or a smaller one
    local at="${LANG_TIMEOUTS["$1_$2"]:-}"
    [ -n "$at" ] && [ "$3" -ge "$at" ]
}
ERRORS=0

# A row that runs out of time at a size is not run there or at any larger size; LANG_TIMEOUTS keeps the smallest size
# each row ran out of time at.
mark_timeout() {  # bench size id seconds
    if [ -z "${LANG_TIMEOUTS["$1_$3"]:-}" ] || [ "$2" -lt "${LANG_TIMEOUTS["$1_$3"]}" ]; then
        LANG_TIMEOUTS["$1_$3"]="$2"
    fi
    echo "$4" > "$RAW_DIR/$1_$2_$3.timeout"
}

# lib/limits.tsv: a size an implementation cannot run (a runtime's stack). Skipped, and the reason recorded.
over_limit() {  # bench size id
    suite_over_limit "$1" "$3" "$2" || return 1
    echo "${LIMIT_REASON["$1 $3"]}" > "$RAW_DIR/$1_$2_$3.limit"
}

for bench in "${BENCHMARKS[@]}"; do
    for size in $(suite_sizes "$bench" | sort -n); do
        expected="$PROGRAMS_DIR/$bench/expected_${size}.txt"
        if [ ! -f "$expected" ]; then
            fail "  $bench has no expected_${size}.txt — every timed size needs its verified output"
            ERRORS=$((ERRORS + 1))
            continue
        fi
        info "Verifying $bench at n=$size..."
        for id in "${LANG_IDS[@]}"; do
            skip_lang "$id" && continue
            suite_has "$bench" "$id" || continue
            timed_out_by "$bench" "$id" "$size" && continue
            if over_limit "$bench" "$size" "$id"; then
                warn "  ${LANG_LABEL[$id]}: not run at n=$size — ${LIMIT_REASON["$bench $id"]}"
                continue
            fi
            cmd=$(suite_run_cmd "$bench" "$id" "$size")
            got=$(mktemp); err=$(mktemp); rc=0
            # The JIT row keeps the machine code it emits: the last size it verifies at is the one the page shows.
            code_dir=""
            if [ "${LANG_ENGINE[$id]}" = "vm+jit" ]; then
                code_dir="$RAW_DIR/${bench}_jitcode"
                rm -rf "$code_dir"
            fi
            LOGOS_JIT_CODE="$code_dir" LOGOS_ENGINE_TRACE=1 timeout "$TIMEOUT" bash -c "$cmd" > "$got" 2> "$err" || rc=$?
            if [ "$rc" -eq 124 ]; then
                warn "  ${LANG_LABEL[$id]}: timed out after ${TIMEOUT}s — not verified or timed at n>=$size"
                mark_timeout "$bench" "$size" "$id" "$TIMEOUT"
            elif [ "$rc" -eq 137 ]; then
                fail "  ${LANG_LABEL[$id]}: killed (exit 137) — over the run's ${BENCH_MEMORY_MAX} memory cap"
                ERRORS=$((ERRORS + 1))
            elif [ "$rc" -ne 0 ]; then
                fail "  ${LANG_LABEL[$id]}: exited $rc: $(head -c 300 "$err")"
                ERRORS=$((ERRORS + 1))
            elif ! cmp -s "$got" "$expected"; then
                fail "  ${LANG_LABEL[$id]}: printed [$(head -c 120 "$got" | od -An -c | tr -s ' ' | head -c 240)] expected [$(head -c 120 "$expected" | od -An -c | tr -s ' ' | head -c 240)]"
                ERRORS=$((ERRORS + 1))
            else
                engine="${LANG_ENGINE[$id]}"
                if [ -n "$engine" ]; then
                    if ! grep -qx "logos-engine: $engine" "$err" \
                        || { [ "$engine" = "vm+jit" ] && ! grep -q '^logos-engine: native-tier ' "$err"; }; then
                        case "$engine" in vm+jit) named="the JIT" ;; vm) named="the VM" ;; treewalker) named="the tree-walker" ;; *) named="$engine" ;; esac
                        fail "  ${LANG_LABEL[$id]}: did not run on $named: $(grep '^logos-engine:' "$err" | tr '\n' ';')"
                        ERRORS=$((ERRORS + 1))
                    fi
                    if [ "$engine" = "vm+jit" ]; then
                        native_summary="$(grep '^logos-engine: native-tier ' "$err" | head -1 || true)"
                        native_declines="$(grep '^logos-engine: native-declines ' "$err" | head -1 || true)"
                        native_re='^logos-engine: native-tier [^:]+: ([0-9]+)/([0-9]+) functions, ([0-9]+)/([0-9]+) loops compiled$'
                        if [[ ! "$native_summary" =~ $native_re ]]; then
                            fail "  ${LANG_LABEL[$id]}: malformed native coverage trace: $native_summary"
                            ERRORS=$((ERRORS + 1))
                        else
                            native_failed=$((BASH_REMATCH[2] - BASH_REMATCH[1] + BASH_REMATCH[4] - BASH_REMATCH[3]))
                            if [ "$native_failed" -gt 0 ] && [ -z "$native_declines" ]; then
                                fail "  ${LANG_LABEL[$id]}: $native_failed native compile(s) declined without a reason"
                                ERRORS=$((ERRORS + 1))
                            elif [[ "$native_declines" == *unclassified=* ]]; then
                                fail "  ${LANG_LABEL[$id]}: unclassified native fallback: $native_declines"
                                ERRORS=$((ERRORS + 1))
                            fi
                        fi
                        grep '^logos-engine:' "$err" > "$RAW_DIR/${bench}_jit.txt" || true
                    fi
                fi
                ok "  ${LANG_LABEL[$id]}"
            fi
            rm -f "$got" "$err"
        done
    done
done

if [ "$ERRORS" -gt 0 ]; then
    fail "Phase 2 failed: $ERRORS verification errors"
    exit 1
fi
ok "Phase 2 complete: every implementation printed its expected output"

# ===========================================================================
# Phase 3: Time — hyperfine, pinned, each benchmark after a fresh quiet-box check
# ===========================================================================
info "Phase 3: Timing..."

QUIET_WAITS=0
for bench in "${BENCHMARKS[@]}"; do
    QUIET_WAITED=0
    quiet_box_wait || { fail "the machine went busy and stayed busy before $bench — rerun on a quiet machine"; exit 1; }
    QUIET_WAITS=$((QUIET_WAITS + QUIET_WAITED))
    for size in $(suite_sizes "$bench" | sort -n); do
        info "Timing $bench at n=$size..."
        for id in "${LANG_IDS[@]}"; do
            skip_lang "$id" && continue
            suite_has "$bench" "$id" || continue
            timed_out_by "$bench" "$id" "$size" && continue
            [ -e "$RAW_DIR/${bench}_${size}_${id}.limit" ] && continue
            cmd=$(suite_run_cmd "$bench" "$id" "$size")
            rc=0
            timeout "$HYPERFINE_TIMEOUT" taskset -c "$(suite_cpus)" hyperfine \
                --warmup "$WARMUP" --runs "$RUNS" \
                --export-json "$PER_LANG_DIR/${bench}_${size}_${id}.json" --time-unit millisecond \
                -n "$id" "$cmd" > /dev/null || rc=$?
            if [ "$rc" -eq 124 ] || [ "$rc" -eq 143 ]; then
                warn "  ${LANG_LABEL[$id]} timed out for $bench at $size — skipping larger sizes"
                rm -f "$PER_LANG_DIR/${bench}_${size}_${id}.json"
                mark_timeout "$bench" "$size" "$id" "$HYPERFINE_TIMEOUT"
            elif [ "$rc" -ne 0 ]; then
                fail "  ${LANG_LABEL[$id]} failed under hyperfine for $bench at $size (exit $rc)"
                exit 1
            fi
        done
    done
done
ok "Phase 3 complete"

# A run template as `setup && program`: the setup to eval and the program to exec directly, so a measurement sees
# the program itself and never a wrapping shell.
split_run() {
    if [[ "$1" == *" && "* ]]; then SETUP="${1% && *}"; PROGRAM="${1##* && }"; else SETUP=""; PROGRAM="$1"; fi
}

# ===========================================================================
# Phase 3.5: Peak memory (RSS) — once per (benchmark, language, size)
# ===========================================================================
if [ "${MEASURE_MEM:-1}" = "1" ] && [ -x /usr/bin/time ]; then
    info "Phase 3.5: Measuring peak memory (GNU time -v)..."
    for bench in "${BENCHMARKS[@]}"; do
        by_size="{}"
        for size in $(suite_sizes "$bench"); do
            langs="{}"
            for id in "${LANG_IDS[@]}"; do
                skip_lang "$id" && continue
                [ "${LANG_MEM[$id]}" = 1 ] || continue
                [ -f "$PER_LANG_DIR/${bench}_${size}_${id}.json" ] || continue
                split_run "$(suite_run_cmd "$bench" "$id" "$size")"
                errf=$(mktemp); kb=null
                if (eval "${SETUP:-true}" && eval "exec taskset -c $(suite_cpus) /usr/bin/time -v $PROGRAM") > /dev/null 2> "$errf"; then
                    kb=$(grep -oP 'Maximum resident set size \(kbytes\): \K[0-9]+' "$errf" || echo null)
                fi
                rm -f "$errf"
                langs=$(jq -c --arg l "$id" --argjson kb "${kb:-null}" '.[$l]=$kb' <<< "$langs")
            done
            by_size=$(jq -c --arg s "$size" --argjson l "$langs" '.[$s]=$l' <<< "$by_size")
        done
        echo "{\"method\":\"gnu_time_v_maxrss_kb\",\"by_size\":$by_size}" > "$RAW_DIR/${bench}_mem.json"
    done
    ok "Phase 3.5 complete"
else
    info "Phase 3.5: memory measurement skipped (MEASURE_MEM=0 or /usr/bin/time absent)"
fi

# ===========================================================================
# Phase 3.6: Binary size
# ===========================================================================
if [ "${MEASURE_SIZE:-1}" = "1" ]; then
    info "Phase 3.6: Measuring binary sizes..."
    RAW_DIR="$RAW_DIR" bash "$SCRIPT_DIR/measure-sizes.sh" --bins-only "${BENCHMARKS[@]}" \
        || { fail "Phase 3.6: binary-size measurement failed"; exit 1; }
    ok "Phase 3.6 complete"
else
    info "Phase 3.6: binary-size measurement skipped (MEASURE_SIZE=0)"
fi

# ===========================================================================
# Phase 4: Compile time
# ===========================================================================
if [ "${MEASURE_COMPILE:-1}" = "1" ]; then
    info "Phase 4: Timing compilation..."
    export LARGO
    export -f logos_build _logos_scaffold
    for bench in "${BENCHMARKS[@]}"; do
        info "Compile time: $bench"
        for cid in "${COMPILER_IDS[@]}"; do
            template="${COMPILER_COMMAND[$cid]}"
            lang="$cid"
            if [[ "$template" == @build\ * ]]; then
                lang="${template#@build }"
                template="${LANG_BUILD[$lang]}"
            fi
            [ -n "${LANG_SOURCE[$lang]:-}" ] || lang=logos_release
            skip_lang "$lang" && continue
            suite_has "$bench" "$lang" || continue
            scratch=$(mktemp -d)
            out="$scratch/out"
            # A LOGOS compile runs in one project, prepared here and untimed: hyperfine's warmup build primes its
            # dependency cache, so the timed builds measure the edit-compile loop — as rustc, gcc and the rest do.
            if [ "${LANG_TIER[$lang]}" = logos ]; then
                logos_project "$bench" "$scratch/proj"
                out="$scratch/proj"
            fi
            cmd=$(suite_fill "$template" "$bench" "$lang" "" "$scratch" "$out")
            rc=0
            timeout "$HYPERFINE_TIMEOUT" taskset -c "$(suite_cpus)" hyperfine --shell=bash \
                --warmup 1 --runs 5 \
                --export-json "$PER_LANG_DIR/compile_${bench}_${cid}.json" --time-unit millisecond \
                -n "$cid" "$cmd" > /dev/null || rc=$?
            if [ "$rc" -eq 124 ] || [ "$rc" -eq 143 ]; then
                warn "  ${COMPILER_LABEL[$cid]} compile timed out"
                rm -f "$PER_LANG_DIR/compile_${bench}_${cid}.json"
                echo "$HYPERFINE_TIMEOUT" > "$RAW_DIR/compile_${bench}_${cid}.timeout"
            elif [ "$rc" -ne 0 ]; then
                fail "  ${COMPILER_LABEL[$cid]} compile failed under hyperfine (exit $rc)"
                exit 1
            fi
            rm -rf "$scratch"
        done
    done
    ok "Phase 4 complete"
else
    info "Phase 4: compile timing skipped (MEASURE_COMPILE=0)"
fi

# ===========================================================================
# Phase 5: Assemble results/latest.json
# ===========================================================================
info "Phase 5: Assembling results JSON..."

detect_cpu() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m
    else
        grep -m1 'model name' /proc/cpuinfo 2>/dev/null | sed 's/.*: //' || uname -m
    fi
}

detect_os() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        echo "$(sw_vers -productName 2>/dev/null || echo macOS) $(sw_vers -productVersion 2>/dev/null) $(uname -m)"
    elif [ -f /etc/os-release ]; then
        local pretty
        pretty=$(grep -m1 PRETTY_NAME /etc/os-release | sed 's/PRETTY_NAME="\?\([^"]*\)"\?/\1/')
        echo "${pretty:-Linux} $(uname -m)"
    else
        echo "$(uname -s) $(uname -r) $(uname -m)"
    fi
}

DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
COMMIT=$(cd "$REPO_ROOT" && git rev-parse --short HEAD 2>/dev/null || echo "unknown")
CPU=$(detect_cpu)
[ "${GITHUB_ACTIONS:-}" = "true" ] && CPU="$CPU (GitHub Actions)"
OS=$(detect_os)
LOGOS_VER="${LOGOS_VERSION:-$(grep '^version' "$REPO_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')}"

# results/history holds one record per RELEASE, so a run is archived only when this tree is v$LOGOS_VER: the
# CHANGELOG's newest released section is $LOGOS_VER and its [Unreleased] section is empty. Anything else is
# unreleased code, whose numbers filed under v$LOGOS_VER would describe a release that never ran them. A run
# written somewhere other than results/ (BENCH_RESULTS_DIR) is a development run and is never archived.
CHANGELOG_TOP=$(sed -n 's/^## \[\([0-9][^]]*\)\] - .*/\1/p' "$REPO_ROOT/CHANGELOG.md" | head -1)
UNRELEASED_LINES=$(awk '/^## \[Unreleased\]/{f=1;next} /^## \[/{if(f)exit} f' "$REPO_ROOT/CHANGELOG.md" | grep -c . || true)
release_archive() {
    local src="$1" name="$2"
    if [ "$CHANGELOG_TOP" != "$LOGOS_VER" ] || [ "$UNRELEASED_LINES" -ne 0 ]; then
        warn "not archived as results/history/$name: this tree is not v$LOGOS_VER (CHANGELOG's newest release is ${CHANGELOG_TOP:-none}; [Unreleased] holds $UNRELEASED_LINES lines)"
        return 0
    fi
    if [ "$RESULTS_DIR" != "$CANONICAL_RESULTS" ]; then
        warn "not archived as results/history/$name: a BENCH_RESULTS_DIR run is a development run"
        return 0
    fi
    cp "$src" "$RESULTS_DIR/history/$name"
}

VERSIONS="{}"
for id in "${LANG_IDS[@]}"; do
    skip_lang "$id" && continue
    version=$( (eval "$(suite_fill "${LANG_VERSION[$id]}" fib "$id")") 2>&1 < /dev/null | sed -n 1p || true)
    VERSIONS=$(jq -c --arg k "$id" --arg v "${version:-unknown}" '.[$k]=$v' <<< "$VERSIONS")
done
VERSIONS=$(jq -c --arg v "$(hyperfine --version)" '.hyperfine=$v' <<< "$VERSIONS")

BUILD_COMMANDS="{}"
for id in "${LANG_IDS[@]}"; do
    BUILD_COMMANDS=$(jq -c --arg k "$id" --arg v "$(suite_fill "${LANG_SHOWN[$id]}" fib "$id")" '.[$k]=$v' <<< "$BUILD_COMMANDS")
done

jq -n \
    --arg date "$DATE" --arg commit "$COMMIT" --arg cpu "$CPU" --arg os "$OS" --arg kernel "$(uname -r)" \
    --arg logos_version "$LOGOS_VER" \
    --argjson warmup "$WARMUP" --argjson runs "$RUNS" \
    --argjson versions "$VERSIONS" --argjson build_commands "$BUILD_COMMANDS" \
    --arg cpus "$(suite_cpus)" --arg cpus_source "$(suite_cpus_source)" --arg governor "$(suite_governors)" \
    --arg gate "$GATE_AT_START" --argjson waits "$QUIET_WAITS" --arg memory_cap "$BENCH_MEMORY_MAX" \
    --arg load_start "$LOAD_START" --arg load_end "$(cut -d' ' -f1 "$BENCH_PROC_ROOT/loadavg")" \
    --arg no_turbo "$(cat "$BENCH_SYS_ROOT/devices/system/cpu/intel_pstate/no_turbo" 2>/dev/null || echo unknown)" \
    '{metadata: {
        date: $date, commit: $commit, logos_version: $logos_version, cpu: $cpu, os: $os, kernel: $kernel,
        warmup: $warmup, runs: $runs, versions: $versions, build_commands: $build_commands,
        environment: {
            pinning: {cpus: $cpus, source: $cpus_source},
            governor: $governor, no_turbo: $no_turbo,
            quiet_box: {gate: $gate, waited_seconds: $waits},
            memory_cap: $memory_cap,
            load: {start: ($load_start|tonumber), end: ($load_end|tonumber)}
        }
    }}' > "$RAW_DIR/run.json"

# What the optimizer did to each program: the page lists the passes that fired and, from the registry, the ones that
# changed the algorithm. A trace that fails stops the run — an empty list would read as "nothing fired".
for bench in "${BENCHMARKS[@]}"; do
    # The JIT trace must execute enough of the program to cross its hotness
    # thresholds.  Use the registry's quick size: it is correctness-locked,
    # finishes on every engine, and makes this trace independent of whether the
    # timing run selected quick, reference, or all sizes.
    if ! "$LARGO" opts "$PROGRAMS_DIR/$bench/main.lg" --json --jit -- \
        "${BENCH_QUICK_SIZE[$bench]}" > "$RAW_DIR/${bench}_opts.json" 2> "$RAW_DIR/${bench}_opts.err"; then
        fail "Phase 5: largo opts failed on $bench ($RAW_DIR/${bench}_opts.err)"
        exit 1
    fi
done

python3 "$SCRIPT_DIR/lib/assemble.py" "$RAW_DIR" "$RESULTS_DIR/latest.json"
ok "Phase 5 complete: results written to $RESULTS_DIR/latest.json"
jq -r '.summary.geometric_mean_speedup_vs_c | to_entries[] | "  \(.key): \(.value)x the speed of C"' "$RESULTS_DIR/latest.json"

release_archive "$RESULTS_DIR/latest.json" "v${LOGOS_VER}.json"

# ===========================================================================
# Phase 6: LOGOS interpreter vs Node/V8 (peer-to-peer, calibrated sizes)
# ===========================================================================
# The interpreter (bytecode VM + JIT) gets its own comparison vs Node's V8. We calibrate so the FASTER of the two
# engines reaches INTERP_CALIBRATION_TARGET ms (calibrate-interp.sh times both), which keeps BOTH off V8's ~30ms
# startup floor. run-interp-vs-js.sh then also measures the cold-start floor. The output
# (results/latest-interp.json) feeds the interpreter section of the page.
INTERP_CALIBRATION_TARGET="${INTERP_CALIBRATION_TARGET:-250}"
# Each script runs through bash, so its file mode decides nothing: a phase its knob enables runs, or stops the run.
if [ "$RUN_INTERP" = "1" ]; then
    info "Phase 6: LOGOS interpreter vs Node/V8 (calibrating to ${INTERP_CALIBRATION_TARGET}ms, off V8's floor)..."
    # OVERSHOOT must accommodate the SLOWER engine at the calibration size: since we crawl on the faster engine,
    # the slower one can be several times the target (up to ~12x on V8-fast string kernels).
    TARGETS="$INTERP_CALIBRATION_TARGET" OVERSHOOT="${INTERP_CAL_OVERSHOOT:-24}" \
        bash "$SCRIPT_DIR/calibrate-interp.sh" || { fail "Phase 6: interpreter calibration failed"; exit 1; }
    OUT="$RESULTS_DIR/latest-interp.json" CALIBRATION_TARGET="$INTERP_CALIBRATION_TARGET" LOGOS_VERSION="$LOGOS_VER" \
        bash "$SCRIPT_DIR/run-interp-vs-js.sh" || { fail "Phase 6 (interpreter vs Node) failed"; exit 1; }
    release_archive "$RESULTS_DIR/latest-interp.json" "v${LOGOS_VER}-interp.json"
    ok "Phase 6 complete"
else
    info "Phase 6 skipped (RUN_INTERP=$RUN_INTERP)"
fi

# ===========================================================================
# Phase 7: Wire codec vs industry serializers (size + enc/dec + random access)
# ===========================================================================
# The LOGOS wire codec head-to-head vs bincode/postcard/MessagePack/CBOR/JSON, plus Apache Arrow and — when protoc +
# capnp are on PATH — Protobuf and Cap'n Proto. It feeds the Transport Codec tab, with Phase 5's machine identity.
if [ "$RUN_CODEC" = "1" ]; then
    if command -v protoc >/dev/null 2>&1 && command -v capnp >/dev/null 2>&1; then
        CODEC_FEATURES="--features heavy"
        info "Phase 7: Wire codec head-to-head (protoc + capnp present -> heavy)..."
    else
        CODEC_FEATURES="--features arrow-bench"
        warn "Phase 7: protoc/capnp absent -> core + Arrow only (install them for the full chart)"
    fi
    WIREBENCH_JSON="$RESULTS_DIR/latest-codec.json" \
    WIREBENCH_ITERS="${WIREBENCH_ITERS:-20000}" \
    WIREBENCH_DATE="$DATE" WIREBENCH_COMMIT="$COMMIT" \
    WIREBENCH_CPU="$CPU" WIREBENCH_OS="$OS" LOGOS_VERSION="$LOGOS_VER" \
        pinned cargo run --release -p logicaffeine-wirebench $CODEC_FEATURES \
        || { fail "Phase 7 (wire codec) failed"; exit 1; }
    release_archive "$RESULTS_DIR/latest-codec.json" "v${LOGOS_VER}-codec.json"
    ok "Phase 7 complete"
else
    info "Phase 7 skipped (RUN_CODEC=$RUN_CODEC)"
fi

# ===========================================================================
# Phase 8: Regenerate README benchmark charts (SVG)
# ===========================================================================
if [ "$RESULTS_DIR" = "$CANONICAL_RESULTS" ]; then
    info "Phase 8: Regenerating README charts (SVG)..."
    python3 "$SCRIPT_DIR/gen-readme-charts.py" || { fail "Phase 8 (README charts) failed"; exit 1; }
    ok "Phase 8 complete: results/charts/*.svg"
else
    info "Phase 8 skipped: charts are drawn from results/, not a development run's $RESULTS_DIR"
fi

info "Benchmark suite complete! Log: $RUN_LOG"
