#!/bin/bash
# Docs-quality gate — the same rustdoc-lint set the deployed site enforces
# (apps/logicaffeine_docs/build.sh). Fails on dead intra-doc links or bare URLs
# anywhere in the workspace docs, including the crate READMEs rendered via
# include_str!. Scoped to rustdoc lints, not blanket -D warnings, so unrelated
# dead-code/unused warnings don't block the docs.
#
# Usage: ./scripts/check-docs.sh
set -e
cd "$(dirname "$0")/.."

export RUSTDOCFLAGS="${RUSTDOCFLAGS:--D rustdoc::broken_intra_doc_links -D rustdoc::bare_urls}"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"

cargo doc --no-deps --workspace
echo "OK: cargo doc --workspace has no broken intra-doc links or bare URLs"
