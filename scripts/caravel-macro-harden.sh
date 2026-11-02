#!/usr/bin/env bash
# caravel-macro-harden.sh — harden a Caravel project's USER MACRO, which `scripts/caravel-harden.sh` then
# places as `mprj` inside `user_project_wrapper`.
#
#   scripts/caravel-macro-harden.sh [project]      # a key of codegen_sva::caravel_project (default: dram)
#
# WHY THIS EXISTS. The wrapper's harden became reproducible-from-the-repo when caravel-harden.sh was
# written; the MACRO it wraps did not. Its OpenLane config lived only in the gitignored
# `tools/OpenLane/designs/logos_upm/config.json`, hand-maintained, so the innermost artifact of the
# whole shuttle submission was the one thing a clean checkout could not rebuild. This closes that.
#
# ORDER MATTERS. The wrapper embeds this macro's .lef/.gds/.lib, so the macro is hardened FIRST and
# the wrapper re-hardened around the result. Running the wrapper against a stale macro silently
# signs off geometry that no longer exists.
#
# Every input is staged from the repo on every run: the macro RTL from benchmarks/silicon, and the pin
# order generated from the harness's fixed wrapper DEF by scripts/gen-caravel-macro-pinorder.py — it
# MUST mirror the wrapper DEF, or the wrapper's Wishbone nets route ~2.4 mm and come back as antenna
# violators.
set -euo pipefail

