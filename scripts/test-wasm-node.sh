#!/usr/bin/env bash
# Browser-networking wasm tests, run LOCALLY under node (FINISH_INTERPRETER.md Phase 9c).
#
# A browser tab cannot host a relay, so this orchestrates the integration the same
# way the headless-browser path does — but without a browser:
#   1. start a NATIVE relay host (a real libp2p-free WS relay) on 127.0.0.1:9944,
#   2. run the REAL browser relay client (`relay_browser::RelayBrowserClient`, a
#      `web-sys` WebSocket) under node via wasm-bindgen-test-runner, with a tiny
#      `ws` polyfill supplying the browser `WebSocket` global,
#   3. the wasm clients subscribe/publish + drive `interpret_for_ui`'s Connect+Sync
#      through the native relay and assert the round-trips.
#
# This is what makes "bundle the headless browser in the test and run the relay
# itself" real in our own suite. The one test that needs a true browser — OPFS
# (`navigator.storage`, no node equivalent) — stays on `scripts/test-wasm-relay.sh`.
#
# Requirements (auto-checked):
#   - node                         (any recent v18+)
#   - wasm-bindgen-test-runner     (cargo install wasm-bindgen-cli, matching wasm-bindgen)
#   - the wasm32 target            (rustup target add wasm32-unknown-unknown)
#   - the `ws` npm package         (auto-installed into scripts/wasm-node/ if missing)
#
# Usage: ./scripts/test-wasm-node.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT=9944
POLYFILL="$ROOT/scripts/wasm-node/ws-polyfill.js"

fail() { echo "ERROR: $*" >&2; exit 1; }

command -v node >/dev/null 2>&1 || fail "node not found (needed to run wasm-bindgen tests without a browser)"
command -v wasm-bindgen-test-runner >/dev/null 2>&1 \
    || fail "wasm-bindgen-test-runner not found — 'cargo install wasm-bindgen-cli' (match the wasm-bindgen version)"
rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown \
    || fail "wasm32 target missing — 'rustup target add wasm32-unknown-unknown'"

# Ensure the `ws` WebSocket polyfill dependency is present.
if [ ! -e "$ROOT/scripts/wasm-node/node_modules/ws/package.json" ]; then
    echo "==> installing the 'ws' polyfill dependency (scripts/wasm-node)"
    ( cd "$ROOT/scripts/wasm-node" && npm install --silent )
fi

echo "==> building + starting the native relay host on 127.0.0.1:$PORT"
cargo build --quiet -p logicaffeine-system --features relay --example relay_host
"$ROOT/target/debug/examples/relay_host" "127.0.0.1:$PORT" &
RELAY_PID=$!
cleanup() { kill "$RELAY_PID" 2>/dev/null || true; }
trap cleanup EXIT

# Wait for the relay to accept connections (poll the port).
for _ in $(seq 1 50); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then exec 3>&- 3<&-; break; fi
    sleep 0.2
done

export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner
export NODE_OPTIONS="--require $POLYFILL"

echo "==> [1/3] browser relay client round-trip (logicaffeine-system :: wasm_relay)"
cargo test -p logicaffeine-system --target wasm32-unknown-unknown --test wasm_relay

echo "==> [2/4] browser interpreter Connect+Sync over the relay (logicaffeine-compile :: wasm_interp_net)"
cargo test -p logicaffeine-compile --target wasm32-unknown-unknown --test wasm_interp_net

echo "==> [3/4] browser concurrency drive loop (logicaffeine-compile :: wasm_concurrency)"
cargo test -p logicaffeine-compile --target wasm32-unknown-unknown --test wasm_concurrency

# WS6: the WASM-JIT emits modules that run on the HOST's real WebAssembly (node's V8 here,
# the browser's in production) — not the wasmi interpreter the native differential uses.
# Needs --features wasm-jit; cross-checks every emitted module against the bytecode VM and
# pins the i64↔BigInt boundary across the full i64 range.
echo "==> [4/4] browser-native WASM-JIT on real V8 (logicaffeine-compile :: wasm_jit_browser)"
cargo test -p logicaffeine-compile --target wasm32-unknown-unknown --features wasm-jit --test wasm_jit_browser

echo "==> browser-networking + concurrency + WASM-JIT wasm tests PASSED under node (no headless browser needed)"
