#!/usr/bin/env bash
# impl-nexys-soc-openxc7 — build the board-runnable nexys_soc bitstream for the Nexys A7-100T
# (Artix-7 xc7a100t) using the OPEN toolchain (yosys -> nextpnr-xilinx -> prjxray), NO Vivado.
# This is the last mile of "run LOGOS bytecode on real silicon": the .bit programs the FPGA with the
# full 113-op soft-core + UART load/run/readback, then scripts/nexys-host.py streams a program in.
#
#   flow:  yosys synth_xilinx -> nextpnr-xilinx (P&R, XDC) -> fasm -> fasm2frames -> xc7frames2bit -> .bit
#   deps:  scripts/setup-openxc7.sh must have built tools/openxc7/ first.
#   run:   only when the box is free (shared machine — CLAUDE rule 11); this is a multi-minute P&R.
#
# Optional first arg TOP selects a smaller top module to de-risk routing before the full SoC
# (e.g. a blink). Default TOP=nexys_soc. Writes benchmarks/silicon/<TOP>.bit + an impl log.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
T="$ROOT/tools/openxc7"
SIL="$ROOT/benchmarks/silicon"
PART="xc7a100tcsg324-1"
DEVICE="xc7a100t"
TOP="${1:-nexys_soc}"

NEXTPNR="$T/nextpnr-xilinx/nextpnr-xilinx"
CHIPDB="$T/nextpnr-xilinx/xilinx/$DEVICE.bin"
FRAMES2BIT="$T/prjxray/build/tools/xc7frames2bit"
# Use the prjxray-db bundled with nextpnr-xilinx (pinned to match the chipdb + metadata) for fasm2frames.
DBROOT="$T/nextpnr-xilinx/xilinx/external/prjxray-db/artix7"
FASM2FRAMES="$T/prjxray/utils/fasm2frames.py"
PYVENV="$T/venv/bin/python"       # has fasm/textx/etc.; prjxray package via PYTHONPATH

for f in "$NEXTPNR" "$CHIPDB" "$FRAMES2BIT" "$FASM2FRAMES"; do
  [ -e "$f" ] || { echo "MISSING: $f — run scripts/setup-openxc7.sh first"; exit 1; }
done
command -v yosys >/dev/null || { echo "yosys not found"; exit 1; }

# The nexys_soc build set (nexys_soc.v defines SOFT_CORE_PROG_LOAD, so it must be read BEFORE soft_core.v).
SRC=( nexys_soc.v soft_core.v alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v )
XDC="$SIL/nexys_soc.xdc"
WORK="$(mktemp -d /tmp/openxc7-$TOP.XXXXXX)"
LOG="$SIL/${TOP}_openxc7_impl.log"

echo "== [1/4] yosys synth_xilinx -top $TOP ==" | tee "$LOG"
( cd "$SIL" && yosys -q -p "
    read_verilog ${SRC[*]}
    synth_xilinx -flatten -nowidelut -family xc7 -top $TOP
    write_json $WORK/$TOP.json
  " ) 2>&1 | tee -a "$LOG"

echo "== [2/4] nextpnr-xilinx place & route ==" | tee -a "$LOG"
# nextpnr-xilinx's XDC parser (xilinx/xdc.cc) is stricter than Vivado's. set_property's get_cells
# strips {} braces, but create_clock's get_nets (xdc.cc:110) does NOT — so `[get_ports {clk}]` makes it
# look up a net literally named "{clk}" and crash (dict::at). BARE `[get_ports clk]` works for BOTH
# commands, so strip the braces (and any inner spaces) around every get_ports target. The -dict and
# -waveform braces are left untouched. Output stays a valid (if plainer) constraints file.
NPXDC="$WORK/$TOP.nextpnr.xdc"
sed -E 's/get_ports \{[[:space:]]*([^}]*[^}[:space:]])[[:space:]]*\}/get_ports \1/g' "$XDC" > "$NPXDC"
"$NEXTPNR" --chipdb "$CHIPDB" --xdc "$NPXDC" --freq 100 \
           --json "$WORK/$TOP.json" --fasm "$WORK/$TOP.fasm" \
           --write "$WORK/${TOP}_routed.json" 2>&1 | tee -a "$LOG"

echo "== [3/4] fasm -> frames ==" | tee -a "$LOG"
PYTHONPATH="$T/prjxray:${PYTHONPATH:-}" XRAY_DIR="$T/prjxray" XRAY_DATABASE_DIR="$DBROOT" \
  "$PYVENV" "$FASM2FRAMES" --db-root "$DBROOT" --part "$PART" "$WORK/$TOP.fasm" "$WORK/$TOP.frames" 2>&1 | tee -a "$LOG"

echo "== [4/4] frames -> bitstream ==" | tee -a "$LOG"
"$FRAMES2BIT" --part_name "$PART" --part_file "$DBROOT/$PART/part.yaml" \
              --frm_file "$WORK/$TOP.frames" --output_file "$SIL/$TOP.bit" 2>&1 | tee -a "$LOG"

echo ""
echo "wrote $SIL/$TOP.bit  (log: $LOG)"
echo "load it:  python3 scripts/lib/ftdi_ports.py flash B $SIL/$TOP.bit"
echo "run it :  python3 scripts/nexys-host.py benchmarks/silicon/programs/sum_loop.hex --expect 15"
