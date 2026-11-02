#!/bin/bash
# The pinned compute-sanitizer (CUDA 12.8.1's cuda_sanitizer_api 12.8.93, tool 2025.1), unpacked into the gitignored
# external/cuda-sanitizer/: the race, memory and barrier checker the lane's cp.async kernels are run under. The toolkit
# beside nvcc ships 2022.4.1, which cannot attach under the 580 driver. The archive's SHA-256 is the one NVIDIA's
# redistrib_12.8.1.json lists.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
dest="$root/external/cuda-sanitizer"
name=cuda_sanitizer_api-linux-x86_64-12.8.93-archive
sha=ae3574f052c0e06c95305962668eb1fe6ab571dfbb58b305fdb14d523bb1b240
mkdir -p "$dest"
cd "$dest"
if [ ! -x "$name/compute-sanitizer/compute-sanitizer" ]; then
  curl -sSfL -o "$name.tar.xz" "https://developer.download.nvidia.com/compute/cuda/redist/cuda_sanitizer_api/linux-x86_64/$name.tar.xz"
  echo "$sha  $name.tar.xz" | sha256sum -c -
  tar -xJf "$name.tar.xz"
  rm "$name.tar.xz"
fi
"$name/compute-sanitizer/compute-sanitizer" --version | tail -1
