#!/usr/bin/env python3
# stream-battery — robustness battery for the reusable, size-parameterized nexys_stream_soc streaming
# edge detector. Streams a series of DISTINCT H×W tiles into the board (each new transfer reloads the
# pipeline) and checks every processed (H-2)×(W-2) result == the reference Sobel. Proves the silicon
# line buffers + stencil handle ARBITRARY runtime images at the synthesized resolution.
# Usage: stream-battery.py [port] [W] [H]   (defaults: Board B's UART, found by its FTDI serial; 16 16)
import os, serial, sys, time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402
REC = 13

def sobel_at(img, W, orr, occ):
    p = [img[(orr + i) * W + (occ + j)] for i in range(3) for j in range(3)]
    gx = -p[0] + p[2] - 2*p[3] + 2*p[5] - p[6] + p[8]
    gy = -p[0] - 2*p[1] - p[2] + p[6] + 2*p[7] + p[8]
    return abs(gx) + abs(gy)

def expect(img, W, H): return [sobel_at(img, W, r, c) for r in range(H - 2) for c in range(W - 2)]

def patterns(W, H):
    def mk(f): return [f(r, c) for r in range(H) for c in range(W)]
    x = 0xC0FFEE; rnd = []
    for _ in range(W * H):
        x = (1103515245 * x + 12345) & 0x7fffffff
        rnd.append((x >> 16) & 0xff)
    return {
        "vertical-edge":   mk(lambda r, c: 0 if c < W // 2 else 255),
        "horizontal-edge": mk(lambda r, c: 0 if r < H // 2 else 255),
        "diagonal-edge":   mk(lambda r, c: 0 if (r + c) < (W + H) // 2 else 255),
        "x-ramp":          mk(lambda r, c: min(255, c * (256 // W))),
        "flat-128":        mk(lambda r, c: 128),
        "center-box":      mk(lambda r, c: 255 if (H//4 <= r < 3*H//4 and W//4 <= c < 3*W//4) else 0),
        "pseudo-random":   rnd,
    }

def read_result(ser, NOUT):
    need = REC * NOUT * 3; buf = bytearray(); t0 = time.time()
    while len(buf) < need and time.time() - t0 < 30:
        c = ser.read(need - len(buf))
        if c: buf += c
    for o in range(len(buf) - REC*NOUT + 1):
        if all(buf[o+i*REC] == 0xA5 and (buf[o+i*REC+1] << 8 | buf[o+i*REC+2]) == i for i in range(NOUT)):
            return [int.from_bytes(buf[o+i*REC+3:o+i*REC+11], "big", signed=True) for i in range(NOUT)]
    return None

def main():
    port = ftdi_ports.port_arg(sys.argv[1] if len(sys.argv) > 1 else None, "B")
    W = int(sys.argv[2]) if len(sys.argv) > 2 else 16
    H = int(sys.argv[3]) if len(sys.argv) > 3 else 16
    NOUT = (H - 2) * (W - 2)
    ser = serial.Serial(port, 115200, timeout=2)
    allok = True
    print(f"streaming {H}x{W} tiles → {H-2}x{W-2} edge maps ({NOUT} px each):")
    for name, img in patterns(W, H).items():
        ser.reset_input_buffer(); ser.reset_output_buffer()
        ser.write(bytes(img)); ser.flush()
        time.sleep(0.3)
        ser.reset_input_buffer()
        out = read_result(ser, NOUT)
        exp = expect(img, W, H)
        ok = out == exp
        allok = allok and ok
        print(f"  {name:16s} {'PASS' if ok else 'FAIL'}")
        if not ok and out is not None:
            bad = [(i, out[i], exp[i]) for i in range(min(len(out), len(exp))) if out[i] != exp[i]]
            print(f"   {len(bad)} mismatches, first: {bad[:5]}")
        elif not ok:
            print("   no frame-lock")
    print(f"\nALL PATTERNS PASS — arbitrary {H}x{W} tiles stream through the line buffers == VM on the Artix-7."
          if allok else "\nSOME PATTERNS FAILED")
    sys.exit(0 if allok else 1)

if __name__ == "__main__": main()
