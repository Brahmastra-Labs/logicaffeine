#!/usr/bin/env python3
"""Assemble the Logicaffeine handbook: one Markdown book from the documents a reader already has.

    handbook.py [--with-source] [--out PATH]    (default PATH: target/docs/LOGOS_DOCUMENTATION.md)

The book is the README's overview, Getting started, every guide, every crate's README, the quick guide,
the error catalog and the project documents, in reading order, under one table of contents. Each document keeps its words; the assembler only nests its headings under its
part, turns links between included documents into links inside the book, and turns every other relative
link into an absolute GitHub URL, so the single file reads the same anywhere it is opened.

`--with-source` appends the core source files named in SOURCE, each captioned by its own `//!` header,
for pasting the whole picture into a model's context.

The assembler refuses rather than guesses: a declared input that is missing, a guide or crate README the
tables leave out, a fence left open, or a link to an anchor the book does not have stops it with the name
of what is wrong.
"""
import argparse
import os
import re
import subprocess
import sys
import tomllib

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
REPO = "https://github.com/Brahmastra-Labs/logicaffeine"

# (part title, [(document, the README sections it contributes, or None for the whole document)])
PARTS = [
    ("Start here", [
        ("README.md", ["What is it?", "What's inside"]),
        ("docs/getting-started.md", None),
    ]),
    ("The language", [
        ("docs/imperative-mode.md", None),
        ("docs/frontend.md", None),
        ("docs/web.md", None),
        ("docs/lgx-reference.md", None),
        ("docs/lgx-stdlib.md", None),
        ("docs/concurrency.md", None),
    ]),
    ("Logic and proof", [
        ("docs/logic-mode.md", None),
        ("docs/proof-and-verification.md", None),
    ]),
    ("Running it", [
        ("docs/execution-and-performance.md", None),
        ("docs/cli.md", None),
        ("docs/studio-and-learn.md", None),
    ]),
    ("Architecture", [
        ("docs/architecture.md", None),
        ("@crate-table", None),
    ]),
    ("The crates", [
        ("@crate-readmes", None),
    ]),
    ("Reference", [
        ("LOGOS_QUICKGUIDE.md", None),
        ("docs/ERROR_CATALOG.md", None),
    ]),
    ("Project", [
        ("docs/project/FAQ.md", None),
        ("docs/project/VERSIONING.md", None),
        ("docs/project/CHALLENGE.md", None),
    ]),
]

# The core of the pipeline, front to back: tokens, the two ASTs, English to logic, the compiler's
# entry points, the VM, the kernel, and the runtime values every tier shares.
SOURCE = [
    "crates/logicaffeine_base/src/lib.rs",
    "crates/logicaffeine_language/src/lib.rs",
    "crates/logicaffeine_language/src/token.rs",
    "crates/logicaffeine_language/src/ast/mod.rs",
    "crates/logicaffeine_language/src/ast/logic.rs",
    "crates/logicaffeine_language/src/ast/stmt.rs",
    "crates/logicaffeine_language/src/compile.rs",
    "crates/logicaffeine_language/src/transpile.rs",
    "crates/logicaffeine_compile/src/lib.rs",
    "crates/logicaffeine_compile/src/vm/mod.rs",
    "crates/logicaffeine_kernel/src/lib.rs",
    "crates/logicaffeine_kernel/src/term.rs",
    "crates/logicaffeine_data/src/lib.rs",
    "crates/logicaffeine_runtime/src/lib.rs",
]

FENCE = re.compile(r"^\s*(```+|~~~+)")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
LINK = re.compile(r"(!?)\[((?:[^\[\]]|\[[^\]]*\])*)\]\(([^)\s]+)(\s+\"[^\"]*\")?\)")


class Refusal(Exception):
    pass


def strip_code(text):
    """The text with fenced blocks and inline code spans blanked, so only prose is inspected."""
    out = []
    in_fence = False
    for line in text.splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            out.append("")
        else:
            out.append("" if in_fence else re.sub(r"``[^`\n]+``|`[^`\n]+`", "", line))
    return "\n".join(out)


def read(rel):
    path = os.path.join(ROOT, rel)
    if not os.path.isfile(path):
        raise Refusal(f"a declared input does not exist: {rel}")
    with open(path, encoding="utf-8") as f:
        return f.read()


def workspace():
    with open(os.path.join(ROOT, "Cargo.toml"), "rb") as f:
        return tomllib.load(f)


def version():
    return workspace()["workspace"]["package"]["version"]


