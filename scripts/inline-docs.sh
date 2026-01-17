#!/bin/bash
# Inline CSS into rustdoc HTML files to create self-contained documentation
#
# This post-processor transforms rustdoc output so each HTML file is standalone:
# - Inlines all CSS directly into each HTML file
# - Replaces web fonts with system fonts
# - Removes font preloading and search JS (not critical for viewing)
#
# Usage: ./inline-docs.sh <docs-directory>

set -e

DOCS_DIR="$1"

if [ -z "$DOCS_DIR" ] || [ ! -d "$DOCS_DIR" ]; then
    echo "Usage: $0 <docs-directory>"
    echo "Error: Please provide a valid documentation directory"
    exit 1
fi

echo "Inlining CSS into HTML files..."

# Find the CSS files
STATIC_DIR="$DOCS_DIR/static.files"

if [ ! -d "$STATIC_DIR" ]; then
    echo "Warning: No static.files directory found in $DOCS_DIR"
    echo "Skipping CSS inlining"
    exit 0
fi

# Find normalize and rustdoc CSS files
NORMALIZE_CSS=$(find "$STATIC_DIR" -name "normalize-*.css" 2>/dev/null | head -1)
RUSTDOC_CSS=$(find "$STATIC_DIR" -name "rustdoc-*.css" 2>/dev/null | head -1)

if [ -z "$NORMALIZE_CSS" ] || [ -z "$RUSTDOC_CSS" ]; then
    echo "Warning: Could not find CSS files"
    echo "  normalize: $NORMALIZE_CSS"
    echo "  rustdoc: $RUSTDOC_CSS"
    exit 0
fi

echo "  Found CSS files:"
echo "    - $(basename "$NORMALIZE_CSS")"
echo "    - $(basename "$RUSTDOC_CSS")"

# Create combined CSS content with system font overrides
CSS_TEMP=$(mktemp)
cat "$NORMALIZE_CSS" "$RUSTDOC_CSS" > "$CSS_TEMP"

# Replace font declarations with system fonts in CSS
sed -i '' \
    -e 's/font-family:[^;}]*"Fira Sans"[^;}]*/font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif/g' \
    -e 's/font-family:[^;}]*"Source Serif 4"[^;}]*/font-family: Georgia, "Times New Roman", serif/g' \
    -e 's/font-family:[^;}]*"Source Code Pro"[^;}]*/font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas, monospace/g' \
    -e 's/font-family:[^;}]*"Fira Mono"[^;}]*/font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas, monospace/g' \
    "$CSS_TEMP"

# Escape CSS for sed replacement (handle special chars)
CSS_ESCAPED=$(cat "$CSS_TEMP" | sed -e 's/[&/\]/\\&/g' | tr '\n' ' ')

# Count HTML files
HTML_COUNT=$(find "$DOCS_DIR" -name "*.html" | wc -l | tr -d ' ')
echo "  Processing $HTML_COUNT HTML files..."

# Process each HTML file using perl (more reliable for complex replacements)
PROCESSED=0
find "$DOCS_DIR" -name "*.html" | while read -r html_file; do
    # Use perl for robust replacement
    perl -i -pe '
        # Remove font preloading script
        s/<script>if\(window\.location\.protocol!=="file:"\)document\.head\.insertAdjacentHTML[^<]*<\/script>//g;

        # Remove normalize stylesheet link
        s/<link[^>]*normalize[^>]*\.css[^>]*>//g;

        # Remove noscript stylesheet link (references static.files which we remove)
        s/<link[^>]*noscript[^>]*\.css[^>]*>//g;

        # Remove font preload links
        s/<link[^>]*rel="preload"[^>]*font[^>]*>//g;
    ' "$html_file"

    PROCESSED=$((PROCESSED + 1))
    if [ $((PROCESSED % 100)) -eq 0 ]; then
        echo "    Processed $PROCESSED files..."
    fi
done

# Now inject CSS into each file (replace rustdoc stylesheet link)
echo "  Injecting inline CSS..."
export CSS_TEMP
PROCESSED=0
find "$DOCS_DIR" -name "*.html" | while read -r html_file; do
    # Create a temp file with the CSS injection
    TEMP_HTML=$(mktemp)

    # Use perl to replace the rustdoc stylesheet link with inline style
    perl -pe '
        BEGIN {
            local $/;
            open(CSS, "<", $ENV{"CSS_TEMP"}) or die "Cannot open CSS: $!";
            $css = <CSS>;
            close(CSS);
            $css =~ s/\n/ /g;  # Remove newlines
        }
        s/<link[^>]*rustdoc[^>]*\.css[^>]*>/<style>$css<\/style>/g;
    ' "$html_file" > "$TEMP_HTML"

    mv "$TEMP_HTML" "$html_file"

    PROCESSED=$((PROCESSED + 1))
    if [ $((PROCESSED % 100)) -eq 0 ]; then
        echo "    Injected $PROCESSED files..."
    fi
done

# Clean up temp file
rm -f "$CSS_TEMP"

# Remove static.files directory since CSS is now inlined
echo "  Removing static.files directory (no longer needed)..."
rm -rf "$STATIC_DIR"

# Remove search-related JS files (optional, not needed for basic viewing)
echo "  Removing search JS files..."
rm -f "$DOCS_DIR/search-index.js"
rm -f "$DOCS_DIR/crates.js"
rm -f "$DOCS_DIR/src-files.js"

echo "  Done! All HTML files are now self-contained."
