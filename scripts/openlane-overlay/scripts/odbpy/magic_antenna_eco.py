# magic_antenna_eco.py — LOGOS Magic/ARC-driven antenna ECO.
#
# OpenLane/OpenROAD's repair_antennas is GRT-based (global route) and can't clear the DETAILED-
# route violations that check_antennas (the ARC signoff gate) reports. This does the standard
# foundry metal-ECO: read check_antennas' EXACT violator net list and drop a protection diode on
# each offending gate's input, wired to the same net.
#
# By the time routing runs, every empty core site is already occupied by physical-only fill/decap
# cells, so there is no "free" site to legalize into (that's what made a naive placer fail with
# 0-diodes / all-no-fit). The real foundry move is a FILLER SWAP: evict the removable fill/decap
# cells under the target footprint and drop the diode in their freed sites. Tap cells (latch-up
# ties) are never evicted. The caller then re-routes and re-checks, iterating to zero.
import os
import odb
import click
from reader import click_odb


def norm(s):
    """Escape-insensitive net name: the DB reports escaped bus nets as R\\[1\\]\\[12\\] but the
    violator report / parser may or may not carry the backslashes. Compare on the stripped form."""
    return s.replace("\\", "")


def is_removable(inst):
    """Physical-only spacers we may evict for a diode. Never tap cells (latch-up ties)."""
    m = inst.getMaster().getConstName().lower()
    return ("fill" in m or "decap" in m or "fakediode" in m) and "tap" not in m


# The diode's DIODE pin lives on li1 and connects up on met1; a foreign net whose wire runs on those
# layers over the diode footprint shorts to it (that is the rout[96] short in the partial-unroute ECO).
# Those overlapping nets must also be unrouted so the re-route lays them AROUND the diode.
_LOW_LAYERS = {"li1", "met1"}
_GRID = 5000  # DBU bucket for the diode-footprint spatial hash (~5um)


