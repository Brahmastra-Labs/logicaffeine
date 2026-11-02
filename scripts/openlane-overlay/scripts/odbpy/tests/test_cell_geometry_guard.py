import importlib.util, os, sys, types

ODBPY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
# Stub the odb/click/reader imports so the geometry can be unit-tested without OpenROAD.
for m in ("odb", "click", "reader"):
    sys.modules.setdefault(m, types.ModuleType(m))
sys.modules["click"].command = lambda *a, **k: (lambda f: f)
sys.modules["click"].option = lambda *a, **k: (lambda f: f)
sys.modules["reader"].click_odb = lambda f: f
sys.path.insert(0, ODBPY)
spec = importlib.util.spec_from_file_location("sat_antenna_jog", os.path.join(ODBPY, "sat_antenna_jog.py"))
jog = importlib.util.module_from_spec(spec); spec.loader.exec_module(jog)
sys.modules["sat_antenna_jog"] = jog
spec = importlib.util.spec_from_file_location("eco", os.path.join(ODBPY, "magic_antenna_eco.py"))
eco = importlib.util.module_from_spec(spec); spec.loader.exec_module(eco)

bad = 0
def check(name, got, want):
    global bad
    ok = got == want
    bad += not ok
    print(f"  {'ok ' if ok else 'FAIL'} {name:70} -> {got} (want {want})")


# ---- a minimal odb: layers, boxes, masters, instances, iterms, nets, a block ---------------------
class Layer:
    def __init__(s, n): s.n = n
    def getConstName(s): return s.n
    def getWidth(s): return 140
    def getSpacing(s): return 140
class Box:
    def __init__(s, layer, x0, y0, x1, y1): s.l, s.r = Layer(layer), (x0, y0, x1, y1)
    def getTechLayer(s): return s.l
    def xMin(s): return s.r[0]
    def yMin(s): return s.r[1]
    def xMax(s): return s.r[2]
    def yMax(s): return s.r[3]
    def getDX(s): return s.r[2] - s.r[0]
    def getDY(s): return s.r[3] - s.r[1]
    def isVia(s): return False
class MPin:
    def __init__(s, boxes): s.b = boxes
    def getGeometry(s): return s.b
class MTerm:
    def __init__(s, n, sig, boxes): s.n, s.sig, s.p = n, sig, [MPin(boxes)]
    def getConstName(s): return s.n
    def getSigType(s): return s.sig
    def getMPins(s): return s.p
class Master:
    def __init__(s, n, w, h, obs, mterms): s.n, s.w, s.h, s.obs, s.mt = n, w, h, obs, mterms
    def getName(s): return s.n
    def getWidth(s): return s.w
    def getHeight(s): return s.h
    def getObstructions(s): return s.obs
    def getMTerms(s): return s.mt
class Net:
    def __init__(s, n): s.n = n
    def getConstName(s): return s.n
    def getWire(s): return None
    def getSWires(s): return []
class ITerm:
    def __init__(s, net): s.net = net
    def getNet(s): return s.net
class Inst:
    def __init__(s, n, master, orient, origin, nets):
        s.n, s.m, s.o, s.org, s.nets = n, master, orient, origin, nets
    def getConstName(s): return s.n
    def getMaster(s): return s.m
    def getOrient(s): return s.o
    def getBBox(s):
        w, h = (s.m.w, s.m.h) if s.o in ("R0", "R180", "MX", "MY") else (s.m.h, s.m.w)
        return Box("none", s.org[0], s.org[1], s.org[0] + w, s.org[1] + h)
    def findITerm(s, n): return ITerm(s.nets.get(n))
class Block:
    def __init__(s, insts, nets=()): s.i, s.nets = insts, list(nets)
    def getInsts(s): return s.i
    def getNets(s): return s.nets


