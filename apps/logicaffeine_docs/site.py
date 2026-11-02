#!/usr/bin/env python3
"""Build docs.logicaffeine.com: document the curated crates, then assemble the site.

    site.py build    --source <workspace> --out <site dir> --routes <routes.toml> <crate>...
    site.py document --source <workspace> [--dry-run] <crate>...
    site.py assemble --source <workspace> --doc <rustdoc output> --out <site dir> --routes <routes.toml> <crate>...

`document` runs one `cargo doc --no-deps --lib` over the listed crates. rustdoc links another
crate's items only when that crate's doc directory exists as rustdoc starts. With `--no-deps`,
cargo does not order one doc job after another, and it deletes a crate's doc directory just
before documenting it — so a crate documented before, or alongside, a dependency came out with
every type it named from that dependency unlinked, and an explicit link to it fell back to a
raw, dead URL. `document` therefore creates every listed crate's directory first and runs the
rustdoc jobs one at a time: a directory is then only ever missing while no other rustdoc runs.
A parallel `cargo check` over the same crates first builds everything those jobs depend on, so
only rustdoc itself is serialized. It stays one invocation of each because splitting it would
unify features per invocation, documenting a dependency without the features its dependents
turn on.

`assemble` ships everything rustdoc wrote, byte for byte, plus a shell of files rustdoc does not
write: the landing page, a 404 page, robots.txt, the sitemap, and the map for agents — /llms.txt,
the front door (what LOGOS is, how the site is addressed, "start here" routes from routes.toml,
every crate with its top-level modules), and /llms/<crate>.txt, every public module and item of a
crate with its one-line summary and address, read from rustdoc's own module pages. Cloudflare Pages answers an
address it has no file for with the nearest 404.html — or, in a deployment without one, with
/index.html and a 200, as if the site were a single-page app. So nothing is hand-picked out of
rustdoc's output: a file left behind would turn into a copy of the landing page wherever a page
asks for it.

The site is refused, and none is left behind, when
  * rustdoc documented a crate that is not listed, or did not document one that is — the
    search index and implementor lists would name pages the site does not ship;
  * a shell file would overwrite something rustdoc wrote;
  * a crate's agent map misses a page rustdoc wrote for it, or a route or map link leads to a page
    the site does not ship;
  * the workspace names no documentation URL, or a listed crate has no description;
  * any page references a file the site does not ship. Two kinds of reference are rustdoc's to
    answer for, not ours: the implementor script a trait page loads, which rustdoc writes only
    when other documented crates implement the trait, and links inside documentation rustdoc
    copies from a trait defined outside the site (a dependency's blanket impl), which are
    relative to that crate's own docs.
"""

import argparse
import html
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import quote, unquote, urlsplit
from xml.sax.saxutils import escape as xml_escape

SITEMAP_NS = "http://www.sitemaps.org/schemas/sitemap/0.9"
SITEMAP_LIMIT = 50_000
LIB_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
URL_SAFE = "/!$&'()*+,;=:@-._~"
REFRESH = re.compile(rb"<meta\s[^>]*http-equiv\s*=\s*[\"']?refresh", re.IGNORECASE)
OPTIONAL_SCRIPT_DIRS = ("/trait.impl/", "/type.impl/")
KIND_LABELS = {
    "constant": "const",
    "attr": "attribute macro",
    "derive": "derive macro",
    "traitalias": "trait alias",
    "foreigntype": "extern type",
}
MARKDOWN_LINK = re.compile(r"\]\(([^)\s]+)\)")

STYLE = """\
        body { font-family: system-ui, sans-serif; max-width: 800px; margin: 40px auto; padding: 0 20px; color: #222; }
        h1 { color: #333; }
        ul { list-style: none; padding: 0; }
        li { margin: 10px 0; line-height: 1.5; }
        a { color: #4a6ee0; text-decoration: none; }
        a:hover { text-decoration: underline; }
        .links { margin: 16px 0 28px; }
        .links a { margin-right: 16px; }
        .agents { background: #f4f6fb; border-left: 3px solid #4a6ee0; padding: 10px 14px; }
        h3 { margin: 20px 0 4px; color: #444; font-size: 1em; }
        ul.modules { margin: 4px 0 0 18px; font-size: 0.92em; }
        ul.modules li { margin: 3px 0; }"""


