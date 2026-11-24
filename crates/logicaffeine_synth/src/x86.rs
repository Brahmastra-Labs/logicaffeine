//! A decoder for the closed x86-64 subset the frame-form integer stencils are built from.
//!
//! [`decode`] reads one instruction: an optional REX prefix, an opcode from the one-byte
//! or `0F` map, its ModRM/SIB addressing, displacement and immediate. The subset is
//! closed: a legacy prefix, an opcode or `/digit` outside the table below, a high-byte
//! register (`ah`, `ch`, `dh`, `bh`), a 32-bit shift, a parity condition or a truncated
//! encoding is a [`DecodeError`], never a guess. Machine code the symbolic executor in
//! [`crate::stencil_proof`] does not model therefore stops a proof instead of being misread.
//!
//! | Family | Encodings |
//! |---|---|
//! | `mov` | `88` `89` `8A` `8B` /r, `B8`+r imm32/imm64, `C6` `C7` /0 imm |
//! | `movzx` | `0F B6` /r (byte source) |
//! | `add` `or` `and` `sub` `xor` `cmp` | `00`–`3B` /r (not `adc`/`sbb`), `80` `81` `83` /digit imm |
//! | `test` | `84` `85` /r, `F6` `F7` /0 imm |
//! | `shl` `shr` `sar` | `C1` `D1` `D3` /4 /5 /7, 64-bit operands only |
//! | `imul` | `0F AF` /r (two-operand) |
//! | `neg` | `F6` `F7` /3 |
//! | `div` `idiv` | `F7` /6 /7, 32- and 64-bit operands |
//! | `cdq` `cqo` | `99` |
//! | `setcc` | `0F 90`–`9F` |
//! | `jcc` | `70`–`7F` rel8, `0F 80`–`8F` rel32 |
//! | `jmp` | `EB` rel8, `E9` rel32, `FF` /4 |
//!
//! `setcc` and `jcc` accept every condition except `p`/`np`: the parity flag is not modelled.
//! Operand size is 32 bits, or 64 with `REX.W`; the byte forms are 8 bits. There is no 16-bit
//! form, since the `66` prefix is outside the subset.

use std::fmt;

/// A general-purpose register by hardware number: 0 = `rax` … 7 = `rdi`, 8–15 = `r8`–`r15`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Gpr(u8);

const NAMES_64: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];
const NAMES_32: [&str; 16] = [
    "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d", "r12d",
    "r13d", "r14d", "r15d",
];
const NAMES_8: [&str; 16] = [
    "al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b", "r12b",
    "r13b", "r14b", "r15b",
];

impl Gpr {
    pub const RAX: Gpr = Gpr(0);
    pub const RCX: Gpr = Gpr(1);
    pub const RDX: Gpr = Gpr(2);
    pub const RBX: Gpr = Gpr(3);
    pub const RSP: Gpr = Gpr(4);
    pub const RBP: Gpr = Gpr(5);
    pub const RSI: Gpr = Gpr(6);
    pub const RDI: Gpr = Gpr(7);
    pub const R8: Gpr = Gpr(8);
    pub const R9: Gpr = Gpr(9);
    pub const R10: Gpr = Gpr(10);
    pub const R11: Gpr = Gpr(11);
    pub const R12: Gpr = Gpr(12);
    pub const R13: Gpr = Gpr(13);
    pub const R14: Gpr = Gpr(14);
    pub const R15: Gpr = Gpr(15);

    /// Every register, in hardware-number order.
    pub const ALL: [Gpr; 16] = [
        Gpr(0),
        Gpr(1),
        Gpr(2),
        Gpr(3),
        Gpr(4),
        Gpr(5),
        Gpr(6),
        Gpr(7),
        Gpr(8),
        Gpr(9),
        Gpr(10),
        Gpr(11),
        Gpr(12),
        Gpr(13),
        Gpr(14),
        Gpr(15),
    ];

    /// The register with hardware number `n`, if `n < 16`.
    pub fn from_number(n: u8) -> Option<Gpr> {
        (n < 16).then_some(Gpr(n))
    }

    /// The hardware number, 0–15.
    pub fn number(self) -> u8 {
        self.0
    }

