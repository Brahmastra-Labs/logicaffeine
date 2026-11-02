#!/usr/bin/env bash
# verify-chip-variant.sh <file.lg ...> — for each program, prove the RTL soft-core at the chip config
# produces the SAME Show output as the VM golden, in RTL simulation (iverilog) — the pre-silicon gate.
# The register file is chosen to match the program's need: a program that fits the lean 32-register
# harness (tb_chip.v, FRAMES=8/REGS=32/IMEM=128/HEAP=2048) runs there; a register-heavy one runs on the
# full-chip harness (tb_chip256.v, FRAMES=64/REGS=256/IMEM=512/HEAP=2048) — the config the real bitstream
# instantiates (see nexys_soc_full.v). Running a 36-register program on a 32-register file would slice its
# high register indices down to 5 bits and silently ALIAS them onto r0..r3 (e.g. coins' r34 -> r2 clobbers
# a live list pointer), so the harness MUST cover the program's max register. Reports word count (must fit
# the chosen harness's imem) and whether it PASSES. A benchmark whose input overflows the chip heap or
# busts imem/regs shows up here, not on the board. Run the PASSing ones on silicon with
# validate-on-nexys-full.sh.
set -uo pipefail
cd "$(dirname "$0")/.."
export Z3_SYS_Z3_HEADER=/usr/include/z3.h
SIL=benchmarks/silicon
pass=0; fail=0
printf "%-16s %6s %6s %-16s %-16s %s\n" PROGRAM WORDS REGS "VM GOLDEN" "RTL SIM" RESULT
for lg in "$@"; do
  name=$(basename "$lg" .lg)
  vm=$(cargo run -q -p logicaffeine-compile --example lgvm -- "$lg" 2>/dev/null | tr '\n' ' ' | sed 's/  */ /g;s/ *$//')
  if ! cargo run -q -p logicaffeine-compile --example lg2hex -- --full "$lg" > "/tmp/cv_$name.hex" 2>/dev/null; then
    printf "%-16s %6s %6s %-16s %-16s %s\n" "$name" "-" "-" "$vm" "-" "ENCODE-ERR"; fail=$((fail+1)); continue
  fi
  nw=$(wc -l < "/tmp/cv_$name.hex")
  # Highest register index the compiled program touches (chip word = [op:8][dst:8][ra:8][rb:8]). Count EVERY
  # register operand across ALL instructions — the register file must be at least maxreg+1 wide or the core
  # slices high indices down to REGS-bits and silently ALIASES them onto low registers. Most ops carry three
  # register fields (dst,ra,rb). The exceptions carry an IMMEDIATE (not a register) in one or more fields, or
  # occupy TWO words (a trailing DATA word that must NOT be decoded as an instruction):
  #   op 5   LoadConst   : [op][dst][imm16]                 → dst only
  #   op 200 LoadConstWide: [op][dst][0][0] + [value]        → dst only, +1 data word
  #   op 34  Jump        : [op][t_lo][t_hi][0]               → no register
  #   op 35/36 JumpIf*   : [op][t_lo][cond:ra][t_hi]         → ra (cond) only
  #   op 37  Call        : [op][dst][args_start:ra][cnt:rb] + [entry] → dst,ra only (rb=count), +1 data word
  #   op 86  IterNext    : [op][dst][exit_lo:ra][exit_hi:rb] → dst only (ra/rb are the imm target)
  # Miscount any of these and the harness pick is wrong: a real 232-register sort mis-decodes Call's trailing
  # entry word as an instruction, drops the following op's registers, undercounts, and wrongly picks REGS=32.
  maxreg=$(python3 - "/tmp/cv_$name.hex" <<'PY'
import sys
words = [int(l.strip(), 16) for l in open(sys.argv[1]) if l.strip()]
maxr = 0; i = 0
while i < len(words):
    w = words[i]; op = (w >> 24) & 0xff; dst = (w >> 16) & 0xff; ra = (w >> 8) & 0xff; rb = w & 0xff
    if op == 200:      maxr = max(maxr, dst);     i += 2; continue  # LoadConstWide: dst + data word
    if op == 5:        maxr = max(maxr, dst);     i += 1; continue  # LoadConst: dst + imm16
    if op == 34:                                  i += 1; continue  # Jump: imm target only
    if op in (35, 36): maxr = max(maxr, ra);      i += 1; continue  # JumpIf*: cond=ra + imm target
    if op == 37:       maxr = max(maxr, dst, ra); i += 2; continue  # Call: dst + args_start=ra (rb=count) + entry word
    if op == 86:       maxr = max(maxr, dst);     i += 1; continue  # IterNext: dst + imm exit target in ra/rb
    maxr = max(maxr, dst, ra, rb); i += 1
print(maxr)
PY
)
  # Pick the smallest harness that covers this program's register file. tb_chip.v = REGS=32/IMEM=128;
  # tb_chip256.v = the full-chip REGS=256/IMEM=512 (matches nexys_soc_full.v).
  if [ "$maxreg" -lt 32 ]; then tb=tb_chip.v;    regs=32;  imem=128
  else                          tb=tb_chip256.v; regs=256; imem=512
  fi
  # A FLOAT program (its image carries a chip-synthetic float opcode 201..=215, or its VM golden reads as a
  # decimal) shows via the 64-bit `outf` port. tb_chip256.v then needs FLOAT_SHOW (print each float Show as
  # its raw f64 bits, since Verilog cannot reproduce f64 decimal formatting) and the fpu.v datapath linked.
  # The RTL's `f<16hex>` tokens are mapped back to decimals in Python and compared to the VM golden, so a
  # float chip variant is checked BIT-EXACT against the VM. Integer programs are unchanged.
  isfloat=$(python3 - "/tmp/cv_$name.hex" <<'PY'
import sys
ws=[int(l.strip(),16) for l in open(sys.argv[1]) if l.strip()]
print("1" if any(201<=((w>>24)&0xff)<=215 for w in ws) else "0")
PY
)
  # soft_core.v ALWAYS instantiates the fpu.v datapath (fp_cmp/addsub/mul/div/sqrt), so fpu.v must be linked
  # for EVERY program or elaboration fails. Float programs also need FLOAT_SHOW (print each float Show as its
  # 64-bit payload). NO_BIGINT stays on for the lean int path; the float path keeps the bigint blocks linked.
  float_def=""
  if [ "$isfloat" = "1" ]; then
    tb=tb_chip256.v; regs=256; imem=512; float_def="-DFLOAT_SHOW"; extra_src="fpu.v bigint_alu.v bigint_div.v"; nobig=""
  else
    extra_src="fpu.v"; nobig="-DNO_BIGINT"
  fi
  rm -f "/tmp/cv_$name.vvp"                                # never run a stale binary if this compile fails
  ( cd "$SIL" && iverilog -g2012 -o "/tmp/cv_$name.vvp" $nobig $float_def -DHEXFILE="\"/tmp/cv_$name.hex\"" -DNWORDS="$nw" \
      soft_core.v alu.v shared_divider.v crdt_merge.v $extra_src "$tb" 2>/dev/null )
  sim=$(timeout 300 vvp "/tmp/cv_$name.vvp" 2>&1 | grep -oE 'RESULT:.*' | sed 's/RESULT: *//')
  fits="ok"; [ "$nw" -gt "$imem" ] && fits=">imem"
  if [ "$isfloat" = "1" ]; then
    # Map RTL `f<16hex>` float tokens -> decimals (repr matches the VM's f64 formatting), leave ints as-is,
    # then compare to the VM golden string. TIMEOUT stays a mismatch.
    sim=$(python3 - "$sim" <<'PY'
import sys, struct
toks=sys.argv[1].split()
out=[]
for t in toks:
    if t.startswith('f') and len(t)==17:
        bits=int(t[1:],16); out.append(repr(struct.unpack('<d', struct.pack('<Q', bits))[0]))
    else:
        out.append(t)
print(' '.join(out))
PY
)
  fi
  if [ "$sim" = "$vm" ] && [ "$nw" -le "$imem" ]; then r="PASS"; pass=$((pass+1)); else r="*** FAIL($fits)"; fail=$((fail+1)); fi
  printf "%-16s %6s %6s %-16s %-16s %s\n" "$name" "$nw" "$regs" "$vm" "${sim:-timeout}" "$r"
done
echo "────────────────────────────────────────────────────────────"
echo "CHIP-SIM ≡ VM: $pass passed, $fail failed of $((pass+fail))"
