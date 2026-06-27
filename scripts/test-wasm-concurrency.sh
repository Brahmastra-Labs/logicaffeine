#!/usr/bin/env bash
# Cross-target runtime trait tests on the BROWSER runtime, run under node (Phase 9a/9b).
#
# The native side (`NativeRuntime` over tokio) is covered by the in-crate `#[tokio::test]`s
# (`cargo nextest run -p logicaffeine-system --features concurrency`). THIS runs the wasm
# twin — `BrowserRuntime`'s `Yield`/`Timer`/`Spawner` (global `setTimeout` + `spawn_local`)
# and the shared tokio-sync `Pipe` channel — as real wasm under node via
# `wasm-bindgen-test-runner`. No browser, no relay, no polyfill needed (the timer binds the
# GLOBAL `setTimeout`, present in node and the browser alike).
#
# Requirements (auto-checked):
#   - node                       (any recent v18+)
#   - wasm-bindgen-test-runner   (cargo install wasm-bindgen-cli, matching wasm-bindgen)
#   - the wasm32 target          (rustup target add wasm32-unknown-unknown)
#
# Usage: ./scripts/test-wasm-concurrency.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() { echo "ERROR: $*" >&2; exit 1; }

command -v node >/dev/null 2>&1 || fail "node not found"
command -v wasm-bindgen-test-runner >/dev/null 2>&1 \
    || fail "wasm-bindgen-test-runner not found — 'cargo install wasm-bindgen-cli' (match the wasm-bindgen version)"
rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown \
    || fail "wasm32 target missing — 'rustup target add wasm32-unknown-unknown'"

export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner

echo "Running BrowserRuntime cross-target tests as wasm under node..."
cargo test -p logicaffeine-system --features concurrency \
    --target wasm32-unknown-unknown --test wasm_concurrency
