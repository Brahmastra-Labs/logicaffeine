"""Minimal multi-page PDF engine — Python 3 standard library only.

Base-14 Helvetica (Regular/Bold/Oblique), WinAnsi/CP1252 text, measured word-wrap,
automatic page breaks, tables, and optional CONFIDENTIAL marking + diagonal
watermark. No font embedding, no external dependency, no network. Shared by every
PDF generator in the repository (`proof_pdf.py` and the report renderers) so the
layout code lives in one place.
"""

# --------------------------------------------------------------- font metrics
_HELV = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
]
_HELVB = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
]
HELV = {c: _HELV[c - 32] for c in range(32, 127)}
HELVB = {c: _HELVB[c - 32] for c in range(32, 127)}
for _t in (HELV, HELVB):
    _t[0x2014] = 1000  # em dash
    _t[0x00B7] = 278   # middle dot
    _t[0x2013] = 556   # en dash

# ------------------------------------------------------------- Symbol font (F4)
# Unicode codepoint -> (Symbol byte, width/1000). Lets the shared base-14 engine
# typeset first-order logic + Greek + math operators with no font embedding: the
# byte is the glyph's slot in Adobe Symbol's built-in encoding; widths are from
# the Symbol AFM so word-wrap and the layout audit stay exact.
SYM = {
    "∀": (34, 713), "∃": (36, 549), "∴": (92, 863), "⊥": (94, 658),
    "≅": (64, 549), "≡": (186, 549), "≈": (187, 549), "≤": (163, 549),
    "≥": (179, 549), "≠": (185, 549), "±": (177, 549), "×": (180, 549),
    "÷": (184, 549), "∞": (165, 713), "∝": (181, 713), "∂": (182, 494),
    "∇": (209, 713), "∑": (229, 713), "∏": (213, 823), "∫": (242, 274),
    "√": (214, 549), "∅": (198, 823), "∈": (206, 713), "∉": (207, 713),
    "∩": (199, 768), "∪": (200, 768), "⊂": (204, 713), "⊃": (201, 713),
    "⊆": (205, 713), "⊇": (202, 713), "∧": (217, 768), "∨": (218, 768),
    "¬": (216, 713), "→": (174, 987), "←": (172, 987), "↑": (173, 603),
    "↓": (175, 603), "↔": (171, 1042), "⇒": (222, 987), "⇐": (220, 987),
    "⇔": (219, 1042), "⊗": (196, 768), "⊕": (197, 768), "∠": (208, 768),
    "⋅": (215, 250), "°": (176, 400), "•": (183, 460), "ℵ": (192, 823),
    "ℑ": (193, 686), "ℜ": (194, 795), "−": (45, 549), "∗": (42, 500),
    # Greek lowercase
    "α": (97, 631), "β": (98, 549), "χ": (99, 549), "δ": (100, 494),
    "ε": (101, 439), "φ": (102, 521), "γ": (103, 411), "η": (104, 603),
    "ι": (105, 329), "κ": (107, 549), "λ": (108, 549), "μ": (109, 576),
    "ν": (110, 521), "ο": (111, 549), "π": (112, 549), "θ": (113, 521),
    "ρ": (114, 549), "σ": (115, 603), "τ": (116, 439), "υ": (117, 576),
    "ϖ": (118, 686), "ω": (119, 686), "ξ": (120, 493), "ψ": (121, 686),
    "ζ": (122, 494), "ς": (86, 439), "ϑ": (74, 631), "ϕ": (106, 603),
    # Greek uppercase
    "Α": (65, 722), "Β": (66, 667), "Χ": (67, 722), "Δ": (68, 612),
    "Ε": (69, 611), "Φ": (70, 763), "Γ": (71, 603), "Η": (72, 722),
    "Ι": (73, 333), "Κ": (75, 722), "Λ": (76, 686), "Μ": (77, 889),
    "Ν": (78, 722), "Ο": (79, 722), "Π": (80, 768), "Θ": (81, 741),
    "Ρ": (82, 556), "Σ": (83, 592), "Τ": (84, 611), "Υ": (85, 690),
    "Ω": (87, 768), "Ξ": (88, 645), "Ψ": (89, 795), "Ζ": (90, 611),
}

