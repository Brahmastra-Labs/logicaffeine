#!/usr/bin/env bash
# Poll until logicaffeine_compile + its tests compile again (concurrent shared-tree
# breakage clears), then run ONLY the wasm AOT lock to verify SeqFloat. Single test
# binary — honors "never run multiple suites at once".
set -u
cd "$(dirname "$0")/.." || exit 2
export Z3_SYS_Z3_HEADER=/usr/include/z3.h
LOG=logs/wasm-seqfloat-verify.log
mkdir -p logs
echo "poll start" > "$LOG"
for i in $(seq 1 80); do
  if cargo check -p logicaffeine-compile --features wasm-jit --tests >/tmp/wasm_poll_check.txt 2>&1; then
    echo "TREE COMPILES on attempt $i" >> "$LOG"
    cargo test -p logicaffeine-compile --features wasm-jit --test wasm_aot_lock \
      >/tmp/wasm_lock_run.txt 2>&1
    code=$?
    tail -50 /tmp/wasm_lock_run.txt >> "$LOG"
    echo "LOCK_EXIT=$code" >> "$LOG"
    exit $code
  fi
  echo "attempt $i: tree still broken (concurrent), waiting 30s" >> "$LOG"
  sleep 30
done
echo "GAVE UP after 80 attempts (~40min) — tree still broken" >> "$LOG"
exit 3
