#!/usr/bin/env bash
# caravel-submission.sh — assemble a Caravel project's SHUTTLE SUBMISSION directory from its signed-off
# artifacts, in the layout `mpw_precheck` expects.
#
#   scripts/caravel-submission.sh [project] [out_dir]
#       project   a key of codegen_sva::caravel_project (default: dram)
#       out_dir   default: the project's submission directory
#
# WHY. `scripts/caravel-harden.sh` produces a signoff-clean `user_project_wrapper` GDS — every
# mpw_precheck GEOMETRY gate at zero (DRC, LVS, golden sky130A_mr, zeroarea, XOR, routing, antenna).
# That is necessary but NOT sufficient to submit: a shuttle accepts a REPOSITORY, and the precheck
# also runs license / Makefile / defaults / documentation / consistency / GPIO-defines / manifest
# checks against that repo's structure. This script builds that structure so those checks have
# something real to run against, instead of the gap being discovered at order time.
#
# It is deliberately NON-DESTRUCTIVE: it copies the fetched efabless scaffold (tools/caravel_user_project),
# drops the scaffold's `user_proj_example`, installs the hardened wrapper and macro, and lays the project's
# overlay (benchmarks/silicon/caravel/<project>/ — README, pad configuration, bring-up bench) over it. The
# precheck's `defaults` check requires exactly that — a submission whose content differs from the template.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
PROJECT="${1:-dram}"
SCAFFOLD="$ROOT/tools/caravel_user_project"
OL="$ROOT/tools/OpenLane/designs"

[ -d "$SCAFFOLD" ] || { echo "no caravel_user_project scaffold at $SCAFFOLD (fetch it first)"; exit 1; }
EMIT="$ROOT/target/debug/examples/emit_flow_config"
[ -x "$EMIT" ] || cargo build -q -p logicaffeine-compile --example emit_flow_config
for _srcf in "$ROOT/crates/logicaffeine_compile/src/codegen_sva/caravel_project.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/signoff_blocks.rs" \
             "$ROOT/crates/logicaffeine_compile/src/codegen_sva/wpe_caravel.rs" \
             "$ROOT/crates/logicaffeine_compile/examples/emit_flow_config.rs"; do
  if [ "$_srcf" -nt "$EMIT" ]; then
    echo "FATAL: $(basename "$_srcf") is NEWER than the prebuilt flow-config emitter; rebuild it:" >&2
    echo "         cargo build -p logicaffeine-compile --example emit_flow_config" >&2
    exit 4
  fi
done
CP=$("$EMIT" --caravel-project "$PROJECT" --pdk "${PDK:-sky130A}") \
  || { echo "FATAL: Caravel project $PROJECT is unknown or does not fit the vehicle (see above)" >&2; exit 4; }
eval "$CP"
# SUBMISSION_OUT assembles it elsewhere: a staging copy a batch of re-signs lands with its ledgers.
OUT="${2:-${SUBMISSION_OUT:-$ROOT/$CP_SUBMISSION}}"
# WRAPPER_LEDGER_OUT and MACRO_LEDGER_OUT read the pair from the staging ledgers a batch of re-signs filed it in.
LEDGER="${WRAPPER_LEDGER_OUT:-$ROOT/$CP_WRAPPER_LEDGER}"
MLEDGER="${MACRO_LEDGER_OUT:-$ROOT/$CP_MACRO_LEDGER}"
OVERLAY="$ROOT/$CP_OVERLAY"
[ -f "$LEDGER" ] || { echo "FATAL: no wrapper ledger $LEDGER — run scripts/caravel-harden.sh $PROJECT" >&2; exit 2; }
[ -d "$OVERLAY" ] || { echo "FATAL: no submission overlay $CP_OVERLAY" >&2; exit 2; }

