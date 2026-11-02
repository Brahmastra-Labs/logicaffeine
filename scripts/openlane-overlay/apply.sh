#!/usr/bin/env bash
# scripts/openlane-overlay/apply.sh — lay the repo's changes to OpenLane over the fetched OpenLane in tools/.
#
#   scripts/openlane-overlay/apply.sh [openlane-tree]
#
# OpenLane is fetched, not vendored, and the flow runs the copy in tools/OpenLane, or the tree a runner names (the
# comparative bench's tools/qor/openlane-bench, OpenLane as released at BASE). The scripts this project
# changed or added — the SAT antenna jog router and its unit tests, the Magic antenna ECO, the post-diode
# upsize, the diode fallback, the repaired `lappend` sites in cts.tcl and resizer_routing_design.tcl, the
# routing and XOR steps they are wired into — live beside this script at their OpenLane-relative paths, over
# the OpenLane commit named in BASE (licensing: LICENSE and NOTICE here). Every runner applies the overlay before it starts a flow,
# so the flow always runs the tracked copies, and `openlane_overlay_lock` holds tools/ to them.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
OV="$ROOT/scripts/openlane-overlay"
OL="${1:-$ROOT/tools/OpenLane}"
[ -d "$OL/scripts" ] || { echo "FATAL: no OpenLane at $OL (fetch it first)" >&2; exit 1; }
BASE=$(tr -d '[:space:]' < "$OV/BASE")
# The overlay's files replace those of one OpenLane commit, and the tree's commit is the tag of the image the flow
# launches (dependencies/get_tag.py). A checkout's is its own HEAD; a release scripts/qor/fetch-qor-tools.py unpacked
# is stamped with its own in .logos-base. Either way an OpenLane at another commit is refused rather than patched
# with files written against a different one.
if [ -e "$OL/.git/HEAD" ]; then
  HEAD=$(cat "$OL/.git/HEAD")
  case "$HEAD" in
    ref:*) REF="${HEAD#ref: }"; HEAD=$(cat "$OL/.git/$REF" 2>/dev/null || sed -n "s| $REF\$||p" "$OL/.git/packed-refs") ;;
  esac
elif [ -e "$OL/.logos-base" ]; then
  HEAD=$(tr -d '[:space:]' < "$OL/.logos-base")
else
  echo "FATAL: $OL is neither a checkout nor a release stamped by scripts/qor/fetch-qor-tools.py: no commit to check" >&2
  exit 2
fi
HEAD=$(printf '%s' "$HEAD" | tr -d '[:space:]')
[ "$HEAD" = "$BASE" ] \
  || { echo "FATAL: $OL is at $HEAD; the overlay is written over $BASE" >&2; exit 2; }
changed=0
while IFS= read -r f; do
  rel="${f#$OV/}"
  if ! cmp -s "$f" "$OL/$rel"; then
    install -D -m644 "$f" "$OL/$rel"
    echo "  openlane overlay: $rel"
    changed=$((changed + 1))
  fi
done < <(find "$OV/scripts" -type f -not -path '*/__pycache__/*' | sort)
echo "== OpenLane overlay over $BASE: $changed file(s) updated =="
