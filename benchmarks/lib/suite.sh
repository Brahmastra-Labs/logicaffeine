# The benchmark suite, from one place. Source it (bash 4+); it loads:
#
#   benchmarks.tsv  the programs, in presentation order: category, name, description, declared complexity, the note
#                   for a benchmark the LOGOS optimizer collapses, whether its argument is a linear size, and the size
#                   each kind of run uses — ref_size (the quick-check and BENCH_SIZES=ref size), interp_size (the
#                   interpreter comparison before calibration), quick_size (run-quick.sh), slow_size (the largest size
#                   verify.sh gives Python and Ruby), small_size (the compiled-vs-VM differential) and dev_size (the
#                   optimization loops' timing size), and what its output proves (proof: `fold` when the program
#                   builds a result collection and prints the fold over all of it, `answer` when its result is a
#                   single value or fixed tuple; output: that in words); the timed sizes are programs/<id>/sizes.txt
#   languages.tsv   every timed implementation: label, colour, tier, source file, and the build, run and
#                   version commands as templates over {bench} {src} {bin} {dir} {n} {largo} {tmp}
#   compilers.tsv   the compile-time rows; `@build <id>` reuses that language's build command, and a LOGOS row runs
#                   `largo build` in a project prepared once per benchmark, so its warmup build primes the dependency
#                   cache and the timed builds measure the edit-compile loop, as for every other compiler
#   limits.tsv      the sizes an implementation cannot run, and why (a runtime's stack, not a bug); a run skips
#                   them and records the reason
#
# and defines the helpers every benchmark script shares: suite_fill (a template → a command line), pinned (run
# on the performance cores), quiet_box_gate (refuse to measure a busy machine), and the LOGOS builders.

SUITE_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SUITE_ROOT="$(cd "$SUITE_LIB/.." && pwd)"
. "$SUITE_LIB/largo.sh"

declare -a BENCHMARKS=() LANG_IDS=() COMPILER_IDS=()
declare -A BENCH_CATEGORY=() BENCH_NAME=() BENCH_DESC=() BENCH_TIME=() BENCH_SPACE=() BENCH_REF=() BENCH_COLLAPSE=()
declare -A BENCH_INTERP_SIZE=() BENCH_QUICK_SIZE=() BENCH_SLOW_SIZE=() BENCH_SMALL_SIZE=() BENCH_DEV_SIZE=() BENCH_LINEAR=()
declare -A BENCH_PROOF=() BENCH_OUTPUT=() BENCH_SHORT=()
declare -A LANG_LABEL=() LANG_COLOR=() LANG_TIER=() LANG_SOURCE=() LANG_BUILD=() LANG_RUN=() LANG_VERSION=()
declare -A LANG_MEM=() LANG_SIZE=() LANG_PROFILE=() LANG_SHOWN=() LANG_ENGINE=() COMPILER_LABEL=() COMPILER_COMMAND=()
declare -A LIMIT_MAX=() LIMIT_REASON=()

