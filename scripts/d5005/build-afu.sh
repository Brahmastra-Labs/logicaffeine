#!/usr/bin/env bash
# Build the D5005 accelerator with Quartus Prime Pro: emit its tree, gate it (every clock crossing a FIFO, the AFU
# elaborating against the FIM's PIM, every large memory a block-RAM wrapper's), compile it, and record the image in
# benchmarks/silicon/d5005/afu_images.json only when its fit, timing and gates pass.
#
#   scripts/d5005/build-afu.sh <work dir> [full72|data64|secded] [--flat] [--floorplan <tcl>] [--user-clock <mhz>] [--divide <n>] [--hybrid <pairs> [--recurrence [--recurrence-width <w>] [--recurrence-lanes <n> [--recurrence-flushes <f> | --recurrence-behind <t>] [--recurrence-credit <lines>] [--delta-norm]] [--vectors [--float-rows <n>] [--vector-passes <n>] [--vector-depth <n>] [--conv-path <lanes>] [--attention [--contexts <n>]]]] [--greedy] [--octets [--octet-lanes <n>]] [--pe-pairs] [--token-pairs] [--slots <n>] [--folds <lanes>] [--hard-float <measurement> | --hard-adds <measurement>]] [--rows <n>] [--function-units <n>] [--engine <header ledger>...]
#
# The line the memory channels carry is d5005_afu's: the ECC lanes as data (the default), the data bits alone, or the
# data bits SECDED-coded in the ECC lanes. With --engine the accelerator is the engine build for the checkpoints the
# header ledgers describe (data64 or secded lines), its compute clock the user clock divided by d5005_afu's default or
# --divide's; with --hybrid a hybrid engine, its matvec jobs on a streaming unit of <pairs> row pairs a channel in the
# user clock (with --recurrence its lazily flushed memories' token-rows on the recurrence unit beside it, --recurrence-width 2 its read-out and flush a binary16 line a clock, --recurrence-lanes <n> a row's value heads on <n> head lanes each on a fast port of its own, --recurrence-flushes <f> each lane's flushing head split over <f> flush units by rows, --recurrence-behind <t> its memories flushed a window of <t> tokens behind, each lane replaying a share of every head's rows a token, --recurrence-credit <lines> the lanes and their coordinator asking up to <lines> lines ahead of those they have taken, --delta-norm each Gated-DeltaNet block's gated norm made on the lanes as its read-outs are, and with --vectors its vector jobs on the vector unit's side beside that, --float-rows <n> its float matrices' inputs held <n> token-rows at a time, --vector-passes <n> a job whose passes stand alone run <n> passes at a time, --vector-depth <n> its registers <n> passes of its deepest job deep, --conv-path <lanes> each convolution job's rows on a path of their own through <lanes> conv lanes, with --attention its attention jobs on the attention side beside that, and --contexts <n> <n> sequencer contexts sharing every unit, each its own core, token-rows and bank of the store; with --greedy its store laid for greedy programs, each row's logits kept as their arg-max; with --octets its Q8_0 matrices taken eight blocks of a row an issue, --octet-lanes <n> an octet's blocks leaving for <n> lanes a row a clock; with --pe-pairs its array's cores in pairs on processing elements in uClk_usr, its user side in uClk_usrDiv2; with --token-pairs each issue of its array's held beat against two token-rows a clock, with --pe-pairs on one set of each core's processing elements in uClk_usr; with --slots <n> every block of its matrices in a slot of a multiple of <n> bytes, its feeds gathering <n> bytes at a time; with --folds <lanes> its step's residual adds and gated products folded into its matrix jobs, its streaming unit folding each sum into its output slot over <lanes> fold lanes; with --hard-float <measurement> its fast side's f32 multiplies and adds on the card's floating-point DSP blocks as the oracle's measurement names them, with --hard-adds <measurement> its adds alone), every unit sized for --rows' token-rows a step (one sequence each, default one), its vector unit's functions on --function-units' units (a divisor of sixteen, default four), the crossing between its two clocks gated on its own. By default the accelerator is a partial-reconfiguration persona built from
# the FIM's PR build template (afu_synth), a GBS fpgaconf loads into the FIM on the card. With --flat it is compiled
# into the FIM itself: the OFS d5005 2024.1-1 FIM's own source (ofs-d5005, its ofs-common and the PIM's
# ofs-platform-afu-bbb at the release's tags, from the archives scripts/d5005/fetch-d5005-tools.sh pins) with its
# floorplan made whole-device (scripts/d5005/flat-fim.sh), built by its build_top.sh with AFU_WITH_PIM naming the
# tree's sources.txt, so the FIM's afu_main instantiates ofs_plat_afu through the PIM in its port: one d5005.sof
# holding the FIM and the accelerator, the accelerator placed over the whole device rather than the persona region,
# loaded over JTAG with no partial reconfiguration; its sources.txt also names the FIM's static exercisers, which the
# release's Stratix 10 afu_main.tcl loads only when no PIM-wrapped AFU is configured. With --floorplan the Tcl file's placement assignments ride in the
# tree as hw/rtl/logos_floorplan.tcl, which Quartus sources in either flow. With --user-clock the AFU JSON asks for
# that user clock, MHz, instead of the engine build's 100.
#
# The gates need yosys, python3 and tools/d5005 (fetch-d5005-tools.sh); the compile needs quartus_sh (Quartus Prime
# Pro 23.4 with Stratix 10 support and its license) on PATH, and --flat the OPAE SDK's afu_synth_setup, packager,
# afu_json_mgr and PACSign on PATH too (an OPAE 2.12 install), which the FIM's build takes from there rather than
# cloning. Where those live in a container, D5005_QUARTUS_RUN is a command that runs its arguments with them on PATH
# and this repository and the work directory at the same paths; the compile runs through it. Without quartus_sh or
# D5005_QUARTUS_RUN the script gates the tree (with --flat also assembles the FIM's source tree) and stops, saying so.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
T="${D5005_TOOLS:-$ROOT/tools/d5005}"
USAGE="usage: build-afu.sh <work dir> [full72|data64|secded] [--flat] [--floorplan <tcl>] [--user-clock <mhz>] [--divide <n>] [--hybrid <pairs> [--recurrence [--recurrence-width <w>] [--recurrence-lanes <n> [--recurrence-flushes <f> | --recurrence-behind <t>] [--recurrence-credit <lines>] [--delta-norm]] [--vectors [--float-rows <n>] [--vector-passes <n>] [--vector-depth <n>] [--conv-path <lanes>] [--attention [--contexts <n>]]]] [--greedy] [--octets [--octet-lanes <n>]] [--pe-pairs] [--token-pairs] [--slots <n>] [--folds <lanes>] [--hard-float <measurement> | --hard-adds <measurement>]] [--rows <n>] [--function-units <n>] [--engine <header ledger>...]"
W="" LINE=full72 FLAT=0
EMIT=()
while [ $# -gt 0 ]; do
  case "$1" in
    --flat) FLAT=1; EMIT+=(--flat) ;;
    --floorplan)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--floorplan "$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"); shift ;;
    full72|data64|secded) LINE="$1" ;;
    --divide)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--divide "$2"); shift ;;
    --user-clock)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--user-clock "$2"); shift ;;
    --hybrid)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--hybrid "$2"); shift ;;
    --recurrence) EMIT+=(--recurrence) ;;
    --recurrence-width)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--recurrence-width "$2"); shift ;;
    --recurrence-lanes)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--recurrence-lanes "$2"); shift ;;
    --recurrence-behind)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--recurrence-behind "$2"); shift ;;
    --recurrence-flushes)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--recurrence-flushes "$2"); shift ;;
    --recurrence-credit)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--recurrence-credit "$2"); shift ;;
    --delta-norm) EMIT+=(--delta-norm) ;;
    --float-rows)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--float-rows "$2"); shift ;;
    --vector-passes)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--vector-passes "$2"); shift ;;
    --vector-depth)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--vector-depth "$2"); shift ;;
    --conv-path)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--conv-path "$2"); shift ;;
    --function-units)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--function-units "$2"); shift ;;
    --rows)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--rows "$2"); shift ;;
    --vectors) EMIT+=(--vectors) ;;
    --greedy) EMIT+=(--greedy) ;;
    --octets) EMIT+=(--octets) ;;
    --octet-lanes)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--octet-lanes "$2"); shift ;;
    --pe-pairs) EMIT+=(--pe-pairs) ;;
    --token-pairs) EMIT+=(--token-pairs) ;;
    --hard-float|--hard-adds)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=("$1" "$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"); shift ;;
    --slots)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--slots "$2"); shift ;;
    --folds)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--folds "$2"); shift ;;
    --attention) EMIT+=(--attention) ;;
    --contexts)
      [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--contexts "$2"); shift ;;
    --engine)
      shift
      [ $# -ge 1 ] || { echo "$USAGE" >&2; exit 2; }
      EMIT+=(--engine)
      for l in "$@"; do EMIT+=("$(cd "$(dirname "$l")" && pwd)/$(basename "$l")"); done
      break ;;
    -*) echo "$USAGE" >&2; exit 2 ;;
    *) if [ -z "$W" ]; then W="$1"; else echo "$USAGE" >&2; exit 2; fi ;;
  esac
  shift
