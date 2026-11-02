import importlib.util, os, sys, types

ODBPY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
# Stub the odb/click/reader imports so the pure-geometry helpers can be unit-tested.
for m in ("odb", "click", "reader"):
    sys.modules.setdefault(m, types.ModuleType(m))
sys.modules["click"].command = lambda *a, **k: (lambda f: f)
sys.modules["click"].option = lambda *a, **k: (lambda f: f)
sys.modules["reader"].click_odb = lambda f: f
spec = importlib.util.spec_from_file_location(
    "jog", os.path.join(ODBPY, "sat_antenna_jog.py"))
jog = importlib.util.module_from_spec(spec)
spec.loader.exec_module(jog)

MIN_W = 140          # sky130 met1/met2 minimum width, DBU
seg = [(1000, 500), (3000, 500)]   # a 2000-DBU horizontal segment

cases = [
    ("centred bridge, both stubs large",      (1800, 500), (2200, 500), False),
    ("p0 only 100 DBU from A  (sliver)",      (1100, 500), (1600, 500), True),
    ("p1 only 120 DBU from B  (sliver)",      (1600, 500), (2880, 500), True),
    ("stub exactly at min width (legal)",     (1140, 500), (2860, 500), False),
    ("p0 exactly on A (no stub at all)",      (1000, 500), (1500, 500), False),
]
bad = 0
for name, p0, p1, want in cases:
    got = jog.bridge_leaves_slivers(seg, p0, p1, MIN_W)
    ok = "ok " if got == want else "FAIL"
    if got != want: bad += 1
    print(f"  {ok} {name:42} -> {got} (want {want})")

# --- via orientation: the enclosure must run ALONG the wire, not across it ---
class _V:
    def __init__(self, n): self.n = n
class _Tech:
    """sky130 ships M1M2_PR and M1M2_PR_R; a PDK without a rotated variant must still work."""
    def __init__(self, names): self.names = names
    def findVia(self, n): return _V(n) if n in self.names else None

sky = _Tech({"M1M2_PR", "M1M2_PR_R", "M2M3_PR", "M2M3_PR_R"})
bare = _Tech({"M1M2_PR"})
via_cases = [
    ("horizontal met1 segment -> unrotated", sky,  "met1", True,  "M1M2_PR"),
    ("VERTICAL met1 segment   -> rotated",   sky,  "met1", False, "M1M2_PR_R"),
    ("horizontal met2 segment -> unrotated", sky,  "met2", True,  "M2M3_PR"),
    ("VERTICAL met2 segment   -> rotated",   sky,  "met2", False, "M2M3_PR_R"),
    ("no rotated variant -> falls back",     bare, "met1", False, "M1M2_PR"),
]
for name, tech, layer, horiz, want in via_cases:
    _, v = jog.bridge_layer_via(tech, layer, horiz)
    got = v.n if v else None
    ok = "ok " if got == want else "FAIL"
    if got != want: bad += 1
    print(f"  {ok} {name:42} -> {got} (want {want})")

# --- via CUT SPACING: the window width IS the via-to-via centre distance ---
# sky130 tech LEF (sky130_fd_sc_hd__*.tlef):
#     LAYER via   WIDTH 0.15  SPACING 0.17   -> centres must be >= 0.32 um apart
#     LAYER via2  WIDTH 0.20  SPACING 0.20   -> centres must be >= 0.40 um apart
class _L:
    def __init__(self, n, sp=0): self.n, self.sp = n, sp
    def getConstName(self): return self.n
    def getSpacing(self): return self.sp
class _B:
    def __init__(self, ly, dx, dy): self.ly, self.dx, self.dy = ly, dx, dy
    def getTechLayer(self): return self.ly
    def getDX(self): return self.dx
    def getDY(self): return self.dy
class _TV:
    """A tech via as odb presents it: a cut box plus the two metal enclosures."""
    def __init__(self, bot, top, boxes): self.bot, self.top, self.boxes = bot, top, boxes
    def getBottomLayer(self): return self.bot
    def getTopLayer(self): return self.top
    def getBoxes(self): return self.boxes
