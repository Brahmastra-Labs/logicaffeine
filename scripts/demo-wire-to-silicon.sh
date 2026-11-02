#!/usr/bin/env bash
# DEMO 1 — "burn the language to the chip."
#
# Compile a real LOGOS program, send it down the wire as a self-contained `binisa` frame, and watch the
# soft-core SILICON execute it — bit-identical to the reference VM. Three legs, all locked green:
#   • THE WIRE   — the program is a checksummed, self-contained byte frame; a machine sharing NOTHING but the
#                  bytes (a fresh interner) decodes and runs it identically to the VM.
#   • THE SILICON— the same program, as the chip-word image, runs on the soft-core (and the ACTUAL soft_core.v
#                  Verilog under iverilog) bit-identical to the VM.
#   • ONE OPCODE MAP — the byte on the wire IS the byte the chip decodes.
# The lock is `wire_to_silicon_is_bit_identical_to_the_vm`; this script is just the narrator over it.
set -uo pipefail
cd "$(dirname "$0")/.."

echo "════════════════════════════════════════════════════════════════════════════"
echo "  LOGOS — code down the wire → the silicon runs it (bit-identical to the VM)"
echo "════════════════════════════════════════════════════════════════════════════"

echo
echo "── [1/2] the tying lock: wire round-trip + silicon-model + shared opcode map ──"
# Build quietly first so cargo's compile chatter doesn't leak into the narration.
cargo build -q -p logicaffeine-compile --tests >/dev/null 2>&1 || true
LOGOS_DEMO=1 cargo test -q -p logicaffeine-compile --lib \
  wire_to_silicon_is_bit_identical_to_the_vm -- --nocapture 2>&1 \
  | grep -E "^=== |^  (LOGOS|binisa|silicon|soft-core)" \
  || { echo "  (lock did not narrate — run: cargo test wire_to_silicon_is_bit_identical_to_the_vm)"; exit 1; }

echo
echo "── [2/2] the ACTUAL soft_core.v Verilog executing those exact programs (iverilog) ──"
cd benchmarks/silicon
if [ ! -f tb_gen_sum_loop.v ]; then
  ( cd ../.. && LOGOS_EMIT_SC_TB=1 cargo test -q -p logicaffeine-compile --lib \
      the_soft_core_runs_logos_programs_identically_to_the_vm >/dev/null 2>&1 )
fi
command -v iverilog >/dev/null 2>&1 || { echo "  iverilog absent — the Rust model already proved silicon==VM above."; exit 0; }
declare -A LABEL=( [sum_loop]="control-flow loop" [sumdown]="recursion" [idx]="data structure" )
for p in sum_loop sumdown idx; do
  if iverilog -o "/tmp/demo_${p}.vvp" alu.v crdt_merge.v soft_core.v "tb_gen_${p}.v" 2>/dev/null \
     && vvp "/tmp/demo_${p}.vvp" 2>&1 | grep -q "^PASS:"; then
    printf "  %-24s soft_core.v RTL: PASS  (== VM)\n" "${LABEL[$p]}"
  else
    printf "  %-24s soft_core.v RTL: FAIL\n" "${LABEL[$p]}"; exit 1
  fi
done

echo
echo "The bytes we send are the bytes the silicon runs. Same program, VM and chip, bit-identical —"
echo "the bytecode IS the instruction set. That is what \"burn the language to the chip\" means."
