#!/usr/bin/env bash
#
# trace-run.sh — run any command while tracing CPU/RSS + system load, so timing
# survives the noisy-neighbor problem on a shared box.
#
# The headline number it captures is CPU-seconds (user+sys), which is
# CONTENTION-INVARIANT: a busy neighbor inflates wall-clock but never changes how
# much CPU work a process performs. So you can blast runs concurrently and parse
# the real signal from the logs afterward, instead of fighting for a quiet box.
#
# Emits under logs/ (timestamped, alongside the normal run log):
#   trace-<ts>.tsv      one row per sample: epoch  load1  scope  pid  %cpu  rss_kb  comm
#                       scope = self (our process group) | foreign (someone else)
#   trace-<ts>.time     raw `/usr/bin/time -v` output for the wrapped command
#   trace-<ts>.summary  the digest (also echoed): CPU-seconds, wall, effective
#                       cores (CPU/wall), peak load, max RSS, neighbor-busy ratio
#
# Usage:   ./scripts/trace-run.sh <command> [args...]
#   e.g.   ./scripts/trace-run.sh ./scripts/run-all-tests-fast.sh --no-ignored
# Env:     TRACE_INTERVAL=5   sampling period in seconds
#          TRACE_CPU_MIN=1.5  only log procs above this %CPU (keeps the tsv lean)
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
mkdir -p logs

TS="$(date +%Y%m%d-%H%M%S)"
TSV="logs/trace-$TS.tsv"
TIMEF="logs/trace-$TS.time"
SUMM="logs/trace-$TS.summary"
INTERVAL="${TRACE_INTERVAL:-5}"
CPU_MIN="${TRACE_CPU_MIN:-1.5}"

if [ "$#" -eq 0 ]; then echo "usage: $0 <command> [args...]" >&2; exit 2; fi

# Our process group: children (cargo/nextest/test bins) inherit it, so we can tag
# samples self vs foreign without walking ppid chains every tick.
SELF_PGID="$(ps -o pgid= -p $$ 2>/dev/null | tr -d ' ')"

printf 'epoch\tload1\tscope\tpid\tpcpu\trss_kb\tcomm\n' > "$TSV"

# --- background sampler -------------------------------------------------------
(
  while :; do
    epoch="$(date +%s)"
    read -r l1 _ < /proc/loadavg
    ps -eo pgid,pid,pcpu,rss,comm --sort=-pcpu --no-headers 2>/dev/null \
      | awk -v e="$epoch" -v l="$l1" -v sg="$SELF_PGID" -v min="$CPU_MIN" '
          ($3 + 0) > min {
            scope = ($1 == sg) ? "self" : "foreign"
            print e "\t" l "\t" scope "\t" $2 "\t" $3 "\t" $4 "\t" $5
          }' >> "$TSV"
    sleep "$INTERVAL"
  done
) &
SAMPLER=$!
trap 'kill "$SAMPLER" 2>/dev/null' EXIT

# --- run the wrapped command under /usr/bin/time ------------------------------
echo "trace-run: $* "
echo "  sampling every ${INTERVAL}s -> $TSV"
START="$(date +%s)"
if command -v /usr/bin/time >/dev/null 2>&1; then
  /usr/bin/time -v -o "$TIMEF" "$@"
  RC=$?
else
  echo "WARNING: /usr/bin/time not found; wall-only (no clean CPU-seconds)" | tee "$TIMEF"
  "$@"; RC=$?
fi
END="$(date +%s)"
kill "$SAMPLER" 2>/dev/null

# --- digest -------------------------------------------------------------------
WALL=$(( END - START ))
field() { awk -F': ' -v k="$1" '$0 ~ k {print $2; exit}' "$TIMEF" 2>/dev/null; }
USER_S="$(field 'User time')"
SYS_S="$(field 'System time')"
MAXRSS="$(field 'Maximum resident set size')"
CPUPCT="$(field 'Percent of CPU')"
CPU_S="$(awk -v u="${USER_S:-0}" -v s="${SYS_S:-0}" 'BEGIN{printf "%.1f", u+s}')"
EFFCORES="$(awk -v c="$CPU_S" -v w="$WALL" 'BEGIN{ if (w>0) printf "%.2f", c/w; else printf "?" }')"
PEAKLOAD="$(awk -F'\t' 'NR>1 && $2+0>m{m=$2}END{printf "%.1f", m+0}' "$TSV")"
SELF_BUSY="$(awk -F'\t' 'NR>1 && $3=="self"{a[$1]=1}END{print length(a)}' "$TSV")"
FOREIGN_BUSY="$(awk -F'\t' 'NR>1 && $3=="foreign" && $5+0>20{a[$1]=1}END{print length(a)}' "$TSV")"
TICKS="$(awk -F'\t' 'NR>1{a[$1]=1}END{print length(a)}' "$TSV")"

{
  echo "════════════ trace summary ($TS) ════════════"
  echo " command         : $*"
  echo " exit            : $RC"
  echo " wall            : ${WALL}s"
  echo " CPU (clean)     : ${CPU_S}s   user=${USER_S:-?}s sys=${SYS_S:-?}s   ← CONTENTION-INVARIANT"
  echo " effective cores : ${EFFCORES}   (CPU/wall — low ⇒ starved by neighbors or serial bottleneck)"
  echo " time's CPU%     : ${CPUPCT:-?}"
  echo " max RSS         : ${MAXRSS:-?} KB"
  echo " peak load1      : ${PEAKLOAD}"
  echo " neighbor-busy   : ${FOREIGN_BUSY}/${TICKS} sampled ticks had a foreign hog >20% CPU"
  echo " trace tsv       : $TSV"
  echo "════════════════════════════════════════════════"
} | tee "$SUMM"

exit "$RC"