class _Broken:
    def getBottomLayer(self): raise RuntimeError("no such method on this odb build")

m1, m2, m3 = _L("met1"), _L("met2"), _L("met3")
v1 = _TV(m1, m2, [_B(_L("via", 170), 150, 150), _B(m1, 320, 260), _B(m2, 320, 260)])
v2 = _TV(m2, m3, [_B(_L("via2", 200), 200, 200), _B(m2, 280, 370), _B(m3, 330, 330)])
v_nospace = _TV(m1, m2, [_B(_L("via", 0), 150, 150), _B(m1, 320, 260), _B(m2, 320, 260)])

rule_cases = [
    ("sky130 via1  -> cut 150, spacing 170", v1,        ("via", 150, 150, 170)),
    ("sky130 via2  -> cut 200, spacing 200", v2,        ("via2", 200, 200, 200)),
    ("cut layer reports no spacing -> REFUSE", v_nospace, (None, 0, 0, 0)),
    ("via cannot be interrogated  -> REFUSE", _Broken(), (None, 0, 0, 0)),
]
for name, via, want in rule_cases:
    got = jog.via_cut_rule(via)
    ok = "ok " if got == want else "FAIL"
    if got != want: bad += 1
    print(f"  {ok} {name:42} -> {got} (want {want})")

# The pitch the router must clear, and the ladder rungs that can never satisfy it.
for via, want_pitch, legal, illegal in ((v1, 320, (500, 340), (240, 180)),
                                        (v2, 400, (500,), (340, 240, 180))):
    _, cw, _ch, sp = jog.via_cut_rule(via)
    pitch = cw + sp
    ok = "ok " if pitch == want_pitch else "FAIL"
    if pitch != want_pitch: bad += 1
    print(f"  {ok} {'pitch for ' + jog.via_cut_rule(via)[0]:42} -> {pitch} (want {want_pitch})")
    for w in legal:
        if w < pitch: bad += 1; print(f"  FAIL {w} DBU window wrongly under pitch {pitch}")
    for w in illegal:
        if w >= pitch: bad += 1; print(f"  FAIL {w} DBU window wrongly accepted at pitch {pitch}")

# The predicate, including the EXACT centres measured in the failing run.
# coproc_core RUN_2026.09.22_05.14.34, layer 68/44 (`via` cut), from coproc_core.klayout.gds.
CW = CH = 150; SP = 170
cut_cases = [
    ("MEASURED spot1: 577.870 vs 578.110 (240 apart)", (577870, 493850), (578110, 493850), True),
    ("MEASURED spot0: 1121.910 vs 1122.170 (260)",     (1121910, 438770), (1122170, 438770), True),
    ("MEASURED spot0: 1122.170 vs 1122.410 (240)",     (1122170, 438770), (1122410, 438770), True),
    ("exactly at the rule: 320 apart (legal)",         (0, 0), (320, 0), False),
    ("comfortably apart: 500 (legal)",                 (0, 0), (500, 0), False),
    ("one DBU under the rule: 319",                    (0, 0), (319, 0), True),
    ("stacked on the same point",                      (0, 0), (0, 0), True),
    ("close in x but far in y -> legal",               (0, 0), (240, 400), False),
    ("close in y but far in x -> legal",               (0, 0), (400, 240), False),
    ("close in BOTH axes -> illegal",                  (0, 0), (240, 240), True),
]
for name, c0, c1, want in cut_cases:
    got = jog.cuts_too_close(c0, c1, CW, CH, SP)
    ok = "ok " if got == want else "FAIL"
    if got != want: bad += 1
    print(f"  {ok} {name:42} -> {got} (want {want})")

if bad == 0:
    print("PASS: sliver predicate correct")
    print("PASS: jog geometry guards correct")
else:
    print(f"FAIL: {bad} case(s)")
sys.exit(1 if bad else 0)
