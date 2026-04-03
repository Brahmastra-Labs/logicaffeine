//! RISC-V ISA Formal Verification Templates
//!
//! Pre-verified instruction semantics as parameterizable SVA templates.
//! Users describe their CPU configuration, templates generate SVA properties.

use super::ProtocolProperty;
use super::SvaAssertionKind;

/// RISC-V extension identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiscvExtension {
    I, // Base integer
    M, // Multiply/Divide
    A, // Atomic
    F, // Single-precision float
    D, // Double-precision float
    C, // Compressed
}

/// RISC-V CPU configuration.
#[derive(Debug, Clone)]
pub struct RiscvConfig {
    pub xlen: u32,
    pub extensions: Vec<RiscvExtension>,
    pub reg_prefix: String,
    pub pc_name: String,
    pub mem_name: String,
}

impl RiscvConfig {
    pub fn rv32i() -> Self {
        Self {
            xlen: 32,
            extensions: vec![RiscvExtension::I],
            reg_prefix: "x".into(),
            pc_name: "pc".into(),
            mem_name: "mem".into(),
        }
    }

    pub fn rv64i() -> Self {
        Self {
            xlen: 64,
            extensions: vec![RiscvExtension::I],
            reg_prefix: "x".into(),
            pc_name: "pc".into(),
            mem_name: "mem".into(),
        }
    }
}

/// Generate ALU instruction properties.
pub fn riscv_alu_properties(config: &RiscvConfig) -> Vec<ProtocolProperty> {
    let w = config.xlen;
    let r = &config.reg_prefix;
    let mut props = vec![
        ProtocolProperty {
            name: "RISCV_ADD".into(),
            spec: format!("When opcode is R-type ADD, rd = rs1 + rs2 ({}bit).", w),
            sva_body: format!(
                "(opcode == 7'h33 && funct3 == 3'h0 && funct7 == 7'h00) |-> (rd == rs1 + rs2)"
            ),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "RISCV_SUB".into(),
            spec: format!("When opcode is R-type SUB, rd = rs1 - rs2 ({}bit).", w),
            sva_body: format!(
                "(opcode == 7'h33 && funct3 == 3'h0 && funct7 == 7'h20) |-> (rd == rs1 - rs2)"
            ),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "RISCV_AND".into(),
            spec: format!("When opcode is R-type AND, rd = rs1 & rs2 ({}bit).", w),
            sva_body: "(opcode == 7'h33 && funct3 == 3'h7 && funct7 == 7'h00) |-> (rd == (rs1 & rs2))".into(),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "RISCV_OR".into(),
            spec: format!("When opcode is R-type OR, rd = rs1 | rs2 ({}bit).", w),
            sva_body: "(opcode == 7'h33 && funct3 == 3'h6 && funct7 == 7'h00) |-> (rd == (rs1 | rs2))".into(),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "RISCV_XOR".into(),
            spec: format!("When opcode is R-type XOR, rd = rs1 ^ rs2 ({}bit).", w),
            sva_body: "(opcode == 7'h33 && funct3 == 3'h4 && funct7 == 7'h00) |-> (rd == (rs1 ^ rs2))".into(),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "RISCV_SLT".into(),
            spec: "When opcode is SLT, rd = (rs1 < rs2) signed.".into(),
            sva_body: "(opcode == 7'h33 && funct3 == 3'h2 && funct7 == 7'h00) |-> (rd == ($signed(rs1) < $signed(rs2)))".into(),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "RISCV_SLTU".into(),
            spec: "When opcode is SLTU, rd = (rs1 < rs2) unsigned.".into(),
            sva_body: "(opcode == 7'h33 && funct3 == 3'h3 && funct7 == 7'h00) |-> (rd == (rs1 < rs2))".into(),
            kind: SvaAssertionKind::Assert,
        },
    ];

    // M extension: MUL, DIV
    if config.extensions.contains(&RiscvExtension::M) {
        props.push(ProtocolProperty {
            name: "RISCV_MUL".into(),
            spec: format!("When opcode is MUL, rd = (rs1 * rs2)[{}:0].", w - 1),
            sva_body: "(opcode == 7'h33 && funct3 == 3'h0 && funct7 == 7'h01) |-> (rd == rs1 * rs2)".into(),
            kind: SvaAssertionKind::Assert,
        });
        props.push(ProtocolProperty {
            name: "RISCV_DIV".into(),
            spec: "When opcode is DIV, rd = rs1 / rs2 (signed).".into(),
            sva_body: "(opcode == 7'h33 && funct3 == 3'h4 && funct7 == 7'h01) |-> (rd == $signed(rs1) / $signed(rs2))".into(),
            kind: SvaAssertionKind::Assert,
        });
    }

    props
}

