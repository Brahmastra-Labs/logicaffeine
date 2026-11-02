#!/usr/bin/env python3
"""ddr2-stage2c-read — dump the DDR2 Stage-2c read-eye across all 32 capture taps.

The ddr2_phy_stage2c bitstream writes PAT=0xA5C3 to bank0/row0/col0, then re-reads it and captures the
16-bit DQ word at 32 successive capture-cycle offsets (tap 0..31) after the READ command. It streams the
66-byte record {0xC7, cap[0][15:8], cap[0][7:0], ..., cap[31][15:8], cap[31][7:0], 0x5A}.

Printing every tap's captured word shows the whole read eye: where the DRAM starts/stops driving, what
the read-window value actually is, and whether it equals PAT (round-trip works), ~PAT (systematic
inversion), or the idle level (0xffff).

  scripts/ddr2-stage2c-read.py [--port DEV|A|B]
"""
import argparse
import sys
import time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

def classify(v, pat, inv):
    if v == pat:
        return "== PAT"
    if v == inv:
        return "== ~PAT"
    if v == 0xFFFF:
        return "idle(1s)"
    if v == 0x0000:
        return "idle(0s)"
    return ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--timeout", type=float, default=10.0)
    ap.add_argument("--pat", type=lambda s: int(s, 0), default=0xA5C3)
    args = ap.parse_args()
    args.port = ftdi_ports.port_arg(args.port, "B")
    PAT = args.pat & 0xFFFF
    INV = (~PAT) & 0xFFFF
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
        while len(buf) >= 68:
            if buf[0] != 0xC7:
                buf.pop(0)
                continue
            if buf[67] != 0x5A:
                buf.pop(0)
                continue
            rec = buf[:68]
            selfcap = (rec[1] << 8) | rec[2]
            words = [(rec[3 + 2 * k] << 8) | rec[4 + 2 * k] for k in range(32)]
            print(f"PAT=0x{PAT:04x}  ~PAT=0x{INV:04x}\n")
            stag = classify(selfcap, PAT, INV)
            print(f"  selfcap (FPGA reads own driven pad during write) = 0x{selfcap:04x} {stag}")
            if selfcap == PAT:
                print("    -> FPGA I/O is CLEAN under drive; the inversion is in the DRAM read path "
                      "(DRAM stores true data) -> correct on READ.")
            elif selfcap == INV:
                print("    -> the pad reads ~PAT while driving PAT (I/O inversion / contention) "
                      "-> DRAM stores ~data -> correct on WRITE.")
            else:
                print("    -> selfcap inconclusive (not PAT/~PAT).")
            print()
            npat = ninv = 0
            for k, v in enumerate(words):
                tag = classify(v, PAT, INV)
                if v == PAT:
                    npat += 1
                if v == INV:
                    ninv += 1
                print(f"  tap {k:2d}: 0x{v:04x} {tag}")
            print()
            if npat:
                print(f"  *** WRITE->READ ROUND-TRIP WORKS: {npat} taps read back PAT=0x{PAT:04x} ***")
            elif ninv:
                print(f"  {ninv} taps read back ~PAT (0x{INV:04x}) -> systematic 1-bit inversion; "
                      f"data round-trips, polarity/edge to fix.")
            else:
                nz = [f"tap{k}=0x{v:04x}" for k, v in enumerate(words) if v not in (0xFFFF, 0x0000)]
                print(f"  no PAT/~PAT match. non-idle taps: {', '.join(nz) if nz else '(none)'}")
            return 0
    print("no 0xC7 record seen within timeout", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
