import importlib.util, os, random, sys, types

ODBPY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
for m in ("odb", "click", "reader"):
    sys.modules.setdefault(m, types.ModuleType(m))
sys.modules["click"].command = lambda *a, **k: (lambda f: f)
sys.modules["click"].option = lambda *a, **k: (lambda f: f)
sys.modules["reader"].click_odb = lambda f: f
sys.path.insert(0, ODBPY)
spec = importlib.util.spec_from_file_location("jog", os.path.join(ODBPY, "sat_antenna_jog.py"))
jog = importlib.util.module_from_spec(spec); spec.loader.exec_module(jog)

class _Rect:
    def __init__(s, r): s.r = r
    def xMin(s): return s.r[0]
    def yMin(s): return s.r[1]
    def xMax(s): return s.r[2]
    def yMax(s): return s.r[3]
class _Wire:
    def __init__(s, r): s.r = r
    def getBBox(s): return (True, _Rect(s.r))
class _Layer:
    def __init__(s, n): s.n = n
    def getConstName(s): return s.n
    def getSpacing(s): return 170
class _Box:
    def __init__(s, r, layer=None, via=None): s.r, s.layer, s.via = r, layer, via
    def xMin(s): return s.r[0]
    def yMin(s): return s.r[1]
    def xMax(s): return s.r[2]
    def yMax(s): return s.r[3]
    def isVia(s): return s.via is not None
    def getTechVia(s): return s.via
    def getTechLayer(s): return _Layer(s.layer)
class _VBox:
    def __init__(s, layer): s.l = _Layer(layer)
    def getTechLayer(s): return s.l
    def getDX(s): return 150
    def getDY(s): return 150
class _TechVia:
    def getBottomLayer(s): return _Layer("met1")
    def getTopLayer(s): return _Layer("met2")
    def getBoxes(s): return [_VBox("met1"), _VBox("via"), _VBox("met2")]
class _SW:
    def __init__(s, boxes): s.boxes = boxes
    def getWires(s): return s.boxes
class _Net:
    def __init__(s, name, r, sw=()): s.name, s.w, s.sw = name, (_Wire(r) if r else None), list(sw)
    def getConstName(s): return s.name
    def getWire(s): return s.w
    def getSWires(s): return s.sw
    def getBlock(s): return _Block(NETS)
# SWIG hands back a NEW proxy object on every `getBlock()`; all of them wrap one C++ pointer.
class _Ptr:
    def __int__(s): return 0x5eed
class _Block:
    def __init__(s, nets): s.nets, s.this = nets, _Ptr()
    def getNets(s): return list(s.nets)

bad = 0
def check(name, got, want):
    global bad
    ok = got == want
    bad += not ok
    print(f"  {'ok ' if ok else 'FAIL'} {name:70} -> {'ok' if ok else (got, want)}")

rng = random.Random(20260928)
def rect(span=400000, size=60000):
    x, y = rng.randrange(0, span), rng.randrange(0, span)
    return (x, y, x + rng.randrange(0, size), y + rng.randrange(0, size))

nets = []
for i in range(1500):
    sw = []
    if i % 50 == 0:
        boxes = [_Box(rect(size=300000), layer=rng.choice(["met4", "met5"])) for _ in range(6)]
        boxes += [_Box((x, y, x + 800, y + 800), via=_TechVia()) for x, y in
                  ((rng.randrange(0, 400000), rng.randrange(0, 400000)) for _ in range(10))]
        sw.append(_SW(boxes))
    nets.append(_Net(f"n{i}" if i % 7 else f"\\bus[{i}]", rect() if i % 11 else None, sw))
NETS = nets
BLOCK = _Block(nets)
idx = jog.net_index(BLOCK)

def brute_near(probe):
    return [n.getConstName() for n in nets if n.getWire() is not None and jog.overlaps(n.getWire().r, probe)]

