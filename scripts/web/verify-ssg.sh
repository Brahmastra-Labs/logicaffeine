#!/usr/bin/env bash
# Verify the SSG prerender output.
#
# `dx build --ssg` is silent about routes whose server render fails (it only logs),
# so CI must prove every route in the manifest actually produced its HTML file and
# that the file carries the per-page head a crawler/unfurler needs. The route list
# is read from public/sitemap.xml, which the sitemap tests hold identical to
# sitemap::prerender_routes() — one source of truth, no second manifest.
#
# Checks per route:
#   - <dir><route>.html exists and <dir><route>/index.html does not (route "/" ->
#     <dir>/index.html): Pages serves <route>.html at the route itself, the address the
#     canonical link and the sitemap name, and a directory's index.html at <route>/
#     after a 308 (DAT-170)
#   - carries its own canonical link (https://logicaffeine.com<route>)
#   - carries exactly ONE og:title (shell+SSR duplication regression)
#   - carries at least one JSON-LD block
#   - the <title> placeholder was substituted (no "{app_title}" remnant)
#   - the document opens in standards mode: <!DOCTYPE html> first, <html lang="en">,
#     exactly one <title>, inside <head>
#   - no raw-text element (style, script, title, textarea) holds a hydration marker
#
# And once for the site: the root 404.html exists, carries the not-found page with a
# noindex robots tag, no canonical link and the wasm loader, and no /* rule in
# _redirects answers before it.
#
# Usage: ./scripts/web/verify-ssg.sh [built-public-dir]
#        (default: target/dx/logicaffeine-web/release/web/public)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PUBLIC="${1:-target/dx/logicaffeine-web/release/web/public}"
SITEMAP="apps/logicaffeine_web/public/sitemap.xml"

[ -d "$PUBLIC" ] || { echo "verify-ssg: build dir $PUBLIC does not exist" >&2; exit 1; }
[ -f "$SITEMAP" ] || { echo "verify-ssg: $SITEMAP does not exist" >&2; exit 1; }

routes="$(grep -oP '<loc>https://logicaffeine\.com\K[^<]*' "$SITEMAP")"
[ -n "$routes" ] || { echo "verify-ssg: no routes parsed from $SITEMAP" >&2; exit 1; }

fail=0
count=0
for route in $routes; do
    count=$((count + 1))
    if [ "$route" = "/" ]; then
        file="$PUBLIC/index.html"
    else
        file="$PUBLIC${route}.html"
        if [ -e "$PUBLIC${route}/index.html" ]; then
            echo "FAIL $route: $PUBLIC${route}/index.html exists, so Pages would answer ${route} with a 308 to ${route}/"
            fail=1
        fi
    fi

    if [ ! -f "$file" ]; then
        echo "FAIL $route: missing $file (route paniced or was skipped during prerender)"
        fail=1
        continue
    fi

    if ! grep -q "rel=\"canonical\" href=\"https://logicaffeine.com${route}\"" "$file" \
       && ! grep -q "href=\"https://logicaffeine.com${route}\" rel=\"canonical\"" "$file"; then
        echo "FAIL $route: canonical link for ${route} not found in $file"
        fail=1
    fi

    og_titles=$(grep -o 'property="og:title"' "$file" | wc -l || true)
    if [ "$og_titles" -ne 1 ]; then
        echo "FAIL $route: expected exactly 1 og:title, found $og_titles"
        fail=1
    fi

    if ! grep -q 'application/ld+json' "$file"; then
        echo "FAIL $route: no JSON-LD structured data"
        fail=1
    fi

    if grep -q '{app_title}' "$file"; then
        echo "FAIL $route: unsubstituted {app_title} placeholder in <title>"
        fail=1
    fi

    # Standards mode, the shell's language, and one <title> inside <head>. The prerender
    # server renders through dioxus's fallback template, which put the title BEFORE
    # `<!DOCTYPE html>` (quirks mode) and dropped `lang` (see merge-ssg.sh).
    prologue="$(python3 - "$file" <<'PY'
import re, sys
html = open(sys.argv[1]).read()
problems = []
if not re.match(r'\s*<!DOCTYPE html>', html):
    problems.append("does not open with <!DOCTYPE html> (quirks mode): " + html.lstrip()[:60].replace("\n", " "))
if not re.search(r'<html\b[^>]*\blang="en"', html):
    problems.append('no <html lang="en">')