done
[ -n "$W" ] || { echo "$USAGE" >&2; exit 2; }
quartus() {
  if [ -n "${D5005_QUARTUS_RUN:-}" ]; then $D5005_QUARTUS_RUN "$@"; else "$@"; fi
}
have_quartus() { [ -n "${D5005_QUARTUS_RUN:-}" ] || command -v quartus_sh >/dev/null; }
mkdir -p "$W/gates"
W="$(cd "$W" && pwd)"
AFU="$W/afu"
RTL="$AFU/hw/rtl"

(cd "$ROOT" && cargo run -q -p logicaffeine-compile --example d5005_afu -- "$AFU" "$LINE" ${EMIT[@]+"${EMIT[@]}"}) > "$W/emit.txt"
CDC="$ROOT/scripts/d5005/cdc_check.py"
ENGINE=()
DOMAINS="$AFU/hw/logos_d5005_domains.json"
for f in logos_compute_clock.v logos_card_port.v logos_engine.v; do [ -f "$RTL/$f" ] && ENGINE+=("$RTL/$f"); done
# An engine's core, and a hybrid engine's user-clock side, each one clock throughout, are proven so on their own
# hierarchies and boxed as the clock-domain map names them rather than flattened into the shell; the step boundary
# between the shell's compute side and the engine is proven on its own.
BRIDGE=0
grep -q '"logos_engine_bridge"' "$DOMAINS" && BRIDGE=1
"$T/venv/bin/python" "$CDC" shell "$AFU/hw/logos_d5005_domains.json" "$RTL/logos_dcfifo.v" \
  "$RTL/logos_secded.v" "$RTL/logos_stream.v" "$RTL/logos_csr.v" "$RTL/logos_bringup_core.v" ${ENGINE[@]+"${ENGINE[@]}"} \
  "$RTL/logos_d5005_shell.v" | tee "$W/gates/cdc_shell.txt"
