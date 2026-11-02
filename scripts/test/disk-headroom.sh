#!/usr/bin/env bash
# Refuse a run whose disks are short of room: `disk-headroom.sh <GB> <dir>…` exits 1, naming each
# directory, when the filesystem holding it has fewer than <GB> gigabytes free. A directory not made
# yet is measured on the disk it will be made on. A full test run writes tens of gigabytes of scratch
# into its TMPDIR and rebuilds into its target directory; a run that fills its disk fails every later
# test for a reason not its own, and every other build on that disk with it. Both full runners call
# this before their first cargo command (`disk_headroom_lock`).
set -euo pipefail
floor="$1"
shift
status=0
for dir in "$@"; do
  probe="$dir"
  while [ ! -e "$probe" ]; do
    probe="$(dirname "$probe")"
  done
  free_gb=$(( $(df -Pk "$probe" | awk 'NR == 2 { print $4 }') / 1024 / 1024 ))
  if [ "$free_gb" -lt "$floor" ]; then
    echo "ERROR: $dir is on a disk with ${free_gb} GB free; a full run needs ${floor} GB there." >&2
    status=1
  fi
done
exit "$status"
