#!/usr/bin/env bash
# Run a LOGOS silicon block through the OpenLane / Sky130 RTL-to-GDSII flow and record a
# SIGNOFF-CLEAN tape-out artifact (GDSII + DRC=0 + LVS-match + antenna=0 + routing=0).
#
#   scripts/openlane-run.sh <block> <clock_port> <clock_period_ns> [rtl.v ...]
#   e.g.  scripts/openlane-run.sh rev_alu_core clk 7.7 rev_alu_core.v
#         scripts/openlane-run.sh soft_core_scalar clk 15 soft_core_scalar.v alu.v
#
# Requires OpenLane v1 installed under tools/OpenLane (Docker image + Sky130 PDK via ciel).
# The RTL is kept PDK-AGNOSTIC (generic Verilog, no ECP5/Xilinx primitives), so the same
# source that fits the Nexys A7 also tapes out on Sky130. Writes benchmarks/silicon/sky130_signoff.json.
set -euo pipefail

# THE WHOLE BODY IS ONE FUNCTION, called on the last line. Bash reads a plain script
# INCREMENTALLY from a byte offset, so editing this file while a flow is executing it made the
# running shell resume in the middle of different text. That happened THREE times on this file
# (a transient syntax error after a successful route that lost the recording step; an 18-line
# insertion reverted within the minute; a one-line insertion reverted before two live flows
# returned from docker), and a dedicated wait-then-apply helper existed and was bypassed anyway.
# A function body is parsed in full before its first command runs, so a mid-run edit can no
# longer reach a running shell. The class is closed by construction, not by remembering.
logos_openlane_run_main() {
cd "$(dirname "$0")/.."
ROOT=$(pwd)
SIL="$ROOT/benchmarks/silicon"
OL="$ROOT/tools/OpenLane"
# OL_STOCK=1 — a CONTROL run: the block through OpenLane as released (its OpenROAD carrying the bench's one fix, as
# every bench run's does, below), at the commit the repo's overlay is written over (scripts/openlane-overlay/BASE),
# from the bench's own stock tree (scripts/qor/fetch-qor-tools.py openlane-stock), so the overlay's contribution is
# the difference between a block's run here and its run on tools/OpenLane. That tree, stamped with that commit, is the
# only one a flow runs without the overlay.
# OL_BENCH=1 — a run of the comparative-synthesis bench: the overlaid flow, in the bench's own tree, OpenLane as
# released at that commit (scripts/qor/fetch-qor-tools.py openlane-bench) with the overlay laid over it, so no bench
# design sits in tools/OpenLane/designs, each of whose designs the repository traces to RTL it tracks.
OL_STOCK="${OL_STOCK:-0}"
OL_BENCH="${OL_BENCH:-0}"
OL_IMAGE=""
if [ "$OL_STOCK" != 0 ] && [ "$OL_STOCK" != 1 ]; then
  echo "FATAL: OL_STOCK is 1 (a control run on stock OpenLane) or 0, not '$OL_STOCK'" >&2
  exit 1
fi
if [ "$OL_BENCH" != 0 ] && [ "$OL_BENCH" != 1 ]; then
  echo "FATAL: OL_BENCH is 1 (a comparative-bench run in the bench's tree) or 0, not '$OL_BENCH'" >&2
  exit 1
fi
if [ "$OL_STOCK" = 1 ] && [ "$OL_BENCH" = 1 ]; then
  echo "FATAL: OL_STOCK=1 and OL_BENCH=1 each name the tree a run flows in, the stock tree and the overlaid bench tree: never both" >&2
  exit 1
fi
if [ "$OL_STOCK" = 1 ] || [ "$OL_BENCH" = 1 ]; then
  BASE=$(tr -d '[:space:]' < "$ROOT/scripts/openlane-overlay/BASE")
  if [ "$OL_STOCK" = 1 ]; then
    OL="$ROOT/tools/qor/openlane-stock"
    if [ "$(tr -d '[:space:]' < "$OL/.logos-base" 2>/dev/null)" != "$BASE" ]; then
      echo "FATAL: no stock OpenLane at the overlay's base $BASE in $OL (scripts/qor/fetch-qor-tools.py openlane-stock)" >&2
      exit 1
    fi
  else
    OL="$ROOT/tools/qor/openlane-bench"
    if [ "$(tr -d '[:space:]' < "$OL/.logos-base" 2>/dev/null)" != "$BASE" ]; then
      echo "FATAL: no bench OpenLane at the overlay's base $BASE in $OL (scripts/qor/fetch-qor-tools.py openlane-bench)" >&2
      exit 1
    fi
  fi
fi
# ANCHOR_IR=1 (the default) replays the run's IR analysis with anchored sources (scripts/anchor-ir-drop.sh) before it
# is recorded; 0 records OpenROAD's own. A bench run and a control record only the anchored analysis: the bench
# compares their verdicts, and signoff_clean gates on the drop.
ANCHOR_IR="${ANCHOR_IR:-1}"
if [ "$ANCHOR_IR" != 0 ] && [ "$ANCHOR_IR" != 1 ]; then
  echo "FATAL: ANCHOR_IR is 1 (the run's IR analysis replayed anchored before it is recorded) or 0, not '$ANCHOR_IR'" >&2
  exit 1
fi
if [ "$ANCHOR_IR" = 0 ] && { [ "$OL_STOCK" = 1 ] || [ "$OL_BENCH" = 1 ]; }; then
  echo "FATAL: ANCHOR_IR=0 with OL_STOCK=1 or OL_BENCH=1: a bench run and a control record only an anchored IR drop" >&2
  exit 1
fi

BLOCK="${1:?usage: openlane-run.sh <block> <clk_port> <period_ns> [rtl.v ...]}"
CLKP="${2:?clock port, e.g. clk}"
PERIOD="${3:?clock period ns, e.g. 10}"
shift 3
RTL=("$@"); [ ${#RTL[@]} -gt 0 ] || RTL=("$BLOCK.v")

[ -d "$OL" ] || { echo "OpenLane not installed at $OL (git clone + make)"; exit 1; }
DES="logos_$BLOCK"
mkdir -p "$OL/designs/$DES/src"
for f in "${RTL[@]}"; do cp "$SIL/$f" "$OL/designs/$DES/src/"; done
# PRE-FLIGHT: the clock port must be a port of the TOP MODULE.
#
# MEASURED, and this is why it earns a check that costs milliseconds: naming `clk` as the clock
# port of `dram_executor_wb`, a top module whose clock is `wb_clk_i`, ran the whole 20-minute flow
# and produced
#   DRC 0 - LVS 0 - XOR 0 - antenna 0/0 - routing 0 - 0/0 violated paths - wns 0.00
# every one of which is TRUE and every one of which is MEANINGLESS: the SDC constrained a port
# that is not on the top module, so STA reported `worst slack INF` and nothing was timed. It
# also skipped CTS, so the cell count fell by half (11,502 -> 5,373) and read as an improvement.
# An unconstrained design is the most dangerous way to look clean, because it passes every gate
# by having no paths rather than by meeting them. The recorder did catch it downstream (None
# slacks -> NOT CLEAN), but the run should never have started.
#
# Scoped to the top module's own port list on purpose. A file-wide grep is NOT enough: this very
# file declares an inner `dram_executor` whose port IS `clk` and wires it `.clk(wb_clk_i)`, so a
# whole-file search matches and the guard passes while the design stays unconstrained.
python3 - "$OL/designs/$DES/src" "$BLOCK" "$CLKP" <<'PYCLK' || exit 4
import glob, os, re, sys
srcdir, top, clkp = sys.argv[1], sys.argv[2], sys.argv[3]
text = ""
for f in sorted(glob.glob(os.path.join(srcdir, "*.v"))):
    with open(f, errors="replace") as fh:
        text += fh.read() + "\n"
text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
text = re.sub(r"//[^\n]*", " ", text)          # a commented-out port must not satisfy this
m = re.search(r"\bmodule\s+" + re.escape(top) + r"\b(.*?);", text, re.S)
if not m:
    sys.stderr.write("FATAL: no `module %s` found in the staged RTL.\n" % top)
    sys.exit(4)
header = m.group(1)
if re.search(r"(^|[^A-Za-z0-9_])" + re.escape(clkp) + r"([^A-Za-z0-9_]|$)", header):
    sys.exit(0)
cands = sorted(set(re.findall(r"\b[A-Za-z_][A-Za-z0-9_]*(?:clk|clock)[A-Za-z0-9_]*\b", header)))
sys.stderr.write("FATAL: '%s' is not a port of top module '%s'.\n" % (clkp, top))
sys.stderr.write("       Clock-looking ports on %s: %s\n"
                 % (top, ", ".join(cands) if cands else "(none)"))
sys.stderr.write("       A clock that is not a top-level port constrains nothing: STA reports\n")
sys.stderr.write("       'worst slack INF' and every timing gate then passes by having no\n")
sys.stderr.write("       paths rather than by meeting them.\n")
sys.exit(4)
PYCLK
# A baked boot ROM ($readmemh "program.hex") is resolved by yosys via the -I include dir, so the
# .hex lives next to the RTL in src/ and VERILOG_INCLUDE_DIRS points there — the RTL stays byte-
# identical to the iverilog/our-SAT-proven source (no absolute paths, no ROM inlining). A block with
# a boot ROM of its own ships it as benchmarks/silicon/<block>.hex; every other ROM-bearing block
# boots the shared program.hex.
if [ -f "$SIL/$BLOCK.hex" ]; then
  cp "$SIL/$BLOCK.hex" "$OL/designs/$DES/src/program.hex"
elif [ -f "$SIL/program.hex" ]; then
  cp "$SIL/program.hex" "$OL/designs/$DES/src/"
fi

# Floorplan knobs (env-overridable). A PAD-LIMITED design — one with many wide top-level I/O buses
# (e.g. the integrated infinichip subsystem: a 64-bit command word + two 72-bit ECC codewords + two
# 64-bit data buses = ~386 pins) — needs an explicit larger die so every pin fits on the boundary.
# Set OL_DIE_AREA="0 0 W H" (µm) for those; the defaults reproduce the auto-sized, 35%-util
# floorplan the first blocks taped out with. The RTL stays byte-identical either way. The config merge
# below receives it as an argument: an unexported shell default never reached its environment, and a new
# block silently placed at OpenLane's own 50.
FP_CORE_UTIL="${FP_CORE_UTIL:-35}"
# NO DEFAULT. `emit_flow_config` carries the MEASURED value (0.45) with the experiment that
# produced it recorded at the field; a default here would shadow it. Set only to override.
PL_TARGET_DENSITY="${PL_TARGET_DENSITY:-}"
# The heuristic diode pass (default on) can over-insert on antenna-dense logic — a wide
# combinational multiply saw it emit MORE diodes than cells, which then fails detailed-
# placement legalization (DPL-0036). It also FLOODS the die with licon-bearing diode cells; where
# they pack densely, Magic and KLayout merge the `npc` (nitride poly cut) layer differently — a
# signoff XOR difference that no metal fix touches. On such blocks prefer the JOG-PRIMARY recipe:
# HEUR_DIODE=0 + the robust antenna jog below (ANTENNA_JOG=1). The jog splits wires (met->met+1) with
# NO cells, so it clears antenna without diodes, without an npc flood, and without an XOR artifact.
HEUR_DIODE="${HEUR_DIODE:-1}"
# Antenna ratio above which the heuristic pass drops a diode. Lower = more diodes (default
# 10 diodes nearly every net). Raise it on antenna-dense logic so only the worst nets get a
# diode — a legalizable count — while the router's GRT repair handles the rest.
# NO DEFAULT — see PL_TARGET_DENSITY. This one had a default of 10, and because the argument
# below was passed UNCONDITIONALLY it silently overrode the compiler's measured 300 on every
# single run that went through this script. The direction is counterintuitive (a HIGHER threshold
# gives a LOWER antenna residual, because a low one floods the die with diodes that crowd out the
# ones the ARC check actually wants), so the shadowed value was also the wrong one:
# systolic_os_pipe8_realbist built at 10 came out 536,104 cells with 8 max-slew violations, and
# the same block at 300 came out 392,221 cells with 3.
HEUR_THRESH="${HEUR_THRESH:-}"
# GRT_ANT_ITERS / GRT_ANT_MARGIN — the global router's own (post-route, targeted) antenna repair: its
# iteration count and its margin. NO DEFAULT: the compiler's measured 25 and 50 govern
# (ClosureSettings::grt_antenna_iters, ::grt_antenna_margin_pct); a default of 15 and 20 here reached no run.
GRT_ANT_ITERS="${GRT_ANT_ITERS:-}"
GRT_ANT_MARGIN="${GRT_ANT_MARGIN:-}"
# GRT_REPAIR=0 — the router's repair off, the pre-route heuristic diodes kept: the compiler's
# AntennaStrategy::HeuristicDiodesAndJog. With HEUR_DIODE=0 as well it is JogOnly. The router's repair
# without the heuristic pass is no strategy, and is refused below.
GRT_REPAIR="${GRT_REPAIR:-}"
# LOGOS robust antenna jog — ON by default. The SAT jog-router (scripts/odbpy/sat_antenna_jog.py,
# driven by run_antenna_eco in tcl_commands/routing.tcl) promotes a violating gate's metal one layer
# up IN PLACE — no diodes, no re-route (so no net stranding), junction-safe (branch references are
# re-based, so LVS net-count is preserved), and it ITERATES to convergence (DRT_ECO_ITERS passes).
# The flow is byte-identical for a design with no antenna (run_or_antenna_check finds none → returns),
# so this is a safe universal default. It makes antenna a solved, reusable step for every block rather
# than per-block firefighting. Set ANTENNA_JOG=0 to fall back to diodes-only.
ANTENNA_JOG="${ANTENNA_JOG:-1}"
# NO DEFAULT — the compiler's measured 30 governs unless a caller overrides. (JOG_LINES and
# DIE_LINES used to be built here for the config heredoc; that heredoc is gone, the compiler emits
# DRT_ANTENNA_ECO/ECO_JOG/DRT_ECO_ITERS itself, and OL_DIE_AREA is applied by the post-processing
# block further down. Both variables were assigned and never read.)
DRT_ECO_ITERS="${DRT_ECO_ITERS:-}"
# SYNTH_MAX_FANOUT and RT_MIN_SIGNAL_LAYER are REFUSED below. The first would move the fanout limit itself:
# MAX_FANOUT_CONSTRAINT is the bar signoff judges every net against, and the overlay's CTS and diode-aware
# steps hold every net within it. The second is MEASURED not to route: met2 as the lowest signal layer failed
# gpu_simt at FP_CORE_UTIL 25 (GRT-0119), and the met1 cost penalty below is the form that works.
SYNTH_MAX_FANOUT="${SYNTH_MAX_FANOUT:-}"
RT_MIN_SIGNAL_LAYER="${RT_MIN_SIGNAL_LAYER:-}"
# GRT_MET1_ADJUST — global-route cost on met1 (0..1, higher = router avoids met1). Softer than raising the
#   min layer (met1 stays available → no congestion) but only shifts GLOBAL routing, not detailed pin
#   access. Unset = the closure recipe's. Forwarded as the compiler's --met1-penalty, which places it in
#   GRT_LAYER_ADJUSTMENTS by the PDK stack's roles.
GRT_MET1_ADJUST="${GRT_MET1_ADJUST:-}"
# PDN_VPITCH — the vertical power-strap pitch in um, for a block whose cells draw current midway between two
#   straps (ClosureSettings::pdn_vpitch_um). Unset = OpenLane's own.
PDN_VPITCH="${PDN_VPITCH:-}"
# HOLD_SLACK_MARGIN — the hold repair's margin in ns, placement and global-route both, for a block whose routed
#   clock skew outgrows the estimate the repair judges (ClosureSettings::hold_slack_margin_ns). Unset = OpenLane's.
HOLD_SLACK_MARGIN="${HOLD_SLACK_MARGIN:-}"
# GRT_OVERFLOW_ITERS / GRT_ADJUSTMENT — global routing's overflow iterations and the share of each gcell's capacity
#   it holds back, for a block OpenLane's own 50 and 0.3 leave over capacity (ClosureSettings::grt_overflow_iters,
#   grt_adjustment). Unset = OpenLane's.
GRT_OVERFLOW_ITERS="${GRT_OVERFLOW_ITERS:-}"
GRT_ADJUSTMENT="${GRT_ADJUSTMENT:-}"
# DPL_PADDING — the sites a side every cell keeps free of its neighbours when placement is legalized, for a block
#   whose abutting cells leave the detailed router shorts it cannot clear (ClosureSettings::dpl_cell_padding_per_side).
#   Unset = OpenLane's none.
DPL_PADDING="${DPL_PADDING:-}"
# SETUP_SLACK_MARGIN — the setup repair's margin in ns, placement and global-route both, for a block whose routed paths
#   at the max-RC corner outgrow the estimate the repair judges (ClosureSettings::setup_slack_margin_ns). Unset =
#   OpenLane's.
SETUP_SLACK_MARGIN="${SETUP_SLACK_MARGIN:-}"

# ELECTRICAL CLOSURE — and a correction, because the first version of this comment was wrong.
#
# The claim was "the resizer's design pass was never enabled". MEASURED: false for this script.
# OpenLane defaults PL_RESIZER_DESIGN/TIMING_OPTIMIZATIONS and their GLB_ twins to 1
# (configuration/placement.tcl:23-24, routing.tcl:50-51), so they were always on here. Re-running
# dram_executor with them set explicitly produced a BYTE-IDENTICAL GDS (2,461,134 B, 3,293 cells)
# and the same 57 max-fanout violations. Setting them explicitly is still worth doing — an
# inherited default is a silent dependency on the tool's config — but it changes nothing by itself.
# (The claim IS true of scripts/caravel-harden.sh, which sets all four to 0 on purpose.)
#
# Fanout: MAX_FANOUT_CONSTRAINT reaches abc's `buffer -c -N` during synthesis and `set_max_fanout` in
# base.sdc, and both resizers repair against the SDC's limit with no flag of their own — every logic net of
# every signed-off block was within it. The loads added after the resizer are what exceeded it, antenna diodes
# (the liberty counts each as one) and clock-tree leaves, and the overlay's CTS and diode-aware fanout step
# hold those within it too (ClosureSettings::cts_within_fanout, ::diode_fanout_repair).
# QUIT_ON_HOLD_VIOLATIONS makes the flow refuse to emit a hold-violating block rather than produce one for a
# recorder to excuse.
RESIZER_DESIGN="${RESIZER_DESIGN:-1}"
RESIZER_TIMING="${RESIZER_TIMING:-1}"
QUIT_ON_HOLD="${QUIT_ON_HOLD:-1}"
# COMPARATIVE=1 — a block measured beside others, not signed off (ClosureSettings::comparative): a setup miss completes
# and is recorded, and the synthesis checks' findings go to the recorder instead of stopping the flow.
COMPARATIVE="${COMPARATIVE:-0}"

# THE CONFIG COMES FROM THE COMPILER, not from this script.
#
# It used to be a heredoc here, and the settings that were MEASURED to be necessary — the resizer's
# slew/cap margin, the heuristic diode threshold, the met1 route penalty, the antenna ECO — lived
# in whoever was running it. Every new block re-learned them, and a block could be signed off
# having silently missed one. `codegen_sva::flow_config` now owns that decision with the
# measurement quoted at each value, `codegen_sva::pdk_target` owns the process, and
# `tests/flow_config_lock.rs` holds both to it. This script's job is to run the flow, not to know
# physics.
#
# The env overrides below still work; they are for deliberate experiments, and a block built with
# something other than the defaults should have a recorded reason.
# NO DEFAULT, deliberately: an unpinned core count is what made detailed_route diverge under
# load (a clean 25->0 became 22090->27653 at load 127), so the flow must not guess one. But
# `set -u` turned the omission into a bare `ROUTING_CORES: unbound variable` with no hint of
# what to set or why, which cost one launch.
if [ -z "${ROUTING_CORES:-}" ]; then
  echo "FATAL: ROUTING_CORES is not set, and this flow will not pick a value for you." >&2
  echo "       OpenLane's multithreaded detailed_route is not deterministic across thread" >&2
  echo "       counts: an oversubscribed box changes the RESULT, not merely the runtime." >&2
  echo "       This box has $(nproc) cores; every signed-off block used ROUTING_CORES=4." >&2
  exit 4
fi
CFG_ARGS=( "$BLOCK" "$CLKP" "$PERIOD" --pdk "${PDK:-sky130A}" --cores "$ROUTING_CORES" )
case "$COMPARATIVE" in
  1) CFG_ARGS+=( --comparative ) ;;
  0) ;;
  *) echo "FATAL: COMPARATIVE is 1 (a block measured beside others) or 0, not '$COMPARATIVE'" >&2; exit 4 ;;
