#!/usr/bin/env bash
# Run a host program against the accelerator's RTL: the emitted program (`sw/logos_<program>.c`) and the simulation
# backend built with it, the shell and the co-simulation top under the simulator, the program's output on stdout.
#
#   scripts/d5005/run-cosim.sh <afu dir> receipt [lines per channel] [host lines]
#   scripts/d5005/run-cosim.sh <afu dir> load <image directory> [--self-test]
#   scripts/d5005/run-cosim.sh <afu dir> step [--inject] <program directory> <tokens to generate> <prompt token>...
#
# <afu dir> is a tree `cargo run -p logicaffeine-compile --example d5005_afu -- <dir>` wrote; the arguments after the
# program's name are its own. The simulator is Icarus Verilog (iverilog, iverilog-vpi and vvp: the program and
# sim/logos_hal_vpi.c a VPI module), or with LOGOS_SIM=verilator Verilator (the one scripts/d5005/fetch-verilator.sh
# put in tools/verilator, or $VERILATOR: the program and sim/logos_hal_dpi.c C objects linked into the cosim built with
# LOGOS_DPI), which the engine build's step needs. An engine build's engine is its simulation twin
# (sim/logos_engine_gated.v: hw/rtl/logos_engine.v with its core's clock gated to the user cycles the core steps or reads
# its memories in, which changes nothing the core does and spares the simulator the rest). Exits 0 only when the program
# did. With LOGOS_DDR4_LOG set to a directory, a tree whose channels are DDR4-timed writes there each channel's log of
# what it scheduled; with LOGOS_MAX_POLLS set, the program gives a command up after that many register reads of waiting
# (logos_core.h). With LOGOS_SIM_SEED set under Verilator, every register and memory no reset reaches starts holding
# noise of that seed, as the card's do after a partial reconfiguration, rather than zeros. With LOGOS_COSIM_LOG naming a
# file, everything the simulation writes, both streams, is appended to it as it is written, so a long run can be read
# while it runs and what it did is kept when it is stopped.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
AFU="$(cd "${1:?usage: run-cosim.sh <afu dir> <program> [arguments...]}" && pwd)"
PROGRAM="${2:?usage: run-cosim.sh <afu dir> <program> [arguments...]}"
shift 2
[ -f "$AFU/sw/logos_$PROGRAM.c" ] || { echo "run-cosim.sh: the tree has no program sw/logos_$PROGRAM.c" >&2; exit 2; }
PLUSARGS=()
for a in "$@"; do PLUSARGS+=("+arg=$a"); done
W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT
R="$AFU/hw/rtl"
ENGINE=()
[ -f "$R/logos_card_port.v" ] && ENGINE+=("$R/logos_card_port.v")
# An engine runs as its simulation twin, the same engine with its core's clock gated to the user cycles the core acts in.
if [ -f "$AFU/sim/logos_engine_gated.v" ]; then
  ENGINE+=("$AFU/sim/logos_engine_gated.v")
elif [ -f "$R/logos_engine.v" ]; then
  ENGINE+=("$R/logos_engine.v")
fi
# A divided build's shell steps its compute side at the compute clock it makes from the user clock.
[ -f "$R/logos_compute_clock.v" ] && ENGINE+=("$R/logos_compute_clock.v")
[ -f "$AFU/sim/logos_fp_mac_model.v" ] && ENGINE+=("$AFU/sim/logos_fp_mac_model.v")
FILES=("$AFU/sim/logos_d5005_cosim.v" "$AFU/sim/logos_avmm_memory_model.v" "$AFU/sim/logos_ddr4_channel_model.v"
  "$R/logos_d5005_shell.v" "$R/logos_dcfifo.v" "$R/logos_secded.v" "$R/logos_stream.v" "$R/logos_csr.v"
  "$R/logos_bringup_core.v" ${ENGINE[@]+"${ENGINE[@]}"})
