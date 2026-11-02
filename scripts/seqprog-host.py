#!/usr/bin/env python3
# seqprog-host — read the whole-program sequenced fabric's UART records and check each result ≡ the VM oracle.
import argparse, sys, serial
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=9600)
    ap.add_argument("--expect", default="", help="comma-separated VM oracle results, in vector order")
    ap.add_argument("--n", type=int, default=3, help="how many records to read")
    a = ap.parse_args()
    a.port = ftdi_ports.port_arg(a.port, "B")
    exp = [int(x) for x in a.expect.split(",")] if a.expect else []
    s = serial.Serial(a.port, a.baud, timeout=5)
    # sync to 0xA5
    got = []
    while len(got) < a.n:
        b = s.read(1)
        if not b:
            print("TIMEOUT waiting for 0xA5"); sys.exit(2)
        if b[0] != 0xA5:
            continue
        rec = s.read(11)   # result[8] + cyc[2] + 0x5A
        if len(rec) != 11 or rec[10] != 0x5A:
            continue
        val = int.from_bytes(rec[0:8], "big", signed=True)
        cyc = int.from_bytes(rec[8:10], "big")
        got.append((val, cyc))
    ok = True
    for i, (val, cyc) in enumerate(got):
        tag = ""
        print(f"record {i}: result={val} cycles={cyc}")
    # The fabric loops the vector ROM continuously, so the host locks on at an ARBITRARY phase — compare the
    # returned results to the VM oracle as a MULTISET (phase-independent).
    ok = (sorted(v for v, _ in got) == sorted(exp)) if exp else True
    print(f"results {sorted(v for v,_ in got)} vs VM {sorted(exp)} -> {'PASS' if ok else 'FAIL'}")
    sys.exit(0 if ok else 1)
main()
