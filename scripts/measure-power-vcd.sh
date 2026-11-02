#!/usr/bin/env bash
# measure-power-vcd — signoff power of one hardened block, end to end, from open tools:
#   the post-route netlist its signoff ledger SHIPS  +  switching activity from a gate-level
#   simulation of that netlist at its signoff clock (VCD)  ->  OpenSTA report_power at every corner.
# The open equivalent of Synopsys PrimePower / Cadence Voltus; every artifact it touches (the
# LVS-matched netlist, the extracted SPEF per corner, the PDK Liberty per corner, the VCD) is standard
# and inspectable.
#
#   scripts/measure-power-vcd.sh <design>        # e.g. decision_hft_realbist  (default)
#
# The measurement itself is scripts/measure-silicon-power.py (its JSON report is kept beside the VCD
# under logs/power/<design>/); scripts/record-silicon-power.py turns reports into the power ledgers.
set -euo pipefail
cd "$(dirname "$0")/.."
DESIGN="${1:-decision_hft_realbist}"
mkdir -p "logs/power/$DESIGN"
python3 scripts/measure-silicon-power.py "$DESIGN" > "logs/power/$DESIGN/report.json"
python3 - "logs/power/$DESIGN/report.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print(f"design: {d['block']}   run: {d['run']}   GDS {d['gds_bytes']} bytes")
print(f"signoff clock: {d['clock_period_ns']:g} ns ({d['clock_mhz']:g} MHz)   gate simulation: {d['simulated_cycles']} cycles, "
      f"signature {'X' if d['signature_has_x'] else 'X-free'}")
for corner, v in d["corners"].items():
    vcd = v.get("vcd")
    print(f"  {corner:14} default-activity {v['default']['total']['total'] * 1e3:8.3f} mW"
          + (f"   VCD-measured {vcd['total']['total'] * 1e3:8.3f} mW   ({v.get('annotated_pins')}/{v.get('design_pins')} pins annotated)"
             if vcd else ""))
print(f"report: {sys.argv[1]}")
PY
