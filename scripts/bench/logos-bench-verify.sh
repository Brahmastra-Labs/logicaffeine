#!/usr/bin/env bash
# Correctness sweep: for each bench package named on argv (or ALL under
# benchmarks/parity/bench/ when given "--all"), build its main.lg with largo and
# confirm it prints meta.json's `expected`. This is the CORRECTNESS half of
# bench-parity.sh (no timing), so it runs fine on a busy box — pass/fail is not
# wall-clock sensitive. Serial by necessity: all builds share one CARGO_TARGET_DIR.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BENCHROOT="$ROOT/benchmarks/parity/bench"
LARGO="$ROOT/target/release/largo"
export LOGOS_WORKSPACE="$ROOT"
export CARGO_TARGET_DIR="$ROOT/benchmarks/.logos-bench-target"
[ -x "$LARGO" ] || { echo "largo not built at $LARGO" >&2; exit 1; }

if [ "${1:-}" = "--all" ]; then
    mapfile -t PKGS < <(cd "$BENCHROOT" && for d in */; do [ -f "$d/meta.json" ] && echo "${d%/}"; done)
else
    PKGS=("$@")
fi

pass=0; fail=0; failed=()
for pkg in "${PKGS[@]}"; do
    meta="$BENCHROOT/$pkg/meta.json"
    [ -f "$meta" ] || { echo "  [SKIP] $pkg (no meta.json)"; continue; }
    exp="$(python3 -c "import json;print(json.load(open('$meta'))['expected'])")"
    arg="$(python3 -c "import json;print(json.load(open('$meta'))['arg'])")"
    T="$(mktemp -d)"; mkdir -p "$T/src"
    cp "$BENCHROOT/$pkg/main.lg" "$T/src/main.lg"
    printf '[package]\nname = "bench"\nversion = "0.1.0"\nentry = "src/main.lg"\n' > "$T/Largo.toml"
    rm -f "$CARGO_TARGET_DIR/release/bench"
    if ( cd "$T" && "$LARGO" build --release >/dev/null 2>&1 ) && [ -x "$CARGO_TARGET_DIR/release/bench" ]; then
        got="$("$CARGO_TARGET_DIR/release/bench" "$arg" 2>/dev/null | tr -d '[:space:]')"
        if [ "$got" = "$exp" ]; then echo "  [ OK ] $pkg  =$got"; pass=$((pass+1))
        else echo "  [FAIL] $pkg  LOGOS=$got expected=$exp"; fail=$((fail+1)); failed+=("$pkg"); fi
    else
        echo "  [BUILD-FAIL] $pkg"; fail=$((fail+1)); failed+=("$pkg")
    fi
    rm -rf "$T"
done
echo
echo "LOGOS correctness: $pass ok, $fail failed"
[ "$fail" -gt 0 ] && { printf '  failed: %s\n' "${failed[*]}"; exit 1; }
exit 0
