#!/usr/bin/env bash
# Learn Logic in a real browser (see test-learn-e2e.mjs).
#
# Drives a headless browser through the Learn page of a BUILT site: keys, clicks, the caret,
# scrolling, focus, the network and storage, the interaction layer the Rust unit and SSR tests
# cannot reach. The .mjs serves the build itself (production SPA fallback, a free port), so
# nothing collides with a running dev server.
#
# Requires Chrome (CHROME_BIN or auto-detected) + node + puppeteer-core; with --firefox it drives
# Firefox (FIREFOX_BIN) instead. Like the SSG scripts it SKIPs loudly when those are missing,
# unless --require-browser is given (scripts/test/tdd.sh always passes it), because a verdict
# must never be read off a suite that did not run.
#
# Usage: ./scripts/web/test-learn-e2e.sh <built-public-dir> [--firefox] [--junit report.xml]
#                                    [--only a,b] [--shots dir] [--require-browser]
#        ./scripts/web/test-learn-e2e.sh --list
#
# Build first, e.g.: dx build -p logicaffeine-web
#   then: ./scripts/web/test-learn-e2e.sh target/dx/logicaffeine-web/debug/web/public

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

REQUIRE=0
ENGINE=chrome
ARGS=()
PUBLIC=""
for arg in "$@"; do
  case "$arg" in
    --require-browser) REQUIRE=1 ;;
    --firefox) ENGINE=firefox ;;
    *) ARGS+=("$arg") ;;
  esac
done

skip() {
  if [ "$REQUIRE" = 1 ]; then
    echo "test-learn-e2e: ERROR — $1" >&2
    exit 2
  fi
  echo "test-learn-e2e: SKIP — $1" >&2
  exit 0
}

command -v node >/dev/null || skip "node not installed"
node -e "require('puppeteer-core')" 2>/dev/null \
  || [ -d "$ROOT/node_modules/puppeteer-core" ] \
  || skip "puppeteer-core not installed (npm install --no-save puppeteer-core, or put it on NODE_PATH)"

if [ "${ARGS[0]:-}" = "--list" ]; then
  exec node "$ROOT/scripts/web/test-learn-e2e.mjs" --list
fi

PUBLIC="${ARGS[0]:-}"
[ -n "$PUBLIC" ] || { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
[ -f "$PUBLIC/index.html" ] || { echo "test-learn-e2e: no built site at $PUBLIC (expected index.html)" >&2; exit 2; }

if [ "$ENGINE" = firefox ]; then
  FIREFOX="${FIREFOX_BIN:-$(command -v firefox || true)}"
  [ -n "$FIREFOX" ] || skip "no Firefox binary (set FIREFOX_BIN)"
  BROWSER=(--firefox "$FIREFOX")
else
  CHROME="${CHROME_BIN:-$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || true)}"
  [ -n "$CHROME" ] || skip "no Chrome binary (set CHROME_BIN)"
  BROWSER=(--chrome "$CHROME")
fi

exec node "$ROOT/scripts/web/test-learn-e2e.mjs" --public "$PUBLIC" "${BROWSER[@]}" "${ARGS[@]:1}"