/// Generate decoder mutual exclusion properties.
pub fn riscv_decoder_properties(config: &RiscvConfig) -> Vec<ProtocolProperty> {
    vec![
        ProtocolProperty {
            name: "RISCV_Decoder_Mutual_Exclusion".into(),
            spec: "At most one instruction type is active per cycle.".into(),
            sva_body: "!(is_r_type && is_i_type) && !(is_r_type && is_s_type) && !(is_r_type && is_b_type) && !(is_i_type && is_s_type) && !(is_i_type && is_b_type) && !(is_s_type && is_b_type)".into(),
            kind: SvaAssertionKind::Assert,
        },
    ]
}

/// Generate register file properties.
pub fn riscv_register_properties(config: &RiscvConfig) -> Vec<ProtocolProperty> {
    let r = &config.reg_prefix;
    let w = config.xlen;
    let mut props = vec![
        ProtocolProperty {
            name: "RISCV_X0_Always_Zero".into(),
            spec: format!("Register {}0 always reads as zero.", r),
            sva_body: format!("{}0 == {}'d0", r, w),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "RISCV_X0_Write_Ignored".into(),
            spec: format!("Writing to {}0 has no effect; it remains zero.", r),
            sva_body: format!("(rd_addr == 5'd0) |=> ({}0 == {}'d0)", r, w),
            kind: SvaAssertionKind::Assert,
        },
    ];

    // PC alignment
    if config.extensions.contains(&RiscvExtension::C) {
        props.push(ProtocolProperty {
            name: "RISCV_PC_Alignment_C".into(),
            spec: format!("PC is always 2-byte aligned (C extension present)."),
            sva_body: format!("{}[0] == 1'b0", config.pc_name),
            kind: SvaAssertionKind::Assert,
        });
    } else {
        props.push(ProtocolProperty {
            name: "RISCV_PC_Alignment".into(),
            spec: format!("PC is always 4-byte aligned (no C extension)."),
            sva_body: format!("{}[1:0] == 2'b00", config.pc_name),
            kind: SvaAssertionKind::Assert,
        });
    }

    props
}

/// Generate branch instruction properties.
pub fn riscv_branch_properties(config: &RiscvConfig) -> Vec<ProtocolProperty> {
    vec![
        ProtocolProperty {
            name: "RISCV_BEQ".into(),
            spec: "When BEQ and rs1 == rs2, PC = PC + immediate.".into(),
            sva_body: "(opcode == 7'h63 && funct3 == 3'h0 && rs1 == rs2) |-> (pc_next == pc + imm_b)".into(),
            kind: SvaAssertionKind::Assert,
        },
    ]
}

/// Generate memory access properties.
pub fn riscv_memory_properties(config: &RiscvConfig) -> Vec<ProtocolProperty> {
    let w = config.xlen;
    vec![
        ProtocolProperty {
            name: "RISCV_LW_SW_Roundtrip".into(),
            spec: "Store word then load word at same address yields same value.".into(),
            sva_body: format!(
                "(sw_valid && sw_addr == addr) |=> (lw_valid && lw_addr == addr) |-> (lw_data == sw_data)"
            ),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "RISCV_Memory_Alignment".into(),
            spec: "LW address is 4-byte aligned.".into(),
            sva_body: "(opcode == 7'h03 && funct3 == 3'h2) |-> (addr[1:0] == 2'b00)".into(),
            kind: SvaAssertionKind::Assert,
        },
    ]
}