    /// The register's name at `width` (`rax`, `eax`, `al`).
    pub fn name(self, width: Width) -> &'static str {
        let names = match width {
            Width::Byte => &NAMES_8,
            Width::Dword => &NAMES_32,
            Width::Qword => &NAMES_64,
        };
        names[self.0 as usize]
    }
}

/// Operand size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    Byte,
    Dword,
    Qword,
}

impl Width {
    pub fn bits(self) -> u32 {
        match self {
            Width::Byte => 8,
            Width::Dword => 32,
            Width::Qword => 64,
        }
    }

    fn ptr(self) -> &'static str {
        match self {
            Width::Byte => "byte",
            Width::Dword => "dword",
            Width::Qword => "qword",
        }
    }
}

/// The base of a memory operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Gpr(Gpr),
    /// `rip`: the address is the end of the instruction plus the displacement.
    Rip,
}

/// A memory operand `[base + index*scale + disp]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mem {
    /// `None` only for the SIB form with no base, which addresses `disp` absolutely.
    pub base: Option<Base>,
    pub index: Option<Gpr>,
    /// 1, 2, 4 or 8; 1 when there is no index.
    pub scale: u8,
    pub disp: i32,
    /// Offset of the displacement field within the decoded code, when the encoding has one.
    /// A relocation patches a `rip`-relative operand at exactly this offset.
    pub disp_at: Option<usize>,
}

/// An instruction operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    Reg(Gpr),
    Mem(Mem),
    /// An immediate, sign-extended to 64 bits; an instruction uses its low `width` bits.
    Imm(i64),
}

/// A condition code, as `jcc` and `setcc` test it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cond {
    /// OF
    O,
    /// ¬OF
    No,
    /// CF
    B,
    /// ¬CF
    Ae,
    /// ZF
    E,
    /// ¬ZF
    Ne,
    /// CF ∨ ZF
    Be,
    /// ¬CF ∧ ¬ZF
    A,
    /// SF
    S,
    /// ¬SF
    Ns,
    /// SF ≠ OF
    L,
    /// SF = OF
    Ge,
    /// ZF ∨ SF ≠ OF
    Le,
    /// ¬ZF ∧ SF = OF
    G,
}

impl Cond {
    /// The condition the low nibble of a `jcc`/`setcc` opcode selects; `None` for the
    /// parity conditions `p` (`A`) and `np` (`B`).
    pub fn from_nibble(n: u8) -> Option<Cond> {
        Some(match n & 0xf {
            0x0 => Cond::O,
            0x1 => Cond::No,
            0x2 => Cond::B,
            0x3 => Cond::Ae,
            0x4 => Cond::E,
            0x5 => Cond::Ne,
            0x6 => Cond::Be,
            0x7 => Cond::A,
            0x8 => Cond::S,
            0x9 => Cond::Ns,
            0xc => Cond::L,
            0xd => Cond::Ge,
            0xe => Cond::Le,
            0xf => Cond::G,
            _ => return None,
        })
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Cond::O => "o",
            Cond::No => "no",
            Cond::B => "b",
            Cond::Ae => "ae",
            Cond::E => "e",
            Cond::Ne => "ne",
            Cond::Be => "be",
            Cond::A => "a",
            Cond::S => "s",
            Cond::Ns => "ns",
            Cond::L => "l",
            Cond::Ge => "ge",
            Cond::Le => "le",
            Cond::G => "g",
        }
    }
}

/// The two-operand ALU operations of the subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AluOp {
    Add,
    Or,
    And,
    Sub,
    Xor,
    /// `sub` that writes only the flags.
    Cmp,
}

impl AluOp {
    /// The operation an ALU opcode row or `/digit` selects: 0 add, 1 or, 4 and, 5 sub,
    /// 6 xor, 7 cmp. Rows 2 (`adc`) and 3 (`sbb`) are outside the subset.
    pub fn from_digit(d: u8) -> Option<AluOp> {
        Some(match d {
            0 => AluOp::Add,
            1 => AluOp::Or,
            4 => AluOp::And,
            5 => AluOp::Sub,
            6 => AluOp::Xor,
            7 => AluOp::Cmp,
            _ => return None,
        })
    }

    fn mnemonic(self) -> &'static str {
        match self {
            AluOp::Add => "add",
            AluOp::Or => "or",
            AluOp::And => "and",
            AluOp::Sub => "sub",
            AluOp::Xor => "xor",
            AluOp::Cmp => "cmp",
        }
    }
}

