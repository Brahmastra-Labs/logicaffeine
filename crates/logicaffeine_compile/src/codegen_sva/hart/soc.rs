//! The system around the hart on Board B, defined once: the physical memory map, the device registers, how the
//! 128 MB of DDR2 is laid out for the firmware, and the flash image the boot ROM loads. The hart model reads the map
//! as its physical memory attributes, the RTL takes it as parameters ([`verilog_defines`]), and the boot ROM and the
//! shim compile against the C header this module writes ([`c_header`]) — so no address is ever written twice.

use std::collections::VecDeque;

use super::model::{Bus, BusError, Hart, Region};

/// The boot ROM: the hart's reset vector. Address 0 stays unmapped, so a null pointer faults.
pub const ROM_BASE: u64 = 0x0001_0000;
pub const ROM_SIZE: u64 = 0x4000;
/// Scratch RAM in block RAM: the boot ROM's stack and data before the DDR2 is calibrated. It never holds code.
pub const SRAM_BASE: u64 = 0x0002_0000;
pub const SRAM_SIZE: u64 = 0x2000;
/// Device registers.
pub const IO_BASE: u64 = 0x1000_0000;
pub const IO_SIZE: u64 = 0x1_0000;
/// The DDR2.
pub const RAM_BASE: u64 = 0x8000_0000;
pub const RAM_SIZE: u64 = 128 << 20;

pub const RESET_PC: u64 = ROM_BASE;

/// The physical memory map the hart enforces.
pub fn memory_map() -> Vec<Region> {
    vec![
        Region::rom(ROM_BASE, ROM_SIZE),
        Region::data(SRAM_BASE, SRAM_SIZE),
        Region::io(IO_BASE, IO_SIZE),
        Region::memory(RAM_BASE, RAM_SIZE),
    ]
}

/// A device register: its name in the C header and its address.
pub struct Reg {
    pub name: &'static str,
    pub addr: u64,
    pub doc: &'static str,
}

const fn reg(name: &'static str, addr: u64, doc: &'static str) -> Reg {
    Reg { name, addr, doc }
}

pub const SYS: u64 = IO_BASE;
pub const UART: u64 = IO_BASE + 0x1000;
pub const LINK: u64 = IO_BASE + 0x2000;
pub const BUTTONS: u64 = IO_BASE + 0x3000;
pub const QSPI: u64 = IO_BASE + 0x4000;
pub const DDR: u64 = IO_BASE + 0x5000;

/// Every device register.
pub const REGS: &[Reg] = &[
    reg("SYS_ID", SYS, "read: the board's identity, SOC_ID"),
    reg("SYS_CYCLES", SYS + 0x08, "read: clock cycles since reset"),
    reg("SYS_FLAGS", SYS + 0x10, "read: bit 0 the DDR2 is ready, bit 1 its calibration failed"),
    reg("SYS_RTL_MD5_LO", SYS + 0x18, "read: the first 8 bytes of the bitstream's RTL fingerprint"),
    reg("SYS_RTL_MD5_HI", SYS + 0x20, "read: the last 8 bytes of the bitstream's RTL fingerprint"),
    reg("SYS_SWITCHES", SYS + 0x28, "read: the 16 slide switches"),
    reg("SYS_RESET", SYS + 0x30, "write SOC_RESET_KEY: restart the board from its reset vector"),
    reg("SYS_EXIT", SYS + 0x38, "write: the firmware's exit status; the board stops there"),
    reg("UART_TX", UART, "write: one byte to the log UART"),
    reg("UART_STATUS", UART + 0x08, "read: bit 0 the transmit queue is full"),
    reg("LINK_TX", LINK, "write: one byte into the terminal link, as the laptop would send it"),
    reg("LINK_RX", LINK + 0x08, "read: bit 8 a byte is there, bits 7:0 the byte (the read takes it)"),
    reg("LINK_STATUS", LINK + 0x10, "read: bit 0 the transmit queue is full, bit 1 a byte is waiting"),
    reg("BUTTONS_STATE", BUTTONS, "read: the debounced buttons: bit 0 centre, 1 up, 2 left, 3 right, 4 down"),
    reg("BUTTONS_LEDS", BUTTONS + 0x08, "read/write: the 16 LEDs"),
    reg("QSPI_START", QSPI, "write: begin a streaming read of the flash at this byte address (it wraps at the end)"),
    reg("QSPI_DATA", QSPI + 0x08, "read: the next 8 bytes of the stream (the read waits for them); all ones with no stream"),
    reg("QSPI_STOP", QSPI + 0x10, "write: end the stream"),
    reg("DDR_STATUS", DDR, "read: bit 0 calibrated, bit 1 failed; bits 15:8 the read latency found"),
    reg("DDR_CAL", DDR + 0x08, "read: the calibration's per-lane taps"),
];

