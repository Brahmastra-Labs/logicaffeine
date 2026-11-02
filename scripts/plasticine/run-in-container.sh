#!/usr/bin/env bash
# Build the deps image once, then run build.sh (or any command) in it with the toolchain tree mounted at
# /work, CPU-capped so it never crowds the test machine. Usage: run-in-container.sh [command...]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
DEST="${LOGOS_PLASTICINE_TOOLCHAIN:-$REPO/target/plasticine-toolchain}"
docker build -q -t logos-plasticine-toolchain "$HERE" >/dev/null
if [ $# -eq 0 ]; then
    set -- bash /build.sh /work
fi
# LOGOS_PLASTICINE_NAME names the container, so a caller holding a deadline can stop exactly this run.
# The Scala chain's caches (the published PIR jar in ivy, sbt's boot and coursier caches) must outlive one
# `docker run --rm`: they live on the host under $DEST/.home.
mkdir -p "$DEST/.home/ivy2" "$DEST/.home/cache" "$DEST/.home/sbt-boot"
exec docker run --rm ${LOGOS_PLASTICINE_NAME:+--name "$LOGOS_PLASTICINE_NAME"} --cpus="${LOGOS_PLASTICINE_CPUS:-8}" -v "$DEST:/work" -v "$HERE/build.sh:/build.sh:ro" -v "$HERE/gate.sh:/gate.sh:ro" -v "$HERE/sim.sh:/sim.sh:ro" \
    -v "$HERE/table4.sh:/table4.sh:ro" -v "$HERE/table4:/table4:ro" \
    -v "$DEST/.home/ivy2:/root/.ivy2" -v "$DEST/.home/cache:/root/.cache" -v "$DEST/.home/sbt-boot:/root/.sbt/boot" \
    -w /work logos-plasticine-toolchain "$@"
