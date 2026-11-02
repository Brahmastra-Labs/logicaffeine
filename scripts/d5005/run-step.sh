#!/usr/bin/env bash
# Run a model on the card's engine build: configure the accelerator image, build the step program against the backend
# this host opens the accelerator through, and decode greedily from a prompt, the program's JSON on stdout.
#
#   scripts/d5005/run-step.sh <afu dir> <image> <program directory> <tokens to generate> <prompt token>...
#
# <afu dir> is the engine build's tree (`d5005_afu <dir> data64|secded --engine <header ledger>...`), <image> its
# persona GBS, which fpgaconf loads into the FIM, or its flat SOF, already on the card over JTAG; the program directory
# is what `cargo run --release -p logicaffeine-compile --example d5005_afu -- --program <gguf> <dir>` wrote for the
# checkpoint. With LOGOS_D5005_VF set the program reaches the accelerator through that VF (afu-vf.sh attaches it once the
# image is on, and detaches it after): under VFIO with an IOMMU, or as root through the physical backend without one.
# A persona whose AFU JSON fixes its user clock has fpgaconf set that clock through the port's user-clock interface, a
# uio device only root opens, so fpgaconf runs as root (through sudo when this script is not). With
# LOGOS_D5005_USERCLK_MHZ the user clock is then set to it, its low clock to half of it, on the bus of the PF at
# $LOGOS_D5005_PF (default 0000:01:00.0) — for an image whose timing closes only below the clock it asks for — and the
# step program is told the clock. Exits as the step program does: 0 only when every token was decoded with no fault.
set -euo pipefail
USAGE="usage: run-step.sh <afu dir> <image> <program directory> <tokens to generate> <prompt token>..."
[ $# -ge 5 ] || { echo "$USAGE" >&2; exit 2; }
AFU="$(cd "$1" && pwd)"
GBS="$2"
PROGRAM="$(cd "$3" && pwd)"
shift 3
grep -q '^#define LOGOS_D5005_ENGINE 1$' "$AFU/sw/logos_d5005.h" || { echo "run-step.sh: $AFU is not an engine build" >&2; exit 2; }
command -v cc >/dev/null || { echo "run-step.sh: cc is not on PATH" >&2; exit 2; }
W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT
as_root() { if [ "$(id -u)" = 0 ]; then "$@"; else sudo -n "$@"; fi; }
case "$GBS" in
  *.sof) echo "run-step.sh: $GBS is a flat image, on the card once it is loaded over JTAG" >&2 ;;
  *) command -v fpgaconf >/dev/null || { echo "run-step.sh: fpgaconf is not on PATH" >&2; exit 2; }
     as_root "$(command -v fpgaconf)" "$GBS" ;;
esac
if [ -n "${LOGOS_D5005_USERCLK_MHZ:-}" ]; then
  command -v userclk >/dev/null || { echo "run-step.sh: userclk is not on PATH" >&2; exit 2; }
  PF="${LOGOS_D5005_PF:-0000:01:00.0}"
  as_root "$(command -v userclk)" -B "0x${PF:5:2}" -H "$LOGOS_D5005_USERCLK_MHZ" \
    -L "$(awk -v m="$LOGOS_D5005_USERCLK_MHZ" 'BEGIN { printf "%g", m / 2 }')"
  export LOGOS_USER_CLOCK_MHZ="$LOGOS_D5005_USERCLK_MHZ"
fi
VFS="$(dirname "$0")/afu-vf.sh"
if [ -n "${LOGOS_D5005_VF:-}" ]; then
  trap '"$VFS" detach; rm -rf "$W"' EXIT
  "$VFS" attach
fi
"$VFS" build "$AFU" step "$W/logos_step"
status=0
"$VFS" exec "$W/logos_step" "$PROGRAM" "$@" || status=$?
exit "$status"
