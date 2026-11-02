#!/usr/bin/env bash
# synth-image.sh DIR TOP — synthesize the board image in DIR (written by the `decode_board_image` or
# `decode_split_images` example) as every decode image is built: yosys synth_xilinx over the board's top, its protocol,
# the program's RTL and the FT245 bridge where the board has one, flattened, DSP slices kept, every mux in LUTs
# (-nowidelut: the open placer cannot legalize MUXF7/MUXF8 in volume). Writes DIR/top.json for place and route,
# netlist.v for the netlist gate (netlist_x_check.py), stat.txt and synth.log.
set -euo pipefail
D=$1; TOP=$2
cd "$D"
# The image's sources in the order every image was built from: the top, its protocol (a decode step's or a split chip's),
# the program, and the FT245 bridge where the board has one.
SRC="$TOP.v"
for f in decode_board.v chip_board.v spatial_top.v ft245_uart_bridge.v; do
  if [ -f "$f" ]; then SRC="$SRC $f"; fi
done
echo "$(date -u +%T) synth start"
yosys -l synth.log -p "read_verilog $SRC; synth_xilinx -family xc7 -flatten -nowidelut -top $TOP; tee -o stat.txt stat; write_verilog -noattr netlist.v; write_json top.json" > synth.out 2>&1 \
  || { echo "synth failed: $D/synth.log"; exit 1; }
echo "$(date -u +%T) synth done"