# ---- placement transform -------------------------------------------------------------------------
# MEASURED: pipe16 (RUN_2026.09.24_06.10.32) `_176600_` sky130_fd_sc_hd__dfxtp_2, orient MX (DEF FS)
# at (1412660, 1079840), master 7820 x 2720. Its LEF met1 OBS wire (570,1800)-(4530,1940) lands at
# y 1080620..1080760 — the exact edge the golden deck reported (1080.76) against an ECO tie.
W, H = 7820, 2720
check("MX (FS) mirrors y inside the cell, then moves to the origin",
      jog.placed_rect((570, 1800, 4530, 1940), W, H, "MX", (1412660, 1079840)),
      (1413230, 1080620, 1417190, 1080760))
check("R0 (N) only moves", jog.placed_rect((570, 1800, 4530, 1940), W, H, "R0", (0, 0)), (570, 1800, 4530, 1940))
check("MY (FN) mirrors x", jog.placed_rect((570, 1800, 4530, 1940), W, H, "MY", (0, 0)), (3290, 1800, 7250, 1940))
check("R180 (S) mirrors both", jog.placed_rect((570, 1800, 4530, 1940), W, H, "R180", (0, 0)), (3290, 780, 7250, 920))
for o in ("R0", "R90", "R180", "R270", "MX", "MY", "MXR90", "MYR90"):
    bw, bh = (W, H) if o in ("R0", "R180", "MX", "MY") else (H, W)
    check(f"{o}: the master box lands exactly on the placed box",
          jog.placed_rect((0, 0, W, H), W, H, o, (100, 200)), (100, 200, 100 + bw, 200 + bh))

# ---- the cell index: obstructions and other nets' pins are foreign; own pins and rails are not ----
dfxtp = Master("sky130_fd_sc_hd__dfxtp_2", W, H,
               obs=[Box("met1", 570, 1800, 4530, 1940), Box("met1", 965, 1460, 4965, 1600), Box("li1", 0, 0, 10, 10)],
               mterms=[MTerm("D", "SIGNAL", [Box("li1", 1370, 715, 1650, 1665)]),
                       MTerm("VPWR", "POWER", [Box("met1", 0, 2480, 7820, 2960)])])
mux = Master("m1pin_cell", 2000, 2720, obs=[],
             mterms=[MTerm("A", "SIGNAL", [Box("met1", 100, 100, 400, 400)]),
                     MTerm("X", "SIGNAL", [Box("met1", 1000, 100, 1300, 400)])])
flop = Inst("_176600_", dfxtp, "MX", (1412660, 1079840), {})
cell = Inst("_m_", mux, "R0", (1420480, 1079840), {"A": Net("_004349_"), "X": Net("other")})
blk = Block([flop, cell])
region = (1410000, 1078000, 1425000, 1084000)
got = sorted(jog.cell_rects(blk, region, "met1", "_004349_"))
check("obstructions and a foreign net's met1 pin are foreign metal",
      got, sorted([(1413230, 1080620, 1417190, 1080760), (1413625, 1080960, 1417625, 1081100),
                   (1421480, 1079940, 1421780, 1080240)]))
check("the net's OWN pin is not in the set", (1420580, 1079940, 1420880, 1080240) in got, False)
check("the power rails are not in the set (special wires already carry them)",
      any(r[3] - r[1] == 480 for r in got), False)
check("a region away from every cell finds nothing", jog.cell_rects(blk, (0, 0, 5000, 5000), "met1", "x"), [])

# ---- the ECO tie must see cell metal ------------------------------------------------------------
class ViaL1M1:
    def getBoxes(s): return [Box("li1", -85, -85, 85, 85), Box("mcon", -85, -85, 85, 85), Box("met1", -145, -115, 145, 115)]
class ViaM1M2:
    def getBoxes(s): return [Box("met1", -160, -130, 160, 130), Box("via", -75, -75, 75, 75), Box("met2", -130, -160, 130, 160)]
class Tech:
    def findLayer(s, n): return Layer(n)
    def findVia(s, n): return {"L1M1_PR": ViaL1M1(), "M1M2_PR": ViaM1M2()}.get(n)
