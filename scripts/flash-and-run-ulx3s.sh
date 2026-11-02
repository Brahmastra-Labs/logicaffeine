#!/usr/bin/env bash
# BOARD-READY — flash a LOGOS silicon block to a ULX3S (Lattice ECP5-85F) and explain how to drive
# it. Runs the moment a board is plugged in; needs no board on disk (it checks for the tool + the
# bitstream and guides you).
#
#   Usage:      scripts/flash-and-run-ulx3s.sh [scalar|coproc]   (default: scalar)
#   Board:      ULX3S 85F  (Lattice ECP5-85F, the open yosys/nextpnr/ecppack flow)
#   Tool:       openFPGALoader
#   Bitstreams: benchmarks/silicon/soft_core_scalar.bit  (scalar — the routable 966-LUT LOGOS CPU)
#               benchmarks/silicon/coproc_soc.bit         (coproc  — coproc_core + a UART result readout)
set -uo pipefail
cd "$(dirname "$0")/.."

TARGET="${1:-scalar}"
BOARD="ulx3s"
case "$TARGET" in
  scalar) BIT="benchmarks/silicon/soft_core_scalar.bit"; BUILD="bash scripts/impl-scalar-core-ecp5.sh"
          TITLE="LOGOS scalar CPU" ;;
  coproc) BIT="benchmarks/silicon/coproc_soc.bit"; BUILD="COPROC_SOC_RECORD=1 cargo test -p logicaffeine-compile coproc_soc_artifacts && bash scripts/synth-soft-core.sh"
          TITLE="coproc SoC (coproc_core + UART readout)" ;;
  *) echo "  unknown target '$TARGET' — use 'scalar' or 'coproc'"; exit 2 ;;
esac

echo "── $TITLE → ULX3S (ECP5-85F) ──"

if [ ! -f "$BIT" ]; then
  echo "  bitstream $BIT not found — build it first:"
  echo "    $BUILD"
  exit 1
fi
echo "  bitstream: $BIT ($(du -h "$BIT" | cut -f1))"

if ! command -v openFPGALoader >/dev/null 2>&1; then
  echo "  openFPGALoader not on PATH — install it, then re-run. To flash manually:"
  echo "    openFPGALoader -b $BOARD $BIT"
  exit 0
fi

echo "  flashing (SRAM, volatile) …"
if openFPGALoader -b "$BOARD" "$BIT"; then
  if [ "$TARGET" = "coproc" ]; then
    echo "  OK — the ULX3S is now running coproc_soc: it issued BADD/BSUB/BMUL/CMERGE to the"
    echo "  real bigint_alu/bigint_mul/crdt_merge blocks and is streaming the retired R2..R5 over"
    echo "  the FTDI UART. Read them back and verify against the software model (run_core):"
    echo "    scripts/coproc-host.py --port <the ULX3S's USB-serial device>"
    echo "  PASS proves a CPU-issued coprocessor op is correct on real silicon."
  else
    echo "  OK — the ULX3S is now running the scalar LOGOS CPU."
    echo
    echo "  Next: drive it over the ULX3S USB-serial link. The soft core reads a binisa chip-word"
    echo "  image and drives the result out; the host-side push/readout harness is the board-bring-up"
    echo "  step. Until then, scripts/demo-wire-to-silicon.sh proves silicon == VM under iverilog."
  fi
else
  echo "  flash failed — is the board plugged in and powered? Try: openFPGALoader --detect -b $BOARD"
  exit 1
fi
