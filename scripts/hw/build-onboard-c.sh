#!/usr/bin/env bash
# build-onboard-c — the board firmware's C, from benchmarks/silicon/onboard/firmware against the soc.h the hart's SoC
# definition writes (`onboard_image header`): the boot ROM, a raw binary for the ROM's block RAM, and the shim, a raw
# binary for SHIM_BASE. The hart has no floating-point unit, so everything is built for rv64imac with the soft-float
# ABI and no libgcc (the cross toolchain's is built for the hard-float ABI). The boot image's build and every test
# that boots the firmware compile it here, and nowhere else.
#
#   bash scripts/hw/build-onboard-c.sh <out dir> <soc.h>
#
# Writes <out>/boot.bin and <out>/shim.bin, with their ELFs beside them.
set -euo pipefail
out=${1:?usage: build-onboard-c.sh <out dir> <soc.h>}
soc=${2:?usage: build-onboard-c.sh <out dir> <soc.h>}
root=$(cd "$(dirname "$0")/../.." && pwd)
fw=$root/benchmarks/silicon/onboard/firmware
sf=$fw/softfloat
mkdir -p "$out/obj"
cp "$soc" "$out/soc.h"

cc=riscv64-linux-gnu-gcc
arch="-march=rv64imac_zicsr_zifencei -mabi=lp64 -mcmodel=medany -fno-pie -ffreestanding -fno-builtin -ffunction-sections -fdata-sections"
own="$arch -Wall -Wextra -Werror -I$out -I$fw -I$sf/source/include -I$sf/build/Linux-RISCV64-GCC"
softfloat="$arch -O2 -DSOFTFLOAT_FAST_INT64 -DSOFTFLOAT_ROUND_ODD -DINLINE_LEVEL=5 -I$sf/source/include -I$sf/source/RISCV -I$sf/build/Linux-RISCV64-GCC"
link="-march=rv64imac -mabi=lp64 -nostdlib -static -no-pie -Wl,--gc-sections -Wl,--no-warn-rwx-segments -Wl,--build-id=none"

sf_objs=()
for f in "$sf"/source/*.c "$sf"/source/RISCV/*.c; do
  o="$out/obj/sf_$(basename "$f" .c).o"
  $cc $softfloat -c "$f" -o "$o"
  sf_objs+=("$o")
done

# The boot ROM: small (-Os), it has 16 KB.
boot_objs=()
for f in boot_start.S boot.c inflate.c sha256.c libc.c; do
  o="$out/obj/boot_${f%.*}.o"
  $cc $own -Os -c "$fw/$f" -o "$o"
  boot_objs+=("$o")
done
$cc $link -T "$fw/boot.ld" "${boot_objs[@]}" -o "$out/boot.elf"
riscv64-linux-gnu-objcopy -O binary "$out/boot.elf" "$out/boot.bin"

shim_objs=()
for f in shim_start.S shim.c fpu.c rvc.c libc.c; do
  o="$out/obj/shim_${f%.*}.o"
  $cc $own -O2 -c "$fw/$f" -o "$o"
  shim_objs+=("$o")
done
$cc $link -T "$fw/shim.ld" "${shim_objs[@]}" "${sf_objs[@]}" -o "$out/shim.elf"
riscv64-linux-gnu-objcopy -O binary "$out/shim.elf" "$out/shim.bin"
