#!/usr/bin/env bash
# Fetch the published `config.json` of each release the Samba book deploys into the gitignored
# external/hf-configs/<org>/<name>/config.json: the configurations `model_hf_intake_lock` reads without their weights
# (`HfCheckpoint::read_configuration`), each read or refused by name. Gated repositories are read through their public
# mirrors (unsloth). Idempotent: a configuration present is left alone. Writes external/hf-configs/SHA256SUMS.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${1:-$ROOT/external/hf-configs}"
mkdir -p "$DEST"

REPOS=(
    unsloth/Llama-3.2-1B-Instruct
    unsloth/Llama-3.2-3B-Instruct
    unsloth/Llama-3.1-8B-Instruct
    unsloth/Llama-3.3-70B-Instruct
    Qwen/Qwen2.5-0.5B-Instruct
    Qwen/Qwen2.5-72B-Instruct
    mistralai/Mixtral-8x7B-Instruct-v0.1
    openai/gpt-oss-20b
    openai/gpt-oss-120b
    deepseek-ai/DeepSeek-V3
    deepseek-ai/DeepSeek-V3-0324
    deepseek-ai/DeepSeek-R1
    unsloth/Llama-4-Scout-17B-16E-Instruct
    unsloth/Llama-4-Maverick-17B-128E-Instruct
    MiniMaxAI/MiniMax-M2
    unsloth/gemma-2b-it
    unsloth/gemma-7b-it
    unsloth/gemma-1.1-2b-it
    unsloth/gemma-1.1-7b-it
    unsloth/gemma-2-2b-it
    unsloth/gemma-2-9b-it
    unsloth/gemma-2-27b-it
    unsloth/gemma-3-1b-it
    unsloth/gemma-3-4b-it
    unsloth/gemma-3-12b-it
    unsloth/gemma-3-27b-it
    microsoft/phi-4
    zai-org/GLM-4-9B-0414
    zai-org/GLM-4-32B-0414
    zai-org/GLM-4.5
    zai-org/GLM-4.5-Air
    zai-org/GLM-4.6
    zai-org/GLM-4.7
    moonshotai/Kimi-K2-Instruct
    unsloth/Kimi-K2-Instruct
    moonshotai/Kimi-K2-Instruct-0905
    moonshotai/Kimi-K2-Thinking
    Qwen/Qwen3-8B
    Qwen/Qwen3-30B-A3B
    Qwen/Qwen3-235B-A22B
    Qwen/Qwen3-Next-80B-A3B-Instruct
    Qwen/Qwen3.5-0.8B
    Qwen/Qwen3.5-2B
    Qwen/Qwen3.5-4B
    Qwen/Qwen3.5-9B
    Qwen/Qwen3.5-27B
    Qwen/Qwen3.5-35B-A3B
    Qwen/Qwen3.5-122B-A10B
    Qwen/Qwen3.5-397B-A17B
    Qwen/Qwen3.8-27B
    Qwen/Qwen3.8-2.4T-A95B
    Qwen/Qwen3.8-Flash-Next
)

for repo in "${REPOS[@]}"; do
    mkdir -p "$DEST/$repo"
    if [ -s "$DEST/$repo/config.json" ]; then
        continue
    fi
    echo "  fetching $repo/config.json"
    curl -sSfL --max-time 120 -o "$DEST/$repo/config.json.part" "https://huggingface.co/$repo/resolve/main/config.json"
    mv "$DEST/$repo/config.json.part" "$DEST/$repo/config.json"
done

(cd "$DEST" && find . -name config.json | sort | xargs sha256sum > SHA256SUMS)
echo "configurations → $DEST"
