#!/usr/bin/env bash
#
# One iteration of the improvement loop's input step: fetch a fresh random
# Wikipedia article, save its clean prose to wikis/<slug>.txt, then triage it
# and print the verdict ("does this page compile to proper FOL yet?").
#
# Usage:
#   ./scripts/next-wiki.sh              # a random article
#   ./scripts/next-wiki.sh -n 12        # retry until an article with >=12 sentences
#
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Fetch a random article; capture its sentences (stdout) and "# Title (N)" header (stderr).
hdr_file="$(mktemp)"
trap 'rm -f "$hdr_file"' EXIT
sentences="$(./scripts/random-wikipedia-sentences.sh "$@" 2> "$hdr_file")"
title="$(sed -E 's/^# (.*) \([0-9]+ sentences\)$/\1/' "$hdr_file")"
slug="$(echo "$title" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9' '-' | sed -E 's/-+/-/g; s/^-|-$//g')"

mkdir -p wikis
article="wikis/${slug}.txt"
printf '%s\n' "$sentences" > "$article"
echo "fetched: $title → $article ($(wc -l < "$article" | tr -d ' ') sentences)"
echo

./scripts/triage-wiki.sh "$article"
echo
echo "verdict: wikis/triage/${slug}/verdict.json"
echo "worklist: wikis/triage/${slug}/worklist.md"
