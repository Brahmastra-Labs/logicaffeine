#!/usr/bin/env python3
"""terminal_link — the host side of the Fabric Terminal's link (`codegen_sva::terminal_protocol`): SLIP frames over
the board's UART, the board's reports as events, the screen's text cells, and a program run on the fabric engine
with the board's ENGINE_TAKEN flow control.

The protocol table is read, never restated: from `fabric_terminal --table` in the repo, or from the `terminal.json`
a board kit carries. Board B is found by its FTDI serial (`ftdi_ports.py`).

    python3 scripts/lib/terminal_link.py ping        the board answers, the screen it draws, and its PS/2 line
    python3 scripts/lib/terminal_link.py ps2-line    the PS/2 line's counts as they change: what the board's USB
                                                     bridge sends while someone types on its keyboard
    python3 scripts/lib/terminal_link.py keys        print every PS/2 byte the board reports (Ctrl-C ends)
    python3 scripts/lib/terminal_link.py selftest    SLIP and frame building against the table, no board
"""
import json
import os
import queue
import subprocess
import sys
import threading
import time

LIB = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(LIB))
sys.path.insert(0, LIB)
import ftdi_ports  # noqa: E402

def glyph_slots(t):
    """The font slot of each character the font holds outside ASCII, by the table's `ui_glyphs` (slot = position)
    and `del_glyph` (slot 0x7F)."""
    slots = {ch: i for i, ch in enumerate(t["screen"]["ui_glyphs"])}
    slots[t["screen"]["del_glyph"]] = 0x7F
    return slots


def glyph(ch, slots):
    """The font slot that draws `ch`: ASCII as itself, the UI characters by `slots` (`glyph_slots`), anything else
    as '?'."""
    o = ord(ch)
    if 0x20 <= o <= 0x7E:
        return o
    return slots.get(ch, ord("?"))


def char_of(g, t):
    """The character font slot `g` draws."""
    ui = t["screen"]["ui_glyphs"]
    return ui[g] if g < len(ui) else (t["screen"]["del_glyph"] if g == 0x7F else chr(g))


def table(path=None):
    """The protocol table: `path`, the file `LOGOS_TERMINAL_TABLE` names, the table a board kit carries (its
    `kit.json` names it), or `fabric_terminal --table`."""
    kit_doc = os.path.join(REPO, "kit.json")
    kit = os.path.join(REPO, json.load(open(kit_doc)).get("terminal", "")) if os.path.exists(kit_doc) else None
    for p in (path, os.environ.get("LOGOS_TERMINAL_TABLE"), kit):
        if p and os.path.isfile(p):
            return json.load(open(p))
    r = subprocess.run(["cargo", "run", "-q", "-p", "logicaffeine-compile", "--example", "fabric_terminal", "--", "--table"],
                       cwd=REPO, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"fabric_terminal --table failed:\n{r.stderr}")
    return json.loads(r.stdout)


LIBRARY = os.path.join(REPO, "benchmarks", "silicon", "programs", "terminal_library.txt")


def library(path=None):
    """The Fabric Terminal's program library: (title, program path relative to the repo, function or None,
    arguments) for each line of `benchmarks/silicon/programs/terminal_library.txt` — or of the file
    `LOGOS_TERMINAL_LIBRARY` names."""
    path = path or os.environ.get("LOGOS_TERMINAL_LIBRARY") or LIBRARY
    entries = []
    for line in open(path):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        title, program, fn, args = (field.strip() for field in line.split("|"))
        source = os.path.relpath(os.path.join(os.path.dirname(os.path.abspath(path)), program), REPO)
        entries.append((title, source, fn or None, args))
    return entries


