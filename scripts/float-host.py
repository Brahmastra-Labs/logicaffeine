#!/usr/bin/env python3
# float-host — read the nexys_float_soc UART stream and verify the FLOAT LANE on real silicon.
#
# The board streams, for each (a,b) vector, a 12-byte record: 0xA5 sync, vec (1B), result (8B = the f64
# hypotenuse sqrt(a*a+b*b), IEEE-754 big-endian bits), cycles (2B, MSB-first). This reader decodes each f64,
# floors it to the integer hypotenuse, and checks it against the reference (baked in below). cycles is the
# multi-cycle FPU pipeline latency (> 1). PASS iff every record matches — the FPU (fp_mul/fp_addsub/fp_sqrt)
# computing real IEEE-754 arithmetic on the Artix-7.
#
#   python3 scripts/float-host.py [--port DEV|A|B] [--baud 115200]
import argparse, struct, sys, time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

try:
    import serial
except ImportError:
    print("pyserial not installed: pip install --user pyserial", file=sys.stderr)
    sys.exit(2)

import math

# (vec_id, expected_floor_hypotenuse) — floor(sqrt(a*a+b*b)) over the driven float_soc_vectors().
EXPECT = [
    (0, 5),    # hypot(3, 4)   = 5.0
    (1, 10),   # hypot(6, 8)   = 10.0
    (2, 13),   # hypot(5, 12)  = 13.0
    (3, 17),   # hypot(8, 15)  = 17.0
    (4, 7),    # hypot(0, 7)   = 7.0
    (5, 6),    # hypot(2.5, 6) = 6.5 -> floor 6
    (6, 15),   # hypot(9, 12)  = 15.0
]
NVEC = len(EXPECT)
REC = 12


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--timeout", type=float, default=5.0)
    a = ap.parse_args()
    a.port = ftdi_ports.port_arg(a.port, "B")

    ser = serial.Serial(a.port, a.baud, timeout=a.timeout)
    time.sleep(0.1)
    ser.reset_input_buffer()
    need = NVEC * REC
    want = need * 2 + REC
    buf = bytearray()
    t0 = time.time()
    while len(buf) < want and time.time() - t0 < a.timeout * 3:
        chunk = ser.read(want - len(buf))
        if chunk:
            buf.extend(chunk)
    ser.close()

    if len(buf) < need:
        print(f"FAIL: read {len(buf)}/{need} bytes (is the board flashed?)", file=sys.stderr)
        sys.exit(1)

    want_vecs = [v for (v, _) in EXPECT]
    start = -1
    for off in range(0, len(buf) - need + 1):
        if all(buf[off + i * REC] == 0xA5 for i in range(NVEC)) and all(
            buf[off + i * REC + 1] == want_vecs[i] for i in range(NVEC)
        ):
            start = off
            break
    if start < 0:
        print(f"FAIL: no aligned frame found in {len(buf)} bytes: {buf[:48].hex()}", file=sys.stderr)
        sys.exit(1)

    ok = True
    for i in range(NVEC):
        rec = buf[start + i * REC : start + (i + 1) * REC]
        vec = rec[1]
        (fval,) = struct.unpack(">d", rec[2:10])  # IEEE-754 double, big-endian (MSB-first)
        cycles = int.from_bytes(rec[10:12], "big")
        floored = math.floor(fval)
        want_vec, want_res = EXPECT[i]
        good = vec == want_vec and floored == want_res and cycles > 1
        ok = ok and good
        print(
            f"vec {i:2d}: hypot={fval:>8.4f} floor={floored:>4} cycles={cycles}  "
            f"{'OK' if good else f'FAIL (want vec={want_vec} floor={want_res} cycles>1)'}"
        )

    if ok:
        print(
            f"\nPASS — the FLOAT LANE computed all {NVEC} hypotenuses on the Artix-7 "
            f"(fp_mul/fp_addsub/fp_sqrt, real IEEE-754, multi-cycle)."
        )
        sys.exit(0)
    else:
        print("\nFAIL — a float-lane result did not match the reference.", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
