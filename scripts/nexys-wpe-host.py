#!/usr/bin/env python3
"""nexys-wpe-host — stream a LOGOS program's flat image OVER THE WIRE to the universal whole-program engine SoC
(`wpe_soc`) on the Nexys A7-100T, run it on real silicon, and read the result back.

Unlike nexys-host.py (which loads a program into the soft-CORE's instruction memory), this speaks to the
config-driven ENGINE: it sends a `.wire` byte stream produced by `WImage::wire_bytes` — for each of the engine's
24 config memories a length-prefixed block, then the params, then the 0x5A GO marker. The on-chip loader fills
its config memories from that stream (the chip SYNTHESIZES the program it is handed), runs the interpret
st-machine, and returns a frame: 0xA5 sync, result[63:0] MSB-first, run_cycles[31:0] MSB-first, 0x5A end.

Usage:
    python3 scripts/nexys-wpe-host.py PROGRAM.wire [--port DEV|A|B] [--baud 115200]
                                      [--expect N] [--clock-mhz 6.25]

PROGRAM.wire is the raw byte stream (one program + its params + GO). Produce it with the `emit_wpe_soc` example
or the wprog_engine_soc_lock test harness (`WImage::wire_bytes`).
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402


def parse_frame(data):
    """Decode 0xA5 + result[63:0] (8 bytes MSB-first, signed) + cycles[31:0] (4 bytes) + 0x5A."""
    if len(data) < 1 + 8 + 4 + 1:
        raise ValueError(f"short frame: {len(data)} bytes ({data.hex()})")
    if data[0] != 0xA5:
        raise ValueError(f"bad sync byte: {data[0]:#04x}")
    raw = int.from_bytes(data[1:9], "big", signed=True)
    cycles = int.from_bytes(data[9:13], "big")
    if data[13] != 0x5A:
        raise ValueError(f"bad end byte: {data[13]:#04x}")
    return raw, cycles


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("wire", help="a .wire byte stream (WImage::wire_bytes output)")
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--expect", type=int, help="expected 64-bit result (fails loudly on mismatch)")
    ap.add_argument("--clock-mhz", type=float, default=6.25, help="core clock for wall-time from cycle count")
    ap.add_argument("--dry-run", action="store_true", help="print the byte stream, do not open the port")
    args = ap.parse_args()

    with open(args.wire, "rb") as f:
        tx = f.read()
    if not tx or tx[-1] != 0x5A:
        sys.exit(f"{args.wire}: not a valid .wire stream (must end with the 0x5A GO marker)")
    print(f"program: {len(tx)} bytes on the wire")

    if args.dry_run:
        print(tx.hex())
        return
    args.port = ftdi_ports.port_arg(args.port, "B")

    import serial  # pyserial; imported late so --dry-run works without it

    with serial.Serial(args.port, args.baud, timeout=5) as ser:
        ser.reset_input_buffer()
        ser.write(tx)
        ser.flush()
        # Wait for the sync byte, then read the fixed 13-byte remainder of the frame.
        head = ser.read(1)
        if not head:
            sys.exit("no reply (timeout waiting for 0xA5 sync)")
        frame = head + ser.read(8 + 4 + 1)

    result, cycles = parse_frame(frame)
    us = cycles / args.clock_mhz
    print(f"result = {result}   on-chip: {cycles} cycles = {us:.3f} us @ {args.clock_mhz} MHz")
    if args.expect is not None:
        if result == args.expect:
            print("OK: matches --expect")
        else:
            sys.exit(f"FAIL: got {result}, expected {args.expect}")


if __name__ == "__main__":
    main()
