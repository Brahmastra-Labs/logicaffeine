#!/usr/bin/env python3
"""logos-terminal — the Fabric Terminal: a LOGOS editor on Board B's VGA screen. Type a program, or pick one from
the library and edit it, then run it on the fabric engine on the same board and watch each `Show` arrive.

    python3 scripts/logos-terminal.py                 Board B with nexys_fabric_terminal.bit loaded
    python3 scripts/logos-terminal.py --flash         load the image first (--bit FILE: another image)
    python3 scripts/logos-terminal.py --headless      the screen in this terminal only, no board (no runs)
    python3 scripts/logos-terminal.py --check         the board with no one at it: the link, every cell of the
                                                      screen read back, every library program SILICON == VM

The board draws the screen and reads the keyboard on its USB port (it arrives as PS/2); this program is the
terminal's host. It holds the editor, compiles each program for the engine on the board, streams the image and
reads the engine's reply. The same screen is mirrored in the terminal this runs in, and that terminal's keyboard
and mouse work too:

    F5 / Ctrl-R  run        Esc  stop a run        F2  library        F1  help        Ctrl-Q  quit
    arrows, Home/End, PgUp/PgDn, Enter, Backspace, Delete, Tab — edit

A program must fit the engine on Board B (8 blocks, 32 cells, 16 registers); one that does not is refused with the
cap it exceeds, as `logos-chip.py compile` reports it. The output pane ends `SILICON == VM` when every line the chip
printed equals the VM's.
"""
import argparse
import errno
import hashlib
import importlib.util
import json
import os
import select
import struct
import sys
import tempfile
import textwrap
import time
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, "lib"))
import terminal_link as tl  # noqa: E402


