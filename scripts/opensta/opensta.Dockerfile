# OpenSTA as the pinned OpenLane image carries it — OpenLane ff5509f6 builds OpenROAD 41a51eaf, whose src/sta is
# The-OpenROAD-Project/OpenSTA@2609cc89, configured as that image's sta is (release build, no CUDD) — with two changes
# to the activity walk report_power reads a pin's activity from. read_power_activities annotates every pin of a VCD,
# but the walk starts at the primary inputs and gives an annotation only to the pins it reaches; every other pin falls
# back to the default input activity.
#   activity-origin.patch  upstream's parallaxsw/OpenSTA@9d69dfbc: a pin whose activity changes origin continues the
#                          walk (before it, only a change above 0.01 did, and on a block whose only data input is a
#                          reset the walk ended there);
#   annotated-seeds.patch  ours: every annotated pin seeds the walk too, so logic no path from an input reaches (a
#                          register holding its own value) reads its annotation.
# The default input activity propagates further as well; under one global activity, where no walk runs, this build and
# the image's report the same power to every printed digit. The script beside this file fetches the source and builds
# it.
FROM ubuntu:22.04 AS build
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential cmake tcl-dev swig bison flex zlib1g-dev patch && rm -rf /var/lib/apt/lists/*
COPY OpenSTA-2609cc89eeb02a06ceca1890624d4fa1932d930b.tar.gz activity-origin.patch annotated-seeds.patch /src/
RUN cd /src && tar xzf OpenSTA-2609cc89eeb02a06ceca1890624d4fa1932d930b.tar.gz \
 && cd OpenSTA-2609cc89eeb02a06ceca1890624d4fa1932d930b \
 && patch -p1 < /src/activity-origin.patch && patch -p1 < /src/annotated-seeds.patch \
 && cmake -S . -B build -DCMAKE_BUILD_TYPE=RELEASE && cmake --build build -j 8 \
 && install -m 755 app/sta /usr/local/bin/sta

FROM ubuntu:22.04
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends tcl zlib1g && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/sta /usr/local/bin/sta
COPY activity-origin.patch annotated-seeds.patch /usr/local/share/opensta/
