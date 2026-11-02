//! The executable specification of the hart: an RV64IMAC processor with the Zicsr and Zifencei extensions, machine
//! and user modes and eight physical-memory-protection entries, one architectural step at a time. The hart's RTL is
//! held to this model instruction by instruction, and this model is held to the RISC-V architecture tests and to
//! `qemu-riscv64`.
//!
//! What the hart leaves to software, by design: there is no floating-point unit, so every F and D instruction (and
//! every access to `fflags`, `frm` or `fcsr`) is an illegal instruction the machine-mode firmware emulates; and a
//! misaligned load, store or atomic traps with its address in `mtval` rather than being split in hardware. Neither
//! occurs often enough in the compiler to matter: the measured floating-point share of a compile is 0.0002% of its
//! instructions.

use super::rvc;

/// `misa`: RV64 with the A, C, I, M and U extensions.
pub const MISA: u64 = (2 << 62) | (1 << 0) | (1 << 2) | (1 << 8) | (1 << 12) | (1 << 20);

/// The number of physical-memory-protection entries the hart implements; entries 8 to 15 read as zero.
pub const PMP_ENTRIES: usize = 8;

/// A privilege mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privilege {
    User = 0,
    Machine = 3,
}

/// A synchronous exception: its `mcause` and the value `mtval` receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trap {
    pub cause: u64,
    pub tval: u64,
}

pub const CAUSE_FETCH_ACCESS: u64 = 1;
pub const CAUSE_ILLEGAL_INSTRUCTION: u64 = 2;
pub const CAUSE_BREAKPOINT: u64 = 3;
pub const CAUSE_MISALIGNED_LOAD: u64 = 4;
pub const CAUSE_LOAD_ACCESS: u64 = 5;
pub const CAUSE_MISALIGNED_STORE: u64 = 6;
pub const CAUSE_STORE_ACCESS: u64 = 7;
pub const CAUSE_USER_ECALL: u64 = 8;
pub const CAUSE_MACHINE_ECALL: u64 = 11;

/// A physical access the bus refuses (no device at the address).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusError;

/// The hart's view of the system: naturally aligned reads and writes of 1, 2, 4 or 8 bytes.
pub trait Bus {
    fn read(&mut self, addr: u64, size: u8) -> Result<u64, BusError>;
    fn write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), BusError>;
}

/// A range of physical addresses and what it may be used for: its physical memory attributes. An access outside
/// every region, or one its region does not permit, is an access fault before it reaches the bus — so a device
/// register is never fetched from, never the target of an atomic, and never read or written in part. These
/// attributes are the only source of access faults: every access they permit, the bus completes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub base: u64,
    pub size: u64,
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    pub atomic: bool,
    /// Every access must be exactly this many bytes (device registers are whole words).
    pub width: Option<u64>,
}

impl Region {
    /// Memory: every kind of access.
    pub const fn memory(base: u64, size: u64) -> Self {
        Region {
            base,
            size,
            read: true,
            write: true,
            execute: true,
            atomic: true,
            width: None,
        }
    }

    /// Memory that holds data and never code.
    pub const fn data(base: u64, size: u64) -> Self {
        Region {
            base,
            size,
            read: true,
            write: true,
            execute: false,
            atomic: true,
            width: None,
        }
    }

    /// Read-only memory that holds code.
    pub const fn rom(base: u64, size: u64) -> Self {
        Region {
            base,
            size,
            read: true,
            write: false,
            execute: true,
            atomic: false,
            width: None,
        }
    }

    /// Device registers: whole-word loads and stores only.
    pub const fn io(base: u64, size: u64) -> Self {
        Region {
            base,
            size,
            read: true,
            write: true,
            execute: false,
            atomic: false,
            width: Some(8),
        }
    }

    fn holds(&self, addr: u64, size: u64) -> bool {
        addr >= self.base && addr.wrapping_add(size) <= self.base.wrapping_add(self.size) && addr.wrapping_add(size) > addr
    }
}

