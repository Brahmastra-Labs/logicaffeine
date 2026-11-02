#!/usr/bin/env python3
"""ftdi_ports — the attached Digilent boards and their FT2232 channels, found the same way on Linux and macOS.

A board's FT2232 shows up as two serial devices, one per USB interface (channel A is interface 0, channel B is
interface 1). Their NAMES carry nothing stable: Linux numbers `/dev/ttyUSBn` in enumeration order, and macOS names
a node after the board's serial plus a driver-specific suffix — the interface digit under Apple's driver
(`/dev/cu.usbserial-210292BFA3101`), the channel letter under FTDI's VCP driver (`…210292BFA310B`). So a channel
is found by (FTDI serial, interface), read from what the OS reports through `serial.tools.list_ports`:

  Linux   the USB location `<bus>-<port path>:<config>.<interface>` names the interface;
  macOS   the node name ends in the serial and then the interface digit or channel letter. Both a callout
          (`cu.`) and a dial-in (`tty.`) node exist per channel; a host opens the callout node, which does not
          wait for carrier.

    python3 scripts/lib/ftdi_ports.py ports                      every board channel this machine sees
    python3 scripts/lib/ftdi_ports.py port A|B                   a board's byte channel (exit 1: not attached)
    python3 scripts/lib/ftdi_ports.py flash A|B BIT [--persist]  load BIT into a board's FPGA (--persist: into its
                                                                 configuration flash, booted at power-on)
    python3 scripts/lib/ftdi_ports.py resolve SERIAL INTERFACE   one channel's device (exit 1: not attached)
    python3 scripts/lib/ftdi_ports.py loader --platform darwin --cable digilent --serial SERIAL -- ARGS…
                                                                 the openFPGALoader argv this OS runs, as JSON

A board is named by its letter (A or B, either case) or its FTDI serial. Every script that talks to a board
asks this module for its port and flashes through it; `board_host_portability_lock` holds `scripts/` to that.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys

BOARD_A = "210276C08FE0"
BOARD_B = "210292BFA310"

# FTDI serial -> (board name, part, openFPGALoader cable, USB interface carrying JTAG). The byte channel is the
# OTHER interface: the Nexys Video puts JTAG on channel B, the Nexys A7 on channel A.
BOARDS = {
    BOARD_A: ("A", "Nexys Video XC7A200T", "digilent_b", 1),
    BOARD_B: ("B", "Nexys A7-100T XC7A100T", "digilent", 0),
}

# Each board's FPGA as device + package: writing the configuration flash goes through a SPI-over-JTAG bridge
# image the loader picks by it.
FPGA_PART = {BOARD_A: "xc7a200tsbg484", BOARD_B: "xc7a100tcsg324"}

# What the loader prints when a configuration-flash write did not happen. It still exits 0 for some of these.
_FLASH_FAILED = ("can't program spi flash", "error", "fail")

# Node-name suffixes after the serial, per macOS driver: Apple's names the interface by digit, FTDI's VCP driver
# by channel letter.
_SUFFIX_INTERFACE = {"0": 0, "1": 1, "A": 0, "B": 1}


def interface_of(device, serial, location):
    """The USB interface number a listed device carries for the board `serial`, or None if it is not that board's."""
    if location and ":" in location:
        config_iface = location.rsplit(":", 1)[1]
        if "." in config_iface:
            n = config_iface.rsplit(".", 1)[1]
            if n.isdigit():
                return int(n)
    name = os.path.basename(device)
    if serial and serial in name:
        return _SUFFIX_INTERFACE.get(name.split(serial, 1)[1])
    return None


def pick_port(listing, serial, interface):
    """The device in `listing` — rows of (device, serial number, location) — carrying (serial, interface).

    A row belongs to the board when the OS reports its serial or names the node after it. A callout node wins
    over a dial-in node for the same channel."""
    hits = []
    for device, sn, location in listing:
        name = os.path.basename(device)
        if sn != serial and serial not in name:
            continue
        if interface_of(device, serial, location) == interface:
            hits.append(device)
    hits.sort(key=lambda d: (not os.path.basename(d).startswith("cu."), d))
    return hits[0] if hits else None


