#!/usr/bin/env bash
# Write a benchmark's expected output at a size: programs/<bench>/expected_<n>.txt.
#
# The bytes come from the reference implementations, never from LOGOS: the C, C++ and Rust ports are built with the
# registry's own build commands (lib/languages.tsv), each runs at n, and the file is written only when all three print
# the same bytes. A disagreement writes nothing and names the ports that differ.
#
# Usage: bash benchmarks/gen-expected.sh <bench> <n> [<n> …]
#        bash benchmarks/gen-expected.sh --missing <size-column>    every benchmark whose expected file at that
#                                                                    lib/benchmarks.tsv column (small_size, …) is absent

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
. "$SCRIPT_DIR/lib/suite.sh"

REFERENCES=(c cpp rust)

die() { echo "gen-expected: $*" >&2; exit 1; }

# generate <bench> <n>: build the references once per call into a scratch directory, run each at n, compare, write.
generate() {
    local bench="$1" n="$2" tmp out id first="" agreed=1
    local expected="$SUITE_PROGRAMS/$bench/expected_${n}.txt"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' RETURN
    for id in "${REFERENCES[@]}"; do
        suite_has "$bench" "$id" || die "$bench has no ${LANG_SOURCE[$id]} reference"
        bash -c "$(suite_fill "${LANG_BUILD[$id]}" "$bench" "$id" "" "$tmp" "$tmp/$id")" \
            || die "$bench: the ${LANG_LABEL[$id]} reference does not build"
        out="$tmp/$id.out"
        timeout 600 bash -c "$(suite_fill "${LANG_RUN[$id]}" "$bench" "$id" "$n" "$tmp" "$tmp/$id")" > "$out" \
            || die "$bench: the ${LANG_LABEL[$id]} reference failed at n=$n"
        if [ -z "$first" ]; then
            first="$out"
        elif ! cmp -s "$first" "$out"; then
            echo "gen-expected: $bench at n=$n: ${LANG_LABEL[$id]} differs from ${LANG_LABEL[${REFERENCES[0]}]}" >&2
            agreed=0
        fi
    done
    [ "$agreed" = 1 ] || die "$bench at n=$n: the references disagree; nothing written"
    cp "$first" "$expected"
    echo "wrote ${expected#"$SCRIPT_DIR/"} ($(wc -c < "$expected") bytes, ${#REFERENCES[@]} references agree)"
}

[ $# -ge 1 ] || die "usage: gen-expected.sh <bench> <n> [<n> …] | --missing <size-column>"

if [ "$1" = --missing ]; then
    column="${2:-}"
    case "$column" in
        small_size) declare -n SIZES=BENCH_SMALL_SIZE ;;
        dev_size) declare -n SIZES=BENCH_DEV_SIZE ;;
        quick_size) declare -n SIZES=BENCH_QUICK_SIZE ;;
        slow_size) declare -n SIZES=BENCH_SLOW_SIZE ;;
        interp_size) declare -n SIZES=BENCH_INTERP_SIZE ;;
        ref_size) declare -n SIZES=BENCH_REF ;;
        *) die "--missing takes a lib/benchmarks.tsv size column: small_size, dev_size, quick_size, slow_size, interp_size, ref_size" ;;
    esac
    for bench in "${BENCHMARKS[@]}"; do
        n="${SIZES[$bench]}"
        [ -f "$SUITE_PROGRAMS/$bench/expected_${n}.txt" ] || generate "$bench" "$n"
    done
else
    bench="$1"; shift
    [ -d "$SUITE_PROGRAMS/$bench" ] || die "no benchmark named $bench"
    [ $# -ge 1 ] || die "name at least one size"
    for n in "$@"; do generate "$bench" "$n"; done
fi
