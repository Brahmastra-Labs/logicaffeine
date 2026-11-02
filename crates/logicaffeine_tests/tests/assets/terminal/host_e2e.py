#!/usr/bin/env python3
"""host_e2e — the Fabric Terminal's host (`scripts/logos-terminal.py`) end to end against a model of the board's
link: the model keeps the text buffer, cursor, pointer and RESULT the way `term_link.v` does, answers like the
board, plays a PS/2 keyboard or mouse on the board's USB port, and plays an engine that takes images and answers
with the reply the lock computed. A session per device the board's USB bridge can serve, then one on telling them
apart:

  keyboard  the library by F2 / arrows / Enter, a run that ends SILICON == VM with the result on the 7-segment
            display, a run whose Shows arrive one by one, a chip that disagrees with the VM called out, a link that
            loses engine bytes stopping the run, an edited program refused by name, Esc stopping a run that never
            ends, a click and keys in the laptop's terminal (no pointer drawn on the board), Ctrl-Q;
  mouse     the device identified and enabled, the pointer drawn where the mouse is, a click on a library entry
            loading it, a click on the run key running it, a click in the editor placing the cursor, a click in
            the laptop's terminal moving the board's pointer, a library longer than its list scrolled and clicked;
  devices   a keyboard that answers no Read ID and whose first key is F5, an empty port, a keyboard and then a mouse
            plugged in while the terminal runs, packets whose bytes equal an acknowledgement, a packet with a damaged
            byte, a port where nothing clocks the host's question in, and a packet in flight when the host asks the
            mouse who it is;
  restart   the board switched off and on, its UART staying or going away meanwhile: the screen drawn again, the
            device asked again, a run the power cut off called out and run again;
  keyboards every printable character and the keypad typed on the board's keyboard and on this computer's, a long
            line scrolled, this computer's keyboards found by their keys on any bus, a refusal to open one named,
            one unplugged and plugged in again, a program picked and run from this computer's keyboard; the board's
            PS/2 line counts logged each second, and a line that clocks with no byte read called out;
  faults    a library entry whose file is gone, an exception while handling a key: each costs that key only;
  history   every run in the run history: the output pane's word on the last ten runs of the image (deterministic,
            or the range), the history outliving the terminal, and `--history`;
  demo      F3: the library run by itself one entry after the next, the highlight stepping down one at a time and
            back to the top, a run that never ends stopped at the limit, a person's key taking over, F3 again;
  fabric    during a run the editor keeps the program and the side list stacks its tiles, each block by its
            role, share of the frame's engine clocks and what it does, the engine's tile bright; a torn reply not
            drawn; F4's fabric view (boxes, the engine's cell blinking); F2 back to the programs; the panel's label
            row naming each tile's role.
Then `--check`, the unattended board check, holds on a sound board and fails on a text buffer with a stuck bit and on
a chip that disagrees with the VM; and on the shell image, which has no engine, the check runs no program and F5
says what to load.

Each check prints `PASS: …`; the first that fails prints `FAIL: …` and exits 1. Inputs, from the lock:
  LOGOS_TERMINAL_TABLE   the terminal protocol table (`terminal_protocol::table_json`)
  E2E_CHIP_TABLE         the engine protocol table (`chip_protocol::table_json`)
  E2E_CASES              the programs, in the order the keyboard session runs them — a loop, Shows, a chip
                         that disagrees, a link that loses bytes, an engine that never answers:
                         [{title, source, fn, args, wire, golden, reply}] (reply hex, or null for an engine that never
                         answers)

The model takes each ENGINE frame's bytes a little later, as the board does once they are inside the engine, and an
ENGINE frame sent before the board reported the last one taken is an error. Where the board paints the activity
panel is read here from the table's panel geometry, not from the host. The host compiles through its own path
(`logos-chip.py`'s `compile_lg`), with `emit_wpe_soc_stand_in.py` in place of the compiler binary.
"""
import contextlib
import errno
import importlib.util
import io
import json
import os
import queue
import struct
import subprocess
import sys
import threading
import time

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 5))
sys.path.insert(0, os.path.join(REPO, "scripts", "lib"))
import terminal_link as tl  # noqa: E402


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


lt = load_module("logos_terminal", os.path.join(REPO, "scripts", "logos-terminal.py"))


def fail(what):
    print(f"FAIL: {what}", flush=True)
    os._exit(1)


def check(cond, what):
    if not cond:
        fail(what)
    print(f"PASS: {what}", flush=True)


