#!/usr/bin/env bash
# run-on-nexys-scalar — run a LOGOS bytecode program on the REAL Nexys A7-100T via the open flow. Builds
# the scalar-core SoC bitstream (once), programs Board B over JTAG, streams the program in over USB-UART,
# runs it, and reads back the Show values + on-chip run-cycle count.
#
#   scripts/run-on-nexys-scalar.sh [--build] <program.hex> [--expect v1,v2,...]
#
# The scalar core runs at 6.25 MHz (100 MHz /16; the open 7-series timing model is approximate so we clock
# conservatively). One program per configuration (the loader FSM latches at L_DONE) — this reprograms the
# board each run. The board's port and loader cable come from scripts/lib/ftdi_ports.py.
set -uo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
BIT=nexys_soc_scalar.bit
SCALAR_SRC="nexys_top_scalar.v nexys_soc_scalar.v soft_core_scalar.v alu.v"
BOARD=B     # default: Board B

BUILD=0; PROG=""; EXPECT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --build) BUILD=1 ;;
    --expect) shift; EXPECT="$1" ;;
    --board) shift; case "$1" in
        a|A) BOARD=A ;;
        b|B) BOARD=B ;;
        *) echo "unknown board '$1' (use a or b)"; exit 1 ;; esac ;;
    --expect=*) EXPECT="${1#*=}" ;;
    *) PROG="$1" ;;
  esac; shift
done
PORT=$(python3 scripts/lib/ftdi_ports.py port "$BOARD") || exit 1
echo "== target: Board $BOARD, UART $PORT =="
[ -n "$PROG" ] || { echo "usage: $0 [--build] <program.hex|program.lg> [--expect v1,v2,...]"; exit 1; }

# A .lg source is compiled to soft_core_scalar .hex first (the general LOGOS->chip path).
case "$PROG" in
  *.lg)
    HEX="${PROG%.lg}.hex"
    echo "== compiling $PROG -> $HEX (lg2hex) =="
    # Compile as the REAL (non-root) user even when the script is invoked under sudo, so cargo never writes
    # root-owned files into target/ or ~/.cargo (which would break later non-sudo builds).
    if [ -n "${SUDO_USER:-}" ]; then
      sudo -u "$SUDO_USER" bash -lc "cd '$(pwd)' && cargo run -q -p logicaffeine-compile --example lg2hex -- '$PROG' > '$HEX'" \
        || { echo "compile failed"; exit 1; }
    else
      cargo run -q -p logicaffeine-compile --example lg2hex -- "$PROG" > "$HEX" || { echo "compile failed"; exit 1; }
    fi
    PROG="$HEX"
    ;;
esac

if [ "$BUILD" = 1 ] || [ ! -s "$SIL/$BIT" ]; then
  echo "== building $BIT (scalar SoC, open flow) =="
  TOP=nexys_top_scalar SRC="$SCALAR_SRC" BIT="$BIT" bash scripts/openxc7-build.sh
fi

echo "== programming Board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$SIL/$BIT" 2>&1 | tail -1

echo "== running $PROG on silicon (6.25 MHz) =="
ARGS=("$PROG" --port "$PORT" --clock-mhz 6.25)
[ -n "$EXPECT" ] && ARGS+=(--expect "$EXPECT")
python3 scripts/nexys-host.py "${ARGS[@]}"
