#!/usr/bin/env bash
#
# gen-crypto.sh — generate the cryptanalysis arena: planted instances of the proof crate's `cipher` module, one
# category per rung of each ladder.
#
#   sha1_s<steps>_k<known>   a preimage of SHA-1's compression function cut to <steps> of its 80 steps, <known> of the
#                            block's 512 bits given; more given bits leave fewer preimages and make it harder
#   trivium_n<n>_k<known>    Trivium's 288-bit state recovered from <n> keystream bits, <known> state bits given
#
# Each instance is made from a known solution, so every one is satisfiable, and `cipher` checks a model by
# recomputing the cipher. The same DIMACS goes to every engine.
#
# Usage: bash benchmarks/arena/gen-crypto.sh [seeds]
set -euo pipefail
cd "$(dirname "$0")/../.."
SEEDS="${1:-3}"
DEST="benchmarks/arena/instances/crypto"
rm -rf "$DEST"
cargo build --release -p logicaffeine-proof --example cipher_instances
GEN="${CARGO_TARGET_DIR:-target}/release/examples/cipher_instances"
for rung in "17 0" "18 0" "18 128" "19 128" "19 192" "20 192" "20 256"; do
  set -- $rung
  "$GEN" sha1 "$1" "$2" "$SEEDS" "$DEST/sha1_s$1_k$2"
done
for rung in "288 155" "288 150" "288 145" "288 140" "320 135"; do
  set -- $rung
  "$GEN" trivium "$1" "$2" "$SEEDS" "$DEST/trivium_n$1_k$2"
done
echo "crypto arena: $(find "$DEST" -name '*.cnf' | wc -l | tr -d ' ') instances in $(ls "$DEST" | wc -l | tr -d ' ') categories → $DEST"
