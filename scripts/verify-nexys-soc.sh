#!/usr/bin/env bash
# Verify the board-runnable SoC (benchmarks/silicon/nexys_soc.v) end-to-end in simulation, plus the
# host driver that talks to it. This is the last-mile lock for "send LOGOS bytecode to the chip,
# run it, read back the Show values AND the on-chip run-cycle count":
#
#   [1/2] iverilog tb_nexys_soc.v — resets, streams 3 real bytecode programs in over UART, runs each
#         on the full 113-op soft_core, and checks the Show values + the 4-byte run-cycle count that
#         come back. The wire framing here is byte-identical to what the board produces.
#   [2/2] nexys-host.py --selftest — the Python host's frame_to_send / parse_reply must agree with
#         nexys_soc.v's L_SEND framing (sync, count, values, 4 cycle bytes, end) byte-for-byte, so the
#         simulation and the real board round-trip are the same protocol by construction.
#
# A green run means the SoC RTL, the testbench, and the host driver all agree — no silent drift.
# Requires iverilog for step 1; step 2 is pure Python. Exits non-zero on any failure.
set -uo pipefail
cd "$(dirname "$0")/.."

SIL=benchmarks/silicon
DEPS="nexys_soc.v soft_core.v alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v fpu.v"

echo "== [1/3] iverilog tb_nexys_soc.v — UART load/run/readback + cycle count =="
if command -v iverilog >/dev/null 2>&1; then
  ( cd "$SIL" \
    && iverilog -g2012 -s tb -o /tmp/tb_nexys_soc.vvp $DEPS tb_nexys_soc.v \
    && vvp /tmp/tb_nexys_soc.vvp ) 2>&1 | grep -vE 'readmemh' | tee /tmp/tb_nexys_soc.out
  grep -q "tb_nexys_soc: PASS" /tmp/tb_nexys_soc.out || { echo "FAIL: tb_nexys_soc did not PASS"; exit 1; }
else
  echo "iverilog not found — skipping the RTL simulation step."
fi

echo ""
echo "== [1b] iverilog tb_nexys_soc_full.v — FULL (FPU) SoC: int + FLOAT Show round-trip over the tagged UART =="
# nexys_soc_full wraps the FPU-bearing soft_core and carries a per-show <tag>+width so a FLOAT Show (ShowF,
# raw IEEE-754 f64 bits) round-trips interleaved-in-order with int Shows. pi_leibniz's bits 0x40091FEF0A9265EB
# must reassemble exactly; the host's fmt_f64() then renders the pi string. Same protocol as nexys-host.py.
if command -v iverilog >/dev/null 2>&1; then
  # nexys_soc_full.v must precede soft_core.v so its NO_BIGINT / SOFT_CORE_PROG_LOAD defines are set.
  ( cd "$SIL" \
    && iverilog -g2012 -s tb -o /tmp/tb_nexys_soc_full.vvp \
         nexys_soc_full.v soft_core.v alu.v shared_divider.v crdt_merge.v fpu.v tb_nexys_soc_full.v \
    && vvp /tmp/tb_nexys_soc_full.vvp ) 2>&1 | grep -vE 'readmemh' | tee /tmp/tb_nexys_soc_full.out
  grep -q "tb_nexys_soc_full: PASS" /tmp/tb_nexys_soc_full.out || { echo "FAIL: tb_nexys_soc_full did not PASS"; exit 1; }
else
  echo "iverilog not found — skipping the full-SoC float round-trip step."
fi

echo ""
echo "== [1c] iverilog tb_nexys_wide.v + tb_nexys_huge.v — the 2-byte-escape loader + the 4096-word BRAM imem =="
# tb_nexys_wide: a 256-word program (>255, impossible on the 1-byte loader) via the escape → Show=8001.
# tb_nexys_huge: a 3004-word program (>2047, the OLD imem ceiling) on the sync-read BRAM imem → Show=3000.
# Together they lock the unbounded-program path: escape length framing AND the block-RAM capacity gain.
if command -v iverilog >/dev/null 2>&1; then
  for tb in tb_nexys_wide tb_nexys_huge; do
    ( cd "$SIL" \
      && iverilog -g2012 -s tb -o "/tmp/${tb}.vvp" \
           nexys_soc_full.v soft_core.v alu.v shared_divider.v crdt_merge.v fpu.v "${tb}.v" \
      && vvp "/tmp/${tb}.vvp" ) 2>&1 | grep -vE 'readmemh|Padding' | tee "/tmp/${tb}.out"
    grep -q "${tb}: PASS" "/tmp/${tb}.out" || { echo "FAIL: ${tb} did not PASS"; exit 1; }
  done
else
  echo "iverilog not found — skipping the escape-loader / capacity step."
fi

echo ""
echo "== [2/3] nexys-host.py --selftest — host framing matches the RTL protocol =="
python3 scripts/nexys-host.py --selftest || { echo "FAIL: host selftest"; exit 1; }

