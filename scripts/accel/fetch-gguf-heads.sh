#!/usr/bin/env bash
# Fetch the head of a published GGUF (its first 16 MiB: the header, the metadata and the tensor directory, never the
# tensors) into the gitignored external/gguf-heads/<org>/<name>/<file>: what llama.cpp's converter wrote from a
# checkpoint whose own code the intake does not run (Kimi K2's tokenizer), read by `model_hf_intake_lock` as the oracle
# of that checkpoint's vocabulary. Idempotent: a head present is left alone. Writes external/gguf-heads/SHA256SUMS.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${1:-$ROOT/external/gguf-heads}"
mkdir -p "$DEST"

# repository  file
HEADS=(
    "unsloth/Kimi-K2-Instruct-GGUF BF16/Kimi-K2-Instruct-BF16-00001-of-00045.gguf"
)

for entry in "${HEADS[@]}"; do
    read -r repo file <<<"$entry"
    out="$DEST/$repo/$(basename "$file")"
    mkdir -p "$(dirname "$out")"
    if [ -s "$out" ]; then
        continue
    fi
    echo "  fetching the head of $repo/$file"
    curl -sSfL --max-time 300 -r 0-16777215 -o "$out.part" "https://huggingface.co/$repo/resolve/main/$file"
    mv "$out.part" "$out"
done

(cd "$DEST" && find . -name '*.gguf' | sort | xargs sha256sum > SHA256SUMS)
echo "heads → $DEST"