class Slip:
    """SLIP framing by the table's bytes: `encode` a frame, `push` received bytes and get whole frames back."""

    def __init__(self, t):
        s = t["slip"]
        self.end, self.esc, self.esc_end, self.esc_esc = s["end"], s["esc"], s["esc_end"], s["esc_esc"]
        self.frame, self.escaped, self.damaged = bytearray(), False, False

    def encode(self, frame):
        out = bytearray()
        for b in frame:
            if b == self.end:
                out += bytes([self.esc, self.esc_end])
            elif b == self.esc:
                out += bytes([self.esc, self.esc_esc])
            else:
                out.append(b)
        out.append(self.end)
        return bytes(out)

    def push(self, data):
        frames = []
        for b in data:
            if b == self.end:
                if self.frame and not self.damaged:
                    frames.append(bytes(self.frame))
                self.frame, self.escaped, self.damaged = bytearray(), False, False
            elif self.escaped:
                self.escaped = False
                if b == self.esc_end:
                    self.frame.append(self.end)
                elif b == self.esc_esc:
                    self.frame.append(self.esc)
                else:
                    self.damaged = True
            elif b == self.esc:
                self.escaped = True
            else:
                self.frame.append(b)
        return frames


class Cell:
    """One text cell: glyph slot, colours from the 16-colour palette, blink."""

    __slots__ = ("glyph", "fg", "bg", "blink")

    def __init__(self, glyph=0x20, fg=7, bg=0, blink=False):
        self.glyph, self.fg, self.bg, self.blink = glyph & 0x7F, fg & 15, bg & 15, blink

    def bytes(self):
        return bytes([(self.bg << 4) | self.fg, (0x80 if self.blink else 0) | self.glyph])

    def __eq__(self, other):
        return (self.glyph, self.fg, self.bg, self.blink) == (other.glyph, other.fg, other.bg, other.blink)


class Screen:
    """The host's copy of the board's text buffer — the screen's cells and the small plane's — and of its cursor,
    pointer and small-text window. Drawing changes the copy; `frames()` returns the frames that bring the board up to
    date: a PUT per run of changed cells in a row (the small plane's rows at their buffer rows), and the cursor,
    pointer and window when they changed."""

    def __init__(self, t):
        self.t = t
        self.cols, self.rows = t["screen"]["cols"], t["screen"]["rows"]
        sc = t["screen"]
        self.small_first, self.small_rows, self.small_cols = sc.get("small_first_row", 0), sc.get("small_rows", 0), sc.get("small_cols", 0)
        self.slots = glyph_slots(t)
        self.want = [[Cell() for _ in range(self.cols)] for _ in range(self.rows)]
        self.have = [[None] * self.cols for _ in range(self.rows)]
        self.small_want = [[Cell() for _ in range(self.small_cols)] for _ in range(self.small_rows)]
        self.small_have = [[None] * self.small_cols for _ in range(self.small_rows)]
        self.cursor = self.pointer = self.window = None
        self.sent_cursor = self.sent_pointer = self.sent_window = False
        self.small_enabled = False

    def put(self, row, col, text, fg=7, bg=0, blink=False):
        for i, ch in enumerate(text):
            c = col + i
            if 0 <= row < self.rows and 0 <= c < self.cols:
                self.want[row][c] = Cell(glyph(ch, self.slots), fg, bg, blink)

    def fill(self, row, col, h, w, ch=" ", fg=7, bg=0):
        for r in range(row, min(row + h, self.rows)):
            self.put(r, col, ch * max(0, min(w, self.cols - col)), fg, bg)

    def put_small(self, srow, scol, text, fg=7, bg=0, blink=False):
        """Write `text` into the small plane at its row `srow`, column `scol`."""
        for i, ch in enumerate(text):
            c = scol + i
            if 0 <= srow < self.small_rows and 0 <= c < self.small_cols:
                self.small_want[srow][c] = Cell(glyph(ch, self.slots), fg, bg, blink)

    def fill_small(self, srow, scol, h, w, ch=" ", fg=7, bg=0):
        for r in range(srow, min(srow + h, self.small_rows)):
            self.put_small(r, scol, ch * max(0, min(w, self.small_cols - scol)), fg, bg)

    def set_window(self, window):
        """Open the small-text window as (row, col, rows, cols) in text cells, or close it (None)."""
        if window != self.window:
            self.window, self.sent_window = window, False

    def forget(self):
        """The board no longer shows what was sent (it restarted): the next `frames()` sends every cell, the cursor
        and the pointer again."""
        self.have = [[None] * self.cols for _ in range(self.rows)]
        self.small_have = [[None] * self.small_cols for _ in range(self.small_rows)]
        self.sent_cursor = self.sent_pointer = self.sent_window = False

    def set_cursor(self, pos):
        if pos != self.cursor:
            self.cursor, self.sent_cursor = pos, False

    def set_pointer(self, pos):
        if pos != self.pointer:
            self.pointer, self.sent_pointer = pos, False

    def frames(self):
        h = self.t["host"]
        out = []
        planes = [(self.want, self.have, 0)]
        if self.small_enabled:
            planes.append((self.small_want, self.small_have, self.small_first))
        for want, have, first in planes:
            for r in range(len(want)):
                c = 0
                while c < len(want[r]):
                    if have[r][c] is not None and have[r][c] == want[r][c]:
                        c += 1
                        continue
                    payload = bytearray([h["put"], first + r, c])
                    while c < len(want[r]) and not (have[r][c] is not None and have[r][c] == want[r][c]):
                        payload += want[r][c].bytes()
                        have[r][c] = Cell(want[r][c].glyph, want[r][c].fg, want[r][c].bg, want[r][c].blink)
                        c += 1
                    out.append(bytes(payload))
        if not self.sent_window and self.small_enabled:
            out.append(bytes([h["window"], *(self.window or (0, 0, 0, 0))]))
            self.sent_window = True
        if not self.sent_cursor:
            r, c = self.cursor or (0, 0)
            out.append(bytes([h["cursor"], r, c, 1 if self.cursor else 0]))
            self.sent_cursor = True
        if not self.sent_pointer:
            r, c = self.pointer or (0, 0)
            out.append(bytes([h["pointer"], r, c, 1 if self.pointer else 0]))
            self.sent_pointer = True
        return out


