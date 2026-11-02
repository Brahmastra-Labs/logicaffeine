#!/usr/bin/env bash
# The card's first day: configure the accelerator image, run and record the bring-up receipt, then load a weight image
# onto the memory channels and prove it there with the loader's self-test.
#
#   scripts/d5005/bringup.sh <afu dir> <accelerator image> <image directory> [lines per channel] [host lines]
#
# The accelerator image is a persona GBS or a flat SOF already loaded over JTAG (run-receipt.sh); the image directory is
# one `cargo run --release -p logicaffeine-compile --example d5005_weight_image -- --write <gguf> <dir> full72` wrote.
# Needs what run-receipt.sh and run-load.sh need. Exits 0 only when both passed.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
AFU="${1:?usage: bringup.sh <afu dir> <accelerator image> <image directory> [lines] [host lines]}"
GBS="${2:?usage: bringup.sh <afu dir> <accelerator image> <image directory> [lines] [host lines]}"
IMAGE="${3:?usage: bringup.sh <afu dir> <accelerator image> <image directory> [lines] [host lines]}"
shift 3
"$HERE/run-receipt.sh" "$AFU" "$GBS" "$@"
"$HERE/run-load.sh" "$AFU" "$IMAGE" --self-test
