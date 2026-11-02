#!/usr/bin/env bash
# ONE command per rented chip: warm, measure llama.cpp, measure us, settle accuracy, print the table.
#
#   ssh pod 'SM=sm_80 setsid bash /workspace/session-qwen38.sh >/workspace/session.log 2>&1 </dev/null & disown'
#   ssh pod 'SM=sm_90 ...'   # H100
#
# Everything it needs lives on the network volume, so a fresh pod is ready the moment it boots. The f64
# oracle (ref.f32) is chip-independent and is computed once, then reused.
set -uo pipefail
W=/workspace
M=$W/qwen38/Qwen3.8-27B-Q8_0.gguf
B=$W/llama.cpp/build/bin
SM=${SM:-sm_80}
# Comma form: the reference takes `<gguf> <n_gen> <tok,tok,...>`, and passing these space-separated once
# made it run a ONE-token prompt with n_gen=760 while printing a perfectly plausible "oracle".
TOKS="760 6511 314 9338 369"
TOKS_CSV="760,6511,314,9338,369"
export PATH=/usr/local/cuda/bin:$PATH
source "$(dirname "$0")/_stage.sh"
GPU=$(nvidia-smi --query-gpu=name --format=csv,noheader)
say "=== Qwen3.8-27B session on $GPU ($SM) ==="

# ── stage: the volume gate, local-NVMe staging and warming now live in _stage.sh, shared with the
# ── vLLM head-to-head session so neither copy can drift from the other. ─────────────────────────
vol_gate "$W/w38/Wout.q"
stage_local "$W/w38" /root/w38 45
warm_files "$W/w38" "$M"

# ── the bar ──
say "--- llama.cpp tg128 ---"
for FA in 1 0; do echo "-fa $FA:"; $B/llama-bench -m "$M" -ngl 99 -fa $FA -p 0 -n 128 -r 5 2>/dev/null | grep -a tg128; done
say "--- llama.cpp logits (its batch-1 decode path, the one that generates tokens) ---"
[ -x $W/dump_logits ] && for FA in 1 0; do LOGOS_FA=$FA $W/dump_logits "$M" $W/llama_fa$FA.f32 $TOKS; done

# ── ours ──
for LANE in ${LANES:-deq0 deq3 deq3_u2 deq3_u4 deq0_u2}; do
  [ -f "$W/$LANE.cu" ] || continue
  say "--- $LANE ---"
  nvcc -O3 -arch=$SM $W/$LANE.cu -o $W/$LANE 2> $W/$LANE.nvcc.log || { say "NVCC FAILED"; tail -20 $W/$LANE.nvcc.log; continue; }
  LOGOS_Q38_LOGITS=$W/$LANE.f32 $W/$LANE > $W/$LANE.out 2> $W/$LANE.err
  grep -a GENERATED_TOKENS $W/$LANE.out || say "(no tokens)"
  grep -aE "GEN_TOK_S_TG128" $W/$LANE.err || { say "(no metric)"; tail -10 $W/$LANE.err; }
done

# ── the oracle, once ever ──
if [ ! -s $W/ref.f32 ]; then
  say "--- f64 oracle (CPU, chip-independent, computed once) ---"
  LOGOS_REF_LOGITS=$W/ref.f32 $W/model_ref "$M" 13 $TOKS_CSV 2>&1 | grep -aE "REF |ref:"
else
  say "oracle already computed ($(stat -c%s $W/ref.f32) bytes)"
fi

if [ -s $W/ref.f32 ]; then
  python3 - <<'PYCHK'
import struct
v = struct.unpack('<248320f', open('/workspace/ref.f32','rb').read())
am = max(range(len(v)), key=lambda i: v[i])
print("oracle self-check: argmax=%d (golden 11751) %s" % (am, "OK" if am == 11751 else "*** NOT THE GOLDEN TOKEN -- the oracle is wrong, do not compare against it ***"))
PYCHK
fi
say "--- accuracy: distance from the f64 oracle ---"
python3 $W/logit_compare.py $W/ref.f32 $W/llama_fa1.f32 $W/llama_fa0.f32 $W/deq0.f32 $W/deq3.f32 $W/deq3_u2.f32 $W/deq3_u4.f32 $W/deq0_u2.f32
say "=== SESSION DONE ($GPU) ==="
