#!/usr/bin/env python3
"""Generate the static SVG benchmark charts embedded in the root README.

Reads the four blessed result JSONs that the web Benchmarks page also bakes in
(benchmarks/results/{solvers,latest,latest-interp,latest-codec}.json) and writes
self-contained, presentation-only SVGs to benchmarks/results/charts/. Gradients,
shapes and text only — no <style>, <script>, <animate>, <foreignObject>, filter or
external font — so GitHub's markdown sanitizer renders them verbatim, and each
carries its own gradient background so it reads the same in light and dark mode.
The palette mirrors apps/logicaffeine_web/src/ui/pages/benchmarks.rs and the
headline numbers are computed the same way the page computes them.

    python3 benchmarks/gen-readme-charts.py           # write the charts
    python3 benchmarks/gen-readme-charts.py --check    # write + validate
"""

import json
import math
import os
import sys
from xml.dom import minidom

HERE = os.path.dirname(os.path.abspath(__file__))
RESULTS = os.path.join(HERE, "results")
CHARTS = os.path.join(RESULTS, "charts")

# --- palette (matches the live benchmarks page) -----------------------------
INK = "#eef2f8"
DIM = "#93a4bd"
FAINT = "#5b6b86"
GRID = "rgba(255,255,255,0.07)"
CYAN = "#00d4ff"      # LOGOS
PURPLE = "#a78bfa"    # Z3
ORANGE = "#fb923c"    # Kissat
GRAY = "#9aa6b8"      # SaDiCaL
ZIG = "#f7a41d"
RUST = "#dea584"
GREEN = "#34d399"
CARD = "rgba(255,255,255,0.018)"
CARDLINE = "rgba(255,255,255,0.06)"
FONT = "ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
MONO = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"

LANG_COLOR = {"c": "#7d8da3", "cpp": "#f34b7d", "rust": RUST, "zig": ZIG,
              "logos_release": CYAN, "go": "#22c3e6", "java": "#c98a52",
              "js": "#f7df1e", "nim": "#ffe953"}
SOLVER_COLOR = {"z3": PURPLE, "kissat": ORANGE, "sadical": GRAY}
SOLVER_LABEL = {"z3": "Z3", "kissat": "Kissat", "sadical": "SaDiCaL"}


def lighten(hexv, amt):
    h = hexv.lstrip("#")
    if len(h) == 3:
        h = "".join(c * 2 for c in h)
    r, g, b = int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)
    f = lambda v: int(v + (255 - v) * amt)
    return f"#{f(r):02x}{f(g):02x}{f(b):02x}"


def esc(s):
    return (str(s).replace("&", "&amp;").replace("<", "&lt;")
            .replace(">", "&gt;").replace('"', "&quot;"))


# --- SVG builder with gradient defs ----------------------------------------
class Svg:
    def __init__(self, w, h, title):
        self.w, self.h, self.title = w, h, title
        self.body, self.defs, self._grad = [], [], {}
        self.bg = self.grad("#0a1224", "#05070e", vertical=True)

    def grad(self, c0, c1=None, vertical=False):
        if c1 is None:
            if not c0.startswith("#"):
                return c0
            c1 = c0
            c0 = lighten(c0, 0.42)
        key = (c0, c1, vertical)
        if key in self._grad:
            return f"url(#{self._grad[key]})"
        gid = f"g{len(self._grad)}"
        self._grad[key] = gid
        v = ('x1="0" y1="0" x2="0" y2="1"' if vertical
             else 'x1="0" y1="0" x2="1" y2="0"')
        self.defs.append(
            f'<linearGradient id="{gid}" {v}><stop offset="0" stop-color="{c0}"/>'
            f'<stop offset="1" stop-color="{c1}"/></linearGradient>')
        return f"url(#{gid})"

    def hatch(self, ident, color):
        self.defs.append(
            f'<pattern id="{ident}" patternUnits="userSpaceOnUse" width="7" '
            f'height="7" patternTransform="rotate(45)">'
            f'<rect width="7" height="7" fill="{color}" opacity="0.10"/>'
            f'<line x1="0" y1="0" x2="0" y2="7" stroke="{color}" '
            f'stroke-width="2.2" opacity="0.5"/></pattern>')

    def rect(self, x, y, w, h, fill, stroke=None, sw=1, rx=0, opacity=None):
        a = (f'<rect x="{x:.1f}" y="{y:.1f}" width="{max(0,w):.1f}" '
             f'height="{max(0,h):.1f}"')
        if rx:
            a += f' rx="{rx:.1f}"'
        a += f' fill="{fill}"'
        if stroke:
            a += f' stroke="{stroke}" stroke-width="{sw}"'
        if opacity is not None:
            a += f' opacity="{opacity}"'
        self.body.append(a + "/>")

    def line(self, x1, y1, x2, y2, stroke, sw=1, dash=None):
        a = (f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
             f'stroke="{stroke}" stroke-width="{sw}"')
        if dash:
            a += f' stroke-dasharray="{dash}"'
        self.body.append(a + "/>")

    def text(self, x, y, s, size=13, fill=INK, anchor="start", weight=None,
             mono=False, opacity=None, spacing=None):
        a = (f'<text x="{x:.1f}" y="{y:.1f}" font-family="{MONO if mono else FONT}" '
             f'font-size="{size}" fill="{fill}" text-anchor="{anchor}"')
        if weight:
            a += f' font-weight="{weight}"'
        if opacity is not None:
            a += f' opacity="{opacity}"'
        if spacing:
            a += f' letter-spacing="{spacing}"'
        self.body.append(a + f">{esc(s)}</text>")

    def header(self, title, subtitle, kicker=None):
        # top accent rule (cyan -> transparent)
        self.body.append(
            f'<rect x="0" y="0" width="{self.w}" height="3.5" '
            f'fill="{self.grad(CYAN, "#05070e")}"/>')
        if kicker:
            self.text(40, 20, kicker, size=10, fill=CYAN, weight="700",
                      spacing="2", mono=True)
        self.rect(24, 30, 6, 19, self.grad(CYAN), rx=3)
        self.text(40, 45, title, size=19, weight="800")
        self.text(40, 64, subtitle, size=12.5, fill=DIM)

    def panel(self, x, y, w, h, label=None, accent=None):
        self.rect(x, y, w, h, CARD, stroke=CARDLINE, sw=1, rx=10)
        if accent:
            self.rect(x, y, 3.5, h, self.grad(accent), rx=2)
        if label:
            self.text(x + 16, y + 20, label, size=13.5, weight="700",
                      fill=accent or INK)

    def done(self):
        out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.w}" '
               f'height="{self.h}" viewBox="0 0 {self.w} {self.h}" role="img" '
               f'aria-label="{esc(self.title)}">',
               f'<title>{esc(self.title)}</title>',
               "<defs>" + "".join(self.defs) + "</defs>",
               f'<rect x="0" y="0" width="{self.w}" height="{self.h}" rx="16" '
               f'fill="{self.bg}"/>',
               f'<rect x="1" y="1" width="{self.w-2}" height="{self.h-2}" '
               f'rx="16" fill="none" stroke="rgba(255,255,255,0.05)"/>']
        out += self.body
        out.append("</svg>")
        return "\n".join(out) + "\n"


