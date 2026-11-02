#!/usr/bin/env bash
# Synthesize the FULL 113-op LOGOS soft-core for the Nexys A7-100T (Xilinx Artix-7
# XC7A100T) and record whether it fits. The board has 15,850 slices x (4 six-input
# LUTs + 8 FFs) = 63,400 LUT6 / 126,800 FF, plus 240 DSP48E1 and 135 BRAM36.
#
# Unlike the ECP5 (4-input TRELLIS_COMB), the Artix-7's 6-input LUTs pack the same
# logic ~3x denser, so the core that is 128,829 4-LUTs on the ECP5 (154%, does NOT
# fit) maps to ~38k LUT6 on the Artix-7 (~60%, FITS). yosys `synth_xilinx` does the
# 7-series tech-map + DSP inference; the final .bit needs Vivado for place-and-route
# (there is no open ecppack path for 7-series), but the LUT6/FF/DSP fit is decided here.
#
# Writes benchmarks/silicon/nexys_a7_fit.json — the citable fit artifact (nexys_fit_lock
# pins the doc claims to it). Runs the FULL default config (HEAP 256): the real 113-op core.
set -euo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
OUT="$SIL/nexys_a7_fit.json"
LOG="$(mktemp /tmp/nexys-fit.XXXXXX.log)"

echo "== synth_xilinx: full 113-op soft-core -> Artix-7 XC7A100T (Nexys A7-100T) =="
yosys -p "
  read_verilog $SIL/soft_core.v $SIL/alu.v $SIL/shared_divider.v $SIL/crdt_merge.v $SIL/bigint_alu.v $SIL/bigint_div.v
  synth_xilinx -family xc7 -top soft_core
  tee -o /dev/stdout stat -top soft_core
" 2>&1 | tee "$LOG"

python3 - "$LOG" "$OUT" <<'EOF'
import json, re, subprocess, sys
log, out = open(sys.argv[1]).read().splitlines(), sys.argv[2]

# Parse the LAST "Number of cells:" stat block (the final tech-mapped netlist).
blocks, cur = [], None
for line in log:
    if 'Number of cells:' in line:
        cur = {}
        blocks.append(cur)
    elif cur is not None:
        m = re.match(r'\s+(\S+)\s+(\d+)\s*$', line)
        if m:
            cur[m.group(1)] = int(m.group(2))
assert blocks, 'no stat block found — synth failed'
cells = blocks[-1]

lut = sum(v for k, v in cells.items() if re.fullmatch(r'LUT[1-6]', k))
ff  = sum(v for k, v in cells.items() if k in ('FDRE', 'FDSE', 'FDCE', 'FDPE'))
dsp = sum(v for k, v in cells.items() if k.startswith('DSP48'))
carry = cells.get('CARRY4', 0)
bram = sum(v for k, v in cells.items() if k.startswith('RAMB'))

# Nexys A7-100T budget.
LUT_CAP, FF_CAP, DSP_CAP, BRAM_CAP = 63400, 126800, 240, 135
fits = lut <= LUT_CAP and ff <= FF_CAP and dsp <= DSP_CAP and bram <= BRAM_CAP

def ver():
    try:
        return subprocess.run(['yosys', '-V'], capture_output=True, text=True).stdout.splitlines()[0].strip()
    except OSError:
        return 'unknown'

date = subprocess.run(['date','-u','+%Y-%m-%dT%H:%M:%SZ'], capture_output=True, text=True).stdout.strip()
doc = {
    'meta': {'date': date, 'yosys': ver(),
             'board': 'Nexys A7-100T', 'part': 'xc7a100t-1csg324c',
             'flow': 'yosys synth_xilinx -family xc7 (P&R+bitstream in Vivado)',
             'config': 'full default (HEAP 256, FRAMES 16, MAXELT 16, MAXPAIR 8, MAXKEY 8)'},
    'budget': {'lut6': LUT_CAP, 'ff': FF_CAP, 'dsp48': DSP_CAP, 'bram36': BRAM_CAP},
    'soft_core_full': {
        'lut6': lut, 'ff': ff, 'dsp48': dsp, 'carry4': carry, 'bram36': bram,
        'lut6_pct': round(lut * 100.0 / LUT_CAP, 1),
        'ff_pct': round(ff * 100.0 / FF_CAP, 1),
        'fits': fits,
    },
}
with open(out, 'w') as f:
    json.dump(doc, f, indent=2); f.write('\n')
verdict = 'FITS' if fits else 'DOES NOT FIT'
print(f"\n== {out} ==")
print(f"  LUT6 {lut}/{LUT_CAP} ({doc['soft_core_full']['lut6_pct']}%)  "
      f"FF {ff}/{FF_CAP}  DSP48 {dsp}/{DSP_CAP}  BRAM36 {bram}/{BRAM_CAP}  CARRY4 {carry}")
print(f"  => the full 113-op LOGOS soft-core {verdict} on the Nexys A7-100T")
sys.exit(0 if fits else 1)
EOF