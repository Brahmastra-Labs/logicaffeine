#!/usr/bin/env python3
"""decode-host — run the tiny transformer's decode step on a Nexys board image and record what the chip did.

The image (benchmarks/silicon/decode_images.json, built by the fabric printer) serves the protocol of
`spatial::lower::fabric_board`: `I` identifies the image and its memory read latency; `S token position` runs one
step and replies `s`, the next token and the step's cycles; `G count` runs that many steps on chip and replies each
next token, then `e` and the cycles of the whole loop. Tokens and positions are the step's 1-based values.

This host runs the model's golden trace: every prompt token with `S` at its position, then `G` for the rest. It holds
every reply to the image record: each prompt step's next token to the evaluator's for that position, each step to the
twin's cycles, every generated token to the trace, and the loop to its own accounting of the steps it ran. It writes
what the chip measured: its tokens, its cycles, the decode time at the image's core clock, when the run began (UTC; a
replay ran on no board and states no time), the host's wall time, and the board's power when `--power-w` gives a
reading.

    python3 scripts/decode-host.py A|B [--flash] [--power-w W] [--out FILE]
    python3 scripts/decode-host.py B --dry-run          # the command bytes, nothing opened
    python3 scripts/decode-host.py B --replay FILE      # replies read from FILE (hex, one reply a line)

Each board's byte channel is the FT2232 channel its JTAG does not hold, found by scripts/lib/ftdi_ports.py and
opened through pyserial: Board B's is a UART bridge at 115200 8N1; Board A's is an asynchronous 245 FIFO (the
Nexys Video's DPTI channel, channel A, JTAG on B), which the image's `ft245_uart_bridge` speaks and the OS
presents as a serial port whose baud the FIFO ignores.
"""
import argparse
import hashlib
import json
import os
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts", "lib"))
import ftdi_ports  # noqa: E402
from board_line import Line  # noqa: E402

PART = {"A": "xc7a200tsbg484-1", "B": "xc7a100tcsg324-1"}


def u16(v):
    return int(v).to_bytes(2, "little")


def identify():
    return b"I"


def step(token, position):
    """`S`: the 0-based token id at the 0-based position, sent as the step's 1-based values."""
    return b"S" + u16(token + 1) + u16(position + 1)


def generate(count):
    return b"G" + u16(count)


def script(prompt, generated):
    """The golden trace as (command, reply length): identify, every prompt token stepped, then the rest generated."""
    out = [(identify(), 6)]
    for pos, tok in enumerate(prompt):
        out.append((step(tok, pos), 7))
    n = len(generated) - 1
    out.append((generate(n), 2 * n + 5))
    return out


def le(b):
    return int.from_bytes(bytes(b), "little")