class Refused(Exception):
    pass


class Workspace:
    """The workspace as cargo reports it: each library's package, and the target directory."""

    def __init__(self, source):
        metadata = subprocess.run(
            [
                os.environ.get("CARGO", "cargo"),
                "metadata",
                "--no-deps",
                "--format-version",
                "1",
                "--manifest-path",
                str(source / "Cargo.toml"),
            ],
            capture_output=True,
            text=True,
        )
        if metadata.returncode != 0:
            raise Refused(f"cargo metadata failed:\n{metadata.stderr}")
        data = json.loads(metadata.stdout)
        self.source = source
        self.doc = Path(data["target_directory"]) / "doc"
        self.packages = {}
        for package in data["packages"]:
            for target in package["targets"]:
                if LIB_KINDS.intersection(target["kind"]):
                    self.packages[target["name"].replace("-", "_")] = package

    def package(self, crate):
        if crate not in self.packages:
            raise Refused(f"{crate} is not a library in the workspace at {self.source}")
        return self.packages[crate]

    def descriptions(self, crates):
        blurbs = {}
        for crate in crates:
            description = (self.package(crate).get("description") or "").strip()
            if not description:
                raise Refused(f"{crate}'s manifest has no description — the landing page lists one for every crate")
            blurbs[crate] = description
        return blurbs


def check_listed_once(crates):
    repeated = sorted({c for c in crates if crates.count(c) > 1})
    if repeated:
        raise Refused(f"listed more than once: {', '.join(repeated)}")


def cargo_commands(workspace, crates):
    """The parallel `cargo check` that builds what rustdoc needs, then the serial `cargo doc`."""
    check_listed_once(crates)
    selection = ["--manifest-path", str(workspace.source / "Cargo.toml")]
    for crate in crates:
        selection += ["-p", workspace.package(crate)["name"]]
    cargo = os.environ.get("CARGO", "cargo")
    return [
        [cargo, "check", "--lib", *selection],
        [cargo, "doc", "--no-deps", "--lib", "--jobs", "1", *selection],
    ]


def document(workspace, crates):
    check, doc = cargo_commands(workspace, crates)
    if subprocess.run(check, cwd=workspace.source).returncode != 0:
        raise Refused("cargo check failed")
    shutil.rmtree(workspace.doc, ignore_errors=True)
    for crate in crates:
        (workspace.doc / crate).mkdir(parents=True)
    if subprocess.run(doc, cwd=workspace.source).returncode != 0:
        raise Refused("cargo doc failed")


def site_origin(source):
    manifest_path = source / "Cargo.toml"
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    url = manifest.get("workspace", {}).get("package", {}).get("documentation")
    if not url:
        raise Refused(f"{manifest_path} has no [workspace.package] documentation URL to serve the site at")
    parts = urlsplit(url)
    if parts.scheme not in ("http", "https") or not parts.netloc:
        raise Refused(f"the workspace documentation URL {url!r} is not an absolute http(s) URL")
    return url.rstrip("/")


def documented(doc):
    crates_js = doc / "crates.js"
    if not crates_js.is_file():
        raise Refused(f"{crates_js} is missing — is {doc} rustdoc output?")
    listing = re.search(r"window\.ALL_CRATES\s*=\s*(\[[^\]]*\])", crates_js.read_text(encoding="utf-8"))
    if not listing:
        raise Refused(f"cannot read the crate list from {crates_js}")
    return json.loads(listing.group(1))


def check_curation(doc, crates):
    check_listed_once(crates)
    have = set(documented(doc))
    extra = sorted(have - set(crates))
    if extra:
        raise Refused(
            f"rustdoc documented {', '.join(extra)}, which the site does not list — its search index "
            f"and implementor lists would name pages the site does not ship"
        )
    missing = sorted(set(crates) - have)
    if missing:
        raise Refused(f"listed but not documented by rustdoc: {', '.join(missing)}")
    for crate in crates:
        if not (doc / crate / "index.html").is_file():
            raise Refused(f"rustdoc lists {crate} but wrote no {crate}/index.html")


