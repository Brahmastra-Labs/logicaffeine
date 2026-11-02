import importlib.util, os, sys, types

ODBPY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
for m in ("odb", "click", "reader"):
    sys.modules.setdefault(m, types.ModuleType(m))
sys.modules["click"].command = lambda *a, **k: (lambda f: f)
sys.modules["click"].option = lambda *a, **k: (lambda f: f)
sys.modules["reader"].click_odb = lambda f: f
sys.path.insert(0, ODBPY)
spec = importlib.util.spec_from_file_location("eco", os.path.join(ODBPY, "magic_antenna_eco.py"))
eco = importlib.util.module_from_spec(spec); spec.loader.exec_module(eco)

class _L:
    def __init__(s, n): s.n = n
    def getConstName(s): return s.n
    def getWidth(s): return 140
    def getSpacing(s): return 140
class _B:
    def __init__(s, l, dx, dy): s.l, s.dx, s.dy = l, dx, dy
    def getTechLayer(s): return s.l
    def getDX(s): return s.dx
    def getDY(s): return s.dy
class _V:
    def getBoxes(s): return [_B(_L("mcon"), 170, 170), _B(_L("li1"), 170, 170), _B(_L("met1"), 290, 230)]
class _Tech:
    def findLayer(s, n): return _L(n)
    def findVia(s, n): return _V()

bad = 0
def check(name, got, want):
    global bad
    ok = got == want
    bad += not ok
    print(f"  {'ok ' if ok else 'FAIL'} {name:58} -> {got} (want {want})")

T = _Tech()
# MEASURED: coproc_core _58895_/A2_N — diode pin (367540,123760), access via (370070,124610).
# The horizontal-first L at y=123760 brushed a foreign met1 wire whose spacing-inflated box was
# (368090,123790)-(369915,124070). The search must fall to the vertical-first L instead.
blocker = (368090, 123790, 369915, 124070)
check("measured blocker forces the vertical-first orientation",
      eco.plan_or_none(T, [blocker], (367540, 123760), (370070, 124610)),
      [(367540, 123760), (367540, 124610), (370070, 124610)])
check("no foreign metal -> horizontal-first is taken",
      eco.plan_or_none(T, [], (367540, 123760), (370070, 124610)),
      [(367540, 123760), (370070, 123760), (370070, 124610)])
check("both orientations blocked -> refuse, never force a short",
      eco.plan_or_none(T, [(360000, 120000, 380000, 130000)], (367540, 123760), (370070, 124610)),
      None)
check("collinear points are collapsed (same row -> one segment)",
      eco.plan_or_none(T, [], (367540, 124610), (370070, 124610)),
      [(367540, 124610), (370070, 124610)])
check("off-grid diode centre is snapped to the 5 DBU grid",
      eco.plan_or_none(T, [], (367542, 123761), (370070, 124610))[0],
      (367540, 123760))
check("path rect is the wire grown by half-width",
      eco.path_rects([(0, 0), (1000, 0)], 70), [(-70, -70, 1070, 70)])

# MEASURED: systolic_pipe16_realbist _174816_/A on net _050941_ (RUN_2026.09.23_03.24.05,
# 30-eco_jog_4.odb). met1 Partial 430.11 vs Required 400.00 — zero diffusion on met1 although a
# heuristic diode sits on the net, because that diode's met1 (y=382670) joins the gate only
# through met2. The gate's own met1 island is its access stub plus a 75 um run west at y=380290.
# No site near the ACCESS VIA admitted a clear tie (601 tried); anywhere on the RUN credits it.
class _VL:
    def __init__(s, b, t): s.b, s.t = _L(b), _L(t)
    def getName(s): return s.b.n.upper() + s.t.n.upper() + "_PR"
    def getBottomLayer(s): return s.b
    def getTopLayer(s): return s.t
L1M1, M1M2 = _VL("li1", "met1"), _VL("met1", "met2")
ops = [("PATH", _L("li1")), ("POINT", 999810, 379610), ("TECH_VIA", L1M1),
       ("POINT", 1000730, 379610), ("TECH_VIA", M1M2), ("POINT", 1000730, 382670),
       ("TECH_VIA", M1M2), ("POINT", 1001650, 382670), ("POINT", 1003490, 382670),
       ("POINT", 1003490, 383010), ("POINT", 1043050, 383010),
       ("JUNCTION", 0, _L("met1")), ("POINT", 999810, 379610), ("POINT", 999810, 380290),
       ("POINT", 924830, 380290)]
segs = eco.wire_segments(ops)
check("layer follows each via (met1 -> met2 -> met1)",
      [l for l, _a, _b in segs],
      ["met1", "met2"] + ["met1"] * 6)
island = eco.met1_island(segs, (999810, 379610))
check("the gate's met1 island holds the stub and the 75 um run",
      sorted(island),
      sorted([((999810, 379610), (1000730, 379610)), ((999810, 379610), (999810, 380290)),
              ((999810, 380290), (924830, 380290))]))
check("met1 reached only THROUGH met2 is a different island (the uncredited diode's)",
      any(a[1] == 382670 or b[1] == 382670 for a, b in island), False)
check("nearest island point to a site under the run is straight above it",
      eco.nearest_on_island(island, (960002, 378000)), (960000, 380290))
check("nearest island point past the run's end is the end itself",
      eco.nearest_on_island(island, (900000, 381000)), (924830, 380290))
check("a site far from the access via still plans a clear tie to the run",
      eco.plan_or_none(T, [blocker], (960000, 378000), eco.nearest_on_island(island, (960000, 378000))),
      [(960000, 378000), (960000, 380290)])

