#!/usr/bin/env bash
# sweep-image.sh DIR TOP PART JOBS VARIANT... — synthesize DIR once (synth-image.sh), then place and route it once per
# VARIANT, JOBS at a time, each in DIR-<name> with its own pnr.recipe, the record image_record.py reads of how it was
# placed and routed. A VARIANT is name:seed:freq:placement, placement `chipdb` (the placer sees the chipdb's own timing,
# NEXTPNR_XC7_PLACE_HARD_TIMING=0) or `hard<W>` (the hard cells timed while placing, HeAP timing weight W). Every
# variant routes and reports with the hard cells timed (route-image.sh).
set -uo pipefail
S=$(dirname "$0"); D=$1; TOP=$2; PART=$3; JOBS=$4; shift 4
[ -s "$D/top.json" ] || bash "$S/synth-image.sh" "$D" "$TOP" || exit 1
run() {
  local name=$1 seed=$2 freq=$3 place=$4 d="$D-$1"
  mkdir -p "$d"; cp "$D/top.json" "$D/stat.txt" "$D/image.json" "$D"/*.xdc "$D"/*.v "$d/"
  local w=10 hard=false extra="--seed $seed --freq $freq"
  case "$place" in
    chipdb) export NEXTPNR_XC7_PLACE_HARD_TIMING=0 ;;
    hard*) w=${place#hard}; hard=true; unset NEXTPNR_XC7_PLACE_HARD_TIMING; extra="$extra --placer-heap-timingweight $w" ;;
    *) echo "$name: placement $place is neither chipdb nor hard<W>"; return 1 ;;
  esac
  echo "{\"placer\": \"heap\", \"placer_heap_timingweight\": $w, \"place_with_hard_timing\": $hard, \"router\": \"router2\", \"seed\": $seed, \"freq_mhz\": $freq}" > "$d/pnr.recipe"
  PNR_EXTRA="$extra" bash "$S/route-image.sh" "$d" "$TOP" "$PART" > "$d.pnr.log" 2>&1
  echo "$(date -u +%T) $name: $(grep -oE "Max frequency for clock +'coreclk': [0-9.]+ MHz" "$d/pnr.log" | tail -1) $(grep -c BITSTREAM "$d.pnr.log") bitstream"
}
for v in "$@"; do
  IFS=: read -r name seed freq place <<< "$v"
  while [ "$(jobs -rp | wc -l)" -ge "$JOBS" ]; do sleep 20; done
  run "$name" "$seed" "$freq" "$place" &
done
wait
echo "$(date -u +%T) sweep done"
