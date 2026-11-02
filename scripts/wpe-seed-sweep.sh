#!/usr/bin/env bash
# wpe-seed-sweep — the open-flow hold-fixing lottery for the over-the-wire engine. nextpnr-xilinx does NO
# hold-time fixing, so a proven-correct netlist can glitch on silicon on a hold-marginal FF->FF path; a
# different placement SEED clears it. This builds the engine bitstream with successive seeds, flashes each to
# Board B, streams wpe_loop.wire (f(5)=15), and stops at the first seed that returns 15 on real hardware.
#
#   scripts/wpe-seed-sweep.sh [seed1 seed2 ...]      (default: 1 3 5 7 9)
set -uo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
PORT=$(python3 scripts/lib/ftdi_ports.py port B) || exit 1
SEEDS=("$@"); [ ${#SEEDS[@]} -eq 0 ] && SEEDS=(1 3 5 7 9)

for s in "${SEEDS[@]}"; do
  echo "======== SEED=$s ========"
  if ! SEED="$s" TOP=nexys_wpe_soc SRC="nexys_wpe_soc.v wpe_soc_fpga.v" BIT="wpe_soc_s$s.bit" \
       bash scripts/openxc7-build.sh > "/tmp/wpe_seed_${s}.log" 2>&1; then
    echo "  seed $s: build FAILED"; tail -2 "/tmp/wpe_seed_${s}.log"; continue
  fi
  [ -s "$SIL/wpe_soc_s$s.bit" ] || { echo "  seed $s: no bitstream"; continue; }
  python3 scripts/lib/ftdi_ports.py flash B "$SIL/wpe_soc_s$s.bit" >/dev/null 2>&1
  sleep 4
  out=$(timeout 30 python3 scripts/nexys-wpe-host.py "$SIL/programs/wpe_loop.wire" \
        --port "$PORT" --baud 9600 --clock-mhz 0.78125 --expect 15 2>&1)
  echo "  seed $s: $(echo "$out" | tail -1)"
  if echo "$out" | grep -q "OK: matches"; then
    echo "SUCCESS: seed $s runs the over-the-wire engine on real silicon (f(5)=15)"
    cp "$SIL/wpe_soc_s$s.bit" "$SIL/wpe_soc.bit"
    exit 0
  fi
done
echo "no seed in {${SEEDS[*]}} cleared the hold-marginal path — try more seeds or reduce the design"
exit 1