def canonical(rel):
    """The address Pages serves the file at `rel` under: it drops `.html` and `index.html`."""
    if rel == "index.html":
        return "/"
    if rel.endswith("/index.html"):
        return "/" + rel[: -len("index.html")]
    if rel.endswith(".html"):
        return "/" + rel[: -len(".html")]
    return "/" + rel


def is_redirect_stub(page):
    with open(page, "rb") as f:
        return REFRESH.search(f.read(4096)) is not None


def sitemap_urls(origin, doc, crates):
    """The landing page and every crate page that is not a redirect stub, at canonical addresses."""
    urls = [origin + "/"]
    for crate in crates:
        for page in sorted((doc / crate).rglob("*.html")):
            if not is_redirect_stub(page):
                urls.append(origin + quote(canonical(page.relative_to(doc).as_posix()), safe=URL_SAFE))
    return urls


def urlset(urls):
    entries = "".join(f"  <url><loc>{xml_escape(url)}</loc></url>\n" for url in urls)
    return f'<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="{SITEMAP_NS}">\n{entries}</urlset>\n'


def sitemap_files(origin, urls, limit):
    """One sitemap, or — past the protocol's per-file limit — an index over numbered parts."""
    if len(urls) <= limit:
        return {"sitemap.xml": urlset(urls)}
    parts = {
        f"sitemap-{n}.xml": urlset(urls[start : start + limit])
        for n, start in enumerate(range(0, len(urls), limit), start=1)
    }
    entries = "".join(f"  <sitemap><loc>{xml_escape(origin)}/{name}</loc></sitemap>\n" for name in parts)
    index = f'<?xml version="1.0" encoding="UTF-8"?>\n<sitemapindex xmlns="{SITEMAP_NS}">\n{entries}</sitemapindex>\n'
    return {"sitemap.xml": index, **parts}


def load_routes(path):
    """The "start here" routes: `[[route]]` entries of section, task, target (a rustdoc file) and
    note, grouped by section in the order sections first appear."""
    try:
        routes = tomllib.loads(path.read_text(encoding="utf-8")).get("route", [])
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise Refused(f"cannot read the routes in {path}: {error}")
    sections = {}
    for route in routes:
        for field in ("section", "task", "target", "note"):
            if not isinstance(route.get(field), str) or not route[field].strip():
                raise Refused(f"a route in {path} has no {field}: {route}")
        sections.setdefault(route["section"], []).append(route)
    return sections


def check_routes(doc, sections):
    for route in (r for routes in sections.values() for r in routes):
        page = doc / route["target"]
        if not route["target"].endswith(".html") or not page.is_file() or is_redirect_stub(page):
            raise Refused(f'the route "{route["task"]}" leads to {route["target"]}, a page the site does not ship')