LOG_DIR="${LOGOS_DDR4_LOG:-}"
# The simulation run in the directory first named, its output into run.log; under LOGOS_COSIM_LOG both its streams also
# appended to that file, each through a tee the pipeline waits for.
simulate() {
  local at="$1"
  shift
  if [ -n "${LOGOS_COSIM_LOG:-}" ]; then
    { (cd "$at" && "$@") 2>&1 1>&3 3>&- | tee -a "$LOGOS_COSIM_LOG" >&2; } 3>&1 | tee -a "$LOGOS_COSIM_LOG" > "$W/run.log"
  else
    (cd "$at" && "$@") > "$W/run.log"
  fi
}
POLLS=${LOGOS_MAX_POLLS:+-DLOGOS_CORE_MAX_POLLS=${LOGOS_MAX_POLLS}ull}
case "${LOGOS_SIM:-icarus}" in
  icarus)
    (cd "$W" && iverilog-vpi --name=logos_cosim -DLOGOS_PROGRAM_MAIN=logos_program_main ${POLLS:+"$POLLS"} -I"$AFU/sw" \
      "$AFU/sim/logos_hal_vpi.c" "$AFU/sw/logos_$PROGRAM.c" -lpthread > build.log 2>&1) || { cat "$W/build.log" >&2; exit 2; }
    iverilog -g2012 ${LOG_DIR:+-Plogos_d5005_cosim.DDR4_LOG=1} -o "$W/cosim" "${FILES[@]}"
    simulate "${LOG_DIR:-$W}" vvp -M "$W" -m logos_cosim "$W/cosim" ${PLUSARGS[@]+"${PLUSARGS[@]}"}
    ;;
  verilator)
    V="${VERILATOR:-$ROOT/tools/verilator/bin/verilator}"
    [ -x "$V" ] || { echo "run-cosim.sh: no Verilator at $V (scripts/d5005/fetch-verilator.sh)" >&2; exit 2; }
    INC="$("$V" --getenv VERILATOR_ROOT)/include/vltstd"
    for c in "$AFU/sim/logos_hal_dpi.c" "$AFU/sw/logos_$PROGRAM.c"; do
      cc -std=c11 -O2 -c -I"$INC" -I"$AFU/sw" -I"$AFU/sim" -DLOGOS_PROGRAM_MAIN=logos_program_main ${POLLS:+"$POLLS"} "$c" \
        -o "$W/$(basename "$c" .c).o" >> "$W/build.log" 2>&1 || { cat "$W/build.log" >&2; exit 2; }
    done
    NOISE=()
    [ -n "${LOGOS_SIM_SEED:-}" ] && NOISE=(--x-assign unique --x-initial unique)
    "$V" --binary --timing -DLOGOS_DPI -Wno-fatal -Wno-lint -Wno-style -Wno-UNOPTFLAT --unroll-count 4096 ${NOISE[@]+"${NOISE[@]}"} \
      --top-module logos_d5005_cosim ${LOG_DIR:+-GDDR4_LOG=1} -Mdir "$W/obj" -j 2 -CFLAGS -O3 \
      -LDFLAGS "$W/logos_hal_dpi.o $W/logos_$PROGRAM.o -lpthread -lm" "${FILES[@]}" >> "$W/build.log" 2>&1 \
      || { cat "$W/build.log" >&2; exit 2; }
    SEEDED=()
    [ -n "${LOGOS_SIM_SEED:-}" ] && SEEDED=(+verilator+rand+reset+2 "+verilator+seed+${LOGOS_SIM_SEED}")
    simulate "${LOG_DIR:-$W}" "$W/obj/Vlogos_d5005_cosim" ${SEEDED[@]+"${SEEDED[@]}"} ${PLUSARGS[@]+"${PLUSARGS[@]}"}
    ;;
  *)
    echo "run-cosim.sh: LOGOS_SIM is icarus or verilator, not ${LOGOS_SIM}" >&2
    exit 2
    ;;
esac
grep -v '^logos_cosim: the program exited' "$W/run.log" || true
grep -q '^logos_cosim: the program exited 0$' "$W/run.log"