/// What one step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The instruction completed and retired.
    Retired,
    /// The instruction took a trap; the hart is now at `mtvec` in machine mode.
    Trapped(Trap),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
    Execute,
    Atomic,
}

/// The architectural state of the hart.
#[derive(Clone, Debug)]
pub struct Hart {
    pub pc: u64,
    pub x: [u64; 32],
    pub privilege: Privilege,
    pub mie_bit: bool,
    pub mpie: bool,
    pub mpp: Privilege,
    pub mprv: bool,
    pub tw: bool,
    pub mie: u64,
    pub mtvec: u64,
    pub mscratch: u64,
    pub mepc: u64,
    pub mcause: u64,
    pub mtval: u64,
    pub mcounteren: u64,
    pub mcountinhibit: u64,
    pub mcycle: u64,
    pub minstret: u64,
    pub pmpcfg: [u8; PMP_ENTRIES],
    pub pmpaddr: [u64; PMP_ENTRIES],
    /// The address an `LR` reserved, until an `SC`, a trap or `mret` releases it.
    pub reservation: Option<u64>,
    /// The physical memory map.
    pub pma: Vec<Region>,
}

/// `mstatus.UXL`: user mode runs at 64 bits.
const MSTATUS_UXL: u64 = 2 << 32;

const PMP_R: u8 = 1;
const PMP_W: u8 = 2;
const PMP_X: u8 = 4;
const PMP_A: u8 = 3 << 3;
const PMP_L: u8 = 1 << 7;
const PMP_TOR: u8 = 1 << 3;
const PMP_NA4: u8 = 2 << 3;
const PMP_NAPOT: u8 = 3 << 3;
/// `pmpaddr` holds bits 55..2 of a 56-bit physical address.
const PMPADDR_MASK: u64 = (1 << 54) - 1;

fn sext32(v: u64) -> u64 {
    v as u32 as i32 as i64 as u64
}

fn illegal(inst: u32) -> Trap {
    Trap {
        cause: CAUSE_ILLEGAL_INSTRUCTION,
        tval: inst as u64,
    }
}

impl Hart {
    /// The hart as reset leaves it: machine mode at `reset_pc`, interrupts off, every protection entry off, over the
    /// physical memory map `pma`.
    pub fn new(reset_pc: u64, pma: Vec<Region>) -> Self {
        Hart {
            pc: reset_pc,
            x: [0; 32],
            privilege: Privilege::Machine,
            mie_bit: false,
            mpie: false,
            mpp: Privilege::User,
            mprv: false,
            tw: false,
            mie: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mcounteren: 0,
            mcountinhibit: 0,
            mcycle: 0,
            minstret: 0,
            pmpcfg: [0; PMP_ENTRIES],
            pmpaddr: [0; PMP_ENTRIES],
            reservation: None,
            pma,
        }
    }

    /// Whether the physical memory map permits the access.
    fn pma_allows(&self, addr: u64, size: u64, access: Access) -> bool {
        self.pma.iter().any(|r| {
            r.holds(addr, size)
                && r.width.map_or(true, |w| w == size)
                && match access {
                    Access::Read => r.read,
                    Access::Write => r.write,
                    Access::Execute => r.execute,
                    Access::Atomic => r.atomic,
                }
        })
    }

    pub fn mstatus(&self) -> u64 {
        MSTATUS_UXL
            | ((self.mie_bit as u64) << 3)
            | ((self.mpie as u64) << 7)
            | ((self.mpp as u64) << 11)
            | ((self.mprv as u64) << 17)
            | ((self.tw as u64) << 21)
    }

    fn write_mstatus(&mut self, v: u64) {
        self.mie_bit = v & (1 << 3) != 0;
        self.mpie = v & (1 << 7) != 0;
        self.mpp = if (v >> 11) & 3 == 3 {
            Privilege::Machine
        } else {
            Privilege::User
        };
        self.mprv = v & (1 << 17) != 0;
        self.tw = v & (1 << 21) != 0;
    }

