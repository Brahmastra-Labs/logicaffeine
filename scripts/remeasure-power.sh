#!/usr/bin/env bash
# remeasure-power — INDEPENDENTLY reproduce the LOGOS Sky130 power measurements that published
# figures quote, by re-running OpenSTA `report_power` on the saved post-route gate netlist +
# extracted parasitics (SPEF) + timing constraints (SDC) + typical-corner Liberty, OUTSIDE the
# OpenLane flow. Same commands and the same default switching activity the signoff used, so a matching
# Total proves the number is a real, reproducible measurement — not a flow artifact or a hand-figure.
#
#   scripts/remeasure-power.sh
#
# Prints, per design, the re-measured Total (and Combinational) power next to the expected value.
# Exits non-zero if any design's re-measured Total drifts >2% from expectation.
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
IMG="ghcr.io/the-openroad-project/openlane:ff5509f65b17bfa4068d5336495ab1718987ff69"
LIB="$HOME/.ciel/sky130A/libs.ref/sky130_fd_sc_hd/lib/sky130_fd_sc_hd__tt_025C_1v80.lib"

# design | expected total mW
DESIGNS=(
  "soft_core_scalar|30.8"
  "decision_hft_bist|1.87"
  "cgra_sumto|1.36"
)

rc=0
printf '%-22s %-16s %-16s %s\n' "DESIGN" "RE-MEASURED mW" "EXPECTED mW" "VERDICT"
for row in "${DESIGNS[@]}"; do
  d="${row%%|*}"; exp="${row#*|}"
  run=$(ls -dt "$ROOT"/tools/OpenLane/designs/logos_"$d"/runs/*/ 2>/dev/null | head -1)
  if [ -z "$run" ] || [ ! -f "$run/results/final/verilog/gl/$d.nl.v" ]; then
    printf '%-22s %-16s %-16s %s\n' "$d" "-" "$exp" "NO SIGNOFF ARTIFACTS"; rc=1; continue
  fi
  runabs="/openlane/designs/logos_$d/runs/$(basename "$run")"
  tcl="$run/results/final/remeasure_power.tcl"
  cat > "$tcl" <<TCL
read_liberty $LIB
read_verilog $runabs/results/final/verilog/gl/$d.nl.v
link_design $d
read_spef $runabs/results/final/spef/$d.spef
read_sdc $runabs/results/final/sdc/$d.sdc
report_power
exit
TCL
  out=$(sg docker -c "docker run --rm \
      -v $ROOT/tools/OpenLane:/openlane -v $HOME:$HOME -v $HOME/.ciel:$HOME/.ciel \
      -e PDK_ROOT=$HOME/.ciel -e PDK=sky130A --user $(id -u):$(id -g) $IMG \
      sh -c 'sta -exit $runabs/results/final/remeasure_power.tcl'" 2>/dev/null)
  tot=$(echo "$out" | awk '/^Total/{print $5}')
  comb=$(echo "$out" | awk '/^Combinational/{print $5}')
  if [ -z "$tot" ]; then
    printf '%-22s %-16s %-16s %s\n' "$d" "STA FAILED" "$exp" "ERROR"; rc=1; continue
  fi
  mw=$(python3 -c "print(f'{float('$tot')*1e3:.2f}')")
  verdict=$(python3 -c "e=$exp; g=float('$tot')*1e3; print('OK  (comb ${comb} W)' if abs(g-e)/e<=0.02 else 'DRIFT >2%')")
  echo "$verdict" | grep -q DRIFT && rc=1
  printf '%-22s %-16s %-16s %s\n' "$d" "$mw" "$exp" "$verdict"
done
echo
[ $rc -eq 0 ] && echo "PROVEN: every design's power re-measures within 2% of the quoted figure." \
             || echo "FAIL: a design did not reproduce (see above)."
exit $rc
