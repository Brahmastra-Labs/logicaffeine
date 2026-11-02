#!/usr/bin/env python3
# fabric-host — read the nexys_fabric_soc UART stream and verify the reconfigurable fabric on real silicon.
#
# The board streams, for each corpus vector, a 12-byte record: 0xA5 sync, cfg (1B), result (8B, MSB-first,
# signed 64-bit), cycles (2B, MSB-first). cycles is what the board measured: its settle sweep captures the
# array's output L clocks after launching the vector, and reports the smallest L from which every capture equals
# the settled result (0: the result did not settle inside the sweep's window). This reader collects all NVEC
# records and checks each result against the VM answer (baked in below, derived from the LOGOS corpus) and each
# cycle count against the window. PASS iff every record matches — the fabric computing every datapath correctly
# on the Artix-7, each in the clocks it measured. --record writes the run as a JSON receipt.
#
#   python3 scripts/fabric-host.py [--port DEV|A|B] [--baud 115200] [--lmax 255] [--bit BIT] [--record JSON]
import argparse, datetime, hashlib, json, sys, time
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

try:
    import serial
except ImportError:
    print("pyserial not installed: pip install --user pyserial", file=sys.stderr)
    sys.exit(2)

# (cfg, expected_result) per vector — the VM answers for affine/clamp/sign/min2 over the driven inputs.
EXPECT = [
    (0, 7),    # affine(0)   = 3*0+7
    (0, 16),   # affine(3)   = 3*3+7
    (0, -8),   # affine(-5)  = 3*-5+7
    (1, 5),    # clamp(5,0,10)
    (1, 0),    # clamp(-3,0,10)
    (1, 10),   # clamp(20,0,10)
    (2, -1),   # sign(-7)
    (2, 0),    # sign(0)
    (2, 1),    # sign(9)
    (3, 3),    # min2(3,8)
    (3, 3),    # min2(8,3)
    (4, 3),    # ratio(10,3)  = 10/3   (a DIVISION Func tile on the fabric)
    (4, 14),   # ratio(100,7) = 100/7
    (4, -5),   # ratio(-20,4) = -20/4
]
NVEC = len(EXPECT)
REC = 12  # bytes per record
# The datapath each config word selects, in FABRIC_SOURCES order.
NAMES = ["affine", "clamp", "sign", "min2", "ratio"]


def s64(b):
    v = int.from_bytes(b, "big", signed=True)
    return v


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--timeout", type=float, default=5.0)
    ap.add_argument("--lmax", type=int, default=255, help="the settle sweep's window (the image's LMAX)")
    ap.add_argument("--bit", help="the bitstream the board was flashed with, hashed into the receipt")
    ap.add_argument("--record", help="write the run as a JSON receipt to this path")
    a = ap.parse_args()
    board = (a.port or "B").upper() if (a.port or "B").upper() in ("A", "B") else None
    a.port = ftdi_ports.port_arg(a.port, "B")

    ser = serial.Serial(a.port, a.baud, timeout=a.timeout)
    time.sleep(0.1)
    ser.reset_input_buffer()
    # Pulse nothing — the SoC streams continuously after reset; the host just reads. (Press the reset
    # switch on the board, or power-cycle, to restart the stream.)
    # Read TWO+ frames so we can lock onto a frame boundary regardless of where the stream is.
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
    # 0xA5 AND the config-word sequence matches the corpus order [0,0,0,1,1,1,2,2,2,3,3]. Result bytes are
    # arbitrary, so a single sync byte is not enough — the full sync+cfg pattern pins the frame uniquely.
    want_cfgs = [c for (c, _) in EXPECT]
    start = -1
    for off in range(0, len(buf) - need + 1):
        if all(buf[off + i * REC] == 0xA5 for i in range(NVEC)) and all(
            buf[off + i * REC + 1] == want_cfgs[i] for i in range(NVEC)
        ):
            start = off
            break
    if start < 0:
        print(f"FAIL: no aligned frame found in {len(buf)} bytes: {buf[:48].hex()}", file=sys.stderr)
        sys.exit(1)
    ok = True
    records = []
    for i in range(NVEC):
        rec = buf[start + i * REC : start + (i + 1) * REC]
        if rec[0] != 0xA5:
            print(f"vec {i}: bad sync {rec[0]:#x}")
            ok = False
            continue
        cfg = rec[1]
        result = s64(rec[2:10])
        cycles = int.from_bytes(rec[10:12], "big")
        want_cfg, want_res = EXPECT[i]
        settled = 1 <= cycles <= a.lmax
        good = cfg == want_cfg and result == want_res and settled
        ok = ok and good
        records.append({"vec": i, "cfg": cfg, "kernel": NAMES[cfg] if cfg < len(NAMES) else None,
                        "result": result, "vm_result": want_res, "cycles": cycles, "ok": good})
        why = "" if good else (
            f"FAIL (want cfg={want_cfg} result={want_res})" if settled
            else f"FAIL (did not settle inside the {a.lmax}-clock window)")
        print(f"vec {i:2d}: cfg={cfg} result={result:>6} cycles={cycles:>3}  {'OK' if good else why}")

    per_kernel = {}
    for r in records:
        if r["kernel"] is not None:
            per_kernel[r["kernel"]] = max(per_kernel.get(r["kernel"], 0), r["cycles"])
    print("\nsettle per datapath (worst vector, clocks of the board clock): "
          + ", ".join(f"{k} {v}" for k, v in per_kernel.items()))
    if a.record:
        receipt = {
            "date_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
            "board": board,
            "port": a.port,
            "baud": a.baud,
            "lmax": a.lmax,
            "bit": a.bit,
            "bit_sha256": hashlib.sha256(open(a.bit, "rb").read()).hexdigest() if a.bit else None,
            "pass": ok,
            "records": records,
            "settle_per_kernel": per_kernel,
        }
        with open(a.record, "w") as f:
            json.dump(receipt, f, indent=2)
            f.write("\n")
        print(f"receipt: {a.record}")

    if ok:
        print(f"\nPASS — the reconfigurable fabric computed all {NVEC} datapaths on the Artix-7 ≡ the VM, each in the clocks it measured.")
        sys.exit(0)
    else:
        print("\nFAIL — a datapath did not match the VM or did not settle inside the window.", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
