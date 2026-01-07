#!/bin/bash

# LOGOS FRONTEND - UI Documentation Generator
# Generates documentation for the Dioxus-based web frontend.

OUTPUT_FILE="LOGOS_FRONTEND_DOCS.md"
echo "Generating FRONTEND documentation..."

# ==============================================================================
# HEADER & TOC
# ==============================================================================
cat > "$OUTPUT_FILE" << 'EOF'
# LOGOS - Frontend Documentation

## Overview
This document covers the **Frontend Layer** of the LOGOS system: the Dioxus web application, UI components, and gamification engine.

## Table of Contents
1. [Architecture](#architecture)
2. [Web Application](#web-application)
3. [Problem Generator](#problem-generator)
4. [Gamification](#gamification)
5. [Relevant Tests](#relevant-tests)

## Architecture

**Stack:**
*   **Framework:** Dioxus 0.6 (React-like Rust framework)
*   **Routing:** Client-side routing (`src/ui/router.rs`)
*   **State:** Signal-based reactivity
*   **Platform:** WASM (Browser) / Desktop (WebView)

EOF

# ==============================================================================
# HELPERS
# ==============================================================================
add_file() {
    local file_path="$1"
    local title="$2"
    local description="$3"
    if [ -f "$file_path" ]; then
        echo "Adding: $file_path"
        {
            echo "### $title"
            echo ""
            echo "**File:** \`$file_path\`"
            echo ""
            echo "$description"
            echo ""
            echo "\
```rust"
            cat "$file_path"
            echo "```"
            echo ""
            echo "---"
            echo ""
        } >> "$OUTPUT_FILE"
    fi
}

add_test_description() {
    local file_path="$1"
    local title="$2"
    local description="$3"
    if [ -f "$file_path" ]; then
        echo "Adding test: $file_path"
        {
            echo "#### $title"
            echo ""
            echo "**File:** \`$file_path\`"
            echo ""
            echo "$description"
            echo ""
            echo "---"
            echo ""
        } >> "$OUTPUT_FILE"
    fi
}

# ==============================================================================
# CONTENT
# ==============================================================================

# UI Core
add_file "src/main.rs" "Entry Point" "App launch configuration."
if [ -d "src/ui" ]; then
    add_file "src/ui/app.rs" "App Component" "Root component and layout."
    add_file "src/ui/router.rs" "Router" "Route definitions."
    add_file "src/ui/state.rs" "Global State" "Signal management."
fi

# Pages
if [ -d "src/ui/pages" ]; then
    add_file "src/ui/pages/home.rs" "Home Page" "Landing page."
    add_file "src/ui/pages/workspace.rs" "Workspace" "Main IDE interface."
    add_file "src/ui/pages/lesson.rs" "Lesson" "Interactive problem solving."
fi

# Problem Generator
add_file "src/content.rs" "Content Engine" "Curriculum loading."
add_file "src/generator.rs" "Generator" "Problem template instantiation."
add_file "src/grader.rs" "Grader" "Semantic answer checking."

# Gamification
add_file "src/game.rs" "Game State" "XP, Level, and Streak tracking."
add_file "src/achievements.rs" "Achievements" "Achievement system logic."
add_file "src/srs.rs" "SRS" "Spaced Repetition System."
add_file "src/audio.rs" "Audio" "Sound effects."

# Tests
cat >> "$OUTPUT_FILE" << 'EOF'
## Relevant Tests
EOF

add_test_description "tests/learn_state_tests.rs" "Learning State" "Progress tracking tests."
add_test_description "tests/unlock_logic_tests.rs" "Unlock Logic" "Lesson unlocking tests."
add_test_description "tests/e2e_collections.rs" "E2E Collections" "Runtime verification (relevant for UI feedback)."

echo "Done! View with: cat $OUTPUT_FILE"
chmod +x "$OUTPUT_FILE"