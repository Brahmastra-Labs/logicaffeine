#!/usr/bin/env python3
"""ddr2-loopback — read Board B's DDR2 PHY probe (`hart_ddr2_loopback_top`, emitted by `onboard_image emit loopback`)
off its UART and say what the PHY's own pads read back: for each byte lane, the order the deserializer hands a frame's
four beats over, whether the lane reads inverted, and how many beats the round trip takes.

The probe drives beat b of frame k with c = (4k + b) mod 256 on lane 0 and 3c mod 256 on lane 1, and sends 0xA5, the
first frame number captured (2 bytes, little-endian), thirty-two captured frames of 64 bits (little-endian: beat 0's
lane 0 byte first), and 0x5A, once a second.

    python3 scripts/hw/ddr2-loopback.py [--board B] [--captures 3]
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
import ftdi_ports  # noqa: E402

FRAMES = 32


def capture(port):
    """One capture: (first frame number, [frame bytes × 32])."""
    while port.read(1) != b"\xa5":
        pass
    body = port.read(2 + 8 * FRAMES + 1)
    if len(body) != 2 + 8 * FRAMES + 1 or body[-1] != 0x5A:
        return None
    first = body[0] | body[1] << 8
    return first, [body[2 + 8 * i: 2 + 8 * (i + 1)] for i in range(FRAMES)]


def lane_bytes(frames, lane, reversed_beats):
    """A lane's bytes in arrival order, taking each frame's beats as the deserializer orders them."""
    out = []
    for f in frames:
        beats = [f[2 * b + lane] for b in range(4)]
        out.extend(reversed(beats) if reversed_beats else beats)
    return out


def explain(first, frames):
    """Which beat order and polarity make each lane the pattern, and where the pattern lands."""
    verdicts = []
    for lane in (0, 1):
        found = None
        for reversed_beats in (False, True):
            for inverted in (False, True):
                got = [b ^ (0xFF if inverted else 0) for b in lane_bytes(frames, lane, reversed_beats)]
                # Lane 1 carries 3c: recover c through 3's inverse mod 256 (171).
                c = got if lane == 0 else [(171 * x) % 256 for x in got]
                if all((c[i + 1] - c[i]) % 256 == 1 for i in range(len(c) - 1)):
                    sent = (4 * first) % 256
                    found = (reversed_beats, inverted, (sent - c[0]) % 256)
        verdicts.append(found)
    return verdicts


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--board", default="B")
    ap.add_argument("--captures", type=int, default=3)
    a = ap.parse_args()
    import serial

    port = serial.Serial(ftdi_ports.require_byte_port(a.board), 115200, timeout=3)
    for n in range(a.captures):
        got = capture(port)
        if got is None:
            print(f"capture {n}: damaged")
            continue
        first, frames = got
        for lane, v in enumerate(explain(first, frames)):
            if v is None:
                print(f"capture {n} lane {lane}: no beat order or polarity makes this the pattern")
            else:
                order, inverted, behind = v
                print(f"capture {n} lane {lane}: FIRST_BEAT_ON_Q4={0 if order else 1} inverted={int(inverted)} "
                      f"round trip {behind} beats")


if __name__ == "__main__":
    main()