def fmt_time(ms):
    if ms >= 1000:
        return f"{ms/1000:.2f}s"
    if ms >= 1:
        return f"{ms:.1f}ms"
    if ms >= 0.001:
        return f"{ms*1000:.0f}µs"
    return f"{ms*1e6:.0f}ns"


def fmt_ns(ns):
    if ns < 1000:
        return f"{ns:.0f}ns"
    if ns < 1e6:
        return f"{ns/1000:.1f}µs"
    return f"{ns/1e6:.2f}ms"


def fmt_bytes(b):
    if b >= 1024:
        return f"{b/1024:.1f} KB"
    return f"{b:.0f} B"


def geomean(xs):
    return math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else 0.0


def load(name):
    with open(os.path.join(RESULTS, name), encoding="utf-8") as f:
        return json.load(f)


def lbl_value(s, x, y, text, color, inside_dark=False, anchor_outside=True):
    s.text(x, y, text, size=10.5, fill=color, mono=True, weight="600")


# ===========================================================================
# 1. SAT solvers  (headline, top of README)
# ===========================================================================
def chart_sat():
    d = load("solvers.json")
    meta = d["metadata"]
    fam = {f["id"]: f for f in d["families"]}
    short = {"tseitin": ("Tseitin parity", "GF(2) Gaussian"),
             "mod_3_tseitin": ("Mod-3 Tseitin", "GF(3) Gaussian"),
             "mod_5_tseitin": ("Mod-5 Tseitin", "GF(5) Gaussian"),
             "mod_7_tseitin": ("Mod-7 Tseitin", "GF(7) Gaussian"),
             "mutilated_chessboard": ("Mutilated board", "Hall witness"),
             "php": ("Pigeonhole PHP", "certified SR proof"),
             "random_3sat": ("Random 3-SAT", "CDCL — control")}
    order = ["tseitin", "mod_3_tseitin", "mod_5_tseitin", "mod_7_tseitin",
             "mutilated_chessboard", "php", "random_3sat"]
    rows = [(fid, max(fam[fid]["rows"], key=lambda x: x["n"])) for fid in order]

    W, X0, X1 = 880, 250, 838
    PLOTW = X1 - X0
    top = 96
    bar_h, bar_gap = 11, 3
    block = 4 * bar_h + 3 * bar_gap + 26
    H = top + len(rows) * block + 58
    s = Svg(W, H, "LOGOS SAT/proof solving vs Z3, Kissat, SaDiCaL")
    s.hatch("to", "#ffffff")
    s.header("SAT & proof solving — vs Z3, Kissat, SaDiCaL",
             "Structural UNSAT, decided & certified in microseconds where "
             "resolution solvers hit the wall. Log scale.",
             kicker="CERTIFIED · BROWSER-IDENTICAL")
    lx = 545
    for name, col in [("LOGOS", CYAN), ("Z3", PURPLE), ("Kissat", ORANGE),
                      ("SaDiCaL", GRAY)]:
        s.rect(lx, 12, 11, 11, s.grad(col), rx=3)
        s.text(lx + 16, 21, name, size=11.5, fill=INK)
        lx += 24 + len(name) * 7.6

    lo, hi = math.log10(0.005), math.log10(60000)

    def xof(ms):
        v = (math.log10(max(ms, 0.005)) - lo) / (hi - lo)
        return X0 + max(0.0, min(1.0, v)) * PLOTW

    gtop, gbot = top - 16, H - 44
    s.text(X0, top - 22, "← faster", size=10, fill=FAINT, anchor="start")
    s.text(X1, top - 22, "slower / timeout →", size=10, fill=FAINT, anchor="end")
    for ms, lab in [(0.01, "10µs"), (1, "1ms"), (100, "100ms"), (10000, "10s")]:
        gx = xof(ms)
        s.line(gx, gtop, gx, gbot, GRID, 1, dash="2,5")
        s.text(gx, gbot + 15, lab, size=10.5, fill=FAINT, anchor="middle",
               mono=True)

    y = top
    for fid, r in rows:
        name, mech = short[fid]
        cy = y + (4 * bar_h + 3 * bar_gap) / 2
        s.text(22, cy - 7, name, size=13, weight="700", fill=INK)
        s.text(22, cy + 9, f"n = {r['n']}", size=11, fill=DIM, mono=True)
        s.text(238, cy + 4, mech, size=10, fill=FAINT, anchor="end")
        bars = [("LOGOS", CYAN, r["ours_ms"], "solved")]
        omap = {o["solver"]: o for o in r["others"]}
        for sid in ("z3", "kissat", "sadical"):
            o = omap.get(sid)
            if o:
                bars.append((SOLVER_LABEL[sid], SOLVER_COLOR[sid], o["ms"],
                             o.get("status")))
        by = y
        for label, col, ms, status in bars:
            bx = xof(ms)
            to = status == "timeout"
            if to:
                s.rect(X0, by, bx - X0, bar_h, "url(#to)", rx=3)
                s.rect(X0, by, bx - X0, bar_h, "none", stroke=col, sw=1.3, rx=3)
                tlab = f"timeout • {fmt_time(ms)}"
            else:
                s.rect(X0, by, bx - X0, bar_h, s.grad(col), rx=3)
                tlab = fmt_time(ms)
            est = len(tlab) * 6.4
            if bx + 6 + est < X1:
                s.text(bx + 6, by + bar_h - 1.5, tlab, size=10.5,
                       fill=(DIM if to else col), mono=True, weight="600")
            else:
                s.text(bx - 6, by + bar_h - 1.5, tlab, size=10.5,
                       fill=(INK if to else "#05070e"), anchor="end",
                       mono=True, weight="700")
            by += bar_h + bar_gap
        y += block

    s.text(24, H - 14,
           f"Measured: wall-clock time to solve + certify (log scale, shorter = "
           f"faster). i9-14900K · LOGOS = median of {meta.get('ours_runs','?')} "
           f"runs · Z3 10s / Kissat 15s / SaDiCaL 45s timeouts.",
           size=9.5, fill=FAINT)
    return s.done(), {"families": len(rows)}