/// The shifts of the subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShiftOp {
    Shl,
    /// Logical shift right.
    Shr,
    /// Arithmetic shift right.
    Sar,
}

impl ShiftOp {
    /// The shift a `/digit` selects: /4 shl, /5 shr, /7 sar. The rotates (/0–/3) and the
    /// `sal` alias (/6) are outside the subset.
    pub fn from_digit(d: u8) -> Option<ShiftOp> {
        Some(match d {
            4 => ShiftOp::Shl,
            5 => ShiftOp::Shr,
            7 => ShiftOp::Sar,
            _ => return None,
        })
    }

    fn mnemonic(self) -> &'static str {
        match self {
            ShiftOp::Shl => "shl",
            ShiftOp::Shr => "shr",
            ShiftOp::Sar => "sar",
        }
    }
}

/// A shift count: `cl`, or an immediate (`D1` encodes the immediate 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftCount {
    Cl,
    Imm(u8),
}

/// A decoded instruction's operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Mov {
        width: Width,
        dst: Operand,
        src: Operand,
    },
    /// Zero-extend a byte into a 32- or 64-bit register.
    Movzx {
        width: Width,
        dst: Gpr,
        src: Operand,
    },
    Alu {
        op: AluOp,
        width: Width,
        dst: Operand,
        src: Operand,
    },
    Test {
        width: Width,
        lhs: Operand,
        rhs: Operand,
    },
    Shift {
        op: ShiftOp,
        width: Width,
        dst: Operand,
        count: ShiftCount,
    },
    /// Two-operand signed multiply: `dst = low(dst * src)`.
    Imul {
        width: Width,
        dst: Gpr,
        src: Operand,
    },
    Neg {
        width: Width,
        dst: Operand,
    },
    /// `div` (unsigned) or `idiv` (signed) of `rdx:rax` (`edx:eax` at 32 bits).
    Div {
        signed: bool,
        width: Width,
        divisor: Operand,
    },
    /// `cdq` (32-bit) or `cqo` (64-bit): fill `edx`/`rdx` with the sign of `eax`/`rax`.
    SignExtend {
        width: Width,
    },
    Setcc {
        cond: Cond,
        dst: Operand,
    },
    /// Conditional jump; `target` is an offset in the decoded code.
    Jcc {
        cond: Cond,
        target: i64,
    },
    /// Direct jump; `target` is an offset in the decoded code.
    Jmp {
        target: i64,
    },
    /// Jump to the 64-bit address in a register or memory.
    JmpIndirect {
        target: Operand,
    },
}

impl Op {
    /// The instruction's memory operand; an x86 instruction has at most one.
    pub fn mem(&self) -> Option<&Mem> {
        let operands: [Option<&Operand>; 2] = match self {
            Op::Mov { dst, src, .. } | Op::Alu { dst, src, .. } => [Some(dst), Some(src)],
            Op::Test { lhs, rhs, .. } => [Some(lhs), Some(rhs)],
            Op::Movzx { src, .. } | Op::Imul { src, .. } => [Some(src), None],
            Op::Shift { dst, .. } | Op::Neg { dst, .. } | Op::Setcc { dst, .. } => {
                [Some(dst), None]
            }
            Op::Div { divisor, .. } => [Some(divisor), None],
            Op::JmpIndirect { target } => [Some(target), None],
            Op::SignExtend { .. } | Op::Jcc { .. } | Op::Jmp { .. } => [None, None],
        };
        operands.into_iter().flatten().find_map(|o| match o {
            Operand::Mem(m) => Some(m),
            _ => None,
        })
    }
}

/// One decoded instruction at `offset`, `len` bytes long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    pub offset: usize,
    pub len: usize,
    pub op: Op,
}

impl Insn {
    /// The offset just past the instruction, which `rip`-relative operands count from.
    pub fn end(&self) -> usize {
        self.offset + self.len
    }
}

/// Bytes the decoder does not accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    /// Offset of the instruction's first byte.
    pub offset: usize,
    pub reason: String,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "+{:#x}: {}", self.offset, self.reason)
    }
}

impl std::error::Error for DecodeError {}

