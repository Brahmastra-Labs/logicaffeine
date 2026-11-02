#!/usr/bin/env bash
# validate-on-nexys-float — flash the FLOAT-LANE SoC to a real Nexys A7-100T (Artix-7) and verify it computes
# every hypotenuse ≡ the reference, ON SILICON. The board-gated proof behind
# fabric_soc_lock::float_lane_runs_on_the_artix7_matching_the_reference (LOGOS_FPGA_BOARD=1).
#
#   scripts/validate-on-nexys-float.sh                 # Board B (Nexys A7-100T)
#   BOARD=a scripts/validate-on-nexys-float.sh         # Board A (Nexys Video)
#
# Exits 0 iff float-host.py reads all hypotenuses back matching the reference. The loader runs
# through scripts/lib/ftdi_ports.py: the right cable per board, `sudo -n` on Linux only.
set -euo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
BIT="$SIL/nexys_float_soc.bit"

BOARD="${BOARD:-b}"
PORT=$(python3 scripts/lib/ftdi_ports.py port "$BOARD") || exit 1

[ -f "$BIT" ] || { echo "missing $BIT — build it: TOP=nexys_float_soc SRC=\"nexys_float_soc.v float_fhypf.v fpu.v\" BIT=nexys_float_soc.bit XDC=$SIL/nexys_float_soc.xdc bash scripts/openxc7-build.sh"; exit 1; }

echo "== flashing $BIT to board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$BIT" 2>&1 | tail -1
sleep 1

echo "== reading the float lane back over UART ($PORT) =="
exec python3 scripts/float-host.py --port "$PORT" --baud 115200