# LAYER-AWARE ISLAND. A met3 violation is evaluated on everything joined to the gate BELOW met3, so
# a diode tied to ANY met1 point of that component is credited — including another met1 branch
# reached through met2. MEASURED motivation: fabric_horner_seq _25250_/C, _33411_/C, _33412_/A2
# (RUN_2026.09.23_10.20.59) violated on met3 and no site within reach of the gate's own tiny met1
# island admitted a clear tie (517-588 tried, twice). A met1 violation must still use only the met1
# island, or the diode would not be credited at met1.
M2M3 = _VL("met2", "met3")
ops3 = [("PATH", _L("li1")), ("POINT", 1000, 1000), ("TECH_VIA", L1M1),
        ("POINT", 1400, 1000),                                   # gate's met1 stub
        ("TECH_VIA", M1M2), ("POINT", 1400, 9000),               # up to met2, run north
        ("TECH_VIA", M1M2), ("POINT", 1400, 9000), ("POINT", 30000, 9000),   # down to a far met1 branch
        ("JUNCTION", 0, _L("met2")), ("POINT", 1400, 9000), ("TECH_VIA", M2M3),
        ("POINT", 1400, 40000)]                                  # met3 riser (the charged metal)
segs3 = eco.wire_segments(ops3)
vias3 = eco.wire_vias(ops3)
check("vias are reported with the layers they join",
      sorted((p, b, t) for p, b, t in vias3),
      sorted([((1000, 1000), "li1", "met1"), ((1400, 1000), "met1", "met2"),
              ((1400, 9000), "met1", "met2"), ((1400, 9000), "met2", "met3")]))
m1only = eco.island_below(segs3, vias3, (1000, 1000), "met1")
check("a met1 violation keeps to the met1 island", sorted(m1only), [((1000, 1000), (1400, 1000))])
m3 = eco.island_below(segs3, vias3, (1000, 1000), "met3")
check("a met3 violation reaches the far met1 branch through met2",
      sorted(m3), sorted([((1000, 1000), (1400, 1000)), ((1400, 9000), (30000, 9000))]))
check("the tie target for a site under the far branch is on that branch",
      eco.nearest_on_island(m3, (20002, 8000)), (20000, 9000))

# TIES ONTO MET2. Where the component below the violating layer has little or no met1 — MEASURED:
# fabric_horner_seq _33412_/A2 leaves its pin through a stacked via and has NO met1 at all, while its
# below-met3 component carries 144 um of met2 — the tie must be able to land on met2 (met1 from the
# diode, then an M1M2 via onto the net's met2). A met2 landing is connected from met2's deposit up, so
# it is credited for violations on met2 and above; a met1 violation must still land on met1.
tt1 = eco.tie_targets(segs3, vias3, (1000, 1000), "met1")
check("a met1 violation offers only met1 targets", sorted({l for l, _a, _b in tt1}), ["met1"])
tt3 = eco.tie_targets(segs3, vias3, (1000, 1000), "met3")
check("a met3 violation also offers the met2 run", ("met2", (1400, 1000), (1400, 9000)) in tt3, True)
check("nearest target to a site beside the met2 run lands ON met2",
      eco.nearest_target(tt3, (1900, 5002)), ("met2", (1400, 5000)))
check("nearest target to a site under the far met1 branch stays on met1",
      eco.nearest_target(tt3, (20002, 8000)), ("met1", (20000, 9000)))

# STACKED ACCESS. fabric_horner_seq _33412_/A2 leaves its pin through L1M1 and M1M2 stacked at ONE
# point: no met1 segment contains the access point at all, so the component was empty and no tie was
# ever tried. The seed must climb the via stack at the access point.
ops_st = [("PATH", _L("li1")), ("POINT", 5000, 5000), ("TECH_VIA", L1M1), ("TECH_VIA", M1M2),
          ("POINT", 5000, 20000), ("TECH_VIA", M2M3), ("POINT", 60000, 20000)]
seg_st, via_st = eco.wire_segments(ops_st), eco.wire_vias(ops_st)
check("a stacked access reaches its met2 run for a met3 violation",
      eco.tie_targets(seg_st, via_st, (5000, 5000), "met3"), [("met2", (5000, 5000), (5000, 20000))])
check("a stacked access offers nothing on met1 for a met1 violation (there is no met1 island)",
      eco.tie_targets(seg_st, via_st, (5000, 5000), "met1"), [])

# DROP-IN TIE. MEASURED: coproc_core _36364_/A_N (net _31234_, RUN_2026.09.24_02.17.38) — every one
# of 1,182 sites along an 83 um met1 island was refused because an L-shaped met1 tie from a diode a
# row away had to cross horizontal tracks full of foreign met1. A diode_2 whose tall li1 pin sits
# directly UNDER the island needs no met1 path at all: one L1M1 via from its own pin into the wire.
pin = (1000, 100, 1500, 2600)                     # the diode pin's li1 bbox
run = [((0, 1800), (5000, 1800))]                 # island met1 crossing over it at y=1800
check("a pin under the island drops one via straight up into it",
      eco.plan_drop(T, [], pin, run), [(1250, 1800)])
check("the via's li1 enclosure must stay inside the pin",
      eco.plan_drop(T, [], (1000, 100, 1150, 2600), run), None)
check("an island that misses the pin gives no drop", eco.plan_drop(T, [], pin, [((0, 3000), (5000, 3000))]), None)
check("a drop whose met1 enclosure hits foreign met1 is refused",
      eco.plan_drop(T, [(1200, 1750, 1300, 1850)], pin, run), None)

print("PASS: met1 tie geometry correct" if bad == 0 else f"FAIL: {bad} case(s)")
sys.exit(1 if bad else 0)
