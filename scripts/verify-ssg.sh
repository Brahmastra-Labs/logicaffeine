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
#   - <dir>/<route>/index.html exists (route "/" -> <dir>/index.html)
#   - carries its own canonical link (https://logicaffeine.com<route>)
#   - carries exactly ONE og:title (shell+SSR duplication regression)
#   - carries at least one JSON-LD block
#   - the <title> placeholder was substituted (no "{app_title}" remnant)
#
# Usage: ./scripts/verify-ssg.sh [built-public-dir]
#        (default: target/dx/logicaffeine-web/release/web/public)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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
        file="$PUBLIC${route}/index.html"
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
done

if [ "$fail" -ne 0 ]; then
    echo "verify-ssg: FAILED (checked $count routes)" >&2
    exit 1
fi
echo "verify-ssg: OK — $count prerendered routes verified in $PUBLIC"