# A registry row is tab-separated, and a field may be empty. Tab is IFS whitespace — adjacent tabs would collapse and
# shift every later column — so rows are read with the ASCII unit separator, which never collapses.
suite_load() {
    local id category name short desc time space ref collapse interp quick slow small dev linear proof output
    while IFS=$'\037' read -r id category name short desc time space ref collapse interp quick slow small dev linear proof output || [ -n "$id" ]; do
        [ -z "$id" ] && continue
        BENCHMARKS+=("$id")
        BENCH_CATEGORY[$id]="$category"; BENCH_NAME[$id]="$name"; BENCH_SHORT[$id]="$short"; BENCH_DESC[$id]="$desc"
        BENCH_TIME[$id]="$time"; BENCH_SPACE[$id]="$space"; BENCH_REF[$id]="$ref"; BENCH_COLLAPSE[$id]="$collapse"
        BENCH_INTERP_SIZE[$id]="$interp"; BENCH_QUICK_SIZE[$id]="$quick"; BENCH_SLOW_SIZE[$id]="$slow"
        BENCH_SMALL_SIZE[$id]="$small"; BENCH_DEV_SIZE[$id]="$dev"; BENCH_LINEAR[$id]="$linear"
        BENCH_PROOF[$id]="$proof"; BENCH_OUTPUT[$id]="$output"
    done < <(tail -n +2 "$SUITE_LIB/benchmarks.tsv" | tr '\t' '\037')

    local label color tier source build run version mem size profile shown engine
    while IFS=$'\037' read -r id label color tier source build run version mem size profile shown engine || [ -n "$id" ]; do
        [ -z "$id" ] && continue
        LANG_IDS+=("$id")
        LANG_LABEL[$id]="$label"; LANG_COLOR[$id]="$color"; LANG_TIER[$id]="$tier"; LANG_SOURCE[$id]="$source"
        LANG_BUILD[$id]="$build"; LANG_RUN[$id]="$run"; LANG_VERSION[$id]="$version"
        LANG_MEM[$id]="$mem"; LANG_SIZE[$id]="$size"; LANG_PROFILE[$id]="$profile"; LANG_SHOWN[$id]="$shown"
        LANG_ENGINE[$id]="$engine"
    done < <(tail -n +2 "$SUITE_LIB/languages.tsv" | tr '\t' '\037')

    local command
    while IFS=$'\037' read -r id label command || [ -n "$id" ]; do
        [ -z "$id" ] && continue
        COMPILER_IDS+=("$id")
        COMPILER_LABEL[$id]="$label"; COMPILER_COMMAND[$id]="$command"
    done < <(tail -n +2 "$SUITE_LIB/compilers.tsv" | tr '\t' '\037')

    local lang bench max_n reason
    while IFS=$'\037' read -r lang bench max_n reason || [ -n "$lang" ]; do
        [ -z "$lang" ] && continue
        LIMIT_MAX["$bench $lang"]="$max_n"; LIMIT_REASON["$bench $lang"]="$reason"
    done < <(tail -n +2 "$SUITE_LIB/limits.tsv" | tr '\t' '\037')
}

# suite_over_limit <bench> <id> <n>: 0 when limits.tsv says <id> cannot run <bench> at size <n>.
suite_over_limit() {
    local max="${LIMIT_MAX["$1 $2"]:-}"
    [ -n "$max" ] && [ "$3" -gt "$max" ]
}

# The directories every script agrees on. A run's binaries and the generated sources it records live under
# BENCH_WORK_DIR when one is given (a test, or a development run beside a real one), so it never overwrites what
# another run built. Exported, so every tool a run starts (the size measurement, the assembler) reads this run's.
SUITE_WORK="${BENCH_WORK_DIR:-$SUITE_ROOT}"
SUITE_BIN="$SUITE_WORK/bin"
SUITE_PROGRAMS="$SUITE_ROOT/programs"
SUITE_GENERATED="$SUITE_WORK/generated"
export SUITE_WORK SUITE_BIN SUITE_PROGRAMS SUITE_GENERATED

# The artifact of language <id> for benchmark <bench>: a file for a compiled language, a directory for Java's classes
# and the JIT's project.
suite_bin() { printf '%s/%s_%s' "$SUITE_BIN" "$1" "$2"; }
suite_dir() { printf '%s/%s/%s' "$SUITE_BIN" "$2" "$1"; }

# suite_fill <template> <bench> <id> [n] [tmp] [out] — the template with every placeholder replaced; [out], when
# given, stands in for {bin} and {dir} (a compile-time run builds into scratch). Paths in this tree carry no spaces or
# shell metacharacters; suite_check_paths refuses to run where that is not so.
suite_fill() {
    local t="$1" bench="$2" id="$3" n="${4:-}" tmp="${5:-}" out="${6:-}"
    t="${t//\{bench\}/$bench}"
    t="${t//\{src\}/$SUITE_PROGRAMS/$bench/${LANG_SOURCE[$id]:-}}"
    t="${t//\{bin\}/${out:-$(suite_bin "$bench" "$id")}}"
    t="${t//\{dir\}/${out:-$(suite_dir "$bench" "$id")}}"
    t="${t//\{n\}/$n}"
    t="${t//\{largo\}/${LARGO:-largo}}"
    t="${t//\{tmp\}/$tmp}"
    [[ "$t" == *"{goamd64}"* ]] && t="${t//\{goamd64\}/$(suite_goamd64)}"
    printf '%s' "$t"
}

# suite_goamd64: the host's x86-64 microarchitecture level (v1–v4) from the CPU's flags — Go's analogue of the
# -march=native / target-cpu=native every other compiled row takes. A level counts only when every feature it needs is
# present.
suite_goamd64() {
    local flags level=v1 f ok
    flags=" $(grep -m1 '^flags' "$BENCH_PROC_ROOT/cpuinfo" | cut -d: -f2) "
    local -A need=(
        [v2]="cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3"
        [v3]="avx avx2 bmi1 bmi2 f16c fma abm movbe xsave"
        [v4]="avx512f avx512bw avx512cd avx512dq avx512vl"
    )
    for l in v2 v3 v4; do
        ok=1
        for f in ${need[$l]}; do [[ "$flags" == *" $f "* ]] || { ok=0; break; }; done
        [ "$ok" = 1 ] || break
        level=$l
    done
    printf '%s' "$level"
}