# ===========================================================================
# 2. Overall — geomean speed vs C, across all languages
# ===========================================================================
def chart_overall():
    d = load("latest.json")
    m = d["summary"]["geometric_mean_speedup_vs_c"]
    label = {x["id"]: x["label"] for x in d["languages"]}

    # website-exact apples geomean (mean_ms, exclude the 5 curated collapses)
    COLLAPSE = {"fib", "ackermann", "binary_trees", "loop_sum", "collect"}
    xs = []
    for b in d["benchmarks"]:
        if b["id"] in COLLAPSE:
            continue
        t = b["scaling"].get(b["reference_size"], {})
        c, l = t.get("c"), t.get("logos_release")
        if c and l and c["mean_ms"] > 0 and l["mean_ms"] > 0:
            xs.append(c["mean_ms"] / l["mean_ms"])
    apples = geomean(xs)

    rows = [("LOGOS — all 32", m["logos_release"], CYAN, "incl. 5 optimizer collapses"),
            ("LOGOS — same algorithm", apples, lighten(CYAN, 0.0), "collapses excluded"),
            (label["rust"], m["rust"], LANG_COLOR["rust"], None),
            (label["zig"], m["zig"], LANG_COLOR["zig"], None),
            (label["cpp"], m["cpp"], LANG_COLOR["cpp"], None),
            ("C  (baseline)", 1.0, LANG_COLOR["c"], None),
            (label["go"], m["go"], LANG_COLOR["go"], None),
            (label["nim"], m["nim"], LANG_COLOR["nim"], None),
            (label["java"], m["java"], LANG_COLOR["java"], None),
            (label["js"], m["js"], LANG_COLOR["js"], None)]

    W, X0, X1 = 880, 210, 720
    PLOTW = X1 - X0
    top = 96
    bh, bg = 24, 12
    H = top + len(rows) * (bh + bg) + 52
    s = Svg(W, H, "Geomean speed vs C across languages")
    s.header("Overall — geomean speed vs C, across 32 benchmarks",
             "Higher = faster than C (gcc -O3 -march=native -flto). LOGOS shown "
             "both ways; everyone runs the same programs.")
    lin_max = 2.75

    def xlin(r):
        return X0 + min(r, lin_max) / lin_max * PLOTW

    for r in (0.5, 1.0, 1.5, 2.0, 2.5):
        gx = xlin(r)
        is_c = abs(r - 1.0) < 1e-9
        s.line(gx, top - 8, gx, top + len(rows) * (bh + bg) - bg + 6,
               LANG_COLOR["c"] if is_c else GRID, 1.6 if is_c else 1,
               dash=None if is_c else "2,5")
        s.text(gx, top - 12, f"{r:g}×", size=10, fill=(INK if is_c else FAINT),
               anchor="middle", mono=True)
    axy = top + len(rows) * (bh + bg) - bg + 22
    base = xlin(1.0)
    s.text((X0 + base) / 2, axy, "← slower than C", size=10, fill=FAINT,
           anchor="middle")
    s.text(base, axy, "C", size=10.5, fill=INK, anchor="middle", mono=True,
           weight="700")
    s.text((base + X1) / 2, axy, "faster than C →", size=10, fill=FAINT,
           anchor="middle")
    s.text(24, H - 14, "Measured: geomean speed vs C across 32 benchmarks "
           "(C time ÷ language time). Higher = faster; everyone runs the same "
           "programs.", size=9.5, fill=FAINT)

    y = top
    for name, val, col, note in rows:
        is_logos = "LOGOS" in name
        same = "same algorithm" in name
        s.text(200, y + bh - 7, name, size=12.5, anchor="end",
               weight="700" if is_logos else "500",
               fill=(CYAN if is_logos else INK))
        w = xlin(val) - X0
        if same:
            s.rect(X0, y, w, bh, "none", stroke=col, sw=1.6, rx=5)
            s.rect(X0, y, w, bh, col, rx=5, opacity=0.18)
        else:
            s.rect(X0, y, w, bh, s.grad(col), rx=5)
        s.text(xlin(val) + 8, y + bh - 7, f"{val:.2f}×", size=12.5,
               fill=(CYAN if is_logos else INK), mono=True, weight="700")
        if note:
            s.text(xlin(val) + 8, y + bh + 7, note, size=9.5, fill=FAINT)
        y += bh + bg
    return s.done(), {"logos_all": m["logos_release"],
                      "logos_apples": round(apples, 3)}


