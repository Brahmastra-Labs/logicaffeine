#!/usr/bin/env bash
# Run the full soft-core synth/sim flow and record the measured numbers into a
# citable artifact: benchmarks/silicon/reports.json
#
#   { meta: {date, yosys, nextpnr},
#     blocks: { soft_core: {trellis_comb, trellis_ff, trellis_io, fmax_mhz, sim}, ... } }
#
# These are the LUT/FF/Fmax numbers any document may cite; a document that quotes them is
# checked against this file. Do not run concurrently
# with benchmarks (nextpnr is CPU-heavy and would skew timing measurements).
set -euo pipefail
cd "$(dirname "$0")/.."

LOG="$(mktemp /tmp/silicon-reports.XXXXXX.log)"
./scripts/synth-soft-core.sh 2>&1 | tee "$LOG"

python3 - "$LOG" <<'EOF'
import json, os, re, subprocess, sys

log = open(sys.argv[1]).read().splitlines()

SECTION = {
    '32-bit reversible RevAdd': 'rev_alu32',
    'multi-op reversible ALU core': 'rev_alu_core',
    'wire-opcode-driven reversible soft-core': 'rev_core',
    'general integer ALU': 'alu',
    'Keccak-f[1600] core': 'keccak_core',
    'ML-KEM NTT butterfly': 'ntt_butterfly',
    'seeded arbiter entropy core': 'seeded_arbiter',
    'DRAM command-stream executor': 'dram_executor',
    'NAND storage command-stream executor': 'storage_controller',
    'the LOGOS scalar soft-core': 'soft_core_scalar',
    'CRDT (G-Counter) join-semilattice merge': 'crdt_merge',
    'the language bounded FIFO channel': 'chan_fifo',
    'the 256-bit BigInt add/subtract/compare datapath': 'bigint_alu',
    'the 64x64 BigInt limb multiplier': 'bigint_mul',
    'the shared iterative divider': 'shared_divider',
    'the 256-bit BigInt divide block': 'bigint_div',
    'the Futamura dispatch-free residual dataflow circuit': 'fabric_horner',
    'the Futamura counted-loop CGRA': 'cgra_sumto',
    'the Futamura predicated CGRA': 'cgra_altsum',
    'while-loop CGRA (fabric-predicate controller': 'while_accw',
    'while-loop CGRA with a branch body': 'while_clampw',
    'nested-loop CGRA (hierarchical': 'nested_nsum',
    'nested-loop CGRA (inner range depends': 'nested_tri',
    'the placed-and-routed CGRA tile array': 'grid_horner',
    'the time-multiplexed CGRA array': 'tmux_linsq',
    'the data-parallel SIMT engine': 'gpu_simt',
    'the coprocessor-issuing core': 'coproc_core',
    'the flashable coproc SoC': 'coproc_soc',
}
SIM_SECTION = {
    'reversible ALU functional round-trip': 'rev_alu_core',
    'wire opcode bytes execute on the reversible soft-core': 'rev_core',
    'wire opcode bytes drive the general integer ALU': 'alu',
    'Keccak-f[1600] RTL core vs the native oracle': 'keccak_core',
    'seeded arbiter draws vs the software scheduler': 'seeded_arbiter',
    'in-order DRAM command-stream executor': 'dram_executor',
    'in-order NAND storage command-stream executor': 'storage_controller',
    'the LOGOS scalar soft-core runs a real bytecode program': 'soft_core_scalar',
    'the full 113-op LOGOS soft-core runs a real bytecode program': 'soft_core_full',
    'CRDT (G-Counter) join-semilattice merge': 'crdt_merge',
    'the language bounded FIFO channel': 'chan_fifo',
    'the 256-bit BigInt add/subtract/compare datapath': 'bigint_alu',
    'the 64x64 BigInt limb multiplier': 'bigint_mul',
    'the shared iterative divider': 'shared_divider',
    'the 256-bit BigInt divide block': 'bigint_div',
    'the Futamura dispatch-free residual dataflow circuit': 'fabric_horner',
    'the Futamura counted-loop CGRA': 'cgra_sumto',
    'the Futamura predicated CGRA': 'cgra_altsum',
    'while-loop CGRA (fabric-predicate controller': 'while_accw',
    'while-loop CGRA with a branch body': 'while_clampw',
    'nested-loop CGRA (hierarchical': 'nested_nsum',
    'nested-loop CGRA (inner range depends': 'nested_tri',
    'the placed-and-routed CGRA tile array': 'grid_horner',
    'the time-multiplexed CGRA array': 'tmux_linsq',
    'the data-parallel SIMT engine': 'gpu_simt',
    'the coprocessor-issuing core': 'coproc_core',
    'the flashable coproc SoC': 'coproc_soc',
}

