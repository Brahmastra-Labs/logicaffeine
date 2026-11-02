#!/usr/bin/env python3
"""ddr2-memtest-read — read the DDR2 memtest result over UART.

The ddr2_memtest bitstream writes {col,~col} to 16 BL8-aligned columns of bank0/row0, re-opens the row,
reads every column back and compares on-chip, then streams the 11-byte record
{0xC9, npass_lo, npass_hi, ntest_lo, ntest_hi, ff_col, ff_got_hi, ff_got_lo, ff_exp_hi, ff_exp_lo, 0x5A}.

npass==ntest => every address round-tripped on the real DDR2 die (Phase 1e gate).

  scripts/ddr2-memtest-read.py [--port DEV|A|B]
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
    ap.add_argument("--timeout", type=float, default=10.0)
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
        while len(buf) >= 11:
            if buf[0] != 0xC9 or buf[10] != 0x5A:
                buf.pop(0)
                continue
            r = buf[:11]
            npass = r[1] | (r[2] << 8)
            ntest = r[3] | (r[4] << 8)
            ffcol = r[5]
            ffgot = (r[6] << 8) | r[7]
            ffexp = (r[8] << 8) | r[9]
            print(f"raw: {' '.join(f'{x:02x}' for x in r)}")
            print(f"  npass = {npass} / {ntest}")
            if ntest and npass == ntest:
                print(f"\n  *** MEMTEST PASS: all {ntest} addresses round-tripped on the real DDR2 die ***")
                return 0
            print(f"  first failure: col=0x{ffcol:02x}  got=0x{ffgot:04x}  expected=0x{ffexp:04x}")
            return 1
    print("no 0xC9 record seen within timeout", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
