#!/usr/bin/env bash
# Fair head-to-head benchmark of the Logos wire codec vs industry serializers.
#
#   ./scripts/bench-wire-vs-protocols.sh          # pure-Rust competitors (no toolchain)
#   ./scripts/bench-wire-vs-protocols.sh --arrow   # + Arrow (pure-Rust, no toolchain)
#   ./scripts/bench-wire-vs-protocols.sh --heavy   # + Arrow + protobuf + Cap'n Proto
#
# The core run compares against bincode, postcard, MessagePack, CBOR and JSON — all
# pure-Rust, no external tools. `--arrow` adds Arrow IPC (our columnar/zero-copy
# sibling), which is also pure-Rust and needs nothing installed. `--heavy` adds the
# prost (protobuf) and capnp (Cap'n Proto) benches too, installing `protoc` and the
# `capnp` compiler first if they are missing. Same logical data, same machine;
# output is a size + encode-ns + decode-ns table per payload, logged under logs/.
set -euo pipefail
cd "$(dirname "$0")/.."

ITERS="${WIREBENCH_ITERS:-30000}"
mkdir -p logs
ts="$(date +%Y%m%d-%H%M%S)"
log="logs/wirebench-${ts}.log"

features=""
case "${1:-}" in
  --arrow)
    features="--features arrow-bench"
    echo ">> arrow mode: pure-Rust columnar sibling (no toolchain needed)"
    ;;
  --heavy)
    echo ">> heavy mode: ensuring protoc + capnp are installed"
    if ! command -v protoc >/dev/null 2>&1; then
      echo "   installing protobuf-compiler (protoc) …"
      sudo apt-get update -qq && sudo apt-get install -y -qq protobuf-compiler || {
        echo "   !! could not install protoc automatically — install it and re-run"; exit 1; }
    fi
    if ! command -v capnp >/dev/null 2>&1; then
      echo "   installing capnproto (capnp) …"
      sudo apt-get install -y -qq capnproto || {
        echo "   !! could not install capnp automatically — install it and re-run"; exit 1; }
    fi
    features="--features heavy"
    echo "   protoc: $(protoc --version)   capnp: $(capnp --version)"
    ;;
esac

echo ">> building + running wirebench (iters=${ITERS}) ${features}"
WIREBENCH_ITERS="${ITERS}" cargo run --release -p logicaffeine-wirebench ${features} 2>&1 | tee "${log}"

echo
echo ">> full output: ${log}"
echo ">> reading: size = encoded bytes (no envelope); enc/dec = ns per whole-message op."
echo ">> smaller size + lower ns is better. 'logos (fixed)' is the memcpy speed dial;"
echo "   'logos (varint)' is the smallest-wire default."
