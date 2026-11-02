#!/usr/bin/env bash
# setup-verilator — one-time build of Verilator under tools/verilator (git-ignored), with the bison and flex it is
# built by, from release tarballs. The board's whole computer simulates the real compiler's boot — about a billion
# hart cycles — which only a compiled simulator runs in minutes. Idempotent: what is built stays built. HEAVY (a
# large C++ build): run only when the box is free.
#
#   bash scripts/hw/setup-verilator.sh        then: tools/verilator/bin/verilator (VERILATOR_ROOT set by the wrapper)
set -euo pipefail
cd "$(dirname "$0")/../.."
T="$(pwd)/tools/verilator"
BISON=3.8.2
FLEX=2.6.4
VERILATOR=5.028
mkdir -p "$T/src" "$T/prefix" "$T/bin"
export PATH="$T/prefix/bin:$PATH"

fetch() {
  [ -s "$T/src/$2" ] && return 0
  curl -fsSL "$1" -o "$T/src/$2.part"
  mv -f "$T/src/$2.part" "$T/src/$2"
}

echo "== [1/3] bison $BISON =="
if [ ! -x "$T/prefix/bin/bison" ]; then
  fetch "https://ftp.gnu.org/gnu/bison/bison-$BISON.tar.xz" "bison-$BISON.tar.xz"
  ( cd "$T/src" && tar xf "bison-$BISON.tar.xz" && cd "bison-$BISON" \
    && ./configure --prefix="$T/prefix" --quiet && make -j"$(nproc)" -s && make install -s )
fi

echo "== [2/3] flex $FLEX =="
if [ ! -x "$T/prefix/bin/flex" ]; then
  fetch "https://github.com/westes/flex/releases/download/v$FLEX/flex-$FLEX.tar.gz" "flex-$FLEX.tar.gz"
  ( cd "$T/src" && tar xf "flex-$FLEX.tar.gz" && cd "flex-$FLEX" \
    && ./configure --prefix="$T/prefix" --quiet && make -j"$(nproc)" -s && make install -s )
fi

echo "== [3/3] verilator $VERILATOR =="
V="$T/src/verilator-$VERILATOR"
if [ ! -x "$V/bin/verilator_bin" ]; then
  fetch "https://github.com/verilator/verilator/archive/refs/tags/v$VERILATOR.tar.gz" "verilator-$VERILATOR.tar.gz"
  ( cd "$T/src" && tar xf "verilator-$VERILATOR.tar.gz" && cd "verilator-$VERILATOR" \
    && autoconf && ./configure --quiet && make -j"$(nproc)" -s )
fi

# Verilator runs from its build tree: the wrapper names its root.
cat > "$T/bin/verilator" <<WRAP
#!/usr/bin/env bash
export VERILATOR_ROOT="$V"
exec "$V/bin/verilator" "\$@"
WRAP
chmod +x "$T/bin/verilator"
"$T/bin/verilator" --version
