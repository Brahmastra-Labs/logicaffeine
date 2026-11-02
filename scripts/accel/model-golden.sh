#!/bin/bash
# The llama.cpp golden for a checkpoint, by the pinned llama.cpp on the CPU alone (scripts/accel/model_golden.cpp): no
# GPU is visible to it and no operation is offloaded.
#
#   scripts/accel/model-golden.sh <gguf> <out.json> [n_gen=16] [threads=16] [flash=0] [kv=f16] [prompts=benchmarks/models/prompts.txt] [batch=0] [device=cpu] [top=5]
#
# device=cuda runs it instead as llama-bench runs the checkpoint (every layer on the GPU, operations offloaded, its
# batches of 2048 tokens in micro-batches of 512): llama.cpp's
# own GPU run, for the scoreboard's quality column; only then is the GPU visible to it.
#
# batch=1 runs every prompt as its own sequence of one context and decodes them together, the weights streaming once a
# step for the corpus: the run for an f32 copy too large for memory (any other checkpoint is refused). The JSON then
# records "batch".
#
# Builds the golden program against external/competitors/llama.cpp (built by external/competitors/build-llama.sh)
# once per source change, then runs it. The JSON names the llama.cpp commit from external/competitors/PINS.tsv.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
llama="$root/external/competitors/llama.cpp"
[ -f "$llama/build/bin/libllama.so" ] || { echo "llama.cpp is not built: run external/competitors/build-llama.sh" >&2; exit 1; }
gguf="$1"; out="$2"; n_gen="${3:-16}"; threads="${4:-16}"; flash="${5:-0}"; kv="${6:-f16}"
prompts="${7:-$root/benchmarks/models/prompts.txt}"; batch="${8:-0}"; device="${9:-cpu}"; top="${10:-5}"
src="$root/scripts/accel/model_golden.cpp"
bin="$llama/build/model_golden-$(sha256sum "$src" | cut -c1-16)"
if [ ! -x "$bin" ]; then
  g++ -O2 -std=c++17 -I"$llama/include" -I"$llama/ggml/include" "$src" -L"$llama/build/bin" -lllama -lggml \
    -lggml-base "-Wl,-rpath,$llama/build/bin" -o "$bin.tmp.$$"
  mv "$bin.tmp.$$" "$bin"
fi
commit="$(awk -F'\t' '$1 == "llama.cpp" { print $4 }' "$root/external/competitors/PINS.tsv")"
mkdir -p "$(dirname "$out")"
if [ "$device" = cuda ]; then
  LLAMA_COMMIT="$commit" "$bin" "$gguf" "$prompts" "$out" "$n_gen" "$threads" "$flash" "$top" "$kv" "$batch" cuda
else
  CUDA_VISIBLE_DEVICES= LLAMA_COMMIT="$commit" "$bin" "$gguf" "$prompts" "$out" "$n_gen" "$threads" "$flash" "$top" "$kv" "$batch"
fi
echo "wrote $out"
