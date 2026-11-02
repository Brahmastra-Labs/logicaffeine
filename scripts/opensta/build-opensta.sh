#!/usr/bin/env bash
# Build the OpenSTA the power measurements run (scripts/opensta/opensta.Dockerfile): the pinned source, checked by its
# SHA-256, with its two patches applied, as the image logos-opensta:<commit>-<build>, <build> the SHA-256 of the
# Dockerfile and the patches together, so a changed build is a new image. Prints the image name.
#
#   scripts/opensta/build-opensta.sh            build it if it is not built, and print its name
#   scripts/opensta/build-opensta.sh --name     print its name only
set -euo pipefail
cd "$(dirname "$0")/../.."
COMMIT=2609cc89eeb02a06ceca1890624d4fa1932d930b
SHA256=026789ce6bc304f12163f9d10bf0a1ada7f9de42d67ae74ad9d880b3a44ed0af
HERE=scripts/opensta
BUILD=$(cat "$HERE/opensta.Dockerfile" "$HERE/activity-origin.patch" "$HERE/annotated-seeds.patch" | sha256sum \
  | cut -c1-12)
IMAGE="logos-opensta:${COMMIT:0:8}-$BUILD"
if [ "${1:-}" = --name ]; then
  echo "$IMAGE"
  exit 0
fi
if docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "$IMAGE"
  exit 0
fi
SRC=tools/opensta
mkdir -p "$SRC"
TARBALL="$SRC/OpenSTA-$COMMIT.tar.gz"
if [ ! -f "$TARBALL" ]; then
  curl -sfL -o "$TARBALL.part" "https://github.com/The-OpenROAD-Project/OpenSTA/archive/$COMMIT.tar.gz"
  mv "$TARBALL.part" "$TARBALL"
fi
echo "$SHA256  $TARBALL" | sha256sum -c --quiet - || { echo "$TARBALL: not the pinned source" >&2; exit 1; }
CTX=$(mktemp -d "$SRC/ctx.XXXXXX")
trap 'rm -rf "$CTX"' EXIT
cp "$TARBALL" "$HERE/activity-origin.patch" "$HERE/annotated-seeds.patch" "$CTX/"
cp "$HERE/opensta.Dockerfile" "$CTX/Dockerfile"
docker build -q -t "$IMAGE" "$CTX" >&2
echo "$IMAGE"