class Link:
    """The board on its UART: frames out, reports in (a reader thread puts each as (kind, payload) on `events`).
    `ser` stands in for the UART — any object with pyserial's `read(n)` (returning within a short timeout),
    `write(data)` and `close()`, and `reopen()` returning it again once it is back — and `port` is then its `name`.

    A UART that goes away (the board switched off takes its USB serial port with it) sets `lost` to the error, is
    closed at once (so the port, when it comes back, is not renamed around a dead handle) and drops every frame sent
    until `reopen()` finds the board again by its FTDI serial number."""

    def __init__(self, t, port=None, ser=None):
        self.t = t
        if ser is None:
            import serial

            def opener():
                self.port = ftdi_ports.port_arg(port, "B")
                return serial.Serial(self.port, t["baud"], timeout=0.05)

            self.opener = opener
            ser = opener()
        else:
            self.port = getattr(ser, "name", "a stand-in UART")
            self.opener = getattr(ser, "reopen", None)
        self.ser = ser
        self.slip = Slip(t)
        self.events = queue.Queue()
        self.lock = threading.Lock()
        self.alive = True
        self.lost = None
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _lose(self, e):
        if self.lost is None:
            self.lost = e
            try:
                self.ser.close()
            except Exception:
                pass

    def _read(self):
        while self.alive and self.lost is None:
            try:
                data = self.ser.read(256)
            except Exception as e:
                self._lose(e)
                return
            for f in self.slip.push(data):
                self.events.put((f[0], f[1:]))

    def send(self, frame):
        with self.lock:
            if self.lost is not None:
                return
            try:
                self.ser.write(self.slip.encode(frame))
            except Exception as e:
                self._lose(e)

    def reopen(self):
        """Open the UART again after it went away; True once it is back and read again."""
        if self.opener is None:
            return False
        try:
            ser = self.opener()
        except (Exception, SystemExit):
            return False
        self.reader.join(timeout=1)
        with self.lock:
            self.ser, self.slip, self.lost = ser, Slip(self.t), None
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        return True

    def close(self):
        self.alive = False
        self.reader.join(timeout=1)
        try:
            self.ser.close()
        except Exception:
            pass

    def wait(self, kind, timeout, keep=None):
        """The next report of `kind`; other reports go to `keep` (a list) or are dropped. None on timeout."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                k, payload = self.events.get(timeout=max(0.0, deadline - time.time()))
            except queue.Empty:
                break
            if k == kind:
                return payload
            if keep is not None:
                keep.append((k, payload))
        return None

    def ping(self, timeout=1.0):
        """PONG's payload — version, cols, rows, panel rows, features — or None when the board does not answer."""
        self.send(bytes([self.t["host"]["ping"]]))
        return self.wait(self.t["board"]["pong"], timeout)

    def has_engine(self, pong):
        """Whether the image that answered `pong` has the fabric engine."""
        return len(pong) > 4 and bool(pong[4] & self.t["features"]["engine"])

    def has_small(self, pong):
        """Whether the image that answered `pong` has the small-text plane and window."""
        return len(pong) > 4 and bool(pong[4] & self.t["features"].get("small", 0))

    def has_totals(self, pong):
        """Whether the image that answered `pong` keeps the run's totals and answers TOTALS."""
        return len(pong) > 4 and bool(pong[4] & self.t["features"].get("totals", 0))

    def ps2_line(self, pong):
        """The PS/2 line since the image started, from `pong`: {edges, shortest_low_us (None until one), bytes,
        damaged} — or None from an image without the line monitor."""
        return ps2_line(self.t, pong)

    def run_engine(self, image, on_bytes, timeout, keep=None):
        """Reset the engine, stream `image` in frames of at most `engine_frame_max` bytes, each after the board
        reports the last taken, then hand every byte the engine sends to `on_bytes` until it returns True (the
        reply is whole) or `timeout` seconds pass. Returns True when the reply completed."""
        h, b = self.t["host"], self.t["board"]
        self.send(bytes([h["engine_reset"]]))
        time.sleep(0.01)
        step = self.t["engine_frame_max"]
        for i in range(0, len(image), step):
            chunk = image[i:i + step]
            self.send(bytes([h["engine"]]) + chunk)
            got = self.wait(b["engine_taken"], 5.0, keep)
            if got is None or got[0] != len(chunk):
                raise RuntimeError(f"the board did not take engine bytes {i}..{i + len(chunk)} (reported {got})")
        deadline = time.time() + timeout
        while time.time() < deadline:
            got = self.wait(b["engine"], deadline - time.time(), keep)
            if got is None:
                return False
            if on_bytes(got):
                return True
        return False


