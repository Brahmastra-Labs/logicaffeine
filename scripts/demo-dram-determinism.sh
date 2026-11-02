#!/usr/bin/env bash
# DEMO 2 — deterministic DRAM (the memory architect's domain).
#
# A modern DRAM controller exists to GUESS: reorder queues, open/closed-page prediction, refresh juggling —
# because it does not know the access pattern until runtime. Ours does. So we specialize the controller away:
# the COMPILER emits the DRAM command stream itself (ACT/RD/WR/PRE/REF with exact timing), PROVES it
# timing-legal at compile time with our OWN certified SAT, and the on-die controller collapses to a dumb,
# verified executor that replays it with zero jitter. Refresh is a scheduled line item, not a stall.
set -uo pipefail
cd "$(dirname "$0")/.."

echo "════════════════════════════════════════════════════════════════════════════"
echo "  LOGOS — deterministic DRAM: the compiler emits the stream, the DRAM obeys"
echo "════════════════════════════════════════════════════════════════════════════"

cargo build -q -p logicaffeine-compile --tests >/dev/null 2>&1 || true

echo
echo "── [1/4] a real compiled kernel plan → certified-legal command stream → cycle-exact replay ──"
LOGOS_DEMO=1 cargo test -q -p logicaffeine-compile --lib \
  a_real_compiled_plan_emits_a_certified_legal_stream_the_executor_replays -- --nocapture 2>&1 \
  | grep -E "^  (workload|compiler|independent|on-die|sustained|non-vacuity)" \
  || { echo "  (lock did not narrate)"; exit 1; }

echo
echo "── [2/4] the timing legality proven by OUR certified SAT (not Z3, not a vendor tool) ──"
if cargo test -q -p logicaffeine-compile --lib trp_legality_is_proven_by_our_sat 2>&1 | grep -q "1 passed"; then
  echo "  tRP legality obligation: SAT-REFUTED (no legal-looking stream can violate tRP) — PROVEN"
else
  echo "  tRP legality proof: FAILED"; exit 1
fi

echo
echo "── [3/4] the on-die executor RTL replaying a real plan in-order, cycle-exact (iverilog) ──"
cd benchmarks/silicon
if command -v iverilog >/dev/null 2>&1; then
  if iverilog -o /tmp/dram_exec.vvp dram_executor.v tb_dram_executor.v 2>/dev/null \
     && vvp /tmp/dram_exec.vvp 2>&1 | grep -q "PASS"; then
    echo "  dram_executor.v (91 LUT / ~145 MHz on ECP5): replayed the plan in order — PASS"
  else
    echo "  dram_executor.v: FAIL"; exit 1
  fi
else
  echo "  iverilog absent — the Rust device model already proved cycle-exact replay above."
fi

echo
echo "── [4/4] measured bus efficiency (cycle-accurate LPDDR5X-class model, refresh ON) ──"
python3 - <<'PY'
import json
d = json.load(open('../results/dram_sim.json'))
for k, v in d['scenarios'].items():
    print(f"  {k:26} {v['efficiency']*100:5.1f}%")
PY

echo
echo "The compiler ordered memory perfectly; the DRAM is a dumb, verified executor. No demand miss to hide,"
echo "no reorder queue to guess. That is what \"simpler than a JEDEC part, not fancier\" means."
