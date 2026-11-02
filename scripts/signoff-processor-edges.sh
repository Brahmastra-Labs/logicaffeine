#!/usr/bin/env bash
# signoff-processor-edges.sh — take the two remaining processor-edge blocks to Sky130
# signoff, completing "every processor edge is manufacturable silicon":
#
#   XPU  coproc_core  — the programmable accelerator core (BigInt + CRDT), proven ==VM
#                       (coproc_core_silicon_lock), ECP5-routed. Needs Sky130.
#   DPU  dpu_ingest   — the sequential near-data merge-on-ingest tile (crdt_rtl), the join
#                       proven ==GCounter + order-invariant (dpu_silicon_lock). Needs Sky130.
#
# CPU (soft_core_scalar) and the NPU dataflow fabric (cgra_sumto) are ALREADY signoff-clean.
# Each run appends to benchmarks/silicon/sky130_signoff.json; sky130_signoff_lock.rs then
# enforces 0 DRC/LVS/antenna/routing on the new blocks. OpenLane is hours per block — this
# serializes them and refuses to start on top of any live OpenLane/cargo run (rule 11).
# NOT `set -e`: the two blocks are independent — one failing to close must not abort the
# other. openlane-run.sh is fail-closed (a run with any violation records NOTHING), so a
# failed block cannot dirty sky130_signoff.json or turn the lock RED.
set -uo pipefail
cd "$(dirname "$0")/.."

# --- collision guard: never run on top of the co-agent's flow (rule 11 / shared box) ---
#
# NOT `pgrep -f`. A `-f` pattern matches the FULL COMMAND LINE of every process, including the
# shell running this very script and any wrapper that mentions these names — so
# `pgrep -f "openlane-run.sh"` can match itself and report BUSY forever. That exact self-match
# once blocked this campaign for over an hour on a box that was completely idle, and the same
# trap has since produced a self-kill (exit 144) and a 25-minute wait loop that matched its own
# cmdline and never ran the job it was waiting for. The rule is written down in three comments
# elsewhere in this repo and it still keeps happening, because the reach for `pgrep -f` is
# faster than the recall.
#
# So: ask the authoritative source instead of the process table. An OpenLane flow IS a docker
# container — `docker ps` cannot match this script. `pgrep -x` matches the executable NAME
# exactly (never the arguments) and so cannot match a shell that merely mentions it.
_busy=""
if command -v docker >/dev/null 2>&1 && [ -n "$(docker ps -q 2>/dev/null)" ]; then
  _busy="a docker container (OpenLane flow)"
fi
if pgrep -x openroad >/dev/null 2>&1; then _busy="${_busy:+$_busy, }openroad"; fi
if pgrep -x cargo-nextest >/dev/null 2>&1; then _busy="${_busy:+$_busy, }cargo-nextest"; fi
if [ -n "$_busy" ]; then
  echo "REFUSING: $_busy is already running — wait for the box to free." >&2
  docker ps --format '  {{.ID}} {{.Command}}' 2>/dev/null >&2 || true
  exit 3
fi

# Both edge blocks are PAD-LIMITED (wide top-level I/O buses), so the auto-sized 35%-util
# floorplan — sized from cell count — is too small in perimeter to seat every pin. Give each
# an explicit die (calibrated from the infinichip: 350µm/side seated 386 pins ≈ 3.65µm/pin):
#   dpu_ingest ~293 pins, tiny logic -> 320µm/side (pin-limited, 4.4µm/pin).
#   coproc_core ~262 pins + a 64x64 combinational multiply core -> 700µm/side (room for both).
# Each clock period is the one `codegen_sva::signoff_blocks` holds for the block — derived from what
# its slow corner meets — asked of the prebuilt emitter, which refuses any other period anyway.
# coproc_core instantiates bigint_alu / bigint_mul / crdt_merge — all their .v must be fed.
EMIT=target/debug/examples/emit_flow_config
P_DPU="$("$EMIT" --registered-period dpu_ingest)" \
  || { echo "FATAL: dpu_ingest has no registered signoff period" >&2; exit 4; }
P_XPU="$("$EMIT" --registered-period coproc_core)" \
  || { echo "FATAL: coproc_core has no registered signoff period" >&2; exit 4; }

echo "== [1/2] DPU: dpu_ingest -> Sky130 (small, high-confidence first) =="
OL_DIE_AREA="0 0 320 320" scripts/openlane-run.sh dpu_ingest clk "$P_DPU" dpu_ingest.v
rc_dpu=$?
echo "== dpu_ingest signoff rc=$rc_dpu =="

echo "== [2/2] XPU: coproc_core (+ bigint_alu/bigint_mul/crdt_merge) -> Sky130 =="
OL_DIE_AREA="0 0 700 700" scripts/openlane-run.sh coproc_core clk "$P_XPU" coproc_core.v bigint_alu.v bigint_mul.v crdt_merge.v
rc_xpu=$?
echo "== coproc_core signoff rc=$rc_xpu =="

echo "== RESULTS: dpu_ingest rc=$rc_dpu  coproc_core rc=$rc_xpu =="
echo "   (fail-closed: only signoff-CLEAN blocks were recorded to sky130_signoff.json)"
# exit 0 only if BOTH landed; the chain verifies locks regardless (a partial win is still green).
[ "$rc_dpu" -eq 0 ] && [ "$rc_xpu" -eq 0 ]