T = Tech()
# MEASURED: pipe16 net _004349_ tie `li1 (1412200 1081200) L1M1_PR (* 1080860) (1415650 *) M1M2_PR`,
# its met1 at y 1080790..1080930 — 30 nm from the flop's OBS on each side (golden m1.2 x8 + m1.7 x3).
tie = [(1412200, 1081200), (1412200, 1080860), (1415650, 1080860)]
foreign = jog.collect_foreign(blk, (1400000, 1070000, 1430000, 1090000), "met1", "_004349_", 70, 140)
check("the measured tie is refused once cell metal is foreign", eco.tie_is_clear(T, foreign, tie), False)
check("with routed metal only (no cells) it slipped through",
      eco.tie_is_clear(T, jog.collect_foreign(Block([]), (1400000, 1070000, 1430000, 1090000), "met1", "_004349_", 70, 140), tie),
      True)

# ---- clearance is measured as DRC measures it: Euclidean between corners ------------------------
# MEASURED: pipe16 net _000908_'s router access via L1M1_PR at (1246270, 711450) — enclosure
# (1246125..1246415, 711335..711565) — sits 110 nm off each axis from a dfxtp OBS bump at
# (1245725..1246015, 711180..711225): 155.6 nm corner to corner, legal against 140. The router put it
# there and it is DRC-clean; a tie may land exactly as close.
bump = (1245725, 711180, 1246015, 711225)
grown = (bump[0] - 140, bump[1] - 140, bump[2] + 140, bump[3] + 140)
check("a corner 110 nm off on both axes (155.6 nm) is clear", jog.conflicts((1246125, 711335, 1246415, 711565), grown, 140), False)
check("the same corner 90 nm off on both axes (127 nm) is not", jog.conflicts((1246105, 711315, 1246395, 711545), grown, 140), True)
check("side by side at 139 nm is not", jog.conflicts((1246154, 711000, 1246400, 711400), grown, 140), True)
check("side by side at 140 nm is", jog.conflicts((1246155, 711000, 1246400, 711400), grown, 140), False)
check("the router's own access via passes the tie clearance",
      eco.tie_is_clear(T, jog.collect_foreign(Block([Inst("_b_", Master("bump", 7820, 2720,
            obs=[Box("met1", 0, 0, 290, 45)], mterms=[]), "R0", (1245725, 711180), {})]),
            (1240000, 705000, 1250000, 715000), "met1", "_000908_", 70, 140), [(1246270, 711450)]), True)

# ---- the ECO tie must not notch ITSELF ----------------------------------------------------------
# MEASURED: pipe16 net _006132_ tie `li1 (634340 1573520) L1M1_PR (634110 *) (* 1573860) M1M2_PR`.
# Two notches, both reported by golden sky130A_mr m1.2 and passed by Magic:
#  * L1M1 met1 enclosure top 1573635 vs M1M2 met1 enclosure bottom 1573730, overlapping in x over
#    634195..634270 where the 140-wide wire (x <= 634180) does not fill: 95 nm;
#  * the L1M1 enclosure's left edge (634195) vs the vertical wire's right edge (634180) above the
#    first segment (y 1573590..1573635): 15 nm.
notched = [(634340, 1573520), (634110, 1573520), (634110, 1573860)]
check("the measured tie notches itself", eco.tie_self_notches(T, notched, T.findVia("M1M2_PR")), True)
check("the 15 nm notch alone (met1 landing, no M1M2 via) is still caught",
      eco.tie_self_notches(T, notched, None), True)
straight = [(634110, 1573520), (634110, 1574000)]
check("a straight tie whose enclosures merge into its wire is clean",
      eco.tie_self_notches(T, straight, T.findVia("M1M2_PR")), False)

# A gap between two of a shape's own rects is a notch only where the shape leaves it EMPTY.
check("two rects 20 nm apart with nothing between them notch", jog.self_notch([(0, 0, 100, 100), (120, 0, 220, 100)], 140), True)
check("the same two rects with the gap filled do not",
      jog.self_notch([(0, 0, 100, 100), (120, 0, 220, 100), (90, 0, 130, 100)], 140), False)
check("a partly filled gap still notches",
      jog.self_notch([(0, 0, 100, 100), (120, 0, 220, 100), (90, 0, 130, 60)], 140), True)

print("PASS: cell geometry guard correct" if bad == 0 else f"FAIL: {bad} case(s)")
sys.exit(1 if bad else 0)
