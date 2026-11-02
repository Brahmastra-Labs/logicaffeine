#!/usr/bin/env bash
# route-image.sh DIR TOP PART — place, route and write the bitstream of DIR's synthesized board image (synth-image.sh):
# nextpnr-xilinx as scripts/setup-openxc7.sh builds it, every path timed through the hard cells the chipdb leaves
# untimed (a DSP48E1, a block RAM, a LUT-RAM's and a shift register's ports, read from prjxray-db's SDF:
# NEXTPNR_XC7_TIMINGS), then prjxray's fasm2frames and xc7frames2bit. Writes DIR/pnr.log, top.fasm, top_routed.json and
# TOP.bit. PNR_EXTRA adds nextpnr options (a seed, a frequency, a placer weight); NEXTPNR and CHIPDB override the binary
# and the part's chipdb.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
T="$ROOT/tools/openxc7"
D=$1; TOP=$2; PART=$3
DEVICE=$(echo "$PART" | grep -oE '^xc7a[0-9]+t')
NEXTPNR="${NEXTPNR:-$T/nextpnr-xilinx/nextpnr-xilinx}"
CHIPDB="${CHIPDB:-$T/nextpnr-xilinx/xilinx/$DEVICE.bin}"
DBROOT="$T/nextpnr-xilinx/xilinx/external/prjxray-db/artix7"
export NEXTPNR_XC7_TIMINGS="${NEXTPNR_XC7_TIMINGS-$DBROOT/timings}"
for f in "$NEXTPNR" "$CHIPDB" "$T/prjxray/build/tools/xc7frames2bit" "$T/prjxray/utils/fasm2frames.py"; do
  [ -e "$f" ] || { echo "MISSING $f — run scripts/setup-openxc7.sh first"; exit 1; }
done
cd "$D" || exit 2
sed -E 's/get_ports \{[[:space:]]*([^}]*[^}[:space:]])[[:space:]]*\}/get_ports \1/g' "$TOP.xdc" > top_np.xdc
echo "$(date -u +%T) pnr start"
"$NEXTPNR" --chipdb "$CHIPDB" --xdc top_np.xdc --json top.json --fasm top.fasm --write top_routed.json ${PNR_EXTRA:-} > pnr.log 2>&1
[ -s top.fasm ] || { echo "P&R produced no FASM"; tail -20 pnr.log; exit 1; }
echo "$(date -u +%T) pnr done"
grep -iE 'Device utilisation|SLICE_|DSP|RAMB|Max frequency' pnr.log | tail -12
PYTHONPATH="$T/prjxray" "$T/venv/bin/python" "$T/prjxray/utils/fasm2frames.py" --db-root "$DBROOT" --part "$PART" top.fasm top.frames > frames.log 2>&1 \
  || { echo "fasm2frames failed"; tail -10 frames.log; exit 1; }
"$T/prjxray/build/tools/xc7frames2bit" --part_name "$PART" --part_file "$DBROOT/$PART/part.yaml" --frm_file top.frames --output_file "$TOP.bit" > bit.log 2>&1
[ -s "$TOP.bit" ] || { echo "bitstream failed"; tail -10 bit.log; exit 1; }
echo "$(date -u +%T) BITSTREAM $D/$TOP.bit $(stat -c%s "$TOP.bit") bytes"