suite_check_paths() {
    case "$SUITE_ROOT${LARGO:-}" in
        *[[:space:]\'\"\$\`\;\&\|\<\>\(\)]*)
            echo "benchmark paths must not contain spaces or shell metacharacters: $SUITE_ROOT" >&2
            return 1 ;;
    esac
}

# The command that runs language <id> on benchmark <bench> at size <n>.
suite_run_cmd() { suite_fill "${LANG_RUN[$2]}" "$1" "$2" "$3"; }

# The artifact language <id>'s build of <bench> produces: its directory when the build command writes one ({dir}),
# else its file.
suite_artifact() {
    case "${LANG_BUILD[$2]}" in
        *"{dir}"*) suite_dir "$1" "$2" ;;
        *) suite_bin "$1" "$2" ;;
    esac
}

# The first line language <id>'s version command prints, asked once per process.
declare -A _SUITE_TOOLCHAIN=()
suite_toolchain() {
    if [ -z "${_SUITE_TOOLCHAIN[$1]+set}" ]; then
        _SUITE_TOOLCHAIN[$1]=$( (eval "$(suite_fill "${LANG_VERSION[$1]}" "" "$1")") 2>&1 < /dev/null | sed -n 1p || true)
    fi
    printf '%s' "${_SUITE_TOOLCHAIN[$1]}"
}

# suite_build <bench> <id> — build language <id>'s implementation of <bench> with the command languages.tsv gives it,
# into its suite_artifact. Every script that runs an implementation builds it here, so every number any script reports
# is of the binary the published command line produces. An artifact is reused only when its stamp, <artifact>.build,
# records exactly this command and toolchain, and it is newer than its source (a LOGOS artifact also newer than largo
# and the JIT image tools); FORCE_BUILD=1 rebuilds regardless. A build that fails leaves neither artifact nor stamp.
# The build's own output goes to stdout and stderr. SUITE_BUILD_REUSED says whether the last call reused the artifact.
# Nothing to do for a row without a build step.
SUITE_BUILD_REUSED=0
suite_build() {
    local bench="$1" id="$2" artifact stamp want src tmp rc=0 input
    SUITE_BUILD_REUSED=0
    [ "${LANG_BUILD[$id]}" = "-" ] && return 0
    artifact=$(suite_artifact "$bench" "$id")
    stamp="$artifact.build"
    src="$SUITE_PROGRAMS/$bench/${LANG_SOURCE[$id]}"
    want=$(printf '%s\n%s' "$(suite_fill "${LANG_BUILD[$id]}" "$bench" "$id" "" "{tmp}")" "$(suite_toolchain "$id")")
    if [ "${FORCE_BUILD:-}" != "1" ] && [ -e "$artifact" ] && [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] \
        && [ "$artifact" -nt "$src" ]; then
        SUITE_BUILD_REUSED=1
        if [ "${LANG_TIER[$id]}" = logos ]; then
            for input in "${LARGO:-}" "${JIT_IMAGE_BUILD:-}" "${JIT_IMAGE_RUN:-}"; do
                [ -z "$input" ] || [ "$artifact" -nt "$input" ] || SUITE_BUILD_REUSED=0
            done
        fi
        [ "$SUITE_BUILD_REUSED" = 1 ] && return 0
    fi
    rm -rf "$artifact" "$stamp"
    mkdir -p "$(dirname "$artifact")"
    tmp=$(mktemp -d)
    (eval "$(suite_fill "${LANG_BUILD[$id]}" "$bench" "$id" "" "$tmp")") || rc=$?
    rm -rf "$tmp"
    if [ "$rc" -eq 0 ] && [ -e "$artifact" ]; then
        printf '%s' "$want" > "$stamp"
        return 0
    fi
    [ "$rc" -eq 0 ] && { echo "the ${LANG_LABEL[$id]} build of $bench succeeded but produced no $artifact"; rc=1; }
    rm -rf "$artifact"
    return "$rc"
}