esac
# Forward ONLY what the caller actually set. Passing these unconditionally made the shell's
# defaults the real defaults and left the compiler's measured constants unreachable — the whole
# point of moving them into `flow_config.rs` was that the measurement travels with the value.
[ -n "${PL_TARGET_DENSITY:-}" ] && CFG_ARGS+=( --density "$PL_TARGET_DENSITY" )
[ -n "${HEUR_THRESH:-}" ]       && CFG_ARGS+=( --antenna-threshold "$HEUR_THRESH" )
[ -n "${DRT_ECO_ITERS:-}" ]     && CFG_ARGS+=( --eco-iters "$DRT_ECO_ITERS" )
[ -n "${SLEW_MARGIN:-}" ]       && CFG_ARGS+=( --slew-margin "$SLEW_MARGIN" )
[ -n "${PL_SLEW_MARGIN:-}" ]    && CFG_ARGS+=( --pl-slew-margin "$PL_SLEW_MARGIN" )
[ "${DIODES_BEFORE_RESIZE:-0}" = "1" ] && CFG_ARGS+=( --diodes-before-resize )
[ "${ECO_DIODE_FALLBACK:-0}" = "1" ]   && CFG_ARGS+=( --eco-diode-fallback )
[ "${POST_DIODE_UPSIZE:-0}" = "1" ]    && CFG_ARGS+=( --post-diode-upsize )
[ -n "${UPSIZE_MARGIN:-}" ]     && CFG_ARGS+=( --upsize-margin "$UPSIZE_MARGIN" )
[ "${UPSIZE_ALL_CORNERS:-0}" = "1" ]   && CFG_ARGS+=( --upsize-all-corners )
[ "${GLB_SKIP_PIN_SWAP:-0}" = "1" ]    && CFG_ARGS+=( --glb-skip-pin-swap )
[ -n "${ANTENNA:-}" ]           && CFG_ARGS+=( --antenna "$ANTENNA" )
[ -n "${SYNTH_STRATEGY:-}" ]    && CFG_ARGS+=( --synth-strategy "$SYNTH_STRATEGY" )
[ -n "$GRT_MET1_ADJUST" ]       && CFG_ARGS+=( --met1-penalty "$GRT_MET1_ADJUST" )
# CLOCK_MAX_WIRE / MAX_WIRE — the longest clock-input and signal wires left unbuffered, in um; unset = the
# recipe's (ClosureSettings::clock_max_wire_um, signal_max_wire_um). POST_ANTENNA_UPSIZE=1 upsizes again
# after global routing's antenna repair (ClosureSettings::post_antenna_upsize).
[ -n "${CLOCK_MAX_WIRE:-}" ]    && CFG_ARGS+=( --clock-max-wire "$CLOCK_MAX_WIRE" )
[ -n "${MAX_WIRE:-}" ]          && CFG_ARGS+=( --max-wire "$MAX_WIRE" )
[ "${POST_ANTENNA_UPSIZE:-0}" = "1" ]  && CFG_ARGS+=( --post-antenna-upsize )
[ "${CTS_FANOUT:-1}" = "0" ]           && CFG_ARGS+=( --no-cts-fanout )
[ "${DIODE_FANOUT:-1}" = "0" ]         && CFG_ARGS+=( --no-diode-fanout )
[ -n "${FANOUT_MARGIN:-}" ]     && CFG_ARGS+=( --fanout-margin "$FANOUT_MARGIN" )
[ -n "$GRT_ANT_ITERS" ]         && CFG_ARGS+=( --grt-antenna-iters "$GRT_ANT_ITERS" )
[ -n "$GRT_ANT_MARGIN" ]        && CFG_ARGS+=( --grt-antenna-margin "$GRT_ANT_MARGIN" )
[ -n "$PDN_VPITCH" ]            && CFG_ARGS+=( --pdn-vpitch "$PDN_VPITCH" )
[ -n "$HOLD_SLACK_MARGIN" ]     && CFG_ARGS+=( --hold-slack-margin "$HOLD_SLACK_MARGIN" )
[ -n "$GRT_OVERFLOW_ITERS" ]    && CFG_ARGS+=( --grt-overflow-iters "$GRT_OVERFLOW_ITERS" )
[ -n "$GRT_ADJUSTMENT" ]        && CFG_ARGS+=( --grt-adjustment "$GRT_ADJUSTMENT" )
[ -n "$DPL_PADDING" ]           && CFG_ARGS+=( --dpl-padding-per-side "$DPL_PADDING" )
[ -n "$SETUP_SLACK_MARGIN" ]    && CFG_ARGS+=( --setup-slack-margin "$SETUP_SLACK_MARGIN" )
# The antenna STRATEGY, which must reach the emitter or it is not in effect. This is the exact bug
# that already cost one experiment: `HEUR_DIODE=0` set a shell variable nothing forwarded, the
# compiler emitted RUN_HEURISTIC_DIODE_INSERTION=1 regardless, and a run believed to be diode-free
# was measured and reasoned about while carrying 106 diodes.
if [ "$HEUR_DIODE" = "0" ] || [ -n "$GRT_REPAIR" ]; then
  if [ -n "${ANTENNA:-}" ]; then
    echo "FATAL: ANTENNA=$ANTENNA and HEUR_DIODE/GRT_REPAIR each choose the antenna strategy — set one." >&2
    exit 4
  fi
  case "$HEUR_DIODE/$GRT_REPAIR" in
    1/1) ;;
    1/0) CFG_ARGS+=( --antenna heuristic+jog ) ;;
    0/|0/0) CFG_ARGS+=( --antenna jog-only ) ;;
    0/1)
      echo "FATAL: HEUR_DIODE=0 GRT_REPAIR=1 (the router's repair without the heuristic pass) is no" >&2
      echo "       AntennaStrategy; the compiler offers diodes+jog, heuristic+jog and jog-only." >&2
      exit 4 ;;
    *)
      echo "FATAL: HEUR_DIODE and GRT_REPAIR are 0 or 1, got HEUR_DIODE=$HEUR_DIODE GRT_REPAIR=$GRT_REPAIR." >&2
      exit 4 ;;
  esac
