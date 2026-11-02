#!/usr/bin/env bash
# openxc7-build — build a Nexys A7-100T (xc7a100t) bitstream from Verilog through the fully-OPEN flow
# (yosys synth_xilinx -> nextpnr-xilinx -> prjxray fasm2frames -> xc7frames2bit), NO Vivado. Requires the
# toolchain built by scripts/setup-openxc7.sh under tools/openxc7/.
#
#   TOP=<top-module> SRC="<space-separated .v files, top wrapper first>" BIT=<out.bit> \
#     bash scripts/openxc7-build.sh
#
# Notes learned bringing the first LOGOS core up on real silicon (see memory: fpga-bringup-campaign):
#  * nextpnr-xilinx's XDC parser needs BARE `[get_ports name]` — no {} braces / inner spaces. We transform
#    benchmarks/silicon/nexys_soc.xdc on the fly (its clk/rst/uart/led pins suit every top here).
#  * A timing "error" from nextpnr is TOLERATED when a FASM is produced — designs here run on a divided
#    (slow) core clock whose real frequency nextpnr can't infer through the divider.
#  * Any WRITE-PORTED memory in the RTL must be `(* ram_style="registers" *)` — the open flow drops
#    LUT-RAM writes (init'd or back-to-back), which silently breaks execution.
#
# Measurement runs (scripts/qor/run-qor-fpga.py) add:
#   REPORT_DIR=<dir>  keep yosys's `stat -json` (stat.json), the synthesis log and nextpnr's log (pnr.log) there
#   FREQ=<MHz>        the target frequency nextpnr times against (its XDC reader ignores create_clock)
#   STOP_AFTER=pnr    stop after place-and-route: the reports, no bitstream
#   XDC=/abs/path     an absolute XDC is read where it is; a relative one from benchmarks/silicon/
#   READ_FLAGS=-sv    flags for read_verilog (SystemVerilog sources)
#   WORK=<dir>        build in this (existing, caller-owned) directory instead of a fresh /tmp/openxc7.* one,
#                     so the caller removes it and can read nextpnr's log while it runs
set -uo pipefail
cd "$(dirname "$0")/.."
: "${TOP:?set TOP=<top module>}"; : "${SRC:?set SRC=<.v files>}"; : "${BIT:?set BIT=<out.bit>}"
# PART/CHIPDB default to the Nexys A7-100T. Override all three for another part, e.g. the Nexys Video:
#   PART=xc7a200tsbg484-1 CHIPDB=tools/openxc7/nextpnr-xilinx/xilinx/xc7a200t.bin \
#     TOP=nexys_video_smoke SRC=nexys_video_smoke.v BIT=nexys_video_smoke.bit XDC=nexys_video.xdc bash scripts/openxc7-build.sh
T=tools/openxc7; SIL=benchmarks/silicon; PART="${PART:-xc7a100tcsg324-1}"
DBROOT="${DBROOT:-$T/nextpnr-xilinx/xilinx/external/prjxray-db/artix7}"
CHIPDB="${CHIPDB:-$T/nextpnr-xilinx/xilinx/xc7a100t.bin}"
for f in "$T/nextpnr-xilinx/nextpnr-xilinx" "$CHIPDB" "$T/prjxray/build/tools/xc7frames2bit" "$T/prjxray/utils/fasm2frames.py"; do
  [ -e "$f" ] || { echo "MISSING $f — run scripts/setup-openxc7.sh first"; exit 1; }
done
W="${WORK:-$(mktemp -d /tmp/openxc7.XXXXXX)}"

echo "== [1/4] yosys synth_xilinx -top $TOP =="
# PRE=<yosys commands> runs between read_verilog and synth — e.g. `chparam -set WITH_FPU 1 nexys_wpe_soc;`.
( cd "$SIL" && yosys -q -l "$W/synth.log" -p "read_verilog ${READ_FLAGS:-} $SRC; ${PRE:-} synth_xilinx -family xc7 ${NODSP--nodsp} ${FLATTEN_FLAG--flatten} ${ABC9:+-abc9} ${SYNTH_EXTRA:-} -top $TOP; tee -q -o $W/stat.json stat -json; write_json $W/top.json" ) \
  || { echo "synth failed"; exit 1; }
keep_reports() {
  [ -n "${REPORT_DIR:-}" ] || return 0
  mkdir -p "$REPORT_DIR" && cp "$W/stat.json" "$W/synth.log" "$REPORT_DIR/" && { [ ! -f "$W/pnr.log" ] || cp "$W/pnr.log" "$REPORT_DIR/"; }
}

