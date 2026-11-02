#!/usr/bin/env bash
# setup-openxc7 — one-time build of the OPEN (no-Vivado) Xilinx 7-series bitstream toolchain, so the
# board-runnable nexys_soc bitstream can be produced entirely with free software. Installs under
# tools/openxc7/ (git-ignored build scratch, NOT part of the cargo workspace). Idempotent: re-running
# skips anything already present. HEAVY — nextpnr-xilinx + prjxray are large C++ builds; run only when
# the box is free (pgrep for cargo/yosys/nextpnr first; this box is shared — CLAUDE rule 11).
#
# Pieces:
#   prjxray-db      — the reverse-engineered device database (has xc7a100t) . f4pga/prjxray-db
#   prjxray         — fasm2frames + xc7frames2bit (fasm -> frames -> .bit)   . f4pga/prjxray
#   nextpnr-xilinx  — the place-and-route engine + its chipdb generator      . gatecat/nextpnr-xilinx
#
# Result used by scripts/openxc7-build.sh and the board image flow (scripts/xc7/):
#   tools/openxc7/nextpnr-xilinx/nextpnr-xilinx                 (binary)
#   tools/openxc7/nextpnr-xilinx/xilinx/xc7a100t.bin            (chipdb, Nexys A7-100T)
#   tools/openxc7/nextpnr-xilinx/xilinx/xc7a200t.bin            (chipdb, Nexys Video)
#   tools/openxc7/prjxray/build/tools/xc7frames2bit             (binary)
#   tools/openxc7/prjxray-db                                    (db root)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
T="$ROOT/tools/openxc7"
# The nextpnr-xilinx commit every patch in scripts/openxc7-patches is written against.
NEXTPNR_XILINX_COMMIT=8f178fc6a6d4dfbc57bef66c3ccff34d558047d5
# The parts a chipdb is generated for: the Nexys A7-100T and the Nexys Video.
PARTS="xc7a100tcsg324-1 xc7a200tsbg484-1"
mkdir -p "$T"

echo "== [1/4] prjxray-db (device database) =="
[ -d "$T/prjxray-db/.git" ] || git clone --depth 1 https://github.com/f4pga/prjxray-db "$T/prjxray-db"

echo "== [2/4] prjxray (fasm2frames + xc7frames2bit) =="
if [ ! -x "$T/prjxray/build/tools/xc7frames2bit" ]; then
  [ -d "$T/prjxray/.git" ] || git clone --recursive https://github.com/f4pga/prjxray "$T/prjxray"
  ( cd "$T/prjxray" && git submodule update --init --recursive && make build -j"$(nproc)" )
  python3 -m pip install --user --quiet fasm textx simplejson intervaltree pyyaml || true
fi

echo "== [3/4] nextpnr-xilinx (place & route) =="
if [ ! -x "$T/nextpnr-xilinx/nextpnr-xilinx" ]; then
  if [ ! -d "$T/nextpnr-xilinx/.git" ]; then
    git clone https://github.com/gatecat/nextpnr-xilinx "$T/nextpnr-xilinx"
    ( cd "$T/nextpnr-xilinx" && git checkout --quiet "$NEXTPNR_XILINX_COMMIT" && git submodule update --init --recursive )
  fi
  # The repository's fixes to nextpnr-xilinx (scripts/openxc7-patches), each applied once, in name order: DSP48E1
  # cascades (an empty cascade-out, an unclustered root, the HCLK row in a column), LUT-RAM (INIT_A..D copied, a
  # RAM32M's bits in order; router2 never permuting a LUT-RAM's or shift register's pins), the annealing placer
  # checking every tile a chain swap leaves, as it checks a plain swap's, hard-cell timing: with
  # NEXTPNR_XC7_TIMINGS naming prjxray-db's SDF directory, the DSP48E1, block RAM, LUT-RAM and shift register ports the
  # chipdb leaves untimed are timed from Vivado's models (scripts/xc7/route-image.sh sets it), and a clock pad's path to
  # the global buffers it feeds bound over dedicated clock resources before routing, as a PLL's input is.
  for patch in "$ROOT"/scripts/openxc7-patches/*.patch; do
    ( cd "$T/nextpnr-xilinx" && git apply --reverse --check "$patch" 2>/dev/null || git apply "$patch" ) \
      && echo "  applied $(basename "$patch")" || { echo "FATAL: $(basename "$patch") does not apply"; exit 1; }
  done
  ( cd "$T/nextpnr-xilinx" \
    && cmake . -DARCH=xilinx -DBUILD_GUI=OFF -DCMAKE_BUILD_TYPE=Release \
    && make nextpnr-xilinx bbasm -j"$(nproc)" )
fi

echo "== [4/4] generate the chipdbs =="
for PART in $PARTS; do
  DEVICE=$(echo "$PART" | grep -oE '^xc7a[0-9]+t')
  if [ ! -s "$T/nextpnr-xilinx/xilinx/$DEVICE.bin" ]; then
    ( cd "$T/nextpnr-xilinx" \
      && PYTHONPATH="$T/prjxray:${PYTHONPATH:-}" XRAY_DIR="$T/prjxray" \
         python3 xilinx/python/bbaexport.py \
           --device "$PART" \
           --db-root "$T/prjxray-db/artix7" \
           --bba "xilinx/$DEVICE.bba" \
      && ./bbasm --l "xilinx/$DEVICE.bba" "xilinx/$DEVICE.bin" \
      && rm -f "xilinx/$DEVICE.bba" )
  fi
done

echo ""
echo "openxc7 toolchain ready under tools/openxc7/. Next: scripts/openxc7-build.sh, or scripts/xc7/sweep-image.sh for a board image"
