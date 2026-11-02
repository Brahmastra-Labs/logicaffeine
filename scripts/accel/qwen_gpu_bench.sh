#!/usr/bin/env bash
# Head-to-head Qwen3-8B Q8_0 generation throughput on the local GPU: OUR from-scratch CUDA engine vs
# llama.cpp (CUDA). Both run the same checkpoint on the same device — an honest apples-to-apples number.
#
#   bash scripts/accel/qwen_gpu_bench.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
MODEL=models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf
[ -f "$MODEL" ] || { echo "checkpoint missing: $MODEL"; exit 1; }

echo "== OURS: fused Q8_0 GEMV + KV-cache (crates/logicaffeine_compile/cuda/qwen_forward.cu) =="
nvcc -O3 -arch=sm_86 crates/logicaffeine_compile/cuda/qwen_forward.cu -lcublas -o /tmp/qwen_forward
/tmp/qwen_forward "$MODEL" 785 6722 315 9625 374 2>&1 | grep -E "GEN|generated"

echo
echo "== llama.cpp (CUDA, all layers on GPU) =="
python3 - <<'PY' 2>&1 | grep -iE "eval time|per second" | grep -v "prompt eval"
from llama_cpp import Llama
llm = Llama(model_path="models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf", n_ctx=256, n_gpu_layers=-1, verbose=True)
_ = llm.create_completion("The capital of France is", max_tokens=64, temperature=0.0)
PY