# ===========================================================================
# 3. Compiled LOGOS detail — collapses + same-algorithm parity vs C / Zig
# ===========================================================================
def chart_compiled():
    d = load("latest.json")
    summ = d["summary"]["geometric_mean_speedup_vs_c"]

    def at(b, lang):
        m = b["scaling"].get(b["reference_size"], {}).get(lang)
        return m and (m.get("mean_ms") or m.get("median_ms"))

    rc, rz = {}, {}
    for b in d["benchmarks"]:
        c, lo, z = at(b, "c"), at(b, "logos_release"), at(b, "zig")
        if c and lo:
            rc[b["id"]] = c / lo
            if z:
                rz[b["id"]] = c / z
    collapse = ["binary_trees", "ackermann", "fib", "collect", "loop_sum"]
    apples = geomean([r for k, r in rc.items() if k not in collapse])
    names = {"binary_trees": "Binary trees", "ackermann": "Ackermann",
             "fib": "Recursive fib", "collect": "Collection ops",
             "loop_sum": "Loop sum", "knapsack": "0/1 knapsack",
             "nqueens": "N-Queens", "primes": "Primes", "gcd": "GCD sum",
             "spectral_norm": "Spectral norm", "graph_bfs": "Graph BFS",
             "quicksort": "Quicksort", "mandelbrot": "Mandelbrot",
             "nbody": "N-body", "heap_sort": "Heap sort", "sieve": "Sieve",
             "fib_iterative": "Iterative fib"}
    parity = [b for b in ["knapsack", "nqueens", "primes", "gcd",
                          "spectral_norm", "graph_bfs", "quicksort",
                          "mandelbrot", "fib_iterative", "sieve", "nbody",
                          "heap_sort"]
              if 0.3 <= rc.get(b, 0) <= 2.7 and 0.3 <= rz.get(b, 0) <= 2.7]

    W = 880
    X0, X1 = 200, 715
    PLOTW = X1 - X0
    s = Svg(W, 706, "Compiled LOGOS vs C and Zig")
    s.header("Compiled LOGOS — the detail vs C and Zig",
             f"Two numbers: {summ['logos_release']:.2f}× C overall (collapses "
             f"included) vs {apples:.2f}× same-algorithm. Zig {summ['zig']:.2f}× "
             f"· Rust {summ['rust']:.2f}×.",
             kicker="HOW THE 2.54× BREAKS DOWN")

    # panel A: collapses (log)
    aY = 96
    s.panel(20, aY, W - 40, 26 + len(collapse) * 27, accent=CYAN)
    s.text(36, aY + 19, "Optimizer proves a shortcut C still runs  (5 of 32)",
           size=13, weight="700", fill=CYAN)
    aTop = aY + 36
    cmax = math.log10(max(rc[b] for b in collapse) * 1.4)

    def xlog(r):
        return X0 + (math.log10(max(r, 1)) / cmax) * PLOTW

    for v, lab in [(1, "1×"), (10, "10×"), (100, "100×"), (1000, "1000×")]:
        gx = xlog(v)
        s.line(gx, aTop - 3, gx, aTop + len(collapse) * 27 - 9, GRID, 1,
               dash="2,5")
        s.text(gx, aTop - 7, lab, size=9.5, fill=FAINT, anchor="middle",
               mono=True)
    y = aTop
    for b in sorted(collapse, key=lambda k: -rc[k]):
        r = rc[b]
        s.text(192, y + 13, names[b], size=12, anchor="end", fill=INK)
        s.rect(X0, y, xlog(r) - X0, 18, s.grad(CYAN), rx=4)
        s.text(xlog(r) + 7, y + 13, f"{r:,.0f}× C", size=11.5, fill=CYAN,
               mono=True, weight="700")
        y += 27

    # panel B: same-algorithm parity (linear, vs C and Zig)
    bY = y + 24
    ph = 30 + len(parity) * 22
    s.panel(20, bY, W - 40, ph, accent=INK)
    s.text(36, bY + 19, "Same algorithm as C — codegen parity", size=13,
           weight="700", fill=INK)
    for nm, col, ox in [("LOGOS", CYAN, 470), ("Zig", ZIG, 560)]:
        s.rect(ox, bY + 9, 11, 11, s.grad(col), rx=3)
        s.text(ox + 16, bY + 18, nm, size=11, fill=INK)
    lin_max = 2.2

    def xlin(r):
        return X0 + min(r, lin_max) / lin_max * PLOTW

    pTop = bY + 32
    base = xlin(1.0)
    s.line(base, pTop - 2, base, pTop + len(parity) * 22 - 4,
           LANG_COLOR["c"], 1.6)
    axy = pTop + len(parity) * 22 + 8
    s.text((X0 + base) / 2, axy, "← slower than C", size=9.5, fill=FAINT,
           anchor="middle")
    s.text(base, axy, "C", size=10, fill=INK, anchor="middle", mono=True,
           weight="700")
    s.text((base + X1) / 2, axy, "faster than C →", size=9.5, fill=FAINT,
           anchor="middle")
    y = pTop
    for b in parity:
        s.text(192, y + 16, names[b], size=11.5, anchor="end", fill=INK)
        for i, (r, col) in enumerate([(rc[b], CYAN), (rz[b], ZIG)]):
            yy = y + i * 9
            s.rect(X0, yy, xlin(r) - X0, 8, s.grad(col), rx=2)
        s.text(max(xlin(rc[b]), xlin(rz[b])) + 6, y + 13, f"{rc[b]:.2f}×",
               size=10, fill=CYAN, mono=True, weight="600")
        y += 22

    s.text(24, 696, "Speed relative to C at the reference size (higher = "
                    "faster). gcc -O3 -march=native -flto · 10 runs, 2 warmup.",
           size=9.5, fill=FAINT)
    return s.done(), {"geomean_vs_c": summ["logos_release"],
                      "same_algo_geomean": round(apples, 3),
                      "parity_n": len(parity)}


