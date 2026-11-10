//! x86 instruction structure for the byte model: a length decoder that follows a byte stream through prefixes,
//! opcode, ModRM, SIB, displacement and immediate, so every byte is predicted knowing which field of which
//! instruction it is. 32-bit operand and address size by default; `66`/`67` switch each to 16 bits.
//!
//! The decoder never fails: an invalid or truncated instruction only means a wrong guess at where the next one
//! starts, and the stream resynchronizes within a few instructions, as x86 code does.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/x86/tests.rs"]
mod tests;

/// The field of an instruction a byte belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Field {
    /// The first byte of an instruction (a prefix or the opcode).
    Start = 0,
    /// The byte after a prefix: another prefix or the opcode.
    AfterPrefix,
    /// The second opcode byte of the `0F` map.
    Escape,
    /// The third opcode byte of the `0F 38` / `0F 3A` maps.
    Escape3,
    ModRm,
    Sib,
    Displacement,
    Immediate,
}

/// What an instruction does: the abstraction of an instruction stream the model keys on, four bits a kind.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Kind {
    /// System instructions, invalid opcodes, and nothing yet.
    Other = 0,
    /// `MOV` in all its forms, `MOVZX`/`MOVSX`, `LEA`, `XCHG`, `CMOVcc`, `CMPXCHG`, `BSWAP`, `LxS`.
    Move,
    /// `PUSH`, `POP`, `PUSHA`/`POPA`, `PUSHF`/`POPF`, `ENTER`, `LEAVE`.
    Stack,
    /// `ADD`, `ADC`, `SUB`, `SBB`, `CMP`, `INC`, `DEC`, `NEG`, `MUL`, `IMUL`, `DIV`, `IDIV`, `XADD`.
    Arith,
    /// `AND`, `OR`, `XOR`, `TEST`, `NOT`.
    Logic,
    /// Shifts and rotates, `SHLD`/`SHRD`.
    Shift,
    /// `BT`/`BTS`/`BTR`/`BTC`, `BSF`/`BSR`, `POPCNT`, `SETcc`.
    Bit,
    Jump,
    Call,
    /// `RET`, `RETF`, `IRET`.
    Return,
    /// `Jcc`, `LOOP`, `JCXZ`.
    Branch,
    /// `INT`, `INT3`, `INTO`, `INT1`.
    Interrupt,
    /// `MOVS`, `CMPS`, `STOS`, `LODS`, `SCAS`, `INS`, `OUTS`.
    String,
    /// `NOP` and hints, flag operations, conversions, decimal adjusts, `IN`/`OUT`, `XLAT`, `HLT`.
    Misc,
    /// x87.
    Float,
    /// MMX, SSE and their three-byte maps.
    Vector,
}

impl Kind {
    /// The kind four bits name.
    pub fn from_bits(bits: u8) -> Self {
        use Kind::*;
        [Other, Move, Stack, Arith, Logic, Shift, Bit, Jump, Call, Return, Branch, Interrupt, String, Misc, Float, Vector][usize::from(bits & 15)]
    }
}

