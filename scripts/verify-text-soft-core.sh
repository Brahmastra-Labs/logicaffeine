#!/usr/bin/env bash
# verify-text-soft-core.sh — prove the TEXT (string) soft-core (soft_core.v) runs LOGOS string programs
# IDENTICALLY to the VM, over the Text differential set + the chip-sized string benchmarks.
#
# The generator is drift-free: `emit_text_soft_core_tb` in codegen_sva/rtl.rs writes one iverilog testbench
# per Text program straight from the SAME shows the Rust `run_soft_core_typed` model produces (and which is
# itself locked ≡ vm_outcome by `text_matches_vm`). A green run means
#     actual soft_core.v RTL  ≡  run_soft_core_typed model  ≡  VM
# over every string program: a `Show <int>` is checked off `out`; a Text `Show` (`ShowText`) is REASSEMBLED
# from the `outs` codepoint burst and compared to the VM's exact string bytes. This covers the whole string
# surface — TextConst (imem string-pool copy), Concat, FormatValue (the int→decimal FSM), Length, the char
# Index (`item i of text`), TextEq/TextNe (char-list content compare), and ShowText streaming.
#
# Requires iverilog. Step 1 regenerates the TBs (so they always match the current model); pass --no-regen
# to skip the cargo run and reuse the checked-in tb_chip_text_*.v.
set -uo pipefail
cd "$(dirname "$0")/.."

export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"

if [ "${1:-}" != "--no-regen" ]; then
  echo "== [1/2] regenerate tb_chip_text_*.v + tb_chip_tsb_*.v from the text model (drift-free) =="
  LOGOS_EMIT_TEXT_TB=1 cargo test -q -p logicaffeine-compile --lib emit_text_tbs 2>&1 | tail -1
  # The TEXT / SEQ BUILTIN plane (chr=233 / contains=234 / repeatSeq=235 / parseInt=236) — same drift-free
  # generator, its own tb_chip_tsb_*.v testbenches.
  LOGOS_EMIT_TEXT_TB=1 cargo test -q -p logicaffeine-compile --lib emit_text_seq_builtin_tbs 2>&1 | tail -1
fi

cd benchmarks/silicon
command -v iverilog >/dev/null 2>&1 || { echo "iverilog not found — cannot run text soft-core verification."; exit 0; }

echo "== [2/2] iverilog every tb_chip_text_*.v + tb_chip_tsb_*.v against soft_core.v =="
pass=0; fail=0; failed=""
shopt -s nullglob
for tb in tb_chip_text_*.v tb_chip_tsb_*.v; do
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
echo "SUMMARY: $pass passed, $fail failed of $((pass+fail)) text soft-core testbenches"
[ -n "$failed" ] && { echo "FAILED:$failed"; exit 1; }
echo "ALL GREEN — soft_core.v RTL ≡ run_soft_core_typed model ≡ VM across the string programs + benchmarks."
