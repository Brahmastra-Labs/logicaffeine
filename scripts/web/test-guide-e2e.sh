#!/usr/bin/env bash
# The Syntax Guide in a real browser (see test-guide-e2e.mjs).
#
# Rests a pointer on the words of the guide's examples, exercises and prose snippets and reads
# the language server's hover, types mistakes and watches the squiggles, measures each editor
# against the code it holds, and checks the corners of every callout with a coloured left edge.
# What the page renders natively is pinned by crates/logicaffeine_tests/tests/guide_render.rs and
# what the language server says about the guide's code by tests/guide_language_server.rs; this is
# the layer they cannot reach.
#
# Requires Chrome (CHROME_BIN or auto-detected) + node + puppeteer-core; with --firefox it drives
# Firefox instead (FIREFOX_BIN or auto-detected). It SKIPs loudly when those are missing — unless
# --require-browser is given, because a verdict must never be read off a suite that did not run.
#
# Usage: ./scripts/web/test-guide-e2e.sh <built-public-dir | http://running-site>
#                                    [--firefox] [--junit report.xml] [--only a,b] [--shots dir]
#                                    [--require-browser]
#        ./scripts/web/test-guide-e2e.sh --list
#
# Against a build: dx build -p logicaffeine-web
#   then: ./scripts/web/test-guide-e2e.sh target/dx/logicaffeine-web/debug/web/public
# Against a dev server: ./scripts/web/test-guide-e2e.sh http://127.0.0.1:8080

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
    echo "test-guide-e2e: ERROR — $1" >&2
    exit 2
  fi
  echo "test-guide-e2e: SKIP — $1" >&2
  exit 0
}

command -v node >/dev/null || skip "node not installed"
node -e "require('puppeteer-core')" 2>/dev/null \
  || [ -d "$ROOT/node_modules/puppeteer-core" ] \
  || skip "puppeteer-core not installed (npm install --no-save puppeteer-core, or put it on NODE_PATH)"

if [ "${ARGS[0]:-}" = "--list" ]; then
  exec node "$ROOT/scripts/web/test-guide-e2e.mjs" --list
fi

SITE="${ARGS[0]:-}"
[ -n "$SITE" ] || { sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
case "$SITE" in
  http://*|https://*) TARGET=(--url "$SITE") ;;
  *)
    [ -f "$SITE/index.html" ] || { echo "test-guide-e2e: no built site at $SITE (expected index.html)" >&2; exit 2; }
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

exec node "$ROOT/scripts/web/test-guide-e2e.mjs" "${TARGET[@]}" "${BROWSER[@]}" "${ARGS[@]:1}"