fi
# `ANTENNA_JOG=0` (diodes, no jog) has no representation in the compiler's AntennaStrategy, which
# is deliberate — the jog is byte-identical on a design with no antenna violations, so there is no
# reason to build a block without it. Refuse rather than accept a setting that would do nothing.
if [ "${ANTENNA_JOG:-1}" = "0" ]; then
  echo "FATAL: ANTENNA_JOG=0 is not expressible — AntennaStrategy is DiodesAndJog or JogOnly." >&2
  echo "       The jog ECO is a no-op when a design has no antenna violations, so there is no" >&2
  echo "       diodes-only mode. Use HEUR_DIODE=0 for jog-only." >&2
  exit 4
fi
# Knobs with no compiler form are refused, never accepted and dropped: the block would build with the compiler's
# value while its caller believed otherwise (see the knob block above for each reason).
if [ -n "$SYNTH_MAX_FANOUT" ]; then
  echo "FATAL: SYNTH_MAX_FANOUT would move the fanout limit signoff judges every net against" >&2
  echo "       (MAX_FANOUT_CONSTRAINT); the flow holds every net within it instead." >&2
  exit 4
fi
if [ -n "$RT_MIN_SIGNAL_LAYER" ]; then
  echo "FATAL: RT_MIN_SIGNAL_LAYER is not a recipe: met2 failed to route gpu_simt at FP_CORE_UTIL 25" >&2
  echo "       (GRT-0119). GRT_MET1_ADJUST is the met1 lever; an experiment can set" >&2
  echo "       OL_EXTRA_JSON='{\"RT_MIN_LAYER\": \"met2\"}' and record why." >&2
  exit 4