    /// The privilege a data access runs at: `mstatus.MPRV` lends machine-mode loads and stores `MPP`'s.
    fn data_privilege(&self) -> Privilege {
        if self.mprv && self.privilege == Privilege::Machine {
            self.mpp
        } else {
            self.privilege
        }
    }

    /// The byte range `[lo, hi)` protection entry `i` covers, or `None` when the entry is off.
    fn pmp_range(&self, i: usize) -> Option<(u64, u64)> {
        let a = self.pmpcfg[i] & PMP_A;
        let addr = self.pmpaddr[i];
        match a {
            PMP_TOR => {
                let lo = if i == 0 { 0 } else { self.pmpaddr[i - 1] << 2 };
                Some((lo, addr << 2))
            }
            PMP_NA4 => Some((addr << 2, (addr << 2) + 4)),
            PMP_NAPOT => {
                let ones = (!addr).trailing_zeros().min(54) as u64;
                let size_log2 = ones + 3;
                let lo = (addr & !((1u64 << ones) - 1)) << 2;
                Some((lo, lo.wrapping_add(1u64 << size_log2)))
            }
            _ => None,
        }
    }

    /// Whether an access of `size` bytes at `addr` is permitted to `privilege`.
    fn pmp_allows(&self, addr: u64, size: u64, access: Access, privilege: Privilege) -> bool {
        let end = addr.wrapping_add(size);
        for i in 0..PMP_ENTRIES {
            let Some((lo, hi)) = self.pmp_range(i) else { continue };
            if lo >= hi {
                continue;
            }
            let any = addr < hi && end > lo;
            if !any {
                continue;
            }
            let all = addr >= lo && end <= hi;
            if !all {
                return false;
            }
            let cfg = self.pmpcfg[i];
            if privilege == Privilege::Machine && cfg & PMP_L == 0 {
                return true;
            }
            return match access {
                Access::Read => cfg & PMP_R != 0,
                Access::Write => cfg & PMP_W != 0,
                Access::Execute => cfg & PMP_X != 0,
                Access::Atomic => cfg & PMP_R != 0 && cfg & PMP_W != 0,
            };
        }
        privilege == Privilege::Machine
    }

    fn load(&mut self, bus: &mut impl Bus, addr: u64, size: u8) -> Result<u64, Trap> {
        if addr % size as u64 != 0 {
            return Err(Trap {
                cause: CAUSE_MISALIGNED_LOAD,
                tval: addr,
            });
        }
        let fault = Trap {
            cause: CAUSE_LOAD_ACCESS,
            tval: addr,
        };
        if !self.pmp_allows(addr, size as u64, Access::Read, self.data_privilege())
            || !self.pma_allows(addr, size as u64, Access::Read)
        {
            return Err(fault);
        }
        bus.read(addr, size).map_err(|_| fault)
    }

    fn store(&mut self, bus: &mut impl Bus, addr: u64, size: u8, value: u64) -> Result<(), Trap> {
        if addr % size as u64 != 0 {
            return Err(Trap {
                cause: CAUSE_MISALIGNED_STORE,
                tval: addr,
            });
        }
        let fault = Trap {
            cause: CAUSE_STORE_ACCESS,
            tval: addr,
        };
        if !self.pmp_allows(addr, size as u64, Access::Write, self.data_privilege())
            || !self.pma_allows(addr, size as u64, Access::Write)
        {
            return Err(fault);
        }
        bus.write(addr, size, value).map_err(|_| fault)
    }

