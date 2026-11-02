#!/usr/bin/env python3
# stream-host — drive the parameterized nexys_stream_soc over UART and verify the LINE-BUFFERED image
# edge-detect on real silicon for an arbitrary M×N tile. The host SENDS an H×W grayscale tile (H*W
# bytes, row-major) into the board's UART RX; the board raster-streams it one pixel/clock through a
# depth-(2W+3) line-buffer shift register, runs `fabric_sobel3` on every interior 3×3 window, and
# streams the (H-2)×(W-2) processed tile back as 13-byte records {0xA5, out_index[15:0] MSB-first,
# edge result[63:0] MSB-first, cycles[15:0]}. Checks the whole processed tile == the VM's Sobel.
# Usage: stream-host.py [--port P] [--w W] [--h H]
import argparse, sys, time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402
try:
    import serial
except ImportError:
    print("pyserial not installed", file=sys.stderr); sys.exit(2)
REC = 13

def sobel_at(img, W, orr, occ):
    p = [img[(orr + i) * W + (occ + j)] for i in range(3) for j in range(3)]
    gx = -p[0] + p[2] - 2*p[3] + 2*p[5] - p[6] + p[8]
    gy = -p[0] - 2*p[1] - p[2] + p[6] + 2*p[7] + p[8]
    return abs(gx) + abs(gy)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--w", type=int, default=16)
    ap.add_argument("--h", type=int, default=16)
    ap.add_argument("--timeout", type=float, default=5.0)
    a = ap.parse_args()
    a.port = ftdi_ports.port_arg(a.port, "B")
    W, H = a.w, a.h
    NOUT = (H - 2) * (W - 2)
    img = [0 if c < W // 2 else 255 for r in range(H) for c in range(W)]   # a vertical edge tile
    expect = [sobel_at(img, W, r, c) for r in range(H - 2) for c in range(W - 2)]

    ser = serial.Serial(a.port, a.baud, timeout=a.timeout)
    ser.reset_input_buffer(); ser.reset_output_buffer()
    t_start = time.time()
    ser.write(bytes(img)); ser.flush()                    # stream the H×W tile into the board
    need = REC * NOUT * 3; buf = bytearray(); t0 = time.time()
    while len(buf) < need and time.time() - t0 < 30:
        c = ser.read(need - len(buf))
        if c: buf += c
    elapsed = time.time() - t_start
    xfer = len(img) + len(buf)
    if len(buf) < need:
        print(f"FAIL: read {len(buf)}/{need} bytes", file=sys.stderr); sys.exit(1)
    off = None
    for o in range(len(buf) - REC*NOUT + 1):
        if all(buf[o+i*REC] == 0xA5 and (buf[o+i*REC+1] << 8 | buf[o+i*REC+2]) == i for i in range(NOUT)):
            off = o; break
    if off is None:
        print("FAIL: could not frame-lock the stream", file=sys.stderr); sys.exit(1)
    out = [int.from_bytes(buf[off+i*REC+3:off+i*REC+11], "big", signed=True) for i in range(NOUT)]
    ok = out == expect
    print(f"processed {H-2}x{W-2} edge image (streamed off the Artix-7, from a {H}x{W} tile):")
    for r in range(H - 2):
        print("  " + " ".join(f"{out[r*(W-2)+c]:4d}" for c in range(W - 2)))
    if ok:
        kbps = xfer / elapsed / 1000.0 if elapsed > 0 else 0.0
        print(f"\nPASS — a {H}x{W} tile streamed in over UART, through line buffers, edge-detected on the Artix-7, every pixel == VM.")
        print(f"link: {a.baud} baud, {xfer} bytes in {elapsed:.3f}s ({kbps:.0f} KB/s)")
        sys.exit(0)
    print("\nexpected:")
    for r in range(H - 2):
        print("  " + " ".join(f"{expect[r*(W-2)+c]:4d}" for c in range(W - 2)))
    print("\nFAIL", file=sys.stderr); sys.exit(1)

if __name__ == "__main__": main()
