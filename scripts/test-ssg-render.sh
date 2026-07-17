#!/usr/bin/env bash
# Per-route render lock (see test-ssg-render.mjs).
#
# Serves the built public/ dir with the SPA fallback, then drives headless Chrome
# through every sitemap route and confirms each one actually renders the LIVE app
# — WITHOUT pumping the event loop, so a stalled/crashed/blank render fails instead
# of being masked (the way the waitForFunction-based screenshot test masked the
# wasm-split blank-homepage regression).
#
# Requires Chrome (CHROME_BIN or auto-detected) + node + puppeteer-core; SKIPs loudly otherwise.
#
# Usage: ./scripts/test-ssg-render.sh [built-public-dir]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PUBLIC="${1:-target/dx/logicaffeine-web/release/web/public}"
PORT="${SSG_RENDER_PORT:-8783}"

CHROME="${CHROME_BIN:-$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || true)}"
if [ -z "$CHROME" ]; then
    echo "test-ssg-render: SKIP — no Chrome binary (set CHROME_BIN)" >&2
    exit 0
fi
command -v node >/dev/null || { echo "test-ssg-render: SKIP — node not installed" >&2; exit 0; }
[ -d "$PUBLIC" ] || { echo "test-ssg-render: build dir $PUBLIC missing" >&2; exit 1; }

if ! node -e "require('puppeteer-core')" 2>/dev/null && [ ! -d "$ROOT/node_modules/puppeteer-core" ] && [ ! -d /tmp/pptr/node_modules/puppeteer-core ]; then
    echo "test-ssg-render: SKIP — puppeteer-core not installed (npm install --no-save puppeteer-core)" >&2
    exit 0
fi

python3 - "$PUBLIC" "$PORT" <<'PY' &
import functools, http.server, os, sys
root, port = sys.argv[1], int(sys.argv[2])
class SpaHandler(http.server.SimpleHTTPRequestHandler):
    def send_head(self):
        path = self.translate_path(self.path)
        if os.path.isdir(path) and os.path.isfile(os.path.join(path, 'index.html')):
            return super().send_head()
        if not os.path.isfile(path):
            self.path = '/index.html'
        return super().send_head()
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(("127.0.0.1", port), functools.partial(SpaHandler, directory=root)).serve_forever()
PY
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT
sleep 1

NODE_PATH_EXTRA=""
[ -d /tmp/pptr/node_modules ] && NODE_PATH_EXTRA="/tmp/pptr/node_modules"
CHROME_BIN="$CHROME" NODE_PATH="${NODE_PATH:-}:$NODE_PATH_EXTRA" \
    node "$ROOT/scripts/test-ssg-render.mjs" "http://127.0.0.1:$PORT" "$CHROME"