    /// An atomic memory operation's read half: an AMO is checked as a store, so a refusal is a store fault.
    fn amo_load(&mut self, bus: &mut impl Bus, addr: u64, size: u8) -> Result<u64, Trap> {
        let fault = Trap {
            cause: CAUSE_STORE_ACCESS,
            tval: addr,
        };
        if addr % size as u64 != 0 {
            return Err(Trap {
                cause: CAUSE_MISALIGNED_STORE,
                tval: addr,
            });
        }
        if !self.pmp_allows(addr, size as u64, Access::Atomic, self.data_privilege())
            || !self.pma_allows(addr, size as u64, Access::Atomic)
        {
            return Err(fault);
        }
        bus.read(addr, size).map_err(|_| fault)
    }

    fn fetch_half(&mut self, bus: &mut impl Bus, addr: u64) -> Result<u16, Trap> {
        let fault = Trap {
            cause: CAUSE_FETCH_ACCESS,
            tval: addr,
        };
        if !self.pmp_allows(addr, 2, Access::Execute, self.privilege) || !self.pma_allows(addr, 2, Access::Execute) {
            return Err(fault);
        }
        bus.read(addr, 2).map(|v| v as u16).map_err(|_| fault)
    }

    /// Fetch the instruction at `pc`: its 32-bit form, its length, and its raw encoding (for `mtval`).
    fn fetch(&mut self, bus: &mut impl Bus) -> Result<(u32, u64, u32), Trap> {
        let lo = self.fetch_half(bus, self.pc)?;
        if rvc::is_compressed(lo) {
            return match rvc::expand(lo) {
                Some(inst) => Ok((inst, 2, lo as u32)),
                None => Err(illegal(lo as u32)),
            };
        }
        let hi = self.fetch_half(bus, self.pc.wrapping_add(2))?;
        let inst = (lo as u32) | ((hi as u32) << 16);
        Ok((inst, 4, inst))
    }

    fn take_trap(&mut self, trap: Trap) {
        self.mepc = self.pc;
        self.mcause = trap.cause;
        self.mtval = trap.tval;
        self.mpie = self.mie_bit;
        self.mie_bit = false;
        self.mpp = self.privilege;
        self.privilege = Privilege::Machine;
        self.pc = self.mtvec & !3;
        self.reservation = None;
    }

    /// Execute one instruction.
    /// The counters count the instruction under the inhibit it leaves behind: one that writes `mcountinhibit` is
    /// counted (or not) by the value it writes.
    pub fn step(&mut self, bus: &mut impl Bus) -> Step {
        let result = self.execute(bus);
        let counting_cycles = self.mcountinhibit & 1 == 0;
        match result {
            Ok(outcome) => {
                if counting_cycles && !outcome.wrote_mcycle {
                    self.mcycle = self.mcycle.wrapping_add(1);
                }
                if self.mcountinhibit & 4 == 0 && !outcome.wrote_minstret {
                    self.minstret = self.minstret.wrapping_add(1);
                }
                self.pc = outcome.next_pc;
                self.x[0] = 0;
                Step::Retired
            }
            Err(trap) => {
                if counting_cycles {
                    self.mcycle = self.mcycle.wrapping_add(1);
                }
                self.take_trap(trap);
                Step::Trapped(trap)
            }
        }
    }

