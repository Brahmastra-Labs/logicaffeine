//! stencil_proof_lock — the frame-form integer stencils, proved against their specs for
//! every input.
//!
//! `logicaffeine_synth::stencil_proof` decodes each stencil's machine code
//! (`logicaffeine_synth::x86`) and executes every path over Z3 bitvectors, symbolic in the
//! operands, the slot indices and every register. Refutation discharges the theorem: the
//! side exit is taken exactly when the spec's precondition fails and leaves the frame
//! untouched, the continuation stores the spec's result to the destination slot and nothing
//! else, no divide raises #DE, and the pinned and callee-saved registers survive.
//! `phase_exodia_forge` runs the same specs on chosen inputs; this lock covers all of them.
//!
//! The canaries keep the proofs honest: a wrong spec and stencil bytes patched into a
//! neighbouring instruction are refuted, bytes outside the decoder's subset stop the proof,
//! and the lowering check rejects a wrong stencil or swapped operands.
//!
//! Z3-backed and x86-64 ELF only: the stencils are the host's object code.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --features verification --test stencil_proof_lock`

#![cfg(all(target_arch = "x86_64", target_os = "linux", feature = "verification"))]

use logicaffeine_forge::jit::MicroOp;
use logicaffeine_forge::{HoleId, Reloc, RelocKind, Stencil, ST_ADD3, ST_AND3, ST_LT3, ST_SUB3};
use logicaffeine_synth::spec::{all_specs, deliberately_wrong_spec_for_canary, OpSpec};
use logicaffeine_synth::stencil_proof::{
    check_lowering, check_lowering_to, prove_code, prove_op, stencil_for, ProofError, StencilProof,
    PRESERVED,
};
use logicaffeine_synth::x86::{
    decode, decode_all, AluOp, Cond, Gpr, Insn, Op, Operand, ShiftOp, Width,
};

const CHECKED_ARITHMETIC: &[&str] = &["add", "sub", "mul"];
const BITWISE: &[&str] = &["and", "or", "xor"];
const SHIFTS: &[&str] = &["shl", "shr"];
const COMPARES: &[&str] = &["lt", "lteq", "eq"];
const DIVISION: &[&str] = &["div", "mod"];
const FAMILIES: [&[&str]; 5] = [CHECKED_ARITHMETIC, BITWISE, SHIFTS, COMPARES, DIVISION];

fn spec(name: &str) -> OpSpec {
    all_specs()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no spec named {name}"))
}

fn prove_family(names: &[&str]) -> Vec<StencilProof> {
    names
        .iter()
        .map(|name| prove_op(&spec(name)).unwrap_or_else(|e| panic!("{name}: {e}")))
        .collect()
}

/// The families below prove every spec `all_specs` returns, each exactly once.
#[test]
fn the_families_tile_every_spec() {
    let mut named: Vec<&str> = FAMILIES.iter().flat_map(|f| f.iter().copied()).collect();
    named.sort_unstable();
    let mut specs: Vec<&str> = all_specs().iter().map(|s| s.name).collect();
    specs.sort_unstable();
    assert_eq!(named, specs, "a spec is unproved or proved twice");
}

/// `add`, `sub` and `mul` are exact: each continues with the wrapped result when the true
/// value fits in i64 and side-exits otherwise. `mul`'s overflow flag is built as the spec
/// builds its precondition, so the 128-bit product is never bit-blasted.
#[test]
fn checked_arithmetic_is_proved_for_every_input() {
    for proof in prove_family(CHECKED_ARITHMETIC) {
        assert!(
            proof.continue_paths >= 1 && proof.exit_paths >= 1,
            "{proof:?}: a checked op needs a continuing path and a side exit"
        );
    }
}

#[test]
fn bitwise_ops_are_proved_for_every_input() {
    for proof in prove_family(BITWISE) {
        assert_eq!(
            proof.exit_paths, 0,
            "{proof:?}: a total op has no side exit"
        );
    }
}

/// The count is `b & 63` and `shr` is arithmetic; the stencils read the count's low byte.
#[test]
fn shifts_are_proved_for_every_input() {
    for proof in prove_family(SHIFTS) {
        assert_eq!(
            proof.exit_paths, 0,
            "{proof:?}: a total op has no side exit"
        );
    }
}