# ONLY A SIGNED-OFF PAIR IS SUBMITTED. The README the overlay ships states that the macro and the wrapper
# are clean on every gate; this is the check that makes the statement true of what is assembled.
python3 - "$LEDGER" "$MLEDGER" "$CP_MACRO" <<'PYCLEAN'
import json, sys
wrapper = json.load(open(sys.argv[1]))["user_project_wrapper"]
macro = json.load(open(sys.argv[2]))["blocks"].get(sys.argv[3]) if sys.argv[2] else None
bad = []
if wrapper.get("signoff_clean") is not True:
    bad.append(f"the wrapper is not signoff-clean in {sys.argv[1]}")
if wrapper.get("macro") != sys.argv[3]:
    bad.append(f"the wrapper wraps {wrapper.get('macro')!r}, not {sys.argv[3]!r}")
if macro is None or macro.get("signoff_clean") is not True:
    bad.append(f"{sys.argv[3]} is not recorded signoff-clean in {sys.argv[2]}")
elif macro.get("gds_bytes") != wrapper.get("macro_gds_bytes"):
    bad.append(f"the wrapper embeds a {wrapper.get('macro_gds_bytes')}-byte macro, the ledger signs off {macro.get('gds_bytes')}")
if bad:
    sys.exit("FATAL: " + "; ".join(bad))
PYCLEAN

# The wrapper run to submit = the one our signoff artifact was recorded from. Resolve it by matching the
# recorded GDS size, so we can NEVER assemble a submission out of a different run than the one the
# wrapper ledger vouches for. (That exact drift — a ledger describing a run whose artifacts were gone —
# is what `sky130_provenance_lock` exists to catch.)
WANT=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['user_project_wrapper']['gds_bytes'])" "$LEDGER")
RUN=""
for d in "$OL/$CP_WRAPPER_DESIGN/runs/"*/; do
  g="$d/results/final/gds/user_project_wrapper.gds"
  [ -f "$g" ] || continue
  [ "$(stat -c%s "$g")" = "$WANT" ] && RUN="${d%/}"
done
[ -n "$RUN" ] || { echo "FATAL: no $CP_WRAPPER_DESIGN run has the recorded GDS ($WANT bytes) — re-run scripts/caravel-harden.sh $PROJECT" >&2; exit 2; }
echo "== submission from run: $RUN =="

# The MACRO run is resolved the same race-free way, by the macro GDS the wrapper run embedded. `ls -td |
# head -1` picks the newest MTIME, which is not the same thing: any tool that rewrites a report inside an
# OLD run directory — the IR-drop anchoring pass does exactly that — bumps its mtime and silently makes it
# "newest", pairing a ledger-backed wrapper with whichever macro run was touched last.
MWANT=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['user_project_wrapper']['macro_gds_bytes'])" "$LEDGER")
MRUN=""
for _d in "$OL/$CP_MACRO_DESIGN/runs/"*/; do
  _g="$_d/results/final/gds/$CP_MACRO.gds"
  if [ -f "$_g" ] && [ "$(stat -c %s "$_g")" = "$MWANT" ]; then MRUN="${_d%/}"; break; fi
done
[ -n "$MRUN" ] || { echo "FATAL: no $CP_MACRO_DESIGN run has the embedded macro GDS ($MWANT bytes) — re-run scripts/caravel-macro-harden.sh $PROJECT" >&2; exit 2; }
echo "== macro from run: $MRUN =="

# A precheck run writes root-owned results INTO the submission tree and reads from it the whole
# time, so rebuilding underneath a live one both corrupts that run and leaves undeletable files.
# Refuse rather than half-destroy it.
if sg docker -c "docker ps -q --filter ancestor=efabless/mpw_precheck:latest" 2>/dev/null | grep -q .; then
  echo "FATAL: an mpw_precheck container is running — wait for it, or kill it first." >&2
  exit 3
