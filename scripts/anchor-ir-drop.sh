#!/usr/bin/env bash
# Re-run a finished run's IR-drop analysis with an ANCHORED source model, by replaying OpenLane's
# OWN irdrop step against that run's saved environment.
#
# WHY. `analyze_power_grid` needs to know where current ENTERS the grid. With `VSRC_LOC_FILES`
# undefined, OpenLane calls it with no `-vsrc`, OpenROAD picks a source node by itself, and the
# reported drop then depends on where that arbitrary node happened to land. The numbers were not
# physically ordered: `systolic_pipe16_realbist` (819,032 cells, 4.05 mm^2) recorded 15.05 mV
# while `fabric_horner_seq` (189,531 cells, 1.09 mm^2) recorded 75.77 mV — a fifth the drop for
# four times the cells. Every `ir_drop_worst_mv` in every ledger was computed that way, and
# `signoff_clean` GATES on it (< 90 mV), so an unspecified choice inside the tool was deciding a
# signoff gate.
#
# WHY IT REPLAYS RATHER THAN REBUILDS. A hand-written `read_lef`/`read_def`/`read_spef` +
# `analyze_power_grid` script looks equivalent and is not: without the liberty it emits a report
# with a header row and no instances (52 bytes, scored as -1.0), and even with the liberty read it
# reported a flat 0.00 mV because the flow's own RC/activity setup was missing. A number produced
# by a subtly different analysis is worse than no number. So this sources the run's `config.tcl` —
# which carries every resolved variable, `CURRENT_ODB`, `CURRENT_SPEF`, `MERGED_LEF`, the LIB_*
# set — overrides only `VSRC_LOC_FILES`, and sources the flow's own `openroad/irdrop.tcl`.
#
# WHERE IT REPLAYS. Where the run flowed: a run is `<tree>/designs/<design>/runs/<run>` in an OpenLane tree, the
# replay mounts that tree as the flow did and sources that tree's irdrop step, and launches the image the run names
# beside itself (`openlane_image`, written by its launcher: the bench's image, as scripts/openroad/build-openroad.sh
# names it), or OpenLane as released for a run in tools/OpenLane that names none. A run anywhere else that names no
# image (the bench's trees among them, whose launcher always names one) has no image to replay in, and is refused
# before any container starts, as is a run naming another image, or an image not on the machine.
#
# The source MODEL is read from the run's own config, never chosen by hand: `FP_PDN_CORE_RING 1`
# means the block is fed at its boundary (perimeter sources); 0 means the parent's straps run
# across it (area sources on the strap pitch). Getting that backwards is not a small error —
# `fabric_horner_seq` has no ring, and modelling it as perimeter-fed reported 230.59 mV because
# every instance in the middle of the die was made to draw its current from the edge.
#
#   scripts/anchor-ir-drop.sh <run_dir>
set -euo pipefail
RUN="${1:?usage: anchor-ir-drop.sh <run_dir>}"; RUN="${RUN%/}"
# Absolute, or the container-path derivation below silently fails to strip the OpenLane prefix and
# builds a path that does not exist inside the image.
RUN="$(cd "$RUN" && pwd)"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESIGNS="$(dirname "$(dirname "$(dirname "$RUN")")")"
if [ "$(basename "$(dirname "$RUN")")" != runs ] || [ "$(basename "$DESIGNS")" != designs ]; then
  echo "FATAL: $RUN is not a run of an OpenLane tree (<tree>/designs/<design>/runs/<run>)" >&2; exit 2
fi
OL="$(dirname "$DESIGNS")"
[ -f "$OL/scripts/openroad/irdrop.tcl" ] \
  || { echo "FATAL: $OL is not an OpenLane tree (no scripts/openroad/irdrop.tcl to replay)" >&2; exit 2; }
# One tree however it is reached: a copy of the repository whose tools/ links the checkout's names the same tree.
same_tree() { local a b; a=$(cd "$1" 2>/dev/null && pwd -P) && b=$(cd "$2" 2>/dev/null && pwd -P) && [ "$a" = "$b" ]; }
ARCH=$(sg docker -c "docker version --format '{{.Server.Arch}}'") \
  || { echo "FATAL: docker names no architecture to launch an image for" >&2; exit 2; }
if [ -f "$RUN/openlane_image" ]; then
  NAMED=$(cat "$RUN/openlane_image")
  IMAGE=$(bash "$ROOT/scripts/openroad/build-openroad.sh" --name)
  if [ "$NAMED" != "$IMAGE" ]; then
    echo "FATAL: $RUN/openlane_image names '$NAMED', which is no image build-openroad.sh names ($IMAGE)" >&2; exit 2
  fi
  IMAGE="$IMAGE-$ARCH"
  sg docker -c "docker image inspect $IMAGE" >/dev/null 2>&1 \
    || { echo "FATAL: $IMAGE, the image $RUN flowed in, is not on this machine" >&2; exit 2; }
elif same_tree "$OL" "$ROOT/tools/OpenLane"; then
  IMAGE=ghcr.io/the-openroad-project/openlane:ff5509f65b17bfa4068d5336495ab1718987ff69-$ARCH
else
  echo "FATAL: $RUN names no image (openlane_image), and only a run in tools/OpenLane flows in OpenLane as released" >&2
  exit 2
fi
CFG="$RUN/config.tcl"
[ -f "$CFG" ] || { echo "FATAL: no config.tcl in $RUN" >&2; exit 2; }
tclval() { sed -n "s/.*set ::env($1) \"\?\([^\"]*\)\"\?.*/\1/p" "$CFG" | head -1; }

