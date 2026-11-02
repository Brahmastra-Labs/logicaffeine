//! ════════════════════════════════════════════════════════════════════════════════════════════
//! ISA-AOT TRIPLE LOCK (Tier-5 Silicon) — the register-VM opcode set becomes the native chip ISA,
//! and the [`emit_isa`] emitter must be TOTAL over it and carry a
//! reversibility contract on EVERY instruction. This is the reversible-hardware twin of
//! `reverse_op_lock` (static reversibility) and `wasm_aot_lock` (op-support totality): the Silicon
//! backend can only be adiabatic if every instruction it lowers knows how to run backward.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS FILE TO MAKE A RED CASE PASS.  ⚠️
//!  A RED means an op reached the Silicon emitter without a reversibility class, or the emitter lost
//!  a lowering. The fix is a real strategy in `vm/reverse.rs` + a mnemonic in `vm/isa.rs`, NEVER a
//!  wildcard, a lowered floor, or an unclassified sentinel.

use logicaffeine_compile::compile::compile_to_isa;
use logicaffeine_compile::vm::isa::{emit_isa, rev_tag};
use logicaffeine_compile::vm::reverse::{reversibility, RevClass};
use logicaffeine_compile::vm::{CompiledProgram, Constant, Op};

/// Monotone floor of the ADIABATIC zero-garbage (`Bijection`) ISA — the charge-recovery core. RAISE
/// as ops are promoted; NEVER lower to green a red. Mirrors `reverse_op_lock::BIJECTION_FLOOR`.
const ADIABATIC_FLOOR: usize = 5;

#[test]
fn every_op_lowers_to_a_classified_isa_instruction() {
    // Totality: the emitter has a lowering for every opcode in the ISA, and none reaches silicon as
    // the unclassified `!!!!` sentinel. Because `Op::all_variants()` enumerates the whole ISA and
    // `reversibility` is exhaustive, a newly-added op fails this the instant it lacks a class.
    for op in Op::all_variants() {
        let tag = rev_tag(&op);
        assert!(
            !tag.starts_with("!!!!"),
            "op `{}` reached the Tier-5 Silicon emitter unclassified: {tag}",
            op.name()
        );
    }
}

#[test]
fn no_isa_instruction_is_nonreversible() {
    // The census: `NonReversible` MUST stay ∅ — the Silicon datapath cannot contain an instruction
    // with no backward story (it would leak the erased bits the adiabatic tier is built to recover).
    let offenders: Vec<&str> = Op::all_variants()
        .iter()
        .filter(|op| matches!(reversibility(op), RevClass::NonReversible(_)))
        .map(|op| op.name())
        .collect();
    assert!(
        offenders.is_empty(),
        "non-reversible ISA instructions: {offenders:?}"
    );
}

#[test]
fn adiabatic_bijection_floor_holds_in_the_isa() {
    let bij = Op::all_variants()
        .iter()
        .filter(|op| matches!(reversibility(op), RevClass::Bijection(_)))
        .count();
    assert!(
        bij >= ADIABATIC_FLOOR,
        "adiabatic zero-garbage ISA coverage regressed: {bij} < floor {ADIABATIC_FLOOR}"
    );
}

#[test]
fn emit_isa_produces_an_annotated_listing() {
    // A round program lowers to a listing with the reversibility budget + adiabatic REVADD.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(7)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::RevAdd { place: 0, delta: 1 },
            Op::Show { src: 0 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    let asm = emit_isa(&prog);
    assert!(
        asm.contains("Silicon ISA — Tier 5"),
        "carries the ISA header"
    );
    assert!(
        asm.contains("reversibility budget:"),
        "reports the adiabatic budget"
    );
    assert!(
        asm.contains("REVADD  r0, r1"),
        "lowers the abelian core with operands"
    );
    assert!(
        asm.contains("zero-garbage"),
        "annotates the adiabatic contract"
    );
}

#[test]
fn compile_to_isa_lowers_a_source_program() {
    // End-to-end: a LOGOS source program compiles straight to Tier-5 Silicon ISA text.
    let asm =
        compile_to_isa("## Main\nLet mutable total be 5.\nSet total to total + 37.\nShow total.\n")
            .expect("source compiles to ISA");
    assert!(asm.contains("Silicon ISA — Tier 5"));
    assert!(asm.contains(".text"));
    // Every emitted instruction line carries a reversibility annotation (`;` comment).
    for line in asm.lines().filter(|l| l.trim_start().starts_with('L')) {
        assert!(
            line.contains(';'),
            "instruction line lacks a reversibility contract: {line}"
        );
    }
}
