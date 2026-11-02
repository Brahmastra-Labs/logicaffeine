#!/usr/bin/env python3
"""ddr2-dbquery-read — read the toy compute-in-DRAM DATABASE query result off the real FPGA.

The ddr2_dbquery bitstream models a strictly-typed `users` table (16 rows) with two bitmap columns
resident in the real DDR2: `active` and `region=EU`. It runs the query

    SELECT COUNT(*) FROM users WHERE active AND NOT region = 'EU'

as in-DRAM set algebra — `active \\ eu = active AND NOT eu` — via the ComputeDRAM engine (Ambit NOT +
majority), stores the match bitmap back to DDR2, and counts the matches ON-CHIP. It streams the
14-byte record {0xDB, active(2), eu(2), match(2), ddr2(2), exp(2), count, status, 0x5A}.

  scripts/ddr2-dbquery-read.py [--port DEV|A|B]
"""
import argparse
import sys
import time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402


def bits(v, n=16):
    return "".join("1" if (v >> i) & 1 else "0" for i in range(n))


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
        while len(buf) >= 14:
            if buf[0] != 0xDB or buf[13] != 0x5A:
                buf.pop(0)
                continue
            r = buf[:14]
            active = (r[1] << 8) | r[2]
            eu = (r[3] << 8) | r[4]
            match = (r[5] << 8) | r[6]
            ddr2 = (r[7] << 8) | r[8]
            exp = (r[9] << 8) | r[10]
            count = r[11]
            status = r[12]
            ref_match = active & ~eu & 0xFFFF
            ref_count = bin(ref_match).count("1")
            print("  toy compute-in-DRAM DATABASE — table `users` (16 rows), bitmap columns in real DDR2\n")
            print("  SELECT COUNT(*) FROM users WHERE active AND NOT region = 'EU'\n")
            print(f"  active  bitmap (from DDR2) = {bits(active)}  ({bin(active).count('1')} active users)")
            print(f"  region=EU bitmap (from DDR2)= {bits(eu)}  ({bin(eu).count('1')} EU users)")
            print(f"  match = active \\ EU (in-DRAM) = {bits(match)}   (engine result 0x{match:04x})")
            print(f"  match re-read from DDR2       = 0x{ddr2:04x}")
            print(f"  COUNT(*) (on-chip popcount)   = {count}")
            print(f"  reference (host)              = 0x{ref_match:04x}, COUNT={ref_count}, status={status}\n")
            ok = (match == ref_match) and (ddr2 == ref_match) and (count == ref_count) and status == 1
            if ok:
                print(f"  *** DB QUERY RAN ON THE REAL FPGA: COUNT = {count} "
                      f"(active-and-not-EU), computed in-DRAM, verified. ***")
                return 0
            print("  MISMATCH — see fields above.")
            return 1
    print("no 0xDB record seen within timeout", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
