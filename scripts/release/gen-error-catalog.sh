#!/usr/bin/env bash
# Regenerate the error catalog artifacts from the canonical diagnostic table:
#
#   docs/ERROR_CATALOG.md
#   apps/logicaffeine_web/public/errors.json
#
# Both are byte-locked by crates/logicaffeine_tests/tests/error_catalog_lock.rs;
# this script runs that file's #[ignore]d regenerator, then re-runs the locks
# to prove the checked-in files now match.
set -euo pipefail
cd "$(dirname "$0")/../.."

LOGOS_REGEN=1 cargo test -p logicaffeine-tests --test error_catalog_lock \
    regenerate_error_catalog -- --ignored --exact

cargo test -p logicaffeine-tests --test error_catalog_lock

echo "error catalog regenerated and locks green"