def until(cond, what, timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if cond():
            return
        time.sleep(0.01)
    fail(f"timed out waiting for {what}")


def see(cond, what, timeout=10.0):
    """A check on what the board shows, which the host draws a moment after the event that changed it."""
    until(cond, what, timeout)
    print(f"PASS: {what}", flush=True)


class ModelBoard:
    """Board B with the Fabric Terminal image, on the link, as `term_link.v` behaves — and a UART for `tl.Link`."""

    name = "the board model"

    def __init__(self, t, device, engines):
        self.t, self.h, self.b = t, t["host"], t["board"]
        self.cols, self.rows = t["screen"]["cols"], t["screen"]["rows"]
        sc = t["screen"]
        self.small_first, self.small_rows, self.small_cols = sc["small_first_row"], sc["small_rows"], sc["small_cols"]
        self.depth = 8192
        self.mem = [0x0720] * 8192
        self.window = (0, 0, 0, 0)
        self.cursor = self.pointer = None
        self.result = None
        self.slip = tl.Slip(t)
        self.out = queue.Queue()
        self.device = device
        self.engines = engines
        self.engine_in = bytearray()
        self.engine_frames = []
        self.engine_resets = 0
        self.taking = False
        self.lose_byte_of = None
        self.stuck_glyph_bit = None
        self.features = t["features"]["engine"] | t["features"]["small"] | t["features"]["totals"]
        self.ps2_sent = []
        self.answered = 0
        self.in_flight = []
        self.data_next = False
        self.clocks = True
        self.reporting = False
        self.powered = True
        self.port_present = True
        self.line = {"edges": 0, "low": 255, "bytes": 0, "damaged": 0}
        self.reply_override = None
        self.activity = None
        self.totals = None
        self.totals_skew = 0
        self.totals_cycles_skew = 0
        self.snapshot_cycles = 0
        self.totals_asked = 0
        self.run_time = 0.0
        self.pong_latency = 0.0
        self.errors = []
        self.unasked = []
        self.first_read = None

    # ── the UART ───────────────────────────────────────────────────────────────────────────────────────────
    def write(self, data):
        if not self.port_present:
            raise OSError("the board's UART went away")
        if not self.powered:
            return len(data)
        for f in self.slip.push(data):
            self.frame(f)
        return len(data)

    def read(self, n):
        if not self.port_present:
            raise OSError("the board's UART went away")
        try:
            data = bytearray(self.out.get(timeout=0.05))
        except queue.Empty:
            return b""
        while len(data) < n:
            try:
                data += self.out.get_nowait()
            except queue.Empty:
                break
        return bytes(data)

    def close(self):
        pass

    def reopen(self):
        """The UART opened again, as `tl.Link` does after it went away: there once the board has power; what the
        board sent while no one had it open is gone."""
        if not self.port_present:
            raise OSError("the board's UART is not there")
        while not self.out.empty():
            self.out.get_nowait()
        return self

    def send(self, frame):
        if self.powered:
            self.out.put(self.slip.encode(bytes(frame)))

    # ── the power switch ───────────────────────────────────────────────────────────────────────────────────
    def power_off(self, port_vanishes):
        """The board switched off: nothing answers, and with `port_vanishes` its UART leaves this computer too."""
        self.powered = False
        self.port_present = not port_vanishes
        while not self.out.empty():
            self.out.get_nowait()

    def power_on(self):
        """The board switched on: the image comes back from flash with the text buffer, cursor, pointer, display and
        engine at their reset values, and the USB bridge enumerates its device afresh."""
        self.mem = [0x0720] * 8192
        self.window = (0, 0, 0, 0)
        self.cursor = self.pointer = None
        self.result = None
        self.slip = tl.Slip(self.t)
        self.engine_in = bytearray()
        self.taking = False
        self.line = {"edges": 0, "low": 255, "bytes": 0, "damaged": 0}
        self.first_read = None
        self.port_present = self.powered = True
        if self.device is not None:
            self.plug(self.device)

    # ── the board ──────────────────────────────────────────────────────────────────────────────────────────
    def holds(self, row, col):
        """A cell the text buffer holds: one of the screen's, or one of the small plane's."""
        return (row < self.rows and col < self.cols) or \
            (self.small_first <= row < self.small_first + self.small_rows and col < self.small_cols)

    def addr(self, row, col):
        """A cell's address in the text buffer, {row[5:0], col}; a buffer only `depth` cells deep wraps it."""
        return (((row & 0x3F) << 7) | (col & 0x7F)) % self.depth

    def write_cell(self, row, col, attr, glyph):
        if self.stuck_glyph_bit is not None:
            glyph &= ~(1 << self.stuck_glyph_bit)
        if self.holds(row, col):
            self.mem[self.addr(row, col)] = (attr << 8) | glyph

    def frame(self, f):
        k, a = f[0], f[1:]
        h, b = self.h, self.b
        if k == h["ping"]:
            ln = self.line
            fields = {"version": self.t["version"], "cols": self.cols, "rows": self.rows,
                      "panel_rows": self.t["screen"]["panel_rows"], "features": self.features,
                      "ps2_edges_hi": ln["edges"] >> 8 & 0xFF, "ps2_edges_lo": ln["edges"] & 0xFF,
                      "ps2_shortest_low_us": ln["low"], "ps2_bytes_hi": ln["bytes"] >> 8 & 0xFF,
                      "ps2_bytes_lo": ln["bytes"] & 0xFF, "ps2_damaged": min(255, ln["damaged"])}
            pong = [b["pong"]] + [fields[name] for name in self.t["pong"]]
            if self.pong_latency:
                threading.Timer(self.pong_latency, self.send, (pong,)).start()
            else:
                self.send(pong)
        elif k == h["put"]:
            row, col = a[0], a[1]
            if row >= self.rows and not self.features & self.t["features"]["small"]:
                self.unasked.append((time.time(), f"a PUT at row {row}"))
            for i in range(2, len(a) - 1, 2):
                self.write_cell(row, col, a[i], a[i + 1])
                row, col = (row + 1, 0) if col == self.cols - 1 else (row, (col + 1) & 0x7F)
        elif k == h["fill"]:
            r0, c0, hh, ww, attr, glyph = a[:6]
            for r in range(r0, r0 + hh):
                for c in range(c0, c0 + ww):
                    self.write_cell(r & 0x7F, c & 0x7F, attr, glyph)
        elif k == h["cursor"]:
            self.cursor = (a[0], a[1]) if a[2] else None
        elif k == h["pointer"]:
            self.pointer = (a[0], a[1]) if a[2] else None
        elif k == h["window"]:
            if not self.features & self.t["features"]["small"]:
                self.unasked.append((time.time(), "a WINDOW frame"))
            self.window = tuple(x & 0x7F for x in a[:4])
        elif k == h["result"]:
            self.result = int.from_bytes(a[:4], "big")
        elif k == h["engine_reset"]:
            self.engine_resets += 1
            self.engine_in = bytearray()
            self.totals = None
        elif k == h["engine"]:
            if self.taking:
                self.errors.append("an ENGINE frame came before the board reported the last one taken")
            self.taking = True
            self.engine_frames.append(len(a))
            self.engine_in += a
            taken = len(a) - 1 if self.lose_byte_of is not None and self.lose_byte_of.startswith(bytes(self.engine_in)) else len(a)
            threading.Timer(0.002, self.taken, (taken,)).start()
        elif k == h["ps2_send"]:
            self.ps2_sent.append(a[0])
            self.ps2(*self.in_flight)
            self.in_flight = []
            if self.clocks:
                self.clock_only(11)
            self.send([b["ps2_sent"], 0 if self.clocks else self.t["ps2"]["no_ack"]])
            self.ps2(*self.answer(a[0]))
            self.answered += 1
        elif k == h["activity"]:
            self.send([b["activity"]] + self.activity_bytes())
        elif k == h["totals"]:
            self.totals_asked += 1
            self.send([b["totals"]] + self.layout_bytes(self.t["totals"], self.totals or {}))
        elif k == h["read"]:
            row, col, n = a[0], a[1], a[2]
            if self.first_read is None:
                self.first_read = time.time()
            out = [b["cells"], row, col]
            for _ in range(n):
                cell = self.mem[self.addr(row, col)]
                out += [cell >> 8, cell & 0xFF]
                row, col = (row + 1, 0) if col == self.cols - 1 else (row, (col + 1) & 0x7F)
            self.send(out)
        else:
            self.errors.append(f"unknown host frame kind {k:#04x}")

    def activity_bytes(self):
        """The model engine's last frame as ACTIVITY's payload, by the table's layout: `activity` names the fields set
        (a list for the per-block and per-cell clocks, padded with zeros); the rest are zero."""
        return self.layout_bytes(self.t["activity"], self.activity or {})

    @staticmethod
    def layout_bytes(layout, values):
        """A report's payload by its table layout, from `values` by field name (a list for a field of several,
        padded with zeros); a field not named is zero."""
        out = []
        for f in layout:
            v = values.get(f["name"], [] if f["count"] > 1 else 0)
            for x in ((list(v) + [0] * f["count"])[: f["count"]] if f["count"] > 1 else [v]):
                out += list(int(x).to_bytes(f["bytes"], "big"))
        return out

    def taken(self, n):
        self.taking = False
        self.send([self.b["engine_taken"], n])
        self.engine_answer()

    def engine_answer(self):
        image = bytes(self.engine_in)
        for case in self.engines:
            wire = case["wire_bytes"]
            if image == wire:
                reply = self.reply_override if self.reply_override is not None else case["reply"]
                if reply is not None:
                    if self.run_time:
                        threading.Timer(self.run_time, self.run_ends, (case, reply)).start()
                    else:
                        self.run_ends(case, reply)
                return
            if wire.startswith(image):
                return
        self.errors.append(f"the engine was sent {len(image)} bytes that begin no program's image")

    def run_ends(self, case, reply):
        """The engine's run ends, as the board has it: its totals are in, its frames count no more clocks, and its
        result goes out; the cycles TOTALS opens with are the activity panel's snapshot's, which takes the run's
        last count at the next frame's start (FRAME_S later)."""
        if case.get("totals") is not None:
            blocks = list(case["totals"]["block_totals"])
            blocks[0] += self.totals_skew
            final = case["totals"]["cycles"] + self.totals_cycles_skew
            self.totals = dict(case["totals"], block_totals=blocks, cycles=self.snapshot_cycles)
            threading.Timer(FRAME_S, self.snapshot, (final,)).start()
        if self.activity:
            f = self.activity["frame"] + 1
            self.activity = dict(self.activity, running=0, frame=f, frame_again=f,
                                 block_clocks=[0] * len(self.activity["block_clocks"]),
                                 cell_clocks=[0] * len(self.activity["cell_clocks"]))
        reply = bytes.fromhex(reply)
        for i in range(0, len(reply), 16):
            self.send([self.b["engine"]] + list(reply[i:i + 16]))

    def snapshot(self, cycles):
        self.snapshot_cycles = cycles
        if self.totals is not None:
            self.totals = dict(self.totals, cycles=cycles)

    # ── the USB device ─────────────────────────────────────────────────────────────────────────────────────
    def answer(self, cmd):
        """What the device on the USB port answers a command with (the Nexys A7 reference manual, §7). `keyboard`
        answers Read ID as a PS/2 keyboard does; `quiet keyboard` answers only the commands of the manual's keyboard
        table (ED and F3 with their data byte, EE, FE, FF); `mouse` answers as its mouse table says; None is an empty
        port."""
        if self.device is None:
            return []
        if self.device == "quiet keyboard":
            if self.data_next:
                self.data_next = False
                return [0xFA]
            if cmd in (0xED, 0xF3):
                self.data_next = True
                return [0xFA]
            return {0xEE: [0xEE], 0xFF: [0xFA, 0xAA]}.get(cmd, [])
        if self.device == "keyboard":
            return {0xF2: [0xFA, 0xAB, 0x83], 0xEE: [0xEE], 0xFF: [0xFA, 0xAA]}.get(cmd, [0xFA])
        if cmd in (0xF4, 0xF5, 0xFF):
            self.reporting = cmd == 0xF4
        return {0xF2: [0xFA, 0x00], 0xFF: [0xFA, 0xAA, 0x00]}.get(cmd, [0xFA])

    def plug(self, device):
        """`device` plugged into the USB port: the bridge reports its self-test, 0xAA, and a mouse its ID, 0x00; a
        mouse reports no movement until it is sent Enable Data Reporting (0xF4)."""
        self.device = device
        self.data_next = False
        self.reporting = False
        self.ps2(0xAA, *([0x00] if device == "mouse" else []))

    def ps2(self, *codes):
        for c in codes:
            self.clock_only(11)
            self.line["bytes"] = (self.line["bytes"] + 1) & 0xFFFF
            self.send([self.b["ps2"], c, 0])

    def ps2_damaged(self, code):
        self.clock_only(11)
        self.line["damaged"] += 1
        self.send([self.b["ps2"], code, self.t["ps2"]["parity_error"]])

    def clock_only(self, edges, low_us=40):
        """The PS/2 clock moving `edges` times with `low_us` low phases, as the board's line monitor counts it — on
        its own, a bridge that clocks but whose bytes the board does not read."""
        self.line["edges"] = (self.line["edges"] + edges) & 0xFFFF
        self.line["low"] = min(self.line["low"], low_us)

    def press(self, code, extended=False):
        pre = [0xE0] if extended else []
        self.ps2(*pre, code, *pre, 0xF0, code)

    def chord(self, mod, code):
        self.ps2(mod, code, 0xF0, code, 0xF0, mod)

    def mouse(self, dx, dy, left=False):
        """One movement packet: dx right, dy up, each within the packet's nine signed bits — sent only while the
        mouse's reports are enabled."""
        flags = 0x08 | (0x01 if left else 0) | (0x10 if dx < 0 else 0) | (0x20 if dy < 0 else 0)
        if self.reporting:
            self.ps2(flags, dx & 0xFF, dy & 0xFF)

    def move_to(self, mouse, x, y):
        """Packets that take the host's pointer (`mouse`, its `Mouse`) to screen pixel (x, y)."""
        dx, dy = x - mouse.x, mouse.y - y
        while dx or dy:
            step_x, step_y = max(-255, min(255, dx)), max(-255, min(255, dy))
            self.mouse(step_x, step_y)
            dx, dy = dx - step_x, dy - step_y

    def click(self):
        self.mouse(0, 0, left=True)
        self.mouse(0, 0)

    # ── what the screen shows ──────────────────────────────────────────────────────────────────────────────
    def text(self, row):
        out = []
        for col in range(self.cols):
            g = self.mem[self.addr(row, col)] & 0x7F
            out.append(tl.char_of(g, self.t))
        return "".join(out)

    def small_text(self, srow):
        """Row `srow` of the small plane, as written."""
        return "".join(tl.char_of(self.mem[self.addr(self.small_first + srow, c)] & 0x7F, self.t)
                       for c in range(self.small_cols))

    def shown_cells(self, row):
        """What the monitor shows on text row `row`, as `term_vga.v` draws it: the row's cells, or where the
        small-text window lies over it, two lines of 8 pixels — each the small plane's row under the window there,
        and the row's own cells either side of it. Each line is a list of cells."""
        wr, wc, wrows, wcols = self.window
        inside = wrows != 0 and wr <= row < wr + wrows
        lines = []
        for half in ((0, 1) if inside else (None,)):
            line = []
            for c in range(self.cols):
                if half is not None and wc <= c < wc + wcols:
                    line.append(self.mem[self.addr(self.small_first + ((row - wr) << 1) + half, c - wc)])
                else:
                    line.append(self.mem[self.addr(row, c)])
            lines.append(line)
        return lines

    def shown(self, row):
        """The text of what the monitor shows on text row `row` (`shown_cells`), its lines joined by newlines."""
        return "\n".join("".join(tl.char_of(x & 0x7F, self.t) for x in line) for line in self.shown_cells(row))

    def screen(self):
        return "\n".join(self.text(r) for r in range(self.rows))

    def panel_cells(self):
        """The cells the board paints the activity panel over (`term_activity.v`), from the table's geometry."""
        p = self.t["panel"]
        cells = [(p["title_row"], c) for c in range(p["cycles_col"], p["cycles_col"] + 10)]
        cells += [(p["block_row"], c) for c in range(p["block_col"], p["block_col"] + p["blocks"] * p["block_tile"])]
        cells += [(p["block_row"], c) for c in range(p["stack_col"], p["stack_col"] + p["depth"])]
        cells += [(p["block_row"], c) for c in range(p["heap_col"], p["heap_col"] + (p["heap"] + 7) // 8)]
        cells += [(p["cell_row"], c) for c in range(p["cell_col"], p["cell_col"] + 2 * p["cells"])]
        return cells


class Laptop:
    """The terminal the host runs in, as `LocalTerminal` hands it over: the keys and clicks typed there."""

    def __init__(self):
        self.events = queue.Queue()

    def read(self):
        out = []
        while not self.events.empty():
            out.append(self.events.get_nowait())
        return out

    def draw(self, screen):
        pass

    def key(self, k):
        self.events.put(("key", k))

    def click(self, row, col):
        self.events.put(("click", (row, col)))


def chip_for(cases):
    """`scripts/logos-chip.py`, whole, with its compiler binary replaced by `emit_wpe_soc_stand_in.py`, which
    answers from the lock's cases and refuses anything else the way the emitter refuses a program too big for the
    engine — so the host's own compile path (command line, refusal, facts) is the one under test."""
    chip = load_module("logos_chip", os.path.join(REPO, "scripts", "logos-chip.py"))
    chip.KIT = None
    chip._protocol = json.load(open(os.environ["E2E_CHIP_TABLE"]))
    stand_in = os.path.join(os.path.dirname(os.path.abspath(__file__)), "emit_wpe_soc_stand_in.py")
    chip.compiler = lambda name, build=True: stand_in if name == "emit_wpe_soc" else fail(f"the host asked for {name}")
    return chip


# A video frame, as the model board's activity panel takes its snapshot.
FRAME_S = 1 / 60

# Library entries after the cases, so the side list has more entries than it shows and must scroll.
FILLERS = 7


class Notes:
    """The terminal's log (`Terminal(log=…)`): every line it notes, kept for the checks."""

    def __init__(self):
        self.lines = []
        self.lock = threading.Lock()

    def write(self, text):
        with self.lock:
            self.lines.extend(l for l in text.splitlines() if l.strip())

    def flush(self):
        pass

    def has(self, words):
        with self.lock:
            return any(words in l for l in self.lines)


class Session:
    """The host's terminal running against a model board with `device` on its USB port; `in_flight` are bytes the
    device sent before it takes the host's first command, still on their way when the host asks it. `extra` library
    entries follow the cases and fillers; `osinput` stands for this computer's keyboards and mice."""

    def __init__(self, t, cases, device, engine=True, in_flight=(), clocks=True, extra=(), osinput=None, small=True,
                 totals=True):
        self.board = ModelBoard(t, device, cases)
        self.board.features = (t["features"]["engine"] if engine else 0) | (t["features"]["small"] if small else 0) | \
            (t["features"]["totals"] if engine and totals else 0)
        self.board.in_flight = list(in_flight)
        self.board.clocks = clocks
        library = os.path.abspath("library.txt")
        with open(library, "w") as f:
            for c in cases:
                f.write(f"{c['title']} | {c['source']} | {c['fn'] or ''} | {c['args']}\n")
            for k in range(FILLERS):
                f.write(f"more {k} | {cases[0]['source']} | {cases[0]['fn'] or ''} | {cases[0]['args']}\n")
            for title, source, fn, args in extra:
                f.write(f"{title} | {source} | {fn} | {args}\n")
        os.environ["LOGOS_TERMINAL_LIBRARY"] = library
        self.link = tl.Link(t, ser=self.board)
        self.laptop = Laptop()
        self.notes = Notes()
        self.term = lt.Terminal(t, chip_for(cases), self.link, self.laptop, engine=engine, log=self.notes,
                                pong=self.link.ping())
        self.term.osinput = osinput
        self.thread = threading.Thread(target=self.term.loop, daemon=True)
        self.thread.start()

    def bar(self):
        return self.board.text(lt.OUT_BAR)

    def output(self):
        return "\n".join(self.board.text(lt.OUT_TOP + i) for i in range(lt.OUT_ROWS))

    def end(self):
        self.thread.join(timeout=5)
        if self.thread.is_alive():
            fail("the terminal's loop did not end")
        self.link.close()
        if self.board.errors:
            fail("; ".join(self.board.errors))


def keyboard_session(t, cases):
    s = Session(t, cases, "keyboard")
    b, term = s.board, s.term
    titles = [c["title"] for c in cases]
    until(lambda: "Fabric Terminal" in b.text(0) and term.device == "keyboard", "the first screen and the keyboard")
    check(b.ps2_sent[:1] == [0xF2], "the host asks the board's USB device who it is")
    see(lambda: "USB keyboard on the board" in b.text(t["panel"]["title_row"]), "the panel names the keyboard on the board")

    written = [(r, c) for r, c in b.panel_cells() if b.mem[b.addr(r, c)] & 0x7F != 0x20]
    check(not written, f"the cells the board paints the panel over are blank (written: {written[:6]})")
    for label in ("BLOCKS", "CELLS", "STACK", "HEAP", "CYCLES"):
        check(label in b.screen().split("\n", t["panel"]["title_row"])[-1], f"the panel is labelled {label}")

    def pick(title):
        b.press(0x06)
        until(lambda: term.focus == "library", "the library to take the keys")
        for _ in range(term.pick):
            b.press(0x75, extended=True)
        for _ in range(titles.index(title)):
            b.press(0x72, extended=True)
        b.press(0x5A)
        until(lambda: title in b.text(0), f"{title} in the title bar")

    loop, shows, wrong, lossy, forever = cases[:5]
    pick(loop["title"])
    check(term.editor.text() == open(loop["source"]).read(), "Enter on a library entry loads its source into the editor")
    b.press(0x03)
    until(lambda: "SILICON == VM" in s.bar() or "!=" in s.bar() or "refused" in s.bar(), "the run to finish")
    check("SILICON == VM" in s.bar(), "F5 runs the program on the fabric and the answer is the VM's")
    check(bytes(b.engine_in) == loop["wire_bytes"], "the engine received the program's image, every byte")
    check(all(n <= t["engine_frame_max"] for n in b.engine_frames), "no ENGINE frame is longer than the board takes")
    see(lambda: f"= {loop['golden']}" in s.output(), "the output pane shows the result")
    see(lambda: b.result == int(loop["golden"]), "the result goes to the 7-segment display")

    pick(shows["title"])
    b.press(0x03)
    until(lambda: "SILICON == VM" in s.bar() or "!=" in s.bar(), "the Show program to finish")
    check("SILICON == VM" in s.bar(), "a program's Shows are the VM's")
    for line in shows["golden"]:
        see(lambda: f"> {line}" in s.output(), f"the Show {line!r} is printed")

    pick(wrong["title"])
    b.press(0x03)
    see(lambda: "SILICON != VM" in s.bar(), "a chip that disagrees with the VM is called out")

    pick(lossy["title"])
    b.lose_byte_of = lossy["wire_bytes"]
    resets = b.engine_resets
    b.press(0x03)
    see(lambda: "did not take" in s.bar(), "a link that loses engine bytes stops the run and says so")
    see(lambda: b.engine_resets == resets + 2, "the engine is reset after the lost bytes")
    b.lose_byte_of = None

    b.press(0x22)
    until(lambda: "(edited)" in b.text(0), "the title to say edited")
    check(term.editor.changed, "typing on the board's keyboard edits the program")
    b.press(0x03)
    until(lambda: "refused" in s.bar(), "the refusal")
    see(lambda: "9 blocks > hw MAXBLK=8" in s.output(), "a program that does not fit is refused with the cap it exceeds")

    pick(forever["title"])
    resets = b.engine_resets
    b.press(0x03)
    until(lambda: "running on the fabric" in s.bar(), "a run that does not end")
    check(b.engine_resets == resets + 1, "a run starts by resetting the engine")
    b.press(0x76)
    until(lambda: "stopped" in s.bar(), "Esc to stop the run")
    check(b.engine_resets == resets + 2, "Esc resets the engine")

    s.laptop.click(lt.EDIT_TOP + 1, lt.SIDE_COL + 4)
    see(lambda: "PROGRAMS" in b.text(lt.EDIT_TOP)[lt.SIDE_COL:], "after a run, a click on the side list brings back the programs")
    s.laptop.click(lt.EDIT_TOP + 1, lt.SIDE_COL + 4)
    see(lambda: loop["title"] in b.text(0), "a click in the laptop's terminal loads a library entry")
    check(b.pointer is None, "with a keyboard on the board, no pointer is drawn for the laptop's clicks")
    s.laptop.key("F1")
    see(lambda: "THE FABRIC TERMINAL" in b.screen(), "F1 in the laptop's terminal opens the help")
    s.laptop.key("x")
    see(lambda: "THE FABRIC TERMINAL" not in b.screen(), "any key closes the help")

    b.chord(0x14, 0x15)
    s.end()
    check(True, "Ctrl-Q ends the terminal")

    calls = []
    for line in open(os.path.join(os.path.dirname(os.path.abspath(os.environ["E2E_CASES"])), "emit_calls.txt")):
        words = line.split()
        calls.append((os.path.abspath(words[words.index("--program") + 1]), " ".join(words)))
    check(any(p == os.path.abspath(loop["source"]) and "--fn f --args 5" in c and "--profile hw" in c for p, c in calls),
          "a function program is compiled for f(5) against Board B's engine profile")
    check(any(p == os.path.abspath(shows["source"]) and "--main" in c for p, c in calls), "a `## Main` program is compiled whole")


def mouse_session(t, cases):
    s = Session(t, cases, "mouse")
    b, term = s.board, s.term
    until(lambda: term.device == "mouse" and 0xF4 in b.ps2_sent, "the mouse to be identified and enabled")
    check(b.ps2_sent[:2] == [0xF2, 0xF4], "a mouse answers Read ID and the host enables its reports")
    see(lambda: "USB mouse on the board" in b.text(t["panel"]["title_row"]), "the panel names the mouse on the board")

    entry = 1
    row, col = lt.EDIT_TOP + 1 + entry, lt.SIDE_COL + 4
    b.move_to(term.mouse, col * 8 + 4, row * 16 + 8)
    see(lambda: b.pointer == (row, col), "the board draws the pointer where the mouse moved")
    b.click()
    until(lambda: cases[entry]["title"] in b.text(0), "the clicked entry to load")
    check(term.editor.text() == open(cases[entry]["source"]).read(), "a click on a library entry loads it")
    see(lambda: b.pointer == (row, col), "the pointer stays where it clicked")
    keys_top = lt.EDIT_TOP + lt.EDIT_ROWS - len(lt.KEYS)
    b.move_to(term.mouse, (lt.SIDE_COL + 3) * 8 + 4, keys_top * 16 + 8)
    b.click()
    see(lambda: "SILICON == VM" in s.bar(), "a click on the run key runs the program on the fabric")

    line = 1
    row, col = lt.EDIT_TOP + line, lt.GUTTER + 4
    b.move_to(term.mouse, col * 8 + 4, row * 16 + 8)
    until(lambda: b.pointer == (row, col), "the pointer in the editor")
    b.click()
    want = (row, min(lt.GUTTER + len(term.editor.lines[line]), col))
    see(lambda: b.cursor == want, "a click in the editor places the cursor")

    s.laptop.click(lt.EDIT_TOP, lt.GUTTER)
    see(lambda: b.pointer == (lt.EDIT_TOP, lt.GUTTER), "a click in the laptop's terminal moves the board's pointer there")

    last = len(cases) + FILLERS - 1
    s.laptop.key("F2")
    for _ in range(last):
        s.laptop.key("Down")
    first = last - lt.LIBRARY_ROWS + 1
    see(lambda: b.text(lt.EDIT_TOP + lt.LIBRARY_ROWS)[lt.SIDE_COL + 2:].startswith(f"more {FILLERS - 1}"),
        "the library list scrolls to the last entry")
    see(lambda: b.text(lt.EDIT_TOP + 1)[lt.SIDE_COL + 23] == "▲", "the list marks the entries above it")
    b.move_to(term.mouse, (lt.SIDE_COL + 4) * 8 + 4, lt.EDIT_TOP * 16 + 24)
    b.click()
    see(lambda: cases[first]["title"] in b.text(0), "a click on a scrolled list loads the entry shown there")

    term.quit = True
    s.end()


def device_sessions(t, cases):
    """The device on the board's USB port told by what it answers (the Nexys A7 reference manual, §7.1: Read ID; a
    keyboard's Echo; a device plugged in reports 0xAA, a mouse its ID 0x00 right after), never by a key or a packet
    byte that happens to equal an answer; and a mouse's packets kept whole whatever their bytes are."""
    title_row = t["panel"]["title_row"]

    s = Session(t, cases, "quiet keyboard")
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0) and b.ps2_sent[:1] == [0xF2], "the first screen and the host's question")
    b.press(0x03)
    until(lambda: "SILICON == VM" in s.bar() or term.device == "mouse", "F5 to run, or the keyboard taken for a mouse")
    check(term.device == "keyboard" and "SILICON == VM" in s.bar(),
          "F5 first (scan code 0x03, a wheel mouse's ID) on a keyboard that does not answer Read ID runs the program")
    check(0xF4 not in b.ps2_sent, "a keyboard is never sent a mouse's Enable Data Reporting")
    see(lambda: "USB keyboard on the board" in b.text(title_row), "the panel names the keyboard")
    e = term.editor
    row, col = e.row, e.col
    b.ps2(0xE0)
    b.ps2_damaged(0x75)
    b.press(0x1C)
    see(lambda: e.lines[row][col:col + 1] == "a", "a key whose code arrived damaged is dropped, and the next key is itself")
    term.quit = True
    s.end()

    s = Session(t, cases, None)
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0), "the first screen")
    until(lambda: term.device not in ("identifying",) and b.answered == len(b.ps2_sent) >= 2, "the questions to an empty port")
    check(term.device is None and b.ps2_sent == [0xF2, 0xEE],
          "an empty port, asked Read ID and then a keyboard's Echo, answers neither and holds no device")
    see(lambda: "no USB device on the board" in b.text(title_row), "the panel says no device is on the board")
    b.device = "keyboard"
    see(lambda: term.device == "keyboard", "a keyboard the bridge holds whose self-test never reached the board is "
        "found when the host asks the silent port again", timeout=lt.PROBE_S + 3)
    check(sum("none answers" in l for l in s.notes.lines) == 1,
          "the silent port was said once, not at every asking")
    check(not s.notes.has("board USB line"),
          "and the clock edges of the host's own questions are not reported as the line's traffic")

    resets = b.engine_resets
    b.plug("quiet keyboard")
    b.press(0x03)
    until(lambda: b.engine_resets > resets or term.device == "mouse", "F5 to run, or the keyboard taken for a mouse")
    check(term.device == "keyboard" and b.engine_resets == resets + 1,
          "a keyboard plugged in while the terminal runs is a keyboard at its first key, F5 among them")
    until(lambda: "SILICON == VM" in s.bar(), "the run to finish")

    plugged = len(b.ps2_sent)
    b.plug("mouse")
    until(lambda: term.device == "mouse" and b.ps2_sent[-1:] == [0xF4] and b.answered == len(b.ps2_sent),
          "a mouse plugged in to be enabled")
    check(0xF2 not in b.ps2_sent[plugged:],
          "a mouse plugged in is known by the ID it sends after its self-test, without a question")
    see(lambda: "USB mouse on the board" in b.text(title_row), "the panel names the mouse")

    x0, y0 = term.mouse.x, term.mouse.y
    b.mouse(-6, 0)
    b.mouse(0, -6)
    b.mouse(-6, -6)
    see(lambda: (term.mouse.x, term.mouse.y) == (x0 - 12, y0 + 12),
        "packets whose movement bytes are 0xFA (6 left, 6 down), an acknowledgement's value, move the pointer by exactly that")

    x0, y0 = term.mouse.x, term.mouse.y
    b.ps2(0x08)
    b.ps2_damaged(0x05)
    b.ps2(0x08)
    b.mouse(10, 0)
    see(lambda: (term.mouse.x, term.mouse.y) == (x0 + 10, y0),
        "a packet with a damaged byte is dropped whole, and the packet after it moves the pointer by exactly its own")
    term.quit = True
    s.end()

    s = Session(t, cases, None, clocks=False)
    b, term = s.board, s.term
    until(lambda: term.device is None and b.answered == len(b.ps2_sent) >= 1, "the question to a port nothing clocks")
    time.sleep(2 * lt.ANSWER_S)
    check(term.device is None and b.ps2_sent == [0xF2],
          "a question no device clocks in (the board reports it unacknowledged) means no device, with no second question")
    term.quit = True
    s.end()

    s = Session(t, cases, "mouse", in_flight=[0x08, 0x05, 0x00])
    b, term = s.board, s.term
    until(lambda: term.device in ("mouse", "keyboard") and b.answered == len(b.ps2_sent), "the device to be identified")
    check(term.device == "mouse" and b.ps2_sent == [0xF2, 0xF4],
          "a packet the mouse sent before it took Read ID is not its answer: the mouse is identified and enabled")
    term.quit = True
    s.end()