class ItemTables(HTMLParser):
    """The item tables of a rustdoc module page, and the page's own source link. Each entry is an
    item's kind, name, address and summary; rustdoc writes the summary as the `<dd>` right after
    the item's `<dt>`, and writes no `<dd>` at all for an undocumented item — so a summary belongs
    only to the entry directly before it. Re-export tables are skipped: what they name is listed
    where it lives."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.entries = []
        self.source = None
        self.table = False
        self.naming = None
        self.awaiting = None
        self.summary = None

    def handle_starttag(self, tag, attrs):
        values = {name: value for name, value in attrs if value is not None}
        classes = values.get("class", "").split()
        if tag == "a" and "src" in classes and self.source is None and "href" in values:
            self.source = values["href"]
        if tag == "dl" and "item-table" in classes and "reexports" not in classes:
            self.table = True
        if not self.table:
            return
        if tag == "dt":
            self.awaiting = None
            self.naming = False
        elif tag == "a" and self.naming is False and classes and "href" in values:
            self.naming = {"kind": classes[0], "href": values["href"], "title": values.get("title", ""), "name": ""}
        elif tag == "dd" and self.awaiting is not None:
            self.summary = []
        elif tag == "code" and self.summary is not None:
            self.summary.append("`")

    def handle_data(self, data):
        if isinstance(self.naming, dict):
            self.naming["name"] += data
        if self.summary is not None:
            self.summary.append(data)

    def handle_endtag(self, tag):
        if not self.table:
            return
        if tag == "a" and isinstance(self.naming, dict):
            entry = self.naming
            entry["summary"] = None
            self.entries.append(entry)
            self.awaiting = entry
            self.naming = None
        elif tag == "dt":
            self.naming = None
        elif tag == "code" and self.summary is not None:
            self.summary.append("`")
        elif tag == "dd" and self.summary is not None:
            if self.awaiting is not None:
                self.awaiting["summary"] = " ".join("".join(self.summary).split()) or None
            self.summary = None
            self.awaiting = None
        elif tag == "dl":
            self.table = False
            self.awaiting = None


def entry_name(entry):
    title = entry["title"].split(" ", 1)[-1] if entry["title"] else entry["name"]
    return title.rsplit("::", 1)[-1].strip() or entry["name"].strip()


def site_target(page_rel, href):
    """The site path (relative to its root) an item-table href on page `page_rel` names, or None
    when it leaves the site."""
    parts = urlsplit(href)
    if parts.scheme or parts.netloc or not parts.path:
        return None
    path = normalize("/" + os.path.dirname(page_rel) + "/" + parts.path)
    return unquote(path[1:]) if path and path != "/" else None


def crate_map(doc, crate):
    """Every module of `crate`, depth first from its root: (module path, page, summary, entries,
    source link). Entries carry their resolved site `target`."""
    modules, seen = [], set()

    def visit(page, path, summary):
        if page in seen:
            return
        seen.add(page)
        tables = ItemTables()
        tables.feed((doc / page).read_text(encoding="utf-8", errors="replace"))
        tables.close()
        entries = []
        for entry in tables.entries:
            target = site_target(page, entry["href"])
            if target is not None:
                entries.append({**entry, "target": target, "label": KIND_LABELS.get(entry["kind"], entry["kind"])})
        source = site_target(page, tables.source) if tables.source else None
        modules.append((path, page, summary, entries, source))
        for entry in entries:
            if entry["kind"] == "mod" and entry["target"].startswith(f"{crate}/") and entry["target"].endswith("/index.html"):
                visit(entry["target"], f"{path}::{entry_name(entry)}", entry["summary"])

    visit(f"{crate}/index.html", crate, None)
    return modules


def check_map_complete(doc, crate, modules):
    covered = {page for _, page, _, _, _ in modules}
    covered |= {entry["target"] for _, _, _, entries, _ in modules for entry in entries}
    pages = {
        page.relative_to(doc).as_posix()
        for page in (doc / crate).rglob("*.html")
        if not is_redirect_stub(page)
    } - {f"{crate}/all.html"}
    missing = sorted(pages - covered)
    if missing:
        shown = "\n  ".join(missing[:50])
        raise Refused(
            f"the agent map of {crate} misses {len(missing)} page(s) rustdoc wrote — has rustdoc's "
            f"module markup changed?\n  {shown}"
        )


def page_url(origin, rel):
    return origin + quote(canonical(rel), safe=URL_SAFE)


def entry_line(origin, entry):
    line = f"- [{entry['label']} {entry_name(entry)}]({page_url(origin, entry['target'])})"
    return f"{line}: {entry['summary']}" if entry["summary"] else line


def map_counts(modules):
    items = {e["target"] for _, _, _, entries, _ in modules for e in entries if e["kind"] != "mod"}
    return len(modules) - 1, len(items)


def crate_map_text(origin, crate, blurb, modules):
    source = modules[0][4]
    lines = [
        f"# {crate}",
        "",
        f"> {blurb}",
        "",
        f"Every public module and item of `{crate}`, module by module, each with its one-line summary "
        "and its address. Item pages hold the full signature, documentation, trait implementations "
        "and a link to the source.",
        "",
        f"- Crate page: [{crate}]({page_url(origin, f'{crate}/index.html')})",
        f"- All items on one page: [{crate}/all]({origin}/{crate}/all)",
    ]
    if source:
        lines.append(f"- Source: [{source.removesuffix('.html')}]({page_url(origin, source)})")
    lines += [f"- Every crate: [llms.txt]({origin}/llms.txt)", ""]
    for path, _, summary, entries, _ in modules:
        lines.append(f"## {path}")
        if summary:
            lines.append(summary)
        lines.append("")
        lines += [entry_line(origin, entry) for entry in entries]
        lines.append("")
    return "\n".join(lines).rstrip("\n") + "\n"


def front_door(origin, crates, blurbs, maps, sections):
    lines = [
        "# Logicaffeine API documentation",
        "",
        "> The Rust API of LOGOS, the English programming language: English sentences in, first-order "
        "logic out; a small Calculus-of-Constructions kernel that checks every proof; certified SAT, "
        "algebraic and arithmetic proof engines; exact mathematics; and the compiler and runtime that "
        "run English programs. Generated by rustdoc from the source on every build.",
        "",
        "LOGOS reads English. Logic mode turns sentences into first-order logic; Math mode proves "
        "theorems the kernel checks; Imperative mode runs English programs through an interpreter, a "
        "bytecode VM with a JIT, and Rust, C and WASM backends. The language itself — syntax, guides, "
        "examples, error codes — is described at https://logicaffeine.com/llms.txt and, in one file, "
        "https://logicaffeine.com/llms-full.txt. This site is the API underneath it.",
        "",
        "## How to navigate this site",
        "",
        "- Every crate has a complete map, `/llms/<crate>.txt`: each public module and item with its kind, "
        "its one-line summary and its address. Fetch the one you need and search it — the crates are "
        "listed below.",
        "- Addresses are extensionless: a module at `/<crate>/<module path>/`, an item at "
        "`/<crate>/<module path>/<kind>.<Name>` (`struct.`, `enum.`, `trait.`, `fn.`, `type.`, "
        "`constant.`, `macro.`), every item of a crate on one page at `/<crate>/all`, source at "
        "`/src/<crate>/<file>`.",
        "- Item pages carry the full signature, documentation, trait implementations and a source link.",
        "- An address that does not exist answers 404 with a page of routes into every crate — never a "
        "copy of another page.",
        f"- Every page: [sitemap.xml]({origin}/sitemap.xml).",
        "",
        "## Start here",
        "",
        "Where each kind of work begins — logic, proof, mathematics, verification and programs.",
        "",
    ]
    for section, routes in sections.items():
        lines.append(f"### {section}")
        lines += [f"- [{r['task']}]({page_url(origin, r['target'])}): {r['note']}" for r in routes]
        lines.append("")
    lines += [
        "## Crates",
        "",
        "Every crate the site documents: its description, its map, and its top-level modules.",
        "",
    ]
    for crate in crates:
        modules = maps[crate]
        n_modules, n_items = map_counts(modules)
        root_entries = modules[0][3]
        shown = [e for e in root_entries if e["kind"] == "mod"] or root_entries
        lines += [
            f"### [{crate}]({page_url(origin, f'{crate}/index.html')})",
            blurbs[crate],
            "",
            f"- Map: [llms/{crate}.txt]({origin}/llms/{crate}.txt) — {n_modules} modules, {n_items} items",
        ]
        lines += [entry_line(origin, entry) for entry in shown]
        lines.append("")
    return "\n".join(lines).rstrip("\n") + "\n"


def routes_html(sections, prefix):
    parts = []
    for section, routes in sections.items():
        parts.append(f"    <h3>{html.escape(section)}</h3>\n    <ul>\n")
        for r in routes:
            href = prefix + canonical(r["target"]).lstrip("/")
            parts.append(
                f'            <li><a href="{html.escape(href)}">{html.escape(r["task"])}</a> — {html.escape(r["note"], quote=False)}</li>\n'
            )
        parts.append("    </ul>\n")
    return "".join(parts)


def landing(origin, crates, blurbs, sections):
    items = "".join(
        f'            <li><a href="{crate}/">{crate}</a> — {html.escape(blurbs[crate], quote=False)}'
        f' · <a href="llms/{crate}.txt">map</a></li>\n'
        for crate in crates
    )
    routes = routes_html(sections, "")
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Logicaffeine Documentation</title>
    <link rel="canonical" href="{html.escape(origin)}/">
    <style>
{STYLE}
    </style>
</head>
<body>
    <h1>Logicaffeine Documentation</h1>
    <p>API documentation for the Logicaffeine workspace — the LOGOS English
    programming language, its logic pipeline, proof engines, execution tiers,
    and platform crates.</p>
    <div class="links">
        <a href="https://logicaffeine.com">logicaffeine.com</a>
        <a href="https://logicaffeine.com/studio">Studio</a>
        <a href="https://github.com/Brahmastra-Labs/logicaffeine">GitHub</a>
    </div>
    <p class="agents">AI agents: start at <a href="llms.txt">llms.txt</a> — a plain-text map of
    every crate, module and item, with one file per crate.</p>
    <h2>Start here</h2>
{routes}    <h2>Crates</h2>
    <ul>
{items}    </ul>
</body>
</html>
"""