def ps2_line(t, pong):
    """`Link.ps2_line`, for a table `t`."""
    at = {name: i for i, name in enumerate(t.get("pong", []))}
    if "ps2_damaged" not in at or len(pong) <= at["ps2_damaged"]:
        return None
    low = pong[at["ps2_shortest_low_us"]]
    return {"edges": pong[at["ps2_edges_hi"]] << 8 | pong[at["ps2_edges_lo"]],
            "shortest_low_us": None if low == 255 else low,
            "bytes": pong[at["ps2_bytes_hi"]] << 8 | pong[at["ps2_bytes_lo"]],
            "damaged": pong[at["ps2_damaged"]]}


def fields(layout, payload):
    """A report's payload read by its layout (the table's list of {name, bytes, count}): each field's value, or its
    list of values when it has several — or None when the payload is not the layout's length."""
    if not layout or len(payload) != sum(f["bytes"] * f["count"] for f in layout):
        return None
    out, at = {}, 0
    for f in layout:
        values = []
        for _ in range(f["count"]):
            values.append(int.from_bytes(bytes(payload[at:at + f["bytes"]]), "big"))
            at += f["bytes"]
        out[f["name"]] = values if f["count"] > 1 else values[0]
    return out


def activity(t, payload):
    """A board ACTIVITY report: {frame, running, block, cell, depth, heap, cycles, block_clocks, cell_clocks,
    frame_again} — the fabric engine in one video frame, as its activity panel drew it — or None when the payload
    is not the layout's length."""
    out = fields(t.get("activity", []), payload)
    if out is not None:
        out["running"] = bool(out.get("running"))
    return out


def totals(t, payload):
    """A board TOTALS report: {cycles, block_totals, cell_totals} — the run's cycles as the board's activity panel
    last took them, and the engine clocks of the run since the engine's last reset, by block and by cell — or None
    when the payload is not the layout's length."""
    return fields(t.get("totals", []), payload)


def describe_ps2_line(line):
    """One line of text for `ps2_line`'s counts, and what they mean for the device on the board's USB port."""
    if line is None:
        return "this image has no PS/2 line monitor"
    if line["edges"] == 0:
        return "no clock edge yet: the board's USB bridge has sent nothing (no device, or one it does not serve)"
    low = "none timed" if line["shortest_low_us"] is None else f"{line['shortest_low_us']} us"
    text = f"{line['edges']} clock edges (shortest low phase {low}), {line['bytes']} bytes read whole, {line['damaged']} damaged"
    if line["bytes"] == 0 and line["damaged"] == 0:
        text += ": the bridge clocks but no byte is read"
    return text


