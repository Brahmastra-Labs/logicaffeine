#!/usr/bin/env bash
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "Usage: $0 <old-version> <new-version>"
  echo "Example: $0 0.9.17 0.10.0"
  exit 1
fi

OLD="$1"
NEW="$2"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# The lockstep version lives in ONE file: the root Cargo.toml carries it in
# [workspace.package] (inherited by every member via `version.workspace = true`)
# and in the [workspace.dependencies] internal entries. The `.bak` dance keeps
# sed portable across GNU (Linux) and BSD (macOS).
sed -i.bak "s/version = \"$OLD\"/version = \"$NEW\"/g" "$REPO_ROOT/Cargo.toml"
rm -f "$REPO_ROOT/Cargo.toml.bak"
echo "  Updated Cargo.toml (workspace.package version + workspace.dependencies)"

PACKAGE_JSON="$REPO_ROOT/editors/vscode/logicaffeine/package.json"
if [ -f "$PACKAGE_JSON" ]; then
  sed -i.bak "s/\"version\": \"$OLD\"/\"version\": \"$NEW\"/" "$PACKAGE_JSON"
  rm -f "$PACKAGE_JSON.bak"
  echo "  Updated editors/vscode/logicaffeine/package.json"
fi

# nano is a standalone (git-ignored) workspace that pins the lockstep version by
# hand in its package version, its internal dep versions, and its README —
# rewrite all three when the checkout has it so it never drifts.
NANO_DIR="$REPO_ROOT/apps/logicaffeine_nano"
if [ -d "$NANO_DIR" ]; then
  sed -i.bak "s/version = \"$OLD\"/version = \"$NEW\"/g" "$NANO_DIR/Cargo.toml"
  rm -f "$NANO_DIR/Cargo.toml.bak"
  sed -i.bak "s/\`$OLD\`/\`$NEW\`/g" "$NANO_DIR/README.md"
  rm -f "$NANO_DIR/README.md.bak"
  echo "  Updated apps/logicaffeine_nano (Cargo.toml + README.md)"
fi

echo ""
echo "Running cargo check --workspace..."
cd "$REPO_ROOT"
cargo check --workspace

echo ""
echo "Version bump complete: $OLD → $NEW"
