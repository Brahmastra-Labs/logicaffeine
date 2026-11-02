#!/usr/bin/env bash
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "Usage: $0 <old-version> <new-version>"
  echo "Example: $0 0.9.17 0.10.0"
  exit 1
fi

OLD="$1"
NEW="$2"
# The versions as sed patterns: an unescaped `.` matches any character.
OLD_RE="${OLD//./\\.}"
NEW_RE="${NEW//./\\.}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Preconditions before ANY mutation, so a failed bump never leaves the tree
# half-bumped: the CHANGELOG must already carry the new version's released
# section (runbook step 2 precedes the bump) — it is the single source of
# truth for the release date.
REL_DATE="$(sed -n "s/^## \[$NEW_RE\] - //p" "$REPO_ROOT/CHANGELOG.md" | head -1)"
if [ -z "$REL_DATE" ]; then
  echo "ERROR: CHANGELOG.md has no released section '## [$NEW] - <date>' — roll [Unreleased] first" >&2
  exit 1
fi

# A workspace carries version stamps of its own and moves them in its post-bump hook, run last. A hook that is present
# but not executable would skip them without a word, so it is refused here, before anything is written.
HOOK="$REPO_ROOT/work/release-guards/bump/post-bump.sh"
if [ -e "$HOOK" ] && [ ! -x "$HOOK" ]; then
  echo "ERROR: ${HOOK#"$REPO_ROOT/"} is present but not executable — chmod +x it" >&2
  exit 1
fi

# The lockstep version lives in ONE file: the root Cargo.toml carries it in
# [workspace.package] (inherited by every member via `version.workspace = true`)
# and in the [workspace.dependencies] internal entries. Both rewrites are
# anchored to those lines — an external dependency pinned at the same version
# keeps its pin. The `.bak` dance keeps sed portable across GNU (Linux) and BSD
# (macOS).
INTERNAL_OLD="$(grep -c "^logicaffeine-[a-z-]* = { version = \"$OLD_RE\", path = " "$REPO_ROOT/Cargo.toml" || true)"
sed -i.bak -e "s/^version = \"$OLD_RE\"$/version = \"$NEW\"/" \
  -e "s/^\(logicaffeine-[a-z-]* = { version = \"\)$OLD_RE\(\", path = \)/\1$NEW\2/" "$REPO_ROOT/Cargo.toml"
rm -f "$REPO_ROOT/Cargo.toml.bak"
INTERNAL_NEW="$(grep -c "^logicaffeine-[a-z-]* = { version = \"$NEW_RE\", path = " "$REPO_ROOT/Cargo.toml" || true)"
grep -q "^version = \"$NEW_RE\"$" "$REPO_ROOT/Cargo.toml" || { echo "ERROR: Cargo.toml [workspace.package] version missed" >&2; exit 1; }
if [ "$INTERNAL_OLD" -eq 0 ] || [ "$INTERNAL_NEW" -ne "$INTERNAL_OLD" ]; then
  echo "ERROR: Cargo.toml internal dependency versions: $INTERNAL_OLD at $OLD before, $INTERNAL_NEW at $NEW after" >&2
  exit 1
fi
echo "  Updated Cargo.toml (workspace.package version + $INTERNAL_NEW internal workspace.dependencies)"

PACKAGE_JSON="$REPO_ROOT/editors/vscode/logicaffeine/package.json"
if [ -f "$PACKAGE_JSON" ]; then
  sed -i.bak "s/\"version\": \"$OLD_RE\"/\"version\": \"$NEW\"/" "$PACKAGE_JSON"
  rm -f "$PACKAGE_JSON.bak"
  echo "  Updated editors/vscode/logicaffeine/package.json"
fi

# The remaining version surfaces version_lock.rs pins to the workspace:
# README badge, roadmap hero span, installer example, extension lockfile.
sed -i.bak "s|badge/version-$OLD_RE-blue|badge/version-$NEW-blue|" "$REPO_ROOT/README.md"
rm -f "$REPO_ROOT/README.md.bak"
grep -q "badge/version-$NEW-blue" "$REPO_ROOT/README.md" || { echo "ERROR: README.md version badge missed" >&2; exit 1; }
echo "  Updated README.md (version badge)"

ROADMAP="$REPO_ROOT/apps/logicaffeine_web/src/ui/pages/roadmap.rs"
sed -i.bak "s/span { class: \"version\", \"v$OLD_RE\" }/span { class: \"version\", \"v$NEW\" }/" "$ROADMAP"
rm -f "$ROADMAP.bak"
grep -q "span { class: \"version\", \"v$NEW\" }" "$ROADMAP" || { echo "ERROR: roadmap.rs hero version span missed" >&2; exit 1; }
echo "  Updated apps/logicaffeine_web/src/ui/pages/roadmap.rs (hero version span)"

INSTALL_PS1="$REPO_ROOT/apps/logicaffeine_web/public/install.ps1"
sed -i.bak "s/-Version v$OLD_RE/-Version v$NEW/" "$INSTALL_PS1"
rm -f "$INSTALL_PS1.bak"
grep -q "\-Version v$NEW" "$INSTALL_PS1" || { echo "ERROR: install.ps1 example pin missed" >&2; exit 1; }
echo "  Updated apps/logicaffeine_web/public/install.ps1 (example pin)"

PACKAGE_LOCK="$REPO_ROOT/editors/vscode/logicaffeine/package-lock.json"
if [ -f "$PACKAGE_LOCK" ]; then
  # Only the extension's own two entries (root + packages[""]) carry its
  # version; anchoring on the adjacent "name" field keeps dependency versions
  # that happen to collide with $OLD untouched.
  OLD="$OLD" NEW="$NEW" perl -0pi -e 's/("name": "logicaffeine",\n\s*"version": ")\Q$ENV{OLD}\E(")/$1$ENV{NEW}$2/g' "$PACKAGE_LOCK"
  COUNT="$(grep -c "\"version\": \"$NEW\"" "$PACKAGE_LOCK" || true)"
  if [ "$COUNT" -lt 2 ]; then
    echo "ERROR: package-lock.json extension version entries missed (found $COUNT of 2)" >&2
    exit 1
  fi
  echo "  Updated editors/vscode/logicaffeine/package-lock.json (extension version ×2)"
fi

# The workspace's hook moves its own stamps once the public surfaces have moved; a hook that fails fails the bump.
# A public checkout has none, and says so.
if [ -x "$HOOK" ]; then
  "$HOOK" "$OLD" "$NEW"
  echo "  Ran the workspace's post-bump hook"
else
  echo "  No post-bump hook: the workspace's own version stamps were not moved" >&2
fi

echo ""
echo "Running cargo check --workspace..."
cd "$REPO_ROOT"
cargo check --workspace

echo ""
echo "Version bump complete: $OLD → $NEW"
echo "Next: ./scripts/generate-roadmap.sh (version_lock.rs pins roadmap_history.json to $NEW)"