def restart_session(t, cases):
    """The board switched off and on under the terminal: its image comes back from flash with a blank screen, so the
    host draws all of it again and asks the USB device who it is again; a run the power cut off is called out and
    runs again; with the board's UART staying on this computer meanwhile, and with it going away and coming back."""
    forever = cases[4]
    s = Session(t, cases, "keyboard")
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0) and term.device == "keyboard", "the first screen and the keyboard")
    time.sleep(0.5)
    before, cursor, asked = b.screen(), b.cursor, b.ps2_sent.count(0xF2)
    b.power_off(port_vanishes=False)
    time.sleep(0.5)
    b.power_on()
    see(lambda: b.screen() == before and b.cursor == cursor,
        "a board switched off and on, its UART still here, gets the whole screen and the cursor again")
    see(lambda: b.ps2_sent.count(0xF2) == asked + 1 and term.device == "keyboard", "its USB device is asked who it is again")

    b.press(0x06)
    until(lambda: term.focus == "library", "the library to take the keys")
    for _ in range(cases.index(forever) - term.pick):
        b.press(0x72, extended=True)
    b.press(0x5A)
    until(lambda: forever["title"] in b.text(0), "the never-ending program in the title bar")
    b.press(0x03)
    until(lambda: "running on the fabric" in s.bar(), "the run")
    b.power_off(port_vanishes=True)
    time.sleep(3.0)
    check(s.thread.is_alive(), "the terminal outlives its board's UART going away")
    b.power_on()
    see(lambda: "Fabric Terminal" in b.text(0) and forever["title"] in b.text(0),
        "the board whose UART went away and came back gets the screen again")
    see(lambda: "restarted" in s.bar(), "a run the power cut off is called out")
    resets = b.engine_resets
    b.press(0x03)
    see(lambda: "running on the fabric" in s.bar() and b.engine_resets == resets + 1, "and F5 runs it again")
    b.press(0x76)
    until(lambda: "stopped" in s.bar(), "Esc to stop the run")
    term.quit = True
    s.end()

    s = Session(t, cases, "mouse")
    b, term = s.board, s.term
    until(lambda: term.device == "mouse" and b.reporting and b.answered == len(b.ps2_sent), "the mouse to be enabled")
    b.power_off(port_vanishes=True)
    time.sleep(2.5)
    b.power_on()
    see(lambda: "Fabric Terminal" in b.text(0) and b.reporting,
        "a mouse whose self-test went out while the UART was away is enabled again after the restart")
    until(lambda: b.answered == len(b.ps2_sent), "the mouse's answer")
    x0 = term.mouse.x
    b.mouse(10, 0)
    see(lambda: term.mouse.x == x0 + 10, "and its packets move the pointer")
    term.quit = True
    s.end()


