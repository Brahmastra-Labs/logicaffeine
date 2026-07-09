#!/usr/bin/env bash
# Requires bash 4+ for associative arrays (macOS: brew install bash)
# LOGICAFFEINE Binary & Interpreter Size Measurement
#
# The footprint counterpart to the peak-RSS pass in run.sh (Phase 3.5). Stats the
# compiled artifact of every benchmark in every language (as-built and after symbol
# stripping a throwaway copy), and the engine binaries (largo vs node, plus the
# browser WASM bundle). Emits results/raw/{bench}_sizes.json + interpreter_sizes.json,
# the same shape run.sh folds in for timing/memory.
#
# Single source of truth for sizes: run.sh and run-interp-vs-js.sh call it to write the
# raw files; --merge backfills the published latest.json / latest-interp.json (+ the
# matching history snapshot) in place so the page lights up without a full re-run.
#
# Usage:
#   bash benchmarks/measure-sizes.sh [--merge] [--bins-only|--engines-only] [bench...]
#     --merge         after measuring, patch latest.json/latest-interp.json + history
#     --bins-only     measure only per-program binaries (run.sh Phase 3.6)
#     --engines-only  measure only the engine block (run-interp-vs-js.sh)
#     [bench...]      explicit benchmark ids; default = ids from latest.json (or programs/)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BIN_DIR="$SCRIPT_DIR/bin"
PROGRAMS_DIR="$SCRIPT_DIR/programs"
RESULTS_DIR="$SCRIPT_DIR/results"
RAW_DIR="$RESULTS_DIR/raw"
LATEST="$RESULTS_DIR/latest.json"
INTERP="$RESULTS_DIR/latest-interp.json"
WEB_BUILD_DIR="$ROOT/target/dx/logicaffeine-web/release/web/public"

mkdir -p "$RAW_DIR"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

# language id -> binary filename suffix, mirroring Phase 3.5 in run.sh. Java is a class
# directory (bin/java/{bench}) and is handled separately below.
declare -A LANG_SUFFIX=(
    [c]=_c [cpp]=_cpp [rust]=_rs [zig]=_zig [go]=_go [nim]=_nim [logos_release]=_logos_release
)
LANG_ORDER=(c cpp rust zig go nim logos_release)

# Bytes of a single file (portable: GNU vs BSD stat).
size_bytes() {
    if [ "$(uname -s)" = "Darwin" ]; then stat -f %z "$1"; else stat -c %s "$1"; fi
}

# Total bytes of every regular file under a directory (java's class output).
dir_bytes() {
    local total=0 f
    while IFS= read -r -d '' f; do
        total=$((total + $(size_bytes "$f")))
    done < <(find "$1" -type f -print0 2>/dev/null)
    printf '%d' "$total"
}

# Code-only size: strip --strip-all a throwaway copy (never touch the real artifact),
# stat it, discard. Prints `null` when strip is unavailable or fails.
stripped_bytes() {
    command -v strip >/dev/null 2>&1 || { printf 'null'; return; }
    local tmp out
    tmp="$(mktemp)" || { printf 'null'; return; }
    if cp "$1" "$tmp" 2>/dev/null && strip --strip-all "$tmp" 2>/dev/null; then
        out="$(size_bytes "$tmp")"
        rm -f "$tmp"
        printf '%s' "${out:-null}"
    else
        rm -f "$tmp"
        printf 'null'
    fi
}

# {"as_built":N,"stripped":M|null} for one artifact (file or class dir). Empty string
# when the artifact is absent, so callers skip that language.
artifact_sizes_json() {
    local path="$1" as_built stripped
    if [ -f "$path" ]; then
        as_built="$(size_bytes "$path")"
        stripped="$(stripped_bytes "$path")"
    elif [ -d "$path" ]; then
        as_built="$(dir_bytes "$path")"
        stripped="null"
    else
        return 0
    fi
    [ -n "$as_built" ] || return 0
    printf '{"as_built":%s,"stripped":%s}' "$as_built" "$stripped"
}

# {"method":...,"by_language":{lang:{as_built,stripped},...}} for one benchmark.
measure_bench_sizes() {
    local bench="$1" by_lang="{}" lang js
    for lang in "${LANG_ORDER[@]}"; do
        js="$(artifact_sizes_json "$BIN_DIR/${bench}${LANG_SUFFIX[$lang]}")"
        [ -n "$js" ] && by_lang="$(echo "$by_lang" | jq --arg l "$lang" --argjson v "$js" '.[$l]=$v')"
    done
    js="$(artifact_sizes_json "$BIN_DIR/java/$bench")"
    [ -n "$js" ] && by_lang="$(echo "$by_lang" | jq --arg l java --argjson v "$js" '.[$l]=$v')"
    printf '{"method":"file_size_bytes","by_language":%s}' "$by_lang"
}

