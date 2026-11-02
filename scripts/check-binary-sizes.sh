#!/usr/bin/env bash
#
# check-binary-sizes.sh — every shipped binary stays within its budget in .config/size-budgets.toml.
#
#   ./scripts/check-binary-sizes.sh                  # build largo + logicaffeine-lsp with --profile dist, check
#   ./scripts/check-binary-sizes.sh --bin-dir DIR     # check binaries already built into DIR (release.yml)
#   ./scripts/check-binary-sizes.sh --only BIN        # check one binary (largo | logicaffeine-lsp)
#   ./scripts/check-binary-sizes.sh --web             # also build the web WASM as the deploy does and gate it
#
# The budgets are the host (x86_64 Linux) sizes measured when they were last lowered, plus headroom;
# `[binaries]` covers the `dist`-profile builds release.yml ships, `[web]` the WASM the frontend
# deploys. A budget only comes down after a measured shrink; raising one is a deliberate edit.
# Env: CHECK_SIZES_TARGET  scratch target dir (default: ${TMPDIR:-/tmp}/logicaffeine-check-sizes)
# Log: logs/binary-size/check-binary-sizes-<timestamp>.log ; exit 0 iff every checked artifact fits.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"
BUDGETS=".config/size-budgets.toml"

BIN_DIR=""
WEB=0
ONLY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --bin-dir) BIN_DIR="${2:?--bin-dir needs a directory}"; shift ;;
    --web) WEB=1 ;;
    --only) ONLY="${2:?--only needs a binary name}"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

budget() { # $1 = table, $2 = key → the integer, or 0 when absent (the budgets file is flat TOML)
  awk -v t="[$1]" -v k="$2" '
    /^\[/ { in_t = ($0 == t); next }
    in_t { split($0, kv, "="); gsub(/[ \t"]/, "", kv[1]); if (kv[1] == k) { gsub(/[^0-9]/, "", kv[2]); print kv[2]; found = 1; exit } }
    END { if (!found) print 0 }' "$BUDGETS"
}

mkdir -p logs/binary-size
LOG="logs/binary-size/check-binary-sizes-$(date -u +%Y%m%d-%H%M%S).log"
FAILURES=()
{
  echo "check-binary-sizes  $(date -u +%FT%TZ)  budgets=$BUDGETS"
  if [ -z "$BIN_DIR" ]; then
    TARGET="${CHECK_SIZES_TARGET:-${TMPDIR:-/tmp}/logicaffeine-check-sizes}"
    case "$TARGET/" in "$REPO_ROOT"/*) echo "CHECK_SIZES_TARGET must be outside the repo" >&2; exit 2 ;; esac
    echo "── building largo + logicaffeine-lsp (--profile dist) into $TARGET ──"
    if ! CARGO_TARGET_DIR="$TARGET" cargo build -p logicaffeine-cli -p logicaffeine-lsp --profile dist; then
      FAILURES+=("the dist build failed")
    fi
    BIN_DIR="$TARGET/dist"
  fi

  echo "── binaries in $BIN_DIR ──"
  BINS=(largo logicaffeine-lsp)
  [ -n "$ONLY" ] && BINS=("$ONLY")
  for bin in "${BINS[@]}"; do
    path="$BIN_DIR/$bin"
    [ -f "$path" ] || path="$BIN_DIR/$bin.exe"
    if [ ! -f "$path" ]; then
      FAILURES+=("$bin was not built")
      continue
    fi
    bytes=$(stat -c %s "$path" 2>/dev/null || stat -f %z "$path")
    limit=$(budget binaries "$bin")
    printf '  %-20s %11d bytes   budget %11d\n' "$bin" "$bytes" "$limit"
    [ "$bytes" -le "$limit" ] || FAILURES+=("$bin is $bytes bytes, over its $limit budget")
  done

  if [ "$WEB" -eq 1 ]; then
    echo "── web WASM, built as deploy-frontend.yml builds it (dx build -p logicaffeine-web --release --ssg --fullstack) ──"
    if ! command -v dx >/dev/null 2>&1; then
      FAILURES+=("dx (the Dioxus CLI) is not installed — cannot build the web WASM")
    elif ! dx build -p logicaffeine-web --release --ssg --fullstack; then
      FAILURES+=("the web deploy build failed")
    elif ! ./scripts/web/wasm-size-gate.sh "${CARGO_TARGET_DIR:-target}/dx/logicaffeine-web/release/web/public"; then
      FAILURES+=("the web WASM failed the deploy size gate (above)")
    fi
  fi

  echo
  if [ "${#FAILURES[@]}" -eq 0 ]; then
    echo "RESULT: ✅ every shipped binary is within its budget"
  else
    echo "RESULT: ❌ ${#FAILURES[@]} size failure(s):"
    printf '  - %s\n' "${FAILURES[@]}"
  fi
} 2>&1 | tee "$LOG"

grep -q '^RESULT: ✅' "$LOG"
