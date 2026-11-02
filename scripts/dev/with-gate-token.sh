#!/usr/bin/env bash
# Serialize the wave loop's GATE: the full test suite and the benchmark A/B.
#
# This is the hard mutex that guarantees exactly one wave is ever measuring at a
# time — the single global bottleneck the whole design is built around. Benchmark
# timing is noise-sensitive and must own the quiet box; concurrent suites froze
# the machine once (CLAUDE.md rule 11). flock blocks natively and releases when
# the wrapped command exits or dies, so a crashed gate never leaks the token.
#
# Distinct lock file from with-build-token.sh: a dev agent may hold the build
# token (compiling its crate) while another wave legitimately holds the gate.
#
# Usage:   scripts/dev/with-gate-token.sh <command...>
# Env:     LOCK_WAIT=<seconds>   bound the wait and exit 75 on timeout (default: block forever)
#          LOGOS_TOKEN_DIR=<dir>  where the lock file lives (default: ~/.cache/logos-tokens) — one directory
#                                 per machine, so every copy of the repo on it shares the one token
set -euo pipefail
LOCK_DIR="${LOGOS_TOKEN_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/logos-tokens}"
mkdir -p "$LOCK_DIR"
exec 9>"$LOCK_DIR/.gate.lock"
if [ -n "${LOCK_WAIT:-}" ]; then
  flock -w "$LOCK_WAIT" 9 || { echo "with-gate-token: timed out after ${LOCK_WAIT}s waiting for the gate lock" >&2; exit 75; }
else
  flock 9
fi
exec "$@"