probes = [rect(size=rng.choice([500, 5000, 50000, 200000])) for _ in range(400)]
probes += [(x, y, x + 3000, y + 3000) for x, y in ((rng.randrange(1500000, 2000000), rng.randrange(0, 400000)) for _ in range(40))]
check("near() equals the full walk, in block order, over 400 random probes",
      all([n.getConstName() for n in idx.near(p)] == brute_near(p) for p in probes), True)
check("a net with no wire is never returned", any(n.getWire() is None for n in idx.near((0, 0, 10**7, 10**7))), False)
check("net lookup by escape-insensitive name finds the escaped bus net",
      idx.by_norm[jog.norm("\\bus[14]")].getConstName(), "\\bus[14]")

moved = nets[3]
old = moved.w.r
moved.w = _Wire((900000, 900000, 901000, 901000))
check("before touch the index still holds the old box", moved in idx.near(old), True)
jog.touch_net(moved)
check("after touch the net is found at its new place", moved in idx.near((900500, 900500, 900600, 900600)), True)
check("...and no longer at its old one", moved in idx.near(old) and not jog.overlaps(moved.w.r, old), False)
check("every other query still equals the full walk after the move",
      all([n.getConstName() for n in idx.near(p)] == brute_near(p) for p in probes), True)
check("a second proxy of the same block reaches the same index (keyed by identity, not id())",
      jog.net_index(_Block(nets)) is idx, True)

# A bridge re-encodes the wire through `encode_wire`; the index must follow it without the caller
# doing anything, or a later gate's clearance query cannot see the bridge it has to clear.
class _Enc:
    def begin(s, w): s.w, s.pts = w, []
    def newPath(s, *a): pass
    def addPoint(s, x, y, *e): s.pts.append((x, y)); return len(s.pts) - 1
    def end(s):
        xs, ys = [p[0] for p in s.pts], [p[1] for p in s.pts]
        s.w.net.w = _Wire((min(xs), min(ys), max(xs), max(ys)))
class _NewWire:
    def __init__(s, net): s.net = net
jog.odb.dbWire_destroy = lambda w: None
jog.odb.dbWire_create = lambda net: _NewWire(net)
jog.odb.dbWireEncoder = _Enc
bridged = nets[5]
before = bridged.w.r
jog.encode_wire(bridged, [("PATH", _Layer("met3")), ("POINT", 1200000, 50000), ("POINT", 1260000, 50000)])
check("encode_wire re-indexes the net it rewrote (found at its new place)",
      bridged in idx.near((1230000, 49000, 1231000, 51000)), True)
check("...and every query still equals the full walk",
      all([n.getConstName() for n in idx.near(p)] == brute_near(p) for p in probes + [before]), True)

def brute_special(layer, region, grow):
    out = []
    for n in nets:
        for sw in n.getSWires():
            for b in sw.getWires():
                if b.isVia() or b.layer != layer:
                    continue
                g = (b.r[0] - grow, b.r[1] - grow, b.r[2] + grow, b.r[3] + grow)
                if jog.overlaps(g, region):
                    out.append((n.getConstName(), b.r))
    return out
def brute_cuts(region):
    out = []
    for n in nets:
        for sw in n.getSWires():
            for b in sw.getWires():
                if not b.isVia():
                    continue
                c = ((b.r[0] + b.r[2]) // 2, (b.r[1] + b.r[3]) // 2)
                if region[0] <= c[0] <= region[2] and region[1] <= c[1] <= region[3]:
                    out.append((c, "via"))
    return out
check("special_rects equals the full walk (grown by the spacing rule), in order",
      all(idx.special_rects(l, p, 140) == brute_special(l, p, 140) for p in probes for l in ("met4", "met5")), True)
check("special_cuts equals the full walk, in order",
      all(idx.special_cuts(p) == brute_cuts(p) for p in probes), True)
check("the probes exercise hits as well as misses",
      sum(1 for p in probes if brute_special("met4", p, 140)) > 20 and sum(1 for p in probes if brute_cuts(p)) > 5
      and sum(1 for p in probes if not brute_near(p)) > 5, True)

print("PASS: net index equals the full walk" if bad == 0 else f"FAIL: {bad} case(s)")
sys.exit(1 if bad else 0)
