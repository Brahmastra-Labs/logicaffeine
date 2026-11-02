#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

# Gate on the rustdoc lints that render as visibly broken pages — dead intra-doc
# links, links to private items (which the published pages cannot reach), and bare
# URLs — so a broken link in any crate's docs (READMEs included) fails the build
# instead of shipping. Scoped to rustdoc lints on purpose: this is a docs-quality
# gate, not an unrelated dead-code/unused gate. Override with RUSTDOCFLAGS="" for
# a fully permissive local build.
export RUSTDOCFLAGS="${RUSTDOCFLAGS--D rustdoc::broken_intra_doc_links -D rustdoc::private_intra_doc_links -D rustdoc::bare_urls}"

# The curated set includes the Z3-backed crates (verify/tv); default the header
# path so a plain local run works (same seam as run-all-tests-fast.sh).
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"

# Every publishable crate, in presentation order, plus the web app (deliberate
# extra). readme_lock.rs asserts this list covers the publishable set.
CRATES=(
    logicaffeine_language
    logicaffeine_compile
    logicaffeine_proof
    logicaffeine_kernel
    logicaffeine_verify
    logicaffeine_tv
    logicaffeine_lexicon
    logicaffeine_base
    logicaffeine_data
    logicaffeine_system
    logicaffeine_runtime
    logicaffeine_forge
    logicaffeine_jit
    logicaffeine_lsp
    logicaffeine_cli
    logicaffeine_web
)

# site.py documents exactly these libraries in one cargo doc — so rustdoc's
# search index, crate list and implementor scripts name only pages the site
# ships — with every crate's doc directory made first, so every cross-crate type
# links to its docs; then it assembles and gates the site (see its module docs).
python3 apps/logicaffeine_docs/site.py build \
    --source . \
    --out apps/logicaffeine_docs/dist \
    --routes apps/logicaffeine_docs/routes.toml \
    "${CRATES[@]}"
