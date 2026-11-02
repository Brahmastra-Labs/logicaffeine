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

# sky130_fd_sc_hd__nom.tlef, in DBU (1000 per um): width, spacing, minimum area per layer.
RULES = {"li1": (170, 170, 0), "met1": (140, 140, 83000), "met2": (140, 140, 67600),
         "met3": (300, 300, 240000), "met4": (300, 300, 240000), "mcon": (170, 190, 0), "via": (150, 170, 0),
         "via2": (200, 200, 0), "via3": (200, 200, 0)}

class _L:
    def __init__(s, n): s.n = n
    def getConstName(s): return s.n
    def getName(s): return s.n
    def getWidth(s): return RULES[s.n][0]
    def getSpacing(s): return RULES[s.n][1]
    def hasArea(s): return RULES[s.n][2] > 0
    def getArea(s): return RULES[s.n][2] / 1e6
class _B:
    def __init__(s, l, x0, y0, x1, y1): s.l, s.r = _L(l), (x0, y0, x1, y1)
    def getTechLayer(s): return s.l
    def getDX(s): return s.r[2] - s.r[0]
    def getDY(s): return s.r[3] - s.r[1]
    def xMin(s): return s.r[0]
    def yMin(s): return s.r[1]
    def xMax(s): return s.r[2]
    def yMax(s): return s.r[3]
class _Via:
    def __init__(s, name, bot, top, boxes): s.name, s.bot, s.top, s.boxes = name, _L(bot), _L(top), boxes
    def getName(s): return s.name
    def getBottomLayer(s): return s.bot
    def getTopLayer(s): return s.top
    def getBoxes(s): return [_B(*b) for b in s.boxes]
# Every enclosure and cut exactly as the tech LEF draws it.
VIAS = {
    "L1M1_PR":   _Via("L1M1_PR", "li1", "met1", [("mcon", -85, -85, 85, 85), ("li1", -85, -85, 85, 85), ("met1", -145, -115, 145, 115)]),
    "L1M1_PR_R": _Via("L1M1_PR_R", "li1", "met1", [("mcon", -85, -85, 85, 85), ("li1", -85, -85, 85, 85), ("met1", -115, -145, 115, 145)]),
    "M1M2_PR":   _Via("M1M2_PR", "met1", "met2", [("via", -75, -75, 75, 75), ("met1", -160, -130, 160, 130), ("met2", -130, -160, 130, 160)]),
    "M1M2_PR_R": _Via("M1M2_PR_R", "met1", "met2", [("via", -75, -75, 75, 75), ("met1", -130, -160, 130, 160), ("met2", -160, -130, 160, 130)]),
    "M2M3_PR":   _Via("M2M3_PR", "met2", "met3", [("via2", -100, -100, 100, 100), ("met2", -140, -185, 140, 185), ("met3", -165, -165, 165, 165)]),
    "M2M3_PR_R": _Via("M2M3_PR_R", "met2", "met3", [("via2", -100, -100, 100, 100), ("met2", -185, -140, 185, 140), ("met3", -165, -165, 165, 165)]),
    "M3M4_PR":   _Via("M3M4_PR", "met3", "met4", [("via3", -100, -100, 100, 100), ("met3", -190, -160, 190, 160), ("met4", -165, -165, 165, 165)]),
    "M3M4_PR_R": _Via("M3M4_PR_R", "met3", "met4", [("via3", -100, -100, 100, 100), ("met3", -160, -190, 160, 190), ("met4", -165, -165, 165, 165)]),
}
class _Tech:
    def findLayer(s, n): return _L(n)
    def findVia(s, n): return VIAS.get(n)
    def getDbUnitsPerMicron(s): return 1000

bad = 0
def check(name, got, want):
    global bad
    ok = got == want
    bad += not ok
    print(f"  {'ok ' if ok else 'FAIL'} {name:66} -> {got} (want {want})")

T = _Tech()
names = lambda stack: [v.getName() for v in stack]