# ── keyboards ─────────────────────────────────────────────────────────────────────────────────────────────────
# Every printable ASCII character on a US layout, each from its reference rather than from the host's tables: the
# Linux input-event codes (include/uapi/linux/input-event-codes.h) of this computer's keyboards, and PS/2 scan code
# set 2, which the board's USB bridge sends. A row is (codes, the characters, the characters shifted).
EVDEV_ROWS = [(range(2, 14), "1234567890-=", "!@#$%^&*()_+"), (range(16, 28), "qwertyuiop[]", "QWERTYUIOP{}"),
              (range(30, 42), "asdfghjkl;'`", 'ASDFGHJKL:"~'), (range(43, 54), "\\zxcvbnm,./", "|ZXCVBNM<>?")]
EVDEV_SPACE, EVDEV_SHIFT = 57, 42
EVDEV_KEYPAD = dict(zip("7894561230.+-*/", [71, 72, 73, 75, 76, 77, 79, 80, 81, 82, 83, 78, 74, 55, 98]))
EVDEV_NAMED = {"Enter": 28, "End": 107, "PageDown": 109, "Down": 108, "F2": 60, "F5": 63}
SET2_ROWS = [([0x0E, 0x16, 0x1E, 0x26, 0x25, 0x2E, 0x36, 0x3D, 0x3E, 0x46, 0x45, 0x4E, 0x55], "`1234567890-=", "~!@#$%^&*()_+"),
             ([0x15, 0x1D, 0x24, 0x2D, 0x2C, 0x35, 0x3C, 0x43, 0x44, 0x4D, 0x54, 0x5B, 0x5D], "qwertyuiop[]\\", "QWERTYUIOP{}|"),
             ([0x1C, 0x1B, 0x23, 0x2B, 0x34, 0x33, 0x3B, 0x42, 0x4B, 0x4C, 0x52], "asdfghjkl;'", 'ASDFGHJKL:"'),
             ([0x1A, 0x22, 0x21, 0x2A, 0x32, 0x31, 0x3A, 0x41, 0x49, 0x4A], "zxcvbnm,./", "ZXCVBNM<>?")]
SET2_SPACE, SET2_SHIFT = 0x29, 0x12
SET2_KEYPAD = dict(zip("7894561230.+-*", [0x6C, 0x75, 0x7D, 0x6B, 0x73, 0x74, 0x69, 0x72, 0x7A, 0x70, 0x71, 0x79, 0x7B, 0x7C]))
PRINTABLE = "".join(chr(c) for c in range(33, 127)) + " "