/// The kind of the instruction with `opcode` (its map above the byte) and, for a group opcode, the ModRM byte whose reg
/// field picks the operation (`None` before it is read).
pub fn kind_of(opcode: u16, modrm: Option<u8>) -> Kind {
    use Kind::*;
    let op = opcode as u8;
    let reg = modrm.map(|m| (m >> 3) & 7);
    match opcode >> 8 {
        0 => match op {
            0x00..=0x3F if op & 7 < 6 => match op >> 3 {
                1 | 4 | 6 => Logic,
                _ => Arith,
            },
            0x06 | 0x07 | 0x0E | 0x16 | 0x17 | 0x1E | 0x1F => Stack,
            0x27 | 0x2F | 0x37 | 0x3F => Misc,
            0x40..=0x4F => Arith,
            0x50..=0x61 | 0x68 | 0x6A | 0x8F | 0x9C | 0x9D | 0xC8 | 0xC9 => Stack,
            0x69 | 0x6B => Arith,
            0x6C..=0x6F | 0xA4..=0xA7 | 0xAA..=0xAF => String,
            0x70..=0x7F | 0xE0..=0xE3 => Branch,
            0x80..=0x83 => match reg {
                Some(1 | 4 | 6) => Logic,
                _ => Arith,
            },
            0x84 | 0x85 | 0xA8 | 0xA9 => Logic,
            0x86..=0x8E | 0x91..=0x97 | 0xA0..=0xA3 | 0xB0..=0xBF | 0xC4..=0xC7 => Move,
            0x90 | 0x98 | 0x99 | 0x9E | 0x9F | 0xD4..=0xD7 | 0xE4..=0xE7 | 0xEC..=0xEF | 0xF4 | 0xF5 | 0xF8..=0xFD => Misc,
            0x9A | 0xE8 => Call,
            0x9B | 0xD8..=0xDF => Float,
            0xC0 | 0xC1 | 0xD0..=0xD3 => Shift,
            0xC2 | 0xC3 | 0xCA | 0xCB | 0xCF => Return,
            0xCC..=0xCE | 0xF1 => Interrupt,
            0xE9..=0xEB => Jump,
            0xF6 | 0xF7 => match reg {
                Some(0..=2) | None => Logic,
                _ => Arith,
            },
            0xFE | 0xFF => match reg {
                Some(2 | 3) => Call,
                Some(4 | 5) => Jump,
                Some(6) => Stack,
                Some(7) => Other,
                _ => Arith,
            },
            _ => Other,
        },
        1 => match op {
            0x10..=0x17 | 0x28..=0x2F | 0x50..=0x7F | 0xC2..=0xC6 | 0xD0..=0xFF => Vector,
            0x18..=0x1F => Misc,
            0x40..=0x4F | 0xB0..=0xB2 | 0xB4..=0xB7 | 0xBE | 0xBF | 0xC7..=0xCF => Move,
            0x80..=0x8F => Branch,
            0x90..=0x9F | 0xA3 | 0xAB | 0xB3 | 0xB8 | 0xBA..=0xBD => Bit,
            0xA0 | 0xA1 | 0xA8 | 0xA9 => Stack,
            0xA4 | 0xA5 | 0xAC | 0xAD => Shift,
            0xAF | 0xC0 | 0xC1 => Arith,
            _ => Other,
        },
        _ => Vector,
    }
}

const MODRM: u8 = 1;
const IMM1: u8 = 1 << 1;
const IMM2: u8 = 2 << 1;
/// Four bytes, or two under the operand-size prefix.
const IMMZ: u8 = 3 << 1;
const IMM3: u8 = 4 << 1;
/// A far pointer: a `z`-sized offset and a 16-bit selector.
const FAR: u8 = 5 << 1;
/// A memory offset: four bytes, or two under the address-size prefix.
const MOFFS: u8 = 6 << 1;
/// Group 3 (`F6`/`F7`): `TEST` (ModRM reg 0 or 1) carries an immediate, the rest do not.
const GROUP3: u8 = 7 << 1;
const IMM_MASK: u8 = 7 << 1;

/// The one-byte opcode map: whether a ModRM byte follows and the immediate kind.
const fn one_byte(op: u8) -> u8 {
    match op {
        0x00..=0x3F => match op & 7 {
            0..=3 => MODRM,
            4 => IMM1,
            5 => IMMZ,
            _ => 0,
        },
        0x40..=0x61 => 0,
        0x62 | 0x63 => MODRM,
        0x64..=0x67 => 0,
        0x68 => IMMZ,
        0x69 => MODRM | IMMZ,
        0x6A => IMM1,
        0x6B => MODRM | IMM1,
        0x6C..=0x6F => 0,
        0x70..=0x7F => IMM1,
        0x80 | 0x82 | 0x83 => MODRM | IMM1,
        0x81 => MODRM | IMMZ,
        0x84..=0x8F => MODRM,
        0x90..=0x99 => 0,
        0x9A => FAR,
        0x9B..=0x9F => 0,
        0xA0..=0xA3 => MOFFS,
        0xA4..=0xA7 => 0,
        0xA8 => IMM1,
        0xA9 => IMMZ,
        0xAA..=0xAF => 0,
        0xB0..=0xB7 => IMM1,
        0xB8..=0xBF => IMMZ,
        0xC0 | 0xC1 => MODRM | IMM1,
        0xC2 => IMM2,
        0xC3 => 0,
        0xC4 | 0xC5 => MODRM,
        0xC6 => MODRM | IMM1,
        0xC7 => MODRM | IMMZ,
        0xC8 => IMM3,
        0xC9 => 0,
        0xCA => IMM2,
        0xCB | 0xCC => 0,
        0xCD => IMM1,
        0xCE | 0xCF => 0,
        0xD0..=0xD3 => MODRM,
        0xD4 | 0xD5 => IMM1,
        0xD6 | 0xD7 => 0,
        0xD8..=0xDF => MODRM,
        0xE0..=0xE7 => IMM1,
        0xE8 | 0xE9 => IMMZ,
        0xEA => FAR,
        0xEB => IMM1,
        0xEC..=0xF5 => 0,
        0xF6 | 0xF7 => MODRM | GROUP3,
        0xF8..=0xFD => 0,
        0xFE | 0xFF => MODRM,
    }
}

