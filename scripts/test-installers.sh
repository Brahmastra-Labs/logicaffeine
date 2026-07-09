#!/usr/bin/env bash
# Offline TDD harness for the largo installers (apps/logicaffeine_web/public/install.sh).
#
# Builds a fake GitHub-release layout (dummy `largo` binaries, tarballs,
# SHA256SUMS) in a temp dir, serves it over a local HTTP server that mimics
# the `releases/latest` 302 redirect, and drives install.sh through every
# behavioral case — no network, no real binaries.
#
# Usage: scripts/test-installers.sh
# Exits non-zero on the first failing case.

set -u

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALLER="$REPO_ROOT/apps/logicaffeine_web/public/install.sh"

PASS=0
FAIL=0

note()  { printf '  %s\n' "$*"; }
ok()    { PASS=$((PASS+1)); printf 'ok   %s\n' "$*"; }
fail()  { FAIL=$((FAIL+1)); printf 'FAIL %s\n' "$*"; }

[ -f "$INSTALLER" ] || { echo "FAIL install.sh does not exist at $INSTALLER"; exit 1; }

# ---------------------------------------------------------------------------
# Platform mapping (must mirror install.sh's own mapping)
# ---------------------------------------------------------------------------
case "$(uname -s)" in
  Linux)  OS=linux ;;
  Darwin) OS=darwin ;;
  *) echo "harness: unsupported host $(uname -s)"; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64)  ARCH=x64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *) echo "harness: unsupported arch $(uname -m)"; exit 1 ;;
esac

# ---------------------------------------------------------------------------
# Fake release area
# ---------------------------------------------------------------------------
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"; [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null' EXIT

SITE="$WORK/site"
LATEST_TAG="v0.10.0"
OLD_TAG="v0.9.9"
BAD_TAG="v0.6.6"

make_asset() {
  # make_asset <tag> <version-string> <flavor-suffix>
  tag="$1"; version="$2"; suffix="$3"
  dir="$SITE/releases/download/$tag"
  mkdir -p "$dir"
  stage="$WORK/stage-$tag$suffix"
  mkdir -p "$stage"
  cat > "$stage/largo" <<EOF
#!/bin/sh
echo "largo $version"
EOF
  chmod +x "$stage/largo"
  echo "license" > "$stage/LICENSE.md"
  echo "readme"  > "$stage/README.txt"
  tar -czf "$dir/largo$suffix-$OS-$ARCH.tar.gz" -C "$stage" .
}

sums_for() {
  ( cd "$SITE/releases/download/$1" && { command -v sha256sum >/dev/null && sha256sum -- * || shasum -a 256 -- *; } > SHA256SUMS )
}

make_asset "$LATEST_TAG" "0.10.0"        ""
make_asset "$LATEST_TAG" "0.10.0 (full)" "-full"
sums_for "$LATEST_TAG"

make_asset "$OLD_TAG" "0.9.9" ""
sums_for "$OLD_TAG"

# Corrupted: real tarball, wrong recorded checksum.
make_asset "$BAD_TAG" "0.6.6" ""
sums_for "$BAD_TAG"
echo "corrupt" >> "$SITE/releases/download/$BAD_TAG/largo-$OS-$ARCH.tar.gz"

# The redirect target must exist so a followed HEAD returns 200.
mkdir -p "$SITE/releases/tag"
echo "$LATEST_TAG" > "$SITE/releases/tag/$LATEST_TAG"

# ---------------------------------------------------------------------------
# Local server with the /releases/latest 302
# ---------------------------------------------------------------------------
PORT=$(( (RANDOM % 1000) + 8100 ))
python3 - "$SITE" "$PORT" "$LATEST_TAG" <<'PYEOF' &
import http.server, os, sys
site, port, latest = sys.argv[1], int(sys.argv[2]), sys.argv[3]
os.chdir(site)
class H(http.server.SimpleHTTPRequestHandler):
    def _latest(self):
        if self.path == '/releases/latest':
            self.send_response(302)
            self.send_header('Location', f'/releases/tag/{latest}')
            self.end_headers()
            return True
        return False
    def do_GET(self):
        if not self._latest(): super().do_GET()
    def do_HEAD(self):
        if not self._latest(): super().do_HEAD()
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(('127.0.0.1', port), H).serve_forever()
PYEOF
SERVER_PID=$!
BASE="http://127.0.0.1:$PORT"
for _ in $(seq 1 50); do
  curl -fs -o /dev/null "$BASE/releases/tag/$LATEST_TAG" 2>/dev/null && break
  sleep 0.1
done

run_install() {
  # run_install <home> <args...>  — runs install.sh with an isolated HOME
  home="$1"; shift
  HOME="$home" LARGO_BASE_URL="$BASE" sh "$INSTALLER" "$@" 2>&1
}

