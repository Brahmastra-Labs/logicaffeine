#!/usr/bin/env bash
# Redraw the share card, public/assets/OG-photo.png, from apps/logicaffeine_web/share-card/card.html
# (see render-share-card.mjs). Run it after any change to the card, its logo or its words;
# tests/share_card_lock.rs goes red until you do.
#
# Requires Chrome (CHROME_BIN or auto-detected), node 22.2 or later and puppeteer-core, and fetches
# the card's pinned font over the network. A generator run on purpose: anything missing is an error.
#
# Usage: ./scripts/web/render-share-card.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

CHROME="${CHROME_BIN:-$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || true)}"
MAC_CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
if [ -z "$CHROME" ] && [ -x "$MAC_CHROME" ]; then CHROME="$MAC_CHROME"; fi
[ -n "$CHROME" ] || { echo "render-share-card: no Chrome binary (set CHROME_BIN)" >&2; exit 1; }
command -v node >/dev/null || { echo "render-share-card: node is not installed" >&2; exit 1; }

exec node "$ROOT/scripts/web/render-share-card.mjs" "$CHROME"
