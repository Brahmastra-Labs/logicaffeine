#!/usr/bin/env bash
# characterize-vdd-sweep — the Vdd/energy sweep behind benchmarks/silicon/energy_efficiency.json.
#
# Power vs Vdd from real silicon models, not a formula: the block's shipped netlist is gate-level
# simulated at its signoff clock and VCD-annotated OpenSTA report_power runs once per Sky130 PVT
# corner (ss 1.60 V / tt 1.80 V / ff 1.95 V, each with its own parasitics). That measurement is
# scripts/measure-silicon-power.py; scripts/record-silicon-power.py then writes the `vdd_sweep`, the
# efficiency point and every ledger that quotes them, and energy_efficiency_lock.rs re-derives every
# efficiency figure so nothing here can be fudged. BENCH-BOX ONLY: needs the sky130A PDK Liberty, the
# OpenLane/OpenSTA docker image and the block's signoff run (quiet the box first).
#
#   scripts/characterize-vdd-sweep.sh <design>     # default: decision_hft_realbist
set -euo pipefail
cd "$(dirname "$0")/.."
DESIGN="${1:-decision_hft_realbist}"
python3 scripts/measure-silicon-power.py "$DESIGN" > "logs/power-$DESIGN.json"
python3 scripts/record-silicon-power.py
