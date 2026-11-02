#!/usr/bin/env bash
# Merge the SSG prerender output into the deployable public/ dir.
#
# `dx build --ssg` prerenders through the server binary BEFORE the client bundle
# phase writes the final shell (public/index.html), so the prerendered pages are
# missing everything the shell contributes: the wasm loader tags AND the
# invariant head (the /assets/style.css link, viewport meta, KaTeX
# bootstrap, token styles) — without which a prerendered page paints unstyled
# for a beat and renders wrong on mobile. A prerendered `/` written straight
# into public/ would also be clobbered by the shell. So the server renders into
# web/prerendered/ (see main.rs) and this script:
#   1. extracts the invariant head (everything but <title>) and the loader
#      (preload link + module script, with the content-hashed bundle names)
#      from the final shell — fail-closed if the loader is absent,
#   2. gives every page the shell's document prologue (`<!DOCTYPE html>`, the
#      <html lang> tag, <head>) with the page's own <title> inside <head>: the
#      server renders through dioxus's fallback template, which has no lang and
#      no <title> of its own, so the page title came out BEFORE the doctype and
#      every page opened in quirks mode,
#   3. strips hydration markers from raw-text elements: the server marks every
#      dynamic text node with a `<!--node-idN-->` comment, even inside <style>,
#      <script>, <title> and <textarea>, where it is content, not a comment — in a
#      <style> it becomes the first rule's selector and the browser drops the rule
#      (GLOBAL_STYLE's first rule is the palette). The client takes over rather
#      than hydrating, so the markers carry nothing,
#   4. injects the invariant head and the loader into every page that lacks them,
#   5. lays the pages over public/ (the prerendered `/` becomes index.html, so
#      the SPA fallback serves real landing content too).
#
# Usage: ./scripts/web/merge-ssg.sh [web-dir]
#        (default: target/dx/logicaffeine-web/release/web)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WEB="${1:-target/dx/logicaffeine-web/release/web}"

[ -d "$WEB/public" ] || { echo "merge-ssg: $WEB/public does not exist (run dx build first)" >&2; exit 1; }
[ -d "$WEB/prerendered" ] || { echo "merge-ssg: $WEB/prerendered does not exist — did the --ssg prerender run?" >&2; exit 1; }

python3 - "$WEB" <<'PY'
import pathlib, re, sys

web = pathlib.Path(sys.argv[1])
shell = (web / "public" / "index.html").read_text()

script = re.search(r'<script type="module" async src="[^"]*"></script>', shell)
if not script:
    sys.exit("merge-ssg: no module-script loader found in the shell index.html — "
             "is this a complete dx build?")
loader_body = script.group(0)
preload = re.search(r'<link rel="preload" as="script" href="[^"]*" crossorigin>', shell)
loader_head = preload.group(0) if preload else ""

# The shell's invariant head: everything between <head> and </head> except the
# page-specific <title> and the loader bits (injected separately, deduplicated).
m = re.search(r'<head>(.*)</head>', shell, re.DOTALL)
if not m:
    sys.exit("merge-ssg: shell index.html has no <head> block")
invariants = re.sub(r'<title>.*?</title>', '', m.group(1), flags=re.DOTALL)
invariants = invariants.replace(loader_body, '')
if loader_head:
    invariants = invariants.replace(loader_head, '')
invariants = invariants.strip()
if '/assets/style.css' not in invariants:
    sys.exit("merge-ssg: shell head carries no /assets/style.css link — refusing to "
             "propagate a broken head into every prerendered page")

# The shell's document prologue: `<!DOCTYPE html>`, <html lang=…>, <head>.
opening = re.match(r'\s*<!DOCTYPE html>\s*<html\b[^>]*>\s*<head\b[^>]*>', shell)
if not opening:
    sys.exit("merge-ssg: the shell does not open with <!DOCTYPE html><html …><head> — refusing to "
             "propagate a broken prologue into every prerendered page")
prologue = opening.group(0).strip()
TITLE = re.compile(r'<title>.*?</title>', re.DOTALL)
RAW_TEXT = re.compile(r'(<(style|script|title|textarea)\b[^>]*>)(.*?)(</\2>)', re.DOTALL)
MARKER = re.compile(r'<!--node-id\d+-->')

def unmark(m):
    global unmarked
    text = MARKER.sub("", m.group(3))
    if text != m.group(3):
        unmarked += 1
    return m.group(1) + text + m.group(4)

pages = sorted((web / "prerendered").rglob("index.html"))
if not pages:
    sys.exit("merge-ssg: no prerendered pages found")

opened = headed = injected = unmarked = 0
for page in pages:
    html = page.read_text()
    head = re.search(r'<head\b[^>]*>', html)
    close = html.find("</head>")
    if not head or close < head.end():
        sys.exit(f"merge-ssg: {page} has no <head>…</head> block")
    if html[:head.end()].strip() != prologue:
        # The page's own title (the server wrote it ahead of the doctype) goes first in
        # <head>; everything the fallback template put before <head> is replaced.
        titles = TITLE.findall(html[:close])
        inner = TITLE.sub("", html[head.end():close])
        html = prologue + (titles[0] if titles else "") + inner + html[close:]
        opened += 1
    html = RAW_TEXT.sub(unmark, html)
    if "/assets/style.css" not in html:
        # Right after <head>, so the render-blocking stylesheet precedes the
        # prerendered body content exactly as it does in the shell.
        html = html.replace("<head>", f"<head>{invariants}", 1)
        headed += 1
    if 'id="app"' not in html:
        # The client mount point: the wasm renders into #app while the
        # prerendered copy stays visible in #main until the first commit.
        html = html.replace('<div id="main">', '<div id="app"></div><div id="main">', 1)
    if "logicaffeine-web-" not in html:
        if loader_head and "</head>" in html:
            html = html.replace("</head>", f"{loader_head}</head>", 1)
        if "</body>" in html:
            html = html.replace("</body>", f"{loader_body}</body>", 1)
        else:
            html += loader_body
        injected += 1
    page.write_text(html)
    rel = page.relative_to(web / "prerendered")
    dest = web / "public" / rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(html)

print(f"merge-ssg: merged {len(pages)} prerendered pages into {web / 'public'} "
      f"({opened} received the document prologue, {headed} the invariant head, {injected} the wasm loader; "
      f"{unmarked} raw-text element(s) lost a hydration marker)")
PY
