#!/usr/bin/env bash
# Emit the head-to-head lanes locally, for scp to the pod's volume.
#
# These .cu files had NO committed producer: they were emitted ad hoc in an earlier session and left
# on the volume, so every later run measured whatever the emitter looked like months ago while the
# tree moved underneath them. Regenerating them is a prerequisite for any comparison being about the
# compiler as it stands.
#
# 512 prompt tokens / 128 generated is the SAME shape vLLM is benchmarked at
# (--random-input-len 512 --random-output-len 128). The b*.cu lanes with the 5-token golden prompt
# are NOT comparable to that sweep.
set -euo pipefail
OUT=${1:-/tmp/q38-lanes}
mkdir -p "$OUT"
PROMPT=$(python3 -c "
import random
random.seed(20260921)                      # fixed so every re-emit is the same prompt
print(','.join(str(random.randrange(1000, 240000)) for _ in range(512)))")

emit() {                                   # emit <batch> <hl|fast> <prefill_t>
  local b=$1 lane=$2 pt=$3 name
  name="w512_b${b}_${lane}"
  [ "$pt" != 1 ] && name="${name}_pt${pt}"
  local env=(LOGOS_Q38B_EMIT="$OUT/$name.cu" LOGOS_Q38B_BATCH="$b" LOGOS_Q38B_N=128
             LOGOS_Q38B_PROMPT="$PROMPT" LOGOS_Q38B_PREFILL_T="$pt")
  [ "$lane" = hl ] && env+=(LOGOS_Q38B_HL=1)
  # Retried, because this box usually has a co-agent session and a cargo build that races one of
  # their edits fails transiently. A lane silently missing from the volume is worse than a slow emit.
  local try
  for try in 1 2 3; do
    if env "${env[@]}" cargo test -q -p logicaffeine-tests --test qwen38_authored_lock \
         emit_qwen38_batched_cu -- --exact --nocapture 2>"$OUT/$name.emit.log" | grep -a "^wrote"; then
      return 0
    fi
    echo "  (emit $name attempt $try failed; rebuilding)" >&2
  done
  echo "EMIT FAILED $name — see $OUT/$name.emit.log" >&2
  grep -E "^error" "$OUT/$name.emit.log" | head -3 >&2
  return 1
}

for b in 1 8 32 64; do
  for lane in hl fast; do
    emit "$b" "$lane" 32                   # chunked prefill: the lane under test
  done
done
emit 32 hl   1                             # in-boot PT=1 controls, so the chunked-prefill delta is
emit 32 fast 1                             # measured in the SAME boot rather than against a record
echo "--- $(ls -1 "$OUT"/*.cu | wc -l) lanes in $OUT ---"
ls -la "$OUT"/*.cu | awk '{printf "  %-34s %6.1f MB\n", $9, $5/1048576}'