# {"method":...,"engines":{id:{as_built,stripped},...},"wasm_bundle_bytes":N|null}.
# largo and node are the headline; deno/bun fold in when self-contained runtimes exist.
measure_engines() {
    local engines="{}" js bin
    add_engine() {
        js="$(artifact_sizes_json "$2")"
        [ -n "$js" ] && engines="$(echo "$engines" | jq --arg l "$1" --argjson v "$js" '.[$l]=$v')"
    }
    add_engine logos "$ROOT/target/release/largo"
    bin="$(command -v node || true)"; [ -n "$bin" ] && add_engine node "$bin"
    bin="$(command -v deno || true)"; [ -n "$bin" ] && add_engine deno "$bin"
    bin="$(command -v bun  || true)"; [ -n "$bin" ] && add_engine bun  "$bin"

    local wasm_bytes="null" w
    if [ -d "$WEB_BUILD_DIR" ]; then
        w="$(find "$WEB_BUILD_DIR" -name '*.wasm' -printf '%s\n' 2>/dev/null | sort -rn | head -1 || true)"
        [ -n "$w" ] && wasm_bytes="$w"
    fi
    printf '{"method":"file_size_bytes","engines":%s,"wasm_bundle_bytes":%s}' "$engines" "$wasm_bytes"
}

# in-place jq patch via a temp file (atomic, set -e safe).
patch_json() {  # file  jq-filter  jq-args...
    local file="$1"; shift
    local filter="$1"; shift
    [ -f "$file" ] || return 0
    if jq "$@" "$filter" "$file" > "$file.tmp"; then
        mv "$file.tmp" "$file"
    else
        rm -f "$file.tmp"
        fail "failed to patch $file"
        return 1
    fi
}

DO_BINS=1; DO_ENGINES=1; DO_MERGE=0; BENCH_ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --merge)        DO_MERGE=1 ;;
        --bins-only)    DO_ENGINES=0 ;;
        --engines-only) DO_BINS=0 ;;
        -h|--help)      grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --*)            fail "unknown flag: $1"; exit 2 ;;
        *)              BENCH_ARGS+=("$1") ;;
    esac
    shift
done

# Benchmark set: explicit args win; else the ids already in latest.json (so the page's
# benchmark set is matched exactly); else the programs/ directory.
if [ ${#BENCH_ARGS[@]} -gt 0 ]; then
    BENCHES=("${BENCH_ARGS[@]}")
elif [ -f "$LATEST" ]; then
    mapfile -t BENCHES < <(jq -r '.benchmarks[].id' "$LATEST")
else
    mapfile -t BENCHES < <(find "$PROGRAMS_DIR" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort)
fi

SIZE_MAP="{}"
ENGINE_JSON="null"

if [ "$DO_BINS" = 1 ]; then
    info "Measuring per-program binary sizes (${#BENCHES[@]} benchmarks)..."
    for bench in "${BENCHES[@]}"; do
        bj="$(measure_bench_sizes "$bench")"
        echo "$bj" > "$RAW_DIR/${bench}_sizes.json"
        SIZE_MAP="$(echo "$SIZE_MAP" | jq --arg id "$bench" --argjson v "$bj" '.[$id]=$v')"
        n_langs="$(echo "$bj" | jq '.by_language | length')"
        [ "$n_langs" -gt 0 ] || warn "$bench: no binaries found in $BIN_DIR"
    done
    ok "Per-program sizes written to $RAW_DIR/*_sizes.json"
fi

if [ "$DO_ENGINES" = 1 ]; then
    info "Measuring engine sizes (largo / node / wasm bundle)..."
    ENGINE_JSON="$(measure_engines)"
    echo "$ENGINE_JSON" > "$RAW_DIR/interpreter_sizes.json"
    echo "$ENGINE_JSON" | jq -r '
        "  engines: " + (.engines | to_entries | map(.key + "=" + (.value.as_built|tostring) + "B") | join(", ")),
        "  wasm_bundle_bytes: " + (.wasm_bundle_bytes|tostring)'
    ok "Engine sizes written to $RAW_DIR/interpreter_sizes.json"
fi

if [ "$DO_MERGE" = 1 ]; then
    VER="$(jq -r '.metadata.logos_version // empty' "$LATEST" 2>/dev/null || true)"
    info "Backfilling published results (version ${VER:-?})..."
    if [ "$DO_BINS" = 1 ]; then
        BIN_FILTER='.benchmarks |= map(if $m[.id] then .binary_sizes = $m[.id] else . end)'
        patch_json "$LATEST" "$BIN_FILTER" --argjson m "$SIZE_MAP"
        [ -n "$VER" ] && patch_json "$RESULTS_DIR/history/v${VER}.json" "$BIN_FILTER" --argjson m "$SIZE_MAP"
    fi
    if [ "$DO_ENGINES" = 1 ]; then
        patch_json "$INTERP" '.interpreter_sizes = $is' --argjson is "$ENGINE_JSON"
        [ -n "$VER" ] && patch_json "$RESULTS_DIR/history/v${VER}-interp.json" '.interpreter_sizes = $is' --argjson is "$ENGINE_JSON"
    fi
    ok "Backfill complete."
fi