def not_found_page(crates, blurbs, sections, maps):
    """Served for every address with no file, at any depth — so every link is root-relative."""
    items = []
    for crate in crates:
        modules = [e for e in maps[crate][0][3] if e["kind"] == "mod"]
        nested = "".join(
            f'                <li><a href="{html.escape(canonical(e["target"]))}">{html.escape(entry_name(e))}</a>'
            + (f" — {html.escape(e['summary'], quote=False)}" if e["summary"] else "")
            + "</li>\n"
            for e in modules
        )
        items.append(
            f'            <li><a href="/{crate}/">{crate}</a> — {html.escape(blurbs[crate], quote=False)}'
            + (f'\n            <ul class="modules">\n{nested}            </ul>' if nested else "")
            + "</li>\n"
        )
    routes = routes_html(sections, "/")
    crate_items = "".join(items)
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="robots" content="noindex">
    <title>Page not found · Logicaffeine Documentation</title>
    <style>
{STYLE}
    </style>
</head>
<body>
    <h1>Page not found</h1>
    <p>Nothing in the Logicaffeine documentation lives at this address. An item may have
    been renamed or moved; every crate page has a search box, and the routes below lead into
    every crate.</p>
    <div class="links">
        <a href="/">Documentation index</a>
        <a href="https://logicaffeine.com">logicaffeine.com</a>
    </div>
    <p class="agents">AI agents: <a href="/llms.txt">/llms.txt</a> maps every crate, module and
    item in plain text, with one file per crate.</p>
    <h2>Start here</h2>
{routes}    <h2>Crates</h2>
    <ul>
{crate_items}    </ul>
</body>
</html>
"""


def robots(origin):
    return f"User-agent: *\nAllow: /\n\nSitemap: {origin}/sitemap.xml\n"


def is_site_relative(href):
    parts = urlsplit(href)
    return not parts.scheme and not parts.netloc


class References(HTMLParser):
    """Every address a page asks a browser to load or follow — `href` and `src` on any tag, the
    target of a meta refresh, and the runtime assets rustdoc names in its `rustdoc-vars` meta —
    each with what rustdoc says about it: whether it is an optional async script, and which trait
    impl block (if any) it sits in, so documentation copied from a foreign trait can be told
    from our own."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.found = []
        self.impls = []
        self.details = 0
        self.header = False
        self.anchor = None

    def handle_starttag(self, tag, attrs):
        values = {name: value for name, value in attrs if value is not None}
        names = {name for name, _ in attrs}
        classes = values.get("class", "").split()
        if tag == "details":
            self.details += 1
            if "implementors-toggle" in classes:
                self.impls.append({"depth": self.details, "trait": None, "ours": None})
        impl = self.impls[-1] if self.impls else None
        if impl and tag == "section" and "impl" in classes and values.get("id", "").startswith("impl-"):
            head = values["id"][len("impl-") :]
            if "-for-" in head:
                impl["trait"] = re.split(r"<", unquote(head.split("-for-", 1)[0]).lstrip("!"), maxsplit=1)[0]
        if impl and tag == "h3" and "code-header" in classes:
            self.header = True
        if self.header and tag == "a" and "trait" in classes:
            self.anchor = {"href": values.get("href"), "text": ""}

        optional = tag == "script" and "async" in names
        for name in ("href", "src"):
            if name in values:
                self.found.append((values[name], optional, impl))
        if tag != "meta":
            return
        if values.get("http-equiv", "").lower() == "refresh":
            target = re.search(r"url=(.*)", values.get("content", ""), re.IGNORECASE)
            if target:
                self.found.append((target.group(1).strip(), False, None))
        if values.get("name") == "rustdoc-vars" and "data-static-root-path" in values:
            root = values["data-static-root-path"]
            for name, value in values.items():
                if name.startswith("data-") and name.endswith(("-js", "-css")) and value:
                    self.found.append((root + value, False, None))

    def handle_data(self, data):
        if self.anchor is not None:
            self.anchor["text"] += data

    def handle_endtag(self, tag):
        impl = self.impls[-1] if self.impls else None
        if tag == "a" and self.anchor is not None:
            if impl and impl["ours"] is None and self.anchor["text"].strip() == impl["trait"]:
                href = self.anchor["href"]
                impl["ours"] = href is not None and is_site_relative(href)
            self.anchor = None
        elif tag == "h3" and self.header:
            self.header = False
            if impl and impl["ours"] is None:
                impl["ours"] = impl["trait"] is None
        elif tag == "details":
            if impl and impl["depth"] == self.details:
                self.impls.pop()
            self.details -= 1


