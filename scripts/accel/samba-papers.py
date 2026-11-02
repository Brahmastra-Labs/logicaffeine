#!/usr/bin/env python3
"""The pinned sources of benchmarks/PREPARE_FOR_SAMBA.md: their captures under external/ and the text searched in them.

    samba-papers.py text            extract each capture's searchable text (pdftotext for a PDF, the visible text of a
                                    page for HTML, the title and abstract of a Semantic Scholar record `s2-*.json`;
                                    a Markdown, JSON or source capture is its own text)
    samba-papers.py pin             fill each empty sha256 cell of benchmarks/samba_papers.tsv from its capture
    samba-papers.py check           every capture exists and matches its pinned digest

The table is the record: one row per capture (a source may have several), with the source's title, authors, venue,
year, a stable locator, the capture date, the capture's path and SHA-256, and the path of its searchable text.
`samba_papers_lock` holds the book's bibliography and every number it attributes to a source to this table.
"""
import hashlib
import json
import re
import subprocess
import sys
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TABLE = ROOT / "benchmarks" / "samba_papers.tsv"


def rows():
    lines = TABLE.read_text(encoding="utf-8").splitlines()
    head = lines[0].split("\t")
    return head, [dict(zip(head, line.split("\t"))) for line in lines[1:] if line.strip()]


def write(head, table):
    TABLE.write_text("\n".join(["\t".join(head)] + ["\t".join(r[h] for h in head) for r in table]) + "\n", encoding="utf-8")


class Visible(HTMLParser):
    """A page's visible text: scripts and styles dropped, a line break at every block element."""

    def __init__(self):
        super().__init__()
        self.out, self.skip = [], 0

    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style", "noscript"):
            self.skip += 1
        if tag in ("p", "br", "li", "h1", "h2", "h3", "h4", "tr", "div", "td"):
            self.out.append("\n")

    def handle_endtag(self, tag):
        if tag in ("script", "style", "noscript"):
            self.skip -= 1

    def handle_data(self, data):
        if not self.skip:
            self.out.append(data)


def extract(capture: Path, text: Path):
    if capture == text:
        return
    if capture.suffix == ".pdf":
        subprocess.run(["pdftotext", "-layout", str(capture), str(text)], check=True)
    elif capture.suffix == ".html":
        p = Visible()
        p.feed(capture.read_text(encoding="utf-8", errors="replace"))
        text.write_text(re.sub(r"\n\s*\n+", "\n\n", "".join(p.out)), encoding="utf-8")
    elif capture.name.startswith("s2-") and capture.suffix == ".json":
        # A Semantic Scholar record of a paper the publisher keeps closed: its title and abstract, one paragraph each.
        record = json.loads(capture.read_text(encoding="utf-8"))
        text.write_text(f"{record['title']}\n\n{record['abstract']}\n", encoding="utf-8")
    elif capture.name.endswith(".tar.gz"):
        if not text.is_file():
            sys.exit(f"{text}: unpack {capture} beside it")
    elif not text.is_file():
        # A capture no extractor reads (a pull request's JSON) carries its text beside it, written when it was taken.
        sys.exit(f"{capture}: no text extraction for this kind of capture, and no {text} beside it")


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    head, table = rows()
    if cmd == "text":
        for r in table:
            extract(ROOT / r["capture"], ROOT / r["text"])
    elif cmd == "pin":
        for r in table:
            if not r["sha256"]:
                r["sha256"] = hashlib.sha256((ROOT / r["capture"]).read_bytes()).hexdigest()
        write(head, table)
    elif cmd == "check":
        bad = [r["capture"] for r in table
               if not (ROOT / r["capture"]).is_file()
               or hashlib.sha256((ROOT / r["capture"]).read_bytes()).hexdigest() != r["sha256"]]
        if bad:
            sys.exit("captures missing or changed: " + ", ".join(bad))
        print(f"{len(table)} captures match their pins")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
