#!/usr/bin/env bash
# validate-fabric-engine — flash a whole-program fabric-engine bitstream and PROVE it on the board: the benchmark
# manifest, the recorded demo wires, and LOGOS programs (a `## Main` with Show, recursion, text, floats), every
# result checked against the VM by scripts/logos-chip.py. Prints a per-item verdict and a tally; exits non-zero
# only on a real disagreement (a program the host REFUSES because the image cannot hold it is reported, not failed).
#
#   scripts/validate-fabric-engine.sh wpe    wpe_soc_show2.bit
#   scripts/validate-fabric-engine.sh wpe200 nexys_video_wpe_stream_show256.bit
#
# The bitstream's `<bit>.build.txt` recipe (written by the build scripts) is what tells the host the image's caps
# and whether it carries the FPU; a bitstream without one is validated against the engine PROFILE only.
set -uo pipefail
cd "$(dirname "$0")/.."
TARGET="${1:?target: wpe | wpe200}"; BIT="${2:?bitstream file name under benchmarks/silicon/}"
SIL=benchmarks/silicon; P=$SIL/programs
[ -s "$SIL/$BIT" ] || { echo "no $SIL/$BIT"; exit 2; }
echo "##### $TARGET  $BIT  $(date -u +%FT%TZ)"
[ -f "$SIL/$BIT.build.txt" ] && cat "$SIL/$BIT.build.txt" || echo "(no recipe beside the bitstream — profile-only fit check)"
cargo build -q -p logicaffeine-compile --example emit_wpe_soc --example lgvm || { echo "compiler examples failed to build"; exit 2; }

pass=0; fail=0; refused=0
verdict() { # label, host output
  if grep -q 'SILICON == VM\|OK: matches' <<<"$2"; then pass=$((pass+1)); echo "  PASS     $1"
  elif grep -q 'does not fit' <<<"$2"; then refused=$((refused+1)); echo "  REFUSED  $1  ($(grep -o 'does not fit.*' <<<"$2" | head -1 | cut -c1-90))"
  else fail=$((fail+1)); echo "  FAIL     $1"; echo "$2" | grep -v 'Load SRAM' | tail -n 4 | sed 's/^/           /'; fi
}

case "$TARGET" in
  wpe)
    MANIFEST=$P/bench_manifest.txt
    WIRES="wpe_loop.wire:15 wpe_recursion.wire:15 wpe_collections.wire:106 wpe_incr.wire:42 wpe_rec8.wire:36"
    PROGRAMS=("$P/hello_fabric.lg" "$P/wpe_fib_rec.lg --args 8" "$P/wpe_loop.lg --args 5" "$P/wpe_collections.lg --args 7" "$P/text_small.lg" "$P/text_show.lg" "$P/float_show.lg")
    WAIT=90;;
  wpe200)
    MANIFEST=$P/s200_manifest.txt
    WIRES=""
    PROGRAMS=("$P/hello_fabric.lg" "$SIL/fpga/wire_fib.lg" "$P/wpe_fib_rec.lg --args 8" "$P/wpe_fib_rec.lg --args 20" "$P/wpe_loop.lg --args 100" "$P/wpe_collections.lg --args 7" "$P/float_show.lg" "$P/text_show.lg")
    WAIT=180;;
  *) echo "unknown target $TARGET"; exit 2;;
esac

echo "== benchmark manifest $(basename "$MANIFEST") =="
python3 scripts/logos-chip.py suite "$TARGET" --manifest "$MANIFEST" --flash --bit "$BIT" --wait "$WAIT" | grep -v 'Load SRAM'
for w in $WIRES; do
  f=${w%%:*}; e=${w#*:}
  out=$(python3 scripts/logos-chip.py run "$TARGET" "$P/$f" --expect "$e" --bit "$BIT" --wait 60 2>&1); verdict "$f (expect $e)" "$out"
done
echo "== LOGOS programs over the wire =="
for spec in "${PROGRAMS[@]}"; do
  out=$(python3 scripts/logos-chip.py run "$TARGET" $spec --no-build --bit "$BIT" --wait "$WAIT" 2>&1)
  args=""; [[ "$spec" == *" "* ]] && args=" ${spec#* }"
  verdict "$(basename "${spec%% *}")$args" "$out"
  grep -E '^  > ' <<<"$out" | sed 's/^/           /'
done
echo "##### $TARGET $BIT: $pass passed, $fail failed, $refused refused (image too small)  $(date -u +%FT%TZ)"
[ "$fail" -eq 0 ]
