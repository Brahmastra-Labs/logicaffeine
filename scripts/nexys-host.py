#!/usr/bin/env python3
"""nexys-host — stream a LOGOS bytecode program to the board-runnable Nexys A7 SoC over the
USB-UART, run it, and read back the Show results. The host side of nexys_soc.v.

    python3 scripts/nexys-host.py PROGRAM.hex [--port DEV] [--baud 115200] [--expect 7,42]

PROGRAM.hex is one 32-bit instruction word per line, 8 hex digits (the same format soft_core's
$readmemh boot ROM uses). The wire framing exactly matches tb_nexys_soc.v so the simulation and the
board agree:
    host -> FPGA : <N=nwords>, then N*4 program bytes (MSB-first per word)
    FPGA -> host : 0xA5, <M=nshow>, then for EACH show IN ORDER a tagged record —
                     <tag:1> (0=signed int, 1=float, 2=string, 3=UNSIGNED Word32)
                     then 4 value bytes (int/Word32, MSB-first) OR 8 value bytes (float, raw IEEE-754 f64
                     bits, MSB-first) OR — for a string — 4 length bytes (char count, MSB-first) then that
                     many 4-byte codepoints (each MSB-first)
                   then a 4-byte run-cycle count (MSB-first), then 0x5A

Int Shows come back as SIGNED integers; Word32 Shows (ShowWord32, opcode 221) come back as the same 4 value
bytes but tagged UNSIGNED (a SHA-256 hash word > 2³¹ prints large, not negative); float Shows (ShowF) come
back as the raw f64 bits, formatted for display by fmt_f64(); text Shows (ShowText) come back as a length +
Unicode codepoints, reassembled into the string. The tag+width let the host tell them apart, and shows print
in the exact order the core ran them (int / Word32 / float / string interleave correctly).

The run-cycle count is how many clock cycles the core ran before halting — the authoritative on-chip
timing number (cycles / clock = wall time). The host also reports its own round-trip wall-clock.

Without --port the UART is found by Board B's FTDI serial (`scripts/lib/ftdi_ports.py`), on Linux and macOS
alike. With no board attached (or --dry-run) it prints the exact byte stream it WOULD send, so the protocol is
inspectable without hardware.
"""
import argparse
import os
import sys
import time


def load_program(path):
    """Read a .hex file (one 32-bit word per line) into a list of ints."""
    words = []
    with open(path) as f:
        for line in f:
            line = line.split("//")[0].strip()
            if not line:
                continue
            words.append(int(line, 16) & 0xFFFFFFFF)
    if not words:
        sys.exit(f"{path}: no instruction words")
    if len(words) > 4095:
        sys.exit(f"{path}: {len(words)} words exceeds the 4095-word imem")
    return words


def load_blob(path):
    """Read the DATA-segment (weight) blob: a JSON int list `[len, w0, w1, …]` (gen_tiny_transformer_lg.py
    --blob) OR a .hex file (one 32-bit word per line). Returns a list of ints."""
    txt = open(path).read().strip()
    if txt.startswith("["):
        return [int(x) & 0xFFFFFFFF for x in txt.strip("[]").split(",") if x.strip()]
    return [int(line.split("//")[0].strip(), 16) & 0xFFFFFFFF
            for line in txt.splitlines() if line.split("//")[0].strip()]


def frame_data(blob):
    """The DATA phase host->FPGA stream (nexys_soc_tformer): a 2-byte big-endian word count then each
    32-bit word MSB-first. The SoC preloads these into the heap (dmem[0..]) as the `loaded()` Seq at ptr 0."""
    n = len(blob)
    out = bytearray([(n >> 8) & 0xFF, n & 0xFF])
    for w in blob:
        out += bytes([(w >> 24) & 0xFF, (w >> 16) & 0xFF, (w >> 8) & 0xFF, w & 0xFF])
    return bytes(out)


# The first and last bytes of the core's reply frame (nexys_soc.v's L_SEND).
SYNC = 0xA5
END = 0x5A


