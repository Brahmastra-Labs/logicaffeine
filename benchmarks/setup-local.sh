#!/usr/bin/env bash
# LOGICAFFEINE Local Benchmark Toolchain Setup
#
# Installs every compiler/runtime the benchmark suite needs, pinned where
# pinnable so every bench box measures with the same tools.
# Idempotent: tools that already resolve on PATH are left alone.
#
# Usage: bash benchmarks/setup-local.sh

set -euo pipefail

ZIG_VERSION=0.15.2
NIM_VERSION=2.0.4
HYPERFINE_VERSION=1.18.0

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

have() { command -v "$1" &>/dev/null; }

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
    SUDO="sudo"
fi

ARCH="$(uname -m)"
if [ "$ARCH" != "x86_64" ]; then
    warn "This script pins x86_64 artifacts for Zig/Nim/hyperfine; on $ARCH install those manually."
fi

APT_PACKAGES=()
have gcc     || APT_PACKAGES+=(gcc)
have g++     || APT_PACKAGES+=(g++)
have go      || APT_PACKAGES+=(golang-go)
have javac   || APT_PACKAGES+=(default-jdk)
have node    || APT_PACKAGES+=(nodejs)
have python3 || APT_PACKAGES+=(python3)
have ruby    || APT_PACKAGES+=(ruby-full)
have jq      || APT_PACKAGES+=(jq)
have bc      || APT_PACKAGES+=(bc)
have wget    || APT_PACKAGES+=(wget)
have xz      || APT_PACKAGES+=(xz-utils)

if [ ${#APT_PACKAGES[@]} -gt 0 ]; then
    info "Installing via apt: ${APT_PACKAGES[*]}"
    $SUDO apt-get update
    $SUDO apt-get install -y "${APT_PACKAGES[@]}"
else
    ok "All apt-provided tools already present"
fi

if ! have rustc; then
    fail "rustc not found — install Rust via https://rustup.rs (not auto-installed to avoid clobbering toolchain setups)"
fi

if ! have zig; then
    info "Installing Zig $ZIG_VERSION..."
    TMP=$(mktemp -d)
    wget -q "https://ziglang.org/download/$ZIG_VERSION/zig-x86_64-linux-$ZIG_VERSION.tar.xz" -O "$TMP/zig.tar.xz"
    tar xf "$TMP/zig.tar.xz" -C "$TMP"
    $SUDO rm -rf /usr/local/zig
    $SUDO mv "$TMP/zig-x86_64-linux-$ZIG_VERSION" /usr/local/zig
    $SUDO ln -sf /usr/local/zig/zig /usr/local/bin/zig
    rm -rf "$TMP"
    ok "Zig $(zig version) installed"
else
    ok "Zig already present: $(zig version)"
fi

if ! have nim; then
    info "Installing Nim $NIM_VERSION..."
    TMP=$(mktemp -d)
    wget -q "https://nim-lang.org/download/nim-$NIM_VERSION-linux_x64.tar.xz" -O "$TMP/nim.tar.xz"
    tar xf "$TMP/nim.tar.xz" -C "$TMP"
    $SUDO rm -rf /usr/local/nim
    $SUDO mv "$TMP/nim-$NIM_VERSION" /usr/local/nim
    for tool in nim nimble; do
        $SUDO ln -sf "/usr/local/nim/bin/$tool" "/usr/local/bin/$tool"
    done
    rm -rf "$TMP"
    ok "Nim installed: $(nim --version | head -1)"
else
    ok "Nim already present: $(nim --version | head -1)"
fi

if ! have hyperfine; then
    info "Installing hyperfine $HYPERFINE_VERSION..."
    TMP=$(mktemp -d)
    wget -q "https://github.com/sharkdp/hyperfine/releases/download/v$HYPERFINE_VERSION/hyperfine_${HYPERFINE_VERSION}_amd64.deb" -O "$TMP/hyperfine.deb"
    $SUDO dpkg -i "$TMP/hyperfine.deb"
    rm -rf "$TMP"
    ok "hyperfine installed: $(hyperfine --version)"
else
    ok "hyperfine already present: $(hyperfine --version)"
fi

echo
info "Toolchain summary:"
report() {
    local name="$1" cmd="$2" version_cmd="$3"
    if have "$cmd"; then
        printf "  %-12s %s\n" "$name" "$(eval "$version_cmd" 2>&1 | head -1)"
    else
        printf "  %-12s ${RED}MISSING${NC}\n" "$name"
    fi
}
report "C"          gcc      "gcc --version"
report "C++"        g++      "g++ --version"
report "Rust"       rustc    "rustc --version"
report "Zig"        zig      "zig version"
report "Go"         go       "go version"
report "Java"       javac    "javac -version"
report "JavaScript" node     "node --version"
report "Python"     python3  "python3 --version"
report "Ruby"       ruby     "ruby --version"
report "Nim"        nim      "nim --version"
report "hyperfine"  hyperfine "hyperfine --version"
report "jq"         jq       "jq --version"
report "bc"         bc       "bc --version"
