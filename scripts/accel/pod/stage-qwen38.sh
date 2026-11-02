#!/usr/bin/env bash
# Stage everything Qwen3.8-27B needs onto the network volume. Idempotent: safe to re-run, skips what
# is already there. Run DETACHED so the ssh session can drop without killing a 29 GB download:
#
#   ssh pod 'setsid bash /workspace/stage-qwen38.sh >/workspace/stage.log 2>&1 </dev/null & disown'
#   ssh pod 'tail -5 /workspace/stage.log'
#
# Everything lands on /workspace (the network volume) so the pod stays disposable — terminate it and
# the next pod picks up where this one left off.
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive PATH=/usr/local/cuda/bin:$PATH

W=/workspace
M=$W/qwen38
REPO=bartowski/Qwen3.8-27B-GGUF
GGUF=Qwen3.8-27B-Q8_0.gguf          # 29.12 GB — the quant our pipeline supports end to end
MMPROJ=mmproj-Qwen3.8-27B-f16.gguf  # the vision tower
# Build fat: 80=A100, 86=RTX 3060, 89=L40/L40S, 90=H100. One build serves every chip we may rent.
CUDA_ARCHS="${CUDA_ARCHS:-80;86;89;90}"

say() { echo "[$(date -u +%H:%M:%S)] $*"; }
say "=== stage-qwen38 starting ==="
nvidia-smi --query-gpu=name,memory.total,compute_cap --format=csv,noheader || true
nvcc --version | tail -1 || true
df -h $W | tail -1

say "--- apt ---"
apt-get update -qq && apt-get install -y -qq aria2 git cmake build-essential libcurl4-openssl-dev >/dev/null 2>&1
mkdir -p $M

# ---- weights (aria2c: plain wget stalled on the 30B at ~32 GB last campaign) ---------------------
fetch() {  # fetch <filename> <expected-min-bytes>
  local f="$1" min="$2"
  if [ -f "$M/$f" ] && [ "$(stat -c%s "$M/$f")" -ge "$min" ]; then
    say "have $f ($(du -h "$M/$f" | cut -f1))"; return 0
  fi
  say "downloading $f ..."
  aria2c -x16 -s16 -k1M --file-allocation=none --summary-interval=30 --console-log-level=warn \
         -d "$M" -o "$f" "https://huggingface.co/$REPO/resolve/main/$f" || return 1
  say "got $f ($(du -h "$M/$f" | cut -f1))"
}

# Download and build concurrently — the download is network-bound, the build is CPU-bound.
( fetch "$GGUF" 29000000000 && fetch "$MMPROJ" 500000000 ; echo $? > $W/.fetch_rc ) &
FETCH_PID=$!

# ---- llama.cpp ----------------------------------------------------------------------------------
# MUST be recent: builds before ~b10450 miscompile Qwen3.8's Gated-DeltaNet layers on CUDA — the model
# loads, runs at full speed, and emits fluent garbage. A full (not shallow) clone so the version is real.
say "--- llama.cpp ---"
if [ ! -d $W/llama.cpp/.git ]; then
  git clone https://github.com/ggml-org/llama.cpp $W/llama.cpp
fi
cd $W/llama.cpp
git fetch --all --tags -q && git checkout -q master && git pull -q origin master
say "llama.cpp at $(git rev-parse --short HEAD) / $(git describe --tags --abbrev=0 2>/dev/null || echo no-tag)"
cmake -B build -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES="$CUDA_ARCHS" \
      -DLLAMA_CURL=OFF -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build build --config Release -j"$(nproc)" --target llama-bench llama-cli llama-simple 2>&1 | tail -3
say "llama.cpp built"

wait $FETCH_PID
[ "$(cat $W/.fetch_rc 2>/dev/null)" = "0" ] || { say "!! download FAILED"; exit 1; }

# ---- the fresh-.so trap: a rebuilt binary can still load a stale libggml-cuda.so ------------------
say "--- linkage ---"
ldd $W/llama.cpp/build/bin/llama-bench | grep -E 'ggml|llama' || true

# ---- coherence gate: no number is trusted until the model speaks English --------------------------
say "--- coherence gate ---"
OUT=$($W/llama.cpp/build/bin/llama-cli -m $M/$GGUF -ngl 99 -n 40 -st --temp 0 \
      -p 'The capital of France is' 2>/dev/null | tail -5)
echo "$OUT"
case "$OUT" in
  *Paris*) say "COHERENCE OK (says Paris)" ;;
  *) say "!! COHERENCE FAILED — llama.cpp is emitting garbage; do NOT trust any benchmark from this build" ;;
esac

# ---- baseline ------------------------------------------------------------------------------------
say "--- llama-bench tg128 baseline ---"
for FA in 1 0; do
  say "-fa $FA:"
  $W/llama.cpp/build/bin/llama-bench -m $M/$GGUF -ngl 99 -fa $FA -p 0 -n 128 -r 3 2>/dev/null | tail -4
done

say "=== SETUP COMPLETE ==="