# Unicode that is in NEITHER Symbol nor cp1252 -> a legible stand-in, so report
# prose (subscripts, script-l, primes) never crashes the WinAnsi text path. The
# high-fidelity rendering of these goes through the .tex backend; this keeps the
# self-contained PDF honest and legible rather than perfect.
_TRANSLIT = {
    "ℓ": "l", "′": "'", "″": "''", "‴": "'''", "ϵ": "e", "∎": " QED",
    "₀": "0", "₁": "1", "₂": "2", "₃": "3", "₄": "4",
    "₅": "5", "₆": "6", "₇": "7", "₈": "8", "₉": "9",
    "⁰": "0", "¹": "1", "²": "2", "³": "3", "⁴": "4",
    "⁵": "5", "⁶": "6", "⁷": "7", "⁸": "8", "⁹": "9", "ⁿ": "n",
    "≪": "<<", "≫": ">>", "⟨": "<", "⟩": ">", "…": "...",
    "⌊": "|", "⌋": "|", "⌈": "|", "⌉": "|", "∣": "|", "‖": "||",
    "⟸": "<=", "⟹": "=>", "⟺": "<=>", "⟷": "<->", "↦": "->",
    "⇐": "<=", "⇒": "=>", "⇔": "<=>", "→": "->", "←": "<-", "↔": "<->",
    "⊢": "|-", "⊬": "|/-", "⊨": "|=", "ᾱ": "a", "ā": "a",
    # blackboard-bold (number sets) render as plain capitals in the WinAnsi path
    "𝔽": "F", "ℚ": "Q", "ℤ": "Z", "ℝ": "R", "ℂ": "C", "ℕ": "N", "𝔼": "E", "ℙ": "P",
}


def esc_symbol(s):
    return "".join("\\%03o" % SYM[ch][0] for ch in s)


def unrenderable_chars(s):
    """The set of characters in `s` that would silently fall back to '?': not a Symbol
    glyph, not cp1252-encodable, and not covered by the transliteration table. A proof
    document should have NONE — the beautiful-PDF guard asserts this set is empty."""
    bad = set()
    for ch in s:
        if ch in SYM or ch in _TRANSLIT:
            continue
        try:
            ch.encode("cp1252")
        except UnicodeEncodeError:
            bad.add(ch)
    return bad


ASC = 0.72  # baseline drop as fraction of font size


def _cp1252_ok(ch):
    try:
        ch.encode("cp1252")
        return True
    except UnicodeEncodeError:
        return False


def translit(text):
    """Fold the characters WinAnsi cannot carry into their ASCII spellings. `_TRANSLIT` exists for
    exactly this; applying it here is what lets a generator quote real source text (which routinely
    contains arrows and subscripts) instead of hand-sanitizing every excerpt.

    Only characters cp1252 CANNOT encode are folded. Several _TRANSLIT keys — the ellipsis and the
    superscript digits — do have WinAnsi glyphs, and those must keep rendering as themselves, or
    adding this pass would silently downgrade typography in every deck that already used them."""
    if not any(ch in _TRANSLIT and not _cp1252_ok(ch) for ch in text):
        return text
    return "".join(_TRANSLIT[ch] if (ch in _TRANSLIT and not _cp1252_ok(ch)) else ch for ch in text)


def esc(text):
    text = translit(text)
    try:
        raw = text.encode("cp1252")
    except UnicodeEncodeError:
        # Fail with the offending characters named, rather than a cp1252 traceback from deep inside
        # a draw call. Either add them to _TRANSLIT or route them through the Symbol font.
        raise ValueError(
            "cannot render %r in the WinAnsi text path — add them to _TRANSLIT or use draw_math/"
            "the Symbol font. Offending text: %r"
            % (sorted(unrenderable_chars(text)), text[:120])
        ) from None
    out = []
    for by in raw:
        if by in (0x28, 0x29, 0x5C):
            out.append("\\" + chr(by))
        elif 32 <= by < 127:
            out.append(chr(by))
        else:
            out.append("\\%03o" % by)
    return "".join(out)


def _tbl(font):
    return HELVB if font == "B" else HELV


def tokenize(s):
    runs, buf, bold, k = [], "", False, 0
    while k < len(s):
        if s[k:k + 2] == "**":
            if buf:
                runs.append((buf, bold))
                buf = ""
            bold = not bold
            k += 2
            continue
        buf += s[k]
        k += 1
    if buf:
        runs.append((buf, bold))
    words = []
    for text, b in runs:
        for seg in text.split(" "):
            if seg:
                words.append((seg, b))
    return words


