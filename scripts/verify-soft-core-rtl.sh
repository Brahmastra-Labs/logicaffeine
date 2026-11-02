#!/usr/bin/env bash
# Verify the ACTUAL Verilog (benchmarks/silicon/soft_core.v) against the VM, at the scale of the whole
# ≡VM differential — not just the 43 hand-written testbenches.
#
# The generator is drift-free: `emit_soft_core_tb` in codegen_sva/rtl.rs writes one iverilog testbench
# per differential program straight from the SAME (program, expected) pair the Rust `run_soft_core`
# model asserts on (and which is itself locked ≡ vm_outcome). So a green run here means
#     actual soft_core.v RTL  ≡  run_soft_core model  ≡  VM
# over every program in the differential, and the RTL can't silently drift from the model.
#
# Requires iverilog. Step 1 regenerates the TBs (so they always match the current model); pass
# --no-regen to skip the cargo run and reuse the checked-in tb_gen_*.v.
set -uo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" != "--no-regen" ]; then
  echo "== [1/2] regenerate tb_gen_*.v from the model (drift-free) =="
  LOGOS_EMIT_SC_TB=1 cargo test -q -p logicaffeine-compile --lib \
    the_soft_core_runs_logos_programs_identically_to_the_vm 2>&1 | tail -1
fi

cd benchmarks/silicon
command -v iverilog >/dev/null 2>&1 || { echo "iverilog not found — cannot run RTL verification."; exit 0; }

echo "== [2/2] iverilog every generated TB against soft_core.v + alu.v =="
pass=0; fail=0; failed=""
for tb in tb_gen_*.v; do
  name=$(basename "$tb" .v)
  if ! iverilog -o "/tmp/${name}.vvp" alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v fpu.v soft_core.v "$tb" 2>"/tmp/${name}.cerr"; then
    echo "  COMPILE-FAIL $name"; sed 's/^/      /' "/tmp/${name}.cerr" | head -6
    fail=$((fail+1)); failed="$failed $name"; continue
  fi
  out=$(vvp "/tmp/${name}.vvp" 2>&1)
  if echo "$out" | grep -q "^PASS:"; then
    pass=$((pass+1))
  else
    echo "  SIM-FAIL $name: $(echo "$out" | grep -vE '^$' | head -3 | tr '\n' ' ')"
    fail=$((fail+1)); failed="$failed $name"
  fi
done

echo ""
echo "SUMMARY: $pass passed, $fail failed of $((pass+fail)) generated soft-core RTL testbenches"
[ -n "$failed" ] && { echo "FAILED:$failed"; exit 1; }
echo "ALL GREEN — soft_core.v RTL ≡ run_soft_core model ≡ VM across the whole differential."
