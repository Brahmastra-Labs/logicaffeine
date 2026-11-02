#!/usr/bin/env bash
# Regenerates the kernel's primitive-validation reference terms with the pinned Lean toolchain.
set -euo pipefail
cd "$(dirname "$0")/../../../crates/logicaffeine_kernel/src/dag/primitive_refs"
LEAN="${LEAN:-$HOME/.elan/toolchains/leanprover--lean4---v4.29.1/bin/lean}"
"$LEAN" PrimitiveRefs.lean > lean-4.29.1.ndjson.tmp
mv lean-4.29.1.ndjson.tmp lean-4.29.1.ndjson
echo "wrote $(wc -l < lean-4.29.1.ndjson) records"