def selftest():
    t = table()
    s = Slip(t)
    end, esc = t["slip"]["end"], t["slip"]["esc"]
    frame = bytes([0x01, end, 0x02, esc, 0x03])
    ok = s.push(s.encode(frame) + s.encode(b"\x3f")) == [frame, b"\x3f"]
    print(f"  {'ok ' if ok else 'BAD'} SLIP round trip with END and ESC inside a frame")
    damaged = bytes([0x05, esc, 0x99, end]) + s.encode(b"\x3f")
    ok2 = Slip(t).push(damaged) == [b"\x3f"]
    print(f"  {'ok ' if ok2 else 'BAD'} a frame with a bad escape is dropped, the next one kept")
    scr = Screen(t)
    scr.put(1, 2, "AB")
    first = scr.frames()
    scr.put(1, 3, "C")
    second = scr.frames()
    put = t["host"]["put"]
    ok3 = (len([f for f in first if f[0] == put]) == t["screen"]["rows"]
           and second == [bytes([put, 1, 3, 0x07, ord("C")])])
    print(f"  {'ok ' if ok3 else 'BAD'} the screen sends every row once, then only the cells that changed")
    slots = glyph_slots(t)
    ok4 = (glyph("┌", slots) == 2 and glyph("A", slots) == 0x41 and glyph("€", slots) == ord("?")
           and glyph(t["screen"]["del_glyph"], slots) == 0x7F
           and all(char_of(glyph(ch, slots), t) == ch for ch in t["screen"]["ui_glyphs"] + "Az~"))
    print(f"  {'ok ' if ok4 else 'BAD'} UI characters map to their font slots and back")
    good = ok and ok2 and ok3 and ok4
    if os.path.exists(LIBRARY):
        entries = library(LIBRARY)
        ok5 = bool(entries) and all(os.path.exists(os.path.join(REPO, e[1])) for e in entries)
        print(f"  {'ok ' if ok5 else 'BAD'} the library's {len(entries)} programs are all here")
        good = good and ok5
    print("terminal_link selftest: " + ("OK" if good else "FAILED"))
    return 0 if good else 1


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "ping"
    if cmd == "selftest":
        return selftest()
    t = table()
    link = Link(t, sys.argv[2] if len(sys.argv) > 2 else None)
    try:
        if cmd == "ping":
            p = link.ping()
            if p is None:
                print(f"no answer from {link.port} — is nexys_fabric_terminal.bit (or nexys_terminal_shell.bit) loaded?")
                return 1
            engine = "the fabric engine" if link.has_engine(p) else "no engine (the screen-and-keyboard shell)"
            print(f"Fabric Terminal v{p[0]} on {link.port}: {p[1]} x {p[2]} text, {p[3]} panel rows, {engine}")
            print(f"PS/2 line: {describe_ps2_line(link.ps2_line(p))}")
            return 0
        if cmd == "ps2-line":
            print("the PS/2 line from the board's USB port, twice a second: type on the keyboard plugged into the "
                  "board (Ctrl-C ends)", flush=True)
            last = None
            while True:
                p = link.ping()
                line = None if p is None else link.ps2_line(p)
                if line != last:
                    print(time.strftime("%H:%M:%S"), "no answer" if p is None else describe_ps2_line(line), flush=True)
                    last = line
                time.sleep(0.5)
        if cmd == "keys":
            print("press keys on the board's keyboard (Ctrl-C ends)")
            while True:
                k, payload = link.events.get()
                if k == t["board"]["ps2"]:
                    status = {0: "", t["ps2"]["parity_error"]: " PARITY ERROR", t["ps2"]["framing_error"]: " FRAMING ERROR"}
                    print(f"ps2 {payload[0]:02x}{status.get(payload[1], f' status {payload[1]}')}", flush=True)
    except KeyboardInterrupt:
        return 0
    finally:
        link.close()
    print(f"unknown command {cmd}: ping | ps2-line | keys | selftest")
    return 2


if __name__ == "__main__":
    sys.exit(main())
