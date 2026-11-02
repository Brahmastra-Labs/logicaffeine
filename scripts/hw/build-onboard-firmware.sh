#!/usr/bin/env bash
# build-onboard-firmware — a program for the board's own CPU: an example of logicaffeine-compile built for
# riscv64gc-unknown-linux-musl, the Linux ABI the board's hart runs, statically linked against musl's self-contained
# runtime. The compiler crates are built unchanged; the cross toolchain is named here and nowhere else.
#
#   bash scripts/hw/build-onboard-firmware.sh <example> [<cargo profile>]       (profile default: test-opt)
#
# Needs the rustup target (`rustup target add riscv64gc-unknown-linux-musl`) and riscv64-linux-gnu-gcc. The built
# program's path is the last line of output.
set -euo pipefail
example=${1:?usage: build-onboard-firmware.sh <example> [<cargo profile>]}
profile=${2:-test-opt}
root=$(cd "$(dirname "$0")/../.." && pwd)
target=riscv64gc-unknown-linux-musl

export CC_riscv64gc_unknown_linux_musl=riscv64-linux-gnu-gcc
export AR_riscv64gc_unknown_linux_musl=riscv64-linux-gnu-ar
export CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER=riscv64-linux-gnu-gcc
# The cross gcc defaults to _FORTIFY_SOURCE, which turns a C dependency's memcpy into glibc's __memcpy_chk; musl has
# no checked variants, so the crates' C sources are compiled without it.
export CFLAGS_riscv64gc_unknown_linux_musl="-U_FORTIFY_SOURCE"
# One static executable at a fixed address in the board's DDR2: musl's C library from the target's own self-contained
# runtime (never the cross toolchain's glibc), no interpreter and no relocations, so the board's loader copies its
# segments and jumps. qemu-riscv64 runs this same file.
export CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-feature=+crt-static -C link-self-contained=yes \
-C relocation-model=static -C link-arg=-Wl,-Ttext-segment=0x80200000"

cd "$root"
cargo build -q -p logicaffeine-compile --example "$example" --target "$target" --profile "$profile"
dir=$profile
[ "$profile" = dev ] && dir=debug
echo "${CARGO_TARGET_DIR:-$root/target}/$target/$dir/examples/$example"
