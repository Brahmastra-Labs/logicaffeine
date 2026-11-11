#!/usr/bin/env bash
# Fetch each model family's published tokenizer files into the gitignored external/hf-tokenizers/<family>/, the
# tokenizers the Hugging Face intake's tiny twins (scripts/accel/hf-tiny-twin.py) are built with: a twin keeps its
# family's real vocabulary so llama.cpp's converter recognizes the tokenizer and writes the GGUF the intake is held to.
# Idempotent: a family whose files are present is left alone. Writes external/hf-tokenizers/SHA256SUMS.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${1:-$ROOT/external/hf-tokenizers}"
mkdir -p "$DEST"

# family  repository  files
FAMILIES=(
    "llama unsloth/Llama-3.2-1B-Instruct tokenizer.json tokenizer_config.json"
    "qwen2 Qwen/Qwen2.5-0.5B-Instruct tokenizer.json tokenizer_config.json vocab.json merges.txt"
    "qwen3 Qwen/Qwen3-0.6B tokenizer.json tokenizer_config.json vocab.json merges.txt"
    "qwen35 Qwen/Qwen3.5-0.8B tokenizer.json tokenizer_config.json vocab.json merges.txt"
    "gemma3 unsloth/gemma-3-1b-it tokenizer.json tokenizer_config.json tokenizer.model"
    "gpt-oss openai/gpt-oss-20b tokenizer.json tokenizer_config.json"
    "deepseek2 deepseek-ai/DeepSeek-V3 tokenizer.json tokenizer_config.json"
    "llama4 unsloth/Llama-4-Scout-17B-16E-Instruct tokenizer.json tokenizer_config.json"
    "mixtral mistralai/Mixtral-8x7B-Instruct-v0.1 tokenizer.json tokenizer_config.json tokenizer.model"
    "minimax MiniMaxAI/MiniMax-M2 tokenizer.json tokenizer_config.json"
    "phi4 microsoft/phi-4 tokenizer.json tokenizer_config.json vocab.json merges.txt"
    "glm4 zai-org/GLM-4-9B-0414 tokenizer.json tokenizer_config.json"
    "glm45 zai-org/GLM-4.5 tokenizer.json tokenizer_config.json"
    "kimi_k2 moonshotai/Kimi-K2-Instruct tiktoken.model tokenizer_config.json tokenization_kimi.py"
    "kimi_k2_unsloth unsloth/Kimi-K2-Instruct tiktoken.model tokenizer_config.json tokenization_kimi.py special_tokens_map.json chat_template.jinja"
    "ministral3 mistralai/Ministral-3-3B-Instruct-2512 tokenizer.json tokenizer_config.json tekken.json"
    "phimoe microsoft/Phi-3.5-MoE-instruct tokenizer.json tokenizer_config.json tokenizer.model added_tokens.json special_tokens_map.json"
    "deepseek32 deepseek-ai/DeepSeek-V3.2 tokenizer.json tokenizer_config.json"
    "glm5 zai-org/GLM-5 tokenizer.json tokenizer_config.json"
    "lfm2 LiquidAI/LFM2-1.2B tokenizer.json tokenizer_config.json"
    "mamba2 EleutherAI/gpt-neox-20b tokenizer.json tokenizer_config.json"
    "falcon-h1 tiiuae/Falcon-H1-0.5B-Instruct tokenizer.json tokenizer_config.json"
    "nemotron_h nvidia/NVIDIA-Nemotron-Nano-9B-v2 tokenizer.json tokenizer_config.json special_tokens_map.json"
    "jamba ai21labs/AI21-Jamba-Reasoning-3B tokenizer.json tokenizer_config.json special_tokens_map.json"
    "gemma3n unsloth/gemma-3n-E2B-it tokenizer.model tokenizer.json tokenizer_config.json special_tokens_map.json"
    "kimi-linear moonshotai/Kimi-Linear-48B-A3B-Instruct tiktoken.model tokenizer_config.json tokenization_kimi.py special_tokens_map.json"
    "minimax-01 MiniMaxAI/MiniMax-Text-01 tokenizer.json tokenizer_config.json"
    "gemma4 google/gemma-4-E2B-it tokenizer.json tokenizer_config.json chat_template.jinja"
    "rwkv7 fla-hub/rwkv7-1.5B-world rwkv_vocab_v20230424.txt tokenizer_config.json added_tokens.json special_tokens_map.json hf_rwkv_tokenizer.py"
)

for entry in "${FAMILIES[@]}"; do
    read -r family repo files <<<"$entry"
    mkdir -p "$DEST/$family"
    for f in $files; do
        if [ -s "$DEST/$family/$f" ]; then
            continue
        fi
        echo "  fetching $repo/$f"
        curl -sSfL --max-time 300 -o "$DEST/$family/$f.part" "https://huggingface.co/$repo/resolve/main/$f"
        mv "$DEST/$family/$f.part" "$DEST/$family/$f"
    done
    echo "$repo" > "$DEST/$family/SOURCE"
done

(cd "$DEST" && find . -type f ! -name SHA256SUMS | sort | xargs sha256sum > SHA256SUMS)
echo "tokenizers → $DEST"