echo ""
echo "== [3/3] run the demo .hex programs through the SoC sim (tb_nexys_run) and check results =="
if command -v iverilog >/dev/null 2>&1; then
  ( cd "$SIL" \
    && iverilog -g2012 -s tb -o /tmp/tb_nexys_run.vvp $DEPS tb_nexys_run.v ) || { echo "FAIL: compile tb_nexys_run"; exit 1; }
  # program : expected Show values (space-separated) — each proven ≡ VM (see rtl.rs differential)
  demos=( "sum_loop.hex:15" "expr_13.hex:13" "const_7_42.hex:7 42" )
  for d in "${demos[@]}"; do
    f="programs/${d%%:*}"; exp="${d##*:}"
    n=$(grep -cE '^[0-9a-fA-F]' "$SIL/$f")
    got=$( cd "$SIL" && vvp /tmp/tb_nexys_run.vvp +HEX="$f" +N="$n" 2>&1 | sed -n 's/^RESULT:\s*\([0-9 ]*\)  CYCLES.*/\1/p' | sed 's/ *$//' )
    if [ "$got" = "$exp" ]; then
      echo "  OK   ${d%%:*} -> [$got]"
    else
      echo "  FAIL ${d%%:*} -> [$got] expected [$exp]"; exit 1
    fi
  done
else
  echo "iverilog not found — skipping the demo-program simulation step."
fi

echo ""
echo "== [4/4] SCALAR SoC (nexys_soc_scalar) — the board-runnable open-flow build, PROVEN on real xc7a100t =="
# nexys_soc_scalar wraps soft_core_scalar (routable LOGOS CPU, no async dmem/bigint) with the same UART
# loader + cycle counter. This is the design that actually runs on the Nexys A7 via yosys+nextpnr-xilinx.
# Same demos must run ≡ VM in sim (the sim twin instantiates it via -DSOC_MODULE=nexys_soc_scalar).
if command -v iverilog >/dev/null 2>&1; then
  ( cd "$SIL" && iverilog -g2012 -s tb -DSOC_MODULE=nexys_soc_scalar -o /tmp/tb_scalar_run.vvp \
      nexys_soc_scalar.v soft_core_scalar.v alu.v tb_nexys_run.v ) || { echo "FAIL: compile scalar sim"; exit 1; }
  for d in "sum_loop.hex:15" "expr_13.hex:13" "const_7_42.hex:7 42"; do
    f="programs/${d%%:*}"; exp="${d##*:}"; n=$(grep -cE '^[0-9a-fA-F]' "$SIL/$f")
    got=$( cd "$SIL" && vvp /tmp/tb_scalar_run.vvp +HEX="$f" +N="$n" 2>&1 | sed -n 's/^RESULT:\s*\([0-9 ]*\)  CYCLES.*/\1/p' | sed 's/ *$//' )
    [ "$got" = "$exp" ] && echo "  OK   scalar ${d%%:*} -> [$got]" || { echo "  FAIL scalar ${d%%:*} -> [$got] expected [$exp]"; exit 1; }
  done
else
  echo "iverilog not found — skipping the scalar SoC simulation step."
fi

