#!/usr/bin/env python3
"""split-host — run the tiny transformer's decode step split over the two Nexys boards and record what the chips did.

The split ledger (benchmarks/silicon/decode_split_images.json) holds one image a chip of the step split Megatron's way:
chip 0 (the embedding, the residuals, the head, one head of attention and half the FFN) on Board A, chip 1 (the other head
and half) on Board B. Each serves the protocol of `spatial::lower::fabric_board::chip_board_verilog`: `I` identifies it,
`W`/`R` write and read a word, and `X` starts the chip's step or continues it past the halt it stands at, replying once it
halts again (`x`, the link, the cycles it has run) or finishes (`x`, 0xFFFF, the cycles).

The ledger also holds the relay the compiler derived from both chips' halts: a list of runs (`run` chip `c` to its link
`to`) and copies (`copy` a tensor from one chip to the other). This host executes that relay, step by step through the
golden trace, and holds every reply to the record: each halt at the link the relay reaches, each count of cycles to the
chip's twin, each prompt step's next token to the evaluator's, the generated tokens to the trace. A run issues `X` and
moves on; the host waits on a chip only when it next copies from it or runs it, so the chips' heads run at once.

The link between the boards is this host: each copy reads the tensor's words from one board and writes them to the
other, over Board A's FIFO and Board B's UART. The record says so, and states the wall time each step took.

    python3 scripts/split-host.py [--flash] [--out FILE]
    python3 scripts/split-host.py --dry-run          # each command's board and bytes, nothing opened
    python3 scripts/split-host.py --replay FILE      # replies read from FILE (`A|B hex`, one a line, each board's in order)
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

DONE = 0xFFFF


def u16(v):
    return int(v).to_bytes(2, "little")


def u32(v):
    return int(v).to_bytes(4, "little")


def u64(v):
    return (int(v) & (2**64 - 1)).to_bytes(8, "little")


def le(b):
    return int.from_bytes(bytes(b), "little")


def signed(v):
    return v - 2**64 if v >= 2**63 else v


class Refused(Exception):
    pass


class Replay:
    """Replies from a file, `A|B hex` a line: each board's in the order it answered its own commands, as a board does.
    What the host sends is printed under --dry-run."""

    def __init__(self, path, dry):
        self.replies = {}
        if not dry:
            for line in open(path):
                if line.strip():
                    board, hexed = line.split()
                    self.replies.setdefault(board, []).append(bytes.fromhex(hexed))
        self.dry = dry

    def send(self, board, data):
        if self.dry:
            print(board, data.hex())

    def recv(self, board, n):
        if self.dry:
            return None
        if not self.replies.get(board):
            raise Refused(f"board {board}: the replay ran out of replies")
        return self.replies[board].pop(0)[:n]


class Boards:
    """Both boards' byte channels."""

    def __init__(self, boards, timeout):
        self.lines = {b: Line(b) for b in boards}
        self.timeout = timeout
        self.dry = False

    def send(self, board, data):
        self.lines[board].send(data)

    def recv(self, board, n):
        r = self.lines[board].recv(n, time.time() + self.timeout)
        if len(r) < n:
            raise Refused(f"board {board}: {len(r)} of {n} reply bytes in {self.timeout}s ({bytes(r).hex()})")
        return bytes(r)

    def close(self):
        for l in self.lines.values():
            l.close()


class Chip:
    """One chip's image and where its step stands."""

    def __init__(self, record):
        self.r = record
        self.board = record["board"]
        self.links = record["links"]
        self.at = None
        self.pending = None
        self.replies = []

    def link_of(self, name, side):
        for l in self.links:
            for t in l[side]:
                if t["name"] == name:
                    return t
        raise Refused(f"chip {self.r['chip']} has no {side[:-1]} of {name}")


