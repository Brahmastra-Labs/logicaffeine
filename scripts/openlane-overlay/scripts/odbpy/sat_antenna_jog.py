# sat_antenna_jog.py — SAT-based antenna-repair jog router for LOGOS silicon.
#
# check_antennas' side-area (perimeter) antenna violations cannot be fixed by diodes (which only satisfy the
# gate-AREA rule). They need the offending metal broken up — a portion of the layer-L wire promoted to L+1 so
# that, at layer L's deposit, less of it connects to the gate. This module does that surgery: it re-encodes a
# net's routed wire, promoting chosen segments to the next metal up (with M{L}M{L+1} vias at the cut points).
# Which segments to promote is a shared, multi-gate constraint problem solved by our own SAT engine.
#
# Phase 1 (this file, first cut): the wire re-encode primitive + a faithful round-trip, the foundation every
# later phase stands on. If a wire can't be decoded and rebuilt byte-faithfully, no jog is safe.
import os
import re
import functools
import odb
import click
from reader import click_odb


def norm(s):
    return s.replace("\\", "")


LAYER_UP = {"li1": "met1", "met1": "met2", "met2": "met3", "met3": "met4", "met4": "met5"}
# tech via between a layer and the one above it (sky130 SC_HD names)
VIA_UP = {"li1": "L1M1_PR", "met1": "M1M2_PR", "met2": "M2M3_PR", "met3": "M3M4_PR", "met4": "M4M5_PR"}
# for a top-layer (met5) violation there is nothing above to jog to, so bridge DOWN one metal instead: moving
# the run to L-1 removes it from layer L's side-area (L-1 exists at L deposit, so the gate stays connected).
LAYER_DOWN = {"met2": "met1", "met3": "met2", "met4": "met3", "met5": "met4"}
VIA_DOWN = {"met2": "M1M2_PR", "met3": "M2M3_PR", "met4": "M3M4_PR", "met5": "M4M5_PR"}


def _oriented_via(tech, base, horizontal):
    """The variant of `base` whose enclosure on the VIOLATING layer runs along the wire.

    sky130 ships each via in two orientations that are transposes of each other:
        M1M2_PR    met1 320 x 260   met2 260 x 320
        M1M2_PR_R  met1 260 x 320   met2 320 x 260
    so the pair is built for the preferred-direction stack (horizontal met1, vertical met2) and
    NEITHER variant puts both enclosures along the same axis. A bridge runs both layers the same
    way, so one enclosure is always across its wire; this picks the variant that suits the layer
    the violation is on, whose stub is the one being left behind.

    THIS DID NOT FIX THE XOR AND IS NOT CLAIMED TO. Measured on coproc_core: XOR was 3 with the
    sliver guard alone and 3 with this as well — identical. The cause was via CUT SPACING, not
    enclosure orientation; see `via_cut_rule`. Recorded here so the next reader does not inherit a
    false cause. Kept because matching the violating layer is the better default, not because it
    moved a number.

    Falls back to the unrotated via when a PDK ships no rotated variant, so a process without
    them still bridges rather than losing the repair entirely.
    """
    name = base if horizontal else f"{base}_R"
    v = tech.findVia(name)
    return v if v is not None else tech.findVia(base)


def bridge_layer_via(tech, layer, horizontal=True):
    """Where to route the bridge for a violation on `layer`, and the via to get there. Up one metal normally;
    down one metal for the top layer. `horizontal` is the SEGMENT's direction, which selects the via
    orientation whose enclosure lies along the wire. Returns (bridge_layer_name, dbTechVia) or (None, None)."""
    up = LAYER_UP.get(layer)
    if up is not None:
        return up, _oriented_via(tech, VIA_UP.get(layer, ""), horizontal)
    dn = LAYER_DOWN.get(layer)
    if dn is not None:
        return dn, _oriented_via(tech, VIA_DOWN.get(layer, ""), horizontal)
    return None, None


def decode_wire(w):
    """Decode a dbWire into a flat op list we can replay through the encoder. Each op is a tuple whose first
    element is the kind. Points carry absolute coords; PATH carries the dbTechLayer; vias carry the master."""
    dec = odb.dbWireDecoder()
    dec.begin(w)
    ops = []
    op = dec.next()
    while op != odb.dbWireDecoder.END_DECODE:
        if op == odb.dbWireDecoder.PATH:
            ops.append(("PATH", dec.getLayer()))
        elif op == odb.dbWireDecoder.JUNCTION:
            ops.append(("JUNCTION", dec.getJunctionValue(), dec.getLayer()))
        elif op == odb.dbWireDecoder.SHORT:
            ops.append(("SHORT", dec.getJunctionValue(), dec.getLayer()))
        elif op == odb.dbWireDecoder.VWIRE:
            ops.append(("VWIRE", dec.getJunctionValue(), dec.getLayer()))
        elif op == odb.dbWireDecoder.POINT:
            x, y = dec.getPoint()
            ops.append(("POINT", x, y))
        elif op == odb.dbWireDecoder.POINT_EXT:
            p = dec.getPoint()
            ext = p[2] if len(p) > 2 else 0
            ops.append(("POINT_EXT", p[0], p[1], ext))
        elif op == odb.dbWireDecoder.TECH_VIA:
            ops.append(("TECH_VIA", dec.getTechVia()))
        elif op == odb.dbWireDecoder.VIA:
            ops.append(("VIA", dec.getVia()))
        elif op == odb.dbWireDecoder.ITERM:
            ops.append(("ITERM", dec.getITerm()))
        elif op == odb.dbWireDecoder.BTERM:
            ops.append(("BTERM", dec.getBTerm()))
        elif op == odb.dbWireDecoder.RECT:
            ops.append(("RECT",))   # via-enclosure patch; not needed for occupancy or these nets' re-encode
        op = dec.next()
    return ops


def def_route_rects(def_text, net_name):
    """Every RECT patch on `net_name`'s route in a DEF, as (layer, x, y, dx1, dy1, dx2, dy2) in
    route order — each anchored at the point it is written after, on its layer.

    The odb Python binding cannot read a RECT patch (`dbWireDecoder.getRect` takes C++ out-
    parameters), so the DEF of the same database state is where the geometry is read from.
    MEASURED: 1,344 such patches in fabric_horner_seq's routed DEF; refusing every wire that carried
    one left net580 (met3, ratio 1.40) with no repair at all."""
    # Only the NETS section: a net named like a port (`clk`) also opens an entry in PINS.
    ns = re.search(r"^NETS\s", def_text, re.M)
    nets = def_text[ns.start():] if ns else def_text
    m = re.search(r"^\s*-\s+" + re.escape(net_name) + r"\s", nets, re.M)
    if not m:
        return []
    end = re.search(r"^\s*-\s+\S|^END NETS", nets[m.end():], re.M)
    body = nets[m.start(): m.end() + (end.start() if end else len(nets))]
    pat = re.compile(r"(?:\+\s*ROUTED|NEW)\s+(\S+)\s+\(\s*(-?\d+)\s+(-?\d+)(?:\s+-?\d+)?\s*\)\s+"
                     r"RECT\s+\(\s*(-?\d+)\s+(-?\d+)\s+(-?\d+)\s+(-?\d+)\s*\)")
    return [(g[0],) + tuple(int(v) for v in g[1:]) for g in pat.findall(body)]


def attach_rects(ops, rects):
    """Give every bare ("RECT",) op its geometry from `rects`, matched on its anchor — the last point
    and the layer the path is on there. Returns None when any patch cannot be matched, or any parsed
    patch is left over: a wire is re-encoded with every patch it had, or not at all, because a dropped
    patch is geometry Magic and KLayout then disagree about (an XOR difference)."""
    pool = list(rects)
    out, layer, last = [], None, None
    name = lambda l: l.getConstName() if l is not None else None
    for o in ops:
        k = o[0]
        if k == "PATH":
            layer, last = name(o[1]), None
        elif k in ("JUNCTION", "SHORT", "VWIRE"):
            layer, last = name(o[2]), None
        elif k in ("POINT", "POINT_EXT"):
            last = (o[1], o[2])
        elif k in ("TECH_VIA", "VIA"):
            bot, top = name(o[1].getBottomLayer()), name(o[1].getTopLayer())
            layer = top if layer == bot else bot
        elif k == "RECT" and len(o) == 1:
            hit = next((r for r in pool if last is not None and r[:3] == (layer, last[0], last[1])), None)
            if hit is None:
                return None
            pool.remove(hit)
            out.append(("RECT",) + hit[3:])
            continue
        out.append(o)
    return None if pool else out


def encode_wire(net, ops):
    """Rebuild net's wire from a decoded op list. Junctions are re-established through the encoder's junction
    ids: every POINT returns a fresh id, and we map the decoder's junction value to it so JUNCTION/SHORT ops
    can branch from the right point."""
    old = net.getWire()
    if old is not None:
        odb.dbWire_destroy(old)
    w = odb.dbWire_create(net)
    enc = odb.dbWireEncoder()
    enc.begin(w)
    dec_jid_of_point = {}   # decoder point-index -> decoder junction id (position in stream)
    enc_jid = {}            # decoder junction id -> encoder junction id
    point_ct = 0
    started = False
    for o in ops:
        k = o[0]
        if k == "PATH":
            enc.newPath(o[1], "ROUTED")
            started = True
        elif k in ("JUNCTION", "SHORT", "VWIRE"):
            djid, layer = o[1], o[2]
            ejid = enc_jid.get(djid, 0)
            if k == "JUNCTION":
                enc.newPath(ejid, layer, "ROUTED")
            elif k == "SHORT":
                enc.newPathShort(ejid, layer, "ROUTED")
            else:
                enc.newPathVirtualWire(ejid, layer, "ROUTED")
            started = True
        elif k == "POINT":
            jid = enc.addPoint(o[1], o[2])
            enc_jid[point_ct] = jid
            point_ct += 1
        elif k == "POINT_EXT":
            jid = enc.addPoint(o[1], o[2], o[3])
            enc_jid[point_ct] = jid
            point_ct += 1
        elif k == "TECH_VIA":
            enc.addTechVia(o[1])
        elif k == "VIA":
            enc.addVia(o[1])
        elif k == "ITERM":
            enc.addITerm(o[1])
        elif k == "BTERM":
            enc.addBTerm(o[1])
        elif k == "RECT":
            if len(o) != 5:
                raise ValueError(f"{net.getConstName()}: a RECT patch without geometry would be DROPPED "
                                 "on re-encode — attach_rects must run first")
            enc.addRect(o[1], o[2], o[3], o[4])
    enc.end()
    touch_net(net)
    return point_ct


