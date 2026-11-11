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
    allenai/OLMo-2-0425-1B
    allenai/OLMo-2-1124-7B
    allenai/OLMo-2-1124-13B
    allenai/OLMo-2-0325-32B
    allenai/Olmo-3-7B-Instruct
    mistralai/Mistral-7B-Instruct-v0.3
    mistralai/Ministral-3-3B-Instruct-2512
    mistralai/Ministral-3-8B-Instruct-2512
    mistralai/Ministral-3-14B-Instruct-2512
    mistralai/Mistral-Small-3.1-24B-Instruct-2503
    mistralai/Mistral-Small-3.2-24B-Instruct-2506
    deepseek-ai/DeepSeek-V3.2-Exp
    deepseek-ai/DeepSeek-V3.2
    deepseek-ai/DeepSeek-V3.2-Speciale
    zai-org/GLM-5
    zai-org/GLM-5.1
    zai-org/GLM-5.2
    LiquidAI/LFM2-350M
    LiquidAI/LFM2-700M
    LiquidAI/LFM2-1.2B
    LiquidAI/LFM2-2.6B
    LiquidAI/LFM2.5-1.2B-Instruct
    mistralai/Mamba-Codestral-7B-v0.1
    tiiuae/Falcon-H1-0.5B-Instruct
    tiiuae/Falcon-H1-1.5B-Instruct
    tiiuae/Falcon-H1-1.5B-Deep-Instruct
    tiiuae/Falcon-H1-3B-Instruct
    tiiuae/Falcon-H1-7B-Instruct
    tiiuae/Falcon-H1-34B-Instruct
    nvidia/Nemotron-H-8B-Base-8K
    nvidia/Nemotron-H-47B-Base-8K
    nvidia/Nemotron-H-56B-Base-8K
    nvidia/NVIDIA-Nemotron-Nano-9B-v2
    nvidia/NVIDIA-Nemotron-Nano-12B-v2
    ai21labs/AI21-Jamba-Reasoning-3B
    ai21labs/Jamba-v0.1
    unsloth/gemma-3n-E2B-it
    unsloth/gemma-3n-E4B-it
    moonshotai/Kimi-Linear-48B-A3B-Instruct
    moonshotai/Kimi-Linear-48B-A3B-Base
    MiniMaxAI/MiniMax-Text-01
    MiniMaxAI/MiniMax-Text-01-hf
    MiniMaxAI/MiniMax-M1-40k
    MiniMaxAI/MiniMax-M1-80k
    fla-hub/rwkv7-0.1B-g1
    fla-hub/rwkv7-191M-world
    fla-hub/rwkv7-1.5B-world
    fla-hub/rwkv7-2.9B-world
    SmerkyG/RWKV7-Goose-0.1B-World2.8-HF
    mollysama/rwkv-7-world-0b4-hf
    mollysama/rwkv-7-world-1b5-hf
    google/gemma-4-E2B-it
    google/gemma-4-E4B-it
    google/gemma-4-26B-A4B-it
    google/gemma-4-31B-it
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