/// `SYS_ID`: "LOGOSHRT" read as a little-endian word.
pub const SOC_ID: u64 = u64::from_le_bytes(*b"LOGOSHRT");
/// The value `SYS_RESET` takes to restart the board.
pub const SOC_RESET_KEY: u64 = 0x5245_5345_545f_4b45;

/// How the DDR2 is laid out once the shim runs: the shim itself, the firmware's image and heap, the region its
/// anonymous mappings come from, the boot image's staging area, and the firmware's stack.
pub const SHIM_BASE: u64 = RAM_BASE;
pub const SHIM_SIZE: u64 = 0x20_0000;
pub const FIRMWARE_BASE: u64 = RAM_BASE + SHIM_SIZE;
pub const MMAP_BASE: u64 = 0x8400_0000;
pub const MMAP_SIZE: u64 = 0x200_0000;
pub const STAGING_BASE: u64 = 0x8600_0000;
pub const STAGING_SIZE: u64 = 0x180_0000;
pub const STACK_BASE: u64 = 0x8780_0000;
pub const STACK_SIZE: u64 = 0x80_0000;
/// The boot ROM's working tables (the inflater's), in the mapping region: nothing uses it before the compiler runs,
/// and the shim zeroes every page it hands out there.
pub const BOOT_WORK_BASE: u64 = MMAP_BASE;

/// The flash: the bitstream at 0, the boot image here.
pub const FLASH_SIZE: u64 = 16 << 20;
pub const IMAGE_OFFSET: u64 = 0x40_0000;

/// The boot image's header: `IMAGE_MAGIC`, a version and the header's size, the RTL fingerprint it was built for,
/// the body's inflated and compressed lengths, and the sha256 of the compressed body.
pub const IMAGE_MAGIC: u64 = u64::from_le_bytes(*b"LOGOSIMG");
pub const IMAGE_VERSION: u32 = 1;
pub const IMAGE_HEADER_SIZE: u64 = 128;

/// A section of the boot image's body.
pub const SECTION_SHIM: u32 = 1;
pub const SECTION_FIRMWARE: u32 = 2;
pub const SECTION_BUNDLE: u32 = 3;
pub const SECTION_ARGS: u32 = 4;

/// Every constant the firmware compiles against, by its C name.
pub const CONSTANTS: &[(&str, u64)] = &[
    ("ROM_BASE", ROM_BASE),
    ("ROM_SIZE", ROM_SIZE),
    ("SRAM_BASE", SRAM_BASE),
    ("SRAM_SIZE", SRAM_SIZE),
    ("IO_BASE", IO_BASE),
    ("IO_SIZE", IO_SIZE),
    ("RAM_BASE", RAM_BASE),
    ("RAM_SIZE", RAM_SIZE),
    ("RESET_PC", RESET_PC),
    ("SOC_ID", SOC_ID),
    ("SOC_RESET_KEY", SOC_RESET_KEY),
    ("SHIM_BASE", SHIM_BASE),
    ("SHIM_SIZE", SHIM_SIZE),
    ("FIRMWARE_BASE", FIRMWARE_BASE),
    ("MMAP_BASE", MMAP_BASE),
    ("MMAP_SIZE", MMAP_SIZE),
    ("STAGING_BASE", STAGING_BASE),
    ("STAGING_SIZE", STAGING_SIZE),
    ("STACK_BASE", STACK_BASE),
    ("STACK_SIZE", STACK_SIZE),
    ("BOOT_WORK_BASE", BOOT_WORK_BASE),
    ("FLASH_SIZE", FLASH_SIZE),
    ("IMAGE_OFFSET", IMAGE_OFFSET),
    ("IMAGE_MAGIC", IMAGE_MAGIC),
    ("IMAGE_VERSION", IMAGE_VERSION as u64),
    ("IMAGE_HEADER_SIZE", IMAGE_HEADER_SIZE),
    ("SECTION_SHIM", SECTION_SHIM as u64),
    ("SECTION_FIRMWARE", SECTION_FIRMWARE as u64),
    ("SECTION_BUNDLE", SECTION_BUNDLE as u64),
    ("SECTION_ARGS", SECTION_ARGS as u64),
    ("HART_HZ", HART_HZ),
    ("HART_HWCAP", HART_HWCAP),
];