def segments(ops):
    """Group the flat op list into segments: each is (start_index, layer_name, [point ops], has_via).
    A segment is one PATH/JUNCTION/SHORT run and its points. Only simple 2-point via-free segments are
    promotion candidates (promoting through an intermediate via/branch would corrupt connectivity)."""
    segs = []
    cur = None
    for i, o in enumerate(ops):
        if o[0] in ("PATH", "JUNCTION", "SHORT", "VWIRE"):
            if cur is not None:
                segs.append(cur)
            layer = o[1] if o[0] == "PATH" else o[2]
            cur = {"start": i, "layer": layer.getConstName() if layer else None,
                   "pts": [], "via": False}
        elif o[0] in ("POINT", "POINT_EXT") and cur is not None:
            cur["pts"].append((o[1], o[2]))
        elif o[0] in ("TECH_VIA", "VIA") and cur is not None:
            cur["via"] = True
        elif o[0] == "RECT" and cur is not None:
            cur["rect"] = True
    if cur is not None:
        segs.append(cur)
    return segs


def seg_len(s):
    if len(s["pts"]) < 2:
        return 0
    (ax, ay), (bx, by) = s["pts"][0], s["pts"][-1]
    return abs(bx - ax) + abs(by - ay)


def promote_ops(ops, tech, promote_starts):
    """Return a new op list with the segments whose PATH is at an index in promote_starts rebuilt one metal
    up: a via at the first point (layer -> layer+1), the run on layer+1, a via at the last point (back down).
    So at the original layer's deposit the run is gone; the endpoints stay on the original layer to keep the
    coincident-point connectivity to neighbours."""
    out = []
    i = 0
    n = len(ops)
    while i < n:
        o = ops[i]
        if o[0] == "PATH" and i in promote_starts:
            layer = o[1]
            lname = layer.getConstName()
            up, via = bridge_layer_via(tech, lname)
            # collect this segment's points (until the next PATH-like op)
            pts = []
            j = i + 1
            while j < n and ops[j][0] in ("POINT", "POINT_EXT"):
                pts.append(ops[j])
                j += 1
            if up and via is not None and len(pts) >= 2:
                out.append(("PATH", layer))
                out.append(pts[0])              # first point on original layer
                out.append(("TECH_VIA", via))   # up to layer+1
                for p in pts[1:]:
                    out.append(p)               # run on layer+1
                out.append(("TECH_VIA", via))   # back down to original layer
            else:
                out.append(o)
                out.extend(pts)
            i = j
        else:
            out.append(o)
            i += 1
    return out


def seg_rect(pts, half):
    (ax, ay), (bx, by) = pts[0], pts[-1]
    return (min(ax, bx) - half, min(ay, by) - half, max(ax, bx) + half, max(ay, by) + half)


def overlaps(r1, r2):
    return not (r1[2] < r2[0] or r2[2] < r1[0] or r1[3] < r2[1] or r2[3] < r1[1])


def rect_gap(a, b):
    """Edge-to-edge distance between two rects (0 when they touch or overlap); Euclidean between
    corners when they are offset on both axes, as a DRC space check measures it."""
    dx = max(0, b[0] - a[2], a[0] - b[2])
    dy = max(0, b[1] - a[3], a[1] - b[3])
    if dx == 0 or dy == 0:
        return dx + dy
    return (dx * dx + dy * dy) ** 0.5


def conflicts(r, grown, spacing):
    """Does `r` (a real extent) come within `spacing` of the foreign shape behind `grown`?
    Every rect `collect_foreign` returns is its shape's real extent grown by exactly the spacing
    rule, so the shape is recovered by shrinking it back and the distance measured as a DRC space
    check does — Euclidean between corners. Box overlap against the grown rect alone reads two
    rects offset on BOTH axes by less than the rule as a violation even when their corners are
    legally apart (110 nm each way is 155.6 nm, clear of met1's 140)."""
    real = (grown[0] + spacing, grown[1] + spacing, grown[2] - spacing, grown[3] - spacing)
    return rect_gap(r, real) < spacing


def makes_notch(new_rects, net_rects, spacing):
    """True when a new shape sits closer than `spacing` to metal of its OWN net without merging
    with it. Same-net metal either overlaps (one polygon) or keeps the full spacing; anything in
    between is a notch.

    Magic's DRC does not flag a same-net notch; the maximal-rule deck a shuttle accepts against
    does. MEASURED: infinichip net308 (RUN_2026.09.23_09.31.39) — a bridge via at (7930, 191250)
    put its met1 enclosure 35 nm from the pin's L1M1 access-via enclosure, and golden sky130A_mr
    reported m1.2 on that exact edge pair while OpenLane's Magic DRC read 0."""
    return any(0 < rect_gap(n, e) < spacing for n in new_rects for e in net_rects)


def neck_width(a, b):
    """The width of the neck two same-layer rects leave where they merge, or None where they leave
    none. Rects that overlap or touch while each sticks out past the other on BOTH axes join through
    their overlap alone, and the merged polygon is only as wide there as that overlap's diagonal —
    the distance a Euclidean width check measures between its two concave corners (zero for corners
    touching at a point, the shared length for edges abutting along part of a side). Rects disjoint
    on an axis are the notch guard's case; one spanning the other on an axis merges as a step."""
    if rect_gap(a, b) > 0:
        return None
    ox = min(a[2], b[2]) - max(a[0], b[0])
    oy = min(a[3], b[3]) - max(a[1], b[1])
    spans_x = (a[0] <= b[0] and b[2] <= a[2]) or (b[0] <= a[0] and a[2] <= b[2])
    spans_y = (a[1] <= b[1] and b[3] <= a[3]) or (b[1] <= a[1] and a[3] <= b[3])
    if spans_x or spans_y:
        return None
    return (ox * ox + oy * oy) ** 0.5


def makes_neck(new_rects, net_rects, width):
    """True when a new shape merges with metal of its own net, or with another new shape, through a
    neck narrower than the width rule — unless the neck is buried in the net's solid metal (a square
    of the width round the overlap, covered by the shapes together). Same-net metal either overlaps
    into a polygon at least the width across, or keeps the spacing (makes_notch).

    The maximal-rule deck checks the merged polygon; Magic does not. MEASURED: gemm_mac_realbist net
    _06199_ (RUN_2026.10.03_08.33.45) — a met3->met4 bridge's met4 enclosure over the net's own met4
    via enclosure, 215 nm apart on both axes: a 75 x 75 nm corner overlap, golden m4.1 x2."""
    shapes = list(new_rects) + list(net_rects)
    pairs = [(a, b) for i, a in enumerate(new_rects) for b in list(new_rects[i + 1:]) + list(net_rects)]
    for a, b in pairs:
        w = neck_width(a, b)
        if w is None or w >= width:
            continue
        cx = (max(a[0], b[0]) + min(a[2], b[2])) / 2
        cy = (max(a[1], b[1]) + min(a[3], b[3])) / 2
        box = (cx - width / 2, cy - width / 2, cx + width / 2, cy + width / 2)
        clipped = [(max(r[0], box[0]), max(r[1], box[1]), min(r[2], box[2]), min(r[3], box[3])) for r in shapes]
        clipped = [c for c in clipped if c[0] < c[2] and c[1] < c[3]]
        if union_area(clipped) < width * width:
            return True
    return False


def union_area(rects):
    """Area of the union of axis-aligned rects (DBU^2), by coordinate compression."""
    xs = sorted({v for r in rects for v in (r[0], r[2])})
    ys = sorted({v for r in rects for v in (r[1], r[3])})
    area = 0
    for i in range(len(xs) - 1):
        for j in range(len(ys) - 1):
            cx, cy = (xs[i] + xs[i + 1]) / 2, (ys[j] + ys[j + 1]) / 2
            if any(r[0] <= cx <= r[2] and r[1] <= cy <= r[3] for r in rects):
                area += (xs[i + 1] - xs[i]) * (ys[j + 1] - ys[j])
    return area


def connected_area(seeds, pool):
    """Area of the metal island formed by `seeds` plus every `pool` rect joined to it (touching counts)."""
    island, rest = list(seeds), list(pool)
    grown = True
    while grown:
        grown = False
        for r in list(rest):
            if any(overlaps(r, q) for q in island):
                island.append(r); rest.remove(r); grown = True
    return union_area(island)


def min_area_dbu2(tech, layer_name):
    """The layer's minimum area in DBU^2 (0 when the tech declares none)."""
    try:
        ly = tech.findLayer(layer_name)
        a = ly.getArea() if ly is not None and ly.hasArea() else 0.0
        dbu = tech.getDbUnitsPerMicron()
        return int(round(a * dbu * dbu))
    except Exception:
        return 0


