#!/usr/bin/env bash
# run-on-nexys-hft — run the FULL LOGOS tick-to-trade pipeline on the REAL Nexys A7-100T over the port.
# Wraps the HFT SoC (nexys_soc_hft.v: 512-word imem + 2-byte UART length) so a ~330-word program actually
# loads — past the 255-word limit of the stock full-core SoC. Compiles the `.lg` to full-core `.hex`
# (lg2hex --full), builds the bitstream once via the open flow (integer-only: fpu_stub.v, so it routes),
# programs the board over JTAG, streams the program in over USB-UART with the 2-byte length, runs it, and
# reads back the order stream + on-chip run-cycle count.
#
#   scripts/run-on-nexys-hft.sh [--build] [--board a|b] [program.lg|program.hex] [--expect v1,...]
#
# Default program: the committed benchmarks/silicon/programs/hft_pipeline.lg.
set -uo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
BIT=nexys_hft.bit
HFT_SRC="nexys_top_hft.v nexys_soc_hft.v soft_core.v alu.v shared_divider.v crdt_merge.v fpu_stub.v"
BOARD=B     # default: Board B
BUILD=0; PROG="$SIL/programs/hft_pipeline.lg"; EXPECT=""

while [ $# -gt 0 ]; do
  case "$1" in
    --build) BUILD=1 ;;
    --expect) shift; EXPECT="$1" ;;
    --expect=*) EXPECT="${1#*=}" ;;
    --board) shift; case "$1" in
        a|A) BOARD=A ;;
        b|B) BOARD=B ;;
        *) echo "unknown board '$1' (use a or b)"; exit 1 ;; esac ;;
    *) PROG="$1" ;;
  esac; shift
done
PORT=$(python3 scripts/lib/ftdi_ports.py port "$BOARD") || exit 1
echo "== target: Board $BOARD, UART $PORT =="

case "$PROG" in
  *.lg)
    HEX="${PROG%.lg}.hex"
    echo "== compiling $PROG -> $HEX (lg2hex --full) =="
    if [ -n "${SUDO_USER:-}" ]; then
      sudo -u "$SUDO_USER" bash -lc "cd '$(pwd)' && cargo run -q -p logicaffeine-compile --example lg2hex -- --full '$PROG' > '$HEX'" \
        || { echo "compile failed"; exit 1; }
    else
      cargo run -q -p logicaffeine-compile --example lg2hex -- --full "$PROG" > "$HEX" || { echo "compile failed"; exit 1; }
    fi
    PROG="$HEX"
    ;;
esac

if [ "$BUILD" = 1 ] || [ ! -s "$SIL/$BIT" ]; then
  echo "== building $BIT (HFT SoC, open flow, integer-only) =="
  SEED="${SEED:-3}" TOP=nexys_top_hft SRC="$HFT_SRC" BIT="$BIT" bash scripts/openxc7-build.sh
fi

echo "== programming Board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$SIL/$BIT" 2>&1 | tail -1

echo "== streaming the pipeline over the port (2-byte length, 6.25 MHz core) =="
ARGS=("$PROG" --port "$PORT" --baud 115200 --words16 --clock-mhz 6.25)
[ -n "$EXPECT" ] && ARGS+=(--expect "$EXPECT")
python3 scripts/nexys-host.py "${ARGS[@]}"
