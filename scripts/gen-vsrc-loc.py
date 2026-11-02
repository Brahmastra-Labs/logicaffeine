#!/usr/bin/env python3
"""Generate an OpenROAD voltage-source location file for a block's power grid.

WHY THIS EXISTS. `analyze_power_grid` needs to know WHERE current enters the grid. With
`VSRC_LOC_FILES` undefined OpenLane calls it with no `-vsrc`, OpenROAD picks a source node itself,
and the reported IR drop then depends on where that arbitrary node happens to land. The numbers
that came out were not physically ordered: `systolic_pipe16_realbist` (819,032 cells, 4.05 mm^2)
recorded 15.05 mV while `fabric_horner_seq` (189,531 cells, 1.09 mm^2) recorded 75.77 mV — a fifth
the drop for four times the cells. Every `ir_drop_worst_mv` in every signoff ledger was computed
this way, and one of them was the sole gate blocking a block that is otherwise clean.

THE MODEL, and it depends on how the block is powered — which is a property of its floorplan, not
a preference:

  * ring    — the block has a power ring around its core (`FP_PDN_CORE_RING` 1). Current enters at
              the boundary, so sources go around the PERIMETER at the strap pitch.
  * area    — the block has no ring (`FP_PDN_CORE_RING` 0) and is powered by the parent's straps
              running ACROSS it, connecting at every strap crossing over its whole area. Sources
              go on a GRID at the strap pitch.

Using the wrong one is not a small error. `fabric_horner_seq` has no core ring, so its power
arrives across its area; modelled as a perimeter-fed block it reported 230.59 mV of drop, because
every instance in the middle of the die was made to draw its current from the edge.

  gen-vsrc-loc.py <out.loc> <die_w_um> <die_h_um> <pitch_um> <voltage> [ring|area]
"""
import sys


def perimeter_sources(w, h, pitch, v):
    """One source per strap crossing the boundary: along both horizontal edges (vertical straps
    arrive there) and both vertical edges (horizontal straps arrive there)."""
    pts = []
    n_v = max(1, int(w // pitch))
    n_h = max(1, int(h // pitch))
    for i in range(n_v + 1):
        x = min(w, i * pitch)
        pts.append((x, 0.0))
        pts.append((x, h))
    for j in range(n_h + 1):
        y = min(h, j * pitch)
        pts.append((0.0, y))
        pts.append((w, y))
    # Dedupe the four corners, which both loops produce.
    seen, out = set(), []
    for p in pts:
        k = (round(p[0], 3), round(p[1], 3))
        if k not in seen:
            seen.add(k)
            out.append(k)
    return [(x, y, v) for x, y in out]


def area_sources(w, h, pitch, v):
    """A source at every strap crossing over the block — the parent grid connecting down across the
    whole area, which is how a ring-less macro is actually fed."""
    pts = []
    nx = max(1, int(w // pitch))
    ny = max(1, int(h // pitch))
    for i in range(nx + 1):
        for j in range(ny + 1):
            pts.append((min(w, i * pitch), min(h, j * pitch), v))
    return pts


def main():
    if len(sys.argv) not in (6, 7):
        sys.exit(__doc__)
    out, w, h, pitch, v = (
        sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
    )
    mode = sys.argv[6] if len(sys.argv) == 7 else "ring"
    src = area_sources(w, h, pitch, v) if mode == "area" else perimeter_sources(w, h, pitch, v)
    # COMMA-separated. PDNSim's own IR report is CSV ("Instance name, X location, Y location,
    # Voltage") and it reads the vsrc file the same way. A space-separated file is accepted without
    # complaint and then mis-parsed: the first attempt at this produced instance voltages around
    # 867 against a 1.8 V rail, which the recorder turned into an IR drop of -46,330 mV. Garbage
    # that LOOKS like a measurement is the failure mode this whole exercise exists to remove.
    with open(out, "w") as fh:
        for x, y, volts in src:
            fh.write(f"{x:.3f},{y:.3f},{volts}\n")
    print(f"gen-vsrc-loc: {len(src)} sources at {v} V, mode={mode}, {w}x{h} um, pitch {pitch} um -> {out}")


if __name__ == "__main__":
    main()