def layout(rows, space):
    """character → (code, shifted) for every printable character."""
    keys = {" ": (space, False)}
    for codes, plain, shifted in rows:
        for code, a, b in zip(codes, plain, shifted):
            keys[a], keys[b] = (code, False), (code, True)
    return keys


EVDEV_KEYS, SET2_KEYS = layout(EVDEV_ROWS, EVDEV_SPACE), layout(SET2_ROWS, SET2_SPACE)

# /proc/bus/input/devices as this computer lists them: KEY and REL are capability bitmaps, 64-bit words, the most
# significant first.
USB_KEYBOARD = '''I: Bus=0003 Vendor=046d Product=c31c Version=0110
N: Name="Logitech USB Keyboard"
P: Phys=usb-0000:00:14.0-1/input0
H: Handlers=sysrq kbd leds event7
B: EV=120013
B: KEY=1000000000007 ff9f207ac14057ff febeffdfffefffff fffffffffffffffe
'''
BLUETOOTH_KEYBOARD = '''I: Bus=0005 Vendor=05ac Product=024f Version=0001
N: Name="Keychron K2"
P: Phys=a4:c3:f0:11:22:33
H: Handlers=sysrq kbd leds event9
B: EV=12001f
B: KEY=1000000000007 ff9f207ac14057ff febeffdfffefffff fffffffffffffffe
'''
POWER_BUTTON = '''I: Bus=0019 Vendor=0000 Product=0001 Version=0000
N: Name="Power Button"
P: Phys=LNXPWRBN/button/input0
H: Handlers=kbd event1
B: EV=3
B: KEY=10000000000000 0
'''
LIGHTING = '''I: Bus=0003 Vendor=1462 Product=7e06 Version=0110
N: Name="MSI MYSTIC LIGHT "
P: Phys=usb-0000:00:14.0-2/input0
H: Handlers=event4
B: EV=9
B: ABS=10000000000
'''
USB_MOUSE = '''I: Bus=0003 Vendor=046d Product=c077 Version=0111
N: Name="Logitech USB Optical Mouse"
P: Phys=usb-0000:00:14.0-3/input0
H: Handlers=mouse0 event8
B: EV=17
B: KEY=70000 0 0 0 0
B: REL=903
'''


class EventNode:
    """A /dev/input/event node as the host reads it: whole input_event records (struct timeval, u16 type, u16 code,
    s32 value), non-blocking, and ENODEV once the device is unplugged."""

    SIZE = struct.calcsize("llHHi")

    def __init__(self, path):
        self.name = path
        self.data = bytearray()
        self.lock = threading.Lock()
        self.unplugged = False

    def read(self, n):
        with self.lock:
            if self.unplugged:
                raise OSError(errno.ENODEV, os.strerror(errno.ENODEV))
            take = len(self.data[:n]) // self.SIZE * self.SIZE
            if take == 0:
                raise BlockingIOError(errno.EAGAIN, os.strerror(errno.EAGAIN))
            out, self.data = bytes(self.data[:take]), self.data[take:]
            return out

    def close(self):
        pass

    def events(self, *evs):
        with self.lock:
            for etype, code, value in evs:
                self.data += struct.pack("llHHi", 0, 0, etype, code, value)

    def tap(self, code, shifted=False):
        k = 1
        evs = ([(k, EVDEV_SHIFT, 1)] if shifted else []) + [(k, code, 1), (k, code, 0)] + ([(k, EVDEV_SHIFT, 0)] if shifted else [])
        self.events(*evs)

    def type(self, text):
        for ch in text:
            self.tap(*EVDEV_KEYS[ch])


class Computer:
    """This computer's input devices: the listing the host reads and the nodes it opens; a node may refuse to open
    (`refuse[path] = errno`)."""

    def __init__(self):
        self.blocks, self.nodes, self.refuse = {}, {}, {}
        self.listing = os.path.abspath("input_devices.txt")
        self.write()

    def write(self):
        with open(self.listing + ".tmp", "w") as f:
            f.write("\n".join(self.blocks.values()))
        os.replace(self.listing + ".tmp", self.listing)

    def plug(self, block, path):
        self.blocks[path] = block
        self.nodes[path] = EventNode(path)
        self.write()
        return self.nodes[path]

    def unplug(self, path):
        self.nodes[path].unplugged = True
        del self.blocks[path]
        self.write()

    def input(self, match=""):
        computer = self

        class Here(lt.OsInput):
            def open_device(self, path):
                if path in computer.refuse:
                    raise OSError(computer.refuse[path], os.strerror(computer.refuse[path]))
                node = computer.nodes.get(path)
                if node is None or node.unplugged:
                    raise FileNotFoundError(errno.ENOENT, os.strerror(errno.ENOENT), path)
                return node

        return Here(match, devices_file=self.listing)


def type_set2(board, text):
    for ch in text:
        code, shifted = SET2_KEYS[ch]
        board.ps2(*([SET2_SHIFT] if shifted else []), code, 0xF0, code, *([0xF0, SET2_SHIFT] if shifted else []))


def new_last_line(press):
    """Keys that put the cursor on a new line after the program's last one."""
    for k in ("PageDown", "PageDown", "End", "Enter"):
        press(k)


def keyboard_sessions(t, cases):
    """Typing, from the board's USB keyboard and from keyboards plugged into this computer: every printable character
    and the numeric keypad arrive as themselves; a line longer than the editor scrolls so the cursor and the text
    beside it stay on screen. This computer's keyboards are found by the keys they have, on any bus; a node the host
    may not open is named with the reason; one unplugged and plugged in again is let go and taken back."""
    title_row = t["panel"]["title_row"]

    s = Session(t, cases, "keyboard")
    b, term = s.board, s.term
    until(lambda: term.device == "keyboard", "the keyboard on the board")
    set2_named = {"PageDown": (0xE0, 0x7A), "End": (0xE0, 0x69), "Enter": (0x5A,)}
    new_last_line(lambda k: b.ps2(*set2_named[k], *set2_named[k][:-1], 0xF0, set2_named[k][-1]))
    until(lambda: term.editor.lines[term.editor.row].strip() == "" and term.editor.row == len(term.editor.lines) - 1, "a new last line")
    type_set2(b, PRINTABLE)
    see(lambda: term.editor.lines[term.editor.row].endswith(PRINTABLE),
        "every printable character typed on the board's keyboard arrives as itself")
    see(lambda: b.cursor is not None and lt.GUTTER <= b.cursor[1] < lt.EDIT_COLS
        and b.text(b.cursor[0])[b.cursor[1] - 12:b.cursor[1]] == PRINTABLE[-12:],
        "a line longer than the editor scrolls: the cursor and the characters before it stay on screen")
    for ch in "7894561230.+-*":
        b.ps2(SET2_KEYPAD[ch], 0xF0, SET2_KEYPAD[ch])
    see(lambda: term.editor.lines[term.editor.row].endswith("7894561230.+-*"),
        "the numeric keypad on the board's keyboard types its digits and signs")
    see(lambda: s.notes.has("board USB line: +") and s.notes.has("bytes read whole, +0 damaged (shortest low phase 40 us)"),
        "the terminal logs what the board's PS/2 line carried each second: clock edges, bytes, the shortest low phase")
    b.clock_only(33, low_us=6)
    see(lambda: s.notes.has("+33 clock edges, +0 bytes read whole, +0 damaged (shortest low phase 6 us): the bridge clocks but no byte is read"),
        "a line that clocks with no byte read is called out, with its shortest low phase")
    term.quit = True
    s.end()

    here = Computer()
    usb = here.plug(USB_KEYBOARD, "/dev/input/event7")
    bt = here.plug(BLUETOOTH_KEYBOARD, "/dev/input/event9")
    here.plug(POWER_BUTTON, "/dev/input/event1")
    here.plug(LIGHTING, "/dev/input/event4")
    here.plug(USB_MOUSE, "/dev/input/event8")
    os_in = here.input()
    check(sorted(os_in.added) == ["Keychron K2 (keyboard)", "Logitech USB Keyboard (keyboard)", "Logitech USB Optical Mouse (mouse)"],
          "this computer's keyboards are found by their keys, on USB and on Bluetooth; a power button and a lighting "
          "controller are not keyboards")

    refused = Computer()
    refused.plug(USB_KEYBOARD, "/dev/input/event7")
    refused.plug(BLUETOOTH_KEYBOARD, "/dev/input/event9")
    refused.refuse = {"/dev/input/event7": errno.EACCES, "/dev/input/event9": errno.EBUSY}
    problems = " | ".join(refused.input().problems)
    check("Logitech USB Keyboard" in problems and "input group" in problems
          and "Keychron K2" in problems and "another program" in problems,
          f"a keyboard the host may not open is named with the reason: the input group, or another program holding it ({problems})")

    s = Session(t, cases, None, osinput=os_in)
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0), "the first screen")
    see(lambda: "keyboard on this computer" in b.text(title_row), "the panel names the keyboard on this computer")
    for k in ("PageDown", "PageDown", "End", "Enter"):
        usb.tap(EVDEV_NAMED[k])
    until(lambda: term.editor.lines[term.editor.row].strip() == "" and term.editor.row == len(term.editor.lines) - 1, "a new last line")
    usb.type(PRINTABLE)
    see(lambda: term.editor.lines[term.editor.row].endswith(PRINTABLE),
        "every printable character typed on this computer's keyboard arrives as itself")
    for ch in "7894561230.+-*/":
        usb.tap(EVDEV_KEYPAD[ch])
    see(lambda: term.editor.lines[term.editor.row].endswith("7894561230.+-*/"),
        "the numeric keypad on this computer's keyboard types its digits and signs")

    here.unplug("/dev/input/event7")
    see(lambda: s.notes.has("unplugged: Logitech USB Keyboard"), "a keyboard unplugged from this computer is let go, and said so")
    bt.type("bt")
    see(lambda: term.editor.lines[term.editor.row].endswith("bt"), "the other keyboard types on")
    usb = here.plug(USB_KEYBOARD, "/dev/input/event7")
    see(lambda: s.notes.has("input from this computer: Logitech USB Keyboard"), "plugged in again, it is taken back", timeout=6.0)
    usb.type("usb")
    see(lambda: term.editor.lines[term.editor.row].endswith("btusb"), "and types again")

    usb.tap(EVDEV_NAMED["F2"])
    until(lambda: term.focus == "library", "the library")
    usb.tap(EVDEV_NAMED["Down"])
    usb.tap(EVDEV_NAMED["Enter"])
    until(lambda: cases[1]["title"] in b.text(0), "the picked program")
    usb.tap(EVDEV_NAMED["F5"])
    see(lambda: "SILICON == VM" in s.bar(), "this computer's keyboard picks a program and runs it on the fabric")
    term.quit = True
    s.end()


