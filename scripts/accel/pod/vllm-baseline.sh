#!/usr/bin/env bash
# Phase 1 of the vLLM head-to-head: ONE rented H100, both engines, same boot. This session writes no
# kernels — it establishes the real gap, because every later ranking depends on it and the house rule is
# ablate before optimising.
#
#   ssh pod 'setsid bash /workspace/vllm-baseline.sh >/workspace/vllm-baseline.log 2>&1 </dev/null & disown'
#
# ── WHERE THE WEIGHTS LIVE ───────────────────────────────────────────────────────────────────────
# $W = /workspace IS the RunPod NETWORK VOLUME (rvq94a2gft, 250 GB, US-KS-2). Everything downloaded
# there SURVIVES pod termination, so the 54 GB bf16 checkpoint is fetched ONCE, ever. Pods are
# disposable; this directory is not. The run then STAGES a copy onto the pod's local NVMe, because the
# volume is MooseFS and has served anywhere from 50 MB/s to 1.8 MB/s on different days — vLLM restarts
# its server four times in this sweep and must not pay a slow 54 GB read each time.
#
# Resumable: each stage drops a marker in $OUT/.done-<stage>, so a failed stage can be re-run without
# paying for the ones that already succeeded. Run one stage alone with STAGES="vllm_sweep".
#
# Facts pinned from vLLM main @ 674b6d95d6d0 (2026-09-19), read before renting anything:
#   * Qwen3_5ForConditionalGeneration -> qwen3_5           (models/registry.py:596)  -- fully supported
#   * Qwen3_5MTP -> qwen3_5_mtp, a valid speculative method (config/speculative.py:51)
#   * headline metric = output_throughput = sum(output_lens)/dur_s  (benchmarks/serve.py:750)
set -uo pipefail
W=/workspace                                   # the NETWORK VOLUME — persists across pods
OUT=${OUT:-$W/vllm_baseline}
HF_MODEL=${HF_MODEL:-Qwen/Qwen3.8-27B}
HF_DIR=${HF_DIR:-$W/qwen38/hf}                 # permanent home of the bf16 checkpoint (~54 GB)
LOCAL_HF=${LOCAL_HF:-/root/hf}                 # fast staged copy on the pod's own NVMe
GGUF=${GGUF:-$W/qwen38/Qwen3.8-27B-Q8_0.gguf}  # ours, already on the volume from the llama.cpp campaign
SM=${SM:-sm_90}
MAXLEN=${MAXLEN:-4096}          # NOT the model's 262144: that would size the KV/state pool for nothing
IN_LEN=${IN_LEN:-512}
OUT_LEN=${OUT_LEN:-128}
CONCURRENCIES=${CONCURRENCIES:-"1 8 32 64"}
STAGES=${STAGES:-"env install download stage verify vllm_sweep ours accuracy report"}
export PATH=/usr/local/cuda/bin:$PATH
export HF_HOME=${HF_HOME:-$W/hf_home}          # HF's own cache also lands on the volume, not the pod
mkdir -p "$OUT" "$(dirname "$HF_DIR")"
source "$(dirname "$0")/_stage.sh"

MODEL_PATH=""   # resolved by s_stage: the local staged copy when it exists, else the volume copy