# MEASURED: wpe_user_project net1123 (RUN_2026.09.27_18.14.14, 38-eco_jog_6.odb). `_353370_/B2`
# violated on met3 at ratio 2.79 (Partial 1114.57 vs Required 400). Its component below met3: its
# own L1M1+M1M2 access stack at (2455250, 2248250), 850 nm of met2 up to (2455250, 2249100), and
# 168 um of met3 west to (2289170, 2248420). Every one of 629 L-tie sites was refused.
B2 = (2455250, 2248250)
ops = [("PATH", _L("li1")), ("POINT", 2455250, 2248250), ("TECH_VIA", VIAS["L1M1_PR"]), ("TECH_VIA", VIAS["M1M2_PR"]),
       ("POINT", 2455250, 2249100), ("TECH_VIA", VIAS["M2M3_PR"]), ("POINT", 2455940, 2249100),
       ("JUNCTION", 3, _L("met3")), ("POINT", 2455250, 2249100), ("POINT", 2452950, 2249100),
       ("POINT", 2452950, 2249780), ("POINT", 2449730, 2249780), ("POINT", 2449730, 2248420),
       ("POINT", 2289170, 2248420)]
segs, vias = eco.wire_segments(ops), eco.wire_vias(ops)
t3 = eco.drop_targets(segs, vias, B2, "met3")
check("a met3 violation offers its 160 um met3 run as a drop target",
      ("met3", (2449730, 2248420), (2289170, 2248420)) in t3, True)
check("...and the met2 access stub below it", ("met2", (2455250, 2248250), (2455250, 2249100)) in t3, True)
check("a met2 violation offers no met3 (a met3 landing is uncredited at met2)",
      sorted({l for l, _a, _b in eco.drop_targets(segs, vias, B2, "met2")}), ["met2"])
# The gate's met1 stub drops back to li1, runs 8 um on li1, and climbs a pure stack
# (L1M1 + M1M2 + M2M3 at one point, no met1 or met2 segment) onto a met3 run.
ops_li = [("PATH", _L("li1")), ("POINT", 1000, 1000), ("TECH_VIA", VIAS["L1M1_PR"]), ("POINT", 5000, 1000),
          ("TECH_VIA", VIAS["L1M1_PR"]), ("POINT", 5000, 9000), ("TECH_VIA", VIAS["L1M1_PR"]),
          ("TECH_VIA", VIAS["M1M2_PR"]), ("TECH_VIA", VIAS["M2M3_PR"]), ("POINT", 60000, 9000)]
seg_li, via_li = eco.wire_segments(ops_li), eco.wire_vias(ops_li)
check("a pure via stack mid-route joins its layers (the met3 run is in the component)",
      sorted(eco.drop_targets(seg_li, via_li, (1000, 1000), "met3")),
      [("met1", (1000, 1000), (5000, 1000)), ("met3", (5000, 9000), (60000, 9000))])
check("...and the li1 run it passes through is never a landing (a stack starts at met1)",
      any(l == "li1" for l, _a, _b in eco.drop_targets(seg_li, via_li, (1000, 1000), "met3")), False)
check("the same stack capped at met2 stops short of the met3 run",
      sorted(eco.drop_targets(seg_li, via_li, (1000, 1000), "met2")), [("met1", (1000, 1000), (5000, 1000))])
check("tie_targets still stops at met2 for the L-tie",
      sorted({l for l, _a, _b in eco.tie_targets(segs, vias, B2, "met3")}), ["met2"])

check("met1 -> met3 stacks: both orientations of each via",
      sorted(names(s) for s in eco.via_stacks(T, "met1", "met3")),
      [["M1M2_PR", "M2M3_PR"], ["M1M2_PR", "M2M3_PR_R"], ["M1M2_PR_R", "M2M3_PR"], ["M1M2_PR_R", "M2M3_PR_R"]])
check("a landing on met1 needs no stack", eco.via_stacks(T, "met1", "met1"), [[]])