def fault_session(t, cases):
    """A fault in handling one key or one library entry costs that key, never the terminal: a library entry whose
    file is gone says so on the screen, an exception is logged, and the next key works."""
    gone = os.path.abspath("gone.lg")
    s = Session(t, cases, "keyboard", extra=[("gone", gone, "f", "1")])
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0) and term.device == "keyboard", "the first screen and the keyboard")
    s.laptop.key("F2")
    for _ in range(len(cases) + FILLERS):
        s.laptop.key("Down")
    s.laptop.key("Enter")
    see(lambda: "cannot read" in s.bar() and "gone.lg" in s.output(), "a library entry whose file is gone says so on the screen")
    check(s.thread.is_alive(), "and the terminal runs on")

    editor_key = term.editor.key
    fired = []

    def broken(k):
        if not fired:
            fired.append(k)
            raise RuntimeError("a fault in the editor")
        return editor_key(k)

    term.editor.key = broken
    b.press(0x1C)
    see(lambda: fired and s.notes.has("RuntimeError: a fault in the editor"), "an exception while handling a key is logged")
    check(s.thread.is_alive(), "and the terminal runs on")
    b.press(0x32)
    see(lambda: "b" in "".join(term.editor.lines), "the next key works")
    s.laptop.key("F2")
    for _ in range(len(cases) + FILLERS):
        s.laptop.key("Up")
    s.laptop.key("Enter")
    until(lambda: cases[0]["title"] in b.text(0), "the first program")
    s.laptop.key("F5")
    see(lambda: "SILICON == VM" in s.bar(), "and a program runs after the faults")
    last = len(cases) + FILLERS
    s.laptop.key("F2")
    s.laptop.key("End")
    see(lambda: term.focus == "library" and term.pick == last, "End in the library goes to its last entry")
    s.laptop.key("Home")
    s.laptop.key("Down")
    s.laptop.key("Enter")
    see(lambda: cases[1]["title"] in b.text(0), "Home goes to its first: Home, Down, Enter loads the second")
    term.quit = True
    s.end()

    check([lt.os_input_choice(None, False, tty) for tty in (False, True)] == ["", None]
          and lt.os_input_choice("Logitech", False, False) == "Logitech" and lt.os_input_choice(None, True, False) is None,
          "with no terminal window the host reads this computer's keyboards unasked (--no-input opts out); in one, on --input")


def history_session(t, cases):
    """Every run on the fabric goes into the run history (`LOGOS_TERMINAL_HISTORY`): after a run the output pane says
    how many cycles the same image took in each of its last runs, up to ten — deterministic when they are all equal,
    the range when they are not; the history outlives the terminal, and `--history` prints it."""
    loop = cases[0]
    every_run = os.environ["LOGOS_TERMINAL_HISTORY"]
    history = os.path.abspath("runs.jsonl")
    if os.path.exists(history):
        os.remove(history)
    os.environ["LOGOS_TERMINAL_HISTORY"] = history
    cycles = int.from_bytes(bytes.fromhex(loop["reply"])[-5:-1], "big")

    def session():
        s = Session(t, cases, "keyboard")
        until(lambda: "Fabric Terminal" in s.board.text(0) and s.term.device == "keyboard", "the first screen and the keyboard")
        return s

    def run(s):
        ended = lambda: sum("run ends" in l for l in s.notes.lines)
        n = ended()
        s.board.press(0x03)
        until(lambda: ended() == n + 1, "the run to end")

    s = session()
    run(s)
    see(lambda: "the first run of this image" in s.output(), "the first run of a program on the fabric says so")
    run(s)
    see(lambda: f"{cycles:,} cycles in each of the last 2 runs: deterministic" in s.output(),
        "the second run says the image took the same cycles both times: deterministic")
    for _ in range(10):
        run(s)
    see(lambda: f"{cycles:,} cycles in each of the last 10 runs: deterministic" in s.output(),
        "after twelve runs the output pane speaks of the last ten")
    s.board.reply_override = loop["reply_one_cycle_more"]
    run(s)
    see(lambda: f"cycles varied over the last 10 runs: {cycles:,} to {cycles + 1:,}" in s.output(),
        "a run that took one cycle more is called out with the range")
    s.board.reply_override = None
    s.term.quit = True
    s.end()
    runs = [json.loads(l) for l in open(history)]
    check(len(runs) == 13 and all(r["title"] == "loop sum" and r["same"] for r in runs)
          and [r["cycles"] for r in runs] == [cycles] * 12 + [cycles + 1],
          "the history file holds every run: its program, its cycles, and that it equalled the VM")

    s = session()
    run(s)
    see(lambda: f"cycles varied over the last 10 runs: {cycles:,} to {cycles + 1:,}" in s.output(),
        "the history outlives the terminal: a new one counts the runs before it")
    s.term.quit = True
    s.end()
    report = subprocess.run([sys.executable, os.path.join(REPO, "scripts", "logos-terminal.py"), "--history"],
                            capture_output=True, text=True, env=dict(os.environ))
    line = next((l for l in report.stdout.splitlines() if "loop sum" in l), "")
    check(report.returncode == 0 and f"varied: {cycles:,} to {cycles + 1:,}" in line and line.count(f"{cycles:,}") >= 9,
          f"--history prints each program's last ten cycle counts and whether they agree ({line.strip()!r})")
    os.environ["LOGOS_TERMINAL_HISTORY"] = every_run


def demo_session(t, cases):
    """Demo mode (F3): the terminal runs its library by itself, one program after the next — the list opens on the
    program just run, the highlight steps down one entry, Enter loads it, F5 runs it on the fabric, its output stays
    up, and after the last entry the highlight goes back to the first; a run that does not end is stopped at the
    demo's limit. Any key a person presses takes over; F3 starts the demo again."""
    timing = {name: getattr(lt, name) for name in ("DEMO_STEP_S", "DEMO_SHOW_S", "DEMO_HOLD_S", "DEMO_RUN_LIMIT_S")}
    lt.DEMO_STEP_S, lt.DEMO_SHOW_S, lt.DEMO_HOLD_S, lt.DEMO_RUN_LIMIT_S = 0.05, 0.05, 0.1, 1.0
    s = Session(t, cases, "keyboard")
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0) and term.device == "keyboard", "the first screen and the keyboard")
    n = len(cases) + FILLERS
    picks, sampling = [term.pick], [True]

    def sample():
        while sampling[0]:
            if term.pick != picks[-1]:
                picks.append(term.pick)
            time.sleep(0.001)

    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    loaded = lambda: [l.split(" load ", 1)[1] for l in s.notes.lines if " load " in l]
    s.laptop.key("F3")
    see(lambda: "DEMO" in s.bar(), "F3 starts demo mode, and the output bar says so")
    until(lambda: len(loaded()) >= n + 2, "demo mode to go round the whole library", timeout=60)
    sampling[0] = False
    sampler.join()
    loads = loaded()
    titles = [c["title"] for c in cases] + [f"more {k}" for k in range(FILLERS)]
    check(loads[:n + 2] == titles + titles[:2],
          f"demo mode loads the library in order, one entry after the next, and starts again at the top ({loads[:n + 2]})")
    steps = [b2 - a2 for a2, b2 in zip(picks, picks[1:])]
    check(all(d == 1 for d in steps if d != -(n - 1)) and steps.count(-(n - 1)) >= 1,
          f"the highlight steps down one entry at a time and goes back to the top once after the last ({picks})")
    ran = sum("run starts" in l for l in s.notes.lines)
    check(ran >= n and any("run stopped: the demo's time limit" in l for l in s.notes.lines),
          "each program it loads is run on the fabric, and a run that does not end is stopped at the demo's limit")
    s.laptop.key("x")
    see(lambda: term.demo is None and "DEMO" not in s.bar(), "a key a person presses takes over from demo mode")
    settled = len(loaded())
    time.sleep(0.5)
    check(len(loaded()) == settled, "and the demo loads nothing more")
    s.laptop.key("F3")
    see(lambda: len(loaded()) > settled, "F3 starts it again")
    s.laptop.key("F3")
    see(lambda: term.demo is None, "and F3 stops it")
    term.quit = True
    s.end()
    for name, value in timing.items():
        setattr(lt, name, value)


