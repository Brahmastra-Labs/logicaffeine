#!/usr/bin/env bash
# A private, minimal workspace (logicaffeine_base + logicaffeine_kernel, its dag/arena suites, its
# paradox corpus, its oracle differential, the LOGOS-theory parity, mirror and observational-equality locks
# and the recursion and defect conformance suites) for
# mutation tests and experiments, so that no session ever builds a mutant of the shared tree.
#   scripts/lean/arena-crown/private-ws.sh [dir]   → prints the workspace path
# Mutate files under <dir>/crates/logicaffeine_kernel/src, then build or test there, e.g.
#   (cd <dir> && cargo test -p logicaffeine-kernel --test dag_sem_lsub)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
WS="${1:-$ROOT/target/private-ws}"
mkdir -p "$WS/crates"
# By content, not preserving times: a file restored after a mutant must look newer than the
# mutant's build, or cargo keeps the mutant's library and every later test runs against it.
rsync -rlp --checksum --delete --exclude target "$ROOT/crates/logicaffeine_base/" "$WS/crates/logicaffeine_base/"
rsync -rlp --checksum --delete --exclude target "$ROOT/crates/logicaffeine_kernel/" "$WS/crates/logicaffeine_kernel/"
# The kernel's dag/arena suites live in crates/logicaffeine_tests; the private copy gets them back as
# its own integration tests, with the corpora they read, in the layout they resolve against.
HOME_TESTS="$ROOT/crates/logicaffeine_tests/tests"
KT="$WS/crates/logicaffeine_kernel/tests"
mkdir -p "$KT/fixtures"
rsync -rlp --checksum "$HOME_TESTS"/dag_*.rs "$HOME_TESTS"/arena_corpus.rs "$HOME_TESTS"/arena_corpus_lock.rs \
  "$HOME_TESTS"/kernel_paradox_corpus.rs "$HOME_TESTS"/kernel_oracle_differential.rs "$HOME_TESTS"/theory_logos_parity.rs \
  "$HOME_TESTS"/context_admission_log_lock.rs "$HOME_TESTS"/kernel_observational.rs "$HOME_TESTS"/dag_observational.rs \
  "$HOME_TESTS"/lean_conformance_defects.rs "$HOME_TESTS"/lean_conformance_t04_recursion.rs "$KT/"
rsync -rlp --checksum --delete "$HOME_TESTS/arena_corpus/" "$KT/arena_corpus/"
for corpus in arena_corpus crafted; do
  rsync -rlp --checksum --delete "$HOME_TESTS/fixtures/$corpus/" "$KT/fixtures/$corpus/"
done
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
echo "$WS"
