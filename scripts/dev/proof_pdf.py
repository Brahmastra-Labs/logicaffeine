"""proof_pdf.py — render a LOGOS derivation IR (derivation.json) to a beautiful PDF.

    python3 scripts/dev/proof_pdf.py  IN.json  OUT.pdf     # render a derivation document
    python3 scripts/dev/proof_pdf.py  --selftest           # render an embedded fixture + assert

This is the self-contained PDF backend of the two the engine emits (the other is a
`.tex` file rendered by LaTeX). It consumes the SAME JSON the Rust proof engine
writes, so the JSON is the durable artifact and this file is a pure view over it.

────────────────────────────────────────────────────────────────────────────────
IR SCHEMA (every math value carries both `latex`, consumed by the .tex backend,
and `unicode`, consumed here via the base-14 Symbol font):

  document      = { title, subtitle?, meta?: [[k,v]...], sections: [section...] }
  section       = { heading, blocks: [block...] }
  math          = { latex, unicode }
  block.kind ∈
    "prose"       { spans: [ {t:"text", s} | {t:"math", latex, unicode} ... ] }
    "display"     math                              # a centered display formula
    "derivation"  { node }                          # a Gentzen inference tree
        node      = { conclusion: math, rule: {latex,text}, premises: [node...],
                      discharge?: "1" }
    "steps"       { steps: [step...] }              # a numbered Fitch derivation
        step      = { n, formula: math, rule: {latex,text}, refs: [n...],
                      depth, assumption? }
    "certificate" { kind, statement: math, lines: [ {label:math, value:math,
                      note?} ], conclusion }        # an algebraic identity cert
    "reduction"   { steps: [ {claim:math, reduces_to:math, status:"proven"|"open",
                      note?} ], verdict? }          # a backward-derivation MAP
    "atoms"       { atoms: [ {kind:"grounded", text} |
                      {kind:"gap", what, missing} ] }   # proven vs open frontier
    "table"       { headers: [str...], rows: [[math...]...] }
    "note"        { text }
────────────────────────────────────────────────────────────────────────────────
"""

import sys
import os
import json

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _pdfmini as pm  # noqa: E402

CAP = 0.62  # cap-height as a fraction of the font size (for tree vertical spacing)