# Whether language <id> has an implementation of <bench>.
suite_has() { [ -f "$SUITE_PROGRAMS/$1/${LANG_SOURCE[$2]}" ]; }

# The sizes <bench> is timed at, ascending: its interpreter-scale size (interp_size, which every engine finishes, the
# tree-walker included) and then every size in sizes.txt; under BENCH_SIZES=ref, the interpreter-scale size, the
# reference size and the smallest size in sizes.txt; under BENCH_SIZES=quick, exactly the registry's quick_size. A row
# that times out at a size is not timed at a larger one.
suite_sizes() {
    local listed sizes
    listed=$(tr -s ' \n' '\n' < "$SUITE_PROGRAMS/$1/sizes.txt" | sed '/^$/d')
    case "${BENCH_SIZES:-all}" in
        quick) sizes="${BENCH_QUICK_SIZE[$1]}" ;;
        ref) sizes=$(printf '%s\n' "${BENCH_INTERP_SIZE[$1]}" "${BENCH_REF[$1]}" "$(printf '%s\n' "$listed" | sort -n | head -1)") ;;
        all) sizes=$(printf '%s\n' "${BENCH_INTERP_SIZE[$1]}" "$listed") ;;
        *) echo "BENCH_SIZES must be all, ref, or quick (got '${BENCH_SIZES}')" >&2; return 2 ;;
    esac
    printf '%s\n' "$sizes" | sed '/^$/d' | sort -n -u
}

# ---------------------------------------------------------------------------------------------------------------------
# LOGOS builders. A LOGOS program is built the way a user builds it: a project with a Largo.toml, `largo build`.
# ---------------------------------------------------------------------------------------------------------------------

_logos_scaffold() {
    mkdir -p "$2/src"
    cp "$SUITE_PROGRAMS/$1/main.lg" "$2/src/main.lg"
    printf '[package]\nname = "bench"\nversion = "0.1.0"\nentry = "src/main.lg"\n' > "$2/Largo.toml"
}

# logos_build <rust|c|cbc|debug> <bench> <out>: `largo build --release` (through Rust), `largo build --release --emit c`
# (C from the program), `--emit c-bytecode` (C from its bytecode) or a debug `largo build`, in a fresh project; the
# artifact is copied to <out> and the generated source to generated/<bench>.rs, .c or .bytecode.c. Built UNSTRIPPED (CARGO_PROFILE_RELEASE_STRIP=false, which both backends honour) so the
# as-built size compares fairly with the other toolchains, which keep their symbol tables; measure-sizes.sh strips a
# copy for the shipped figure. The symbol table lives in non-loaded sections: runtime and RSS are unaffected.
logos_build() {
    local backend="$1" bench="$2" out="$3" proj rc=0 mode=release
    local -a flags=(--release)
    case "$backend" in
        rust) ;;
        c) flags+=(--emit c) ;;
        cbc) flags+=(--emit c-bytecode) ;;
        debug) flags=(); mode=debug ;;
        *) echo "logos_build: unknown backend '$backend'" >&2; return 2 ;;
    esac
    proj=$(mktemp -d)
    _logos_scaffold "$bench" "$proj"
    (cd "$proj" && CARGO_PROFILE_RELEASE_STRIP=false "$LARGO" -q build "${flags[@]}") || rc=$?
    if [ "$rc" -eq 0 ]; then
        if [ -f "$proj/target/$mode/bench" ]; then
            cp "$proj/target/$mode/bench" "$out"
            mkdir -p "$SUITE_GENERATED"
            case "$backend" in
                rust) cp "$proj/target/release/build/src/main.rs" "$SUITE_GENERATED/$bench.rs" ;;
                c) cp "$proj/target/release/build-c/main.c" "$SUITE_GENERATED/$bench.c" ;;
                cbc) cp "$proj/target/release/build-c/main.c" "$SUITE_GENERATED/$bench.bytecode.c" ;;
            esac
        else
            echo "largo build ${flags[*]} succeeded but published no artifact at target/$mode/bench"
            rc=1
        fi
    fi
    rm -rf "$proj"
    return "$rc"
}

# logos_project <bench> <dir>: the project `largo run --interpret` runs from, kept under bin/ for the timing phase.
logos_project() {
    rm -rf "$2"
    _logos_scaffold "$1" "$2"
}

