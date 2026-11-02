#!/usr/bin/env bash
# Per-declaration instruction costs of our checker and the scored rival on one input, and the diff.
#   scripts/lean/arena-crown/decl-costs.sh <input.ndjson> <label> [ours|rival|both]
# Ours is built with the Arena entry's recipe (arena profile, target-cpu=native, PGO trained on
# init-prelude) in its own target directory, then snapshotted by content, so a rebuild of the shared
# tree mid-run cannot change what is measured. The rival is the cost-patched copy of the scored
# mathgraph flash (rivals.sh + rival-decl-costs.py), run with its Arena config (2 threads; each
# thread counts its own declarations). Both run pinned to one performance core under perf access.
# OURS_BIN=<binary> measures that binary instead of building one (e.g. a snapshot already taken).
# Writes logs/arena-crown/costs/<label>/{ours.tsv,rival.*,diff.txt,joined.tsv}.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
IN="$(realpath "$1")"
LABEL="$2"
WHICH="${3:-both}"
OUT="$ROOT/logs/arena-crown/costs/$LABEL"
mkdir -p "$OUT" "$ROOT/logs/arena-crown/bin"
RIVAL="${RIVALS_DIR:-$HOME/.cache/arena-rivals}/mathgraph-flash-costs"

if [ "$WHICH" != rival ] && [ -n "${OURS_BIN:-}" ]; then
  BIN="$(realpath "$OURS_BIN")"
  echo "ours: $BIN" | tee "$OUT/ours.bin"
  sudo -n taskset -c 2 "$BIN" --check sem --decl-costs "$OUT/ours.tsv" "$IN" 2>"$OUT/ours.stderr" || true
  sudo -n chown "$(id -u):$(id -g)" "$OUT/ours.tsv"
  tail -1 "$OUT/ours.stderr"
elif [ "$WHICH" != rival ]; then
  BIN="$("$ROOT/scripts/lean/arena-crown/snapshot.sh" "$OUT/build.log")"
  echo "ours: $BIN" | tee "$OUT/ours.bin"
  sudo -n taskset -c 2 "$BIN" --check sem --decl-costs "$OUT/ours.tsv" "$IN" 2>"$OUT/ours.stderr" || true
  sudo -n chown "$(id -u):$(id -g)" "$OUT/ours.tsv"
  tail -1 "$OUT/ours.stderr"
fi

if [ "$WHICH" != ours ]; then
  [ -x "$RIVAL/target/release/sokonanoda" ] || { echo "no cost-patched rival at $RIVAL; build it with rivals.sh after rival-decl-costs.py" >&2; exit 2; }
  rm -f "$OUT"/rival.*
  sudo -n env MATHGRAPH_DECL_COSTS="$OUT/rival" taskset -c 2 sh -c "'$RIVAL/target/release/sokonanoda' '$RIVAL/config.json' < '$IN'" \
    >"$OUT/rival.stdout" 2>"$OUT/rival.stderr" || echo "rival exit $?" >&2
  sudo -n chown "$(id -u):$(id -g)" "$OUT"/rival.*
fi

python3 "$ROOT/scripts/lean/arena-crown/diff-decls.py" "$OUT/ours.tsv" "$OUT/rival" --top 60 --joined "$OUT/joined.tsv" >"$OUT/diff.txt"
head -8 "$OUT/diff.txt"
echo "report: $OUT/diff.txt"
