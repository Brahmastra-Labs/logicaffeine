#!/usr/bin/env bash
# Put the D5005 accelerator on the PCI function a host program opens, or take it back, and build and run a host program
# against the backend that opens it. On the OFS FIM the PF/VF mux routes PF0's VF1 to the PR region's AFU (VF0 is the
# FIM's loopback exerciser, VF2 its HSSI interface), but the DFL driver enables only as many VFs as ports it released,
# one, which is VF0. attach therefore runs once the image is configured: it moves the PF from dfl-pci to pci-pf-stub and
# enables every VF with no driver probed. With an IOMMU it binds the AFU's VF to vfio-pci for the calling user, and a
# program opens it through OPAE's vfio plugin (sw/logos_hal_opae.c); with none (VT-d off) it leaves the VF to no
# driver, and a program run as root drives its PCI function directly (sw/logos_hal_pci.c). detach unbinds the VF,
# disables the VFs and puts the PF back on dfl-pci.
#
#   scripts/d5005/afu-vf.sh attach|detach
#   scripts/d5005/afu-vf.sh hal
#   scripts/d5005/afu-vf.sh build <afu dir> <program> <executable>
#   scripts/d5005/afu-vf.sh exec <executable> [argument...]
#   scripts/d5005/afu-vf.sh clock <gbs>
#
# hal names the backend a program opens the accelerator through: $LOGOS_D5005_HAL when set, else with $LOGOS_D5005_VF
# set the one this host's VF takes (opae with an IOMMU, pci without), else opae (the port on the PF, under the DFL
# driver). build compiles <afu dir>/sw/logos_<program>.c against that backend; exec runs a program built so, as root
# for the physical backend with LOGOS_USER_CLOCK_MHZ and LOGOS_D5005_VF passed through. A VF has no user-clock
# interface, so a program reaching it takes the user clock from LOGOS_USER_CLOCK_MHZ: clock prints the one a persona's
# GBS has fpgaconf set (its metadata's clock-frequency-high).
#
# Needs root through sudo, the PF at $LOGOS_D5005_PF (default 0000:01:00.0) and the AFU's VF at $LOGOS_D5005_VF
# (default 0000:01:00.2), the kernel's pci-pf-stub, and with an IOMMU OPAE's opaevfio in $OPAE_ADMIN (default
# /opt/opae-admin/bin); build needs cc, and libopae-c for the OPAE backend; clock needs python3.
set -euo pipefail
ADMIN="${OPAE_ADMIN:-/opt/opae-admin/bin}"
PF="${LOGOS_D5005_PF:-0000:01:00.0}"
VF="${LOGOS_D5005_VF:-0000:01:00.2}"
P="/sys/bus/pci/devices/$PF"
usage() {
  echo "usage: afu-vf.sh attach|detach|hal | build <afu dir> <program> <executable> | exec <executable> [argument...] | clock <gbs>" >&2
  exit 2
}
vf_hal() {
  local groups=(/sys/kernel/iommu_groups/*)
  if [ -e "${groups[0]}" ]; then echo opae; else echo pci; fi
}
hal() {
  if [ -n "${LOGOS_D5005_HAL:-}" ]; then
    case "$LOGOS_D5005_HAL" in opae|pci) echo "$LOGOS_D5005_HAL" ;; *) echo "afu-vf.sh: LOGOS_D5005_HAL is opae or pci, not $LOGOS_D5005_HAL" >&2; exit 2 ;; esac
  elif [ -n "${LOGOS_D5005_VF:-}" ]; then
    vf_hal
  else
    echo opae
  fi
}
driver_of() {
  if [ -e "/sys/bus/pci/devices/$1/driver" ]; then basename "$(readlink -f "/sys/bus/pci/devices/$1/driver")"; fi
}
put() { echo "$1" | sudo tee "$2" > /dev/null; }
case "${1:-}" in
  hal) hal ;;
  attach)
    [ "$(driver_of "$PF")" = dfl-pci ] || { echo "afu-vf.sh: $PF is on '$(driver_of "$PF")', not dfl-pci: configure the image, or detach first" >&2; exit 2; }
    backend="$(vf_hal)"
    [ "$backend" = pci ] || [ -x "$ADMIN/opaevfio" ] || { echo "afu-vf.sh: no $ADMIN/opaevfio (OPAE's admin tools)" >&2; exit 2; }
    sudo modprobe pci-pf-stub
    put "$PF" /sys/bus/pci/drivers/dfl-pci/unbind
    put pci-pf-stub "$P/driver_override"
    put "$PF" /sys/bus/pci/drivers/pci-pf-stub/bind
    put 0 "$P/sriov_drivers_autoprobe"
    put "$(cat "$P/sriov_totalvfs")" "$P/sriov_numvfs"
    [ -e "/sys/bus/pci/devices/$VF" ] || { echo "afu-vf.sh: enabling the PF's VFs made no $VF" >&2; exit 2; }
    if [ "$backend" = opae ]; then
      sudo "$ADMIN/opaevfio" -i "$VF" -u "$(id -un)"
      echo "afu-vf.sh: the accelerator is $VF under vfio-pci, for $(id -un)"
    else
      echo "afu-vf.sh: the accelerator is $VF with no driver bound (no IOMMU: a program run as root drives it through sw/logos_hal_pci.c)"
    fi
    ;;
  detach)
    if [ -e "/sys/bus/pci/devices/$VF" ] && [ "$(driver_of "$VF")" = vfio-pci ]; then sudo "$ADMIN/opaevfio" -r "$VF"; fi
    if [ "$(driver_of "$PF")" = pci-pf-stub ]; then
      put 0 "$P/sriov_numvfs"
      put 1 "$P/sriov_drivers_autoprobe"
      put "$PF" /sys/bus/pci/drivers/pci-pf-stub/unbind
    fi
    put "" "$P/driver_override"
    [ -n "$(driver_of "$PF")" ] || put "$PF" /sys/bus/pci/drivers_probe
    [ "$(driver_of "$PF")" = dfl-pci ] || { echo "afu-vf.sh: $PF came back on '$(driver_of "$PF")', not dfl-pci" >&2; exit 2; }
    echo "afu-vf.sh: $PF is back on dfl-pci"
    ;;
  build)
    [ $# = 4 ] || usage
    afu="$2"; program="$3"; out="$4"
    backend="$(hal)"
    libs=()
    [ "$backend" = opae ] && libs=(-lopae-c)
    cc -std=c99 -O2 -Wall -Werror -I"$afu/sw" "$afu/sw/logos_$program.c" "$afu/sw/logos_hal_$backend.c" "${libs[@]}" -o "$out"
    ;;
  exec)
    [ $# -ge 2 ] || usage
    shift
    if [ "$(hal)" = pci ]; then
      exec sudo env "LOGOS_USER_CLOCK_MHZ=${LOGOS_USER_CLOCK_MHZ:-}" ${LOGOS_D5005_VF:+"LOGOS_D5005_VF=$LOGOS_D5005_VF"} "$@"
    fi
    exec "$@"
    ;;
  clock)
    [ $# = 2 ] || usage
    python3 - "$2" <<'PY'
import json, struct, sys
with open(sys.argv[1], "rb") as f:
    head = f.read(20)
    if len(head) < 20 or head[:16] != bytes.fromhex("58656f6e46504741b747425376303031"):
        sys.exit(f"afu-vf.sh: {sys.argv[1]} is not a GBS")
    meta = json.loads(f.read(struct.unpack_from("<I", head, 16)[0]))
print(meta["afu-image"]["clock-frequency-high"])
PY
    ;;
  *) usage ;;
esac
