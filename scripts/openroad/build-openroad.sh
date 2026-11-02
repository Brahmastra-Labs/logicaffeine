#!/usr/bin/env bash
# Build the OpenLane image the comparative bench's flows run (scripts/openroad/openroad.Dockerfile): the image at the
# overlay's base with its OpenROAD rebuilt from the pinned sources, every one checked against sources.txt, with
# looped-route.patch applied, as logos-openlane:<base>-openroad-<build>, <build> the SHA-256 of the Dockerfile, the
# patch and the source list together, so a changed build is a new image. Prints the image name.
#
#   scripts/openroad/build-openroad.sh            build it if it is not built, and print its name
#   scripts/openroad/build-openroad.sh --name     print its name only
#   scripts/openroad/build-openroad.sh --built    exit 0 when it is built under the name OpenLane launches
#
# OpenLane's Makefile launches $(OPENLANE_IMAGE_NAME)-$(DOCKER_ARCH), so the image is also tagged with the machine's
# architecture as docker names it, and is built only when that name exists.
#
# JOBS (default 12) bounds the compile, so a build leaves the rest of the box to the flows beside it.
set -euo pipefail
cd "$(dirname "$0")/../.."
HERE=scripts/openroad
BASE=$(tr -d '[:space:]' < scripts/openlane-overlay/BASE)
BUILD=$(cat "$HERE/openroad.Dockerfile" "$HERE/looped-route.patch" "$HERE/sources.txt" | sha256sum | cut -c1-12)
IMAGE="logos-openlane:${BASE:0:8}-openroad-$BUILD"
if [ "${1:-}" = --name ]; then
  echo "$IMAGE"
  exit 0
fi
ARCH=$(docker version --format '{{.Server.Arch}}')
if [ "${1:-}" = --built ]; then
  docker image inspect "$IMAGE-$ARCH" >/dev/null 2>&1
  exit
fi
if docker image inspect "$IMAGE" >/dev/null 2>&1; then
  docker tag "$IMAGE" "$IMAGE-$ARCH"
  echo "$IMAGE"
  exit 0
fi
SRC=tools/openroad
mkdir -p "$SRC"
while read -r sum file url; do
  if [ ! -f "$SRC/$file" ]; then
    curl -sfL -o "$SRC/$file.part" "$url"
    mv "$SRC/$file.part" "$SRC/$file"
  fi
  echo "$sum  $SRC/$file" | sha256sum -c --quiet - || { echo "$SRC/$file: not the pinned source" >&2; exit 1; }
done < "$HERE/sources.txt"
CTX=$(mktemp -d "$SRC/ctx.XXXXXX")
trap 'rm -rf "$CTX"' EXIT
while read -r _ file _; do
  ln "$SRC/$file" "$CTX/$file" 2>/dev/null || cp "$SRC/$file" "$CTX/$file"
done < "$HERE/sources.txt"
cp "$HERE/looped-route.patch" "$CTX/"
cp "$HERE/openroad.Dockerfile" "$CTX/Dockerfile"
docker build -t "$IMAGE" --build-arg "OPENLANE_IMAGE=ghcr.io/the-openroad-project/openlane:$BASE" \
  --build-arg "JOBS=${JOBS:-12}" "$CTX" >&2
docker tag "$IMAGE" "$IMAGE-$ARCH"
echo "$IMAGE"