class ProofDoc(pm.Doc):
    """A base-14 PDF Doc taught to render the proof-IR block set."""

    def __init__(self, **kw):
        super().__init__(**kw)
        self.green = (0.13, 0.46, 0.24)
        self.amber = (0.66, 0.44, 0.06)

    # ---- inline mixed text+math paragraph, word-wrapped by measured width ----
    def para_math(self, x0, s, size, maxw, leading, color, base="R"):
        words = s.split(" ")
        sw = self.char_w(" ", "R", size)
        self.ensure(leading)
        baseline = self.pen_y - pm.ASC * size
        x, start = x0, True
        for w in words:
            if w == "":
                continue
            ww = self.math_w(w, size, base)
            if not start and x + sw + ww > x0 + maxw:
                self.pen_y -= leading
                self.ensure(leading)
                baseline = self.pen_y - pm.ASC * size
                x, start = x0, True
            if not start:
                x += sw
            self.draw_math(x, baseline, w, size, color, base)
            x += ww
            start = False
        self.pen_y -= leading

    # =====================================================================
    #  the document frame
    # =====================================================================
    def title_block(self, doc):
        self.gap(6)
        self.line(self.ml, doc["title"], "B", 17.0, 22.0, self.accent)
        if doc.get("subtitle"):
            self.para_math(self.ml, doc["subtitle"], 10.5, self.cw, 15.0, self.gray)
        meta = doc.get("meta") or []
        if meta:
            self.gap(2)
            for k, v in meta:
                self.ensure(12.5)
                self.draw_text(self.ml, self.pen_y - pm.ASC * 8.6, k.upper(),
                               "B", 8.6, self.gray)
                self.para_math(self.ml + 96, v, 9.4, self.cw - 96, 12.5, self.ink)
                self.gap(1)
        self.gap(4)
        self.hline(self.ml, self.PW - self.mr, self.pen_y, self.hair, 0.8)
        self.gap(6)

    def render(self, doc):
        self.title_block(doc)
        for sec in doc["sections"]:
            self.section(sec["heading"])
            for blk in sec["blocks"]:
                self.block(blk)

    def block(self, blk):
        k = blk["kind"]
        fn = getattr(self, "blk_" + k, None)
        if fn is None:
            self.blk_note({"text": "[unknown block: %s]" % k})
        else:
            fn(blk)
        self.gap(4)

    # =====================================================================
    #  leaf blocks
    # =====================================================================
    def blk_note(self, blk):
        self.para_math(self.ml, blk["text"], 9.4, self.cw, 13.2, self.gray)

    def blk_prose(self, blk):
        s = "".join(sp["s"] if sp["t"] == "text" else sp["unicode"]
                    for sp in blk["spans"])
        self.para_math(self.ml, s, 10.0, self.cw, 14.0, self.ink)

    def blk_display(self, blk):
        self.gap(3)
        self.display_math(blk["unicode"], 12.5)
        self.gap(3)

    def blk_table(self, blk):
        heads = blk["headers"]
        rows = blk["rows"]
        n = len(heads)
        colw = self.cw / n
        size = 9.6
        self.ensure(16)
        base = self.pen_y - pm.ASC * size
        for j, h in enumerate(heads):
            self.draw_math(self.ml + j * colw, base, h, size, self.accent)
        self.pen_y -= 14
        self.hline(self.ml, self.PW - self.mr, self.pen_y + 3, self.hair, 0.6)
        self.gap(3)
        for row in rows:
            self.ensure(14)
            base = self.pen_y - pm.ASC * size
            for j, cell in enumerate(row):
                self.draw_math(self.ml + j * colw, base, cell["unicode"], size, self.ink)
            self.pen_y -= 13
            self.hline(self.ml, self.PW - self.mr, self.pen_y + 2, (0.90, 0.90, 0.92), 0.4)

    # =====================================================================
    #  numbered natural-deduction (Fitch) derivation
    # =====================================================================
    def blk_steps(self, blk):
        size = 9.8
        numw = 22.0
        scope = 12.0     # indent per assumption depth
        just_x = self.PW - self.mr - 150.0
        for st in blk["steps"]:
            self.ensure(14.5)
            base = self.pen_y - pm.ASC * size
            self.draw_text(self.ml, base, "(%d)" % st["n"], "B", size, self.accent)
            depth = st.get("depth", 0)
            fx = self.ml + numw + depth * scope
            # a vertical scope rule for each open assumption depth
            for dch in range(depth):
                gx = self.ml + numw + dch * scope - 3
                self.seg(gx, base - 2, gx, base + CAP * size + 2, self.hair, 0.5)
            if st.get("assumption"):
                self.draw_text(fx, base, "assume", "O", size - 0.6, self.amber)
                fx += self.text_w("assume ", "O", size - 0.6)
            self.draw_math(fx, base, st["formula"]["unicode"], size, self.ink)
            # justification, right-aligned column
            rule = st.get("rule", {}).get("text", "")
            refs = st.get("refs", [])
            just = rule + ((" " + ",".join(str(r) for r in refs)) if refs else "")
            self.draw_math(just_x, base, just, size - 0.8, self.gray)
            self.pen_y -= 14.5

    # =====================================================================
    #  Gentzen inference tree  (the beautiful centerpiece)
    # =====================================================================
    def _measure(self, node, size):
        concl = node["conclusion"]["unicode"]
        cw = self.math_w(concl, size)
        rule = node.get("rule", {}).get("text", "")
        rw = (self.math_w(rule, size - 2.5) + 6.0) if rule else 0.0
        prem = node.get("premises") or []
        if not prem:
            w = cw
            return {"w": w, "cw": cw, "concl": concl, "rule": rule, "rw": rw,
                    "prem": [], "depth": 1, "discharge": node.get("discharge")}
        ms = [self._measure(p, size) for p in prem]
        gap = 16.0
        row_w = sum(m["w"] for m in ms) + gap * (len(ms) - 1)
        barlen = max(row_w, cw)
        w = barlen + rw
        depth = 1 + max(m["depth"] for m in ms)
        return {"w": w, "cw": cw, "concl": concl, "rule": rule, "rw": rw,
                "prem": ms, "row_w": row_w, "barlen": barlen, "gap": gap,
                "depth": depth, "discharge": node.get("discharge")}

    def _draw_node(self, m, x, yb, size, level):
        """Draw a measured node; conclusion baseline at yb, block occupying [x, x+m['w']]."""
        barlen = m.get("barlen", m["cw"])
        cx = x + barlen / 2.0          # centre of the inference bar / conclusion
        # conclusion, centred under the bar
        self.draw_math(cx - m["cw"] / 2.0, yb, m["concl"], size, self.ink)
        if not m["prem"]:
            return
        bar_y = yb + CAP * size + 4.0
        self.hline(x, x + barlen, bar_y, self.ink, 0.7)
        if m["rule"]:
            self.draw_math(x + barlen + 4.0, bar_y - (size - 2.5) * 0.30,
                           m["rule"], size - 2.5, self.accent)
        if m.get("discharge"):
            self.draw_text(x - 9.0, yb, "[%s]" % m["discharge"], "O", size - 2.5,
                           self.amber)
        prem_yb = yb + level
        px = cx - m["row_w"] / 2.0
        for pm_ in m["prem"]:
            self._draw_node(pm_, px, prem_yb, size, level)
            px += pm_["w"] + m["gap"]

    def blk_derivation(self, blk):
        node = blk["node"]
        size = 10.5
        m = self._measure(node, size)
        # auto-fit width: shrink the font until the tree fits the text column
        if m["w"] > self.cw:
            size = max(7.0, size * self.cw / m["w"])
            m = self._measure(node, size)
        level = size + 16.0
        total_h = m["depth"] * level
        self.ensure(total_h + 8)
        self.gap(6)
        yb_root = self.pen_y - total_h + (level - CAP * size)
        x_root = self.ml + max(0.0, (self.cw - m["w"]) / 2.0)
        self._draw_node(m, x_root, yb_root, size, level)
        self.pen_y -= total_h + 4

    # =====================================================================
    #  algebraic certificate (Nullstellensatz / Farkas / DRAT identity)
    # =====================================================================
    def blk_certificate(self, blk):
        size = 10.0
        self.subhead(blk.get("cert_kind", "Certificate"))
        self.gap(2)
        self.display_math(blk["statement"]["unicode"], 12.0)
        self.gap(2)
        notecol = 118.0    # reserved right column for the per-line note (the clause)
        for ln in blk["lines"]:
            self.ensure(14)
            base = self.pen_y - pm.ASC * size
            lw = self.draw_math(self.ml + 12, base, ln["label"]["unicode"], size, self.ink)
            self.draw_text(lw + 5, base, "=", "R", size, self.gray)
            vx = lw + 5 + self.char_w("=", "R", size) + 5
            if ln.get("note"):
                self.draw_math(self.PW - self.mr - notecol, base, ln["note"], size - 1.3,
                               self.gray)
            # the value (polynomial) wraps within the column left of the note
            maxw = (self.PW - self.mr - notecol - 8) - vx
            self.para_math(vx, ln["value"]["unicode"], size, maxw, 13.5, self.ink)
            self.gap(1)
        self.gap(2)
        self.ensure(15)
        self.draw_math(self.ml + 12, self.pen_y - pm.ASC * (size + 0.5),
                       "**" + blk["conclusion"] + "**", size + 0.5, self.green)
        self.pen_y -= 15

    # =====================================================================
    #  reduction MAP (backward_deriver: claim ⟸ reduces_to, tagged proven/open)
    # =====================================================================
    def blk_reduction(self, blk):
        size = 10.2
        tagcol = 60.0    # reserved right column for the proven/OPEN chip
        for st in blk["steps"]:
            self.ensure(16)
            proven = st["status"] == "proven"
            col = self.green if proven else self.red
            tag = "proven" if proven else "OPEN"
            top = self.pen_y - pm.ASC * size
            self.draw_text(self.ml, top, "+" if proven else "!", "B", size, col)
            self.draw_text(self.PW - self.mr - self.text_w(tag, "B", 8.4),
                           top + CAP * size - 1, tag, "B", 8.4, col)
            # claim ⇐ reduces_to, wrapped within the text column (clear of the chip)
            combined = st["claim"]["unicode"] + "   ⇐   " + st["reduces_to"]["unicode"]
            self.para_math(self.ml + 14, combined, size, self.cw - 14 - tagcol, 14.0, self.ink)
            if st.get("note"):
                self.para_math(self.ml + 14, st["note"], 8.8, self.cw - 14, 12.0, self.gray)
            self.gap(2)
        if blk.get("verdict"):
            self.gap(3)
            self.para_math(self.ml, "**Verdict.** " + blk["verdict"], 9.8, self.cw, 13.5,
                           self.ink)

    # =====================================================================
    #  atom ledger (grounded facts vs. named capability gaps / open frontier)
    # =====================================================================
    def blk_atoms(self, blk):
        size = 9.6
        for a in blk["atoms"]:
            self.ensure(14)
            base = self.pen_y - pm.ASC * size
            if a["kind"] == "grounded":
                self.draw_text(self.ml, base, "+", "B", size, self.green)
                self.para_math(self.ml + 14, a["text"], size, self.cw - 14, 13.2, self.ink)
            else:
                self.draw_text(self.ml, base, "!", "B", size, self.red)
                self.para_math(self.ml + 14, "**" + a["what"] + "** — missing: " + a["missing"],
                               size, self.cw - 14, 13.2, self.ink)
            self.gap(2)