/// The extensions the compiler sees, as Linux reports them in `AT_HWCAP` (a bit per extension letter): the hart's
/// I, M, A and C, and the F and D the shim provides — exactly what `qemu-riscv64` reports for the same binary.
pub const HART_HWCAP: u64 = letter(b'i') | letter(b'm') | letter(b'a') | letter(b'f') | letter(b'd') | letter(b'c');

const fn letter(c: u8) -> u64 {
    1 << (c - b'a')
}

/// The hart's clock: the DDR2 controller's divided clock, which the hart shares.
pub const HART_HZ: u64 = 62_500_000;

/// The C header the boot ROM and the shim compile against.
pub fn c_header() -> String {
    let mut s = String::from(
        "// Generated from codegen_sva::hart::soc — the one definition of the board's addresses. Do not edit.\n\
         #pragma once\n\n",
    );
    for (name, v) in CONSTANTS {
        s.push_str(&format!("#define {name} 0x{v:x}ull\n"));
    }
    s.push('\n');
    for r in REGS {
        s.push_str(&format!("// {}\n#define {} 0x{:x}ull\n", r.doc, r.name, r.addr));
    }
    s
}

/// The RTL's view of the same map, as `define`s the SoC's Verilog includes (once, however many files include it).
pub fn verilog_defines() -> String {
    let mut s = String::from("// Generated from codegen_sva::hart::soc. Do not edit.\n`ifndef SOC_DEFINES_VH\n`define SOC_DEFINES_VH\n");
    for (name, v) in CONSTANTS {
        s.push_str(&format!("`define SOC_{name} 64'h{v:016x}\n"));
    }
    for r in REGS {
        s.push_str(&format!("`define SOC_{} 64'h{:016x}\n", r.name, r.addr));
    }
    s.push_str("`endif\n");
    s
}

/// The board's devices, as the model of the system around the hart: memory, the flash behind the streaming port,
/// and every register in [`REGS`] with the behaviour its line documents. The RTL SoC is held to these. The bus
/// completes every access the memory map permits ([`memory_map`]): a device address no register holds reads as
/// zero and ignores writes.
pub struct Devices {
    rom: Vec<u8>,
    sram: Vec<u8>,
    ram: Vec<u8>,
    /// The flash: the bitstream's 4 MB, then the boot image.
    pub flash: Vec<u8>,
    /// Every byte written to the log UART.
    pub uart: Vec<u8>,
    /// Bytes the hart sent into the terminal link.
    pub link_out: VecDeque<u8>,
    /// Bytes waiting for the hart from the terminal link.
    pub link_in: VecDeque<u8>,
    pub buttons: u64,
    pub leds: u64,
    pub switches: u64,
    /// The bitstream's RTL fingerprint.
    pub rtl_md5: [u8; 16],
    /// Whether the DDR2 has calibrated (and whether it failed to).
    pub ddr_ready: bool,
    pub ddr_failed: bool,
    /// Clock cycles since reset, as `SYS_CYCLES` reads them.
    pub cycles: u64,
    /// The status the firmware stopped the board with.
    pub exit: Option<u64>,
    /// The firmware asked the board to restart.
    pub reset: bool,
    stream: Option<u64>,
}

fn read_le(bytes: &[u8], o: usize, size: u8) -> u64 {
    bytes[o..o + size as usize].iter().rev().fold(0, |v, &b| (v << 8) | b as u64)
}

fn write_le(bytes: &mut [u8], o: usize, size: u8, value: u64) {
    for (i, b) in bytes[o..o + size as usize].iter_mut().enumerate() {
        *b = (value >> (8 * i)) as u8;
    }
}

impl Devices {
    /// The devices at reset, with `rom` in the boot ROM and `flash` behind the streaming port.
    pub fn new(rom: &[u8], flash: Vec<u8>, rtl_md5: [u8; 16]) -> Self {
        assert!(rom.len() as u64 <= ROM_SIZE, "the boot ROM image is larger than the ROM");
        assert_eq!(flash.len() as u64, FLASH_SIZE, "the flash is {FLASH_SIZE} bytes");
        let mut rom_bytes = vec![0; ROM_SIZE as usize];
        rom_bytes[..rom.len()].copy_from_slice(rom);
        Devices {
            rom: rom_bytes,
            sram: vec![0; SRAM_SIZE as usize],
            ram: vec![0; RAM_SIZE as usize],
            flash,
            uart: Vec::new(),
            link_out: VecDeque::new(),
            link_in: VecDeque::new(),
            buttons: 0,
            leds: 0,
            switches: 0,
            rtl_md5,
            ddr_ready: true,
            ddr_failed: false,
            cycles: 0,
            exit: None,
            reset: false,
            stream: None,
        }
    }

