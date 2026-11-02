#!/usr/bin/env bash
# Fetch what the D5005 accelerator flow is checked against and built from, none of it licensed: the OFS d5005
# release's images (its FIM's PR build template, unpacked to pr-template/), the OPAE SDK's source (afu_synth_setup,
# rtl_src_config, the AFU JSON schema and the C headers, unpacked to opae-sdk/), pyslang in venv/, and the FIM's own
# source for build-afu.sh --flat (ofs-d5005 at ofs-2024.1-1, ofs-fim-common at the commit that tag's ofs-common
# submodule names, ofs-platform-afu-bbb at its ofs-2024.1-1 tag, kept as archives in dl/), all under tools/d5005 (or
# $D5005_TOOLS). Each download must have the SHA-256 written here; any other stops the script.
#
#   scripts/d5005/fetch-d5005-tools.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
T="${D5005_TOOLS:-$ROOT/tools/d5005}"
DL="$T/dl"
mkdir -p "$DL"

fetch() {
  local name=$1 sha=$2 url=$3
  if [ ! -f "$DL/$name" ]; then
    curl -fL --retry 3 -o "$DL/$name.part" "$url"
    mv "$DL/$name.part" "$DL/$name"
  fi
  if [ "$(sha256sum "$DL/$name" | awk '{ print $1 }')" != "$sha" ]; then
    echo "$DL/$name is not the file this script pins (sha256 $sha)" >&2
    exit 1
  fi
}

fetch d5005-images_ofs_2024-1-1.tar.gz b524bd78321e040e728af18ac4e1302c00f45481b9065eeb01fefeb603793fb0 \
  https://github.com/OFS/ofs-d5005/releases/download/ofs-2024.1-1/d5005-images_ofs_2024-1-1.tar.gz
fetch opae-sdk-2.12.0-5.tar.gz a615e98f02d3e038ca1a42eb403a7f90fa5f149df69c05cbe448a30a6f5b7c63 \
  https://github.com/OFS/opae-sdk/archive/refs/tags/2.12.0-5.tar.gz
fetch ofs-d5005-ofs-2024.1-1.tar.gz 8683dc0804d61427a70cadf574733cf3dcbb51cbbe514ebec9fa43d849e52279 \
  https://github.com/OFS/ofs-d5005/archive/refs/tags/ofs-2024.1-1.tar.gz
fetch ofs-fim-common-374fd3ccb014e31c0b1fad207b1299e8081e4fd9.tar.gz fb6264c8458cf0bc43a59dd2c6bebebd5f796b0c3ed32de911199cd604afb9aa \
  https://github.com/OFS/ofs-fim-common/archive/374fd3ccb014e31c0b1fad207b1299e8081e4fd9.tar.gz
fetch ofs-platform-afu-bbb-ofs-2024.1-1.tar.gz 9dfa9baf85894864ae3f74f49dcae1df7bf72947532e648c212028635771359b \
  https://github.com/OFS/ofs-platform-afu-bbb/archive/refs/tags/ofs-2024.1-1.tar.gz

if [ ! -f "$T/pr-template/hw/lib/build/platform/ofs_plat_if/sim/platform_if_addenda.txt" ]; then
  tmp=$(mktemp -d "$T/.unpack.XXXXXX")
  tar -xzf "$DL/d5005-images_ofs_2024-1-1.tar.gz" -C "$tmp" d5005-images_ofs_2024-1-1/pr_build_template.tar.gz
  mkdir -p "$T/pr-template"
  tar -xzf "$tmp/d5005-images_ofs_2024-1-1/pr_build_template.tar.gz" -C "$T/pr-template"
  rm -rf "$tmp"
fi

if [ ! -f "$T/opae-sdk/platforms/platmgr/tools/rtl_src_config.py" ] || [ ! -f "$T/opae-sdk/include/opae/fpga.h" ]; then
  mkdir -p "$T/opae-sdk"
  tar -xzf "$DL/opae-sdk-2.12.0-5.tar.gz" -C "$T/opae-sdk" --strip-components=1 opae-sdk-2.12.0-5/platforms \
    opae-sdk-2.12.0-5/python/packager opae-sdk-2.12.0-5/include
fi

[ -x "$T/venv/bin/python" ] || python3 -m venv "$T/venv"
"$T/venv/bin/pip" install --quiet --require-hashes -r "$ROOT/scripts/d5005/requirements.txt"
"$T/venv/bin/python" -c "import pyslang"
echo "$T: pr-template, opae-sdk, venv and the FIM's source archives ready"
