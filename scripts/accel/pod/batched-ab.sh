#!/usr/bin/env bash
# BATCHED head-to-head: aggregate token generation throughput at B parallel sequences, ours vs llama.cpp.
#
# The comparison is llama.cpp's own `llama-batched-bench`, whose `S_TG t/s` column is exactly what our
# AGGREGATE_TOK_S measures: tokens per second summed across B sequences decoding together. Both share one
# pass over the weights per step, which is the entire reason batching helps.
set -uo pipefail
W=/workspace
M=$W/qwen38/Qwen3.8-27B-Q8_0.gguf
B=$W/llama.cpp/build/bin
SM=${SM:-sm_90}
BATCHES=${BATCHES:-1 2 4 8 16 32}
export PATH=/usr/local/cuda/bin:$PATH
say(){ echo "[$(date -u +%H:%M:%S)] $*"; }
say "=== batched A/B on $(nvidia-smi --query-gpu=name --format=csv,noheader) ($SM) ==="

say "warming weights"
find $W/w38 -type f -print0 2>/dev/null | xargs -0 -P 24 -I{} cat {} > /dev/null 2>&1
cat "$M" > /dev/null 2>&1

# ── llama.cpp's parallel decode: S_TG is its aggregate generation throughput ──
say "--- llama.cpp llama-batched-bench (S_TG = aggregate t/s) ---"
NPL=$(echo $BATCHES | tr ' ' ',')
$B/llama-batched-bench -m "$M" -c 8192 -b 2048 -ub 512 -npp 5 -ntg 128 -npl "$NPL" -ngl 99 2>/dev/null \
  | grep -aE "^\|" | tail -n +3

# ── ours ──
for b in $BATCHES; do
  L=b$b
  [ -f "$W/$L.cu" ] || { say "(no $L.cu)"; continue; }
  say "--- ours batch $b ---"
  nvcc -O3 -arch=$SM "$W/$L.cu" -o "$W/$L" 2> $W/$L.nvcc.log || { say "NVCC FAILED"; tail -12 $W/$L.nvcc.log; continue; }
  "$W/$L" > $W/$L.out 2> $W/$L.err
  grep -aE "SEQ0_TOKENS|BATCH_AGREEMENT" $W/$L.out
  grep -aE "AGGREGATE_TOK_S" $W/$L.err || { say "(no metric)"; tail -8 $W/$L.err; }
done
say "=== BATCHED AB DONE ==="