def listing():
    """Every serial device this machine reports, as (device, serial number, location) rows."""
    from serial.tools import list_ports

    return [(p.device, p.serial_number or "", p.location or "") for p in list_ports.comports()]


def port_for(serial, interface):
    """The device carrying (FTDI serial, USB interface) on this machine, or None when it is not attached."""
    return pick_port(listing(), serial, interface)


def ports_for(serial):
    """(jtag device, byte-channel device) for a board, each None when absent."""
    jtag_if = BOARDS[serial][3]
    rows = listing()
    return pick_port(rows, serial, jtag_if), pick_port(rows, serial, 1 - jtag_if)


def byte_port(serial):
    """The device carrying a board's byte channel (its UART bridge or FT245 FIFO), or None when absent."""
    return ports_for(serial)[1]


def board_serial(board):
    """The FTDI serial of a board named by letter (A or B, either case) or by serial."""
    for serial, (letter, *_) in BOARDS.items():
        if board.upper() in (letter, serial):
            return serial
    sys.exit(f"no board {board!r}: the boards are " + ", ".join(f"{v[0]} ({v[1]}, FTDI {k})" for k, v in BOARDS.items()))


def require_byte_port(board):
    """A board's byte channel, or exit naming the board when it is not attached."""
    serial = board_serial(board)
    port = byte_port(serial)
    if port is None:
        letter, part = BOARDS[serial][:2]
        sys.exit(f"Board {letter} ({part}, FTDI {serial}) is not attached — plug it in and power it on")
    return port


def port_arg(value, board=None):
    """A host's `--port` value as a device: a board letter or serial resolves to that board's byte channel, any
    other value is a device already. With no value, `board`'s channel — or None when the host has no board to
    default to and runs offline without one."""
    if value is None:
        return None if board is None else require_byte_port(board)
    upper = value.upper()
    if any(upper in (letter, serial) for serial, (letter, *_) in BOARDS.items()):
        return require_byte_port(value)
    return value


def sudo_prefix(platform=None, env=None):
    """What precedes the loader on this OS. Linux runs it through `sudo -n` (the USB node is root-owned unless a
    udev rule grants it; `-n` fails at once instead of waiting on a password); macOS runs it as the user, where
    a sudo would stop mid-demo to ask for one. `LOGOS_CHIP_SUDO=1` or `0` forces either way."""
    platform = platform or sys.platform
    env = os.environ if env is None else env
    forced = env.get("LOGOS_CHIP_SUDO")
    if forced == "0":
        return []
    linux = platform.startswith("linux")
    if forced != "1" and not linux:
        return []
    if hasattr(os, "geteuid") and os.geteuid() == 0:
        return []
    return ["sudo", "-n"] if linux else ["sudo"]


def loader_argv(cable, serial, args, platform=None, env=None):
    """The openFPGALoader argv for one board. The loader is named by its full path, so a sudo whose secure PATH
    omits Homebrew's bin still runs the one the user's PATH finds."""
    exe = shutil.which("openFPGALoader") or "openFPGALoader"
    return sudo_prefix(platform, env) + [exe, "-c", cable, "--ftdi-serial", serial] + list(args)


# What libftdi reports when the OS's own FTDI driver holds the channel the loader needs.
_CLAIM_REFUSED = ("unable to claim usb device", "default ftdi driver", "libusb_error_access", "access denied",
                  "insufficient permissions")


def claim_refused(output):
    """Did the loader fail because the OS's FTDI driver holds the board's JTAG channel?"""
    low = output.lower()
    return any(s in low for s in _CLAIM_REFUSED)


def _run(argv, echo, timeout):
    """Run argv, returning (exit status, its combined output). `echo` streams the output as it comes instead of
    bounding the run by `timeout` seconds; a run past its bound returns 124, as `timeout(1)` does."""
    if not echo:
        try:
            r = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
        except subprocess.TimeoutExpired as e:
            return 124, (e.stdout or b"").decode(errors="replace") + f"\n(no answer within {timeout} s)"
        return r.returncode, r.stdout.decode(errors="replace")
    p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    chunks = []
    while True:
        b = p.stdout.read1(4096)
        if not b:
            break
        chunks.append(b)
        if echo:
            sys.stdout.buffer.write(b)
            sys.stdout.buffer.flush()
    return p.wait(), b"".join(chunks).decode(errors="replace")


