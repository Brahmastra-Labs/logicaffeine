#!/usr/bin/env bash
#
# Triage a wiki article through the LOGOS English→FOL compiler: classify every
# sentence into the kind of work it implies (lexicon / parser / semantic / human
# / isolate), localize it, derive an oracle where a paraphrase already parses,
# and emit an actionable worklist + machine records for the improvement loop.
#
# READ-ONLY: writes only under wikis/triage/<article>/. Never edits source.
#
# Usage:
#   ./scripts/triage-wiki.sh wikis/paul-corballis.txt
#   ./scripts/triage-wiki.sh wikis/paul-corballis.txt some/other/outdir
#
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

input="${1:?usage: triage-wiki.sh <input.txt> [output_dir]}"
stem="$(basename "${input%.*}")"
outdir="${2:-${repo_root}/wikis/triage/${stem}}"

cargo build -p wiki-trace --bin wiki-triage --quiet
cargo run -p wiki-trace --bin wiki-triage --quiet -- "$input" "$outdir"
