#!/usr/bin/env bash
# Generate the roadmap release-history data from CHANGELOG.md + git tags.
#
# Emits apps/logicaffeine_web/src/ui/pages/roadmap_history.json — a JSON array,
# newest-first, of { version, date, title, tagged } — which the web app bakes at
# compile time (see roadmap_history.rs). The CHANGELOG is the source of truth
# (it carries dates + content and includes prepared-but-untagged releases);
# git tags are cross-checked to set "tagged".
#
# Also prints a staging report: releases not yet tagged, tags with no changelog
# entry, and commits on HEAD since the latest tag (the "what isn't on the
# roadmap yet" surface). Run on demand; commit the regenerated JSON.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
CHANGELOG="$ROOT/CHANGELOG.md"
OUT="$ROOT/apps/logicaffeine_web/src/ui/pages/roadmap_history.json"

# --- Parse CHANGELOG into "version<TAB>date<TAB>title-source" records ---------
# For each "## [x.y.z] - YYYY-MM-DD" block, the title source is the first
# meaningful line: the first bullet of the first section, or a prose preamble
# (e.g. "First public release.", "(skipped — …)").
records="$(awk '
  /^## \[/ {
    if (ver != "") print ver "\t" date "\t" title
    line = $0
    ver = line; sub(/^## \[/, "", ver); sub(/\].*/, "", ver)
    date = line; if (line ~ /\] - /) { date = line; sub(/.*\] - /, "", date) } else { date = "" }
    title = ""; want = 1
    next
  }
  {
    if (want == 1) {
      l = $0
      if (l ~ /^[[:space:]]*$/) next     # skip blank lines
      if (l ~ /^### /) next              # skip section headers
      sub(/^- /, "", l)                  # strip a bullet marker if present
      title = l; want = 0
    }
  }
  END { if (ver != "") print ver "\t" date "\t" title }
' "$CHANGELOG")"

# --- Derive a brief, clean headline from a title source ----------------------
clean_title() {
  local t="$1" out
  if [[ "$t" == *'**'* ]]; then
    # Prefer the first **bold lead** — that is the curated headline.
    out="$(printf '%s' "$t" | grep -oP '\*\*\K[^*]+' | head -1)"
  else
    # Prose preamble: drop wrapping parentheses, keep the line.
    out="$t"
    out="${out#(}"; out="${out%)}"
  fi
  out="${out//\`/}"                                   # drop backticks
  out="$(printf '%s' "$out" | sed -E 's/[[:space:]]+/ /g; s/^ +//; s/ +$//')"
  out="${out%.}"                                      # drop a trailing period
  if (( ${#out} > 72 )); then                         # cap to a headline length
    out="$(printf '%s' "$out" | cut -c1-72)"
    out="${out% *}…"
  fi
  printf '%s' "$out"
}

# Releases whose primary content is release plumbing (CI, benchmark infra,
# deploy, editor tooling) or that were skipped — collapsed by default on the
# roadmap. Edit this list as new maintenance releases land.
MAINTENANCE="$(cat <<'EOF'
0.9.2
0.9.1
0.8.9
0.8.8
0.8.7
0.8.6
0.8.5
0.8.3
0.8.1
EOF
)"

# --- Emit the JSON array (newest-first, preserving CHANGELOG order) -----------
while IFS=$'\t' read -r ver date tsrc; do
  [[ -z "$ver" ]] && continue
  title="$(clean_title "$tsrc")"
  if [[ -n "$(git -C "$ROOT" tag -l "v$ver")" ]]; then tagged=true; else tagged=false; fi
  if grep -qx "$ver" <<< "$MAINTENANCE"; then maint=true; else maint=false; fi
  jq -n --arg v "$ver" --arg d "$date" --arg t "$title" --argjson g "$tagged" --argjson m "$maint" \
     '{version: $v, date: $d, title: $t, tagged: $g, maintenance: $m}'
done <<< "$records" | jq -s '.' > "$OUT"

count="$(jq length "$OUT")"

# --- Staging report ----------------------------------------------------------
latest_tag="$(git -C "$ROOT" tag --sort=-v:refname | head -1)"
cl_versions="$(printf '%s\n' "$records" | cut -f1)"

maint_count="$(jq '[.[] | select(.maintenance)] | length' "$OUT")"
echo "Wrote $OUT — $count releases ($maint_count maintenance, collapsed by default). generated_from $(git -C "$ROOT" rev-parse --short HEAD)"
echo
echo "== Staging report =="
echo "Latest git tag: ${latest_tag:-<none>}"
echo
echo "Releases in CHANGELOG with no git tag (prepared / unreleased):"
while IFS=$'\t' read -r ver date _; do
  [[ -z "$ver" ]] && continue
  [[ -z "$(git -C "$ROOT" tag -l "v$ver")" ]] && echo "  - $ver ($date)"
done <<< "$records"
echo
echo "Git tags with no CHANGELOG entry:"
git -C "$ROOT" tag -l 'v*' | sed 's/^v//' | while read -r tg; do
  grep -qx "$tg" <<< "$cl_versions" || echo "  - v$tg"
done
echo
if [[ -n "$latest_tag" ]]; then
  echo "Commits on HEAD since $latest_tag (not in any tagged release):"
  git -C "$ROOT" log --oneline "$latest_tag"..HEAD || true
fi