"$T/venv/bin/python" "$CDC" fifo logos_dcfifo_mlab "$RTL/logos_dcfifo.v" | tee "$W/gates/cdc_fifo_mlab.txt"
"$T/venv/bin/python" "$CDC" fifo logos_dcfifo_m20k "$RTL/logos_dcfifo.v" | tee "$W/gates/cdc_fifo_m20k.txt"
if [ "$BRIDGE" = 1 ]; then
  "$T/venv/bin/python" "$CDC" related logos_engine_bridge "$RTL/logos_engine.v" | tee "$W/gates/cdc_bridge.txt"
fi
# A paired engine's processing elements in uclk2 (two cores', or a core's two token-rows'): every value out of a pair
# leaves it through uclk's registers.
for PAIR in logos_matvec_pass_x2 logos_matvec_pass_x2t; do
  if grep -q "\"$PAIR\"" "$DOMAINS"; then
    "$T/venv/bin/python" "$CDC" pair "$PAIR" "$RTL/logos_engine.v" | tee "$W/gates/cdc_pair.txt"
  fi
done
"$T/venv/bin/python" "$ROOT/scripts/d5005/sv_check.py" "$T/pr-template" "$RTL" | tee "$W/gates/sv_check.txt"
# Every memory of 64 Kbit or more is a block-RAM wrapper's own: Quartus may build any other in flip-flops, unreported.
python3 "$ROOT/scripts/d5005/ram_check.py" logos_d5005_shell 65536 "$RTL/logos_dcfifo.v" "$RTL/logos_secded.v" \
  "$RTL/logos_stream.v" "$RTL/logos_csr.v" "$RTL/logos_bringup_core.v" ${ENGINE[@]+"${ENGINE[@]}"} "$RTL/logos_d5005_shell.v" \
  | tee "$W/gates/ram_check.txt"