class Doc:
    def __init__(self, page=(612.0, 792.0), margins=(64.0, 64.0, 66.0, 60.0),
                 confidential=False, ident="logicaffeine · LOGOS"):
        self.PW, self.PH = page
        self.ml, self.mr, self.mt, self.mb = margins
        self.cw = self.PW - self.ml - self.mr
        self.confidential = confidential
        self.ident = ident
        self.pages = [[]]
        self.cur = 0
        self.pen_y = self.PH - self.mt
        # palette
        self.ink = (0.12, 0.12, 0.14)
        self.gray = (0.40, 0.40, 0.42)
        self.accent = (0.09, 0.32, 0.51)
        self.hair = (0.78, 0.78, 0.80)
        self.red = (0.63, 0.13, 0.13)
        # layout DRC ledger: every text run and every registered text-container box, so a
        # generator can assert no word bleeds outside its cell (a drafting design-rule check).
        self._runs = []
        self._cells = []
        self._furniture()

    # ---- low level ----
    def emit(self, s):
        self.pages[self.cur].append(s)

    def char_w(self, ch, font, size):
        if font == "S":
            return SYM.get(ch, (0, 600))[1] / 1000.0 * size
        return _tbl(font).get(ord(ch), 556) / 1000.0 * size

    def text_w(self, s, font, size):
        # Measure what will actually be DRAWN: `esc` folds _TRANSLIT before encoding, and a
        # transliteration can change length ("=>" is two glyphs), so measuring the raw string would
        # make the layout DRC quietly wrong about its own output.
        return sum(self.char_w(c, font, size) for c in (s if font == "S" else translit(s)))

    def draw_text(self, x, y, s, font, size, color):
        fk = {"R": "F1", "B": "F2", "O": "F3", "S": "F4"}[font]
        self.emit("BT")
        self.emit("%.3f %.3f %.3f rg" % color)
        self.emit("/%s %.2f Tf" % (fk, size))
        self.emit("1 0 0 1 %.2f %.2f Tm" % (x, y))
        self.emit("(%s) Tj" % (esc_symbol(s) if font == "S" else esc(s)))
        self.emit("ET")
        if s.strip():
            self._runs.append((self.cur, x, y, self.text_w(s, font, size), size, s))

    def rect(self, x, y, w, h, color):
        self.emit("%.3f %.3f %.3f rg" % color)
        self.emit("%.2f %.2f %.2f %.2f re f" % (x, y, w, h))

    def hline(self, x1, x2, y, color, lw=0.6):
        self.emit("%.3f %.3f %.3f RG" % color)
        self.emit("%.2f w %.2f %.2f m %.2f %.2f l S" % (lw, x1, y, x2, y))

    def _dash(self, dash):
        if dash:
            self.emit("[%s] 0 d" % " ".join("%.1f" % d for d in dash))

    def seg(self, x1, y1, x2, y2, color, lw=0.8, dash=None):
        self.emit("q")
        self.emit("%.3f %.3f %.3f RG" % color)
        self._dash(dash)
        self.emit("%.2f w %.2f %.2f m %.2f %.2f l S" % (lw, x1, y1, x2, y2))
        self.emit("Q")

    def stroke_rect(self, x, y, w, h, color, lw=0.9, dash=None):
        self.emit("q")
        self.emit("%.3f %.3f %.3f RG" % color)
        self._dash(dash)
        self.emit("%.2f w %.2f %.2f %.2f %.2f re S" % (lw, x, y, w, h))
        self.emit("Q")
        # Every drawn box is a candidate text-cell for the layout DRC — so no box can be
        # forgotten. `audit()` checks each word against the SMALLEST box containing it.
        self._cells.append((self.cur, x, y, w, h))

    def fill_poly(self, pts, color):
        self.emit("%.3f %.3f %.3f rg" % color)
        self.emit("%.2f %.2f m" % (pts[0][0], pts[0][1]))
        for x, y in pts[1:]:
            self.emit("%.2f %.2f l" % (x, y))
        self.emit("h f")

    # ---- layout design-rule check (DRC): text must stay inside its cell ----
    def container(self, x, y, w, h):
        """Register a text-holding cell on the current page so `audit()` can verify that every
        word drawn inside it stays within its walls (a drafting rule: text never bleeds out)."""
        self._cells.append((self.cur, x, y, w, h))

    def audit(self, pad=1.6, report=1.5):
        """Return a list of legibility violations: any text run whose ink extends past the walls
        of the SMALLEST drawn box that contains its start anchor, by more than `report` points
        (a small tolerance so page furniture / hairline decoration is not noise). Empty == clean.

        Free labels (bus/axis annotations placed outside any box) are exempt. The check catches
        the real failure modes: a word too wide for its box (left/right) and over-wrapped lines
        below it (bottom). Because every stroke_rect self-registers, no box can be forgotten."""
        viol = []
        DESC, CAP = 0.21, 0.72  # Helvetica descender / cap-height as a fraction of the size
        for (pg, tx, ty, tw, tsz, s) in self._runs:
            my = ty + 0.30 * tsz
            # Match the start AND the centre — so CENTRED text wider than its box (whose start
            # anchor falls outside) is still associated with the box it overflows.
            anchors = ((tx + 0.6, my), (tx + tw / 2.0, my))
            best = None
            for (cpg, cx, cy, cw, ch) in self._cells:
                if cpg != pg:
                    continue
                if any(cx <= ax <= cx + cw and cy <= ay <= cy + ch for (ax, ay) in anchors):
                    if best is None or cw * ch < best[0]:
                        best = (cw * ch, cx, cy, cw, ch)
            if best is None:
                continue
            _, cx, cy, cw, ch = best
            page = pg + 1
            over_r = (tx + tw) - (cx + cw - pad)
            over_l = (cx + pad) - tx
            over_b = (cy + pad) - (ty - DESC * tsz)
            over_t = (ty + CAP * tsz) - (cy + ch - pad)
            if over_r > report:
                viol.append("p%d: %r overflows cell RIGHT by %.1fpt" % (page, s, over_r))
            elif over_l > report:
                viol.append("p%d: %r overflows cell LEFT by %.1fpt" % (page, s, over_l))
            if over_b > report:
                viol.append("p%d: %r overflows cell BOTTOM by %.1fpt" % (page, s, over_b))
            elif over_t > report:
                viol.append("p%d: %r overflows cell TOP by %.1fpt" % (page, s, over_t))
        return viol

    def _run_boxes(self):
        DESC, CAP = 0.21, 0.72
        return [(pg, x, y - DESC * sz, x + w, y + CAP * sz, s)
                for (pg, x, y, w, sz, s) in self._runs]

    def overlaps(self, min_area=4.0):
        """Pairs of text runs whose ink rectangles overlap by more than `min_area` pt^2 on the
        same page — catches free labels colliding into one another (which the cell check misses)."""
        b = self._run_boxes()
        out = []
        for i in range(len(b)):
            for j in range(i + 1, len(b)):
                if b[i][0] != b[j][0]:
                    continue
                ox = min(b[i][3], b[j][3]) - max(b[i][1], b[j][1])
                oy = min(b[i][4], b[j][4]) - max(b[i][2], b[j][2])
                if ox > 0.4 and oy > 0.4 and ox * oy > min_area:
                    out.append("p%d: %r  x  %r overlap %.0fpt^2" % (b[i][0] + 1, b[i][5], b[j][5], ox * oy))
        return out

    def offpage(self, margin=14.0):
        """Text whose ink falls outside the printable page (a hard clip at the paper edge)."""
        out = []
        for (pg, x1, y1, x2, y2, s) in self._run_boxes():
            if x1 < margin or x2 > self.PW - margin or y1 < margin or y2 > self.PH - margin:
                out.append("p%d: %r at (%.0f,%.0f)-(%.0f,%.0f) is off the page" % (pg + 1, s, x1, y1, x2, y2))
        return out

    def intrusions(self, min_overlap=8.0):
        """Text whose ink bleeds into a box it does NOT belong to — a bus/connector label
        straddling into an adjacent box, or a free note landing on a box. Each run is assigned
        to the smallest box containing its centre; overlapping THAT box or any box enclosing it
        is fine (nesting), overlapping any OTHER box is a bug."""
        cells = [(pg, x, y, x + w, y + h) for (pg, x, y, w, h) in self._cells]

        def encloses(o, i):  # box o encloses box i
            return o[0] <= i[0] and o[1] <= i[1] and o[2] >= i[2] and o[3] >= i[3]
        out = []
        for (pg, tx, ty, tw, tsz, s) in self._runs:
            t = (tx, ty - 0.21 * tsz, tx + tw, ty + 0.72 * tsz)
            cxp, cyp = tx + tw / 2.0, ty + 0.30 * tsz
            assigned = None
            for c in cells:
                if c[0] == pg and c[1] <= cxp <= c[3] and c[2] <= cyp <= c[4]:
                    a = (c[3] - c[1]) * (c[4] - c[2])
                    if assigned is None or a < assigned[0]:
                        assigned = (a, c[1], c[2], c[3], c[4])
            ab = assigned[1:] if assigned else None
            for c in cells:
                if c[0] != pg:
                    continue
                b = (c[1], c[2], c[3], c[4])
                if ab is not None and (b == ab or encloses(b, ab)):
                    continue
                ox = min(t[2], b[2]) - max(t[0], b[0])
                oy = min(t[3], b[3]) - max(t[1], b[1])
                if ox > 0.5 and oy > 0.5 and ox * oy > min_overlap:
                    out.append("p%d: %r bleeds into a box it is not part of (overlap %.0fpt^2)" % (pg + 1, s, ox * oy))
                    break
        return out

    # ---- page furniture (watermark + confidential mark) ----
    def _furniture(self):
        if self.confidential:
            self.emit("q")
            c = 0.70710678
            self.emit("0.930 0.930 0.945 rg")
            self.emit("BT /F2 68.0 Tf")
            self.emit("%.5f %.5f %.5f %.5f %.1f %.1f Tm" % (c, c, -c, c, 108, 250))
            self.emit("(%s) Tj ET" % esc("CONFIDENTIAL"))
            self.emit("Q")
            self.draw_text(self.PW - self.mr - self.text_w("CONFIDENTIAL", "B", 8),
                           self.PH - self.mt + 16, "CONFIDENTIAL", "B", 8, self.red)

    def new_page(self):
        self.pages.append([])
        self.cur = len(self.pages) - 1
        self.pen_y = self.PH - self.mt
        self._furniture()

    def ensure(self, h):
        if self.pen_y - h < self.mb:
            self.new_page()

    def gap(self, dy):
        self.pen_y -= dy

    # ---- text blocks ----
    def para(self, x0, s, size, maxw, leading, color, base="R"):
        words = tokenize(s)
        sw = self.char_w(" ", "R", size)
        self.ensure(leading)
        baseline = self.pen_y - ASC * size
        x, start = x0, True
        for w, b in words:
            font = "B" if b else base
            ww = self.text_w(w, font, size)
            if not start and x + sw + ww > x0 + maxw:
                self.pen_y -= leading
                self.ensure(leading)
                baseline = self.pen_y - ASC * size
                x, start = x0, True
            if not start:
                x += sw
            self.draw_text(x, baseline, w, font, size, color)
            x += ww
            start = False
        self.pen_y -= leading

    def line(self, x, s, font, size, leading, color, align="L"):
        self.ensure(leading)
        if align == "R":
            x -= self.text_w(s, font, size)
        self.draw_text(x, self.pen_y - ASC * size, s, font, size, color)
        self.pen_y -= leading

    # ---- math runs: mixed Helvetica + Symbol, laid out by measured width ----
    def _cp1252_safe(self, ch):
        try:
            ch.encode("cp1252")
            return ch
        except UnicodeEncodeError:
            return _TRANSLIT.get(ch, "?")

    def segment_math(self, s, base="R"):
        """Split a Unicode math string into consecutive (text, font) runs: glyphs in
        the Symbol table go to font 'S', a `**` toggles bold, everything else goes to
        `base` with a cp1252 stand-in for anything outside both fonts. Adjacent chars
        of the same font coalesce into one run so the PDF stays compact."""
        runs, buf, cur, bold = [], "", None, False

        def flush():
            nonlocal buf, cur
            if buf:
                runs.append((buf, cur))
            buf = ""

        i = 0
        while i < len(s):
            if s[i:i + 2] == "**":
                flush()
                cur = None
                bold = not bold
                i += 2
                continue
            ch = s[i]
            i += 1
            if ch in SYM:
                f, out = "S", ch
            else:
                f, out = ("B" if bold else base), self._cp1252_safe(ch)
            if cur is not None and cur != f:
                flush()
            cur = f
            buf += out
        flush()
        return runs

    def math_w(self, s, size, base="R"):
        return sum(self.text_w(t, f, size) for t, f in self.segment_math(s, base))

    def draw_math(self, x, baseline, s, size, color, base="R"):
        """Draw a mixed Helvetica/Symbol math string at a baseline; returns the end x."""
        for t, f in self.segment_math(s, base):
            self.draw_text(x, baseline, t, f, size, color)
            x += self.text_w(t, f, size)
        return x

    def display_math(self, s, size=11.5, leading=None, base="R", color=None,
                     align="C", indent=0.0):
        """A centered (or left/right) display formula on its own line."""
        color = self.ink if color is None else color
        leading = (size + 8.0) if leading is None else leading
        self.ensure(leading)
        w = self.math_w(s, size, base)
        if align == "C":
            x = self.ml + max(0.0, (self.cw - w) / 2.0)
        elif align == "R":
            x = self.PW - self.mr - w
        else:
            x = self.ml + indent
        self.draw_math(x, self.pen_y - ASC * size, s, size, color, base)
        self.pen_y -= leading
        return w

    def section(self, name, size=11.0):
        self.ensure(30)
        self.gap(14)
        baseline = self.pen_y - ASC * size
        self.draw_text(self.ml, baseline, name, "B", size, self.accent)
        rule_y = baseline - 5.0
        self.hline(self.ml, self.PW - self.mr, rule_y, self.hair, 0.6)
        self.pen_y = rule_y - 11.0

    def subhead(self, text, size=9.7):
        self.ensure(20)
        self.gap(7)
        self.line(self.ml, text, "B", size, size + 4.5, self.ink)

    def kv(self, label, value, labelcol=104.0, size=9.7, leading=13.0):
        self.ensure(leading)
        self.draw_text(self.ml, self.pen_y - ASC * size, label, "B", size, self.ink)
        self.para(self.ml + labelcol, value, size, self.cw - labelcol, leading, self.ink)
        self.gap(4)

    def numbered(self, n, text, size=9.7, leading=13.2, indent=20.0):
        self.ensure(leading)
        self.draw_text(self.ml, self.pen_y - ASC * size, "%d." % n, "B", size, self.accent)
        self.para(self.ml + indent, text, size, self.cw - indent, leading, self.ink)
        self.gap(5)

    def bullet(self, text, size=9.7, leading=13.0, indent=14.0):
        self.ensure(leading)
        self.draw_text(self.ml, self.pen_y - ASC * size, "·", "B", size, self.accent)
        self.para(self.ml + indent, text, size, self.cw - indent, leading, self.ink)
        self.gap(3)

    # ---- table: cols = [(header, width, align)], rows = [[cell,...]] ----
    def table(self, cols, rows, size=9.2, leading=12.4, pad=4.0, zebra=None):
        """`align` is honoured: "L" (default), "R" or "C", for the header and every cell. It was
        accepted and discarded for a long time, which left numeric columns ragged — in a table of
        measurements the digits should line up.

        `zebra` tints alternate rows; pass a colour to switch it on."""
        total = sum(w for _h, w, _a in cols)

        def offset(al, line_w, w):
            avail = w - pad
            if al == "R":
                return max(0.0, avail - line_w)
            if al == "C":
                return max(0.0, (avail - line_w) / 2.0)
            return 0.0

        def header():
            self.ensure(leading + 4)
            x = self.ml
            base = self.pen_y - ASC * size
            for head, w, al in cols:
                self.draw_text(x + offset(al, self.text_w(head, "B", size), w), base,
                               head, "B", size, self.accent)
                x += w
            self.pen_y -= leading
            self.hline(self.ml, self.ml + total, self.pen_y + 3, self.hair, 0.6)
            self.gap(4)
        header()
        for i, row in enumerate(rows):
            counts = []
            for (h, w, _al), cell in zip(cols, row):
                counts.append(self._cell_lines(cell, w - pad, size))
            nlines = max(counts)
            need = nlines * leading + 3
            if self.pen_y - need < self.mb:
                self.new_page()
                header()
            top = self.pen_y
            if zebra is not None and i % 2 == 1:
                self.rect(self.ml - 2, top - nlines * leading + 1, total + 4,
                          nlines * leading + 1, zebra)
            x = self.ml
            for (h, w, al), cell in zip(cols, row):
                self._draw_cell(x, top, cell, w - pad, size, leading, self.ink, al, pad)
                x += w
            self.pen_y = top - nlines * leading
            self.hline(self.ml, self.ml + total, self.pen_y + 2, (0.90, 0.90, 0.92), 0.4)
            self.gap(3)

    def _cell_lines(self, s, maxw, size):
        words = tokenize(s)
        sw = self.char_w(" ", "R", size)
        x, lines, start = 0.0, 1, True
        for w, b in words:
            ww = self.text_w(w, "B" if b else "R", size)
            if start:
                x, start = ww, False
            elif x + sw + ww > maxw:
                lines, x = lines + 1, ww
            else:
                x += sw + ww
        return lines

    def _draw_cell(self, x0, top, s, maxw, size, leading, color, al="L", pad=0.0):
        # Lay the words out into lines first, so a right- or centre-aligned cell can be offset by
        # its own measured width. Drawing greedily left-to-right cannot align.
        sw = self.char_w(" ", "R", size)
        lines, cur, cur_w = [], [], 0.0
        for w, b in tokenize(s):
            font = "B" if b else "R"
            ww = self.text_w(w, font, size)
            if cur and cur_w + sw + ww > maxw:
                lines.append((cur, cur_w))
                cur, cur_w = [(w, font, ww)], ww
            else:
                cur_w = ww if not cur else cur_w + sw + ww
                cur.append((w, font, ww))
        if cur:
            lines.append((cur, cur_w))

        baseline = top - ASC * size
        for words, line_w in lines:
            if al == "R":
                x = x0 + max(0.0, maxw - line_w)
            elif al == "C":
                x = x0 + max(0.0, (maxw - line_w) / 2.0)
            else:
                x = x0
            for i, (w, font, ww) in enumerate(words):
                if i:
                    x += sw
                self.draw_text(x, baseline, w, font, size, color)
                x += ww
            baseline -= leading

    # ---- figure vocabulary: the composed shapes every deck draws ----
    # These were copy-pasted into each deck generator until three of them carried near-identical
    # copies. They live here now so a new deck inherits the house figure language instead of
    # re-deriving it, and so a fix to one reaches all of them.

    def wrap(self, s, size, maxw, font="R"):
        """Measured word-wrap: split `s` into lines that fit `maxw` at `size`."""
        lines, cur = [], ""
        for w in s.split(" "):
            t = (cur + " " + w).strip()
            if self.text_w(t, font, size) > maxw and cur:
                lines.append(cur)
                cur = w
            else:
                cur = t
        if cur:
            lines.append(cur)
        return lines

    def arrow(self, x1, y1, x2, y2, color, lw=1.1, head=4.0):
        """A line with a filled triangular head. Horizontal or vertical only — a diagonal head
        would need rotation, and no deck has wanted one."""
        self.seg(x1, y1, x2, y2, color, lw)
        s = head
        if abs(y1 - y2) < 0.5:
            xs = x2 - s if x2 > x1 else x2 + s
            self.fill_poly([(x2, y2), (xs, y2 + s * 0.7), (xs, y2 - s * 0.7)], color)
        elif abs(x1 - x2) < 0.5:
            ys = y2 + s if y2 > y1 else y2 - s
            self.fill_poly([(x2, y2), (x2 - s * 0.7, ys), (x2 + s * 0.7, ys)], color)

    def panel(self, x, ybot, w, h, fill, border, dash=None, lw=1.0):
        """A filled, stroked box. `ybot` is the BOTTOM edge (PDF convention)."""
        self.rect(x, ybot, w, h, fill)
        self.stroke_rect(x, ybot, w, h, border, lw, dash)

    def code_lines(self, lines, size, maxw):
        """Hard-wrap code at measured width, continuing over-long lines at a deeper indent."""
        out = []
        for ln in lines:
            indent = len(ln) - len(ln.lstrip(" "))
            body = ln.rstrip()
            if self.text_w(body, "R", size) <= maxw:
                out.append(body)
                continue
            cur = body
            while self.text_w(cur, "R", size) > maxw and len(cur) > 8:
                k = len(cur)
                while k > 8 and self.text_w(cur[:k], "R", size) > maxw:
                    k -= 4
                out.append(cur[:k])
                cur = " " * (indent + 4) + cur[k:].lstrip()
            out.append(cur)
        return out

    def codebox(self, x, ytop, w, code, title=None, size=6.4, fill=(0.960, 0.965, 0.972),
                border=(0.62, 0.65, 0.69)):
        """A titled code panel. Takes the TOP edge and returns the resulting bottom y, so the
        caller advances with `d.pen_y = d.codebox(...) - 6`.

        Pass `ytop=None` to flow: the box is measured, a page break is taken if it would not fit,
        and it is drawn at the current pen. Passing an explicit `ytop` draws exactly there and will
        run off the page if the box is taller than the remaining column — the flowing form exists
        because that is a trap every caller otherwise has to compute around by hand."""
        lines = self.code_lines(code.rstrip("\n").split("\n"), size, w - 14)
        lh = size + 1.7
        pad_t = 10.5 if title else 6.0
        h = pad_t + (10.0 if title else 0.0) + len(lines) * lh + 5.0
        if ytop is None:
            self.ensure(h + 4)
            ytop = self.pen_y
        self.panel(x, ytop - h, w, h, fill, border, lw=0.8)
        y = ytop - pad_t
        if title:
            self.draw_text(x + 6, y - 1, title, "B", 6.6, self.accent)
            y -= 10.0
        for ln in lines:
            self.draw_text(x + 6, y - size * 0.72 + 3, ln, "R", size, (0.20, 0.22, 0.28))
            y -= lh
        return ytop - h

    def caption(self, s, color=(0.46, 0.49, 0.54)):
        """A small oblique caption under a figure."""
        self.gap(2)
        self.para(self.ml, s, 7.3, self.cw, 9.6, color, base="O")
        self.gap(2)

    def chip(self, x, ybot, w, h, label, sub, fill=(0.966, 0.966, 0.972), border=None, labcol=None):
        """A small labelled box with an optional subtitle — the unit of every card row."""
        self.panel(x, ybot, w, h, fill, border or self.accent, lw=0.9)
        self.draw_text(x + 5, ybot + h - 10, label, "B", 7.2, labcol or self.ink)
        if sub:
            self.draw_text(x + 5, ybot + h - 19, sub, "R", 5.9, (0.46, 0.49, 0.54))

    def callout(self, text, fill, border, textcol, size=8.8, leading=11.6, bold=True):
        """The hero/warning/CTA box: a wrapped paragraph in a tinted panel, sized to its own content.
        The three deck callouts were three copies of exactly this.

        `**bold**` markup is honoured, like `para` — drawing the string straight would print the
        asterisks, which is exactly what the first version of this did. `bold=True` makes the whole
        box bold and the markup then only marks emphasis that is already there, so the markers are
        simply consumed."""
        base = "B" if bold else "R"
        runs = tokenize(text)
        sw = self.char_w(" ", base, size)
        maxw = self.cw - 20

        # Lay the words out once to learn the height, then draw with the same arithmetic.
        laid, cur, x = [], [], 0.0
        for w, b in runs:
            font = "B" if (b or bold) else "R"
            ww = self.text_w(w, font, size)
            if cur and x + sw + ww > maxw:
                laid.append(cur)
                cur, x = [(w, font, ww)], ww
            else:
                x = ww if not cur else x + sw + ww
                cur.append((w, font, ww))
        if cur:
            laid.append(cur)

        h = len(laid) * leading + 14
        self.ensure(h + 6)
        top = self.pen_y
        self.panel(self.ml, top - h, self.cw, h, fill, border, lw=1.2)
        y = top - 13
        for line in laid:
            x = self.ml + 10
            for i, (w, font, ww) in enumerate(line):
                if i:
                    x += sw
                self.draw_text(x, y, w, font, size, textcol)
                x += ww
            y -= leading
        self.pen_y = top - h - 4

    # ---- footers (page numbers + confidential notice), then assemble ----
    def _footers(self):
        n = len(self.pages)
        for i in range(n):
            self.cur = i
            self.hline(self.ml, self.PW - self.mr, 50, self.hair, 0.7)
            left = self.ident + ("  ·  CONFIDENTIAL" if self.confidential else "")
            self.draw_text(self.ml, 38, left, "R", 8.0, self.gray)
            right = "p. %d / %d" % (i + 1, n)
            self.draw_text(self.PW - self.mr - self.text_w(right, "R", 8.0), 38,
                           right, "R", 8.0, self.gray)

    def build(self):
        self._footers()
        objs = []
        objs.append(b"<< /Type /Catalog /Pages 2 0 R >>")
        kids = " ".join("%d 0 R" % (7 + 2 * i) for i in range(len(self.pages)))
        objs.append(("<< /Type /Pages /Kids [%s] /Count %d >>"
                     % (kids, len(self.pages))).encode("ascii"))
        objs.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
        objs.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>")
        objs.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique /Encoding /WinAnsiEncoding >>")
        objs.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>")
        for i, page in enumerate(self.pages):
            content = "\n".join(page).encode("ascii")
            cnum = 8 + 2 * i
            objs.append(("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 %.0f %.0f] /Rotate 0 "
                         "/Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R /F4 6 0 R >> >> "
                         "/Contents %d 0 R >>" % (self.PW, self.PH, cnum)).encode("ascii"))
            objs.append(b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream")
        # renumber: catalog=1,pages=2,fonts=3-6 (Helv R/B/O + Symbol),then page/content pairs 7..
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for i, body in enumerate(objs, 1):
            offsets.append(len(out))
            out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
        xref_at = len(out)
        out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
        for off in offsets:
            out += b"%010d 00000 n \n" % off
        out += (b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n"
                % (len(objs) + 1, xref_at))
        return bytes(out)
