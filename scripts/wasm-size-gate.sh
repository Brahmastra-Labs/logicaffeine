#!/usr/bin/env bash
# WASM bundle size gate (FINISH_INTERPRETER.md Phase 9).
#
# The default Studio build must stay within a payload budget — net-libp2p-wasm and other
# heavy transports are feature-gated OUT of the default bundle, and this gate locks that in:
# if a dependency creeps into the default build and inflates the `.wasm`, CI fails here
# rather than shipping a bloated page.
#
# Usage:
#   ./scripts/wasm-size-gate.sh [build-dir] [budget-mb]
# Defaults: build-dir = target/dx/logicaffeine-web/release/web/public, budget = 14.7 MB.
# Run it AFTER `dx build --release` (the artifact must exist).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_DIR="${1:-$ROOT/target/dx/logicaffeine-web/release/web/public}"
BUDGET_MB="${2:-14.7}"

# Budget in bytes (awk handles the fractional MB).
BUDGET_BYTES="$(awk "BEGIN { printf \"%d\", $BUDGET_MB * 1024 * 1024 }")"

if [ ! -d "$BUILD_DIR" ]; then
    echo "ERROR: build dir not found: $BUILD_DIR" >&2
    echo "       run 'dx build --release' first (from the repo root)." >&2
    exit 2
fi

# The largest .wasm under the build dir is the app bundle.
largest_wasm="$(find "$BUILD_DIR" -name '*.wasm' -printf '%s %p\n' 2>/dev/null | sort -rn | head -1)"
if [ -z "$largest_wasm" ]; then
    echo "ERROR: no .wasm found under $BUILD_DIR" >&2
    exit 2
fi

size_bytes="${largest_wasm%% *}"
wasm_path="${largest_wasm#* }"
size_mb="$(awk "BEGIN { printf \"%.2f\", $size_bytes / 1024 / 1024 }")"
budget_mb_fmt="$(awk "BEGIN { printf \"%.2f\", $BUDGET_BYTES / 1024 / 1024 }")"

echo "WASM bundle : $wasm_path"
echo "size        : ${size_mb} MB (${size_bytes} bytes)"
echo "budget      : ${budget_mb_fmt} MB (${BUDGET_BYTES} bytes)"

if [ "$size_bytes" -gt "$BUDGET_BYTES" ]; then
    echo "RESULT      : ❌ OVER BUDGET by $(awk "BEGIN { printf \"%.2f\", ($size_bytes - $BUDGET_BYTES)/1024/1024 }") MB" >&2
    echo "A heavy dependency likely leaked into the default bundle — keep transports feature-gated." >&2
    exit 1
fi

echo "RESULT      : ✅ within budget"