head = re.search(r'<head\b[^>]*>(.*?)</head>', html, re.DOTALL)
if head and '<title' in html[:head.start()]:
    problems.append("a <title> precedes <head>")
in_head = len(re.findall(r'<title>', head.group(1))) if head else 0
if in_head != 1:
    problems.append(f"{in_head} <title> tags inside <head>, expected exactly one")
print("; ".join(problems))
PY
)"
    if [ -n "$prologue" ]; then
        echo "FAIL $route: $prologue"
        fail=1
    fi

    # A `<!--node-idN-->` hydration marker inside a raw-text element is content, not a
    # comment: in a <style> it becomes the first rule's selector and drops the rule (the
    # palette, for GLOBAL_STYLE). merge-ssg.sh strips them.
    marked="$(python3 - "$file" <<'PY'
import re, sys
html = open(sys.argv[1]).read()
tags = [m.group(1) for m in re.finditer(r'<(style|script|title|textarea)\b[^>]*>(.*?)</\1>', html, re.DOTALL)
        if '<!--node-id' in m.group(2)]
print(f"{len(tags)} raw-text element(s) hold a hydration marker ({', '.join(sorted(set(tags)))})" if tags else "")
PY
)"
    if [ -n "$marked" ]; then
        echo "FAIL $route: $marked"
        fail=1
    fi

    if ! grep -q '<script type="module" async src="[^"]*logicaffeine-web-[^"]*"></script>' "$file"; then
        echo "FAIL $route: no wasm loader script — the page would never boot the app"
        fail=1
    fi

    if ! grep -q '/assets/style.css' "$file"; then
        echo "FAIL $route: no /assets/style.css link — the page would paint unstyled"
        fail=1
    fi

    if ! grep -q 'name="viewport"' "$file"; then
        echo "FAIL $route: no viewport meta — the page would misrender on mobile"
        fail=1
    fi

    if ! grep -q 'id="app"' "$file"; then
        echo "FAIL $route: no #app mount point — the wasm takeover has nowhere to render"
        fail=1
    fi

    # #main must carry real prerendered body, not an empty shell: the whole point
    # of SSG is a working first paint (and the takeover's static fallback). An
    # empty #main passed every head-only check above while the page rendered blank.
    main_body_chars="$(python3 - "$file" <<'PY'
import re, sys
html = open(sys.argv[1]).read()
m = re.search(r'<div id="main">(.*?)</body>', html, re.DOTALL)
body = m.group(1) if m else ""
print(len(re.sub(r'<[^>]+>|\s', '', body)))
PY
)"
    if [ "${main_body_chars:-0}" -lt 200 ]; then
        echo "FAIL $route: #main carries only ${main_body_chars:-0} chars of body text — empty/near-empty prerender"
        fail=1
    fi
done

# The root 404.html (DAT-171): Cloudflare Pages serves it with a 404 status for every path no
# file and no _redirects rule answers. merge-ssg.sh moves the page prerendered at /404 there.
not_found="$PUBLIC/404.html"
if [ ! -f "$not_found" ]; then
    echo "FAIL 404: missing $not_found — Pages would answer every unknown path with the landing page and a 200"
    fail=1
else
    if [ -e "$PUBLIC/404/index.html" ]; then
        echo "FAIL 404: $PUBLIC/404/index.html is left beside 404.html"
        fail=1
    fi
    if grep -q 'rel="canonical"' "$not_found"; then
        echo "FAIL 404: the 404 page claims a canonical address"
        fail=1
    fi
    if ! grep -q 'name="robots" content="noindex"' "$not_found" && ! grep -q 'content="noindex" name="robots"' "$not_found"; then
        echo "FAIL 404: the 404 page is not marked noindex"
        fail=1
    fi
    if ! grep -q 'Page not found' "$not_found"; then
        echo "FAIL 404: the 404 page does not carry the not-found page"
        fail=1
    fi
    if ! grep -q '<script type="module" async src="[^"]*logicaffeine-web-[^"]*"></script>' "$not_found"; then
        echo "FAIL 404: no wasm loader script — the 404 page would never boot the app"
        fail=1
    fi
fi
if grep -qE '^/\*[[:space:]]' "$PUBLIC/_redirects" 2>/dev/null; then
    echo "FAIL _redirects: a /* rule answers every unknown path before the 404 page can"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "verify-ssg: FAILED (checked $count routes)" >&2
    exit 1
fi
echo "verify-ssg: OK — $count prerendered routes verified in $PUBLIC"
