#!/usr/bin/env python3
"""Generate `crates/logicaffeine_language/src/lgx/entities.rs` — the WHATWG named
character references.

The list is not ours to invent: it is fixed by the HTML standard, and Python's
`html.entities.html5` IS that list. Generating rather than hand-typing is the whole point —
2,125 names with 93 of them mapping to more than one code point is exactly the shape of table
that is right in the entries someone checked and wrong in the rest.

Only the names spelled WITH a trailing `;` are emitted. The semicolon-less legacy forms
(`&amp` without the `;`) are valid only in specific parser states this tokenizer does not
have, and accepting them here would decode text that a browser leaves alone.

Run: python3 scripts/dev/gen-html-entities.py
"""
import html.entities

OUT = "crates/logicaffeine_language/src/lgx/entities.rs"

named = sorted((k[:-1], v) for k, v in html.entities.html5.items() if k.endswith(";"))
longest = max(len(k) for k, _ in named)


def rust_str(s: str) -> str:
    out = []
    for ch in s:
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ch == "\n":
            out.append("\\n")
        elif ord(ch) < 0x20 or ord(ch) == 0x7F or ord(ch) > 0x7E:
            out.append(f"\\u{{{ord(ch):X}}}")
        else:
            out.append(ch)
    return '"' + "".join(out) + '"'


lines = [
    "//! The WHATWG named character references — GENERATED, do not hand-edit.",
    "//!",
    "//! Regenerate with `python3 scripts/dev/gen-html-entities.py`; `html_entity_table_lock` fails if",
    "//! this file and that generator disagree.",
    "//!",
    "//! ★ WHY THE WHOLE LIST. The tokenizer used to know fifteen names and pass everything else",
    "//! through as literal text, so `&eacute;` painted the six characters `&eacute;` onto the page",
    "//! instead of `é` — a silent wrong render, in ordinary prose, with nothing anywhere reporting",
    "//! it. Ninety-three of these names map to MORE THAN ONE code point, which is why the value is a",
    "//! string and not a `char`.",
    "",
    "/// The longest name in [`NAMED`], excluding the `&` and `;`. The tokenizer scans no further",
    "/// than this looking for the closing `;`, so a stray `&` in prose costs a bounded look-ahead.",
    f"pub const LONGEST_NAME: usize = {longest};",
    "",
    "/// Every named reference the HTML standard defines, sorted by name for binary search.",
    "pub static NAMED: &[(&str, &str)] = &[",
]
for name, value in named:
    lines.append(f"    ({rust_str(name)}, {rust_str(value)}),")
lines += [
    "];",
    "",
    "/// The text a named reference stands for, or `None` when the name is not one the standard",
    "/// defines (in which case the source text was never a reference and must survive as typed).",
    "pub fn named(name: &str) -> Option<&'static str> {",
    "    NAMED.binary_search_by_key(&name, |(n, _)| *n).ok().map(|i| NAMED[i].1)",
    "}",
    "",
]
with open(OUT, "w") as f:
    f.write("\n".join(lines))
print(f"wrote {OUT}: {len(named)} names, longest {longest}")
