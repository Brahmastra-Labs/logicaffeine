#!/usr/bin/env bash
# The A/B session: token identity, THROUGHPUT vs llama.cpp, and LOGIT ACCURACY vs an f64 oracle.
#
#   ssh pod 'setsid bash /workspace/run-qwen38-ab.sh >/workspace/ab.log 2>&1 </dev/null & disown'
#
# Expects on /workspace: qwen38/Qwen3.8-27B-Q8_0.gguf, llama.cpp (built), w38/ (extracted), and the
# emitted gen38*.cu. SM=sm_80 for A100, sm_90 for H100.
set -uo pipefail
W=/workspace
M=$W/qwen38/Qwen3.8-27B-Q8_0.gguf
B=$W/llama.cpp/build/bin
SM=${SM:-sm_80}
PROMPT_TOKS="760 6511 314 9338 369"
export PATH=/usr/local/cuda/bin:$PATH
say(){ echo "[$(date -u +%H:%M:%S)] $*"; }

say "=== qwen38 A/B on $(nvidia-smi --query-gpu=name --format=csv,noheader) ($SM) ==="

# ── 0. warm the weights into page cache ──────────────────────────────────────────────────────────
# The network volume serves 917 extracted tensor files, and the loader opens them one at a time; cold, that
# is ~12 minutes of rented GPU doing nothing per lane. A parallel read pulls the whole set into RAM once, so
# every lane after it loads at memory speed. Costs nothing but the first pass, which we were paying anyway.
say "warming $(du -sh $W/w38 2>/dev/null | cut -f1) of weights into page cache"
find $W/w38 -type f -print0 2>/dev/null | xargs -0 -P 16 -I{} cat {} > /dev/null 2>&1
say "warm ($(free -g | awk "/^Mem:/{print \$6}") GB cached)"

# ── 1. throughput + tokens, every emitted lane ───────────────────────────────────────────────────
for LANE in gen38_precise gen38 gen38_max; do
  [ -f "$W/$LANE.cu" ] || continue
  say "--- $LANE: build ---"
  if ! nvcc -O3 -arch=$SM "$W/$LANE.cu" -o "$W/$LANE" 2> $W/$LANE.nvcc.log; then
    say "!! nvcc FAILED for $LANE:"; tail -25 $W/$LANE.nvcc.log; continue
  fi
  say "--- $LANE: run ---"
  "$W/$LANE" > $W/$LANE.out 2> $W/$LANE.err
  grep -a "GENERATED_TOKENS" $W/$LANE.out || say "(no token line)"
  grep -aE "GEN_TOK_S" $W/$LANE.err || { say "!! no throughput line"; tail -15 $W/$LANE.err; }
done

# ── 2. the bar, same instance, 5 reps ────────────────────────────────────────────────────────────
say "--- llama-bench tg128 (the bar) ---"
for FA in 1 0; do
  say "-fa $FA:"; $B/llama-bench -m "$M" -ngl 99 -fa $FA -p 0 -n 128 -r 5 2>/dev/null | grep -a tg128
done

# ── 3. LOGIT ACCURACY: both implementations against the f64 oracle ───────────────────────────────
# The oracle is our CPU forward in f64 (Q8_0 codes and scales are exact in f64, so its only error is f64
# rounding). Whoever lands closer to it computes the model more accurately -- a claim that does not depend
# on either GPU implementation being "the" right answer.
say "--- logit accuracy ---"
if [ ! -f "$W/dump_logits" ]; then
  say "building dump_logits"
  g++ -O2 $W/dump_logits.cpp -I$W/llama.cpp/include -I$W/llama.cpp/ggml/include \
      -L$B -lllama -Wl,-rpath,$B -o $W/dump_logits 2> $W/dump_logits.log \
      || { say "!! dump_logits build failed"; tail -20 $W/dump_logits.log; }
fi
if [ -x "$W/dump_logits" ]; then
  for FA in 1 0; do
    LOGOS_FA=$FA $W/dump_logits "$M" $W/llama_fa$FA.f32 $PROMPT_TOKS || say "!! dump_logits -fa $FA failed"
  done
fi
say "oracle (f64 CPU reference; several minutes)"
LOGOS_REF_LOGITS=$W/ref.f32 $W/model_ref "$M" $PROMPT_TOKS 2>&1 | tail -4
for LANE in gen38_precise gen38 gen38_max; do
  [ -x "$W/$LANE" ] || continue
  LOGOS_Q38_LOGITS=$W/$LANE.f32 "$W/$LANE" > /dev/null 2>&1
done
python3 $W/logit_compare.py $W/ref.f32 $W/llama_fa1.f32 $W/llama_fa0.f32 \
        $W/gen38_precise.f32 $W/gen38.f32 $W/gen38_max.f32 2>&1 | tail -20
say "=== AB DONE ==="