#[derive(Clone, Copy)]
struct Rex {
    present: bool,
    w: bool,
    r: bool,
    x: bool,
    b: bool,
}

impl Rex {
    const NONE: Rex = Rex {
        present: false,
        w: false,
        r: false,
        x: false,
        b: false,
    };

    fn from_byte(byte: u8) -> Rex {
        Rex {
            present: true,
            w: byte & 8 != 0,
            r: byte & 4 != 0,
            x: byte & 2 != 0,
            b: byte & 1 != 0,
        }
    }

    fn width(self) -> Width {
        if self.w {
            Width::Qword
        } else {
            Width::Dword
        }
    }
}

#[derive(Clone, Copy)]
struct ModRm {
    md: u8,
    reg: u8,
    rm: u8,
}

struct Cursor<'a> {
    code: &'a [u8],
    start: usize,
    pos: usize,
}

impl Cursor<'_> {
    fn err(&self, reason: impl Into<String>) -> DecodeError {
        DecodeError {
            offset: self.start,
            reason: reason.into(),
        }
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        let b = *self
            .code
            .get(self.pos)
            .ok_or_else(|| self.err("truncated instruction"))?;
        self.pos += 1;
        Ok(b)
    }

    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let end = self.pos + N;
        let slice = self
            .code
            .get(self.pos..end)
            .ok_or_else(|| self.err("truncated instruction"))?;
        self.pos = end;
        Ok(slice.try_into().expect("slice of length N"))
    }

    fn i8(&mut self) -> Result<i8, DecodeError> {
        Ok(self.byte()? as i8)
    }

    fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.bytes::<4>()?))
    }

    fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.bytes::<8>()?))
    }

    fn modrm(&mut self) -> Result<ModRm, DecodeError> {
        let b = self.byte()?;
        Ok(ModRm {
            md: b >> 6,
            reg: (b >> 3) & 7,
            rm: b & 7,
        })
    }

    /// The register `ext:low` names at `width`. Without a REX prefix, byte registers 4–7
    /// are `ah`, `ch`, `dh`, `bh`, which the subset excludes.
    fn gpr(&self, ext: bool, low: u8, width: Width, rex: Rex) -> Result<Gpr, DecodeError> {
        let n = (u8::from(ext) << 3) | low;
        if width == Width::Byte && !rex.present && (4..8).contains(&n) {
            return Err(self.err("a high-byte register (ah, ch, dh, bh) is outside the subset"));
        }
        Ok(Gpr(n))
    }

    fn disp8(&mut self) -> Result<(i32, Option<usize>), DecodeError> {
        let at = self.pos;
        Ok((i32::from(self.i8()?), Some(at)))
    }

    fn disp32(&mut self) -> Result<(i32, Option<usize>), DecodeError> {
        let at = self.pos;
        Ok((self.i32()?, Some(at)))
    }

    fn mem(&mut self, m: ModRm, rex: Rex) -> Result<Mem, DecodeError> {
        if m.rm == 4 {
            let sib = self.byte()?;
            let idx = (u8::from(rex.x) << 3) | ((sib >> 3) & 7);
            let index = (idx != 4).then_some(Gpr(idx));
            let scale = if index.is_some() {
                1u8 << (sib >> 6)
            } else {
                1
            };
            let base_low = sib & 7;
            let base = if base_low == 5 && m.md == 0 {
                None
            } else {
                Some(Base::Gpr(Gpr((u8::from(rex.b) << 3) | base_low)))
            };
            let (disp, disp_at) = match m.md {
                0 if base.is_none() => self.disp32()?,
                0 => (0, None),
                1 => self.disp8()?,
                _ => self.disp32()?,
            };
            return Ok(Mem {
                base,
                index,
                scale,
                disp,
                disp_at,
            });
        }
        if m.md == 0 && m.rm == 5 {
            let (disp, disp_at) = self.disp32()?;
            return Ok(Mem {
                base: Some(Base::Rip),
                index: None,
                scale: 1,
                disp,
                disp_at,
            });
        }
        let base = Some(Base::Gpr(Gpr((u8::from(rex.b) << 3) | m.rm)));
        let (disp, disp_at) = match m.md {
            0 => (0, None),
            1 => self.disp8()?,
            _ => self.disp32()?,
        };
        Ok(Mem {
            base,
            index: None,
            scale: 1,
            disp,
            disp_at,
        })
    }

    fn rm(&mut self, m: ModRm, rex: Rex, width: Width) -> Result<Operand, DecodeError> {
        if m.md == 3 {
            Ok(Operand::Reg(self.gpr(rex.b, m.rm, width, rex)?))
        } else {
            Ok(Operand::Mem(self.mem(m, rex)?))
        }
    }

    /// A `/r` form: the register named by ModRM.reg and the ModRM.rm operand.
    fn reg_rm(&mut self, rex: Rex, width: Width) -> Result<(Gpr, Operand), DecodeError> {
        let m = self.modrm()?;
        let reg = self.gpr(rex.r, m.reg, width, rex)?;
        let rm = self.rm(m, rex, width)?;
        Ok((reg, rm))
    }

    fn imm(&mut self, width: Width) -> Result<i64, DecodeError> {
        Ok(match width {
            Width::Byte => i64::from(self.i8()?),
            Width::Dword | Width::Qword => i64::from(self.i32()?),
        })
    }

    fn rel8_target(&mut self) -> Result<i64, DecodeError> {
        let rel = i64::from(self.i8()?);
        Ok(self.pos as i64 + rel)
    }

    fn rel32_target(&mut self) -> Result<i64, DecodeError> {
        let rel = i64::from(self.i32()?);
        Ok(self.pos as i64 + rel)
    }

    fn cond(&self, nibble: u8) -> Result<Cond, DecodeError> {
        Cond::from_nibble(nibble)
            .ok_or_else(|| self.err("a parity condition (p/np) is outside the subset"))
    }

    fn one_byte(&mut self, rex: Rex, opcode: u8) -> Result<Op, DecodeError> {
        Ok(match opcode {
            o if o < 0x40 && (o & 7) < 4 => {
                let op = AluOp::from_digit(o >> 3).ok_or_else(|| {
                    self.err(format!("opcode {o:#04x} (adc/sbb) is outside the subset"))
                })?;
                let width = if o & 1 == 0 { Width::Byte } else { rex.width() };
                let (reg, rm) = self.reg_rm(rex, width)?;
                let (dst, src) = if o & 2 == 0 {
                    (rm, Operand::Reg(reg))
                } else {
                    (Operand::Reg(reg), rm)
                };
                Op::Alu {
                    op,
                    width,
                    dst,
                    src,
                }
            }
            0x70..=0x7f => {
                let cond = self.cond(opcode)?;
                Op::Jcc {
                    cond,
                    target: self.rel8_target()?,
                }
            }
            0x80 | 0x81 | 0x83 => {
                let width = if opcode == 0x80 {
                    Width::Byte
                } else {
                    rex.width()
                };
                let m = self.modrm()?;
                let op = AluOp::from_digit(m.reg).ok_or_else(|| {
                    self.err(format!(
                        "{opcode:#04x} /{} (adc/sbb) is outside the subset",
                        m.reg
                    ))
                })?;
                let dst = self.rm(m, rex, width)?;
                let imm = if opcode == 0x81 {
                    self.imm(width)?
                } else {
                    i64::from(self.i8()?)
                };
                Op::Alu {
                    op,
                    width,
                    dst,
                    src: Operand::Imm(imm),
                }
            }
            0x84 | 0x85 => {
                let width = if opcode == 0x84 {
                    Width::Byte
                } else {
                    rex.width()
                };
                let (reg, rm) = self.reg_rm(rex, width)?;
                Op::Test {
                    width,
                    lhs: rm,
                    rhs: Operand::Reg(reg),
                }
            }
            0x88..=0x8b => {
                let width = if opcode & 1 == 0 {
                    Width::Byte
                } else {
                    rex.width()
                };
                let (reg, rm) = self.reg_rm(rex, width)?;
                let (dst, src) = if opcode & 2 == 0 {
                    (rm, Operand::Reg(reg))
                } else {
                    (Operand::Reg(reg), rm)
                };
                Op::Mov { width, dst, src }
            }
            0x99 => Op::SignExtend { width: rex.width() },
            0xb8..=0xbf => {
                let width = rex.width();
                let dst = self.gpr(rex.b, opcode & 7, width, rex)?;
                let imm = if rex.w {
                    self.i64()?
                } else {
                    i64::from(self.i32()?)
                };
                Op::Mov {
                    width,
                    dst: Operand::Reg(dst),
                    src: Operand::Imm(imm),
                }
            }
            0xc1 | 0xd1 | 0xd3 => {
                let m = self.modrm()?;
                let op = ShiftOp::from_digit(m.reg).ok_or_else(|| {
                    self.err(format!("{opcode:#04x} /{} is outside the subset", m.reg))
                })?;
                if !rex.w {
                    return Err(self.err("a 32-bit shift is outside the subset"));
                }
                let dst = self.rm(m, rex, Width::Qword)?;
                let count = match opcode {
                    0xc1 => ShiftCount::Imm(self.byte()?),
                    0xd1 => ShiftCount::Imm(1),
                    _ => ShiftCount::Cl,
                };
                Op::Shift {
                    op,
                    width: Width::Qword,
                    dst,
                    count,
                }
            }
            0xc6 | 0xc7 => {
                let width = if opcode == 0xc6 {
                    Width::Byte
                } else {
                    rex.width()
                };
                let m = self.modrm()?;
                if m.reg != 0 {
                    return Err(self.err(format!("{opcode:#04x} /{} is outside the subset", m.reg)));
                }
                let dst = self.rm(m, rex, width)?;
                Op::Mov {
                    width,
                    dst,
                    src: Operand::Imm(self.imm(width)?),
                }
            }
            0xe9 => Op::Jmp {
                target: self.rel32_target()?,
            },
            0xeb => Op::Jmp {
                target: self.rel8_target()?,
            },
            0xf6 | 0xf7 => {
                let width = if opcode == 0xf6 {
                    Width::Byte
                } else {
                    rex.width()
                };
                let m = self.modrm()?;
                match (opcode, m.reg) {
                    (_, 0) => {
                        let lhs = self.rm(m, rex, width)?;
                        Op::Test {
                            width,
                            lhs,
                            rhs: Operand::Imm(self.imm(width)?),
                        }
                    }
                    (_, 3) => Op::Neg {
                        width,
                        dst: self.rm(m, rex, width)?,
                    },
                    (0xf7, 6 | 7) => Op::Div {
                        signed: m.reg == 7,
                        width,
                        divisor: self.rm(m, rex, width)?,
                    },
                    _ => {
                        return Err(
                            self.err(format!("{opcode:#04x} /{} is outside the subset", m.reg))
                        )
                    }
                }
            }
            0xff => {
                let m = self.modrm()?;
                if m.reg != 4 {
                    return Err(self.err(format!("0xff /{} is outside the subset", m.reg)));
                }
                Op::JmpIndirect {
                    target: self.rm(m, rex, Width::Qword)?,
                }
            }
            o => return Err(self.err(format!("opcode {o:#04x} is outside the subset"))),
        })
    }

    fn two_byte(&mut self, rex: Rex) -> Result<Op, DecodeError> {
        let opcode = self.byte()?;
        Ok(match opcode {
            0x80..=0x8f => {
                let cond = self.cond(opcode)?;
                Op::Jcc {
                    cond,
                    target: self.rel32_target()?,
                }
            }
            0x90..=0x9f => {
                let cond = self.cond(opcode)?;
                let m = self.modrm()?;
                Op::Setcc {
                    cond,
                    dst: self.rm(m, rex, Width::Byte)?,
                }
            }
            0xaf => {
                let width = rex.width();
                let (dst, src) = self.reg_rm(rex, width)?;
                Op::Imul { width, dst, src }
            }
            0xb6 => {
                let width = rex.width();
                let m = self.modrm()?;
                let dst = self.gpr(rex.r, m.reg, width, rex)?;
                let src = self.rm(m, rex, Width::Byte)?;
                Op::Movzx { width, dst, src }
            }
            o => return Err(self.err(format!("opcode 0x0f {o:#04x} is outside the subset"))),
        })
    }
}