# Row 822 (y 2246720, N): the met3 run's y=2248420 is 1.700 um into the row, inside the diode's
# li1 pin (0.255..2.465) and clear of VPWR (met1 from 2.480). A diode_2 at x=2400060 has its pin
# at x 2400145..2400895.
row_y = 2246720
pin = (2400145, row_y + 255, 2400895, row_y + 2465)
run = [("met3", (2449730, 2248420), (2289170, 2248420))]
own = {"met1": [], "met2": [], "met3": [eco.path_rects([(2289170, 2248420), (2449730, 2248420)], 150)[0]]}
got = eco.plan_stack_drop(T, pin, run, {}, own, {})
check("a diode under the met3 run stacks straight up into it",
      (got[0], got[1], names(got[2])) if got else None, ((2400520, 2248420), "met3", ["M1M2_PR", "M2M3_PR"]))

vpwr = (2400000 - 140, row_y + 2480 - 140, 2401000 + 140, row_y + 2960 + 140)
check("the VPWR rail two tracks up does not block it",
      eco.plan_stack_drop(T, pin, run, {"met1": [vpwr]}, own, {}) is not None, True)
check("a run over the rail's end of the pin gives no drop (li1 enclosure would leave the pin)",
      eco.plan_stack_drop(T, pin, [("met3", (2289170, row_y + 2420), (2449730, row_y + 2420))], {}, own, {}), None)

m2_track = (2400520 - 70 - 140, 2240000, 2400520 + 70 + 140, 2260000)
check("a foreign met2 wire through the landing blocks every orientation",
      eco.plan_stack_drop(T, pin, run, {"met2": [m2_track]}, own, {}), None)
# A horizontal met2 wire whose bottom edge sits 290 nm above the landing: M2M3_PR's met2 reaches
# 185 nm up (a 105 nm gap) and M1M2_PR's 160 (130 nm), both inside the 140 nm rule; only the flat
# pair, reaching 140 and 130, clears it (150 nm).
m2_above = (2390000 - 140, 2248420 + 290 - 140, 2410000 + 140, 2248420 + 290 + 140 + 140)
got = eco.plan_stack_drop(T, pin, run, {"met2": [m2_above]}, own, {})
check("a met2 wire above the landing forces both vias flat",
      names(got[2]) if got else None, ["M1M2_PR_R", "M2M3_PR_R"])
check("nested enclosures are preferred: the plain stack draws no jog",
      eco.stack_jogs((0, 0), [VIAS["L1M1_PR"], VIAS["M1M2_PR"], VIAS["M2M3_PR"]]), 0)
check("L1M1_PR under M1M2_PR_R steps on met1 (15 nm each side)",
      eco.stack_jogs((0, 0), [VIAS["L1M1_PR"], VIAS["M1M2_PR_R"], VIAS["M2M3_PR_R"]]), 1)
check("a via2 cut 250 nm away breaks the 200 nm cut spacing -> refused",
      eco.plan_stack_drop(T, pin, run, {}, own, {"via2": [(2400520 + 250, 2248420)]}), None)
check("a via2 cut 400 nm away is legal",
      eco.plan_stack_drop(T, pin, run, {}, own, {"via2": [(2400520 + 400, 2248420)]}) is not None, True)
check("a via cut on the met1-met2 layer 200 nm away is refused (170 rule, 150 cut)",
      eco.plan_stack_drop(T, pin, run, {}, own, {"via": [(2400520 + 200, 2248420)]}), None)
own_notch = dict(own, met2=[(2400520 + 185 + 50, 2248000, 2400520 + 185 + 50 + 140, 2249000)])
check("own met2 50 nm from the landing's met2 is a same-net notch -> refused",
      eco.plan_stack_drop(T, pin, run, {}, own_notch, {}), None)

# Minimum area: the nested met1 island is the L1M1 enclosure inside the M1M2 one, 320 x 260 =
# 83,200 DBU^2 against met1's 83,000; the crossed one (M1M2_PR_R) is 90,100. A rule between the
# two refuses the nested stack and falls to the crossed one; a rule above both refuses the drop.
RULES["met1"] = (140, 140, 83300)
got = eco.plan_stack_drop(T, pin, run, {}, own, {})
check("a met1 island under the layer's minimum area is refused (the nested stack goes)",
      names(got[2]) if got else None, ["M1M2_PR_R", "M2M3_PR_R"])
