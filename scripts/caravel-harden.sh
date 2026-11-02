#!/usr/bin/env bash
# caravel-harden.sh — harden a Caravel project's `user_project_wrapper` at the FIXED 2920×3520 user area
# with its certified macro placed as `mprj`, run the efabless GOLDEN shuttle DRC on the streamed GDS, and
# record EVERY physical gate into the project's wrapper ledger.
#
#   scripts/caravel-harden.sh [project]      # a key of codegen_sva::caravel_project (default: dram)
#
# WHY THIS SCRIPT EXISTS. `caravel_signoff_lock` named it as the producer of the artifact, but it was
# never written — the wrapper was hardened by hand and its OpenLane config lived only under the
# gitignored `tools/` tree. That made the single most important physical artifact we own the ONE
# thing in the repo that could not be reproduced from the repo. This script emits the config itself,
# so the wrapper harden is reproducible from a clean checkout plus the fetched Caravel inputs.
#
# Every input is staged on every run, from the repo or from the fetched efabless scaffold
# (tools/caravel_user_project), never taken from what a previous run left in the design directory:
#   src/<wrapper RTL>, src/defines.v         benchmarks/silicon (the golden interface + our instantiation),
#                                            and the scaffold's defines.v
#   base_user_project_wrapper.sdc            benchmarks/silicon/caravel — the scaffold's constraints with
#                                            the clock-source latency declared once (see the file), timed
#                                            at the chip's signoff clock (CV_*, from the vehicle) and at
#                                            every clock the project runs (CW_*)
#   fixed_dont_change/, pin_order.cfg        the scaffold's fixed die template and wrapper pin order
#   macro.cfg                                the project's placement of `mprj`
# The hardened macro views under macro/ are published by scripts/caravel-macro-harden.sh.
set -euo pipefail

