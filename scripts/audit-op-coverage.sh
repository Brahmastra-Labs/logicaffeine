#!/usr/bin/env bash
# audit-op-coverage — a human-readable report of the soft-core opcode surface the universal-interpreter
# corpus drives through the chip. It runs `op_coverage_lock::corpus_exercises_core_opcode_surface` with
# output captured, so you see the exact opcode→mnemonic table the `compile_to_full_words` encoder emits
# for the `universal_completeness_lock` corpus (the words the FPGA actually executes). Green == the
# corpus exercises the required core surface; the printed list is the full exercised set.
#
#   scripts/audit-op-coverage.sh
#
# This is the reproducible companion to the CI ratchet: the lock FAILS if coverage regresses, this script
# SHOWS what is (and isn't) covered so a gap is obvious.
set -uo pipefail
cd "$(dirname "$0")/.."

echo "== soft-core op coverage (universal-interpreter corpus) =="
cargo nextest run -p logicaffeine-tests \
  -E 'test(corpus_exercises_core_opcode_surface)' \
  --no-capture 2>&1 \
  | grep -E '── soft-core|^  [0-9]{1,3}  [A-Za-z]|distinct opcodes|PASS|FAIL|regressed' \
  || { echo "coverage audit FAILED — see the assertion above"; exit 1; }