RULES["met1"] = (140, 140, 90200)
check("...and with no orientation large enough, no drop at all",
      eco.plan_stack_drop(T, pin, run, {}, own, {}), None)
RULES["met1"] = (140, 140, 83000)
check("...and passes at sky130's 0.083 um^2",
      eco.plan_stack_drop(T, pin, run, {}, own, {}) is not None, True)

# A STACKED JOG. An upper-layer run of the gate's component that passes BESIDE the diode's pin is reached by the
# stack to the layer below it and one straight jog there. MEASURED on coproc_core (RUN_2026.10.03_15.18.47,
# 36-eco_jog_5.odb): `_45844_/B1` violated on met4 (ratio 1.12) with 34 um of met3 and 172 um of met4 in its
# component, and all 1,200 sites tried were refused — a straight stack needs the run directly over the pin, which a
# vertical run at one x or a horizontal one at one y almost never is.
pc = (2400520, row_y + 1360)
m4 = [("met4", (2402000, 2240000), (2402000, 2260000))]
own4 = {"met1": [], "met2": [], "met3": [], "met4": [eco.path_rects([(2402000, 2240000), (2402000, 2260000)], 150)[0]]}
check("a met4 run 1.48 um beside the pin has no straight stack",
      eco.plan_stack_drop(T, pin, m4, {}, own4, {}), None)
got = eco.plan_stack_jog(T, pin, m4, {}, own4, {})
check("...but a stack to met3, a met3 jog under the run and a via3 onto it",
      (got[0], got[1], got[2], names(got[3]), got[4].getName()) if got else None,
      (pc, (2402000, pc[1]), "met4", ["M1M2_PR", "M2M3_PR"], "M3M4_PR"))
check("a run beyond the jog's reach is not tied",
      eco.plan_stack_jog(T, pin, [("met4", (2400520 + 12000, 2240000), (2400520 + 12000, 2260000))], {}, own4, {}), None)
check("a run directly over the pin is the straight stack's, not a jog's",
      eco.plan_stack_jog(T, pin, [("met4", (2400520, 2240000), (2400520, 2260000))], {}, own4, {}), None)
cross3 = (2401200 - 150 - 300, 2240000, 2401200 + 150 + 300, 2260000)
check("a foreign met3 wire across the jog refuses it",
      eco.plan_stack_jog(T, pin, m4, {"met3": [cross3]}, own4, {}), None)
check("a via3 cut 250 nm from the landing breaks the 200 nm cut spacing",
      eco.plan_stack_jog(T, pin, m4, {}, own4, {"via3": [(2402000 + 250, pc[1])]}), None)
check("a via3 cut 450 nm away is legal",
      eco.plan_stack_jog(T, pin, m4, {}, own4, {"via3": [(2402000 + 450, pc[1])]}) is not None, True)
own3_notch = dict(own4, met3=[(2401000, pc[1] + 150 + 100, 2401600, pc[1] + 150 + 100 + 300)])
check("own met3 100 nm from the jog is a same-net notch -> refused",
      eco.plan_stack_jog(T, pin, m4, {}, own3_notch, {}), None)
m3 = [("met3", (2390000, row_y + 4000), (2410000, row_y + 4000))]
own3 = {"met1": [], "met2": [], "met3": [eco.path_rects([(2390000, row_y + 4000), (2410000, row_y + 4000)], 150)[0]]}
got = eco.plan_stack_jog(T, pin, m3, {}, own3, {})
check("a met3 run above the pin's row is reached by a vertical met2 jog and a via2",
      (got[1], got[2], names(got[3]), got[4].getName()) if got else None,
      ((pc[0], row_y + 4000), "met3", ["M1M2_PR"], "M2M3_PR"))
check("a met1 run takes no jog (met1 is the L-tie's)",
      eco.plan_stack_jog(T, pin, [("met1", (2390000, row_y + 4000), (2410000, row_y + 4000))], {}, own3, {}), None)

