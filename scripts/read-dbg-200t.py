#!/usr/bin/env python3
"""read-dbg-200t — stream the whole streamed-transformer's config+weights to the DEBUG_READBACK bitstream on
Board A (Nexys Video / XC7A200T) over the DPTI byte channel (FT2232 channel A, found by Board A's FTDI serial; plain pyserial +
the on-FPGA ft245_uart_bridge), read back the on-chip DEBUG FRAME + result frame, and DIFF the debug frame
byte-for-byte AND value-for-value against the golden reference captured in simulation — localizing WHICH
config/heap BRAM the open synthesis flow mis-maps on silicon.

The engine (built with DEBUG_READBACK=1) returns, at the load->run boundary and BEFORE running the interpret:
  DEBUG FRAME  (322 bytes): 0xD8 sync, then 5 memories in order
                            {cell_kind, cell_op0, cell_op1, blk_term_kind, heap}, each 8 entries [0..8),
                            each entry a 64-bit BIG-ENDIAN signed word (8 bytes MSB-first), then 0x8D end.
Then it runs the whole autoregressive generation on-chip and sends the usual result frame:
  RESULT FRAME (14  bytes): 0xA5 sync, result[63:0] MSB-first (signed), cycles[31:0] MSB-first, 0x5A end.

The golden reference (/tmp/golden_dbg.bin, 322 bytes) is captured in sim by the test
`full_transformer_debug_readback_captures_golden_frame`. On known-good sim the result is 80113 = "mat"; on the
buggy silicon it comes back mismatched (e.g. 3). That's fine — the DEBUG FRAME is the point: it says WHICH memory
diverged and at which indices. This is a diagnostic, not a pass/fail gate: it always exits 0 after printing.

Prereq: Board A flashed with the DEBUG_READBACK=1 streamed-transformer bitstream.

  python3 scripts/read-dbg-200t.py [--wire /tmp/transformer.wire] [--golden /tmp/golden_dbg.bin]
                                   [--port DEV|A|B] [--baud 115200] [--wait 220]
"""
import argparse
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

# The DEBUG FRAME layout, matched to the engine's DEBUG_READBACK FSM.
DBG_SYNC = 0xD8
DBG_END = 0x8D
DBG_MEMS = ["cell_kind", "cell_op0", "cell_op1", "blk_term_kind", "heap"]
DBG_SLICE = 8                                # entries per memory (addresses 0..7)
DBG_NWORDS = len(DBG_MEMS) * DBG_SLICE       # 5 * 8 = 40 words
DBG_FRAME_LEN = 1 + DBG_NWORDS * 8 + 1       # 0xD8 + 40*8 + 0x8D = 322 bytes
RES_FRAME_LEN = 14                           # 0xA5 + 8 (result) + 4 (cycles) + 0x5A
TOTAL_LEN = DBG_FRAME_LEN + RES_FRAME_LEN    # 322 + 14 = 336


def decode_words(frame):
    """frame = the 322 raw debug bytes. Returns {mem_name: [8 signed int64]} (skips sync/end)."""
    if len(frame) != DBG_FRAME_LEN:
        raise ValueError(f"debug frame must be {DBG_FRAME_LEN} bytes, got {len(frame)}")
    off = 1  # skip the 0xD8 sync
    out = {}
    for mem in DBG_MEMS:
        vals = []
        for _ in range(DBG_SLICE):
            vals.append(int.from_bytes(frame[off:off + 8], "big", signed=True))
            off += 8
        out[mem] = vals
    return out


