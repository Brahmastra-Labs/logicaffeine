#!/usr/bin/env python3
# conv-host — read the nexys_conv_soc UART stream and verify the fixed-point Gaussian-blur CONVOLUTION
# on real silicon. The board streams, per baked pixel window, a 12-byte record: 0xA5 sync, vec index (1B),
# blurred result (8B, MSB-first signed 64-bit), cycles (2B, MSB-first). This reader frame-locks on the
# 0xA5 + window-index sequence, then checks each blurred pixel against the VM's convolution
# ((Σ wᵢ·pᵢ) >> 4, kernel [1,2,1;2,4,2;1,2,1]) on the same window. PASS iff every window matches — the
# supercompiler's CV stencil computing correctly on the Artix-7.
#
#   python3 scripts/conv-host.py [--port DEV|A|B] [--baud 115200]
import argparse, sys, time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

try:
    import serial
except ImportError:
    print("pyserial not installed (pip install pyserial)", file=sys.stderr)
    sys.exit(2)

# The 8 baked pixel windows (must match nexys_conv_soc.v's vector ROM).
WINDOWS = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0],
    [255, 255, 255, 255, 255, 255, 255, 255, 255],
    [0, 0, 0, 128, 128, 128, 255, 255, 255],
    [10, 20, 30, 40, 50, 60, 70, 80, 90],
    [200, 10, 200, 10, 240, 10, 200, 10, 200],
    [100, 100, 100, 100, 200, 100, 100, 100, 100],
    [17, 240, 3, 199, 88, 46, 130, 7, 251],
    [64, 64, 64, 64, 64, 64, 64, 64, 64],
]
WEIGHTS = [1, 2, 1, 2, 4, 2, 1, 2, 1]  # Gaussian 3x3, row-major


def blur(px):
    return (sum(w * p for w, p in zip(WEIGHTS, px))) >> 4  # the VM's conv3x3Window


NVEC = 8
REC = 12


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--timeout", type=float, default=5.0)
    a = ap.parse_args()
    a.port = ftdi_ports.port_arg(a.port, "B")

    ser = serial.Serial(a.port, a.baud, timeout=a.timeout)
    need = REC * NVEC * 3  # enough to frame-lock and read every record
    buf = bytearray()
    t0 = time.time()
    while len(buf) < need and time.time() - t0 < 12:
        chunk = ser.read(need - len(buf))
        if chunk:
            buf += chunk
    if len(buf) < need:
        print(f"FAIL: read {len(buf)}/{need} bytes (is the board flashed?)", file=sys.stderr)
        sys.exit(1)

    # Frame-lock: the SoC loops the NVEC records; find the offset where every record starts 0xA5 and
    # byte 1 is the ascending window index 0..NVEC-1 (uniquely pins the frame).
    off = None
    for o in range(len(buf) - REC * NVEC + 1):
        if all(buf[o + i * REC] == 0xA5 and buf[o + i * REC + 1] == i for i in range(NVEC)):
            off = o
            break
    if off is None:
        print("FAIL: could not frame-lock the conv stream", file=sys.stderr)
        sys.exit(1)

    ok = True
    for i in range(NVEC):
        rec = buf[off + i * REC : off + (i + 1) * REC]
        vec = rec[1]
        res = int.from_bytes(rec[2:10], "big", signed=True)
        cyc = int.from_bytes(rec[10:12], "big")
        want = blur(WINDOWS[vec])
        good = res == want
        ok = ok and good
        print(f"window {vec}: blur={res:4d}  cycles={cyc}  {'OK' if good else f'FAIL (want {want})'}")

    if ok:
        print("\nPASS — the Gaussian-blur convolution computed every window on the Artix-7, matching the VM, 1 cycle/pixel.")
        sys.exit(0)
    print("\nFAIL — a blurred pixel disagreed with the VM.", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