def via_extent_along(via, layer_name, horizontal):
    """How far `via`'s enclosure on `layer_name` extends along a horizontal or vertical wire."""
    for b in via.getBoxes():
        ly = b.getTechLayer()
        if ly is not None and ly.getConstName() == layer_name:
            return b.getDX() if horizontal else b.getDY()
    return 0


def min_bridge_window(extent_along, spacing):
    """The shortest bridge window whose two vias' enclosures clear the spacing rule on every layer
    they touch: max over the layers of (enclosure extent along the wire + spacing). MEASURED on
    sky130: M1M2 460, M2M3 630, M3M4 680 DBU — a fixed 500/340/240/180 ladder self-notched every
    met3->met4 bridge (see self_notch)."""
    return max(extent_along[l] + spacing[l] for l in extent_along)


def bridge_widths(floor, ladder=(500, 340, 240, 180)):
    """The window ladder lifted to `floor`, longest first, without duplicates."""
    return sorted({max(w, floor) for w in ladder}, reverse=True)


def self_notch(rects, spacing):
    """True when two NEW shapes of one bridge notch each other. A bridge's two vias sit one window
    apart; if their enclosures are closer than the spacing rule they notch on the lifted layer (its
    metal between them is gone) and on the bridge layer (the enclosures overhang the bridge wire).
    MEASURED: fabric_horner_seq net _07269_ (RUN_2026.09.23_12.05.48), a 340 nm window — met1
    enclosures 20 nm apart, met2 80 nm apart; golden sky130A_mr m1.2 + 2x m2.2, Magic silent."""
    return any(0 < rect_gap(a, b) < spacing and not gap_filled(a, b, rects)
               for i, a in enumerate(rects) for b in rects[i + 1:])


def gap_rect(a, b):
    """The empty region between two disjoint rects: across the axis they are separated on, over the
    span they share on the other (the corner square when they are separated on both)."""
    xs = sorted((a[0], a[2], b[0], b[2]))[1:3] if a[2] < b[0] or b[2] < a[0] else (max(a[0], b[0]), min(a[2], b[2]))
    ys = sorted((a[1], a[3], b[1], b[3]))[1:3] if a[3] < b[1] or b[3] < a[1] else (max(a[1], b[1]), min(a[3], b[3]))
    return (xs[0], ys[0], xs[1], ys[1])


def gap_filled(a, b, rects):
    """Is the gap between `a` and `b` entirely covered by the shape's own metal? Then the merged
    polygon has no facing edges there, and no notch."""
    g = gap_rect(a, b)
    area = (g[2] - g[0]) * (g[3] - g[1])
    clipped = [(max(r[0], g[0]), max(r[1], g[1]), min(r[2], g[2]), min(r[3], g[3])) for r in rects]
    clipped = [c for c in clipped if c[0] < c[2] and c[1] < c[3]]
    return area > 0 and union_area(clipped) >= area


def via_layer_rects(via, pt, layer_name):
    """The absolute rects `via` places on `layer_name` when dropped at `pt`."""
    out = []
    for b in via.getBoxes():
        ly = b.getTechLayer()
        if ly is not None and ly.getConstName() == layer_name:
            out.append((pt[0] + b.xMin(), pt[1] + b.yMin(), pt[0] + b.xMax(), pt[1] + b.yMax()))
    return out


def net_layer_rects(ops, tech, layer_name):
    """Every shape a net's wire already draws on `layer_name`: its segments at the layer's width,
    and the enclosure of every via that touches the layer."""
    half = layer_min_width(tech, layer_name) // 2
    out, layer, last = [], None, None
    for o in ops:
        k = o[0]
        if k == "PATH":
            layer, last = (o[1].getConstName() if o[1] else None), None
        elif k in ("JUNCTION", "SHORT", "VWIRE"):
            layer, last = (o[2].getConstName() if o[2] else None), None
        elif k in ("POINT", "POINT_EXT"):
            p = (o[1], o[2])
            if last is not None and layer == layer_name:
                out.append(seg_rect([last, p], half))
            last = p
        elif k == "TECH_VIA" and last is not None:
            v = o[1]
            out += via_layer_rects(v, last, layer_name)
            bot, top = v.getBottomLayer().getConstName(), v.getTopLayer().getConstName()
            layer = top if layer == bot else bot
    return out


def layer_spacing(tech, layer_name, fallback=140):
    """The layer's minimum SPACING from the tech, in DBU. Falls back to 140 (0.14 um, the sky130
    met1/met2 rule) if the tech cannot answer, because a clearance of zero is never right."""
    try:
        ly = tech.findLayer(layer_name)
        sp = ly.getSpacing() if ly is not None else 0
        return int(sp) if sp and int(sp) > 0 else fallback
    except Exception:
        return fallback


def layer_min_width(tech, layer_name, fallback=140):
    """The layer's minimum WIDTH from the tech, in DBU (sky130 met1/met2 = 0.14 um = 140)."""
    try:
        ly = tech.findLayer(layer_name)
        w = ly.getWidth() if ly is not None else 0
        return int(w) if w and int(w) > 0 else fallback
    except Exception:
        return fallback


def via_cut_rule(via):
    """The cut-spacing rule for a tech via, read from the PDK: (cut_layer_name, cw, ch, spacing) in DBU.

    THE DEFECT THIS EXISTS TO PREVENT. A bridge places a via at each end of its window, so the
    window width IS the via-to-via CENTRE distance. The ladder tried 500/340/240/180 nm — but
    sky130's `via` (met1<->met2) is WIDTH 0.15 SPACING 0.17, so two cuts need 0.32 um between
    centres. 240 and 180 are illegal on every layer; 340 is illegal for via2/via3 as well
    (WIDTH 0.20 SPACING 0.20 -> 0.40). Two of the four rungs could never produce legal geometry.

    MEASURED on coproc_core RUN_2026.09.22_05.14.34 — the run that carried BOTH earlier geometry
    fixes and still failed XOR with 3. Reading the two GDS files at the XOR coordinates, on layer
    68/44, which is the `via` CUT layer and not any enclosure:
        KLAYOUT (streams the DEF verbatim)  cuts at 577.795-577.945 and 578.035-578.185
                                            -> 0.090 um apart, against a 0.170 um rule
        MAGIC   (paints contacts as AREAS, regenerating cuts when it streams GDS)
                                            merges them into ONE cut at 577.915-578.065
    and at the other site KLayout wrote 3 cuts (gaps 0.110, 0.090) where Magic wrote 2 (gap 0.170).
    So Magic silently REPAIRS the illegal spacing as it writes, while the DEF — the thing that
    describes the fabricated article — keeps it. Keep measurement and inference apart here.
    MEASURED: the two GDS files disagree, and KLayout's copy breaks the 0.17 um rule.
    INFERRED: Magic DRC cannot flag it, since that deck runs on Magic's own database and the
    merge happens before the check.
    NOT MEASURED either way, because XOR runs BEFORE Magic DRC and killed every coproc run at
    step 40 — no coproc run has ever reached the DRC step. XOR was the only gate that saw this.

    Returns (None, 0, 0, 0) when the via cannot be interrogated; the caller then REFUSES to bridge
    rather than placing geometry on a guessed rule.
    """
    try:
        bot = via.getBottomLayer().getConstName()
        top = via.getTopLayer().getConstName()
        boxes = via.getBoxes()
    except Exception:
        return (None, 0, 0, 0)
    for b in boxes:
        try:
            ly = b.getTechLayer()
        except Exception:
            continue
        if ly is None:
            continue
        name = ly.getConstName()
        if name in (bot, top):
            continue                      # the two enclosures; what is left is the cut
        try:
            sp = int(ly.getSpacing() or 0)
        except Exception:
            sp = 0
        if sp <= 0:
            return (None, 0, 0, 0)
        return (name, int(b.getDX()), int(b.getDY()), sp)
    return (None, 0, 0, 0)


def cuts_too_close(c0, c1, cw, ch, spacing):
    """Do two cut CENTRES violate the cut layer's SPACING rule?

    Two rectangles are legally separated once the gap along at least one axis reaches `spacing`;
    when both gaps fall short — overlap included, where a gap goes negative — the rule is broken."""
    gap_x = abs(c0[0] - c1[0]) - cw
    gap_y = abs(c0[1] - c1[1]) - ch
    return max(gap_x, gap_y) < spacing


def _wire_bbox(n):
    """A net's routed-wire bounding box as (x0, y0, x1, y1), or None when it has no wire."""
    w = n.getWire()
    if w is None:
        return None
    bb = w.getBBox()
    if bb is None:
        return None
    if isinstance(bb, tuple):
        if len(bb) == 2:
            if not bb[0]:
                return None
            r = bb[1]
            return (r.xMin(), r.yMin(), r.xMax(), r.yMax())
        return bb if len(bb) == 4 else None
    return (bb.xMin(), bb.yMin(), bb.xMax(), bb.yMax())


NET_BUCKET = 20000
_NET_INDEX = {}