fi
if [ -n "${OL_EXTRA:-}" ]; then
  echo "FATAL: OL_EXTRA (raw config lines) is gone with the config heredoc; set OL_EXTRA_JSON to a JSON object." >&2
  exit 4
fi
if [ "$RESIZER_DESIGN/$RESIZER_TIMING/$QUIT_ON_HOLD" != "1/1/1" ]; then
  echo "FATAL: the compiler emits every resizer pass and QUIT_ON_HOLD_VIOLATIONS on; no recipe turns them off" >&2
  echo "       (RESIZER_DESIGN=$RESIZER_DESIGN RESIZER_TIMING=$RESIZER_TIMING QUIT_ON_HOLD=$QUIT_ON_HOLD)." >&2
  exit 4
fi

# Use the PREBUILT emitter when it exists, and only fall back to building it. A physical-design
# flow must not depend on the workspace's compile state at run time: this box runs several agent
# sessions against one tree, and a crate broken mid-refactor by someone else would otherwise abort
# a multi-hour route. (Observed twice today.) Build it once, deliberately, before a long queue:
#   cargo build -p logicaffeine-compile --example emit_flow_config
# It is where cargo builds it: under CARGO_TARGET_DIR when that is set, the checkout's target/ otherwise.
EMIT="${CARGO_TARGET_DIR:-$ROOT/target}/debug/examples/emit_flow_config"
if [ ! -x "$EMIT" ]; then
  echo "== building the flow-config emitter (one-off) =="
  cargo build -q -p logicaffeine-compile --example emit_flow_config \
    || { echo "FATAL: cannot build emit_flow_config; build it when the tree compiles" >&2; exit 4; }