# logos_jit_image <bench> <dir>: compile the eagerly optimized bytecode once,
# then time only image decode + the real runtime copy-and-patch tier. This is
# the JIT analogue of every compiled row's build/run split: optimizer work is
# a build cost, while native machine code is still generated during execution.
logos_jit_image() {
    logos_project "$1" "$2"
    if [ -x "${JIT_IMAGE_BUILD:-}" ] && [ -x "${JIT_IMAGE_RUN:-}" ]; then
        "$JIT_IMAGE_BUILD" "$2/src/main.lg" "$2/program.lgb" || return 1
        printf '#!/bin/sh\nexec "%s" "%s" "$@"\n' \
            "$JIT_IMAGE_RUN" "$2/program.lgb" > "$2/run-jit"
    else
        # The hermetic harness fixture has a stub `largo`, not Rust binaries;
        # keep it exercising engine/answer/timeout gates through the source path.
        printf '#!/bin/sh\ncd "%s"\nexec "%s" -q run --interpret --engine jit -- "$@"\n' \
            "$2" "$LARGO" > "$2/run-jit"
    fi
    chmod +x "$2/run-jit"
}

# A LOGOS tuning variable left in the caller's environment would change what is measured (LOGOS_NO_JIT turns the JIT
# off, LOGOS_TIER_PROFILE picks another optimizer schedule). Every run measures the product default.
suite_clean_logos_env() {
    local var
    for var in $(compgen -e | grep '^LOGOS_' || true); do
        case "$var" in
            LOGOS_WORKSPACE) ;;
            *) unset "$var" ;;
        esac
    done
}

# ---------------------------------------------------------------------------------------------------------------------
# The machine. Timed commands run on the performance cores; a busy machine is refused before it can skew a number.
# ---------------------------------------------------------------------------------------------------------------------

BENCH_SYS_ROOT="${BENCH_SYS_ROOT:-/sys}"
BENCH_PROC_ROOT="${BENCH_PROC_ROOT:-/proc}"

# The CPUs timed commands run on: BENCH_CPUS, else the hybrid part's performance cores, else every online CPU.
suite_cpus() {
    if [ -n "${BENCH_CPUS:-}" ]; then
        printf '%s' "$BENCH_CPUS"
    elif [ -r "$BENCH_SYS_ROOT/devices/cpu_core/cpus" ]; then
        tr -d '\n' < "$BENCH_SYS_ROOT/devices/cpu_core/cpus"
    else
        tr -d '\n' < "$BENCH_SYS_ROOT/devices/system/cpu/online"
    fi
}

suite_cpus_source() {
    if [ -n "${BENCH_CPUS:-}" ]; then echo BENCH_CPUS
    elif [ -r "$BENCH_SYS_ROOT/devices/cpu_core/cpus" ]; then echo performance-cores
    else echo all-online; fi
}

# pinned <command…>: run on the timed CPUs. Everything a timed command starts inherits the mask.
pinned() { taskset -c "$(suite_cpus)" "$@"; }

# Expand a cpulist ("0-15,24") into one CPU number per line.
_suite_cpu_list() {
    local part a b
    for part in ${1//,/ }; do
        if [[ "$part" == *-* ]]; then a="${part%-*}"; b="${part#*-}"; seq "$a" "$b"; else echo "$part"; fi
    done
}

# The scaling governor of every timed CPU, space-separated and deduplicated.
suite_governors() {
    local cpu f
    for cpu in $(_suite_cpu_list "$(suite_cpus)"); do
        f="$BENCH_SYS_ROOT/devices/system/cpu/cpu$cpu/cpufreq/scaling_governor"
        [ -r "$f" ] && cat "$f"
    done | sort -u | tr '\n' ' ' | sed 's/ $//'
}

# CPU ticks (user + system) a process has used, from /proc/<pid>/stat; the fields after the `(comm)` group, so a
# process name holding spaces or parentheses cannot shift them.
_suite_ticks() {
    local line rest
    line=$(cat "$BENCH_PROC_ROOT/$1/stat" 2>/dev/null) || return 1
    rest="${line##*) }"
    local -a f
    read -r -a f <<< "$rest"
    echo $(( ${f[11]:-0} + ${f[12]:-0} ))
}

_suite_comm() {
    local line
    line=$(cat "$BENCH_PROC_ROOT/$1/stat" 2>/dev/null) || return 1
    line="${line#*(}"
    printf '%s' "${line%)*}"
}

# The processes that would steal from a measurement, one "pid comm %cpu" line each: a compiler or build tool at any
# load, or any other process outside this script's own tree that used more than BENCH_MAX_FOREIGN_CPU percent of a
# core (default 10) over a two-second sample.
suite_competitors() {
    local window=2 pid p ticks
    local -A own=() before=()
    local -a frontier=("$$") next=()
    own[$$]=1
    while [ "${#frontier[@]}" -gt 0 ]; do
        next=()
        for p in "${frontier[@]}"; do
            for pid in $(ps -o pid= --ppid "$p" 2>/dev/null); do own[$pid]=1; next+=("$pid"); done
        done
        frontier=("${next[@]}")
    done
    for p in "$BENCH_PROC_ROOT"/[0-9]*; do
        pid="${p##*/}"
        [ -n "${own[$pid]:-}" ] && continue
        ticks=$(_suite_ticks "$pid") && before[$pid]="$ticks"
    done
    sleep "$window"
    local hz limit="${BENCH_MAX_FOREIGN_CPU:-10}" comm now pct
    hz=$(getconf CLK_TCK 2>/dev/null || echo 100)
    for pid in "${!before[@]}"; do
        now=$(_suite_ticks "$pid") || continue
        comm=$(_suite_comm "$pid") || continue
        pct=$(( (now - before[$pid]) * 100 / (hz * window) ))
        case "$comm" in
            cargo|rustc|cargo-nextest|cc1|cc1plus|ld|ld.lld|dx|zig|javac|go|nim)
                echo "$pid $comm $pct" ;;
            *)
                if [ "$pct" -ge "$limit" ]; then echo "$pid $comm $pct"; fi ;;
        esac
    done
}

