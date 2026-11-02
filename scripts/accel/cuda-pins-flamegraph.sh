#!/usr/bin/env bash
# Run an emitted LOGOS CUDA forward binary with kernel-phase pins and render a flamegraph SVG.
#
#   scripts/accel/cuda-pins-flamegraph.sh ./qwen30b_full out.svg "Qwen3-30B forward (A100)"
#
# GPU hardware counters (ncu) are permission-gated on cloud pods, so the forward times itself:
# LOGOS_CUDA_PINS=1 brackets every kernel launch with CUDA events → per-phase `PIN` lines on stderr.
# Pipe an existing PIN capture straight into the python renderer if you can't run the binary here.
set -euo pipefail
BIN="${1:?usage: cuda-pins-flamegraph.sh <forward-binary> [out.svg] [title]}"
OUT="${2:-cuda_forward_flamegraph.svg}"
TITLE="${3:-LOGOS Qwen3-30B forward — GPU phase flamegraph}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOGOS_CUDA_PINS=1 "$BIN" 2>&1 >/dev/null | grep '^PIN' \
  | python3 "$HERE/cuda-pins-flamegraph.py" --title "$TITLE" -o "$OUT"
