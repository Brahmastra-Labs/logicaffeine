#!/usr/bin/env bash
# Apply a queued edit to scripts/openlane-run.sh ONLY when no process is executing it.
#
# WHY THIS EXISTS. Bash reads a script INCREMENTALLY, from a byte offset it keeps as it goes. Edit
# the file while a flow is running it and the running shell resumes at an offset that now points
# into different text — it executes garbage. That has happened twice: once mid-queue (a transient
# "syntax error near unexpected token" AFTER the block had already succeeded, losing only the
# recording step), and once today when an 18-line insertion went in while two flows were live and
# had to be reverted within the minute.
#
# `bash -n` does not protect against this and never did — it checks the file, not the readers.
set -euo pipefail
cd "$(dirname "$0")/.."
BLOCK_FILE="${1:?usage: apply-pending-runner-edit.sh <block-file>}"
[ -f "$BLOCK_FILE" ] || { echo "no queued block at $BLOCK_FILE" >&2; exit 2; }

# Match the EXECUTABLE, never a -f cmdline pattern: this script's own command line contains the
# string "openlane-run.sh", so `pgrep -f openlane-run.sh` would match the watcher and never clear.
# That self-match burned an hour today in a wait condition (see the pkill/pgrep memory).
running=$(ps -eo args --no-headers | grep -c '[o]penlane-run\.sh' || true)
if [ "$running" -gt 0 ]; then
  echo "REFUSING: $running process(es) are executing scripts/openlane-run.sh right now." >&2
  echo "          Re-run this when the queue is idle; the edit stays queued." >&2
  exit 3
fi

python3 - "$BLOCK_FILE" <<'PY'
import sys
block = open(sys.argv[1]).read()
p = 'scripts/openlane-run.sh'
s = open(p).read()
anchor = 'OUT_JSON="${OUT_JSON:-$SIL/sky130_signoff.json}"'
if block.strip().splitlines()[0] in s:
    sys.exit("already applied")
assert anchor in s, "anchor line not found — openlane-run.sh changed shape"
open(p, 'w').write(s.replace(anchor, block + anchor, 1))
print("applied")
PY
bash -n scripts/openlane-run.sh && echo "parses — verify by RUNNING it, not by bash -n"
