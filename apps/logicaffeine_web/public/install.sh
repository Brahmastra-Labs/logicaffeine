#!/bin/sh
# largo installer — the LOGOS build tool.
#
#   curl -fsSL https://logicaffeine.com/install.sh | sh
#   curl -fsSL https://logicaffeine.com/install.sh | sh -s -- --full
#
# Flags:
#   --full             install the full build (Z3 verification statically linked)
#   --version vX.Y.Z   install an exact release instead of the latest
#   --to DIR           install directory (default: ~/.local/bin)
#   --help             this text
#
# Environment:
#   LARGO_INSTALL_DIR  same as --to
#   LARGO_BASE_URL     release host override (default: the GitHub repo)
#
# The script downloads the prebuilt binary for your platform, verifies its
# SHA-256 against the release's SHA256SUMS, and installs it atomically as
# `largo`. It never uses sudo and never edits your shell configuration.

set -eu

BASE_URL="${LARGO_BASE_URL:-https://github.com/Brahmastra-Labs/logicaffeine}"
INSTALL_DIR="${LARGO_INSTALL_DIR:-$HOME/.local/bin}"
FLAVOR=""
VERSION=""

say()  { printf '%s\n' "$*" >&2; }
die()  { say "error: $*"; exit 1; }

usage() {
  sed -n '2,20p' "$0" 2>/dev/null || true
  say "usage: install.sh [--full] [--version vX.Y.Z] [--to DIR]"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --full) FLAVOR="-full" ;;
    --version) shift; [ $# -gt 0 ] || die "--version needs an argument"; VERSION="$1" ;;
    --to) shift; [ $# -gt 0 ] || die "--to needs an argument"; INSTALL_DIR="$1" ;;
    --help|-h) usage; exit 0 ;;
    *) die "unknown flag: $1 (see --help)" ;;
  esac
  shift
done

# Normalize a bare X.Y.Z to vX.Y.Z.
if [ -n "$VERSION" ]; then
  case "$VERSION" in v*) ;; *) VERSION="v$VERSION" ;; esac
fi

# --- downloader ------------------------------------------------------------
if command -v curl >/dev/null 2>&1; then
  fetch()     { curl -fsSL -o "$2" "$1"; }
  final_url() { curl -fsSLI -o /dev/null -w '%{url_effective}' "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch()     { wget -q -O "$2" "$1"; }
  final_url() { wget -q --max-redirect=10 --server-response --spider "$1" 2>&1 \
                 | awk '/^  Location: /{u=$2} END{print u}'; }
else
  die "neither curl nor wget is available — install one and re-run"
fi

# --- platform --------------------------------------------------------------
case "$(uname -s)" in
  Linux)  OS=linux ;;
  Darwin) OS=darwin ;;
  MINGW*|MSYS*|CYGWIN*)
    die "this is the Unix installer — on Windows run:
  powershell -ExecutionPolicy Bypass -c \"irm https://logicaffeine.com/install.ps1 | iex\"
(or use WSL)" ;;
  *) die "unsupported platform $(uname -s) — try: cargo install logicaffeine-cli" ;;
esac
case "$(uname -m)" in
  x86_64|amd64)  ARCH=x64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *) die "unsupported architecture $(uname -m) (prebuilt: linux/macos x64+arm64, windows x64) — try: cargo install logicaffeine-cli" ;;
esac

ASSET="largo${FLAVOR}-${OS}-${ARCH}.tar.gz"

# --- resolve the release tag (no GitHub API — rate-limit-proof) -------------
if [ -z "$VERSION" ]; then
  effective="$(final_url "$BASE_URL/releases/latest")" \
    || die "cannot reach $BASE_URL to resolve the latest release"
  VERSION="$(basename "$effective")"
  case "$VERSION" in
    v*) ;;
    *) die "could not resolve the latest release tag (got '$VERSION')" ;;
  esac
fi

DOWNLOAD="$BASE_URL/releases/download/$VERSION"

# --- download + verify -----------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

say "downloading largo $VERSION (${OS}-${ARCH}${FLAVOR:+, full}) ..."
fetch "$DOWNLOAD/$ASSET" "$TMP/$ASSET" || die "no prebuilt binary '$ASSET' for $VERSION
  release page: $BASE_URL/releases
  fallback:     cargo install logicaffeine-cli"

fetch "$DOWNLOAD/SHA256SUMS" "$TMP/SHA256SUMS" \
  || die "SHA256SUMS missing for $VERSION — refusing to install unverified binaries"

expected="$(grep " $ASSET\$" "$TMP/SHA256SUMS" | awk '{print $1}')"
[ -n "$expected" ] || die "no checksum entry for $ASSET — refusing to install"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$TMP/$ASSET" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "$TMP/$ASSET" | awk '{print $1}')"
fi
if [ "$expected" != "$actual" ]; then
  die "checksum mismatch for $ASSET
  expected: $expected
  actual:   $actual
The download is corrupt or tampered with — nothing was installed.
Please retry, and report persistent mismatches at $BASE_URL/issues"
fi

# --- extract + install (atomic) ---------------------------------------------
tar -xzf "$TMP/$ASSET" -C "$TMP"
[ -f "$TMP/largo" ] || die "archive did not contain a largo binary"
chmod +x "$TMP/largo"

mkdir -p "$INSTALL_DIR"
mv -f "$TMP/largo" "$INSTALL_DIR/largo.tmp.$$"
mv -f "$INSTALL_DIR/largo.tmp.$$" "$INSTALL_DIR/largo"

# --- proof of life -----------------------------------------------------------
installed_version="$("$INSTALL_DIR/largo" --version 2>/dev/null)" \
  || die "installed binary failed to run — please report this at $BASE_URL/issues"
say "installed $installed_version -> $INSTALL_DIR/largo"

# --- PATH advice (never edits rc files) --------------------------------------
case ":$PATH:" in
  *:"$INSTALL_DIR":*) ;;
  *)
    say ""
    say "note: $INSTALL_DIR is not on your PATH."
    case "${SHELL:-}" in
      */zsh)  say "  add it:  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc" ;;
      */fish) say "  add it:  fish_add_path $INSTALL_DIR" ;;
      *)      say "  add it:  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc" ;;
    esac
    say "  this session:  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

if [ -z "$FLAVOR" ]; then
  say ""
  say "tip: the full build bundles Z3 static verification (largo verify):"
  say "  curl -fsSL https://logicaffeine.com/install.sh | sh -s -- --full"
fi
say ""
say "get started:  largo new hello && cd hello && largo run"
say "uninstall:    rm $INSTALL_DIR/largo"