# ===========================================================================
# 4. Interpreted / JIT LOGOS vs V8 — eager VM and the JIT
# ===========================================================================
def chart_interp():
    d = load("latest-interp.json")
    startup = d["startup"]["engines"]

    def ref(b):
        return b["scaling"].get(b["reference_size"], {})

    # Per-benchmark eager-VM speed vs V8 (Node time / LOGOS time), median.
    eager = {}
    for b in d["benchmarks"]:
        m = ref(b)
        js, ea = m.get("js"), m.get("logos_interp")
        jm = js and (js.get("median_ms") or js.get("mean_ms"))
        if jm and ea:
            eager[b["id"]] = jm / (ea.get("median_ms") or ea["mean_ms"])

    # Headline geomean the way the page computes it (interp_speed_vs_v8): mean_ms,
    # skipping benchmarks where Node sits on its ~30ms startup floor (js mean <
    # 60ms). The JSON's `summary` field disagrees with its own per-benchmark data,
    # so we recompute from the data, like the page does.
    ea_off = []
    for b in d["benchmarks"]:
        m = ref(b)
        js, ea = m.get("js"), m.get("logos_interp")
        if js and js["mean_ms"] >= 60 and ea and ea["mean_ms"] > 0:
            ea_off.append(js["mean_ms"] / ea["mean_ms"])
    eager_geo = geomean(ea_off)

    names = {"collatz": "Collatz", "counting_sort": "Counting sort",
             "string_search": "String search", "array_fill": "Array fill",
             "strings": "String assembly", "two_sum": "Two-sum",
             "prefix_sum": "Prefix sum", "fib_iterative": "Iterative fib",
             "array_reverse": "Array reverse", "fib": "Recursive fib",
             "binary_trees": "Binary trees", "sieve": "Sieve",
             "collect": "Collection ops", "quicksort": "Quicksort",
             "nbody": "N-body"}
    show = [b for b in ["counting_sort", "collatz", "strings", "array_fill",
                        "two_sum", "string_search", "prefix_sum",
                        "fib_iterative", "array_reverse", "fib", "binary_trees",
                        "sieve", "collect", "quicksort", "nbody"]
            if b in eager]
    show.sort(key=lambda b: -eager[b])

    W, X0, X1 = 880, 200, 700
    PLOTW = X1 - X0
    top = 102
    rowh = 23
    H = top + len(show) * rowh + 132
    s = Svg(W, H, "Interpreted LOGOS vs V8")
    s.header("Interpreted LOGOS — vs V8 (Node)",
             f"Eager VM {eager_geo:.2f}× V8 (geomean) — a from-scratch bytecode "
             f"interpreter, same naive program on both. Higher = faster.",
             kicker="ENGLISH, INTERPRETED, FASTER THAN V8")

    lin_max = 3.5

    def xlin(r):
        return X0 + min(r, lin_max) / lin_max * PLOTW

    for r in (1, 2, 3):
        gx = xlin(r)
        is1 = r == 1
        s.line(gx, top - 8, gx, top + len(show) * rowh - 4,
               LANG_COLOR["c"] if is1 else GRID, 1.6 if is1 else 1,
               dash=None if is1 else "2,5")
        s.text(gx, top - 12, f"{r}×", size=10, fill=(INK if is1 else FAINT),
               anchor="middle", mono=True)
    axy = top + len(show) * rowh + 8
    bx = xlin(1.0)
    s.text((X0 + bx) / 2, axy, "← slower than V8", size=10, fill=FAINT,
           anchor="middle")
    s.text(bx, axy, "V8", size=10.5, fill=INK, anchor="middle", mono=True,
           weight="700")
    s.text((bx + X1) / 2, axy, "faster than V8 →", size=10, fill=FAINT,
           anchor="middle")

    y = top
    for b in show:
        s.text(192, y + 12, names[b], size=12, anchor="end", fill=INK)
        s.rect(X0, y, xlin(eager[b]) - X0, 14, s.grad(CYAN), rx=3)
        s.text(xlin(eager[b]) + 6, y + 12, f"{eager[b]:.2f}×", size=10.5,
               fill=CYAN, mono=True, weight="700")
        y += rowh

    # cold-start panel
    cy = y + 44
    lo_ms = startup["logos_interp"]["median_ms"]
    js_ms = startup["js"]["median_ms"]
    s.panel(20, cy - 20, W - 40, 74, accent=CYAN)
    s.text(36, cy, f"Cold start — first output {js_ms/lo_ms:.1f}× faster than "
                   f"Node", size=13, weight="700", fill=CYAN)
    smax = max(lo_ms, js_ms) * 1.2
    bx0 = 200
    for i, (lab, ms, col) in enumerate([("LOGOS", lo_ms, CYAN),
                                        ("Node", js_ms, ORANGE)]):
        by = cy + 12 + i * 18
        s.text(192, by + 10, lab, size=12, anchor="end", fill=INK)
        w = (ms / smax) * (X1 - bx0)
        s.rect(bx0, by, w, 13, s.grad(col), rx=3)
        s.text(bx0 + w + 7, by + 10, f"{ms:.1f}ms", size=10.5, fill=col,
               mono=True, weight="600")
    s.text(24, H - 13, "Speed = Node time ÷ LOGOS time, off-floor geomean "
           "(Node off its ~30ms startup floor). Same naive algorithm, no manual "
           "tuning. i9-14900K, 10 runs.", size=9.5, fill=FAINT)
    return s.done(), {"eager_vs_v8": round(eager_geo, 3),
                      "coldstart_x": round(js_ms / lo_ms, 2)}