    fn execute(&mut self, bus: &mut impl Bus) -> Result<Outcome, Trap> {
        let (inst, len, raw) = self.fetch(bus)?;
        let mut out = Outcome {
            next_pc: self.pc.wrapping_add(len),
            wrote_mcycle: false,
            wrote_minstret: false,
        };
        let opcode = inst & 0x7f;
        let rd = ((inst >> 7) & 31) as usize;
        let funct3 = (inst >> 12) & 7;
        let rs1 = ((inst >> 15) & 31) as usize;
        let rs2 = ((inst >> 20) & 31) as usize;
        let funct7 = inst >> 25;
        let a = self.x[rs1];
        let b = self.x[rs2];
        let imm_i = ((inst as i32) >> 20) as i64 as u64;
        let imm_s = ((((inst as i32) >> 25) << 5) as i64 as u64) | ((inst >> 7) & 31) as u64;
        let imm_b = ((((inst as i32) >> 31) << 12) as i64 as u64)
            | (((inst >> 7) & 1) << 11) as u64
            | (((inst >> 25) & 0x3f) << 5) as u64
            | (((inst >> 8) & 0xf) << 1) as u64;
        let imm_u = (inst & 0xffff_f000) as i32 as i64 as u64;
        let imm_j = ((((inst as i32) >> 31) << 20) as i64 as u64)
            | (inst & 0x000f_f000) as u64
            | (((inst >> 20) & 1) << 11) as u64
            | (((inst >> 21) & 0x3ff) << 1) as u64;
        let bad = || illegal(raw);
        match opcode {
            0x37 => self.x[rd] = imm_u,
            0x17 => self.x[rd] = self.pc.wrapping_add(imm_u),
            0x6f => {
                self.x[rd] = self.pc.wrapping_add(len);
                out.next_pc = self.pc.wrapping_add(imm_j);
            }
            0x67 => {
                if funct3 != 0 {
                    return Err(bad());
                }
                let target = a.wrapping_add(imm_i) & !1;
                self.x[rd] = self.pc.wrapping_add(len);
                out.next_pc = target;
            }
            0x63 => {
                let taken = match funct3 {
                    0 => a == b,
                    1 => a != b,
                    4 => (a as i64) < (b as i64),
                    5 => (a as i64) >= (b as i64),
                    6 => a < b,
                    7 => a >= b,
                    _ => return Err(bad()),
                };
                if taken {
                    out.next_pc = self.pc.wrapping_add(imm_b);
                }
            }
            0x03 => {
                let addr = a.wrapping_add(imm_i);
                let v = match funct3 {
                    0 => self.load(bus, addr, 1)? as u8 as i8 as i64 as u64,
                    1 => self.load(bus, addr, 2)? as u16 as i16 as i64 as u64,
                    2 => sext32(self.load(bus, addr, 4)?),
                    3 => self.load(bus, addr, 8)?,
                    4 => self.load(bus, addr, 1)? as u8 as u64,
                    5 => self.load(bus, addr, 2)? as u16 as u64,
                    6 => self.load(bus, addr, 4)? as u32 as u64,
                    _ => return Err(bad()),
                };
                self.x[rd] = v;
            }
            0x23 => {
                let addr = a.wrapping_add(imm_s);
                match funct3 {
                    0 => self.store(bus, addr, 1, b & 0xff)?,
                    1 => self.store(bus, addr, 2, b & 0xffff)?,
                    2 => self.store(bus, addr, 4, b & 0xffff_ffff)?,
                    3 => self.store(bus, addr, 8, b)?,
                    _ => return Err(bad()),
                }
            }
            0x13 => {
                let shamt = (inst >> 20) & 0x3f;
                self.x[rd] = match funct3 {
                    0 => a.wrapping_add(imm_i),
                    2 => ((a as i64) < (imm_i as i64)) as u64,
                    3 => (a < imm_i) as u64,
                    4 => a ^ imm_i,
                    6 => a | imm_i,
                    7 => a & imm_i,
                    1 if inst >> 26 == 0 => a << shamt,
                    5 if inst >> 26 == 0 => a >> shamt,
                    5 if inst >> 26 == 0x10 => ((a as i64) >> shamt) as u64,
                    _ => return Err(bad()),
                };
            }
            0x1b => {
                let shamt = (inst >> 20) & 0x1f;
                self.x[rd] = match (funct3, funct7) {
                    (0, _) => sext32(a.wrapping_add(imm_i)),
                    (1, 0) => sext32(((a as u32) << shamt) as u64),
                    (5, 0) => sext32(((a as u32) >> shamt) as u64),
                    (5, 0x20) => ((a as i32) >> shamt) as i64 as u64,
                    _ => return Err(bad()),
                };
            }
            0x33 => {
                self.x[rd] = match (funct7, funct3) {
                    (0, 0) => a.wrapping_add(b),
                    (0x20, 0) => a.wrapping_sub(b),
                    (0, 1) => a << (b & 63),
                    (0, 2) => ((a as i64) < (b as i64)) as u64,
                    (0, 3) => (a < b) as u64,
                    (0, 4) => a ^ b,
                    (0, 5) => a >> (b & 63),
                    (0x20, 5) => ((a as i64) >> (b & 63)) as u64,
                    (0, 6) => a | b,
                    (0, 7) => a & b,
                    (1, 0) => a.wrapping_mul(b),
                    (1, 1) => (((a as i64 as i128) * (b as i64 as i128)) >> 64) as u64,
                    (1, 2) => (((a as i64 as i128) * (b as u128 as i128)) >> 64) as u64,
                    (1, 3) => (((a as u128) * (b as u128)) >> 64) as u64,
                    (1, 4) => {
                        let (x, y) = (a as i64, b as i64);
                        if y == 0 {
                            u64::MAX
                        } else {
                            x.wrapping_div(y) as u64
                        }
                    }
                    (1, 5) => a.checked_div(b).unwrap_or(u64::MAX),
                    (1, 6) => {
                        let (x, y) = (a as i64, b as i64);
                        if y == 0 {
                            a
                        } else {
                            x.wrapping_rem(y) as u64
                        }
                    }
                    (1, 7) => a.checked_rem(b).unwrap_or(a),
                    _ => return Err(bad()),
                };
            }
            0x3b => {
                let (x, y) = (a as u32, b as u32);
                self.x[rd] = match (funct7, funct3) {
                    (0, 0) => sext32(x.wrapping_add(y) as u64),
                    (0x20, 0) => sext32(x.wrapping_sub(y) as u64),
                    (0, 1) => sext32((x << (y & 31)) as u64),
                    (0, 5) => sext32((x >> (y & 31)) as u64),
                    (0x20, 5) => ((x as i32) >> (y & 31)) as i64 as u64,
                    (1, 0) => sext32(x.wrapping_mul(y) as u64),
                    (1, 4) => {
                        let (p, q) = (x as i32, y as i32);
                        if q == 0 {
                            u64::MAX
                        } else {
                            p.wrapping_div(q) as i64 as u64
                        }
                    }
                    (1, 5) => {
                        if y == 0 {
                            u64::MAX
                        } else {
                            sext32((x / y) as u64)
                        }
                    }
                    (1, 6) => {
                        let (p, q) = (x as i32, y as i32);
                        if q == 0 {
                            p as i64 as u64
                        } else {
                            p.wrapping_rem(q) as i64 as u64
                        }
                    }
                    (1, 7) => {
                        if y == 0 {
                            sext32(x as u64)
                        } else {
                            sext32((x % y) as u64)
                        }
                    }
                    _ => return Err(bad()),
                };
            }
            0x2f => self.atomic(bus, inst, rd, a, b, raw)?,
            0x0f => match funct3 {
                0 | 1 => {}
                _ => return Err(bad()),
            },
            0x73 => self.system(inst, raw, rd, rs1, funct3, &mut out)?,
            _ => return Err(bad()),
        }
        Ok(out)
    }

