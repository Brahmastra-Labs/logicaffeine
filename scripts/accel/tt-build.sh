#!/usr/bin/env bash
# Build the Tenstorrent (TT-Metalium) toolchain and compile a LOGOS-emitted bundle against it.
#
# Phase 0 of the Tenstorrent backend campaign. This box has no card, but the SDK BUILDS without one —
# so this script proves the toolchain and compiles our emitted host+kernel. It runs in two modes:
#
#   * If a built tt-metal SDK is reachable ($TT_METAL_HOME, or ~/tt-metal), it emits a LOGOS program to
#     a TT-Metalium bundle and compiles the host against the SDK (cmake).
#   * Otherwise it prints the exact clone+build steps and exits 2 — no silent success.
#
# tt-metal is submodule-heavy and must be cloned with git (do that step yourself):
#     git clone --recurse-submodules https://github.com/tenstorrent/tt-metal ~/tt-metal
# then build it (Ubuntu 24.04 differs from tt-metal's 22.04 baseline — build inside its Docker image):
#     cd ~/tt-metal && ./build_metal.sh          # or the documented Docker build
#     export TT_METAL_HOME=~/tt-metal
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TT_HOME="${TT_METAL_HOME:-$HOME/tt-metal}"
OUT="${1:-$ROOT/target/tt-bundle}"

echo "== LOGOS → Tenstorrent build =="
echo "repo:         $ROOT"
echo "tt-metal:     $TT_HOME"
echo "bundle out:   $OUT"

# 1. Always: emit a LOGOS program to a TT-Metalium bundle (this needs no card and no SDK).
SAMPLE="$ROOT/target/tt-sample.lg"
mkdir -p "$(dirname "$SAMPLE")"
cat > "$SAMPLE" <<'LG'
## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).
## Main
Show fib(10).
LG
echo "-- emitting bundle (fib(10)) --"
cargo run -q -p logicaffeine-cli -- emit tenstorrent "$SAMPLE" -o "$OUT"
echo "bundle files:"; ls -1 "$OUT"

# The C oracle is the ≡VM compute the kernel runs — compile+run it here as a toolchain smoke test.
if command -v gcc >/dev/null 2>&1; then
  echo "-- oracle smoke (gcc, ≡VM) --"
  gcc -O2 -o "$OUT/oracle" "$OUT/logos_oracle.c" -lm
  echo "oracle output: $("$OUT/oracle")   (expected: 55)"
fi

# 2. If a built SDK is present, compile the host against it. Otherwise, print instructions.
if [ -d "$TT_HOME" ] && [ -e "$TT_HOME/tt_metal" ]; then
  echo "-- tt-metal found: building host against the SDK --"
  export TT_METAL_HOME="$TT_HOME"
  ( cd "$OUT" && cmake -B build -S . >/dev/null && cmake --build build )
  echo "host built: $OUT/build/logos_tt_host"
  echo "NOTE: running it needs a Tensix card (Phase 5): $ROOT/scripts/accel/measure-tenstorrent-pareto.sh"
else
  cat <<EOF

tt-metal SDK not found at: $TT_HOME
Clone and build it (the clone needs git; do it yourself), then re-run:

    git clone --recurse-submodules https://github.com/tenstorrent/tt-metal ~/tt-metal
    cd ~/tt-metal && ./build_metal.sh        # or the documented Docker build (recommended on Ubuntu 24.04)
    export TT_METAL_HOME=~/tt-metal
    $ROOT/scripts/accel/tt-build.sh

The bundle above ($OUT) is ready to compile the moment the SDK is present.
EOF
  exit 2
fi
