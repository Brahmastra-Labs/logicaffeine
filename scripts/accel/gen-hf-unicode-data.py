#!/usr/bin/env python3
r"""The Unicode data the Hugging Face tokenizer port reads, generated from the Unicode Character Database files.

    gen-hf-unicode-data.py > crates/logicaffeine_compile/src/model/hf/unicode_data.rs

`tokenizers` reads the database twice, at two versions: its patterns run on Oniguruma, whose general categories,
White_Space property and case folding are Unicode 16.0's, and its normal forms on unicode-normalization-alignments,
whose decompositions and combining classes are Unicode 9.0's. This script writes each part from the files of its
own version under external/unicode/ (scripts/accel/fetch-unicode-data.sh): every codepoint's general category, the
White_Space codepoints, the word characters `\w` matches, the Han script `\p{Han}` matches (a tiktoken pattern's,
Kimi K2's), the simple case folding `(?i)` compares by, and the canonical decompositions, combining classes and
primary composites NFC is computed from. `model_hf_intake_lock`
regenerates the file and holds the checked-in copy equal to it.
"""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PATTERNS = "16.0.0"
NORMAL_FORMS = "9.0.0"

CATEGORIES = ["Lu", "Ll", "Lt", "Lm", "Lo", "Mn", "Mc", "Me", "Nd", "Nl", "No", "Pc", "Pd", "Ps", "Pe", "Pi", "Pf",
              "Po", "Sm", "Sc", "Sk", "So", "Zs", "Zl", "Zp", "Cc", "Cf", "Cs", "Co", "Cn"]
MAX = 0x110000


def ucd(version, name):
    return (ROOT / "external" / "unicode" / version / name).read_text(encoding="utf-8")


def unicode_data(version):
    """`cp -> (category, ccc, decomposition)` for every codepoint UnicodeData.txt assigns, its ranges expanded."""
    out = {}
    first = None
    for line in ucd(version, "UnicodeData.txt").splitlines():
        f = line.split(";")
        cp, name, cat, ccc, decomp = int(f[0], 16), f[1], f[2], int(f[3]), f[5]
        if name.endswith(", First>"):
            first = cp
            continue
        if name.endswith(", Last>"):
            for c in range(first, cp + 1):
                out[c] = (cat, ccc, decomp)
            first = None
            continue
        out[cp] = (cat, ccc, decomp)
    return out


def ranges_of(path_text, prop):
    """The codepoints a property file lists under `prop`."""
    out = set()
    for line in path_text.splitlines():
        line = line.split("#")[0].strip()
        if not line:
            continue
        r, p = [x.strip() for x in line.split(";")[:2]]
        if p != prop:
            continue
        lo, _, hi = r.partition("..")
        out.update(range(int(lo, 16), int(hi or lo, 16) + 1))
    return out


# Below U+0100 Oniguruma reads its word class from a Latin-1 ctype table, which also marks the superscript digits
# and the vulgar fractions.
LATIN1_WORD = {0xB2, 0xB3, 0xB9, 0xBC, 0xBD, 0xBE}


def word_ranges(data):
    """`(first, last)` runs of Oniguruma's word characters: Alphabetic, Mark, Decimal_Number, Connector_Punctuation."""
    alphabetic = ranges_of(ucd(PATTERNS, "DerivedCoreProperties.txt"), "Alphabetic")
    word = {cp for cp, (cat, _, _) in data.items() if cat[0] == "M" or cat in ("Nd", "Pc")} | alphabetic | LATIN1_WORD
    out = []
    for cp in sorted(word):
        if out and out[-1][1] == cp - 1:
            out[-1] = (out[-1][0], cp)
        else:
            out.append((cp, cp))
    return out


def category_ranges(data):
    out = []
    prev = None
    for cp in range(MAX):
        cat = CATEGORIES.index(data[cp][0] if cp in data else "Cn")
        if cat != prev:
            out.append((cp, cat))
            prev = cat
    return out


def simple_folds():
    """`cp -> fold` from CaseFolding.txt's common and simple mappings."""
    out = []
    for line in ucd(PATTERNS, "CaseFolding.txt").splitlines():
        line = line.split("#")[0].strip()
        if not line:
            continue
        code, status, mapping = [x.strip() for x in line.split(";")[:3]]
        if status in ("C", "S"):
            out.append((int(code, 16), int(mapping, 16)))
    return sorted(out)


def canonical_decompositions(data):
    """`cp -> (a, b)` (b = 0 for a singleton) for every codepoint with a canonical decomposition."""
    out = []
    for cp in sorted(data):
        d = data[cp][2]
        if not d or d.startswith("<"):
            continue
        parts = [int(x, 16) for x in d.split()]
        assert len(parts) in (1, 2), (hex(cp), d)
        out.append((cp, parts[0], parts[1] if len(parts) == 2 else 0))
    return out


def combining_classes(data):
    """`(first, last, class)` runs of codepoints with a nonzero canonical combining class."""
    out = []
    for cp in sorted(data):
        ccc = data[cp][1]
        if ccc == 0:
            continue
        if out and out[-1][1] == cp - 1 and out[-1][2] == ccc:
            out[-1] = (out[-1][0], cp, ccc)
        else:
            out.append((cp, cp, ccc))
    return out