# ── THE TOOLKIT vLLM NEEDS, WHICH IS NOT THE ONE WE USE ─────────────────────────────────────────
# FlashInfer 0.6.18 JIT-compiles its sampling kernels with `--compress-mode=size`, which nvcc only
# understands from CUDA 12.8 on. This image ships nvcc 12.4 under /usr/local/cuda while its torch is
# built against cu130, so the JIT dies with `nvcc fatal: Unknown option '--compress-mode=size'` and
# vLLM reports it as the far less helpful "Engine core initialization failed".
# torch brings a complete CUDA 13.4 toolkit with it; hand that one to vLLM.
# OUR lanes deliberately keep /usr/local/cuda (12.4): that is the compiler every recorded Qwen3.8
# number was taken with, and swapping it mid-comparison would change our side for a reason unrelated
# to the comparison.
CU13=${CU13:-/usr/local/lib/python3.11/dist-packages/nvidia/cu13}
# Setting CUDA_HOME is NOT enough: FlashInfer's JIT hardcodes `-isystem /usr/local/cuda/include`, so a
# cu13 compiler still picked up 12.4 headers and failed with
#   "CUDA compiler and CUDA toolkit headers are incompatible".
# The cu13 wheel is a COMPLETE toolkit (bin + include + cccl + nvvm), so repoint the symlink and give
# vLLM one consistent CUDA 13 toolchain rather than handicapping it by disabling its sampler.
# OUR lanes then name nvcc 12.4 explicitly -- the compiler every recorded Qwen3.8 number was taken with.
NVCC=${NVCC:-/usr/local/cuda-12.4/bin/nvcc}
use_cuda13_for_vllm() {
  [ -x "$CU13/bin/nvcc" ] || { say "no CUDA 13 toolkit at $CU13"; return 1; }
  if [ -L /usr/local/cuda ] && [ "$(readlink -f /usr/local/cuda)" != "$(readlink -f "$CU13")" ]; then
    ln -sfn "$CU13" /usr/local/cuda
    say "/usr/local/cuda -> $CU13 (for vLLM's JIT); our lanes use $NVCC"
  fi
}
# FlashInfer's JIT of its SAMPLING kernels could not be made to build in this image under either
# toolkit (12.4 nvcc rejects `--compress-mode=size`; the cu13 toolkit then collides with FlashInfer's
# own bundled cccl headers). `VLLM_USE_FLASHINFER_SAMPLER=0` is vLLM's own supported fallback to its
# native PyTorch sampler and is the honest thing to do rather than burn rented GPU on a JIT yak-shave:
# it touches ONLY the sampling step -- an argmax over 248320 logits under greedy decoding -- and none
# of the attention, GDN or GEMM kernels the comparison is actually about. Recorded in the results so
# the config is never quoted without it. Everything else still gets the CUDA 13 toolchain.
# `vllm serve` spawns an EngineCore CHILD that survives killing the parent -- it was found holding
# 87.5 GB of 95.8 GB after the sweep, so our lanes died with "CUDA ERROR after weight load: invalid
# argument". Whichever engine runs second must start on an empty GPU or the comparison is garbage.
# Kill by PID from nvidia-smi, NEVER by `pkill -f <pattern>`: the pattern matches the very ssh command
# running the check and takes out the session (recorded trap, hit again this session).
free_gpu() {
  for _ in $(seq 1 12); do
    local pids; pids=$(nvidia-smi --query-compute-apps=pid --format=csv,noheader 2>/dev/null)
    [ -z "$pids" ] && break
    for q in $pids; do kill -TERM "$q" 2>/dev/null; done
    sleep 5
  done
  for q in $(nvidia-smi --query-compute-apps=pid --format=csv,noheader 2>/dev/null); do kill -KILL "$q" 2>/dev/null; done
  sleep 5
  local used; used=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null)
  say "gpu free: ${used:-?} MiB still in use"
  [ "${used:-0}" -lt 2000 ] 2>/dev/null || say "!! GPU still holds ${used} MiB -- the next engine will not fit"
}

vllm_run() {
  if [ -x "$CU13/bin/nvcc" ]; then
    VLLM_USE_FLASHINFER_SAMPLER=0 CUDA_HOME="$CU13" CUDA_PATH="$CU13" PATH="$CU13/bin:$PATH" "$@"
  else
    VLLM_USE_FLASHINFER_SAMPLER=0 "$@"
  fi
}

