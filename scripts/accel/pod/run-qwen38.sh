#!/usr/bin/env bash
# End-to-end Qwen3.8-27B on the pod: extract the weights, build the emitted CUDA, verify the token stream
# against llama.cpp, and measure throughput both ways.
#
#   ssh pod 'setsid bash /workspace/run-qwen38.sh >/workspace/run38.log 2>&1 </dev/null & disown'
#
# Expects on /workspace: qwen38/Qwen3.8-27B-Q8_0.gguf, llama.cpp/build, extract_qwen38, gen38.cu (and
# optionally gen38_precise.cu). SM defaults to sm_80 (A100); pass SM=sm_90 for H100.
set -uo pipefail
W=/workspace
M=$W/qwen38/Qwen3.8-27B-Q8_0.gguf
B=$W/llama.cpp/build/bin
WD=$W/w38
SM=${SM:-sm_80}
NGEN=${NGEN:-24}
export PATH=/usr/local/cuda/bin:$PATH
say(){ echo "[$(date -u +%H:%M:%S)] $*"; }

say "=== run-qwen38 on $(nvidia-smi --query-gpu=name --format=csv,noheader) ($SM) ==="

# ── 1. weights → per-tensor files (idempotent; ~29 GB, several minutes the first time) ──
if [ ! -f "$WD/Wout.q" ]; then
  say "--- extracting weights to $WD ---"
  mkdir -p "$WD"
  "$W/extract_qwen38" "$M" "$WD" || { say "!! extraction FAILED"; exit 1; }
else
  say "weights already extracted ($(du -sh "$WD" | cut -f1))"
fi
say "extracted: $(ls "$WD" | wc -l) files, $(du -sh "$WD" | cut -f1)"

# ── 2. llama.cpp's golden token stream for the same prompt (greedy, no speculation) ──
say "--- llama.cpp golden (greedy, temp 0) ---"
$B/llama-cli -m "$M" -ngl 99 -n "$NGEN" -st --temp 0 --verbose-prompt \
    -p 'The capital of France is' 2>&1 | tail -30 | tee $W/golden38.txt

# ── 3. build and run OUR emitted program ──
for LANE in gen38 gen38_precise; do
  [ -f "$W/$LANE.cu" ] || continue
  say "--- building $LANE.cu for $SM ---"
  if ! nvcc -O3 -arch=$SM -lineinfo "$W/$LANE.cu" -o "$W/$LANE" 2> $W/$LANE.nvcc.log; then
    say "!! nvcc FAILED for $LANE:"; tail -25 $W/$LANE.nvcc.log; continue
  fi
  say "--- running $LANE ---"
  "$W/$LANE" > $W/$LANE.out 2> $W/$LANE.err
  grep -a "GENERATED_TOKENS" $W/$LANE.out || say "(no token line)"
  grep -aE "GEN_TOK_S" $W/$LANE.err || { say "!! no throughput line; stderr:"; tail -15 $W/$LANE.err; }
done

# ── 4. llama.cpp throughput, the apples-to-apples bar (5 reps — 3 is not enough for this model) ──
say "--- llama-bench tg128 (the bar) ---"
for FA in 1 0; do
  say "-fa $FA:"; $B/llama-bench -m "$M" -ngl 99 -fa $FA -p 0 -n 128 -r 5 2>/dev/null | tail -3
done
say "=== RUN38 DONE ==="
