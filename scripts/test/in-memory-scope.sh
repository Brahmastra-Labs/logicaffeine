#!/usr/bin/env bash
# in-memory-scope.sh — sourced as a heavy runner's first act, with the memory cap it needs:
#   source "$SCRIPT_DIR/test/in-memory-scope.sh" <MemoryMax> "$SCRIPT_DIR/<runner>.sh" "$@"
# Re-executes the runner inside a transient user systemd scope with that MemoryMax and swap off, so a
# runaway test is killed before it takes the box. A runner already inside its scope (LOGOS_MEMORY_SCOPE
# set), or a machine without a user systemd, runs as it is.
_cap="$1"; _runner="$2"; shift 2
if [ -z "${LOGOS_MEMORY_SCOPE:-}" ] && command -v systemd-run >/dev/null 2>&1 \
   && systemd-run --user --scope --quiet true >/dev/null 2>&1; then
  export LOGOS_MEMORY_SCOPE="$_cap"
  exec systemd-run --user --scope --quiet -p MemoryMax="$_cap" -p MemorySwapMax=0 "$_runner" "$@"
fi
unset _cap _runner