def commit():
    try:
        out = subprocess.run(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"], capture_output=True, text=True)
        return out.stdout.strip() if out.returncode == 0 else ""
    except OSError:
        return ""


def member_dirs():
    """Every workspace member directory with a README, by expanding `members`."""
    dirs = []
    for m in workspace()["workspace"]["members"]:
        if m.endswith("/*"):
            parent = os.path.join(ROOT, m[:-2])
            for d in sorted(os.listdir(parent)):
                if os.path.isfile(os.path.join(parent, d, "Cargo.toml")):
                    dirs.append(f"{m[:-2]}/{d}")
        else:
            dirs.append(m)
    return [d for d in dirs if os.path.isfile(os.path.join(ROOT, d, "README.md"))]


def package(dir_):
    with open(os.path.join(ROOT, dir_, "Cargo.toml"), "rb") as f:
        return tomllib.load(f)["package"]


def tiers():
    """Crate short name → tier, from the tier table in docs/architecture.md (held to the manifests by
    tier_map_lock)."""
    out = {}
    for line in read("docs/architecture.md").splitlines():
        m = re.match(r"^\|\s*\[([a-z_]+)\]\([^)]*\)\s*\|\s*(\d+)\s*\|", line)
        if m:
            out[m.group(1)] = int(m.group(2))
    return out


def ordered_crates():
    """Crate README directories in tier order, then by name; members the tier table omits come last."""
    t = tiers()
    rows = []
    for d in member_dirs():
        short = package(d)["name"].removeprefix("logicaffeine-").replace("-", "_")
        rows.append((t.get(short, 99), short, d))
    return [d for _, _, d in sorted(rows)]


def split_readme_sections(text, wanted):
    """The README's `## <title>` sections, in the order asked, each up to the next `##` or rule."""
    lines = text.splitlines()
    out = []
    for title in wanted:
        start = next((i for i, l in enumerate(lines) if l.strip() == f"## {title}"), None)
        if start is None:
            raise Refusal(f"README.md has no section `## {title}`")
        end = start + 1
        in_fence = False
        while end < len(lines):
            if FENCE.match(lines[end]):
                in_fence = not in_fence
            elif not in_fence and (lines[end].startswith("## ") or lines[end].strip() == "---"):
                break
            end += 1
        out.append("\n".join(lines[start:end]).strip("\n"))
    return "\n\n".join(out) + "\n"


def strip_footer(text):
    """Drop a document's closing `---` rule and navigation line."""
    lines = text.rstrip("\n").splitlines()
    if len(lines) >= 2 and lines[-2].strip() == "---" and lines[-1].lstrip().startswith("["):
        lines = lines[:-2]
    return "\n".join(lines).rstrip("\n") + "\n"


def slug(title):
    """GitHub's anchor for a heading: the rendered text, lower-cased, with everything but letters, digits,
    `_`, `-` and spaces dropped, and each space a hyphen."""
    text = re.sub(r"`([^`]*)`", r"\1", title)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = text.replace("*", "").strip().lower()
    text = re.sub(r"[^\w\- ]", "", text, flags=re.UNICODE)
    return text.replace(" ", "-")


class Book:
    def __init__(self):
        self.lines = []
        self.slugs = {}
        self.toc = []
        self.chapters = {}

    def unique(self, title):
        base = slug(title)
        n = self.slugs.get(base, 0)
        self.slugs[base] = n + 1
        return base if n == 0 else f"{base}-{n}"


def top_level(text):
    """The shallowest heading level a document uses outside its code fences."""
    levels = []
    in_fence = False
    for line in text.splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
        elif not in_fence and (m := HEADING.match(line)):
            levels.append(len(m.group(1)))
    return min(levels) if levels else 1


def demote(text, rel, depth, book, anchors):
    """Nest the document's headings `depth` levels down and record each heading's anchor, so links into
    the document can be pointed at the book."""
    out = []
    in_fence = False
    first = None
    for line in text.splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            out.append(line)
            continue
        m = None if in_fence else HEADING.match(line)
        if m:
            level = min(len(m.group(1)) + depth, 6)
            title = m.group(2)
            anchor = book.unique(title)
            anchors[(rel, slug(title))] = anchor
            if first is None:
                first = anchor
                anchors[(rel, "")] = anchor
            out.append(f"{'#' * level} {title}")
        else:
            out.append(line)
    if in_fence:
        raise Refusal(f"{rel} leaves a code fence open")
    return "\n".join(out) + "\n", first


def resolve(rel, target):
    """The repository path a relative link in `rel` names, or None when it leaves the repository."""
    base = os.path.dirname(rel)
    path = os.path.normpath(os.path.join(base, target))
    return None if path.startswith("..") else path.replace(os.sep, "/")


def rewrite_links(text, rel, anchors, included, partial):
    """Point every link at the book or at GitHub. Returns the text and the in-book anchors it cites."""
    cited = []

    def one(m):
        bang, label, target, title = m.group(1), m.group(2), m.group(3), m.group(4) or ""
        if re.match(r"^[a-z][a-z0-9+.-]*:", target):
            prefix = f"{REPO}/blob/main/"
            if target.startswith(prefix) and not bang:
                path, _, frag = target[len(prefix):].partition("#")
                if path in included:
                    anchor = anchors.get((path, frag.lower())) if frag else anchors.get((path, ""))
                    if anchor is not None:
                        cited.append(anchor)
                        return f"[{label}](#{anchor})"
            return m.group(0)
        if target.startswith("#"):
            anchor = anchors.get((rel, target[1:].lower()))
            if anchor is None:
                raise Refusal(f"{rel} links to #{target[1:]}, which it does not have")
            cited.append(anchor)
            return f"{bang}[{label}](#{anchor}{title})"
        path, _, frag = target.partition("#")
        repo_path = resolve(rel, path)
        if repo_path is None:
            raise Refusal(f"{rel} links outside the repository: {target}")
        if repo_path in included and not bang:
            anchor = anchors.get((repo_path, frag.lower())) if frag else anchors.get((repo_path, ""))
            if anchor is not None:
                cited.append(anchor)
                return f"[{label}](#{anchor})"
            if repo_path not in partial:
                raise Refusal(f"{rel} links to {target}, and the book has no such heading")
        if not os.path.exists(os.path.join(ROOT, repo_path)):
            raise Refusal(f"{rel} links to {target}, which does not exist")
        kind = "tree" if os.path.isdir(os.path.join(ROOT, repo_path)) else "blob"
        url = f"{REPO}/{kind}/main/{repo_path}" + (f"#{frag}" if frag else "")
        if bang:
            url += "?raw=true"
        return f"{bang}[{label}]({url}{title})"

    def prose(chunk):
        spans = []

        def mask(m):
            spans.append(m.group(0))
            return f"\x00{len(spans) - 1}\x00"

        masked = re.sub(r"``[^`\n]+``|`[^`\n]+`", mask, chunk)
        linked = LINK.sub(one, masked)
        return re.sub(r"\x00(\d+)\x00", lambda m: spans[int(m.group(1))], linked)

    out = []
    pending = []
    in_fence = False
    for line in text.splitlines():
        if FENCE.match(line):
            if not in_fence and pending:
                out.append(prose("\n".join(pending)))
                pending = []
            in_fence = not in_fence
            out.append(line)
        elif in_fence:
            out.append(line)
        else:
            pending.append(line)
    if pending:
        out.append(prose("\n".join(pending)))
    return "\n".join(out) + "\n", cited


def crate_table():
    t = tiers()
    rows = ["# The crates at a glance", "",
            "Every workspace member, by dependency tier: a crate depends only on crates in lower tiers.", "",
            "| Crate | Tier | What it is |", "|-------|------|------------|"]
    for d in ordered_crates():
        p = package(d)
        short = p["name"].removeprefix("logicaffeine-").replace("-", "_")
        tier = t.get(short)
        desc = p.get("description", "").replace("|", "\\|")
        rows.append(f"| [`{p['name']}`]({d}/README.md) | {tier if tier is not None else '—'} | {desc} |")
    return "\n".join(rows) + "\n"


def first_doc_line(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as f:
        for line in f:
            s = line.strip()
            if s.startswith("//!"):
                text = s[3:].strip()
                if text:
                    return text
            elif s and not s.startswith("#!["):
                break
    return ""


def source_appendix(book):
    book.unique("Appendix: the core source")
    out = ["# Appendix: the core source", "",
           "The pipeline's core files, front to back, each introduced by its own header.", ""]
    for rel in SOURCE:
        body = read(rel)
        caption = first_doc_line(rel)
        title = f"`{rel}`"
        book.unique(title)
        out += [f"## {title}", "", caption, "", "```rust", body.rstrip("\n"), "```", ""]
    return "\n".join(out) + "\n"


def coverage():
    """Every guide, project document and crate README must be in the book."""
    declared = {doc for _, docs in PARTS for doc, _ in docs if not doc.startswith("@")}
    want = {f"docs/{f}" for f in os.listdir(os.path.join(ROOT, "docs")) if f.endswith(".md") and f != "README.md"}
    want |= {f"docs/project/{f}" for f in os.listdir(os.path.join(ROOT, "docs/project")) if f.endswith(".md")}
    missing = sorted(want - declared)
    if missing:
        raise Refusal(f"the handbook's PARTS leave out {missing}")


def build(with_source):
    coverage()
    book = Book()
    book.unique("The Logicaffeine Handbook")
    book.unique("Contents")
    documents = []
    for part, docs in PARTS:
        for doc, sections in docs:
            if doc == "@crate-readmes":
                for d in ordered_crates():
                    documents.append((part, f"{d}/README.md", strip_footer(read(f"{d}/README.md"))))
            elif doc == "@crate-table":
                documents.append((part, "@crate-table", crate_table()))
            elif sections:
                documents.append((part, doc, split_readme_sections(read(doc), sections)))
            else:
                documents.append((part, doc, strip_footer(read(doc))))

    anchors = {}
    parts_seen = []
    rendered = []
    for part, rel, text in documents:
        if not parts_seen or parts_seen[-1][0] != part:
            anchor = book.unique(part)
            parts_seen.append((part, anchor, []))
            rendered.append((None, f"# {part}\n"))
        body, first = demote(text, rel, 2 - top_level(text), book, anchors)
        title = next((l.lstrip("#").strip() for l in body.splitlines() if HEADING.match(l)), rel)
        parts_seen[-1][2].append((title, first))
        rendered.append((rel, body))

    included = {rel for _, rel, _ in documents if not rel.startswith("@")}
    partial = {doc for _, docs in PARTS for doc, sections in docs if sections}
    cited = []
    chunks = []
    for rel, body in rendered:
        if rel is None:
            chunks.append(body)
            continue
        text, used = rewrite_links(body, rel if not rel.startswith("@") else "README.md", anchors, included, partial)
        cited += used
        chunks.append(text)

    ver = version()
    sha = commit()
    head = [f"# The Logicaffeine Handbook", "",
            f"LOGOS {ver}" + (f" · generated from commit `{sha}`" if sha else "") +
            " · regenerate with `./scripts/generate-docs.sh`", "",
            "Everything a reader needs in one file: what LOGOS is, how to start, every guide, every crate, "
            "and the reference material, in reading order. The documents it is built from are the source; "
            "this book changes when they do.", "", "## Contents", ""]
    for part, anchor, chapters in parts_seen:
        head.append(f"- [{part}](#{anchor})")
        for title, first in chapters:
            clean = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", title)
            head.append(f"  - [{clean}](#{first})")
    tail = source_appendix(book) if with_source else ""
    text = "\n".join(head) + "\n\n" + "\n".join(c.rstrip("\n") + "\n" for c in chunks) + ("\n" + tail if tail else "")

    headings = set()
    in_fence = False
    counts = {}
    for line in text.splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        m = None if in_fence else HEADING.match(line)
        if m:
            base = slug(m.group(2))
            n = counts.get(base, 0)
            counts[base] = n + 1
            headings.add(base if n == 0 else f"{base}-{n}")
    if in_fence:
        raise Refusal("the book leaves a code fence open")
    relative = sorted({t for t in re.findall(r"\]\(([^)\s#][^)\s]*)\)", strip_code(text)) if not re.match(r"^[a-z][a-z0-9+.-]*:", t)})
    if relative:
        raise Refusal(f"the book keeps relative links: {relative[:12]}")
    dangling = sorted({a for a in re.findall(r"\]\(#([^)\s]+)\)", text) if a not in headings})
    if dangling:
        raise Refusal(f"the book links to anchors it does not have: {dangling[:12]}")
    return text


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--with-source", action="store_true", help="append the core source files")
    ap.add_argument("--out", help="where to write (default: target/docs/LOGOS_DOCUMENTATION.md)")
    ap.add_argument("--root", help="assemble from this tree instead of the repository holding the script")
    args = ap.parse_args()
    global ROOT
    if args.root:
        ROOT = os.path.abspath(args.root)
    if not args.out:
        args.out = os.path.join(ROOT, "target", "docs", "LOGOS_DOCUMENTATION.md")
    try:
        text = build(args.with_source)
    except Refusal as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(text)
    print(f"wrote {os.path.relpath(args.out, ROOT) if args.out.startswith(ROOT) else args.out} "
          f"({len(text.splitlines())} lines, {len(text.encode()) // 1024} KiB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
