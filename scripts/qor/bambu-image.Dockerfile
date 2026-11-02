# The PandA-Bambu runtime: Bambu's front end compiles for i386, so it needs the 32-bit C headers. The extracted
# AppImage (tools/qor/bambu-<version>, scripts/qor/fetch-qor-tools.py) is mounted at /opt/bambu. The base is pinned by
# digest in scripts/qor/qor-tools.toml; fetch-qor-tools.py bambu builds this image.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      gcc-multilib libc6-dev-i386 make perl && rm -rf /var/lib/apt/lists/*
ENV APPDIR=/opt/bambu ARGV0=bambu LC_ALL=C
WORKDIR /work
ENTRYPOINT ["/bin/sh", "-c", "OWD=$PWD exec /opt/bambu/AppRun \"$@\"", "bambu"]
