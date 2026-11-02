#!/usr/bin/env python3
"""gen-computedram-energy — Phase 6 energy accounting for in-DRAM compute.

The EXACT, structural win of ComputeDRAM is bytes-not-moved: a row-wide bitwise op computes in place,
moving ZERO bytes across the memory bus, versus a CPU/fabric that must read both operands and write the
result. Energy figures are PROJECTED from published DDR2 access-energy assumptions and flagged
`measured:false` — no on-silicon energy is measured (that needs the board + power instrumentation).
"""

import json
import os

ROW_BYTES = 2048          # DDR2-800 x16 page (Micron MT47H64M16)
# Projected DDR2 access energy. ~20 pJ/bit is a representative figure for DDR2-era DRAM I/O + array
# access energy; stated explicitly so the projection is auditable, NOT presented as measured.
PJ_PER_BIT_BUS = 20.0
# An in-DRAM RowClone/bitwise op costs a handful of ACT/PRE commands (array activation), no bus burst.
# Representative ACT+PRE array energy per op (projected).
PJ_PER_INDRAM_OP = 250.0


def account(op, operand_rows):
    """A row-wide bitwise op: CPU moves (operand_rows reads + 1 write) × ROW_BYTES; in-DRAM moves 0."""
    bytes_cpu = (operand_rows + 1) * ROW_BYTES
    bits_cpu = bytes_cpu * 8
    e_cpu_pj = bits_cpu * PJ_PER_BIT_BUS            # projected bus/access energy the CPU path pays
    e_indram_pj = PJ_PER_INDRAM_OP                  # projected in-DRAM op energy (ACT/PRE only)
    return {
        "op": op,
        "row_bytes": ROW_BYTES,
        "bytes_moved_cpu": bytes_cpu,               # EXACT / structural
        "bytes_moved_in_dram": 0,                   # EXACT / structural
        "bytes_not_moved": bytes_cpu,               # EXACT / structural — the core win
        "energy_cpu_pj_projected": round(e_cpu_pj, 1),
        "energy_in_dram_pj_projected": round(e_indram_pj, 1),
        "energy_reduction_x_projected": round(e_cpu_pj / e_indram_pj, 1),
    }


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    out = os.path.join(here, "..", "benchmarks", "silicon", "computedram_energy.json")
    doc = {
        "part": "Micron MT47H64M16HR-25:H (Nexys A7-100T DDR2-800)",
        "exact": "bytes_moved_* and bytes_not_moved are structural (independent of any energy model)",
        "projection_assumptions": {
            "pj_per_bit_bus": PJ_PER_BIT_BUS,
            "pj_per_in_dram_op": PJ_PER_INDRAM_OP,
            "note": "representative DDR2-era figures, stated for audit; NOT measured on silicon",
        },
        "measured": False,   # honesty: no on-silicon energy measured (needs board + power capture)
        "provenance": "projected_from_stated_assumptions",
        "workloads": [
            account("bulk_and", 2),      # AND of two rows
            account("bulk_or", 2),       # OR of two rows
            account("rowclone_copy", 1), # copy one row
        ],
    }
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print(f"wrote {out} (measured={doc['measured']}, provenance={doc['provenance']})")


if __name__ == "__main__":
    main()
