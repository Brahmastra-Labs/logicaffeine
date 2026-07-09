#!/usr/bin/env bash
# WS5 (work/FINISH_INTERPRETER.md Phase 12) — true-multicore WebAssembly, verified headlessly.
#
# Compiles the single-file probe `scripts/wasm-threads/worker_probe.rs` for wasm32 with
# ATOMICS + SHARED MEMORY (the same build a browser Web-Worker pool needs), then drives it from
# node `worker_threads` (real OS threads — the headless analog of browser Web Workers). The
# module only clears its concurrency barrier if N threads make progress at once, so a PASS is
# proof of genuine multicore — not a cooperative fake.
#
# No crate, no `build-std`, no nightly: the atomic ops inline into the probe so stock `core`
# suffices, and a raw `rustc` emits a ~800-byte module. This is the load-bearing primitive under
# WS5; the browser worker-pool reuses this exact build + shared-memory + atomics shape, and the
# determinism contract (a worker-driven run equals the cooperative run byte-for-byte on the same
# seed) is the already-proven native cooperative==work-stealing equivalence
# (`concurrency_differential.rs`).
#
# Requirements (auto-checked): node (v18+), the wasm32 target.
#
# Usage: ./scripts/test-wasm-threads.sh [numWorkers] [iters]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/scripts/wasm-threads/worker_probe.rs"
RUNNER="$ROOT/scripts/wasm-threads/run.mjs"
WORKERS="${1:-4}"
ITERS="${2:-1000000}"

fail() { echo "ERROR: $*" >&2; exit 1; }

command -v node >/dev/null 2>&1 || fail "node not found (needed to drive the worker_threads)"
command -v rustc >/dev/null 2>&1 || fail "rustc not found"
rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown \
    || fail "wasm32 target missing — 'rustup target add wasm32-unknown-unknown'"

WASM="$(mktemp /tmp/worker_probe-XXXX.wasm)"
trap 'rm -f "$WASM"' EXIT

echo "==> building the threads probe (wasm32, +atomics, shared+imported memory; raw rustc, no build-std)"
# 2 MiB initial / 16 MiB max — must match run.mjs's WebAssembly.Memory pages.
rustc --target wasm32-unknown-unknown --crate-type=cdylib -O \
    -C target-feature=+atomics,+bulk-memory,+mutable-globals \
    -C link-arg=--shared-memory \
    -C link-arg=--import-memory \
    -C link-arg=--initial-memory=2097152 \
    -C link-arg=--max-memory=16777216 \
    -C link-arg=--export=__stack_pointer \
    "$SRC" -o "$WASM"

echo "==> running $WORKERS node workers over one shared WebAssembly.Memory ($ITERS iters each)"
timeout 120 node "$RUNNER" "$WASM" "$WORKERS" "$ITERS"

echo "==> true-multicore WebAssembly verified under node (no browser needed)"
