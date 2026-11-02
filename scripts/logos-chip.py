#!/usr/bin/env python3
"""logos-chip — send LOGOS code over the wire to the spatial compute fabric on an FPGA and read the answer back.

THE DEMO, one command, no hardware knowledge required:

    scripts/logos-chip.py run wpe200 benchmarks/silicon/programs/hello_fabric.lg --flash

That compiles the LOGOS SOURCE on this PC (a whole `## Main` program, functions and all, into the whole-program
fabric engine's flat image), derives the VM's golden output from the same source, streams the bytes over the
board's byte channel, prints every `Show` as the chip streams it back, and ends with `SILICON == VM` — or exactly
how they differ. A function-form program (`## To f …`, no `## Main`) runs `f(--args)` and checks one result.
The chip SYNTHESIZES the program it is handed: nothing is rebuilt or re-flashed per program; only the bytes on
the wire change.

Two boards are attached, they are NOT the same part, and almost everything about talking to them differs:

    Board A  Nexys Video      XC7A200T  cable digilent_b  JTAG on channel B, byte channel on A (an FT245 FIFO)
    Board B  Nexys A7-100T    XC7A100T  cable digilent    JTAG on channel A, byte channel on B (a UART bridge)

Getting any one of those wrong looks identical to dead hardware — `--detect` prints "empty", or the loader
never syncs and no reply ever comes. Both have cost real debugging time, so this script derives the ports from
the FTDI serial + USB interface number (`scripts/lib/ftdi_ports.py`, the same on Linux and macOS — never a
hard-coded /dev/ttyUSBn, which renumbers on re-enumeration) and keeps the per-target bitstream, baud, core clock
and ENGINE PROFILE in ONE table instead of spread across a dozen call sites.

    scripts/logos-chip.py detect
    scripts/logos-chip.py flash wpe200
    scripts/logos-chip.py run wpe200 prog.lg [--flash]                            a ## Main program, checked vs the VM
    scripts/logos-chip.py run wpe200 prog.lg --fn f --args a,b                     a function-form program
    scripts/logos-chip.py run wpe    benchmarks/silicon/programs/wpe_loop.lg --args 5
    scripts/logos-chip.py compile wpe200 prog.lg --args 8 --out /tmp/out           just the bytes + golden, no board
    scripts/logos-chip.py run wpe200 benchmarks/silicon/programs/wpe_loop.wire --expect 15   a pre-built stream
    scripts/logos-chip.py run core benchmarks/silicon/programs/gcd.lg --flash      soft-core bytecode, checked vs the VM
    scripts/logos-chip.py run core /tmp/gcd.hex --expect 1335                                 soft-core bytecode words
    scripts/logos-chip.py suite wpe200 --manifest benchmarks/silicon/programs/s200_manifest.txt
    scripts/logos-chip.py selftest                                                 the reply decoders, no hardware
    scripts/logos-chip.py kit OUT wpe prog.lg wpe_fib_rec.lg@8                     export a board kit (below)

A BOARD KIT drives the boards from a machine with no Rust toolchain — a laptop with Python, pyserial and
openFPGALoader. `kit OUT TARGET PROGRAM[@ARGS]…` writes a sparse copy of this repo into OUT: the host scripts,
the target's image and build recipe, each program compiled for the target with its VM golden (recorded in
`OUT/kit.json`), and the chip protocol's byte table as `chip_host --table` prints it. Run `kit` once per target
into the same OUT. Inside OUT the same commands work with no compiler — `run wpe prog.lg` sends the stream the
kit carries and checks it against the recorded golden; anything the kit does not carry is refused by name.

Targets:
    wpe      Board B  the config-driven whole-program FABRIC engine, flip-flop profile  (.lg / .wire)
    term     Board B  the same engine inside the Fabric Terminal image, over the terminal's link (.lg / .wire)
    wpe200   Board A  the same engine on the 200T, config in block RAM                 (.lg / .wire)
    core     Board B  the soft-core CPU — the sequential interpreter on silicon         (.lg / .hex, bytecode words)

A program compiled for one engine profile is REFUSED for the other when it does not fit: a `.wire` carries no
record of the profile it was built for, and an over-cap image loads fine and returns a plausible wrong number.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIL = os.path.join(REPO, "benchmarks", "silicon")

sys.path.insert(0, os.path.join(REPO, "scripts", "lib"))
import ftdi_ports  # noqa: E402
from ftdi_ports import BOARDS  # noqa: E402

# A board kit is marked by this file at its root; this repo never has one. Inside a kit the host reads the
# protocol table and every program's stream and golden from the kit, and never runs the compiler.
KIT_FILE = os.path.join(REPO, "kit.json")
KIT = json.load(open(KIT_FILE)) if os.path.exists(KIT_FILE) else None

# What every kit carries besides its programs: the host scripts a kit machine runs, and the two wires the
# selftest's wire-parser checks read.
HOST_SCRIPTS = (
    "scripts/logos-chip.py",
    "scripts/nexys-host.py",
    "scripts/lib/ftdi_ports.py",
    "scripts/lib/terminal_link.py",
    "scripts/logos-terminal.py",
    "scripts/run-tformer-200t.py",
    "scripts/matmul-host.py",
)
SELFTEST_WIRES = ("benchmarks/silicon/programs/bench_pi_leibniz.wire", "benchmarks/silicon/programs/wpe_loop.wire")

# target -> board serial, bitstream, baud, core clock MHz, reply-frame dialect, engine profile.
# baud/clock are NOT free parameters: they follow the divider and CLKS_PER_BIT compiled into the board top.
# nexys_wpe_soc.v divides /128 -> 781.25 kHz with CLKS_PER_BIT=81, i.e. 9600 baud. Passing the "obvious"
# 115200 there produces no reply at all. `profile` names the engine caps a LOGOS program must fit before it is
# streamed (`emit_wpe_soc --profile`): `hw` = the XC7A100T flip-flop engine, `stream` = the XC7A200T block-RAM one.
TARGETS = {
    # The 100T image carries the IEEE-754 FPU since 2026-09-19 (seed 3 of the hold-lottery sweep; its recipe says
    # FPU=1, so the host lets float programs through). The profile's other caps are the flip-flop budget's.
    "wpe": dict(serial="210292BFA310", bit="wpe_soc.bit", baud=9600, clock=0.78125, frame="wpe", profile="hw"),
    # The 200T engine profile: MAXCELL=1024 / MAXARG=24 / DEPTH=32, sized from the MEASURED benchmark ceiling, with
    # the frame-stack overflow guard. That image has not routed yet: the file at `wpe200`'s path is byte-identical to
    # `wpe200old`, the DEPTH=4 image the whole-transformer "mat" run is proven on, built before the guard and before
    # Show records. It carries no build recipe, so the host refuses a `## Main` program for it, and a recursion
    # deeper than 4 frames answers wrong (bench_fib 21 -> 8, reported SILICON != VM).
    "wpe200": dict(
        serial="210276C08FE0", bit="nexys_video_wpe_stream.bit", baud=115200, clock=0.78125, frame="wpe",
        profile="stream",
    ),
    "wpe200old": dict(
        serial="210276C08FE0", bit="nexys_video_wpe_bank_sd8.bit", baud=115200, clock=0.78125, frame="wpe",
        profile="stream",
    ),
    "core": dict(serial="210292BFA310", bit="full_core_all35.bit", baud=115200, clock=6.25, frame="core",
                 profile=None),
    # The Fabric Terminal on Board B: the same engine profile as `wpe`, reached through the terminal's link
    # (`scripts/lib/terminal_link.py`) beside the screen and keyboard it serves (`scripts/logos-terminal.py`).
    "term": dict(serial="210292BFA310", bit="nexys_fabric_terminal.bit", baud=1_000_000, clock=0.78125,
                 frame="term", profile="hw"),
}

# The compiler examples the LOGOS-source path shells out to. Built ONCE per invocation through cargo (an
# up-to-date tree costs a second; a stale binary would silently encode yesterday's compiler), unless --no-build.
COMPILER_EXAMPLES = ("emit_wpe_soc", "lgvm", "chip_host", "lg2hex", "fabric_terminal")
_compiler_built = False
_target_dir = None


def target_dir():
    """The workspace's cargo target directory as cargo itself resolves it: `CARGO_TARGET_DIR`, a `build.target-dir`
    config, or `<workspace>/target`. `cargo build` writes the examples there, so they are read from there."""
    global _target_dir
    if _target_dir is None:
        r = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=REPO,
                           capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"cargo metadata failed:\n{r.stderr}")
        _target_dir = json.loads(r.stdout)["target_directory"]
    return _target_dir


def compiler(name, build=True):
    """Path to a compiler example binary, freshly built unless `build` is False."""
    global _compiler_built
    if build and not _compiler_built:
        cmd = ["cargo", "build", "-q", "-p", "logicaffeine-compile"]
        for ex in COMPILER_EXAMPLES:
            cmd += ["--example", ex]
        r = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"building the compiler examples failed:\n{r.stderr}")
        _compiler_built = True
    exe = os.path.join(target_dir(), "debug", "examples", name)
    if not os.path.exists(exe):
        sys.exit(f"{exe}: not built — rerun without --no-build")
    return exe


def ports_for(serial):
    """(jtag device, byte-channel device) for a board — the byte channel is whichever interface JTAG is not on."""
    return ftdi_ports.ports_for(serial)


def detect():
    rc = 0
    if KIT is not None:
        print(f"board kit: {REPO}  (targets: {', '.join(sorted(KIT['targets']))})")
    for serial, (name, part, cable, _) in BOARDS.items():
        jtag, data = ports_for(serial)
        print(f"Board {name}  {part}")
        print(f"  serial {serial}   cable {cable}   jtag {jtag or '-'}   bytes {data or '-'}")
        try:
            _, out = ftdi_ports.run_loader(cable, serial, ["--freq", "1000000", "--detect"], timeout=60)
        except FileNotFoundError:
            print("  openFPGALoader is not installed (macOS: `brew install openfpgaloader`)")
            return 1
        idcode = next((l.split()[-1] for l in out.splitlines() if "idcode" in l), None)
        if idcode:
            print(f"  idcode {idcode}  UP")
        elif jtag is None and data is None:
            print("  NOT ENUMERATED — board unplugged or powered off")
            rc = 1
        else:
            print("  no idcode — check the board's power switch, that the cable is right for this part, or (macOS) "
                  "that no other program holds the JTAG channel")
            for line in out.strip().splitlines()[-3:]:
                print(f"    {line}")
            rc = 1
    return rc


def flash(target, bit_override=None):
    t = TARGETS[target]
    bit = os.path.join(SIL, bit_override or t["bit"])
    if not os.path.exists(bit):
        sys.exit(f"{bit}: no such bitstream")
    cable = BOARDS[t["serial"]][2]
    print(f"flashing {bit_override or t['bit']} -> board {BOARDS[t['serial']][0]} ({BOARDS[t['serial']][1]})")
    try:
        code, out = ftdi_ports.run_loader(cable, t["serial"], ["--freq", "1000000", bit])
    except FileNotFoundError:
        sys.exit("openFPGALoader is not installed (macOS: `brew install openfpgaloader`)")
    if code != 0:
        sys.exit(f"flash failed:\n{out}")
    print("flashed")
    time.sleep(2)


# ── compiling LOGOS source for the fabric engine ──────────────────────────────────────────────────────────

def flashed_caps(target, bit_override=None):
    """The caps of the image on the board, from the `<bitstream>.build.txt` the build writes beside it.

    The recipe line carries `(NREG=512, MAXBLK=160, …)` — the RTL's localparams at build time — and a `NREG=`
    override when the engine was re-emitted narrower. Returns `NAME=N,…` or None when the image has no record
    (then only the engine PROFILE guards, which is blind to the data-dependent DEPTH and HEAP).
    """
    t = TARGETS[target]
    rec = os.path.join(SIL, (bit_override or t["bit"]) + ".build.txt")
    if not os.path.exists(rec):
        return None
    import re
    text = open(rec).read()
    m = re.search(r"\(([^)]*)\)", text)
    if not m:
        return None
    caps = {}
    for kv in m.group(1).replace(";", "").split(","):
        if "=" in kv:
            k, v = kv.strip().split("=", 1)
            caps[k.strip()] = v.strip()
    fpu = re.search(r"\bFPU=([01])\b", text)
    if fpu:
        caps["FPU"] = fpu.group(1)   # an image built without the float lane refuses float programs
    return ",".join(f"{k}={v}" for k, v in caps.items()) or None


def has_main(lg):
    """A program with a `## Main` block runs as a whole program; otherwise it is function-form (`--fn`/`--args`)."""
    with open(lg) as f:
        return any(line.strip() == "## Main" for line in f)


def compile_lg(target, lg, func, args_csv, out_dir, build=True, caps=None, bit_override=None):
    """LOGOS source -> (stream path, VM golden, facts) for a fabric-engine target.

    The whole compile is `emit_wpe_soc`. A `## Main` program is lowered whole (`--main`); the golden is the VM's
    OUTPUT — one line per `Show`, reported as `golden_json`. A function-form program lowers `func` the way the
    benchmark lane does (single-function CFG lifter, else the call-stack lifter) and the golden is `f(args)`.
    Either way the lowering is PROVED ≡ VM before a byte is written, and fit-checked against the target's engine
    profile — so the comparison at the end of `run` is silicon vs the language, not one model vs another.
    """
    t = TARGETS[target]
    if KIT is not None:
        sys.exit(f"this is a board kit ({REPO}): it carries precompiled streams and no compiler. Compile in the "
                 f"repo, or export the program into a kit there: scripts/logos-chip.py kit <dir> {target} {lg}")
    os.makedirs(out_dir, exist_ok=True)
    if t["frame"] == "core":
        return compile_core(lg, out_dir, build)
    name = os.path.splitext(os.path.basename(lg))[0]
    wire = os.path.join(out_dir, f"{name}.wire")
    cmd = [compiler("emit_wpe_soc", build), "--program", lg, "--wire", wire, "--profile", t["profile"]]
    cmd += ["--main"] if has_main(lg) else ["--fn", func, "--args", args_csv]
    caps = caps or flashed_caps(target, bit_override)
    if caps:
        cmd += ["--caps", caps]
    r = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True)
    if r.returncode != 0:
        # emit_wpe_soc says WHY on stderr: the lifter refused, the VM could not run it, or (the important one)
        # the program does not fit this target's engine — that message is the refusal, and nothing is streamed.
        sys.exit(r.stderr.rstrip() or f"emit_wpe_soc failed with exit status {r.returncode}")
    facts = dict(l.split("=", 1) for l in r.stdout.splitlines() if "=" in l)
    if "golden_json" in facts:
        return wire, json.loads(facts["golden_json"]), facts   # a Main program: the VM's output lines
    if "golden" not in facts:
        sys.exit(f"emit_wpe_soc reported no golden for {lg}:\n{r.stdout}\n{r.stderr}")
    return wire, facts["golden"], facts


def compile_core(lg, out_dir, build=True):
    """LOGOS source -> (the soft core's .hex words, the VM golden, facts) for the `core` target.

    The words are `lg2hex --full`'s, the wide core's compiler; the golden is `lgvm`'s output, one line per `Show`,
    joined with commas the way the core's reply decoder joins the Shows it reads back. The core runs a whole
    `## Main` program."""
    if not has_main(lg):
        sys.exit(f"{lg}: the soft core runs a whole `## Main` program; this one has none")
    name = os.path.splitext(os.path.basename(lg))[0]
    words = os.path.join(out_dir, f"{name}.hex")
    r = subprocess.run([compiler("lg2hex", build), "--full", lg], cwd=REPO, capture_output=True, text=True)
    if r.returncode != 0:
        sys.stderr.write(r.stderr)
        sys.exit(1)
    with open(words, "w") as f:
        f.write(r.stdout)
    v = subprocess.run([compiler("lgvm", build), lg], cwd=REPO, capture_output=True, text=True)
    if v.returncode != 0:
        sys.stderr.write(v.stderr)
        sys.exit(1)
    n = sum(1 for line in r.stdout.splitlines() if line.split("//")[0].strip())
    golden = ",".join(v.stdout.strip().splitlines())
    return words, golden, {"bytes": str(4 * n), "lifter": "full_asm", "profile": "core"}


def compile_cmd(target, lg, func, args_csv, out_dir, build=True, caps=None, bit_override=None):
    wire, golden, facts = compile_lg(target, lg, func, args_csv, out_dir, build, caps, bit_override)
    print(f"target={target}")
    print(f"caps={caps or flashed_caps(target, bit_override) or 'profile only'}")
    print(f"source={lg}")
    print(f"profile={facts.get('profile', '')}")
    print(f"lifter={facts.get('lifter', '')}")
    print(f"stream={wire}")
    print(f"bytes={facts.get('bytes', os.path.getsize(wire))}")
    if isinstance(golden, list):
        print(f"golden_json={json.dumps(golden)}")
    else:
        print(f"golden={golden}")
    return 0


# ── reply decoding ─────────────────────────────────────────────────────────────────────────────────────────

_protocol = None


def protocol(build=True):
    """The engine's byte protocol: image, record markers, frame bytes, value tags and host calls, read from its one
    definition (`codegen_sva::chip_protocol`) through `chip_host --table` — inside a board kit, from the table
    `chip_host --table` printed when the kit was exported. This host restates none of it."""
    global _protocol
    if _protocol is None and KIT is not None:
        _protocol = json.load(open(os.path.join(REPO, KIT["protocol"])))
    if _protocol is None:
        r = subprocess.run([compiler("chip_host", build), "--table"], capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"chip_host --table failed:\n{r.stderr}")
        _protocol = json.loads(r.stdout)
    return _protocol


def wire_func_codes(tx):
    """The functional-unit codes a pre-built `.wire` stream will execute: the `cell_code` of every cell whose
    `cell_kind` is 2. Parsed straight off the wire (WImage::wire_bytes order: 13 blk_* tables, then cell_kind,
    cell_code, …), so a manifest wire built for a profile can be checked against the image actually flashed."""
    p = 0
    mems = []
    for _ in range(23):
        n = int.from_bytes(tx[p : p + 2], "big"); p += 2
        mems.append([int.from_bytes(tx[p + 4 * i : p + 4 * i + 4], "big") for i in range(n)])
        p += 4 * n
    kinds, codes = mems[13], mems[14]
    return [c for k, c in zip(kinds, codes) if k == 2]


def wire_needs_fpu(tx):
    """Does a pre-built `.wire` use the FPU lane (float units, or popcount which rides it)?"""
    return any(5 <= c <= 11 or 13 <= c <= 20 for c in wire_func_codes(tx))


def image_has_fpu(target, bit_override=None):
    """None when the flashed image's recipe says nothing; else True/False from its `FPU=` field."""
    caps = flashed_caps(target, bit_override)
    if not caps:
        return None
    for kv in caps.split(","):
        if kv.startswith("FPU="):
            return kv[4:] == "1"
    return None


def render_show(p, tag, payload):
    """Render one streamed Show record the way the VM prints it (the engine's `render_show` contract)."""
    tags = p["tags"]
    if tag == tags["text"]:
        return payload.decode("utf-8", errors="replace")
    v = int.from_bytes(payload, "big", signed=True)
    if tag == tags["float"]:
        return nexys_host().fmt_f64(v & 0xFFFFFFFFFFFFFFFF)
    if tag == tags["word32"]:
        return str(v & 0xFFFFFFFF)
    if tag == tags["bool"]:
        return "true" if v != 0 else "false"
    return str(v)


def decode_wpe(p, buf):
    """Decode the whole-program engine's reply out of a raw byte buffer, by the protocol table `p`.

    The engine streams one show record per `Show` as the program runs (the show marker, the tag, then 8 value
    bytes MSB-first, or for a Text a 4-byte length then that many bytes) and then the result frame (the sync
    byte, result[63:0] MSB-first and signed, cycles[31:0] MSB-first, one end byte). Returns (result, cycles,
    status, shows) with status one of: `ok`; `overflow`, the call-stack end: the program recursed deeper than the
    engine's DEPTH, and the result bytes are NOT an answer; `heap`, the heap end: the program's heap outgrew the
    engine's HEAP, NOT an answer either; `short`, a record or the frame was cut off; `nosync`, no marker found.
    Bytes before the first marker (a power-on-reset byte) are skipped. `shows` holds the rendered records, in
    program order.
    """
    rec, frame, tags = p["records"], p["frame"], p["tags"]
    shows = []
    i = 0
    while i < len(buf) and buf[i] not in (frame["sync"], rec["show"]):
        i += 1
    while i < len(buf) and buf[i] == rec["show"]:
        if i + 2 > len(buf):
            return None, None, "short", shows
        tag = buf[i + 1]
        if tag == tags["text"]:
            if i + 6 > len(buf):
                return None, None, "short", shows
            n = int.from_bytes(buf[i + 2 : i + 6], "big")
            if i + 6 + n > len(buf):
                return None, None, "short", shows
            shows.append(render_show(p, tag, buf[i + 6 : i + 6 + n]))
            i += 6 + n
        else:
            if i + 10 > len(buf):
                return None, None, "short", shows
            shows.append(render_show(p, tag, buf[i + 2 : i + 10]))
            i += 10
    if i >= len(buf) or buf[i] != frame["sync"]:
        return None, None, ("short" if shows else "nosync"), shows
    fr = buf[i : i + 14]
    if len(fr) < 14:
        return None, None, "short", shows
    result = int.from_bytes(fr[1:9], "big", signed=True)
    cycles = int.from_bytes(fr[9:13], "big")
    if fr[13] == frame["end"]:
        return result, cycles, "ok", shows
    if fr[13] == frame["end_call_stack"]:
        return None, cycles, "overflow", shows
    if fr[13] == frame["end_heap"]:
        return None, cycles, "heap", shows
    return None, None, "short", shows


def read_frame_wpe(ser, deadline, p):
    """Collect the engine's reply until the line goes quiet, then decode it."""
    buf = b""
    while time.time() < deadline:
        b = ser.read(32)
        if b:
            buf += b
        elif buf:
            break
    result, cycles, status, shows = decode_wpe(p, buf)
    return result, cycles, buf, status, shows


def nexys_host():
    """The soft-core frame codec, loaded from nexys-host.py rather than restated: that parser is the one pinned
    against the RTL by `nexys-host.py --selftest`, and two copies of a wire format is how they drift."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("nexys_host", os.path.join(REPO, "scripts", "nexys-host.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def decode_core(buf):
    """The soft-core's tagged multi-Show reply (sync, count, count x (tag + value), cycles[31:0], end), by the
    codec nexys-host.py pins against the RTL."""
    mod = nexys_host()
    try:
        shows, cycles = mod.parse_reply(buf)
    except Exception:
        return None, None, "short" if mod.SYNC in buf else "nosync"
    return ",".join(mod.show_str(tag, raw) for tag, raw in shows), cycles, "ok"


def read_frame_core(ser, deadline, p):
    buf = b""
    while time.time() < deadline:
        b = ser.read(64)
        if b:
            buf += b
        elif buf:
            break
    result, cycles, status = decode_core(buf)
    return result, cycles, buf, status, []


def describe(status, raw):
    if status == "overflow":
        return "FRAME-STACK OVERFLOW: the program recursed deeper than this engine's DEPTH — not an answer"
    if status == "heap":
        return "HEAP OVERFLOW: the program's heap outgrew this engine's HEAP — not an answer"
    if status == "short":
        return f"INCOMPLETE REPLY (raw: {raw.hex()})"
    return f"NO REPLY  (raw: {raw.hex() if raw else 'nothing'})"


# ── running ────────────────────────────────────────────────────────────────────────────────────────────────

def wire_bytes(target, program, build=True):
    """The bytes a pre-built stream puts on the wire: a `.wire` as it is, soft-core `.hex` words framed with their
    count (one byte, or 0xFF and two bytes past 254 words) ahead of each word MSB-first."""
    if TARGETS[target]["frame"] in ("wpe", "term"):
        tx = open(program, "rb").read()
        if not tx or tx[-1] != protocol(build)["image"]["go"]:
            sys.exit(f"{program}: not a .wire stream (it must end with the image's GO byte)")
        return tx
    words = []
    for line in open(program):
        line = line.split("//")[0].strip()
        if line:
            words.append(int(line, 16) & 0xFFFFFFFF)
    n = len(words)
    head = bytes([0xFF, (n >> 8) & 0xFF, n & 0xFF]) if n > 254 else bytes([n])
    return head + b"".join(w.to_bytes(4, "big") for w in words)


def _run_term(target, program, wait, build=True):
    """Run one pre-built stream on the Fabric Terminal's engine through the terminal's link. Returns what `_run_one`
    returns."""
    import terminal_link

    p = protocol(build)
    link = terminal_link.Link(terminal_link.table(), ports_for(TARGETS[target]["serial"])[1])
    buf = bytearray()
    decoded = [None, None, "nosync", []]

    def on_bytes(chunk):
        buf.extend(chunk)
        decoded[:] = decode_wpe(p, bytes(buf))
        return decoded[2] not in ("short", "nosync")

    try:
        pong = link.ping()
        if pong is None:
            sys.exit(f"Board B on {link.port} does not answer the terminal's link — flash it: logos-chip.py flash {target}")
        if not link.has_engine(pong):
            sys.exit(f"the image on Board B has no fabric engine — flash {TARGETS[target]['bit']}: logos-chip.py flash {target}")
        link.run_engine(wire_bytes(target, program, build), on_bytes, wait)
    finally:
        link.close()
    result, cycles, status, shows = decoded
    return result, cycles, bytes(buf), status, shows


def _run_one(target, program, wait, build=True):
    """Send one pre-built stream and decode the reply. Returns (result, cycles, raw, status, shows)."""
    import serial as pyserial

    if TARGETS[target]["frame"] == "term":
        return _run_term(target, program, wait, build)
    p = protocol(build)
    t = TARGETS[target]
    _, port = ports_for(t["serial"])
    if port is None:
        sys.exit(f"board {BOARDS[t['serial']][0]} has no byte channel — run `detect`")
    tx = wire_bytes(target, program, build)

    ser = pyserial.Serial(port, t["baud"], timeout=2)
    # Settle and DRAIN before writing. The power-on reset can emit a byte after the port opens, and a stale
    # byte ahead of the frame makes a perfectly good reply fail its sync check — which reads as a dead board.
    time.sleep(0.3)
    ser.read(256)
    ser.reset_input_buffer()
    ser.write(tx)
    ser.flush()
    reader = read_frame_wpe if t["frame"] == "wpe" else read_frame_core
    result, cycles, raw, status, shows = reader(ser, time.time() + wait, p)
    ser.close()
    return result, cycles, raw, status, shows


def kit_program(target, program, func, args_csv):
    """The stream, golden and facts a board kit carries for (target, LOGOS source, function, arguments).

    A source is named by its path in the repo (the kit mirrors the repo's layout) or by its file name. A `## Main`
    program is carried once; a function-form program once per function and argument list. Anything the kit does
    not carry is refused by name, with the export command that would add it."""
    rel = os.path.relpath(os.path.abspath(program), REPO)
    entries = KIT["targets"].get(target, {}).get("programs", [])

    def same_source(e):
        return e["source"] == rel or os.path.basename(e["source"]) == os.path.basename(program)

    def carried(e):
        return os.path.basename(e["source"]) + ("" if e["fn"] is None else f" {e['fn']}({e['args']})")

    hit = next((e for e in entries if same_source(e) and (e["fn"] is None or (e["fn"], e["args"]) == (func, args_csv))), None)
    if hit is None:
        what = os.path.basename(program) + (f" {func}({args_csv})" if args_csv else "")
        sys.exit(f"{what} is not in this kit for {target} — it carries: {', '.join(map(carried, entries)) or 'nothing'}. "
                 f"Export it from the repo: scripts/logos-chip.py kit <dir> {target} {program}"
                 + (f"@{args_csv}" if args_csv else ""))
    facts = {"bytes": str(hit["bytes"]), "lifter": hit["lifter"], "profile": hit["profile"], "main": hit["fn"] is None}
    return os.path.join(REPO, hit["stream"]), hit["golden"], facts


def shown_path(path):
    """A path as a reader names it: relative to the repo (or kit) root when it lies inside it."""
    rel = os.path.relpath(os.path.abspath(path), REPO)
    return path if rel.startswith("..") else rel


def run(target, program, expect, wait, func="f", args_csv="", out_dir=None, build=True, caps=None, bit_override=None,
        dry_run=False):
    t = TARGETS[target]
    golden = None
    stream = program
    if program.endswith(".lg"):
        if KIT is not None:
            stream, golden, facts = kit_program(target, program, func, args_csv)
        else:
            stream, golden, facts = compile_lg(target, program, func, args_csv, out_dir or "/tmp/logos-chip", build, caps, bit_override)
            facts["main"] = has_main(program)
        what = "Main" if facts["main"] else f"{func}({args_csv})"
        print(f"{os.path.basename(program)}: {what} -> {facts.get('bytes', '?')} bytes on the wire "
              f"({facts.get('lifter', '?')} lifter, {facts.get('profile', '?')} profile)")
        if isinstance(golden, list):
            print(f"VM output ({len(golden)} lines): " + " | ".join(golden))
            # A Main program's output comes back as Show records, which only images built from a tree that has the
            # Show cell stream. Every such image carries a build recipe; one without a recipe predates it and would
            # run the program silently (no records, no error) — refuse instead of reporting a mismatch as silicon.
            if flashed_caps(target, bit_override) is None:
                print(f"{os.path.basename(program)}: does not fit the flashed image: it has no build recipe, so it "
                      f"predates Show records — flash an image built by this tree (its .build.txt sits beside it)")
                return 1
        else:
            print(f"VM golden {golden}")
        if expect is None:
            expect = golden
    if stream.endswith(".wire") and image_has_fpu(target, bit_override) is False:
        if wire_needs_fpu(open(stream, "rb").read()):
            print(f"{os.path.basename(program)}: does not fit the flashed image: it uses the FPU lane and the image was built FPU=0")
            return 1
    if dry_run:
        print(f"stream={shown_path(stream)}")
        print(f"bytes={len(wire_bytes(target, stream, build))}")
        if isinstance(expect, list):
            print(f"golden_json={json.dumps(expect)}")
        elif expect is not None:
            print(f"golden={expect}")
        print(f"port={ports_for(t['serial'])[1] or '-'}")
        print("dry run: nothing sent")
        return 0
    result, cycles, raw, status, shows = _run_one(target, stream, wait, build)
    name = os.path.basename(program)
    for line in shows:
        print(f"  > {line}")
    if result is None:
        print(f"{name}: {describe(status, raw)}")
        return 1
    us = cycles / t["clock"] if cycles else 0.0
    if isinstance(expect, list):
        print(f"{name}: {len(shows)} lines streamed back   {cycles} cycles = {us:.1f} us @ {t['clock']} MHz")
        if shows == expect:
            print("SILICON == VM")
            return 0
        print(f"SILICON != VM: chip printed {shows}, the VM prints {expect}")
        return 1
    print(f"{name}: result = {result}   {cycles} cycles = {us:.1f} us @ {t['clock']} MHz")
    if expect is not None:
        if str(result) == str(expect):
            print("SILICON == VM" if golden is not None else "OK: matches --expect")
            return 0
        print(f"SILICON != VM: chip {result}, expected {expect}" if golden is not None
              else f"FAIL: got {result}, expected {expect}")
        return 1
    return 0


def suite(target, manifest, wait, bit_override=None):
    """Stream every program in a manifest and check each against its VM golden.

    The manifest is written by `generate_benchmark_wires_for_the_fpga`, which asserts engine-oracle == VM
    before recording a line — so the third column IS the VM's answer, and a mismatch here is silicon
    disagreeing with the language, not two models agreeing with each other.
    """
    rows = []
    for line in open(manifest):
        if line.lstrip().startswith("#"):
            continue
        parts = line.split()
        # THREE fields are required and a fourth is allowed. Insisting on exactly three looks
        # harmless and silently skipped every row of bench_silicon_manifest.txt, whose lines carry a
        # heap_words column too — the suite then ran nothing at all and reported "0 passed, 0 failed
        # of 0", which reads like success at a glance.
        if len(parts) >= 3:
            rows.append(parts[:3])
    if not rows:
        print(f"REFUSING: {manifest} yielded no runnable rows — a manifest that parses to nothing "
              f"is not a passing suite. Expected lines of 'name arg golden [heap]'.")
        return 2
    prefix = os.path.basename(manifest).rsplit("_manifest", 1)[0]
    progdir = os.path.dirname(manifest)

    fpu = image_has_fpu(target, bit_override)
    print(f"{'benchmark':18} {'arg':>7} {'VM golden':>14} {'silicon':>14} {'cycles':>11}  result")
    print("-" * 82)
    npass = nfail = nrefused = 0
    for name, arg, golden in rows:
        wire = os.path.join(progdir, f"{prefix}_{name}.wire")
        if not os.path.exists(wire):
            print(f"{name:18} {arg:>7} {golden:>14} {'-':>14} {'-':>11}  NO WIRE")
            nfail += 1
            continue
        # A wire built for the engine PROFILE may still exceed the flashed IMAGE: an FPU=0 image has no float lane,
        # so a float benchmark is refused (not streamed to return a zero and be counted as silicon disagreeing).
        if fpu is False and wire_needs_fpu(open(wire, "rb").read()):
            print(f"{name:18} {arg:>7} {golden:>14} {'-':>14} {'-':>11}  NO FPU (image built FPU=0)")
            nrefused += 1
            continue
        result, cycles, _raw, status, _shows = _run_one(target, wire, wait)
        got = "-" if result is None else str(result)
        if status == "overflow":
            got = "OVERFLOW"
        cyc = "-" if cycles is None else str(cycles)
        ok = got == golden
        print(f"{name:18} {arg:>7} {golden:>14} {got:>14} {cyc:>11}  {'PASS' if ok else '*** FAIL'}")
        npass += ok
        nfail += not ok
    print("-" * 82)
    print(f"SILICON == VM: {npass} passed, {nfail} failed of {len(rows)}"
          + (f" ({nrefused} refused: the flashed image has no FPU)" if nrefused else ""))
    return 1 if nfail else 0


# ── board kits ───────────────────────────────────────────────────────────────────────────────────────────────

def kit_export(out, target, programs, func, manifests, files, build=True):
    """Export programs for `target` into the board kit at `out`, merging into a kit already there.

    The kit is a sparse copy of this repo: the host scripts and the selftest's wires, the target's image and its
    build recipe, each LOGOS source beside its stream with the VM golden in `kit.json`, the protocol table exactly
    as `chip_host --table` printed it, and the manifests (with their wires) and extra files asked for. A program is
    `path.lg` for a `## Main` program or `path.lg@a,b` for `func` at those arguments; a `term` kit given none carries
    the Fabric Terminal's library (`terminal_link.library`), each entry with its title. Each is compiled by the path
    `run` takes, so a kit's bytes and goldens are the repo's."""
    if KIT is not None:
        sys.exit(f"this is a board kit ({REPO}): export kits from the repo")
    t = TARGETS[target]
    out = os.path.abspath(out)
    if out == REPO:
        sys.exit("a kit is written to its own directory, not over the repo")

    def put(rel, src):
        dst = os.path.join(out, rel)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copyfile(src, dst)
        shutil.copymode(src, dst)

    doc_path = os.path.join(out, "kit.json")
    doc = (json.load(open(doc_path)) if os.path.exists(doc_path)
           else {"protocol": "kit/protocol.json", "targets": {}, "files": []})
    for rel in HOST_SCRIPTS + SELFTEST_WIRES:
        put(rel, os.path.join(REPO, rel))
    image = os.path.join(SIL, t["bit"])
    for path in (image, image + ".build.txt"):
        if os.path.exists(path):
            put(os.path.relpath(path, REPO), path)
    table = subprocess.run([compiler("chip_host", build), "--table"], capture_output=True)
    if table.returncode != 0:
        sys.exit(f"chip_host --table failed:\n{table.stderr.decode(errors='replace')}")
    os.makedirs(os.path.join(out, os.path.dirname(doc["protocol"])), exist_ok=True)
    with open(os.path.join(out, doc["protocol"]), "wb") as f:
        f.write(table.stdout)
    if t["frame"] == "term":
        terminal = subprocess.run([compiler("fabric_terminal", build), "--table"], capture_output=True)
        if terminal.returncode != 0:
            sys.exit(f"fabric_terminal --table failed:\n{terminal.stderr.decode(errors='replace')}")
        doc["terminal"] = "kit/terminal.json"
        with open(os.path.join(out, doc["terminal"]), "wb") as f:
            f.write(terminal.stdout)

    items = [(None, lg, func, args_csv) for lg, _, args_csv in (spec.partition("@") for spec in programs)]
    if not items and t["frame"] == "term":
        import terminal_link

        items = [(title, os.path.join(REPO, source), fn or func, args)
                 for title, source, fn, args in terminal_link.library()]
    slot = doc["targets"].setdefault(target, {"programs": []})
    slot["image"] = t["bit"]
    with tempfile.TemporaryDirectory() as tmp:
        for title, lg, func, args_csv in items:
            spec = lg + (f"@{args_csv}" if args_csv else "")
            rel = os.path.relpath(os.path.abspath(lg), REPO)
            main = has_main(lg)
            if main and args_csv:
                sys.exit(f"{spec}: a `## Main` program takes no arguments")
            if main and t["frame"] in ("wpe", "term") and flashed_caps(target) is None:
                sys.exit(f"{spec}: {t['bit']} has no build recipe, so it predates Show records and cannot run a "
                         f"`## Main` program — export a function-form program for {target}, or use an image built "
                         f"by this tree")
            stream, golden, facts = compile_lg(target, lg, func, args_csv, tmp, build)
            stem = os.path.splitext(os.path.basename(lg))[0] + ("" if main else "@" + args_csv.replace(",", "_"))
            kit_stream = f"kit/{target}/{stem}{os.path.splitext(stream)[1]}"
            put(kit_stream, stream)
            put(rel, lg)
            entry = {
                "source": rel,
                "fn": None if main else func,
                "args": "" if main else args_csv,
                "stream": kit_stream,
                "golden": golden,
                "bytes": int(facts.get("bytes", os.path.getsize(stream))),
                "lifter": facts.get("lifter", ""),
                "profile": facts.get("profile", ""),
            }
            if title:
                entry["title"] = title
            key = (entry["source"], entry["fn"], entry["args"])
            slot["programs"] = [e for e in slot["programs"] if (e["source"], e["fn"], e["args"]) != key] + [entry]
            shown = golden if not isinstance(golden, list) else " | ".join(golden)
            print(f"kit {target}: {rel}{'' if main else f' {func}({args_csv})'} -> {kit_stream} "
                  f"({entry['bytes']} bytes)   VM: {shown}")
    for manifest in manifests:
        put(os.path.relpath(os.path.abspath(manifest), REPO), manifest)
        prefix = os.path.basename(manifest).rsplit("_manifest", 1)[0]
        for line in open(manifest):
            parts = line.split()
            if len(parts) >= 3 and not line.lstrip().startswith("#"):
                wire = os.path.join(os.path.dirname(manifest), f"{prefix}_{parts[0]}.wire")
                put(os.path.relpath(os.path.abspath(wire), REPO), wire)
        print(f"kit: {manifest} and its wires")
    for spec in files:
        src, _, dest = spec.partition("=")
        dest = dest or os.path.relpath(os.path.abspath(src), REPO)
        if dest.startswith(".."):
            sys.exit(f"{spec}: {src} lies outside the repo; name where it goes in the kit as {src}=<path>")
        put(dest, src)
        if dest not in doc["files"]:
            doc["files"].append(dest)
        print(f"kit: {src} -> {dest}")
    with open(doc_path, "w") as f:
        json.dump(doc, f, indent=1)
        f.write("\n")
    print(f"kit: {out}")
    return 0


# ── self-test of the decoders (no hardware) ──────────────────────────────────────────────────────────────

def decode(reply, build=True):
    """Decode a captured engine reply (its raw bytes, from a file) and print it as JSON."""
    result, cycles, status, shows = decode_wpe(protocol(build), open(reply, "rb").read())
    print(json.dumps({"result": result, "cycles": cycles, "status": status, "shows": shows}))
    return 0


def selftest(build=True):
    """Exercise both reply decoders on synthetic frames, so a decoder change is caught here rather than read as
    a dead board. Every case is the exact byte layout the RTL emits, built from the protocol table."""
    import struct

    p = protocol(build)
    rec, fr, tags = p["records"], p["frame"], p["tags"]
    checks = []

    def check(label, cond):
        checks.append((label, cond))
        print(f"  {'ok ' if cond else 'BAD'} {label}")

    def result_frame(result, cycles, end):
        return bytes([fr["sync"]]) + result.to_bytes(8, "big", signed=True) + cycles.to_bytes(4, "big") + bytes([end])

    def show(tag, payload):
        return bytes([rec["show"], tag]) + payload

    frame = result_frame(-42, 12345, fr["end"])
    check("engine frame: negative 64-bit result + cycles", decode_wpe(p, frame) == (-42, 12345, "ok", []))
    check("engine frame: a power-on byte before the sync is skipped", decode_wpe(p, b"\x00" + frame) == (-42, 12345, "ok", []))
    big = result_frame(2**62 + 7, 1, fr["end"])
    check("engine frame: a result above 2^32 is read whole", decode_wpe(p, big) == (2**62 + 7, 1, "ok", []))
    ovf = result_frame(99, 500, fr["end_call_stack"])
    check("engine frame: the call-stack end is a frame-stack overflow, never a result", decode_wpe(p, ovf) == (None, 500, "overflow", []))
    heap = result_frame(99, 500, fr["end_heap"])
    check("engine frame: the heap end is a heap overflow, never a result", decode_wpe(p, heap) == (None, 500, "heap", []))
    check("engine frame: truncated after the sync is `short`", decode_wpe(p, frame[:9]) == (None, None, "short", []))
    check("engine frame: no sync byte is `nosync`", decode_wpe(p, b"\x00\x01\x02") == (None, None, "nosync", []))
    check("engine frame: a wrong terminator is not an answer", decode_wpe(p, frame[:13] + b"\x00") == (None, None, "short", []))
    rec_int = show(tags["int"], (-7).to_bytes(8, "big", signed=True))
    rec_bool = show(tags["bool"], (1).to_bytes(8, "big"))
    rec_text = show(tags["text"], (9).to_bytes(4, "big") + b"hi fabric")
    rec_float = show(tags["float"], struct.pack(">d", 3.75))
    rec_w32 = show(tags["word32"], (0xFFFFFFFF).to_bytes(8, "big"))
    check("show records: int, bool, text, float, word32 render as the VM prints them",
          decode_wpe(p, rec_int + rec_bool + rec_text + rec_float + rec_w32 + frame) == (-42, 12345, "ok", ["-7", "true", "hi fabric", "3.75", "4294967295"]))
    check("show records: an empty text is an empty line", decode_wpe(p, show(tags["text"], (0).to_bytes(4, "big")) + frame) == (-42, 12345, "ok", [""]))
    check("show records: a power-on byte before the first record is skipped", decode_wpe(p, b"\x00" + rec_int + frame)[3] == ["-7"])
    check("show records: a text cut off mid-payload is `short`", decode_wpe(p, rec_text[:-3])[2] == "short")
    check("show records: records with no frame after them are `short`", decode_wpe(p, rec_int + rec_bool)[2] == "short")

    core_codec = nexys_host()
    core = (bytes([core_codec.SYNC, 2]) + b"\x00" + (7).to_bytes(4, "big") + b"\x00" + (-3 & 0xFFFFFFFF).to_bytes(4, "big")
            + (99).to_bytes(4, "big") + bytes([core_codec.END]))
    check("soft-core frame: two int Shows, signed, in program order + cycles", decode_core(core) == ("7,-3", 99, "ok"))
    check("soft-core frame: truncated is `short`", decode_core(core[:6])[2] == "short")

    pi = open(os.path.join(SIL, "programs", "bench_pi_leibniz.wire"), "rb").read()
    loop = open(os.path.join(SIL, "programs", "wpe_loop.wire"), "rb").read()
    check("wire parser: bench_pi_leibniz.wire uses the FPU lane", wire_needs_fpu(pi))
    check("wire parser: wpe_loop.wire does not", not wire_needs_fpu(loop))
    check("wire parser: the GO byte ends both streams", pi[-1] == p["image"]["go"] and loop[-1] == p["image"]["go"])

    bad = [l for l, c in checks if not c]
    if bad:
        print(f"selftest: FAILED {len(bad)} of {len(checks)}: {bad}")
        return 1
    print(f"selftest: OK ({len(checks)} checks)")
    return 0


def main():
    # Line-buffer stdout so a long suite streams its rows into a log as they happen, not at exit.
    try:
        sys.stdout.reconfigure(line_buffering=True)
    except Exception:
        pass
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("detect")
    st = sub.add_parser("selftest")
    st.add_argument("--no-build", action="store_true", help="use the compiler binaries as built, skip cargo")
    d = sub.add_parser("decode", help="decode a captured engine reply (raw bytes) and print it as JSON")
    d.add_argument("reply", help="a file holding the reply bytes")
    d.add_argument("--no-build", action="store_true", help="use the compiler binaries as built, skip cargo")
    f = sub.add_parser("flash")
    f.add_argument("target", choices=sorted(TARGETS))
    f.add_argument("--bit", help="use this bitstream instead of the target's")
    c = sub.add_parser("compile", help="LOGOS source -> the target's wire bytes + the VM golden, no board")
    c.add_argument("target", choices=sorted(TARGETS))
    c.add_argument("program", help="a .lg file")
    c.add_argument("--fn", default="f", help="the function to run (default f)")
    c.add_argument("--args", default="", help="comma-separated Int arguments")
    c.add_argument("--out", default="/tmp/logos-chip", help="where to write the .wire")
    c.add_argument("--no-build", action="store_true", help="use the compiler binaries as built, skip cargo")
    c.add_argument("--caps", help="override the flashed image's caps (NAME=N,…); default: its .build.txt record")
    c.add_argument("--bit", help="the bitstream whose recorded caps apply")
    r = sub.add_parser("run")
    r.add_argument("target", choices=sorted(TARGETS))
    r.add_argument("program", help="a .lg (compiled here, checked vs the VM), a .wire, or a .hex")
    r.add_argument("--fn", default="f", help="for a .lg: the function to run (default f)")
    r.add_argument("--args", default="", help="for a .lg: comma-separated Int arguments")
    r.add_argument("--out", default="/tmp/logos-chip", help="for a .lg: where to write the .wire")
    r.add_argument("--no-build", action="store_true", help="use the compiler binaries as built, skip cargo")
    r.add_argument("--caps", help="override the flashed image's caps (NAME=N,…); default: its .build.txt record")
    r.add_argument("--expect", help="expected result (a .lg defaults to its VM golden)")
    r.add_argument("--wait", type=float, default=30.0, help="seconds to wait for the reply frame")
    r.add_argument("--flash", action="store_true", help="flash the target's bitstream first")
    r.add_argument("--bit", help="use this bitstream instead of the target's — e.g. one our own place and route built")
    r.add_argument("--dry-run", action="store_true", help="resolve the stream, golden and port, send nothing")
    k = sub.add_parser("kit", help="export a board kit: these programs, runnable with no compiler on another machine")
    k.add_argument("out", help="the kit directory (created, or merged into)")
    k.add_argument("target", choices=sorted(TARGETS))
    k.add_argument("programs", nargs="*", help="path.lg (a ## Main program) or path.lg@a,b (--fn at those arguments)")
    k.add_argument("--fn", default="f", help="the function a path.lg@args program runs (default f)")
    k.add_argument("--manifest", action="append", default=[], help="a suite manifest, copied with its wires")
    k.add_argument("--file", action="append", default=[], help="SRC[=DEST]: copy SRC into the kit at DEST")
    k.add_argument("--no-build", action="store_true", help="use the compiler binaries as built, skip cargo")
    su = sub.add_parser("suite")
    su.add_argument("target", choices=sorted(TARGETS))
    su.add_argument("--manifest", required=True)
    su.add_argument("--wait", type=float, default=120.0)
    su.add_argument("--flash", action="store_true")
    su.add_argument("--bit", help="use this bitstream instead of the target's")
    a = ap.parse_args()

    if a.cmd == "detect":
        return detect()
    if a.cmd == "selftest":
        return selftest(build=not a.no_build)
    if a.cmd == "decode":
        return decode(a.reply, build=not a.no_build)
    if a.cmd == "flash":
        flash(a.target, getattr(a, "bit", None))
        return 0
    if a.cmd == "kit":
        return kit_export(a.out, a.target, a.programs, a.fn, a.manifest, a.file, build=not a.no_build)
    if a.cmd == "compile":
        return compile_cmd(a.target, a.program, a.fn, a.args, a.out, build=not a.no_build, caps=a.caps, bit_override=a.bit)
    if a.cmd == "suite":
        if a.flash:
            flash(a.target, getattr(a, "bit", None))
        return suite(a.target, a.manifest, a.wait, getattr(a, "bit", None))
    if a.flash:
        flash(a.target, getattr(a, "bit", None))
    return run(a.target, a.program, a.expect, a.wait, a.fn, a.args, a.out, build=not a.no_build, caps=a.caps,
               bit_override=getattr(a, "bit", None), dry_run=a.dry_run)


if __name__ == "__main__":
    sys.exit(main())
