#!/usr/bin/env bash
#
# Fetch a random Wikipedia article and print its plain prose sentences,
# one per line. No wiki markup, no infoboxes, no citations, no headings —
# just the sentences from the article body.
#
# Usage:
#   ./random-wikipedia-sentences.sh            # one random article
#   ./random-wikipedia-sentences.sh -n 5       # retry until >= 5 sentences
#   ./random-wikipedia-sentences.sh -l de      # a different language wiki
#
set -euo pipefail

lang="en"
min_sentences=0
max_tries=8

while getopts "l:n:h" opt; do
  case "$opt" in
    l) lang="$OPTARG" ;;
    n) min_sentences="$OPTARG" ;;
    h) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "usage: $0 [-l lang] [-n min_sentences]" >&2; exit 2 ;;
  esac
done

api="https://${lang}.wikipedia.org/w/api.php"

fetch_one() {
  # generator=random picks a random article in the main namespace (grnnamespace=0).
  # prop=extracts with explaintext=1 returns the body as plain UTF-8 text,
  # already stripped of wiki/HTML markup, reference markers, and infoboxes.
  curl -fsSL -G "$api" \
    --data-urlencode "action=query" \
    --data-urlencode "format=json" \
    --data-urlencode "generator=random" \
    --data-urlencode "grnnamespace=0" \
    --data-urlencode "grnlimit=1" \
    --data-urlencode "prop=extracts" \
    --data-urlencode "explaintext=1" \
    --data-urlencode "exsectionformat=plain" \
    --data-urlencode "redirects=1" \
    -H 'User-Agent: random-wikipedia-sentences/1.0 (https://logicaffeine.com)'
}

extract_sentences() {
  WIKI_JSON="$1" MIN_SENTENCES="$min_sentences" python3 <<'PY'
import json, os, re, sys

min_sentences = int(os.environ["MIN_SENTENCES"])
data = json.loads(os.environ["WIKI_JSON"])

pages = data.get("query", {}).get("pages", {})
page = next(iter(pages.values()), {})
title = page.get("title", "(untitled)")
text = page.get("extract", "")

# Drop section headings (the API leaves them as their own lines) and any
# leftover blank/structural lines — we only want flowing prose paragraphs.
paragraphs = []
for line in text.split("\n"):
    line = line.strip()
    if not line:
        continue
    # A heading line has no sentence-ending punctuation and tends to be short.
    if not re.search(r"[.!?]['\")\]]?$", line) and len(line) < 80:
        continue
    paragraphs.append(line)

prose = " ".join(paragraphs)

# Protect periods that don't end a sentence (common abbreviations and
# single-letter initials) by swapping the dot for a placeholder, so the
# splitter below never breaks on them. Restored after splitting.
DOT = "\x00"
ABBREVS = ["Mr", "Mrs", "Ms", "Dr", "Prof", "Sr", "Jr", "St", "Mt",
           "vs", "etc", "e.g", "i.e", "al", "No", "Inc", "Ltd", "Co"]
for ab in ABBREVS:
    prose = re.sub(rf"\b{re.escape(ab)}\.", ab + DOT, prose)
# Single capital-letter initials, e.g. "John F. Kennedy".
prose = re.sub(r"\b([A-Z])\.", rf"\1{DOT}", prose)

# Now break after . ! ? (plus any closing quote/bracket) when the next
# non-space character starts a new sentence (capital, digit, or open quote).
splitter = re.compile(r"(?<=[.!?][\"')\]])\s+(?=[\"'(\[]?[A-Z0-9])"
                      r"|(?<=[.!?])\s+(?=[\"'(\[]?[A-Z0-9])")

sentences = []
for chunk in splitter.split(prose):
    s = chunk.replace(DOT, ".").strip()
    # Keep only things that read like real sentences: end with terminal
    # punctuation and contain at least a few words.
    if re.search(r"[.!?][\"')\]]?$", s) and len(s.split()) >= 3:
        sentences.append(s)

sys.stderr.write(f"# {title} ({len(sentences)} sentences)\n")
print("\n".join(sentences))

# Non-zero exit tells the bash wrapper this article was too thin to use.
sys.exit(0 if len(sentences) >= min_sentences else 3)
PY
}

for ((try = 1; try <= max_tries; try++)); do
  json="$(fetch_one)"
  if out="$(extract_sentences "$json")"; then
    printf '%s\n' "$out"
    exit 0
  fi
  # Article had fewer than -n sentences; try another random one.
done

echo "gave up after $max_tries tries without an article of >= $min_sentences sentences" >&2
exit 1
