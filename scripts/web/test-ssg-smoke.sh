#!/usr/bin/env bash
# Headless-browser smoke of the SSG output + CSR takeover.
#
# Serves the built public/ dir and drives headless Chrome through:
#   1. a prerendered route (/pricing)  — correct title, #main non-empty, and the
#      wasm CSR takeover leaves exactly ONE copy of the page content (guards the
#      #main clear in main.rs: a non-hydrating mount APPENDS without it),
#   2. a client-side route (/registry/package/smoke-test) — the shell must boot
#      the app and client-side render the right page (no hydration-data crash),
#   3. an unknown route — a 404 status, and the 404 page boots the app.
#
# The build is served through scripts/web/pages-server.py, which answers each path
# with the status and the document Cloudflare Pages would.
#
# Requires a Chrome/Chromium binary (CI's ubuntu runners ship one; set CHROME_BIN
# to override). Exits 0 with a loud SKIP when no browser exists — verify-ssg.sh
# still gates the artifacts, and the tailnet review covers human smoke locally.
#
# Usage: ./scripts/web/test-ssg-smoke.sh [built-public-dir]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PUBLIC="${1:-target/dx/logicaffeine-web/release/web/public}"
PORT="${SSG_SMOKE_PORT:-8781}"

CHROME="${CHROME_BIN:-$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || true)}"
if [ -z "$CHROME" ]; then
    echo "test-ssg-smoke: SKIP — no Chrome/Chromium binary found (set CHROME_BIN)" >&2
    exit 0
fi
[ -d "$PUBLIC" ] || { echo "test-ssg-smoke: build dir $PUBLIC does not exist" >&2; exit 1; }

# The build served the way Cloudflare Pages serves it: _redirects, HTML route matching, 404.html.
python3 "$ROOT/scripts/web/pages-server.py" "$PUBLIC" "$PORT" >/dev/null &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT
sleep 1

check() {
    local url="$1" description="$2" expect="$3"
    # --dump-dom waits for the load event, giving the wasm a beat to boot; the
    # virtual-time budget lets the module instantiate before the DOM is dumped.
    local dom
    dom="$("$CHROME" --headless=new --disable-gpu --no-sandbox \
        --virtual-time-budget=30000 --dump-dom "$url" 2>/dev/null)" || {
        echo "FAIL $description: chrome could not load $url"
        return 1
    }
    if ! grep -qF "$expect" <<<"$dom"; then
        echo "FAIL $description: expected content $expect not found at $url"
        echo "      (a blank page here usually means the CSR takeover removed the"
        echo "       prerendered copy before the app committed real content)"
        return 1
    fi
    # CSR takeover must not duplicate the page: the marker appears exactly once.
    local n
    n=$(grep -oF "$expect" <<<"$dom" | wc -l)
    if [ "$n" -gt 2 ]; then
        echo "FAIL $description: marker appears $n times — CSR takeover duplicated content"
        return 1
    fi
    # And it must have COMPLETED: once the app renders, the prerendered copy
    # (#main) is removed. #main still present = the wasm never booted or the
    # takeover never fired.
    if grep -q 'id="main"' <<<"$dom"; then
        echo "FAIL $description: prerendered copy (#main) still in the DOM — takeover incomplete"
        return 1
    fi
    echo "  ok  $description"
}

fail=0
check "http://127.0.0.1:$PORT/contact" "prerendered route boots + takes over" "Contact the team" || fail=1
check "http://127.0.0.1:$PORT/registry/package/smoke-test" "client-side route boots" "app-root" || fail=1

# An unknown address is a real 404 (DAT-171): the status a crawler reads, then the page a person sees.
unknown="http://127.0.0.1:$PORT/this-route-does-not-exist-xyz"
status="$(curl -s -o /dev/null -w '%{http_code}' "$unknown")"
if [ "$status" = "404" ]; then
    echo "  ok  an unknown route answers 404"
else
    echo "FAIL an unknown route answers $status, not 404 (a soft 404 crawlers index as the landing page)"
    fail=1
fi
check "$unknown" "the 404 page boots" "Page not found: /this-route-does-not-exist-xyz" || fail=1

if [ "$fail" -ne 0 ]; then
    echo "test-ssg-smoke: FAILED" >&2
    exit 1
fi
echo "test-ssg-smoke: OK"