def build_diode_grid(diodes, margin=200):
    """Spatial hash of diode footprints as plain (xlo,ylo,xhi,yhi) tuples (avoids odb.Rect SWIG quirks)."""
    grid = {}
    for d in diodes:
        bb = d.getBBox()
        rect = (bb.xMin() - margin, bb.yMin() - margin, bb.xMax() + margin, bb.yMax() + margin)
        for gx in range(rect[0] // _GRID, rect[2] // _GRID + 1):
            for gy in range(rect[1] // _GRID, rect[3] // _GRID + 1):
                grid.setdefault((gx, gy), []).append(rect)
    return grid


def wire_overlaps_diode(net, grid):
    w = net.getWire()
    if w is None:
        return False
    D = odb.dbWireDecoder
    dec = D()
    dec.begin(w)
    layer = None
    prev = None
    while True:
        op = dec.next()
        if op == D.END_DECODE:
            break
        if op in (D.PATH, D.JUNCTION, D.SHORT, D.VWIRE):
            L = dec.getLayer()
            layer = L.getName() if L is not None else None
            prev = None
        elif op in (D.POINT, D.POINT_EXT):
            p = dec.getPoint()
            x, y = p[0], p[1]
            if layer in _LOW_LAYERS and prev is not None:
                sxlo, sylo = min(prev[0], x) - 90, min(prev[1], y) - 90
                sxhi, syhi = max(prev[0], x) + 90, max(prev[1], y) + 90
                for gx in range(sxlo // _GRID, sxhi // _GRID + 1):
                    for gy in range(sylo // _GRID, syhi // _GRID + 1):
                        for (dxlo, dylo, dxhi, dyhi) in grid.get((gx, gy), ()):
                            if not (sxhi < dxlo or sxlo > dxhi or syhi < dylo or sylo > dyhi):
                                return True
            prev = (x, y)
    return False


def build_wire_grid(block, output_only=False):
    """Spatial hash of li1/met1 wire segments (int tuples). With output_only, only the OUTPUT-port nets
    (rout[*], halted) — the LONG die-spanning met1 nets that are the ONLY ones that short to a near-placed
    diode (every observed short was rout[*], never an internal net). Placing diodes clear of just these
    keeps them NEAR their gate (antenna discharges → converges to 0) while never landing under an output
    wire — so no static short, and we never touch/re-route the outputs (which just moves the short)."""
    grid = {}
    D = odb.dbWireDecoder
    for net in block.getNets():
        if net.isSpecial():
            continue
        if output_only and not any(bt.getIoType() == "OUTPUT" for bt in net.getBTerms()):
            continue
        w = net.getWire()
        if w is None:
            continue
        dec = D()
        dec.begin(w)
        layer = None
        prev = None
        while True:
            op = dec.next()
            if op == D.END_DECODE:
                break
            if op in (D.PATH, D.JUNCTION, D.SHORT, D.VWIRE):
                L = dec.getLayer()
                layer = L.getName() if L is not None else None
                prev = None
            elif op in (D.POINT, D.POINT_EXT):
                p = dec.getPoint()
                x, y = p[0], p[1]
                if layer in _LOW_LAYERS and prev is not None:
                    seg = (min(prev[0], x) - 90, min(prev[1], y) - 90,
                           max(prev[0], x) + 90, max(prev[1], y) + 90)
                    for gx in range(seg[0] // _GRID, seg[2] // _GRID + 1):
                        for gy in range(seg[1] // _GRID, seg[3] // _GRID + 1):
                            grid.setdefault((gx, gy), []).append(seg)
                prev = (x, y)
    return grid


def area_is_clear(grid, xlo, ylo, xhi, yhi):
    for gx in range(xlo // _GRID, xhi // _GRID + 1):
        for gy in range(ylo // _GRID, yhi // _GRID + 1):
            for (wxlo, wylo, wxhi, wyhi) in grid.get((gx, gy), ()):
                if not (xhi < wxlo or xlo > wxhi or yhi < wylo or ylo > wyhi):
                    return False
    return True


def build_rows(block):
    """One record per placement row: origin, site width, count, and a per-site occupancy map
    (None = free, else the occupying inst)."""
    rows = []
    for row in block.getRows():
        bb = row.getBBox()
        sw = row.getSite().getWidth()
        n = (bb.xMax() - bb.xMin()) // sw
        rows.append({"x0": bb.xMin(), "y": bb.yMin(), "sw": sw, "n": int(n),
                     "orient": row.getOrient(), "occ": [None] * int(n)})
    rows.sort(key=lambda r: r["y"])
    return rows


def nearest_row(rows, by_y, gy):
    r = by_y.get(gy)
    if r is not None:
        return r
    return min(rows, key=lambda rr: abs(rr["y"] - gy)) if rows else None


def mark_occupancy(rows, by_y, block):
    for inst in block.getInsts():
        if not (inst.isPlaced() or inst.isFixed()):
            continue
        loc = inst.getLocation()
        r = nearest_row(rows, by_y, loc[1])
        if r is None:
            continue
        start = (loc[0] - r["x0"]) // r["sw"]
        span = (inst.getMaster().getWidth() + r["sw"] - 1) // r["sw"]
        for s in range(max(0, start), min(r["n"], start + span)):
            r["occ"][s] = inst


def reserve(rows, by_y, gx, gy, diode_span, wire_grid=None, diode_h=0):
    """Nearest run of diode_span sites (free or removable-fill) to the gate that is ALSO clear of foreign
    li1/met1 wiring (so the diode pin can't short). Returns (row, start_site, evict_set) or None."""
    r = nearest_row(rows, by_y, gy)
    if r is None:
        return None
    center = (gx - r["x0"]) // r["sw"]
    for d in range(0, r["n"]):
        for s in (center - d, center + d):
            if s < 0 or s + diode_span > r["n"]:
                continue
            evict = set()
            ok = True
            for k in range(diode_span):
                occ = r["occ"][s + k]
                if occ is None:
                    continue
                if is_removable(occ):
                    evict.add(occ)
                else:
                    ok = False
                    break
            if not ok:
                continue
            if wire_grid is not None:
                xlo = r["x0"] + s * r["sw"]
                if not area_is_clear(wire_grid, xlo, r["y"], xlo + diode_span * r["sw"], r["y"] + diode_h):
                    continue
            return (r, s, evict)
    return None


def reserve_candidates(rows, gx, gy, diode_span, wire_grid=None, diode_h=0, row_reach=3, site_reach=80):
    """Every legal diode site near the gate (free or removable fill, clear of foreign output wiring),
    across the gate's row and `row_reach` rows either side, nearest first. `reserve` returns only
    the single nearest; a met1 tie needs alternatives, because the nearest site's path is often
    blocked by a foreign wire the next site's is not."""
    near = sorted(rows, key=lambda rr: abs(rr["y"] - gy))[: 2 * row_reach + 1]
    out = []
    for r in near:
        center = (gx - r["x0"]) // r["sw"]
        for s in range(max(0, center - site_reach), min(r["n"] - diode_span, center + site_reach) + 1):
            evict = set(); ok = True
            for k in range(diode_span):
                occ = r["occ"][s + k]
                if occ is None:
                    continue
                if is_removable(occ):
                    evict.add(occ)
                else:
                    ok = False; break
            if not ok:
                continue
            if wire_grid is not None:
                xlo = r["x0"] + s * r["sw"]
                if not area_is_clear(wire_grid, xlo, r["y"], xlo + diode_span * r["sw"], r["y"] + diode_h):
                    continue
            dist = abs(r["x0"] + s * r["sw"] - gx) + abs(r["y"] - gy)
            out.append((dist, r, s, evict))
    out.sort(key=lambda t: t[0])
    return out


def island_candidates(rows, island, diode_span, wire_grid=None, diode_h=0, row_reach=3, site_reach=80):
    """Diode sites along the WHOLE met1 island, ordered by the length of the tie each would need.

    The site search used to be anchored on the gate alone, where the cells are densest: on pipe16
    _174816_/A all 601 sites near the access via were blocked, while its island ran 75 um west
    through open rows. Anchors are sampled along every island segment at less than the search
    reach, so no stretch of the island is out of range of some anchor."""
    step = max(1, site_reach) * (rows[0]["sw"] if rows else 460)
    anchors = []
    for a, b in island:
        n = max(1, (abs(b[0] - a[0]) + abs(b[1] - a[1])) // step)
        for k in range(n + 1):
            anchors.append((a[0] + (b[0] - a[0]) * k // n, a[1] + (b[1] - a[1]) * k // n))
    best = {}
    for ax, ay in anchors:
        for _d, r, s, evict in reserve_candidates(rows, ax, ay, diode_span, wire_grid, diode_h,
                                                  row_reach, site_reach):
            key = (r["y"], s)
            if key in best:
                continue
            xy = (r["x0"] + s * r["sw"], r["y"])
            t = nearest_on_island(island, xy)
            best[key] = (abs(t[0] - xy[0]) + abs(t[1] - xy[1]), r, s, evict)
    return sorted(best.values(), key=lambda c: c[0])


def gate_access_point(net, sink_it, margin=100):
    """Where the ROUTE touches this gate pin: the li1->met1 via on `net` inside the pin's bbox.

    That via's met1 enclosure is part of the gate's met1 island — the island `check_antennas`
    evaluates at met1's deposit — so a diode tied to THIS point is credited on met1 and above.
    Returns (x, y) or None."""
    from sat_antenna_jog import decode_wire
    w = net.getWire()
    if w is None:
        return None
    bb = sink_it.getBBox()
    x0, y0, x1, y1 = bb.xMin() - margin, bb.yMin() - margin, bb.xMax() + margin, bb.yMax() + margin
    last = None
    for o in decode_wire(w):
        if o[0] in ("POINT", "POINT_EXT"):
            last = (o[1], o[2])
        elif o[0] == "TECH_VIA" and last is not None:
            if o[1].getName().startswith("L1M1") and x0 <= last[0] <= x1 and y0 <= last[1] <= y1:
                return last
    return None


def wire_segments(ops):
    """Flatten decoded wire ops into (layer_name, (x0, y0), (x1, y1)) segments.

    A via moves the path to its OTHER layer, so the layer is tracked through every via, not only
    at PATH/JUNCTION ops — otherwise met2 between two M1M2 vias reads as met1 and two separate
    met1 islands merge into one."""
    name = lambda l: l.getConstName() if l is not None else None
    segs, layer, last = [], None, None
    for o in ops:
        k = o[0]
        if k == "PATH":
            layer, last = name(o[1]), None
        elif k in ("JUNCTION", "SHORT", "VWIRE"):
            layer, last = name(o[2]), None
        elif k in ("TECH_VIA", "VIA"):
            bot, top = name(o[1].getBottomLayer()), name(o[1].getTopLayer())
            layer = top if layer == bot else bot
        elif k in ("POINT", "POINT_EXT"):
            p = (o[1], o[2])
            if last is not None and layer is not None and p != last:
                segs.append((layer, last, p))
            last = p
    return segs


def _on_segment(p, a, b):
    return (min(a[0], b[0]) <= p[0] <= max(a[0], b[0])) and (min(a[1], b[1]) <= p[1] <= max(a[1], b[1])) \
        and (a[0] == b[0] == p[0] or a[1] == b[1] == p[1])


def met1_island(segs, access):
    """The met1 segments geometrically joined to `access` THROUGH MET1 ONLY — the island
    `check_antennas` evaluates at met1's deposit. Metal reached through met2 is another island
    and earns the gate no met1 credit, which is exactly why heuristic diodes joined via met2 fail."""
    m1 = [(a, b) for l, a, b in segs if l == "met1"]
    island = [s for s in m1 if _on_segment(access, *s)]
    rest = [s for s in m1 if s not in island]
    grown = True
    while grown:
        grown = False
        for s in list(rest):
            if any(_on_segment(e, *t) for t in island for e in s) or \
               any(_on_segment(e, *s) for t in island for e in t):
                island.append(s); rest.remove(s); grown = True
    return island


def wire_vias(ops):
    """Every via on a decoded wire as (point, bottom_layer_name, top_layer_name)."""
    name = lambda l: l.getConstName() if l is not None else None
    out, layer, last = [], None, None
    for o in ops:
        k = o[0]
        if k == "PATH":
            layer, last = name(o[1]), None
        elif k in ("JUNCTION", "SHORT", "VWIRE"):
            layer, last = name(o[2]), None
        elif k in ("POINT", "POINT_EXT"):
            last = (o[1], o[2])
        elif k in ("TECH_VIA", "VIA") and last is not None:
            bot, top = name(o[1].getBottomLayer()), name(o[1].getTopLayer())
            out.append((last, bot, top))
            layer = top if layer == bot else bot
    return out


def island_rank(vias, layer):
    """How far up the stack `layer` sits, read from the vias (0 for the lowest)."""
    order, frontier = {}, set()
    bots = {b for _p, b, _t in vias}
    tops = {t for _p, _b, t in vias}
    frontier = bots - tops
    r = 0
    while frontier:
        for l in frontier:
            order.setdefault(l, r)
        frontier = {t for _p, b, t in vias if b in frontier and t not in order}
        r += 1
    return order.get(layer, r)


def _component_below(segs, vias, access, max_layer, tie_layer="met1"):
    """The (layer, a, b) segments joined to the gate through layers no higher than `max_layer`,
    and the stack rank each layer was given (read from the vias)."""
    above = {}
    for _p, b, t in vias:
        above.setdefault(b, set()).add(t)
    rank = {}

    def depth(l, seen=()):
        if l in rank:
            return rank[l]
        below = [b for b, ts in above.items() if l in ts and b not in seen]
        rank[l] = 1 + max((depth(b, seen + (l,)) for b in below), default=-1)
        return rank[l]

    for l in set(above) | {t for ts in above.values() for t in ts}:
        depth(l)
    cap = rank.get(max_layer, rank.get(tie_layer, 0))
    ok = lambda l: rank.get(l, cap + 1) <= cap
    nodes = [(l, a, b) for l, a, b in segs if ok(l)]
    joins = [(p, b, t) for p, b, t in vias if ok(b) and ok(t)]
    # Seed from every layer the via STACK standing on the access point reaches: a pin left through
    # stacked L1M1+M1M2 has no met1 segment at the access point at all (fabric_horner_seq _33412_/A2).
    at = {tie_layer}
    grew = True
    while grew:
        grew = False
        for p, b, t in joins:
            if p == access and (b in at) != (t in at):
                at |= {b, t}
                grew = True
    comp = [n for n in nodes if n[0] in at and _on_segment(access, n[1], n[2])]
    rest = [n for n in nodes if n not in comp]
    # Every layer each via point joins: a stack at one point (L1M1 + M1M2 + M2M3, no segment
    # between) connects all four layers there, exactly as the stack at the access point does.
    stacked = {}
    for p, b, t in joins:
        s = stacked.setdefault(p, [])
        s.append((b, t))

    def joined(p, layer):
        s, grew = {layer}, True
        while grew:
            grew = False
            for b, t in stacked.get(p, ()):
                if (b in s) != (t in s):
                    s |= {b, t}
                    grew = True
        return s

    grown = True
    while grown:
        grown = False
        for n in list(rest):
            hit = any(n[0] == c[0] and (any(_on_segment(e, c[1], c[2]) for e in (n[1], n[2]))
                                        or any(_on_segment(e, n[1], n[2]) for e in (c[1], c[2])))
                      for c in comp)
            if not hit:
                hit = any(_on_segment(p, n[1], n[2]) and _on_segment(p, c[1], c[2]) and n[0] in joined(p, c[0])
                          for p in stacked for c in comp)
            if hit:
                comp.append(n); rest.remove(n); grown = True
    return comp, rank

def island_below(segs, vias, access, max_layer, tie_layer="met1"):
    """The `tie_layer` segments joined to the gate through metal NO HIGHER than `max_layer`.

    An antenna violation on layer L is judged on what is connected to the gate while L is being
    etched — every layer up to L — so a diode tied anywhere in that component is credited at L.
    For a met1 violation that is exactly the gate's met1 island (`met1_island`). For a met3
    violation it also takes in met2 and every met1 branch reachable beneath met3, which is where
    a clear tie usually exists when the gate's own met1 stub sits in dense logic. Layer order is
    read from the vias themselves (bottom below top), so no process's layer names are assumed.
    """
    comp, _rank = _component_below(segs, vias, access, max_layer, tie_layer)
    return [(a, b) for l, a, b in comp if l == tie_layer]


def tie_targets(segs, vias, access, max_layer, tie_layer="met1", up_layer="met2"):
    """Where a diode tie may land: the component's `tie_layer` segments, and — when the violation
    is on `up_layer` or above — its `up_layer` segments too, reached by one via at the landing.
    A landing on `up_layer` is connected from that layer's deposit up, so it is credited there and
    above; a violation below it must land on `tie_layer`. Returns (layer, a, b) triples."""
    comp, rank = _component_below(segs, vias, access, max_layer, tie_layer)
    allowed = {tie_layer}
    if rank.get(max_layer, -1) >= rank.get(up_layer, 1 << 30):
        allowed.add(up_layer)
    return [(l, a, b) for l, a, b in comp if l in allowed]


def drop_targets(segs, vias, access, max_layer, tie_layer="met1"):
    """Where a via STACK standing on a diode's own pin may land: every segment of the gate's
    component on a layer from `tie_layer` up to `max_layer`, itself included.

    A landing on layer L joins the diode to the component at L's deposit, so it is credited for a
    violation on L and above. `tie_targets` stops at met2 because an L-shaped met1 tie reaches
    one layer up at its far end; a stack rises straight from the pin, so it can reach the
    violating layer itself. MEASURED on wpe_user_project net1123 (RUN_2026.09.27_18.14.14,
    38-eco_jog_6.odb): `_353370_/B2` violated on met3 at ratio 2.79, and its component below met3
    held 168 um of met3 over open rows but only its own 850 nm met2 access stack below it, in
    dense logic, where all 629 candidate L-tie sites were refused."""
    comp, rank = _component_below(segs, vias, access, max_layer, tie_layer)
    return [(l, a, b) for l, a, b in comp if rank.get(l, 0) >= rank.get(tie_layer, 0)]


def lef_antenna_rules(lef_path, diode_cell, diode_pin):
    """The antenna rules a diode's own island is judged by, read from the LEF the flow loaded: each
    routing layer's THICKNESS (um) and ANTENNADIFFSIDEAREARATIO PWL [(diffusion um^2, ratio)], and the
    diode pin's ANTENNAGATEAREA and ANTENNADIFFAREA. The odb binding exposes neither the PWL tables
    nor a layer's thickness (`getThickness` takes a C++ out-parameter), so the text is the source."""
    import re
    try:
        txt = open(lef_path).read()
    except (OSError, TypeError):
        return {"layers": {}, "gate": 0.0, "diff": 0.0}
    layers = {}
    for m in re.finditer(r"^LAYER\s+(\S+)(.*?)^END\s+\1\b", txt, re.M | re.S):
        t = re.search(r"THICKNESS\s+([\d.]+)", m.group(2))
        pwl = re.search(r"ANTENNADIFFSIDEAREARATIO\s+PWL\s*\((.*?)\)\s*;", m.group(2), re.S)
        if t and pwl:
            pts = [(float(a), float(b)) for a, b in re.findall(r"\(\s*([-\d.eE+]+)\s+([-\d.eE+]+)\s*\)", pwl.group(1))]
            layers[m.group(1)] = (float(t.group(1)), pts)
    gate = diff = 0.0
    mac = re.search(r"^MACRO\s+" + re.escape(diode_cell) + r"\b(.*?)^END\s+" + re.escape(diode_cell) + r"\b",
                    txt, re.M | re.S)
    if mac:
        pin = re.search(r"PIN\s+" + re.escape(diode_pin) + r"\b(.*?)END\s+" + re.escape(diode_pin) + r"\b",
                        mac.group(1), re.S)
        if pin:
            g = re.search(r"ANTENNAGATEAREA\s+([\d.]+)", pin.group(1))
            d = re.search(r"ANTENNADIFFAREA\s+([\d.]+)", pin.group(1))
            gate = float(g.group(1)) if g else 0.0
            diff = float(d.group(1)) if d else 0.0
    return {"layers": layers, "gate": gate, "diff": diff}


def pwl_at(pts, x):
    """A LEF piecewise-linear table at `x`, held at its end values outside the points it lists."""
    if not pts:
        return 0.0
    if x <= pts[0][0]:
        return pts[0][1]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        if x <= x1:
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0) if x1 != x0 else y1
    return pts[-1][1]


def landing_is_safe(segs, vias, access, target, viol_layer, rules, dbu, widths, margin=0.9, gate_comps=None):
    """Can a diode land on `target` without becoming an antenna violator itself?

    A diode tied to a segment the gate reaches only through a HIGHER layer shares, at every layer below
    that join, an island with that segment's metal and no gate but its own: sky130's diode_2 declares its
    pin's gate area. At each layer from the landing up to the violating one where the target is not in the
    gate's component, the island's side area on that layer, over the diode's gate area, must stay under the
    layer's diffusion side-area ratio at the diode's own diffusion (less `margin`). MEASURED on
    systolic_pipe16_realbist (RUN_2026.09.29_02.09.26): clkbuf_0_clk/A's met4 violation drew a diode onto a
    met3 run of `clk` joined to the gate only through met4, and the checker flagged the diode on met3,
    3162.42 against 2773.88. Without rules for a layer, only a landing in the gate's component is safe."""
    layer, a, b = target
    comp, rank = _component_below(segs, vias, access, viol_layer)
    lo, hi = rank.get(layer, 0), rank.get(viol_layer, 0)
    gate_comps = gate_comps if gate_comps is not None else {}
    for x in sorted((l for l, r in rank.items() if lo <= r <= hi), key=rank.get):
        if x not in gate_comps:
            gate_comps[x] = set(_component_below(segs, vias, access, x)[0])
        if target in gate_comps[x]:
            return True
        rule = rules["layers"].get(x)
        if rule is None or rules["gate"] <= 0:
            return False
        island, _r = _component_below(segs, vias, a, x, tie_layer=layer)
        thick, pwl = rule
        side = sum(2.0 * (abs(q[0] - p[0]) + abs(q[1] - p[1]) + widths.get(l, 0)) / dbu * thick
                   for l, p, q in island if l == x)
        if side > pwl_at(pwl, rules["diff"]) * rules["gate"] * margin:
            return False
    return True


def via_stacks(tech, from_layer, to_layer):
    """Every orientation choice of the via stack from `from_layer` up to `to_layer`, as lists of
    tech vias, or [[]] when the two are the same layer. Each step is the process's via between
    a layer and the one above it; both of its transposed variants are offered, because which one
    clears the neighbours depends on the wires around the landing."""
    from sat_antenna_jog import LAYER_UP, VIA_UP
    steps, layer = [], from_layer
    while layer != to_layer:
        base = VIA_UP.get(layer)
        up = LAYER_UP.get(layer)
        if base is None or up is None:
            return []
        variants = []
        for name in (base, base + "_R"):
            v = tech.findVia(name)
            if v is not None and all(v is not w for w in variants):
                variants.append(v)
        if not variants:
            return []
        steps.append(variants)
        layer = up
    stacks = [[]]
    for variants in steps:
        stacks = [s + [v] for s in stacks for v in variants]
    return stacks


def stack_jogs(pt, vias):
    """On how many metal layers the stack's enclosures do NOT nest — their union is not a single
    rectangle, so the drawn shape carries steps as small as 15 nm (L1M1_PR's met1 is 290 x 230,
    M1M2_PR_R's 260 x 320). A nested stack draws exactly what the router draws at a pin."""
    from sat_antenna_jog import via_layer_rects, union_area
    per = {}
    for v in vias:
        for ly in (v.getBottomLayer().getConstName(), v.getTopLayer().getConstName()):
            per.setdefault(ly, []).extend(via_layer_rects(v, pt, ly))
    jogs = 0
    for rects in per.values():
        box = (min(r[0] for r in rects), min(r[1] for r in rects), max(r[2] for r in rects), max(r[3] for r in rects))
        jogs += union_area(rects) != (box[2] - box[0]) * (box[3] - box[1])
    return jogs


def stack_is_clear(tech, pt, stack, foreign, own, cuts, l1m1=None):
    """Is the via stack `stack`, standing on the L1M1 via at `pt`, legal geometry?

    Per metal layer it touches: its new enclosures clear every foreign shape by the spacing rule
    (`foreign[layer]`, grown by that rule), sit either merged with or a full space away from the
    net's own metal (`own[layer]`), do not notch one another, and — merged with whatever own metal
    they touch — meet the layer's minimum area. Per cut: the spacing rule to every existing cut on
    its cut layer (`cuts[cut_layer]`). Returns False on the first failure."""
    from sat_antenna_jog import (conflicts, makes_notch, self_notch, connected_area, min_area_dbu2,
                                 layer_spacing, via_layer_rects, via_cut_rule, cuts_too_close)
    l1m1 = l1m1 if l1m1 is not None else tech.findVia("L1M1_PR")
    vias = ([l1m1] if l1m1 is not None else []) + list(stack)
    new = {}
    for v in vias:
        for ly in (v.getBottomLayer().getConstName(), v.getTopLayer().getConstName()):
            if ly != "li1":
                new.setdefault(ly, []).extend(via_layer_rects(v, pt, ly))
    for ly, rects in new.items():
        sp = layer_spacing(tech, ly)
        if any(conflicts(r, f, sp) for r in rects for f in foreign.get(ly, ())):
            return False
        if makes_notch(rects, own.get(ly, ()), sp) or self_notch(rects, sp):
            return False
        if connected_area(rects, own.get(ly, ())) < min_area_dbu2(tech, ly):
            return False
    for v in stack:
        cut, cw, ch, csp = via_cut_rule(v)
        if cut is None:
            return False
        if any(cuts_too_close(pt, c, cw, ch, csp) for c in cuts.get(cut, ())):
            return False
    return True


def plan_stack_drop(tech, pin_bbox, targets, foreign, own, cuts):
    """A tie with NO routed metal onto a layer above met1: the L1M1 via from the diode's own li1
    pin, then a via stack straight up into a target segment passing over the pin.

    `plan_drop` is the met1 case; this reaches the target's own layer, so a diode under a long
    upper-layer run is credited there without a single wire crossing the congested tracks below
    it. Returns (point, target_layer, [vias]) for the first legal landing, or None."""
    l1m1 = tech.findVia("L1M1_PR")
    if l1m1 is None:
        return None
    li_half = 0
    for b in l1m1.getBoxes():
        ly = b.getTechLayer()
        if ly is not None and ly.getConstName() == "li1":
            li_half = max(li_half, max(b.getDX(), b.getDY()) // 2)
    x0, y0, x1, y1 = pin_bbox[0] + li_half, pin_bbox[1] + li_half, pin_bbox[2] - li_half, pin_bbox[3] - li_half
    snap = lambda v: int(round(v / 5)) * 5
    for layer, a, b in targets:
        if a[1] == b[1] and y0 <= a[1] <= y1:
            lo, hi = max(x0, min(a[0], b[0])), min(x1, max(a[0], b[0]))
            if lo > hi:
                continue
            pt = (min(max(snap((lo + hi) / 2), lo), hi), a[1])
        elif a[0] == b[0] and x0 <= a[0] <= x1:
            lo, hi = max(y0, min(a[1], b[1])), min(y1, max(a[1], b[1]))
            if lo > hi:
                continue
            pt = (a[0], min(max(snap((lo + hi) / 2), lo), hi))
        else:
            continue
        for stack in sorted(via_stacks(tech, "met1", layer), key=lambda s: stack_jogs(pt, [l1m1] + s)):
            if stack_is_clear(tech, pt, stack, foreign, own, cuts, l1m1):
                return pt, layer, stack
    return None


def plan_stack_jog(tech, pin_bbox, targets, foreign, own, cuts, reach=10000):
    """A tie onto an upper-layer run that passes BESIDE the diode's pin: the L1M1 via from its li1 pin, a via
    stack straight up to the layer below the run, one straight jog on that layer to under the run, and the via
    up onto it. Returns (pin point, landing point, target layer, [stack vias], landing via), or None.

    `plan_stack_drop` needs the run directly over the pin, which a vertical run at one x — or a horizontal one
    at one y — almost never is: the pin is a few hundred nm wide and the sites step 460 nm. MEASURED on
    coproc_core `_45844_/B1` (RUN_2026.10.03_15.18.47): a met4 violation with 34 um of met3 and 172 um of met4 in
    its component, and every one of 1,200 sites refused. The diode joins the component at the run's own layer,
    exactly as a straight stack does, so it is credited there and above. A met1 run is the L-tie's, and a jog
    longer than `reach` would be a route."""
    from sat_antenna_jog import (LAYER_UP, VIA_UP, conflicts, makes_notch, makes_neck, self_notch, connected_area,
                                 min_area_dbu2, layer_spacing, layer_min_width, via_layer_rects, via_cut_rule,
                                 cuts_too_close, seg_rect)
    l1m1 = tech.findVia("L1M1_PR")
    if l1m1 is None:
        return None
    li_half = 0
    for b in l1m1.getBoxes():
        ly = b.getTechLayer()
        if ly is not None and ly.getConstName() == "li1":
            li_half = max(li_half, max(b.getDX(), b.getDY()) // 2)
    x0, y0, x1, y1 = pin_bbox[0] + li_half, pin_bbox[1] + li_half, pin_bbox[2] - li_half, pin_bbox[3] - li_half
    if x0 > x1 or y0 > y1:
        return None
    snap = lambda v: int(round(v / 5)) * 5
    pt = (min(max(snap((x0 + x1) / 2), x0), x1), min(max(snap((y0 + y1) / 2), y0), y1))
    below = {up: low for low, up in LAYER_UP.items()}
    for layer, a, b in targets:
        low = below.get(layer)
        if low is None or low == "li1":
            continue
        if a[0] == b[0] and min(a[1], b[1]) <= pt[1] <= max(a[1], b[1]):
            q = (a[0], pt[1])
        elif a[1] == b[1] and min(a[0], b[0]) <= pt[0] <= max(a[0], b[0]):
            q = (pt[0], a[1])
        else:
            continue
        if q == pt or abs(q[0] - pt[0]) + abs(q[1] - pt[1]) > reach:
            continue
        landings = []
        for name in (VIA_UP.get(low, ""), VIA_UP.get(low, "") + "_R"):
            v = tech.findVia(name)
            if v is not None and all(v is not w for w in landings):
                landings.append(v)
        jog = seg_rect([pt, q], layer_min_width(tech, low) // 2)
        for stack in sorted(via_stacks(tech, "met1", low), key=lambda st: stack_jogs(pt, [l1m1] + st)):
            for land in landings:
                new = {}
                for v, at in [(l1m1, pt)] + [(v, pt) for v in stack] + [(land, q)]:
                    for ly in (v.getBottomLayer().getConstName(), v.getTopLayer().getConstName()):
                        if ly != "li1":
                            new.setdefault(ly, []).extend(via_layer_rects(v, at, ly))
                new[low].append(jog)
                ok = True
                for ly, rects in new.items():
                    sp = layer_spacing(tech, ly)
                    if (any(conflicts(r, f, sp) for r in rects for f in foreign.get(ly, ()))
                            or makes_notch(rects, own.get(ly, ()), sp) or self_notch(rects, sp)
                            or makes_neck(rects, own.get(ly, ()), layer_min_width(tech, ly))
                            or connected_area(rects, own.get(ly, ())) < min_area_dbu2(tech, ly)):
                        ok = False
                        break
                if ok:
                    for v, at in [(v, pt) for v in stack] + [(land, q)]:
                        cut, cw, ch, csp = via_cut_rule(v)
                        if cut is None or any(cuts_too_close(at, c, cw, ch, csp) for c in cuts.get(cut, ())):
                            ok = False
                            break
                if ok:
                    return pt, q, layer, stack, land
    return None


def nearest_target(targets, xy):
    """The (layer, point) of the target closest (Manhattan) to `xy`, on the 5 DBU grid."""
    best = None
    for l, a, b in targets:
        p = nearest_on_island([(a, b)], xy)
        d = abs(p[0] - xy[0]) + abs(p[1] - xy[1])
        if best is None or d < best[0]:
            best = (d, (l, p))
    return best[1] if best else None

def nearest_on_island(island, xy):
    """The point of the island's centreline closest (Manhattan) to `xy`, on the 5 DBU grid."""
    snap = lambda v: int(round(v / 5)) * 5
    best = None
    for a, b in island:
        px = min(max(xy[0], min(a[0], b[0])), max(a[0], b[0]))
        py = min(max(xy[1], min(a[1], b[1])), max(a[1], b[1]))
        p = (snap(px) if a[0] != b[0] else px, snap(py) if a[1] != b[1] else py)
        d = abs(p[0] - xy[0]) + abs(p[1] - xy[1])
        if best is None or d < best[0]:
            best = (d, p)
    return best[1] if best else None


def path_rects(pts, half):
    """One rect per segment of an axis-aligned path, grown by `half` on every side."""
    out = []
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        out.append((min(x0, x1) - half, min(y0, y1) - half, max(x0, x1) + half, max(y0, y1) + half))
    return out


def foreign_met1_near(block, tech, net, gx, gy, reach_x=40000, reach_y=12000):
    """Foreign met1 shapes (inflated by the spacing rule) around one gate — collected ONCE for the
    whole site search. `collect_foreign` walks every net, so calling it per candidate site made a
    single pin cost hundreds of full-design scans."""
    from sat_antenna_jog import collect_foreign, layer_spacing
    region = (gx - reach_x, gy - reach_y, gx + reach_x, gy + reach_y)
    # Grown by the wire's own half-width AND the spacing rule: collect_foreign inflates each foreign
    # CENTRELINE, so a zero half-width let a tie land half a wire-width too close.
    from sat_antenna_jog import layer_min_width
    return collect_foreign(block, region, "met1", net.getConstName(),
                           layer_min_width(tech, "met1") // 2, layer_spacing(tech, "met1"))


def tie_is_clear(tech, foreign, pts):
    """True when the met1 tie along `pts`, plus the li1->met1 via's met1 enclosure at its start —
    both at their real extents — overlaps no foreign met1 shape (already grown by the spacing rule;
    rails and cell metal included)."""
    from sat_antenna_jog import conflicts, layer_min_width, layer_spacing
    rects = path_rects(pts, layer_min_width(tech, "met1") // 2)
    via = tech.findVia("L1M1_PR")
    for bx in (via.getBoxes() if via is not None else ()):
        ly = bx.getTechLayer()
        if ly is not None and ly.getConstName() == "met1":
            # A tech via's enclosures are centred on its cut, so its extent is DX x DY about the point.
            hx, hy = bx.getDX() // 2, bx.getDY() // 2
            rects.append((pts[0][0] - hx, pts[0][1] - hy, pts[0][0] + hx, pts[0][1] + hy))
    sp = layer_spacing(tech, "met1")
    return not any(conflicts(r, f, sp) for r in rects for f in foreign)


def plan_drop(tech, foreign, pin_bbox, segs):
    """A tie with NO routed metal: one L1M1 via from the diode's own li1 pin straight up into an
    island met1 segment that passes over the pin. Returns [(x, y)] or None.

    MEASURED: coproc_core _36364_/A_N (RUN_2026.09.24_02.17.38) — all 1,182 sites along its 83 um
    met1 island were refused because an L-shaped met1 tie from a diode a row away had to cross
    horizontal tracks full of foreign met1. sky130's diode_2 pin is a tall li1 rectangle, so a diode
    placed under the island reaches it with the via alone; its met1 enclosure merges with the wire.
    The via's li1 enclosure must sit inside the pin and its met1 enclosure must clear foreign met1."""
    via = tech.findVia("L1M1_PR")
    if via is None:
        return None
    li_half = 0
    for b in via.getBoxes():
        ly = b.getTechLayer()
        if ly is not None and ly.getConstName() == "li1":
            li_half = max(li_half, max(b.getDX(), b.getDY()) // 2)
    x0, y0, x1, y1 = pin_bbox[0] + li_half, pin_bbox[1] + li_half, pin_bbox[2] - li_half, pin_bbox[3] - li_half
    snap = lambda v: int(round(v / 5)) * 5
    for a, b in segs:
        if a[1] == b[1] and y0 <= a[1] <= y1:
            lo, hi = max(x0, min(a[0], b[0])), min(x1, max(a[0], b[0]))
            if lo <= hi:
                pt = (min(max(snap((lo + hi) / 2), lo), hi), a[1])
                if tie_is_clear(tech, foreign, [pt]):
                    return [pt]
        elif a[0] == b[0] and x0 <= a[0] <= x1:
            lo, hi = max(y0, min(a[1], b[1])), min(y1, max(a[1], b[1]))
            if lo <= hi:
                pt = (a[0], min(max(snap((lo + hi) / 2), lo), hi))
                if tie_is_clear(tech, foreign, [pt]):
                    return [pt]
    return None


def plan_or_none(tech, foreign, diode_xy, access):
    """Tie the diode onto the gate's OWN met1 island, so check_antennas credits its diffusion.

    WHY NOT THE li1 STUB THIS REPLACED. That stub ran li1 from the gate pin's CENTRE to the diode
    pin. Measured on coproc_core net _18239_ (RUN_2026.09.22_21.58.12, 28-arc.log): after it was
    placed, the gate's per-layer report still read `li1 Partial area 0.00` and
    `met1 Required 400.00 (Side area) (VIOLATED)` — 400 is the ZERO-diffusion value of
    sky130's met1 ANTENNADIFFSIDEAREARATIO PWL((0 400)(0.0125 400)(0.0225 2609)(22.5 11600)).
    One credited diode_2 (0.4347 um^2) would lift it to ~2774 and the 531.89 ratio would pass at
    0.19. The stub never joined the gate's routed island (the route reaches the pin at its access
    via, not its centre), so the checker never saw it.

    And the same report showed why "already dioded" pins kept violating all along: met2/met3/met4
    read Required 3295/4627/4974 — the heuristic diodes WERE credited, but only from met2 up,
    because the router joined them to the net through met2. At met1's deposit the gate's long met1
    run had no diode on its island. The fix is not more diodes; it is the right LAYER.

    Refuses (returns False) rather than risk a short: the new met1 must clear every foreign met1
    shape by the spacing rule, power rails included — they live in special wires."""
    snap = lambda v: int(round(v / 5)) * 5
    dx, dy = snap(diode_xy[0]), snap(diode_xy[1])
    ax, ay = access
    for pts in ([(dx, dy), (ax, dy), (ax, ay)], [(dx, dy), (dx, ay), (ax, ay)]):
        pts = [p for k, p in enumerate(pts) if k == 0 or p != pts[k - 1]]
        if tie_is_clear(tech, foreign, pts):
            return pts
    return None


def tie_notches_own_net(tech, pts, own_m1):
    """Would the tie's met1 (its path plus the L1M1 enclosure at the diode) sit closer than the
    spacing rule to its OWN net's met1 without merging with it? It merges where it lands; anywhere
    else, a near miss is a same-net notch — which the maximal-rule deck a shuttle is accepted against
    flags and Magic does not (see sat_antenna_jog.makes_notch)."""
    from sat_antenna_jog import makes_notch, layer_min_width, layer_spacing, via_layer_rects
    new = path_rects(pts, layer_min_width(tech, "met1") // 2)
    l1m1 = tech.findVia("L1M1_PR")
    if l1m1 is not None:
        new += via_layer_rects(l1m1, pts[0], "met1")
    return makes_notch(new, own_m1, layer_spacing(tech, "met1"))


def tie_self_notches(tech, pts, land_via=None):
    """Would the tie's OWN met1 notch itself? Its shapes — the path at wire width, the L1M1
    enclosure at the diode and, when it lands on met2, the M1M2 enclosure at the far end — merge
    into one polygon, and two of them closer than the spacing rule with the gap between them left
    empty face each other across a same-net notch. A short L puts the landing enclosure over the
    diode's; an L1M1 enclosure wider than the wire hangs beside the path's next leg."""
    from sat_antenna_jog import self_notch, layer_min_width, layer_spacing, via_layer_rects
    shapes = path_rects(pts, layer_min_width(tech, "met1") // 2)
    l1m1 = tech.findVia("L1M1_PR")
    if l1m1 is not None:
        shapes += via_layer_rects(l1m1, pts[0], "met1")
    if land_via is not None:
        shapes += via_layer_rects(land_via, pts[-1], "met1")
    return self_notch(shapes, layer_spacing(tech, "met1"))


def landing_clear(enc1, enc2, foreign1, foreign2, cut_pt, cuts, rule, sp1=140, sp2=140):
    """Is an M1M2 landing via at the end of a tie legal? Its met1 enclosure must clear foreign met1
    by `sp1`, its met2 enclosure foreign met2 by `sp2` (both foreign sets grown by exactly those
    rules), and its cut must keep the via1 cut-spacing rule to every existing cut nearby."""
    from sat_antenna_jog import conflicts, cuts_too_close
    if any(conflicts(e, f, sp1) for e in enc1 for f in foreign1):
        return False
    if any(conflicts(e, f, sp2) for e in enc2 for f in foreign2):
        return False
    if rule is not None:
        cw, ch, sp = rule
        if any(cuts_too_close(cut_pt, c, cw, ch, sp) for c in cuts):
            return False
    return True


def apply_tie(tech, net, diode_it, pts, land_via=None):
    """Encode the tie: li1 at the diode pin, via up, then `pts` in the encoder's own terms — a point continues the
    wire on the current layer, a tech via climbs from it (the stacked jog's rise) — and, when the target is above
    that layer, the landing via (or via stack, a list) at the end onto the net's metal."""
    via = tech.findVia("L1M1_PR"); li1 = tech.findLayer("li1")
    wire = net.getWire()
    if via is None or li1 is None or wire is None:
        return False
    enc = odb.dbWireEncoder()
    enc.append(wire)
    enc.newPath(li1, "ROUTED")
    enc.addPoint(pts[0][0], pts[0][1])
    enc.addITerm(diode_it)
    enc.addTechVia(via)
    for p in pts[1:]:
        if hasattr(p, "getBottomLayer"):
            enc.addTechVia(p)
        else:
            enc.addPoint(p[0], p[1])
    for v in (land_via if isinstance(land_via, list) else [land_via] if land_via is not None else []):
        enc.addTechVia(v)
    enc.end()
    from sat_antenna_jog import touch_net
    touch_net(net)
    diode_it.setConnected()
    return True


def commit(r, start, diode_span, evict, diode, gaps):
    """Evict fillers (clearing their whole footprint in this row) and claim the diode's sites. Record the
    sites the eviction freed but the diode does NOT occupy — only THOSE need re-filling (nwell continuity)."""
    freed = set()
    for e in evict:
        for i in range(r["n"]):
            if r["occ"][i] is e:
                r["occ"][i] = None
                freed.add(i)
        odb.dbInst_destroy(e)
    for k in range(diode_span):
        r["occ"][start + k] = diode
        freed.discard(start + k)
    for i in freed:
        gaps.append((r, i))


@click.command()
@click.option("--diode-cell", required=True)
@click.option("--diode-pin", required=True)
@click.option("--violator-nets", default=None)
@click.option("--violator-pins", default=None)
@click.option("--manual-stub", is_flag=True, default=False)
@click.option("--max-fanout", type=int, default=0,
              help="Never take a net past this many loads (the SDC's set_max_fanout); 0 sets no limit")
@click_odb
def eco(reader, input_lef, diode_cell, diode_pin, violator_nets, violator_pins, manual_stub, max_fanout):
    block = reader.block
    antenna_rules = lef_antenna_rules(input_lef, diode_cell, diode_pin)
    diode_master = block.getDataBase().findMaster(diode_cell)
    if diode_master is None:
        raise SystemExit(f"[LOGOS-ECO] diode master {diode_cell} not found")
    manual = manual_stub or os.environ.get("ECO_MANUAL_STUB") == "1"
    stubbed = 0
    rows = build_rows(block)
    by_y = {r["y"]: r for r in rows}
    mark_occupancy(rows, by_y, block)
    out_wire_grid = build_wire_grid(block, output_only=True)
    diode_w = diode_master.getWidth()
    diode_h = diode_master.getHeight()
    sw = rows[0]["sw"] if rows else diode_w
    diode_span = (diode_w + sw - 1) // sw

    total = sum(r["n"] for r in rows)
    occ = sum(1 for r in rows for c in r["occ"] if c is not None)
    rem = sum(1 for r in rows for c in r["occ"] if c is not None and is_removable(c))
    print(f"[LOGOS-ECO] rows={len(rows)} sites={total} occupied={occ} "
          f"removable-fill={rem} free={total - occ} diode_span={diode_span}")

    # Build the exact gate pins to protect. PIN-MODE (--violator-pins, "inst/pin" per line straight from the ARC
    # report's `Net X, Pin inst/pin`) drops the diode on the VIOLATING GATE ITSELF — mandatory for a credited
    # fix: check_antennas only counts a diode's diffusion for a gate whose own local branch reaches it at the
    # violating layer's deposit. A diode on a DIFFERENT pin of the same net is unreachable then → uncredited
    # (the exact failure the net-mode manual stub hit: violation at _32913_/C_N, diode on _36883_/A1). NET-MODE
    # (--violator-nets, legacy re-route path) targets the first input pins, since the re-route reworks the net.
    targets = []
    # Each line is "inst/pin" optionally followed by the layer it violates on. A pin violating on
    # several layers is tied for its LOWEST one: that component is the smallest, and a diode on it is
    # connected at every higher layer too.
    viol_layer = {}
    if violator_pins:
        for line in open(violator_pins):
            fields = line.split()
            if not fields or fields[0].endswith("/DIODE"):
                continue
            s = fields[0]
            if len(fields) > 1:
                viol_layer.setdefault(norm(s), []).append(fields[1])
            inst_name, _, pin_name = s.rpartition("/")
            inst = block.findInst(inst_name) or block.findInst(inst_name.replace("\\", ""))
            if inst is None:
                continue
            it = inst.findITerm(pin_name)
            if it is None or it.getNet() is None or it.getNet().isSpecial():
                continue
            targets.append(it)
    elif violator_nets:
        wanted = set(norm(l.strip()) for l in open(violator_nets) if l.strip())
        for net in block.getNets():
            if net.isSpecial() or norm(net.getConstName()) not in wanted:
                continue
            cnt = 0
            for it in net.getITerms():
                if it.isInputSignal() and cnt < 3:
                    targets.append(it)
                    cnt += 1
    else:
        raise SystemExit("[LOGOS-ECO] need --violator-pins or --violator-nets")

    from sat_antenna_jog import decode_wire
    inserted = nofit = refused_fanout = 0
    # A diode is a load on its net (the liberty gives DIODE the default fanout load, 1), so it lands only on a net
    # with room under the limit. The pre-route fanout step holds every net carrying a diode one load under it for
    # exactly this diode (logos_diode_fanout.tcl); a net without room keeps its violation for the next pass to see.
    net_loads = {}
    seen = set()
    touched = set()
    diodes = []
    evict_gaps = []
    for it in targets:
        inst = it.getInst()
        key = inst.getConstName() + "/" + it.getMTerm().getConstName()
        if key in seen:
            continue
        seen.add(key)
        loc = inst.getLocation()
        name = "ANTENNA_ECO_" + inst.getConstName() + "_" + it.getMTerm().getConstName()
        if block.findInst(name) is not None:
            continue
        net = it.getNet()
        if max_fanout > 0:
            nname = net.getConstName()
            if nname not in net_loads:
                net_loads[nname] = sum(1 for t in net.getITerms() if t.isInputSignal())
            if net_loads[nname] + 1 > max_fanout:
                print(f"[LOGOS-ECO] fanout refused {key}: net {nname} carries {net_loads[nname]} loads, "
                      f"at the limit of {max_fanout}")
                refused_fanout += 1
                continue
        tech = block.getDataBase().getTech()
        access = gate_access_point(it.getNet(), it) if manual else None
        if manual and access is None:
            print(f"[LOGOS-ECO] tie refused {key}: no li1->met1 access via on the net inside the pin")
            nofit += 1
            continue
        if manual:
            wops = decode_wire(it.getNet().getWire())
            layers = viol_layer.get(norm(key), [])
            vias = wire_vias(wops)
            lowest = min(layers, key=lambda l: island_rank(vias, l)) if layers else "met1"
            from sat_antenna_jog import net_layer_rects
            own_m1 = net_layer_rects(wops, tech, "met1")
            own_m2 = net_layer_rects(wops, tech, "met2")
            wsegs = wire_segments(wops)
            from sat_antenna_jog import layer_min_width
            widths = {l: layer_min_width(tech, l) for l in antenna_rules["layers"]}
            dbu = tech.getDbUnitsPerMicron()
            gate_comps = {}
            safe = lambda t: landing_is_safe(wsegs, vias, access, t, lowest, antenna_rules, dbu, widths,
                                             gate_comps=gate_comps)
            targets_t = tie_targets(wire_segments(wops), vias, access, lowest)
            targets_t = [t for t in targets_t if safe(t)]
            if not targets_t:
                targets_t = [("met1", access, access)]
            stack_t = [t for t in drop_targets(wsegs, vias, access, lowest) if t[0] != "met1" and safe(t)]
            island = [(a, b) for _l, a, b in targets_t] + [(a, b) for _l, a, b in stack_t]
        else:
            island, targets_t, stack_t = [], [], []
        if manual:
            xs = [x for a, b in island for x in (a[0], b[0])]
            ys = [y for a, b in island for y in (a[1], b[1])]
            foreign = foreign_met1_near(block, tech, it.getNet(), (min(xs) + max(xs)) // 2,
                                        (min(ys) + max(ys)) // 2,
                                        reach_x=(max(xs) - min(xs)) // 2 + 40000,
                                        reach_y=(max(ys) - min(ys)) // 2 + 12000)
            cands = island_candidates(rows, island, diode_span, out_wire_grid, diode_h)
            land = tech.findVia("M1M2_PR")
            foreign2, cuts, rule = [], [], None
            if any(l == "met2" for l, _a, _b in targets_t) and land is not None:
                from sat_antenna_jog import collect_foreign, collect_cuts, via_cut_rule, layer_spacing, layer_min_width
                region = ((min(xs) + max(xs)) // 2 - (max(xs) - min(xs)) // 2 - 40000,
                          (min(ys) + max(ys)) // 2 - (max(ys) - min(ys)) // 2 - 12000,
                          (min(xs) + max(xs)) // 2 + (max(xs) - min(xs)) // 2 + 40000,
                          (min(ys) + max(ys)) // 2 + (max(ys) - min(ys)) // 2 + 12000)
                foreign2 = collect_foreign(block, region, "met2", it.getNet().getConstName(),
                                           layer_min_width(tech, "met2") // 2, layer_spacing(tech, "met2"))
                cut_name, cw, ch, csp = via_cut_rule(land)
                cuts = collect_cuts(block, region, cut_name)
                rule = (cw, ch, csp)
            stack_foreign, stack_own, stack_cuts = {"met1": foreign}, {"met1": own_m1}, {}
            if stack_t:
                from sat_antenna_jog import collect_foreign, collect_cuts, via_cut_rule, layer_spacing, layer_min_width
                region = (min(xs) - 40000, min(ys) - 12000, max(xs) + 40000, max(ys) + 12000)
                top = max((l for l, _a, _b in stack_t), key=lambda l: island_rank(vias, l))
                for stack in via_stacks(tech, "met1", top):
                    for v in stack:
                        for ly in (v.getBottomLayer().getConstName(), v.getTopLayer().getConstName()):
                            if ly not in stack_foreign:
                                stack_foreign[ly] = collect_foreign(block, region, ly, it.getNet().getConstName(),
                                                                    layer_min_width(tech, ly) // 2, layer_spacing(tech, ly))
                                stack_own[ly] = net_layer_rects(wops, tech, ly)
                        cut = via_cut_rule(v)[0]
                        if cut is not None and cut not in stack_cuts:
                            stack_cuts[cut] = collect_cuts(block, region, cut)
        else:
            foreign, cands, targets_t, stack_t = [], [], [], []
        if not manual:
            spot = reserve(rows, by_y, loc[0], loc[1], diode_span, out_wire_grid, diode_h)
            cands = [(0,) + spot] if spot else []
        placed = None
        for _dist, r, start, evict in cands[:1200]:
            d = odb.dbInst_create(block, diode_master, name)
            d.setOrient(r["orient"])
            d.setLocation(int(r["x0"] + start * r["sw"]), int(r["y"]))
            d.setPlacementStatus("PLACED")
            dit = d.findITerm(diode_pin)
            if manual:
                ok, px, py = dit.getAvgXY()
                bb = dit.getBBox()
                drop = plan_drop(tech, foreign, (bb.xMin(), bb.yMin(), bb.xMax(), bb.yMax()),
                                 [(a, b) for l, a, b in targets_t if l == "met1"])
                stacked = None
                if drop is not None:
                    tl, access, pts = "met1", drop[0], drop
                else:
                    pin_bb = (bb.xMin(), bb.yMin(), bb.xMax(), bb.yMax())
                    stacked = plan_stack_drop(tech, pin_bb, stack_t, stack_foreign, stack_own, stack_cuts) if stack_t else None
                    jogged = None if stacked is not None or not stack_t else \
                        plan_stack_jog(tech, pin_bb, stack_t, stack_foreign, stack_own, stack_cuts)
                    if stacked is not None:
                        tl, access, pts = stacked[1], stacked[0], [stacked[0]]
                    elif jogged is not None:
                        tl, access, pts = jogged[2], jogged[1], [jogged[0]] + list(jogged[3]) + [jogged[1]]
                        stacked = (jogged[1], jogged[2], jogged[4])
                    else:
                        tl, access = nearest_target(targets_t, (px, py)) if ok else (None, None)
                        pts = plan_or_none(tech, foreign, (px, py), access) if ok else None
                landing = stacked[2] if stacked is not None else None
                if pts is not None and stacked is None and tie_notches_own_net(tech, pts, own_m1):
                    pts = None
                if pts is not None and stacked is None and tie_self_notches(tech, pts, land if tl == "met2" else None):
                    pts = None
                if pts is not None and stacked is None and tl == "met2":
                    from sat_antenna_jog import via_layer_rects
                    from sat_antenna_jog import makes_notch, layer_spacing as _sp
                    own_ok = not makes_notch(via_layer_rects(land, access, "met1"), own_m1, _sp(tech, "met1")) \
                        and not makes_notch(via_layer_rects(land, access, "met2"), own_m2, _sp(tech, "met2"))
                    if own_ok and landing_clear(via_layer_rects(land, access, "met1"), via_layer_rects(land, access, "met2"),
                                                foreign, foreign2, access, cuts, rule,
                                                _sp(tech, "met1"), _sp(tech, "met2")):
                        landing = land
                    else:
                        pts = None
                if pts is None:
                    odb.dbInst_destroy(d)
                    continue
            else:
                pts = None
            commit(r, start, diode_span, evict, d, evict_gaps)
            dit.connect(it.getNet())
            if manual:
                apply_tie(tech, it.getNet(), dit, pts, landing)
                stubbed += 1
            placed = d
            break
        if placed is None:
            print(f"[LOGOS-ECO] no diode site near {key} admits a clear met1 tie "
                  f"({len(cands)} candidate sites tried)")
            nofit += 1
            continue
        d = placed
        touched.add(it.getNet().getConstName())
        diodes.append(d)
        inserted += 1
        if it.getNet().getConstName() in net_loads:
            net_loads[it.getNet().getConstName()] += 1
    hit = len(targets)

    # Diodes are placed NEAR their gate (antenna discharges → 0) but CLEAR of output-net wiring (reserve's
    # out_wire_grid), so none lands under a rout[*] wire — no static short, and the outputs are never touched
    # (rerouting them just relocates the short, as the prior run showed). Only the violators are unrouted, so
    # antenna still converges. This is the reconciliation: antenna=0 AND short-free AND minimal perturbation.
    # MANUAL-STUB mode connects every diode with its own li1 stub above (no router), so the routing is already
    # complete and consistent — DON'T unroute anything and DON'T re-route. Any router re-route (full OR
    # incremental) is the fundamental LVS-breaker: the full pass rebuilds the whole design and STRANDS
    # ~800-1200 nets; the incremental pass leaves the diode pins unconnected and crashes check_antennas.
    # The stub sidesteps both — the diode is already wired, the base route is untouched, LVS stays clean.
    cleared = dnt = 0
    if not manual:
        from sat_antenna_jog import touch_net
        for net in block.getNets():
            if net.isSpecial() or net.getConstName() not in touched:
                continue
            w = net.getWire()
            if w is not None:
                odb.dbWire_destroy(w)
                touch_net(net)
                cleared += 1

        # Mark EVERY net except the unrouted violators as don't-touch, so the caller's re-route routes ONLY
        # the ~15 diode nets and PRESERVES the other 36k. (Empirically the routers IGNORE this flag, which is
        # why the manual stub above is the real fix — kept here only for the legacy re-route path.)
        for net in block.getNets():
            if net.isSpecial():
                continue
            keep = net.getConstName() not in touched
            net.setDoNotTouch(keep)
            if keep:
                dnt += 1

    # Re-fill ONLY the sites MY eviction freed (evict_gaps) with a 1-site filler, so the nwell/implant stays
    # continuous across each eviction boundary — that boundary is the tiny XOR sliver (Magic vs KLayout).
    # Filling every empty site in the design (22k+) instead was the LVS-breaker: it desynced the netlist vs
    # the extracted layout. The design's OTHER empty sites were XOR-clean in the baseline, so leave them.
    fill1 = block.getDataBase().findMaster("sky130_fd_sc_hd__fill_1")
    filled = 0
    if fill1 is not None:
        for (r, s) in evict_gaps:
            occ = r["occ"]
            if occ[s] is not None:
                continue
            name = "ANTENNA_ECO_FILL_%d_%d" % (r["y"], s)
            if block.findInst(name) is not None:
                continue
            f = odb.dbInst_create(block, fill1, name)
            f.setOrient(r["orient"])
            f.setLocation(int(r["x0"] + s * r["sw"]), int(r["y"]))
            f.setPlacementStatus("PLACED")
            occ[s] = f
            filled += 1

    print(f"[LOGOS-ECO] {hit} target pins; inserted {inserted} near+output-clear diodes "
          f"({nofit} no-fit, {refused_fanout} at the fanout limit); stubbed {stubbed} (manual={manual}); "
          f"unrouted {cleared} violator nets; "
          f"refilled {filled} gap sites")


if __name__ == "__main__":
    eco()