#[test]
fn compares_are_proved_for_every_input() {
    for proof in prove_family(COMPARES) {
        assert_eq!(
            proof.exit_paths, 0,
            "{proof:?}: a total op has no side exit"
        );
    }
}

/// `div` side-exits on a zero divisor and on `i64::MIN / -1`; `mod` only on a zero divisor,
/// with `MIN % -1 = 0` from its `-1` short-circuit. Both take a 32-bit `div` when
/// `(a | b) >> 32 = 0`, which must equal the 64-bit signed spec, and no divide may fault.
#[test]
fn div_and_mod_are_proved_for_every_input() {
    for proof in prove_family(DIVISION) {
        assert!(
            proof.exit_paths >= 1,
            "{proof:?}: a checked op needs a side exit"
        );
        assert!(
            proof.divisions >= 2,
            "{proof:?}: both the 32-bit and the 64-bit divide carry a no-fault obligation"
        );
        assert!(
            proof.narrowed_paths >= 1,
            "{proof:?}: the 32-bit fast path is proved over narrowed operands"
        );
    }
}

/// The stencils proved are the ones the JIT emits for the specs' micro-ops, with holes
/// 0/1/2 bound to the left, right and destination slots and the continuations to the next
/// piece and the deopt terminal.
#[test]
fn the_jit_lowers_each_spec_to_its_proved_stencil() {
    for spec in all_specs() {
        check_lowering(&spec).unwrap_or_else(|e| panic!("{}: {e}", spec.name));
    }
}

#[test]
fn the_lowering_check_rejects_another_stencil() {
    assert!(
        check_lowering_to(&spec("add"), &ST_SUB3).is_err(),
        "add lowers to add3, so sub3 must not match the JIT's piece"
    );
}