def _buckets(r):
    return [(gx, gy) for gx in range(r[0] // NET_BUCKET, r[2] // NET_BUCKET + 1)
            for gy in range(r[1] // NET_BUCKET, r[3] // NET_BUCKET + 1)]


class NetIndex:
    """Every net's wire bounding box, and the power grid's special-wire shapes and cuts, bucketed
    on a NET_BUCKET grid — built once per block, so a region query touches the nets near it
    instead of all of them.

    MEASURED on wpe_user_project (RUN_2026.09.27_18.14.14): the first jog pass over 342 violating
    gates took 21 hours, because every candidate segment of every gate walked all ~310k nets
    three or four times (`collect_foreign` per layer, `collect_cuts`) to find the few hundred
    near it. Queries return nets and shapes in the block's own order, so every caller sees exactly
    the sequence the full walk produced: the second pass of that run (50 gates) took 81 s against
    1,264 s and wrote a byte-identical DEF. A wire that is re-encoded is re-indexed (`touch`) by the
    function that re-encodes it, so geometry placed earlier in a pass is foreign metal to every
    later query."""

    def __init__(self, block):
        self.nets = list(block.getNets())
        self.order, self.by_norm, self.box, self.cells, self._keys = {}, {}, {}, {}, {}
        for i, n in enumerate(self.nets):
            name = n.getConstName()
            self.order[name] = i
            self.by_norm.setdefault(norm(name), n)
            self._insert(name, _wire_bbox(n))
        # Special wires never change during a pass: one bucketed list per layer of
        # (sequence, net, rect), and one of (sequence, cut centre, cut layer) for their vias.
        self.swires, self.swire_cuts, seq = {}, {}, 0
        for n in self.nets:
            for sw in n.getSWires():
                for b in sw.getWires():
                    seq += 1
                    r = (b.xMin(), b.yMin(), b.xMax(), b.yMax())
                    if b.isVia():
                        try:
                            tv = b.getTechVia()
                        except Exception:
                            tv = None
                        cname = via_cut_rule(tv)[0] if tv is not None else None
                        c = ((r[0] + r[2]) // 2, (r[1] + r[3]) // 2)
                        for k in _buckets((c[0], c[1], c[0], c[1])):
                            self.swire_cuts.setdefault(k, []).append((seq, c, cname))
                        continue
                    try:
                        ly = b.getTechLayer()
                    except Exception:
                        continue
                    if ly is None:
                        continue
                    grid = self.swires.setdefault(ly.getConstName(), {})
                    for k in _buckets(r):
                        grid.setdefault(k, []).append((seq, n.getConstName(), r))

    def _insert(self, name, bb):
        self.box[name] = bb
        keys = _buckets(bb) if bb is not None else []
        for k in keys:
            self.cells.setdefault(k, set()).add(name)
        self._keys[name] = keys

    def touch(self, net):
        """Re-read one net's wire bounding box after its wire was re-encoded."""
        name = net.getConstName()
        for k in self._keys.get(name, ()):
            self.cells.get(k, set()).discard(name)
        self._insert(name, _wire_bbox(net))

    def near(self, probe):
        """The nets whose wire bounding box overlaps `probe`, in block order."""
        found = set()
        for k in _buckets(probe):
            found |= self.cells.get(k, set())
        hits = [nm for nm in found if self.box[nm] is not None and overlaps(self.box[nm], probe)]
        return [self.nets[self.order[nm]] for nm in sorted(hits, key=self.order.__getitem__)]

    def special_rects(self, layer_name, region, grow):
        """Special-wire rects on `layer_name`, each with its net, whose rect grown by `grow`
        overlaps `region` — in the order the nets and their wires were walked."""
        grid = self.swires.get(layer_name, {})
        probe = (region[0] - grow, region[1] - grow, region[2] + grow, region[3] + grow)
        seen = {}
        for k in _buckets(probe):
            for seq, net, r in grid.get(k, ()):
                if seq not in seen and overlaps(r, probe):
                    seen[seq] = (net, r)
        return [seen[q] for q in sorted(seen)]

    def special_cuts(self, region):
        """Special-wire via cuts whose centre lies inside `region`, as (centre, cut layer), in
        walk order."""
        x0, y0, x1, y1 = region
        seen = {}
        for k in _buckets(region):
            for seq, c, cname in self.swire_cuts.get(k, ()):
                if x0 <= c[0] <= x1 and y0 <= c[1] <= y1:
                    seen[seq] = (c, cname)
        return [seen[q] for q in sorted(seen)]


def block_key(block):
    """The identity of the C++ block behind a SWIG proxy. Every `getBlock()` returns a NEW proxy
    (their `id()`s differ), so a cache keyed on `id()` misses whenever a caller reaches the block
    another way; the wrapped pointer is the same for all of them."""
    try:
        return int(block.this)
    except (AttributeError, TypeError):
        return id(block)


def net_index(block):
    idx = _NET_INDEX.get(block_key(block))
    if idx is None:
        idx = _NET_INDEX[block_key(block)] = NetIndex(block)
    return idx


def touch_net(net):
    """Re-index `net` in its block's index, if one has been built."""
    idx = _NET_INDEX.get(block_key(net.getBlock()))
    if idx is not None:
        idx.touch(net)


def collect_cuts(block, region, cut_layer_name):
    """Centres of every existing via cut on `cut_layer_name` that falls inside `region`.

    Every net, INCLUDING the one being bridged and including the power grid's special wires.
    Read straight out of the DEF the signoff tools consumed, tmp/routing/28-eco_jog_3.def:

        spot0   NEW met1 ( 1088590 438770 ) ( 1121910 * ) M1M2_PR ( 1122410 * ) M1M2_PR ...
                this bridge's own vias are 500 apart and perfectly legal — but net _06073_
                carries a via at 1122170, which is 260 from each of them against a 320 rule.

    So the crowding via is on a FOREIGN net, and a guard that only looked at the net being
    bridged would have sailed straight past it. The PDN is swept for the same reason: a bridge
    landing beside a power via is the identical defect, and nothing else would catch it.

    A via whose cut layer cannot be identified is collected rather than skipped: over-avoiding
    costs at worst one bridge, under-avoiding ships illegal geometry."""
    x0, y0, x1, y1 = region
    def inside(p):
        return x0 <= p[0] <= x1 and y0 <= p[1] <= y1
    idx = net_index(block)
    cuts = []
    for n in idx.near(region):
        last = None
        for o in decode_wire(n.getWire()):
            if o[0] in ("POINT", "POINT_EXT"):
                last = (o[1], o[2])
            elif o[0] in ("TECH_VIA", "VIA") and last is not None and inside(last):
                cname = via_cut_rule(o[1])[0]
                if cname is None or cname == cut_layer_name:
                    cuts.append(last)
    # The power grid lives in SPECIAL wires, which `getWire()` never returns.
    for c, cname in idx.special_cuts(region):
        if cname is None or cname == cut_layer_name:
            cuts.append(c)
    return cuts


def bridge_leaves_slivers(seg_pts, p0, p1, min_w):
    """Would this bridge leave a piece of metal NARROWER THAN MINIMUM WIDTH behind?

    The bridge rewrites `A -> B` as `A -> p0 -> (via) -> p1 -> (via) -> B`, so it leaves two
    stubs on the original layer: A..p0 and p1..B. A stub shorter than the layer's minimum width
    is a SLIVER, and a sliver is exactly what Magic and KLayout resolve differently — Magic
    re-derives geometry when it streams GDS while KLayout streams the DEF, so a sub-minimum
    fragment appears in one and not the other and the run fails signoff XOR.

    MEASURED on coproc_core: the XOR items are 0.150 x 0.120 um and 0.240 x 0.150 um on met1.pin,
    and met1's minimum width is 0.140 um — every one of them is under it in a dimension. The
    count tracks bridge count exactly across two blocks and four runs (7 bridges -> XOR 0,
    15 -> 0, 55 -> 7, 159 -> 10), which is what a per-bridge probability of leaving a sliver
    looks like.
    """
    (ax, ay), (bx, by) = seg_pts[0], seg_pts[-1]
    for (qx, qy), (rx, ry) in (((ax, ay), p0), (p1, (bx, by))):
        stub = abs(qx - rx) + abs(qy - ry)     # axis-aligned, so Manhattan IS the length
        if 0 < stub < min_w:
            return True
    return False


# Farther than any sky130 via enclosure extends from its via centre (met4's via3/via4 enclosures
# are the widest, under 0.5 um), so a net whose centreline is outside a region by less than this can
# still put metal inside it.
ENCLOSURE_REACH = 1000


def placed_rect(r, w, h, orient, origin):
    """A master-coordinate rect as placed on an instance: oriented inside the master's w x h box
    (OpenDB orientation names — DEF N/S/FN/FS are R0/R180/MY/MX), then moved so that box's
    lower-left corner sits at `origin`, the placed bounding box's lower-left."""
    f = {
        "R0": lambda x, y: (x, y),
        "R180": lambda x, y: (w - x, h - y),
        "MY": lambda x, y: (w - x, y),
        "MX": lambda x, y: (x, h - y),
        "R90": lambda x, y: (h - y, x),
        "R270": lambda x, y: (y, w - x),
        "MXR90": lambda x, y: (y, x),
        "MYR90": lambda x, y: (h - y, w - x),
    }[orient]
    ax, ay = f(r[0], r[1])
    bx, by = f(r[2], r[3])
    return (origin[0] + min(ax, bx), origin[1] + min(ay, by), origin[0] + max(ax, bx), origin[1] + max(ay, by))


CELL_BUCKET = 10000
_CELL_SHAPES = {}


def cell_layer_shapes(block, layer_name):
    """The metal every placed cell carries on `layer_name` that no wire of the design accounts for:
    its master's OBSTRUCTIONS, and its signal pins' geometry tagged with the net each carries.
    Power and ground pins are left out — their rails are the special wires already collected.
    Built once per block and layer, bucketed on a CELL_BUCKET grid; it stores placed rects, never
    instance handles, so an ECO destroying a filler cannot leave it dangling."""
    key = (block_key(block), layer_name)
    grid = _CELL_SHAPES.get(key)
    if grid is not None:
        return grid
    grid, per_master = {}, {}
    on_layer = lambda b: b.getTechLayer() is not None and b.getTechLayer().getConstName() == layer_name
    box = lambda b: (b.xMin(), b.yMin(), b.xMax(), b.yMax())
    for inst in block.getInsts():
        m = inst.getMaster()
        shapes = per_master.get(m.getName())
        if shapes is None:
            obs = [box(b) for b in m.getObstructions() if on_layer(b)]
            pins = []
            for mt in m.getMTerms():
                if mt.getSigType() in ("POWER", "GROUND"):
                    continue
                rs = [box(b) for mp in mt.getMPins() for b in mp.getGeometry() if on_layer(b)]
                if rs:
                    pins.append((mt.getConstName(), rs))
            shapes = per_master[m.getName()] = (obs, pins, m.getWidth(), m.getHeight())
        obs, pins, w, h = shapes
        if not obs and not pins:
            continue
        bb = inst.getBBox()
        origin, orient = (bb.xMin(), bb.yMin()), inst.getOrient()
        placed = [(placed_rect(r, w, h, orient, origin), None) for r in obs]
        for mt_name, rs in pins:
            it = inst.findITerm(mt_name)
            net = it.getNet() if it is not None else None
            owner = net.getConstName() if net is not None else None
            placed += [(placed_rect(r, w, h, orient, origin), owner) for r in rs]
        for r, owner in placed:
            for gx in range(r[0] // CELL_BUCKET, r[2] // CELL_BUCKET + 1):
                for gy in range(r[1] // CELL_BUCKET, r[3] // CELL_BUCKET + 1):
                    grid.setdefault((gx, gy), []).append((r, owner))
    _CELL_SHAPES[key] = grid
    return grid


def cell_rects(block, region, layer_name, exclude_net):
    """Every placed cell's metal on `layer_name` overlapping `region` that does not belong to
    `exclude_net`: obstructions always, pins unless they carry that net."""
    grid = cell_layer_shapes(block, layer_name)
    out = set()
    for gx in range(region[0] // CELL_BUCKET, region[2] // CELL_BUCKET + 1):
        for gy in range(region[1] // CELL_BUCKET, region[3] // CELL_BUCKET + 1):
            for r, owner in grid.get((gx, gy), ()):
                if owner != exclude_net and overlaps(r, region):
                    out.add(r)
    return sorted(out)


def collect_foreign(block, region, layer_name, exclude_net, half, spacing=0):
    """All foreign segment rects on `layer_name` overlapping `region`, INFLATED BY THE SPACING RULE —
    built once, then a window scan can test candidate bridge windows against this local set cheaply
    (no re-iterating 36k nets per window).

    The inflation is the whole point and it was missing. `free_window` rejects a candidate window by
    OVERLAP, so without it a bridge may be placed flush against a foreign wire — touching but not
    intersecting — which passes every check here and then fails signoff as
    `Metal2 spacing < 0.14um (met2.2)`. MEASURED: the Caravel wrapper's 133 in-place bridges produced
    359 Magic DRC violations that way, while TritonRoute reported 0 (it never re-checks geometry the
    ECO inserts after routing). Growing each foreign rect by the spacing rule makes "no overlap" mean
    "legally separated".

    The growth is exact, and the callers test REAL extents (a bridge wire at its width, an enclosure
    as drawn). A foreign wire is known only by its centreline, so it grows by its half-width `half`
    plus the spacing rule, reaching the keep-out edge. Via enclosures, special wires and cell metal
    are real extents already and grow by the spacing rule alone — adding `half` to them too demanded
    210 nm of met1 clearance where sky130 asks 140, and refused legal geometry wholesale."""
    rects = []
    # Nets are pre-filtered by their wire's CENTRELINE bounding box, so the probe must be grown by
    # everything that reaches inward from a centreline: this function's own inflation plus the
    # widest via enclosure. MEASURED: on infinichip `net148` lies wholly above y=22950 while the
    # query region stopped at y=22810, so the whole net was skipped — and its L1M1 enclosure sat
    # 95 nm from a bridge via, which golden sky130A_mr reported as m1.2.
    g = half + spacing + ENCLOSURE_REACH
    probe = (region[0] - g, region[1] - g, region[2] + g, region[3] + g)
    idx = net_index(block)
    for n in idx.near(probe):
        if n.getConstName() == exclude_net:
            continue
        ops = decode_wire(n.getWire())
        for s in segments(ops):
            if s["layer"] != layer_name or len(s["pts"]) < 1:
                continue
            pts = s["pts"] if len(s["pts"]) >= 2 else s["pts"] * 2
            rects.append(seg_rect(pts, half + spacing))
        # Foreign VIA ENCLOSURES are metal on this layer too, and they are wider than the wire they
        # sit on. Without them a bridge can land its own enclosure a few tens of nm from another
        # net's: MEASURED on infinichip (RUN_2026.09.23_09.44.40), golden sky130A_mr m1.2 between a
        # bridge via's met1 enclosure (bottom edge y=191.120) and the L1M1 enclosure of `row[4]`'s
        # access via (top edge 191.025) — 95 nm, flagged by the acceptance deck, passed by Magic.
        last = None
        for o in ops:
            if o[0] in ("POINT", "POINT_EXT"):
                last = (o[1], o[2])
            elif o[0] in ("PATH", "JUNCTION", "SHORT", "VWIRE"):
                last = None
            elif o[0] == "TECH_VIA" and last is not None:
                for (x0, y0, x1, y1) in via_layer_rects(o[1], last, layer_name):
                    rects.append((x0 - spacing, y0 - spacing, x1 + spacing, y1 + spacing))

    # THE POWER GRID, which `getWire()` does not return. Its straps live in SPECIAL wires, and on
    # sky130 OpenLane puts them on met4/met5 — exactly where a met3 violation bridges UP to.
    #
    # MEASURED on coproc_core RUN_2026.09.22_06.25.14, the first run ever to get past XOR and
    # reach LVS: netgen found `Number of nets: 36223` against the source's `36224`, and the
    # missing net was not deleted but SHORTED — all six pins of `\R[1][47]`, a register output,
    # appeared on VGND in the layout:
    #     and2b_1/B, mux2_1/A1, and2_2/A, dfxtp_2/Q, diode_2/DIODE, mux4_1/A1  -> VGND
    # and the routing log shows why:
    #     [SAT-JOG] R[1][47] met3->met4 bridged @ (486570, 621860) (seg len 10350)
    #     [SAT-JOG] R[1][47] met3->met4 bridged @ (478940, 622540) (seg len 31280)
    # Two met4 bridges on a net, onto a layer whose foreign metal this function could not see.
    # A register tied to ground is not a spacing nit; it is a dead chip.
    for net, r in idx.special_rects(layer_name, region, spacing):
        if net != exclude_net:
            rects.append((r[0] - spacing, r[1] - spacing, r[2] + spacing, r[3] + spacing))

    # The metal inside placed cells, which no wire of the design describes: master obstructions
    # (sky130's flops run met1 across their width inside the cell) and signal pins on this layer.
    # Metal placed against wires alone can land inside the spacing rule of it — and between two
    # such obstruction wires, enclose a hole under the minimum enclosed area.
    g = spacing
    for c in cell_rects(block, (region[0] - g, region[1] - g, region[2] + g, region[3] + g), layer_name, exclude_net):
        rects.append((c[0] - g, c[1] - g, c[2] + g, c[3] + g))
    return rects


def free_window(seg_pts, foreign, w, half, gate_xy, cuts=(), cut_rule=None, reject=None):
    """Scan a met segment for a length-w window whose bridge rect hits no foreign rect. Scan starts from the
    GATE-SIDE end (endpoint nearest the violating gate) and moves away, so the first free window found sits
    close to the gate and isolates the MOST far metal from it at layer-L deposit."""
    (ax, ay), (bx, by) = seg_pts[0], seg_pts[-1]
    horiz = abs(bx - ax) >= abs(by - ay)
    axis = ay if horiz else ax
    da = abs((ax if horiz else ay) - gate_xy[0 if horiz else 1]) + abs(axis - gate_xy[1 if horiz else 0])
    db = abs((bx if horiz else by) - gate_xy[0 if horiz else 1]) + abs(axis - gate_xy[1 if horiz else 0])
    a_coord = ax if horiz else ay
    b_coord = bx if horiz else by
    gate_end = a_coord if da <= db else b_coord   # end nearest the gate
    far_end = b_coord if da <= db else a_coord
    step = 340 if far_end >= gate_end else -340
    # start one window-length in from the gate-side end and walk toward the far end
    pos = gate_end + step
    while (far_end - (pos + (w if step > 0 else -w))) * (1 if step > 0 else -1) >= step:
        s0 = min(pos, pos + (w if step > 0 else -w))
        s1 = s0 + w
        if horiz:
            wr = (s0 - half, axis - half, s1 + half, axis + half)
            p0, p1 = (s0, axis), (s1, axis)
        else:
            wr = (axis - half, s0 - half, axis + half, s1 + half)
            p0, p1 = (axis, s0), (axis, s1)
        ok = not any(overlaps(wr, fr) for fr in foreign)
        if ok and cut_rule is not None:
            # The window's two ends are where the vias land, so both must clear every cut
            # already on this layer. Keep SCANNING on a violation rather than giving up — the
            # next position along the segment is usually free.
            cw, ch, sp = cut_rule
            ok = not any(cuts_too_close(p0, c, cw, ch, sp) or cuts_too_close(p1, c, cw, ch, sp)
                         for c in cuts)
        if ok and reject is not None:
            ok = not reject(p0, p1)
        if ok:
            return p0, p1
        pos += step
    return None


def bridge_ops(ops, tech, seg_start, p0, p1, up_via):
    """Replace the segment starting at seg_start with A -> p0 (layer L), a via up, p0 -> p1 on L+1, a via
    back down, p1 -> B (layer L). A short L+1 bridge that isolates the far metal from the gate at L deposit.

    Junction references are RE-BASED. A JUNCTION/SHORT/VWIRE op branches from a point by its *index* in the
    wire's point stream (`dbWireDecoder` junction value). This bridge inserts TWO points (p0, p1) between the
    segment's A (original point index `a`) and B (index `a+1`), so every point at index >= a+1 shifts by +2.
    Without shifting the junction values to match, a fan-out branch off B (or any later point) would reconnect
    to an inserted via point and OPEN the net — the LVS net-count diff. This shift is a no-op for branch-free
    wires (no junctions), so it cannot regress nets that bridged cleanly before."""
    a = sum(1 for o in ops[:seg_start] if o[0] in ("POINT", "POINT_EXT"))
    out = []
    i, n = 0, len(ops)
    while i < n:
        o = ops[i]
        if o[0] == "PATH" and i == seg_start:
            layer = o[1]
            pts = []
            j = i + 1
            while j < n and ops[j][0] in ("POINT", "POINT_EXT"):
                pts.append((ops[j][1], ops[j][2]))
                j += 1
            A, B = pts[0], pts[-1]
            out.append(("PATH", layer))
            out.append(("POINT", A[0], A[1]))
            out.append(("POINT", p0[0], p0[1]))
            out.append(("TECH_VIA", up_via))
            out.append(("POINT", p1[0], p1[1]))
            out.append(("TECH_VIA", up_via))
            out.append(("POINT", B[0], B[1]))
            i = j
        elif o[0] in ("JUNCTION", "SHORT", "VWIRE") and o[1] >= a + 1:
            out.append((o[0], o[1] + 2, o[2]))
            i += 1
        else:
            out.append(o)
            i += 1
    return out


def foreign_metal_in(block, region, layer_name, exclude_net, half):
    """Any wire on `layer_name` from a net other than exclude_net whose segment rect overlaps `region`?
    Uses each net's wire bounding box as a cheap pre-filter before decoding."""
    hits = []
    for n in block.getNets():
        if n.getConstName() == exclude_net or n.getWire() is None:
            continue
        bb = n.getWire().getBBox()
        if bb is None:
            continue
        if isinstance(bb, tuple):
            if len(bb) == 2:            # (found, rect)
                if not bb[0]:
                    continue
                r = bb[1]
                nb = (r.xMin(), r.yMin(), r.xMax(), r.yMax())
            elif len(bb) == 4:          # plain coord tuple
                nb = bb
            else:
                continue
        else:
            nb = (bb.xMin(), bb.yMin(), bb.xMax(), bb.yMax())
        if not overlaps(nb, region):
            continue
        segs = segments(decode_wire(n.getWire()))
        for s in segs:
            if s["layer"] != layer_name or len(s["pts"]) < 1:
                continue
            r = seg_rect(s["pts"] if len(s["pts"]) >= 2 else s["pts"] * 2, half)
            if overlaps(r, region):
                hits.append(n.getConstName())
                break
    return hits


def gate_region(segs, gate_xy, layer):
    """The gate's connected layer-L antenna component: layer-L segments reachable from the segment nearest
    the gate by walking shared endpoints. Bridging the longest segment IN this component (not the globally
    longest, which may be an unrelated branch) is what actually reduces the gate's side-area."""
    lsegs = [s for s in segs if s["layer"] == layer and len(s["pts"]) >= 2]
    if not lsegs:
        return []
    pmap = {}
    for idx, s in enumerate(lsegs):
        for p in (s["pts"][0], s["pts"][-1]):
            pmap.setdefault(p, []).append(idx)

    def dist(p):
        return abs(p[0] - gate_xy[0]) + abs(p[1] - gate_xy[1])

    start = min(range(len(lsegs)), key=lambda i: min(dist(lsegs[i]["pts"][0]), dist(lsegs[i]["pts"][-1])))
    seen = {start}
    q = [start]
    while q:
        i = q.pop()
        for p in (lsegs[i]["pts"][0], lsegs[i]["pts"][-1]):
            for j in pmap.get(p, []):
                if j not in seen:
                    seen.add(j)
                    q.append(j)
    return [lsegs[i] for i in seen]


def audit_cut_spacing(block, placed):
    """Re-read the database after all bridges are placed and confirm not one cut this pass added
    sits closer to any other cut on its layer than the PDK rule allows.

    The window filter and the scan-time check should make this impossible. It exists anyway
    because the previous two geometry guards were BELIEVED to work and did not: the pin-overlap
    guard ran happily and rejected zero windows, and the enclosure-orientation fix changed no
    number at all. A guard nobody measures is indistinguishable from no guard. This one measures
    the finished geometry and is FATAL, so a defect cannot again reach GDS and surface three
    stages later as an unexplained XOR count.

    It also covers a gap XOR cannot. Magic only merges cuts — and so only makes the defect
    visible to XOR — when the via ENCLOSURES overlap. For sky130 via1 the enclosure (320) happens
    to equal the cut pitch (150 + 170), so every illegal pair does merge and XOR does catch it.
    That coincidence does not hold elsewhere: mcon is illegal below 360 but merges only below 290,
    via2 below 400 merging below 370, via3 below 400 merging below 380. Nothing in the present
    ladder lands in those blind bands, but nothing structural keeps a future rung out of them.

    One collect per cut layer over the union of the placed-cut neighbourhoods, then a grid bucket,
    so the cost is one pass rather than one per via."""
    if not placed:
        return []
    by_layer = {}
    for cut_name, cw, ch, sp, c in placed:
        pad = max(cw + sp, ch + sp)
        e = by_layer.setdefault(cut_name, {"pad": 0, "box": None, "pts": []})
        e["pad"] = max(e["pad"], pad)
        b = (c[0] - pad, c[1] - pad, c[0] + pad, c[1] + pad)
        e["box"] = b if e["box"] is None else (min(e["box"][0], b[0]), min(e["box"][1], b[1]),
                                               max(e["box"][2], b[2]), max(e["box"][3], b[3]))
    grids = {}
    for cut_name, e in by_layer.items():
        pad = max(e["pad"], 1)
        g = {}
        for c in collect_cuts(block, e["box"], cut_name):
            g.setdefault((c[0] // pad, c[1] // pad), []).append(c)
        grids[cut_name] = (g, pad)

    bad = []
    for cut_name, cw, ch, sp, c in placed:
        g, pad = grids.get(cut_name, ({}, 1))
        gx, gy = c[0] // pad, c[1] // pad
        skipped_self = False
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for other in g.get((gx + dx, gy + dy), ()):
                    if other == c and not skipped_self:
                        skipped_self = True      # this is the cut itself, not a neighbour
                        continue
                    if cuts_too_close(c, other, cw, ch, sp):
                        bad.append((cut_name, c, other))
    return bad


def bridge_all(block, tech, path, no_junction=False):
    """Bridge every violating gate listed in a "net<TAB>gate<TAB>layer" file: for each, promote a short
    free-window met bridge on the longest violating-layer segment, from the gate side. One re-encode per net.
    This greedy pass handles the single-gate majority; the SAT phase takes the shared / no-free-window residue."""
    def_cache = {}
    triples = []
    for line in open(path):
        parts = line.strip().split("\t")
        if len(parts) >= 3:
            triples.append((parts[0], parts[1], parts[2]))
    done = 0
    unbridged = []
    placed_cuts = []
    for nname, gname, layer in triples:
        tn = net_index(block).by_norm.get(norm(nname))
        if tn is None or tn.getWire() is None:
            print(f"[SAT-JOG] {nname}: net/wire missing")
            unbridged.append((nname, gname, "net/wire missing"))
            continue
        gxy = None
        for it in tn.getITerms():
            if norm(it.getInst().getConstName()) == norm(gname):
                ok, gx, gy = it.getAvgXY()
                gxy = (gx, gy)
        tops = decode_wire(tn.getWire())
        # A wire carrying a via-enclosure RECT patch is re-encoded WITH its patches, read from a DEF of
        # this same database state: the odb Python binding cannot read a patch (getRect takes C++
        # out-parameters), and re-encoding without them drops geometry — an XOR difference. Proven on
        # fabric_horner_seq: all 925 patch-carrying nets decoded, re-encoded and re-written gave a
        # byte-identical NETS section. A wire whose patches cannot all be matched is still refused.
        if any(o[0] == "RECT" for o in tops):
            if def_cache.get("text") is None:
                import tempfile
                with tempfile.NamedTemporaryFile(suffix=".def", delete=False) as tf:
                    odb.write_def(block, tf.name)
                    def_cache["text"] = open(tf.name).read()
                os.unlink(tf.name)
            tops = attach_rects(tops, def_route_rects(def_cache["text"], tn.getConstName()))
            if tops is None:
                print(f"[SAT-JOG] {nname}: via-enclosure RECT patch not matched in the DEF — skip jog (XOR-safe)")
                unbridged.append((nname, gname, "unmatched via-enclosure RECT patch"))
                continue
        # LOGOS_JOG_NO_JUNCTION: refuse to bridge a wire that CARRIES BRANCHES at all.
        #
        # `bridge_ops` re-bases JUNCTION/SHORT/VWIRE indices by +2 because it inserts two points,
        # and that arithmetic is believed correct — but "believed" is the problem. A single
        # mis-rebased branch reconnects to an inserted via point and OPENS the net, which shows up
        # as exactly `net count difference = 1, unmatched nets = 1` at LVS and fails the flow after
        # a full route. Measured on coproc_core with SYNTH_SIZING=1: 16 bridges placed, slew
        # 282 -> 126, antenna 17 -> 2, and ONE opened net threw the run away. The same single-net
        # signature appeared on systolic_pipe16_realbist.
        #
        # The docstring's own guarantee is that the shift "is a no-op for branch-free wires", so
        # restricting to branch-free wires is the one case that needs no argument. Default OFF so
        # every already-recorded block keeps the behaviour it was signed off with.
        if no_junction and any(
            o[0] in ("JUNCTION", "SHORT", "VWIRE") for o in tops
        ):
            print(f"[SAT-JOG] {nname}: wire carries branches — skipped (--no-junction), "
                  f"bridging it risks re-basing a branch onto a via point and OPENING the net")
            unbridged.append((nname, gname, "branch-bearing wire (--no-junction)"))
            continue
        allsegs = segments(tops)
        region = gate_region(allsegs, gxy, layer) if gxy else []
        # ONLY simple 2-point straight segments: bridge_ops keeps just the endpoints, so a segment with a
        # middle point that another wire T-connects to (a junction reference) would lose that point and OPEN
        # the branch (the lone _28062_ LVS net-diff). 2-point segments have no middle point to drop.
        #
        # ORDER: nearest the gate FIRST, longest as the tie-break. A bridge isolates everything BEYOND it
        # from the gate, so cutting the segment the gate actually hangs off removes the most side-area.
        # Sorting by length instead (what this did) can bridge a long branch three hops away and leave the
        # whole path between it and the gate still charging — a bridge that is placed, reported, and useless.
        #
        # There is deliberately NO global fallback here. Segments outside `gate_region` are, by the
        # definition of that component walk, NOT connected to this gate on this layer, so bridging one
        # CANNOT reduce this gate's partial ratio. The old code appended them as a fallback and counted the
        # result as success, which is how gemm_mac_realbist logged "bridged 1 gates" for thirty consecutive
        # ECO iterations while its ratio sat unmoved at 1.41.
        def gate_dist(s):
            if not gxy:
                return 0
            return min(abs(p[0] - gxy[0]) + abs(p[1] - gxy[1]) for p in (s["pts"][0], s["pts"][-1]))

        rseg = sorted(
            # A segment carrying a RECT patch is never bridged: the patch is anchored to its point,
            # and bridge_ops rewrites the segment's point run (it would split the patch from it).
            [s for s in region if not s["via"] and not s.get("rect") and len(s["pts"]) == 2],
            key=lambda s: (gate_dist(s), -seg_len(s)),
        )
        tsegs = rseg
        if not tsegs:
            print(f"[SAT-JOG] {nname} {layer}: gate {gname} has no 2-point segment in its own "
                  f"component ({len(region)} segment(s) in region, {len(allsegs)} on the net) — "
                  f"nothing here can reduce THIS gate's ratio; needs a pre-route diode or re-route")
            unbridged.append((nname, gname, "no-segment-in-component"))
            continue
        placed = False
        if layer not in ("met1", "met2", "met3"):
            # met1/met2/met3 up-bridges (via1/via2/via3) are XOR-clean and isolate the far metal. met4 would
            # bridge up via4 (M4M5) into the 1.6um-wide met5 — that geometry XOR-mismatches Magic vs KLayout
            # (verified: all XOR diffs sat on the lone met5->met4 down-bridge). met5 is the top layer (a
            # down-bridge can't isolate). Both are skipped; those detours need a re-route/better base route.
            print(f"[SAT-JOG] {nname} {layer}: skipped (met4/met5 — via4 XOR / top-layer, needs re-route)")
            unbridged.append((nname, gname, f"{layer} upper-layer detour (needs re-route)"))
            continue
        blayer, _via_probe = bridge_layer_via(tech, layer)
        # met4 is a wide layer and via3's met4 enclosure extends past the window ends, so a 100-unit clearance
        # let _28062_'s met4 bridge touch neighbour _28047_ (a net MERGE = the 1-net LVS diff). Give met4
        # bridges a wide clearance so the whole jog footprint (via enclosure included) stays off foreign metal.
        mar = 300 if blayer == "met4" else 100
        for pick in tsegs:
            # The via orientation follows THIS segment's direction, not the net's or the layer's.
            (ax, ay), (bx, by) = pick["pts"][0], pick["pts"][-1]
            horizontal = abs(bx - ax) >= abs(by - ay)
            blayer, via = bridge_layer_via(tech, layer, horizontal)
            if blayer is None or via is None:
                break
            cut_name, cut_w, cut_h, cut_sp = via_cut_rule(via)
            if cut_name is None:
                print(f"[SAT-JOG] {nname} {layer}: cannot read {blayer} cut spacing from the PDK "
                      f"— refusing to bridge on a guessed rule")
                continue
            # The window width IS the via-to-via centre distance, so it has to clear the cut rule
            # along the segment's own direction.
            cut_pitch = (cut_w if horizontal else cut_h) + cut_sp
            region = seg_rect(pick["pts"], mar + 100)
            foreign = collect_foreign(block, region, blayer, tn.getConstName(), mar,
                                      layer_spacing(tech, blayer))
            # Reach one full pitch further than the routing region: a cut just outside it can
            # still be too close to a via at the window's end.
            region_cuts = collect_cuts(block, seg_rect(pick["pts"], mar + 100 + cut_pitch),
                                       cut_name)
            # Try progressively SHORTER bridges before giving up. A bridge only has to lift a
            # length of metal off the violating layer; 500 nm is a comfortable default, not a
            # requirement, and a shorter one needs a smaller free window. This matters more since
            # `collect_foreign` began inflating foreign rects by the DRC spacing (which it must, or
            # bridges land flush against neighbours and fail signoff): stricter clearance means
            # fewer 500 nm windows exist. Without this fallback the ECO simply plateaus — measured
            # on gemm_mac_realbist, 1 violator surviving 29 identical iterations, and on
            # fabric_horner_seq and gpu_simt before their other levers cleared them.
            # A bridge REPLACES part of the segment with `A -> p0 -> via -> p1 -> via -> B`. If the
            # window is comparable to the segment's whole length there is no metal left either
            # side of it, the two via stacks land on top of each other, and the net comes APART.
            #
            # Measured, and this is the failure that cost the campaign a day: on coproc_core the
            # router bridged `_30465_` twice — once on a 16,330-unit segment (fine) and once on a
            # 340-unit one, with windows of 500/340/240/180 nm. LVS then reported
            # `(no matching net) | Net: _30465_`: 36,259 nets in the layout against 36,260 in the
            # source. The net the router bridged is exactly the net that vanished.
            #
            # So the window must fit INSIDE the segment with room for both via enclosures and the
            # clearance margin at each end. `w + 4*mar` is that, with the margin counted twice per
            # side: anything tighter is the degenerate case above.
            win = None
            seglen = seg_len(pick)
            min_w = layer_min_width(tech, layer)
            widths = [_w for _w in (500, 340, 240, 180) if _w >= cut_pitch]
            if not widths:
                print(f"[SAT-JOG] {nname} {layer}: no window width clears the {cut_name} cut "
                      f"pitch ({cut_pitch} DBU) — cannot bridge legally")
                continue
            # ...and no rung shorter than the enclosures allow: two vias one window apart must clear
            # the spacing rule on both layers, or the bridge notches itself.
            window_floor = min_bridge_window(
                {l: via_extent_along(via, l, horizontal) for l in (layer, blayer)},
                {l: layer_spacing(tech, l) for l in (layer, blayer)})
            widths = bridge_widths(window_floor, widths)
            # Same-net geometry on both layers the bridge touches: its new via enclosures (and the
            # bridge wire) must merge with this net's metal or clear it by the spacing rule.
            own = {l: net_layer_rects(tops, tech, l) for l in (layer, blayer)}
            bhalf = layer_min_width(tech, blayer) // 2
            # The bridge's vias also put NEW metal on the violating layer — their enclosures at p0
            # and p1. The bridge-layer scan above never looked at that layer, so check those
            # enclosures against every foreign shape there (wires at their true half-width, via
            # enclosures, the power grid), each grown by the spacing rule.
            vfor = collect_foreign(block, region, layer, tn.getConstName(),
                                   layer_min_width(tech, layer) // 2, layer_spacing(tech, layer))

            # The bridge layer is checked by its REAL new shapes too: the window rectangle the scan
            # tests is +/-mar round the centreline, narrower than a via enclosure (M2M3's met3 is
            # +/-165), and foreign wires there were grown by mar rather than their own half-width.
            # MEASURED: coproc_core R[1][20] (RUN_2026.09.24_00.26.17), an M2M3 enclosure 195 nm from
            # a foreign 300 nm met3 wire against a 300 nm rule — golden m3.2 x2.
            bfor = collect_foreign(block, region, blayer, tn.getConstName(), bhalf, layer_spacing(tech, blayer))
            half_l = layer_min_width(tech, layer) // 2
            A, B = pick["pts"][0], pick["pts"][-1]
            own_rest = {l: [r for r in own[l] if r != seg_rect([A, B], half_l)] if l == layer else own[l]
                        for l in (layer, blayer)}

            def kept(l, p0, p1):
                """The net's own metal on `l` once the bridge is in: on the lifted layer the segment
                gives way to its two stubs, A..p0 and p1..B."""
                if l != layer:
                    return own[l]
                return own_rest[l] + [seg_rect([e, p], half_l) for e, p in ((A, p0), (p1, B)) if e != p]

            def notch(p0, p1):
                for l in (layer, blayer):
                    encl = via_layer_rects(via, p0, l) + via_layer_rects(via, p1, l)
                    if self_notch(encl, layer_spacing(tech, l)):
                        return True
                    new = encl + ([seg_rect([p0, p1], bhalf)] if l == blayer else [])
                    if makes_notch(new, own[l], layer_spacing(tech, l)):
                        return True
                    if makes_neck(new, kept(l, p0, p1), layer_min_width(tech, l)):
                        return True
                enc = via_layer_rects(via, p0, layer) + via_layer_rects(via, p1, layer)
                if any(overlaps(e, f) for e in enc for f in vfor):
                    return True
                bnew = via_layer_rects(via, p0, blayer) + via_layer_rects(via, p1, blayer) + [seg_rect([p0, p1], bhalf)]
                if any(overlaps(e, f) for e in bnew for f in bfor):
                    return True
                # MINIMUM AREA of every piece the bridge leaves: each stub on the lifted layer with its
                # new enclosure and whatever own metal it still touches, and the bridge itself.
                # MEASURED: fabric_horner_seq net580 (RUN_2026.09.24_00.25.18) — a 0.229 um^2 met3
                # island between the bridge via and the net's own M2M3 via (golden m3.6, min 0.24).
                amin_l, amin_b = min_area_dbu2(tech, layer), min_area_dbu2(tech, blayer)
                for end, p in ((A, p0), (B, p1)):
                    piece = via_layer_rects(via, p, layer) + ([seg_rect([end, p], half_l)] if end != p else [])
                    if connected_area(piece, own_rest[layer]) < amin_l:
                        return True
                return connected_area(bnew, own_rest[blayer]) < amin_b

            scan = functools.partial(free_window, reject=notch)
            for _w in widths:
                if seglen < _w + 4 * mar:
                    continue
                cand = scan(pick["pts"], foreign, _w, mar, gxy or pick["pts"][-1],
                            region_cuts, (cut_w, cut_h, cut_sp))
                if cand is None:
                    continue
                # SELF-CHECK: never hand on geometry the signoff tools will disagree about.
                if bridge_leaves_slivers(pick["pts"], cand[0], cand[1], min_w):
                    print(f"[SAT-JOG] {nname} {layer}: window {_w} would leave a sub-min-width "
                          f"stub (< {min_w} DBU) — skipped, it would fail signoff XOR")
                    continue
                win = cand
                break
            if win is None and seglen < widths[-1] + 4 * mar:
                print(f"[SAT-JOG] {nname} {layer}: segment too short to bridge safely "
                      f"(len {seglen} < {widths[-1] + 4 * mar}) — a bridge here would consume the "
                      f"whole segment and OPEN the net")
                continue
            if win is None:
                print(f"[SAT-JOG] {nname} {layer}: no free {blayer} window at any legal width "
                      f"({'/'.join(str(x) for x in widths)} nm, {len(foreign)} foreign rects, "
                      f"{len(region_cuts)} {cut_name} cuts) — cannot bridge")
                continue
            encode_wire(tn, bridge_ops(tops, tech, pick["start"], win[0], win[1], via))
            placed_cuts.append((cut_name, cut_w, cut_h, cut_sp, win[0]))
            placed_cuts.append((cut_name, cut_w, cut_h, cut_sp, win[1]))
            print(f"[SAT-JOG] {nname} {layer}->{blayer} bridged @ {win[0]} (seg len {seg_len(pick)})")
            done += 1
            placed = True
            break
        if not placed:
            print(f"[SAT-JOG] {nname} {layer}: NO free window on any segment")
            unbridged.append((nname, gname, "no free window on any segment"))
    violations = audit_cut_spacing(block, placed_cuts)
    if violations:
        print(f"[SAT-JOG] FATAL: {len(violations)} cut-spacing violations in the geometry this "
              f"pass placed — refusing to hand defective geometry to signoff")
        for cut_name, c, other in violations[:20]:
            d = max(abs(c[0] - other[0]), abs(c[1] - other[1]))
            print(f"[SAT-JOG]   {cut_name} cut at {c} is {d} DBU from {other}")
        raise SystemExit(1)
    print(f"[SAT-JOG] cut-spacing audit clean: {len(placed_cuts)} cuts placed, 0 violations")
    print(f"[SAT-JOG] bridged {done}/{len(triples)} violating gates")
    for nname, gname, why in unbridged:
        print(f"[SAT-JOG]   UNBRIDGED {nname} ({gname}): {why}")
    # The ECO loop reads this. Without it the loop could only see how many violators it ASKED about, so it
    # printed that number as though it were work done and then re-ran an identical no-op until it hit the
    # iteration cap. A count of 0 here means this iteration changed NOTHING and re-running cannot help.
    try:
        with open(path + ".done", "w") as fh:
            fh.write(str(done))
    except OSError as e:
        print(f"[SAT-JOG] could not write progress file: {e}")


@click.command()
@click.option("--net", required=True)
@click.option("--gate", default=None)
@click.option("--mode", default="roundtrip")
# An EXPLICIT flag, not an environment variable. `set ::env(X)` in the flow config did
# reach config.tcl and did NOT reach this process: the run with it set produced
# byte-identical bridge counts (15 of 17, then 1 of 3) to the run without it. A lever
# that silently does nothing is the most expensive kind — this one costs a 40-minute
# route per non-observation. On the command line it is visible in the log.
@click.option("--no-junction", is_flag=True, default=False,
              help="Bridge only BRANCH-FREE wires. `bridge_ops` re-bases junction "
                   "indices by +2 and that arithmetic is believed correct but not "
                   "proven; one mis-rebased branch reconnects to an inserted via "
                   "point and OPENS the net (LVS: net count difference = 1).")
@click_odb
def cli(reader, net, gate, mode, no_junction):
    b = reader.block
    tech = b.getDataBase().getTech()
    if mode == "bridge_all":
        bridge_all(b, tech, net, no_junction=no_junction)
        return
    target = None
    for n in b.getNets():
        if norm(n.getConstName()) == norm(net):
            target = n
            break
    if target is None:
        raise SystemExit(f"[SAT-JOG] net {net} not found")
    gate_xy = None
    if gate:
        for it in target.getITerms():
            if norm(it.getInst().getConstName()) == norm(gate):
                ok, gx, gy = it.getAvgXY()
                gate_xy = (gx, gy)
    ops = decode_wire(target.getWire())
    if mode == "roundtrip":
        kinds = {}
        for o in ops:
            kinds[o[0]] = kinds.get(o[0], 0) + 1
        print(f"[SAT-JOG] {target.getConstName()}: {len(ops)} ops {kinds}")
        n_pts = encode_wire(target, ops)
        print(f"[SAT-JOG] re-encoded {n_pts} points; wire now: {target.getWire() is not None}")
    elif mode == "promote_longest":
        segs = segments(ops)
        cand = [s for s in segs if s["layer"] and s["layer"].startswith("met") and not s["via"] and len(s["pts"]) >= 2]
        cand.sort(key=seg_len, reverse=True)
        if not cand:
            print("[SAT-JOG] no promotable segment"); return
        pick = cand[0]
        print(f"[SAT-JOG] promoting {pick['layer']}->{LAYER_UP.get(pick['layer'])} len={seg_len(pick)} pts={pick['pts']}")
        new_ops = promote_ops(ops, tech, {pick["start"]})
        encode_wire(target, new_ops)
        print("[SAT-JOG] promoted + re-encoded")
    elif mode == "bridge":
        segs = segments(ops)
        cand = [s for s in segs if s["layer"] and s["layer"].startswith("met") and not s["via"] and len(s["pts"]) >= 2]
        cand.sort(key=seg_len, reverse=True)
        for pick in cand:
            up = LAYER_UP.get(pick["layer"])
            via = tech.findVia(VIA_UP.get(pick["layer"], ""))
            if up is None or via is None:
                continue
            cut_name, cut_w, cut_h, cut_sp = via_cut_rule(via)
            if cut_name is None:
                print(f"[SAT-JOG] {pick['layer']}: cannot read {up} cut spacing from the PDK "
                      f"— refusing to bridge on a guessed rule")
                continue
            (sax, say), (sbx, sby) = pick["pts"][0], pick["pts"][-1]
            cut_pitch = ((cut_w if abs(sbx - sax) >= abs(sby - say) else cut_h) + cut_sp)
            if 500 < cut_pitch:
                print(f"[SAT-JOG] {pick['layer']}: 500 DBU window is under the {cut_name} cut "
                      f"pitch ({cut_pitch} DBU) — cannot bridge legally")
                continue
            region = seg_rect(pick["pts"], 200)
            foreign = collect_foreign(b, region, up, target.getConstName(), 100,
                                      layer_spacing(tech, up))
            win = free_window(pick["pts"], foreign, 500, 100, gate_xy or pick["pts"][-1],
                              collect_cuts(b, seg_rect(pick["pts"], 200 + cut_pitch), cut_name),
                              (cut_w, cut_h, cut_sp))
            if win is None:
                print(f"[SAT-JOG] {pick['layer']} len={seg_len(pick)}: NO free {up} window ({len(foreign)} foreign)")
                continue
            print(f"[SAT-JOG] bridging {pick['layer']}->{up} len={seg_len(pick)} window={win} (foreign_local={len(foreign)})")
            new_ops = bridge_ops(ops, tech, pick["start"], win[0], win[1], via)
            encode_wire(target, new_ops)
            print("[SAT-JOG] bridged + re-encoded")
            break
    elif mode == "check_promote":
        # For the longest promotable met segment, is the layer+1 track it would occupy free of foreign metal?
        segs = segments(ops)
        cand = [s for s in segs if s["layer"] and s["layer"].startswith("met") and not s["via"] and len(s["pts"]) >= 2]
        cand.sort(key=seg_len, reverse=True)
        for pick in cand[:5]:
            up = LAYER_UP.get(pick["layer"])
            region = seg_rect(pick["pts"], 100)
            hits = foreign_metal_in(b, region, up, target.getConstName(), 100)
            print(f"[SAT-JOG] {pick['layer']}->{up} len={seg_len(pick)} region={region} foreign_{up}={hits[:6]} ({len(hits)})")


if __name__ == "__main__":
    cli()