/// Decode the instruction at `at` in `code`.
pub fn decode(code: &[u8], at: usize) -> Result<Insn, DecodeError> {
    let mut c = Cursor {
        code,
        start: at,
        pos: at,
    };
    let mut opcode = c.byte()?;
    let rex = if (0x40..=0x4f).contains(&opcode) {
        let rex = Rex::from_byte(opcode);
        opcode = c.byte()?;
        rex
    } else {
        Rex::NONE
    };
    let op = if opcode == 0x0f {
        c.two_byte(rex)?
    } else {
        c.one_byte(rex, opcode)?
    };
    Ok(Insn {
        offset: at,
        len: c.pos - at,
        op,
    })
}

/// Decode `code` from its first byte to its last, one instruction after another. Every
/// byte must belong to an instruction of the subset.
pub fn decode_all(code: &[u8]) -> Result<Vec<Insn>, DecodeError> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < code.len() {
        let insn = decode(code, at)?;
        at = insn.end();
        out.push(insn);
    }
    Ok(out)
}

struct Target(i64);

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 0 {
            write!(f, "-{:#x}", self.0.unsigned_abs())
        } else {
            write!(f, "{:#x}", self.0)
        }
    }
}

struct Shown<'a>(&'a Operand, Width);

impl fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Operand::Reg(r) => f.write_str(r.name(self.1)),
            Operand::Imm(v) => write!(f, "{v}"),
            Operand::Mem(m) => {
                write!(f, "{} ptr [", self.1.ptr())?;
                let mut first = true;
                match m.base {
                    Some(Base::Gpr(r)) => {
                        f.write_str(r.name(Width::Qword))?;
                        first = false;
                    }
                    Some(Base::Rip) => {
                        f.write_str("rip")?;
                        first = false;
                    }
                    None => {}
                }
                if let Some(ix) = m.index {
                    if !first {
                        f.write_str("+")?;
                    }
                    write!(f, "{}*{}", ix.name(Width::Qword), m.scale)?;
                    first = false;
                }
                let show_disp = m.disp != 0 || first || m.base == Some(Base::Rip);
                if show_disp {
                    let mag = i64::from(m.disp).unsigned_abs();
                    if m.disp < 0 {
                        write!(f, "-{mag:#x}")?;
                    } else if first {
                        write!(f, "{mag:#x}")?;
                    } else {
                        write!(f, "+{mag:#x}")?;
                    }
                }
                f.write_str("]")
            }
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Op::Mov { width, dst, src } => {
                write!(f, "mov {}, {}", Shown(&dst, width), Shown(&src, width))
            }
            Op::Movzx { width, dst, src } => {
                write!(f, "movzx {}, {}", dst.name(width), Shown(&src, Width::Byte))
            }
            Op::Alu {
                op,
                width,
                dst,
                src,
            } => write!(
                f,
                "{} {}, {}",
                op.mnemonic(),
                Shown(&dst, width),
                Shown(&src, width)
            ),
            Op::Test { width, lhs, rhs } => {
                write!(f, "test {}, {}", Shown(&lhs, width), Shown(&rhs, width))
            }
            Op::Shift {
                op,
                width,
                dst,
                count,
            } => {
                write!(f, "{} {}, ", op.mnemonic(), Shown(&dst, width))?;
                match count {
                    ShiftCount::Cl => f.write_str("cl"),
                    ShiftCount::Imm(n) => write!(f, "{n}"),
                }
            }
            Op::Imul { width, dst, src } => {
                write!(f, "imul {}, {}", dst.name(width), Shown(&src, width))
            }
            Op::Neg { width, dst } => write!(f, "neg {}", Shown(&dst, width)),
            Op::Div {
                signed,
                width,
                divisor,
            } => write!(
                f,
                "{} {}",
                if signed { "idiv" } else { "div" },
                Shown(&divisor, width)
            ),
            Op::SignExtend { width } => {
                f.write_str(if width == Width::Qword { "cqo" } else { "cdq" })
            }
            Op::Setcc { cond, dst } => {
                write!(f, "set{} {}", cond.suffix(), Shown(&dst, Width::Byte))
            }
            Op::Jcc { cond, target } => write!(f, "j{} {}", cond.suffix(), Target(target)),
            Op::Jmp { target } => write!(f, "jmp {}", Target(target)),
            Op::JmpIndirect { target } => write!(f, "jmp {}", Shown(&target, Width::Qword)),
        }
    }
}

impl fmt::Display for Insn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "+{:#x}: {}", self.offset, self.op)
    }
}
