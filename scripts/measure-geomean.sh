#!/usr/bin/env bash
# Interp-vs-Node geomean over the 31 compute-calibrated benchmarks.
# Best-of-3 per bench. Prints per-bench ratio + geomean. Reusable.
set -u
cd /home/tristen/logicaffeine
LARGO=/home/tristen/logicaffeine/target/release/largo
[ -x "$LARGO" ] || { echo "no release largo; build first"; exit 1; }
TMP=$(mktemp -d); LOG="$TMP/logs.txt"; : > "$LOG"
echo "load: $(cut -d' ' -f1 /proc/loadavg)"
data="fannkuch 10 203.2
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
while read b n node; do
  d="$TMP/$b"; mkdir -p "$d/src"; cp "benchmarks/programs/$b/main.lg" "$d/src/main.lg"
  printf '[package]\nname="bench"\nversion="0.1.0"\nentry="src/main.lg"\n' > "$d/Largo.toml"
  best=999999
  for i in 1 2 3; do t=$( cd "$d" && { /usr/bin/time -f '%e' timeout 90 "$LARGO" run --interpret "$n" >/dev/null; } 2>&1 | tail -1 ); ms=$(awk "BEGIN{printf \"%d\",$t*1000}"); [ "$ms" -lt "$best" ] && best=$ms; done
  awk "BEGIN{printf \"%-14s %6dms %5.2fx\n\",\"$b\",$best,$best/$node; print log($best/$node) > \"/dev/stderr\"}" 2>>"$LOG"
done <<< "$data"
echo "----"
awk '{s+=$1;n++; if(exp($1)<1)c++} END{printf "GEOMEAN(31): %.3fx Node   beat V8: %d/31\n", exp(s/n), c}' "$LOG"
rm -rf "$TMP"