def frame_to_send(words, words16=False):
    """The exact host->FPGA byte stream: length then MSB-first program words. `words16` uses a plain 2-byte
    big-endian length (nexys_soc_hft). The default (nexys_soc_full) is BACKWARD-COMPATIBLE: a 1-byte length
    for <=254 words (the legacy protocol), and a `0xFF` ESCAPE + 2-byte big-endian length for wider programs
    (up to the 1024-word imem) — matching nexys_soc_full.v's L_LEN/L_LEN_HI/L_LEN_LO loader (P0d)."""
    n = len(words)
    if words16:
        out = bytearray([(n >> 8) & 0xFF, n & 0xFF])                  # HFT SoC: plain 2-byte length
    elif n > 254:
        out = bytearray([0xFF, (n >> 8) & 0xFF, n & 0xFF])            # full SoC: 0xFF escape + 2-byte length
    else:
        out = bytearray([n])                                          # full SoC: legacy 1-byte length
    for w in words:
        out += bytes([(w >> 24) & 0xFF, (w >> 16) & 0xFF, (w >> 8) & 0xFF, w & 0xFF])
    return bytes(out)


def parse_reply(data):
    """Decode a 0xA5 / count / tagged-records / cycles / 0x5A reply.

    Each of the M records is <tag:1> then either 4 MSB-first bytes (tag 0 = signed int, tag 3 = UNSIGNED
    Word32), 8 MSB-first bytes (tag 1 = float, raw IEEE-754 f64 bits), or — for tag 2 = string — 4 length
    bytes (char count) then that many 4-byte codepoints. Returns (shows, run_cycles) where `shows` is a list
    of (tag, raw) tuples in program order — raw is the 32-bit value (tag 0 signed / tag 3 unsigned), the
    64-bit f64 bit-pattern (tag 1), or the decoded `str` (tag 2). Records are variable-width, so the frame
    length depends on the mix of shows.
    """
    if len(data) < 2 or data[0] != SYNC:
        raise ValueError(f"bad sync byte: {data[:1].hex()}")
    m = data[1]
    shows = []
    p = 2
    for i in range(m):
        if p >= len(data):
            raise ValueError(f"short reply: truncated before record {i} tag")
        tag = data[p]
        p += 1
        if tag not in (0, 1, 2, 3):
            raise ValueError(f"bad show tag {tag} in record {i}")
        if tag == 2:
            # STRING: a 4-byte char count, then that many 4-byte Unicode codepoints (all MSB-first).
            if p + 4 > len(data):
                raise ValueError(f"short reply: truncated in record {i} string length")
            slen = (data[p] << 24) | (data[p + 1] << 16) | (data[p + 2] << 8) | data[p + 3]
            p += 4
            if p + 4 * slen > len(data):
                raise ValueError(f"short reply: truncated in record {i} string body ({slen} chars)")
            chars = []
            for _ in range(slen):
                cp = (data[p] << 24) | (data[p + 1] << 16) | (data[p + 2] << 8) | data[p + 3]
                p += 4
                chars.append(chr(cp))
            shows.append((2, "".join(chars)))
            continue
        width = 8 if tag == 1 else 4
        if p + width > len(data):
            raise ValueError(f"short reply: truncated in record {i} value ({width} bytes)")
        raw = 0
        for k in range(width):
            raw = (raw << 8) | data[p + k]
        p += width
        shows.append((tag, raw))
    if p + 4 + 1 > len(data):
        raise ValueError(f"short reply: got {len(data)} bytes, missing cycle bytes / end")
    c = data[p:p + 4]
    cycles = (c[0] << 24) | (c[1] << 16) | (c[2] << 8) | c[3]
    p += 4
    if data[p] != END:
        raise ValueError(f"bad end byte: {data[p:p+1].hex()}")
    return shows, cycles


