"""board_line — a board's byte channel, found by its letter through ftdi_ports and opened through pyserial.

Board B's channel is the FT2232's UART bridge at 115200 8N1; Board A's is its asynchronous 245 FIFO, which the OS presents
as a serial port whose baud the FIFO ignores. Every host that talks to a board opens it here.
"""
import time

import ftdi_ports


class Line:
    """A board's byte channel through the OS's serial driver."""

    def __init__(self, board):
        import serial  # imported late so a host's --dry-run and --replay need no pyserial

        self.port = serial.Serial(ftdi_ports.require_byte_port(board), 115200, timeout=0.05)
        # An FTDI bridge holds a short reply until its latency timer (16 ms by default) runs out; the low-latency mode
        # sets it to 1 ms, so a host that waits on each reply waits on the board, not the timer. Linux only, and a driver
        # may refuse it: the line works the same without, only slower.
        try:
            self.port.set_low_latency_mode(True)
        except (AttributeError, NotImplementedError, OSError, ValueError):
            pass
        # A byte the reset or the configuration left on the line would sit ahead of the first reply: settle, drain.
        time.sleep(0.3)
        self.port.read(256)
        self.port.reset_input_buffer()

    def send(self, data):
        self.port.write(data)

    def recv(self, n, deadline):
        buf = bytearray()
        while len(buf) < n and time.time() < deadline:
            buf += self.port.read(n - len(buf))
        return buf

    def close(self):
        self.port.close()
