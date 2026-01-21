#!/bin/bash
set -e
cd "$(dirname "$0")/../.."

# Clean previous docs to avoid stale dependency references
rm -rf target/doc

cargo doc --no-deps --workspace

rm -rf apps/logicaffeine_docs/dist
mkdir -p apps/logicaffeine_docs/dist

# Crates to include (exclude logicaffeine_tests)
CRATES=(
    logicaffeine_base
    logicaffeine_cli
    logicaffeine_compile
    logicaffeine_data
    logicaffeine_kernel
    logicaffeine_language
    logicaffeine_lexicon
    logicaffeine_proof
    logicaffeine_system
    logicaffeine_web
)

# Copy only specified crates
for crate in "${CRATES[@]}"; do
    if [ -d "target/doc/$crate" ]; then
        cp -r "target/doc/$crate" apps/logicaffeine_docs/dist/
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

# Create index with crate listing
cat > apps/logicaffeine_docs/dist/index.html << 'EOF'
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta http-equiv="refresh" content="0; url=logicaffeine_language/index.html">
    <title>LOGICAFFEINE Docs</title>
    <style>
        body { font-family: system-ui, sans-serif; max-width: 800px; margin: 40px auto; padding: 0 20px; }
        h1 { color: #333; }
        ul { list-style: none; padding: 0; }
        li { margin: 8px 0; }
        a { color: #4a6ee0; text-decoration: none; }
        a:hover { text-decoration: underline; }
    </style>
</head>
<body>
    <h1>LOGICAFFEINE Documentation</h1>
    <p>Redirecting to <a href="logicaffeine_language/index.html">logicaffeine_language</a>...</p>
    <noscript>
        <h2>All Crates</h2>
        <ul>
            <li><a href="logicaffeine_language/index.html">logicaffeine_language</a> - Core language</li>
            <li><a href="logicaffeine_compile/index.html">logicaffeine_compile</a> - Compiler</li>
            <li><a href="logicaffeine_kernel/index.html">logicaffeine_kernel</a> - Runtime kernel</li>
            <li><a href="logicaffeine_proof/index.html">logicaffeine_proof</a> - Proof assistant</li>
            <li><a href="logicaffeine_lexicon/index.html">logicaffeine_lexicon</a> - Lexicon</li>
            <li><a href="logicaffeine_base/index.html">logicaffeine_base</a> - Base types</li>
            <li><a href="logicaffeine_data/index.html">logicaffeine_data</a> - Data structures</li>
            <li><a href="logicaffeine_system/index.html">logicaffeine_system</a> - System layer</li>
            <li><a href="logicaffeine_cli/index.html">logicaffeine_cli</a> - CLI (largo)</li>
            <li><a href="logicaffeine_web/index.html">logicaffeine_web</a> - Web frontend</li>
        </ul>
    </noscript>
</body>
</html>
EOF

echo "Docs built to apps/logicaffeine_docs/dist/"