    /// A whole-word read of a device register; an address no register holds reads as zero.
    fn io_read(&mut self, addr: u64) -> u64 {
        let md5 = |i: usize| u64::from_le_bytes(self.rtl_md5[8 * i..8 * i + 8].try_into().unwrap());
        match addr {
            a if a == SYS => SOC_ID,
            a if a == SYS + 0x08 => self.cycles,
            a if a == SYS + 0x10 => (self.ddr_ready as u64) | ((self.ddr_failed as u64) << 1),
            a if a == SYS + 0x18 => md5(0),
            a if a == SYS + 0x20 => md5(1),
            a if a == SYS + 0x28 => self.switches,
            a if a == UART + 0x08 => 0,
            a if a == LINK + 0x08 => self.link_in.pop_front().map(|b| 0x100 | b as u64).unwrap_or(0),
            a if a == LINK + 0x10 => (!self.link_in.is_empty() as u64) << 1,
            a if a == BUTTONS => self.buttons,
            a if a == BUTTONS + 0x08 => self.leds,
            a if a == QSPI + 0x08 => match self.stream {
                Some(at) => {
                    let word = (0..8)
                        .map(|i| self.flash[((at + i) & (FLASH_SIZE - 1)) as usize] as u64)
                        .rev()
                        .fold(0, |v, b| (v << 8) | b);
                    self.stream = Some((at + 8) & (FLASH_SIZE - 1));
                    word
                }
                None => u64::MAX,
            },
            a if a == DDR => (self.ddr_ready as u64) | ((self.ddr_failed as u64) << 1),
            _ => 0,
        }
    }

    /// A whole-word write of a device register; an address no register holds ignores it.
    fn io_write(&mut self, addr: u64, value: u64) {
        match addr {
            a if a == SYS + 0x30 => self.reset = value == SOC_RESET_KEY,
            a if a == SYS + 0x38 => self.exit = Some(value),
            a if a == UART => self.uart.push(value as u8),
            a if a == LINK => self.link_out.push_back(value as u8),
            a if a == BUTTONS + 0x08 => self.leds = value & 0xffff,
            a if a == QSPI => self.stream = Some(value & (FLASH_SIZE - 1)),
            a if a == QSPI + 0x10 => self.stream = None,
            _ => {}
        }
    }
}

impl Bus for Devices {
    fn read(&mut self, addr: u64, size: u8) -> Result<u64, BusError> {
        match addr {
            a if (ROM_BASE..ROM_BASE + ROM_SIZE).contains(&a) => Ok(read_le(&self.rom, (a - ROM_BASE) as usize, size)),
            a if (SRAM_BASE..SRAM_BASE + SRAM_SIZE).contains(&a) => Ok(read_le(&self.sram, (a - SRAM_BASE) as usize, size)),
            a if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&a) => Ok(read_le(&self.ram, (a - RAM_BASE) as usize, size)),
            a if (IO_BASE..IO_BASE + IO_SIZE).contains(&a) && size == 8 => Ok(self.io_read(a)),
            _ => Err(BusError),
        }
    }

    fn write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), BusError> {
        match addr {
            a if (SRAM_BASE..SRAM_BASE + SRAM_SIZE).contains(&a) => {
                write_le(&mut self.sram, (a - SRAM_BASE) as usize, size, value);
                Ok(())
            }
            a if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&a) => {
                write_le(&mut self.ram, (a - RAM_BASE) as usize, size, value);
                Ok(())
            }
            a if (IO_BASE..IO_BASE + IO_SIZE).contains(&a) && size == 8 => {
                self.io_write(a, value);
                Ok(())
            }
            _ => Err(BusError),
        }
    }
}

/// How a run of the board ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// The firmware wrote this status to `SYS_EXIT`.
    Exited(u64),
    /// The step limit came first.
    StepLimit,
}

/// The whole board: the hart at its reset vector over the devices.
pub struct Board {
    pub hart: Hart,
    pub devices: Devices,
}

impl Board {
    pub fn new(rom: &[u8], flash: Vec<u8>, rtl_md5: [u8; 16]) -> Self {
        Board {
            hart: Hart::new(RESET_PC, memory_map()),
            devices: Devices::new(rom, flash, rtl_md5),
        }
    }

    /// Run until the firmware exits or `steps` instructions have run.
    pub fn run(&mut self, steps: u64) -> Stopped {
        for _ in 0..steps {
            self.hart.step(&mut self.devices);
            self.devices.cycles += 1;
            if let Some(status) = self.devices.exit {
                return Stopped::Exited(status);
            }
            if self.devices.reset {
                self.devices.reset = false;
                self.hart = Hart::new(RESET_PC, memory_map());
            }
        }
        Stopped::StepLimit
    }
}
