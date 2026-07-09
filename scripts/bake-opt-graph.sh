#!/usr/bin/env bash
#
# Bake the per-program optimization graph (fired + blockers + dependencies) into
# benchmarks/results/latest.json, so the /benchmarks toggle tree pops in instantly
# on benchmark switch instead of recompiling in the browser.
#
# This is the standalone "for now" generator — the benchmark run (run.sh) bakes the
# same fields during a full run. Run this to refresh the graph without a full run:
#
#   ./scripts/bake-opt-graph.sh                       # patches results/latest.json
#   RESULTS=path/to/other.json ./scripts/bake-opt-graph.sh
#
# Requires: jq. Builds the `largo` CLI in release if not already present.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROGRAMS_DIR="$ROOT/benchmarks/programs"
RESULTS="${RESULTS:-$ROOT/benchmarks/results/latest.json}"

command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }
[ -f "$RESULTS" ] || { echo "error: results file not found: $RESULTS" >&2; exit 1; }

echo "Building largo (release)…"
cargo build --release -p logicaffeine-cli --bin largo >/dev/null 2>&1
LARGO="$ROOT/target/release/largo"
[ -x "$LARGO" ] || { echo "error: largo binary not found at $LARGO" >&2; exit 1; }

tmp="$(mktemp)"
cp "$RESULTS" "$tmp"
count=0
for dir in "$PROGRAMS_DIR"/*/; do
    id="$(basename "$dir")"
    lg="$dir/main.lg"
    [ -f "$lg" ] || continue
    # Only patch benchmarks that exist in the results file.
    if [ "$(jq --arg id "$id" 'any(.benchmarks[]; .id == $id)' "$tmp")" != "true" ]; then
        continue
    fi
    graph="$("$LARGO" opts "$lg" --json)"
    jq --arg id "$id" --argjson g "$graph" \
        '.benchmarks |= map(if .id == $id
            then . + {fired: $g.fired, blockers: $g.blockers, dependencies: $g.dependencies}
            else . end)' "$tmp" > "$tmp.next"
    mv "$tmp.next" "$tmp"
    fired_n="$(echo "$graph" | jq '.fired | length')"
    dep_n="$(echo "$graph" | jq '.dependencies | length')"
    blk_n="$(echo "$graph" | jq '.blockers | length')"
    printf '  %-18s fired=%s blockers=%s deps=%s\n' "$id" "$fired_n" "$blk_n" "$dep_n"
    count=$((count + 1))
done

mv "$tmp" "$RESULTS"
echo "Baked optimization graph into $RESULTS ($count benchmarks)."