have_stage() { [[ " $STAGES " == *" $1 "* ]]; }
done_marker() { echo "$OUT/.done-$1"; }
stage() {                                   # stage <name> <fn>
  local n="$1" fn="$2"
  have_stage "$n" || { say "skip $n (not in STAGES)"; return 0; }
  [ -f "$(done_marker "$n")" ] && { say "skip $n (already done)"; return 0; }
  say "===== stage: $n ====="
  if "$fn"; then touch "$(done_marker "$n")"; say "===== $n OK ====="
  else say "!!!!! $n FAILED — stopping so the rest of the rented hour is not spent on a broken base"; return 1; fi
}

s_env() {
  nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv | tee "$OUT/gpu.txt"
  { nvcc --version | tail -2; df -h / "$W"; free -g | head -2; } 2>&1 | tee "$OUT/env.txt"
}

s_install() {
  python3 -c "import vllm" 2>/dev/null || pip install -q vllm 2>&1 | tail -5
  python3 -c "import vllm, torch; print('vllm', vllm.__version__); print('torch', torch.__version__)" \
    | tee "$OUT/versions.txt" || return 1
  # Which GDN prefill backends this install actually has. vLLM picks among Triton/FLA, FlashInfer and
  # CuTeDSL (layers/mamba/gdn/qwen_gdn_linear_attn.py); "their best config" means whichever wins here.
  python3 - <<'PY' | tee "$OUT/gdn_backends.txt"
for name, mod in (("fla", "fla"), ("flashinfer", "flashinfer"), ("causal_conv1d", "causal_conv1d")):
    try:
        __import__(mod); print(f"{name}: available")
    except Exception as e:
        print(f"{name}: MISSING ({type(e).__name__})")
PY
  # Assert THIS release registers our architecture BEFORE anything downloads 54 GB. main@674b6d95d6d0
  # has it (registry.py:596); an older pinned release may not, and discovering that after the download
  # wastes an hour of a rented H100.
  python3 - <<'ARCH' | tee "$OUT/arch_support.txt" || return 1
import sys
from vllm.model_executor.models.registry import ModelRegistry
have = set(ModelRegistry.get_supported_archs())
for a in ("Qwen3_5ForConditionalGeneration", "Qwen3_5ForCausalLM", "Qwen3_5MTP"):
    print(f"{a}: {'YES' if a in have else 'NO'}")
if not ({"Qwen3_5ForConditionalGeneration", "Qwen3_5ForCausalLM"} & have):
    print("FATAL: this vLLM build cannot serve Qwen3.8-27B"); sys.exit(1)
ARCH
}

# Fetch to the NETWORK VOLUME so this is paid once across the whole campaign, not once per pod.
s_download() {
  if [ -f "$HF_DIR/config.json" ]; then
    say "bf16 checkpoint already on the volume: $(du -sh "$HF_DIR" | cut -f1)"; return 0
  fi
  command -v hf >/dev/null 2>&1 || pip install -q "huggingface_hub[cli]" 2>&1 | tail -2
  say "downloading $HF_MODEL (~54 GB) -> $HF_DIR   [network volume: survives pod termination]"
  hf download "$HF_MODEL" --local-dir "$HF_DIR" 2>&1 | tail -8 || return 1
  [ -f "$HF_DIR/config.json" ] || { say "!! download finished but no config.json — incomplete"; return 1; }
  du -sh "$HF_DIR" | tee -a "$OUT/env.txt"
}

# Gate the volume, then put a copy on local NVMe so the four server starts do not each re-read 54 GB
# across MooseFS. Whichever copy we end up using becomes MODEL_PATH for every later stage.
s_stage() {
  [ -f "$GGUF" ] && vol_gate "$GGUF"
  local need free
  need=$(du -sB1 "$HF_DIR" 2>/dev/null | cut -f1); need=$(( ${need:-0} / 1000000000 + 10 ))
  free=$(df -BG --output=avail / | tail -1 | tr -dc '0-9')
  if [ "${free:-0}" -gt "$need" ]; then
    stage_local "$HF_DIR" "$LOCAL_HF" "$need"
  else
    say "local disk has ${free:-?} GB free, need >${need} — running vLLM straight off the volume"
  fi
  return 0
}

