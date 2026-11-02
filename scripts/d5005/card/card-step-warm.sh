#!/usr/bin/env bash
# dcdev-fpga: card-step-uclk.sh with a warm-up step before the decode, for an image built before its engine decoded
# from any power-up state (hybrid3b): its core's fold read-port hand-off powers up live and its column feed's gather
# register holds power-up bits until a step reaches a lazily flushed recurrence job and a streamed matvec. So after the
# PR and the clock, one step of the prompt's first token runs (its own image load) and is discarded, then the decode
# runs from a fresh load of the image.
#   USERCLK_MHZ=<mhz> card-step-warm.sh <afu sw dir> <gbs> <program dir> <tokens to generate> <prompt token>...
set -euo pipefail
C="$HOME/d5005/card"
MHZ="${USERCLK_MHZ:?USERCLK_MHZ is the user clock to run at, in MHz}"
SW="$(cd "${1:?usage}" && pwd)"; GBS="${2:?usage}"; PROGRAM="$(cd "${3:?usage}" && pwd)"; shift 3
[ "$(lspci -n -s 01:00.0 | awk '{print $3}')" = 8086:bcce ] || { echo "card-step-warm.sh: 01:00.0 is not the OFS FIM's PF" >&2; exit 2; }
[ ! -e /sys/bus/pci/devices/0000:01:00.1 ] || { echo "card-step-warm.sh: the VFs are already enabled; detach first" >&2; exit 2; }
grep -q '^#define LOGOS_D5005_ENGINE 1$' "$SW/logos_d5005.h" || { echo "card-step-warm.sh: $SW is not an engine build's" >&2; exit 2; }
sha256sum "$GBS"
# A GBS asking for a fixed user clock has fpgaconf set it through the port's user-clock uio, which only root opens.
sudo -n /usr/local/bin/fpgaconf -V -B 0x01 "$GBS"
sudo -n python3 "$HOME/d5005/fme_pr_regs.py" | grep -E "PR_STS|PR_ERR"
sudo -n /usr/local/bin/userclk -B 0x01 -H "$MHZ" -L "$(awk -v m="$MHZ" 'BEGIN { printf "%g", m / 2 }')"
export LOGOS_D5005_VF=0000:01:00.2 LOGOS_USER_CLOCK_MHZ="$MHZ"
cc -std=c99 -O2 -Wall -Werror "-DLOGOS_CORE_MAX_POLLS=${MAX_POLLS:-(1ull << 36)}" -I"$SW" "$SW/logos_step.c" "$SW/logos_hal_pci.c" -lm -o "$C/logos_step"
trap '"$C/scripts/afu-vf.sh" detach' EXIT
"$C/scripts/afu-vf.sh" attach
start=$(date +%s)
"$C/scripts/afu-vf.sh" exec "$C/logos_step" "$PROGRAM" 0 "$2" > /dev/null || echo "card-step-warm.sh: the warm-up step failed" >&2
echo "card-step-warm.sh: warm-up done after $(( $(date +%s) - start )) s" >&2
status=0
"$C/scripts/afu-vf.sh" exec "$C/logos_step" "$PROGRAM" "$@" || status=$?
echo "card-step-warm.sh: exit $status after $(( $(date +%s) - start )) s at $MHZ MHz"
exit "$status"
