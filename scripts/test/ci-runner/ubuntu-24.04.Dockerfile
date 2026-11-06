# The GitHub-hosted ubuntu-24.04 runner, as far as this repository's jobs can tell: the image
# `scripts/run-ci-locally.py` runs each job on `ubuntu-latest`, `ubuntu-24.04` or `ubuntu-latest-m` in. The package set
# is the runner image's published inventory (actions/runner-images, images/ubuntu/Ubuntu2404-Readme.md) reduced to what
# a build or a test here can observe: the apt table, its compilers and interpreters, cloud-init's python3-yaml, the
# libraries Chrome and VS Code's Electron load. What a workflow installs for itself (z3, mold, clang, iverilog, qemu,
# the RISC-V cross compiler) is left out, so its own step installs it here as it does there. Node and Chrome come from
# the host at run time (read-only mounts), Rust from the runner's toolchain store, so this image holds no third-party
# binary.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

RUN apt-get update && apt-get install -y --no-install-recommends \
      acl autoconf automake binutils bison brotli build-essential bzip2 ca-certificates cmake coreutils curl dbus \
      dnsutils dpkg-dev fakeroot file findutils flex g++ gcc gfortran git git-lfs gnupg2 iproute2 iputils-ping jq \
      less libc6-dev libicu-dev libnss3-tools libsqlite3-dev libssl-dev libtool libyaml-dev locales lz4 m4 make \
      net-tools netcat-openbsd ninja-build openssh-client p7zip-full parallel patch patchelf perl pigz pkg-config \
      procps psmisc python-is-python3 python3 python3-pip python3-venv python3-yaml rsync shellcheck sqlite3 sudo \
      swig tar texinfo time tk tree tzdata unzip upx-ucl wget xvfb xz-utils zip zstd fonts-liberation \
      fonts-noto-color-emoji libasound2t64 libatk-bridge2.0-0t64 libatk1.0-0t64 libatspi2.0-0t64 libcairo2 \
      libcups2t64 libcurl4t64 libdbus-1-3 libexpat1 libgbm1 libglib2.0-0t64 libgtk-3-0t64 libnspr4 libnss3 \
      libpango-1.0-0 libudev1 libvulkan1 libx11-6 libxcb1 libxcomposite1 libxdamage1 libxext6 libxfixes3 \
      libxkbcommon0 libxrandr2 xdg-utils libsecret-1-0 libxkbfile1 \
    && rm -rf /var/lib/apt/lists/*

# The runner's account: uid 1001 with passwordless sudo, in the groups the hosted image puts it in. A
# step runs as this user, never as root, so a test that depends on a permission behaves as it does there.
RUN userdel -r ubuntu 2>/dev/null || true \
    && groupadd -f docker && groupadd -f systemd-journal \
    && useradd -m -u 1001 -s /bin/bash -G adm,docker,systemd-journal runner \
    && echo 'runner ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/runner && chmod 0440 /etc/sudoers.d/runner \
    && mkdir -p /home/runner/work /home/runner/.cargo /home/runner/.rustup \
    && chown -R runner:runner /home/runner

# Node and Chrome are bind-mounted from the host at these paths; the links stand where the runner image puts them.
RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && ln -s /opt/google/chrome/google-chrome /usr/bin/google-chrome \
    && ln -s /opt/google/chrome/google-chrome /usr/bin/google-chrome-stable

USER runner
WORKDIR /home/runner