fi
# The prebuilt emitter must not be OLDER than the compiler source it came from.
#
# This script uses the PREBUILT binary on purpose, so a peer breaking the workspace mid-refactor
# cannot abort a multi-hour route. The cost of that choice is that a change to
# `codegen_sva/flow_config.rs` reaches NO RUN until somebody rebuilds — the knowledge sits in the
# file looking authoritative and the silicon never sees it.
#
# MEASURED: the binary was a full day stale (built 09-21 08:48) while flow_config.rs had been
# edited the same afternoon. Two newly added levers (GRT_ANT_MARGIN / GRT_ANT_ITERS) emitted
# nothing at all, and every run that day used the previous day's compiled defaults. Only the
# values passed explicitly on the command line were ever in effect. That is the same failure
# family as the three `lappend -flag` bugs in the vendored Tcl: a setting that exists, reads as
# configured, and reaches no tool.
for _srcf in "$ROOT/crates/logicaffeine_compile/src/codegen_sva/flow_config.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/pdk_target.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/signoff_blocks.rs" \
             "$ROOT/crates/logicaffeine_compile/examples/emit_flow_config.rs"; do
  if [ -f "$_srcf" ] && [ "$_srcf" -nt "$EMIT" ]; then
    echo "FATAL: $(basename "$_srcf") is NEWER than the prebuilt flow-config emitter." >&2
    echo "         source: $(date -r "$_srcf" '+%F %T')" >&2
    echo "         binary: $(date -r "$EMIT" '+%F %T')" >&2
    echo "       The emitter is what turns compiler knowledge into a config.json, so this run" >&2
    echo "       would silently use the OLD values and your change would reach no silicon." >&2
    echo "       Rebuild it deliberately, then re-launch:" >&2
    echo "         cargo build -p logicaffeine-compile --example emit_flow_config" >&2
    exit 4
  fi
