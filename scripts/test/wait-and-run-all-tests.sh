#!/usr/bin/env bash
# Wait for the box to go genuinely quiet (no competing cargo/nextest/rustc from
# any agent, low load), SUSTAINED, then run the full fast suite ONCE — nothing
# skipped (verification + ignored + doctests). Avoids the two-concurrent-Z3-suites
# contention (rule 11) that SIGTERM'd the occasion tests.
set -u
cd "$(dirname "$0")/../.."
STAMP=$(date +%Y%m%d-%H%M%S)
STATUS=logs/optimization/wait-and-run-${STAMP}.status
: > "$STATUS"
log() { echo "[$(date +%H:%M:%S)] $*" >> "$STATUS"; }

LOAD_MAX=8.0
SUSTAIN=2          # consecutive quiet polls required
POLL=60            # seconds
MAX_POLLS=120      # ~2h ceiling

busy() {
  # any nextest suite (mine or theirs)
  pgrep -x cargo-nextest >/dev/null 2>&1 && { echo "cargo-nextest"; return 0; }
  # any cargo test / cargo build invocation.
  #
  # NOT `pgrep -af`. A `-a`/`-f` pattern matches the FULL COMMAND LINE, so this very script —
  # whose own cmdline and body mention `cargo test` — matched itself and had to be filtered back
  # out with `grep -vq 'wait-and-run'`. That hand-rolled exclusion only knows ONE name: rename
  # the script, wrap it, or invoke it via a shell whose cmdline carries the pattern, and the
  # guard silently reports BUSY forever on an idle box. Measured elsewhere in this repo: a
  # self-matching wait span 25 minutes without running the job it was waiting for, and a
  # self-matching kill took out its own shell (exit 144).
  #
  # `pgrep -x` matches the EXECUTABLE NAME only and can never match a shell that merely mentions
  # it, so no exclusion list is needed and none can go stale.
  pgrep -x cargo >/dev/null 2>&1 && { echo "cargo test/build"; return 0; }
  # raw rustc (a competing build; the suite isn't running yet, so any rustc is foreign)
  pgrep -x rustc >/dev/null 2>&1 && { echo "rustc"; return 0; }
  # load gate
  local l; l=$(cut -d' ' -f1 /proc/loadavg)
  awk "BEGIN{exit !($l > $LOAD_MAX)}" && { echo "load=$l"; return 0; }
  return 1
}

log "watcher armed (LOAD_MAX=$LOAD_MAX, need $SUSTAIN consecutive quiet polls)"
quiet=0
for i in $(seq 1 $MAX_POLLS); do
  reason=$(busy)
  if [ -n "$reason" ]; then
    quiet=0
    log "poll $i: BUSY ($reason) — waiting"
  else
    quiet=$((quiet+1))
    log "poll $i: quiet ($quiet/$SUSTAIN), load=$(cut -d' ' -f1 /proc/loadavg)"
    if [ "$quiet" -ge "$SUSTAIN" ]; then
      log "QUIET sustained — launching full suite"
      echo "RUNNING" >> "$STATUS"
      ./scripts/run-all-tests-fast.sh
      rc=$?
      log "suite finished, exit=$rc"
      echo "SUITE_EXIT=$rc" >> "$STATUS"
      # surface the suite's own SUMMARY line
      grep -aE 'Summary \[|tests run:|RESULT' logs/latest-fast.log 2>/dev/null | tail -4 >> "$STATUS"
      exit $rc
    fi
  fi
  sleep $POLL
done
log "TIMEOUT — box never went quiet in $MAX_POLLS polls; suite NOT run"
echo "NEVER_QUIET" >> "$STATUS"
exit 2