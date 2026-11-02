#!/usr/bin/env bash
# ser-build-200t — a controlled XC7A200T build of the STREAMED whole-program fabric engine (`wpe_soc_stream`:
# the config-in-block-RAM profile, WPE_STREAM caps). yosys → nextpnr-xilinx → fasm2frames → xc7frames2bit.
# Run once, uninterrupted. Every knob is an environment variable so a bitstream is reproducible from the one
# line that built it, and that line is RECORDED beside the bitstream (`<BIT>.build.txt`) — a `.bit` carries no
# record of the RTL, caps or FPU it was built with, and the one time that record was missing the "current" 200T
# image turned out to be the old DEPTH=4 engine (fib=37, nqueens=0 on silicon).
#
#   FPU=1|0             keep (1) or prune (0: `chparam WITH_FPU 0`) the IEEE-754 FPU           default 1
#   SEED=<n>            nextpnr placement seed                                                   default 8
#   ROUTER=router1|2    nextpnr router                                                           default router1
#   PLACE_TIMEOUT=<n>   --placer-heap-cell-placement-timeout (0 = never give up on a cell)      default 0
#   PLACEWEIGHT=<n>     --placer-heap-timingweight (lower = the placer spreads dense logic; 5 unstuck an
#                       FPU build whose router2 oscillated on its last few overused wires)     default: nextpnr's
#   NREG=<n>            re-emit the engine at this register-file depth instead of the checked-in RTL's
#   W=<dir>             work dir (logs, json, fasm, frames)                                      default /tmp/serb
#   BIT=<path>          output bitstream                        default benchmarks/silicon/nexys_video_wpe_stream.bit
#
#   FPU=1 SEED=8 bash scripts/ser-build-200t.sh
set -uo pipefail
cd "$(dirname "$0")/.."
T=tools/openxc7; PART=xc7a200tsbg484-1; SIL=benchmarks/silicon
CHIPDB=$T/nextpnr-xilinx/xilinx/xc7a200t.bin
DBROOT=$T/nextpnr-xilinx/xilinx/external/prjxray-db/artix7
FPU="${FPU:-1}"; SEED="${SEED:-8}"; ROUTER="${ROUTER:-router1}"; PLACE_TIMEOUT="${PLACE_TIMEOUT:-0}"; PLACEWEIGHT="${PLACEWEIGHT:-}"
W=$(realpath -m "${W:-/tmp/serb}"); mkdir -p "$W"   # absolute: the synth step runs from inside benchmarks/silicon
BIT="${BIT:-$SIL/nexys_video_wpe_stream.bit}"
RTL="$SIL/wpe_soc_stream.v"
if [ -n "${NREG:-}" ]; then
  RTL="$W/wpe_soc_stream.v"
  EMIT=target/debug/examples/emit_wpe_stream_rtl
  [ -x "$EMIT" ] || cargo build -q -p logicaffeine-compile --example emit_wpe_stream_rtl || exit 1
  "$EMIT" "$RTL" "$NREG" || exit 1
fi
CAPS=$(grep -m1 'localparam NREG=' "$RTL" | sed 's/^ *localparam //')
# The wrapper exposes WITH_FPU as a TOP-LEVEL parameter (it passes it down to the engine instance); set it there —
# a chparam on the engine module alone is overridden by the wrapper's explicit instance parameter.
FPU_PARAM="chparam -set WITH_FPU $FPU nexys_video_wpe_stream;"
RTL_REL=$(realpath --relative-to="$SIL" "$RTL")
RECIPE="FPU=$FPU SEED=$SEED ROUTER=$ROUTER PLACE_TIMEOUT=$PLACE_TIMEOUT PLACEWEIGHT=${PLACEWEIGHT:-default} NREG=${NREG:-checked-in} RTL=$RTL ($CAPS)"
echo "== build: $RECIPE"

echo "== [1/4] synth (WITH_FPU=$FPU) $(date +%T) =="
( cd "$SIL" && yosys -q -p "read_verilog -sv nexys_video_wpe_stream.v $RTL_REL ft245_uart_bridge.v; $FPU_PARAM synth_xilinx -family xc7 -nodsp -nowidelut -flatten -top nexys_video_wpe_stream; stat; write_json $W/top.json" ) > "$W/synth.log" 2>&1
[ -s "$W/top.json" ] || { echo "SYNTH FAILED"; tail -8 "$W/synth.log"; exit 1; }
grep -aE "Number of cells|LUT6 |MUXF7 |RAMB36E1|FDRE " "$W/synth.log" | tail -5
echo "   synth done $(date +%T)"

echo "== [2/4] nextpnr $ROUTER seed=$SEED place-timeout=$PLACE_TIMEOUT $(date +%T) =="
sed -E 's/get_ports \{[[:space:]]*([^}]*[^}[:space:]])[[:space:]]*\}/get_ports \1/g' "$SIL/nexys_video_fifo.xdc" > "$W/top.xdc"
"$T/nextpnr-xilinx/nextpnr-xilinx" --chipdb "$CHIPDB" --xdc "$W/top.xdc" --seed "$SEED" --router "$ROUTER" \
  --placer-heap-cell-placement-timeout "$PLACE_TIMEOUT" ${PLACEWEIGHT:+--placer-heap-timingweight "$PLACEWEIGHT"} \
  --json "$W/top.json" --fasm "$W/top.fasm" --write "$W/routed.json" > "$W/pnr.log" 2>&1
[ -s "$W/top.fasm" ] || { echo "NO FASM (place/route failed)"; tail -8 "$W/pnr.log"; exit 1; }
grep -aE "Device utilisation|SLICE_LUTX|SLICE_FFX|RAMB36|Max frequency" "$W/pnr.log" | tail -6
echo "   route done $(date +%T)"

echo "== [3/4] fasm2frames $(date +%T) =="
PYTHONPATH="$T/prjxray" "$T/venv/bin/python" "$T/prjxray/utils/fasm2frames.py" \
  --db-root "$DBROOT" --part "$PART" "$W/top.fasm" "$W/top.frames" > "$W/f2f.log" 2>&1
[ -s "$W/top.frames" ] || { echo "FASM2FRAMES FAILED"; tail -6 "$W/f2f.log"; exit 1; }

echo "== [4/4] frames2bit $(date +%T) =="
"$T/prjxray/build/tools/xc7frames2bit" --part_name "$PART" --part_file "$DBROOT/$PART/part.yaml" \
  --frm_file "$W/top.frames" --output_file "$BIT" > "$W/bit.log" 2>&1
[ -s "$BIT" ] || { echo "BIT FAILED"; tail -6 "$W/bit.log"; exit 1; }
{
  echo "built $(date -u +%FT%TZ) by scripts/ser-build-200t.sh"
  echo "$RECIPE"
  echo "rtl md5 $(md5sum "$RTL" | cut -d' ' -f1)"
  grep -aE "Device utilisation|Max frequency" "$W/pnr.log" | tail -4
} > "$BIT.build.txt"
echo "★★★ BITSTREAM READY: $BIT ($(stat -c%s "$BIT") bytes) $(date +%T) — recipe in $BIT.build.txt"
