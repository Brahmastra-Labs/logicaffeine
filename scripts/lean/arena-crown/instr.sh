#!/usr/bin/env bash
# Instructions executed by the arena checker on one input: the Arena's ranking metric.
#   scripts/lean/arena-crown/instr.sh <input.ndjson> [checker]
#   scripts/lean/arena-crown/instr.sh <input.ndjson> --run '<shell command reading "$IN">'
# A checker is a `logos_kernel` binary (a snapshot.sh snapshot, or else the `arena` profile built
# here: fat LTO and one codegen unit, no PGO), run with the logos-kernel entry's own Arena run line
# as arena/logos-kernel.yaml states it (`--check sem`: the value engine the Arena runs, not the
# substitution engine the binary defaults to). Any other command (logos-dual's run line, a rival's
# from rivals.sh) goes through `--run`. Pins to a P-core, counts user-space instructions with perf
# stat, and prints (CORE=<n> picks the performance core, default 2)
# `<input> <exit> <instructions> <seconds-at-6G/s> <wall>`.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
IN="$(realpath "$1")"
BIN="${2:-}"
if [ "$BIN" = "--run" ]; then
  RUN="${3:?--run needs a command}"
else
  if [ -z "$BIN" ]; then
    (cd "$ROOT" && cargo build --profile arena -p logicaffeine-kernel --features unchecked-access --example logos_kernel >/dev/null 2>&1)
    BIN="$ROOT/target/arena/examples/logos_kernel"
  fi
  ENTRY_ARGS="$(python3 -c '
import re, shlex, sys
text = open(sys.argv[1]).read()
run = re.search(r"^run: \|\n((?:  .*\n?)+)", text, re.M).group(1).strip()
words = shlex.split(run)
assert words[0].endswith("logos_kernel") and words[-1] == "$IN", run
print(" ".join(shlex.quote(w) for w in words[1:-1]))
' "$ROOT/crates/logicaffeine_kernel/arena/logos-kernel.yaml")"
  RUN="$(printf '%q' "$BIN") $ENTRY_ARGS \"\$IN\""
fi
CORE="${CORE:-2}"
mkdir -p "$ROOT/logs/arena-crown"
OUT="$ROOT/logs/arena-crown/perfstat.$$"
set +e
IN="$IN" sudo -n --preserve-env=IN perf stat -x, -e cpu_core/instructions/u -o "$OUT" -- taskset -c "$CORE" sh -c "$RUN" >/dev/null 2>&1
CODE=$?
set -e
INSTR="$(grep instructions "$OUT" | cut -d, -f1)"
sudo -n rm -f "$OUT"
python3 -c "import sys; i=int(sys.argv[3]); print(f'{sys.argv[1]} exit={sys.argv[2]} instructions={i:,} virtual={i/6e9:.2f}s')" "$(basename "$IN")" "$CODE" "$INSTR"