def fabric_session(t, cases):
    """The fabric while a program runs: the editor keeps the program; the side list turns into the program's blocks,
    stacked in the small font (the image's small-text window over the list, two small rows to a text row) — each
    one's number and role, its share of the engine's clocks in the board's last ACTIVITY frame, what it does, where
    it goes next and its cells — the engine's own block drawn bright; F2 brings the program list back and closes the
    window. F4 opens the fabric view over the editor (a box per block, its cells along its foot, the engine's cell
    blinking) and closes it. A reply whose two frame numbers differ spans a frame's turnover and is not drawn. The
    panel's label row names each block's role from the program's fabric map."""
    loop, forever = cases[0], cases[4]
    m = loop["map"]["blocks"]
    tile = t["panel"]["block_tile"]
    s = Session(t, cases, "keyboard")
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0) and term.device == "keyboard", "the first screen and the keyboard")
    see(lambda: all(f"{i}{blk['role'][:tile - 2]}" in b.text(t["panel"]["label_row"]) for i, blk in enumerate(m)),
        "the panel's label row names each block's role from the program's fabric map")
    check(b.window[2] == 0, "no small-text window while the side list shows the programs")
    hot, test = fabric_activity(b, m)
    run_forever(s, cases, forever)
    first_line = open(forever["source"]).read().splitlines()[0]
    side = lambda: "\n".join(line[lt.SIDE_COL:] for r in range(lt.EDIT_ROWS) for line in b.shown(lt.EDIT_TOP + r).split("\n"))
    see(lambda: "BLOCKS" in side(), "a run turns the side list into the program's blocks")
    check(first_line in b.text(lt.EDIT_TOP), "and the editor keeps the program")
    width = t["screen"]["cols"] - lt.SIDE_COL - 1
    in_window = (lt.EDIT_TOP, lt.SIDE_COL + 1, lt.LIBRARY_ROWS + 1, width)
    see(lambda: b.window == in_window,
        "the blocks are drawn in the small-text window, over the side list and its head and nothing else")
    check(f"BLOCKS {len(m)}/{t['panel']['blocks']} CELLS {sum(len(x['cells']) for x in m)}/{t['panel']['cells']}"
          in b.small_text(0), "its head row counts the program's blocks and cells against the engine's")
    see(lambda: all(f"{i} {blk['role'].upper()}" in side() for i, blk in enumerate(m)), "the blocks stacked, each named by its role")
    check(all(f"{i} {blk['role'].upper()}" not in "\n".join(b.text(lt.EDIT_TOP + r) for r in range(lt.EDIT_ROWS))
              for i, blk in enumerate(m)), "in the small plane: the screen's own rows under the window hold none of them")
    check(sum(f"{i} {blk['role'].upper()}" in b.small_text(r) for r in range(2 * lt.LIBRARY_ROWS + 2) for i, blk in enumerate(m))
          == len(m), "each block's title is on a small-plane row of its own")
    see(lambda: lt.compact(m[hot]["says"][0]) in side(), "each block says what it does, from the fabric map")
    see(lambda: all(line in side() for line in term.next_lines(m[hot]["next"], width - 2) + term.next_lines(m[test]["next"], width - 2)),
        "and where it goes next: the loop back to its test, the test's branch")
    check(term.next_lines(m[test]["next"], width - 2) == [f"{m[test]['next']['cond']} ▶{m[test]['next']['then']} else ▶{m[test]['next']['otherwise']}"]
          and term.next_text(m[test]["next"]).startswith("if "), "a branch too long for a row drops its if")
    check(term.next_lines({"kind": "branch", "cond": "(n % d) != 0", "then": 4, "otherwise": 5}, width - 2)
          == ["if (n % d) != 0 ▶4", "else ▶5"], "and still too long splits at its else")
    check(lt.pack(['show "hi fabric"', "r0 = 0", "total = 0", "k = 1"], width - 2, "; ")
          == ['show "hi fabric"', "r0 = 0; total = 0", "k = 1"]
          and lt.pack(["x = a very long statement that no line holds"], width - 2, "; ")
          == ["x = a very long", "statement that no", "line holds"],
          "statements wrap whole, as many to a line as fit; only one too long for a line breaks, at its spaces")
    see(lambda: "cells + 37%  + 37%" in side() and "cells <= 25%" in side(),
        "and its cells, each with its share of the frame's engine clocks")
    small_blinking = lambda: "".join(tl.char_of(b.mem[b.addr(b.small_first + r, c)] & 0x7F, t)
                                     for r in range(2 * lt.LIBRARY_ROWS + 2) for c in range(width)
                                     if b.mem[b.addr(b.small_first + r, c)] & 0x80)
    op = m[hot]["cells"][0]
    see(lambda: small_blinking().strip() == lt.CELL_LABEL.get(op, op)[:3].strip(),
        "each block's cells along its foot, the cell the engine is on blinking, and only it")
    see(lambda: " 75%" in side() and " 25%" in side(), "each block shows its share of the frame's engine clocks")

    def attr_at(text, cols):
        for r in range(lt.EDIT_TOP, lt.EDIT_TOP + lt.EDIT_ROWS):
            for line in b.shown_cells(r):
                c = "".join(tl.char_of(x & 0x7F, t) for x in line).find(text, cols[0], cols[1])
                if c >= 0:
                    return line[c] >> 8
        return None
    in_side = (lt.SIDE_COL, t["screen"]["cols"])
    see(lambda: (attr_at(f"{hot} LOOP", in_side) or 0) >> 4 == lt.WHITE, "the block the engine is in is drawn bright")
    torn = dict(b.activity, frame=8, frame_again=9, block_clocks=[100] + [0] * (len(m) - 1))
    b.activity = torn
    time.sleep(0.6)
    check(" 75%" in side() and "100%" not in side(), "a reply that spans two frames is not drawn")
    b.activity = dict(torn, frame=10, frame_again=10, block_clocks=[0] * len(m))
    b.activity["block_clocks"][hot] = 50
    b.activity["block_clocks"][test] = 50
    see(lambda: " 50%" in side() and " 75%" not in side(), "the next whole frame is drawn")

    s.laptop.key("F1")
    see(lambda: "THE FABRIC TERMINAL" in b.text(5) and b.window[2] == 0, "F1's help closes the window it would lie under")
    s.laptop.key("Home")
    see(lambda: b.window[2] != 0 and "THE FABRIC TERMINAL" not in b.text(5), "a key closes the help and the blocks come back")

    s.laptop.key("F4")
    view = lambda: "\n".join(b.text(lt.EDIT_TOP + r)[:lt.EDIT_COLS] for r in range(lt.EDIT_ROWS))
    see(lambda: "THE FABRIC" in b.text(lt.EDIT_TOP), "F4 opens the fabric view over the editor")
    see(lambda: all(f" {i} {blk['role'].upper()} " in view() for i, blk in enumerate(m)), "a box per block, named by its role")
    in_view = (0, lt.EDIT_COLS)
    see(lambda: (attr_at(f" {hot} LOOP ", in_view) or 0) >> 4 == lt.WHITE, "the box of the block the engine is in is drawn bright")
    blinking = lambda: [b.text(r)[c] for r in range(lt.EDIT_TOP, lt.EDIT_TOP + lt.EDIT_ROWS) for c in range(lt.EDIT_COLS)
                        if b.mem[b.addr(r, c)] & 0x80]
    see(lambda: "".join(blinking()).strip() == lt.CELL_LABEL.get(op, op)[:3].strip(), "the cell the engine is on blinks, and only it")
    foot = lambda: b.text(lt.EDIT_TOP + lt.EDIT_ROWS - 1)
    see(lambda: f"LIVE · block {hot} loop · cell {m[hot]['first_cell']} {op}" in foot() and "1,234 cycles" in b.text(lt.EDIT_TOP),
        "the foot says where the engine is, live, and the head its cycle count")
    s.laptop.key("F4")
    see(lambda: first_line in b.text(lt.EDIT_TOP), "F4 closes it: the program again")
    s.laptop.key("F4")
    see(lambda: "THE FABRIC" in b.text(lt.EDIT_TOP), "F4 opens it again")
    s.laptop.key("Home")
    see(lambda: first_line in b.text(lt.EDIT_TOP), "an editing key goes back to the program")
    s.laptop.key("F2")
    see(lambda: "PROGRAMS" in b.text(lt.EDIT_TOP)[lt.SIDE_COL:], "F2 brings the program list back")
    see(lambda: b.window[2] == 0 and forever["title"] in "\n".join(b.shown(lt.EDIT_TOP + 1 + r) for r in range(lt.LIBRARY_ROWS)),
        "and closes the small-text window: the list is seen again")
    s.laptop.key("Esc")
    s.laptop.key("Esc")
    until(lambda: "stopped" in s.bar(), "Esc to stop the run")

    titles = lambda: sum(f"{i} {blk['role'].upper()}" in b.small_text(r) for r in range(2 * lt.LIBRARY_ROWS + 2)
                         for i, blk in enumerate(m))
    s.laptop.key("F5")
    see(lambda: b.window == in_window and titles() == len(m), "a run again: the blocks again, in the small font")
    b.power_off(port_vanishes=False)
    b.power_on()
    see(lambda: s.notes.has("the board restarted") and b.window == in_window and titles() == len(m),
        "the board restarted mid-run: the window opens again and every block is drawn in it again")
    b.power_off(port_vanishes=False)
    b.features = t["features"]["engine"]
    b.pong_latency = 0.2
    b.power_on()
    restarts = lambda: sum("the board restarted" in l for l in s.notes.lines)
    until(lambda: restarts() == 2, "the board restarted again, with an image without the small plane")
    noticed = b.first_read
    side = lambda: "\n".join(b.text(lt.EDIT_TOP + r)[lt.SIDE_COL:] for r in range(lt.EDIT_ROWS))
    see(lambda: all(f"{i} {blk['role'].upper()}" in side() for i, blk in enumerate(m)),
        "an image without the small plane, after a restart: the blocks drawn in the screen's own font")
    time.sleep(0.5)
    check(noticed is not None and not [w for at, w in b.unasked if at >= noticed],
          f"and from the first read that could tell the host so, it is sent no WINDOW frame and no small-plane cell: "
          f"{b.unasked[-3:]}")
    term.quit = True
    s.end()

    s = Session(t, cases, "keyboard", small=False)
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0) and term.device == "keyboard", "an image without the small plane")
    fabric_activity(b, m)
    run_forever(s, cases, forever)
    side = lambda: "\n".join(b.text(lt.EDIT_TOP + r)[lt.SIDE_COL:] for r in range(lt.EDIT_ROWS))
    see(lambda: all(f"{i} {blk['role'].upper()}" in side() for i, blk in enumerate(m)) and " 75%" in side(),
        "on an image without the small plane the blocks are drawn in the screen's own font")
    check(b.window == (0, 0, 0, 0) and not b.unasked, f"and it is sent no WINDOW frame and no small-plane cell: {b.unasked[:3]}")
    s.laptop.key("Esc")
    s.laptop.key("Esc")
    until(lambda: "stopped" in s.bar(), "Esc to stop the run")
    term.quit = True
    s.end()


def fabric_activity(b, m):
    """A frame of the engine looping: the loop block 75% of its clocks, its test 25%, the engine in the loop's first
    cell; the loop's and test's indices."""
    hot = next(i for i, blk in enumerate(m) if blk["role"] == "loop")
    test = next(i for i, blk in enumerate(m) if blk["role"] == "test")
    clocks = [0] * len(m)
    clocks[hot], clocks[test] = 75, 25
    cells = [0] * 32
    cells[m[test]["first_cell"]] = 25
    for k in range(len(m[hot]["cells"])):
        cells[m[hot]["first_cell"] + k] = 75 // len(m[hot]["cells"])
    b.activity = {"frame": 7, "running": 1, "block": hot, "cell": m[hot]["first_cell"], "depth": 0, "heap": 0,
                  "cycles": 1234, "block_clocks": clocks, "cell_clocks": cells, "frame_again": 7}
    return hot, test


def run_forever(s, cases, forever):
    """Load the never-ending program from the side list and run it."""
    s.laptop.key("F2")
    for _ in range(cases.index(forever)):
        s.laptop.key("Down")
    s.laptop.key("Enter")
    until(lambda: forever["title"] in s.board.text(0), "the never-ending program")
    s.laptop.key("F5")


