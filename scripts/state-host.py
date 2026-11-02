#!/usr/bin/env python3
# state-host — read the nexys_state_soc UART stream and verify the STATE PLANE on real silicon.
#
# The board streams, for each grade vector, a 12-byte record: 0xA5 sync, vec (1B), result (8B, MSB-first,
# signed 64-bit), cycles (2B, MSB-first). Unlike the combinational fabric (1 cycle/op), a state-plane kernel
# is a SEQUENCED heap datapath, so `cycles` is a MULTI-cycle latency (> 1) — the honest cost of running a
# collection program on silicon. This reader collects all NVEC records and checks each result against the VM
# answer (grade = clamp(list sum, 0, 100) — collections + arithmetic + control flow) and each cycle count > 1.
#
#   python3 scripts/state-host.py [--port DEV|A|B] [--baud 115200]
import argparse, sys, time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

try:
    import serial
except ImportError:
    print("pyserial not installed: pip install --user pyserial", file=sys.stderr)
    sys.exit(2)

# (vec_id, expected_result) per vector — the VM's grade([a,b,c]) = clamp(a+b+c, 0, 100), a WHOLE multi-op-class
# program (collections + arithmetic + control flow) over the driven state_soc_vectors().
EXPECT = [
    (0, 60),    # grade(10, 20, 30)     = 60  (pass-through)
    (1, 6),     # grade(1, 2, 3)        = 6   (pass-through)
    (2, 100),   # grade(40, 40, 40)     = 120 -> 100 (upper clamp)
    (3, 0),     # grade(-40, -40, -40)  = -120 -> 0 (lower clamp)
    (4, 99),    # grade(33, 33, 33)     = 99  (pass-through)
    (5, 0),     # grade(0, 0, 0)        = 0
    (6, 95),    # grade(70, 20, 5)      = 95  (pass-through)
]
NVEC = len(EXPECT)
REC = 12  # bytes per record


def s64(b):
    return int.from_bytes(b, "big", signed=True)


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

    # Frame-lock: the SoC loops the NVEC records forever, so find the offset where every record starts with
    # 0xA5 AND the vec-id sequence matches [0,1,2,...,NVEC-1]. Result bytes are arbitrary, so the full
    # sync+vec pattern pins the frame uniquely.
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
        if rec[0] != 0xA5:
            print(f"vec {i}: bad sync {rec[0]:#x}")
            ok = False
            continue
        vec = rec[1]
        result = s64(rec[2:10])
        cycles = int.from_bytes(rec[10:12], "big")
        want_vec, want_res = EXPECT[i]
        good = vec == want_vec and result == want_res and cycles > 1
        ok = ok and good
        print(
            f"vec {i:2d}: result={result:>6} cycles={cycles}  "
            f"{'OK' if good else f'FAIL (want vec={want_vec} result={want_res} cycles>1)'}"
        )

    if ok:
        print(
            f"\nPASS — the STATE PLANE ran all {NVEC} grade programs on the Artix-7 "
            f"(collections + arithmetic + control flow, a sequenced heap datapath, multi-cycle)."
        )
        sys.exit(0)
    else:
        print("\nFAIL — a state-plane result did not match the VM.", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