def check(trace, image, replies):
    """Hold the replies to the trace and the image record; the measured record, or the first disagreement."""
    prompt, generated = trace["prompt_tokens"], trace["generated_tokens"]
    ident = replies[0]
    if bytes(ident[:5]) != b"iLOGS":
        return None, f"identify: {bytes(ident).hex()} is not an image (want 69 4c 4f 47 53 and the read latency)"
    if ident[5] != image["read_latency"]:
        return None, f"identify: read latency {ident[5]}, the image record says {image['read_latency']}"
    steps, stepped = [], []
    for pos, (r, want) in enumerate(zip(replies[1:1 + len(prompt)], image["prompt_next_tokens"])):
        if r[0] != ord("s"):
            return None, f"the step at position {pos}: reply {bytes(r).hex()} is not a step's"
        stepped.append(le(r[1:3]) - 1)
        if stepped[-1] != want:
            return None, f"the step at position {pos}: next token {stepped[-1]}, the evaluator's {want}"
        steps.append(le(r[3:7]))
        if le(r[3:7]) != image["cycles_per_step"]:
            return None, f"the step at position {pos}: {le(r[3:7])} cycles, the image's twin {image['cycles_per_step']}"
    first = le(replies[len(prompt)][1:3]) - 1
    if first != generated[0]:
        return None, f"the prompt's last step predicts token {first}, the trace {generated[0]}"
    g = replies[1 + len(prompt)]
    n = len(generated) - 1
    tokens = [le(g[2 * k:2 * k + 2]) - 1 for k in range(n)]
    if tokens != generated[1:]:
        return None, f"generated {tokens}, the trace {generated[1:]}"
    if g[2 * n] != ord("e"):
        return None, f"the loop's reply ends {g[2 * n]:#x}, not e"
    loop = le(g[2 * n + 1:2 * n + 5])
    if loop != n * image["loop_cycles_per_step"]:
        return None, f"the loop took {loop} cycles for {n} steps, its own accounting {n * image['loop_cycles_per_step']}"
    mhz = image["core_mhz"]
    return {
        "tokens": [generated[0]] + tokens,
        "prompt_next_tokens": stepped,
        "step_cycles": steps,
        "generate_steps": n,
        "generate_cycles": loop,
        "core_mhz": mhz,
        "us_per_token": round(loop / n / mhz, 3),
        "tokens_per_s": round(n * mhz * 1e6 / loop, 1),
    }, None


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("board", choices=["A", "B", "a", "b"])
    ap.add_argument("--model", default=os.path.join(ROOT, "benchmarks/silicon/tiny_transformer.json"))
    ap.add_argument("--images", default=os.path.join(ROOT, "benchmarks/silicon/decode_images.json"))
    ap.add_argument("--flash", action="store_true", help="configure the board with the image's bitstream first")
    ap.add_argument("--power-w", type=float, help="the board's power during the run, from a meter")
    ap.add_argument("--timeout", type=float, default=30.0, help="seconds to wait for each reply")
    ap.add_argument("--out", help="where to write the measured record (stdout when absent)")
    ap.add_argument("--dry-run", action="store_true", help="print each command's bytes, open nothing")
    ap.add_argument("--replay", help="read the replies from FILE, one reply a line in hex, instead of a board")
    args = ap.parse_args()
    board = args.board.upper()

    trace = json.load(open(args.model))["golden_trace"]
    cmds = script(trace["prompt_tokens"], trace["generated_tokens"])
    if args.dry_run:
        for cmd, n in cmds:
            print(f"{cmd.hex()} {n}")
        return 0

    images = json.load(open(args.images))["images"]
    image = next((i for i in images if i["part"] == PART[board]), None)
    if image is None:
        sys.exit(f"{args.images}: no image for board {board} ({PART[board]})")
    for field in ("read_latency", "cycles_per_step", "loop_cycles_per_step", "prompt_next_tokens", "core_mhz"):
        if field not in image:
            sys.exit(f"{args.images}: the image for board {board} does not say its `{field}`")
    if len(image["prompt_next_tokens"]) != len(trace["prompt_tokens"]):
        sys.exit(f"{args.images}: {len(image['prompt_next_tokens'])} prompt steps' tokens for a {len(trace['prompt_tokens'])}-token prompt")
    bit = os.path.join(os.path.dirname(args.images), image["bitstream"])

    if args.replay:
        replies = [bytes.fromhex(line.strip()) for line in open(args.replay) if line.strip()]
        if len(replies) != len(cmds):
            sys.exit(f"{args.replay}: {len(replies)} replies for {len(cmds)} commands")
        wall = None
        when = None
    else:
        data = open(bit, "rb").read()
        if hashlib.sha256(data).hexdigest() != image["sha256"]:
            sys.exit(f"{bit}: not the bitstream the image record names")
        if args.flash and ftdi_ports.flash(board, bit) != 0:
            sys.exit(f"board {board}: the flash failed")
        line = Line(board)
        replies = []
        t0 = time.time()
        when = time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime(t0))
        try:
            for cmd, n in cmds:
                line.send(cmd)
                r = line.recv(n, time.time() + args.timeout)
                if len(r) < n:
                    sys.exit(f"command {cmd.hex()}: {len(r)} of {n} reply bytes in {args.timeout}s ({bytes(r).hex()})")
                replies.append(r)
        finally:
            line.close()
        wall = time.time() - t0

    record, why = check(trace, image, replies)
    if record is None:
        sys.exit(f"board {board}: {why}")
    record.update({
        "board": board,
        "part": image["part"],
        "bitstream_sha256": image["sha256"],
        "date": when,
        "host_wall_s": None if wall is None else round(wall, 3),
        "power_w": args.power_w,
        "energy_uj_per_token": None if args.power_w is None else round(args.power_w * record["us_per_token"], 3),
        "replayed": bool(args.replay),
    })
    text = json.dumps(record, indent=2)
    if args.out:
        open(args.out, "w").write(text + "\n")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
