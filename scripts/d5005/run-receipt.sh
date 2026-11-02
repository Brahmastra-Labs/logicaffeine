#!/usr/bin/env bash
# The bring-up receipt on the card: load the accelerator image, build the receipt against OPAE, run it, and record its
# JSON in benchmarks/silicon/d5005/receipts.json against the image it ran on.
#
#   scripts/d5005/run-receipt.sh <afu dir> <image> [lines per channel] [host lines]
#
# <image> is a persona GBS, which fpgaconf loads into the FIM, or a flat image (build-afu.sh --flat), a SOF holding the
# FIM and the accelerator, which is already on the card, loaded over JTAG. Needs the D5005 in this machine with the
# OFS FIM, OPAE 2.12 (libopae-c and its headers; fpgaconf for a GBS) and the image recorded in
# benchmarks/silicon/d5005/afu_images.json (scripts/d5005/build-afu.sh). With LOGOS_D5005_VF set, the receipt reaches
# the accelerator through that VF (afu-vf.sh attaches it once the image is on, and detaches it after): under VFIO with
# an IOMMU, or as root through the physical backend without one; its user clock LOGOS_USER_CLOCK_MHZ, by default the
# one a GBS has fpgaconf set.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
AFU="$(cd "${1:?usage: run-receipt.sh <afu dir> <image> [lines] [host lines]}" && pwd)"
GBS="${2:?usage: run-receipt.sh <afu dir> <image> [lines] [host lines]}"
LINES="${3:-1048576}"
HOST_LINES="${4:-4096}"
TOOLS=(cc)
case "$GBS" in *.sof) ;; *) TOOLS+=(fpgaconf) ;; esac
for tool in "${TOOLS[@]}"; do
  command -v "$tool" >/dev/null || { echo "run-receipt.sh: $tool is not on PATH" >&2; exit 2; }
done
W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT
case "$GBS" in
  *.sof) echo "run-receipt.sh: $GBS is a flat image, on the card once it is loaded over JTAG" >&2 ;;
  *) fpgaconf "$GBS" ;;
esac
VFS="$(dirname "$0")/afu-vf.sh"
if [ -n "${LOGOS_D5005_VF:-}" ]; then
  case "$GBS" in *.sof) ;; *) [ -n "${LOGOS_USER_CLOCK_MHZ:-}" ] || export LOGOS_USER_CLOCK_MHZ="$("$VFS" clock "$GBS")" ;; esac
  trap '"$VFS" detach; rm -rf "$W"' EXIT
  "$VFS" attach
fi
"$VFS" build "$AFU" receipt "$W/logos_receipt"
status=0
"$VFS" exec "$W/logos_receipt" "$LINES" "$HOST_LINES" > "$W/receipt.json" || status=$?
cat "$W/receipt.json"
python3 "$ROOT/scripts/d5005/record_receipt.py" "$W/receipt.json" "$GBS" "$ROOT/benchmarks/silicon/d5005/afu_images.json" \
  "$ROOT/benchmarks/silicon/d5005/receipts.json"
exit "$status"
