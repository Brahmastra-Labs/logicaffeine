#!/usr/bin/env bash
# suite-guard.sh — the brakes every test run on a shared box goes through.
#
# Sourced by scripts/run-all-tests-fast.sh and scripts/test/tdd.sh, so an ad-hoc RED/GREEN run
# obeys exactly the same rules as the full baseline:
#
#   suite_guard_brakes          nice/ionice this shell and cap nextest/cargo concurrency
#   suite_guard_require_nextest refuse without cargo-nextest
#   suite_guard_refuse_overlap  refuse while another cargo test / nextest run is in progress
#   suite_guard_refuse_heavy    refuse while a job the heavy-job check names runs
#
# Each check prints why and returns 1; the caller decides to exit. Locked by
# crates/logicaffeine_tests/tests/tdd_verdict_lock.rs (both callers must source this file).

# --- load governor -----------------------------------------------------------
# The suite must never starve the interactive session that launched it. On
# 2026-09-09 three consecutive baseline runs were DESTROYED mid-flight by this
# exact failure: nextest scheduled one test process per core while the e2e tests
# each shelled out to `cargo build -j<ncpu>`, so a single session scope burned
# 1h44m52s of CPU in 4m19s of wall clock — 24x oversubscription on 32 cores.
# Trivial tests (core_eval_if_false) took 39-85s. The SSH transport starved, the
# VS Code client gave up, and systemd tore down the whole user slice on logout,
# SIGKILLing every rustc/cargo/tmux pane with it. Nothing was wrong with the
# tests; the schedule was wrong.
#
# Two independent brakes, both overridable for a deliberately-quiet bench run:
#
#   1. nice/ionice the whole run. Every cargo, rustc, and test child inherits it.
#      When the box is idle this costs NOTHING (nice only matters under
#      contention) — but when you are typing, the shell and VS Code win the CPU.
#   2. Cap the two multiplying concurrency knobs at 3/4 of the cores each, so
#      nextest's process count times cargo's job count cannot exceed the box.
suite_guard_brakes() {
  renice -n "${LOGOS_TEST_NICE:-15}" -p $$ >/dev/null 2>&1 || true
  ionice -c2 -n7 -p $$ >/dev/null 2>&1 || true

  local _ncpu
  _ncpu="$( (nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 8) )"
  : "${NEXTEST_TEST_THREADS:=$(( _ncpu * 3 / 4 > 0 ? _ncpu * 3 / 4 : 1 ))}"
  : "${CARGO_BUILD_JOBS:=$(( _ncpu * 3 / 4 > 0 ? _ncpu * 3 / 4 : 1 ))}"
  export NEXTEST_TEST_THREADS CARGO_BUILD_JOBS
}

suite_guard_require_nextest() {
  if ! cargo nextest --version >/dev/null 2>&1; then
    echo "ERROR: cargo-nextest not found. Install with:" >&2
    echo '  curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C "$HOME/.cargo/bin"' >&2
    return 1
  fi
}

# --- one suite at a time -----------------------------------------------------
# Never overlap with another test run (shared target dir, shared machine). Detect the
# actual cargo / cargo-nextest PROCESS by executable name (`pgrep -x`), NOT by a
# command-line substring. The old `pgrep -f 'cargo(-nextest)? (test|nextest)'` also
# matched any innocent process whose *command line mentioned* those words — a monitoring
# `pgrep`, a launcher, this very check — so the script refused to start against its own
# watcher shells and wedged. `cargo-nextest` stays alive for a whole nextest run (build +
# run), so detecting it covers the common case; a plain `cargo test` is caught by reading
# the cargo process's own argv. Our own PID and shell are never `cargo`, so no self-match.
suite_guard_refuse_overlap() {
  local overlapping="" _pid _args
  if pgrep -x cargo-nextest >/dev/null 2>&1; then
    overlapping="$(pgrep -x -a cargo-nextest 2>/dev/null)"
  else
    for _pid in $(pgrep -x cargo 2>/dev/null); do
      [ "$_pid" = "$$" ] && continue
      _args="$(tr '\0' ' ' < "/proc/$_pid/cmdline" 2>/dev/null)"
      case " $_args " in
        *" test "*|*" nextest "*) overlapping="$_args"; break ;;
      esac
    done
  fi
  if [ -n "$overlapping" ]; then
    echo "ERROR: another cargo test / nextest run appears to be in progress:" >&2
    echo "$overlapping" >&2
    echo "Refusing to start a second suite. Wait for it to finish." >&2
    return 1
  fi
}

# --- and one HEAVY NON-CARGO job at a time -----------------------------------
# The guard above only knows about cargo. This box also runs place-and-route, synthesis and
# emulation, and those are just as capable of ruining a baseline — with a worse failure mode,
# because the suite does not error, it produces a number that LOOKS real. `bind_address_lock` is
# already serialized into its own nextest group so it does not compete, and still lost its 180s
# budget beside an `ours_flow` place-and-route: serializing against cargo cannot help when the
# competition is not cargo, and widening the budget would only delay the same false red. The fix is
# to not run beside one. The check `$1` (scripts/test/heavy-jobs.sh) names every such job, paused
# containers and frozen processes excepted (`heavy_jobs_lock`); each caller names it at its call.
suite_guard_refuse_heavy() {
  local heavy
  if ! heavy="$("$1")"; then
    echo "ERROR: a heavy non-cargo job is running and would contend with the suite:" >&2
    echo "$heavy" >&2
    echo "Refusing to start: a baseline taken beside place-and-route, synthesis or emulation is" >&2
    echo "not a baseline — it fails load-sensitive tests that pass in isolation and the result" >&2
    echo "cannot be told apart from a real regression. Wait for it, or stop it deliberately." >&2
    return 1
  fi
}
