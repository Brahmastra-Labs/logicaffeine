#!/usr/bin/env bash
# Implement the ROUTABLE scalar LOGOS core all the way to a loadable ECP5 bitstream.
#
# soft_core_scalar.v is the fetch-decode-execute LOGOS CPU over the scalar ISA subset (reversible core,
# LoadConst/Move, branches, Call/Return recursion, ALU via the proven alu.v) — no async-dmem collection
# coprocessor, so it PLACES-AND-ROUTES where the full soft_core.v cannot (see SILICON_COMPLETION E2d/E2f).
#
#   1. derive tb_scalar_*.v from the model-generated tb_gen_*.v (drift-free: same vectors the ≡VM
#      differential asserts), point them at soft_core_scalar, and iverilog each -> PASS;
#   2. yosys synth_ecp5 -> nextpnr-ecp5 --85k (utilization + Fmax) -> ecppack (bitstream).
set -uo pipefail
cd "$(dirname "$0")/../benchmarks/silicon"

TRELLIS="$(cd ../.. && pwd)/tools/prjtrellis"
ECPPACK="$TRELLIS/libtrellis/ecppack"
DB="$TRELLIS/database"
SCALAR_PROGS="sum_loop expr e2 sumdown"   # the scalar differential programs (incl. recursion)

echo "== [1/2] iverilog: scalar core runs the scalar programs ≡ the model (drift-free TBs) =="
pass=0; fail=0
for p in $SCALAR_PROGS; do
  [ -f "tb_gen_${p}.v" ] || { echo "  MISSING tb_gen_${p}.v — run scripts/verify-soft-core-rtl.sh first"; fail=$((fail+1)); continue; }
  sed 's/^\(\s*\)soft_core dut(/\1soft_core_scalar dut(/' "tb_gen_${p}.v" > "tb_scalar_${p}.v"
  if iverilog -o "/tmp/sc_${p}.vvp" soft_core_scalar.v alu.v "tb_scalar_${p}.v" 2>"/tmp/sc_${p}.err" \
     && vvp "/tmp/sc_${p}.vvp" 2>&1 | grep -q "^PASS:"; then
    echo "  PASS $p"; pass=$((pass+1))
  else
    echo "  FAIL $p"; sed 's/^/      /' "/tmp/sc_${p}.err" | head -4; fail=$((fail+1))
  fi
done
[ "$fail" -ne 0 ] && { echo "SCALAR SIM FAILED ($fail)"; exit 1; }
echo "  all $pass scalar programs PASS on soft_core_scalar.v"

command -v nextpnr-ecp5 >/dev/null 2>&1 || { echo "nextpnr-ecp5 absent — skipping implementation."; exit 0; }
echo "== [2/2] ECP5 implementation: synth_ecp5 -> nextpnr --85k -> ecppack =="
yosys -q -p "read_verilog soft_core_scalar.v alu.v; synth_ecp5 -top soft_core_scalar -json soft_core_scalar.json" \
  || { echo "SYNTH FAILED"; exit 1; }
nextpnr-ecp5 --85k --json soft_core_scalar.json --lpf-allow-unconstrained --textcfg soft_core_scalar.config 2>&1 \
  | grep -iE "ERROR|Max frequency|TRELLIS_(COMB|FF|IO):|DP16KD|Device utilisation" || { echo "PNR FAILED"; exit 1; }
"$ECPPACK" --db "$DB" soft_core_scalar.config soft_core_scalar.bit \
  && echo "== BITSTREAM: benchmarks/silicon/soft_core_scalar.bit ($(stat -c %s soft_core_scalar.bit) bytes) — loadable on a ULX3S ECP5-85K =="