def unpack_tokens(packed):
    """base-100 pack: result = ((t0)*100 + t1)*100 + t2  ->  [t0, t1, t2]."""
    toks = []
    x = packed
    for _ in range(3):
        toks.append(x % 100)
        x //= 100
    return list(reversed(toks))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--wire", default="/tmp/transformer.wire")
    ap.add_argument("--golden", default="/tmp/golden_dbg.bin",
                    help="the 322-byte golden debug frame captured in sim")
    ap.add_argument("--port", help="a device, or a board letter (default: Board A, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--wait", type=float, default=220.0,
                    help="seconds to wait for the on-chip debug + result frames")
    args = ap.parse_args()
    args.port = ftdi_ports.port_arg(args.port, "A")

    # Load the golden debug frame (the sim reference every silicon word is diffed against).
    try:
        with open(args.golden, "rb") as f:
            golden = f.read()
    except OSError as e:
        sys.exit(f"cannot read golden frame {args.golden}: {e}")
    if len(golden) != DBG_FRAME_LEN:
        sys.exit(f"{args.golden}: golden frame must be {DBG_FRAME_LEN} bytes, got {len(golden)}")
    if golden[0] != DBG_SYNC or golden[-1] != DBG_END:
        sys.exit(f"{args.golden}: not a valid golden frame (sync/end = {golden[0]:02x}/{golden[-1]:02x})")
    golden_words = decode_words(golden)

    # Load + stream the wire (config image + pre-loaded weights + GO), exactly like run-tformer-200t.py.
    try:
        with open(args.wire, "rb") as f:
            tx = f.read()
    except OSError as e:
        sys.exit(f"cannot read wire {args.wire}: {e}")
    if not tx or tx[-1] != 0x5A:
        sys.exit(f"{args.wire}: not a valid .wire stream (must end with 0x5A GO)")
    print(f"streaming {len(tx)} bytes (config image + pre-loaded weights + GO) to {args.port} ...")

    import serial
    ser = serial.Serial(args.port, args.baud, timeout=1)
    ser.reset_input_buffer()
    ser.reset_output_buffer()
    ser.write(tx)
    ser.flush()
    print(f"streamed. waiting up to {args.wait:.0f}s for the DEBUG FRAME ({DBG_FRAME_LEN}B) + "
          f"result frame ({RES_FRAME_LEN}B) = {TOTAL_LEN}B ...")

    # Tolerant read loop: collect the full 336 bytes (debug + result), or bail after --wait.
    buf = bytearray()
    t0 = time.time()
    while len(buf) < TOTAL_LEN and (time.time() - t0) < args.wait:
        chunk = ser.read(TOTAL_LEN - len(buf))
        if chunk:
            buf += chunk
    ser.close()

    if len(buf) < TOTAL_LEN:
        sys.exit(f"\nno/short reply after {args.wait:.0f}s "
                 f"({len(buf)}/{TOTAL_LEN} bytes: {bytes(buf).hex()})")

    dbg = bytes(buf[:DBG_FRAME_LEN])
    res = bytes(buf[DBG_FRAME_LEN:TOTAL_LEN])

    # ── Validate the debug-frame envelope. ──
    print()
    print("=" * 78)
    print("  ON-CHIP DEBUG FRAME")
    print("=" * 78)
    ok_sync = dbg[0] == DBG_SYNC
    ok_end = dbg[-1] == DBG_END
    print(f"  sync = {dbg[0]:02x} (exp d8)  {'OK' if ok_sync else '★ WRONG'}"
          f"    end = {dbg[-1]:02x} (exp 8d)  {'OK' if ok_end else '★ WRONG'}")

    # ── Byte-for-byte diff of the whole 322-byte frame. ──
    byte_mismatches = [i for i in range(DBG_FRAME_LEN) if dbg[i] != golden[i]]
    if byte_mismatches:
        print(f"  raw bytes: {DBG_FRAME_LEN - len(byte_mismatches)}/{DBG_FRAME_LEN} match golden"
              f"  ★ {len(byte_mismatches)} byte(s) differ (first at offset {byte_mismatches[0]})")
    else:
        print(f"  raw bytes: {DBG_FRAME_LEN}/{DBG_FRAME_LEN} match golden  (all match)")

    # ── Value-for-value per-memory table. ──
    silicon_words = decode_words(dbg)
    diverged = {}  # mem -> [indices that differ]
    for mem in DBG_MEMS:
        g = golden_words[mem]
        s = silicon_words[mem]
        bad = [i for i in range(DBG_SLICE) if g[i] != s[i]]
        if bad:
            diverged[mem] = bad
        print()
        print(f"  [{mem}]" + ("   ★ DIVERGES" if bad else "   ok"))
        print(f"    {'idx':>3}  {'golden (sim)':>22}  {'silicon (chip)':>22}")
        for i in range(DBG_SLICE):
            mark = " ★" if g[i] != s[i] else "  "
            print(f"    {i:>3}  {g[i]:>22}  {s[i]:>22}{mark}")

    # ── Localization summary. ──
    print()
    print("=" * 78)
    print("  LOCALIZATION")
    print("=" * 78)
    if not ok_sync or not ok_end:
        print("  ★ frame envelope wrong (sync/end) — the readback FSM itself did not run cleanly on chip.")
    if not diverged:
        print("  All 5 memories echoed the golden image byte-for-byte — the config/heap BRAMs are")
        print("  loaded correctly; the wrong on-chip answer is NOT a config-load mis-map.")
    else:
        print("  The following memory/memories diverge from the golden sim image (the synthesis bug):")
        for mem in DBG_MEMS:
            if mem in diverged:
                idxs = diverged[mem]
                print(f"    ★ {mem}: {len(idxs)} of {DBG_SLICE} entries differ at indices {idxs}")
        print()
        print("  -> This is the localization: the mis-mapped BRAM is the one(s) marked ★ above.")

    # ── Result frame (expect mismatched on buggy silicon; the debug frame is the diagnostic). ──
    print()
    print("=" * 78)
    print("  RESULT FRAME")
    print("=" * 78)
    if res[0] != 0xA5 or res[13] != 0x5A:
        print(f"  ★ bad result frame envelope: {res.hex()} (sync={res[0]:02x} end={res[13]:02x})")
    else:
        result = int.from_bytes(res[1:9], "big", signed=True)
        cycles = int.from_bytes(res[9:13], "big")
        toks = unpack_tokens(result)
        print(f"  result = {result}   cycles = {cycles}")
        print(f"  tokens = {toks}   (golden 'mat' = 80113 -> [8, 1, 13])")
        if result == 80113:
            print("  result MATCHES the golden 80113 ('mat').")
        else:
            print("  result MISMATCH (golden 80113) — expected on the buggy silicon; "
                  "the DEBUG FRAME above localizes it.")

    print()
    # Always exit 0 — this is a diagnostic, not a pass/fail gate.
    sys.exit(0)


if __name__ == "__main__":
    main()