class _Enc:
    log = []
    def append(s, w): _Enc.log.append(("append",))
    def newPath(s, l, k): _Enc.log.append(("path", l.getConstName()))
    def addPoint(s, x, y): _Enc.log.append(("pt", x, y))
    def addITerm(s, it): _Enc.log.append(("iterm",))
    def addTechVia(s, v): _Enc.log.append(("via", v.getName()))
    def end(s): _Enc.log.append(("end",))
class _Net:
    def getWire(s): return object()
    def getBlock(s): return object()
    def getConstName(s): return "net1123"
class _It:
    def setConnected(s): pass
eco.odb.dbWireEncoder = _Enc
T.findLayer = lambda n: _L(n)
eco.apply_tie(T, _Net(), _It(), [(2400520, 2248420)], [VIAS["M1M2_PR"], VIAS["M2M3_PR"]])
check("the stack is encoded li1 -> L1M1 -> M1M2 -> M2M3 at one point",
      [e for e in _Enc.log if e[0] in ("path", "pt", "via")],
      [("path", "li1"), ("pt", 2400520, 2248420), ("via", "L1M1_PR"), ("via", "M1M2_PR"), ("via", "M2M3_PR")])
_Enc.log = []
eco.apply_tie(T, _Net(), _It(), [(0, 0), (500, 0)], VIAS["M1M2_PR"])
check("a single landing via is still encoded once",
      [e for e in _Enc.log if e[0] == "via"], [("via", "L1M1_PR"), ("via", "M1M2_PR")])
_Enc.log = []
eco.apply_tie(T, _Net(), _It(), [pc, VIAS["M1M2_PR"], VIAS["M2M3_PR"], (2402000, pc[1])], VIAS["M3M4_PR"])
check("a stacked jog is encoded li1 -> L1M1 -> M1M2 -> M2M3, the met3 jog, then via3",
      [e for e in _Enc.log if e[0] in ("path", "pt", "via")],
      [("path", "li1"), ("pt", pc[0], pc[1]), ("via", "L1M1_PR"), ("via", "M1M2_PR"), ("via", "M2M3_PR"),
       ("pt", 2402000, pc[1]), ("via", "M3M4_PR")])

# A DIODE ALONE ON AN ISLAND. sky130's diode_2 declares its pin's own antenna gate area (0.4347 um^2), so a
# diode tied where it is joined to the violating gate only through a HIGHER layer is judged as a gate of its own
# at every layer below that join. MEASURED on systolic_pipe16_realbist (RUN_2026.09.29_02.09.26): the diode for
# clkbuf_0_clk/A (a met4 violation) landed on a met3 run of `clk` that joins the gate only through met4, and the
# checker then flagged the DIODE: met3 side area 3162.42 x 0.4347 against 2773.88. The rule tables come from the
# LEF the ECO is given, never from constants.
LEF = """
LAYER met1
  THICKNESS 0.35 ;
  ANTENNADIFFSIDEAREARATIO PWL ( ( 0 400 ) ( 0.0125 400 ) ( 0.0225 2609 ) ( 22.5 11600 ) ) ;
END met1
LAYER met2
  THICKNESS 0.35 ;
  ANTENNADIFFSIDEAREARATIO PWL ( ( 0 400 ) ( 0.0125 400 ) ( 0.0225 2609 ) ( 22.5 11600 ) ) ;
END met2
LAYER met3
  THICKNESS 0.8 ;
  ANTENNADIFFSIDEAREARATIO PWL ( ( 0 400 ) ( 0.0125 400 ) ( 0.0225 2609 ) ( 22.5 11600 ) ) ;
END met3
LAYER met4
  THICKNESS 0.8 ;
  ANTENNADIFFSIDEAREARATIO PWL ( ( 0 400 ) ( 0.0125 400 ) ( 0.0225 2609 ) ( 22.5 11600 ) ) ;
END met4
MACRO sky130_fd_sc_hd__diode_2
  CLASS CORE ANTENNACELL ;
  PIN DIODE
    ANTENNAGATEAREA 0.434700 ;
    ANTENNADIFFAREA 0.434700 ;
  END DIODE
END sky130_fd_sc_hd__diode_2
"""
import tempfile
with tempfile.NamedTemporaryFile("w", suffix=".lef", delete=False) as fh:
    fh.write(LEF)