LEDGER="$ROOT/benchmarks/silicon/d5005/afu_images.json"

if [ "$FLAT" = 1 ]; then
  # The FIM's source tree as its release tags hold it, its floorplan whole-device.
  OFS="$W/ofs"
  bash "$ROOT/scripts/d5005/flat-fim.sh" "$OFS"
  FIM="$OFS/ofs-d5005-ofs-2024.1-1"
  BBB="$OFS/ofs-platform-afu-bbb-ofs-2024.1-1"
  echo "build-afu.sh: the accelerator $RTL/sources.txt"
  if ! have_quartus; then
    echo "build-afu.sh: the tree is gated; quartus_sh is not on PATH and D5005_QUARTUS_RUN is not set, so nothing was compiled"
    exit 2
  fi
  for c in afu_synth_setup packager afu_json_mgr PACSign; do
    quartus bash -c "command -v $c" >/dev/null || { echo "build-afu.sh: $c is not on PATH (an OPAE SDK 2.12 install)" >&2; exit 2; }
  done
  quartus bash -c 'cd "$1" && env OFS_ROOTDIR="$1" OFS_PLATFORM_AFU_BBB="$2" AFU_WITH_PIM="$3" ./ofs-common/scripts/common/syn/build_top.sh d5005 "$4"' \
    build "$FIM" "$BBB" "$RTL/sources.txt" "$W/fim"
  python3 "$ROOT/scripts/d5005/record_afu_image.py" --flat "$AFU" "$W/fim" "$W/gates" "$LEDGER"
  exit 0
fi

if ! have_quartus; then
  echo "build-afu.sh: the tree is gated; quartus_sh is not on PATH and D5005_QUARTUS_RUN is not set, so nothing was compiled"
  exit 2
fi
export OPAE_PLATFORM_ROOT="$T/pr-template"
python3 "$T/opae-sdk/platforms/platmgr/tools/afu_synth_setup.py" --sources "$RTL/sources.txt" --force "$W/build"
quartus bash -c 'cd "$1" && OPAE_PLATFORM_ROOT="$2" "$2/bin/afu_synth"' build "$W/build" "$OPAE_PLATFORM_ROOT"
mkdir -p "$W/build/hw/rtl"
cp "$RTL/logos_d5005.json" "$W/build/hw/rtl/"
python3 "$ROOT/scripts/d5005/record_afu_image.py" "$W/build" "$W/gates" "$LEDGER"
