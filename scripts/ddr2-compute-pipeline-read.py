#!/usr/bin/env python3
"""ddr2-compute-pipeline-read — read the end-to-end compute-over-DRAM result over UART.

The ddr2_compute_pipeline bitstream: (1) writes operands A,B into the REAL DDR2; (2) reads them back;
(3) computes A&B with the ComputeDRAM majority engine; (4) writes the RESULT back to the real DDR2 and
reads it. It streams {0xCD, A_hi,A_lo, B_hi,B_lo, eng_hi,eng_lo, ddr2_hi,ddr2_lo, exp_hi,exp_lo, status, 0x5A}.

  scripts/ddr2-compute-pipeline-read.py [--port DEV|A|B]
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
    ap.add_argument("--timeout", type=float, default=12.0)
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
        while len(buf) >= 13:
            if buf[0] != 0xCD or buf[12] != 0x5A:
                buf.pop(0)
                continue
            r = buf[:13]
            A = (r[1] << 8) | r[2]
            B = (r[3] << 8) | r[4]
            eng = (r[5] << 8) | r[6]
            ddr2 = (r[7] << 8) | r[8]
            exp = (r[9] << 8) | r[10]
            status = r[11]
            print(f"raw: {' '.join(f'{x:02x}' for x in r)}\n")
            print(f"  A (read from real DDR2)      = 0x{A:04x}")
            print(f"  B (read from real DDR2)      = 0x{B:04x}")
            print(f"  A & B (ComputeDRAM engine)   = 0x{eng:04x}")
            print(f"  result read back from DDR2   = 0x{ddr2:04x}")
            print(f"  expected A & B               = 0x{exp:04x}")
            print(f"  status                       = {status}\n")
            if status == 1 and eng == exp and ddr2 == exp:
                print("  *** COMPUTE OVER DRAM: operands sourced from and result stored to the real DDR2;")
                print("      the ComputeDRAM majority engine computed A & B; DDR2 read-back matches. ***")
                return 0
            print("  mismatch — see fields above.")
            return 1
    print("no 0xCD record seen within timeout", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
