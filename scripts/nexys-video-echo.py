#!/usr/bin/env python3
"""Board-A (Nexys Video) UART echo smoke test.

The nexys_video_bridge_loopback bitstream echoes each byte it receives on the FT245 byte channel. This sends a
test pattern and checks it comes back byte-for-byte. The Nexys Video swaps the FT2232 channels vs the Nexys A7
(JTAG is on channel B), so the byte channel is channel A (interface 00). An image that does not read the channel
leaves the pattern undrained: no echo comes back, and closing the port waits out the driver's drain timeout.

  python3 scripts/lib/ftdi_ports.py flash A benchmarks/silicon/nexys_video_bridge_loopback.bit We try both of Board A's channels, found by its FTDI serial, and
report whichever echoes.

  python3 scripts/nexys-video-echo.py               # auto-try both channels of Board A
  python3 scripts/nexys-video-echo.py --port DEV
"""
import argparse, os, sys, time
import serial

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

PATTERN = bytes("LOGOS on the 200T!\n", "ascii") + bytes([0x00, 0x55, 0xAA, 0xFF, 0x42, 0x7E])


def try_port(port):
    try:
        s = serial.Serial(port, 115200, timeout=1.5, write_timeout=2.0, rtscts=False, dsrdtr=False)
    except Exception as e:
        return None, f"open-fail: {e}"
    try:
        time.sleep(0.2)
        s.reset_input_buffer()
        try:
            s.write(PATTERN)
        except serial.SerialTimeoutException:
            return None, "write stalled: nothing on the FPGA side is reading this channel"
        deadline = time.time() + 2.0
        got = b""
        while len(got) < len(PATTERN) and time.time() < deadline:
            chunk = s.read(len(PATTERN) - len(got))
            if not chunk:
                break
            got += chunk
        return got, None
    finally:
        # Discard what never drained, so closing does not wait on a channel nobody reads.
        s.reset_output_buffer()
        s.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="explicit port; default = auto-try both channels")
    args = ap.parse_args()
    # The byte channel first: the JTAG channel can be left in MPSSE mode by the loader, where a write never drains.
    jtag, data = ftdi_ports.ports_for(ftdi_ports.BOARD_A)
    ports = [args.port] if args.port else [p for p in (data, jtag) if p]
    if not ports:
        sys.exit(f"Board A (FTDI {ftdi_ports.BOARD_A}) is not attached — plug it in and power it on")

    for port in ports:
        got, err = try_port(port)
        label = port.split("/")[-1]
        if err:
            print(f"[{label}] {err}")
            continue
        ok = got == PATTERN
        print(f"[{label}] sent {len(PATTERN)}B, recv {len(got)}B: {got!r}  -> {'ECHO OK' if ok else 'no/partial echo'}")
        if ok:
            print(f"\nPASS — Board A (Nexys Video, XC7A200T) echoes over UART on {label}.")
            sys.exit(0)
    print("\nFAIL — no channel echoed the pattern.", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