def show_str(tag, raw):
    """Render one decoded show record for display: a signed int show (tag 0) as a SIGNED decimal, an unsigned
    Word32 show (tag 3) as its UNSIGNED decimal (a SHA-256 hash word > 2³¹ prints large, not negative), a
    float show (tag 1) via fmt_f64(), a string show (tag 2) as the reassembled text (raw is already a `str`).
    The raw value arrives as an unsigned 32-bit big-endian assembly, so tag 0 sign-extends it to match the
    VM's `i32` Show; tag 3 keeps it unsigned to match the VM's `Word32` / unsigned-`intOfWord32` Show."""
    if tag == 2:
        return raw
    if tag == 1:
        return fmt_f64(raw)
    if tag == 0:
        return str(raw - (1 << 32) if raw >= (1 << 31) else raw)  # signed i32
    return str(raw)  # tag 3: unsigned Word32


def _w32(v):
    return bytes([(v >> 24) & 0xFF, (v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF])


def fmt_f64(bits):
    """Format the 64-bit IEEE-754 pattern of a LOGOS float Show (ShowF) EXACTLY as the language does on
    every engine — the Rust `logicaffeine_data::fmt::fmt_f64` contract: shortest round-trip, NEVER
    scientific notation, integral floats bare (2.0 -> "2"), signed zero kept, NaN/inf spelled out. The
    chip sends the raw bits (soft_core.v's `outf`); the host turns them into the display string, so the
    on-chip Show text is byte-identical to the VM. Mirrors Rust's `{}` Display for f64.
    """
    import struct
    f = struct.unpack(">d", bits.to_bytes(8, "big"))[0]
    if f != f:  # NaN
        return "NaN"
    if f == float("inf"):
        return "inf"
    if f == float("-inf"):
        return "-inf"
    # Signed zero (Rust prints -0 / 0).
    if f == 0.0:
        return "-0" if struct.pack(">d", f)[0] & 0x80 else "0"
    # Python's repr is shortest-round-trip (same algorithm class as Rust). Normalize it to Rust's form:
    # strip a trailing ".0" (integral floats are bare), and expand any exponent into full decimal (Rust's
    # Display never uses scientific notation).
    s = repr(f)
    if "e" not in s and "E" not in s:
        if s.endswith(".0"):
            s = s[:-2]
        return s
    # Expand Python's SHORTEST scientific repr to a plain decimal (keeping the shortest digits, matching
    # Rust's shortest-round-trip then no-exponent rule). Parse `repr` back through Decimal so we expand the
    # exact shortest mantissa, not the full binary value.
    from decimal import Decimal
    s = format(Decimal(repr(f)), "f")
    if "." in s:
        s = s.rstrip("0").rstrip(".")
    return s


def _w64(v):
    return v.to_bytes(8, "big")


def selftest():
    """Golden vectors: the host framing must match nexys_soc_full.v L_SEND / tb_nexys_soc*.v byte-for-byte."""
    # host -> FPGA: length then MSB-first words
    assert frame_to_send([0x05000007, 0x2B000000]) == bytes(
        [2, 0x05, 0x00, 0x00, 0x07, 0x2B, 0x00, 0x00, 0x00]
    )
    # FPGA -> host: sync, count, then per-show <tag>+value bytes, 4 cycle bytes, end.
    # Two INT shows (tag 0, 4 bytes each).
    shows, cyc = parse_reply(
        bytes([0xA5, 2]) + bytes([0]) + _w32(7) + bytes([0]) + _w32(42) + _w32(6) + bytes([0x5A])
    )
    assert (shows, cyc) == ([(0, 7), (0, 42)], 6), (shows, cyc)
    assert [show_str(t, r) for t, r in shows] == ["7", "42"], shows
    # A zero-Show program still carries a cycle count.
    shows, cyc = parse_reply(bytes([0xA5, 0]) + _w32(123) + bytes([0x5A]))
    assert (shows, cyc) == ([], 123), (shows, cyc)
    # One FLOAT show (tag 1, 8 bytes = raw f64 bits) — pi_leibniz's known pattern.
    PI_BITS = 0x40091FEF0A9265EB
    shows, cyc = parse_reply(bytes([0xA5, 1]) + bytes([1]) + _w64(PI_BITS) + _w32(99) + bytes([0x5A]))
    assert shows == [(1, PI_BITS)], shows
    assert show_str(*shows[0]) == fmt_f64(PI_BITS), (show_str(*shows[0]), fmt_f64(PI_BITS))
    assert cyc == 99, cyc
    # INTERLEAVED: int Show THEN float Show, in order (the whole point of the tagged framing).
    shows, cyc = parse_reply(
        bytes([0xA5, 2]) + bytes([0]) + _w32(3) + bytes([1]) + _w64(PI_BITS) + _w32(7) + bytes([0x5A])
    )
    assert shows == [(0, 3), (1, PI_BITS)], shows
    assert [show_str(t, r) for t, r in shows] == ["3", fmt_f64(PI_BITS)], shows
    assert cyc == 7, cyc
    # A short frame (missing the cycle bytes) must be rejected, not silently mis-parsed.
    try:
        parse_reply(bytes([0xA5, 1]) + bytes([0]) + _w32(9) + bytes([0x5A]))
    except ValueError:
        pass
    else:
        raise AssertionError("parse_reply accepted a frame missing its cycle bytes")
    # A bad tag must be rejected (4 is undefined; 0/1/2/3 = int/float/string/Word32).
    try:
        parse_reply(bytes([0xA5, 1]) + bytes([4]) + _w32(9) + _w32(0) + bytes([0x5A]))
    except ValueError:
        pass
    else:
        raise AssertionError("parse_reply accepted a record with a bad tag")
    # A signed int Show (tag 0) with the high bit set renders SIGNED (i32), matching the VM's `Int` Show.
    shows, cyc = parse_reply(bytes([0xA5, 1]) + bytes([0]) + _w32(0xE3B0C442) + _w32(0) + bytes([0x5A]))
    assert shows == [(0, 0xE3B0C442)], shows
    assert show_str(*shows[0]) == "-474954686", show_str(*shows[0])
    # An UNSIGNED Word32 Show (tag 3) with the SAME bits renders UNSIGNED — the SHA-256 hash-word case: the
    # value exceeds 2³¹, so it prints large (3820012610), not negative. Same 4 payload bytes as tag 0; only
    # the tag (hence the host-side sign) differs. This is the keystone of the ShowWord32 round-trip.
    shows, cyc = parse_reply(bytes([0xA5, 1]) + bytes([3]) + _w32(0xE3B0C442) + _w32(0) + bytes([0x5A]))
    assert shows == [(3, 0xE3B0C442)], shows
    assert show_str(*shows[0]) == "3820012610", show_str(*shows[0])
    # Word32 interleaved with a signed int, in order (a mixed program): int -1 then Word32 4294967295 (both
    # 0xFFFFFFFF bits) — the tag disambiguates the identical bits into "-1" vs "4294967295".
    shows, cyc = parse_reply(
        bytes([0xA5, 2]) + bytes([0]) + _w32(0xFFFFFFFF) + bytes([3]) + _w32(0xFFFFFFFF) + _w32(5) + bytes([0x5A])
    )
    assert [show_str(t, r) for t, r in shows] == ["-1", "4294967295"], shows
    # One STRING show (tag 2): a 4-byte char count then that many 4-byte codepoints. "hi" = [104, 105].
    shows, cyc = parse_reply(
        bytes([0xA5, 1]) + bytes([2]) + _w32(2) + _w32(ord("h")) + _w32(ord("i")) + _w32(11) + bytes([0x5A])
    )
    assert shows == [(2, "hi")], shows
    assert [show_str(t, r) for t, r in shows] == ["hi"], shows
    assert cyc == 11, cyc
    # A string interleaved with an int, in order (the binary_trees mix): int 64, then " trees" fragment.
    shows, cyc = parse_reply(
        bytes([0xA5, 2]) + bytes([0]) + _w32(64)
        + bytes([2]) + _w32(3) + _w32(ord("a")) + _w32(ord(" ")) + _w32(ord("b"))
        + _w32(5) + bytes([0x5A])
    )
    assert shows == [(0, 64), (2, "a b")], shows
    assert [show_str(t, r) for t, r in shows] == ["64", "a b"], shows
    # An empty string (len 0) — no codepoints follow.
    shows, cyc = parse_reply(bytes([0xA5, 1]) + bytes([2]) + _w32(0) + _w32(3) + bytes([0x5A]))
    assert shows == [(2, "")], shows
    print("nexys-host selftest OK: tagged framing matches nexys_soc_full.v "
          "(signed-int 4B + float 8B + string [len + codepoints] + UNSIGNED-Word32 4B records, "
          "interleaved in order, + 4-byte run-cycle count)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("program", nargs="?")
    ap.add_argument("--port", help="a device, or a board letter (default: Board B, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--expect", help="comma-separated expected Show values")
    ap.add_argument("--clock-mhz", type=float, default=100.0,
                    help="core clock in MHz, to convert run-cycles -> wall time (default 100)")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--data", help="DATA-segment (weight) blob for nexys_soc_tformer (.json int list or .hex)")
    ap.add_argument("--words16", action="store_true",
                    help="2-byte big-endian program length (nexys_soc_hft, up to 1024 words) instead of 1")
    ap.add_argument("--selftest", action="store_true",
                    help="check the host framing against the RTL protocol (no hardware) and exit")
    args = ap.parse_args()

    if args.selftest:
        selftest()
        return 0
    if args.program is None:
        ap.error("program is required (or pass --selftest)")

    words = load_program(args.program)
    tx = frame_to_send(words, words16=args.words16)
    print(f"program: {len(words)} words -> {len(tx)} bytes on the wire")
    if args.data:
        blob = load_blob(args.data)
        tx += frame_data(blob)
        print(f"data segment: {len(blob)} words -> {len(tx)} bytes total on the wire (weights via loaded())")

    try:
        import serial  # pyserial, optional
    except ImportError:
        serial = None

    if serial is not None and not args.dry_run:
        sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
        import ftdi_ports

        args.port = ftdi_ports.port_arg(args.port) if args.port else ftdi_ports.byte_port(ftdi_ports.BOARD_B)
    if args.dry_run or args.port is None or serial is None:
        why = "dry-run" if args.dry_run else ("no board attached" if args.port is None else "no pyserial")
        print(f"[{why}] would send: {tx.hex()}")
        print("[offline] attach Board B (or pass --port) to run the round-trip")
        return 0

    with serial.Serial(args.port, args.baud, timeout=5) as ser:
        ser.reset_input_buffer()
        t0 = time.perf_counter()
        ser.write(tx)
        head = ser.read(2)
        if len(head) < 2 or head[0] != SYNC:
            sys.exit(f"no/!sync reply: {head.hex()}")
        # Records are variable-width (int 1+4, float 1+8, string 1 + 4 + 4*len), so read each record's tag
        # first and pull the right number of value bytes; then the 4 cycle bytes + end byte.
        m = head[1]
        frame = bytearray(head)
        for _ in range(m):
            tag = ser.read(1)
            if len(tag) < 1:
                sys.exit("short reply: no record tag")
            frame += tag
            if tag[0] == 2:                # STRING: a 4-byte char count, then that many 4-byte codepoints
                lenb = ser.read(4)
                if len(lenb) < 4:
                    sys.exit("short reply: truncated string length")
                frame += lenb
                slen = (lenb[0] << 24) | (lenb[1] << 16) | (lenb[2] << 8) | lenb[3]
                frame += ser.read(4 * slen)
            else:
                frame += ser.read(8 if tag[0] == 1 else 4)
        frame += ser.read(4 + 1)          # cycle bytes + end
        t1 = time.perf_counter()
        shows, cycles = parse_reply(bytes(frame))

    disp = [show_str(t, r) for t, r in shows]
    print("Show values:", disp)
    print(f"on-chip: {cycles} cycles = {cycles / args.clock_mhz:.3f} us @ {args.clock_mhz:.0f} MHz")
    print(f"host round-trip: {(t1 - t0) * 1e3:.3f} ms (includes UART transfer at {args.baud} baud)")
    if args.expect is not None:
        exp = [x for x in args.expect.split(",") if x != ""]
        if disp != exp:
            sys.exit(f"MISMATCH: got {disp}, expected {exp}")
        print("OK: matches --expect")
    return 0


if __name__ == "__main__":
    sys.exit(main())
