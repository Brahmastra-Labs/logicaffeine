#!/usr/bin/env bash
# beat-llama-ab.sh — A/B a compiler-emitted 30B decode lane against llama.cpp on the SAME H100.
#
# The pit-of-success rig for the "beat llama" campaign: builds+runs a lane's
# emitted .cu on the pod, checks it emits the GOLDEN token stream, measures tg128, and compares to llama-bench
# on the same q30.gguf. A lane is only a WIN if golden==Y AND its tg128 > llama's.
#
#   scripts/accel/beat-llama-ab.sh <lane.cu-on-pod>       e.g. gen30b_f5.cu (default golden lane)
#
# Env: POD (ssh host, default "pod"), Q30 (gguf, default /workspace/q30.gguf), SM (arch, default sm_90).
set -uo pipefail
POD="${POD:-pod}"
Q30="${Q30:-/workspace/q30.gguf}"
SM="${SM:-sm_90}"
CU="${1:?usage: beat-llama-ab.sh <lane.cu-on-pod>}"
BIN="${CU%.cu}"

# The golden generation prefix (llama.cpp greedy of "The capital of France is ...").
GOLD="12095 13 576 6722 315 279 3639 15072 374 7148 13 576 6722 315 279 3639 4180 374 6515 11 422 727 13 576"

echo "== building $CU on $POD ($SM) =="
ssh -o BatchMode=yes "$POD" "cd /workspace && /usr/local/cuda/bin/nvcc -O3 -arch=$SM $CU -o $BIN 2>&1 | grep -i error | head; echo BUILT_$BIN"

echo "== running lane $BIN =="
OUT=$(ssh -o BatchMode=yes "$POD" "cd /workspace && ./$BIN 2>&1 | grep -E 'GEN_TOK_S_TG128|GENERATED_TOKENS'")
OURS_TPS=$(echo "$OUT" | grep GEN_TOK_S_TG128 | grep -oE '[0-9]+\.[0-9]+' | head -1)
GENTOK=$(echo "$OUT" | grep GENERATED_TOKENS | sed 's/GENERATED_TOKENS: //')
# golden-match = the generated stream starts with the golden prefix.
if [[ "$GENTOK " == "$GOLD "* ]]; then GMATCH="Y"; else GMATCH="N"; fi

echo "== running llama-bench (synthetic) + llama-simple (REAL steady-state) on $Q30 =="
# Synthetic tg128 (what everyone quotes) — but it over-states llama vs real single-stream generation.
LOUT=$(ssh -o BatchMode=yes "$POD" "cd /workspace && ./llama.cpp/build/bin/llama-bench -m $Q30 -ngl 99 -fa 1 -p 0 -n 128 -r 3 2>/dev/null | grep tg128")
LLAMA_BENCH=$(echo "$LOUT" | awk -F'|' '{v=$(NF-1); split(v,a,"±"); gsub(/[^0-9.]/,"",a[1]); print a[1]}')
# REAL steady-state generation eval (the honest apples-to-apples to our autoregressive decode): -n 128 amortizes
# graph capture; the "eval time ... tokens per second" line is the pure decode rate.
SOUT=$(ssh -o BatchMode=yes "$POD" "cd /workspace && ./llama.cpp/build/bin/llama-simple -m $Q30 -ngl 99 -n 128 'The capital of France is' 2>&1 | grep -i 'eval time' | grep -vi prompt")
# the DECODE eval line: 'eval time = ... ( X ms per token, Y tokens per second)' — Y is the steady-state rate.
LLAMA_TPS=$(echo "$SOUT" | sed -n 's/.*, *\([0-9.]*\) tokens per second.*/\1/p' | head -1)

echo ""
echo "================= A/B (H100 $SM, $Q30) ================="
printf "  lane %-18s : %8s tok/s   golden=%s (full 24-tok)\n" "$BIN" "${OURS_TPS:-?}" "$GMATCH"
printf "  llama.cpp -fa (real steady n=128) : %8s tok/s   <- the honest bar\n" "${LLAMA_TPS:-?}"
printf "  llama.cpp -fa (synthetic bench)   : %8s tok/s   (over-states vs real gen)\n" "${LLAMA_BENCH:-?}"
if [[ -n "${OURS_TPS:-}" && -n "${LLAMA_TPS:-}" ]]; then
  awk -v o="$OURS_TPS" -v l="$LLAMA_TPS" -v g="$GMATCH" 'BEGIN{
    r=o/l*100;
    if (g!="Y") printf "  >> NOT GOLDEN — disqualified regardless of speed (%.1f%% of llama real)\n", r;
    else if (o>l) printf "  >> WIN at real steady state: golden AND %.2f > %.2f \342\234\205\n", o, l;
    else printf "  >> golden, %.1f%% of llama real steady state — %.2f vs %.2f, ratchet on\n", r, o, l;
  }'
fi
echo "==============================================================="
