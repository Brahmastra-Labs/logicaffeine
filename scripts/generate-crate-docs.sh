#!/bin/bash
# Generate rustdoc for logicaffeine crates and copy to web assets
#
# This script builds documentation for all workspace crates and copies
# only the logicaffeine_* crates to the web app's assets directory.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_DIR="$ROOT_DIR/apps/logicaffeine_web/public/crates"

echo "Building crate documentation..."
cd "$ROOT_DIR"

# Generate documentation for all workspace crates
# --no-deps: Don't document dependencies
# --workspace: Document all workspace members
cargo doc --no-deps --workspace

# Source directory for generated docs
DOC_SRC="$ROOT_DIR/target/doc"

# Clean and recreate target directory
rm -rf "$TARGET_DIR"
mkdir -p "$TARGET_DIR"

echo "Copying logicaffeine crates..."

# Copy only logicaffeine_* crate docs
for crate_dir in "$DOC_SRC"/logicaffeine_*; do
    if [ -d "$crate_dir" ]; then
        crate_name=$(basename "$crate_dir")
        echo "  - $crate_name"
        cp -r "$crate_dir" "$TARGET_DIR/"
    fi
done

# Copy shared static files required by rustdoc
echo "Copying static files..."
if [ -d "$DOC_SRC/static.files" ]; then
    cp -r "$DOC_SRC/static.files" "$TARGET_DIR/"
fi

# Copy search index and crates.js for search functionality
if [ -f "$DOC_SRC/search-index.js" ]; then
    cp "$DOC_SRC/search-index.js" "$TARGET_DIR/"
fi
if [ -f "$DOC_SRC/crates.js" ]; then
    cp "$DOC_SRC/crates.js" "$TARGET_DIR/"
fi
if [ -f "$DOC_SRC/help.html" ]; then
    cp "$DOC_SRC/help.html" "$TARGET_DIR/"
fi
if [ -f "$DOC_SRC/settings.html" ]; then
    cp "$DOC_SRC/settings.html" "$TARGET_DIR/"
fi
if [ -f "$DOC_SRC/src-files.js" ]; then
    cp "$DOC_SRC/src-files.js" "$TARGET_DIR/"
fi

# Copy source files for each crate
if [ -d "$DOC_SRC/src" ]; then
    mkdir -p "$TARGET_DIR/src"
    for src_crate in "$DOC_SRC/src"/logicaffeine_*; do
        if [ -d "$src_crate" ]; then
            cp -r "$src_crate" "$TARGET_DIR/src/"
        fi
    done
fi

# Inline CSS into HTML files for self-contained documentation
echo "Inlining CSS into HTML files..."
"$SCRIPT_DIR/inline-docs.sh" "$TARGET_DIR"

# Create index.html that redirects to the main crate
cat > "$TARGET_DIR/index.html" << 'EOF'
<!DOCTYPE html>
<html>
<head>
    <meta http-equiv="refresh" content="0; url=logicaffeine_language/index.html">
    <title>Redirecting to Crate Documentation</title>
</head>
<body>
    <p>Redirecting to <a href="logicaffeine_language/index.html">logicaffeine_language</a>...</p>
</body>
</html>
EOF

# Also copy to Dioxus build output for dev server
# App name is "logos" in Dioxus.toml
DEV_OUTPUT="$ROOT_DIR/target/dx/logos/debug/web/public/crates"
RELEASE_OUTPUT="$ROOT_DIR/target/dx/logicaffeine-web/release/web/public/crates"

# Copy to dev output if it exists
if [ -d "$(dirname "$DEV_OUTPUT")" ]; then
    echo "Copying to dev build output..."
    rm -rf "$DEV_OUTPUT"
    cp -r "$TARGET_DIR" "$DEV_OUTPUT"
fi

# Copy to release output if it exists
if [ -d "$(dirname "$RELEASE_OUTPUT")" ]; then
    echo "Copying to release build output..."
    rm -rf "$RELEASE_OUTPUT"
    cp -r "$TARGET_DIR" "$RELEASE_OUTPUT"
fi

echo ""
echo "Documentation generated successfully!"
echo "Output: $TARGET_DIR"
echo ""
echo "Crates documented:"
ls -1 "$TARGET_DIR" | grep "^logicaffeine_" | sed 's/^/  - /'
