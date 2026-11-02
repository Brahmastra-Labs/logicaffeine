#!/usr/bin/env bash
# The Unicode Character Database files the Hugging Face tokenizer port's tables are generated from, at one version,
# into external/unicode/<version>/ with their SHA256SUMS.
#
#   scripts/accel/fetch-unicode-data.sh 16.0.0
#
# `tokenizers` runs its patterns on Oniguruma and its normal forms on unicode-normalization-alignments, each with the
# database of its own release; scripts/accel/gen-hf-unicode-data.py reads both versions' files.
set -euo pipefail

version="${1:?usage: fetch-unicode-data.sh <version>}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
dest="$root/external/unicode/$version"
mkdir -p "$dest"
cd "$dest"
files=(UnicodeData.txt CaseFolding.txt CompositionExclusions.txt DerivedAge.txt PropList.txt DerivedCoreProperties.txt Scripts.txt)
for f in "${files[@]}"; do
    curl -fsSL --retry 3 -o "$f" "https://www.unicode.org/Public/$version/ucd/$f"
done
sha256sum "${files[@]}" > SHA256SUMS
echo "$dest"
