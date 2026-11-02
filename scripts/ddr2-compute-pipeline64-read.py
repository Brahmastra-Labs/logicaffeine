#!/usr/bin/env python3
"""ddr2-compute-pipeline64-read — read the 64-bit AND/OR/MAJ compute-over-DRAM result over UART.

The ddr2_compute_pipeline64 bitstream, for each op in {AND, OR, MAJ}, writes 64-bit operands into the
real DDR2, reads them back, computes maj3(A,B,Z) with the ComputeDRAM engine (Z = 0 | all-1s | C),
writes the 64-bit result back to the real DDR2 and reads it. It streams {0xCE, A(8), B(8), C(8),
AND_ddr2(8), OR_ddr2(8), MAJ_ddr2(8), status, 0x5A} (MSB-first); status bit i = op i verified on-chip.

  scripts/ddr2-compute-pipeline64-read.py [--port DEV|A|B]
"""
import argparse
import sys
import time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

MASK = (1 << 64) - 1


def be64(bs):
    v = 0
    for x in bs:
        v = (v << 8) | x
    return v


def maj3(a, b, c):
    return (a & b) | (a & c) | (b & c)


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
        while len(buf) >= 51:
            if buf[0] != 0xCE or buf[50] != 0x5A:
                buf.pop(0)
                continue
            r = buf[:51]
            A = be64(r[1:9]); B = be64(r[9:17]); C = be64(r[17:25])
            andD = be64(r[25:33]); orD = be64(r[33:41]); majD = be64(r[41:49])
            status = r[49]
            oand, oor, omaj = (A & B) & MASK, (A | B) & MASK, maj3(A, B, C) & MASK
            print(f"raw status=0b{status:03b}\n")
            print(f"  A = 0x{A:016x}   (read from real DDR2)")
            print(f"  B = 0x{B:016x}   (read from real DDR2)")
            print(f"  C = 0x{C:016x}   (read from real DDR2)\n")
            rows = [
                ("AND  A&B     ", andD, oand, status & 1),
                ("OR   A|B     ", orD, oor, (status >> 1) & 1),
                ("MAJ  maj(A,B,C)", majD, omaj, (status >> 2) & 1),
            ]
            allok = True
            for name, got, exp, sbit in rows:
                ok = (got == exp) and (sbit == 1)
                allok &= ok
                mark = "OK" if ok else "MISMATCH"
                print(f"  {name} = 0x{got:016x}  (DDR2 read-back)  oracle=0x{exp:016x}  on-chip={sbit}  [{mark}]")
            print()
            if allok:
                print("  *** 64-bit AND/OR/MAJ COMPUTE OVER DRAM: all three ops verified — operands and")
                print("      results in the real DDR2 die, ComputeDRAM majority engine computed each. ***")
                return 0
            print("  one or more ops mismatched — see rows above.")
            return 1
    print("no 0xCE record seen within timeout", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