done
if ! "$EMIT" "${CFG_ARGS[@]}" > "$OL/designs/$DES/config.json"; then
  echo "FATAL: the compiler could not emit a flow config for $BLOCK (its reason is above)" >&2
  exit 4
fi
# Fail CLOSED: an empty or truncated config would otherwise be handed to OpenLane, which would
# cheerfully build the block with its own defaults and produce a result nobody configured.
if ! grep -q '"DESIGN_NAME"' "$OL/designs/$DES/config.json"; then
  echo "FATAL: emitted flow config is not a config:" >&2
  cat "$OL/designs/$DES/config.json" >&2
  exit 4
fi
# Block-specific floorplan/RTL knobs this script still owns (die area for pad-limited blocks, the
# extra antenna levers), merged in rather than duplicated in the compiler.
python3 - "$OL/designs/$DES/config.json" "$FP_CORE_UTIL" <<PYEOF
import json,sys,os
p=sys.argv[1]; d=json.load(open(p))
extra=os.environ.get("OL_EXTRA_JSON","")
if extra:
    d.update(json.loads(extra))
for k,v in (("FP_CORE_UTIL", sys.argv[2]),):
    if v: d[k]=float(v) if "." in v else int(v)
if os.environ.get("OL_DIE_AREA"):
    d["FP_SIZING"]="absolute"; d["DIE_AREA"]=os.environ["OL_DIE_AREA"]
