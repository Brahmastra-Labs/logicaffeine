#!/usr/bin/env bash
# verify-word32-soft-core.sh — prove the WORD32 (ℤ/2³²) soft-core (soft_core.v) runs LOGOS Word32 programs
# IDENTICALLY to the VM, over the Word32 differential set + the chip-sized SHA-256 core.
#
# The generator is drift-free: `emit_word32_soft_core_tb` in codegen_sva/rtl.rs writes one iverilog testbench
# per Word32 program straight from the SAME shows the Rust `run_soft_core_typed` model produces (and which is
# itself locked ≡ vm_outcome by `word32_matches_vm`). A green run means
#     actual soft_core.v RTL  ≡  run_soft_core_typed model  ≡  VM
# over every Word32 program. This covers the whole Word32 surface — word32/intOfWord32 (Move), word_and/
# word_or/word_not (BitAnd/BitOr/Not), the rotl/rotr/word32Shr COPROCESSOR (opcode 38, the two-cycle
# regs[args_start+1] read), wrapping Word32 `+`, a `Seq of Word32` build/index, and — the keystone — the
# UNSIGNED `ShowWord32` output port (SHA-256's hash words exceed 2³¹, so the signed int `Show` would misprint
# them). The chip-sized `sha256.lg` runs the REAL round function + message schedule on silicon.
#
# Requires iverilog. Step 1 regenerates the TBs (so they always match the current model); pass --no-regen
# to skip the cargo run and reuse the checked-in tb_chip_word32_*.v.
set -uo pipefail
cd "$(dirname "$0")/.."

export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"

if [ "${1:-}" != "--no-regen" ]; then
  echo "== [1/2] regenerate tb_chip_word32_*.v from the word32 model (drift-free) =="
  LOGOS_EMIT_WORD32_TB=1 cargo test -q -p logicaffeine-compile --lib emit_word32_tbs 2>&1 | tail -1
fi

cd benchmarks/silicon
command -v iverilog >/dev/null 2>&1 || { echo "iverilog not found — cannot run word32 soft-core verification."; exit 0; }

echo "== [2/2] iverilog every tb_chip_word32_*.v against soft_core.v =="
pass=0; fail=0; failed=""
shopt -s nullglob
for tb in tb_chip_word32_*.v; do
  name=$(basename "$tb" .v)
  if ! iverilog -g2012 -o "/tmp/${name}.vvp" \
        alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v fpu.v soft_core.v "$tb" \
        2>"/tmp/${name}.cerr"; then
    echo "  COMPILE-FAIL $name"; sed 's/^/      /' "/tmp/${name}.cerr" | head -6
    fail=$((fail+1)); failed="$failed $name"; continue
  fi
  out=$(vvp "/tmp/${name}.vvp" 2>&1 | grep -vE '^WARNING')
  if echo "$out" | grep -q "^PASS:"; then
    pass=$((pass+1)); echo "  [OK]  $(echo "$out" | grep '^PASS:' | head -1)"
  else
    echo "  SIM-FAIL $name: $(echo "$out" | grep -vE '^$' | head -3 | tr '\n' ' ')"
    fail=$((fail+1)); failed="$failed $name"
  fi
done

echo ""
echo "SUMMARY: $pass passed, $fail failed of $((pass+fail)) word32 soft-core testbenches"
[ -n "$failed" ] && { echo "FAILED:$failed"; exit 1; }
echo "ALL GREEN — soft_core.v RTL ≡ run_soft_core_typed model ≡ VM across the Word32 programs + the SHA-256 core."