blocks, cur, cur_sim = {}, None, None
util = re.compile(r'TRELLIS_(COMB|FF|IO):\s+(\d+)/')
fmax = re.compile(r"Max frequency for clock\s+'[^']*':\s+([\d.]+)\s+MHz")
for line in log:
    if line.startswith('== FPGA:') or line.startswith('== SIM:'):
        cur, cur_sim = None, None
        for key, name in SECTION.items():
            if key in line:
                cur = blocks.setdefault(name, {})
        for key, name in SIM_SECTION.items():
            if key in line:
                cur_sim = blocks.setdefault(name, {})
        continue
    m = util.search(line)
    if m and cur is not None:
        cur['trellis_' + m.group(1).lower()] = int(m.group(2))
    m = fmax.search(line)
    if m and cur is not None:
        cur['fmax_mhz'] = float(m.group(1))  # last one per block wins (post-route)
    if cur_sim is not None and 'PASS' in line:
        cur_sim['sim'] = 'PASS'
    if cur_sim is not None and 'FAIL' in line:
        cur_sim['sim'] = 'FAIL'

def tool(cmd):
    try:
        out = subprocess.run(cmd, capture_output=True, text=True).stdout
        return out.splitlines()[0].strip() if out else 'unknown'
    except OSError:
        return 'absent'

date = subprocess.run(['date', '-u', '+%Y-%m-%dT%H:%M:%SZ'],
                      capture_output=True, text=True).stdout.strip()
# The synth flow must have actually reached the core blocks (guard a broken / partial run).
assert 'soft_core_scalar' in blocks and 'rev_core' in blocks, \
    f'flow did not reach all blocks — got {sorted(blocks)}'
for name in ('soft_core_scalar', 'rev_core', 'alu'):
    assert blocks.get(name, {}).get('sim') == 'PASS', f'{name} sim did not PASS'

# MERGE, never truncate. Preserve every block and field already in reports.json that
# THIS flow does not re-measure — the hand-maintained blocks (secded / keccak_top /
# double_buffer) and the human-written 'note' on each block. Only the freshly measured
# fields (trellis_*, fmax_mhz, sim) are overlaid onto the matching block. A full regen can
# therefore never silently drop a block (the reproducibility trap this closes).
out = 'benchmarks/silicon/reports.json'
prior = {}
if os.path.exists(out):
    try:
        prior = json.load(open(out)).get('blocks', {})
    except (ValueError, OSError):
        prior = {}
merged = {k: dict(v) for k, v in prior.items()}
for name, meas in blocks.items():
    merged.setdefault(name, {}).update(meas)
dropped = set(prior) - set(merged)
assert not dropped, f'MERGE INVARIANT VIOLATED — would drop {sorted(dropped)}'

doc = {'meta': {'date': date, 'yosys': tool(['yosys', '-V']),
                'nextpnr': tool(['nextpnr-ecp5', '--version'])},
       'blocks': merged}
with open(out, 'w') as f:
    json.dump(doc, f, indent=2)
    f.write('\n')
preserved = sorted(set(prior) - set(blocks))
print(f'wrote {out} (merged: {len(merged)} blocks, {len(preserved)} preserved un-measured: {preserved}):')
for name, b in sorted(merged.items()):
    print(f'  {name}: {b}')
EOF
