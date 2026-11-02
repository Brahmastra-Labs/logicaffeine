#!/usr/bin/env bash
# The Studio in a real browser (see test-studio-e2e.mjs).
#
# Drags the Studio's column bars, its debugger drawer grip and its phone split bar in headless
# Chrome and measures what the page paints: which columns move, their minimums, the window
# changing size, an iframe under the pointer. The layout rules themselves are unit-tested in
# apps/logicaffeine_web/src/ui/pages/studio_layout.rs; this is the layer they cannot reach.
#
# Requires Chrome (CHROME_BIN or auto-detected) + node + puppeteer-core; with --firefox it drives
# Firefox instead (FIREFOX_BIN or auto-detected), the browser the Studio's display box runs. It
# SKIPs loudly when those are missing — unless --require-browser is given, because a verdict must
# never be read off a suite that did not run.
#
# Usage: ./scripts/web/test-studio-e2e.sh <built-public-dir | http://running-site>
#                                     [--firefox] [--junit report.xml] [--only a,b] [--shots dir]
#                                     [--require-browser]
#        ./scripts/web/test-studio-e2e.sh --list
#
# Against a build: dx build -p logicaffeine-web
#   then: ./scripts/web/test-studio-e2e.sh target/dx/logicaffeine-web/debug/web/public
# Against a dev server: ./scripts/web/test-studio-e2e.sh http://127.0.0.1:8080

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

REQUIRE=0
ENGINE=chrome
ARGS=()
for arg in "$@"; do
  case "$arg" in
    --require-browser) REQUIRE=1 ;;
    --firefox) ENGINE=firefox ;;
    *) ARGS+=("$arg") ;;
  esac
done

skip() {
  if [ "$REQUIRE" = 1 ]; then
    echo "test-studio-e2e: ERROR — $1" >&2
    exit 2
  fi
  echo "test-studio-e2e: SKIP — $1" >&2
  exit 0
}

command -v node >/dev/null || skip "node not installed"
node -e "require('puppeteer-core')" 2>/dev/null \
  || [ -d "$ROOT/node_modules/puppeteer-core" ] \
  || skip "puppeteer-core not installed (npm install --no-save puppeteer-core, or put it on NODE_PATH)"

if [ "${ARGS[0]:-}" = "--list" ]; then
  exec node "$ROOT/scripts/web/test-studio-e2e.mjs" --list
fi

SITE="${ARGS[0]:-}"
[ -n "$SITE" ] || { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
case "$SITE" in
  http://*|https://*) TARGET=(--url "$SITE") ;;
  *)
    [ -f "$SITE/index.html" ] || { echo "test-studio-e2e: no built site at $SITE (expected index.html)" >&2; exit 2; }
    TARGET=(--public "$SITE") ;;
esac

if [ "$ENGINE" = firefox ]; then
  FIREFOX="${FIREFOX_BIN:-$(command -v firefox || true)}"
  [ -n "$FIREFOX" ] || skip "no Firefox binary (set FIREFOX_BIN)"
  BROWSER=(--firefox "$FIREFOX")
else
  CHROME="${CHROME_BIN:-$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || true)}"
  [ -n "$CHROME" ] || skip "no Chrome binary (set CHROME_BIN)"
  BROWSER=(--chrome "$CHROME")
fi

exec node "$ROOT/scripts/web/test-studio-e2e.mjs" "${TARGET[@]}" "${BROWSER[@]}" "${ARGS[@]:1}"
