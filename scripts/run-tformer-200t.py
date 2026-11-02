#!/usr/bin/env python3
"""run-tformer-200t — stream the whole streamed-transformer's config+weights to the fabric engine on Board A
(Nexys Video / XC7A200T) over the DPTI byte channel (FT2232 channel A, plain pyserial + the
on-FPGA ft245_uart_bridge), wait for the engine to interpret the WHOLE autoregressive generation on-chip, and
decode the returned packed result to the generated tokens ("mat").

The engine returns a frame: 0xA5 sync, result[63:0] MSB-first (signed), cycles[31:0] MSB-first, 0x5A.
result = 80113 = base-100 pack of tokens [8,1,13] = "mat".

Prereq: Board A flashed with nexys_video_wpe_stream.bit (`scripts/logos-chip.py flash wpe200`).

  python3 scripts/run-tformer-200t.py [--wire benchmarks/silicon/programs/transformer.wire] [--port DEV] [--wait 180]

Without --port the byte channel is found by Board A's FTDI serial (`scripts/lib/ftdi_ports.py`), on Linux and
macOS alike. The wire is built by `scripts/gen_tiny_transformer_lg.py --fabric --stream` followed by the
`emit_transformer_wire` example; a board kit carries it at the default path.
"""
import argparse
import os
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(REPO, "scripts", "lib"))
import ftdi_ports  # noqa: E402


def unpack_tokens(packed):
    """base-100 pack: result = ((t0)*100 + t1)*100 + t2  ->  [t0, t1, t2]."""
    toks = []
    x = packed
    for _ in range(3):
        toks.append(x % 100)
        x //= 100
    return list(reversed(toks))


# tiny_transformer vocab index -> char (the model's byte alphabet). 8='m',1='a',13='t' => "mat".
def toks_to_text(toks, json_path="benchmarks/silicon/tiny_transformer.json"):
    try:
        import json
        v = json.load(open(json_path)).get("vocab_chars") or json.load(open(json_path)).get("itos")
        if v:
            return "".join(v[t] if 0 <= t < len(v) else "?" for t in toks)
    except Exception:
        pass
    # fall back to the known mapping for this model
    known = {8: "m", 1: "a", 13: "t"}
    return "".join(known.get(t, f"[{t}]") for t in toks)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--wire", default=os.path.join(REPO, "benchmarks", "silicon", "programs", "transformer.wire"))
    ap.add_argument("--port", help="a device, or a board letter (default: Board A, found by its FTDI serial)")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--wait", type=float, default=180.0, help="seconds to wait for the on-chip result")
    ap.add_argument("--expect", type=int, default=80113)
    args = ap.parse_args()

    if not os.path.exists(args.wire):
        sys.exit(f"{args.wire}: no transformer wire — build it with scripts/gen_tiny_transformer_lg.py --fabric "
                 f"--stream and the emit_transformer_wire example, or pass --wire")
    args.port = ftdi_ports.port_arg(args.port, "A")
    with open(args.wire, "rb") as f:
        tx = f.read()
    if not tx or tx[-1] != 0x5A:
        sys.exit(f"{args.wire}: not a valid .wire stream (must end with 0x5A GO)")
    print(f"streaming {len(tx)} bytes (config image + pre-loaded weights + GO) to {args.port} ...")

    import serial
    ser = serial.Serial(args.port, args.baud, timeout=1)
    ser.reset_input_buffer()
    ser.reset_output_buffer()
    ser.write(tx)
    ser.flush()
    print(f"streamed. waiting up to {args.wait:.0f}s for the engine to run the whole net on-chip ...")

    buf = bytearray()
    t0 = time.time()
    while len(buf) < 14 and (time.time() - t0) < args.wait:
        chunk = ser.read(14 - len(buf))
        if chunk:
            buf += chunk
    ser.close()

    if len(buf) < 14:
        sys.exit(f"\nno/short reply after {args.wait:.0f}s ({len(buf)} bytes: {bytes(buf).hex()})")
    if buf[0] != 0xA5 or buf[13] != 0x5A:
        sys.exit(f"\nbad frame: {bytes(buf).hex()}")
    result = int.from_bytes(buf[1:9], "big", signed=True)
    cycles = int.from_bytes(buf[9:13], "big")
    toks = unpack_tokens(result)
    text = toks_to_text(toks)
    print(f"\n  on-chip result = {result}   ({cycles} cycles @ 0.78 MHz = {cycles/781250:.1f}s)")
    print(f"  generated tokens = {toks}  ->  \"{text}\"")
    if result == args.expect:
        print(f"\n  ★ THE WHOLE LLM RAN ON THE 200T FABRIC — generated \"{text}\" (80113), byte-identical to the reference.")
    else:
        sys.exit(f"\n  MISMATCH: got {result}, expected {args.expect}")


if __name__ == "__main__":
    main()
