#!/usr/bin/env bash
# dcdev-fpga: card-profile-uclk.sh for a flat image (the FIM with the accelerator inside it), already on the card over
# JTAG (~/d5005/jtag-load-sof.sh): no partial reconfiguration; set the user clock to USERCLK_MHZ (its low clock half of
# it) through the port's user-clock interface, build the build's step program against the physical backend, run the
# prompt with its last position a job at a time from the job list given (LOGOS_PROFILE: d5005_afu --trace-store's
# jobs.txt), each job's cycles on stdout, and put the PF back on dfl-pci.
#   USERCLK_MHZ=<mhz> card-profile-flat.sh <afu sw dir> <program dir> <job list> <prompt token>...
set -euo pipefail
C="$HOME/d5005/card"
MHZ="${USERCLK_MHZ:?USERCLK_MHZ is the user clock to run at, in MHz}"
SW="$(cd "${1:?usage}" && pwd)"; PROGRAM="$(cd "${2:?usage}" && pwd)"; JOBS="$(readlink -f "${3:?usage}")"; shift 3
[ "$(lspci -n -s 01:00.0 | awk '{print $3}')" = 8086:bcce ] || { echo "card-profile-flat.sh: 01:00.0 is not the OFS FIM's PF" >&2; exit 2; }
[ ! -e /sys/bus/pci/devices/0000:01:00.1 ] || { echo "card-profile-flat.sh: the VFs are already enabled; detach first" >&2; exit 2; }
grep -q '^#define LOGOS_D5005_ENGINE 1$' "$SW/logos_d5005.h" || { echo "card-profile-flat.sh: $SW is not an engine build's" >&2; exit 2; }
[ -s "$JOBS" ] || { echo "card-profile-flat.sh: no job list at $JOBS" >&2; exit 2; }
fpgainfo fme 2>&1 | grep -E "Bitstream Id|Pr Interface Id"
sudo -n /usr/local/bin/userclk -B 0x01 -H "$MHZ" -L "$(awk -v m="$MHZ" 'BEGIN { printf "%g", m / 2 }')" | grep -A2 "Approximate" | tail -2
export LOGOS_D5005_VF=0000:01:00.2 LOGOS_USER_CLOCK_MHZ="$MHZ"
cc -std=c99 -O2 -Wall -Werror "-DLOGOS_CORE_MAX_POLLS=${MAX_POLLS:-(1ull << 36)}" -I"$SW" "$SW/logos_step.c" "$SW/logos_hal_pci.c" -lm -o "$C/logos_step"
trap '"$C/scripts/afu-vf.sh" detach' EXIT
"$C/scripts/afu-vf.sh" attach
"$C/scripts/afu-vf.sh" exec env LOGOS_PROFILE="$JOBS" "$C/logos_step" "$PROGRAM" 0 "$@"
