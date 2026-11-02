#!/usr/bin/env bash
# run-on-nexys-full — run a LOGOS program on the REAL Nexys A7-100T via the WIDE full core (soft_core.v,
# NO_BIGINT): functions, div/modulo, and the structured heap (lists/tuples/ranges/sets/maps) — the whole
# integer benchmark subset. Compiles `.lg` → full-core `.hex` (`lg2hex --full`), builds the full-core SoC
# bitstream (once) via the open flow, programs Board B over JTAG, streams the program in over USB-UART,
# runs it, and reads back the Show values + on-chip run-cycle count.
#
#   scripts/run-on-nexys-full.sh [--build] <program.lg|program.hex> [--expect v1,v2,...] [--board a|b]
#
# The wire protocol (nexys_soc_full.v) is identical to the scalar SoC, so scripts/nexys-host.py is reused.
set -uo pipefail
cd "$(dirname "$0")/.."
SIL=benchmarks/silicon
BIT=full_core.bit
# fpu.v LAST — soft_core.v instantiates fp_addsub/fp_mul/fp_div/fp_sqrt from it. nexys_soc_full.v precedes
# soft_core.v so its NO_BIGINT / SOFT_CORE_PROG_LOAD defines are set first.
FULL_SRC="nexys_top_full.v nexys_soc_full.v soft_core.v alu.v shared_divider.v crdt_merge.v fpu.v"
BOARD=B     # default: Board B

BUILD=0; PROG=""; EXPECT=""
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
[ -n "$PROG" ] || { echo "usage: $0 [--build] <program.lg|program.hex> [--expect v1,v2,...]"; exit 1; }

# A .lg source compiles to full-core .hex first (the general LOGOS->wide-chip path).
case "$PROG" in
  *.lg)
    HEX="${PROG%.lg}.hex"
    echo "== compiling $PROG -> $HEX (lg2hex --full) =="
    # Compile as the REAL (non-root) user even under sudo, so cargo never writes root-owned files.
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
  echo "== building $BIT (full-core SoC + FPU, open flow, NO_BIGINT) =="
  # SEED pinned to a placement PROVEN on silicon (SEED=1 + router2 for the current escape-loader RTL). The
  # open flow has no hold-fixing, so placement is hold-marginal; this seed lands a clean one. Override with
  # SEED=<n> to sweep if the RTL changes.
  # ABC9 keeps the sequential FPU inside the synth memory budget; PLACERELAX spreads the dense FPU so it
  # routes without congestion. (The FPU exponent path is pipelined in fpu.v, so it no longer relies on a
  # lucky placement to meet the exponent-tail timing — see the_fpu_exponent_path_stays_pipelined_off_registers.)
  # ROUTER=router2: the 2-byte-escape loader adds enough nets that the default router1 stalls on a small
  # placement-congestion hotspot (overuse=4); router2's net-based congestion resolution clears it. This
  # placement is PROVEN on silicon (a 256-word program loaded via the escape ran bit-exact ≡ VM).
  ABC9="${ABC9:-1}" PLACERELAX="${PLACERELAX:-1}" SEED="${SEED:-1}" ROUTER="${ROUTER:-router2}" \
    TOP=nexys_top_full SRC="$FULL_SRC" BIT="$BIT" bash scripts/openxc7-build.sh
fi

echo "== programming Board $BOARD =="
python3 scripts/lib/ftdi_ports.py flash "$BOARD" "$SIL/$BIT" 2>&1 | tail -1

echo "== running $PROG on silicon (6.25 MHz) =="
ARGS=("$PROG" --port "$PORT" --clock-mhz 6.25)
[ -n "$EXPECT" ] && ARGS+=(--expect "$EXPECT")
python3 scripts/nexys-host.py "${ARGS[@]}"
