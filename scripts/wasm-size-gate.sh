#!/usr/bin/env bash
# WASM bundle size gate (work/FINISH_INTERPRETER.md Phase 9).
#
# The default Studio build must stay within a payload budget — net-libp2p-wasm and
# other heavy transports are feature-gated OUT of the bundle, and images and
# heavyweight data live outside the binary (include_dir scoped to
# assets/curriculum/, runtime /data fetch). This gate locks all of that in with
# two budgets:
#
#   MAIN  — the eager bundle (logicaffeine-web_bg-*.wasm) every visitor downloads
#           before anything is interactive. wasm-split is TEMPORARILY DISABLED
#           (the fork's splitter miscompiles rendering — see deploy-frontend.yml),
#           so the LOGOS engine now ships in the eager bundle: measures ~12 MiB,
#           budget 14 MiB. Restore the ~2 MiB budget when split is re-enabled. A
#           heavy NEW dependency leaking in still fails here (14 MiB is a tight
#           cap over the measured ~12 MiB).
#   TOTAL — all .wasm artifacts together (eager bundle + any lazy chunks).
#           A loose backstop against runaway growth: budget 60 MiB.
#
# Usage:
#   ./scripts/wasm-size-gate.sh [build-dir] [main-budget-mb] [total-budget-mb]
# Defaults: build-dir = target/dx/logicaffeine-web/release/web/public,
#           main = 14 MiB, total = 60 MiB.
# Run it AFTER `dx build --release --ssg --fullstack`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_DIR="${1:-$ROOT/target/dx/logicaffeine-web/release/web/public}"
MAIN_BUDGET_MB="${2:-14}"
TOTAL_BUDGET_MB="${3:-60}"

MAIN_BUDGET_BYTES="$(awk "BEGIN { printf \"%d\", $MAIN_BUDGET_MB * 1024 * 1024 }")"
TOTAL_BUDGET_BYTES="$(awk "BEGIN { printf \"%d\", $TOTAL_BUDGET_MB * 1024 * 1024 }")"

if [ ! -d "$BUILD_DIR" ]; then
    echo "ERROR: build dir not found: $BUILD_DIR" >&2
    echo "       run the dx build first (from the repo root)." >&2
    exit 2
fi

main_wasm="$(find "$BUILD_DIR" -name 'logicaffeine-web_bg-*.wasm' -printf '%s %p\n' 2>/dev/null | sort -rn | head -1)"
if [ -z "$main_wasm" ]; then
    echo "ERROR: no logicaffeine-web_bg-*.wasm found under $BUILD_DIR" >&2
    exit 2
fi
main_bytes="${main_wasm%% *}"
main_path="${main_wasm#* }"

total_bytes="$(find "$BUILD_DIR" -name '*.wasm' -printf '%s\n' | awk '{ s += $1 } END { print s }')"
chunk_count="$(find "$BUILD_DIR" -name '*.wasm' | wc -l)"

fmt_mb() { awk "BEGIN { printf \"%.2f\", $1 / 1024 / 1024 }"; }

echo "MAIN bundle : $main_path"
echo "  size      : $(fmt_mb "$main_bytes") MB (${main_bytes} bytes)"
echo "  budget    : $(fmt_mb "$MAIN_BUDGET_BYTES") MB"
echo "TOTAL wasm  : $(fmt_mb "$total_bytes") MB across $chunk_count artifacts"
echo "  budget    : $(fmt_mb "$TOTAL_BUDGET_BYTES") MB"

fail=0
if [ "$main_bytes" -gt "$MAIN_BUDGET_BYTES" ]; then
    echo "RESULT      : ❌ MAIN bundle over budget by $(fmt_mb "$((main_bytes - MAIN_BUDGET_BYTES))") MB" >&2
    echo "A heavy dependency likely leaked into the default bundle — keep transports feature-gated." >&2
    fail=1
fi
if [ "$total_bytes" -gt "$TOTAL_BUDGET_BYTES" ]; then
    echo "RESULT      : ❌ TOTAL wasm over budget by $(fmt_mb "$((total_bytes - TOTAL_BUDGET_BYTES))") MB" >&2
    fail=1
fi
[ "$fail" -ne 0 ] && exit 1

echo "RESULT      : ✅ within budget"
