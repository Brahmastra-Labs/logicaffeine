#!/usr/bin/env bash
# Emit the background-prefetch manifest for the deploy artifact.
#
# A code-split build (the `split` feature + dx `--wasm-split` — off in deploy
# builds while the fork's splitter miscompiles the wasm; see deploy-frontend.yml)
# puts every `#[component(lazy)]` route body in its own content-hashed
# `module_*.wasm` / `chunk_*.wasm` blob. That keeps the first paint cheap, but the FIRST visit to
# every other page then blocks on downloading that page's chunk. To hide that
# cost, the shell (index.html) warms every chunk in the background once the first
# page is idle — but it can only warm URLs it knows, and the hashes are only
# known here, after the build. So this script writes the list it reads:
#
#   public/chunks.json = [ { "url": "/assets/module_<hash>.wasm", "bytes": N }, ... ]
#
# The eager main bundle (`logicaffeine-web_bg-*.wasm`) is EXCLUDED — every visitor
# already downloads it on first paint, so prefetching it would be pure waste.
# Entries are sorted smallest-first so more pages become "ready" sooner. A build
# with no split chunks (today's deploy build, or a plain `dx serve`) yields `[]`,
# and the runtime prefetch no-ops.
#
# Usage: ./scripts/web/gen-prefetch-manifest.sh [web-dir]
#        (default: target/dx/logicaffeine-web/release/web)
# Run it AFTER `dx build --release --ssg --fullstack`
# (and after merge-ssg.sh, since both operate on public/).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WEB="${1:-target/dx/logicaffeine-web/release/web}"

[ -d "$WEB/public" ] || { echo "gen-prefetch-manifest: $WEB/public does not exist (run dx build first)" >&2; exit 1; }

python3 - "$WEB" <<'PY'
import json, pathlib, sys

web = pathlib.Path(sys.argv[1])
assets = web / "public" / "assets"

# The lazy split chunks — NOT the eager main bundle (logicaffeine-web_bg-*.wasm),
# which is loaded on first paint. Match the same content-hashed families that
# public/_headers marks immutable.
chunks = []
if assets.is_dir():
    for wasm in assets.glob("*.wasm"):
        name = wasm.name
        if name.startswith("module_") or name.startswith("chunk_"):
            chunks.append({"url": f"/assets/{name}", "bytes": wasm.stat().st_size})

chunks.sort(key=lambda c: (c["bytes"], c["url"]))

out = web / "public" / "chunks.json"
out.write_text(json.dumps(chunks) + "\n")

total = sum(c["bytes"] for c in chunks)
mb = total / 1024 / 1024
print(f"gen-prefetch-manifest: wrote {out} — {len(chunks)} chunk(s), {mb:.2f} MB total")
PY
