#!/usr/bin/env bash
# Weight-free decode attribution: every kernel of one GDN layer, one attention layer and the LM head,
# at the shipped decode shape (B=32, context 640), timed in isolation on random buffers and scaled to a
# whole step (x48 GDN, x16 attention, x1 head). Runs on any local GPU with ~4 GB free — no weights, no pod.
#
# It exists because every decision past the GEMM had been argued from a model, and five of those
# arguments were retracted. This one found that attention re-read each KV head six times; the
# GQA-shared kernel it motivated is bit-identical and 1.9x faster. Absolute times are the local chip's;
# the SHARES are what transfer, and a pod run of `scripts/accel/pod/qwen38-measure.sh attribution` confirms them.
set -euo pipefail
cd "$(dirname "$0")/../../.."
OUT="${TMPDIR:-/tmp}/qwen38-attr"; mkdir -p "$OUT"
PROMPT=$(python3 -c "
import random; random.seed(20260921)
print(','.join(str(random.randrange(1000,240000)) for _ in range(512)))")
LOGOS_Q38B_EMIT="$OUT/emitted.cu" LOGOS_Q38B_BATCH=32 LOGOS_Q38B_N=128 LOGOS_Q38B_PROMPT="$PROMPT" \
LOGOS_Q38B_PREFILL_T=32 cargo test -q -p logicaffeine-tests --test qwen38_authored_lock \
  emit_qwen38_batched_cu -- --exact --nocapture >/dev/null 2>&1
ARCH="${ARCH:-sm_$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d .)}"
nvcc -std=c++17 -O3 -arch="$ARCH" -w ${EXTRA_FLAGS:-} -DQWEN38_EMITTED="\"$OUT/emitted.cu\"" \
  -o "$OUT/attr" scripts/accel/qwen38/decode_attribution.cu
"$OUT/attr"