resolve_model() {
  if [ -f "$LOCAL_HF/config.json" ]; then MODEL_PATH="$LOCAL_HF"; else MODEL_PATH="$HF_DIR"; fi
  say "model path: $MODEL_PATH"
  [ -f "$MODEL_PATH/config.json" ] || { say "!! no config.json at $MODEL_PATH"; return 1; }
  # A local PATH, never the hub id: passing the repo id would send vLLM back to the network to
  # re-download 54 GB it is already sitting on.
  warm_files "$MODEL_PATH"
}

# Does it load at all, and does it say something coherent? A fast engine that emits garbage is the trap
# llama.cpp's pre-b10450 GDN CUDA path set; never trust a throughput number from an unchecked model.
s_verify() {
  resolve_model || return 1
  use_cuda13_for_vllm
  rm -rf /root/.cache/flashinfer 2>/dev/null
  # vLLM spawns its EngineCore in a child process, which re-opens the parent's script by path. A python
  # heredoc piped on stdin has no path, so the child dies with
  #   FileNotFoundError: '/root/<stdin>'
  # and the failure surfaces as the misleading "Engine core initialization failed". Write a real file.
  # ... and it must sit behind a __main__ guard: the spawned worker re-imports this module, so any
  # top-level LLM() construction recurses into "start a new process before bootstrapping finished".
  cat > "$OUT/verify.py" <<'VPY'
import sys, time
from vllm import LLM, SamplingParams

def main():
    q, d, ml = sys.argv[1], sys.argv[2], int(sys.argv[3])
    import os as _os
    kw = dict(model=d, max_model_len=ml,
              gpu_memory_utilization=float(_os.environ.get("VLLM_GPU_UTIL", "0.92")),
              max_num_seqs=int(_os.environ.get("VLLM_MAX_SEQS", "128")),
              enforce_eager=False)
    if q != "none":
        kw["quantization"] = q
    t0 = time.time()
    llm = LLM(**kw)
    print("LOAD_SECONDS %.1f" % (time.time() - t0))
    o = llm.generate(["The capital of France is"], SamplingParams(temperature=0, max_tokens=24))
    print("COHERENCE_TEXT", repr(o[0].outputs[0].text))
    print("COHERENCE_IDS", list(o[0].outputs[0].token_ids))

if __name__ == "__main__":
    main()
VPY
  local any=1
  for Q in ${QUANTS:-fp8 none}; do
    say "--- load check: quantization=$Q ---"
    vllm_run timeout 2400 python3 "$OUT/verify.py" "$Q" "$MODEL_PATH" "$MAXLEN" 2>&1 | tee "$OUT/verify_$Q.txt"
    if grep -q COHERENCE_TEXT "$OUT/verify_$Q.txt"; then say "$Q loads and generates"; any=0
    else say "$Q did NOT produce output"; fi
  done
  return $any
}