echo ""
echo "== [4b] SCALAR CORE opcode-coverage tbs — every EMITTED scalar opcode runs ≡ VM on soft_core_scalar.v =="
# The SoC demos above exercise only arithmetic + loop control. The scalar-encoder tb corpus
# (tb_scalar_gen_*.v, auto-generated from compile_to_scalar_words ≡ VM under LOGOS_EMIT_SC_TB) closes the
# rest — the BITWISE / SHIFT / non-`<=` COMPARISON / `Swap` opcodes the scalar encoder EMITS and
# soft_core_scalar.v DECODES but no demo touched. This is GUARD B of scalar_fifty_million_dollar_test.rs:
# without it a hardware bug in (e.g.) the BitXor arm would ship silently to the Sky130 tape-out. Each tb
# runs standalone on soft_core_scalar.v + alu.v and checks its Show values against the VM golden.
if command -v iverilog >/dev/null 2>&1; then
  shopt -s nullglob
  scalar_tbs=( "$SIL"/tb_scalar_gen_*.v )
  shopt -u nullglob
  if [ ${#scalar_tbs[@]} -eq 0 ]; then
    echo "FAIL: no tb_scalar_gen_*.v — regenerate with LOGOS_EMIT_SC_TB=1 cargo test -p logicaffeine-compile --lib scalar_encoder_tbs_match_vm"; exit 1
  fi
  for tb in "${scalar_tbs[@]}"; do
    b=$(basename "$tb" .v); nm=${b#tb_scalar_gen_}
    ( cd "$SIL" && iverilog -g2012 -s tb -o "/tmp/${b}.vvp" soft_core_scalar.v alu.v "${b}.v" ) \
      || { echo "  FAIL scalar-cov ${nm}: compile"; exit 1; }
    out=$( cd "$SIL" && vvp "/tmp/${b}.vvp" 2>&1 )
    if echo "$out" | grep -q "PASS: ${nm}"; then
      echo "  OK   scalar-cov ${nm}"
    else
      echo "  FAIL scalar-cov ${nm} -> $(echo "$out" | grep -E 'FAIL|seen' | head -3)"; exit 1
    fi
  done
else
  echo "iverilog not found — skipping the scalar opcode-coverage step."
fi

echo ""
echo "== [5/6] iverilog tb_sevenseg.v — the 8-digit 7-seg display driver (hex table, anode cycle, blank) =="
if command -v iverilog >/dev/null 2>&1; then
  ( cd "$SIL" && iverilog -g2012 -s tb_sevenseg -o /tmp/tb_sevenseg.vvp tb_sevenseg.v sevenseg.v \
    && vvp /tmp/tb_sevenseg.vvp ) 2>&1 | tee /tmp/tb_sevenseg.out
  grep -q "PASS: sevenseg" /tmp/tb_sevenseg.out || { echo "FAIL: tb_sevenseg did not PASS"; exit 1; }
else
  echo "iverilog not found — skipping the sevenseg step."
fi

echo ""
echo "== [6/6] iverilog tb_nexys_tformer.v — the DATA-segment weight loader: loaded() reads streamed weights =="
# The transformer's weights arrive as a UART DATA SEGMENT preloaded into the heap; the compiled `loaded()`
# reads them at pointer 0. Checks the NEW RTL (DATA phase + core data-preload port + hp_init + disp_val) on
# the real full core with a tiny `loaded()` reader (sum item 1..3 of loaded() over blob [3,10,20,30] = 60).
# The WHOLE transformer's correctness is locked separately on the soft-core model (tiny_transformer_lock).
if command -v iverilog >/dev/null 2>&1; then
  LDLG=$(mktemp /tmp/loaded_XXXX.lg)
  printf '## To native loaded () -> Seq of Int\n## Main\nLet s be 0.\nLet i be 1.\nWhile i is at most 3:\n    Set s to s + item i of loaded().\n    Set i to i + 1.\nShow s.\n' > "$LDLG"
  cargo run -q -p logicaffeine-compile --example lg2hex -- --full "$LDLG" 2>/dev/null > "$SIL/tformer_prog.hex"
  N=$(grep -c . "$SIL/tformer_prog.hex")
  printf '00000003\n0000000a\n00000014\n0000001e\n' > "$SIL/tformer_data.hex"
  ( cd "$SIL" && iverilog -g2012 -s tb -o /tmp/tb_nexys_tformer.vvp \
      nexys_soc_tformer.v soft_core.v alu.v shared_divider.v crdt_merge.v fpu.v tb_nexys_tformer.v \
    && vvp /tmp/tb_nexys_tformer.vvp +N="$N" +ND=4 +EXP=60 ) 2>&1 | grep -vE 'readmemh|Padding' | tee /tmp/tb_nexys_tformer.out
  grep -q "tb_nexys_tformer: PASS" /tmp/tb_nexys_tformer.out || { echo "FAIL: tb_nexys_tformer did not PASS"; exit 1; }
  rm -f "$SIL/tformer_prog.hex" "$SIL/tformer_data.hex" "$LDLG"
else
  echo "iverilog not found — skipping the DATA-segment loader step."
fi

echo "== [7/7] iverilog tb_nexys_matmul_soc — UART two-vector int8 dot-product accelerator (the matmul demo) =="
if command -v iverilog >/dev/null 2>&1; then
  # the compute core is AUTO-EMITTED FROM LOGOS (int8-native datapath, 1 DSP/MAC).
  cargo run -q -p logicaffeine-compile --example emit_matmul_core -- "$SIL/cgra_dot.v" >/dev/null 2>&1 \
    || { echo "FAIL: emit cgra_dot from LOGOS"; exit 1; }
  # the signed-decimal 7-seg display driver (renders a·b on the board), unit-checked first
  ( cd "$SIL" && iverilog -g2012 -s tb -o /tmp/tb_ssdec.vvp sevenseg_dec.v tb_sevenseg_dec.v \
    && vvp /tmp/tb_ssdec.vvp ) 2>&1 | tee /tmp/tb_ssdec.out
  grep -q "PASS: sevenseg_dec" /tmp/tb_ssdec.out || { echo "FAIL: sevenseg_dec did not PASS"; exit 1; }
  # the whole SoC: UART operands → int8 DSP dot → result on the 7-seg display
  ( cd "$SIL" \
    && iverilog -g2012 -s tb -o /tmp/tb_matmul_soc.vvp cgra_dot.v sevenseg_dec.v nexys_matmul_soc.v tb_nexys_matmul_soc.v \
    && vvp /tmp/tb_matmul_soc.vvp ) 2>&1 | tee /tmp/tb_matmul_soc.out
  grep -q "PASS: matmul SoC" /tmp/tb_matmul_soc.out || { echo "FAIL: tb_nexys_matmul_soc did not PASS"; exit 1; }
else
  echo "iverilog not found — skipping the matmul-SoC step."
fi

echo ""
echo "ALL GREEN — nexys_soc.v ≡ tb_nexys_soc.v ≡ nexys-host.py; demo bytecode runs ≡ VM on both the full and"
echo "scalar SoCs; the 7-seg driver + the DATA-segment weight loader (nexys_soc_tformer) pass. The scalar SoC"
echo "(nexys_soc_scalar) is the open-flow build proven on the real Nexys A7-100T."
