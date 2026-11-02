#!/usr/bin/env bash
# dcdev-fpga: card-step-uclk.sh's sequence with logos_probe's reads: partially reconfigure (the persona out of reset at
# its metadata's clock), set the user clock to USERCLK_MHZ (default 40), with PORT_RESET=1 reset the port at that clock,
# then build logos_probe against the physical backend, decode, and put the PF back on dfl-pci.
#   card-probe-uclk.sh <afu sw dir> <gbs> <program dir> <probe dir> <prompt token>...
set -euo pipefail
C="$HOME/d5005/card"
MHZ="${USERCLK_MHZ:-40}"
SW="$(cd "${1:?usage}" && pwd)"; GBS="${2:?usage}"; PROGRAM="$(cd "${3:?usage}" && pwd)"; PROBE="$(cd "${4:?usage}" && pwd)"; shift 4
[ "$(lspci -n -s 01:00.0 | awk '{print $3}')" = 8086:bcce ] || { echo "card-probe-uclk.sh: 01:00.0 is not the OFS FIM's PF" >&2; exit 2; }
[ ! -e /sys/bus/pci/devices/0000:01:00.1 ] || { echo "card-probe-uclk.sh: the VFs are already enabled; detach first" >&2; exit 2; }
sha256sum "$GBS"
sudo -n /usr/local/bin/fpgaconf -V -B 0x01 "$GBS"
sudo -n /usr/local/bin/userclk -B 0x01 -H "$MHZ" -L "$(awk -v m="$MHZ" 'BEGIN { printf "%g", m / 2 }')" | grep -A2 "Approximate" | tail -2
if [ "${PORT_RESET:-0}" = 1 ]; then
  cc -std=c99 -O2 -Wall -Werror "$C/logos_port_reset.c" -o "$C/logos_port_reset"
  sudo -n "$C/logos_port_reset" /dev/dfl-port.0
fi
export LOGOS_D5005_VF=0000:01:00.2 LOGOS_USER_CLOCK_MHZ="$MHZ"
cc -std=c99 -O2 -Wall -Werror "-DLOGOS_CORE_MAX_POLLS=(1ull << 36)" -I"$SW" "$C/logos_probe.c" "$SW/logos_hal_pci.c" -lm -o "$C/logos_probe"
trap '"$C/scripts/afu-vf.sh" detach' EXIT
"$C/scripts/afu-vf.sh" attach
"$C/scripts/afu-vf.sh" exec env LOGOS_PROBE="$PROBE" "$C/logos_probe" "$PROGRAM" "${GENERATE:-2}" "$@"
