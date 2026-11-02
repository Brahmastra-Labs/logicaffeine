#!/usr/bin/env bash
# Print the `--test <binary>` flags for every test binary named in
# scripts/test/wasm-jit-filter.txt (the ONE source of truth for the wasm-jit pass), so the
# runners build ONLY those binaries. cargo would otherwise build every test target of
# the crate for the pass, and an unrelated test file mid-edit in another session could
# take the whole frontend pass down with it. `wasm_gated_suite_lock` pins that both
# runners go through this script.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
grep -o 'binary([a-z_0-9]*)' "$here/wasm-jit-filter.txt" | sed 's/binary(\(.*\))/--test \1/' | tr '\n' ' '
