#!/usr/bin/env bash
# One driver for every qwen38 pod measurement this campaign still owes, because the ad-hoc versions
# lived in a session scratchpad and that scratchpad has already been wiped once mid-campaign.
#
# Usage:
#   scripts/accel/pod/qwen38-measure.sh attribution   # DABLATE x6 — WHERE THE DECODE TIME GOES. Run this first.
#   scripts/accel/pod/qwen38-measure.sh state         # f32 vs f16 GDN state
#   scripts/accel/pod/qwen38-measure.sh kv            # f32 vs f16 KV cache, and both together
#   scripts/accel/pod/qwen38-measure.sh deepgemm      # int8 vs 1-pass fp8 vs 2-pass fp8 (needs sm_90)
#
# Four operational lessons are baked in; each one cost real money to learn:
#   1. GATE THE VOLUME FIRST. A degraded network-volume attachment does not look like a broken mount,
#      it looks like a slow model load. Not gating cost $3 and 26 minutes watching read_bytes stay 0.
#   2. GATE ON A FILE THAT EXISTS. `w38/gu0.q`, not `w38/gu0_q.f`. Three redeploys, ~$0.45.
#   3. BUILD CHEAP, MEASURE EXPENSIVE. nvcc does not need the measurement chip: cross-compiling
#      -arch=sm_90a on an L40 took 2m3s and $0.03; the same build on an H100 is $0.70.
#   4. RUNTIME KNOBS, NOT BUILD FLAGS. Every ablation here is one binary and N runs. As build flags
#      they would be N ten-minute nvcc runs on rented time.
set -uo pipefail
MODE="${1:-attribution}"
cd "$(dirname "$0")/../../.."
OUT="${LOGOS_POD_OUT:-/tmp/qwen38-measure-$MODE.txt}"; : > "$OUT"
say(){ echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT"; }
export RP_IMAGE="${RP_IMAGE:-runpod/pytorch:2.8.0-py3.11-cuda12.8.1-cudnn-devel-ubuntu22.04}"
GPU="${LOGOS_POD_GPU:-dev}"; [ "$MODE" = deepgemm ] && GPU=h100     # DeepGEMM needs Hopper
MIN_MB="${LOGOS_VOL_MIN_MB:-150}"

emit(){   # $1=out .cu   — the shipped decode shape; PREFILL_T=32 is the chunked lane, NOT the PT=1 default
  local prompt; prompt=$(python3 -c "
import random; random.seed(20260921)
print(','.join(str(random.randrange(1000,240000)) for _ in range(512)))")
  # NOT `| grep -q`: under `pipefail`, grep -q exits on the first match, cargo takes SIGPIPE, and the
  # pipeline reports failure on a successful emit. The dry run below caught exactly that on its first
  # execution, which is the argument for having it.
  # The EXIT STATUS decides, not the word "wrote": the emit test writes the file BEFORE its census
  # asserts, so a program with a broken lowering still prints "wrote". Treating that as success hid a
  # red census for a whole session.
  local out rc
  out=$(LOGOS_Q38B_EMIT="$1" LOGOS_Q38B_BATCH="${LOGOS_Q38B_BATCH:-32}" LOGOS_Q38B_N=128 \
        LOGOS_Q38B_PROMPT="$prompt" LOGOS_Q38B_PREFILL_T=32 \
          cargo test -q -p logicaffeine-tests --test qwen38_authored_lock \
            emit_qwen38_batched_cu -- --exact --nocapture 2>&1); rc=$?
  [ "$rc" = 0 ] && case "$out" in *"wrote "*) return 0 ;; esac
  echo "$out" | grep -aE 'panicked|assert|expected' | head -5 >&2
  return 1
}
up(){
  for i in $(seq 1 6); do
    timeout 400 ./scripts/accel/runpod-up.sh --gpu "$GPU" --disk 150 --name "logos-$MODE" >>"$OUT" 2>&1 && return 0
    sleep 30
  done; return 1
}
gate(){  # lesson 1 + 2
  local g; g=$(timeout 120 ssh -o BatchMode=yes pod '
    timeout 30 ls /workspace/w38/gu0.q >/dev/null 2>&1 || { echo GATE_FAIL_NO_WEIGHTS; exit 0; }
    echo "GATE $(timeout 60 dd if=/workspace/w38/gu0.q of=/dev/null bs=4M count=40 iflag=direct 2>&1 \
          | tail -1 | grep -oE "[0-9.]+ [MG]B/s" | tail -1)"' 2>/dev/null)
  say "volume: $g"
  case "$g" in
    *GB/s*) return 0 ;;
    *MB/s*) local n=${g#GATE }; n=${n%% *}
            awk -v n="${n:-0}" -v m="$MIN_MB" 'BEGIN{exit !(n>=m)}' && return 0 ;;
  esac
  say "volume below ${MIN_MB} MB/s — a 24.33 GB load would cost more than the run. Stopping."
  return 1
}
down(){ timeout 200 ./scripts/accel/runpod-down.sh >>"$OUT" 2>&1; say "pod terminated"; }

case "$MODE" in
  attribution) BUILDS="base:"; RUNS="none gdn conv attn norm ffn"; VAR=LOGOS_Q38B_DABLATE ;;
  state)       BUILDS="base: s16:-DBQ_STATE16"; RUNS=""; VAR= ;;
  kv)          BUILDS="base: kv16:-DBQ_KV16 both:-DBQ_KV16+-DBQ_STATE16"; RUNS=""; VAR= ;;
  deepgemm)    BUILDS="base: dg:-DBQ_ENABLE_DG"; RUNS=""; VAR= ;;
  *) say "unknown mode $MODE"; down; exit 2 ;;