    fn atomic(&mut self, bus: &mut impl Bus, inst: u32, rd: usize, a: u64, b: u64, raw: u32) -> Result<(), Trap> {
        let funct3 = (inst >> 12) & 7;
        let funct5 = inst >> 27;
        let size: u8 = match funct3 {
            2 => 4,
            3 => 8,
            _ => return Err(illegal(raw)),
        };
        let widen = |v: u64| if size == 4 { sext32(v) } else { v };
        match funct5 {
            0x02 => {
                if (inst >> 20) & 31 != 0 {
                    return Err(illegal(raw));
                }
                if a % size as u64 != 0 {
                    return Err(Trap {
                        cause: CAUSE_MISALIGNED_LOAD,
                        tval: a,
                    });
                }
                if !self.pma_allows(a, size as u64, Access::Atomic) {
                    return Err(Trap {
                        cause: CAUSE_LOAD_ACCESS,
                        tval: a,
                    });
                }
                let v = self.load(bus, a, size)?;
                self.reservation = Some(a);
                self.x[rd] = widen(v);
            }
            0x03 => {
                if a % size as u64 != 0 {
                    return Err(Trap {
                        cause: CAUSE_MISALIGNED_STORE,
                        tval: a,
                    });
                }
                if self.reservation == Some(a) {
                    if !self.pma_allows(a, size as u64, Access::Atomic) {
                        return Err(Trap {
                            cause: CAUSE_STORE_ACCESS,
                            tval: a,
                        });
                    }
                    let value = if size == 4 { b & 0xffff_ffff } else { b };
                    self.store(bus, a, size, value)?;
                    self.x[rd] = 0;
                } else {
                    self.x[rd] = 1;
                }
                self.reservation = None;
            }
            _ => {
                let old = self.amo_load(bus, a, size)?;
                let (x, y) = if size == 4 { (sext32(old), sext32(b)) } else { (old, b) };
                let new = match funct5 {
                    0x01 => y,
                    0x00 => x.wrapping_add(y),
                    0x04 => x ^ y,
                    0x0c => x & y,
                    0x08 => x | y,
                    0x10 => {
                        if (x as i64) < (y as i64) {
                            x
                        } else {
                            y
                        }
                    }
                    0x14 => {
                        if (x as i64) > (y as i64) {
                            x
                        } else {
                            y
                        }
                    }
                    // Sign extension preserves unsigned order, so the widened operands compare as the words do.
                    0x18 => x.min(y),
                    0x1c => x.max(y),
                    _ => return Err(illegal(raw)),
                };
                let mask = if size == 4 { 0xffff_ffff } else { u64::MAX };
                self.store(bus, a, size, new & mask)
                    .map_err(|t| Trap {
                        cause: CAUSE_STORE_ACCESS,
                        tval: t.tval,
                    })?;
                self.x[rd] = widen(old);
            }
        }
        Ok(())
    }

