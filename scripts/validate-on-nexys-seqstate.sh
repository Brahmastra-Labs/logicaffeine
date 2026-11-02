#!/usr/bin/env bash
# validate-on-nexys-seqstate — flash the whole-program STATE-PLANE fabric (a Seq built + looped on the shared
# BRAM heap) to a real Nexys A7-100T (Artix-7) and verify it streams the right results back over the UART ≡ the
# VM, ON SILICON. This is the sequenced whole-program fabric (emit_seq_program_verilog with the state plane) —
# distinct from the single-block state SoC in validate-on-nexys-state.sh.
#
# The program is `lsum(n) = sum([n, n*2, n*3, n+7]) = 7n+7`: the fabric allocates the 4-element Seq in its heap,
# fills it, then LOOPS reading `item i of xs` across a back-edge. Stream n in (baked ROM n=1/3/5/8), get the sum
# back. EXPECT below is asserted against the VM by fabric_seq_datapath_lock. Regenerate the RTL with
# `scripts/test/gen-silicon-goldens.sh lsum-soc`, rebuild the bitstream (the build line below), and run this
# script: a regenerated SoC is proven only here.
#
#   scripts/validate-on-nexys-seqstate.sh              # Board B (Nexys A7-100T)
#   BOARD=a scripts/validate-on-nexys-seqstate.sh      # Board A (Nexys Video)
set -euo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
BIT="$SIL/nexys_seqprog_lsum.bit"
EXPECT="14,28,42,63"

BOARD="${BOARD:-b}"
PORT=$(python3 scripts/lib/ftdi_ports.py port "$BOARD") || exit 1

[ -f "$BIT" ] || { echo "missing $BIT — build it: TOP=nexys_seqprog_lsum SRC=\"nexys_seqprog_lsum.v\" BIT=nexys_seqprog_lsum.bit ABC9=1 ROUTER=router1 PLACERELAX=1 SEED=3 bash scripts/openxc7-build.sh"; exit 1; }

echo "== flashing $BIT to board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$BIT" 2>&1 | tail -1
sleep 2

echo "== streaming the state-plane fabric's results back over UART ($PORT) =="
exec python3 scripts/seqprog-host.py --port "$PORT" --baud 9600 --n 4 --expect "$EXPECT"