def run_loader(cable, serial, args, echo=False, timeout=600, platform=None, env=None):
    """Run the loader for one board; returns (exit status, its output). On macOS only root may detach Apple's FTDI
    driver from a channel, so a claim the driver refuses is retried once under sudo, which asks for the password.
    Raises FileNotFoundError when the loader is not installed."""
    platform = platform or sys.platform
    argv = loader_argv(cable, serial, args, platform, env)
    code, out = _run(argv, echo, timeout)
    if code != 0 and platform == "darwin" and argv[0] != "sudo" and claim_refused(out):
        print("macOS's FTDI driver holds this board's JTAG channel: running the loader under sudo", file=sys.stderr)
        code, out = _run(["sudo"] + argv, echo, timeout)
    return code, out


def flash(board, bit, persist=False, freq=1_000_000):
    """Load a bitstream into a board's FPGA, or with `persist` write it to the board's configuration flash (read back
    and compared) so it boots at power-on. Returns the loader's exit status; its output streams to this process's
    stdout."""
    serial = board_serial(board)
    if not os.path.exists(bit):
        sys.exit(f"{bit}: no such bitstream")
    args = ["--freq", str(freq)]
    if persist:
        args += ["--write-flash", "--verify", "--fpga-part", FPGA_PART[serial]]
    try:
        code, out = run_loader(BOARDS[serial][2], serial, args + [bit], echo=True)
    except FileNotFoundError:
        sys.exit("openFPGALoader is not installed (macOS: `brew install openfpgaloader`)")
    if code == 0 and persist and any(s in out.lower() for s in _FLASH_FAILED):
        print("the configuration flash was not written (see the loader's output above)", file=sys.stderr)
        return 1
    return code


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("ports", help="every attached board's channels")
    po = sub.add_parser("port", help="print a board's byte channel; exit 1 when it is not attached")
    po.add_argument("board", help="A or B, or an FTDI serial")
    fl = sub.add_parser("flash", help="load a bitstream into a board's FPGA")
    fl.add_argument("board", help="A or B, or an FTDI serial")
    fl.add_argument("bit")
    fl.add_argument("--persist", action="store_true", help="write the configuration flash: the board boots it at power-on")
    fl.add_argument("--freq", type=int, default=1_000_000, help="JTAG clock in Hz (default 1 MHz)")
    r = sub.add_parser("resolve", help="print the device carrying (serial, interface); exit 1 when absent")
    r.add_argument("serial")
    r.add_argument("interface", type=int)
    r.add_argument("--listing", help="a JSON list of [device, serial, location] rows in place of this machine's")
    lo = sub.add_parser("loader", help="print the openFPGALoader argv for one board as JSON")
    lo.add_argument("--platform", default=sys.platform)
    lo.add_argument("--cable", required=True)
    lo.add_argument("--serial", required=True)
    lo.add_argument("args", nargs=argparse.REMAINDER)
    a = ap.parse_args()

    if a.cmd == "ports":
        rows = listing()
        for serial, (name, part, cable, jtag_if) in BOARDS.items():
            jtag, data = pick_port(rows, serial, jtag_if), pick_port(rows, serial, 1 - jtag_if)
            print(f"Board {name}  {part}  serial {serial}  cable {cable}  jtag {jtag or '-'}  bytes {data or '-'}")
        return 0
    if a.cmd == "port":
        print(require_byte_port(a.board))
        return 0
    if a.cmd == "flash":
        return flash(a.board, a.bit, a.persist, a.freq)
    if a.cmd == "resolve":
        rows = [tuple(row) for row in json.loads(a.listing)] if a.listing else listing()
        dev = pick_port(rows, a.serial, a.interface)
        if dev is None:
            return 1
        print(dev)
        return 0
    args = a.args[1:] if a.args[:1] == ["--"] else a.args
    print(json.dumps(loader_argv(a.cable, a.serial, args, a.platform)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
