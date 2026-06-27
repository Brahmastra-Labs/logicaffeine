#!/usr/bin/env bash
# Faithful interleaved A/B for a single LOGOS_* kill-switch flag — the promotion
# gate this campaign uses. For each benchmark it runs OFF (FLAG=0) and ON (FLAG=1)
# of the SAME release binary, ALTERNATING the order every trial so monotonic
# machine drift (thermal/frequency/background load) cancels in pairs, and takes
# the best-of-N min per arm (noise is one-sided — it only adds time). It prints a
# per-benchmark OFF/ON/delta table, the geomean of both arms, and flags any
# benchmark whose ON arm regresses beyond the noise band. This is why a stale
# fresh-vs-old-baseline geomean LIES: only an interleaved same-session A/B is
# trustworthy near parity (a prior run read a false +0.9% "regression" that was
# pure drift). See the plan: ~/.claude/plans/we-want-to-make-partitioned-lemon.md
#
# Usage:   scripts/gate-ab.sh LOGOS_RUN_SCALARIZE            # all benchmarks
#          ONLY=coins,graph_bfs scripts/gate-ab.sh LOGOS_RUN_AFFINE
# Env:     REPS=5 (per arm), NOISE=2.0 (% band), ONLY=<csv subset>
set -u
cd /home/tristen/logicaffeine
FLAG="${1:?usage: gate-ab.sh LOGOS_FLAG_NAME}"
REPS="${REPS:-5}"
NOISE="${NOISE:-2.0}"
LARGO="$(pwd)/target/release/largo"
[ -x "$LARGO" ] || { echo "no release largo at $LARGO — build first"; exit 1; }

# benchmark  calibrated-N  node-ms  (the campaign's frozen interp-vs-Node sizes/baselines)
DATA="fannkuch 10 203.2
string_search 14856116 218.9
nbody 5613145 243.7
histogram 184400072 246.9
matrix_mult 510 243.0
knapsack 6040 250.7
graph_bfs 2633876 248.5
spectral_norm 2012 245.0
bubble_sort 18193 259.8
heap_sort 1603988 248.7
mandelbrot 2356 246.1
nqueens 14 190.8
strings 1960222 249.0
binary_trees 19 219.3
quicksort 1799948 246.7
coins 14848272 248.0
fib 38 208.9
pi_leibniz 273270549 247.7
sieve 45299987 250.9
mergesort 961828 243.5
primes 2341604 249.0
prefix_sum 27180813 251.0
array_reverse 33554432 191.1
gcd 4660 244.5
fib_iterative 105682072 248.0
array_fill 26846991 247.0
collatz 362777 243.1
collect 2097152 215.7
two_sum 5770178 239.2
counting_sort 7558380 242.1
loop_sum 103922713 247.1"

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
SEL=""
[ -n "${ONLY:-}" ] && SEL=",${ONLY},"
run1() { ( cd "$1" && { /usr/bin/time -f '%e' env "$FLAG=$2" "$LARGO" run --interpret "$3" >/dev/null; } 2>&1 | tail -1 ); }

printf "Faithful A/B for %s  (REPS=%s, interleaved, best-of-min, noise band %s%%)\n" "$FLAG" "$REPS" "$NOISE"
printf "%-14s %8s %8s %9s %8s\n" bench OFFms ONms delta note
echo "----------------------------------------------------------"
LOGOFF=""; LOGON=""; NREG=0; NWIN=0
while read -r b n node; do
  [ -z "$b" ] && continue
  [ -n "$SEL" ] && case "$SEL" in *",$b,"*) ;; *) continue;; esac
  d="$TMP/$b"; mkdir -p "$d/src"; cp "benchmarks/programs/$b/main.lg" "$d/src/main.lg"
  printf '[package]\nname="bench"\nversion="0.1.0"\nentry="src/main.lg"\n' > "$d/Largo.toml"
  bo=999999; bn=999999
  for ((i=1;i<=REPS;i++)); do
    if (( i % 2 )); then a=0; c=1; else a=1; c=0; fi
    t1=$(run1 "$d" "$a" "$n"); t2=$(run1 "$d" "$c" "$n")
    if [ "$a" = 0 ]; then o=$t1; w=$t2; else o=$t2; w=$t1; fi
    mo=$(awk "BEGIN{printf \"%d\",$o*1000}"); mw=$(awk "BEGIN{printf \"%d\",$w*1000}")
    [ "$mo" -gt 0 ] && [ "$mo" -lt "$bo" ] && bo=$mo
    [ "$mw" -gt 0 ] && [ "$mw" -lt "$bn" ] && bn=$mw
  done
  delta=$(awk "BEGIN{printf \"%.1f\", ($bn-$bo)*100.0/$bo}")
  note=""
  awk "BEGIN{exit !(($bn-$bo)*100.0/$bo > $NOISE)}" && { note="REGRESS"; NREG=$((NREG+1)); }
  awk "BEGIN{exit !(($bo-$bn)*100.0/$bo > $NOISE)}" && { note="win"; NWIN=$((NWIN+1)); }
  printf "%-14s %8s %8s %8s%% %8s\n" "$b" "$bo" "$bn" "$delta" "$note"
  LOGOFF="$LOGOFF $(awk "BEGIN{print log($bo/$node)}")"
  LOGON="$LOGON $(awk "BEGIN{print log($bn/$node)}")"
done <<< "$DATA"
echo "----------------------------------------------------------"
GOFF=$(awk '{s=0;n=0;for(i=1;i<=NF;i++){s+=$i;n++} if(n)printf "%.3f",exp(s/n)}' <<< "$LOGOFF")
GON=$(awk '{s=0;n=0;for(i=1;i<=NF;i++){s+=$i;n++} if(n)printf "%.3f",exp(s/n)}' <<< "$LOGON")
printf "GEOMEAN vs Node:  OFF %s   ON %s   (wins:%s regress:%s)\n" "$GOFF" "$GON" "$NWIN" "$NREG"
[ "$NREG" -gt 0 ] && echo "*** $NREG benchmark(s) regress >${NOISE}% — re-measure (noise?) before promoting." || echo "No regression beyond the noise band."