FAIR_NAME = {"ints": "Int array", "floats": "Float array",
             "timeseries": "Timeseries", "points": "Point cloud",
             "records": "Records", "strings": "Strings"}


def codec_fair(d):
    out = []
    for sc in d["scenarios"]:
        if sc.get("kind") == "fair":
            out.append(sc)
    return out


# ===========================================================================
# 5. Codec — wire size vs the field
# ===========================================================================
def chart_codec_size():
    d = load("latest-codec.json")
    rows, ratios = [], []
    for sc in codec_fair(d):
        lo = min((r for r in sc["rows"] if r["codec"].startswith("logos")),
                 key=lambda r: r["size"])
        ot = min((r for r in sc["rows"] if not r["codec"].startswith("logos")),
                 key=lambda r: r["size"])
        rows.append((sc["id"], lo, ot))
        ratios.append(ot["size"] / lo["size"])
    geo = geomean(ratios)
    wins = sum(1 for _, lo, ot in rows if lo["size"] <= ot["size"])
    olbl = {"protobuf/grpc": "protobuf", "messagepack": "msgpack",
            "capnproto": "capnp", "arrow (ipc)": "arrow"}

    W, X0, X1 = 880, 150, 700
    PLOTW = X1 - X0
    top = 100
    blk = 2 * 13 + 3 + 17
    H = top + len(rows) * blk + 28
    s = Svg(W, H, "LOGOS wire size vs the field")
    s.header("Wire codec — encoded size vs the field",
             f"Smallest wire on {wins}/{len(rows)} fair workloads · geomean "
             f"{geo:.2f}× smaller than the best of each.",
             kicker="SMALLEST PAYLOAD")
    s.text(40, 75, "vs Cap’n Proto, Protobuf, MessagePack, bincode, postcard, "
                   "CBOR, Arrow, JSON.", size=11, fill=FAINT)
    for nm, col, ox in [("LOGOS", CYAN, 470), ("best of the rest", GRAY, 555)]:
        s.rect(ox, 12, 11, 11, s.grad(col), rx=3)
        s.text(ox + 16, 21, nm, size=11, fill=INK)

    y = top
    for sid, lo, ot in rows:
        rmax = max(lo["size"], ot["size"]) * 1.16
        s.text(140, y + 15, FAIR_NAME[sid], size=13, anchor="end",
               weight="700", fill=INK)
        oc = olbl.get(ot["codec"], ot["codec"])
        for i, (row, col, tag) in enumerate([
                (lo, CYAN, f"{fmt_bytes(lo['size'])}  — "
                           f"{ot['size']/lo['size']:.2f}× smaller"),
                (ot, GRAY, f"{fmt_bytes(ot['size'])}  ({oc})")]):
            by = y + i * 16
            w = (row["size"] / rmax) * PLOTW
            s.rect(X0, by, w, 13, s.grad(col), rx=3)
            s.text(X0 + w + 7, by + 11, tag, size=10.5,
                   fill=(CYAN if i == 0 else DIM), mono=True,
                   weight="600" if i == 0 else None)
        y += blk
    s.text(24, H - 11, "Random data per workload · encoded payload bytes "
                       "(lower = smaller), normalized per row · LOGOS at its "
                       "smallest dial vs each rival’s smallest.", size=9.5,
                       fill=FAINT)
    return s.done(), {"fair_wins": wins, "fair_total": len(rows),
                      "geomean_smaller": round(geo, 3)}


