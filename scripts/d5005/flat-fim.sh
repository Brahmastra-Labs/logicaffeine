#!/usr/bin/env bash
# The source tree a flat image is compiled from: the OFS d5005 2024.1-1 FIM's own source (ofs-d5005 with ofs-common at
# the commit its submodule names) and the PIM's ofs-platform-afu-bbb, from the archives scripts/d5005/fetch-d5005-tools.sh
# pins, unpacked under <dir> as ofs-d5005-ofs-2024.1-1 and ofs-platform-afu-bbb-ofs-2024.1-1. The FIM's floorplan is
# made whole-device: its base revision declares no partial-reconfiguration partition and no place or route region for
# afu_main, so the accelerator the FIM's afu_main instantiates through the PIM is placed over the whole device beside
# the FIM's static logic, with every DSP block and M20K the FIM leaves, instead of inside the persona region (601,050
# ALMs, 3,971 DSP blocks, 7,583 M20Ks). Nothing else of the FIM changes.
#
#   scripts/d5005/flat-fim.sh <dir>
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
T="${D5005_TOOLS:-$ROOT/tools/d5005}"
[ $# -eq 1 ] || { echo "usage: flat-fim.sh <dir>" >&2; exit 2; }
OFS="$1"
rm -rf "$OFS"
mkdir -p "$OFS"
tar -xzf "$T/dl/ofs-d5005-ofs-2024.1-1.tar.gz" -C "$OFS"
FIM="$OFS/ofs-d5005-ofs-2024.1-1"
rm -rf "$FIM/ofs-common"
tar -xzf "$T/dl/ofs-fim-common-374fd3ccb014e31c0b1fad207b1299e8081e4fd9.tar.gz" -C "$OFS"
mv "$OFS/ofs-fim-common-374fd3ccb014e31c0b1fad207b1299e8081e4fd9" "$FIM/ofs-common"
tar -xzf "$T/dl/ofs-platform-afu-bbb-ofs-2024.1-1.tar.gz" -C "$OFS"
BBB="$OFS/ofs-platform-afu-bbb-ofs-2024.1-1"
REGION="$FIM/syn/setup/sr_logic_lock_region.tcl"
for f in "$FIM/ofs-common/scripts/common/syn/build_top.sh" "$FIM/syn/scripts/build_var_setup.sh" \
  "$FIM/ofs-common/src/fpga_family/stratix10/port_gasket/afu_main_pim/afu_main.sv" "$BBB/plat_if_develop/ofs_plat_if" \
  "$REGION"; do
  [ -e "$f" ] || { echo "flat-fim.sh: the FIM's source tree has no $f" >&2; exit 2; }
done
# The release's floorplan is the base revision of a PR design: REVISION_TYPE PR_BASE, the partition persona1 at
# afu_main, its PR flag, and a reserved, core-only place region with its route region. A flat image keeps none of them.
for a in 'REVISION_TYPE PR_BASE' 'PARTITION persona1' 'PARTIAL_RECONFIGURATION_PARTITION ON' 'PLACE_REGION ' \
  'ROUTE_REGION ' 'RESERVE_PLACE_REGION ON' 'CORE_ONLY_PLACE_REGION ON'; do
  grep -q -- "-name $a" "$REGION" || { echo "flat-fim.sh: $REGION no longer assigns $a; read the FIM's floorplan again" >&2; exit 2; }
done
cat > "$REGION" <<'EOF'
# A flat image (scripts/d5005/flat-fim.sh): afu_main is no partial-reconfiguration partition and has no place or route
# region, so the accelerator is placed over the whole device beside the FIM's static logic.
EOF
echo "flat-fim.sh: the FIM's source tree is $FIM, the PIM $BBB; afu_main takes the whole device"
