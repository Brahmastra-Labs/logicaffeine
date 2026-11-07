#!/usr/bin/env bash
# A command run under a watch for standing still. It runs as a session and process group of its own, its output (its
# standard output and error together) passed on through the watch line by line, and the CPU time its processes take
# together (their own and their reaped children's) is read every STALL_POLL seconds (default 5): a group that takes
# under <least> seconds of it and writes nothing in <window> seconds is stopped, as is one still running <bound> seconds
# after it started — TERM to the group, KILL to what is left of it STALL_GRACE seconds later (default 30) — and the
# watch exits 124 saying which. Otherwise it exits with the command's own status.
#
#   stall_watch.sh <window seconds> <least CPU seconds> <bound seconds> <command> [argument...]
#
# A Quartus IP generation stands still this way when a Tcl helper one of its generators starts dies at its own start
# (the DDM start-up check): the generator waits for the helper's connection with no timeout, asleep, writing nothing,
# and every process of the generation waits on it. A working one keeps a core busy or keeps writing: a synthesis's pass
# over the IP it finds generated takes little CPU and logs each IP.
set -uo pipefail
[ $# -ge 4 ] || { echo "usage: stall_watch.sh <window seconds> <least CPU seconds> <bound seconds> <command> [argument...]" >&2; exit 2; }
window=$1 least=$2 bound=$3
shift 3
poll=${STALL_POLL:-5}
grace=${STALL_GRACE:-30}
tck=$(getconf CLK_TCK 2>/dev/null || echo 100)

# The clock ticks of CPU the processes of group `$1` have taken, with their reaped children's.
group_ticks() {
  local group=$1 total=0 f line
  for f in /proc/[0-9]*/stat; do
    read -r line < "$f" 2>/dev/null || continue
    # The fields after the command's name, which may hold spaces and parentheses: state, parent, group, ... utime (12th),
    # stime, cutime, cstime.
    set -- ${line##*) }
    [ "$3" = "$group" ] && total=$((total + ${12} + ${13} + ${14} + ${15}))
  done
  echo "$total"
}

# Stop group `$1`: TERM to it, then KILL to what is left of it after the grace, each process found by its group id.
stop_group() {
  local waited=0
  pkill -TERM -g "$1" 2>/dev/null
  while pgrep -g "$1" > /dev/null && [ "$waited" -lt "$grace" ]; do
    sleep 1
    waited=$((waited + 1))
  done
  pkill -KILL -g "$1" 2>/dev/null
}

own=$(mktemp -d)
trap 'rm -rf "$own"' EXIT
reason=$own/reason
seen=$own/seen
mkfifo "$own/out"
: > "$seen"
# The command's output passed on a line at a time, the time of its last line kept as the mtime of `seen`.
(
  while IFS= read -r line || [ -n "$line" ]; do
    printf '%s\n' "$line"
    : > "$seen"
  done < "$own/out"
) &
reader=$!
setsid "$@" > "$own/out" 2>&1 &
pid=$!
(
  start=$(date +%s)
  mark=$start
  marked=$(group_ticks "$pid")
  while sleep "$poll"; do
    now=$(date +%s)
    ticks=$(group_ticks "$pid")
    if [ $((ticks - marked)) -ge $((least * tck)) ] || [ "$(stat -c %Y "$seen")" -gt "$mark" ]; then
      mark=$now
      marked=$ticks
    elif [ $((now - mark)) -ge "$window" ]; then
      took=$(((ticks - marked) * 100 / tck))
      echo "stall_watch.sh: $1 took $((took / 100)).$(printf '%02d' $((took % 100))) s of CPU in $((now - mark)) s and wrote nothing, under $least; its processes stopped" > "$reason"
      stop_group "$pid"
      exit 0
    fi
    if [ $((now - start)) -ge "$bound" ]; then
      echo "stall_watch.sh: $1 still ran $bound s after it started; its processes stopped" > "$reason"
      stop_group "$pid"
      exit 0
    fi
  done
) > /dev/null 2>&1 < /dev/null &
watcher=$!
wait "$pid"
status=$?
# The output's last lines passed on, unless something the command left behind still holds the pipe.
for _ in $(seq 50); do
  kill -0 "$reader" 2>/dev/null || break
  sleep 0.1
done
kill "$reader" 2>/dev/null
wait "$reader" 2>/dev/null
if [ -s "$reason" ]; then
  wait "$watcher"
  cat "$reason" >&2
  exit 124
fi
kill "$watcher" 2>/dev/null
wait "$watcher" 2>/dev/null
exit "$status"
