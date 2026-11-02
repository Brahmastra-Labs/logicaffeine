#!/usr/bin/env bash
# check-wpe-psram-timing.sh — close the whole-program engine's PSRAM read window through its hardened Caravel
# macro, and record the result.
#
#   scripts/check-wpe-psram-timing.sh <macro_run_dir> [out.json]     # default: benchmarks/silicon/wpe_psram_timing.json
#
# WHY. A PSRAM read leaves the macro on SCLK, crosses Caravel's harness and the board, waits the part's tACLK, and
# comes back as SIO data that must reach its capture flop by the edge the half SCLK period puts it on.
# `WpeCaravel::check_harness` budgets everything OUTSIDE the macro (efabless's characterized harness paths, the
# board, the part); the macro's own share — the SCLK flop to its port, the SIO ports to their capture flops, and the
# clock tree between them — exists only once the macro is placed and routed. No signoff gate sees it: the macro's
# SDC treats its GPIO ports as asynchronous. This runs OpenSTA on the routed netlist at the slowest corner over the
# max-RC parasitics with the SIO data constrained to arrive the outside round trip after SCLK leaves, so the reported
# slack is the read window's real margin.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
RUN="${1:?usage: check-wpe-psram-timing.sh <macro_run_dir> [out.json]}"
RUN="$(cd "$RUN" && pwd)"
OUT="${2:-$ROOT/benchmarks/silicon/wpe_psram_timing.json}"
IMG="ghcr.io/the-openroad-project/openlane:ff5509f65b17bfa4068d5336495ab1718987ff69-amd64"
CFG="$RUN/config.tcl"
[ -f "$CFG" ] || { echo "FATAL: no config.tcl in $RUN" >&2; exit 2; }
tclval() { sed -n "s/.*set ::env($1) \"\?\([^\"]*\)\"\?.*/\1/p" "$CFG" | head -1; }
DESIGN=$(tclval DESIGN_NAME)
[ "$DESIGN" = "wpe_user_project" ] || { echo "FATAL: $RUN is a $DESIGN run, not wpe_user_project" >&2; exit 2; }
LIB=$(tclval LIB_SLOWEST)
NL="$RUN/results/final/verilog/gl/$DESIGN.v"
SPEF="$RUN/results/final/spef/multicorner/$DESIGN.max.spef"
GDS="$RUN/results/final/gds/$DESIGN.gds"
for f in "$LIB" "$NL" "$SPEF" "$GDS"; do [ -f "$f" ] || { echo "FATAL: missing $f" >&2; exit 2; }; done

EMIT="$ROOT/target/debug/examples/emit_wpe_caravel"
[ -x "$EMIT" ] || cargo build -q -p logicaffeine-compile --example emit_wpe_caravel
if [ "$ROOT/crates/logicaffeine_compile/src/codegen_sva/wpe_caravel.rs" -nt "$EMIT" ]; then
  echo "FATAL: wpe_caravel.rs is NEWER than the prebuilt emit_wpe_caravel; rebuild it:" >&2
  echo "         cargo build -p logicaffeine-compile --example emit_wpe_caravel" >&2
  exit 4
fi
TCL=$(mktemp "$ROOT/logs/wpe-psram-sta.XXXXXX.tcl")
trap 'rm -f "$TCL"' EXIT
"$EMIT" --psram-read-sta "$LIB" "$NL" "$SPEF" > "$TCL"

echo "== PSRAM read window through $RUN (slowest corner, max RC) =="
STA_OUT=$(sg docker -c "docker run --rm --user $(id -u):$(id -g) -v $HOME:$HOME -v $ROOT:$ROOT $IMG sta -no_splash -exit $TCL" 2>&1) || {
  printf '%s\n' "$STA_OUT" | grep -v '^Warning' | tail -20 >&2
  echo "FATAL: OpenSTA failed on the read-window script ($TCL)" >&2
  exit 3
}
SCLK=$(printf '%s\n' "$STA_OUT" | sed -n 's/^SCLK_ARRIVAL_NS //p' | tail -1)
SLACK=$(printf '%s\n' "$STA_OUT" | sed -n 's/^SIO_CAPTURE_SLACK_NS //p' | tail -1)
if [ -z "$SCLK" ] || [ -z "$SLACK" ]; then
  printf '%s\n' "$STA_OUT" | tail -30 >&2
  echo "FATAL: OpenSTA reported no read-window result" >&2
  exit 3
fi

python3 - "$OUT" "$ROOT" "$RUN" "$GDS" "$LIB" "$SPEF" "$SCLK" "$SLACK" <<'PY'
import json, os, sys
out, root, run, gds, lib, spef, sclk, slack = sys.argv[1:9]
rel = lambda p: os.path.relpath(p, root)
doc = {
    "meta": {
        "check": "the PSRAM read window through the hardened macro (scripts/check-wpe-psram-timing.sh): SIO data "
                 "constrained to arrive the harness + board + tACLK round trip after SCLK's latest arrival at its port, "
                 "captured at the edge the half SCLK period puts it on (codegen_sva::wpe_caravel::wpe_psram_read_sta_tcl)",
    },
    "run": rel(run),
    "gds_bytes": os.path.getsize(gds),
    "corner_lib": os.path.basename(lib),
    "spef": os.path.basename(spef),
    "sclk_arrival_ns": float(sclk),
    "sio_capture_slack_ns": float(slack),
    "read_window_closed": float(slack) >= 0.0,
}
json.dump(doc, open(out, "w"), indent=2)
open(out, "a").write("\n")
print(f"  SCLK leaves its port at {float(sclk):.3f} ns; SIO capture slack {float(slack):.3f} ns -> "
      f"{'CLOSED' if doc['read_window_closed'] else 'VIOLATED'}")
sys.exit(0 if doc["read_window_closed"] else 1)
PY
