#!/usr/bin/env bash
# Regression gate: compare a candidate benchmark JSON against a baseline.
#
# For every benchmark, the LOGOS-vs-C speedup at its reference size is
# c.mean_ms / logos_release.mean_ms (higher = LOGOS faster). We flag any
# benchmark whose fresh speedup fell below its baseline mark, and check the
# geometric-mean speedup against a floor.
#
# Usage:
#   bash benchmarks/compare-vs-baseline.sh <candidate.json> [baseline.json] [geomean_floor]
#
# Defaults: baseline = results/history/v0.9.17.json, floor = 2.5
#
# Exit codes:
#   0  all good (geomean >= floor, no benchmark >3% below its baseline mark)
#   1  a real per-benchmark regression (>3% slower than baseline)
#   2  geomean below the floor
#   3  bad input
#
# Marginal dips within 3% (noise band) are printed as WARN but do not fail;
# re-run the affected benchmark with more runs to confirm before shipping.
set -euo pipefail

CAND="${1:-}"
BASE="${2:-$(dirname "$0")/results/history/v0.9.17.json}"
FLOOR="${3:-2.5}"

if [ -z "$CAND" ] || [ ! -f "$CAND" ]; then
  echo "error: candidate JSON not found: '$CAND'" >&2; exit 3
fi
if [ ! -f "$BASE" ]; then
  echo "error: baseline JSON not found: '$BASE'" >&2; exit 3
fi

echo "candidate: $CAND"
echo "baseline : $BASE"
echo "floor    : ${FLOOR}x geomean"
echo

# Per-benchmark join at each candidate benchmark's reference size.
# Emits TSV: status  name  old  new  pct   (pct = (new-old)/old*100)
rows="$(jq -rs '
  def sp($b): ($b.scaling[$b.reference_size]) as $s
              | if ($s == null or $s.logos_release == null or $s.c == null) then null
                else ($s.c.mean_ms / $s.logos_release.mean_ms) end;
  .[0] as $cand | .[1] as $base
  | ($base.benchmarks | map({key: .name, value: sp(.)}) | from_entries) as $bmap
  | $cand.benchmarks[]
  | .name as $n
  | (sp(.)) as $new
  | ($bmap[$n]) as $old
  | if $new == null then "MISSING\t\($n)\t-\t-\t-"
    elif $old == null then "NEWONLY\t\($n)\t-\t\($new)\t-"
    else
      (($new - $old) / $old * 100) as $pct
      | (if $new >= $old then "OK"
         elif $pct > -3 then "WARN"
         else "REGRESS" end) as $st
      | "\($st)\t\($n)\t\($old)\t\($new)\t\($pct)"
    end
' "$CAND" "$BASE")"

printf "%-8s %-26s %10s %10s %9s\n" "STATUS" "BENCHMARK" "OLD(x)" "NEW(x)" "DELTA%"
printf -- "------------------------------------------------------------------------\n"
regress=0; warn=0
while IFS=$'\t' read -r st name old new pct; do
  [ -z "$st" ] && continue
  if [ "$old" = "-" ]; then oldf="$old"; else oldf=$(printf "%.3f" "$old"); fi
  if [ "$new" = "-" ]; then newf="$new"; else newf=$(printf "%.3f" "$new"); fi
  if [ "$pct" = "-" ]; then pctf="$pct"; else pctf=$(printf "%+.1f" "$pct"); fi
  printf "%-8s %-26s %10s %10s %9s\n" "$st" "$name" "$oldf" "$newf" "$pctf"
  [ "$st" = "REGRESS" ] && regress=$((regress+1))
  [ "$st" = "WARN" ] && warn=$((warn+1))
  [ "$st" = "MISSING" ] && regress=$((regress+1))
done <<< "$rows"
echo

# Geomean: prefer the candidate's own summary, else compute from per-bench rows.
geo_new="$(jq -r '.summary.geometric_mean_speedup_vs_c.logos_release // empty' "$CAND")"
if [ -z "$geo_new" ]; then
  geo_new="$(jq -rs '
    def sp($b): ($b.scaling[$b.reference_size]) as $s
                | if ($s==null or $s.logos_release==null or $s.c==null) then null
                  else ($s.c.mean_ms/$s.logos_release.mean_ms) end;
    [ .[0].benchmarks[] | sp(.) | select(. != null) ] as $xs
    | ($xs | map(log) | add / length | exp)
  ' "$CAND" "$CAND")"
fi
geo_old="$(jq -r '.summary.geometric_mean_speedup_vs_c.logos_release // empty' "$BASE")"

echo "geomean  baseline: ${geo_old}x   candidate: ${geo_new}x   (floor ${FLOOR}x)"
echo "regressions(>3%): $regress    marginal(WARN): $warn"
echo

rc=0
awk -v g="$geo_new" -v f="$FLOOR" 'BEGIN{ exit !(g+0 >= f+0) }' || { echo "FAIL: geomean ${geo_new}x < floor ${FLOOR}x"; rc=2; }
if [ "$regress" -gt 0 ]; then echo "FAIL: $regress benchmark(s) regressed >3% vs baseline"; [ "$rc" -eq 0 ] && rc=1; fi
if [ "$rc" -eq 0 ]; then echo "PASS: geomean clears floor and no benchmark regressed >3%"; fi
exit "$rc"
