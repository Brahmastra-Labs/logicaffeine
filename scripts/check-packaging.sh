#!/usr/bin/env bash
#
# check-packaging.sh — build every published crate from the tarball crates.io would receive.
#
# `crates/logicaffeine_tests/tests/packaging_lock.rs` (pure parsing, runs every baseline) proves each
# published crate declares an `include` allowlist and that its production code embeds only files the
# allowlist ships. This script is the ground truth behind it:
#
#   1. `cargo package` assembles every published crate exactly as `cargo publish` uploads it, unpacks
#      each tarball and builds it against a local overlay of its sibling tarballs: no workspace root,
#      no sibling directories, nothing the registry will not have.
#   2. Every docs.rs feature set (`[package.metadata.docs.rs] features`) is built from the unpacked
#      tarballs, because that is exactly what docs.rs builds.
#   3. Every `.crate` stays within its budget in `.config/size-budgets.toml` `[crates]` and under the
#      8 MiB hard ceiling (crates.io refuses uploads over 10 MiB).
#   4. No tarball ships a test: no `#[test]` anywhere in an unpacked crate (tests live in
#      crates/logicaffeine_tests; `unit_home_lock` keeps them out of every `src/`).
#
# The published set is derived the way publish.yml derives it: every workspace member whose manifest
# has no `publish` key. The work happens OUTSIDE the repo (an unpacked tarball under the repo would be
# mistaken for a member of the root workspace).
#
# Usage: ./scripts/check-packaging.sh [--allow-dirty]
#   --allow-dirty   package a working tree with uncommitted changes (CI checks out clean and omits it)
# Env:   CHECK_PACKAGING_TARGET  scratch target dir (default: ${TMPDIR:-/tmp}/logicaffeine-check-packaging)
# Log:   logs/packaging/check-packaging-<timestamp>.log ; exit 0 iff every check passes.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

ALLOW_DIRTY=()
for arg in "$@"; do
  case "$arg" in
    --allow-dirty) ALLOW_DIRTY=(--allow-dirty) ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

TARGET="${CHECK_PACKAGING_TARGET:-${TMPDIR:-/tmp}/logicaffeine-check-packaging}"
case "$TARGET/" in
  "$REPO_ROOT"/*) echo "CHECK_PACKAGING_TARGET must be outside the repo ($TARGET)" >&2; exit 2 ;;
esac
mkdir -p "$TARGET" logs/packaging
LOG="logs/packaging/check-packaging-$(date -u +%Y%m%d-%H%M%S).log"
MAX_CRATE_BYTES=$((8 * 1024 * 1024))

# name<TAB>version<TAB>space-separated docs.rs features, one line per published crate.
PUBLISHED=$(cargo metadata --format-version 1 --no-deps | python3 -c '
import json, sys
meta = json.load(sys.stdin)
for p in sorted(meta["packages"], key=lambda p: p["name"]):
    if p["publish"] is not None:
        continue
    docs = ((p.get("metadata") or {}).get("docs") or {}).get("rs") or {}
    print(p["name"], p["version"], " ".join(docs.get("features") or []), sep="\t")
')
[ -n "$PUBLISHED" ] || { echo "no published crates found — cargo metadata failed?" >&2; exit 1; }

FAILURES=()
{
  echo "check-packaging  $(date -u +%FT%TZ)  target=$TARGET"
  echo "published crates:"
  echo "$PUBLISHED" | sed 's/^/  /'
  echo

  echo "── 1. cargo package (every tarball built against its siblings' tarballs) ──"
  PKG_ARGS=()
  while IFS=$'\t' read -r name _version _features; do PKG_ARGS+=(-p "$name"); done <<< "$PUBLISHED"
  if ! CARGO_TARGET_DIR="$TARGET" cargo package "${PKG_ARGS[@]}" "${ALLOW_DIRTY[@]}"; then
    FAILURES+=("cargo package: a published crate does not build from its own tarball")
  fi

  echo
  echo "── 2. docs.rs feature sets, built from the unpacked tarballs ──"
  PATCH_ARGS=()
  while IFS=$'\t' read -r name version _features; do
    PATCH_ARGS+=(--config "patch.crates-io.${name}.path='$TARGET/package/${name}-${version}'")
  done <<< "$PUBLISHED"
  while IFS=$'\t' read -r name version features; do
    [ -n "$features" ] || continue
    dir="$TARGET/package/${name}-${version}"
    echo "  ${name} --features ${features// /,}"
    if [ ! -f "$dir/Cargo.toml" ]; then
      FAILURES+=("docs.rs: ${name} has no unpacked tarball at ${dir}")
      continue
    fi
    if ! CARGO_TARGET_DIR="$TARGET/docsrs" cargo check --manifest-path "$dir/Cargo.toml" \
         --features "${features// /,}" "${PATCH_ARGS[@]}"; then
      FAILURES+=("docs.rs: ${name} does not build with its docs.rs features (${features})")
    fi
  done <<< "$PUBLISHED"

  echo
  echo "── 3. tarball sizes (budgets: .config/size-budgets.toml [crates]; hard ceiling $MAX_CRATE_BYTES bytes) ──"
  while IFS=$'\t' read -r name version _features; do
    crate="$TARGET/package/${name}-${version}.crate"
    if [ ! -f "$crate" ]; then
      FAILURES+=("size: ${name}-${version}.crate was not produced")
      continue
    fi
    bytes=$(stat -c %s "$crate" 2>/dev/null || stat -f %z "$crate")
    limit=$(awk -v k="$name" '/^\[/ { in_t = ($0 == "[crates]"); next } in_t { split($0, kv, "="); gsub(/[ \t"]/, "", kv[1]); if (kv[1] == k) { gsub(/[^0-9]/, "", kv[2]); print kv[2]; found = 1; exit } } END { if (!found) print 0 }' .config/size-budgets.toml)
    printf '  %-28s %10d bytes   budget %10d\n' "${name}" "$bytes" "$limit"
    if [ "$limit" -eq 0 ]; then
      FAILURES+=("size: ${name} has no budget in .config/size-budgets.toml [crates]")
    elif [ "$bytes" -gt "$limit" ]; then
      FAILURES+=("size: ${name}-${version}.crate is ${bytes} bytes, over its ${limit} budget")
    fi
    if [ "$bytes" -gt "$MAX_CRATE_BYTES" ]; then
      FAILURES+=("size: ${name}-${version}.crate is ${bytes} bytes, over the ${MAX_CRATE_BYTES} ceiling")
    fi
  done <<< "$PUBLISHED"

  echo
  echo "── 4. no tarball ships a test ──"
  while IFS=$'\t' read -r name version _features; do
    dir="$TARGET/package/${name}-${version}"
    [ -d "$dir" ] || continue
    hits=$(grep -rlE '^\s*#\[([a-z_]+::)?test\]' --include='*.rs' "$dir" 2>/dev/null | sed "s|$TARGET/package/||")
    if [ -n "$hits" ]; then
      echo "$hits" | sed 's/^/  /'
      FAILURES+=("tests: ${name} ships test code ($(echo "$hits" | wc -l) file(s))")
    fi
  done <<< "$PUBLISHED"

  echo
  if [ "${#FAILURES[@]}" -eq 0 ]; then
    echo "RESULT: ✅ every published crate builds from its tarball, docs.rs features build, sizes within budget, no tests shipped"
  else
    echo "RESULT: ❌ ${#FAILURES[@]} packaging failure(s):"
    printf '  - %s\n' "${FAILURES[@]}"
  fi
} 2>&1 | tee "$LOG"

grep -q '^RESULT: ✅' "$LOG"
