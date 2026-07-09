#!/bin/bash
set -e
cd "$(dirname "$0")/../.."

# Gate on the rustdoc lints that render as visibly-broken pages — dead intra-doc
# links and bare URLs — so a broken link in any crate's docs (READMEs included)
# fails the build instead of shipping. Scoped to rustdoc lints on purpose: this
# is a docs-quality gate, not an unrelated dead-code/unused gate. Override with
# RUSTDOCFLAGS="" for a fully permissive local build.
export RUSTDOCFLAGS="${RUSTDOCFLAGS:--D rustdoc::broken_intra_doc_links -D rustdoc::bare_urls}"

# The workspace includes the Z3-backed crates (verify/tv/synth); default the
# header path so a plain local run works (same seam as run-all-tests-fast.sh).
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"

# Clean previous docs to avoid stale dependency references
rm -rf target/doc

cargo doc --no-deps --workspace

rm -rf apps/logicaffeine_docs/dist
mkdir -p apps/logicaffeine_docs/dist

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

# Copy only specified crates
for crate in "${CRATES[@]}"; do
    if [ -d "target/doc/$crate" ]; then
        cp -r "target/doc/$crate" apps/logicaffeine_docs/dist/
    else
        echo "ERROR: target/doc/$crate missing — did cargo doc skip it?" >&2
        exit 1
    fi
done

# Copy rustdoc assets
cp -r target/doc/static.files apps/logicaffeine_docs/dist/

# Copy search index (required for search functionality)
if [ -d "target/doc/search.index" ]; then
    cp -r target/doc/search.index apps/logicaffeine_docs/dist/
fi

# Copy src-files.js (required for source navigation)
if [ -f "target/doc/src-files.js" ]; then
    cp target/doc/src-files.js apps/logicaffeine_docs/dist/
fi

# Generate filtered crates.js with only our crates
CRATE_LIST=$(printf '"%s",' "${CRATES[@]}" | sed 's/,$//')
echo "window.ALL_CRATES = [$CRATE_LIST];" > apps/logicaffeine_docs/dist/crates.js

# Copy source files for our crates only
if [ -d "target/doc/src" ]; then
    mkdir -p apps/logicaffeine_docs/dist/src
    for crate in "${CRATES[@]}"; do
        if [ -d "target/doc/src/$crate" ]; then
            cp -r "target/doc/src/$crate" apps/logicaffeine_docs/dist/src/
        fi
    done
fi

# Landing page: one entry per crate, description pulled from its Cargo.toml at
# build time so the blurbs can never drift from the manifests.
crate_dir() {
    if [ -d "crates/$1" ]; then echo "crates/$1"; else echo "apps/$1"; fi
}

LIST_HTML=""
for crate in "${CRATES[@]}"; do
    manifest="$(crate_dir "$crate")/Cargo.toml"
    desc=$(grep -m1 '^description' "$manifest" | sed 's/^description *= *"\(.*\)"$/\1/')
    if [ -z "$desc" ]; then
        echo "ERROR: no description in $manifest — the landing page needs one" >&2
        exit 1
    fi
    desc=$(printf '%s' "$desc" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')
    LIST_HTML+="            <li><a href=\"$crate/index.html\">$crate</a> — $desc</li>"$'\n'
done

cat > apps/logicaffeine_docs/dist/index.html << EOF
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Logicaffeine Documentation</title>
    <style>
        body { font-family: system-ui, sans-serif; max-width: 800px; margin: 40px auto; padding: 0 20px; color: #222; }
        h1 { color: #333; }
        ul { list-style: none; padding: 0; }
        li { margin: 10px 0; line-height: 1.5; }
        a { color: #4a6ee0; text-decoration: none; }
        a:hover { text-decoration: underline; }
        .links { margin: 16px 0 28px; }
        .links a { margin-right: 16px; }
    </style>
</head>
<body>
    <h1>Logicaffeine Documentation</h1>
    <p>API documentation for the Logicaffeine workspace — the LOGOS English
    programming language, its logic pipeline, proof engines, execution tiers,
    and platform crates.</p>
    <div class="links">
        <a href="https://logicaffeine.com">logicaffeine.com</a>
        <a href="https://logicaffeine.com/studio">Studio</a>
        <a href="https://github.com/Brahmastra-Labs/logicaffeine">GitHub</a>
    </div>
    <h2>Crates</h2>
    <ul>
$LIST_HTML    </ul>
</body>
</html>
EOF

echo "Docs built to apps/logicaffeine_docs/dist/"
