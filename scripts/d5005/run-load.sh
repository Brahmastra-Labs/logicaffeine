#!/usr/bin/env bash
# Load a weight image into the card's memory channels and prove it is there: build the image loader against OPAE and
# run it on an image directory the image writer made (`cargo run --release -p logicaffeine-compile --example
# d5005_weight_image -- --write <gguf> <dir> [data64|full72]`), its JSON on stdout.
#
#   scripts/d5005/run-load.sh <afu dir> <image directory> [--self-test]
#
# Needs the D5005 in this machine with the OFS FIM, OPAE 2.12 (libopae-c and its headers), and the accelerator the
# <afu dir> describes configured on it (scripts/d5005/run-receipt.sh). With LOGOS_D5005_VF set, the loader reaches it
# through that VF (afu-vf.sh, attached for the run and detached after): under VFIO with an IOMMU, or as root through
# the physical backend without one. Exits as the loader does: 0 only when every check held, 2 when the image or the
# card cannot be had.
set -euo pipefail
AFU="$(cd "${1:?usage: run-load.sh <afu dir> <image directory> [--self-test]}" && pwd)"
IMAGE="${2:?usage: run-load.sh <afu dir> <image directory> [--self-test]}"
shift 2
command -v cc >/dev/null || { echo "run-load.sh: cc is not on PATH" >&2; exit 2; }
W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT
VFS="$(dirname "$0")/afu-vf.sh"
if [ -n "${LOGOS_D5005_VF:-}" ]; then
  trap '"$VFS" detach; rm -rf "$W"' EXIT
  "$VFS" attach
fi
"$VFS" build "$AFU" load "$W/logos_load"
status=0
"$VFS" exec "$W/logos_load" "$IMAGE" "$@" || status=$?
exit "$status"
