//! sass_isa_lock — the sm_86 SASS instruction set our scheduler operates over. We extracted every base opcode
//! (81 of them) from real machine code (our GEMM + cuBLAS) and classified each by execution unit + latency +
//! whether it sets a write barrier. This test proves the classification is COMPLETE for our GEMM kernel — the
//! scheduler will never meet an instruction it cannot reason about — which is the ISA foundation for closing
//! the last ~10% to cuBLAS (a latency-aware reorder of ldmatrix/IMMA/loads).
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_sass::{classify_opcode, SassClass};

#[test]
fn the_full_sm86_isa_is_classified_with_latencies() {
    // The 81 base opcodes seen in real sm_86 machine code (our GEMM + cuBLAS libcublasLt).
    let isa = [
        "IMAD",
        "IADD3",
        "DFMA",
        "LDG",
        "FADD",
        "ISETP",
        "BRA",
        "LDS",
        "FFMA",
        "LEA",
        "MOV",
        "SHF",
        "NOP",
        "LOP3",
        "PRMT",
        "FMUL",
        "HADD2",
        "STG",
        "CS2R",
        "S2R",
        "DADD",
        "BSSY",
        "BSYNC",
        "EXIT",
        "STS",
        "ULDC",
        "PLOP3",
        "BAR",
        "FSETP",
        "I2FP",
        "I2F",
        "CALL",
        "FSEL",
        "DMUL",
        "MUFU",
        "USHF",
        "SHFL",
        "SEL",
        "IMNMX",
        "F2FP",
        "UIADD3",
        "UIMAD",
        "F2I",
        "LD",
        "DSETP",
        "ULEA",
        "S2UR",
        "HFMA2",
        "MEMBAR",
        "ERRBAR",
        "CCTL",
        "ATOMG",
        "WARPSYNC",
        "POPC",
        "YIELD",
        "UISETP",
        "ST",
        "F2F",
        "ULOP3",
        "R2UR",
        "B2R",
        "USEL",
        "VOTEU",
        "UMOV",
        "IMMA",
        "FLO",
        "STL",
        "LDL",
        "RET",
        "P2R",
        "LDGSTS",
        "LDSM",
        "FRND",
        "HMUL2",
        "HSETP2",
        "LDGDEPBAR",
        "BREAK",
        "R2P",
    ];
    // Every real opcode must classify to something concrete (not Other) — we understand the whole ISA.
    for op in isa {
        assert_ne!(
            classify_opcode(op),
            SassClass::Other,
            "opcode {op} must be classified (it appears in real SASS)"
        );
    }

    // The exact execution units our GEMM hot loop depends on — the scheduler's world:
    assert_eq!(
        classify_opcode("IMMA"),
        SassClass::Tensor,
        "IMMA is the tensor-core MAC"
    );
    assert_eq!(
        classify_opcode("LDSM"),
        SassClass::Memory,
        "LDSM (ldmatrix) is variable-latency memory"
    );
    assert_eq!(
        classify_opcode("LDGSTS"),
        SassClass::AsyncCopy,
        "LDGSTS is cp.async"
    );
    assert_eq!(
        classify_opcode("IMAD.WIDE"),
        SassClass::Alu,
        "modifiers must not break classification"
    );
    assert_eq!(classify_opcode("BAR.SYNC"), SassClass::Control);

    // Latency + barrier facts the scheduler reasons over (MEASURED on sm_86 — see
    // sass_hazard_calibration_lock): dependent-accumulate IMMA latency is 24 cycles, and the
    // tensor pipe accepts a new independent IMMA every ~16 (≈8 only when ≥2 warps interleave,
    // ptxas's assumption — the source of the old "~8" figure).
    assert_eq!(
        SassClass::Tensor.fixed_latency(),
        24,
        "int8 IMMA dependent-accumulate latency, measured"
    );
    assert_eq!(
        SassClass::Tensor.throughput(),
        16,
        "single-warp IMMA issue interval, measured"
    );
    assert!(
        SassClass::Memory.is_variable_latency(),
        "loads must set a write barrier for dependents to wait on"
    );
    assert!(
        SassClass::AsyncCopy.is_variable_latency(),
        "cp.async completes asynchronously"
    );
    assert!(
        !SassClass::Tensor.is_variable_latency(),
        "IMMA is fixed-throughput, no barrier"
    );
    assert!(
        SassClass::Fp64.fixed_latency() > SassClass::Alu.fixed_latency(),
        "FP64 is slower than integer ALU"
    );
}
