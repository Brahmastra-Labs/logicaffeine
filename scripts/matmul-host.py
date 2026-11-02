#!/usr/bin/env python3
"""matmul-host — drive the int8 dot-product accelerator SoC (nexys_matmul_soc) over UART.

Sends two int8 vectors INTERLEAVED [L][a0,b0,a1,b1,…] and reads back the 11-byte record
{0xA5, result[63:0] MSB-first, cycles[15:0]}. The board computes a·b on the LOGOS-emitted int8 DSP MAC
array (1 MAC/clock) and returns the answer + measured cycle count (= interleaved element count = 2·pairs).

  python3 scripts/matmul-host.py --a 3,-4,5 --b 6,7,-8          # real board
  python3 scripts/matmul-host.py --demo                         # four dots back-to-back on one connection
  python3 scripts/matmul-host.py --selftest                     # frame/oracle check, no board

Without --port the UART is found by Board B's FTDI serial (`scripts/lib/ftdi_ports.py`), on Linux and macOS
alike; with Board B absent the request bytes are printed instead.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402


def interleave(a, b):
    assert len(a) == len(b), "a and b must have equal length"
    xs = []
    for x, y in zip(a, b):
        xs += [x, y]
    return xs


def frame(a, b):
    """Host → board: length (interleaved element count) then the signed-int8 interleaved operands."""
    xs = interleave(a, b)
    assert len(xs) <= 255, "interleaved length must fit one byte (≤127 pairs)"
    return bytes([len(xs)]) + bytes((v & 0xFF) for v in xs)


def decode(rec):
    """board → host: {0xA5, result[63:0] MSB-first, cycles[15:0]} = 11 bytes → (result, cycles)."""
    assert len(rec) == 11 and rec[0] == 0xA5, f"bad record {rec!r}"
    return int.from_bytes(rec[1:9], "big", signed=True), int.from_bytes(rec[9:11], "big", signed=False)


def read_record(ser):
    """One reply record off the board, or exit naming the image the request needs."""
    rec = ser.read(11)
    if len(rec) != 11 or rec[0] != 0xA5:
        sys.exit(f"no reply record from {ser.port} (got {rec.hex() or 'nothing'}) — is nexys_matmul_soc.bit loaded? "
                 f"python3 scripts/lib/ftdi_ports.py flash B benchmarks/silicon/nexys_matmul_soc.bit")
    return decode(rec)


def oracle(a, b):
    return sum(x * y for x, y in zip(a, b))


def selftest():
    for a, b in ([[3, -4, 5], [6, 7, -8]], [[1, 2, 3, 4], [10, -128, 127, -1]], [[7], [9]], [[], []]):
        exp = oracle(a, b)
        rec = bytes([0xA5]) + (exp & ((1 << 64) - 1)).to_bytes(8, "big") + (2 * len(a)).to_bytes(2, "big")
        got, cyc = decode(rec)
        assert got == exp and cyc == 2 * len(a), f"selftest mismatch {a}·{b}: {got} cyc {cyc} vs {exp}"
        assert frame(a, b)[0] == 2 * len(a)
    print("matmul-host selftest OK — interleave framing + record decode + a·b oracle consistent")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", help="comma-separated int8 vector a, e.g. 3,-4,5")
    ap.add_argument("--b", help="comma-separated int8 vector b, e.g. 6,7,-8")
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--demo", action="store_true", help="fire several dots back-to-back on ONE connection (no re-flash)")
    # merge `--a -5,10` → `--a=-5,10` so a leading-minus vector isn't mistaken for a flag.
    argv = []
    for tok in sys.argv[1:]:
        if argv and argv[-1] in ("--a", "--b") and tok and tok[0] == "-":
            argv[-1] = f"{argv[-1]}={tok}"
        else:
            argv.append(tok)
    args = ap.parse_args(argv)
    if args.selftest:
        selftest()
        return
    args.port = ftdi_ports.port_arg(args.port) if args.port else ftdi_ports.byte_port(ftdi_ports.BOARD_B)
    if args.demo:
        cases = [
            ([3, -4, 5], [6, 7, -8]),
            ([10, -7, 25, -3], [100, -128, 127, -1]),
            ([127, 127], [127, 127]),
            ([1, 2, 3, 4, 5, 6, 7, 8], [8, 7, 6, 5, 4, 3, 2, 1]),
        ]
        if not args.port:
            print(f"--demo needs Board B (FTDI {ftdi_ports.BOARD_B}) attached, or --port")
            return
        import serial
        allok = True
        with serial.Serial(args.port, args.baud, timeout=5) as s:
            print(f"flashed once — firing {len(cases)} dot requests back-to-back on {args.port}:")
            for a, b in cases:
                s.reset_input_buffer()
                s.write(frame(a, b))
                result, cycles = read_record(s)
                exp = oracle(a, b)
                ok = result == exp
                allok = allok and ok
                print(f"  a·b({len(a)}) = {result:>7} in {cycles:>2} cycles   (oracle {exp:>7})  {'OK ≡' if ok else 'MISMATCH'}")
        sys.exit(0 if allok else 1)
    a = [int(x) for x in args.a.split(",")] if args.a else [3, -4, 5]
    b = [int(x) for x in args.b.split(",")] if args.b else [6, 7, -8]
    if not args.port:
        print("Board B is not attached; showing the request bytes + expected result (a·b):")
        print(f"  send: {frame(a, b).hex()}")
        print(f"  expect result = {oracle(a, b)}, cycles = {2 * len(a)}")
        return
    import serial  # pyserial, only for a real board run
    with serial.Serial(args.port, args.baud, timeout=5) as s:
        s.reset_input_buffer()
        s.write(frame(a, b))
        result, cycles = read_record(s)
        exp = oracle(a, b)
        ok = "OK ≡" if result == exp else "MISMATCH"
        print(f"board: a·b = {result} in {cycles} cycles   (oracle {exp})  {ok}")
        sys.exit(0 if result == exp else 1)


if __name__ == "__main__":
    main()
