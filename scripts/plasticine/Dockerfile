# The dependency image for the public Plasticine toolchain (Spatial → PIR → plastiroute → Tungsten), 2023
# snapshot: JDK 8 + sbt for the Scala chain, Python 3.6 for PIR's scripts (both native on bionic), gcc-10 from the
# toolchain PPA for plastiroute (C99) and Tungsten (C++20).
# The sources are NOT baked in — `build.sh` runs inside `docker run` with target/plasticine-toolchain mounted,
# so the built chain persists on the host and the image stays a pure environment.
FROM ubuntu:18.04
ENV DEBIAN_FRONTEND=noninteractive TZ=UTC LANG=C.UTF-8
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl gnupg software-properties-common apt-transport-https \
    && add-apt-repository -y ppa:ubuntu-toolchain-r/test \
    && echo "deb https://repo.scala-sbt.org/scalasbt/debian all main" > /etc/apt/sources.list.d/sbt.list \
    && curl -sL "https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x2EE0EA64E40A89B84B2DF73499E82A75642AC823" | gpg --dearmor > /etc/apt/trusted.gpg.d/sbt.gpg \
    && apt-get update && apt-get install -y --no-install-recommends \
        openjdk-8-jdk-headless sbt \
        python3.6 python3.6-venv python3.6-dev python3-pip python3-setuptools \
        build-essential gcc-10 g++-10 make cmake pkg-config \
        libglib2.0-dev libisl-dev libgmp-dev libtbb-dev libgomp1 graphviz time \
    && rm -rf /var/lib/apt/lists/*
RUN update-alternatives --install /usr/bin/gcc gcc /usr/bin/gcc-10 100 \
    && update-alternatives --install /usr/bin/g++ g++ /usr/bin/g++-10 100 \
    && python3.6 -m pip install --no-cache-dir virtualenv
ENV JAVA_HOME=/usr/lib/jvm/java-8-openjdk-amd64
# The chain's log summarizer shells out to `git` for a commit hash; the tree is a tarball snapshot, so a stub
# answers every invocation with a fixed label instead of installing git in the image.
RUN printf '#!/bin/sh\ncase "$1" in log) echo "tarball-snapshot,2023-01-31 00:00:00 +0000";; *) echo tarball-snapshot-2023-01-31;; esac\n' > /usr/local/bin/git && chmod +x /usr/local/bin/git
# The generated simulator's Makefile and PIR's runners call bare `python`: route it to PIR's venv (on the
# mounted tree) so those scripts see the pinned numpy/pandas.
RUN ln -s /work/spatial/pir/env/bin/python /usr/local/bin/python
# sbt 1.1.1 (the snapshot's project/build.properties) predates the Bintray shutdown: route every resolver
# through Maven Central + the scala-sbt repo so the launcher and plugins still resolve. The list lives where every
# user reads it, so a build run as the host's user (its outputs the host's to remove) resolves the same.
RUN mkdir -p /etc/sbt && printf '[repositories]\n  local\n  maven-central\n  sbt-plugin-releases: https://repo.scala-sbt.org/scalasbt/sbt-plugin-releases/, [organization]/[module]/(scala_[scalaVersion]/)(sbt_[sbtVersion]/)[revision]/[type]s/[artifact](-[classifier]).[ext]\n  typesafe-ivy-releases: https://repo.typesafe.com/typesafe/ivy-releases/, [organization]/[module]/[revision]/[type]s/[artifact](-[classifier]).[ext], bootOnly\n' > /etc/sbt/repositories \
    && chmod a+r /etc/sbt/repositories
# PIR recurses deeply on wide-parallel apps (GEMM_17 overflowed the default 1 MB thread stack): give the JVM room.
ENV SBT_OPTS="-Dsbt.override.build.repos=true -Dsbt.repository.config=/etc/sbt/repositories -Xmx6g -Xss512m" JAVA_OPTS="-Xss512m"
WORKDIR /work
CMD ["bash"]