def primary_composites(data, decomps):
    """`(a, b) -> c`: the two-codepoint canonical decompositions not in Full_Composition_Exclusion (the listed
    exclusions, and decompositions of or beginning with a non-starter)."""
    excluded = set()
    for line in ucd(NORMAL_FORMS, "CompositionExclusions.txt").splitlines():
        line = line.split("#")[0].strip()
        if line:
            excluded.add(int(line, 16))
    out = []
    for cp, a, b in decomps:
        if not b or cp in excluded or data[cp][1] != 0 or data.get(a, ("", 0, ""))[1] != 0:
            continue
        out.append((a, b, cp))
    return sorted(out)


def rows(items, fmt, per_line):
    lines = []
    for i in range(0, len(items), per_line):
        lines.append("    " + " ".join(fmt(x) for x in items[i:i + per_line]))
    return "\n".join(lines)


def runs(points):
    """`(first, last)` runs of ascending codepoints."""
    out = []
    for c in sorted(points):
        if out and out[-1][1] == c - 1:
            out[-1][1] = c
        else:
            out.append([c, c])
    return [tuple(r) for r in out]


def main():
    patterns = unicode_data(PATTERNS)
    normal = unicode_data(NORMAL_FORMS)
    cats = category_ranges(patterns)
    white = sorted(ranges_of(ucd(PATTERNS, "PropList.txt"), "White_Space"))
    word = word_ranges(patterns)
    han = runs(ranges_of(ucd(PATTERNS, "Scripts.txt"), "Han"))
    folds = simple_folds()
    decomps = canonical_decompositions(normal)
    classes = combining_classes(normal)
    composites = primary_composites(normal, decomps)
    w = sys.stdout.write
    w(f"//! Unicode data generated by `scripts/accel/gen-hf-unicode-data.py` from the Unicode Character Database: the\n"
      f"//! pattern tables from {PATTERNS}, the normal-form tables from {NORMAL_FORMS}; do not edit.\n\n")
    w("/// The Unicode version of the tables the patterns read (Oniguruma's).\n")
    w(f"pub const PATTERN_VERSION: &str = \"{PATTERNS}\";\n\n")
    w("/// The Unicode version of the tables the normal forms read (unicode-normalization-alignments').\n")
    w(f"pub const NORMAL_FORM_VERSION: &str = \"{NORMAL_FORMS}\";\n\n")
    w("/// General categories, in the order [`CATEGORY_RANGES`] indexes them.\n")
    w("pub(super) const CATEGORY_NAMES: [&str; 30] = [" + ", ".join(f'"{c}"' for c in CATEGORIES) + "];\n\n")
    w("/// `first << 5 | category`: the category every codepoint holds up to the next entry's first.\n")
    w(f"pub(super) static CATEGORY_RANGES: [u32; {len(cats)}] = [\n")
    w(rows(cats, lambda x: f"0x{(x[0] << 5) | x[1]:08X},", 8) + "\n];\n\n")
    w("/// The White_Space codepoints, ascending.\n")
    w(f"pub(super) static WHITE_SPACE: [u32; {len(white)}] = [\n")
    w(rows(white, lambda x: f"0x{x:X},", 10) + "\n];\n\n")
    w("/// `(first, last)`: runs of the codepoints `\\w` matches, ascending.\n")
    w(f"pub(super) static WORD: [(u32, u32); {len(word)}] = [\n")
    w(rows(word, lambda x: f"(0x{x[0]:X}, 0x{x[1]:X}),", 6) + "\n];\n\n")
    w("/// `(first, last)`: runs of the codepoints of the Han script, ascending.\n")
    w(f"pub(super) static HAN: [(u32, u32); {len(han)}] = [\n")
    w(rows(han, lambda x: f"(0x{x[0]:X}, 0x{x[1]:X}),", 6) + "\n];\n\n")
    w("/// `(codepoint, simple case fold)` for every codepoint whose fold differs, ascending.\n")
    w(f"pub(super) static SIMPLE_FOLDS: [(u32, u32); {len(folds)}] = [\n")
    w(rows(folds, lambda x: f"(0x{x[0]:X}, 0x{x[1]:X}),", 6) + "\n];\n\n")
    w("/// `(codepoint, first, second)`: canonical decompositions, one step each (`second` 0 for a singleton), ascending.\n")
    w(f"pub(super) static CANONICAL_DECOMPOSITIONS: [(u32, u32, u32); {len(decomps)}] = [\n")
    w(rows(decomps, lambda x: f"(0x{x[0]:X}, 0x{x[1]:X}, 0x{x[2]:X}),", 4) + "\n];\n\n")
    w("/// `(first, last, class)`: runs of codepoints with a nonzero canonical combining class, ascending.\n")
    w(f"pub(super) static COMBINING_CLASSES: [(u32, u32, u8); {len(classes)}] = [\n")
    w(rows(classes, lambda x: f"(0x{x[0]:X}, 0x{x[1]:X}, {x[2]}),", 5) + "\n];\n\n")
    w("/// `(first, second, composite)`: the pairs NFC composes, ascending by pair.\n")
    w(f"pub(super) static PRIMARY_COMPOSITES: [(u32, u32, u32); {len(composites)}] = [\n")
    w(rows(composites, lambda x: f"(0x{x[0]:X}, 0x{x[1]:X}, 0x{x[2]:X}),", 4) + "\n];\n")


if __name__ == "__main__":
    main()
