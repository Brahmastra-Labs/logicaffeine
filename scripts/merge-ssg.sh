#!/usr/bin/env bash
# Merge the SSG prerender output into the deployable public/ dir.
#
# `dx build --ssg` prerenders through the server binary BEFORE the client bundle
# phase writes the final shell (public/index.html), so the prerendered pages are
# missing everything the shell contributes: the wasm loader tags AND the
# invariant head (the /assets/style.css link, viewport meta, KaTeX/audio
# bootstrap, token styles) — without which a prerendered page paints unstyled
# for a beat and renders wrong on mobile. A prerendered `/` written straight
# into public/ would also be clobbered by the shell. So the server renders into
# web/prerendered/ (see main.rs) and this script:
#   1. extracts the invariant head (everything but <title>) and the loader
#      (preload link + module script, with the content-hashed bundle names)
#      from the final shell — fail-closed if the loader is absent,
#   2. injects both into every prerendered page that lacks them,
#   3. lays the pages over public/ (the prerendered `/` becomes index.html, so
#      the SPA fallback serves real landing content too).
#
# Usage: ./scripts/merge-ssg.sh [web-dir]
#        (default: target/dx/logicaffeine-web/release/web)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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

pages = sorted((web / "prerendered").rglob("index.html"))
if not pages:
    sys.exit("merge-ssg: no prerendered pages found")

headed = injected = 0
for page in pages:
    html = page.read_text()
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
      f"({headed} received the invariant head, {injected} the wasm loader)")
PY
