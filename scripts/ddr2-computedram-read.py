#!/usr/bin/env python3
"""ddr2-computedram-read — read the ComputeDRAM RowClone sweep result over UART.

The ddr2_computedram bitstream sweeps the sub-JEDEC (T1,T2) timings, each trial writing PAT_A to ROW_SRC
and ~PAT_A to ROW_DST, issuing ACT(SRC)-T1-PRE-T2-ACT(DST), then reading ROW_DST and counting how many
of NCOL(=4) columns became PAT_A (copied by the sense amps) rather than staying ~PAT_A. It streams
{0xCB, T1LO,T1HI,T2LO,T2HI, grid[0..17], lastdst0_hi,lastdst0_lo, patA0_hi,patA0_lo, patB0_hi,patB0_lo, 0x5A}.

grid[i] = copied-count for trial i (row-major over T1 x T2). Any cell > 0 => this DDR2 exhibits in-DRAM
row copy at that timing. A whole-zero grid is a valid, honest result (ComputeDRAM was shown on DDR3).

  scripts/ddr2-computedram-read.py [--port DEV|A|B]
"""
import argparse
import sys
import time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--timeout", type=float, default=15.0)
    args = ap.parse_args()
    args.port = ftdi_ports.port_arg(args.port, "B")
    try:
        import serial
    except ImportError:
        print("pyserial not installed: pip install pyserial", file=sys.stderr)
        return 2

    ser = serial.Serial(args.port, args.baud, timeout=args.timeout)
    buf = bytearray()
    t0 = time.time()
    while time.time() - t0 < args.timeout:
        b = ser.read(1)
        if not b:
            continue
        buf += b
        while len(buf) >= 30:
            if buf[0] != 0xCB or buf[29] != 0x5A:
                buf.pop(0)
                continue
            r = buf[:30]
            t1lo, t1hi, t2lo, t2hi = r[1], r[2], r[3], r[4]
            grid = list(r[5:23])
            lastdst0 = (r[23] << 8) | r[24]
            patA0 = (r[25] << 8) | r[26]
            patB0 = (r[27] << 8) | r[28]
            n2 = t2hi - t2lo + 1
            print(f"raw: {' '.join(f'{x:02x}' for x in r)}")
            print(f"PAT_A[col0]=0x{patA0:04x}  PAT_B[col0]=0x{patB0:04x}  (NCOL=4 per trial)")
            print(f"last-trial ROW_DST[col0] readback = 0x{lastdst0:04x}\n")
            print("copied-count grid (columns copied SRC->DST, max 4):")
            hdr = "  T1\\T2 " + "".join(f"{t2:>4}" for t2 in range(t2lo, t2hi + 1))
            print(hdr)
            best = (0, None, None)
            for i, t1 in enumerate(range(t1lo, t1hi + 1)):
                row = []
                for j, t2 in enumerate(range(t2lo, t2hi + 1)):
                    v = grid[i * n2 + j] if i * n2 + j < len(grid) else 0
                    row.append(v)
                    if v > best[0]:
                        best = (v, t1, t2)
                print(f"  {t1:>4}  " + "".join(f"{v:>4}" for v in row))
            print()
            if best[0] > 0:
                print(f"  *** IN-DRAM ROW COPY DETECTED: {best[0]}/4 columns copied at "
                      f"T1={best[1]}, T2={best[2]} — the real DDR2 computes RowClone ***")
                return 0
            print("  no in-DRAM copy at any swept (T1,T2) — honest null at 125 MHz "
                  "(finer timing / other row pairs are the next lever).")
            return 1
    print("no 0xCB record seen within timeout", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
