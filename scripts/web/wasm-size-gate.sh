#!/usr/bin/env bash
# WASM bundle size gate.
#
# The deploy build must stay within its payload budget: net-libp2p-wasm and the other heavy transports are
# feature-gated OUT of the bundle, and images and heavyweight data live outside the binary (include_dir scoped
# to assets/curriculum/, runtime /data fetch). Two budgets, both read from .config/size-budgets.toml [web]:
#
#   wasm        — the eager bundle (logicaffeine-web_bg-*.wasm) every visitor downloads before anything is
#                 interactive. wasm-split is disabled (the fork's splitter miscompiles rendering — see
#                 deploy-frontend.yml), so the whole LOGOS engine ships in it; the budget comes down to the
#                 split entry's size when split is re-enabled.
#   wasm_total  — every .wasm artifact together (the eager bundle plus any lazy chunks).
#
# Each is a measured size plus 3%, so a heavy new dependency leaking in fails here; raising one is a
# deliberate edit of the budgets file (size_budget_lock holds this gate to it).
#
# Usage: ./scripts/web/wasm-size-gate.sh [build-dir]
#   build-dir defaults to target/dx/logicaffeine-web/release/web/public.
# Run it AFTER `dx build -p logicaffeine-web --release --ssg --fullstack`.
# Exit: 0 within budget, 1 over budget, 2 no budget or no build.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_DIR="${1:-$ROOT/target/dx/logicaffeine-web/release/web/public}"
BUDGETS="$ROOT/.config/size-budgets.toml"

budget() { # $1 = a [web] key → its integer (the budgets file is flat TOML)
    awk -v k="$1" '
        /^\[/ { in_web = ($0 == "[web]"); next }
        in_web { split($0, kv, "="); gsub(/[ \t"]/, "", kv[1]); if (kv[1] == k) { gsub(/[^0-9]/, "", kv[2]); print kv[2]; exit } }' "$BUDGETS"
}
MAIN_BUDGET_BYTES="$(budget wasm)"
TOTAL_BUDGET_BYTES="$(budget wasm_total)"
if [ -z "$MAIN_BUDGET_BYTES" ] || [ -z "$TOTAL_BUDGET_BYTES" ]; then
    echo "ERROR: $BUDGETS has no [web] wasm / wasm_total budget" >&2
    exit 2
fi

if [ ! -d "$BUILD_DIR" ]; then
    echo "ERROR: build dir not found: $BUILD_DIR" >&2
    echo "       run the dx build first (from the repo root)." >&2
    exit 2
fi

main_path=""
main_bytes=0
total_bytes=0
chunk_count=0
while IFS= read -r -d '' wasm; do
    bytes="$(wc -c < "$wasm" | tr -d ' ')"
    total_bytes=$((total_bytes + bytes))
    chunk_count=$((chunk_count + 1))
    case "$(basename "$wasm")" in
        logicaffeine-web_bg-*.wasm)
            if [ -z "$main_path" ] || [ "$bytes" -gt "$main_bytes" ]; then
                main_path="$wasm"
                main_bytes="$bytes"
            fi ;;
    esac
done < <(find "$BUILD_DIR" -name '*.wasm' -print0)

if [ -z "$main_path" ]; then
    echo "ERROR: no logicaffeine-web_bg-*.wasm found under $BUILD_DIR" >&2
    exit 2
fi

fmt_mb() { awk "BEGIN { printf \"%.2f\", $1 / 1024 / 1024 }"; }

echo "MAIN bundle : $main_path"
echo "  size      : $(fmt_mb "$main_bytes") MiB (${main_bytes} bytes)"
echo "  budget    : $(fmt_mb "$MAIN_BUDGET_BYTES") MiB (${MAIN_BUDGET_BYTES} bytes, [web] wasm)"
echo "TOTAL wasm  : $(fmt_mb "$total_bytes") MiB across $chunk_count artifacts (${total_bytes} bytes)"
echo "  budget    : $(fmt_mb "$TOTAL_BUDGET_BYTES") MiB (${TOTAL_BUDGET_BYTES} bytes, [web] wasm_total)"

fail=0
if [ "$main_bytes" -gt "$MAIN_BUDGET_BYTES" ]; then
    echo "RESULT      : ❌ MAIN bundle over budget by $((main_bytes - MAIN_BUDGET_BYTES)) bytes" >&2
    echo "A heavy dependency likely leaked into the default bundle — keep transports feature-gated." >&2
    fail=1
fi
if [ "$total_bytes" -gt "$TOTAL_BUDGET_BYTES" ]; then
    echo "RESULT      : ❌ TOTAL wasm over budget by $((total_bytes - TOTAL_BUDGET_BYTES)) bytes" >&2
    exit 1
fi

echo "RESULT      : ✅ within budget"