# vLLM's own server + its own benchmark, so the metrics are theirs by construction rather than by our
# re-implementation. Both quantizations, with and without MTP speculation, across concurrency.
s_vllm_sweep() {
  resolve_model || return 1
  use_cuda13_for_vllm
  for Q in ${QUANTS:-fp8 none}; do
    grep -q COHERENCE_TEXT "$OUT/verify_$Q.txt" 2>/dev/null || { say "skip sweep for $Q (did not verify)"; continue; }
    for SPEC in ${SPECS:-off mtp}; do
      local args=(--model "$MODEL_PATH" --served-model-name qwen38 --max-model-len "$MAXLEN"
                  --gpu-memory-utilization "${VLLM_GPU_UTIL:-0.92}"
                  --max-num-seqs "${VLLM_MAX_SEQS:-128}" --port 8000)
      [ "$Q" != none ] && args+=(--quantization "$Q")
      [ "$SPEC" = mtp ] && args+=(--speculative-config '{"method":"qwen3_5_mtp","num_speculative_tokens":2}')
      say "--- vllm serve: quant=$Q spec=$SPEC ---"
      vllm_run vllm serve "${args[@]}" > "$OUT/serve_${Q}_${SPEC}.log" 2>&1 &
      local pid=$! ok=0
      for _ in $(seq 1 180); do
        curl -sf localhost:8000/health >/dev/null 2>&1 && { ok=1; break; }
        kill -0 $pid 2>/dev/null || break
        sleep 10
      done
      if [ "$ok" != 1 ]; then
        say "!! server did not become ready for quant=$Q spec=$SPEC"; tail -25 "$OUT/serve_${Q}_${SPEC}.log"
        kill $pid 2>/dev/null; wait $pid 2>/dev/null; continue
      fi
      for C in $CONCURRENCIES; do
        say "  bench serve: quant=$Q spec=$SPEC concurrency=$C"
        vllm_run vllm bench serve --backend openai --model qwen38 --tokenizer "$MODEL_PATH" \
          --dataset-name random --random-input-len "$IN_LEN" --random-output-len "$OUT_LEN" \
          --num-prompts $((C * 8)) --max-concurrency "$C" --ignore-eos \
          --percentile-metrics ttft,tpot,itl,e2el \
          --save-result --result-filename "$OUT/vllm_${Q}_${SPEC}_c${C}.json" \
          > "$OUT/bench_${Q}_${SPEC}_c${C}.log" 2>&1 \
          || say "  (bench failed: see $OUT/bench_${Q}_${SPEC}_c${C}.log)"
        grep -aE "Output token throughput|Median TTFT|Median ITL|Median TPOT" "$OUT/bench_${Q}_${SPEC}_c${C}.log" | sed 's/^/    /'
      done
      kill $pid 2>/dev/null; wait $pid 2>/dev/null
      free_gpu
    done
  done
}

# Ours, in the SAME boot, on the SAME chip. Today this is the static-B lane; its TTFT is expected to be
# poor because prefill replays the T=1 decode graph per prompt token (qwen38.rs:513) -- measuring that
# deficit is a deliverable of this session, not a surprise to be discovered later.
s_ours() {
  free_gpu
  # Our extracted Q8 weight set must be local too, for the same reason vLLM's is: whichever side reads
  # cold pays a penalty the other does not, which is exactly the defect that made the 2026-09-09
  # llama.cpp numbers unfair and forced the 09-10 re-measure.
  [ -d /workspace/w38 ] && stage_local /workspace/w38 /root/w38 35
  # The w512_* lanes carry a 512-token prompt and 128 generated tokens -- the SAME shape vLLM is being
  # benchmarked at (--random-input-len 512 --random-output-len 128). The older b*.cu lanes on this
  # volume use the 5-token golden prompt and are NOT comparable to that sweep.
  for LANE in ${LANES:-w512_b1_hl w512_b1_fast w512_b8_hl w512_b8_fast w512_b32_hl w512_b32_fast w512_b64_hl w512_b64_fast}; do
    [ -f "$W/$LANE.cu" ] || { say "no $W/$LANE.cu on this pod"; continue; }
    say "--- ours: $LANE ---"
    "$NVCC" -O3 -arch=$SM "$W/$LANE.cu" -o "$W/$LANE" 2> "$OUT/$LANE.nvcc.log" \
      || { say "NVCC FAILED $LANE"; tail -20 "$OUT/$LANE.nvcc.log"; continue; }
    "$W/$LANE" > "$OUT/ours_$LANE.out" 2> "$OUT/ours_$LANE.err"
    grep -aE "PREFILL_MS|TTFT_MS|GEN_TOK_S_TG128|AGGREGATE_TOK_S|BATCH_AGREEMENT" \
      "$OUT/ours_$LANE.out" "$OUT/ours_$LANE.err" | sed 's/^/  /'
  done
  return 0
}