fi
if [ -d "$OUT" ] && ! rm -rf "$OUT" 2>/dev/null; then
  # Prior precheck output is owned by the container's root; drop it the same way it was made.
  sg docker -c "docker run --rm -v $(dirname "$OUT"):/w efabless/mpw_precheck:latest rm -rf /w/$(basename "$OUT")" >/dev/null 2>&1 || true
  rm -rf "$OUT" 2>/dev/null || true
fi
mkdir -p "$OUT"
cp -r "$SCAFFOLD/." "$OUT/"
rm -f "$OUT/gds/user_proj_example.gds" "$OUT/verilog/rtl/user_proj_example.v" "$OUT/verilog/gl/user_proj_example.v"

# --- our signed-off artifacts, replacing the scaffold defaults ---
install -D -m644 "$RUN/results/final/gds/user_project_wrapper.gds"      "$OUT/gds/user_project_wrapper.gds"
install -D -m644 "$RUN/results/final/verilog/gl/user_project_wrapper.v" "$OUT/verilog/gl/user_project_wrapper.v"
install -D -m644 "$ROOT/benchmarks/silicon/$CP_WRAPPER_RTL"             "$OUT/verilog/rtl/user_project_wrapper.v"
install -D -m644 "$MRUN/results/final/gds/$CP_MACRO.gds"                "$OUT/gds/$CP_MACRO.gds"
install -D -m644 "$MRUN/results/final/lef/$CP_MACRO.lef"                "$OUT/lef/$CP_MACRO.lef"
install -D -m644 "$MRUN/results/final/lib/$CP_MACRO.lib"                "$OUT/lib/$CP_MACRO.lib"
install -D -m644 "$MRUN/results/final/verilog/gl/$CP_MACRO.v"           "$OUT/verilog/gl/$CP_MACRO.v"
install -D -m644 "$ROOT/benchmarks/silicon/$CP_MACRO.v"                 "$OUT/verilog/rtl/$CP_MACRO.v"

# Per-macro signoff statistics the precheck asks for.
for pair in "user_project_wrapper:$RUN" "$CP_MACRO:$MRUN"; do
  name="${pair%%:*}"; rd="${pair#*:}"
  [ -f "$rd/reports/metrics.csv" ] || continue
  install -D -m644 "$rd/reports/metrics.csv" "$OUT/signoff/$name/final_summary_report.csv"
done

# --- the project's own files: README, GPIO power-on modes, bring-up bench ---
# The precheck rejects a README byte-identical to the template, refuses the scaffold's `GPIO_MODE_INVALID`
# placeholders, and a bring-up suite that drives `user_proj_example` tests a design that is not on the die.
cp -r "$OVERLAY/." "$OUT/"
for req in README.md verilog/rtl/user_defines.v verilog/dv/logos_bringup/README.md verilog/dv/logos_bringup/tb_user_project_wrapper.v; do
  [ -f "$OUT/$req" ] || { echo "FATAL: the $PROJECT overlay has no $req" >&2; exit 2; }
done

# Drop the scaffold's example DV/test content — it exercises `user_proj_example`, which is NOT what we
# ship — and the stray macOS metadata file that breaks the SPDX scanner outright.
rm -rf "$OUT/verilog/dv/cocotb" "$OUT/.readthedocs.yaml"
find "$OUT" -name ".DS_Store" -delete 2>/dev/null || true

