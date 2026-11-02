#!/usr/bin/env bash
# Instruction-sampled profile + flamegraph of the arena checker on one input.
#   [CORE=n] [PERIOD=instructions-per-sample] scripts/lean/arena-crown/profile.sh <input.ndjson> [label] [checker flags...]
# Samples user-space instructions PRECISELY (PEBS: the retiring instruction itself, not the one
# after a stall) on a pinned P-core — the Arena's ranking metric — with frame
# pointers, then folds with scripts/dev/flamefold.py and renders logs/arena-crown/<label>.svg.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
IN="$1"; LABEL="${2:-$(basename "$IN" .ndjson)}"; shift; shift || true
OUT="$ROOT/logs/arena-crown"; mkdir -p "$OUT"
cd "$ROOT"
RUSTFLAGS="-C force-frame-pointers=yes" CARGO_TARGET_DIR="$ROOT/target/arena-prof" \
  cargo build --profile arena -p logicaffeine-kernel --example logos_kernel >/dev/null
# Profile a content-addressed copy: `perf script` symbolises against the binary on disk, and a
# later rebuild of target/arena-prof (another profile) must not change what this one resolves to.
BUILT="$ROOT/target/arena-prof/arena/examples/logos_kernel"
mkdir -p "$OUT/bin"
BIN="$OUT/bin/logos_kernel-prof-$(sha256sum "$BUILT" | cut -c1-16)"
if [ ! -e "$BIN" ]; then
  TMP="$(mktemp "$BIN.XXXXXX")"
  cp "$BUILT" "$TMP" && chmod +x "$TMP" && mv -f "$TMP" "$BIN"
fi
echo "profiling $BIN"
sudo -n perf record -q -e cpu_core/instructions/upp -c "${PERIOD:-2000000}" --call-graph fp -o "$OUT/$LABEL.perf.data" \
  -- taskset -c "${CORE:-2}" sudo -u "$(id -un)" -- "$BIN" "$@" "$IN" || true
sudo -n chown "$(id -un)" "$OUT/$LABEL.perf.data"
perf script -i "$OUT/$LABEL.perf.data" 2>/dev/null | python3 scripts/dev/flamefold.py --folded "$OUT/$LABEL.folded" --summary "$OUT/$LABEL.summary.json" >/dev/null
python3 scripts/lean/arena-crown/folded-to-svg.py "$OUT/$LABEL.folded" "$OUT/$LABEL.svg" --title "logos-kernel: $LABEL (instructions)"
echo "flamegraph: $OUT/$LABEL.svg"
