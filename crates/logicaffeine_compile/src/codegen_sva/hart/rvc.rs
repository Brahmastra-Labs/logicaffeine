//! The RV64C decompressor: every 16-bit instruction is a shorthand for one 32-bit instruction, and the hart executes
//! the expansion. [`expand`] returns that 32-bit instruction, or `None` for an encoding the C extension reserves
//! (an illegal instruction). HINT encodings expand to the base instruction whose destination is `x0` or whose effect
//! is nil, as the specification defines them. The hart's RTL expander is held to this function over all 65,536
//! encodings, and this function is held to the GNU disassembler.

fn bit(c: u16, i: u32) -> u32 {
    ((c >> i) & 1) as u32
}

fn bits(c: u16, hi: u32, lo: u32) -> u32 {
    ((c as u32) >> lo) & ((1 << (hi - lo + 1)) - 1)
}

/// The register `x8 + n` a 3-bit compressed register field `n` names.
fn creg(n: u32) -> u32 {
    8 + n
}

fn sext(v: u32, width: u32) -> i32 {
    ((v << (32 - width)) as i32) >> (32 - width)
}

fn r_type(funct7: u32, rs2: u32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

fn i_type(imm: i32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    (((imm as u32) & 0xfff) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

fn s_type(imm: i32, rs2: u32, rs1: u32, funct3: u32, opcode: u32) -> u32 {
    let imm = imm as u32;
    (((imm >> 5) & 0x7f) << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | ((imm & 0x1f) << 7) | opcode
}

fn b_type(imm: i32, rs2: u32, rs1: u32, funct3: u32, opcode: u32) -> u32 {
    let imm = imm as u32;
    (((imm >> 12) & 1) << 31)
        | (((imm >> 5) & 0x3f) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | (((imm >> 1) & 0xf) << 8)
        | (((imm >> 11) & 1) << 7)
        | opcode
}

fn u_type(imm20: u32, rd: u32, opcode: u32) -> u32 {
    ((imm20 & 0xfffff) << 12) | (rd << 7) | opcode
}

fn j_type(imm: i32, rd: u32, opcode: u32) -> u32 {
    let imm = imm as u32;
    (((imm >> 20) & 1) << 31)
        | (((imm >> 1) & 0x3ff) << 21)
        | (((imm >> 11) & 1) << 20)
        | (((imm >> 12) & 0xff) << 12)
        | (rd << 7)
        | opcode
}

const OP_LOAD: u32 = 0x03;
const OP_LOAD_FP: u32 = 0x07;
const OP_IMM: u32 = 0x13;
const OP_IMM_32: u32 = 0x1b;
const OP_STORE: u32 = 0x23;
const OP_STORE_FP: u32 = 0x27;
const OP: u32 = 0x33;
const OP_LUI: u32 = 0x37;
const OP_32: u32 = 0x3b;
const OP_BRANCH: u32 = 0x63;
const OP_JALR: u32 = 0x67;
const OP_JAL: u32 = 0x6f;
const OP_SYSTEM: u32 = 0x73;

/// Whether the low two bits mark a 16-bit (compressed) instruction.
pub fn is_compressed(low_half: u16) -> bool {
    low_half & 3 != 3
}

/// The 32-bit instruction the compressed instruction `c` stands for, or `None` for a reserved encoding.
pub fn expand(c: u16) -> Option<u32> {
    let funct3 = bits(c, 15, 13);
    let rd = bits(c, 11, 7);
    let rs2 = bits(c, 6, 2);
    let rdp = creg(bits(c, 4, 2));
    let rs1p = creg(bits(c, 9, 7));
    let imm6 = sext((bit(c, 12) << 5) | bits(c, 6, 2), 6);
    match c & 3 {
        0 => match funct3 {
            0 => {
                // C.ADDI4SPN: addi rd', x2, nzuimm
                let nzuimm =
                    (bits(c, 12, 11) << 4) | (bits(c, 10, 7) << 6) | (bit(c, 6) << 2) | (bit(c, 5) << 3);
                (nzuimm != 0).then(|| i_type(nzuimm as i32, 2, 0, rdp, OP_IMM))
            }
            1 => {
                // C.FLD
                let uimm = (bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6);
                Some(i_type(uimm as i32, rs1p, 3, rdp, OP_LOAD_FP))
            }
            2 => {
                // C.LW
                let uimm = (bits(c, 12, 10) << 3) | (bit(c, 6) << 2) | (bit(c, 5) << 6);
                Some(i_type(uimm as i32, rs1p, 2, rdp, OP_LOAD))
            }
            3 => {
                // C.LD
                let uimm = (bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6);
                Some(i_type(uimm as i32, rs1p, 3, rdp, OP_LOAD))
            }
            4 => None,
            5 => {
                // C.FSD
                let uimm = (bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6);
                Some(s_type(uimm as i32, rdp, rs1p, 3, OP_STORE_FP))
            }
            6 => {
                // C.SW
                let uimm = (bits(c, 12, 10) << 3) | (bit(c, 6) << 2) | (bit(c, 5) << 6);
                Some(s_type(uimm as i32, rdp, rs1p, 2, OP_STORE))
            }
            _ => {
                // C.SD
                let uimm = (bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6);
                Some(s_type(uimm as i32, rdp, rs1p, 3, OP_STORE))
            }
        },
        1 => match funct3 {
            // C.ADDI (rd = 0: C.NOP and its HINTs)
            0 => Some(i_type(imm6, rd, 0, rd, OP_IMM)),
            // C.ADDIW (rd = 0 reserved)
            1 => (rd != 0).then(|| i_type(imm6, rd, 0, rd, OP_IMM_32)),
            // C.LI
            2 => Some(i_type(imm6, 0, 0, rd, OP_IMM)),
            3 if rd == 2 => {
                // C.ADDI16SP
                let nzimm = (bit(c, 12) << 9)
                    | (bit(c, 6) << 4)
                    | (bit(c, 5) << 6)
                    | (bits(c, 4, 3) << 7)
                    | (bit(c, 2) << 5);
                (nzimm != 0).then(|| i_type(sext(nzimm, 10), 2, 0, 2, OP_IMM))
            }
            3 => {
                // C.LUI
                let nzimm = (bit(c, 12) << 5) | bits(c, 6, 2);
                (nzimm != 0).then(|| u_type(sext(nzimm, 6) as u32, rd, OP_LUI))
            }
            4 => {
                let shamt = (bit(c, 12) << 5) | bits(c, 6, 2);
                match bits(c, 11, 10) {
                    // C.SRLI
                    0 => Some(i_type(shamt as i32, rs1p, 5, rs1p, OP_IMM)),
                    // C.SRAI
                    1 => Some(i_type((0x400 | shamt) as i32, rs1p, 5, rs1p, OP_IMM)),
                    // C.ANDI
                    2 => Some(i_type(imm6, rs1p, 7, rs1p, OP_IMM)),
                    _ => {
                        let rs2p = creg(bits(c, 4, 2));
                        match (bit(c, 12), bits(c, 6, 5)) {
                            (0, 0) => Some(r_type(0x20, rs2p, rs1p, 0, rs1p, OP)), // C.SUB
                            (0, 1) => Some(r_type(0, rs2p, rs1p, 4, rs1p, OP)),    // C.XOR
                            (0, 2) => Some(r_type(0, rs2p, rs1p, 6, rs1p, OP)),    // C.OR
                            (0, _) => Some(r_type(0, rs2p, rs1p, 7, rs1p, OP)),    // C.AND
                            (_, 0) => Some(r_type(0x20, rs2p, rs1p, 0, rs1p, OP_32)), // C.SUBW
                            (_, 1) => Some(r_type(0, rs2p, rs1p, 0, rs1p, OP_32)), // C.ADDW
                            _ => None,
                        }
                    }
                }
            }
            5 => {
                // C.J
                let imm = (bit(c, 12) << 11)
                    | (bit(c, 11) << 4)
                    | (bits(c, 10, 9) << 8)
                    | (bit(c, 8) << 10)
                    | (bit(c, 7) << 6)
                    | (bit(c, 6) << 7)
                    | (bits(c, 5, 3) << 1)
                    | (bit(c, 2) << 5);
                Some(j_type(sext(imm, 12), 0, OP_JAL))
            }
            _ => {
                // C.BEQZ / C.BNEZ
                let imm = (bit(c, 12) << 8)
                    | (bits(c, 11, 10) << 3)
                    | (bits(c, 6, 5) << 6)
                    | (bits(c, 4, 3) << 1)
                    | (bit(c, 2) << 5);
                Some(b_type(sext(imm, 9), 0, rs1p, if funct3 == 6 { 0 } else { 1 }, OP_BRANCH))
            }
        },
        2 => match funct3 {
            // C.SLLI
            0 => {
                let shamt = (bit(c, 12) << 5) | bits(c, 6, 2);
                Some(i_type(shamt as i32, rd, 1, rd, OP_IMM))
            }
            1 => {
                // C.FLDSP
                let uimm = (bit(c, 12) << 5) | (bits(c, 6, 5) << 3) | (bits(c, 4, 2) << 6);
                Some(i_type(uimm as i32, 2, 3, rd, OP_LOAD_FP))
            }
            2 => {
                // C.LWSP (rd = 0 reserved)
                let uimm = (bit(c, 12) << 5) | (bits(c, 6, 4) << 2) | (bits(c, 3, 2) << 6);
                (rd != 0).then(|| i_type(uimm as i32, 2, 2, rd, OP_LOAD))
            }
            3 => {
                // C.LDSP (rd = 0 reserved)
                let uimm = (bit(c, 12) << 5) | (bits(c, 6, 5) << 3) | (bits(c, 4, 2) << 6);
                (rd != 0).then(|| i_type(uimm as i32, 2, 3, rd, OP_LOAD))
            }
            4 => match (bit(c, 12), rd, rs2) {
                (0, 0, 0) => None,                                   // C.JR with rs1 = 0 is reserved
                (0, _, 0) => Some(i_type(0, rd, 0, 0, OP_JALR)),     // C.JR
                (0, _, _) => Some(r_type(0, rs2, 0, 0, rd, OP)),     // C.MV
                (_, 0, 0) => Some(i_type(1, 0, 0, 0, OP_SYSTEM)),    // C.EBREAK
                (_, _, 0) => Some(i_type(0, rd, 0, 1, OP_JALR)),     // C.JALR
                _ => Some(r_type(0, rs2, rd, 0, rd, OP)),            // C.ADD
            },
            5 => {
                // C.FSDSP
                let uimm = (bits(c, 12, 10) << 3) | (bits(c, 9, 7) << 6);
                Some(s_type(uimm as i32, rs2, 2, 3, OP_STORE_FP))
            }
            6 => {
                // C.SWSP
                let uimm = (bits(c, 12, 9) << 2) | (bits(c, 8, 7) << 6);
                Some(s_type(uimm as i32, rs2, 2, 2, OP_STORE))
            }
            _ => {
                // C.SDSP
                let uimm = (bits(c, 12, 10) << 3) | (bits(c, 9, 7) << 6);
                Some(s_type(uimm as i32, rs2, 2, 3, OP_STORE))
            }
        },
        _ => None,
    }
}