# NETLIST MANIFEST. `uprj_netlists.v` is what every simulation of this submission elaborates, and the
# scaffold's copy includes `user_proj_example` in BOTH the GL and RTL branches. Rewrite it to name the
# macro the wrapper actually instantiates, so the top cell's `mprj` resolves in either mode.
cat > "$OUT/verilog/rtl/uprj_netlists.v" <<UPRJ
// SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
// SPDX-License-Identifier: MIT
//
// The netlist manifest every simulation of this submission elaborates: the wrapper and the macro it
// instantiates as \`mprj\`, \`$CP_MACRO\`.
\`include "defines.v"
\`define USE_POWER_PINS

\`ifdef GL
    // GL netlists carry no wire declarations.
    \`default_nettype wire
    \`include "gl/user_project_wrapper.v"
    \`include "gl/$CP_MACRO.v"
\`else
    \`include "user_project_wrapper.v"
    \`include "$CP_MACRO.v"
\`endif
UPRJ

# SPDX sweep over the WHOLE submission tree. The scanner rejects the entire submission for ONE missing
# header, so this is a set-level rule rather than a header added at each install site, and it looks at
# the WHOLE file rather than its first lines (the scaffold's defines.v carries its tag further down).
# `$readmemh` files take `//` comments like Verilog does.
find "$OUT" -type f \( -name '*.v' -o -name '*.md' -o -name '*.sha256' -o -name '*.cfg' -o -name '*.hex' \) \
     -not -path "*/precheck_results/*" -not -path "*/.git/*" 2>/dev/null | while read -r f; do
  grep -q 'SPDX-License-Identifier' "$f" && continue
  case "$f" in
    *.md)     pre='<!-- SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
     SPDX-License-Identifier: MIT -->' ;;
    *.sha256|*.cfg)
              pre='# SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
# SPDX-License-Identifier: MIT' ;;
    *)        pre='// SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
// SPDX-License-Identifier: MIT' ;;
  esac
  printf '%s\n%s' "$pre" "$(cat "$f")" > "$f.tmp" && mv "$f.tmp" "$f"
  echo "  SPDX header added to ${f#$OUT/}"
done

# RTL PROVENANCE. Ship the hash of the RTL the macro was actually hardened from, so a consumer (and
# `caravel_bringup_lock`) can prove the GDS in this directory corresponds to the source in the repo.
[ -f "$MRUN/logos_rtl.sha256" ] || { echo "FATAL: $MRUN records no logos_rtl.sha256 — provenance cannot be shipped" >&2; exit 2; }
install -m644 "$MRUN/logos_rtl.sha256" "$OUT/logos_rtl.sha256"
echo "  RTL provenance: $(grep -v '^#' "$OUT/logos_rtl.sha256" | awk '{print $1}' | cut -c1-12)"

# LVS config. The scaffold's lvs_config.json lists the TEMPLATE macro `user_proj_example.v`, so
# device-level LVS would extract OUR macro as an empty black box and the top cell would fail pin
# matching. Point it at our gate-level macro netlist.
python3 - "$OUT" "$CP_MACRO" <<'PYLVS'
import json, sys, os
out, macro = sys.argv[1], sys.argv[2]
p = os.path.join(out, "lvs", "user_project_wrapper", "lvs_config.json")
if os.path.exists(p):
    cfg = json.load(open(p))
    cfg["LVS_VERILOG_FILES"] = [
        f"$UPRJ_ROOT/verilog/gl/{macro}.v",
        "$UPRJ_ROOT/verilog/gl/$TOP_SOURCE.v",
    ]
    json.dump(cfg, open(p, "w"), indent=4)
    print(f"  lvs_config.json -> {macro}")
PYLVS

# External repositories this submission stands on (the precheck reads this verbatim).
mkdir -p "$OUT/third_party"
cat > "$OUT/third_party/used_external_repos.csv" <<'CSV'
repository name,commit hash
efabless/caravel_user_project,scaffold (fetched tarball, see tools/caravel_user_project)
efabless/mpw_precheck,scaffold (fetched tarball, see tools/mpw_precheck)
The-OpenROAD-Project/OpenLane,ff5509f65b17bfa4068d5336495ab1718987ff69
CSV

echo
echo "== $PROJECT submission assembled at $OUT =="
find "$OUT" -maxdepth 2 \( -name "user_project_wrapper.*" -o -name "$CP_MACRO.*" \) | sed 's|^|  |' | sort
echo
echo "Next: run the acceptance battery against it (runs as your user, sets PDK_ROOT, records the result) —"
echo "  scripts/run-mpw-precheck.sh $PROJECT"
