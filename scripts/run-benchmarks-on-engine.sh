#!/usr/bin/env bash
# run-benchmarks-on-engine — flash the UNIVERSAL wpe_soc engine ONCE, then stream each benchmark program's
# bytecode over the UART and check the returned result ≡ the VM. The engine re-arms after every program
# (wpe_soc_fpga.v loops back for the next program over the wire), so one bitstream runs them all — this is the
# "send the bytecode, get the answer back" model: the program is DATA on the wire, not baked into the silicon.
#
# Regenerate the .wire byte streams (any fitting program) with, e.g.:
#   cargo run -q -p logicaffeine-compile --example emit_wpe_soc -- \
#     --program prog.lg --fn f --args 10 --wire benchmarks/silicon/programs/wpe_prog.wire
#
#   scripts/run-benchmarks-on-engine.sh            # Board B (Nexys A7-100T)
#   BOARD=a scripts/run-benchmarks-on-engine.sh    # Board A (Nexys Video)
set -uo pipefail
cd "$(dirname "$0")/.."
BOARD="${BOARD:-b}"
PORT=$(python3 scripts/lib/ftdi_ports.py port "$BOARD") || exit 1
BIT=benchmarks/silicon/wpe_soc.bit
P=benchmarks/silicon/programs

[ -f "$BIT" ] || { echo "missing $BIT — build it: TOP=nexys_wpe_soc SRC=\"nexys_wpe_soc.v wpe_soc_fpga.v\" BIT=wpe_soc.bit ABC9=1 ROUTER=router1 PLACERELAX=1 SEED=3 bash scripts/openxc7-build.sh"; exit 1; }

echo "== flashing the universal engine $BIT to board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$BIT" 2>&1 | tail -1
sleep 2

# program : VM-oracle expected result (each program is streamed as bytecode; the engine runs it and replies).
# The deployed engine is the BIG BRAM engine (NREG=16, MAXBLK=8, MAXCELL=32, HEAP=64, DEPTH=8) — heap + frame
# stack in block RAM, so deep recursion (wpe_rec8 = depth-8) and larger collections run on real silicon. Programs
# exceeding the caps (e.g. primes = 10 blocks) are rejected up front by wprog_config_fits_hw_engine — honest.
PROGS="wpe_demo:3 wpe_loop_sum:55 wpe_fib_iter:55 wpe_collatz:111 wpe_gcd:7 wpe_factorial:3628800 wpe_rec8:36"
pass=0; total=0
for pe in $PROGS; do
  name=${pe%:*}; exp=${pe#*:}; total=$((total + 1))
  echo "== streaming $name.wire (VM oracle $exp) =="
  if python3 scripts/nexys-wpe-host.py "$P/$name.wire" --port "$PORT" --baud 9600 --clock-mhz 0.78125 --expect "$exp" 2>&1 | tail -3; then
    pass=$((pass + 1))
  fi
  sleep 1
done
echo "== $pass/$total benchmark programs ran ≡ VM on the universal engine, streamed as bytecode over UART =="
[ "$pass" = "$total" ] || exit 1