/// The JIT lowers `a > b` to `lt3` with the operands swapped (hole 0 = rhs); the check must
/// notice the holes are not bound as `lt3`'s proof assumes.
#[test]
fn the_lowering_check_rejects_swapped_operands() {
    let lt = spec("lt");
    let gt = OpSpec {
        name: "gt",
        kind: lt.kind,
        build: || MicroOp::Gt {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        pre: lt.pre,
        result: lt.result,
    };
    let err = check_lowering_to(&gt, &ST_LT3).expect_err("gt binds lt3's holes swapped");
    assert!(err.contains("hole 0"), "unexpected rejection: {err}");
}

/// The witness harness's canary, against the proof: `add` claiming subtraction.
#[test]
fn a_wrong_spec_is_refuted() {
    let wrong = deliberately_wrong_spec_for_canary();
    match prove_op(&wrong) {
        Err(ProofError::Refuted { .. }) => {}
        other => panic!("the add-claiming-sub spec must be refuted, got {other:?}"),
    }
}

/// One stencil instruction patched into a neighbouring one, in place.
struct Mutation {
    op: &'static str,
    what: &'static str,
    pick: fn(&Insn) -> bool,
    patch: fn(&mut [u8], usize),
    becomes: fn(&Op) -> bool,
}

/// The offset of `insn`'s opcode byte, past a REX prefix.
fn opcode_at(code: &[u8], insn: &Insn) -> usize {
    insn.offset + usize::from((0x40..=0x4f).contains(&code[insn.offset]))
}

const MUTATIONS: &[Mutation] = &[
    Mutation {
        op: "add",
        what: "add → sub",
        pick: |i| matches!(i.op, Op::Alu { op: AluOp::Add, .. }),
        patch: |c, at| c[at] += 0x28,
        becomes: |o| matches!(o, Op::Alu { op: AluOp::Sub, .. }),
    },
    Mutation {
        op: "add",
        what: "jo → jno",
        pick: |i| matches!(i.op, Op::Jcc { cond: Cond::O, .. }),
        patch: |c, at| c[at] ^= 1,
        becomes: |o| matches!(o, Op::Jcc { cond: Cond::No, .. }),
    },
    Mutation {
        op: "sub",
        what: "sub → add",
        pick: |i| matches!(i.op, Op::Alu { op: AluOp::Sub, .. }),
        patch: |c, at| c[at] -= 0x28,
        becomes: |o| matches!(o, Op::Alu { op: AluOp::Add, .. }),
    },
    Mutation {
        op: "mul",
        what: "jo → jno",
        pick: |i| matches!(i.op, Op::Jcc { cond: Cond::O, .. }),
        patch: |c, at| c[at] ^= 1,
        becomes: |o| matches!(o, Op::Jcc { cond: Cond::No, .. }),
    },
    Mutation {
        op: "and",
        what: "and → or",
        pick: |i| matches!(i.op, Op::Alu { op: AluOp::And, .. }),
        patch: |c, at| c[at] -= 0x18,
        becomes: |o| matches!(o, Op::Alu { op: AluOp::Or, .. }),
    },
    Mutation {
        op: "or",
        what: "or → xor",
        pick: |i| matches!(i.op, Op::Alu { op: AluOp::Or, .. }),
        patch: |c, at| c[at] += 0x28,
        becomes: |o| matches!(o, Op::Alu { op: AluOp::Xor, .. }),
    },
    Mutation {
        op: "xor",
        what: "xor → and",
        pick: |i| matches!(i.op, Op::Alu { op: AluOp::Xor, .. }),
        patch: |c, at| c[at] -= 0x10,
        becomes: |o| matches!(o, Op::Alu { op: AluOp::And, .. }),
    },
    Mutation {
        op: "shl",
        what: "shl → shr",
        pick: |i| {
            matches!(
                i.op,
                Op::Shift {
                    op: ShiftOp::Shl,
                    ..
                }
            )
        },
        patch: |c, at| c[at + 1] ^= 0x08,
        becomes: |o| {
            matches!(
                o,
                Op::Shift {
                    op: ShiftOp::Shr,
                    ..
                }
            )
        },
    },
    Mutation {
        op: "shr",
        what: "sar → shr",
        pick: |i| {
            matches!(
                i.op,
                Op::Shift {
                    op: ShiftOp::Sar,
                    ..
                }
            )
        },
        patch: |c, at| c[at + 1] ^= 0x10,
        becomes: |o| {
            matches!(
                o,
                Op::Shift {
                    op: ShiftOp::Shr,
                    ..
                }
            )
        },
    },
    Mutation {
        op: "lt",
        what: "setl → setle",
        pick: |i| matches!(i.op, Op::Setcc { cond: Cond::L, .. }),
        patch: |c, at| c[at + 1] = 0x9e,
        becomes: |o| matches!(o, Op::Setcc { cond: Cond::Le, .. }),
    },
    Mutation {
        op: "lteq",
        what: "setle → setl",
        pick: |i| matches!(i.op, Op::Setcc { cond: Cond::Le, .. }),
        patch: |c, at| c[at + 1] = 0x9c,
        becomes: |o| matches!(o, Op::Setcc { cond: Cond::L, .. }),
    },
    Mutation {
        op: "eq",
        what: "sete → setne",
        pick: |i| matches!(i.op, Op::Setcc { cond: Cond::E, .. }),
        patch: |c, at| c[at + 1] = 0x95,
        becomes: |o| matches!(o, Op::Setcc { cond: Cond::Ne, .. }),
    },
    Mutation {
        op: "div",
        what: "the fast path's div → idiv",
        pick: |i| {
            matches!(
                i.op,
                Op::Div {
                    signed: false,
                    width: Width::Dword,
                    ..
                }
            )
        },
        patch: |c, at| c[at + 1] ^= 0x08,
        becomes: |o| {
            matches!(
                o,
                Op::Div {
                    signed: true,
                    width: Width::Dword,
                    ..
                }
            )
        },
    },
    Mutation {
        op: "mod",
        what: "the -1 short-circuit tests -2",
        pick: |i| {
            matches!(
                i.op,
                Op::Alu {
                    op: AluOp::Cmp,
                    src: Operand::Imm(-1),
                    ..
                }
            )
        },
        patch: |c, at| c[at + 2] = 0xfe,
        becomes: |o| {
            matches!(
                o,
                Op::Alu {
                    op: AluOp::Cmp,
                    src: Operand::Imm(-2),
                    ..
                }
            )
        },
    },
    Mutation {
        op: "mod",
        what: "rdx restored from rax instead of r10",
        pick: |i| {
            matches!(
                i.op,
                Op::Mov {
                    width: Width::Qword,
                    dst: Operand::Reg(Gpr::RDX),
                    src: Operand::Reg(Gpr::R10),
                }
            )
        },
        patch: |c, at| {
            c[at - 1] &= !0x04;
            c[at + 1] &= !0x38;
        },
        becomes: |o| {
            matches!(
                o,
                Op::Mov {
                    width: Width::Qword,
                    dst: Operand::Reg(Gpr::RDX),
                    src: Operand::Reg(Gpr::RAX),
                }
            )
        },
    },
];

fn mutated(stencil: &Stencil, m: &Mutation) -> Vec<u8> {
    let mut code = stencil.code.to_vec();
    let insns = decode_all(&code).unwrap_or_else(|e| panic!("{}: {e}", stencil.name));
    let picked: Vec<Insn> = insns.into_iter().filter(|i| (m.pick)(i)).collect();
    assert_eq!(
        picked.len(),
        1,
        "{} ({}): expected exactly one instruction to patch, found {picked:?}",
        stencil.name,
        m.what
    );
    let insn = picked[0];
    let at = opcode_at(&code, &insn);
    (m.patch)(&mut code, at);
    let patched = decode(&code, insn.offset).unwrap_or_else(|e| {
        panic!(
            "{} ({}): the patch does not decode: {e}",
            stencil.name, m.what
        )
    });
    assert!(
        patched.len == insn.len && (m.becomes)(&patched.op),
        "{} ({}): the patch decodes as `{}`",
        stencil.name,
        m.what,
        patched.op
    );
    code
}

/// Each patched stencil still decodes but no longer meets its spec, so its proof must be
/// refuted with a counterexample.
#[test]
fn patched_stencils_are_refuted() {
    let mut accepted = Vec::new();
    for m in MUTATIONS {
        let spec = spec(m.op);
        let stencil = stencil_for(&spec).unwrap_or_else(|e| panic!("{e}"));
        let code = mutated(stencil, m);
        let label = format!("{} ({})", stencil.name, m.what);
        match prove_code(&spec, &label, &code, stencil.relocs) {
            Err(ProofError::Refuted { .. }) => {}
            other => accepted.push(format!("{label}: {other:?}")),
        }
    }
    assert!(
        accepted.is_empty(),
        "mutants that were not refuted:\n{}",
        accepted.join("\n")
    );
}

/// Every mutated family is one the lock proves, so no canary aims at an unproved stencil.
#[test]
fn every_family_has_a_mutant() {
    for family in FAMILIES {
        assert!(
            MUTATIONS.iter().any(|m| family.contains(&m.op)),
            "no mutant exercises {family:?}"
        );
    }
}

/// Encodings from the Intel manual, read as it reads them.
#[test]
fn the_decoder_reads_the_manual_encodings() {
    let cases: &[(&[u8], &str)] = &[
        (
            &[0x48, 0x8b, 0x05, 0, 0, 0, 0],
            "mov rax, qword ptr [rip+0x0]",
        ),
        (&[0x48, 0x8b, 0x00], "mov rax, qword ptr [rax]"),
        (&[0x48, 0x8b, 0x04, 0xc7], "mov rax, qword ptr [rdi+rax*8]"),
        (&[0x4a, 0x89, 0x04, 0xd7], "mov qword ptr [rdi+r10*8], rax"),
        (&[0x48, 0x89, 0xc8], "mov rax, rcx"),
        (&[0x89, 0xd0], "mov eax, edx"),
        (&[0x48, 0x8b, 0x47, 0x10], "mov rax, qword ptr [rdi+0x10]"),
        (&[0x48, 0x8b, 0x47, 0xf8], "mov rax, qword ptr [rdi-0x8]"),
        (&[0xb8, 0x01, 0, 0, 0], "mov eax, 1"),
        (&[0x48, 0xc7, 0xc0, 0xff, 0xff, 0xff, 0xff], "mov rax, -1"),
        (&[0x0f, 0xb6, 0x0c, 0xcf], "movzx ecx, byte ptr [rdi+rcx*8]"),
        (&[0x4a, 0x03, 0x04, 0xd7], "add rax, qword ptr [rdi+r10*8]"),
        (&[0x4a, 0x2b, 0x04, 0xd7], "sub rax, qword ptr [rdi+r10*8]"),
        (&[0x4c, 0x23, 0x14, 0xc7], "and r10, qword ptr [rdi+rax*8]"),
        (&[0x4c, 0x09, 0xda], "or rdx, r11"),
        (&[0x45, 0x31, 0xdb], "xor r11d, r11d"),
        (&[0x31, 0xd2], "xor edx, edx"),
        (&[0x4a, 0x3b, 0x04, 0xd7], "cmp rax, qword ptr [rdi+r10*8]"),
        (&[0x49, 0x83, 0xfb, 0xff], "cmp r11, -1"),
        (&[0x48, 0x81, 0xf9, 0x00, 0x01, 0, 0], "cmp rcx, 256"),
        (&[0x4d, 0x85, 0xdb], "test r11, r11"),
        (&[0x48, 0xc1, 0xea, 0x20], "shr rdx, 32"),
        (&[0x49, 0xd3, 0xe2], "shl r10, cl"),
        (&[0x49, 0xd3, 0xfa], "sar r10, cl"),
        (&[0x48, 0xd1, 0xe0], "shl rax, 1"),
        (
            &[0x4a, 0x0f, 0xaf, 0x04, 0xd7],
            "imul rax, qword ptr [rdi+r10*8]",
        ),
        (&[0x49, 0xf7, 0xda], "neg r10"),
        (&[0x49, 0xf7, 0xfb], "idiv r11"),
        (&[0x41, 0xf7, 0xf3], "div r11d"),
        (&[0x48, 0x99], "cqo"),
        (&[0x99], "cdq"),
        (&[0x41, 0x0f, 0x9c, 0xc3], "setl r11b"),
        (&[0x41, 0x0f, 0x9e, 0xc3], "setle r11b"),
        (&[0x41, 0x0f, 0x94, 0xc3], "sete r11b"),
        (&[0x40, 0x0f, 0x94, 0xc6], "sete sil"),
        (&[0x70, 0x17], "jo 0x19"),
        (&[0x74, 0xf0], "je -0xe"),
        (&[0x0f, 0x85, 0x10, 0, 0, 0], "jne 0x16"),
        (&[0xeb, 0x05], "jmp 0x7"),
        (&[0xe9, 0x00, 0x01, 0, 0], "jmp 0x105"),
        (&[0xff, 0xe0], "jmp rax"),
        (&[0x41, 0xff, 0xe3], "jmp r11"),
    ];
    for (bytes, expected) in cases {
        let insn = decode(bytes, 0).unwrap_or_else(|e| panic!("{bytes:02x?}: {e}"));
        assert_eq!(
            insn.len,
            bytes.len(),
            "{bytes:02x?} decoded as {} bytes",
            insn.len
        );
        assert_eq!(&insn.op.to_string(), expected, "{bytes:02x?}");
    }
}

/// Bytes outside the closed subset are an error, never a guess.
#[test]
fn the_decoder_rejects_bytes_outside_the_subset() {
    let cases: &[(&[u8], &str)] = &[
        (&[0x90], "nop"),
        (&[0xc3], "ret"),
        (&[0x0f, 0x0b], "ud2"),
        (&[0xe8, 0, 0, 0, 0], "call rel32"),
        (&[0xff, 0xd0], "call rax"),
        (&[0x50], "push rax"),
        (&[0x66, 0x89, 0xc8], "the operand-size prefix"),
        (&[0xf3, 0x48, 0xab], "rep stosq"),
        (&[0x40, 0x48, 0x89, 0xc8], "a second REX prefix"),
        (&[0x88, 0xe0], "mov al, ah"),
        (&[0x11, 0xc8], "adc eax, ecx"),
        (&[0x48, 0x83, 0xd0, 0x01], "adc rax, 1"),
        (&[0x05, 0x01, 0, 0, 0], "add eax, imm32 (accumulator form)"),
        (&[0xf7, 0xd0], "not eax"),
        (&[0x48, 0xf7, 0xe1], "mul rcx"),
        (&[0xf6, 0xf1], "div cl"),
        (&[0x48, 0x6b, 0xc0, 0x03], "imul rax, rax, 3"),
        (&[0xc1, 0xe0, 0x05], "shl eax, 5"),
        (&[0x48, 0xd3, 0xc0], "rol rax, cl"),
        (&[0x7a, 0x00], "jp"),
        (&[0x0f, 0x9b, 0xc0], "setnp al"),
        (&[0x48, 0x0f, 0x4c, 0xc1], "cmovl rax, rcx"),
        (&[0x48, 0x63, 0xc1], "movsxd rax, ecx"),
        (&[0x48, 0x8d, 0x04, 0xc7], "lea rax, [rdi+rax*8]"),
        (&[0xf2, 0x0f, 0x58, 0xc1], "addsd xmm0, xmm1"),
        (&[0x48, 0x8b], "a truncated mov"),
        (&[0x48], "a lone REX prefix"),
    ];
    for (bytes, what) in cases {
        assert!(
            decode(bytes, 0).is_err(),
            "{what} ({bytes:02x?}) decoded as {:?}",
            decode(bytes, 0)
        );
    }
}

/// A stencil byte patched outside the subset stops the proof at the decoder.
#[test]
fn an_undecodable_stencil_is_not_proved() {
    let mut code = ST_ADD3.code.to_vec();
    let jo = decode_all(&code)
        .expect("add3 decodes")
        .into_iter()
        .find(|i| matches!(i.op, Op::Jcc { cond: Cond::O, .. }))
        .expect("add3 branches on overflow");
    let at = opcode_at(&code, &jo);
    code[at] = 0x7a;
    match prove_code(&spec("add"), "add3 (jo → jp)", &code, ST_ADD3.relocs) {
        Err(ProofError::Decode { .. }) => {}
        other => panic!("a parity jump must stop the decoder, got {other:?}"),
    }
}

/// Every `rip`-relative operand must be a relocated GOT slot, and every relocation must
/// patch one.
#[test]
fn relocations_must_bind_exactly() {
    let add = spec("add");
    let missing: Vec<_> = ST_ADD3.relocs[1..].to_vec();
    match prove_code(&add, "add3 (a relocation dropped)", ST_ADD3.code, &missing) {
        Err(ProofError::Unmodelled { .. }) => {}
        other => panic!("an operand without its relocation must not be modelled, got {other:?}"),
    }
    let mut stray = ST_ADD3.relocs.to_vec();
    stray[0].offset += 1;
    match prove_code(&add, "add3 (a relocation moved)", ST_ADD3.code, &stray) {
        Err(ProofError::Unmodelled { .. }) => {}
        other => panic!("a relocation off its operand must not be modelled, got {other:?}"),
    }
}

/// `xor r32, r32` zeroes a register whatever it holds, a hole's address included: `and3`
/// behind a prologue that loads hole 2's GOT slot into `r11` and zeroes it is still proved.
#[test]
fn the_zeroing_idiom_clears_a_pointer_register() {
    let prologue = [0x4c, 0x8b, 0x1d, 0, 0, 0, 0, 0x45, 0x31, 0xdb];
    let shift = prologue.len() as u32;
    let mut code = prologue.to_vec();
    code.extend_from_slice(ST_AND3.code);
    let mut relocs = vec![Reloc {
        offset: 3,
        kind: RelocKind::GotRel32,
        target: HoleId::ConstI64(2),
        addend: -4,
    }];
    relocs.extend(ST_AND3.relocs.iter().map(|r| Reloc {
        offset: r.offset + shift,
        ..*r
    }));
    assert_eq!(
        decode(&code, 7).map(|i| i.op.to_string()),
        Ok("xor r11d, r11d".to_string())
    );
    prove_code(
        &spec("and"),
        "and3 behind a zeroed GOT load",
        &code,
        &relocs,
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// The registers the proof holds to their entry values include the stencil ABI's: the
/// frame and operand-stack pointers, `rsp`, and the pinned argument registers.
#[test]
fn the_preserved_registers_cover_the_stencil_abi() {
    for r in [
        Gpr::RDI,
        Gpr::RSI,
        Gpr::RSP,
        Gpr::RDX,
        Gpr::RCX,
        Gpr::R8,
        Gpr::R9,
    ] {
        assert!(
            PRESERVED.contains(&r),
            "{} must be preserved",
            r.name(Width::Qword)
        );
    }
}