# quiet_box_gate: 0 when the machine is fit to measure — load below BENCH_MAX_LOAD, no competitor, and every timed
# CPU on the performance governor (BENCH_GOVERNOR_OK names another governor to accept). BENCH_QUIET=off skips the
# check and marks the run noisy. Writes its findings to QUIET_* for the results metadata.
QUIET_STATE="unchecked"
QUIET_COMPETITORS=""
quiet_box_gate() {
    if [ "${BENCH_QUIET:-on}" = "off" ]; then
        QUIET_STATE="overridden"
        return 0
    fi
    local load max="${BENCH_MAX_LOAD:-1.0}" gov ok_gov="${BENCH_GOVERNOR_OK:-performance}" problems=()
    load=$(cut -d' ' -f1 "$BENCH_PROC_ROOT/loadavg")
    awk -v l="$load" -v m="$max" 'BEGIN{exit !(l < m)}' || problems+=("load $load ≥ $max")
    gov=$(suite_governors)
    for g in $gov; do
        [ "$g" = "$ok_gov" ] || [ "$g" = performance ] || problems+=("governor $g on the timed CPUs")
    done
    QUIET_COMPETITORS=$(suite_competitors | sort -u)
    [ -n "$QUIET_COMPETITORS" ] && problems+=("competing processes: $(echo "$QUIET_COMPETITORS" | tr '\n' ';')")
    if [ "${#problems[@]}" -gt 0 ]; then
        QUIET_STATE="busy"
        printf 'the machine is not quiet: %s\n' "${problems[@]}" >&2
        return 1
    fi
    QUIET_STATE="passed"
}

# quiet_box_wait: quiet_box_gate, retried with backoff for up to BENCH_QUIET_WAIT seconds (default 1800) before
# giving up. Returns 1 when the machine never went quiet.
QUIET_WAITED=0
quiet_box_wait() {
    local budget="${BENCH_QUIET_WAIT:-1800}" delay=15
    until quiet_box_gate; do
        if [ "$QUIET_WAITED" -ge "$budget" ]; then
            echo "gave up after ${QUIET_WAITED}s waiting for a quiet machine" >&2
            return 1
        fi
        sleep "$delay"
        QUIET_WAITED=$((QUIET_WAITED + delay))
        if [ "$delay" -lt 120 ]; then delay=$((delay * 2)); fi
    done
}

suite_load

# BENCH_ONLY=fib,nbody narrows a development run to those benchmarks; an unknown name is an error, not a no-op.
if [ -n "${BENCH_ONLY:-}" ]; then
    declare -a _suite_only=()
    for _b in ${BENCH_ONLY//,/ }; do
        [ -n "${BENCH_NAME[$_b]:-}" ] || { echo "BENCH_ONLY names an unknown benchmark: $_b" >&2; return 1 2>/dev/null || exit 1; }
        _suite_only+=("$_b")
    done
    BENCHMARKS=("${_suite_only[@]}")
    unset _suite_only _b
fi
