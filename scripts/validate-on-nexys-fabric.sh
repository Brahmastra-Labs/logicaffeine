#!/usr/bin/env bash
# validate-on-nexys-fabric — flash the reconfigurable-fabric SoC to a real Nexys A7-100T (Artix-7) and
# verify it computes every datapath ≡ the VM, ON SILICON. The board-gated proof behind
# fabric_soc_lock::fabric_runs_on_the_artix7_matching_the_vm (LOGOS_FPGA_BOARD=1).
#
#   scripts/validate-on-nexys-fabric.sh                 # Board B (Nexys A7-100T)
#   BOARD=a scripts/validate-on-nexys-fabric.sh         # Board A (Nexys Video)
#   RECORD=logs/fabric/run.json scripts/validate-on-nexys-fabric.sh   # also write the run's JSON receipt
#
# Exits 0 iff fabric-host.py reads all datapaths back matching the VM, each settled inside the board's sweep
# window. The loader runs through scripts/lib/ftdi_ports.py: the right cable per board, `sudo -n` on Linux only.
set -euo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
BIT="$SIL/nexys_fabric_soc.bit"

BOARD="${BOARD:-b}"
PORT=$(python3 scripts/lib/ftdi_ports.py port "$BOARD") || exit 1

[ -f "$BIT" ] || { echo "missing $BIT — build it: TOP=nexys_fabric_soc SRC=\"nexys_fabric_soc.v config_fabric.v\" BIT=nexys_fabric_soc.bit XDC=$SIL/nexys_fabric_soc.xdc bash scripts/openxc7-build.sh"; exit 1; }

echo "== flashing $BIT to board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$BIT" 2>&1 | tail -1
sleep 1

echo "== reading the fabric back over UART ($PORT) =="
exec python3 scripts/fabric-host.py --port "$BOARD" --baud 115200 --bit "$BIT" ${RECORD:+--record "$RECORD"}
