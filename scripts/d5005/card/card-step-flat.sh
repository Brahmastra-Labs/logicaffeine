#!/usr/bin/env bash
# dcdev-fpga: card-step-uclk.sh for a flat image (the FIM with the accelerator inside it), already on the card over
# JTAG (~/d5005/jtag-load-sof.sh): no partial reconfiguration; set the user clock to USERCLK_MHZ (its low clock half of
# it) through the port's user-clock interface, build the step program against the physical backend, decode, and put
# the PF back on dfl-pci.
#   USERCLK_MHZ=<mhz> card-step-flat.sh <afu sw dir> <program dir> <tokens to generate> <prompt token>...
set -euo pipefail
C="$HOME/d5005/card"
MHZ="${USERCLK_MHZ:?USERCLK_MHZ is the user clock to run at, in MHz}"
SW="$(cd "${1:?usage}" && pwd)"; PROGRAM="$(cd "${2:?usage}" && pwd)"; shift 2
[ "$(lspci -n -s 01:00.0 | awk '{print $3}')" = 8086:bcce ] || { echo "card-step-flat.sh: 01:00.0 is not the OFS FIM's PF" >&2; exit 2; }
[ ! -e /sys/bus/pci/devices/0000:01:00.1 ] || { echo "card-step-flat.sh: the VFs are already enabled; detach first" >&2; exit 2; }
grep -q '^#define LOGOS_D5005_ENGINE 1$' "$SW/logos_d5005.h" || { echo "card-step-flat.sh: $SW is not an engine build's" >&2; exit 2; }
fpgainfo fme 2>&1 | grep -E "Bitstream Id|Pr Interface Id"
sudo -n /usr/local/bin/userclk -B 0x01 -H "$MHZ" -L "$(awk -v m="$MHZ" 'BEGIN { printf "%g", m / 2 }')"
export LOGOS_D5005_VF=0000:01:00.2 LOGOS_USER_CLOCK_MHZ="$MHZ"
cc -std=c99 -O2 -Wall -Werror "-DLOGOS_CORE_MAX_POLLS=${MAX_POLLS:-(1ull << 36)}" -I"$SW" "$SW/logos_step.c" "$SW/logos_hal_pci.c" -lm -o "$C/logos_step"
trap '"$C/scripts/afu-vf.sh" detach' EXIT
"$C/scripts/afu-vf.sh" attach
start=$(date +%s)
status=0
"$C/scripts/afu-vf.sh" exec "$C/logos_step" "$PROGRAM" "$@" || status=$?
echo "card-step-flat.sh: exit $status after $(( $(date +%s) - start )) s at $MHZ MHz"
exit "$status"