    fn system(
        &mut self,
        inst: u32,
        raw: u32,
        rd: usize,
        rs1: usize,
        funct3: u32,
        out: &mut Outcome,
    ) -> Result<(), Trap> {
        if funct3 == 0 {
            return match inst {
                0x0000_0073 => Err(Trap {
                    cause: if self.privilege == Privilege::Machine {
                        CAUSE_MACHINE_ECALL
                    } else {
                        CAUSE_USER_ECALL
                    },
                    tval: 0,
                }),
                0x0010_0073 => Err(Trap {
                    cause: CAUSE_BREAKPOINT,
                    tval: self.pc,
                }),
                0x3020_0073 if self.privilege == Privilege::Machine => {
                    self.privilege = self.mpp;
                    self.mie_bit = self.mpie;
                    self.mpie = true;
                    if self.mpp != Privilege::Machine {
                        self.mprv = false;
                    }
                    self.mpp = Privilege::User;
                    self.reservation = None;
                    out.next_pc = self.mepc;
                    Ok(())
                }
                0x1050_0073 if self.privilege == Privilege::Machine || !self.tw => Ok(()),
                _ => Err(illegal(raw)),
            };
        }
        if funct3 == 4 {
            return Err(illegal(raw));
        }
        let csr = inst >> 20;
        let operand = if funct3 & 4 != 0 { rs1 as u64 } else { self.x[rs1] };
        let writes = match funct3 & 3 {
            1 => true,
            _ => rs1 != 0,
        };
        let old = self.read_csr(csr, raw)?;
        if writes {
            if (csr >> 10) & 3 == 3 {
                return Err(illegal(raw));
            }
            let new = match funct3 & 3 {
                1 => operand,
                2 => old | operand,
                _ => old & !operand,
            };
            self.write_csr(csr, new, out);
        }
        self.x[rd] = old;
        Ok(())
    }