/// The `0F` two-byte opcode map.
const fn two_byte(op: u8) -> u8 {
    match op {
        0x00..=0x03 | 0x0D => MODRM,
        0x04..=0x0C | 0x0E | 0x0F => 0,
        0x10..=0x2F => MODRM,
        0x30..=0x37 => 0,
        0x38..=0x6F => MODRM,
        0x70..=0x73 => MODRM | IMM1,
        0x74..=0x76 | 0x78..=0x7F => MODRM,
        0x77 => 0,
        0x80..=0x8F => IMMZ,
        0x90..=0x9F => MODRM,
        0xA0..=0xA2 | 0xA6..=0xAA => 0,
        0xA3 | 0xA5 | 0xAB | 0xAD..=0xAF => MODRM,
        0xA4 | 0xAC => MODRM | IMM1,
        0xB0..=0xB9 | 0xBB..=0xC1 | 0xC3 | 0xC7 => MODRM,
        0xBA | 0xC2 | 0xC4..=0xC6 => MODRM | IMM1,
        0xC8..=0xCF => 0,
        0xD0..=0xFF => MODRM,
    }
}

const ONE: [u8; 256] = {
    let mut t = [0; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = one_byte(i as u8);
        i += 1;
    }
    t
};

const TWO: [u8; 256] = {
    let mut t = [0; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = two_byte(i as u8);
        i += 1;
    }
    t
};

const PREFIX_OPERAND: u8 = 1;
const PREFIX_ADDRESS: u8 = 2;
const PREFIX_REP: u8 = 4;
const PREFIX_OTHER: u8 = 8;

#[inline]
fn prefix_bit(b: u8) -> Option<u8> {
    match b {
        0x66 => Some(PREFIX_OPERAND),
        0x67 => Some(PREFIX_ADDRESS),
        0xF2 | 0xF3 => Some(PREFIX_REP),
        0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0xF0 => Some(PREFIX_OTHER),
        _ => None,
    }
}

/// The decoder: where the next byte falls, and the instruction it belongs to.
#[derive(Clone)]
pub struct X86 {
    next: Field,
    /// Position of the next byte inside its field (0 for the field's first byte).
    index: u8,
    prefixes: u8,
    /// The current instruction's opcode: map (0 one-byte, 1 `0F`, 2 `0F 38`, 3 `0F 3A`) above the byte.
    opcode: u16,
    modrm: u8,
    has_modrm: bool,
    displacement: u8,
    immediate: u8,
    /// The operand bytes of the current field so far, latest lowest.
    operand: u32,
    /// The two instructions before this one, by opcode.
    previous: [u16; 2],
    /// The last eight instructions, latest first: opcode, ModRM (0x100 for none) and prefixes, one word each.
    recent: [u32; 8],
    /// The last eight instructions' kinds, four bits each, the latest lowest.
    kinds: u32,
}

impl Default for X86 {
    fn default() -> Self {
        Self::new()
    }
}

impl X86 {
    pub fn new() -> Self {
        Self {
            next: Field::Start,
            index: 0,
            prefixes: 0,
            opcode: 0,
            modrm: 0,
            has_modrm: false,
            displacement: 0,
            immediate: 0,
            operand: 0,
            previous: [0; 2],
            recent: [0; 8],
            kinds: 0,
        }
    }

    /// The field the next byte falls in.
    #[inline]
    pub fn field(&self) -> Field {
        self.next
    }

    /// Position of the next byte inside its field.
    #[inline]
    pub fn index(&self) -> u8 {
        self.index
    }

    #[inline]
    pub fn opcode(&self) -> u16 {
        self.opcode
    }

    /// The current instruction's ModRM byte, once read (0x100 when it has none yet).
    #[inline]
    pub fn modrm(&self) -> u16 {
        if self.has_modrm { u16::from(self.modrm) } else { 0x100 }
    }

    #[inline]
    pub fn prefixes(&self) -> u8 {
        self.prefixes
    }

    #[inline]
    pub fn previous(&self) -> [u16; 2] {
        self.previous
    }

    /// The last eight instructions, latest first: opcode above ModRM (0x100 for none) above the prefix bits.
    #[inline]
    pub fn recent(&self) -> [u32; 8] {
        self.recent
    }

    /// The last eight instructions' kinds, four bits each, the latest lowest.
    #[inline]
    pub fn kinds(&self) -> u32 {
        self.kinds
    }

    /// The current instruction's kind as far as it is known: [`Kind::Other`] before its opcode, and a group
    /// opcode's provisional kind until its ModRM byte.
    #[inline]
    pub fn kind(&self) -> Kind {
        match self.next {
            Field::Start | Field::AfterPrefix | Field::Escape | Field::Escape3 => Kind::Other,
            _ => kind_of(self.opcode, self.has_modrm.then_some(self.modrm)),
        }
    }

