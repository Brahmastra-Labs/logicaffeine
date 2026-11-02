#!/usr/bin/env python3
"""Read-only: walk the D5005's FME device feature list in BAR0 and print the PR feature's registers
(drivers/fpga/dfl-fme-mgr.c offsets and fields)."""
import mmap, os, struct, sys

BAR = "/sys/bus/pci/devices/0000:01:00.0/resource0"
fd = os.open(BAR, os.O_RDONLY | os.O_SYNC)
size = os.fstat(fd).st_size
m = mmap.mmap(fd, size, mmap.MAP_SHARED, mmap.PROT_READ)
rd = lambda off: struct.unpack_from("<Q", m, off)[0]

off = 0
features = []
while True:
    dfh = rd(off)
    fid, rev, nxt, eol, typ = dfh & 0xfff, (dfh >> 12) & 0xf, (dfh >> 16) & 0xffffff, (dfh >> 40) & 1, dfh >> 60
    features.append((off, typ, fid, rev))
    print(f"DFH @0x{off:06x}: type {typ} id 0x{fid:03x} rev {rev} next 0x{nxt:x} eol {eol}")
    if eol or nxt == 0 or len(features) > 64:
        break
    off += nxt

pr = next((o for o, t, f, r in features if t == 3 and f == 0x5), None)
if pr is None:
    sys.exit("no FME PR feature (private feature id 0x5) in the list")
ctrl, sts, err = rd(pr + 0x8), rd(pr + 0x10), rd(pr + 0x20)
print(f"\nFME PR feature @0x{pr:06x}")
print(f"PR_CTRL 0x{ctrl:016x}: RST {ctrl & 1} RSTACK {(ctrl >> 4) & 1} RGN_ID {(ctrl >> 7) & 7} START {(ctrl >> 12) & 1} COMPLETE {(ctrl >> 13) & 1}")
print(f"PR_STS  0x{sts:016x}: CREDIT {sts & 0x1ff} STS {(sts >> 16) & 1} CTRLR_STS {(sts >> 20) & 7} HOST_STS {(sts >> 24) & 0xf}")
print(f"PR_ERR  0x{err:016x}: OPERATION {err & 1} CRC {(err >> 1) & 1} INCOMPATIBLE {(err >> 2) & 1} PROTOCOL {(err >> 3) & 1} FIFO_OVERFLOW {(err >> 4) & 1}")
lo, hi = rd(pr + 0xA8), rd(pr + 0xB0)
print(f"PR_INTFC_ID {hi:016x}{lo:016x}")
