#!/usr/bin/env bash
# Headless-browser smoke of the SSG output + CSR takeover.
#
# Serves the built public/ dir and drives headless Chrome through:
#   1. a prerendered route (/pricing)  — correct title, #main non-empty, and the
#      wasm CSR takeover leaves exactly ONE copy of the page content (guards the
#      #main clear in main.rs: a non-hydrating mount APPENDS without it),
#   2. an SPA-fallback route (/registry/package/smoke-test) — the shell must boot
#      the app and client-side render the right page (no hydration-data crash).
#
# Requires a Chrome/Chromium binary (CI's ubuntu runners ship one; set CHROME_BIN
# to override). Exits 0 with a loud SKIP when no browser exists — verify-ssg.sh
# still gates the artifacts, and the tailnet review covers human smoke locally.
#
# Usage: ./scripts/test-ssg-smoke.sh [built-public-dir]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PUBLIC="${1:-target/dx/logicaffeine-web/release/web/public}"
PORT="${SSG_SMOKE_PORT:-8781}"

CHROME="${CHROME_BIN:-$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || true)}"
if [ -z "$CHROME" ]; then
    echo "test-ssg-smoke: SKIP — no Chrome/Chromium binary found (set CHROME_BIN)" >&2
    exit 0
fi
[ -d "$PUBLIC" ] || { echo "test-ssg-smoke: build dir $PUBLIC does not exist" >&2; exit 1; }

# Static server with the SPA fallback Cloudflare provides via _redirects.
python3 - "$PUBLIC" "$PORT" <<'PY' &
import functools, http.server, os, sys

root, port = sys.argv[1], int(sys.argv[2])

class SpaHandler(http.server.SimpleHTTPRequestHandler):
    def send_head(self):
        path = self.translate_path(self.path)
        if os.path.isdir(path):
            index = os.path.join(path, "index.html")
            if os.path.isfile(index):
                return super().send_head()
        if not os.path.isfile(path):
            self.path = "/index.html"
        return super().send_head()

    def log_message(self, *args):
        pass

http.server.ThreadingHTTPServer(("127.0.0.1", port), functools.partial(SpaHandler, directory=root)).serve_forever()
PY
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
check "http://127.0.0.1:$PORT/pricing" "prerendered route boots + takes over" "Contact Us" || fail=1
check "http://127.0.0.1:$PORT/registry/package/smoke-test" "SPA fallback route boots" "app-root" || fail=1

if [ "$fail" -ne 0 ]; then
    echo "test-ssg-smoke: FAILED" >&2
    exit 1
fi
echo "test-ssg-smoke: OK"
