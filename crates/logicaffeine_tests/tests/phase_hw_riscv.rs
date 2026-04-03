//! SUPERCRUSH Sprint S2E: RISC-V ISA Formal Verification Templates

use logicaffeine_compile::codegen_sva::protocols::riscv::*;
use logicaffeine_compile::codegen_sva::sva_model::parse_sva;

#[test]
fn riscv_add_semantics() {
    let props = riscv_alu_properties(&RiscvConfig::rv32i());
    let add = props.iter().find(|p| p.name == "RISCV_ADD").unwrap();
    assert!(add.sva_body.contains("rs1 + rs2"), "ADD should have rs1 + rs2");
}

#[test]
fn riscv_sub_semantics() {
    let props = riscv_alu_properties(&RiscvConfig::rv32i());
    let sub = props.iter().find(|p| p.name == "RISCV_SUB").unwrap();
    assert!(sub.sva_body.contains("rs1 - rs2"), "SUB should have rs1 - rs2");
}

#[test]
fn riscv_and_or_xor() {
    let props = riscv_alu_properties(&RiscvConfig::rv32i());
    let names: Vec<&str> = props.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"RISCV_AND"), "Should have AND");
    assert!(names.contains(&"RISCV_OR"), "Should have OR");
    assert!(names.contains(&"RISCV_XOR"), "Should have XOR");
}

#[test]
fn riscv_slt_signed() {
    let props = riscv_alu_properties(&RiscvConfig::rv32i());
    let slt = props.iter().find(|p| p.name == "RISCV_SLT").unwrap();
    assert!(slt.sva_body.contains("$signed"), "SLT should use signed comparison");
}

#[test]
fn riscv_sltu_unsigned() {
    let props = riscv_alu_properties(&RiscvConfig::rv32i());
    let sltu = props.iter().find(|p| p.name == "RISCV_SLTU").unwrap();
    assert!(!sltu.sva_body.contains("$signed"), "SLTU should use unsigned comparison");
}

#[test]
fn riscv_decoder_mutual_exclusion() {
    let props = riscv_decoder_properties(&RiscvConfig::rv32i());
    assert!(!props.is_empty(), "Should have decoder properties");
    let mutual = &props[0];
    assert!(mutual.sva_body.contains("is_r_type"), "Should reference instruction types");
}

#[test]
fn riscv_x0_always_zero() {
    let props = riscv_register_properties(&RiscvConfig::rv32i());
    let x0 = props.iter().find(|p| p.name == "RISCV_X0_Always_Zero").unwrap();
    assert!(x0.sva_body.contains("x0 =="), "x0 should always be zero");
}

#[test]
fn riscv_x0_write_ignored() {
    let props = riscv_register_properties(&RiscvConfig::rv32i());
    let x0_write = props.iter().find(|p| p.name == "RISCV_X0_Write_Ignored").unwrap();
    assert!(x0_write.sva_body.contains("rd_addr == 5'd0"), "Should check write to x0");
}

#[test]
fn riscv_pc_alignment() {
    let props = riscv_register_properties(&RiscvConfig::rv32i());
    let pc = props.iter().find(|p| p.name == "RISCV_PC_Alignment").unwrap();
    assert!(pc.sva_body.contains("[1:0] == 2'b00"), "PC should be 4-byte aligned");
}

#[test]
fn riscv_pc_alignment_c() {
    let mut config = RiscvConfig::rv32i();
    config.extensions.push(RiscvExtension::C);
    let props = riscv_register_properties(&config);
    let pc = props.iter().find(|p| p.name == "RISCV_PC_Alignment_C").unwrap();
    assert!(pc.sva_body.contains("[0] == 1'b0"), "With C, PC should be 2-byte aligned");
}

#[test]
fn riscv_beq_semantics() {
    let props = riscv_branch_properties(&RiscvConfig::rv32i());
    assert!(!props.is_empty(), "Should have branch properties");
    let beq = &props[0];
    assert!(beq.sva_body.contains("rs1 == rs2"), "BEQ should check rs1 == rs2");
}

#[test]
fn riscv_lw_sw_roundtrip() {
    let props = riscv_memory_properties(&RiscvConfig::rv32i());
    let roundtrip = props.iter().find(|p| p.name == "RISCV_LW_SW_Roundtrip").unwrap();
    assert!(roundtrip.sva_body.contains("sw_data") || roundtrip.sva_body.contains("lw_data"),
        "Should reference load/store data");
}

#[test]
fn riscv_memory_alignment() {
    let props = riscv_memory_properties(&RiscvConfig::rv32i());
    let align = props.iter().find(|p| p.name == "RISCV_Memory_Alignment").unwrap();
    assert!(align.sva_body.contains("[1:0] == 2'b00"), "LW should be 4-byte aligned");
}

#[test]
fn riscv_sva_generated() {
    let props = riscv_alu_properties(&RiscvConfig::rv32i());
    for prop in &props {
        assert!(!prop.sva_body.is_empty(), "Property {} should have non-empty SVA body", prop.name);
    }
}

#[test]
fn riscv_rv32_config() {
    let config = RiscvConfig::rv32i();
    assert_eq!(config.xlen, 32);
    let props = riscv_alu_properties(&config);
    let add = props.iter().find(|p| p.name == "RISCV_ADD").unwrap();
    assert!(add.spec.contains("32"), "RV32 spec should mention 32-bit");
}

#[test]
fn riscv_rv64_config() {
    let config = RiscvConfig::rv64i();
    assert_eq!(config.xlen, 64);
    let props = riscv_alu_properties(&config);
    let add = props.iter().find(|p| p.name == "RISCV_ADD").unwrap();
    assert!(add.spec.contains("64"), "RV64 spec should mention 64-bit");
}

#[test]
fn riscv_m_extension() {
    let mut config = RiscvConfig::rv32i();
    config.extensions.push(RiscvExtension::M);
    let props = riscv_alu_properties(&config);
    let names: Vec<&str> = props.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"RISCV_MUL"), "M extension should add MUL");
    assert!(names.contains(&"RISCV_DIV"), "M extension should add DIV");
}

#[test]
fn riscv_no_m_without_extension() {
    let config = RiscvConfig::rv32i(); // I only, no M
    let props = riscv_alu_properties(&config);
    let names: Vec<&str> = props.iter().map(|p| p.name.as_str()).collect();
    assert!(!names.contains(&"RISCV_MUL"), "Without M, should NOT have MUL");
}

#[test]
fn riscv_parameterized_xlen() {
    let props32 = riscv_register_properties(&RiscvConfig::rv32i());
    let props64 = riscv_register_properties(&RiscvConfig::rv64i());
    let x0_32 = props32.iter().find(|p| p.name == "RISCV_X0_Always_Zero").unwrap();
    let x0_64 = props64.iter().find(|p| p.name == "RISCV_X0_Always_Zero").unwrap();
    assert!(x0_32.sva_body.contains("32"), "RV32 x0 should use 32-bit width");
    assert!(x0_64.sva_body.contains("64"), "RV64 x0 should use 64-bit width");
}

#[test]
fn riscv_all_properties_have_kind() {
    let config = RiscvConfig::rv32i();
    let all_props: Vec<_> = [
        riscv_alu_properties(&config),
        riscv_decoder_properties(&config),
        riscv_register_properties(&config),
        riscv_branch_properties(&config),
        riscv_memory_properties(&config),
    ].concat();
    for prop in &all_props {
        // All should be Assert type
        assert!(matches!(prop.kind, logicaffeine_compile::codegen_sva::SvaAssertionKind::Assert),
            "Property {} should be Assert kind", prop.name);
    }
}
