#!/usr/bin/env python3
"""Scrape full, openly-licensed language-guide content into COMPETITION_GUIDES/.

Targets sources whose licenses explicitly permit copying (MIT / Apache-2.0 / BSD),
fetches their real source markdown (not rendered HTML), and writes a per-guide
MANIFEST with source + license attribution. Polite: small delay between requests,
identifying User-Agent, idempotent (re-runs overwrite).

Usage:  python3 COMPETITION_GUIDES/scrape.py [guide ...]
        (no args = all guides)
"""
import json
import os
import sys
import time
import urllib.request
import urllib.error

ROOT = os.path.dirname(os.path.abspath(__file__))
UA = {"User-Agent": "logicaffeine-guide-audit (local reference scrape)"}
DELAY = 0.15  # seconds between requests — be polite


def fetch(url, binary=False):
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req, timeout=30) as r:
        data = r.read()
    time.sleep(DELAY)
    return data if binary else data.decode("utf-8", "replace")


def write(relpath, text):
    path = os.path.join(ROOT, relpath)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return len(text.encode("utf-8"))


def gh_contents(owner, repo, path, ref="HEAD"):
    """List a directory via the GitHub contents API (recurses into subdirs)."""
    url = f"https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={ref}"
    items = json.loads(fetch(url))
    out = []
    for it in items:
        if it["type"] == "file":
            out.append(it)
        elif it["type"] == "dir":
            out.extend(gh_contents(owner, repo, it["path"], ref))
    return out


# ---------------------------------------------------------------------------

def scrape_rust_book():
    """rust-lang/book — MIT OR Apache-2.0. Full text is markdown under src/."""
    base = "https://raw.githubusercontent.com/rust-lang/book/main"
    summary = fetch(f"{base}/src/SUMMARY.md")
    write("rust-book/src/SUMMARY.md", summary)
    # Pull every `(something.md)` link out of the SUMMARY.
    import re
    md_files = sorted(set(re.findall(r"\(([^)]+\.md)\)", summary)))
    total, ok, miss = 0, 0, []
    for rel in md_files:
        try:
            text = fetch(f"{base}/src/{rel}")
            total += write(f"rust-book/src/{rel}", text)
            ok += 1
        except urllib.error.HTTPError as e:
            miss.append(f"{rel} ({e.code})")
    for lic in ("LICENSE-MIT", "LICENSE-APACHE"):
        try:
            write(f"rust-book/{lic}", fetch(f"{base}/{lic}"))
        except Exception:
            pass
    write("rust-book/MANIFEST.md",
          f"# Rust Book (full source)\n\n"
          f"- Source: https://github.com/rust-lang/book (branch main, src/)\n"
          f"- License: MIT OR Apache-2.0 (see LICENSE-MIT / LICENSE-APACHE)\n"
          f"- Chapters fetched: {ok}/{len(md_files)} markdown files, {total} bytes\n"
          + (f"- Missing: {', '.join(miss)}\n" if miss else ""))
    return ok, total, miss


def scrape_go_tour():
    """golang/website — BSD-3-Clause. Tour lessons are .article + code under _content/tour/."""
    files = gh_contents("golang", "website", "_content/tour")
    total, ok = 0, 0
    for it in files:
        try:
            text = fetch(it["download_url"])
        except Exception:
            continue
        rel = it["path"].replace("_content/tour/", "")
        total += write(f"go-tour/{rel}", text)
        ok += 1
    try:
        write("go-tour/LICENSE", fetch("https://raw.githubusercontent.com/golang/website/master/LICENSE"))
    except Exception:
        pass
    write("go-tour/MANIFEST.md",
          f"# A Tour of Go (full content)\n\n"
          f"- Source: https://github.com/golang/website (_content/tour/)\n"
          f"- License: BSD-3-Clause (see LICENSE)\n"
          f"- Files fetched: {ok} (.article lessons + code snippets), {total} bytes\n")
    return ok, total, []


# NOTE: Gleam tour (gleam-lang/language-tour) and Inform 7 docs are intentionally NOT
# bulk-copied — neither carries a clear permissive license on the docs content, so we keep
# only the summary notes (gleam-tour.md, inform7.md) + source links, which is fair use.

GUIDES = {
    "rust-book": scrape_rust_book,
    "go-tour": scrape_go_tour,
}


def main():
    want = sys.argv[1:] or list(GUIDES)
    for name in want:
        fn = GUIDES.get(name)
        if not fn:
            print(f"SKIP {name}: unknown guide")
            continue
        try:
            ok, total, miss = fn()
            print(f"OK   {name}: {ok} files, {total} bytes"
                  + (f", missing {len(miss)}" if miss else ""))
        except Exception as e:
            print(f"FAIL {name}: {type(e).__name__}: {e}")


if __name__ == "__main__":
    main()
