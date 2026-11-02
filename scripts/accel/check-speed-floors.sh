#!/usr/bin/env bash
#
# check-speed-floors.sh — measure the compiled lane on this GPU now and hold every rate to its speed floor.
#
# benchmarks/results/speed_floors.json keeps a floor five per cent under the best of every speed on record
# (crates/logicaffeine_tests/tests/speed_floor_lock.rs). The lock reads the committed ledgers; this runner checks a
# change before its ledger is re-recorded: it runs the bring-up scoreboard (scripts/accel/model_scoreboard.py) into
# logs/speed-floors/<tag>.json, then runs speed_floor_lock with that measurement read in the scoreboard's place
# (LOGOS_SPEED_LEDGERS), so each rate and each rate over llama.cpp (and over each engine named) is judged against its
# floor, and every floor the run did not measure is named.
#
# The GPU must be the run's alone: rates measured beside another GPU job are not the rates the floors were set from.
#
# Usage: scripts/accel/check-speed-floors.sh [--models m1,m2] [--engines vllm,exllamav3] [--repeats N]
#   (defaults: every scoreboard model, no serving engines, 3 repeats)
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

MODELS=()
ENGINES="--engines="
REPEATS=3
while [ $# -gt 0 ]; do
  case "$1" in
    --models) MODELS=(--models "$2"); shift ;;
    --engines) ENGINES="--engines=$2"; shift ;;
    --repeats) REPEATS="$2"; shift ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done

TAG="$(date -u +%Y%m%d-%H%M%S)"
mkdir -p logs/speed-floors
MEASURED="logs/speed-floors/scoreboard-$TAG.json"

echo "== measuring into $MEASURED"
if ! python3 scripts/accel/model_scoreboard.py --tag "floors-$TAG" "${MODELS[@]}" "$ENGINES" --repeats "$REPEATS" --ledger "$MEASURED"; then
  echo "the scoreboard run failed; nothing judged" >&2
  exit 1
fi

echo "== judging $MEASURED against benchmarks/results/speed_floors.json"
LOGOS_SPEED_LEDGERS="benchmarks/results/models/rtx3060_scoreboard.json=$MEASURED" \
  exec cargo nextest run -p logicaffeine-tests --test speed_floor_lock --no-capture -E 'test(every_measurement_on_record_holds_its_floor)'
