#!/usr/bin/env bash
# verify-fpu.sh — prove benchmarks/silicon/fpu.v BIT-EXACT to Rust f64.
#
# Flow: (1) build+run the Rust golden-vector generator (fpu_gen.rs) which emits fpu_<op>.hex vectors
# straight from Rust `f64` arithmetic; (2) iverilog each op's testbench against fpu.v; (3) assert the
# DUT reproduces every Rust result bit-for-bit. A green run means: fpu.v RTL ≡ Rust f64 over the full
# edge-case cross product + a 20k-pair deterministic random sweep, per op.
#
# Requires: rustc, iverilog. Deterministic (fixed LCG seed). Pass --no-regen to reuse existing .hex.
set -uo pipefail
cd "$(dirname "$0")/../benchmarks/silicon"

command -v iverilog >/dev/null 2>&1 || { echo "iverilog not found — cannot run FPU verification."; exit 2; }

GENBIN=/tmp/fpu_gen
if [ "${1:-}" != "--no-regen" ]; then
  command -v rustc >/dev/null 2>&1 || { echo "rustc not found — cannot regenerate vectors."; exit 2; }
  echo "== [1/3] build + run Rust f64 golden-vector generator =="
  rustc -O fpu_gen.rs -o "$GENBIN" 2>&1 || { echo "rustc build FAILED"; exit 2; }
  "$GENBIN" . 2>&1
fi

echo ""
echo "== [2/3] compile testbenches (iverilog) =="
iverilog -g2012 -o /tmp/tb_fpu_comb.vvp fpu.v tb_fpu_comb.v 2>/tmp/fpu_comb.cerr \
  || { echo "COMPILE-FAIL comb:"; sed 's/^/   /' /tmp/fpu_comb.cerr; exit 1; }
iverilog -g2012 -o /tmp/tb_fpu_seq.vvp  fpu.v tb_fpu_seq.v  2>/tmp/fpu_seq.cerr \
  || { echo "COMPILE-FAIL seq:";  sed 's/^/   /' /tmp/fpu_seq.cerr;  exit 1; }

echo ""
echo "== [3/3] run each op, assert DUT bits === Rust f64 bits =="
pass=0; fail=0; summary=""
run_op () {  # $1=vvp  $2=op  $3=hexfile
  local out
  out=$(vvp "$1" +OP=$2 +HEX=$3 2>&1)
  local line
  line=$(echo "$out" | grep -E '^PASS:|^FAILSUM:' | head -1)
  if echo "$line" | grep -q '^PASS:'; then
    pass=$((pass+1)); echo "  [OK]  $line"
  else
    fail=$((fail+1)); echo "  [FAIL] $2"; echo "$out" | grep -E '^FAIL' | head -6 | sed 's/^/       /'
    echo "$out" | grep -E '^FAILSUM:' | sed 's/^/       /'
  fi
  summary="$summary $2"
}

for op in lt eq;                  do run_op /tmp/tb_fpu_comb.vvp $op fpu_${op}.hex; done
for op in add sub mul div sqrt;   do run_op /tmp/tb_fpu_seq.vvp  $op fpu_${op}.hex; done

echo ""
echo "FPU ≡ f64: $pass passed, $fail failed"
[ "$fail" -eq 0 ] || exit 1