def run(ledger, trace, io):
    """Execute the trace over both chips through the ledger's relay; the measured record, or a `Refused`."""
    chips = [Chip(c) for c in sorted(ledger["chips"], key=lambda c: c["chip"])]
    prompt, generated = trace["prompt_tokens"], trace["generated_tokens"]
    steps = len(prompt) + len(generated) - 1

    def ask(chip, cmd, n):
        io.send(chip.board, cmd)
        return io.recv(chip.board, n)

    def settle(chip):
        """Read the reply a run left pending on `chip`, and hold it to the link the run reached and the twin's count."""
        if chip.pending is None:
            return
        want_at, want_cycles = chip.pending
        chip.pending = None
        r = io.recv(chip.board, 7)
        if r is None:
            return
        if r[0] != ord("x"):
            raise Refused(f"chip {chip.r['chip']}: reply {r.hex()} to X is not a run's")
        at, cycles = le(r[1:3]), le(r[3:7])
        if at != want_at:
            raise Refused(f"chip {chip.r['chip']}: halted at {at:#x}, the relay reaches link {want_at:#x}")
        if cycles != want_cycles:
            raise Refused(f"chip {chip.r['chip']}: {cycles} cycles at {at:#x}, the twin's {want_cycles}")
        chip.replies.append(cycles)

    for chip in chips:
        r = ask(chip, b"I", 6)
        if r is not None and (r[:5] != b"iLOGS" or r[5] != chip.r["read_latency"]):
            raise Refused(f"chip {chip.r['chip']}: identify replied {r.hex()}, not the image's iLOGS and read latency")
    tokens, stepped, step_wall, step_cycles = [], [], [], []
    tok = prompt[0]
    t_all = time.time()
    for pos in range(steps):
        t0 = time.time()
        for chip in chips:
            chip.at, chip.replies = None, []
            for inp in chip.r["inputs"]:
                value = tok + 1 if inp["name"] == "tok" else pos + 1
                r = ask(chip, b"W" + u16(inp["tensor"]) + u32(inp["words"][0]) + u64(value), 1)
                if r is not None and r != b"k":
                    raise Refused(f"chip {chip.r['chip']}: write of {inp['name']} replied {r.hex()}")
        for st in ledger["relay"]:
            if "run" in st:
                chip, to = chips[st["run"]], st["to"]
                settle(chip)
                seg, links = chip.r["segments"], chip.links
                # A link before the first section is the host's to serve before the chip starts; a halting link is
                # where the run stops; a link after the last section, or the end past every link, is where the chip
                # finishes — once: a run past a link at its end runs nothing.
                ended = lambda l: l["after"] is not None and not l["halts"]
                if to < len(links) and links[to]["after"] is None:
                    pass
                elif to < len(links) and links[to]["halts"]:
                    io.send(chip.board, b"X")
                    chip.pending = (to, seg[sum(1 for l in links[:to] if l["halts"])])
                elif to < len(links) or chip.at is None or not ended(links[chip.at]):
                    io.send(chip.board, b"X")
                    chip.pending = (DONE, seg[-1])
                chip.at = to
            else:
                src, dst = chips[st["from"]], chips[st["to"]]
                settle(src)
                settle(dst)
                s, d = src.link_of(st["copy"], "sends"), dst.link_of(st["copy"], "receives")
                if len(s["words"]) != len(d["words"]):
                    raise Refused(f"{st['copy']}: {len(s['words'])} words sent, {len(d['words'])} received")
                for ws, wd in zip(s["words"], d["words"]):
                    r = ask(src, b"R" + u16(s["tensor"]) + u32(ws), 9)
                    v = 0 if r is None else signed(le(r[1:9]))
                    if r is not None and r[0] != ord("r"):
                        raise Refused(f"chip {src.r['chip']}: read of {st['copy']} replied {r.hex()}")
                    w = ask(dst, b"W" + u16(d["tensor"]) + u32(wd) + u64(v), 1)
                    if w is not None and w != b"k":
                        raise Refused(f"chip {dst.r['chip']}: write of {st['copy']} replied {w.hex()}")
        for chip in chips:
            settle(chip)
        head = chips[0]
        r = ask(head, b"R" + u16(head.r["next"]["tensor"]) + u32(head.r["next"]["words"][0]), 9)
        nxt = None if r is None else le(r[1:9]) - 1
        step_wall.append(time.time() - t0)
        step_cycles.append([c.replies[-1] if c.replies else None for c in chips])
        if nxt is None:
            tok = prompt[pos + 1] if pos + 1 < len(prompt) else generated[pos + 1 - len(prompt)]
            continue
        if pos < len(prompt):
            want = head.r["prompt_next_tokens"][pos]
            if nxt != want:
                raise Refused(f"the step at position {pos}: next token {nxt}, the evaluator's {want}")
            stepped.append(nxt)
        if pos + 1 >= len(prompt):
            tokens.append(nxt)
        tok = prompt[pos + 1] if pos + 1 < len(prompt) else nxt
    if tokens and tokens != generated:
        raise Refused(f"generated {tokens}, the trace {generated}")
    wall = time.time() - t_all
    mhz = [c.r["core_mhz"] for c in chips]
    chip_us = [round(c.r["segments"][-1] / m, 3) for c, m in zip(chips, mhz)]
    return {
        "tokens": tokens,
        "prompt_next_tokens": stepped,
        "steps": steps,
        "step_cycles": step_cycles,
        "chip_us_per_step": chip_us,
        "wall_s_per_step": round(sum(step_wall) / len(step_wall), 6),
        "host_wall_s": round(wall, 3),
        "link": "host relay: each copy read from one board and written to the other, over Board A's FT245 FIFO and Board B's 115200-baud UART",
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--model", default=os.path.join(ROOT, "benchmarks/silicon/tiny_transformer.json"))
    ap.add_argument("--images", default=os.path.join(ROOT, "benchmarks/silicon/decode_split_images.json"))
    ap.add_argument("--flash", action="store_true", help="configure both boards with their images' bitstreams first")
    ap.add_argument("--timeout", type=float, default=30.0, help="seconds to wait for each reply")
    ap.add_argument("--out", help="where to write the measured record (stdout when absent)")
    ap.add_argument("--dry-run", action="store_true", help="print each command's board and bytes, open nothing")
    ap.add_argument("--replay", help="read the replies from FILE (`A|B hex`, one a line, each board's in order) instead of the boards")
    args = ap.parse_args()
    trace = json.load(open(args.model))["golden_trace"]
    ledger = json.load(open(args.images))
    for field in ("relay", "transfers", "chips"):
        if field not in ledger:
            sys.exit(f"{args.images}: the split ledger does not say its `{field}`")
    for c in ledger["chips"]:
        for field in ("chip", "board", "part", "bitstream", "sha256", "read_latency", "core_mhz", "segments", "links", "inputs"):
            if field not in c:
                sys.exit(f"{args.images}: chip {c.get('chip')} does not say its `{field}`")
    if args.dry_run or args.replay:
        io = Replay(args.replay, args.dry_run)
    else:
        for c in ledger["chips"]:
            bit = os.path.join(os.path.dirname(args.images), c["bitstream"])
            if hashlib.sha256(open(bit, "rb").read()).hexdigest() != c["sha256"]:
                sys.exit(f"{bit}: not the bitstream chip {c['chip']}'s record names")
            if args.flash and ftdi_ports.flash(c["board"], bit) != 0:
                sys.exit(f"board {c['board']}: the flash failed")
        io = Boards(sorted({c["board"] for c in ledger["chips"]}), args.timeout)
    when = None if (args.dry_run or args.replay) else time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime())
    try:
        record = run(ledger, trace, io)
    except Refused as e:
        sys.exit(f"split run refused: {e}")
    finally:
        if isinstance(io, Boards):
            io.close()
    if args.dry_run:
        return 0
    record.update({
        "boards": {c["board"]: {"chip": c["chip"], "part": c["part"], "bitstream_sha256": c["sha256"], "core_mhz": c["core_mhz"]} for c in ledger["chips"]},
        "date": when,
        "replayed": bool(args.replay),
    })
    text = json.dumps(record, indent=2)
    if args.out:
        open(args.out, "w").write(text + "\n")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