def normalize(path):
    """Resolves `.` and `..` in an absolute path; None when it climbs above the root."""
    trailing = path.endswith(("/", "/.", "/.."))
    segments = []
    for segment in path.split("/"):
        if segment in ("", "."):
            continue
        if segment == "..":
            if not segments:
                return None
            segments.pop()
        else:
            segments.append(segment)
    resolved = "/" + "/".join(segments)
    return resolved + "/" if trailing and resolved != "/" else resolved


def served(site, path):
    """Whether Pages answers `path` with a file, after its `.html` and `index.html` redirects."""
    rel = path.lstrip("/")
    if path.endswith("/"):
        bare = rel.rstrip("/")
        return (site / rel / "index.html").is_file() or bool(bare) and (site / f"{bare}.html").is_file()
    return (site / rel).is_file() or (site / f"{rel}.html").is_file() or (site / rel / "index.html").is_file()


def dangling(site):
    """`page → reference` for every reference the site does not serve and rustdoc does not own."""
    broken = []
    verdicts = {}
    for page in sorted(site.rglob("*.html")):
        rel = page.relative_to(site).as_posix()
        url = canonical(rel)
        base = url[: url.rfind("/") + 1]
        parser = References()
        parser.feed(page.read_text(encoding="utf-8", errors="replace"))
        parser.close()
        for reference, optional, impl in parser.found:
            parts = urlsplit(reference.strip())
            if parts.scheme or parts.netloc or not parts.path:
                continue
            path = normalize(parts.path if parts.path.startswith("/") else base + parts.path)
            if path is None:
                broken.append(f"  {rel} → {reference} (climbs above the site root)")
                continue
            path = unquote(path)
            if path not in verdicts:
                verdicts[path] = served(site, path)
            if verdicts[path]:
                continue
            if optional and path.startswith(OPTIONAL_SCRIPT_DIRS):
                continue
            if impl is not None and impl["ours"] is False:
                continue
            broken.append(f"  {rel} → {reference}")
    return broken


