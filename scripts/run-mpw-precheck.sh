#!/usr/bin/env bash
# run-mpw-precheck.sh — run the efabless/ChipFoundry `mpw_precheck` acceptance battery on a Caravel
# project's assembled submission and record the result into the project's precheck ledger.
#
#   scripts/run-mpw-precheck.sh [project]      # a key of codegen_sva::caravel_project (default: dram)
#
# The image does not carry the precheck code: efabless's own docker-mount.sh mounts the fetched
# repository at /opt/mpw_precheck and passes the submission and PDK at their HOST paths. Two things
# that helper leaves out are required here, both measured:
#   * PDK_ROOT. The PDK's .magicrc resolves its tech file from $PDK_ROOT and otherwise falls back to
#     $HOME/.volare/...; with neither set, Magic loaded no technology, the XOR check never wrote its
#     erased GDS, and the run reported "XOR CHECK FILE NOT FOUND" as a failed check (2026-09-23).
#   * The user. Run as root, the precheck writes root-owned results into the submission tree.
logos_run_mpw_precheck_main() {
  set -euo pipefail
  cd "$(dirname "$0")/.."
  local ROOT SUB SUBDIR PDKR PDK_NAME LOG EMIT CP
  ROOT=$(pwd)
  EMIT="$ROOT/target/debug/examples/emit_flow_config"
  [ -x "$EMIT" ] || cargo build -q -p logicaffeine-compile --example emit_flow_config
  CP=$("$EMIT" --caravel-project "${1:-dram}" --pdk "${PDK:-sky130A}") \
    || { echo "FATAL: Caravel project ${1:-dram} is unknown or does not fit the vehicle (see above)" >&2; exit 4; }
  eval "$CP"
  # SUBMISSION_OUT checks a submission assembled elsewhere, a staging copy a batch of re-signs lands with its ledgers; the
  # verdict names the project's own directory, where it lands.
  SUBDIR="${SUBMISSION_OUT:-$CP_SUBMISSION}"
  [ -d "$SUBDIR" ] || { echo "FATAL: no submission at $SUBDIR — run scripts/caravel-submission.sh $CP_KEY" >&2; exit 2; }
  SUB=$(cd "$SUBDIR" && pwd)
  PDKR="${PDK_ROOT:-$HOME/.ciel}"
  PDK_NAME="${PDK:-sky130A}"
  [ -d "$PDKR/$PDK_NAME" ] || { echo "FATAL: no PDK at $PDKR/$PDK_NAME" >&2; exit 2; }
  [ -f "$ROOT/tools/mpw_precheck/mpw_precheck.py" ] || { echo "FATAL: tools/mpw_precheck is not fetched" >&2; exit 2; }
  LOG="$ROOT/logs/mpw_precheck_${CP_KEY}_$(date +%Y%m%d_%H%M%S).log"
  docker run --rm \
    -v "$ROOT/tools/mpw_precheck:/opt/mpw_precheck" -v "$SUB:$SUB" -v "$PDKR:$PDKR" \
    -e INPUT_DIRECTORY="$SUB" -e PDK_PATH="$PDKR/$PDK_NAME" -e PDK_ROOT="$PDKR" -e PDK="$PDK_NAME" \
    -u "$(id -u):$(id -g)" efabless/mpw_precheck:latest \
    bash -c "cd /opt/mpw_precheck && python3 mpw_precheck.py --input_directory $SUB --pdk_path $PDKR/$PDK_NAME" \
    > "$LOG" 2>&1 || true
  echo "== mpw_precheck log: $LOG"
  grep -E "\{\{(FAILURE|SUCCESS)\}\}" "$LOG" || true
  # PRECHECK_LEDGER_OUT files the verdict in another ledger — a staging copy a batch of re-signs lands from at once.
  python3 "$ROOT/scripts/record-mpw-precheck.py" "$LOG" "$SUB" "${PRECHECK_LEDGER_OUT:-$ROOT/$CP_PRECHECK_LEDGER}" "$ROOT/$CP_SUBMISSION"
}
logos_run_mpw_precheck_main "$@"; exit $?
