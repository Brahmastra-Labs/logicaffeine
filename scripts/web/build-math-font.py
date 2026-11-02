#!/usr/bin/env python3
"""Build Logos Math Mono, the font the Studio's Math editor sets its text in.

Math files are ASCII — the kernel reads `forall`, `fun`, `->` — and the editor shows them as the
symbols they stand for. The substitution lives in the font so that the editor's two layers (the
transparent textarea that edits and the highlighted copy under it) shape the same glyphs, and the
caret, selection and clicks stay where the text is. The font is DejaVu Sans Mono, renamed (its
licence requires a modified font to drop the Bitstream and Vera names), subset to what the Math
editor shows, with contextual ligatures that fire on whole words and whole operators only.

    build-math-font.py <DejaVuSansMono.ttf> <out.woff2>

Needs fontTools, brotli and uharfbuzz. After building, it shapes a set of probes with HarfBuzz and
fails if a ligature fires where it must not, or does not fire where it must.
"""

import copy
import os
import sys
import tempfile

import uharfbuzz as hb
from fontTools import subset
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.ttLib import TTFont

FAMILY = "Logos Math Mono"
POSTSCRIPT = "LogosMathMono-Regular"

# Each ASCII spelling the kernel reads, the symbol it is drawn as, and the ligature glyph's name.
WORDS = [
    ("forall", 0x2200, "forall.liga"),
    ("exists", 0x2203, "exists.liga"),
    ("fun", 0x03BB, "fun.liga"),
]
OPERATORS = [
    ("->", 0x2192, "arrow.liga"),
    ("=>", 0x21D2, "darrow.liga"),
    ("<=", 0x2264, "le.liga"),
    (">=", 0x2265, "ge.liga"),
]

# Characters that continue a word or an operator: a ligature next to one of these does not fire.
WORD_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_'"
OPERATOR_CHARS = "-<>=|:!~*+/\\"

# What the Math editor shows: ASCII, Latin-1 (Gödel), Greek, general punctuation, arrows,
# mathematical operators and the supplemental arrows.
UNICODES = (
    list(range(0x20, 0x7F))
    + list(range(0xA0, 0x100))
    + list(range(0x370, 0x400))
    + list(range(0x2000, 0x2070))
    + list(range(0x2190, 0x2200))
    + list(range(0x2200, 0x2300))
    + list(range(0x27F0, 0x2800))
)

# (text, ligatures that must appear, ligatures that must not)
PROBES = [
    ("forall x, P x", {"forall.liga"}, set()),
    ("Check forall.", {"forall.liga"}, set()),
    ("(forall (A : Type), A -> A)", {"forall.liga", "arrow.liga"}, set()),
    ("fun x => x", {"fun.liga", "darrow.liga"}, set()),
    ("exists n, n <= m", {"exists.liga", "le.liga"}, set()),
    ("m >= n", {"ge.liga"}, set()),
    ("funny function defun foralls forall_x x_forall exists' fun1", set(), {"fun.liga", "forall.liga", "exists.liga"}),
    ("a --> b <-> c ->> d =>> e <=> f >== g <<= h", set(), {"arrow.liga", "darrow.liga", "le.liga", "ge.liga"}),
    ("x := y :: z", set(), {"arrow.liga", "darrow.liga", "le.liga", "ge.liga"}),
]


def glyph_of(font, char):
    return font.getBestCmap()[ord(char)]


