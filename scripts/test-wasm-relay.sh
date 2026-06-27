#!/usr/bin/env bash
# Headless-browser test for the thin WebSocket relay (FINISH_INTERPRETER.md Phase 9c).
#
# A browser tab cannot host a relay, so this orchestrates the integration:
#   1. start a NATIVE relay host (a real libp2p-free WS relay) on 127.0.0.1:9944,
#   2. run the REAL browser relay client (`relay_browser::RelayBrowserClient`,
#      a web-sys WebSocket) inside a headless browser via wasm-pack,
#   3. the browser subscribes + publishes through the native relay and asserts the
#      round-trip (tests/wasm_relay.rs).
#
# For LOCAL runs without a browser, use `scripts/test-wasm-node.sh` — it runs the
# same wasm_relay + wasm_interp_net clients under node with a `ws` WebSocket
# polyfill. This headless-browser script additionally covers the OPFS large-buffer
# test (`navigator.storage`, which has no node equivalent), so it is the CI path.
#
# Requirements (CI provides these; a dev box needs them installed):
#   - wasm-pack            (cargo install wasm-pack)
#   - a headless browser   (chromium/chrome or firefox + the matching driver)
#   - the wasm32 target    (rustup target add wasm32-unknown-unknown)
#
# Usage: ./scripts/test-wasm-relay.sh [--firefox]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="$ROOT/crates/logicaffeine_system"
PORT=9944
BROWSER="${1:---chrome}"   # --chrome (default) or --firefox

echo "==> building + starting the native relay host on 127.0.0.1:$PORT"
cargo run --quiet -p logicaffeine-system --features relay \
    --example relay_host -- "127.0.0.1:$PORT" &
RELAY_PID=$!
cleanup() { kill "$RELAY_PID" 2>/dev/null || true; }
trap cleanup EXIT

# Wait for the relay to accept connections (poll the port).
for _ in $(seq 1 50); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then exec 3>&- 3<&-; break; fi
    sleep 0.2
done

echo "==> running the headless-browser RELAY CLIENT test ($BROWSER)"
wasm-pack test --headless "$BROWSER" "$ROOT/crates/logicaffeine_system" -- --test wasm_relay

echo "==> running the headless-browser INTERPRETER networking test ($BROWSER)"
wasm-pack test --headless "$BROWSER" "$ROOT/crates/logicaffeine_compile" -- --test wasm_interp_net

echo "==> running the headless OPFS large-buffer test ($BROWSER)"
wasm-pack test --headless "$BROWSER" "$ROOT/crates/logicaffeine_system" \
    --features persistence -- --test wasm_opfs

echo "==> headless relay + interpreter + OPFS round-trips PASSED"
