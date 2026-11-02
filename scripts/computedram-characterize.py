#!/usr/bin/env python3
"""computedram-characterize — Phase 3 of the DRAM-into-Compute campaign.

Sweeps the ComputeDRAM violation window (ACT->PRE = T1, PRE->ACT = T2) across rows/banks and
records whether the on-board Micron MT47H64M16 DDR2 actually performs RowClone / bulk bitwise
AND / OR at each (T1, T2). Two modes:

  --sim      : fill the PREDICTED (logical) result from the RowStore oracle (what the effect SHOULD
               compute if the chip exhibits it). Runnable now, no hardware. `measured` stays null.
  --measure  : drive the real board over UART (raw sub-JEDEC command engine), read back, and fill
               the MEASURED success rate per (T1, T2). Requires the Nexys A7 flashed with the
               raw-command SoC + pyserial. This is the honest experiment — the result may be full,
               partial (e.g. copy-only), or NULL: ComputeDRAM is chip/timing/temperature dependent
               and was originally demonstrated on DDR3, not DDR2.

Writes benchmarks/silicon/computedram_characterization.json. NEVER fabricates measured values.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

PART = "Micron MT47H64M16HR-25:H (Nexys A7-100T on-board DDR2-800)"
T1_GRID = list(range(1, 7))   # ACT -> PRE gap (controller cycles), deliberately sub-tRAS
T2_GRID = list(range(1, 5))   # PRE -> ACT gap, deliberately sub-tRP
OPS = ["rowclone", "and", "or"]
ROWS = [0, 1, 2, 7, 15, 100, 1000]  # representative rows across the address space


def oracle(op, a, b):
    """The RowStore/VM oracle: what the effect SHOULD compute (predicted logical result)."""
    if op == "rowclone":
        return a
    if op == "and":
        return a & b
    if op == "or":
        return a | b
    raise ValueError(op)


def predicted_grid():
    """The sim-predicted result: the effect is logically correct at every (T1, T2) IF the chip
    exhibits it. This encodes the ORACLE, not a physical prediction of which windows work — that is
    exactly what --measure must determine on real silicon."""
    a, b = 0xDEADBEEFCAFEF00D, 0x0123456789ABCDEF
    entries = []
    for op in OPS:
        want = oracle(op, a, b)
        entries.append({
            "op": op,
            "operand_a": f"{a:016x}",
            "operand_b": (f"{b:016x}" if op != "rowclone" else None),
            "predicted_result": f"{want:016x}",
            "predicted_by": "rowstore_oracle (codegen_sva::dram::RowStore)",
        })
    return entries


def measure_grid(port):
    """Drive the real board and fill measured success rates. GATED: requires hardware + the flashed
    raw-command SoC. Returns None until that exists (honest: no fabricated numbers)."""
    try:
        import serial  # noqa: F401
    except ImportError:
        print("pyserial not installed and no board flashed — measurement is gated on hardware.",
              file=sys.stderr)
        return None
    # The raw-command SoC (Phase 2 on-board) is not flashed yet; refuse to invent numbers.
    print(f"[measure] would sweep T1={T1_GRID} T2={T2_GRID} over rows {ROWS} on {port},",
          file=sys.stderr)
    print("[measure] issuing RowClone/AND/OR via the raw sub-JEDEC command engine and reading back.",
          file=sys.stderr)
    print("[measure] GATED: flash the raw-command SoC to the Nexys A7 first; measurement pending.",
          file=sys.stderr)
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sim", action="store_true", help="fill predicted (oracle) results, no hardware")
    ap.add_argument("--measure", action="store_true", help="drive the real board (gated on hardware)")
    ap.add_argument("--port", help="UART for --measure: a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--out", default=None, help="output JSON path")
    args = ap.parse_args()

    here = os.path.dirname(os.path.abspath(__file__))
    out = args.out or os.path.join(here, "..", "benchmarks", "silicon",
                                   "computedram_characterization.json")

    measured = measure_grid(ftdi_ports.port_arg(args.port, "B")) if args.measure else None
    doc = {
        "part": PART,
        "sweep": {"t1_act_to_pre": T1_GRID, "t2_pre_to_act": T2_GRID, "ops": OPS, "rows": ROWS},
        "predicted": predicted_grid(),
        "measured": measured,   # null until the board runs it — NEVER fabricated
        "provenance": ("real_board_uart" if measured is not None else "pending_hardware"),
        "honesty": (
            "ComputeDRAM is chip/timing/temperature dependent and was demonstrated on DDR3, not "
            "DDR2. Whether this Micron DDR2 exhibits the effect is an open experiment; `measured` is "
            "null until it runs on the flashed Nexys A7. `predicted` is the logical oracle result, "
            "not a claim about which physical (T1,T2) windows work."
        ),
    }
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print(f"wrote {out} (provenance={doc['provenance']})")


if __name__ == "__main__":
    main()
