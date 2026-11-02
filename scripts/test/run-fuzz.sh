#!/usr/bin/env bash
#
# run-fuzz.sh — drive the cargo-fuzz targets. libFuzzer
# (coverage-guided, structure-aware) explores byte space and catches the aborts / stack
# overflows / OOM that the in-suite `catch_unwind` never-panic lock cannot. Every crash it
# finds is minimized under fuzz/regressions/ and read back by never_panic_lock.rs FOREVER
# (Invariant VII) — a fixed crash can never silently regress.
#
# Runs out-of-band (nightly toolchain), NOT in the per-PR suite. Bounded by default so a CI
# nightly lane can call it; pass a big time for a real soak.
#
# Usage: ./scripts/test/run-fuzz.sh [--time SECONDS] [target ...]
#   no targets → run all;  --time 0 → run forever (interactive soak).
set -uo pipefail

MAX_TIME=300
TARGETS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --time) MAX_TIME="$2"; shift ;;
    *) TARGETS+=("$1") ;;
  esac
  shift
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"
if ! cargo fuzz --version >/dev/null 2>&1; then
  echo "ERROR: cargo-fuzz not found. Install with:  cargo install cargo-fuzz" >&2
  exit 1
fi
if ! rustup toolchain list 2>/dev/null | grep -q nightly; then
  echo "ERROR: cargo-fuzz needs a nightly toolchain (rustup toolchain install nightly)." >&2
  exit 1
fi

# The detached fuzz crates and their targets. Keep in sync with the fuzz/ Cargo.tomls.
#   <crate-dir>::<target>
ALL=(
  "crates/logicaffeine_language::fuzz_lex"
  "crates/logicaffeine_language::fuzz_parse"
  "crates/logicaffeine_compile::fuzz_wire_decode"
)

RUN=()
if [ "${#TARGETS[@]}" -eq 0 ]; then
  RUN=("${ALL[@]}")
else
  for want in "${TARGETS[@]}"; do
    for a in "${ALL[@]}"; do [ "${a##*::}" = "$want" ] && RUN+=("$a"); done
  done
fi

FAIL=0
for entry in "${RUN[@]}"; do
  dir="${entry%%::*}"; target="${entry##*::}"
  echo "════════════════════════════════════════════════════════════════════"
  echo " fuzz: $target  (in $dir, ${MAX_TIME}s)"
  echo "════════════════════════════════════════════════════════════════════"
  ( cd "$dir" && cargo +nightly fuzz run "$target" -- -max_total_time="$MAX_TIME" )
  rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "❌ $target found a crash (rc=$rc). The reproducer is under $dir/fuzz/artifacts/$target/."
    echo "   → minimize it (cargo +nightly fuzz tmin $target <artifact>), commit under"
    echo "     $dir/fuzz/regressions/, and add it to never_panic_lock.rs (Invariant VII)."
    FAIL=1
  else
    echo "✓ $target: no crash in ${MAX_TIME}s."
  fi
done

exit "$FAIL"
