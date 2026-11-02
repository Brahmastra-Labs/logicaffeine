#!/usr/bin/env python3
"""wpe-echo-test — send bytes to the loopback diagnostic bitstream and check they echo back. Confirms the core
clock, both UART pins, and the input route all work (isolating engine-logic faults from infrastructure faults)."""
import os
import sys
import serial

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402

port = ftdi_ports.port_arg(sys.argv[1] if len(sys.argv) > 1 else None, "B")
baud = int(sys.argv[2]) if len(sys.argv) > 2 else 9600
probe = bytes([0x5A, 0xA5, 0x01, 0xFF, 0x42, 0x00, 0x7E])
with serial.Serial(port, baud, timeout=3) as ser:
    ser.reset_input_buffer()
    ser.write(probe)
    ser.flush()
    got = ser.read(len(probe))
print(f"sent: {probe.hex()}")
print(f"got:  {got.hex()}")
if got == probe:
    print("ECHO OK — clock + both UART pins + input route all WORK (engine no-reply is engine LOGIC)")
elif len(got) == 0:
    print("NO ECHO — infrastructure fault (clock / UART pin / input route dead)")
else:
    print(f"PARTIAL/GARBLED ECHO ({len(got)}/{len(probe)} bytes) — baud/clock mismatch or marginal I/O")
