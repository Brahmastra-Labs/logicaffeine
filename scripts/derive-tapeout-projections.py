#!/usr/bin/env python3
"""Re-derive every "what this runs at on the chip" projection from the soft-core's signoff clock.

The soft-core is deterministic — a program's cycle count equals its VM op count — so its time on the
hardened part is exactly cycles x the clock period the slow corner meets. That period is the one
`soft_core_scalar` is recorded at in `sky130_signoff.json` (built at its registered period,
`signoff_ready_lock`). Nothing is re-measured here: the cycle counts already recorded in each artifact
are kept, and only the projection derived from them is recomputed.

    python3 scripts/derive-tapeout-projections.py            # rewrite both artifacts
    python3 scripts/derive-tapeout-projections.py --check    # exit 1 if either is stale
"""
import json
import os
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
SIL = os.path.join(ROOT, "benchmarks", "silicon")


def signoff_period_ns():
    b = json.load(open(os.path.join(SIL, "sky130_signoff.json")))["blocks"]["soft_core_scalar"]
    if not b.get("signoff_clean"):
        sys.exit("soft_core_scalar is not recorded signoff-clean; there is no guaranteed clock to project at")
    return float(b["clock_period_ns"])


def scalar_timing(doc, period):
    mhz = round(1000.0 / period, 2)
    doc["tapeout_mhz"] = mhz
    doc["note"] = ("scalar SoC on-chip timing, Nexys A7-100T. cycles are deterministic (==VM op count); FPGA clock "
                   "6.25 MHz (open-flow conservative /16 divider), tapeout projection at the soft_core_scalar Sky130 "
                   f"signoff clock — the period every corner meets ({mhz} MHz / {period:g} ns).")
    for d in doc["demos"]:
        d["tapeout_ns"] = round(d["cycles"] * period, 1)
    return doc


def hft_latency(doc, period):
    tier = doc["chip"]["latency_by_tier"]["Sky130 tapeout"]
    tier["mhz"] = round(1000.0 / period, 2)
    tier["ns_per_msg"] = round(doc["chip"]["per_message_cycles_median"] * period, 1)
    return doc


def main():
    check = "--check" in sys.argv[1:]
    period = signoff_period_ns()
    stale = []
    # Each artifact keeps the layout its own writer gave it.
    for name, derive, fmt in (("scalar_timing.json", scalar_timing, dict(indent=2, ensure_ascii=False)),
                              ("hft_latency.json", hft_latency, dict(indent=1))):
        path = os.path.join(SIL, name)
        raw = open(path).read()
        new = json.dumps(derive(json.loads(raw), period), **fmt) + ("\n" if raw.endswith("\n") else "")
        if json.loads(new) != json.loads(raw):
            stale.append(name)
            if not check:
                # Replace, never write in place: a flow container can leave these root-owned.
                tmp = path + ".tmp"
                open(tmp, "w").write(new)
                os.replace(tmp, path)
    verb = "stale" if check else "rewrote"
    print(f"signoff clock {1000.0 / period:.2f} MHz ({period:g} ns): {verb} {', '.join(stale) if stale else 'nothing'}")
    return 1 if (check and stale) else 0


if __name__ == "__main__":
    sys.exit(main())