DESIGN=$(tclval DESIGN_NAME); DIE=$(tclval DIE_AREA); RING=$(tclval FP_PDN_CORE_RING)
PITCH=$(tclval FP_PDN_VPITCH); PDKN=$(tclval PDK)
VDD=$(tclval VDD_NETS); [ -n "$VDD" ] || VDD=VPWR
GND=$(tclval GND_NETS); [ -n "$GND" ] || GND=VGND
for v in DESIGN DIE PITCH; do [ -n "${!v}" ] || { echo "FATAL: no $v in $CFG" >&2; exit 2; }; done

read -r LLX LLY URX URY <<<"$DIE"
W=$(python3 -c "print(f'{float('$URX')-float('$LLX'):.3f}')")
H=$(python3 -c "print(f'{float('$URY')-float('$LLY'):.3f}')")
MODE=area; [ "$RING" = "1" ] && MODE=ring
echo "== anchoring IR for $DESIGN ($(basename "$RUN")) =="
echo "   die ${W} x ${H} um  pitch ${PITCH} um  FP_PDN_CORE_RING=${RING} -> ${MODE} sources"

# Paths INSIDE the container: the run dir is under its tree $OL, which mounts at /openlane.
REL="${RUN#"$OL"/}"
CRUN="/openlane/$REL"
TMP="$RUN/tmp"; mkdir -p "$TMP"
python3 "$ROOT/scripts/gen-vsrc-loc.py" "$TMP/vsrc_$VDD.loc" "$W" "$H" "$PITCH" 1.8 "$MODE" >/dev/null
python3 "$ROOT/scripts/gen-vsrc-loc.py" "$TMP/vsrc_$GND.loc" "$W" "$H" "$PITCH" 0.0 "$MODE" >/dev/null
echo "   $(wc -l < "$TMP/vsrc_$VDD.loc") source nodes per net"

# The recorder takes the LEXICALLY FIRST `irdrop-VPWR.rpt`, so an unindexed name would never be
# read ("62-" sorts before "i"). Supersede the file it actually reads, keeping the unanchored
# original beside it as evidence of what changed rather than destroying it.
FIRST=$(ls "$RUN"/reports/signoff/*irdrop-"$VDD".rpt 2>/dev/null | sort | head -1)
[ -n "$FIRST" ] || { echo "FATAL: $RUN has no irdrop report to supersede" >&2; exit 2; }
# `irdrop.tcl` writes `$_tmp_save_rpt_prefix-$net.rpt`, so the prefix must keep the `-irdrop`
# part. Stripping it too (leaving `34`) sends the output to `34-VPWR.rpt`, which nothing reads —
# and the analysis then LOOKS like it ran, because the verification below would re-read the
# untouched original and print its number as though it were new.
PREFIX_HOST="${FIRST%-$VDD.rpt}"
PREFIX_CT="$CRUN/reports/signoff/$(basename "$PREFIX_HOST")"
for n in "$VDD" "$GND"; do
  [ -f "$PREFIX_HOST-$n.rpt" ] && [ ! -f "$PREFIX_HOST-$n.rpt.unanchored" ] \
    && cp "$PREFIX_HOST-$n.rpt" "$PREFIX_HOST-$n.rpt.unanchored"
done
# Proof-of-write. Without this an outfile that lands somewhere else is indistinguishable from a
# successful re-analysis, because the check below would read the untouched original and report its
# value as the anchored one. That happened: seven blocks came back with numbers identical to their
# ledger entries, and they were identical because nothing had been rewritten.
BEFORE_STAMP=$(stat -c %Y "$PREFIX_HOST-$VDD.rpt" 2>/dev/null || echo 0)

cat > "$TMP/anchor_ir.tcl" <<TCL
source $CRUN/config.tcl
set ::env(SCRIPTS_DIR) /openlane/scripts
set ::env(_tmp_save_rpt_prefix) $PREFIX_CT
set ::env(VSRC_LOC_FILES) "$VDD $CRUN/tmp/vsrc_$VDD.loc $GND $CRUN/tmp/vsrc_$GND.loc"
source /openlane/scripts/openroad/irdrop.tcl
TCL

sg docker -c "docker run --rm \
  -v $OL:/openlane -v $OL/designs:/openlane/install \
  -v $HOME:$HOME -v ${PDK_ROOT:-$HOME/.ciel}:${PDK_ROOT:-$HOME/.ciel} \
  -e PDK_ROOT=${PDK_ROOT:-$HOME/.ciel} -e PDK=$PDKN -e STD_CELL_LIBRARY=$(tclval STD_CELL_LIBRARY) \
  -w /openlane $IMAGE \
  openroad -exit $CRUN/tmp/anchor_ir.tcl" 2>&1 | grep -vE "PSM-0030" | tail -8

AFTER_STAMP=$(stat -c %Y "$PREFIX_HOST-$VDD.rpt" 2>/dev/null || echo 0)
if [ "$AFTER_STAMP" = "$BEFORE_STAMP" ]; then
  echo "   FATAL: $PREFIX_HOST-$VDD.rpt was NOT rewritten — the analysis ran but its outfile went" >&2
  echo "          elsewhere. Refusing to report the stale file's number as an anchored result." >&2
  exit 3
fi
python3 - "$PREFIX_HOST-$VDD.rpt" <<'PY'
import csv, sys
v = []
for r in csv.reader(open(sys.argv[1])):
    try: v.append(float(r[-1]))
    except (ValueError, IndexError): pass
if not v:
    sys.exit("   FATAL: anchored report carries no instance voltages — analysis did not run")
print(f"   ANCHORED: {len(v)} instances, worst IR drop {(1.8 - min(v)) * 1000:.2f} mV")
PY