# ===========================================================================
# 6. Codec — encode / decode speed + random access
# ===========================================================================
def chart_codec_speed():
    d = load("latest-codec.json")
    fair = codec_fair(d)

    def fastest_logos(rows, k):
        return min((r for r in rows if r["codec"].startswith("logos")),
                   key=lambda r: r[k])

    def best_other(rows, k):
        return min((r for r in rows if not r["codec"].startswith("logos")),
                   key=lambda r: r[k])

    enc, dec = [], []
    for sc in fair:
        le, oe = fastest_logos(sc["rows"], "enc_ns"), best_other(sc["rows"], "enc_ns")
        ld, od = fastest_logos(sc["rows"], "dec_ns"), best_other(sc["rows"], "dec_ns")
        enc.append((sc["id"], oe["enc_ns"] / le["enc_ns"], oe["codec"]))
        dec.append((sc["id"], od["dec_ns"] / ld["dec_ns"], od["codec"]))

    # random access
    ra = next(sc for sc in d["scenarios"] if sc.get("kind") == "random_access")
    ra_rows = []
    for nm in ["logos (struct-view)", "capnproto", "arrow (ipc)", "bincode",
               "json"]:
        r = next((x for x in ra["rows"] if x["codec"] == nm and x.get("read_one_ns")), None)
        if r:
            ra_rows.append((nm, r["read_one_ns"]))

    W = 880
    X0, X1 = 200, 700
    PLOTW = X1 - X0
    top = 100
    s = Svg(W, 600, "LOGOS codec encode/decode speed and random access")
    s.header("Wire codec — encode / decode speed + random access",
             "LOGOS’s fast dial (fixed/varint) — size↔speed is a knob.",
             kicker="AND IT’S FAST")
    s.text(40, 75, "Bars show × faster than the best competitor on each "
                   "workload (lower ns = faster).", size=11, fill=FAINT)
    olbl = {"protobuf/grpc": "protobuf", "messagepack": "msgpack",
            "capnproto": "capnp", "arrow (ipc)": "arrow"}

    def speed_panel(py, title, data, accent):
        s.panel(20, py, W - 40, 28 + len(data) * 20, accent=accent)
        s.text(36, py + 19, title, size=13, weight="700", fill=accent)
        mx = max(v for _, v, _ in data) * 1.18

        def xf(v):
            return X0 + v / mx * PLOTW
        bt = py + 32
        base = xf(1.0)
        s.line(base, bt - 2, base, bt + len(data) * 20 - 6, LANG_COLOR["c"], 1.4)
        y = bt
        for sid, v, ocodec in data:
            s.text(192, y + 12, FAIR_NAME[sid], size=11.5, anchor="end", fill=INK)
            s.rect(X0, y, xf(v) - X0, 13, s.grad(accent), rx=3)
            s.text(xf(v) + 7, y + 11,
                   f"{v:.1f}× faster than {olbl.get(ocodec, ocodec)}",
                   size=10, fill=accent, mono=True, weight="600")
            y += 20
        return py + 28 + len(data) * 20

    y = speed_panel(top, "Encode — throughput vs the best competitor", enc, CYAN)
    y = speed_panel(y + 16, "Decode — throughput vs the best competitor", dec, "#34d399")

    # random access (log)
    ry = y + 16
    s.panel(20, ry, W - 40, 30 + len(ra_rows) * 19, accent=PURPLE)
    s.text(36, ry + 19, "Random-access single-field read (open + read one) — "
                        "log scale", size=13, weight="700", fill=PURPLE)
    rlo, rhi = math.log10(8), math.log10(300000)

    def xlog(ns):
        return X0 + (math.log10(max(ns, 8)) - rlo) / (rhi - rlo) * PLOTW
    rt = ry + 32
    yy = rt
    for nm, ns in ra_rows:
        is_lo = nm.startswith("logos")
        disp = "LOGOS (struct-view)" if is_lo else olbl.get(nm, nm)
        col = CYAN if is_lo else (PURPLE if nm == "capnproto" else GRAY)
        s.text(192, yy + 12, disp, size=11.5, anchor="end",
               fill=(CYAN if is_lo else INK),
               weight="700" if is_lo else None)
        s.rect(X0, yy, xlog(ns) - X0, 13, s.grad(col), rx=3)
        s.text(xlog(ns) + 7, yy + 11, fmt_ns(ns), size=10, fill=col,
               mono=True, weight="600")
        yy += 19
    s.text(24, 586, f"Random data · ns/op over {d.get('iters', 14000):,} "
           f"iterations (lower = faster) · LOGOS’s fast fixed/varint dial; its "
           f"smallest-size dial is slower — see the leaderboard.", size=9.5,
           fill=FAINT)
    return s.done(), {"enc_max": round(max(v for _, v, _ in enc), 2),
                      "dec_max": round(max(v for _, v, _ in dec), 2),
                      "ra_logos_ns": ra_rows[0][1]}


# ===========================================================================
# 7. Codec — full leaderboard (one workload, every codec, all three metrics)
# ===========================================================================
def chart_codec_board():
    d = load("latest-codec.json")
    iters = d.get("iters", 14000)
    sc = next(s for s in d["scenarios"] if s["id"] == "records")
    rows = sorted(sc["rows"], key=lambda r: r["size"])
    disp = {"logos (BEST: all knobs)": "LOGOS · all knobs",
            "logos (varint)": "LOGOS · varint", "logos (fixed)": "LOGOS · fixed",
            "protobuf/grpc": "protobuf", "messagepack": "msgpack",
            "capnproto": "capnp", "arrow (ipc)": "arrow"}
    cols = [("Size", "size", "B"), ("Encode", "enc_ns", "ns"),
            ("Decode", "dec_ns", "ns")]
    col_x = [212, 442, 672]
    barw = 104
    best = {k: min(r[k] for r in rows) for _, k, _ in cols}

    W = 880
    top = 126
    rh = 23
    H = top + len(rows) * rh + 54
    s = Svg(W, H, "Wire codec full leaderboard")
    s.header("Wire codec — full leaderboard (one workload, every codec)",
             "Every codec on the same data, all three metrics together. "
             "Lower = better in every column.",
             kicker="ALL CODECS · SIZE · ENCODE · DECODE")
    s.text(40, 80, "200 random records {id, name, active}.", size=11, fill=FAINT)
    for (label, key, unit), cx in zip(cols, col_x):
        s.text(cx, top - 11, f"{label} ({unit}) — lower better", size=10.5,
               fill=DIM, weight="600")
    y = top
    for r in rows:
        is_lo = r["codec"].startswith("logos")
        if is_lo:
            s.rect(12, y - 1, W - 24, rh - 2, CYAN, rx=5, opacity=0.06)
        s.text(196, y + 14, disp.get(r["codec"], r["codec"]), size=12,
               anchor="end", fill=(CYAN if is_lo else INK),
               weight="700" if is_lo else None)
        for (label, key, unit), cx in zip(cols, col_x):
            v = r[key]
            vals = [rr[key] for rr in rows]
            lo = math.log10(min(vals) * 0.85)
            hi = math.log10(max(vals) * 1.12)
            w = max(0.05, (math.log10(v) - lo) / (hi - lo)) * barw
            is_best = v == best[key]
            col = GREEN if is_best else (CYAN if is_lo else GRAY)
            s.rect(cx, y + 3, w, 12, s.grad(col), rx=3)
            val = (f"{v:,.0f}" if unit == "B"
                   else (f"{v/1000:.1f}µs" if v >= 1000 else f"{v:.0f}ns"))
            s.text(cx + w + 6, y + 13, val + (" ★" if is_best else ""),
                   size=9.5, fill=col, mono=True,
                   weight="700" if is_best else "600")
        y += rh
    s.text(24, H - 13, f"★ = best in column · random data · {iters:,} iterations "
           f"· ns/op. LOGOS dials trade size vs speed: ‘fixed/varint’ are fast, "
           f"‘all knobs’ is smallest. i9-14900K.", size=9.5, fill=FAINT)
    return s.done(), {"codecs": len(rows)}


