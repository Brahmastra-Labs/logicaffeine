# Competition Guides — reference notes

Curated structural notes on the best programming-language guides, captured to audit the LOGOS
Syntax Guide (`apps/logicaffeine_web/src/ui/pages/guide/`) against the field. These are **summaries +
source links**, not verbatim copies of copyrighted books — enough to compare structure, pedagogy,
and QOL features.

Envy items (things they do that we should consider) are distilled in `../COMPETITION_DISCOVERIES.md`.

## Full scrapes (`scrape.py`)

`python3 scrape.py [guide ...]` downloads the **full source content** of the openly-licensed guides
into per-guide subdirectories, each with its LICENSE and a `MANIFEST.md`. Only sources whose license
clearly permits copying are bulk-copied:

| Dir | Guide | License | Source |
|-----|-------|---------|--------|
| `rust-book/` | The Rust Programming Language | MIT OR Apache-2.0 | github.com/rust-lang/book (`src/`, 111 md files) |
| `go-tour/` | A Tour of Go | BSD-3-Clause | github.com/golang/website (`_content/tour/`, lessons + code) |

## Summary-only (notes + links, no bulk copy)

These docs carry **no clear permissive license** on their content, so we keep only curated notes +
source links (fair use), not full copies:

| File | Guide | Why it's a comparable |
|------|-------|------------------------|
| [gleam-tour.md](gleam-tour.md) | Gleam Language Tour | Compiles-as-you-type in-browser; modern statically-typed FP onboarding |
| [inform7.md](inform7.md) | Inform 7 docs | The closest spiritual comparable: a **natural-language** programming language |

Hand notes for the full-scrape guides also live alongside: [rust-book.md](rust-book.md),
[go-tour.md](go-tour.md).

Captured 2026-06-25 during the `/loop` syntax-guide audit.
