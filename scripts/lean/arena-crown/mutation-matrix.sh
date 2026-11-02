#!/usr/bin/env bash
# The kernel-rule mutation matrix: every piece of the dag core's admission and conversion logic
# must be load-bearing. cargo-mutants disables or inverts one piece at a time (a comparison, a
# condition, a returned value), rebuilds, and runs the Arena corpus + adversarial tests; a mutant
# the tests do not catch is a rule no test holds.
#
#   scripts/lean/arena-crown/mutation-matrix.sh [--shard K/N] [--jobs J] [file ...]
#
# It works on a private, minimal workspace (logicaffeine_base + logicaffeine_kernel copied into
# the scratch directory), so no other session's files are ever mutated and nothing the repo
# gitignores is copied. Results: logs/arena-crown/mutants/<stamp>/ (outcomes.json, caught.txt,
# missed.txt, timeout.txt, unviable.txt).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
SHARD=""; JOBS=2; FILES=()
while [ $# -gt 0 ]; do
  case "$1" in
    --shard) SHARD="$2"; shift 2 ;;
    --jobs) JOBS="$2"; shift 2 ;;
    *) FILES+=("$1"); shift ;;
  esac
done
if [ ${#FILES[@]} -eq 0 ]; then
  FILES=(tc.rs inductive.rs check.rs level.rs primitive.rs primitive_wf.rs)
fi
STAMP="$(date +%Y%m%d-%H%M%S)"
WS="${MUTATION_WS:-$ROOT/target/mutation-ws-$STAMP}"
OUT="$ROOT/logs/arena-crown/mutants/$STAMP"
mkdir -p "$WS/crates" "$OUT"

# The minimal workspace: the two crates and a root manifest carrying the workspace metadata they
# inherit (package fields + the dependency table).
cp -r "$ROOT/crates/logicaffeine_base" "$ROOT/crates/logicaffeine_kernel" "$WS/crates/"
rm -rf "$WS/crates/logicaffeine_kernel/target" "$WS/crates/logicaffeine_base/target"
# Only the suites the matrix runs. They live in crates/logicaffeine_tests; the private copy of the
# kernel gets them back as its own integration tests, with the corpus they read, in the layout they
# resolve against (`CARGO_MANIFEST_DIR/tests/fixtures/arena_corpus`, `tests/arena_corpus/cases.rs`),
# so cargo-mutants runs them against each mutant of the kernel alone.
HOME_TESTS="$ROOT/crates/logicaffeine_tests/tests"
KT="$WS/crates/logicaffeine_kernel/tests"
mkdir -p "$KT/fixtures"
cp "$HOME_TESTS"/{arena_corpus,arena_corpus_lock,dag_mutations,dag_primitives}.rs "$KT/"
cp -r "$HOME_TESTS/arena_corpus" "$KT/"
cp -r "$HOME_TESTS/fixtures/arena_corpus" "$KT/fixtures/"
python3 - "$ROOT/Cargo.toml" "$WS/Cargo.toml" <<'PY'
import re, sys
src = open(sys.argv[1]).read()
def section(name):
    m = re.search(r'^\[' + re.escape(name) + r'\]\n(.*?)(?=^\[)', src, re.S | re.M)
    return m.group(1) if m else ""
out = ['[workspace]', 'resolver = "2"', 'members = ["crates/logicaffeine_base", "crates/logicaffeine_kernel"]', '',
       '[workspace.package]', section('workspace.package'),
       '[workspace.lints.rust]', section('workspace.lints.rust'),
       '[workspace.lints.rustdoc]', section('workspace.lints.rustdoc'),
       '[workspace.dependencies]', section('workspace.dependencies')]
open(sys.argv[2], 'w').write('\n'.join(out))
PY
cp "$ROOT/Cargo.lock" "$WS/Cargo.lock"
# The heavy corpus cases read reference-kernels/ through ../../; they are #[ignore]d and not run.

ARGS=(-p logicaffeine-kernel --test-tool nextest -j "$JOBS" -o "$OUT" --timeout-multiplier 4)
for f in "${FILES[@]}"; do ARGS+=(-f "crates/logicaffeine_kernel/src/dag/$f"); done
if [ -n "$SHARD" ]; then ARGS+=(--shard "$SHARD"); fi
echo "workspace $WS -> $OUT"
cd "$WS"
nice -n 10 cargo mutants "${ARGS[@]}" -- --test arena_corpus --test dag_mutations --test dag_primitives || true
echo "results: $OUT/mutants.out"
