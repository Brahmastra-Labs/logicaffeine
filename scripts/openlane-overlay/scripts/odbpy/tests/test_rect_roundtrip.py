import importlib.util, os, sys, types

ODBPY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
for m in ("odb", "click", "reader"):
    sys.modules.setdefault(m, types.ModuleType(m))
sys.modules["click"].command = lambda *a, **k: (lambda f: f)
sys.modules["click"].option = lambda *a, **k: (lambda f: f)
sys.modules["reader"].click_odb = lambda f: f
spec = importlib.util.spec_from_file_location("jog", os.path.join(ODBPY, "sat_antenna_jog.py"))
jog = importlib.util.module_from_spec(spec)
spec.loader.exec_module(jog)

bad = 0
def check(name, got, want):
    global bad
    bad += got != want
    print(f"  {'ok ' if got == want else 'FAIL'} {name:62} -> {got} (want {want})")

class _L:
    def __init__(s, n): s.n = n
    def getConstName(s): return s.n
class _V:
    def __init__(s, b, t): s.b, s.t = _L(b), _L(t)
    def getBottomLayer(s): return s.b
    def getTopLayer(s): return s.t
M3M4 = _V("met3", "met4")

# MEASURED: fabric_horner_seq net580 (RUN_2026.09.23_12.05.48, 31-eco_jog_5.def). The odb Python
# binding cannot read a RECT patch (getRect takes C++ out-parameters), so the jog refused every such
# wire; the DEF carries each patch anchored to a point on a layer.
DEF = """
    - net579 ( a X ) + USE SIGNAL
      + ROUTED met1 ( 1 1 ) ( 2 * )
      NEW met3 ( 548780 780300 ) RECT ( 9 9 9 9 )  ;
    - net580 ( max_cap580 X ) ( _35896_ B2 ) ( _29447_ A ) + USE SIGNAL
      + ROUTED met2 ( 549470 686630 ) ( * 687140 )
      NEW met3 ( 548780 687140 ) M3M4_PR
      NEW met3 ( 548780 780300 ) RECT ( 0 -150 390 150 )  ;
    - net581 ( b X ) + USE SIGNAL ;
"""
rects = jog.def_route_rects(DEF, "net580")
check("the net's own RECT is parsed, not a neighbour's", rects,
      [("met3", 548780, 780300, 0, -150, 390, 150)])
ops = [("PATH", _L("met3")), ("POINT", 548780, 687140), ("TECH_VIA", M3M4),
       ("POINT", 548780, 780300), ("TECH_VIA", M3M4), ("RECT",)]
# After the second via the path is back on met3 at (548780, 780300): the patch's anchor.
got = jog.attach_rects(ops, rects)
check("the patch is attached to its anchored RECT op", got[-1] if got else None,
      ("RECT", 0, -150, 390, 150))
check("an unmatched patch refuses the whole wire (never drop geometry)",
      jog.attach_rects(ops, [("met3", 1, 1, 0, 0, 1, 1)]), None)
check("a wire without patches is returned unchanged", jog.attach_rects(ops[:-1], []), ops[:-1])
check("a patch left over after matching also refuses (counts must agree)",
      jog.attach_rects(ops, rects + [("met1", 5, 5, 0, 0, 1, 1)]), None)

# MEASURED: the one refusal of 925 on horner was `clk` — its name also opens an entry in the PINS
# section, which the search found first. The parse must be confined to NETS.
DEF2 = """PINS 1 ;
    - clk + NET clk + DIRECTION INPUT + USE SIGNAL ;
END PINS
NETS 1 ;
    - clk ( PIN clk ) ( r0 CLK ) + USE CLOCK
      + ROUTED met3 ( 513130 523940 ) RECT ( 0 -150 390 150 )  ;
END NETS
"""
check("a net named like a pin is read from NETS, not PINS", jog.def_route_rects(DEF2, "clk"),
      [("met3", 513130, 523940, 0, -150, 390, 150)])

print("PASS: RECT patches round-trip" if bad == 0 else f"FAIL: {bad} case(s)")
sys.exit(1 if bad else 0)