# ★ THE ACCURACY COLUMN — without it, "faster" is not a claim, it is a trade nobody priced.
#
# The single-pass lane keeps 8 activation bits with a per-32-block scale; vLLM's fp8 e4m3 keeps 4
# significant bits. That SHOULD favour us, but it is measured, never assumed: both engines greedily
# continue the same prompt and are scored against the f64 CPU oracle's golden stream. An engine that
# is faster and diverges earlier has not won anything.
s_accuracy() {
  resolve_model || return 1
  free_gpu
  cat > "$OUT/vllm_greedy.py" <<'VPY'
import json, sys
from vllm import LLM, SamplingParams

def main():
    path, ml, n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    import os as _os
    llm = LLM(model=path, quantization="fp8", max_model_len=ml,
              gpu_memory_utilization=float(_os.environ.get("VLLM_GPU_UTIL", "0.92")),
              max_num_seqs=int(_os.environ.get("VLLM_MAX_SEQS", "128")))
    o = llm.generate(["The capital of France is"],
                     SamplingParams(temperature=0, max_tokens=n, logprobs=1))
    out = o[0].outputs[0]
    print("VLLM_TOKENS " + json.dumps(list(out.token_ids)))
    print("VLLM_TEXT " + json.dumps(out.text))

if __name__ == "__main__":
    main()
VPY
  vllm_run timeout 2400 python3 "$OUT/vllm_greedy.py" "$MODEL_PATH" "$MAXLEN" 24 2>&1 \
    | tee "$OUT/vllm_greedy.txt" | grep -a "VLLM_TOKENS\|VLLM_TEXT"
  # ours, same prompt, both lanes -- the golden stream lives on the volume from the llama.cpp campaign
  [ -f "$W/qwen38_golden_tokens.txt" ] && say "golden: $(cat "$W/qwen38_golden_tokens.txt")"
  for LANE in ${ACC_LANES:-w512_b1_hl w512_b1_fast}; do
    [ -f "$OUT/ours_$LANE.out" ] && grep -a "SEQ0_TOKENS\|GENERATED_TOKENS" "$OUT/ours_$LANE.out" | sed "s/^/  $LANE: /"
  done
  return 0
}

s_report() {
  python3 - "$OUT" <<'PY' | tee "$OUT/summary.json"
import glob, json, os, sys
out = sys.argv[1]
rows = []
for f in sorted(glob.glob(os.path.join(out, "vllm_*_c*.json"))):
    try: d = json.load(open(f))
    except Exception: continue
    if isinstance(d, list): d = d[0] if d else {}
    base = os.path.basename(f)[len("vllm_"):-len(".json")]
    quant, spec, conc = base.rsplit("_", 2)
    rows.append({"engine": "vllm", "quant": quant, "spec": spec, "concurrency": int(conc[1:]),
                 "output_throughput": d.get("output_throughput"),
                 "median_ttft_ms": d.get("median_ttft_ms"), "median_itl_ms": d.get("median_itl_ms"),
                 "median_tpot_ms": d.get("median_tpot_ms"), "request_throughput": d.get("request_throughput")})
rows.sort(key=lambda r: (r["quant"], r["spec"], r["concurrency"]))
best = max((r for r in rows if r["output_throughput"]), key=lambda r: r["output_throughput"], default=None)
print(json.dumps({"_what": "Phase 1 baseline: vLLM's numbers, its own benchmark, its own metrics.",
                  "vllm_rows": rows, "vllm_best": best,
                  "note": "ours is in ours_*.out/.err in the same directory; the two are merged into "
                          "benchmarks/results/qwen38_27b.json only after both are sanity-checked."}, indent=1))
PY
}

GPU=$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null || echo "no-gpu")
say "=== vLLM head-to-head baseline on $GPU — out=$OUT ==="
stage env        s_env        || exit 1
stage install    s_install    || exit 1
stage download   s_download   || exit 1
stage stage      s_stage      || exit 1
stage verify     s_verify     || exit 1
stage vllm_sweep s_vllm_sweep || exit 1
stage ours       s_ours       || exit 1
stage accuracy   s_accuracy   || true
stage report     s_report     || exit 1
say "=== BASELINE DONE — copy $OUT off the pod, then TERMINATE it ==="