def check_map_links(origin, site, texts):
    for name, text in texts.items():
        for url in MARKDOWN_LINK.findall(text):
            if not url.startswith(origin + "/") or not served(site, unquote(url[len(origin):])):
                raise Refused(f"{name} links {url}, a page the site does not ship")


def assemble(workspace, doc, staging, crates, limit, routes):
    origin = site_origin(workspace.source)
    check_curation(doc, crates)
    blurbs = workspace.descriptions(crates)
    sections = load_routes(routes)
    check_routes(doc, sections)
    maps = {crate: crate_map(doc, crate) for crate in crates}
    for crate in crates:
        check_map_complete(doc, crate, maps[crate])
    urls = sitemap_urls(origin, doc, crates)
    agent_maps = {"llms.txt": front_door(origin, crates, blurbs, maps, sections)}
    agent_maps.update(
        {f"llms/{crate}.txt": crate_map_text(origin, crate, blurbs[crate], maps[crate]) for crate in crates}
    )
    shell = {
        "index.html": landing(origin, crates, blurbs, sections),
        "404.html": not_found_page(crates, blurbs, sections, maps),
        "robots.txt": robots(origin),
        **sitemap_files(origin, urls, limit),
        **agent_maps,
    }
    clashes = sorted({name.split("/")[0] for name in shell if (doc / name.split("/")[0]).exists()})
    if clashes:
        raise Refused(f"rustdoc wrote {', '.join(clashes)}; the site shell would overwrite it")

    shutil.copytree(doc, staging, ignore=shutil.ignore_patterns(".*"))
    for name, text in shell.items():
        (staging / name).parent.mkdir(parents=True, exist_ok=True)
        (staging / name).write_text(text, encoding="utf-8")
    check_map_links(origin, staging, agent_maps)

    broken = dangling(staging)
    if broken:
        shown = broken[:100]
        more = f"\n  … and {len(broken) - len(shown)} more" if len(broken) > len(shown) else ""
        raise Refused(
            f"{len(broken)} reference(s) name files the site does not ship:\n" + "\n".join(shown) + more
        )
    return len(urls)


