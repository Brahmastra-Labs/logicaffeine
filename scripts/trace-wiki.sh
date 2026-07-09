#!/usr/bin/env bash
#
# Compile an English wiki article through the LOGOS English→FOL compiler and
# dump all traces (tokens, AST, FOL, ambiguity readings, errors).
#
# Usage:
#   ./scripts/trace-wiki.sh wikis/paul-corballis.txt
#   ./scripts/trace-wiki.sh wikis/paul-corballis.txt some/other/outdir
#
# Output goes to wikis/traces/<article-name>/ by default. See summary.txt for
# the bug map and traces.jsonl for machine-readable per-sentence results.
#
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

input="${1:?usage: trace-wiki.sh <input.txt> [output_dir]}"
stem="$(basename "${input%.*}")"
outdir="${2:-${repo_root}/wikis/traces/${stem}}"

# Build once quietly, then run; keeps cargo's progress noise out of the trace run.
cargo build -p wiki-trace --quiet
cargo run -p wiki-trace --quiet -- "$input" "$outdir"
