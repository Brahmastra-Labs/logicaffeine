#!/usr/bin/env bash
# Builds the Arena entry's binary from the current tree with the entry's recipe (the Arena's rustc,
# `--locked`, arena profile, target-cpu=native, PGO trained on init-prelude
# with the run line) in its own target directory, and copies it to a content-addressed snapshot, so a
# later rebuild of the shared tree cannot change what a long measurement runs. Prints the snapshot path.
# `arena_entry_lock` holds its RUSTFLAGS to the entry's.
#   [TRAIN=<export>] [ARENA_RUSTC=<version>] scripts/lean/arena-crown/snapshot.sh [build-log]
# TRAIN is the PGO training input (default: the vendored init-prelude, as the entry uses). As in the
# entry's `set -eu` build, a training input that is missing or not accepted stops the build: a
# profile trained on nothing ships a binary about a fifth slower, with no other symptom.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
LOG="${1:-$ROOT/logs/arena-crown/snapshot-build.log}"
TGT="$ROOT/target/arena-costs"
PGO="$TGT/pgo"
TRAIN="${TRAIN:-$ROOT/crates/logicaffeine_tests/tests/fixtures/arena_corpus/good/init-prelude.ndjson}"
ARENA_RUSTC="${ARENA_RUSTC:-1.95.0}"
PROFDATA="$(rustc +"$ARENA_RUSTC" --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata"
[ -f "$TRAIN" ] || { echo "snapshot: no training input at $TRAIN" >&2; exit 1; }
[ -x "$PROFDATA" ] || { echo "snapshot: rustup component add llvm-tools --toolchain $ARENA_RUSTC" >&2; exit 1; }
mkdir -p "$ROOT/logs/arena-crown/bin" "$(dirname "$LOG")"
rm -rf "$PGO"
(cd "$ROOT" && CARGO_TARGET_DIR="$TGT" RUSTFLAGS="-C target-cpu=native -C profile-generate=$PGO" \
  cargo +"$ARENA_RUSTC" build --locked --profile arena -p logicaffeine-kernel --example logos_kernel >"$LOG" 2>&1)
"$TGT/arena/examples/logos_kernel" --check sem "$TRAIN" >/dev/null 2>>"$LOG" \
  || { echo "snapshot: the training run did not accept $TRAIN (see $LOG)" >&2; exit 1; }
"$PROFDATA" merge -o "$PGO/merged.profdata" "$PGO"
(cd "$ROOT" && CARGO_TARGET_DIR="$TGT" RUSTFLAGS="-C target-cpu=native -C profile-use=$PGO/merged.profdata" \
  cargo +"$ARENA_RUSTC" build --locked --profile arena -p logicaffeine-kernel --example logos_kernel >>"$LOG" 2>&1)
SHA="$(sha256sum "$TGT/arena/examples/logos_kernel" | cut -c1-16)"
BIN="$ROOT/logs/arena-crown/bin/logos_kernel-costs-$SHA"
if [ ! -f "$BIN" ]; then
  TMP="$(mktemp "$BIN.XXXX")"
  cp "$TGT/arena/examples/logos_kernel" "$TMP"
  chmod 755 "$TMP"
  mv "$TMP" "$BIN"
fi
echo "$BIN"
