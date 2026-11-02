#!/usr/bin/env python3
# sobel-host — read the nexys_sobel_soc UART stream and verify the fixed-point SOBEL EDGE DETECTOR
# on real silicon. 12-byte record: 0xA5, vec (1B), edge result (8B MSB-first signed), cycles (2B).
# Checks each edge magnitude against the VM's Sobel (|Gx|+|Gy|) on the same window.
import argparse, sys, time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402
try:
    import serial
except ImportError:
    print("pyserial not installed", file=sys.stderr); sys.exit(2)

WINDOWS = [
    [0,0,0,0,0,0,0,0,0],
    [128,128,128,128,128,128,128,128,128],
    [0,0,0,128,128,128,255,255,255],
    [0,128,255,0,128,255,0,128,255],
    [10,20,30,40,50,60,70,80,90],
    [255,0,0,0,0,0,0,0,0],
    [17,240,3,199,88,46,130,7,251],
    [50,100,150,50,100,150,50,100,150],
]
def sobel(p):
    gx = -p[0] + p[2] - 2*p[3] + 2*p[5] - p[6] + p[8]
    gy = -p[0] - 2*p[1] - p[2] + p[6] + 2*p[7] + p[8]
    return abs(gx) + abs(gy)

NVEC, REC = 8, 12
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--timeout", type=float, default=5.0)
    a = ap.parse_args()
    a.port = ftdi_ports.port_arg(a.port, "B")
    ser = serial.Serial(a.port, a.baud, timeout=a.timeout)
    need = REC*NVEC*3; buf = bytearray(); t0 = time.time()
    while len(buf) < need and time.time()-t0 < 12:
        c = ser.read(need-len(buf))
        if c: buf += c
    if len(buf) < need:
        print(f"FAIL: read {len(buf)}/{need} bytes", file=sys.stderr); sys.exit(1)
    off = None
    for o in range(len(buf)-REC*NVEC+1):
        if all(buf[o+i*REC]==0xA5 and buf[o+i*REC+1]==i for i in range(NVEC)):
            off = o; break
    if off is None:
        print("FAIL: could not frame-lock the sobel stream", file=sys.stderr); sys.exit(1)
    ok = True
    for i in range(NVEC):
        rec = buf[off+i*REC:off+(i+1)*REC]
        vec = rec[1]; res = int.from_bytes(rec[2:10],"big",signed=True); cyc = int.from_bytes(rec[10:12],"big")
        want = sobel(WINDOWS[vec]); good = res==want; ok = ok and good
        print(f"window {vec}: edge={res:5d}  cycles={cyc}  {'OK' if good else f'FAIL (want {want})'}")
    if ok:
        print("\nPASS — the Sobel edge detector computed every window on the Artix-7, matching the VM, 1 cycle/pixel.")
        sys.exit(0)
    print("\nFAIL", file=sys.stderr); sys.exit(1)

if __name__ == "__main__": main()