esac

CU=$(mktemp /tmp/qwen38-XXXX.cu); emit "$CU" || { say "emit failed"; exit 1; }
say "emitted $(wc -c <"$CU") bytes"

# `DRY=1` exercises everything that does not touch RunPod: the emit, the mode table, and the fact that
# every build this mode asks for actually compiles. An instrument that has never run is a comment, and
# this one is the campaign's handoff — it must not first execute on a rented chip with a live balance.
if [ "${DRY:-0}" = 1 ]; then
  say "DRY RUN — no pod, no spend"
  say "  gpu=$GPU  volume floor=${MIN_MB} MB/s"
  say "  builds: $BUILDS"
  say "  runs:   ${RUNS:-<per-build reps>}  var=${VAR:-<none>}"
  ARCH="${DRY_ARCH:-sm_90a}"
  for b in $BUILDS; do
    n=${b%%:*}; f=${b#*:}; [ "$f" = "$n" ] && f=; f=${f//+/ }
    case "$f" in *BQ_ENABLE_DG*) say "  build '$n': skipped (needs DeepGEMM headers, pod-only)"; continue;; esac
    if nvcc -std=c++17 -arch="$ARCH" -c -o /dev/null $f "$CU" 2>/dev/null; then
      say "  build '$n' ($ARCH ${f:-default}): COMPILES"
    else say "  build '$n' ($ARCH ${f:-default}): FAILED"; fi
  done
  rm -f "$CU"; exit 0
fi
up || { say "no $GPU stock"; exit 1; }
for i in $(seq 1 40); do
  timeout 20 ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o ConnectTimeout=12 pod 'echo up' 2>/dev/null | grep -q up && break
  [ "$i" = 40 ] && { say "no ssh"; down; exit 1; }; sleep 12
done
say "chip: $(timeout 30 ssh -o BatchMode=yes pod 'nvidia-smi --query-gpu=name,memory.total --format=csv,noheader' 2>/dev/null)"
gate || { down; exit 1; }
timeout 300 scp -q "$CU" pod:/workspace/m.cu 2>>"$OUT"


timeout 3000 ssh -o BatchMode=yes pod "export PATH=/usr/local/cuda/bin:\$PATH; W=/workspace
CC=\$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader|head -1|tr -d .)
DGI='-I/root/dg/DeepGEMM-main/deep_gemm/include -I/root/dg/cutlass-3.8.0/include --expt-relaxed-constexpr --diag-suppress 177 -lcuda'
for b in $BUILDS; do
  n=\${b%%:*}; f=\${b#*:}; [ \"\$f\" = \"\$n\" ] && f=; f=\${f//+/ }
  case \"\$f\" in *BQ_ENABLE_DG*) f=\"\$f \$DGI\";; esac
  nvcc -O3 -arch=sm_\$CC \$f -o \$W/\$n \$W/m.cu 2>\$W/\$n.log || { echo \"BUILD_FAIL \$n\"; grep -aoE 'error[^\\\"]{0,80}' \$W/\$n.log|head -3; }
done
run(){ timeout 600 env \$2 \$W/\$1 > \$W/\$1.\$3.out 2> \$W/\$1.\$3.err
  printf '%-6s %-12s ' \"\$1\" \"\$3\"
  grep -aoE 'PREFILL_MS [0-9.]+' \$W/\$1.\$3.err|tail -1|tr -d '\n'; printf '  '
  grep -aoE 'AGGREGATE_TOK_S [0-9.]+' \$W/\$1.\$3.err|tail -1|tr -d '\n'
  printf '  tok '; grep -a SEQ0_TOKENS \$W/\$1.\$3.out|head -1|cut -c14-40; }
if [ -n '$RUNS' ]; then for m in $RUNS; do run base \"$VAR=\$m\" \$m; done
else for b in $BUILDS; do n=\${b%%:*}; for r in 1 2; do [ -x \$W/\$n ] && run \$n '' rep\$r; done; done; fi" >>"$OUT" 2>&1

say "measured — results in $OUT"
down