def logos_chip():
    """`scripts/logos-chip.py` as a module: the compiler front door, the engine-reply decoder and the kit lookup."""
    spec = importlib.util.spec_from_file_location("logos_chip", os.path.join(HERE, "logos-chip.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# ── colours (indices into the board's 16-colour palette) ───────────────────────────────────────────────────────
BLACK, BLUE, GREEN, CYAN, RED, MAGENTA, BROWN, GREY = range(8)
DGREY, LBLUE, LGREEN, LCYAN, LRED, LMAGENTA, YELLOW, WHITE = range(8, 16)

# ── the layout, in text cells ───────────────────────────────────────────────────────────────────────────────────
EDIT_TOP, EDIT_ROWS, EDIT_COLS = 1, 18, 56          # rows 1..18, cols 0..55 (a 4-column gutter of line numbers)
SIDE_COL = 56                                         # the library and keys, cols 56..79
OUT_BAR, OUT_TOP, OUT_ROWS = 19, 20, 6                # the output bar, then rows 20..25
GUTTER = 4
LIBRARY_ROWS = EDIT_ROWS - 8                          # library entries in the side list; the keys fill its foot
# The side list's foot: each key, what it does, and whether a click on its row does it too (a mouse on the board's
# USB port drives the whole demo; quitting stays a key, so no stray click ends it).
KEYS = [("F5", "run", True), ("Esc", "stop", True), ("F2", "programs", True), ("F3", "demo", True),
        ("F4", "fabric view", True), ("F1", "help", True), ("C-q", "quit", False)]
ANSWER_S = 0.25                                       # how long the device on the board's USB port has to answer
WATCH_S = 1.0                                         # how often the host looks for a board that restarted
# Demo mode: how long the list shows each step of its highlight, a loaded program's source shows before it runs,
# and a run's output stays up; a run still going after DEMO_RUN_LIMIT_S is stopped.
DEMO_STEP_S, DEMO_SHOW_S, DEMO_HOLD_S, DEMO_RUN_LIMIT_S = 0.8, 3.0, 6.0, 120.0
ACTIVITY_S = 0.08                                     # how often the fabric view asks the board for a frame
TOTALS_WAIT_S = 2.0                                   # how long after a run the host asks for its totals
PROBE_S = 3.0                                         # how often the host asks a silent USB port again
PS2_BYTE_EDGES = 11                                   # clock edges the bridge drives to take one byte from the host
# The fabric view's heat: a share of the frame's engine clocks → a colour, coolest first.
HEAT_RAMP = ((0.02, BLUE), (0.06, LBLUE), (0.15, CYAN), (0.3, LGREEN), (0.5, YELLOW), (0.75, LRED))
CELL_LABEL = {"show": "out", "copy": "cpy", "floor": "flr", "sqrt": "sqr", "float": "flt", "popcount": "pop",
              "new": "new", "ld": "ld", "st": "st", "at": "at"}

KEYWORDS = {"Let", "Set", "Show", "Return", "If", "Otherwise", "While", "Repeat", "for", "in", "from", "to", "be",
            "is", "of", "and", "or", "not", "mutable", "Push", "Pop", "item", "length", "a", "new", "true", "false",
            "less", "greater", "than", "equal", "equals", "at", "least", "most", "into", "with"}


def panel_regions(t):
    """The text cells the board paints the activity panel over, as (row, first column, columns): the cycle digits,
    the block tiles, the stack and heap gauges and the cell squares."""
    p = t["panel"]
    return [
        (p["title_row"], p["cycles_col"], 10),
        (p["block_row"], p["block_col"], p["blocks"] * p["block_tile"]),
        (p["block_row"], p["stack_col"], p["depth"]),
        (p["block_row"], p["heap_col"], (p["heap"] + 7) // 8),
        (p["cell_row"], p["cell_col"], p["cells"] * 2),
    ]


def library_window(pick, entries):
    """The first library entry the side list shows when entry `pick` of `entries` is picked: the list scrolls just
    far enough to keep it in view."""
    return min(max(0, pick - LIBRARY_ROWS + 1), max(0, entries - LIBRARY_ROWS))


class Keyboard:
    """PS/2 scan code set 2 (what the Nexys A7's USB bridge sends for a keyboard) to key names: printable characters
    as themselves, and 'Enter', 'Backspace', 'Up', 'F5', 'C-r' (Ctrl+R) and so on."""

    PLAIN = {0x1C: "a", 0x32: "b", 0x21: "c", 0x23: "d", 0x24: "e", 0x2B: "f", 0x34: "g", 0x33: "h", 0x43: "i",
             0x3B: "j", 0x42: "k", 0x4B: "l", 0x3A: "m", 0x31: "n", 0x44: "o", 0x4D: "p", 0x15: "q", 0x2D: "r",
             0x1B: "s", 0x2C: "t", 0x3C: "u", 0x2A: "v", 0x1D: "w", 0x22: "x", 0x35: "y", 0x1A: "z",
             0x45: "0", 0x16: "1", 0x1E: "2", 0x26: "3", 0x25: "4", 0x2E: "5", 0x36: "6", 0x3D: "7", 0x3E: "8",
             0x46: "9", 0x0E: "`", 0x4E: "-", 0x55: "=", 0x54: "[", 0x5B: "]", 0x5D: "\\", 0x4C: ";", 0x52: "'",
             0x41: ",", 0x49: ".", 0x4A: "/", 0x29: " "}
    SHIFTED = {"`": "~", "1": "!", "2": "@", "3": "#", "4": "$", "5": "%", "6": "^", "7": "&", "8": "*", "9": "(",
               "0": ")", "-": "_", "=": "+", "[": "{", "]": "}", "\\": "|", ";": ":", "'": '"', ",": "<", ".": ">",
               "/": "?"}
    NAMED = {0x5A: "Enter", 0x66: "Backspace", 0x0D: "Tab", 0x76: "Esc", 0x05: "F1", 0x06: "F2", 0x04: "F3",
             0x0C: "F4", 0x03: "F5", 0x0B: "F6", 0x83: "F7", 0x0A: "F8", 0x01: "F9", 0x09: "F10", 0x78: "F11",
             0x07: "F12"}
    EXTENDED = {0x75: "Up", 0x72: "Down", 0x6B: "Left", 0x74: "Right", 0x6C: "Home", 0x69: "End", 0x7D: "PageUp",
                0x7A: "PageDown", 0x71: "Delete", 0x70: "Insert", 0x5A: "Enter", 0x4A: "/"}
    KEYPAD = {0x70: "0", 0x69: "1", 0x72: "2", 0x7A: "3", 0x6B: "4", 0x73: "5", 0x74: "6", 0x6C: "7", 0x75: "8",
              0x7D: "9", 0x71: ".", 0x79: "+", 0x7B: "-", 0x7C: "*"}

    def __init__(self):
        self.extended = self.release = False
        self.shift = self.ctrl = self.alt = self.caps = False

    def push(self, b, damaged=False):
        """Feed one byte; returns a key name when this byte completes a key press. A `damaged` byte (received with a
        parity or framing error) ends the code it was part of, unread."""
        if damaged:
            self.extended = self.release = False
            return None
        if b == 0xE0:
            self.extended = True
            return None
        if b == 0xF0:
            self.release = True
            return None
        ext, rel = self.extended, self.release
        self.extended = self.release = False
        if b in (0x12, 0x59) and not ext:
            self.shift = not rel
            return None
        if b == 0x14:
            self.ctrl = not rel
            return None
        if b == 0x11:
            self.alt = not rel
            return None
        if rel:
            return None
        if b == 0x58 and not ext:
            self.caps = not self.caps
            return None
        if ext:
            return self.EXTENDED.get(b)
        if b in self.NAMED:
            return self.NAMED[b]
        if b in self.KEYPAD:
            return self.KEYPAD[b]
        ch = self.PLAIN.get(b)
        if ch is None:
            return None
        if self.ctrl and ch.isalpha():
            return "C-" + ch
        if ch.isalpha():
            return ch.upper() if self.shift != self.caps else ch
        return self.SHIFTED.get(ch, ch) if self.shift else ch


class Mouse:
    """PS/2 mouse packets (3 bytes: buttons and sign bits, dx, dy) to a pointer on the 640 x 480 screen."""

    def __init__(self):
        self.packet = []
        self.x, self.y = 320, 240
        self.left = False

    def push(self, b, damaged=False):
        """Feed one byte; returns ('move', (row, col)) or ('click', (row, col)) when a packet completes. A `damaged`
        byte (received with a parity or framing error) still takes its place in the packet, so the packets after it
        stay aligned; its packet is dropped whole."""
        if not self.packet and not damaged and not (b & 0x08):
            return None
        self.packet.append(None if damaged else b)
        if len(self.packet) < 3:
            return None
        packet, self.packet = self.packet, []
        if None in packet:
            return None
        flags, dx, dy = packet
        dx -= 256 if flags & 0x10 else 0
        dy -= 256 if flags & 0x20 else 0
        self.x = min(639, max(0, self.x + dx))
        self.y = min(479, max(0, self.y - dy))
        pos = (self.y // 16, self.x // 8)
        pressed = bool(flags & 0x01)
        clicked = pressed and not self.left
        self.left = pressed
        return ("click", pos) if clicked else ("move", pos)


class OsInput:
    """Keyboards and mice plugged into this computer, read from the kernel's input devices (Linux evdev) with no
    terminal window, and grabbed, so what is typed goes to the Fabric Terminal and not to the desktop. A keyboard is
    a device that has the keys to type a program (letters, Enter, space), on any bus — USB, Bluetooth, a laptop's
    own; a mouse has X and Y movement and a left button. `match` picks devices by name (case-insensitive substring);
    empty takes every keyboard and mouse. Reading them needs the `input` group: a device this program may not open,
    or that another program holds, is named in `problems` with the reason, once. Devices plugged in are picked up
    every two seconds (`added`), and one unplugged is let go (`removed`)."""

    EV_KEY, EV_REL = 1, 2
    REL_X, REL_Y = 0, 1
    BTN_LEFT = 0x110
    EVIOCGRAB = 0x40044590
    EVENT = struct.Struct("llHHi")
    DEVICES = "/proc/bus/input/devices"
    WORD = struct.calcsize("l") * 8
    TYPING = [28, 57] + list(range(16, 26)) + list(range(30, 39)) + list(range(44, 51))
    PLAIN = {2: "1", 3: "2", 4: "3", 5: "4", 6: "5", 7: "6", 8: "7", 9: "8", 10: "9", 11: "0", 12: "-", 13: "=",
             16: "q", 17: "w", 18: "e", 19: "r", 20: "t", 21: "y", 22: "u", 23: "i", 24: "o", 25: "p", 26: "[",
             27: "]", 30: "a", 31: "s", 32: "d", 33: "f", 34: "g", 35: "h", 36: "j", 37: "k", 38: "l", 39: ";",
             40: "'", 41: "`", 43: "\\", 44: "z", 45: "x", 46: "c", 47: "v", 48: "b", 49: "n", 50: "m", 51: ",",
             52: ".", 53: "/", 57: " "}
    KEYPAD = {71: "7", 72: "8", 73: "9", 74: "-", 75: "4", 76: "5", 77: "6", 78: "+", 79: "1", 80: "2", 81: "3",
              82: "0", 83: ".", 55: "*", 98: "/"}
    NAMED = {1: "Esc", 14: "Backspace", 15: "Tab", 28: "Enter", 96: "Enter", 102: "Home", 103: "Up", 104: "PageUp",
             105: "Left", 106: "Right", 107: "End", 108: "Down", 109: "PageDown", 110: "Insert", 111: "Delete",
             59: "F1", 60: "F2", 61: "F3", 62: "F4", 63: "F5", 64: "F6", 65: "F7", 66: "F8", 67: "F9", 68: "F10",
             87: "F11", 88: "F12"}
    SHIFT, CTRL, CAPS = {42, 54}, {29, 97}, 58

    def __init__(self, match="", devices_file=None):
        self.match = match
        self.devices_file = devices_file or self.DEVICES
        self.devices = []
        self.refused = {}
        self.added, self.removed, self.problems = [], [], []
        self.x, self.y = 320, 240
        self.shift = self.ctrl = self.caps = False
        self.scanned = 0.0
        self.added.extend(self.rescan())

    @property
    def mouse(self):
        return any("mouse" in kinds for _f, _name, kinds, _path in self.devices)

    def keyboards(self):
        return [name for _f, name, kinds, _path in self.devices if "keyboard" in kinds]

    def open_device(self, path):
        """The event node at `path`, opened non-blocking and grabbed: its keys reach only this program."""
        import fcntl

        f = open(path, "rb", buffering=0)
        try:
            fcntl.ioctl(f, self.EVIOCGRAB, 1)
            os.set_blocking(f.fileno(), False)
        except OSError:
            f.close()
            raise
        return f

    @staticmethod
    def refusal(e):
        if e.errno in (errno.EACCES, errno.EPERM):
            return "this program may not read it: it needs the input group (log in again, or start it under `sg input`)"
        if e.errno == errno.EBUSY:
            return "another program holds it (a second terminal?)"
        return e.strerror or str(e)

    def rescan(self):
        """Open and grab every matching device not open yet (one plugged in since); returns the names opened."""
        opened = []
        listed = self.find()
        for path in [p for p in self.refused if p not in {path for _n, path, _k in listed}]:
            del self.refused[path]
        held = {path for _f, _name, _kinds, path in self.devices}
        for name, path, kinds in listed:
            if path in held or path in self.refused or (self.match and self.match.lower() not in name.lower()):
                continue
            try:
                f = self.open_device(path)
            except FileNotFoundError:
                continue
            except OSError as e:
                self.refused[path] = e
                self.problems.append(f"{name} ({path}): {self.refusal(e)}")
                continue
            self.devices.append((f, name, kinds, path))
            opened.append(f"{name} ({', '.join(sorted(kinds))})")
        self.scanned = time.time()
        return opened

    def bits(self, words):
        """A capability bitmap as /proc/bus/input/devices prints it (hex words, the most significant first)."""
        value = 0
        for w in words.split():
            value = (value << self.WORD) | int(w, 16)
        return value

    def find(self):
        """(name, /dev/input/eventN, kinds) for each keyboard and mouse the kernel lists; kinds holds 'keyboard'
        and/or 'mouse'."""
        found = []
        try:
            text = open(self.devices_file).read()
        except OSError:
            return found
        for block in text.split("\n\n"):
            name, handlers, caps = "", [], {}
            for line in block.splitlines():
                tag, _, rest = line.partition(": ")
                if tag == "N":
                    name = rest.partition("=")[2].strip().strip('"').strip()
                elif tag == "H":
                    handlers = rest.partition("=")[2].split()
                elif tag == "B":
                    k, _, v = rest.partition("=")
                    caps[k] = self.bits(v)
            event = next((h for h in handlers if h.startswith("event")), None)
            if event is None:
                continue
            keys, rel = caps.get("KEY", 0), caps.get("REL", 0)
            kinds = ({"keyboard"} if all(keys >> code & 1 for code in self.TYPING) else set()) | \
                    ({"mouse"} if rel & 3 == 3 and keys >> self.BTN_LEFT & 1 else set())
            if kinds:
                found.append((name, f"/dev/input/{event}", kinds))
        return found

    def translate(self, events):
        """(type, code, value) kernel events → ('key', name) / ('move', (row, col)) / ('click', (row, col))."""
        out = []
        for etype, code, value in events:
            if etype == self.EV_KEY:
                if code in self.SHIFT:
                    self.shift = value != 0
                elif code in self.CTRL:
                    self.ctrl = value != 0
                elif code == self.CAPS and value == 1:
                    self.caps = not self.caps
                elif code == self.BTN_LEFT and value == 1:
                    out.append(("click", (self.y // 16, self.x // 8)))
                elif value in (1, 2):
                    if code in self.NAMED:
                        out.append(("key", self.NAMED[code]))
                    elif code in self.KEYPAD:
                        out.append(("key", self.KEYPAD[code]))
                    elif code in self.PLAIN:
                        ch = self.PLAIN[code]
                        if self.ctrl and ch.isalpha():
                            out.append(("key", "C-" + ch))
                        elif ch.isalpha():
                            out.append(("key", ch.upper() if self.shift != self.caps else ch))
                        else:
                            out.append(("key", Keyboard.SHIFTED.get(ch, ch) if self.shift else ch))
            elif etype == self.EV_REL and code in (self.REL_X, self.REL_Y):
                if code == self.REL_X:
                    self.x = min(639, max(0, self.x + value))
                else:
                    self.y = min(479, max(0, self.y + value))
                out.append(("move", (self.y // 16, self.x // 8)))
        return out

    def read(self):
        """Every key, pointer move and click since the last call; devices plugged in are picked up every two
        seconds, and one unplugged is let go."""
        if time.time() - self.scanned > 2.0:
            self.added.extend(self.rescan())
        events = []
        for entry in list(self.devices):
            f, name = entry[0], entry[1]
            while True:
                try:
                    data = f.read(self.EVENT.size * 64)
                except BlockingIOError:
                    break
                except OSError:
                    self.devices.remove(entry)
                    self.removed.append(name)
                    f.close()
                    break
                if not data:
                    break
                for i in range(0, len(data) - self.EVENT.size + 1, self.EVENT.size):
                    _sec, _usec, etype, code, value = self.EVENT.unpack_from(data, i)
                    events.append((etype, code, value))
        return self.translate(events)


class LocalTerminal:
    """The terminal this program runs in: raw keyboard input, xterm mouse reports, and a mirror of the board's
    screen drawn with the 16 ANSI colours."""

    ANSI = [30, 34, 32, 36, 31, 35, 33, 37, 90, 94, 92, 96, 91, 95, 93, 97]
    SEQ = {"[A": "Up", "[B": "Down", "[C": "Right", "[D": "Left", "[H": "Home", "[F": "End", "[1~": "Home",
           "[4~": "End", "[5~": "PageUp", "[6~": "PageDown", "[3~": "Delete", "[2~": "Insert", "OP": "F1",
           "OQ": "F2", "OR": "F3", "OS": "F4", "[15~": "F5", "[17~": "F6", "[18~": "F7", "[19~": "F8",
           "[20~": "F9", "[21~": "F10", "[23~": "F11", "[24~": "F12"}

    def __init__(self, mirror=True):
        import termios
        import tty

        self.fd = sys.stdin.fileno()
        self.saved = termios.tcgetattr(self.fd)
        tty.setraw(self.fd)
        self.mirror = mirror
        self.pending = ""
        if mirror:
            sys.stdout.write("\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h")
            sys.stdout.flush()

    def restore(self):
        import termios

        if self.mirror:
            sys.stdout.write("\x1b[?1006l\x1b[?1000l\x1b[?25h\x1b[?1049l")
            sys.stdout.flush()
        termios.tcsetattr(self.fd, termios.TCSADRAIN, self.saved)

    def read(self):
        """Every key and mouse click typed since the last call, as ('key', name) / ('click', (row, col))."""
        out = []
        while select.select([self.fd], [], [], 0)[0]:
            self.pending += os.read(self.fd, 64).decode("latin-1")
        s = self.pending
        i = 0
        while i < len(s):
            ch = s[i]
            if ch == "\x1b":
                if i + 1 >= len(s):
                    out.append(("key", "Esc"))
                    i += 1
                    continue
                if s.startswith("\x1b[<", i):
                    end = next((j for j in range(i, len(s)) if s[j] in "Mm"), None)
                    if end is None:
                        break
                    b, x, y = (int(v) for v in s[i + 3:end].split(";"))
                    if s[end] == "M" and b == 0:
                        out.append(("click", (y - 1, x - 1)))
                    i = end + 1
                    continue
                body = s[i + 1:i + 6]
                for seq, name in sorted(self.SEQ.items(), key=lambda kv: -len(kv[0])):
                    if body.startswith(seq):
                        out.append(("key", name))
                        i += 1 + len(seq)
                        break
                else:
                    out.append(("key", "Esc"))
                    i += 1
                continue
            o = ord(ch)
            if ch in "\r\n":
                out.append(("key", "Enter"))
            elif o in (8, 127):
                out.append(("key", "Backspace"))
            elif ch == "\t":
                out.append(("key", "Tab"))
            elif 1 <= o <= 26:
                out.append(("key", "C-" + chr(o + 96)))
            elif 32 <= o < 127:
                out.append(("key", ch))
            i += 1
        self.pending = s[i:]
        return out

    def draw(self, screen):
        if not self.mirror:
            return
        lines = ["\x1b[H"]
        for r in range(screen.rows):
            row, last = [], None
            for c in range(screen.cols):
                cell = screen.want[r][c]
                attr = (cell.fg, cell.bg)
                if (r, c) == screen.pointer or ((r, c) == screen.cursor):
                    attr = (cell.bg, cell.fg)
                if attr != last:
                    row.append(f"\x1b[{self.ANSI[attr[0]]};{self.ANSI[attr[1]] + 10}m")
                    last = attr
                g = cell.glyph
                row.append(tl.char_of(g, screen.t))
            lines.append("".join(row) + "\x1b[0m" + ("\r\n" if r < screen.rows - 1 else ""))
        sys.stdout.write("".join(lines))
        sys.stdout.flush()


class Library:
    """The programs the terminal offers, as (title, source, function or None, arguments): the board kit's, every one
    compiled with its golden, else the repo's library (`terminal_link.library`)."""

    def __init__(self, chip):
        kit = getattr(chip, "KIT", None)
        if kit is not None and "term" in kit.get("targets", {}):
            self.entries = [(e.get("title") or os.path.splitext(os.path.basename(e["source"]))[0].replace("_", " "),
                             e["source"], e["fn"], e["args"]) for e in kit["targets"]["term"]["programs"]]
        else:
            self.entries = tl.library()

    def source(self, i):
        return open(os.path.join(REPO, self.entries[i][1])).read()


class Editor:
    """The program being edited: lines of text, a cursor, and the window on screen — its first line `top` and its
    first column `left`, each moved just far enough to keep the cursor inside."""

    WIDTH = EDIT_COLS - GUTTER

    def __init__(self, text=""):
        self.load(text)

    def load(self, text):
        self.lines = text.split("\n")
        if self.lines and self.lines[-1] == "" and len(self.lines) > 1:
            self.lines.pop()
        self.row = self.col = self.top = self.left = 0
        self.changed = False

    def text(self):
        return "\n".join(self.lines) + "\n"

    def key(self, k):
        line = self.lines[self.row]
        if k == "Up":
            self.row = max(0, self.row - 1)
        elif k == "Down":
            self.row = min(len(self.lines) - 1, self.row + 1)
        elif k == "Left":
            if self.col > 0:
                self.col -= 1
            elif self.row > 0:
                self.row -= 1
                self.col = len(self.lines[self.row])
        elif k == "Right":
            if self.col < len(line):
                self.col += 1
            elif self.row < len(self.lines) - 1:
                self.row, self.col = self.row + 1, 0
        elif k == "Home":
            self.col = 0
        elif k == "End":
            self.col = len(line)
        elif k == "PageUp":
            self.row = max(0, self.row - EDIT_ROWS)
        elif k == "PageDown":
            self.row = min(len(self.lines) - 1, self.row + EDIT_ROWS)
        elif k == "Enter":
            indent = len(line) - len(line.lstrip(" "))
            if line.rstrip().endswith(":"):
                indent += 4
            self.lines[self.row] = line[:self.col]
            self.lines.insert(self.row + 1, " " * indent + line[self.col:])
            self.row, self.col = self.row + 1, indent
            self.changed = True
        elif k == "Backspace":
            if self.col > 0:
                self.lines[self.row] = line[:self.col - 1] + line[self.col:]
                self.col -= 1
            elif self.row > 0:
                prev = self.lines[self.row - 1]
                self.lines[self.row - 1] = prev + line
                del self.lines[self.row]
                self.row, self.col = self.row - 1, len(prev)
            self.changed = True
        elif k == "Delete":
            if self.col < len(line):
                self.lines[self.row] = line[:self.col] + line[self.col + 1:]
            elif self.row < len(self.lines) - 1:
                self.lines[self.row] = line + self.lines[self.row + 1]
                del self.lines[self.row + 1]
            self.changed = True
        elif k == "Tab":
            self.lines[self.row] = line[:self.col] + "    " + line[self.col:]
            self.col += 4
            self.changed = True
        elif len(k) == 1:
            self.lines[self.row] = line[:self.col] + k + line[self.col:]
            self.col += 1
            self.changed = True
        else:
            return False
        self.col = min(self.col, len(self.lines[self.row]))
        if self.row < self.top:
            self.top = self.row
        elif self.row >= self.top + EDIT_ROWS:
            self.top = self.row - EDIT_ROWS + 1
        if self.col < self.left:
            self.left = self.col
        elif self.col >= self.left + self.WIDTH:
            self.left = self.col - self.WIDTH + 1
        return True


def highlight(line):
    """Foreground colours for one line of LOGOS source, a colour per character."""
    colours = [GREY] * len(line)
    if line.lstrip().startswith("##"):
        return [LCYAN] * len(line)
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == '"':
            j = line.find('"', i + 1)
            j = len(line) - 1 if j < 0 else j
            for k in range(i, j + 1):
                colours[k] = YELLOW
            i = j + 1
        elif ch.isdigit():
            j = i
            while j < len(line) and (line[j].isdigit() or line[j] == "."):
                colours[j] = LMAGENTA
                j += 1
            i = j
        elif ch.isalpha():
            j = i
            while j < len(line) and (line[j].isalnum() or line[j] == "_"):
                j += 1
            if line[i:j] in KEYWORDS:
                for k in range(i, j):
                    colours[k] = LGREEN
            i = j
        else:
            i += 1
    return colours


class Terminal:
    """The REPL: the editor, the library, a run, and the screen that shows them."""

    def __init__(self, t, chip, link, local, bit=None, engine=True, engine_target="term", log=None, pong=None):
        """`engine_target` names the `logos-chip.py` target whose engine runs the programs: the terminal's own
        (`term`, on the board that draws the screen, its activity on the panel) or an engine on another board.
        `pong` is the board's answer to the first PING, whose features say what its image draws and keeps beyond
        the screen (`take_features`; each PONG after says again).
        `log` (a file) gets a line per thing that happens — the device on the board's USB port, each key and click,
        each run and its verdict — for a terminal running with no screen of its own on this machine."""
        self.t, self.chip, self.link, self.local, self.bit, self.engine = t, chip, link, local, bit, engine
        self.log = log
        self.osinput = None
        self.engine_target = engine_target
        self.work = tempfile.TemporaryDirectory(prefix="logos-term-")
        self.screen = tl.Screen(t)
        self.rows, self.cols = self.screen.rows, self.screen.cols
        self.library = Library(chip)
        self.pick = 0
        self.focus = "editor"
        self.editor = Editor(self.library.source(0) if self.library.entries else "## Main\n    Show \"hi\".\n")
        self.entry = 0 if self.library.entries else None
        self.output = []
        self.status = ("ready", GREY)
        self.run = None
        self.keyboard, self.mouse = Keyboard(), Mouse()
        self.device = None
        self.asking = None
        self.acks_due = 0
        self.last_command = None
        self.watch = {"next": 0.0, "expect": None, "missed": 0, "gone": False}
        self.line = None
        self.history = RunHistory()
        self.demo = None
        self.map = None
        self.compile_cache = {}
        self.view = "source"
        self.side = "programs"
        self.take_features(pong)
        self.act, self.act_hist, self.act_next = None, [], 0.0
        self.act_busy, self.whole, self.totals_next = None, None, 0.0
        self.probe_next, self.silence_said, self.sends = 0.0, False, 0
        self.faulted = False
        self.help = False
        self.quit = False

    # ── drawing ────────────────────────────────────────────────────────────────────────────────────────────
    def draw(self):
        s = self.screen
        s.fill(0, 0, 1, self.cols, " ", WHITE, BLUE)
        title = self.library.entries[self.entry][0] if self.entry is not None else "untitled"
        if self.editor.changed:
            title += " (edited)"
        s.put(0, 1, "LOGOS ▶ Fabric Terminal", WHITE, BLUE)
        s.put(0, 28, f"{title:<26}"[:26], YELLOW, BLUE)
        s.put(0, 56, f"{self.engine_board()} fabric", LCYAN, BLUE)
        if self.view == "fabric" and self.map:
            self.draw_fabric()
        else:
            self.draw_editor()
        self.draw_side()
        self.draw_output()
        self.draw_panel()
        if self.help:
            self.draw_help()
        self.flush()

    def draw_editor(self):
        s, e = self.screen, self.editor
        for i in range(EDIT_ROWS):
            n = e.top + i
            r = EDIT_TOP + i
            s.fill(r, 0, 1, EDIT_COLS, " ", GREY, BLACK)
            if n < len(e.lines):
                s.put(r, 0, f"{n + 1:>3}" + ("◀" if e.left and len(e.lines[n]) > 0 else " "), DGREY, BLACK)
                full = e.lines[n]
                shown = full[e.left:e.left + e.WIDTH]
                for k, (ch, fg) in enumerate(zip(shown, highlight(full)[e.left:])):
                    s.put(r, GUTTER + k, ch, fg, BLACK)
        if self.focus == "editor" and not self.help:
            s.set_cursor((EDIT_TOP + e.row - e.top, min(GUTTER + e.col - e.left, EDIT_COLS - 1)))
        else:
            s.set_cursor(None)

    def draw_side(self):
        s = self.screen
        s.fill(EDIT_TOP, SIDE_COL, EDIT_ROWS, self.cols - SIDE_COL, " ", GREY, BLUE)
        s.put(EDIT_TOP, SIDE_COL, "│", LBLUE, BLUE)
        if self.side == "blocks" and self.map and self.focus != "library":
            if self.small and not self.help:
                self.draw_side_blocks_small()
            else:
                s.set_window(None)
                self.draw_side_blocks()
        else:
            s.set_window(None)
            self.draw_side_programs()
        base = EDIT_TOP + EDIT_ROWS - len(KEYS)
        for i, (k, what, clickable) in enumerate(KEYS):
            if clickable:
                s.put(base + i, SIDE_COL + 2, f" {k:<4}{what:<13}", BLACK, CYAN)
                s.put(base + i, SIDE_COL + 2, f" {k:<4}", BLUE, CYAN)
            else:
                s.put(base + i, SIDE_COL + 2, f" {k:<4}", YELLOW, BLUE)
                s.put(base + i, SIDE_COL + 7, what, GREY, BLUE)
        for r in range(EDIT_TOP + 1, EDIT_TOP + EDIT_ROWS):
            s.put(r, SIDE_COL, "│", LBLUE, BLUE)

    def draw_side_blocks(self):
        """The running program's blocks stacked in the side list: each one's number and role, a bar and the share of
        the engine's clocks it took in the last frame, and what it does; the engine's own block drawn bright. Up to
        five blocks get two rows each, more a row each."""
        s, m, a = self.screen, self.map, self.shown_activity()
        p = self.t["panel"]
        cells = sum(len(b["cells"]) for b in m)
        s.put(EDIT_TOP, SIDE_COL + 2, f"BLOCKS {len(m)}/{p['blocks']}  CELLS {cells}/{p['cells']}"[:22], WHITE, BLUE)
        n = len(m)
        total = sum(a["block_clocks"][:n]) if a else 0
        width = self.cols - SIDE_COL - 1
        s.fill(EDIT_TOP + 1, SIDE_COL + 1, LIBRARY_ROWS, width, " ", GREY, BLACK)
        two = 2 * n <= LIBRARY_ROWS
        row = EDIT_TOP + 1
        for i, b in enumerate(m):
            clocks = a["block_clocks"][i] if a else 0
            share = clocks / total if total else 0.0
            current = bool(a and a["running"] and a["block"] == i)
            idle = b["role"] == "idle"
            edge = WHITE if current else (self.heat(share) if clocks else DGREY)
            title = f"{i} {b['role'].upper():<4}"
            pct = f"{share * 100:3.0f}%" if a and total else ""
            does = (pack([compact(x) for x in b["says"]], width - 2, "; ") or [self.next_text(b["next"])])[0]
            c0 = SIDE_COL + 1
            if two:
                bar = ("█" * round(share * 7)).ljust(7, "░") if a and total else "─" * 7
                s.put(row, c0, "┌" + "─" * (width - 2) + "┐", edge, BLACK)
                s.put(row, c0 + 1, title, BLACK if current else (DGREY if idle else WHITE), edge if current else BLACK)
                s.put(row, c0 + 8, bar, edge, BLACK)
                s.put(row, c0 + width - 1 - len(pct), pct, edge, BLACK)
                s.put(row + 1, c0, "└" + does[: width - 2].ljust(width - 2, "─") + "┘", edge, BLACK)
                s.put(row + 1, c0 + 1, does[: width - 2], DGREY if idle else GREY, BLACK)
                row += 2
            else:
                s.put(row, c0, "[" + " " * (width - 2) + "]", edge, BLACK)
                s.put(row, c0 + 1, title, BLACK if current else (DGREY if idle else WHITE), edge if current else BLACK)
                s.put(row, c0 + 8, does[: width - 14], GREY, BLACK)
                s.put(row, c0 + width - 1 - len(pct), pct, edge, BLACK)
                row += 1

    def draw_side_blocks_small(self):
        """The running program's blocks stacked in the side list in the small font, two of its rows to a text row,
        under a head row counting them and their cells: each block a box — its number and role, a bar and its share
        of the engine's clocks in the last frame, what it does, its cells' shares and where it goes next, and its
        cells along its foot, heat-coloured, the engine's own cell blinking — the engine's own block drawn bright.
        Every box gets its title and foot; the rows left go to the boxes with the most to say, each up to all of it."""
        s, m, a = self.screen, self.map, self.shown_activity()
        p = self.t["panel"]
        n = len(m)
        cells = sum(len(b["cells"]) for b in m)
        width = self.cols - SIDE_COL - 1
        inner = width - 2
        srows = 2 * (LIBRARY_ROWS + 1)
        s.set_window((EDIT_TOP, SIDE_COL + 1, LIBRARY_ROWS + 1, width))
        s.fill_small(0, 0, 2, width, " ", WHITE, BLUE)
        s.put_small(0, 1, f"BLOCKS {n}/{p['blocks']} CELLS {cells}/{p['cells']}"[: width - 1], WHITE, BLUE)
        said, colour = self.shares_said(width - 1)
        s.put_small(1, 1, said, colour, BLUE)
        s.fill_small(2, 0, srows - 2, width, " ", GREY, BLACK)
        total = sum(a["block_clocks"][:n]) if a else 0
        lines = [self.block_lines(b, a, total, inner) for b in m]
        rows = share_rows(srows - 2, [2 + len(x) for x in lines])
        r0 = 2
        for i, b in enumerate(m):
            h = rows[i]
            clocks = a["block_clocks"][i] if a else 0
            share = clocks / total if total else 0.0
            current = bool(a and a["running"] and a["block"] == i)
            idle = b["role"] == "idle"
            edge = WHITE if current else (self.heat(share) if clocks else DGREY)
            pct = f"{share * 100:3.0f}%" if a and total else ""
            s.put_small(r0, 0, "┌" + "─" * inner + "┐", edge, BLACK)
            s.put_small(r0, 1, f"{i} {b['role'].upper():<4}", BLACK if current else (DGREY if idle else WHITE),
                        edge if current else BLACK)
            if a and total:
                s.put_small(r0, 8, ("█" * round(share * 8)).ljust(8, "░"), edge, BLACK)
            s.put_small(r0, width - 1 - len(pct), pct, edge, BLACK)
            keep = sorted(sorted(range(len(lines[i])), key=lambda k: lines[i][k][2])[: h - 2])
            for k in range(h - 2):
                s.put_small(r0 + 1 + k, 0, "│", edge, BLACK)
                s.put_small(r0 + 1 + k, width - 1, "│", edge, BLACK)
                if k < len(keep):
                    text, fg, _rank = lines[i][keep[k]]
                    s.put_small(r0 + 1 + k, 1, text[:inner], DGREY if idle else fg, BLACK)
            s.put_small(r0 + h - 1, 0, "└" + "─" * inner + "┘", edge, BLACK)
            for col, text, fg, bg, blink in self.cell_chips(b, a, total, inner):
                s.put_small(r0 + h - 1, 1 + col, text, fg, bg, blink)
            r0 += h

    def shares_said(self, width):
        """What the blocks' shares are, in `width` columns, and its colour: each frame's while a program runs, the
        whole run's once its totals are in, its last frame's from an image without them."""
        a, run = self.shown_activity(), self.run
        if a is not None and a.get("whole"):
            return first_fit([f"WHOLE RUN {a['cycles']:,} cycles", f"WHOLE RUN {a['cycles']:,} cyc",
                              f"WHOLE RUN {a['cycles']:,}"], width), YELLOW
        if run is not None and not run.get("done"):
            return first_fit(["LIVE · share per frame", "LIVE"], width), LGREEN
        if a is not None and sum(a["block_clocks"]):
            return first_fit(["LAST FRAME OF THE RUN", "LAST FRAME"], width), GREY
        return "", GREY

    def block_lines(self, b, a, total, inner):
        """What a block's box in the side list says, in the order it is drawn, as (text, colour, rank): what it does,
        its cells' shares of the frame's engine clocks, where it goes next. A box with fewer rows keeps the lines of
        lowest rank: the first of what it does, then where it goes next, then its cells, then the rest."""
        says = pack([compact(x) for x in b["says"]], inner, "; ")
        out = [(line, GREY, 0 if k == 0 else 30 + k) for k, line in enumerate(says)]
        if b["cells"] and a and total:
            out += [(line, DGREY, 20 if k == 0 else 60 + k) for k, line in enumerate(self.cell_shares(b, a, total, inner))]
        out += [(line, LCYAN, 10 + k) for k, line in enumerate(self.next_lines(b["next"], inner))]
        return out

    def next_lines(self, nx, width):
        """Where a block goes next (`next_text`) in lines of `width`: a branch too long for one drops its `if`, and
        still too long splits at its else."""
        text = self.next_text(nx)
        if len(text) <= width:
            return [text]
        if nx.get("kind") == "branch":
            short = f"{nx['cond']} ▶{nx['then']} else ▶{nx['otherwise']}"
            if len(short) <= width:
                return [short]
            return wrap(f"if {nx['cond']} ▶{nx['then']}", width) + wrap(f"else ▶{nx['otherwise']}", width)
        return wrap(text, width)

    def cell_chips(self, b, a, total, room):
        """A block's cells as the fabric draws them along a box's foot, in `room` columns: each cell's operation,
        three, two or one characters wide as they fit, on its heat colour, the engine's own cell blinking; the
        cells past the room counted as +N. Each one (column, text, fg, bg, blink)."""
        ops = b["cells"]
        cw = 3 if 3 * len(ops) <= room else 2 if 2 * len(ops) <= room else 1
        shown = ops[: room // cw]
        if len(shown) < len(ops):
            shown = ops[: max(0, (room - len(f"+{len(ops)}")) // cw)]
        out = []
        for k, op in enumerate(shown):
            cell = b["first_cell"] + k
            c_clocks = a["cell_clocks"][cell] if a and cell < len(a["cell_clocks"]) else 0
            c_share = c_clocks / total if total else 0.0
            here = bool(a and a["running"] and a["cell"] == cell)
            label = CELL_LABEL.get(op, op)
            label = f"{label[:cw]:^{cw}}" if cw > 1 else label[0]
            bg = WHITE if here else (self.heat(c_share) if c_clocks else BLACK)
            fg = BLACK if bg in (WHITE, YELLOW, LGREEN, LCYAN, CYAN) else (DGREY if bg == BLACK else WHITE)
            out.append((k * cw, label, fg, bg, here))
        if len(shown) < len(ops):
            out.append((len(shown) * cw, f"+{len(ops) - len(shown)}", DGREY, BLACK, False))
        return out

    def draw_side_programs(self):
        s = self.screen
        s.put(EDIT_TOP, SIDE_COL + 2, "PROGRAMS        F2", WHITE, BLUE)
        first = self.library_first()
        for row, i in enumerate(range(first, min(first + LIBRARY_ROWS, len(self.library.entries)))):
            title, _src, fn, args = self.library.entries[i]
            label = f"{title}" + (f" {fn}({args})" if fn else "")
            selected = self.focus == "library" and i == self.pick
            fg, bg = (BLUE, YELLOW) if selected else ((YELLOW if i == self.entry else GREY), BLUE)
            s.put(EDIT_TOP + 1 + row, SIDE_COL + 2, f"{label:<21}"[:21], fg, bg)
        if first > 0:
            s.put(EDIT_TOP + 1, SIDE_COL + 23, "▲", LCYAN, BLUE)
        if first + LIBRARY_ROWS < len(self.library.entries):
            s.put(EDIT_TOP + LIBRARY_ROWS, SIDE_COL + 23, "▼", LCYAN, BLUE)

    def library_first(self):
        return library_window(self.pick, len(self.library.entries))

    def draw_output(self):
        s = self.screen
        text, fg = self.status
        s.fill(OUT_BAR, 0, 1, self.cols, "─", LBLUE, BLACK)
        s.put(OUT_BAR, 2, " OUTPUT ", WHITE, BLACK)
        s.put(OUT_BAR, 12, f" {text} ", fg, BLACK)
        if self.demo is not None:
            label = " DEMO MODE · any key takes over "
            s.put(OUT_BAR, self.cols - len(label) - 2, label, YELLOW, BLACK)
        lines = self.output[-OUT_ROWS:]
        for i in range(OUT_ROWS):
            s.fill(OUT_TOP + i, 0, 1, self.cols, " ", GREY, BLACK)
            if i < len(lines):
                line, colour = lines[i]
                s.put(OUT_TOP + i, 2, line[:self.cols - 4], colour, BLACK)

    def draw_panel(self):
        """The activity panel's text: its title, the device on the board's USB port, and the labels beside the
        graphics the board draws over the cells `panel_regions` names — those cells stay blank here."""
        s, p = self.screen, self.t["panel"]
        s.fill(p["title_row"], 0, self.rows - p["title_row"], self.cols, " ", GREY, BLACK)
        s.fill(p["title_row"], 0, 1, self.cols, "─", DGREY, BLACK)
        s.put(p["title_row"], 2, " FABRIC ", WHITE, BLACK)
        dev = {"keyboard": "USB keyboard on the board", "mouse": "USB mouse on the board"}.get(self.device)
        if dev is None and self.osinput is not None:
            dev = "keyboard on this computer" if self.osinput.keyboards() else "plug a keyboard into this computer"
        dev = dev or "no USB device on the board"
        s.put(p["title_row"], 11, f" {dev} ", DGREY, BLACK)
        s.put(p["title_row"], p["cycles_col"] - 9, " CYCLES ", WHITE, BLACK)
        for row, col, width in panel_regions(self.t):
            s.fill(row, max(0, col - 1), 1, width + 2, " ", GREY, BLACK)
        for i in range(p["blocks"]):
            role = self.map[i]["role"] if self.map and i < len(self.map) else ""
            at = p["block_col"] + i * p["block_tile"]
            s.put(p["label_row"], at, str(i), DGREY, BLACK)
            s.put(p["label_row"], at + 1, role[: p["block_tile"] - 2], LCYAN if role and role != "idle" else DGREY, BLACK)
        s.put(p["label_row"], p["stack_col"], "STACK", DGREY, BLACK)
        s.put(p["label_row"], p["heap_col"], "HEAP", DGREY, BLACK)
        s.put(p["block_row"], 1, "BLOCKS", GREY, BLACK)
        s.put(p["cell_row"], 1, "CELLS", GREY, BLACK)

    # ── the fabric view ─────────────────────────────────────────────────────────────────────────────────────
    # The program as the engine holds it, live: a box per block with its role, what it does and where it goes next,
    # its cells along its foot, each block and cell coloured by its share of the engine's clocks in the last video
    # frame the board reported (ACTIVITY), the engine's block drawn bright and its cell blinking.
    def draw_fabric(self):
        s, m, a = self.screen, self.map, self.shown_activity()
        top, w = EDIT_TOP, EDIT_COLS
        s.fill(top, 0, EDIT_ROWS, w, " ", GREY, BLACK)
        cells = sum(len(b["cells"]) for b in m)
        title = self.library.entries[self.entry][0] if self.entry is not None else "untitled"
        s.fill(top, 0, 1, w, " ", WHITE, BLUE)
        clock_text = f"{a['cycles']:,} cycles" if a else "F4 source"
        p = self.t["panel"]
        s.put(top, 1, fit(["THE FABRIC", title, f"{len(m)}/{p['blocks']} blocks", f"{cells}/{p['cells']} cells"],
                          w - len(clock_text) - 3), WHITE, BLUE)
        s.put(top, w - 1 - len(clock_text), clock_text, YELLOW if a else LCYAN, BLUE)
        n = len(m)
        per_row = 2
        rows = (n + per_row - 1) // per_row
        h = max(4, min(8, (EDIT_ROWS - 2) // max(1, rows)))
        total = sum(a["block_clocks"][:n]) if a else 0
        for i, b in enumerate(m):
            self.draw_block(i, b, top + 1 + (i // per_row) * h, (i % per_row) * (w // per_row), w // per_row, h, a, total)
        foot = top + EDIT_ROWS - 1
        if a and a.get("whole"):
            text = fit(["WHOLE RUN", f"{a['cycles']:,} cycles + the clock that ends it",
                        "each block's and cell's share of every one"], w - 1)
            s.put(foot, 1, text, YELLOW, BLACK)
        elif a and (a["running"] or total):
            role = m[a["block"]]["role"] if a["block"] < n else "?"
            op = self.cell_op_at(a["cell"])
            text = fit([f"{'LIVE' if a['running'] else 'DONE'}", f"block {a['block']} {role}", f"cell {a['cell']} {op}",
                        f"depth {a['depth']}", f"heap {a['heap']}", f"frame {a['frame']}"], w - 1)
            s.put(foot, 1, text, LGREEN if a["running"] else YELLOW, BLACK)
        elif self.engine_target != "term":
            s.put(foot, 0, f" the engine on {self.engine_board()} reports no live activity"[:w], DGREY, BLACK)
        else:
            s.put(foot, 0, " F5 runs it: the board reports every frame of the engine here"[:w], DGREY, BLACK)

    def cell_op_at(self, cell):
        for b in self.map:
            if b["first_cell"] <= cell < b["first_cell"] + len(b["cells"]):
                return b["cells"][cell - b["first_cell"]]
        return ""

    @staticmethod
    def heat(share):
        if share <= 0:
            return DGREY
        for limit, colour in HEAT_RAMP:
            if share < limit:
                return colour
        return WHITE

    def draw_block(self, i, b, r0, c0, bw, h, a, total):
        s = self.screen
        clocks = a["block_clocks"][i] if a else 0
        share = clocks / total if total else 0.0
        current = bool(a and a["running"] and a["block"] == i)
        idle = b["role"] == "idle"
        edge = WHITE if current else self.heat(share)
        s.put(r0, c0, "┌" + "─" * (bw - 2) + "┐", edge, BLACK)
        s.put(r0, c0 + 1, f" {i} {b['role'].upper()} ", BLACK if current else edge, edge if current else BLACK)
        if a and total:
            hist = "".join(spark(x["block_clocks"][i], sum(x["block_clocks"])) for x in self.act_hist[-6:])
            tail = f" {hist} {share * 100:3.0f}% "
            s.put(r0, c0 + bw - 1 - len(tail), tail, edge, BLACK)
        for r in range(1, h - 1):
            s.put(r0 + r, c0, "│", edge, BLACK)
            s.put(r0 + r, c0 + bw - 1, "│", edge, BLACK)
        inner = bw - 3
        room = h - 3
        legend = self.cell_legend(b, a, total, inner) if room >= 2 and b["cells"] else None
        says = "; ".join(compact(x) for x in b["says"])
        lines = [says[k:k + inner] for k in range(0, len(says), inner)][: room - (1 if legend else 0)] if says else []
        for k, line in enumerate(lines):
            s.put(r0 + 1 + k, c0 + 2, line, DGREY if idle else GREY, BLACK)
        if legend:
            s.put(r0 + h - 3, c0 + 2, legend[:inner], DGREY, BLACK)
        s.put(r0 + h - 2, c0 + 2, self.next_text(b["next"])[:inner], DGREY if idle else LCYAN, BLACK)
        s.put(r0 + h - 1, c0, "└" + "─" * (bw - 2) + "┘", edge, BLACK)
        for col, text, fg, bg, blink in self.cell_chips(b, a, total, bw - 4):
            s.put(r0 + h - 1, c0 + 2 + col, text, fg, bg, blink)

    def cell_parts(self, b, a, total):
        """A block's cells, each its operation and its share of the frame's engine clocks."""
        parts = []
        for k, op in enumerate(b["cells"]):
            cell = b["first_cell"] + k
            clocks = a["cell_clocks"][cell] if a and cell < len(a["cell_clocks"]) else 0
            parts.append(f"{op} {clocks * 100 // total}%" if a and total else op)
        return parts

    def cell_legend(self, b, a, total, width):
        """A block's cells and each one's share of the frame's engine clocks, as far as `width` allows."""
        text = "cells " + "  ".join(self.cell_parts(b, a, total))
        return text if len(text) <= width else f"{len(b['cells'])} cells"

    def cell_shares(self, b, a, total, width):
        """A block's cells and each one's share of the frame's engine clocks, in lines of `width`, whole cells to a
        line, the first line headed `cells`."""
        parts = self.cell_parts(b, a, total)
        return pack([f"cells {parts[0]}"] + parts[1:], width, "  ") if parts else []

    def next_text(self, nx):
        roles = [b["role"] for b in self.map]
        name = lambda t: f"{t} {roles[t]}" if t < len(roles) else str(t)
        kind = nx.get("kind")
        if kind == "jump":
            return f"▶ {name(nx['to'])}"
        if kind == "branch":
            return f"if {nx['cond']} ▶{nx['then']} else ▶{nx['otherwise']}"
        if kind == "call":
            return f"{nx['result']} = block {nx['callee']}({', '.join(nx['args'])}) ▶{nx['back']}"
        return f"return {nx.get('value', '')}"

    def poll_activity(self):
        """While a program runs on this board's engine, and a moment after it ends (for its last frame), ask the
        board for the engine's last frame every ACTIVITY_S; once it has ended, ask for its totals as often, until
        they add up (`on_totals`) or TOTALS_WAIT_S has passed, which is said in the log."""
        now = time.time()
        run = self.run
        live = run is not None and (not run.get("done") or now < run.get("ended", 0) + 1.5)
        if self.engine_target != "term" or self.link.lost is not None:
            return False
        if live and now >= self.act_next:
            self.act_next = now + ACTIVITY_S
            self.link.send(bytes([self.t["host"]["activity"]]))
        if run is None or not run.get("done") or run.get("cycles") is None or not self.totals or self.whole is not None:
            return False
        if now < run["ended"] + TOTALS_WAIT_S:
            if now >= self.totals_next:
                self.totals_next = now + ACTIVITY_S
                self.link.send(bytes([self.t["host"]["totals"]]))
        elif "totals_off" in run and not run.get("totals_said"):
            run["totals_said"] = True
            board, blocks, cells = run["totals_off"]
            self.note(f"the board's run totals add up to {blocks:,} by block and {cells:,} by cell for a run it "
                      f"names {board:,} cycles, not the run's {run['cycles']:,} cycles + 1: its shares are not shown")
        return False

    def on_activity(self, payload):
        """An ACTIVITY report: kept when it is one frame's counts (its two frame numbers agree) and a new frame."""
        a = tl.activity(self.t, payload)
        if a is None or a["frame"] != a["frame_again"] or (self.act and a["frame"] == self.act["frame"]):
            return
        if self.act is None:
            self.note(f"activity: frame {a['frame']} · block {a['block']} · cell {a['cell']} · "
                      f"{sum(a['block_clocks'])} engine clocks in the frame")
        self.act = a
        self.act_hist = (self.act_hist + [a])[-8:]
        if sum(a["block_clocks"]):
            self.act_busy = a

    def on_totals(self, payload):
        """A TOTALS report: once the run has ended, the board names its cycles, and its block totals and its cell
        totals each add up to them and one, they are its whole profile, which its blocks and cells keep on screen."""
        run, got = self.run, tl.totals(self.t, payload)
        if got is None or run is None or not run.get("done") or run.get("cycles") is None or self.whole is not None:
            return
        sums = (sum(got["block_totals"]), sum(got["cell_totals"]))
        if not totals_hold(got, run["cycles"]):
            run["totals_off"] = (got["cycles"],) + sums
            return
        last = self.act_busy or self.act or {"frame": 0, "block": 0, "cell": 0, "depth": 0, "heap": 0}
        self.whole = dict(last, running=False, cycles=run["cycles"], whole=True,
                          block_clocks=got["block_totals"], cell_clocks=got["cell_totals"])
        n = len(self.map or [])
        roles = [b["role"] for b in self.map or []]
        shares = sorted(((c, i) for i, c in enumerate(got["block_totals"][:n]) if c), reverse=True)
        self.note(f"run totals: {sums[0]:,} engine clocks = {run['cycles']:,} cycles + 1, by block: "
                  + ", ".join(f"{i} {roles[i]} {c * 100 / sums[0]:.1f}%" for c, i in shares))
        self.draw()

    def shown_activity(self):
        """What the blocks' and cells' shares are drawn from: while a program runs, the board's last frame; once it
        has ended, the run's totals when the image keeps them and they are in, else the last frame that counted a
        clock — so a finished run's shares stay on screen."""
        if self.whole is not None:
            return self.whole
        a = self.act
        if a is not None and not a["running"] and not sum(a["block_clocks"]) and self.act_busy is not None:
            return dict(self.act_busy, running=False)
        return a

    def draw_help(self):
        s = self.screen
        lines = [
            "THE FABRIC TERMINAL",
            "",
            "Type LOGOS in the editor; F5 compiles it on the host,",
            "streams the image to the fabric engine on this board,",
            "and prints each Show as the chip sends it back.",
            "",
            "F2 opens the program library: Up/Down, Enter loads one.",
            "Esc stops a running program (the engine is reset).",
            "F3 demo mode: the library runs by itself until a key.",
            "F4 the fabric view: the program as the engine holds it.",
            "",
            "THE FABRIC: a program loads as up to 8 blocks of up",
            "to 32 cells; a cell is one operation, its inputs",
            "routed from registers, constants or earlier cells. The",
            "engine walks the block it is in, cell by cell, then",
            "the block's last step picks the next block. Colour is",
            "each block's and cell's share of the engine's clocks",
            "in the last video frame, read live from the board; the",
            "blinking cell is the one the engine is on.",
            "",
            "Any key closes this.",
        ]
        top, left, w = 4, 10, 60
        s.fill(top, left, len(lines) + 2, w, " ", WHITE, CYAN)
        for i, line in enumerate(lines):
            s.put(top + 1 + i, left + 2, line, BLACK if i else BLUE, CYAN)

    def flush(self):
        for f in self.screen.frames():
            if self.link:
                self.link.send(f)
        if self.local:
            self.local.draw(self.screen)

    # ── input ──────────────────────────────────────────────────────────────────────────────────────────────
    def note(self, what):
        if self.log:
            print(f"{time.strftime('%H:%M:%S')} {what}", file=self.log, flush=True)

    def key(self, k):
        self.note(f"key {k}")
        if self.demo is not None:
            self.stop_demo(f"key {k}")
            if k == "F3":
                return
        elif k == "F3":
            self.start_demo()
            return
        if self.help:
            self.help = False
            return
        if k in ("C-q",):
            self.quit = True
        elif k in ("F5", "C-r"):
            self.start_run()
        elif k == "Esc":
            if self.run and not self.run.get("done"):
                self.stop_run("stopped")
            elif self.focus == "library":
                self.focus = "editor"
        elif k == "F1":
            self.help = True
        elif k == "F4":
            if self.map:
                self.view = "source" if self.view == "fabric" else "fabric"
            else:
                self.status = ("no fabric map: F5 compiles the program", YELLOW)
        elif k == "F2":
            self.focus = "library" if self.focus == "editor" else "editor"
        elif self.focus == "library":
            if k == "Up":
                self.pick = max(0, self.pick - 1)
            elif k == "Down":
                self.pick = min(len(self.library.entries) - 1, self.pick + 1)
            elif k == "Home":
                self.pick = 0
            elif k == "End":
                self.pick = len(self.library.entries) - 1
            elif k == "Enter":
                self.load(self.pick)
        else:
            self.view = "source"
            self.editor.key(k)

    def click(self, pos):
        r, c = pos
        self.note(f"click row {r} col {c}")
        if self.demo is not None:
            self.stop_demo("a click")
        self.screen.set_pointer(pos if self.device == "mouse" or (self.osinput and self.osinput.mouse) else None)
        if self.help:
            self.help = False
            return
        base = EDIT_TOP + EDIT_ROWS - len(KEYS)
        if c >= SIDE_COL and base <= r < base + len(KEYS):
            key, _what, clickable = KEYS[r - base]
            if clickable:
                self.key(key)
            return
        first = self.library_first()
        shown = min(LIBRARY_ROWS, len(self.library.entries) - first)
        if c >= SIDE_COL and EDIT_TOP <= r <= EDIT_TOP + LIBRARY_ROWS and self.side == "blocks" and self.focus != "library":
            self.side = "programs"
        elif c >= SIDE_COL and EDIT_TOP + 1 <= r < EDIT_TOP + 1 + shown:
            self.pick = first + r - EDIT_TOP - 1
            self.load(self.pick)
        elif c < EDIT_COLS and EDIT_TOP <= r < EDIT_TOP + EDIT_ROWS:
            self.view = "source"
            e = self.editor
            n = e.top + r - EDIT_TOP
            if n < len(e.lines):
                e.row, e.col = n, max(0, min(len(e.lines[n]), c - GUTTER + e.left))
            self.focus = "editor"

    def load(self, i):
        title, path = self.library.entries[i][:2]
        self.note(f"load {title}")
        self.focus = "editor"
        try:
            source = self.library.source(i)
        except OSError as e:
            self.note(f"cannot read {title}: {path}: {e.strerror}")
            self.status = (f"cannot read the program of {title}", LRED)
            self.output = [(f"{os.path.basename(path)}: {e.strerror}", LRED), (path, DGREY)]
            return
        self.entry = i
        self.editor.load(source)
        self.view = "source"
        self.side = "programs"
        self.map = self.map_of(i)
        self.output = []
        self.status = ("ready", GREY)

    # ── demo mode ───────────────────────────────────────────────────────────────────────────────────────────
    # F3, or --demo: the terminal runs its library by itself the way a person at the keyboard would. The list opens
    # on the program just run, the highlight steps down one entry (back to the first after the last), Enter loads
    # it, F5 runs it on the fabric, and its output stays up a while; then the next. A key or click from a person
    # takes over at once.
    def start_demo(self):
        self.note("demo mode starts")
        self.demo = {"step": "open", "at": time.time(), "ran": False, "started": 0.0}

    def stop_demo(self, why):
        self.note(f"demo mode stops ({why})")
        self.demo = None

    def demo_tick(self):
        """Take demo mode's next step when its time has come; True when the screen changed."""
        d, now = self.demo, time.time()
        if d["step"] == "running":
            if self.run and not self.run.get("done") and now - d["started"] < DEMO_RUN_LIMIT_S:
                return False
            if self.run and not self.run.get("done"):
                self.stop_run("the demo's time limit")
            d["step"], d["at"] = "hold", now + DEMO_HOLD_S
            return True
        if now < d["at"]:
            return False
        step = d["step"]
        if step in ("open", "hold"):
            self.help = False
            self.focus = "library"
            d["step"], d["at"] = "move", now + DEMO_STEP_S
        elif step == "move":
            if d["ran"]:
                self.pick = (self.pick + 1) % len(self.library.entries)
            d["step"], d["at"] = "load", now + DEMO_STEP_S
        elif step == "load":
            d["step"], d["at"] = "run", now + DEMO_SHOW_S
            self.load(self.pick)
        elif step == "run":
            d["step"], d["started"], d["ran"] = "running", now, True
            self.start_run()
        return True

    # ── the device on the board's USB port ───────────────────────────────────────────────────────────────────
    # Told by what it answers (the Nexys A7 reference manual, §7.1): Read ID (0xF2) is answered 0xFA then 0x00 by a
    # mouse (0x03/0x04 by wheel mice) and 0xFA 0xAB 0x83 by a keyboard; a keyboard that leaves Read ID unanswered
    # answers Echo (0xEE) with 0xEE; a device plugged in reports its self-test, 0xAA, and a mouse its ID, 0x00, right
    # after. `asking` is (stage, deadline): a stage ending `?` is a question the board has not yet reported sent, and
    # bytes then predate it. A byte is an answer only in the stage that expects it, so neither a key (F5 is 0x03) nor
    # a packet already on its way is taken for one; any other byte names the device a keyboard and is its first key.
    def wait_for(self, stage):
        self.device = "identifying"
        self.asking = (stage, time.time() + ANSWER_S)

    def ask(self, cmd, stage):
        self.link.send(bytes([self.t["host"]["ps2_send"], cmd]))
        self.wait_for(stage + "?")

    def identified(self, device, why):
        self.note(f"device: {device} ({why})")
        self.device, self.asking, self.silence_said = device, None, False
        if device == "mouse":
            self.mouse.packet = []
            self.acks_due = 0
            self.command(0xF4)

    def command(self, cmd):
        """A command to the mouse, which acknowledges it (0xFA) between packets."""
        self.last_command = cmd
        self.acks_due += 1
        self.link.send(bytes([self.t["host"]["ps2_send"], cmd]))

    def ps2_sent(self, status):
        """The board sent the device the byte it was given; `status` is NO_ACK when no device clocked it in."""
        self.sends += 1
        if self.asking and self.asking[0].endswith("?"):
            if status:
                self.silent("device: none on the board's USB port (nothing takes a question)")
            else:
                self.wait_for(self.asking[0][:-1])
        elif status and self.acks_due:
            self.acks_due -= 1

    def ps2_tick(self):
        """Settle an identification whose answer did not come in time, and ask a silent port again every PROBE_S —
        the bridge may hold a keyboard whose self-test never reached this image (sent before it was configured), and
        such a keyboard answers Read ID; True when it moved on."""
        now = time.time()
        if self.device is None and not self.asking and self.link is not None and now >= self.probe_next:
            self.ask(0xF2, "read-id")
            return False
        if not self.asking or now < self.asking[1]:
            return False
        stage = self.asking[0].rstrip("?")
        if stage == "read-id":
            self.ask(0xEE, "echo")
        elif stage == "echo":
            self.silent("device: none answers on the board's USB port")
        else:
            self.identified("keyboard", f"nothing more after {stage}")
        return True

    def silent(self, said):
        """Nothing on the board's USB port answers: said once until something does, and asked again PROBE_S on."""
        if not self.silence_said:
            self.note(said)
            self.silence_said = True
        self.device, self.asking = None, None
        self.probe_next = time.time() + PROBE_S

    def ps2(self, b, damaged=False):
        """A PS/2 byte from the board (`damaged`: received with a parity or framing error): an answer while the
        device is being identified, else a byte of the identified device's stream."""
        if self.device == "mouse":
            if not self.mouse.packet and not damaged:
                if b == 0xAA:
                    self.wait_for("plugged")
                    return
                if self.acks_due and b in (0xFA, 0xFC, 0xFE):
                    if b == 0xFE:
                        self.link.send(bytes([self.t["host"]["ps2_send"], self.last_command]))
                    else:
                        self.acks_due -= 1
                    return
            ev = self.mouse.push(b, damaged)
            if ev:
                kind, pos = ev
                self.screen.set_pointer(pos)
                if kind == "click":
                    self.click(pos)
            return
        if damaged:
            if self.device != "identifying":
                self.keyboard.push(b, damaged=True)
            return
        if b == 0xAA:
            self.wait_for("plugged")
            return
        if self.device == "identifying":
            stage = self.asking[0]
            if stage.endswith("?"):
                return
            if stage == "read-id" and b == 0xFA:
                return self.wait_for("id")
            if stage == "read-id" and b in (0xFC, 0xFE):
                return self.ask(0xEE, "echo")
            if stage == "id" and b in (0x00, 0x03, 0x04):
                return self.identified("mouse", f"ID {b:02x}")
            if stage == "id" and b == 0xAB:
                return self.wait_for("keyboard-id")
            if stage == "keyboard-id" and b in (0x83, 0x41, 0xC1):
                return self.identified("keyboard", f"ID ab {b:02x}")
            if stage == "echo" and b == 0xEE:
                return self.identified("keyboard", "it answers Echo")
            if stage == "plugged" and b == 0x00:
                return self.identified("mouse", "ID 00 after its self-test")
            self.identified("keyboard", f"scan code {b:02x}")
        elif self.device is None:
            self.identified("keyboard", f"scan code {b:02x}")
        k = self.keyboard.push(b)
        if k:
            self.key(k)

    # ── running a program on the fabric ──────────────────────────────────────────────────────────────────────
    def start_run(self):
        if self.run and not self.run.get("done"):
            return
        self.output = []
        if self.link is None:
            self.status = ("no board: --headless shows the screen only", LRED)
            return
        if not self.engine:
            self.status = ("this image has no fabric engine: load nexys_fabric_terminal.bit", LRED)
            return
        entry = self.library.entries[self.entry] if self.entry is not None else None
        fn, args = (entry[2], entry[3]) if entry else ("f", "")
        try:
            stream, golden, facts = self.compiled(entry, fn, args)
            self.map = fabric_map_of(facts)
        except SystemExit as refused:
            self.note(f"refused: {str(refused).strip()[:200]}")
            self.status = ("refused", LRED)
            for line in str(refused).strip().splitlines()[-OUT_ROWS:]:
                self.output.append((line, LRED))
            return
        image = open(stream, "rb").read()
        self.note(f"run starts: {len(image)} bytes")
        self.act, self.act_hist, self.act_next = None, [], 0.0
        self.act_busy, self.whole, self.totals_next = None, None, 0.0
        self.side = "blocks"
        title = (entry[0] if entry else "untitled") + (" (edited)" if self.editor.changed else "")
        self.run = {"golden": golden, "facts": facts, "buf": b"", "shown": 0, "start": time.time(), "done": False,
                    "title": title, "image": hashlib.sha256(image).hexdigest()[:16]}
        self.output.append((f"image {len(image)} bytes · {facts.get('lifter', '?')} lifter · {facts.get('profile', '?')} profile", DGREY))
        if self.engine_target != "term":
            self.status = (f"running on {self.engine_board()}", YELLOW)
            self.draw()
            result, cycles, raw, status, shows = self.chip._run_one(self.engine_target, stream, 60.0, build=False)
            self.run["buf"] = raw
            for line in shows:
                self.output.append((f"> {line}", WHITE))
            self.finish(result, cycles, status, shows)
            return
        self.status = (f"loading {len(image)} bytes", YELLOW)
        self.draw()
        keep = []
        try:
            ok = self.link.run_engine(image, self.engine_bytes, timeout=0.0, keep=keep)
        except RuntimeError as e:
            self.stop_run(str(e))
            return
        for kind, payload in keep:
            self.report(kind, payload)
        if not ok and not self.run.get("done"):
            self.status = ("running on the fabric", YELLOW)

    def map_of(self, i):
        """Library entry `i`'s fabric map, compiling it (once: F5 runs what this compiled), or None when it is
        refused or the compiler gives none."""
        entry = self.library.entries[i]
        try:
            return fabric_map_of(self.compiled(entry, entry[2] or "f", entry[3])[2])
        except SystemExit as refused:
            self.note(f"no fabric map for {entry[0]}: {str(refused).strip()[:200]}")
            return None

    def compiled(self, entry, fn, args):
        """`compile`, once per program text: what a load compiled (to show its fabric map) F5 runs."""
        key = (self.editor.text(), entry[1] if entry else None, fn, args, self.engine_target, self.bit)
        if self.compile_cache.get("key") != key:
            self.compile_cache = {"key": key, "result": self.compile(entry, fn, args)}
        return self.compile_cache["result"]

    def compile(self, entry, fn, args):
        """The program's engine image, VM golden and facts. Unedited library programs come from the kit when
        there is one; anything else is compiled by `logos-chip.py`'s front door for Board B's engine."""
        bit = self.bit if self.engine_target == "term" else None
        if entry and not self.editor.changed:
            return compile_entry(self.chip, entry, self.work.name, bit, self.engine_target)
        if getattr(self.chip, "KIT", None) is not None:
            raise SystemExit("an edited program needs the compiler: run the terminal from the repo")
        path = os.path.join(self.work.name, "program.lg")
        with open(path, "w") as f:
            f.write(self.editor.text())
        main = any(l.strip() == "## Main" for l in self.editor.lines)
        return self.chip.compile_lg(self.engine_target, path, fn or "f", "" if main else args, self.work.name,
                                    bit_override=bit)

    def engine_bytes(self, payload):
        """Engine bytes during a run: show each new line, and finish when the result frame is whole."""
        run = self.run
        run["buf"] += bytes(payload)
        result, cycles, status, shows = self.chip.decode_wpe(self.chip.protocol(False), run["buf"])
        for line in shows[run["shown"]:]:
            self.output.append((f"> {line}", WHITE))
        run["shown"] = len(shows)
        if status in ("short", "nosync"):
            self.draw()
            return False
        self.finish(result, cycles, status, shows)
        return True

    def finish(self, result, cycles, status, shows):
        """A run's end: the verdict against the VM, the cycles, and an Int result on the 7-segment display."""
        self.note(f"run ends: {status}, result {result}, shows {shows}, {cycles} cycles, golden {self.run['golden']}")
        run = self.run
        run["done"], run["ended"] = True, time.time()
        run["cycles"] = cycles if status == "ok" else None
        golden = run["golden"]
        if status != "ok":
            self.status = (self.chip.describe(status, run["buf"]), LRED)
        elif isinstance(golden, list):
            same = shows == golden
            self.status = ("SILICON == VM" if same else "SILICON != VM", LGREEN if same else LRED)
        else:
            same = str(result) == str(golden)
            self.output.append((f"= {result}", WHITE))
            self.status = ("SILICON == VM" if same else "SILICON != VM", LGREEN if same else LRED)
        if status == "ok":
            us = cycles / 0.78125
            self.output.append((f"{cycles} engine cycles = {us:,.0f} us at 781 kHz", DGREY))
            self.history.add(run["title"], run["image"], self.engine_board(), cycles, self.status[0] == "SILICON == VM")
            said = self.history.verdict(self.history.last(run["image"]))
            self.note(f"history: {run['title']}: {said}")
            self.output.append((said, LGREEN if said.endswith("deterministic") else (YELLOW if "varied" in said else DGREY)))
            if not isinstance(golden, list):
                self.link.send(bytes([self.t["host"]["result"]]) + (int(result) & 0xFFFFFFFF).to_bytes(4, "big"))
        self.draw()

    def engine_board(self):
        """The board whose fabric runs the programs, as its name reads on screen."""
        name, part = self.chip.BOARDS[self.chip.TARGETS[self.engine_target]["serial"]][:2]
        return f"Board {name} {part.split()[-1]}"

    def stop_run(self, why):
        self.note(f"run stopped: {why}")
        if self.link:
            self.link.send(bytes([self.t["host"]["engine_reset"]]))
        if self.run:
            self.run["done"], self.run["ended"] = True, time.time()
        self.status = (why, LRED)

    # ── the board itself ─────────────────────────────────────────────────────────────────────────────────────
    # Once a second the host reads back the screen's corner cell, which it always draws (the title bar). A board
    # switched off and on comes back from flash with a blank text buffer, so a corner that reads back as anything
    # but what was drawn there means the board restarted: it gets the whole screen again, a run the restart cut off
    # is called out, and the device on its USB port is woken again (a mouse comes back with reports disabled). A
    # UART that went away with the board's power is reopened once the board is back, and the board taken as
    # restarted.
    def watch_board(self):
        """True when the board was found restarted and the screen must be drawn again."""
        now = time.time()
        if now < self.watch["next"]:
            return False
        self.watch["next"] = now + WATCH_S
        if self.link.lost is not None:
            if not self.watch["gone"]:
                self.note(f"the board's UART went away ({self.link.lost}): waiting for it")
                self.watch["gone"] = True
            if not self.link.reopen():
                return False
            self.restarted("its UART came back")
            return True
        if self.watch["expect"] is not None:
            self.watch["missed"] += 1
            if self.watch["missed"] == 3:
                self.note("the board does not answer")
        corner = self.screen.have[0][0]
        if corner is not None:
            self.watch["expect"] = corner.bytes()
            self.link.send(bytes([self.t["host"]["read"], 0, 0, 1]))
        self.link.send(bytes([self.t["host"]["ping"]]))
        return False

    def take_features(self, pong):
        """What the image that answered `pong` draws and keeps beyond the screen: the small-text window (the side
        list's blocks are drawn in it) and the run's totals (the shares a finished run leaves on screen) — neither
        without a PONG that says so."""
        has = lambda f: pong is not None and self.link is not None and f(pong)
        self.small = self.screen.small_enabled = has(lambda p: self.link.has_small(p))
        self.totals = has(lambda p: self.link.has_totals(p))

    def pong(self, payload):
        """The board's PONG: what its image holds, and what its PS/2 line carried since the last one, in the log."""
        self.engine = self.link.has_engine(payload)
        self.take_features(payload)
        line, last = self.link.ps2_line(payload), self.line
        self.line = line
        sends, self.sends = self.sends, 0
        if line is None or last is None:
            return
        edges = (line["edges"] - last["edges"]) % 65536
        whole = (line["bytes"] - last["bytes"]) % 65536
        damaged = max(0, line["damaged"] - last["damaged"])
        if edges and not (whole == damaged == 0 and edges == PS2_BYTE_EDGES * sends):
            low = line["shortest_low_us"]
            self.note(f"board USB line: +{edges} clock edges, +{whole} bytes read whole, +{damaged} damaged "
                      f"(shortest low phase {'none timed' if low is None else f'{low} us'})"
                      + ("" if whole or damaged else ": the bridge clocks but no byte is read"))

    def cells(self, payload):
        """The corner cell the watch read back."""
        expect = self.watch["expect"]
        if expect is None or len(payload) < 4 or payload[0] != 0 or payload[1] != 0:
            return
        if self.watch["missed"] >= 3:
            self.note("the board answers again")
        self.watch["expect"], self.watch["missed"] = None, 0
        if bytes(payload[2:4]) != expect:
            self.restarted("its screen came back blank")

    def restarted(self, why):
        """The board restarted: the screen is drawn again, whole — without the small-text window or the run's totals
        until the image it came back with says in its PONG that it has them."""
        self.note(f"the board restarted ({why}): drawing the whole screen again")
        self.watch.update(expect=None, missed=0, gone=False)
        self.line = None
        self.screen.forget()
        self.take_features(None)
        if self.run and not self.run.get("done"):
            self.run["done"] = True
            self.status = ("the board restarted during the run: run it again", LRED)
        self.keyboard = Keyboard()
        self.link.send(bytes([self.t["host"]["ping"]]))
        if self.device == "mouse":
            self.mouse.packet = []
            self.acks_due = 0
            self.command(0xF4)
        else:
            self.ask(0xF2, "read-id")

    def report(self, kind, payload):
        b = self.t["board"]
        if kind == b["ps2"]:
            self.ps2(payload[0], damaged=payload[1] != 0)
        elif kind == b["ps2_sent"]:
            self.ps2_sent(payload[0])
        elif kind == b["cells"]:
            self.cells(payload)
        elif kind == b["pong"]:
            self.pong(payload)
        elif kind == b["activity"]:
            self.on_activity(payload)
        elif kind == b["totals"]:
            self.on_totals(payload)
        elif kind == b["engine"] and self.run and not self.run.get("done"):
            self.engine_bytes(payload)

    def safely(self, what, fn, *args):
        """Run one handler: a fault costs that event only — it is logged with its traceback, said on the screen, and
        the terminal runs on."""
        try:
            return fn(*args)
        except Exception:
            self.note(f"fault in {what}: " + " | ".join(traceback.format_exc().strip().splitlines()))
            self.status = ("an internal fault (in the log): the terminal runs on", LRED)
            self.faulted = True
            return None

    def input_events(self):
        """This computer's keyboards and mice: what was plugged in, unplugged or refused, then each key and click."""
        events = self.osinput.read()
        for names, said in ((self.osinput.added, "input from this computer: "), (self.osinput.removed, "unplugged: "),
                            (self.osinput.problems, "cannot read an input device: ")):
            while names:
                self.note(said + names.pop(0))
        return events

    def loop(self):
        if self.entry is not None and self.map is None:
            self.map = self.safely("compiling the first program", self.map_of, self.entry)
        self.safely("drawing", self.draw)
        if self.link:
            self.ask(0xF2, "read-id")
        while not self.quit:
            changed = False
            if self.link:
                deadline = time.time() + 0.03
                while time.time() < deadline:
                    try:
                        kind, payload = self.link.events.get(timeout=0.01)
                    except Exception:
                        continue
                    self.safely("a report from the board", self.report, kind, payload)
                    changed = True
                if self.safely("identifying the board's USB device", self.ps2_tick):
                    changed = True
                if self.safely("watching the board", self.watch_board):
                    changed = True
            else:
                time.sleep(0.03)
            if self.demo is not None and self.safely("demo mode", self.demo_tick):
                changed = True
            if self.link and self.safely("asking the board for its activity", self.poll_activity):
                changed = True
            sources = []
            if self.local:
                sources += self.safely("this terminal's keys", self.local.read) or []
            if self.osinput:
                sources += self.safely("this computer's keyboards", self.input_events) or []
            for kind, v in sources:
                if kind == "key":
                    self.safely(f"key {v}", self.key, v)
                elif kind == "click":
                    self.safely("a click", self.click, v)
                else:
                    self.screen.set_pointer(v)
                changed = True
            if changed or self.faulted:
                self.faulted = False
                self.safely("drawing", self.draw)

HISTORY_KEEP = 10


class RunHistory:
    """Every run on the fabric, one JSON line each in `LOGOS_TERMINAL_HISTORY`, else `logs/fabric-runs.jsonl`: when,
    the program's title, the sha256 of the image the engine ran (an edited program is another image), the board it
    ran on, the cycles it took, and whether it equalled the VM. The same image takes the same cycles every time on
    the fabric; `verdict` says whether its last runs did."""

    def __init__(self, path=None):
        self.path = path or os.environ.get("LOGOS_TERMINAL_HISTORY") or os.path.join(REPO, "logs", "fabric-runs.jsonl")

    def add(self, title, image, board, cycles, same):
        os.makedirs(os.path.dirname(self.path) or ".", exist_ok=True)
        with open(self.path, "a") as f:
            f.write(json.dumps({"time": time.strftime("%Y-%m-%dT%H:%M:%S"), "title": title, "image": image,
                                "board": board, "cycles": cycles, "same": same}) + "\n")

    def runs(self):
        out = []
        try:
            lines = open(self.path).read().splitlines()
        except OSError:
            return out
        for line in lines:
            try:
                out.append(json.loads(line))
            except ValueError:
                continue
        return out

    def last(self, image):
        """The most recent HISTORY_KEEP runs of `image`, oldest first."""
        return [r for r in self.runs() if r.get("image") == image][-HISTORY_KEEP:]

    @staticmethod
    def verdict(last):
        cycles = [r["cycles"] for r in last]
        if len(cycles) < 2:
            return "the first run of this image on the fabric"
        if len(set(cycles)) == 1:
            return f"{cycles[0]:,} cycles in each of the last {len(cycles)} runs: deterministic"
        return f"cycles varied over the last {len(cycles)} runs: {min(cycles):,} to {max(cycles):,}"

    def report(self):
        """Each image's last runs, a line each: its title, the cycle counts, and whether they agree."""
        by_image = {}
        for r in self.runs():
            by_image.setdefault(r["image"], []).append(r)
        lines = [f"the last {HISTORY_KEEP} runs of each program on the fabric ({self.path}):"]
        for image, runs in by_image.items():
            last = runs[-HISTORY_KEEP:]
            cycles = [r["cycles"] for r in last]
            agree = "deterministic" if len(set(cycles)) == 1 else f"varied: {min(cycles):,} to {max(cycles):,}"
            title = last[-1]["title"]
            lines.append(f"  {title:<18} {len(runs):>4} runs   {' '.join(f'{c:,}' for c in cycles)}   {agree}"
                         + ("" if all(r["same"] for r in last) else "   (a run did not equal the VM)"))
        return "\n".join(lines)


def fit(parts, width):
    """`parts` joined by " · ", as many of them as fit `width`, the first ones kept."""
    text = ""
    for part in parts:
        longer = f"{text} · {part}" if text else part
        if len(longer) > width:
            break
        text = longer
    return text or parts[0][:width]


def spark(clocks, total):
    """One frame of a block's history in its title: its share of the frame's clocks, from nothing to all."""
    if clocks == 0 or total == 0:
        return "·"
    share = clocks / total
    return "░" if share < 0.1 else "▒" if share < 0.3 else "▓" if share < 0.6 else "█"


def compact(statement):
    """A fabric map statement as the fabric view prints it: `x = x + e` as `x += e`."""
    name, eq, rest = statement.partition(" = ")
    if eq:
        for op in ("+", "-", "*"):
            if rest.startswith(f"{name} {op} "):
                return f"{name} {op}= {rest[len(name) + 3:]}"
    return statement


def wrap(text, width):
    """`text` in lines of at most `width`, broken at spaces where it can be; none for no text."""
    return textwrap.wrap(text, width, break_on_hyphens=False) if text else []


def totals_hold(got, cycles):
    """Whether a TOTALS report (`tl.totals`) is a finished run of `cycles` cycles, whole: the board names those
    cycles, and its block totals and its cell totals each add up to them and one (the clock that sees it done)."""
    return got is not None and got["cycles"] == cycles and \
        sum(got["block_totals"]) == cycles + 1 and sum(got["cell_totals"]) == cycles + 1


def first_fit(texts, width):
    """The first of `texts` no longer than `width` (the last cut to it when none is)."""
    return next((x for x in texts if len(x) <= width), texts[-1][:width])


def pack(items, width, sep):
    """`items` in lines of at most `width`, as many whole ones to a line as fit with `sep` between them; only an
    item too long for a line of its own is broken, at its spaces where it can be."""
    lines = []
    for item in items:
        parts = wrap(item, width)
        if lines and len(parts) == 1 and len(lines[-1]) + len(sep) + len(parts[0]) <= width:
            lines[-1] += sep + parts[0]
        else:
            lines += parts
    return lines


def share_rows(total, needs):
    """Rows for boxes stacked in `total` rows, each needing `needs[i]` (two at least, its title and foot): every box
    gets two; the rest go a row at a time to the box with the fewest that needs more (the first of equals), so every
    box gets a row of what it says before any gets a second, until each has all it needs or none are left."""
    rows = [2] * len(needs)
    left = total - 2 * len(needs)
    while left > 0:
        short = [(rows[i], i) for i in range(len(needs)) if rows[i] < needs[i]]
        if not short:
            break
        rows[min(short)[1]] += 1
        left -= 1
    return rows


def fabric_map_of(facts):
    """The fabric map in a compile's facts (`emit_wpe_soc`'s `map_json`): per block its role, first cell, cells'
    operations and pseudo-code — or None when the facts carry none (a kit built without it)."""
    try:
        return json.loads(facts["map_json"])["blocks"]
    except (KeyError, ValueError, TypeError):
        return None


def os_input_choice(input_arg, no_input, tty):
    """Which of this computer's keyboards and mice the terminal reads, as `OsInput`'s name match: those `--input
    NAME` names; with no terminal window to type in, every one, unasked; with `--no-input`, none (None)."""
    if no_input:
        return None
    if input_arg is not None:
        return input_arg
    return None if tty else ""


def compile_entry(chip, entry, work, bit=None, target="term"):
    """A library entry's engine image, VM golden and facts for `target`'s engine (the terminal's own by default):
    from the kit when there is one, else compiled into the directory `work` by `logos-chip.py`'s front door —
    checked against the caps of `bit` when the board holds an image other than the target's own."""
    _title, source, fn, args = entry
    path = os.path.normpath(os.path.join(REPO, source))
    if getattr(chip, "KIT", None) is not None:
        return chip.kit_program(target, path, fn or "f", args)
    return chip.compile_lg(target, path, fn or "f", args, work, bit_override=bit)


def check(t, chip, link, entries, wait=60.0, bit=None):
    """Board B with no one at it: the link answers as this table's terminal; every cell of the screen, written with
    every glyph, colour and blink, reads back as written — and on an image with the small plane, every one of its
    cells too, the screen's left as they were; and each library program in `entries` runs on the fabric engine and
    answers what the VM answers — and on an image that keeps run totals, they add up to the run's cycles and one.
    Prints a line per check; returns True when all of them hold."""
    oks = []

    def report(ok, what):
        print(f"  {'ok ' if ok else 'BAD'} {what}", flush=True)
        oks.append(bool(ok))

    rows, cols = t["screen"]["rows"], t["screen"]["cols"]
    pong = link.ping()
    report(pong is not None and list(pong[:4]) == [t["version"], cols, rows, t["screen"]["panel_rows"]],
           f"the board answers as Fabric Terminal v{t['version']}, {cols} x {rows}")
    if pong is not None:
        print(f"  --  PS/2 line since the image started: {tl.describe_ps2_line(link.ps2_line(pong))}", flush=True)
    if pong is not None and not link.has_engine(pong):
        print("  --  the image has no fabric engine (the screen-and-keyboard shell): no program is run", flush=True)
        entries = []

    screen = tl.Screen(t)
    for r in range(rows):
        for c in range(cols):
            screen.want[r][c] = tl.Cell((r * cols + c) % 128, (r + c) % 16, (r // 2 + c // 5) % 16, (r * cols + c) % 7 == 0)
    for f in screen.frames():
        link.send(f)
    wrong = []
    for r in range(rows):
        link.send(bytes([t["host"]["read"], r, 0, cols]))
        got = link.wait(t["board"]["cells"], 2.0)
        want = bytes([r, 0]) + b"".join(screen.want[r][c].bytes() for c in range(cols))
        if got != want:
            wrong.append(r)
    report(not wrong, f"every cell of the screen reads back as written (rows wrong: {wrong[:8]})")
    if pong is not None and link.has_small(pong):
        first, srows, scols = screen.small_first, screen.small_rows, screen.small_cols
        screen.small_enabled = True
        for r in range(srows):
            for c in range(scols):
                screen.small_want[r][c] = tl.Cell((r * scols + c + 64) % 128, (r + c + 3) % 16, (r + c // 3) % 16,
                                                  (r * scols + c) % 5 == 0)
        for f in screen.frames():
            link.send(f)
        wrong = []
        for r in range(srows):
            link.send(bytes([t["host"]["read"], first + r, 0, scols]))
            got = link.wait(t["board"]["cells"], 2.0)
            want = bytes([first + r, 0]) + b"".join(screen.small_want[r][c].bytes() for c in range(scols))
            if got != want:
                wrong.append(r)
        report(not wrong, f"every cell of the small plane reads back as written (small rows wrong: {wrong[:8]})")
        moved = []
        for r in range(rows):
            link.send(bytes([t["host"]["read"], r, 0, cols]))
            if link.wait(t["board"]["cells"], 2.0) != bytes([r, 0]) + b"".join(screen.want[r][c].bytes() for c in range(cols)):
                moved.append(r)
        report(not moved, f"and writing it left the screen's cells as they were (rows changed: {moved[:8]})")
        link.send(bytes([t["host"]["fill"], first, 0, srows, scols, 0x07, 0x20]))
    link.send(bytes([t["host"]["fill"], 0, 0, rows, cols, 0x07, 0x20]))

    work = tempfile.TemporaryDirectory(prefix="logos-term-")
    for entry in entries:
        title = entry[0]
        try:
            stream, golden, _facts = compile_entry(chip, entry, work.name, bit)
        except SystemExit as refused:
            report(False, f"{title}: {str(refused).strip()}")
            continue
        buf = bytearray()
        decoded = []

        def take(payload):
            buf.extend(payload)
            got = chip.decode_wpe(chip.protocol(False), bytes(buf))
            if got[2] in ("short", "nosync"):
                return False
            decoded.append(got)
            return True

        try:
            done = link.run_engine(open(stream, "rb").read(), take, timeout=wait)
        except RuntimeError as e:
            report(False, f"{title}: {e}")
            continue
        if not done:
            report(False, f"{title}: no whole reply in {wait:.0f} s")
            continue
        result, cycles, status, shows = decoded[0]
        same = status == "ok" and (shows == golden if isinstance(golden, list) else str(result) == str(golden))
        shown = " | ".join(shows) if isinstance(golden, list) else str(result)
        report(same, f"{title}: {'SILICON == VM' if same else 'SILICON != VM'} ({shown}; {cycles} cycles)")
        if status == "ok" and pong is not None and link.has_totals(pong):
            got, deadline = None, time.time() + 1.0
            while time.time() < deadline and not totals_hold(got, cycles):
                link.send(bytes([t["host"]["totals"]]))
                got = tl.totals(t, link.wait(t["board"]["totals"], 2.0) or b"")
                time.sleep(0.02)
            said = "no TOTALS answer" if got is None else \
                f"a run it names {got['cycles']:,} cycles; {sum(got['block_totals']):,} and {sum(got['cell_totals']):,}"
            report(totals_hold(got, cycles),
                   f"{title}: the board's run totals add up to its {cycles:,} cycles + 1, by block and by cell ({said})")
    work.cleanup()
    return all(oks)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", help="Board B's UART: a device or a board letter (default: found by its FTDI serial)")
    ap.add_argument("--flash", action="store_true", help="load nexys_fabric_terminal.bit into Board B first")
    ap.add_argument("--bit", help="the image on the board when it is not nexys_fabric_terminal.bit (a seed being "
                    "validated): --flash loads it, and programs are checked against its caps")
    ap.add_argument("--check", action="store_true", help="check the board end to end with no one at it; exit 1 if not")
    ap.add_argument("--input", nargs="?", const="", default=None, metavar="NAME",
                    help="also read keyboards and mice plugged into this computer (Linux, the input group); with no "
                         "terminal window this is the default. NAME picks devices by name (default: every one)")
    ap.add_argument("--no-input", action="store_true", help="read no keyboard or mouse plugged into this computer")
    ap.add_argument("--engine", default="term", metavar="TARGET",
                    help="the logos-chip.py target whose engine runs the programs: the terminal's own (term, the default, "
                         "its activity drawn on the panel) or an engine on another board, e.g. wpe200 (Board A)")
    ap.add_argument("--headless", action="store_true", help="no board: draw the screen in this terminal only")
    ap.add_argument("--no-mirror", action="store_true", help="do not draw the screen in this terminal")
    ap.add_argument("--selftest", action="store_true", help="check the keyboard decoder and editor, no board")
    ap.add_argument("--demo", action="store_true", help="start in demo mode: the library runs by itself until a key")
    ap.add_argument("--history", action="store_true",
                    help="print each program's last ten runs on the fabric, their cycles and whether they agree")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.history:
        print(RunHistory().report())
        return 0
    chip = logos_chip()
    t = tl.table()
    if a.engine not in chip.TARGETS or chip.TARGETS[a.engine]["frame"] not in ("wpe", "term"):
        sys.exit(f"--engine {a.engine}: not a fabric-engine target ({', '.join(k for k, v in chip.TARGETS.items() if v['frame'] in ('wpe', 'term'))})")
    if a.engine != "term" and chip.TARGETS[a.engine]["serial"] == chip.TARGETS["term"]["serial"]:
        sys.exit(f"--engine {a.engine}: that engine is on the board that draws the screen; use an engine on the other board")
    if a.flash and not a.headless:
        chip.flash("term", a.bit)
    link = pong = None
    if not a.headless:
        link = tl.Link(t, a.port)
        pong = link.ping()
        if pong is None:
            link.close()
            sys.exit(f"Board B on {link.port} does not answer — load the terminal image: "
                     f"python3 scripts/logos-terminal.py --flash")
        if getattr(chip, "KIT", None) is None and (link.has_engine(pong) or a.engine != "term"):
            # Every run compiles through these; building them now keeps a fresh checkout's first F5 from freezing
            # the screen behind cargo.
            print("building the compiler (once a checkout)…", flush=True)
            chip.compiler("emit_wpe_soc")
    if a.check:
        try:
            ok = check(t, chip, link, Library(chip).entries, bit=a.bit)
        finally:
            link.close()
        print("logos-terminal check: " + ("OK" if ok else "FAILED"))
        return 0 if ok else 1
    local = LocalTerminal(mirror=not a.no_mirror) if sys.stdin.isatty() else None
    try:
        engine = link is None or a.engine != "term" or link.has_engine(pong)
        term = Terminal(t, chip, link, local, a.bit, engine=engine, engine_target=a.engine,
                        log=None if local else sys.stdout, pong=pong)
        if a.demo:
            term.start_demo()
        choice = os_input_choice(a.input, a.no_input, local is not None)
        if choice is not None:
            term.osinput = OsInput(choice)
            if not term.osinput.devices and not term.osinput.problems:
                term.note("no keyboard or mouse on this computer yet: plug one in and it is picked up")
        term.loop()
    finally:
        if local:
            local.restore()
        if link:
            link.close()
    return 0


def selftest():
    checks = []

    def check(label, cond):
        checks.append(cond)
        print(f"  {'ok ' if cond else 'BAD'} {label}")

    kb = Keyboard()
    typed = [k for b in [0x12, 0x33, 0xF0, 0x12, 0x43, 0xF0, 0x43, 0x29, 0xE0, 0x75, 0xE0, 0xF0, 0x75, 0x14, 0x2D,
                         0xF0, 0x2D, 0xF0, 0x14, 0x03, 0x12, 0x16, 0xF0, 0x16, 0xF0, 0x12] for k in [kb.push(b)] if k]
    check("set-2 scan codes: shift, release, extended arrows, Ctrl+R, F5, shifted digit",
          typed == ["H", "i", " ", "Up", "C-r", "F5", "!"])
    m = Mouse()
    events = [e for b in [0x09, 0x08, 0x00, 0x28, 0x10, 0xF0] for e in [m.push(b)] if e]
    check("mouse packets: a left click 8 px right, then a move 16 px right and 16 px down (dy sign bit)",
          events == [("click", (15, 41)), ("move", (16, 43))])
    e = Editor("## Main\n    Show 1.")
    for k in ["Down", "End", "Enter"] + list("Show 2."):
        e.key(k)
    check("editor: Enter keeps the indent, typing inserts", e.text() == "## Main\n    Show 1.\n    Show 2.\n")
    for k in ["Home", "Backspace"]:
        e.key(k)
    check("editor: Backspace at a line's start joins it to the one above", e.lines[1] == "    Show 1.    Show 2.")
    check("highlight: headers, keywords, numbers, strings",
          highlight("## Main")[0] == LCYAN and highlight('Show "x" 42')[:4] == [LGREEN] * 4
          and highlight('Show "x" 42')[5] == YELLOW and highlight('Show "x" 42')[9] == LMAGENTA)
    os_in = OsInput.__new__(OsInput)
    os_in.x, os_in.y, os_in.shift, os_in.ctrl, os_in.caps = 320, 240, False, False, False
    K, R = OsInput.EV_KEY, OsInput.EV_REL
    got = os_in.translate([(K, 42, 1), (K, 35, 1), (K, 35, 0), (K, 42, 0), (K, 23, 1), (K, 23, 2), (K, 103, 1),
                           (K, 29, 1), (K, 19, 1), (K, 29, 0), (K, 63, 1), (K, 2, 1),
                           (R, OsInput.REL_X, 16), (R, OsInput.REL_Y, -32), (K, OsInput.BTN_LEFT, 1), (K, OsInput.BTN_LEFT, 0)])
    check("this computer's keyboard and mouse: shift, autorepeat, an arrow, Ctrl+R, F5, a digit, a move and a click",
          got == [("key", "H"), ("key", "i"), ("key", "i"), ("key", "Up"), ("key", "C-r"), ("key", "F5"), ("key", "1"),
                  ("move", (15, 42)), ("move", (13, 42)), ("click", (13, 42))])
    check("the fabric view writes `x = x + e` as `x += e`, and leaves other statements as they are",
          [compact(x) for x in ("total = total + (i * i)", "k = k - 1", "n = (3 * n) + 1", "show total", "a = b")]
          == ["total += (i * i)", "k -= 1", "n = (3 * n) + 1", "show total", "a = b"])
    check("a status line keeps as many of its parts as fit, the first ones first",
          fit(["LIVE", "block 2 loop", "cell 1 +", "depth 0"], 25) == "LIVE · block 2 loop" and fit(["toolong"], 4) == "tool")
    check("statements pack whole into lines, as many to a line as fit; only one too long for a line breaks",
          pack(["a = 1", "b = 2", "show total"], 12, "; ") == ["a = 1; b = 2", "show total"]
          and pack(["show long text"], 6, "; ") == ["show", "long", "text"] and pack([], 6, "; ") == []
          and pack(["a", "bb ccccccccccc"], 12, "; ") == ["a", "bb", "ccccccccccc"])
    check("the side list's boxes all get a title and foot, then a row each in turn while any needs more",
          share_rows(21, [5, 4, 3, 6, 6, 5, 3]) == [3, 3, 3, 3, 3, 3, 3]
          and share_rows(20, [4, 5, 5, 3, 3]) == [4, 5, 5, 3, 3] and share_rows(20, [9]) == [9]
          and share_rows(16, [2] * 8) == [2] * 8 and share_rows(9, [6, 6, 3]) == [3, 3, 3])
    n = LIBRARY_ROWS
    check("the library list scrolls just far enough to show the entry picked",
          [library_window(p, n + 5) for p in (0, n - 1, n, n + 4)] == [0, 0, 1, 5]
          and library_window(3, n - 2) == 0)
    good = all(checks)
    print("logos-terminal selftest: " + ("OK" if good else "FAILED"))
    return 0 if good else 1


if __name__ == "__main__":
    sys.exit(main())
