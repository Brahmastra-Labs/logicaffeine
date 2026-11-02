#!/usr/bin/env bash
# Run the EFABLESS GOLDEN signoff DRC decks (the ones mpw_precheck uses to ACCEPT a shuttle
# submission) on a taped-out block's GDS, and record the result in benchmarks/silicon/golden_precheck.json.
#
# This closes the honest "open deck != foundry deck" gap for DRC: OpenLane's built-in Magic DRC is
# necessary but not the golden deck. Here we run KLayout with `${PDK}_mr.drc` (the full foundry/
# efabless maximal-rule deck, FEOL+BEOL) plus the golden `zeroarea` geometry check — the same decks
# mpw_precheck bundles — and count violations exactly as the precheck does (`<item>` entries).
#
#   scripts/run-golden-precheck-drc.sh <top_cell> <path/to/block.gds> [key]
#
# The verdict is filed under <key>, which defaults to the top cell. Every Caravel wrapper's top cell is
# `user_project_wrapper`, so a wrapper passes its project's key (codegen_sva::caravel_project) to keep
# one project's verdict from overwriting another's. The KLayout reports are written beside the GDS they
# judge (`<gds stem>.golden_drc.xml`, `<gds stem>.zeroarea.xml`), so the evidence stays with the run.
#
# Requires: the OpenLane docker image (bundles KLayout) + the efabless precheck decks under
# tools/mpw_precheck (fetched, not vendored). Fail-CLOSED: nonzero exit unless every golden deck is 0.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
BLOCK="${1:?usage: run-golden-precheck-drc.sh <top_cell> <gds> [key]}"
GDS="${2:?path to the block GDS file}"
KEY="${3:-$BLOCK}"
IMG="ghcr.io/the-openroad-project/openlane:latest"
# The golden deck is per-PDK. efabless ships sky130A/sky130B/gf180mcuC/gf180mcuD maximal-rule decks
# under the same naming convention, so the deck follows $PDK rather than being wired to one process.
PDK="${PDK:-sky130A}"
MR="tools/mpw_precheck/checks/tech-files/${PDK}_mr.drc"
ZA="tools/mpw_precheck/checks/drc_checks/klayout/zeroarea.rb.drc"
# FEOL/BEOL are switches of the SKY130 maximal-rule deck; the gf180 decks do not take them, and
# gf180 puts metal density in its own `gf180mcu_density.lydrc` instead. Select accordingly rather
# than passing sky130 switches to a deck that ignores them (which would silently check less).
case "$PDK" in
  sky130*) MR_ARGS=(-rd feol=true -rd beol=true) ;;
  *)       MR_ARGS=() ;;
esac
# GOLDEN_OUT files the verdict in another ledger — a staging copy a batch of re-signs lands from at once.
OUT="${GOLDEN_OUT:-$ROOT/benchmarks/silicon/golden_precheck.json}"
GDS="${GDS#$ROOT/}"                                        # normalize to repo-relative
[ -f "$ROOT/$GDS" ] || { echo "no GDS at $GDS"; exit 1; }
[ -f "$ROOT/$MR" ] || { echo "efabless precheck decks absent (tools/mpw_precheck) — fetch them first"; exit 1; }

GXML="$ROOT/${GDS%.gds}.golden_drc.xml"
ZXML="$ROOT/${GDS%.gds}.zeroarea.xml"

echo "== golden precheck DRC: $KEY (top cell $BLOCK) =="
# Run a KLayout deck under docker; verify the RDB report actually landed (a cold docker start can
# occasionally not flush before we read it), retrying once. The host path is inside $ROOT, which the
# container sees as /work.
klayout_deck () {  # <deck-rel> <host-xml> <extra -rd args...>
  local deck="$1" hostxml="$2"; shift 2
  local try
  for try in 1 2; do
    rm -f "$hostxml" 2>/dev/null || true
    sg docker -c "docker run --rm --user $(id -u):$(id -g) -v $ROOT:/work $IMG klayout -b -r /work/$deck -rd input=/work/$GDS -rd topcell=$BLOCK -rd report=/work/${hostxml#$ROOT/} $*" >/dev/null 2>&1 || true
    [ -s "$hostxml" ] && return 0
    echo "  (report $(basename "$hostxml") not written on try $try; retrying)"
  done
  return 1
}

klayout_deck "$MR" "$GXML" ${MR_ARGS[@]+"${MR_ARGS[@]}"}
klayout_deck "$ZA" "$ZXML"

PDK="$PDK" python3 "$ROOT/scripts/record-golden-precheck.py" "$KEY" "$GXML" "$ZXML" "$OUT" "$ROOT/$GDS"
