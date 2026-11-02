//! emit_matmul_core — the matmul demo's compute core, AUTO-EMITTED FROM LOGOS. `cgra_dot` computes a real
//! two-vector int8 dot product `Σ a[j]·b[j]` (the matmul inner kernel): the two vectors are streamed
//! interleaved [a0,b0,a1,b1,…] into the on-chip operand buffer, and a parity+temp carry turns the single
//! MemRead into a pairwise MAC — exactly ONE int8 multiply per pair (→ one DSP48E1), the gate is a Select.
//! Prints the LOGOS source so the demo shows the code lowering to the fabric. Usage:
//!   cargo run -q -p logicaffeine-compile --example emit_matmul_core -- [out.v]
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_loop_fabric_verilog_int8, loop_fabric_from_source, AluOp, LoopCell,
};
use std::fs;

// The dot product, in LOGOS. xs is the two vectors interleaved; odd iterations MAC the pair.
const DOT: &str = "## To dot (xs: Seq of Int) -> Int:\n\
    \x20   Let acc be 0.\n\
    \x20   Let tmp be 0.\n\
    \x20   Let odd be 0.\n\
    \x20   Repeat for x in xs:\n\
    \x20       Let prod be tmp * x.\n\
    \x20       If odd is 1:\n\
    \x20           Set acc to acc + prod.\n\
    \x20       Set tmp to x.\n\
    \x20       Set odd to 1 - odd.\n\
    \x20   Return acc.\n";

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "benchmarks/silicon/cgra_dot.v".into());
    println!("── the compute kernel, in LOGOS (this is what lowers to the fabric) ──");
    for line in DOT.lines() {
        println!("  {line}");
    }
    let lf = loop_fabric_from_source(DOT, "dot");
    let muls = lf
        .body
        .iter()
        .filter(|c| matches!(c, LoopCell::Arith { op: AluOp::Mul, .. }))
        .count();
    let mems = lf
        .body
        .iter()
        .filter(|c| matches!(c, LoopCell::MemRead { .. }))
        .count();
    let sels = lf
        .body
        .iter()
        .filter(|c| matches!(c, LoopCell::Select { .. }))
        .count();
    assert_eq!(
        muls, 1,
        "the dot must lower to exactly ONE int8 multiply (one DSP48E1 per MAC)"
    );
    assert_eq!(
        mems, 1,
        "one operand read per element (the streamed interleaved vectors)"
    );
    let rtl = emit_loop_fabric_verilog_int8("dot", &lf, 8);
    fs::write(&out, &rtl).unwrap();
    println!(
        "── lowered to the fabric: {muls} int8 multiply (1 DSP/MAC), {mems} operand read, {sels} select (parity gate) ──"
    );
    println!("emitted cgra_dot (operand-fed int8 dot product) FROM LOGOS → {out}");
}
