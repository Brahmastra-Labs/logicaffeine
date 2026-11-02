#!/usr/bin/env bash
# tdd.sh — run tests and demand a RED or GREEN verdict: the CLAUDE.md TDD loop, mechanised.
#
#   scripts/test/tdd.sh red   rust <nextest args...>            every selected test must FAIL
#   scripts/test/tdd.sh green rust <nextest args...>            every selected test must PASS
#   scripts/test/tdd.sh red   learn-e2e <public-dir> [--firefox] [scenario...]
#   scripts/test/tdd.sh green learn-e2e <public-dir> [--firefox] [scenario...]
#   scripts/test/tdd.sh red   site-e2e <public-dir | url> [--firefox] [scenario...]
#   scripts/test/tdd.sh green site-e2e <public-dir | url> [--firefox] [scenario...]
#   scripts/test/tdd.sh red   junit <report.xml>...             verdict on reports produced elsewhere
#   scripts/test/tdd.sh green junit <report.xml>...
#
# Examples:
#   scripts/test/tdd.sh red   rust -p logicaffeine-web -E 'binary(learn_curriculum)'
#   scripts/test/tdd.sh green rust -p logicaffeine-web
#   scripts/test/tdd.sh red   learn-e2e target/dx/logicaffeine-web/debug/web/public space_key_does_not_scroll
#   scripts/test/tdd.sh red   site-e2e http://127.0.0.1:8080 --firefox every_roadmap_dot_sits_on_the_line
#
# A RED is only a RED when the new tests compile, run, and fail on their own assertions. A
# filter that selects nothing, a test that already passes, a skipped test, or a build that
# never got as far as running a test is refused — "it didn't compile" proves nothing about
# the behaviour, so stub the missing API until the test runs and fails for the right reason.
# The verdict itself is scripts/lib/tdd-verdict.py over JUnit reports, never scraped output.
#
# Rust runs use the `tdd` nextest profile (.config/nextest.toml: no retries, no fail-fast,
# JUnit report) and go through the same brakes as the full baseline (scripts/lib/suite-guard.sh):
# niced, concurrency-capped, and refused while another suite or a heavy job is running.
#
# Exit status: 0 when the expectation holds, 1 when it does not, 2 on usage or harness errors.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

usage() { sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

[ $# -ge 2 ] || usage
EXPECT="$1"; SUITE="$2"; shift 2
case "$EXPECT" in red|green) ;; *) usage ;; esac

VERDICT="$REPO_ROOT/scripts/lib/tdd-verdict.py"

case "$SUITE" in
  junit)
    [ $# -ge 1 ] || usage
    exec python3 "$VERDICT" "$EXPECT" "$@"
    ;;

  rust)
    [ $# -ge 1 ] || { echo "tdd.sh: name what to run (e.g. -p <crate> -E 'test(name)')" >&2; exit 2; }
    source "$REPO_ROOT/scripts/lib/suite-guard.sh"
    suite_guard_brakes
    suite_guard_require_nextest || exit 2
    suite_guard_refuse_overlap || exit 2
    suite_guard_refuse_heavy "$REPO_ROOT/scripts/test/heavy-jobs.sh" || exit 2

    TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
    REPORT="$TARGET_DIR/nextest/tdd/junit.xml"
    rm -f "$REPORT"

    BUILD_LOG="$(mktemp -t tdd-build.XXXXXX)"
    trap 'rm -f "$BUILD_LOG"' EXIT
    echo "tdd.sh: cargo nextest run --profile tdd --no-tests=fail $*"
    cargo nextest run --profile tdd --no-tests=fail "$@" 2>&1 | tee "$BUILD_LOG" | grep -E '^\s+(PASS|FAIL|SIGABRT|SIGSEGV|TIMEOUT|LEAK)|Summary \[|^error(\[|:)'
    status=${PIPESTATUS[0]}

    if [ ! -s "$REPORT" ]; then
      echo "tdd.sh: INVALID — no test ran (nextest exit $status). The build or the selection failed:"
      grep -E '^error' -A 12 "$BUILD_LOG" | head -60
      [ -s "$BUILD_LOG" ] && ! grep -q '^error' "$BUILD_LOG" && tail -20 "$BUILD_LOG"
      exit 2
    fi
    python3 "$VERDICT" "$EXPECT" "$REPORT"
    exit $?
    ;;

  learn-e2e)
    [ $# -ge 1 ] || { echo "tdd.sh: learn-e2e needs the built public dir" >&2; exit 2; }
    PUBLIC="$1"; shift
    REPORT="$(mktemp -t tdd-learn-e2e.XXXXXX).xml"
    trap 'rm -f "$REPORT"' EXIT
    ENGINE=()
    NAMES=()
    for arg in "$@"; do
      case "$arg" in
        --firefox) ENGINE=(--firefox) ;;
        *) NAMES+=("$arg") ;;
      esac
    done
    ONLY=()
    [ ${#NAMES[@]} -gt 0 ] && ONLY=(--only "$(IFS=,; echo "${NAMES[*]}")")
    "$REPO_ROOT/scripts/web/test-learn-e2e.sh" "$PUBLIC" "${ENGINE[@]}" --junit "$REPORT" --require-browser "${ONLY[@]}"
    status=$?
    if [ ! -s "$REPORT" ]; then
      echo "tdd.sh: INVALID — the browser suite wrote no report (exit $status)"
      exit 2
    fi
    python3 "$VERDICT" "$EXPECT" "$REPORT"
    exit $?
    ;;

  site-e2e)
    [ $# -ge 1 ] || { echo "tdd.sh: site-e2e needs the built public dir or the running site's url" >&2; exit 2; }
    SITE="$1"; shift
    REPORT="$(mktemp -t tdd-site-e2e.XXXXXX).xml"
    trap 'rm -f "$REPORT"' EXIT
    ENGINE=()
    NAMES=()
    for arg in "$@"; do
      case "$arg" in
        --firefox) ENGINE=(--firefox) ;;
        *) NAMES+=("$arg") ;;
      esac
    done
    ONLY=()
    [ ${#NAMES[@]} -gt 0 ] && ONLY=(--only "$(IFS=,; echo "${NAMES[*]}")")
    "$REPO_ROOT/scripts/web/test-site-e2e.sh" "$SITE" "${ENGINE[@]}" --junit "$REPORT" --require-browser "${ONLY[@]}"
    status=$?
    if [ ! -s "$REPORT" ]; then
      echo "tdd.sh: INVALID — the browser suite wrote no report (exit $status)"
      exit 2
    fi
    python3 "$VERDICT" "$EXPECT" "$REPORT"
    exit $?
    ;;

  *) usage ;;
esac
