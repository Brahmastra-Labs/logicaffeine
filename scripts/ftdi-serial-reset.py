#!/usr/bin/env python3
"""Reset a Digilent FT2232's channels back to UART/serial mode via FTDI SET_BITMODE.

openFPGALoader drives a JTAG channel in MPSSE/bitbang mode and leaves it there, which wedges the
board's USB-UART (opens/tcdrain hang, no data flows) until the FTDI bitmode is reset. This detaches
ftdi_sio, issues SET_BITMODE(reset) to both channels over usbfs, and reattaches ftdi_sio so the
/dev/ttyUSB UART works again. Needed on the Nexys Video (Board A) between a JTAG flash and talking to the
design over UART. Linux only (usbfs and ftdi_sio); run as root.

  sudo python3 scripts/ftdi-serial-reset.py [SERIAL]      # default SERIAL = 210276C08FE0 (Board A)
"""
import ctypes, fcntl, glob, os, struct, sys, time

SERIAL = sys.argv[1] if len(sys.argv) > 1 else "210276C08FE0"

# usbfs ioctls (Linux usbdevice_fs.h)
USBDEVFS_CONTROL          = 0xC0185500  # _IOWR('U', 0, struct usbdevfs_ctrltransfer)
USBDEVFS_CLAIMINTERFACE   = 0x8004550F  # _IOR('U', 15, unsigned int)
USBDEVFS_RELEASEINTERFACE = 0x80045510  # _IOR('U', 16, unsigned int)

# FTDI vendor request
SIO_SET_BITMODE = 0x0B
BITMODE_RESET   = 0x00


class ctrltransfer(ctypes.Structure):
    _fields_ = [("bRequestType", ctypes.c_uint8), ("bRequest", ctypes.c_uint8),
                ("wValue", ctypes.c_uint16), ("wIndex", ctypes.c_uint16),
                ("wLength", ctypes.c_uint16), ("timeout", ctypes.c_uint32),
                ("data", ctypes.c_void_p)]


def find_dev():
    for sf in glob.glob("/sys/bus/usb/devices/*/serial"):
        try:
            if open(sf).read().strip() == SERIAL:
                d = os.path.dirname(sf)
                return d, int(open(d + "/busnum").read()), int(open(d + "/devnum").read())
        except OSError:
            pass
    raise SystemExit(f"no USB device with serial {SERIAL}")


def main():
    devdir, bus, dev = find_dev()
    base = os.path.basename(devdir)                       # e.g. 1-5.2
    ifaces = [f"{base}:1.0", f"{base}:1.1"]               # channel A, B
    drv = "/sys/bus/usb/drivers/ftdi_sio"

    for i in ifaces:                                      # detach the UART driver so we can claim
        if os.path.exists(f"{drv}/{i}"):
            try: open(f"{drv}/unbind", "w").write(i)
            except OSError as e: print(f"  unbind {i}: {e}")
    time.sleep(0.3)

    node = f"/dev/bus/usb/{bus:03d}/{dev:03d}"
    fd = os.open(node, os.O_RDWR)
    try:
        for ifnum in (0, 1):                             # reset bitmode on both channels
            try:
                fcntl.ioctl(fd, USBDEVFS_CLAIMINTERFACE, struct.pack("I", ifnum))
                ct = ctrltransfer(0x40, SIO_SET_BITMODE, (BITMODE_RESET << 8) | 0x00, ifnum + 1, 0, 1000, None)
                fcntl.ioctl(fd, USBDEVFS_CONTROL, ct)
                fcntl.ioctl(fd, USBDEVFS_RELEASEINTERFACE, struct.pack("I", ifnum))
                print(f"  SET_BITMODE reset -> channel {'AB'[ifnum]} (if{ifnum:02d})")
            except OSError as e:
                print(f"  channel {'AB'[ifnum]}: {e}")
    finally:
        os.close(fd)

    time.sleep(0.3)
    for i in ifaces:                                      # reattach the UART driver
        try: open(f"{drv}/bind", "w").write(i)
        except OSError as e:
            if e.errno not in (16, 2): print(f"  bind {i}: {e}")
    time.sleep(0.5)
    print(f"done — {SERIAL} channels reset to serial mode")


if __name__ == "__main__":
    main()
