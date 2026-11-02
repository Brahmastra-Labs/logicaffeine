#!/usr/bin/env bash
# Writes the Logicaffeine handbook to target/docs/LOGOS_DOCUMENTATION.md: the README's overview, Getting
# started, every guide, every crate's README and the reference material, as one book with one table of
# contents. The documents under docs/ and each crate's README are the source; the book is assembled from
# them by scripts/release/docs/handbook.py and never edited by hand.
#
#   ./scripts/generate-docs.sh                  the handbook
#   ./scripts/generate-docs.sh --with-source    the handbook plus the core source files, for a model's context
#   ./scripts/generate-docs.sh --out PATH       write somewhere else
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
exec python3 "$here/release/docs/handbook.py" "$@"
