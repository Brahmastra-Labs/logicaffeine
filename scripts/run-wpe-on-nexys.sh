#!/usr/bin/env bash
# run-wpe-on-nexys — stream a LOGOS program OVER THE WIRE to the UNIVERSAL whole-program engine on a real
# Nexys A7-100T and read the result back. Unlike run-on-nexys-*.sh (which load the soft-CORE's instruction
# memory), this speaks to the config-driven ENGINE: it emits the program's flat image as a `.wire` byte stream
# (WImage::wire_bytes) and streams it into the on-chip loader, which fills the engine's config memories (the
# chip synthesizes the program it is handed) and runs it — the "send any program over the wire" endpoint.
#
#   scripts/run-wpe-on-nexys.sh [--build] <program.lg> [--fn f] [--args a,b,...] [--expect N] [--board b]
#
# --build (re)synthesizes the engine bitstream (wpe_soc.bit) via the open flow. Subsequent runs reuse it —
# the engine is UNIVERSAL, so one bitstream runs every fitting program; only the streamed `.wire` changes.
set -uo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
BIT=wpe_soc.bit
BUILD=0 FN=f ARGS="" EXPECT="" BOARD=B PROG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --build) BUILD=1;;
    --fn) FN="$2"; shift;;
    --args) ARGS="$2"; shift;;
    --expect) EXPECT="$2"; shift;;
    --board) BOARD="$2"; shift;;
    *) PROG="$1";;
  esac; shift
done
[ -n "$PROG" ] || { echo "usage: $0 [--build] <program.lg> [--fn f] [--args a,b] [--expect N] [--board a|b]"; exit 1; }
PORT=$(python3 scripts/lib/ftdi_ports.py port "$BOARD") || exit 1

# 1. Emit the engine RTL (wpe_soc_fpga.v) + the program's wire bytes (<prog>.wire) from LOGOS.
WIRE="${PROG%.lg}.wire"
echo "== emitting engine + $PROG -> $WIRE (fn=$FN args=[$ARGS]) =="
cargo run -q -p logicaffeine-compile --example emit_wpe_soc -- \
  --engine "$SIL/wpe_soc_fpga.v" --program "$PROG" --fn "$FN" --args "$ARGS" --wire "$WIRE" \
  || { echo "emit failed"; exit 1; }

# 2. Synthesize the universal engine bitstream once (open flow); reuse for every subsequent program.
if [ "$BUILD" = 1 ] || [ ! -s "$SIL/$BIT" ]; then
  echo "== synthesizing $BIT (universal over-the-wire engine, open flow) =="
  TOP=nexys_wpe_soc SRC="nexys_wpe_soc.v wpe_soc_fpga.v" BIT="$BIT" bash scripts/openxc7-build.sh \
    || { echo "synth failed"; exit 1; }
fi

# 3. Flash + stream the program over the real UART, read the result frame back.
echo "== programming Board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$SIL/$BIT" 2>&1 | tail -1
sleep 2
echo "== streaming $WIRE over $PORT and reading the result =="
ARGV=("$WIRE" --port "$PORT" --baud 9600 --clock-mhz 0.78125)
[ -n "$EXPECT" ] && ARGV+=(--expect "$EXPECT")
python3 scripts/nexys-wpe-host.py "${ARGV[@]}"
