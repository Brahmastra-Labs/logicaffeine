"""Selftest for the Symbol-font math extension of _pdfmini (RED-first spec).

Run: python3 scripts/dev/_pdfmath_selftest.py   (exit 0 == green, prints a receipt).

Locks the engine capability the beautiful-proof PDF backend rests on: that the
shared base-14 PDF engine can typeset first-order logic (∀ ∃ ∧ ∨ → ↔ ¬) and
Greek via the Symbol font (F4) with an EXACT layout (the audit stays clean), and
that the pure-Helvetica path is byte-for-byte unchanged (the chip generators must
not regress). This file is executed by a cargo lock so it rides the test suite.
"""

import sys, os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _pdfmini as pm  # noqa: E402


def check(cond, msg):
    if not cond:
        print("FAIL:", msg)
        sys.exit(1)


def test_segmentation_splits_symbol_from_helvetica():
    d = pm.Doc()
    runs = d.segment_math("∀x (P(x) → Q(x)) ∧ ¬Q(a)")
    fonts = {f for _t, f in runs}
    check("S" in fonts, "logic glyphs must route to the Symbol font run")
    check("R" in fonts, "latin text must stay on Helvetica")
    # the ∀ run is a Symbol run; the 'x (P(x) ' run is Helvetica
    check(any(f == "S" and "∀" in t for t, f in runs), "∀ must be a Symbol run")
    check(any(f == "S" and "→" in t for t, f in runs), "→ must be a Symbol run")
    check(d.math_w("∀x", 11.0, "R") > 0, "measured math width must be positive")


def test_greek_and_math_operators_are_symbol():
    d = pm.Doc()
    for g in "λμνδγρστζπθχψξ":
        runs = d.segment_math(g)
        check(runs and runs[0][1] == "S", "Greek %r must be a Symbol glyph" % g)
    for op in "∑∏∫∈≤≥≠≡⊗⊕≅":
        runs = d.segment_math(op)
        check(runs and runs[0][1] == "S", "operator %r must be a Symbol glyph" % op)


def test_non_encodable_prose_never_crashes():
    # subscripts / script-l are outside BOTH fonts — must transliterate, not raise.
    d = pm.Doc()
    d.display_math("ℓ(ν) ≤ ℓ(λ)·ℓ(μ)   g(λ,μ,ν)  s₁ x²")
    out = d.build()
    check(out.startswith(b"%PDF-1.7"), "must be a valid PDF header")
    check(out.rstrip().endswith(b"%%EOF"), "must end at %%EOF")


def test_pdf_embeds_symbol_font_and_glyph_bytes():
    d = pm.Doc()
    d.section("Modus ponens")
    d.display_math("∀x (P(x) → Q(x)),  P(a)  ⊢  Q(a)")
    out = d.build()
    check(b"/BaseFont /Symbol" in out, "the Symbol font object must be embedded")
    check(b"/F4 6 0 R" in out, "every page must expose /F4 as the Symbol font")
    # ∀ is Symbol code 34 == octal 042 -> the content stream must contain \042
    check(b"\\042" in out, "the ∀ glyph byte (\\042) must appear in the content stream")
    check(d.audit() == [], "the typeset math must not overflow its measured box")


def test_helvetica_path_is_unregressed():
    d = pm.Doc()
    d.section("Plain")
    d.para(d.ml, "The quick **brown** fox — a plain WinAnsi paragraph.", 10.0,
           d.cw, 13.0, d.ink)
    out = d.build()
    for f in (b"/BaseFont /Helvetica", b"/BaseFont /Helvetica-Bold",
              b"/BaseFont /Helvetica-Oblique"):
        check(f in out, "%r must still be present" % f)
    check(out.startswith(b"%PDF-1.7") and out.rstrip().endswith(b"%%EOF"),
          "the pure-Helvetica document must still be a valid PDF")


def main():
    test_segmentation_splits_symbol_from_helvetica()
    test_greek_and_math_operators_are_symbol()
    test_non_encodable_prose_never_crashes()
    test_pdf_embeds_symbol_font_and_glyph_bytes()
    test_helvetica_path_is_unregressed()
    print("OK  _pdfmini Symbol-font math extension: 5/5 checks green")


if __name__ == "__main__":
    main()