# ─────────────────────────────────────────────────────────────────────────────
def render_document(doc, out_path):
    d = ProofDoc()
    d.render(doc)
    data = d.build()
    with open(out_path, "wb") as f:
        f.write(data)
    return d, data


def _fixture():
    """A representative document exercising every block: a formal FOL proof (tree +
    Fitch), an algebraic certificate, and — pointed at the RH research modules — an
    honest reduction MAP + atom ledger. Also the source of truth for --selftest."""
    def M(u, l=None):
        return {"latex": l or u, "unicode": u}
    return {
        "title": "A LOGOS Derivation",
        "subtitle": "Machine-found, kernel-checked — typeset from the proof engine's derivation IR.",
        "meta": [["Engine", "LOGOS backward chainer"],
                 ["Status", "kernel-verified"],
                 ["Backend", "self-contained base-14 (Symbol font)"]],
        "sections": [
            {"heading": "Theorem", "blocks": [
                {"kind": "prose", "spans": [
                    {"t": "text", "s": "From "},
                    {"t": "math", "unicode": "∀x (P(x) → Q(x))", "latex": r"\forall x\,(P(x)\to Q(x))"},
                    {"t": "text", "s": " and "},
                    {"t": "math", "unicode": "P(a)", "latex": "P(a)"},
                    {"t": "text", "s": " the engine derives "},
                    {"t": "math", "unicode": "Q(a)", "latex": "Q(a)"},
                    {"t": "text", "s": "."}]},
                {"kind": "display", "unicode": "∀x (P(x) → Q(x)),  P(a)  ⊢  Q(a)",
                 "latex": r"\forall x\,(P(x)\to Q(x)),\ P(a)\ \vdash\ Q(a)"}]},
            {"heading": "Derivation (natural deduction)", "blocks": [
                {"kind": "derivation", "node": {
                    "conclusion": M("Q(a)"),
                    "rule": {"text": "→E", "latex": r"\to E"},
                    "premises": [
                        {"conclusion": M("P(a) → Q(a)"),
                         "rule": {"text": "∀E", "latex": r"\forall E"},
                         "premises": [{"conclusion": M("∀x (P(x) → Q(x))"),
                                       "rule": {"text": "prem", "latex": "prem"},
                                       "premises": []}]},
                        {"conclusion": M("P(a)"),
                         "rule": {"text": "prem", "latex": "prem"}, "premises": []}]}}]},
            {"heading": "The same proof, as numbered lines", "blocks": [
                {"kind": "steps", "steps": [
                    {"n": 1, "formula": M("∀x (P(x) → Q(x))"), "rule": {"text": "premise"},
                     "refs": [], "depth": 0},
                    {"n": 2, "formula": M("P(a)"), "rule": {"text": "premise"}, "refs": [], "depth": 0},
                    {"n": 3, "formula": M("P(a) → Q(a)"), "rule": {"text": "∀E"}, "refs": [1], "depth": 0},
                    {"n": 4, "formula": M("Q(a)"), "rule": {"text": "→E"}, "refs": [3, 2], "depth": 0}]}]},
            {"heading": "Algebraic certificate (Nullstellensatz, GF(2))", "blocks": [
                {"kind": "certificate",
                 "cert_kind": "Nullstellensatz refutation over GF(2)",
                 "statement": M("∑_C g_C · p_C  =  1", r"\sum_C g_C\,p_C = 1"),
                 "lines": [
                     {"label": M("g_1 · p_1"), "value": M("x·(x+1)"), "note": "clause 1"},
                     {"label": M("g_2 · p_2"), "value": M("1 + x·(x+1)"), "note": "clause 2"}],
                 "conclusion": "⊢ UNSAT  (the ideal contains 1)"}]},
            {"heading": "Where a proof does NOT yet exist: the RH reduction map", "blocks": [
                {"kind": "prose", "spans": [{"t": "text", "s":
                    "Pointed at backward_deriver, the engine renders the reduction chain honestly "
                    "— each link tagged proven or open — rather than a proof it does not have:"}]},
                {"kind": "reduction", "steps": [
                    {"claim": M("RH over 𝔽_q"), "reduces_to": M("Frobenius eigenvalue positivity"),
                     "status": "proven", "note": "Weil / Deligne — étale cohomology + purity."},
                    {"claim": M("RH over ℚ"), "reduces_to": M("archimedean Hodge–Riemann positivity"),
                     "status": "open",
                     "note": "the single open link: a Weil-cohomology / polarization for the motive of Spec ℤ."}],
                 "verdict": "The needle threads end-to-end over 𝔽_q; over ℚ it stops at exactly one archimedean positivity."},
                {"kind": "atoms", "atoms": [
                    {"kind": "grounded", "text": "the explicit formula derivably recovers primes from the zeros (a theorem)."},
                    {"kind": "grounded", "text": "the Weil quadratic form is PSD on the critical line (verified on every truncation)."},
                    {"kind": "gap", "what": "uniform archimedean SoS identity",
                     "missing": "an infinite-dimensional Positivstellensatz valid for all test functions  ⇔  real spectrum  ⇔  RH."}]}]}]}


