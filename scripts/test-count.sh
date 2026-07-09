#!/usr/bin/env bash
# Compute the workspace test count exactly as the CI badge does, and print the
# badge JSON that lands on the README. This is the local dry-run of the
# `archive` job's "Count tests" step in .github/workflows/test.yml — same
# feature set, same `nextest list --test-count`, same jq path.
#
# Listing (--no-run) compiles every test binary once, so the first run is slow;
# it does not execute any test.
set -euo pipefail

# Same env the Linux CI/local runs need for the Z3-backed `verification` crates.
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"

# Same feature set CI archives with (kept in one place in the workflow env).
SUITE_FEATURES="logicaffeine-tests/ffi-link-tests,logicaffeine-tests/verification"

echo "Listing workspace tests (compiles test binaries; no tests run)..." >&2
N=$(cargo nextest list --workspace --features "$SUITE_FEATURES" \
      --profile full --message-format json 2>/dev/null \
    | jq '."test-count"' 2>/dev/null || true)

if [ -z "$N" ] || [ "$N" = "null" ]; then
  echo "ERROR: could not compute test count" >&2
  exit 1
fi

echo "Suite size: $N tests" >&2
printf '{"schemaVersion":1,"label":"tests","message":"%s+","color":"brightgreen"}\n' "$N"
