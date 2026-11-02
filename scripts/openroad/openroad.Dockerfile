# The pinned OpenLane image with its OpenROAD (The-OpenROAD-Project/OpenROAD@41a51eaf, src/sta OpenSTA@2609cc89,
# third-party/abc abc@95b3543e) rebuilt with one change, looped-route.patch. rsz builds a net's buffered tree from its
# global route by walking from the driver to every adjacent route point but the one it came from; on a route that
# closes a cycle that walk never ends, and repair_timing overflows the stack (SIGSEGV). The patch looks for a cycle
# first: a route without one is walked exactly as before, and a route with one is walked once over a spanning tree.
# Everything else is OpenLane's own docker/openroad_app recipe at that commit — devtoolset-8, Boost 1.80, Eigen 3.3,
# Lemon 1.3.1, spdlog 1.8.1, SWIG 4.0.1, OR-Tools 9.4 — on the CentOS 7 userland the image runs its OpenROAD in, its
# packages from the CentOS vault and EPEL's archive, the release mirrors being gone. The script beside this file
# fetches every source, checks it against sources.txt and builds this image.
ARG OPENLANE_IMAGE

FROM centos:centos7 AS build
ARG JOBS
RUN rm -f /etc/yum.repos.d/*.repo \
 && printf '%s\n' '[base]' 'name=CentOS 7.9 base (vault)' 'baseurl=http://vault.centos.org/7.9.2009/os/x86_64/' 'gpgcheck=1' \
      'gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-CentOS-7' '[updates]' \
      'name=CentOS 7.9 updates (vault)' 'baseurl=http://vault.centos.org/7.9.2009/updates/x86_64/' 'gpgcheck=1' \
      'gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-CentOS-7' '[extras]' \
      'name=CentOS 7.9 extras (vault)' 'baseurl=http://vault.centos.org/7.9.2009/extras/x86_64/' 'gpgcheck=1' \
      'gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-CentOS-7' > /etc/yum.repos.d/vault.repo \
 && yum install -y centos-release-scl epel-release \
 && rm -f /etc/yum.repos.d/CentOS-*.repo /etc/yum.repos.d/epel*.repo \
 && printf '%s\n' '[sclo-rh]' 'name=CentOS 7.9 SCLo rh (vault)' 'baseurl=http://vault.centos.org/7.9.2009/sclo/x86_64/rh/' 'gpgcheck=1' \
      'gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-CentOS-SIG-SCLo' '[sclo-sclo]' \
      'name=CentOS 7.9 SCLo sclo (vault)' 'baseurl=http://vault.centos.org/7.9.2009/sclo/x86_64/sclo/' 'gpgcheck=1' \
      'gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-CentOS-SIG-SCLo' '[epel]' \
      'name=EPEL 7 (archive)' 'baseurl=https://archives.fedoraproject.org/pub/archive/epel/7/x86_64/' 'gpgcheck=1' \
      'gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-7' >> /etc/yum.repos.d/vault.repo \
 && yum install --setopt=skip_missing_names_on_install=False -y devtoolset-8 devtoolset-8-libatomic-devel autoconf \
      automake bison boost169-devel boost169-static bzip2 clang curl flex gawk gcc gettext-devel git glibc-static \
      libffi-devel libstdc++-static libxml2-devel make ncurses-devel patch pcre-devel python3 python3-devel \
      qt5-qtbase-devel readline-devel spdlog-devel swig3 tcl tcl-devel tcl-tclreadline-devel tcllib tclx tk-devel \
      wget which zlib-devel zlib-static \
 && yum clean all \
 && python3 -m pip install --no-cache-dir 'pip==21.3.1' \
 && python3 -m pip install --no-cache-dir 'cmake==3.26.4'
ENV CC=/opt/rh/devtoolset-8/root/usr/bin/gcc \
    CPP=/opt/rh/devtoolset-8/root/usr/bin/cpp \
    CXX=/opt/rh/devtoolset-8/root/usr/bin/g++ \
    PATH=/usr/local/bin:/opt/rh/devtoolset-8/root/usr/bin:$PATH \
    LD_LIBRARY_PATH=/opt/rh/devtoolset-8/root/usr/lib64:/opt/rh/devtoolset-8/root/usr/lib \
    LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 LC_CTYPE=en_US.UTF-8
RUN localedef -c -f UTF-8 -i en_US en_US.UTF-8 || true
COPY boost_1_80_0.tar.bz2 eigen-3.3.tar.gz lemon-graph-1.3.1.tar.gz spdlog-1.8.1.tar.gz swig-4.0.1.tar.gz \
     or-tools-9.4.tar.gz /src/
RUN mkdir /boost && tar --strip-components=1 -xjf /src/boost_1_80_0.tar.bz2 -C /boost && cd /boost \
 && ./bootstrap.sh && ./b2 install --with-iostreams --with-test --with-serialization --with-system --with-thread -j "$JOBS"
RUN mkdir -p /eigen/build && tar --strip-components=1 -xzf /src/eigen-3.3.tar.gz -C /eigen && cd /eigen/build \
 && cmake .. && make -j "$JOBS" && make install
RUN mkdir /lemon && tar --strip-components=1 -xzf /src/lemon-graph-1.3.1.tar.gz -C /lemon && cd /lemon \
 && cmake -B build . && cmake --build build -j "$JOBS" --target install
RUN mkdir -p /spdlog/build && tar --strip-components=1 -xzf /src/spdlog-1.8.1.tar.gz -C /spdlog && cd /spdlog/build \
 && cmake .. && make install -j "$JOBS"
RUN yum remove -y swig3 && mkdir /swig && tar --strip-components=1 -xzf /src/swig-4.0.1.tar.gz -C /swig && cd /swig \
 && ./autogen.sh && ./configure --prefix=/usr && make -j "$JOBS" && make install
RUN mkdir /ortools && tar --strip-components=1 -xzf /src/or-tools-9.4.tar.gz -C /ortools && cd /ortools \
 && cmake -B build . -DBUILD_DEPS:BOOL=ON && cmake --build build -j "$JOBS" --target install
COPY OpenROAD-41a51eaf4ca2171c92ff38afb91eb37bbd3f36da.tar.gz OpenSTA-2609cc89eeb02a06ceca1890624d4fa1932d930b.tar.gz \
     abc-95b3543e928640dfa25f9e882e72a090a8883a9c.tar.gz looped-route.patch /src/
RUN mkdir /openroad && cd /openroad \
 && tar --strip-components=1 -xzf /src/OpenROAD-41a51eaf4ca2171c92ff38afb91eb37bbd3f36da.tar.gz \
 && tar --strip-components=1 -xzf /src/OpenSTA-2609cc89eeb02a06ceca1890624d4fa1932d930b.tar.gz -C src/sta \
 && tar --strip-components=1 -xzf /src/abc-95b3543e928640dfa25f9e882e72a090a8883a9c.tar.gz -C third-party/abc \
 && patch -p1 < /src/looped-route.patch \
 && sed -i "s/GITDIR-NOTFOUND/41a51eaf4ca2171c92ff38afb91eb37bbd3f36da/" cmake/GetGitRevisionDescription.cmake \
 && sed -i "s/-m64//" src/par/CMakeLists.txt \
 && sed -i "s/static char Cases/static signed char Cases/" third-party/abc/src/misc/extra/extraUtilMisc.c \
 && mkdir build && cd build && cmake -DCMAKE_INSTALL_PREFIX=/openroad/install .. && make -j "$JOBS" && make install

FROM ${OPENLANE_IMAGE}
COPY --from=build /openroad/install/bin/openroad /build/bin/openroad
COPY --from=build /usr/local/lib64/libortools.so.9 /build/lib/libortools.so.9
COPY looped-route.patch /build/version/openroad-looped-route.patch
RUN echo "with looped-route.patch (/build/version/openroad-looped-route.patch)" >> /build/version/openroad.version
