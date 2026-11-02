#!/usr/bin/env bash
# Headless-browser smoke of a LOGOS PAGE — the framework's own output, not the marketing site.
#
# `scripts/web/test-ssg-smoke.sh` drives the Dioxus site in `apps/logicaffeine_web`. This drives what
# `largo new --web` produces: a LOGOS page compiled to WebAssembly, its markup pre-rendered into
# the document, and the module taking over in a real browser.
#
# ★ IT IS A THIN WRAPPER ON PURPOSE. The smoke itself lives in `browser_smoke.rs`, which builds
# the page through the SAME functions `largo build` calls. A shell script that assembled the site
# itself would be a second implementation of the build, free to drift from it and to SKIP for
# reasons the real build would never hit — the failure mode being a green smoke that tested
# nothing. So this sets the browser and hands over.
#
# `LOGOS_REQUIRE_TOOLS=chrome` turns a missing browser into a LOUD FAILURE rather than a skip,
# which is what CI wants; without a browser here, it exits 0 with a visible SKIP.
#
# Usage: ./scripts/web/test-web-smoke.sh   (CHROME_BIN overrides the browser)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

CHROME="${CHROME_BIN:-$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || true)}"
if [ -z "$CHROME" ]; then
    echo "test-web-smoke: SKIP — no Chrome/Chromium binary found (set CHROME_BIN)" >&2
    exit 0
fi

export CHROME_BIN="$CHROME"
# Required, not merely available: with a browser present, a skip would be a bug in the gate.
export LOGOS_REQUIRE_TOOLS=chrome

echo "test-web-smoke: driving $CHROME"
exec cargo test -p logicaffeine-tests --test browser_smoke -- --nocapture