# THE WHOLE BODY IS ONE FUNCTION, called on the last line: a macro harden runs for hours and bash reads a plain
# script incrementally, so an edit under a running harden would resume its shell inside different text (openlane-run.sh).
logos_caravel_macro_harden_main() {
cd "$(dirname "$0")/.."
ROOT=$(pwd)
OL="$ROOT/tools/OpenLane"
PROJECT="${1:-dram}"
SCAFFOLD="$ROOT/tools/caravel_user_project"
WRAPPER_DEF="$SCAFFOLD/openlane/user_project_wrapper/fixed_dont_change/user_project_wrapper.def"

[ -d "$OL" ] || { echo "OpenLane not installed at $OL"; exit 1; }
[ -f "$WRAPPER_DEF" ] || { echo "no Caravel wrapper DEF at $WRAPPER_DEF (fetch tools/caravel_user_project)"; exit 1; }

# THE CONFIG COMES FROM THE COMPILER, like every other block's. This script used to write its own
# config.json, so the innermost artifact of the shuttle was the one block none of the measured
# closure settings reached: re-read at every corner on 2026-09-23 its latest run carried 33
# slow-corner max-slew violations, built without the calibrated estimator or the slew recipe.
# `--vehicle-macro` carries the vehicle's part — die, harness power nets, the routing ceiling its grid
# leaves it (`--macro-grid`: its own grid under the wrapper's, or fed by the wrapper's), the pin order,
# the hold margin (see codegen_sva::flow_config::VehicleMacro).
# The project itself — macro, die, placement, clock, designs, ledgers — is codegen_sva::caravel_project.
EMIT="$ROOT/target/debug/examples/emit_flow_config"
[ -x "$EMIT" ] || cargo build -q -p logicaffeine-compile --example emit_flow_config
for _srcf in "$ROOT/crates/logicaffeine_compile/src/codegen_sva/flow_config.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/pdk_target.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/signoff_blocks.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/caravel_project.rs" \
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
DES="$CP_MACRO_DESIGN"
D="$OL/designs/$DES"

# STAGE THE RTL FROM THE REPO, every time, and ONLY it: the flow compiles `dir::src/*.v`, so a file left
# behind in src/ by an earlier run would be synthesised too. This script previously only CHECKED that
# the RTL existed there, so the harden input was whatever a previous run had left in a gitignored
# working directory: an RTL fix applied to benchmarks/silicon was re-hardened into a byte-identical
# macro because the OLD file was re-synthesised, and nothing downstream noticed, because every gate
# checked that artifacts EXISTED rather than that they matched their source.
mkdir -p "$D/src"
rm -f "$D/src/"*.v
install -m644 "$ROOT/benchmarks/silicon/$CP_MACRO.v" "$D/src/$CP_MACRO.v"
echo "== staged RTL from benchmarks/silicon (sha $(sha256sum "$D/src/$CP_MACRO.v" | cut -c1-12)) =="
python3 "$ROOT/scripts/gen-caravel-macro-pinorder.py" "$WRAPPER_DEF" "$D/src/$CP_MACRO.v" "$D/pin_order.cfg" "$CP_MACRO"

DENSITY_ARGS=()
[ "$CP_DENSITY" = default ] || DENSITY_ARGS=(--density "$CP_DENSITY")
MET1_ARGS=()
[ "$CP_MET1_PENALTY" = default ] || MET1_ARGS=(--met1-penalty "$CP_MET1_PENALTY")
GRT_ARGS=()
[ "$CP_GRT_OVERFLOW_ITERS" = default ] || GRT_ARGS+=(--grt-overflow-iters "$CP_GRT_OVERFLOW_ITERS")
[ "$CP_GRT_ADJUSTMENT" = default ] || GRT_ARGS+=(--grt-adjustment "$CP_GRT_ADJUSTMENT")
[ "$CP_GRT_ANT_MARGIN" = default ] || GRT_ARGS+=(--grt-antenna-margin "$CP_GRT_ANT_MARGIN")
[ "$CP_GRT_ANT_ITERS" = default ] || GRT_ARGS+=(--grt-antenna-iters "$CP_GRT_ANT_ITERS")
[ "$CP_ANTENNA_THRESHOLD" = default ] || GRT_ARGS+=(--antenna-threshold "$CP_ANTENNA_THRESHOLD")
[ "$CP_DPL_PADDING" = default ] || GRT_ARGS+=(--dpl-padding-per-side "$CP_DPL_PADDING")
[ "$CP_SETUP_MARGIN" = default ] || GRT_ARGS+=(--setup-slack-margin "$CP_SETUP_MARGIN")
"$EMIT" "$CP_MACRO" "$CP_CLOCK_PORT" "$CP_MACRO_PERIOD_NS" --pdk "${PDK:-sky130A}" --cores "${ROUTING_CORES:-4}" \
    --synth-strategy "$CP_SYNTH_STRATEGY" ${DENSITY_ARGS[@]+"${DENSITY_ARGS[@]}"} ${MET1_ARGS[@]+"${MET1_ARGS[@]}"} ${GRT_ARGS[@]+"${GRT_ARGS[@]}"} \
    --vehicle-macro "$CP_MACRO_DIE_W" "$CP_MACRO_DIE_H" --hold-margin "$CP_HOLD_MARGIN_NS" --macro-grid "$CP_MACRO_GRID" \
    --post-diode-upsize --upsize-all-corners --upsize-margin "$CP_UPSIZE_MARGIN" --post-antenna-upsize --eco-diode-fallback > "$D/config.json"
grep -q '"DESIGN_NAME"' "$D/config.json" || { echo "FATAL: the emitter produced no config" >&2; exit 4; }
# The macro's ports are timed against the harness's real minimum input delays (codegen_sva::flow_config::
# vehicle_macro_sdc), so its hold repair closes the port-to-flop paths the wrapper would see; a macro on user_clock2
# times the harness's bus on wb_clk_i, asynchronous to its own clock.
if grep -q '"BASE_SDC_FILE": "dir::vehicle_macro.sdc"' "$D/config.json"; then
  "$EMIT" --vehicle-macro-sdc "$CP_CLOCK_PORT" --pdk "${PDK:-sky130A}" > "$D/vehicle_macro.sdc"
fi

# Race-free run capture: record the run THIS invocation created, never "the newest mtime" — the same
# discipline openlane-run.sh uses, for the same reason (a touched mtime once stamped a clean verdict
# onto a run that had actually failed).
BEFORE=$(ls -1d "$D/runs/"*/ 2>/dev/null | sort || true)
echo "== Caravel user MACRO harden: $CP_MACRO (${CP_MACRO_DIE_W}x${CP_MACRO_DIE_H}, ${CP_MACRO_PERIOD_NS} ns) =="
# The flow runs the repo's OpenLane changes, laid over the fetched OpenLane every run (scripts/openlane-overlay).
bash "$ROOT/scripts/openlane-overlay/apply.sh"
( cd "$OL" && sg docker -c "make quick_run QUICK_RUN_DESIGN=$DES" )
RUN=$(comm -13 <(printf '%s\n' "$BEFORE") <(ls -1d "$D/runs/"*/ 2>/dev/null | sort) | tail -1)
[ -n "$RUN" ] || RUN=$(ls -td "$D/runs/"*/ | head -1)
RUN="${RUN%/}"
echo "== run: $RUN =="

GDS="$RUN/results/final/gds/$CP_MACRO.gds"
[ -f "$GDS" ] || { echo "FATAL: no macro GDSII streamed at $GDS" >&2; exit 2; }

# PROVENANCE, RECORDED — not inferred from names. An earlier version of this gate required every
# register the RTL declares to appear in the gate-level netlist; that is FALSE. Measured on
# dram_user_project: yosys preserves `cmd_word_r` (192 refs) and `cmd_valid_r` (4) while renaming
# `op_c`, `bank_c`, `row_c`, `issued_c` and `issue_count` out of existence entirely. The gate
# blocked a CORRECT build. Synthesis may rename anything; only a hash of the input is reliable.
#
# So record the hash of the RTL this run consumed, next to its artifacts. Downstream (and
# `caravel_bringup_lock::the_shipped_netlist_reflects_the_current_rtl`) compares it against the
# current source, which no amount of renaming can confuse.
NL="$RUN/results/final/verilog/gl/$CP_MACRO.v"
[ -f "$NL" ] || { echo "FATAL: no gate-level netlist at $NL" >&2; exit 2; }
RTL_SHA=$(sha256sum "$D/src/$CP_MACRO.v" | awk '{print $1}')
{ printf '# SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.\n'
  printf '# SPDX-License-Identifier: MIT\n'
  printf '# Hash of the RTL this macro was hardened from. Readers skip `#` lines.\n'
  printf '%s  %s.v\n' "$RTL_SHA" "$CP_MACRO"
} > "$RUN/logos_rtl.sha256"

# And the symptom itself: a source change that produces a BYTE-IDENTICAL rebuild. That is what
# happened when the src directory held a stale file — same GDS, same cell count, fresh timestamp,
# and every existence check downstream satisfied.
PREV=$(ls -1dt "$D/runs/"*/ 2>/dev/null | sed -n 2p)
if [ -n "$PREV" ] && [ -f "$PREV/logos_rtl.sha256" ] && [ -f "$PREV/results/final/gds/$CP_MACRO.gds" ]; then
  PREV_SHA=$(grep -v '^#' "$PREV/logos_rtl.sha256" | awk '{print $1}')
  if [ "$PREV_SHA" != "$RTL_SHA" ] && \
     [ "$(stat -c %s "$PREV/results/final/gds/$CP_MACRO.gds")" = "$(stat -c %s "$GDS")" ]; then
    echo "FATAL: the RTL hash changed since the previous run but the GDS is byte-identical." >&2
    echo "       A source change that rebuilds to the same bytes means the flow did not read the" >&2
    echo "       source you think it did. Check $D/src against benchmarks/silicon." >&2
    exit 2
  fi
fi
echo "== RTL provenance recorded: ${RTL_SHA:0:12} (cells: $(grep -c '^ *sky130_fd_sc_hd__' "$NL")) =="

# The macro is a hardened block like any other, so it is signed off like one before anything is built
# around it: IR anchored, the golden shuttle deck on its own GDS, and every gate recorded into the
# project's macro ledger. Both records are written whatever they say; a macro that misses any gate then
# stops here, and its views are never published to the wrapper.
bash "$ROOT/scripts/anchor-ir-drop.sh" "$RUN" 2>&1 | grep -E "anchoring|ANCHORED|FATAL" \
  || echo "WARNING: IR anchoring failed for $RUN — its ir_drop_worst_mv is UNANCHORED" >&2
GOLDEN_OK=1; bash "$ROOT/scripts/run-golden-precheck-drc.sh" "$CP_MACRO" "$GDS" || GOLDEN_OK=0
# MACRO_LEDGER_OUT and GOLDEN_OUT file the records in other ledgers — staging copies a batch of re-signs lands from at once.
SIGNOFF_OK=1; python3 "$ROOT/scripts/record-sky130-signoff.py" "$RUN" "$CP_MACRO" "${MACRO_LEDGER_OUT:-$ROOT/$CP_MACRO_LEDGER}" || SIGNOFF_OK=0
if [ "$GOLDEN_OK" != 1 ] || [ "$SIGNOFF_OK" != 1 ]; then
  echo "FATAL: $CP_MACRO is not signoff-clean (golden deck ok=$GOLDEN_OK, block gates ok=$SIGNOFF_OK);" >&2
  echo "       its views stay unpublished, so no wrapper is built around it." >&2
  exit 5
fi

# Publish the freshly hardened views where caravel-harden.sh reads them. Without this the wrapper
# would re-harden around the PREVIOUS macro and its signoff would describe geometry that is gone.
UPW="$OL/designs/$CP_WRAPPER_DESIGN/macro"
mkdir -p "$UPW"
for ext in gds lef lib v; do
  src="$RUN/results/final/$( [ "$ext" = v ] && echo verilog/gl || echo "$ext" )/$CP_MACRO.$ext"
  [ -f "$src" ] || { echo "FATAL: macro view missing: $src" >&2; exit 3; }
  install -D -m644 "$src" "$UPW/$CP_MACRO.$ext"
done
echo "== published macro views to designs/$CP_WRAPPER_DESIGN/macro/ =="
echo "   next: scripts/caravel-harden.sh $PROJECT   (wrapper around THIS macro)"
}
logos_caravel_macro_harden_main "$@"; exit $?
