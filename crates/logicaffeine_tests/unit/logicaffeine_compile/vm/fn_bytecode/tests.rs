use super::*;
use crate::intern::Interner;
use crate::vm::instruction::CompiledFunction;

fn jump_target(op: &Op) -> Option<usize> {
    match *op {
        Op::Jump { target } | Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
            Some(target)
        }
        _ => None,
    }
}

#[test]
fn slice_rebases_jumps_zero_relative_and_round_trips() {
    let mut it = Interner::new();
    let name = it.intern("f");
    // Main = [Halt]@0 ; function `f` @1 = a self-loop with a forward exit:
    //   @1 JumpIfFalse cond0 -> abs 3   (exit)      -> rel target 2
    //   @2 Jump        -> abs 1          (loop back) -> rel target 0
    //   @3 Halt
    let code = vec![
        Op::Halt,
        Op::JumpIfFalse { cond: 0, target: 3 },
        Op::Jump { target: 1 },
        Op::Halt,
    ];
    let prog = CompiledProgram {
        code,
        functions: vec![CompiledFunction {
            name,
            entry_pc: 1,
            param_count: 0,
            register_count: 1,
            captures: vec![],
            named_regs: vec![true],
            param_kinds: vec![],
            ret_kind: None,
            param_types: vec![],
            return_type: None,
            mutable_param_regs: vec![],
            cow_exempt_regs: vec![],
        }],
        ..Default::default()
    };

    let fnbc = slice_function(&prog, 0);
    assert_eq!(fnbc.code.len(), 3, "body is entry..next-entry");

    // Jumps are now 0-relative to the slice.
    assert!(matches!(fnbc.code[0], Op::JumpIfFalse { target: 2, .. }));
    assert!(matches!(fnbc.code[1], Op::Jump { target: 0 }));

    // 0-relative validity: every jump target lands inside the slice.
    for op in &fnbc.code {
        if let Some(t) = jump_target(op) {
            assert!(t < fnbc.code.len(), "jump target {t} out of slice bounds");
        }
    }

    // Round-trip: re-absolutizing by +entry_pc reproduces the original span
    // exactly (Op has no PartialEq, so compare its Debug form).
    let reabs: Vec<String> = fnbc
        .code
        .iter()
        .map(|&op| format!("{:?}", rebase(op, 1)))
        .collect();
    let orig: Vec<String> = prog.code[1..4]
        .iter()
        .map(|op| format!("{:?}", op))
        .collect();
    assert_eq!(reabs, orig, "slice must invert back to the original body");

    // Metadata is carried through.
    assert_eq!(fnbc.register_count, 1);
    assert_eq!(fnbc.named_regs, vec![true]);
}

fn body(code: Vec<Op>) -> FnBytecode {
    FnBytecode {
        code,
        constants: vec![],
        register_count: 1,
        param_count: 0,
        param_kinds: vec![],
        ret_kind: None,
        named_regs: vec![],
    }
}

#[test]
fn is_well_formed_accepts_a_valid_body() {
    // JumpIfFalse exits forward, Jump loops back, ends in ReturnNothing — all in range.
    let good = body(vec![
        Op::JumpIfFalse { cond: 0, target: 2 },
        Op::Jump { target: 0 },
        Op::ReturnNothing,
    ]);
    assert!(good.is_well_formed(1));
}

#[test]
fn is_well_formed_rejects_malformed_bodies() {
    // Empty body.
    assert!(!body(vec![]).is_well_formed(1));
    // No terminal op (would fall through into the next warm body / past the buffer).
    assert!(!body(vec![Op::Jump { target: 0 }]).is_well_formed(1));
    // Out-of-range jump target.
    assert!(!body(vec![Op::Jump { target: 99 }, Op::ReturnNothing]).is_well_formed(1));
    // Call to a non-existent function index (func 5 ≥ n_funcs 1).
    assert!(!body(vec![
        Op::Call {
            dst: 0,
            func: 5,
            args_start: 0,
            arg_count: 0
        },
        Op::ReturnNothing,
    ])
    .is_well_formed(1));
    // Same call IS in range when there are enough functions.
    assert!(body(vec![
        Op::Call {
            dst: 0,
            func: 5,
            args_start: 0,
            arg_count: 0
        },
        Op::ReturnNothing,
    ])
    .is_well_formed(6));
}
