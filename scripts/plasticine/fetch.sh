#!/usr/bin/env bash
# Fetch the public Plasticine toolchain (the 2023-01-31 GitLab dataplane-ai snapshot) as ARCHIVE TARBALLS —
# no git — into $DEST (default: target/plasticine-toolchain, ignored by the repo), laid out the way the
# chain's own build expects (pir inside spatial; plastiroute/plastisim/tungsten inside pir; fmt inside
# tungsten). The submodule URLs in the snapshot point at private GitHub repos, so the same projects are
# placed from the public mirrors instead. Idempotent: an already-extracted tree is left alone.
set -euo pipefail
DEST="${1:-$(cd "$(dirname "$0")/../.." && pwd)/target/plasticine-toolchain}"
mkdir -p "$DEST"
fetch() {
    local url="$1" into="$2"
    if [ -d "$into" ] && [ -n "$(ls -A "$into" 2>/dev/null)" ]; then
        echo "  present: $into"
        return
    fi
    echo "  fetching $url"
    mkdir -p "$into"
    curl -sSL --max-time 900 "$url" | tar -xz --strip-components=1 -C "$into"
}
gl() { echo "https://gitlab.com/dataplane-ai/$1/-/archive/$2/$1-$2.tar.gz"; }
echo "Plasticine toolchain → $DEST"
# Spatial's `pir` branch (2023-01-31) is the front end whose PIR code generator matches PIR master's node API
# (StridedCounter/ForeverCounter); Spatial master (2020-11) still emits the older `Counter(par=…)`.
fetch "$(gl spatial pir)"               "$DEST/spatial"
fetch "$(gl pir master)"                "$DEST/spatial/pir"
fetch "$(gl plastiroute spdram_place)"  "$DEST/spatial/pir/plastiroute"
fetch "$(gl plastisim master)"          "$DEST/spatial/pir/plastisim"
# PIR master pins a Tungsten commit on Tungsten `master` (2020-11-21), which still carries plasticine/resources
# (the `mlir` branch dropped it and PIR's TungstenPIRGen refuses without it).
fetch "$(gl tungsten master)"           "$DEST/spatial/pir/tungsten"
# tungsten's `fmt` submodule (fmtlib) — the snapshot predates fmt 8's API changes — and its header-only
# `parallelstl` submodule (Intel Parallel STL, the 2019 release contemporaneous with Tungsten master);
# its `tbb` submodule is replaced by the image's system TBB (see build.sh).
fetch "https://github.com/fmtlib/fmt/archive/refs/tags/7.1.3.tar.gz" "$DEST/spatial/pir/tungsten/fmt"
fetch "https://github.com/intel/parallelstl/archive/refs/tags/20190522.tar.gz" "$DEST/spatial/pir/tungsten/parallelstl"
echo "LOGOS_PLASTICINE_TOOLCHAIN=$DEST"
du -sh "$DEST"
