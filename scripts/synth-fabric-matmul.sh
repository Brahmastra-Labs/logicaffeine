#!/usr/bin/env bash
# synth-fabric-matmul — the automatic "write the matmul in LOGOS → the compiler emits the fabric RTL →
# synthesize → bitstream" pipeline. It emits a P-wide spatial int8 MAC array FROM LOGOS
# (examples/emit_matmul_array — NO hand-written RTL), verifies it under iverilog, then drives it through the
# fully-open flow (yosys synth_xilinx → nextpnr-xilinx → prjxray) to a Nexys A7 (xc7a100t) bitstream. This is
# the generalization to any model: point the same path at any model's matmul by swapping the LOGOS.
#
#   bash scripts/synth-fabric-matmul.sh <P>          # P = MACs/clock (multiply-tile count)
#
# DSP inference is ON (the array maps to the board's DSP48E1s) and -flatten is OFF (the flattened wide
# MAC-tree stalls yosys/abc). Set SYNTH_ONLY=1 to stop after the yosys gate (fast; reports DSP/LUT).
set -uo pipefail
cd "$(dirname "$0")/.."
P="${1:-8}"
MODE="${2:-}"            # "int8" selects the int8-native datapath (1 DSP/MAC) + attempts the DSP path
SIL=benchmarks/silicon
TAG="r$P"; MULENV=(); DSPTRY=0
if [ "$MODE" = "int8" ]; then TAG="i8_r$P"; MULENV=(MUL_BITS=8); DSPTRY=1; fi
W="$SIL/_fabric_matmul_$TAG"

echo "== [1/4] emit the ${P}-wide spatial MAC array FROM LOGOS (datapath: ${MODE:-64bit}) =="
env "${MULENV[@]}" cargo run -q -p logicaffeine-compile --example emit_matmul_array -- "$P" "$W" || { echo "emit failed"; exit 1; }
cp "$W/matmul_array.v"   "$SIL/matmul_array_$TAG.v"
cp "$W/matmul_array.xdc" "$SIL/matmul_array_$TAG.xdc"

echo "== [2/4] verify the EMITTED RTL under iverilog (correctness before bitstream) =="
if command -v iverilog >/dev/null 2>&1; then
  iverilog -g2012 -s tb -o "$W/tb.vvp" "$W/tb_matmul_array.v" \
    && for N in 8 16; do printf "  N=%s -> " "$N"; vvp "$W/tb.vvp" +N="$N" | grep -o 'RESULT:.*'; done
else
  echo "  (iverilog absent — skipping the sim check)"
fi

echo "== [3/4] yosys synth_xilinx (xc7, DSP inference ON, no flatten) — the SYNTHESIS gate =="
yosys -p "read_verilog $SIL/matmul_array_$TAG.v; synth_xilinx -family xc7 -top matmul_array_top; stat" \
  >"$W/synth.log" 2>"$W/synth.err"
sed -n '/=== matmul_array_top ===/,/End of script/p' "$W/synth.log" | grep -iE "Number of cells|DSP48|LUT6|CARRY4|FDRE" || true
grep -iE "^ERROR|error:" "$W/synth.err" && { echo "SYNTH ERRORS ^"; exit 1; }
echo "  synthesized OK → $W/synth.log"

if [ "${SYNTH_ONLY:-0}" = "1" ]; then
  echo "SYNTH_ONLY=1 → stopping after the synthesis gate."
  exit 0
fi

echo "== [4/4] nextpnr → prjxray → bitstream (open flow) =="
# int8 mode → one DSP48E1 per MAC (no wide-multiply DSP cascade), so we attempt the DSP path (DSPTRY=1); the
# 64-bit path defaults to LUT-multipliers because a 64×64 multiply's DSP cascade tripped openxc7's packer.
# DSP=1 forces the DSP path; DSP=0 forces LUT-multipliers.
NODSP_FLAG="-nodsp"; [ "${DSP:-$DSPTRY}" = "1" ] && NODSP_FLAG=""
echo "  datapath=${MODE:-64bit}  DSP path=$([ -z "$NODSP_FLAG" ] && echo ON || echo OFF)"
NODSP="$NODSP_FLAG" FLATTEN_FLAG="" TOP=matmul_array_top SRC="matmul_array_$TAG.v" \
  XDC="matmul_array_$TAG.xdc" BIT="fabric_matmul_$TAG.bit" \
  bash scripts/openxc7-build.sh
