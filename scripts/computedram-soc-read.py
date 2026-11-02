#!/usr/bin/env python3
"""computedram-soc-read — read the on-silicon ComputeDRAM engine result over UART.

The computedram_soc bitstream streams an 11-byte record {0xA5, result[63:0] MSB-first, status, 0x5A},
where result = the in-DRAM bitwise AND (MAJ(a,b,0)) of A=0xF0F0F0F0F0F0F0F0 and B=0xFF00FF00FF00FF00,
computed by the ComputeDRAM device on the FPGA. Expected = A & B = 0xF000F000F000F000; status=1 iff
the on-chip compare passed.

  scripts/computedram-soc-read.py [--port DEV|A|B]
"""
import argparse
import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

EXPECT = 0xF000F000F000F000


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--timeout", type=float, default=5.0)
    args = ap.parse_args()
    args.port = ftdi_ports.port_arg(args.port, "B")
    try:
        import serial
    except ImportError:
        print("pyserial not installed: pip install pyserial", file=sys.stderr)
        return 2

    ser = serial.Serial(args.port, args.baud, timeout=args.timeout)
    buf = bytearray()
    import time
    deadline = time.time() + args.timeout + 2
    while time.time() < deadline:
        b = ser.read(64)
        if b:
            buf.extend(b)
        # find a complete record: 0xA5 .. (8) .. status .. 0x5A
        for i in range(len(buf) - 10):
            if buf[i] == 0xA5 and buf[i + 10] == 0x5A:
                result = int.from_bytes(buf[i + 1:i + 9], "big")
                status = buf[i + 9]
                ok = result == EXPECT
                print(f"result = 0x{result:016x}")
                print(f"expected(A&B) = 0x{EXPECT:016x}")
                print(f"on-chip status byte = {status}")
                print("PASS: on-silicon ComputeDRAM AND == oracle" if ok and status == 1
                      else f"FAIL: mismatch (host_ok={ok}, chip_status={status})")
                return 0 if (ok and status == 1) else 1
    print("no valid record received (is the board flashed with computedram_soc.bit?)", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
