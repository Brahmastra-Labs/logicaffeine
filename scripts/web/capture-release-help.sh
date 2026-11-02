#!/usr/bin/env bash
# Record what the released largo offers, for the front page's agent instructions.
#
# The instructions on logicaffeine.com tell a person and their coding agent which largo commands to
# run, and the install script installs the latest GitHub release, not the tree. This script installs
# that release the way a reader does, with the public install script, into a throwaway directory,
# and writes `largo --version`, `largo --help` and every command's `--help` into the fixture that
# `crates/logicaffeine_tests/tests/landing_page.rs` holds the instructions to. Rerun it after a
# release, then read the suite's verdict: a command the instructions name that the new release
# dropped, or a newer command the guide still calls unreleased, turns it red.
#
# Usage: ./scripts/web/capture-release-help.sh [--version vX.Y.Z]

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/crates/logicaffeine_tests/tests/fixtures/largo_release_help"
INSTALLER="${LARGO_INSTALLER_URL:-https://logicaffeine.com/install.sh}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

curl -fsSL "$INSTALLER" -o "$WORK/install.sh"
sh "$WORK/install.sh" --to "$WORK/bin" "$@"
LARGO="$WORK/bin/largo"

STAGE="$WORK/help"
mkdir -p "$STAGE"
"$LARGO" --version > "$STAGE/VERSION"
"$LARGO" --help > "$STAGE/largo.txt"
commands="$(awk '/^Commands:/ { listing = 1; next } /^Options:/ { listing = 0 } listing && NF { print $1 }' "$STAGE/largo.txt")"
[ -n "$commands" ] || { echo "capture-release-help: largo --help listed no commands" >&2; exit 1; }
for command in $commands; do
    [ "$command" = help ] && continue
    "$LARGO" "$command" --help > "$STAGE/$command.txt"
done

rm -rf "$OUT"
mkdir -p "$OUT"
cp "$STAGE"/* "$OUT/"
echo "capture-release-help: $(cat "$OUT/VERSION") → $OUT ($(ls "$OUT" | wc -l | tr -d ' ') files)"
