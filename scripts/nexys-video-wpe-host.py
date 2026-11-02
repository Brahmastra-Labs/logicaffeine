#!/usr/bin/env python3
"""nexys-video-wpe-host — stream a .wire byte stream to the whole-program engine on Board A (Nexys Video,
XC7A200T) and read the result back, over the board's FT2232H **DPTI parallel FIFO**.

The Nexys Video's FT2232H carries JTAG on channel B and a 245-style asynchronous FIFO (the "DPTI") on channel A.
On the FPGA an `ft245_uart_bridge` adapts that FIFO to the engine's serial uart_rx/uart_tx, so the UNMODIFIED
`wpe_soc_stream` engine runs unchanged. The OS presents the FIFO channel as a serial port whose baud the FIFO
ignores, found by scripts/lib/ftdi_ports.py as the board's byte channel. The wire/result protocol is identical to
nexys-wpe-host.py: send the .wire bytes (per-memory length-prefixed config + params + optional 0x48 heap-preload
segment + 0x5A GO), then read a 14-byte reply frame: 0xA5 sync, result[63:0] MSB-first, cycles[31:0], 0x5A.

    python3 scripts/nexys-video-wpe-host.py /tmp/transformer.wire [--port A] [--expect 80113]

Requires: pip install pyserial. The FPGA must be POWERED and flashed with nexys_video_wpe_stream.bit.
"""
import argparse
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402


def parse_frame(data):
    """Decode 0xA5 + result[63:0] (8 bytes MSB-first, signed) + cycles[31:0] (4 bytes) + 0x5A."""
    if len(data) < 14:
        raise ValueError(f"short frame: {len(data)} bytes ({bytes(data).hex()})")
    if data[0] != 0xA5:
        raise ValueError(f"bad sync 0x{data[0]:02x} (want 0xA5); frame={bytes(data[:14]).hex()}")
    raw = int.from_bytes(data[1:9], "big", signed=True)
    cycles = int.from_bytes(data[9:13], "big")
    if data[13] != 0x5A:
        raise ValueError(f"bad end 0x{data[13]:02x} (want 0x5A); frame={bytes(data[:14]).hex()}")
    return raw, cycles


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("wire", help="a .wire byte stream (WImage::wire_bytes / wire_bytes_preloaded output)")
    ap.add_argument("--port", help="the board (A, or its FTDI serial) or a serial device; Board A's byte channel by default")
    ap.add_argument("--expect", type=int, help="expected 64-bit result (fails loudly on mismatch)")
    ap.add_argument("--clock-mhz", type=float, default=0.78125, help="core clock (clk/128) for wall-time")
    ap.add_argument("--timeout", type=float, default=60.0, help="seconds to wait for the reply frame")
    ap.add_argument("--dry-run", action="store_true", help="print the byte stream, do not open the FIFO")
    args = ap.parse_args()

    with open(args.wire, "rb") as f:
        tx = f.read()
    if not tx or tx[-1] != 0x5A:
        sys.exit(f"{args.wire}: not a valid .wire stream (must end with the 0x5A GO marker)")
    print(f"program: {len(tx)} bytes on the wire")
    if args.dry_run:
        print(tx.hex())
        return

    import serial  # imported late so --dry-run works without pyserial

    port = serial.Serial(ftdi_ports.port_arg(args.port, "A"), 115200, timeout=0.05)
    # A byte the reset left on the line would sit ahead of the frame: settle, drain, then stream.
    try:
        time.sleep(0.3)
        port.read(256)
        port.reset_input_buffer()
        port.write(bytes(tx))
        port.flush()
        buf = bytearray()
        t0 = time.time()
        while len(buf) < 14 and (time.time() - t0) < args.timeout:
            buf += port.read(14 - len(buf))
    finally:
        port.close()

    if len(buf) < 14:
        sys.exit(f"no/short reply after {args.timeout}s ({len(buf)} bytes: {bytes(buf).hex()})")
    result, cycles = parse_frame(buf)
    us = cycles / args.clock_mhz
    print(f"result = {result}   on-chip: {cycles} cycles = {us:.1f} us @ {args.clock_mhz} MHz")
    if args.expect is not None:
        if result == args.expect:
            print(f"OK: matches --expect ({args.expect}) — the whole LLM ran on the 200T fabric")
        else:
            sys.exit(f"FAIL: got {result}, expected {args.expect}")


if __name__ == "__main__":
    main()