# ===========================================================================
# 1b. SAT arena — every loaded instance across 20 SATLIB families
# ===========================================================================
def chart_sat_arena():
    d = load("arena/sat.json")
    summ = d["summary"]
    cats = d["categories"]
    timeout = d.get("timeout_s", 20)
    total = len(d["instances"])
    order = sorted(cats.items(),
                   key=lambda kv: (kv[1]["ours"] - max(kv[1]["kissat"],
                                                       kv[1]["cadical"]),
                                   kv[1]["ours"]),
                   reverse=True)

    W = 880
    X0, X1 = 190, 760
    trackW = X1 - X0
    top = 180
    rh = 25
    H = top + len(order) * rh + 44
    s = Svg(W, H, "SAT arena — 133 loaded instances")
    s.header(f"SAT arena — {total} loaded instances, {len(cats)} SATLIB families",
             f"Ours vs Kissat & CaDiCaL at a {timeout}s timeout. We solve the "
             f"most, at the lowest PAR-2, and machine-check every UNSAT.",
             kicker="SATLIB · BROAD COVERAGE")
    # scoreboard
    s.panel(20, 84, W - 40, 74)
    cw = (W - 40) / 3
    for i, (nm, col, key) in enumerate([("LOGOS (ours)", CYAN, "ours"),
                                        ("Kissat", ORANGE, "kissat"),
                                        ("CaDiCaL", GRAY, "cadical")]):
        st = summ[key]
        cx = 34 + i * cw
        s.rect(cx, 98, 11, 11, s.grad(col), rx=3)
        s.text(cx + 16, 107, nm, size=12.5, weight="700", fill=col)
        s.text(cx, 132, f"{st['solved']}/{total}", size=18, weight="800",
               fill=INK, mono=True)
        s.text(cx + 88, 132, "solved", size=11, fill=DIM)
        s.text(cx, 150, f"PAR-2 {st['par2']:.0f}  ·  {st['verified']} proofs"
                        f"{' ✓' if st['verified'] else ''}", size=10.5, fill=DIM)

    s.text(X0, top - 10, "fraction of each family solved within "
                         f"{timeout}s — longer = more solved", size=10,
           fill=FAINT)
    y = top
    for name, c in order:
        n = c["count"]
        s.text(180, y + 13, name.replace("_", " · ", 1), size=11,
               anchor="end", fill=INK)
        s.rect(X0, y, trackW, 20, "rgba(255,255,255,0.035)", rx=3)
        for j, (key, col) in enumerate([("ours", CYAN), ("kissat", ORANGE),
                                        ("cadical", GRAY)]):
            frac = (c[key] / n) if n else 0
            s.rect(X0, y + 1 + j * 6, frac * trackW, 5, s.grad(col), rx=2)
        s.text(X1 + 8, y + 13,
               f"{c['ours']}/{c['kissat']}/{c['cadical']} of {n}", size=9.5,
               fill=DIM, mono=True)
        y += rh
    s.text(24, H - 13, "Bars = ours / Kissat / CaDiCaL solved per family "
           "(ordered by our margin). Lower PAR-2 = better; ‘proofs ✓’ = UNSATs "
           "we machine-check, the others emit none.", size=9.5, fill=FAINT)
    return s.done(), {"ours_solved": summ["ours"]["solved"],
                      "kissat_solved": summ["kissat"]["solved"],
                      "cadical_solved": summ["cadical"]["solved"],
                      "families": len(cats), "instances": total}


CHARTS_DEF = [
    ("sat-solvers.svg", chart_sat),
    ("sat-arena.svg", chart_sat_arena),
    ("overall-vs-c.svg", chart_overall),
    ("compiled-vs-c.svg", chart_compiled),
    ("interp-vs-v8.svg", chart_interp),
    ("codec-board.svg", chart_codec_board),
    ("codec-size.svg", chart_codec_size),
    ("codec-speed.svg", chart_codec_speed),
]

BANNED = ("<script", "<animate", "<foreignobject", "<filter", "onload",
          "javascript:")

EXPECT = {
    "sat-solvers.svg": ["Tseitin parity", "n = 110", "Pigeonhole PHP",
                        "timeout", "Random 3-SAT"],
    "sat-arena.svg": ["SAT arena", "solved", "PAR-2", "CaDiCaL", "proofs"],
    "overall-vs-c.svg": ["LOGOS — all 32", "same algorithm", "slower than C",
                         "faster than C", "Rust", "Zig"],
    "compiled-vs-c.svg": ["Binary trees", "× C", "slower than C",
                          "same-algorithm"],
    "interp-vs-v8.svg": ["slower than V8", "faster than V8", "Cold start",
                         "Collatz", "Eager VM"],
    "codec-board.svg": ["all three metrics", "random data", "★",
                        "LOGOS · fixed"],
    "codec-size.svg": ["fair workloads", "smaller", "Timeseries"],
    "codec-speed.svg": ["Encode", "Decode", "Random-access", "faster"],
}


def main():
    check = "--check" in sys.argv
    os.makedirs(CHARTS, exist_ok=True)
    summary = {}
    for fname, fn in CHARTS_DEF:
        svg, meta = fn()
        path = os.path.join(CHARTS, fname)
        with open(path, "w", encoding="utf-8") as f:
            f.write(svg)
        summary[fname] = meta
        low = svg.lower()
        for bad in BANNED:
            assert bad not in low, f"{fname}: contains banned token {bad}"
        minidom.parseString(svg.encode("utf-8"))
        for want in EXPECT.get(fname, []):
            assert want in svg, f"{fname}: expected text {want!r} missing"
        print(f"wrote {os.path.relpath(path, HERE)}  ({len(svg):,} bytes)  {meta}")

    if check:
        print("\n[check] all SVGs well-formed UTF-8 XML, no stripped elements, "
              "expected labels present.")
        for k, v in summary.items():
            print(f"   {k}: {v}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