def totals_session(t, cases):
    """A run's end keeps its shares on screen. While it runs, its blocks show each frame's shares, headed LIVE;
    once it ends, the host asks the board for the run's totals and, when they add up to its cycles and one, the
    blocks show each one's share of every clock of the run, headed WHOLE RUN and its cycles, and they stay — in the
    fabric view too, and in demo mode through its pause. Totals that do not add up are not shown, and the log says
    so: the run's last frame stays instead, headed LAST FRAME OF THE RUN, as it does on an image without totals,
    which is never asked for them."""
    loop = cases[0]
    m = loop["map"]["blocks"]
    tot = loop["totals"]
    clocks = sum(tot["block_totals"])
    width = t["screen"]["cols"] - lt.SIDE_COL - 1

    def side(b):
        return "\n".join(line[lt.SIDE_COL:] for r in range(lt.EDIT_ROWS) for line in b.shown(lt.EDIT_TOP + r).split("\n"))

    def title(b, i):
        role = m[i]["role"].upper()
        return next((x for x in side(b).split("\n") if x[2:].startswith(f"{i} {role}")), "")

    s = Session(t, cases, "keyboard")
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0) and term.device == "keyboard", "the first screen and the keyboard")
    b.run_time = 0.6
    fabric_activity(b, m)
    s.laptop.key("F5")
    see(lambda: "LIVE · share per frame" in side(b) and " 75%" in side(b), "while a run goes, each frame's shares, headed LIVE")
    see(lambda: f"WHOLE RUN {clocks - 1:,} cycles" in side(b), "once it ends, the whole run's shares, headed WHOLE RUN and its cycles")
    want = [f"{c * 100 / clocks:3.0f}%" for c in tot["block_totals"][: len(m)]]
    see(lambda: all(title(b, i).rstrip("┐").endswith(want[i]) for i in range(len(m))),
        f"each block's share of every clock of the run, from the board's totals ({want})")
    check(s.notes.has(f"run totals: {clocks:,} engine clocks = {clocks - 1:,} cycles + 1, by block: "),
          "the log has the run's totals, block by block")
    time.sleep(1.0)
    check(f"WHOLE RUN {clocks - 1:,} cycles" in side(b) and all(title(b, i).rstrip("┐").endswith(want[i]) for i in range(len(m))),
          "and they stay: nothing tears them away")
    s.laptop.key("F4")
    see(lambda: "WHOLE RUN" in b.text(lt.EDIT_TOP + lt.EDIT_ROWS - 1), "the fabric view's foot says WHOLE RUN too")
    s.laptop.key("F4")

    b.totals_skew = 1
    fabric_activity(b, m)
    s.laptop.key("F5")
    see(lambda: "LIVE · share per frame" in side(b), "a run again")
    see(lambda: s.notes.has(f"the board's run totals add up to {clocks + 1:,} by block and {clocks:,} by cell for a run it "
                            f"names {clocks - 1:,} cycles, not the run's"),
        "totals that do not add up to the run's cycles and one are not shown, and the log says so",
        timeout=lt.TOTALS_WAIT_S + 5)
    check("WHOLE RUN" not in side(b) and "LAST FRAME OF THE RUN" in side(b) and " 75%" in side(b),
          "the run's last frame stays instead, headed LAST FRAME OF THE RUN")
    b.totals_skew = 0

    timing = {name: getattr(lt, name) for name in ("DEMO_STEP_S", "DEMO_SHOW_S", "DEMO_HOLD_S")}
    lt.DEMO_STEP_S, lt.DEMO_SHOW_S, lt.DEMO_HOLD_S = 0.05, 0.05, 1.5
    fabric_activity(b, m)
    s.laptop.key("F3")
    see(lambda: f"WHOLE RUN {clocks - 1:,} cycles" in side(b), "demo mode runs it, and at its end the whole run's shares",
        timeout=15)
    time.sleep(0.6)
    check(f"WHOLE RUN {clocks - 1:,} cycles" in side(b) and term.demo["step"] == "hold",
          "stay up through demo mode's pause before the next program")
    s.laptop.key("x")
    for name, value in timing.items():
        setattr(lt, name, value)
    see(lambda: term.demo is None, "a key takes over from demo mode")

    hello = next(c for c in cases if c["title"] == "hello fabric")
    hm = hello["map"]["blocks"]
    inner = width - 2
    check(len(hm) > (2 * (lt.LIBRARY_ROWS + 1) - 2) // 3 and all(len(lt.compact(x["says"][0])) <= inner for x in hm if x["says"]),
          f"hello fabric has more blocks ({len(hm)}) than the side list has rows for each to say all it does, "
          f"and each first statement fits a row")
    s.laptop.key("F2")
    for _ in range(abs(cases.index(hello) - term.pick)):
        s.laptop.key("Down" if cases.index(hello) > term.pick else "Up")
    s.laptop.key("Enter")
    until(lambda: hello["title"] in b.text(0), "hello fabric loaded")
    b.run_time = 1.5
    f = b.activity["frame"] + 1
    b.activity = dict(b.activity, running=1, frame=f, frame_again=f,
                      block_clocks=[10] * len(hm) + [0] * (len(b.activity["block_clocks"]) - len(hm)))
    s.laptop.key("F5")

    def first_body(i):
        rows = [b.small_text(r) for r in range(2 * lt.LIBRARY_ROWS + 2)]
        at = next((r for r, x in enumerate(rows) if x[1:].startswith(f"{i} {hm[i]['role'].upper()}")), None)
        return rows[at + 1][1:inner + 1] if at is not None and not rows[at + 1].startswith("└") else ""
    see(lambda: all(lt.compact(x["says"][0]) in first_body(i) for i, x in enumerate(hm) if x["says"])
        and all(("▶" in first_body(i) or "return" in first_body(i)) for i, x in enumerate(hm)
                if not x["says"] and x["role"] != "idle"),
        "crowded, every box says the first of what it does, whole — or, doing nothing, where it goes next")
    hello_cycles = sum(hello["totals"]["block_totals"]) - 1
    see(lambda: f"WHOLE RUN {hello_cycles:,} cycles" in side(b), "and at its end its whole run's shares")
    term.quit = True
    s.end()

    s = Session(t, cases, "keyboard", totals=False)
    b, term = s.board, s.term
    until(lambda: "Fabric Terminal" in b.text(0) and term.device == "keyboard", "an image without totals")
    b.run_time = 0.6
    fabric_activity(b, m)
    s.laptop.key("F5")
    see(lambda: "LIVE · share per frame" in side(b), "a run on it")
    see(lambda: "LAST FRAME OF THE RUN" in side(b) and " 75%" in side(b), "at its end the run's last frame stays, headed so")
    time.sleep(lt.TOTALS_WAIT_S / 2)
    check(b.totals_asked == 0 and "LAST FRAME OF THE RUN" in side(b), "and the image is never asked for totals")
    term.quit = True
    s.end()


def model_totals(case, chip, t):
    """The totals a board reports once `case` has run, standing in for the engine whose own the lock's simulation
    counts: every clock of its cycles and one, over its fabric map's blocks by role (most in its loops, then its
    tests, none in its idle slot) and in each block on its first cell. None for a case that never ends."""
    if case["reply"] is None or not case.get("map"):
        return None
    _result, cycles, _status, _shows = chip.decode_wpe(chip.protocol(False), bytes.fromhex(case["reply"]))
    m = case["map"]["blocks"]
    weight = {"loop": 6, "test": 3, "step": 2, "call": 2, "fn": 2, "idle": 0}
    w = [weight.get(b["role"], 1) for b in m]
    clocks = cycles + 1
    blocks = [clocks * x // sum(w) for x in w]
    blocks[w.index(max(w))] += clocks - sum(blocks)
    n_blocks, n_cells = (f["count"] for f in t["totals"] if f["name"] != "cycles")
    cells = [0] * n_cells
    for b, c in zip(m, blocks):
        cells[min(b["first_cell"], n_cells - 1)] += c
    return {"cycles": cycles, "block_totals": blocks + [0] * (n_blocks - len(blocks)), "cell_totals": cells}


def check_sessions(t, cases):
    loop, shows, wrong = cases[0], cases[1], cases[2]
    entry = lambda c: (c["title"], c["source"], c["fn"], c["args"])
    for what, entries, stuck, depth, skew, holds, says in [
        ("a sound board", [loop, shows], None, 8192, (0, 0), True, "ok  and writing it left the screen's cells as they were"),
        ("a sound board", [loop, shows], None, 8192, (0, 0), True,
         f"ok  loop sum: the board's run totals add up to its {loop['totals']['cycles']:,} cycles + 1"),
        ("a text buffer with a stuck bit", [loop], 6, 8192, (0, 0), False, "BAD every cell of the screen reads back"),
        ("a text buffer a row bit short", [loop], None, 4096, (0, 0), False, "BAD and writing it left the screen's cells as they were (rows changed: [0,"),
        ("a chip that disagrees with the VM", [wrong], None, 8192, (0, 0), False, "BAD wrong: SILICON != VM"),
        ("a board whose totals miss a clock", [loop], None, 8192, (-1, 0), False, "BAD loop sum: the board's run totals add up to"),
        ("a board whose totals name another run", [loop], None, 8192, (0, 1), False,
         f"BAD loop sum: the board's run totals add up to its {loop['totals']['cycles']:,} cycles + 1, by block and by cell "
         f"(a run it names {loop['totals']['cycles'] + 1:,} cycles"),
    ]:
        board = ModelBoard(t, None, cases)
        board.stuck_glyph_bit = stuck
        board.depth = depth
        board.totals_skew, board.totals_cycles_skew = skew
        link = tl.Link(t, ser=board)
        printed = io.StringIO()
        with contextlib.redirect_stdout(printed):
            got = lt.check(t, chip_for(cases), link, [entry(c) for c in entries], wait=5.0)
        sys.stdout.write(printed.getvalue())
        link.close()
        check(got == holds and says in printed.getvalue(), f"--check on {what} says {'OK' if holds else 'FAILED'}: {says}")
        check("PS/2 line since the image started: no clock edge yet" in printed.getvalue(),
              f"--check on {what} reports the board's PS/2 line")

    board = ModelBoard(t, None, cases)
    board.features = t["features"]["small"]
    link = tl.Link(t, ser=board)
    got = lt.check(t, chip_for(cases), link, [entry(loop)], wait=5.0)
    link.close()
    check(got and not board.engine_frames, "--check on the shell image holds and runs no program")

    s = Session(t, cases, "keyboard", engine=False)
    until(lambda: "Fabric Terminal" in s.board.text(0), "the shell's first screen")
    s.laptop.key("F5")
    see(lambda: "no fabric engine" in s.bar(), "F5 on the shell image says it has no engine")
    check(not s.board.engine_frames, "and sends the engine nothing")
    s.term.quit = True
    s.end()


def main():
    # Every run of every session goes to a history of this test's own, never to the repo's logs/.
    every_run = os.path.abspath("every_run.jsonl")
    if os.path.exists(every_run):
        os.remove(every_run)
    os.environ["LOGOS_TERMINAL_HISTORY"] = every_run
    t = tl.table()
    cases = json.load(open(os.environ["E2E_CASES"]))
    chip = chip_for(cases)
    for c in cases:
        c["wire_bytes"] = open(c["wire"], "rb").read()
        c["totals"] = model_totals(c, chip, t)
    keyboard_session(t, cases)
    mouse_session(t, cases)
    device_sessions(t, cases)
    restart_session(t, cases)
    keyboard_sessions(t, cases)
    fault_session(t, cases)
    history_session(t, cases)
    demo_session(t, cases)
    fabric_session(t, cases)
    totals_session(t, cases)
    check_sessions(t, cases)
    print("host_e2e: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