def write_site(workspace, doc, out, crates, limit, routes):
    staging = out.parent / f".{out.name}.staging"
    shutil.rmtree(staging, ignore_errors=True)
    out.parent.mkdir(parents=True, exist_ok=True)
    try:
        pages = assemble(workspace, doc, staging, crates, limit, routes)
        staging.rename(out)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    files = sum(1 for p in out.rglob("*") if p.is_file())
    print(f"site.py: {out} — {files} files, {pages} pages in the sitemap")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)

    def command(name, help):
        sub = commands.add_parser(name, help=help)
        sub.add_argument("--source", type=Path, required=True, help="the workspace root")
        sub.add_argument("crates", nargs="+", help="the curated crates, in landing-page order")
        return sub

    build = command("build", "document the crates, then assemble the site")
    build.add_argument("--out", type=Path, required=True, help="where the site is written")
    build.add_argument("--routes", type=Path, required=True, help="the start-here routes (routes.toml)")
    build.add_argument("--sitemap-limit", type=int, default=SITEMAP_LIMIT, help="URLs per sitemap file")

    doc = command("document", "run cargo doc over the crates, doc directories first, one rustdoc at a time")
    doc.add_argument("--dry-run", action="store_true", help="print the cargo arguments, one command per line")

    assemble_cmd = command("assemble", "assemble the site from existing rustdoc output")
    assemble_cmd.add_argument("--doc", type=Path, required=True, help="rustdoc's output directory")
    assemble_cmd.add_argument("--out", type=Path, required=True, help="where the site is written")
    assemble_cmd.add_argument("--routes", type=Path, required=True, help="the start-here routes (routes.toml)")
    assemble_cmd.add_argument("--sitemap-limit", type=int, default=SITEMAP_LIMIT, help="URLs per sitemap file")

    args = parser.parse_args()
    if getattr(args, "sitemap_limit", 1) < 1:
        parser.error("--sitemap-limit must be at least 1")

    out = getattr(args, "out", None)
    if out is not None:
        shutil.rmtree(out, ignore_errors=True)
    try:
        workspace = Workspace(args.source)
        if args.command == "document" and args.dry_run:
            for cmd in cargo_commands(workspace, args.crates):
                print(" ".join(cmd[1:]))
            return 0
        if args.command in ("document", "build"):
            document(workspace, args.crates)
        if args.command == "build":
            write_site(workspace, workspace.doc, out, args.crates, args.sitemap_limit, args.routes)
        if args.command == "assemble":
            write_site(workspace, args.doc, out, args.crates, args.sitemap_limit, args.routes)
    except Refused as refusal:
        print(f"site.py: {refusal}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
