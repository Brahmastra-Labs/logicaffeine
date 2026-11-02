#!/usr/bin/env bash
# decode-gpu-measure.sh RUN_DIR — the RTX 3060's side of the int8 decode receipt, in this boot: the hand-written reference
# and LOGOS's one-block and general programs of the decode step, each printed from this checkout and held to its oracle
# (the decode_gpu_programs example: the trainer's tokens, the evaluator), timed, then run sustained for 15 s with
# power.draw and utilization sampled every 0.25 s. Every timed run waits for 60 s with no other process on the GPU and no
# nvcc, and records whether one appeared. scripts/accel/decode_gpu_ledger.py turns RUN_DIR into the ledger.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
D=$1; P="$D/programs"
mkdir -p "$D" "$P" && cd "$D"
others() { nvidia-smi --query-compute-apps=pid,process_name --format=csv,noheader | grep -v "^$"; }
quiet() {
  local q=0
  while [ $q -lt 60 ]; do
    if [ -n "$(others)" ] || pgrep -x nvcc > /dev/null || pgrep -x cicc > /dev/null; then q=0; else q=$((q + 5)); fi
    sleep 5
  done
  echo "  window at $(date -u '+%T') UTC, load average $(cut -d' ' -f1-3 /proc/loadavg)" >> log.txt
}
# Sample power and utilization every 0.25 s while PID runs, and name any other GPU process seen meanwhile.
sample() {
  local pid=$1 out=$2 me=$3 seen=""
  : > "$out"
  while kill -0 "$pid" 2>/dev/null; do
    nvidia-smi --query-gpu=power.draw,utilization.gpu,pstate --format=csv,noheader,nounits >> "$out"
    # nvidia-smi prints `[No data]` while no process holds a context: that is no process.
    while read -r l; do case "$l" in *$me*|"[No data]"|"") ;; *) seen="$seen $l";; esac; done < <(nvidia-smi --query-compute-apps=process_name --format=csv,noheader)
    sleep 0.25
  done
  echo "${seen:-none}" > "$out.others"
}
echo "boot $(uptime -s) UTC; driver $(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" > log.txt
# Build the printer, then print, build and check the reference outside the timed windows.
( cd "$ROOT" && cargo build -q --release -p logicaffeine-compile --example decode_gpu_programs ) > build.log 2>&1 \
  || { echo "decode_gpu_programs did not build: $D/build.log" >> log.txt; exit 1; }
programs() { ( cd "$ROOT" && cargo run -q --release -p logicaffeine-compile --example decode_gpu_programs -- "$P" "$1" ); }
programs reference >> log.txt 2>&1; grep -q "^reference: MATCH" log.txt || { echo "the reference does not generate the trainer's tokens" >> log.txt; exit 1; }
quiet
echo "clean window from $(date -u '+%F %T') UTC" >> log.txt
for i in $(seq 1 80); do nvidia-smi --query-gpu=power.draw,pstate --format=csv,noheader,nounits; sleep 0.25; done > idle.txt
"$P/reference" > ref.out 2>&1; cat ref.out >> log.txt
"$P/reference" 400000 > ref_long.out 2>&1 & PID=$!; sleep 1; sample $PID ref_long.power reference; wait $PID
echo "reference sustained: $(tr '\n' ' ' < ref_long.out) others: $(cat ref_long.power.others)" >> log.txt
# LOGOS: its programs printed, held to the evaluator and timed (best of 200) in a window of their own; then each
# sustained 15 s.
quiet
programs logos > logos.out 2>&1; cat logos.out >> log.txt
grep -q "^flat: .* MATCH" logos.out && grep -q "^general: .* MATCH" logos.out || { echo "a LOGOS program does not match the evaluator" >> log.txt; exit 1; }
for prog in flat general; do
  us=$(grep -oE "^$prog: .*US_PER_STEP [0-9.]+" logos.out | grep -oE "[0-9.]+$")
  python3 "$ROOT/scripts/accel/decode_gpu_sustain.py" "$P/${prog}_bench.cu" "$D/${prog}_sustain.cu" 15 "$us" || exit 1
  nvcc -O3 -arch=sm_86 -std=c++17 -o "$D/${prog}_sustain" "$D/${prog}_sustain.cu" >> log.txt 2>&1 || { echo "nvcc $prog failed" >> log.txt; exit 1; }
done
for prog in flat general; do
  quiet
  ( cd "$P" && exec "$D/${prog}_sustain" ) > ${prog}_sustain.out 2>&1 & PID=$!; sleep 1; sample $PID ${prog}_sustain.power ${prog}_sustain; wait $PID
  echo "$prog sustained: $(tr '\n' ' ' < ${prog}_sustain.out) others: $(cat ${prog}_sustain.power.others)" >> log.txt
done
echo done >> log.txt