rules = eco.lef_antenna_rules(fh.name, "sky130_fd_sc_hd__diode_2", "DIODE")
os.unlink(fh.name)
check("the LEF gives each layer's thickness", (rules["layers"]["met1"][0], rules["layers"]["met3"][0]), (0.35, 0.8))
check("...and the diode's own gate and diffusion area", (rules["gate"], rules["diff"]), (0.4347, 0.4347))
check("the diode's allowance is the PWL at its own diffusion (the checker's 2773.88)",
      round(eco.pwl_at(rules["layers"]["met3"][1], rules["diff"]), 2), 2773.88)

# The gate reaches met2 by its access stack, climbs to met4, and on met4 meets a met3 run that goes back down
# 860 um away: at met3's deposit that run is not connected to the gate.
AC = (1000, 1000)
ops_clk = [("PATH", _L("li1")), ("POINT", 1000, 1000), ("TECH_VIA", VIAS["L1M1_PR"]), ("TECH_VIA", VIAS["M1M2_PR"]),
           ("POINT", 1000, 3000), ("TECH_VIA", VIAS["M2M3_PR"]), ("POINT", 1000, 4000), ("TECH_VIA", _Via("M3M4_PR", "met3", "met4", [])),
           ("POINT", 5000, 4000), ("TECH_VIA", _Via("M3M4_PR", "met3", "met4", [])),
           ("POINT", 5000, 4000), ("POINT", 865000, 4000),
           ("JUNCTION", 3, _L("met3")), ("POINT", 1000, 4000), ("POINT", 1000, 6000)]
seg_c, via_c = eco.wire_segments(ops_clk), eco.wire_vias(ops_clk)
W = {"met1": 140, "met2": 140, "met3": 300, "met4": 300}
far = ("met3", (5000, 4000), (865000, 4000))
near = ("met3", (1000, 4000), (1000, 6000))
check("the far met3 run is a drop target for a met4 violation (it is in the component below met4)",
      far in eco.drop_targets(seg_c, via_c, AC, "met4"), True)
check("...but a diode landing on it would sit alone on 860 um of met3 at met3's deposit: refused",
      eco.landing_is_safe(seg_c, via_c, AC, far, "met4", rules, 1000, W), False)
check("the gate's own met3 stub joins the gate at met3: accepted",
      eco.landing_is_safe(seg_c, via_c, AC, near, "met4", rules, 1000, W), True)
short = [op for op in ops_clk]
short[11] = ("POINT", 60000, 4000)
seg_s, via_s = eco.wire_segments(short), eco.wire_vias(short)
check("a gateless met3 island of 55 um is within the diode's allowance: accepted",
      eco.landing_is_safe(seg_s, via_s, AC, ("met3", (5000, 4000), (60000, 4000)), "met4", rules, 1000, W), True)
long_own = [("PATH", _L("li1")), ("POINT", 1000, 1000), ("TECH_VIA", VIAS["L1M1_PR"]), ("TECH_VIA", VIAS["M1M2_PR"]),
            ("POINT", 1000, 3000), ("TECH_VIA", VIAS["M2M3_PR"]), ("POINT", 901000, 3000)]
seg_o, via_o = eco.wire_segments(long_own), eco.wire_vias(long_own)
check("900 um of met3 in the gate's own component is safe however long (the diode joins the gate there)",
      eco.landing_is_safe(seg_o, via_o, AC, ("met3", (1000, 3000), (901000, 3000)), "met3", rules, 1000, W), True)
check("a met1 landing on the gate's own met1 island is always accepted",
      eco.landing_is_safe(seg_li, via_li, (1000, 1000), ("met1", (1000, 1000), (5000, 1000)), "met3", rules, 1000, W), True)

print("PASS: stacked diode drop geometry correct" if bad == 0 else f"FAIL: {bad} case(s)")
sys.exit(1 if bad else 0)