# ---------------------------------------------------------------------------
# Case 1+9: default install lands in ~/.local/bin under sh, dash, bash --posix
# ---------------------------------------------------------------------------
for shell in sh dash "bash --posix"; do
  command -v "${shell%% *}" >/dev/null 2>&1 || { note "skip: $shell not present"; continue; }
  h="$WORK/home-$(echo "$shell" | tr -d ' -')"
  mkdir -p "$h"
  out=$(HOME="$h" LARGO_BASE_URL="$BASE" $shell "$INSTALLER" 2>&1)
  bin="$h/.local/bin/largo"
  if [ -x "$bin" ] && [ "$("$bin")" = "largo 0.10.0" ]; then
    ok "default install under $shell"
  else
    fail "default install under $shell: $out"
  fi
done

# ---------------------------------------------------------------------------
# Case 2: --full fetches the full flavor (still installed as `largo`)
# ---------------------------------------------------------------------------
h="$WORK/home-full"; mkdir -p "$h"
out=$(run_install "$h" --full)
bin="$h/.local/bin/largo"
if [ -x "$bin" ] && [ "$("$bin")" = "largo 0.10.0 (full)" ]; then
  ok "--full installs the full flavor as largo"
else
  fail "--full: $out"
fi

# ---------------------------------------------------------------------------
# Case 3: --version pins an exact tag (no latest resolution)
# ---------------------------------------------------------------------------
h="$WORK/home-pin"; mkdir -p "$h"
out=$(run_install "$h" --version "$OLD_TAG")
bin="$h/.local/bin/largo"
if [ -x "$bin" ] && [ "$("$bin")" = "largo 0.9.9" ]; then
  ok "--version pins the exact tag"
else
  fail "--version pin: $out"
fi

# ---------------------------------------------------------------------------
# Case 4a: --to DIR overrides the install dir
# ---------------------------------------------------------------------------
h="$WORK/home-to"; mkdir -p "$h"
out=$(run_install "$h" --to "$h/custom")
if [ -x "$h/custom/largo" ]; then
  ok "--to DIR respected"
else
  fail "--to DIR: $out"
fi

# ---------------------------------------------------------------------------
# Case 4b: LARGO_INSTALL_DIR respected
# ---------------------------------------------------------------------------
h="$WORK/home-env"; mkdir -p "$h"
out=$(HOME="$h" LARGO_BASE_URL="$BASE" LARGO_INSTALL_DIR="$h/envdir" sh "$INSTALLER" 2>&1)
if [ -x "$h/envdir/largo" ]; then
  ok "LARGO_INSTALL_DIR respected"
else
  fail "LARGO_INSTALL_DIR: $out"
fi

# ---------------------------------------------------------------------------
# Case 5: re-run = clean upgrade
# ---------------------------------------------------------------------------
h="$WORK/home-upgrade"; mkdir -p "$h"
run_install "$h" >/dev/null 2>&1
out=$(run_install "$h"); rc=$?
bin="$h/.local/bin/largo"
if [ "$rc" -eq 0 ] && [ -x "$bin" ] && [ "$("$bin")" = "largo 0.10.0" ]; then
  ok "re-run upgrades cleanly"
else
  fail "re-run upgrade (rc=$rc): $out"
fi

# ---------------------------------------------------------------------------
# Case 6: checksum mismatch aborts, installs nothing
# ---------------------------------------------------------------------------
h="$WORK/home-corrupt"; mkdir -p "$h"
if out=$(run_install "$h" --version "$BAD_TAG"); then
  fail "corrupted tarball must abort (got success): $out"
else
  if [ ! -e "$h/.local/bin/largo" ] && echo "$out" | grep -qi "checksum"; then
    ok "checksum mismatch aborts with nothing installed"
  else
    fail "corrupted tarball: wrong failure shape: $out"
  fi
fi

# ---------------------------------------------------------------------------
# Case 7: missing asset (404) is actionable and mentions cargo fallback
# ---------------------------------------------------------------------------
h="$WORK/home-404"; mkdir -p "$h"
if out=$(run_install "$h" --version v0.0.1); then
  fail "404 must fail (got success): $out"
else
  if echo "$out" | grep -q "cargo install logicaffeine-cli"; then
    ok "404 failure mentions the cargo fallback"
  else
    fail "404 failure not actionable: $out"
  fi
fi

# ---------------------------------------------------------------------------
# Case 8: PATH advice fires when the dest is not on PATH
# ---------------------------------------------------------------------------
h="$WORK/home-path"; mkdir -p "$h"
out=$(run_install "$h" --to "$h/offpath")
if echo "$out" | grep -q "PATH"; then
  ok "PATH advice printed for off-PATH install dir"
else
  fail "PATH advice missing: $out"
fi

# ---------------------------------------------------------------------------
echo
echo "installer harness: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
