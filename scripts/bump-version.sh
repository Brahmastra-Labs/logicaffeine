#!/usr/bin/env bash
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "Usage: $0 <old-version> <new-version>"
  echo "Example: $0 0.8.1 0.8.2"
  exit 1
fi

OLD="$1"
NEW="$2"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

CARGO_FILES=(
  "crates/logicaffeine_base/Cargo.toml"
  "crates/logicaffeine_data/Cargo.toml"
  "crates/logicaffeine_kernel/Cargo.toml"
  "crates/logicaffeine_lexicon/Cargo.toml"
  "crates/logicaffeine_system/Cargo.toml"
  "crates/logicaffeine_proof/Cargo.toml"
  "crates/logicaffeine_language/Cargo.toml"
  "crates/logicaffeine_compile/Cargo.toml"
  "crates/logicaffeine_lsp/Cargo.toml"
  "crates/logicaffeine_tests/Cargo.toml"
  "apps/logicaffeine_cli/Cargo.toml"
  "apps/logicaffeine_web/Cargo.toml"
  "crates/logicaffeine_verify/Cargo.toml"
)

echo "Bumping version: $OLD → $NEW"

for file in "${CARGO_FILES[@]}"; do
  filepath="$REPO_ROOT/$file"
  if [ -f "$filepath" ]; then
    sed -i '' "s/version = \"$OLD\"/version = \"$NEW\"/g" "$filepath"
    echo "  Updated $file"
  else
    echo "  SKIP (not found): $file"
  fi
done

PACKAGE_JSON="$REPO_ROOT/editors/vscode/logicaffeine/package.json"
if [ -f "$PACKAGE_JSON" ]; then
  sed -i '' "s/\"version\": \"$OLD\"/\"version\": \"$NEW\"/" "$PACKAGE_JSON"
  echo "  Updated editors/vscode/logicaffeine/package.json"
fi

echo ""
echo "Running cargo check --workspace..."
cd "$REPO_ROOT"
cargo check --workspace

echo ""
echo "Version bump complete: $OLD → $NEW"
