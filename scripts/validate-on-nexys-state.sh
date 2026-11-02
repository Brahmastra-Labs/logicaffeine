#!/usr/bin/env bash
# validate-on-nexys-state — flash the STATE-PLANE SoC to a real Nexys A7-100T (Artix-7) and verify it
# runs every `grade` program (collections + arithmetic + control flow) ≡ the VM, ON SILICON. The proof behind
# fabric_soc_lock::state_plane_runs_on_the_artix7_matching_the_vm (LOGOS_FPGA_BOARD=1).
#
#   scripts/validate-on-nexys-state.sh                 # Board B (Nexys A7-100T)
#   BOARD=a scripts/validate-on-nexys-state.sh         # Board A (Nexys Video)
#
# Exits 0 iff state-host.py reads all grade results back matching the VM. The loader runs
# through scripts/lib/ftdi_ports.py: the right cable per board, `sudo -n` on Linux only.
set -euo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
BIT="$SIL/nexys_state_soc.bit"

BOARD="${BOARD:-b}"
PORT=$(python3 scripts/lib/ftdi_ports.py port "$BOARD") || exit 1

[ -f "$BIT" ] || { echo "missing $BIT — build it: TOP=nexys_state_soc SRC=\"nexys_state_soc.v state_grade.v\" BIT=nexys_state_soc.bit XDC=$SIL/nexys_state_soc.xdc bash scripts/openxc7-build.sh"; exit 1; }

echo "== flashing $BIT to board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$BIT" 2>&1 | tail -1
sleep 1

echo "== reading the state plane back over UART ($PORT) =="
exec python3 scripts/state-host.py --port "$PORT" --baud 115200
