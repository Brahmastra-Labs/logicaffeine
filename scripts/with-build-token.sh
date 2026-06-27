#!/usr/bin/env bash
# Serialize every cargo build / cargo nextest invocation in the wave loop.
#
# All crates share one target/ dir and a build saturates every core, so two
# concurrent `cargo` processes are exactly the rule-11 disaster at smaller scale
# (CLAUDE.md). Dev agents EDIT crate-disjoint surfaces in parallel but must hold
# this token to COMPILE or TEST. flock blocks natively (no sleep-spin) and the
# lock is released automatically when the wrapped command exits or dies, so a
# crashed wave never leaks the token.
#
# Usage:   scripts/with-build-token.sh <command...>
# Env:     LOCK_WAIT=<seconds>   bound the wait and exit 75 on timeout (default: block forever)
set -euo pipefail
LOCK_DIR="/home/tristen/logicaffeine/logs/optimization"
mkdir -p "$LOCK_DIR"
exec 9>"$LOCK_DIR/.build.lock"
if [ -n "${LOCK_WAIT:-}" ]; then
  flock -w "$LOCK_WAIT" 9 || { echo "with-build-token: timed out after ${LOCK_WAIT}s waiting for the build lock" >&2; exit 75; }
else
  flock 9
fi
exec "$@"