    /// The bytes of the current displacement or immediate read so far, latest lowest.
    #[inline]
    pub fn operand(&self) -> u32 {
        self.operand
    }

    fn immediate_size(&self, kind: u8) -> u8 {
        let z = if self.prefixes & PREFIX_OPERAND != 0 { 2 } else { 4 };
        match kind {
            IMM1 => 1,
            IMM2 => 2,
            IMMZ => z,
            IMM3 => 3,
            FAR => z + 2,
            MOFFS => {
                if self.prefixes & PREFIX_ADDRESS != 0 {
                    2
                } else {
                    4
                }
            }
            _ => 0,
        }
    }

    fn finish(&mut self) {
        self.kinds = self.kinds << 4 | kind_of(self.opcode, self.has_modrm.then_some(self.modrm)) as u32;
        self.previous = [self.opcode, self.previous[0]];
        self.recent.rotate_right(1);
        self.recent[0] = u32::from(self.opcode) << 16 | u32::from(self.modrm()) << 4 | u32::from(self.prefixes);
        self.next = Field::Start;
        self.index = 0;
        self.prefixes = 0;
        self.has_modrm = false;
        self.operand = 0;
    }

    /// After the opcode and ModRM/SIB: the displacement, then the immediate, then the next instruction.
    fn after_addressing(&mut self) {
        self.index = 0;
        self.operand = 0;
        if self.displacement > 0 {
            self.next = Field::Displacement;
        } else if self.immediate > 0 {
            self.next = Field::Immediate;
        } else {
            self.finish();
        }
    }

    fn opcode_read(&mut self, map: u16, byte: u8, info: u8) {
        self.opcode = map << 8 | u16::from(byte);
        self.displacement = 0;
        let kind = info & IMM_MASK;
        self.immediate = if kind == GROUP3 { 0 } else { self.immediate_size(kind) };
        if info & MODRM != 0 {
            self.next = Field::ModRm;
            self.index = 0;
        } else {
            self.after_addressing();
        }
    }

    /// Advance over `b`, the byte in the field `field()` named.
    pub fn push(&mut self, b: u8) {
        match self.next {
            Field::Start | Field::AfterPrefix => {
                if let Some(bit) = prefix_bit(b) {
                    self.prefixes |= bit;
                    self.next = Field::AfterPrefix;
                } else if b == 0x0F {
                    self.next = Field::Escape;
                } else {
                    self.opcode_read(0, b, ONE[usize::from(b)]);
                }
            }
            Field::Escape => match b {
                0x38 => {
                    self.opcode = 0x200;
                    self.next = Field::Escape3;
                }
                0x3A => {
                    self.opcode = 0x300;
                    self.next = Field::Escape3;
                }
                _ => self.opcode_read(1, b, TWO[usize::from(b)]),
            },
            Field::Escape3 => {
                let map = self.opcode >> 8;
                self.opcode_read(map, b, if map == 3 { MODRM | IMM1 } else { MODRM });
            }
            Field::ModRm => {
                self.modrm = b;
                self.has_modrm = true;
                let (md, rm) = (b >> 6, b & 7);
                let mut sib = false;
                self.displacement = if self.prefixes & PREFIX_ADDRESS != 0 {
                    match (md, rm) {
                        (0, 6) | (2, _) => 2,
                        (1, _) => 1,
                        _ => 0,
                    }
                } else {
                    sib = md != 3 && rm == 4;
                    match md {
                        0 if rm == 5 => 4,
                        1 => 1,
                        2 => 4,
                        _ => 0,
                    }
                };
                let info = if self.opcode >> 8 == 0 { ONE[usize::from(self.opcode as u8)] } else { 0 };
                if info & IMM_MASK == GROUP3 && (b >> 3) & 7 < 2 {
                    self.immediate = if self.opcode == 0xF6 { 1 } else { self.immediate_size(IMMZ) };
                }
                if sib {
                    self.next = Field::Sib;
                    self.index = 0;
                } else {
                    self.after_addressing();
                }
            }
            Field::Sib => {
                if self.modrm >> 6 == 0 && b & 7 == 5 {
                    self.displacement = 4;
                }
                self.after_addressing();
            }
            Field::Displacement => {
                self.operand = self.operand << 8 | u32::from(b);
                self.index += 1;
                if self.index >= self.displacement {
                    self.displacement = 0;
                    self.after_addressing();
                }
            }
            Field::Immediate => {
                self.operand = self.operand << 8 | u32::from(b);
                self.index += 1;
                if self.index >= self.immediate {
                    self.finish();
                }
            }
        }
    }
}