def _selftest():
    import tempfile
    doc = _fixture()
    tmp = os.path.join(tempfile.gettempdir(), "logos_proof_selftest.pdf")
    d, data = render_document(doc, tmp)

    def check(c, m):
        if not c:
            print("FAIL:", m)
            sys.exit(1)

    # no glyph in the whole document may silently fall back to '?'
    def walk(x):
        if isinstance(x, str):
            yield x
        elif isinstance(x, dict):
            for v in x.values():
                yield from walk(v)
        elif isinstance(x, list):
            for v in x:
                yield from walk(v)
    bad = set()
    for s in walk(doc):
        bad |= pm.unrenderable_chars(s)
    check(not bad, "every glyph must render (no '?' fallback); offenders: %r" % bad)

    check(data.startswith(b"%PDF-1.7"), "valid PDF header")
    check(data.rstrip().endswith(b"%%EOF"), "ends at %%EOF")
    check(b"/BaseFont /Symbol" in data, "Symbol font embedded")
    check(b"\\042" in data, "the ∀ glyph byte (\\042) is present")
    check(b"\\256" in data, "the → glyph byte (\\256) is present")   # arrowright = 174 = 0o256
    viol = d.audit()
    check(viol == [], "layout audit clean, got: %r" % viol)
    off = d.offpage()
    check(off == [], "nothing off the page, got: %r" % off)
    check(len(d.pages) >= 1, "at least one page")
    print("OK  proof_pdf renderer: PDF %d bytes, %d page(s), audit clean, wrote %s"
          % (len(data), len(d.pages), tmp))


def main(argv):
    if len(argv) == 2 and argv[1] == "--selftest":
        _selftest()
        return
    if len(argv) == 3:
        with open(argv[1], "r", encoding="utf-8") as f:
            doc = json.load(f)
        _d, data = render_document(doc, argv[2])
        print("wrote %s (%d bytes)" % (argv[2], len(data)))
        return
    print(__doc__)
    sys.exit(2)


if __name__ == "__main__":
    main(sys.argv)
