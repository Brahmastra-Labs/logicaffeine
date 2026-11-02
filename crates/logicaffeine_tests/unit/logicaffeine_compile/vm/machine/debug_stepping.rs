//! The debug stepper (`STEPPED = true`): `run_steps` advances exactly one op
//! per call, yields `Paused` between ops, exposes the paused state via
//! `debug_view`, and — the soundness invariant — produces output BYTE-IDENTICAL
//! to a single-shot `run()`. This is the contract the Studio debug drawer rides.
use super::*;

/// `Let x be 6. Let y be 7. Show x times y.` hand-lowered to bytecode.
fn mul_program() -> CompiledProgram {
    CompiledProgram {
        constants: vec![Constant::Int(6), Constant::Int(7)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::Mul {
                dst: 2,
                lhs: 0,
                rhs: 1,
            },
            Op::Show { src: 2 },
            Op::Halt,
        ],
        register_count: 3,
        ..Default::default()
    }
}

#[test]
fn run_steps_advances_one_op_and_pauses() {
    let prog = mul_program();
    let mut vm = Vm::new(&prog);
    // First op (LoadConst R0 = 6) → paused at pc 1 with R0 visible.
    assert!(matches!(vm.run_steps(1).unwrap(), VmStep::Paused));
    let v = vm.debug_view();
    assert_eq!(v.pc, 1, "stopped before the second instruction");
    assert_eq!(v.frames.len(), 1, "single Main frame");
    assert_eq!(
        v.frames[0].registers[0],
        (0u16, "Int".to_string(), "6".to_string())
    );
    // Second op (LoadConst R1 = 7).
    assert!(matches!(vm.run_steps(1).unwrap(), VmStep::Paused));
    let v = vm.debug_view();
    assert_eq!(v.pc, 2);
    assert_eq!(
        v.frames[0].registers[1],
        (1u16, "Int".to_string(), "7".to_string())
    );
    // Third op (Mul R2 = R0 * R1).
    assert!(matches!(vm.run_steps(1).unwrap(), VmStep::Paused));
    assert_eq!(
        vm.debug_view().frames[0].registers[2],
        (2u16, "Int".to_string(), "42".to_string())
    );
}

#[test]
fn stepped_run_is_byte_identical_to_single_shot() {
    let prog = mul_program();

    // Stepped to completion, one op at a time.
    let mut stepper = Vm::new(&prog);
    let mut pauses = 0usize;
    loop {
        match stepper.run_steps(1).unwrap() {
            VmStep::Paused => pauses += 1,
            VmStep::Done(_) => break,
            VmStep::Blocked => unreachable!("no concurrency op in this program"),
        }
    }
    let stepped_out = stepper.into_lines();

    // Single-shot run of the very same program.
    let mut oneshot = Vm::new(&prog);
    oneshot.run().unwrap();
    let oneshot_out = oneshot.into_lines();

    assert_eq!(
        stepped_out, oneshot_out,
        "stepping must not change observable output"
    );
    assert_eq!(stepped_out, vec!["42".to_string()]);
    assert_eq!(pauses, 4, "one pause after each of the 4 ops before Halt");
}

#[test]
fn larger_budget_runs_several_ops_then_pauses() {
    let prog = mul_program();
    let mut vm = Vm::new(&prog);
    // Budget of 3 runs the two loads + the Mul, then pauses at the Show (pc 3).
    assert!(matches!(vm.run_steps(3).unwrap(), VmStep::Paused));
    let v = vm.debug_view();
    assert_eq!(v.pc, 3);
    assert_eq!(
        v.frames[0].registers[2],
        (2u16, "Int".to_string(), "42".to_string())
    );
    // The rest completes.
    assert!(matches!(vm.run_steps(u64::MAX).unwrap(), VmStep::Done(_)));
    assert_eq!(vm.into_lines(), vec!["42".to_string()]);
}
