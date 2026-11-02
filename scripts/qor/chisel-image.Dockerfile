# The Chisel elaboration image of the comparative-synthesis bench: Temurin JDK 17 pinned by digest and sbt from its
# release archive pinned by sha256 (both in scripts/qor/qor-tools.toml). Scala and chisel3 arrive through sbt from
# Maven Central at the versions the elaboration's build.sbt names, into a cache volume the builder mounts as $HOME.
ARG JDK_IMAGE
FROM ${JDK_IMAGE}
COPY sbt.tgz /tmp/sbt.tgz
RUN tar -xzf /tmp/sbt.tgz -C /opt && rm /tmp/sbt.tgz && chmod -R a+rX /opt/sbt
ENV PATH=/opt/sbt/bin:$PATH
