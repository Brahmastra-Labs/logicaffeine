#!/usr/bin/env bash
# verify-fpu-soft-core.sh — prove the FLOAT soft-core (soft_core.v + fpu.v) runs LOGOS float programs
# BIT-EXACT to the VM, at the wide FPGA config (REGS=256), over the chip-sized float benchmarks.
#
# The generator is drift-free: `emit_fpu_soft_core_tb` in codegen_sva/rtl.rs writes one iverilog
# testbench per FLOAT chip bench straight from the SAME (program, expected-f64-bits) pair the Rust
# `run_soft_core_typed` model asserts on (and which is itself locked ≡ vm_outcome). A green run means
#     actual soft_core.v + fpu.v RTL  ≡  run_soft_core_typed model  ≡  VM
# over every float benchmark, and each ShowF is compared BIT-FOR-BIT against the VM's f64.
#
# Requires iverilog. Step 1 regenerates the TBs (so they always match the current model); pass
# --no-regen to skip the cargo run and reuse the checked-in tb_chip_fpu_*.v.
set -uo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" != "--no-regen" ]; then
  echo "== [1/2] regenerate tb_chip_fpu_*.v from the float model (drift-free) =="
  # The FPU-datapath benches (pi_leibniz/mandelbrot/spectral_norm/nbody), the FLOAT-HEAP differential
  # (Seq of Float build/index/COW/SetIndex/param — the `fdmem` plane), AND the FLOAT-COMPARE/ITERATE ops
  # (FLe 229 / FNe 230 / FIterNext 231) all emit tb_chip_fpu_*.v — regenerate every set so none can drift.
  LOGOS_EMIT_FPU_TB=1 cargo test -q -p logicaffeine-compile --lib \
    float_chip_benchmarks_match_vm_and_emit_rtl_tb 2>&1 | tail -1
  LOGOS_EMIT_FPU_TB=1 cargo test -q -p logicaffeine-compile --lib \
    emit_float_heap_tbs 2>&1 | tail -1
  LOGOS_EMIT_FPU_TB=1 cargo test -q -p logicaffeine-compile --lib \
    emit_float_cmp_iter_tbs 2>&1 | tail -1
fi

cd benchmarks/silicon
command -v iverilog >/dev/null 2>&1 || { echo "iverilog not found — cannot run FPU-soft-core verification."; exit 0; }

echo "== [2/2] iverilog every tb_chip_fpu_*.v against soft_core.v + fpu.v =="
pass=0; fail=0; failed=""
shopt -s nullglob
for tb in tb_chip_fpu_*.v; do
  name=$(basename "$tb" .v)
  if ! iverilog -g2012 -o "/tmp/${name}.vvp" \
        alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v fpu.v soft_core.v "$tb" \
        2>"/tmp/${name}.cerr"; then
    echo "  COMPILE-FAIL $name"; sed 's/^/      /' "/tmp/${name}.cerr" | head -6
    fail=$((fail+1)); failed="$failed $name"; continue
  fi
  out=$(vvp "/tmp/${name}.vvp" 2>&1 | grep -vE '^WARNING' )
  if echo "$out" | grep -q "^PASS:"; then
    pass=$((pass+1)); echo "  [OK]  $(echo "$out" | grep '^PASS:' | head -1)"
  else
    echo "  SIM-FAIL $name: $(echo "$out" | grep -vE '^$' | head -3 | tr '\n' ' ')"
    fail=$((fail+1)); failed="$failed $name"
  fi
done

echo ""
echo "SUMMARY: $pass passed, $fail failed of $((pass+fail)) FPU soft-core testbenches"
[ -n "$failed" ] && { echo "FAILED:$failed"; exit 1; }
echo "ALL GREEN — soft_core.v + fpu.v RTL ≡ run_soft_core_typed model ≡ VM across the float benchmarks."