json.dump(d, open(p,"w"), indent=2); open(p,"a").write("\n")
PYEOF

# Snapshot the run dirs BEFORE the flow so we record the EXACT run THIS invocation produced. The old
# `ls -td | head -1` records the newest-MTIME dir, which RACES: a concurrent run, or a touched mtime on
# an older run, silently makes the recorder distill the WRONG run's reports. That once stamped a stale
# CLEAN verdict (a 30ns run's +13.51 setup slack) onto a re-sign that actually FAILED setup (6ns, WNS
# -4.36) — a false signoff. The new-dir diff below is race-free.
# `|| true`: a design being taped out for the FIRST time has no runs/ directory, so `ls` exits
# nonzero — and under `set -o pipefail` + `set -e` that killed the whole script SILENTLY (exit 2,
# no output at all). Every new block was unlaunchable; only blocks with a run history worked.
BEFORE_RUNS=$(ls -1d "$OL/designs/$DES/runs/"*/ 2>/dev/null | sort || true)
# PRE-FLIGHT: refuse to start when another OpenLane flow could OOM this one.
#
# MEASURED, from the kernel's own log on 2026-09-22:
#   Out of memory: Killed process 1293216 (openroad) total-vm:107176476kB anon-rss:98749324kB
# One openroad reached 98.7 GB resident on a 125 GB box and the kernel took it. That is why
# three runs today "failed" with an EMPTY .errors file and a log that simply stops mid-step:
# a SIGKILL leaves no diagnostic behind, so the failure is indistinguishable from a crash.
#
# The bound is RESIDENT MEMORY, not core count. ROUTING_CORES pins threads and does nothing
# about RSS, so "only two flows" was a policy calibrated on the wrong variable — it was never
# safe, it just had not failed yet. A 457k-cell design passed 60 GB at 17 minutes and was
# still climbing; a second flow entering detailed routing behind it loses BOTH.
OL_MIN_FREE_GB="${OL_MIN_FREE_GB:-80}"
# `|| true` on BOTH: under `set -euo pipefail` a pipeline whose grep matches NOTHING exits 1,
# the assignment inherits that status, and set -e kills the script SILENTLY — no output at all,
# exit 1, before the first echo. That is precisely the idle-box case this guard exists to allow,
# so without the guard-on-the-guard the memory check refuses every run when nothing else is
# running. Measured: it did exactly that on the first launch after it was installed.
_free_gb=$(free -g | awk '/^Mem:/{print $7}' || true)
_peak_gb=$(ps -eo rss,args 2>/dev/null | grep '[o]penroad' | awk '{if($1>m)m=$1}END{printf "%.0f", m/1048576}' || true)
_peak_gb=${_peak_gb:-0}
if [ "${_free_gb:-0}" -lt "$OL_MIN_FREE_GB" ]; then
  echo "FATAL: only ${_free_gb}GB available; this flow needs headroom up to ~99GB at peak." >&2
  echo "       Another openroad is already holding ${_peak_gb:-0}GB." >&2
  echo "       An OOM kill leaves an EMPTY .errors file and a log that stops mid-step, so the" >&2
  echo "       run looks like a mystery crash and costs the full routing time to rediscover." >&2
  echo "       Wait for the other flow, or set OL_MIN_FREE_GB to override deliberately." >&2
  exit 4
fi
echo "== OpenLane / Sky130 RTL-to-GDSII: $BLOCK =="
# The flow runs the repo's OpenLane changes, laid over the fetched OpenLane every run (scripts/openlane-overlay) —
# every run but a control, which runs OpenLane as released.
if [ "$OL_STOCK" != 1 ]; then
  bash "$ROOT/scripts/openlane-overlay/apply.sh" "$OL"
fi
# A bench run and a control flow in the bench's image: the image at the overlay's base with its OpenROAD rebuilt with
# the one fix that keeps repair_timing off a looped global route (scripts/openroad/build-openroad.sh), so neither
# tree's runs die where the other's would not. A tree fetched as an archive has no checkout to read its image from,
# so the run names it; every other run takes the image its tree's checkout names.
if [ "$OL_STOCK" = 1 ] || [ "$OL_BENCH" = 1 ]; then
  OL_IMAGE=$(bash "$ROOT/scripts/openroad/build-openroad.sh" --name)
  if ! sg docker -c "bash $ROOT/scripts/openroad/build-openroad.sh --built" >/dev/null 2>&1; then
    echo "FATAL: the bench's OpenLane image $OL_IMAGE is not built (scripts/openroad/build-openroad.sh)" >&2
    exit 1
  fi
