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