# THE WHOLE BODY IS ONE FUNCTION, called on the last line: a flow runs for hours and bash reads a plain script
# incrementally, so an edit under a running harden would resume its shell inside different text (openlane-run.sh).
logos_caravel_harden_main() {
cd "$(dirname "$0")/.."
ROOT=$(pwd)
OL="$ROOT/tools/OpenLane"
PROJECT="${1:-dram}"
SCAFFOLD="$ROOT/tools/caravel_user_project"
CUP="$SCAFFOLD/openlane/user_project_wrapper"

[ -d "$OL" ] || { echo "OpenLane not installed at $OL"; exit 1; }
[ -d "$CUP" ] || { echo "no Caravel scaffold at $SCAFFOLD (fetch it first)"; exit 1; }
EMIT="$ROOT/target/debug/examples/emit_flow_config"
[ -x "$EMIT" ] || cargo build -q -p logicaffeine-compile --example emit_flow_config
for _srcf in "$ROOT/crates/logicaffeine_compile/src/codegen_sva/caravel_project.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/signoff_blocks.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/wpe_caravel.rs" \
             "$ROOT/crates/logicaffeine_compile/examples/emit_flow_config.rs"; do
  if [ "$_srcf" -nt "$EMIT" ]; then
    echo "FATAL: $(basename "$_srcf") is NEWER than the prebuilt flow-config emitter; rebuild it:" >&2
    echo "         cargo build -p logicaffeine-compile --example emit_flow_config" >&2
    exit 4
  fi
done
CP=$("$EMIT" --caravel-project "$PROJECT" --pdk "${PDK:-sky130A}") \
  || { echo "FATAL: Caravel project $PROJECT is unknown or does not fit the vehicle (see above)" >&2; exit 4; }
eval "$CP"
SIL="$ROOT/benchmarks/silicon"
DES="$CP_WRAPPER_DESIGN"
D="$OL/designs/$DES"

for req in "macro/$CP_MACRO.lef" "macro/$CP_MACRO.gds" "macro/$CP_MACRO.lib" "macro/$CP_MACRO.v"; do
  [ -e "$D/$req" ] || { echo "missing hardened macro view designs/$DES/$req — run scripts/caravel-macro-harden.sh $PROJECT"; exit 1; }
done
mkdir -p "$D/src" "$D/fixed_dont_change"
rm -f "$D/src/"*.v
install -m644 "$SIL/$CP_WRAPPER_RTL" "$D/src/$CP_WRAPPER_RTL"
install -m644 "$SCAFFOLD/verilog/rtl/defines.v" "$D/src/defines.v"
install -m644 "$SIL/caravel/base_user_project_wrapper.sdc" "$D/base_user_project_wrapper.sdc"
install -m644 "$CUP/fixed_dont_change/user_project_wrapper.def" "$D/fixed_dont_change/user_project_wrapper.def"
install -m644 "$CUP/pin_order.cfg" "$D/pin_order.cfg"
printf 'mprj %s %s N\n' "$CP_MACRO_X" "$CP_MACRO_Y" > "$D/macro.cfg"

# ANTENNA. The wrapper shipped with GRT_REPAIR_ANTENNAS=0 — antenna repair was never enabled on it at
# all — which is why its route left 14 violators on met3/met4/met5, every one a Wishbone pin into
# `mprj`. `mpw_precheck` GATES on antenna, so those block a shuttle submission.
#
# The repair we use is the JOG ECO (scripts/odbpy/sat_antenna_jog.py), not diodes. That matters here:
# the wrapper deliberately runs with RUN_FILL_INSERTION=0 and RUN_TAP_DECAP_INSERTION=0 (it is a pure
# routing shell around one macro), so there are no well taps for a diode cell to sit next to. The jog
# promotes the offending metal one layer instead — ZERO cells added, nothing for Magic to flag — and
# it bridges DOWN for a top-layer (met5) violation where there is nothing above to climb to.
ANTENNA_JOG="${ANTENNA_JOG:-1}"
DRT_ECO_ITERS="${DRT_ECO_ITERS:-14}"
JOG_LINES=""
if [ "$ANTENNA_JOG" = "1" ]; then
  JOG_LINES='  "DRT_ANTENNA_ECO": 1,
  "ECO_JOG": 1,
  "DRT_ECO_ITERS": '"$DRT_ECO_ITERS"',
'
fi
# The wrapper is timed at every clock the chip runs (codegen_sva::caravel_project::CaravelProject::wrapper_clocks_ns):
# `clk` on wb_clk_i at CW_CORE_PERIOD_NS, and for a project whose logic runs on user_clock2, that clock's period too,
# from which the scaffold SDC creates `usr_clk`, asynchronous to `clk`.
USER_CLOCK_LINES=""
if [ "$CW_USER_CLOCK2_PERIOD_NS" != none ]; then USER_CLOCK_LINES="  \"LOGOS_USER_CLOCK2_PERIOD\": $CW_USER_CLOCK2_PERIOD_NS,"$'\n'; fi
# Diode-based repair stays OFF by default for the reason above. GRT_REPAIR_ANTENNAS=1 inserts diode
# cells; set CARAVEL_GRT_DIODES=1 only if the jog alone cannot converge AND you have re-enabled taps.
GRT_REPAIR="${CARAVEL_GRT_DIODES:-0}"

# THE WRAPPER IS A PURE ROUTING SHELL, and the fix for its timing belongs in the MACRO.
#
# Every resizer, tap and fill is OFF here: the shell holds one macro and no standard cells. Its hold
# paths all run from a harness input port to a flip-flop INSIDE `mprj` (97 of 97, read from
# reports/signoff/*rcx_sta.min.rpt), so they are closed on the macro's side — the macro is hardened to a
# hold margin that covers the port-to-pin arrival (codegen_sva::flow_config::VehicleMacro::hold_margin_ns).
#
# Buffering them here was MEASURED and is wrong. The resizers took the shell from no cells to 517,296,
# and with them: taps/fill off -> Magic `N-well spacing < 1.27um` on the isolated buffers; taps/fill
# on -> 359 `Metal2 spacing` violations where the jog-router's in-place bridges met the new fill; and
# LVS 0 -> 7,725. Input-port buffering alone still needed taps and fill, and took LVS 0 -> 7,755.
cat > "$D/config.json" <<EOF
{
  "DESIGN_NAME": "user_project_wrapper",
  "VERILOG_FILES": ["dir::src/defines.v", "dir::src/$CP_WRAPPER_RTL"],
  "VERILOG_FILES_BLACKBOX": ["dir::macro/$CP_MACRO.v"],
  "EXTRA_LEFS": "dir::macro/$CP_MACRO.lef",
  "EXTRA_GDS_FILES": "dir::macro/$CP_MACRO.gds",
  "EXTRA_LIBS": "dir::macro/$CP_MACRO.lib",
  "ROUTING_CORES": 1,
  "CLOCK_PERIOD": $CW_CORE_PERIOD_NS,
  "CLOCK_PORT": "wb_clk_i",
  "CLOCK_NET": "mprj.wb_clk_i",
${USER_CLOCK_LINES}  "FP_PDN_MACRO_HOOKS": "mprj vccd1 vssd1 vccd1 vssd1",
  "MACRO_PLACEMENT_CFG": "dir::macro.cfg",
  "MAGIC_DEF_LABELS": 0,
  "BASE_SDC_FILE": "dir::base_user_project_wrapper.sdc",
  "SYNTH_CLOCK_UNCERTAINTY": $CV_CLOCK_UNCERTAINTY_NS,
  "SYNTH_TIMING_DERATE": $CV_TIMING_DERATE,
  "IO_SYNC": 0,
  "MAX_TRANSITION_CONSTRAINT": 1.5,
  "RUN_LINTER": 0,
  "QUIT_ON_SYNTH_CHECKS": 0,
  "FP_PDN_CHECK_NODES": 0,
  "SYNTH_ELABORATE_ONLY": 1,
  "PL_RANDOM_GLB_PLACEMENT": 1,
  "PL_RESIZER_DESIGN_OPTIMIZATIONS": 0,
  "PL_RESIZER_TIMING_OPTIMIZATIONS": 0,
  "GLB_RESIZER_DESIGN_OPTIMIZATIONS": 0,
  "GLB_RESIZER_TIMING_OPTIMIZATIONS": 0,
  "PL_RESIZER_BUFFER_INPUT_PORTS": 0,
  "FP_PDN_ENABLE_RAILS": 0,
${JOG_LINES}  "GRT_REPAIR_ANTENNAS": ${GRT_REPAIR},
  "GRT_ALLOW_CONGESTION": 1,
  "RUN_FILL_INSERTION": 0,
  "RUN_TAP_DECAP_INSERTION": 0,
  "FP_PDN_VPITCH": 180,
  "FP_PDN_HPITCH": 180,
  "RUN_CTS": 0,
  "FP_PDN_VOFFSET": 5,
  "FP_PDN_HOFFSET": 5,
  "MAGIC_ZEROIZE_ORIGIN": 0,
  "FP_SIZING": "absolute",
  "DIE_AREA": "0 0 2920 3520",
  "RT_MAX_LAYER": "met5",
  "FP_DEF_TEMPLATE": "dir::fixed_dont_change/user_project_wrapper.def",
  "UNIT": 2.4,
  "FP_IO_VEXTEND": "expr::2 * \$UNIT",
  "FP_IO_HEXTEND": "expr::2 * \$UNIT",
  "FP_IO_VLENGTH": "expr::\$UNIT",
  "FP_IO_HLENGTH": "expr::\$UNIT",
  "FP_IO_VTHICKNESS_MULT": 4,
  "FP_IO_HTHICKNESS_MULT": 4,
  "FP_PDN_CORE_RING": 1,
  "FP_PDN_CORE_RING_VWIDTH": 3.1,
  "FP_PDN_CORE_RING_HWIDTH": 3.1,
  "FP_PDN_CORE_RING_VOFFSET": 12.45,
  "FP_PDN_CORE_RING_HOFFSET": 12.45,
  "FP_PDN_CORE_RING_VSPACING": 1.7,
  "FP_PDN_CORE_RING_HSPACING": 1.7,
  "FP_PDN_VWIDTH": 3.1,
  "FP_PDN_HWIDTH": 3.1,
  "FP_PDN_VSPACING": "expr::(5 * \$FP_PDN_CORE_RING_VWIDTH)",
  "FP_PDN_HSPACING": "expr::(5 * \$FP_PDN_CORE_RING_HWIDTH)",
  "VDD_NETS": ["vccd1", "vccd2", "vdda1", "vdda2"],
  "GND_NETS": ["vssd1", "vssd2", "vssa1", "vssa2"],
  "SYNTH_USE_PG_PINS_DEFINES": "USE_POWER_PINS",
  "RUN_IRDROP_REPORT": 0,
  "QUIT_ON_LVS_ERROR": 0,
  "GRT_ADJUSTMENT": 0.15,
  "QUIT_ON_HOLD_VIOLATIONS": 1
}
EOF

# Race-free run capture (same discipline as openlane-run.sh): record the run THIS invocation created,
# never "the newest mtime" — that race once stamped a clean verdict onto a run that failed setup.
BEFORE=$(ls -1d "$D/runs/"*/ 2>/dev/null | sort || true)
echo "== Caravel user_project_wrapper harden: $PROJECT, $CP_MACRO at ($CP_MACRO_X, $CP_MACRO_Y) (fixed 2920x3520) =="
# The flow runs the repo's OpenLane changes, laid over the fetched OpenLane every run (scripts/openlane-overlay).
bash "$ROOT/scripts/openlane-overlay/apply.sh"
( cd "$OL" && sg docker -c "make quick_run QUICK_RUN_DESIGN=$DES" )
RUN=$(comm -13 <(printf '%s\n' "$BEFORE") <(ls -1d "$D/runs/"*/ 2>/dev/null | sort) | tail -1)
[ -n "$RUN" ] || RUN=$(ls -td "$D/runs/"*/ | head -1)
RUN="${RUN%/}"
echo "== run: $RUN =="

GDS="$RUN/results/final/gds/user_project_wrapper.gds"
[ -f "$GDS" ] || { echo "FATAL: no wrapper GDSII streamed at $GDS" >&2; exit 2; }

# The GOLDEN shuttle DRC on the streamed GDS — the deck mpw_precheck accepts a submission against. The top
# cell is always `user_project_wrapper`; the verdict is filed under the project's own key.
bash "$ROOT/scripts/run-golden-precheck-drc.sh" user_project_wrapper "$GDS" "$CP_GOLDEN_KEY"

# Record EVERY gate, passing or not. Exits non-zero unless the wrapper is shuttle-clean, so a
# regression on any gate makes this command (and the lock reading the JSON) go RED.
# WRAPPER_LEDGER_OUT and GOLDEN_OUT file the records in other ledgers — staging copies a batch of re-signs lands from at once.
python3 "$ROOT/scripts/record-caravel-signoff.py" \
  "$RUN" "${GDS%.gds}.golden_drc.xml" "$CP_MACRO" "${WRAPPER_LEDGER_OUT:-$ROOT/$CP_WRAPPER_LEDGER}"
}
logos_caravel_harden_main "$@"; exit $?
