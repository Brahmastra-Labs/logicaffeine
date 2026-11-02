#!/usr/bin/env python3
"""coproc-host — read coproc_soc's UART result stream off a flashed ULX3S and verify it against
the software model (the same semantics as codegen_sva/coproc_core.rs::run_core).

The SoC streams: one 0xA5 sync byte, then registers R2..R5 (256-bit each) MSB-byte-first — the
retired results of the CPU-issued BADD / BSUB / BMUL / CMERGE. This host reconstructs them and
checks against the model; PASS proves the coprocessor-issuing core is correct on real silicon.

Usage:
    scripts/coproc-host.py --port B                   # read from Board B (needs pyserial)
    scripts/coproc-host.py --hex <128-byte-hex-dump>  # verify a captured dump (no board)

Python-3 stdlib only; pyserial is used only for --port.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

MASK = (1 << 64) - 1
# The demo operands baked into coproc_core (COPROC_A / COPROC_B), little-endian u64 limbs.
A = [0xFFFFFFFFFFFFFFFF, 0x1111111111111111, 0x2, 0x3]
B = [1, 1, 1, 1]


def limb_add(a, b):
    out, carry = [], 0
    for i in range(4):
        s = a[i] + b[i] + carry
        out.append(s & MASK)
        carry = s >> 64
    return out


def limb_sub(a, b):
    out, borrow = [], 0
    for i in range(4):
        d = a[i] - b[i] - borrow
        out.append(d & MASK)
        borrow = 1 if d < 0 else 0
    return out


def limb_mul_lo(a, b):
    p = (a[0] * b[0]) & ((1 << 128) - 1)
    return [p & MASK, p >> 64, 0, 0]


def merge_low128(a, b):
    def lanes(x):
        return [x[0] & 0xFFFFFFFF, x[0] >> 32, x[1] & 0xFFFFFFFF, x[1] >> 32]
    la, lb = lanes(a), lanes(b)
    m = [max(la[i], lb[i]) for i in range(4)]
    return [m[0] | (m[1] << 32), m[2] | (m[3] << 32), 0, 0]


def model_results():
    """R2..R5 as 256-bit big integers, exactly what run_core retires."""
    def to_int(limbs):
        return sum((limbs[i] & MASK) << (64 * i) for i in range(4))
    return {
        2: to_int(limb_add(A, B)),        # BADD
        3: to_int(limb_sub(A, B)),        # BSUB
        4: to_int(limb_mul_lo(A, B)),     # BMUL
        5: to_int(merge_low128(A, B)),    # CMERGE
    }


def parse_stream(data):
    """data: bytes starting at (or before) the 0xA5 sync; returns {reg: int} for R2..R5."""
    i = data.find(0xA5)
    if i < 0 or len(data) - i - 1 < 128:
        raise ValueError("no sync byte + 128 payload bytes found in the stream")
    body = data[i + 1:i + 1 + 128]
    regs = {}
    for g in range(4):
        chunk = body[g * 32:(g + 1) * 32]           # MSB byte first
        regs[2 + g] = int.from_bytes(chunk, "big")
    return regs


def verify(regs):
    exp = model_results()
    ok = True
    for r in (2, 3, 4, 5):
        tag = {2: "BADD", 3: "BSUB", 4: "BMUL", 5: "CMERGE"}[r]
        got, want = regs[r], exp[r]
        status = "ok" if got == want else "MISMATCH"
        if got != want:
            ok = False
        print("  R%d %-6s : %s  (got 0x%064x)" % (r, tag, status, got))
    print("PASS: coproc_soc issued+retired all four ops correctly on silicon" if ok
          else "FAIL: a retired register does not match the model")
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (A or B)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--hex", help="hex dump of the 129-byte stream (sync + 128), for off-board test")
    args = ap.parse_args()

    if args.hex:
        data = bytes.fromhex(args.hex.replace(" ", "").replace("\n", ""))
    elif args.port:
        import serial  # pyserial
        with serial.Serial(ftdi_ports.port_arg(args.port), args.baud, timeout=2) as s:
            data = s.read(200)
    else:
        ap.error("give --port <device or board letter> or --hex <dump>")
        return 2
    return 0 if verify(parse_stream(data)) else 1


if __name__ == "__main__":
    sys.exit(main())