def feature_file(font):
    """The GSUB rules, in the feature-file language."""
    cls = lambda chars: "[" + " ".join(glyph_of(font, c) for c in chars) + "]"
    seq = lambda text: [glyph_of(font, c) for c in text]
    lines = ["languagesystem DFLT dflt;", "languagesystem latn dflt;", f"@WORD = {cls(WORD_CHARS)};", f"@OP = {cls(OPERATOR_CHARS)};"]
    for text, _, name in WORDS + OPERATORS:
        lines.append(f"lookup L_{name.split('.')[0]} {{ sub {' '.join(seq(text))} by {name}; }} L_{name.split('.')[0]};")
    lines.append("feature calt {")
    for text, _, name in WORDS:
        glyphs = seq(text)
        marked = " ".join(g + "'" for g in glyphs)
        lines.append(f"  lookup W_{name.split('.')[0]} {{")
        lines.append(f"    ignore sub @WORD {marked};")
        lines.append(f"    ignore sub {marked} @WORD;")
        lines.append(f"    sub {glyphs[0]}' lookup L_{name.split('.')[0]} {' '.join(g + chr(39) for g in glyphs[1:])};")
        lines.append(f"  }} W_{name.split('.')[0]};")
    for text, _, name in OPERATORS:
        glyphs = seq(text)
        marked = " ".join(g + "'" for g in glyphs)
        lines.append(f"  lookup O_{name.split('.')[0]} {{")
        lines.append(f"    ignore sub @OP {marked};")
        lines.append(f"    ignore sub {marked} @OP;")
        lines.append(f"    sub {glyphs[0]}' lookup L_{name.split('.')[0]} {' '.join(g + chr(39) for g in glyphs[1:])};")
        lines.append(f"  }} O_{name.split('.')[0]};")
    lines.append("} calt;")
    return "\n".join(lines) + "\n"


def add_ligature_glyphs(font):
    """Each ligature glyph is a copy of its symbol's glyph, one cell wide."""
    glyf, hmtx = font["glyf"], font["hmtx"]
    for _, codepoint, name in WORDS + OPERATORS:
        source = font.getBestCmap()[codepoint]
        glyf[name] = copy.deepcopy(glyf[source])
        hmtx[name] = hmtx[source]
    # Setting a glyph adds it to the glyf table's order; the font takes that order.
    font.setGlyphOrder(list(glyf.glyphOrder))


def rename(font):
    names = font["name"]
    notice = f"{FAMILY} is a modified version of DejaVu Sans Mono (Bitstream Vera Sans Mono)."
    for record in list(names.names):
        if record.nameID in (1, 16):
            names.setName(FAMILY, record.nameID, record.platformID, record.platEncID, record.langID)
        elif record.nameID == 3:
            names.setName(f"{POSTSCRIPT};{notice}", 3, record.platformID, record.platEncID, record.langID)
        elif record.nameID == 4:
            names.setName(FAMILY, 4, record.platformID, record.platEncID, record.langID)
        elif record.nameID == 6:
            names.setName(POSTSCRIPT, 6, record.platformID, record.platEncID, record.langID)
        elif record.nameID == 0:
            text = record.toUnicode()
            if notice not in text:
                names.setName(f"{text} {notice}", 0, record.platformID, record.platEncID, record.langID)
    font["post"].formatType = 2.0


def shaped_glyphs(path, text):
    blob = hb.Blob.from_file_path(path)
    font = hb.Font(hb.Face(blob))
    buf = hb.Buffer()
    buf.add_str(text)
    buf.guess_segment_properties()
    hb.shape(font, buf, {"calt": True})
    return {font.glyph_to_string(info.codepoint) for info in buf.glyph_infos}


def check(path):
    failures = []
    for text, must, must_not in PROBES:
        glyphs = shaped_glyphs(path, text)
        for name in must - glyphs:
            failures.append(f"{text!r}: {name} did not fire")
        for name in must_not & glyphs:
            failures.append(f"{text!r}: {name} fired inside a longer word or operator")
    return failures


def main(source, out):
    font = TTFont(source)
    add_ligature_glyphs(font)
    addOpenTypeFeaturesFromString(font, feature_file(font), tables=["GSUB"])
    rename(font)
    options = subset.Options()
    options.layout_features = ["calt"]
    options.glyph_names = True
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.notdef_outline = True
    options.flavor = "woff2"
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=UNICODES)
    subsetter.subset(font)
    font.flavor = "woff2"
    font.save(out)
    # HarfBuzz reads the uncompressed form; check the very glyphs that ship.
    shipped = TTFont(out)
    shipped.flavor = None
    with tempfile.TemporaryDirectory() as scratch:
        check_path = os.path.join(scratch, "shipped.ttf")
        shipped.save(check_path)
        failures = check(check_path)
    if failures:
        print("\n".join(failures), file=sys.stderr)
        sys.exit(1)
    print(f"{out}: {len(shipped.getGlyphOrder())} glyphs, {len(PROBES)} probes shaped as expected")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