    fn counter_permitted(&self, bit: u32) -> bool {
        self.privilege == Privilege::Machine || self.mcounteren & (1 << bit) != 0
    }

    fn read_csr(&self, csr: u32, raw: u32) -> Result<u64, Trap> {
        if (csr >> 8) & 3 > self.privilege as u32 {
            return Err(illegal(raw));
        }
        Ok(match csr {
            0x300 => self.mstatus(),
            0x301 => MISA,
            0x304 => self.mie,
            0x305 => self.mtvec,
            0x306 => self.mcounteren,
            0x30a => 0,
            0x320 => self.mcountinhibit,
            0x323..=0x33f => 0,
            0x340 => self.mscratch,
            0x341 => self.mepc,
            0x342 => self.mcause,
            0x343 => self.mtval,
            0x344 => 0,
            0x3a0 => self.pmpcfg.iter().enumerate().map(|(i, &c)| (c as u64) << (8 * i)).sum(),
            0x3a2 => 0,
            0x3b0..=0x3bf => {
                let i = (csr - 0x3b0) as usize;
                if i < PMP_ENTRIES {
                    self.pmpaddr[i]
                } else {
                    0
                }
            }
            0xb00 => self.mcycle,
            0xb02 => self.minstret,
            0xb03..=0xb1f => 0,
            0xc00 if self.counter_permitted(0) => self.mcycle,
            0xc01 if self.counter_permitted(1) => self.mcycle,
            0xc02 if self.counter_permitted(2) => self.minstret,
            0xc03..=0xc1f if self.counter_permitted(csr - 0xc00) => 0,
            0xf11..=0xf15 => 0,
            _ => return Err(illegal(raw)),
        })
    }

    fn write_csr(&mut self, csr: u32, v: u64, out: &mut Outcome) {
        match csr {
            0x300 => self.write_mstatus(v),
            0x304 => self.mie = v & ((1 << 3) | (1 << 7) | (1 << 11)),
            0x305 => self.mtvec = v & !3,
            0x306 => self.mcounteren = v & 7,
            0x320 => self.mcountinhibit = v & 5,
            0x340 => self.mscratch = v,
            0x341 => self.mepc = v & !1,
            0x342 => self.mcause = v,
            0x343 => self.mtval = v,
            0x3a0 => {
                for i in 0..PMP_ENTRIES {
                    if self.pmpcfg[i] & PMP_L != 0 {
                        continue;
                    }
                    let mut cfg = (v >> (8 * i)) as u8 & (PMP_R | PMP_W | PMP_X | PMP_A | PMP_L);
                    if cfg & PMP_W != 0 && cfg & PMP_R == 0 {
                        cfg &= !PMP_W;
                    }
                    self.pmpcfg[i] = cfg;
                }
            }
            0x3b0..=0x3bf => {
                let i = (csr - 0x3b0) as usize;
                if i < PMP_ENTRIES && !self.pmpaddr_locked(i) {
                    self.pmpaddr[i] = v & PMPADDR_MASK;
                }
            }
            0xb00 => {
                self.mcycle = v;
                out.wrote_mcycle = true;
            }
            0xb02 => {
                self.minstret = v;
                out.wrote_minstret = true;
            }
            _ => {}
        }
    }

    /// `pmpaddr[i]` is locked by its own entry's lock bit, or by the next entry's when that entry is a locked TOR
    /// range whose bottom it is.
    fn pmpaddr_locked(&self, i: usize) -> bool {
        self.pmpcfg[i] & PMP_L != 0
            || (i + 1 < PMP_ENTRIES
                && self.pmpcfg[i + 1] & PMP_L != 0
                && self.pmpcfg[i + 1] & PMP_A == PMP_TOR)
    }
}

struct Outcome {
    next_pc: u64,
    wrote_mcycle: bool,
    wrote_minstret: bool,
}
