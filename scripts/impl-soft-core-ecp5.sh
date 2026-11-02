#!/usr/bin/env bash
# Full ECP5 implementation of the WHOLE 113-op LOGOS soft-core (post efficiency-refactor).
#
# The coarse `synth_soft_core_full.ys` proves the memories infer as RAM (proc+opt+memory, 3122 cells).
# THIS script takes the same refactored RTL all the way through the real Lattice ECP5 flow —
# yosys synth_ecp5 -> nextpnr-ecp5 --85k -> ecppack — to honest whole-CPU numbers (LUT/FF/BRAM/IO +
# Fmax) and a LOADABLE bitstream. The demo memory config (chparam) keeps the per-cell address decode
# bounded; the datapath + decode + microcode FSMs (all 113 ops) are at full logic complexity, so the
# LUT/Fmax figures are representative — only the BRAM count scales with the heap/frame sizes.
#
# Tools: yosys + nextpnr-ecp5 on PATH; ecppack from the in-repo prjtrellis build.
set -euo pipefail
cd "$(dirname "$0")/../benchmarks/silicon"

TRELLIS="$(cd ../.. && pwd)/tools/prjtrellis"
ECPPACK="$TRELLIS/libtrellis/ecppack"
export TRELLIS_DB="$TRELLIS/database"
LOG=soft_core_ecp5_impl.log
: > "$LOG"

echo "== [1/3] yosys synth_ecp5: whole soft_core.v + alu.v -> ECP5 netlist ==" | tee -a "$LOG"
cat > synth_soft_core_ecp5.ys <<'EOF'
read_verilog soft_core.v alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v
chparam -set HEAP 64 -set FRAMES 4 -set MAXELT 8 -set MAXPAIR 4 -set MAXKEY 4 soft_core
synth_ecp5 -top soft_core -json soft_core_ecp5.json
EOF
/usr/bin/time -v yosys -q synth_soft_core_ecp5.ys >> "$LOG" 2>&1 || { echo "SYNTH FAILED"; tail -30 "$LOG"; exit 1; }
echo "   synth OK -> soft_core_ecp5.json" | tee -a "$LOG"

echo "== [2/3] nextpnr-ecp5 --85k: place-and-route (utilization + Fmax) ==" | tee -a "$LOG"
/usr/bin/time -v nextpnr-ecp5 --85k --json soft_core_ecp5.json \
  --lpf-allow-unconstrained --textcfg soft_core_ecp5.config >> "$LOG" 2>&1 \
  || { echo "PNR FAILED"; tail -30 "$LOG"; exit 1; }
echo "   pnr OK -> soft_core_ecp5.config" | tee -a "$LOG"

echo "== [3/3] ecppack: -> loadable bitstream ==" | tee -a "$LOG"
"$ECPPACK" soft_core_ecp5.config soft_core_ecp5.bit >> "$LOG" 2>&1 \
  || { echo "ECPPACK FAILED"; tail -30 "$LOG"; exit 1; }
echo "   pack OK -> soft_core_ecp5.bit ($(stat -c %s soft_core_ecp5.bit) bytes)" | tee -a "$LOG"

echo "== IMPLEMENTATION COMPLETE — whole 113-op LOGOS soft-core on ECP5-85K ==" | tee -a "$LOG"