fi
( cd "$OL" && sg docker -c "make quick_run QUICK_RUN_DESIGN=$DES${OL_IMAGE:+ OPENLANE_IMAGE_NAME=$OL_IMAGE}" )
RUN=$(comm -13 <(printf '%s\n' "$BEFORE_RUNS") <(ls -1d "$OL/designs/$DES/runs/"*/ 2>/dev/null | sort || true) | tail -1)
[ -n "$RUN" ] || RUN=$(ls -td "$OL/designs/$DES/runs/"*/ | head -1)   # fallback: a re-run into an existing dir
# A run that flowed in an image of its own (a bench run's, a control's) names it beside itself — the name
# build-openroad.sh prints — before anything reads the run: the IR replay launches it, and the recorder records it.
# A run in OpenLane as released names none.
if [ -n "$OL_IMAGE" ]; then printf '%s\n' "$OL_IMAGE" > "${RUN%/}/openlane_image"; fi
# Integrity guard: the recorded run MUST carry the clock we asked for. A mismatch means we grabbed the
# wrong run — fail LOUD rather than distill a stale run's (possibly clean) numbers into the signoff JSON.
REC_CLK=$(sed -n 's/.*CLOCK_PERIOD)[^0-9.]*\([0-9.]\+\).*/\1/p' "$RUN"config.tcl 2>/dev/null | head -1)
if [ -n "$REC_CLK" ] && ! python3 -c "import sys;sys.exit(0 if abs(float('$REC_CLK')-float('$PERIOD'))<1e-6 else 1)" 2>/dev/null; then
  echo "FATAL: recorded run $RUN has CLOCK_PERIOD=$REC_CLK, not the requested ${PERIOD}ns — refusing to record a mismatched run." >&2
  exit 3
fi
# Distill the FULL signoff evidence (top-line gates + deep: setup/hold slack, VIOLATED paths, XOR,
# antenna violators, LVS-clean verdict, IR-drop, multi-corner) from the run's reports into the
# checked-in artifact. record-sky130-signoff.py is fail-closed and exits non-zero unless every gate
# passes, so a regression on any gate makes this command (and the lock reading the JSON) go RED.
# OUT_JSON lets a second block record its signoff alongside the first without clobbering it (the
# default reproduces the original soft-core path exactly).
# ANCHOR THE IR ANALYSIS BEFORE RECORDING IT. OpenLane runs `analyze_power_grid` with no `-vsrc`
# unless VSRC_LOC_FILES is set, so OpenROAD picks a source node by itself and the reported drop
# depends on where that arbitrary node landed. The numbers that produced were not physically
# ordered — systolic_pipe16_realbist (819k cells) read 15.05 mV while fabric_horner_seq (190k)
# read 75.77 — and `signoff_clean` GATES on the value (< 90 mV). The vsrc cannot be built before
# the flow, because it is derived from the FINAL die size and the run's own FP_PDN_CORE_RING, so
# it is done here, between the flow and the recorder, replaying OpenLane's own irdrop step.
#
# Non-fatal on purpose: a block whose PDN this cannot model should still record, with its
# unanchored number and a loud line saying so, rather than losing the whole run. A bench run and a
# control stop instead: the bench compares their verdicts, and one whose IR gate a source node
# OpenROAD chose on its own decided compares with nothing.
if [ "$ANCHOR_IR" = 1 ]; then
  if ! bash "$ROOT/scripts/anchor-ir-drop.sh" "$RUN" 2>&1 | grep -E "anchoring|ANCHORED|FATAL"; then
    if [ "$OL_STOCK" = 1 ] || [ "$OL_BENCH" = 1 ]; then
      echo "FATAL: IR anchoring failed for $RUN — a bench run and a control record no verdict whose IR gate a" >&2
      echo "       source node OpenROAD chose on its own decided." >&2
      exit 5
    fi
    echo "WARNING: IR anchoring failed for $RUN — the recorded ir_drop_worst_mv is UNANCHORED," >&2
    echo "         i.e. computed against a source node OpenROAD chose on its own." >&2
  fi
fi
# A block already recorded in SOME ledger must be UPDATED THERE. Defaulting to
# sky130_signoff.json wrote a SECOND entry for a block first recorded elsewhere, and
# systolic_os_pipe8_realbist ended up reading signoff_clean=TRUE in one ledger and FALSE in
# another — which answer you got depended on which file you opened. `ledger_uniqueness_lock`
# now forbids the state; this stops the flow from creating it.
if [ -z "${OUT_JSON:-}" ]; then
  OUT_JSON=$(python3 - "$ROOT" "$SIL" "$BLOCK" <<'PYLEDGER'
import json, os, sys
root, sil, block = sys.argv[1], sys.argv[2], sys.argv[3]
for led in json.load(open(os.path.join(sil, "sky130_ledgers.json")))["ledgers"]:
    p = os.path.join(root, led["path"])
    if os.path.exists(p) and block in (json.load(open(p)).get("blocks") or {}):
        print(p); break
else:
    print(os.path.join(sil, "sky130_signoff.json"))
PYLEDGER
)
fi
python3 "$ROOT/scripts/record-sky130-signoff.py" "$RUN" "$BLOCK" "$OUT_JSON"
}
# The call and the exit share ONE line. When the function returns, bash reads its NEXT command from
# the byte offset where it stopped; with the exit on a line of its own, a file whose length changed
# mid-run would have that offset land inside the function text and execute stray lines after the
# flow. On one line, the exit is parsed before the call runs, and nothing after it is ever read.
logos_openlane_run_main "$@"; exit $?