echo "== [2/4] nextpnr-xilinx place & route =="
XDC_SRC="${XDC:-nexys_soc.xdc}"; case "$XDC_SRC" in /*) ;; *) XDC_SRC="$SIL/$XDC_SRC";; esac
sed -E 's/get_ports \{[[:space:]]*([^}]*[^}[:space:]])[[:space:]]*\}/get_ports \1/g' "$XDC_SRC" > "$W/top.xdc"
# SEED=<n> varies the placement (open flow has no hold-fixing; a different placement can clear a
# hold-marginal path that makes a proven-correct design glitch on silicon). No-op when SEED is unset.
# ROUTER=router1 selects the older router (often resolves congestion that router2 oscillates on).
# PLACERELAX=1 loosens the placer so a dense design (e.g. the FPU) spreads out and routes.
"$T/nextpnr-xilinx/nextpnr-xilinx" --chipdb "$CHIPDB" --xdc "$W/top.xdc" ${SEED:+--seed "$SEED"} \
  ${ROUTER:+--router "$ROUTER"} ${PLACERELAX:+--placer-heap-timingweight ${PLACEWEIGHT:-5}} ${FREQ:+--freq "$FREQ"} \
  --json "$W/top.json" --fasm "$W/top.fasm" --write "$W/top_routed.json" > "$W/pnr.log" 2>&1
keep_reports
[ -s "$W/top.fasm" ] || { echo "P&R produced no FASM:"; tail -15 "$W/pnr.log"; exit 1; }
grep -iE 'Device utilisation|SLICE_|Max frequency' "$W/pnr.log" | tail -4
[ "${STOP_AFTER:-}" = pnr ] && { echo "PLACED AND ROUTED: reports in ${REPORT_DIR:-$W}"; exit 0; }

# VREF900: nextpnr hardcodes SSTL VREF to 675 mV; DDR2 (1.8 V) needs 900 mV. Opt-in FASM patch.
[ "${VREF900:-0}" = 1 ] && { sed -i 's/VREF\.V_675_MV/VREF.V_900_MV/g' "$W/top.fasm"; echo "== VREF patched 675 -> 900 mV =="; }

echo "== [3/4] fasm -> frames =="
PYTHONPATH="$T/prjxray" "$T/venv/bin/python" "$T/prjxray/utils/fasm2frames.py" \
  --db-root "$DBROOT" --part "$PART" "$W/top.fasm" "$W/top.frames" > "$W/frames.log" 2>&1 \
  || { echo "fasm2frames failed"; tail -15 "$W/frames.log"; exit 1; }

echo "== [4/4] frames -> bitstream =="
"$T/prjxray/build/tools/xc7frames2bit" --part_name "$PART" --part_file "$DBROOT/$PART/part.yaml" \
  --frm_file "$W/top.frames" --output_file "$SIL/$BIT" > "$W/bit.log" 2>&1
[ -s "$SIL/$BIT" ] || { echo "bitstream failed"; tail -15 "$W/bit.log"; exit 1; }
# The recipe, beside the bitstream: a .bit carries no record of the RTL, seed, router or FPU it was built with, and
# `scripts/logos-chip.py` reads the engine caps + FPU from this file to refuse programs the image cannot run.
CAPS=""; for f in $SRC; do c=$(grep -m1 'localparam NREG=' "$SIL/$f" 2>/dev/null | sed 's/^ *localparam //'); [ -n "$c" ] && CAPS="$c"; done
FPU=0; case "${PRE:-}" in *"WITH_FPU 1"*) FPU=1;; esac
{
  echo "built $(date -u +%FT%TZ) by scripts/openxc7-build.sh"
  echo "FPU=$FPU SEED=${SEED:-default} ROUTER=${ROUTER:-router2} TOP=$TOP SRC=\"$SRC\" PRE=\"${PRE:-}\" ($CAPS)"
  echo "rtl md5 $(cat $(for f in $SRC; do echo "$SIL/$f"; done) | md5sum | cut -d' ' -f1)"
  grep -iE 'Device utilisation|Max frequency' "$W/pnr.log" | tail -4
} > "$SIL/$BIT.build.txt"
echo "BITSTREAM READY: $SIL/$BIT ($(stat -c%s "$SIL/$BIT") bytes) — recipe in $SIL/$BIT.build.txt"
