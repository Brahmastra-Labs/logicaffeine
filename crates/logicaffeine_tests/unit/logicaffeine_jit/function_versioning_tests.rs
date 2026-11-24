use super::*;
use logicaffeine_forge::jit::{compile_straightline, ChainOutcome, Cmp};

const FLAG_THEN_DOUBLE_ELSE_ZERO: [MicroOp; 5] = [
    MicroOp::JumpIfFalse { cond: 0, target: 3 },
    MicroOp::Add { dst: 2, lhs: 1, rhs: 1 },
    MicroOp::Jump { target: 4 },
    MicroOp::Sub { dst: 2, lhs: 1, rhs: 1 },
    MicroOp::Return { src: 2 },
];

#[test]
fn a_branch_on_a_held_flag_takes_its_side_and_the_other_side_is_gone() {
    let mut on = FLAG_THEN_DOUBLE_ELSE_ZERO.to_vec();
    let guards = fold_known_branches(&mut on, &[Some(ScalarValue::Bool(true)), None, None]);
    assert_eq!(guards, vec![(0, ScalarValue::Bool(true))]);
    assert_eq!(
        on,
        vec![
            MicroOp::Jump { target: 1 },
            MicroOp::Add { dst: 2, lhs: 1, rhs: 1 },
            MicroOp::Jump { target: 4 },
            MicroOp::Jump { target: 3 },
            MicroOp::Return { src: 2 },
        ]
    );

    let mut off = FLAG_THEN_DOUBLE_ELSE_ZERO.to_vec();
    let guards = fold_known_branches(&mut off, &[Some(ScalarValue::Bool(false)), None, None]);
    assert_eq!(guards, vec![(0, ScalarValue::Bool(false))]);
    assert_eq!(
        off,
        vec![
            MicroOp::Jump { target: 3 },
            MicroOp::Jump { target: 1 },
            MicroOp::Jump { target: 2 },
            MicroOp::Sub { dst: 2, lhs: 1, rhs: 1 },
            MicroOp::Return { src: 2 },
        ]
    );
}

/// `k > 5`, fused, against a constant loaded in the same block.
const ABOVE_FIVE: [MicroOp; 6] = [
    MicroOp::LoadConst { dst: 2, value: 5 },
    MicroOp::Branch { cmp: Cmp::Gt, lhs: 0, rhs: 2, target: 4 },
    MicroOp::LoadConst { dst: 3, value: 1 },
    MicroOp::Return { src: 3 },
    MicroOp::LoadConst { dst: 3, value: 0 },
    MicroOp::Return { src: 3 },
];

#[test]
fn a_fused_comparison_of_a_held_int_with_a_constant_is_decided_both_ways() {
    for (k, taken, dead) in [(9, 2, [4, 5]), (6, 2, [4, 5]), (5, 4, [2, 3]), (i64::MIN, 4, [2, 3])] {
        let mut micro = ABOVE_FIVE.to_vec();
        let guards = fold_known_branches(&mut micro, &[Some(ScalarValue::Int(k)), None]);
        assert_eq!(guards, vec![(0, ScalarValue::Int(k))], "k = {k}");
        assert_eq!(micro[1], MicroOp::Jump { target: taken }, "k = {k}");
        for at in dead {
            assert_eq!(micro[at], MicroOp::Jump { target: at }, "k = {k}: op {at} is never reached");
        }
        assert_eq!(micro[0], ABOVE_FIVE[0]);
    }
}

#[test]
fn a_comparison_materialized_from_a_copy_of_a_held_int_is_decided() {
    let code = [
        MicroOp::Move { dst: 3, src: 0 },
        MicroOp::LoadConst { dst: 4, value: 10 },
        MicroOp::Eq { dst: 5, lhs: 3, rhs: 4 },
        MicroOp::JumpIfTrue { cond: 5, target: 5 },
        MicroOp::Return { src: 1 },
        MicroOp::Return { src: 0 },
    ];
    let mut equal = code.to_vec();
    assert_eq!(fold_known_branches(&mut equal, &[Some(ScalarValue::Int(10)), None]), vec![(0, ScalarValue::Int(10))]);
    assert_eq!(equal[3], MicroOp::Jump { target: 5 });
    assert_eq!(equal[4], MicroOp::Jump { target: 4 });

    let mut unequal = code.to_vec();
    assert_eq!(fold_known_branches(&mut unequal, &[Some(ScalarValue::Int(11)), None]), vec![(0, ScalarValue::Int(11))]);
    assert_eq!(unequal[3], MicroOp::Jump { target: 4 });
    assert_eq!(unequal[5], MicroOp::Jump { target: 5 });
}

#[test]
fn a_negated_held_flag_is_decided() {
    let mut micro = vec![
        MicroOp::NotBool { dst: 1, src: 0 },
        MicroOp::JumpIfFalse { cond: 1, target: 3 },
        MicroOp::Return { src: 0 },
        MicroOp::Return { src: 1 },
    ];
    assert_eq!(fold_known_branches(&mut micro, &[Some(ScalarValue::Bool(true))]), vec![(0, ScalarValue::Bool(true))]);
    assert_eq!(micro[1], MicroOp::Jump { target: 3 });
    assert_eq!(micro[2], MicroOp::Jump { target: 2 });
}

#[test]
fn a_parameter_the_function_writes_is_not_folded_on() {
    let code = vec![
        MicroOp::JumpIfFalse { cond: 0, target: 3 },
        MicroOp::LoadConst { dst: 0, value: 0 },
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 0 },
    ];
    let mut micro = code.clone();
    assert_eq!(fold_known_branches(&mut micro, &[Some(ScalarValue::Bool(true))]), vec![]);
    assert_eq!(micro, code);
}

#[test]
fn a_value_is_forgotten_where_paths_join_and_where_it_is_overwritten() {
    let join = vec![
        MicroOp::LoadConst { dst: 2, value: 1 },
        MicroOp::JumpIfFalse { cond: 1, target: 3 },
        MicroOp::LoadConst { dst: 2, value: 0 },
        MicroOp::JumpIfTrue { cond: 2, target: 5 },
        MicroOp::Return { src: 1 },
        MicroOp::Return { src: 2 },
    ];
    let mut micro = join.clone();
    assert_eq!(fold_known_branches(&mut micro, &[None, None, None]), vec![]);
    assert_eq!(micro, join, "slot 2 is 1 or 0 where the paths meet");

    let overwritten = vec![
        MicroOp::LoadConst { dst: 2, value: 3 },
        MicroOp::Add { dst: 2, lhs: 2, rhs: 1 },
        MicroOp::Branch { cmp: Cmp::Eq, lhs: 2, rhs: 0, target: 4 },
        MicroOp::Return { src: 0 },
        MicroOp::Return { src: 2 },
    ];
    let mut micro = overwritten.clone();
    assert_eq!(fold_known_branches(&mut micro, &[Some(ScalarValue::Int(3)), None, None]), vec![]);
    assert_eq!(micro, overwritten, "slot 2 no longer holds the constant once the add writes it");
}

#[test]
fn a_held_parameter_no_branch_reads_folds_nothing() {
    let code = vec![MicroOp::Add { dst: 2, lhs: 0, rhs: 1 }, MicroOp::Return { src: 2 }];
    let mut micro = code.clone();
    assert_eq!(fold_known_branches(&mut micro, &[Some(ScalarValue::Int(4)), Some(ScalarValue::Int(5)), None]), vec![]);
    assert_eq!(micro, code);
}

#[test]
fn the_versioned_stream_checks_each_value_then_runs_one_body_of_two() {
    let specialized = [MicroOp::Jump { target: 1 }, MicroOp::Return { src: 0 }];
    let generic = [
        MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 2 },
        MicroOp::Return { src: 1 },
        MicroOp::Return { src: 0 },
    ];
    let guards = [(0, ScalarValue::Int(7)), (2, ScalarValue::Bool(true))];
    assert_eq!(
        version_on_entry(&specialized, &generic, &guards, 9),
        vec![
            MicroOp::LoadConst { dst: 9, value: 7 },
            MicroOp::Branch { cmp: Cmp::Eq, lhs: 0, rhs: 9, target: 6 },
            MicroOp::LoadConst { dst: 9, value: 1 },
            MicroOp::Branch { cmp: Cmp::Eq, lhs: 2, rhs: 9, target: 6 },
            MicroOp::Jump { target: 5 },
            MicroOp::Return { src: 0 },
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 8 },
            MicroOp::Return { src: 1 },
            MicroOp::Return { src: 0 },
        ]
    );
}

fn run(micro: &[MicroOp], frame: &mut [i64]) -> ChainOutcome {
    compile_straightline(micro).expect("the stream compiles").run_with_frame(frame)
}

/// `x / k` and `x % k`, specialized on `k = 10`: a call with ten runs the reciprocal, any other the division.
#[test]
fn a_function_versioned_on_its_divisor_divides_by_every_divisor_it_is_called_with() {
    let generic = vec![
        MicroOp::Div { dst: 2, lhs: 1, rhs: 0 },
        MicroOp::Mod { dst: 3, lhs: 1, rhs: 0 },
        MicroOp::Add { dst: 2, lhs: 2, rhs: 3 },
        MicroOp::Return { src: 2 },
    ];
    let mut specialized = generic.clone();
    let values = [Some(ScalarValue::Int(10)), None];
    let guards = specialize_invariant_divisors(&mut specialized, &[(0, SlotKind::Int), (1, SlotKind::Int)], &values);
    assert_eq!(guards, vec![(0, ScalarValue::Int(10))]);
    assert!(matches!(specialized[0], MicroOp::MagicDivS { .. }) && matches!(specialized[1], MicroOp::MagicDivS { .. }));
    let versioned = version_on_entry(&specialized, &generic, &guards, 4);
    for k in [10, 3, 7, -10, 1, -1, 11, i64::MAX] {
        for x in [0, 1, -1, 9, 10, 11, -11, 99, 12345, -98765, i64::MAX, i64::MIN + 1] {
            let mut frame = [k, x, 0, 0, 0, 0, 0];
            let expected = x.wrapping_div(k) + x.wrapping_rem(k);
            assert_eq!(run(&versioned, &mut frame), ChainOutcome::Return(expected), "k = {k}, x = {x}");
        }
    }
    let mut frame = [0, 5, 0, 0, 0, 0, 0];
    assert!(run(&versioned, &mut frame).is_deopt(), "a zero divisor takes the generic division, which side-exits");
}

#[test]
fn a_function_versioned_on_its_flag_computes_the_generic_answer_for_either_flag() {
    for held in [true, false] {
        let mut specialized = FLAG_THEN_DOUBLE_ELSE_ZERO.to_vec();
        let guards = fold_known_branches(&mut specialized, &[Some(ScalarValue::Bool(held)), None, None]);
        let versioned = version_on_entry(&specialized, &FLAG_THEN_DOUBLE_ELSE_ZERO, &guards, 3);
        for flag in [0, 1] {
            for x in [-3, 0, 21] {
                let mut frame = [flag, x, 0, 0, 0, 0];
                let expected = if flag == 1 { x + x } else { 0 };
                assert_eq!(
                    run(&versioned, &mut frame),
                    ChainOutcome::Return(expected),
                    "specialized on {held}, called with {flag}, x = {x}"
                );
            }
        }
    }
}

/// R1's lowering of `If n < k: Return n. Return d1(k, n / k).`: the self tail call becomes a loop that rewrites both
/// parameters, `k` with a copy of itself.
const TAIL_CALL_DIVIDES: [MicroOp; 8] = [
    MicroOp::Branch { cmp: Cmp::Lt, lhs: 1, rhs: 0, target: 2 },
    MicroOp::Return { src: 1 },
    MicroOp::Move { dst: 2, src: 0 },
    MicroOp::Div { dst: 3, lhs: 1, rhs: 0 },
    MicroOp::Move { dst: 0, src: 2 },
    MicroOp::Move { dst: 1, src: 3 },
    MicroOp::Jump { target: 0 },
    MicroOp::Return { src: 0 },
];

#[test]
fn a_parameter_a_tail_call_passes_on_unchanged_holds_its_entry_value_everywhere() {
    let holds = entry_copies(&TAIL_CALL_DIVIDES, &[0, 1]);
    for i in 0..7 {
        assert_eq!(holds[i][0], Some(0), "k holds its entry value before op {i}");
    }
    assert_eq!(holds[3][2], Some(0), "the copy holds it too");
    assert_eq!(holds[0][1], None, "n is rewritten with the quotient, which is not its entry value");
    assert_eq!(holds[3][1], None);
}

#[test]
fn a_tail_call_passing_its_divisor_on_unchanged_divides_by_the_reciprocal() {
    let mut micro = TAIL_CALL_DIVIDES.to_vec();
    let guards = specialize_invariant_divisors(&mut micro, &[(0, SlotKind::Int), (1, SlotKind::Int)], &[
        Some(ScalarValue::Int(10)),
        None,
    ]);
    assert_eq!(guards, vec![(0, ScalarValue::Int(10))]);
    assert!(matches!(micro[3], MicroOp::MagicDivS { dst: 3, lhs: 1, mul_back: 0, .. }), "{:?}", micro[3]);
    let versioned = version_on_entry(&micro, &TAIL_CALL_DIVIDES, &guards, 4);
    for k in [10, 3, 2, 1000] {
        for n in [0, 9, 10, 99, 12345, i64::MAX] {
            let mut frame = [k, n, 0, 0, 0, 0, 0];
            let mut expected = n;
            while expected >= k {
                expected /= k;
            }
            assert_eq!(run(&versioned, &mut frame), ChainOutcome::Return(expected), "k = {k}, n = {n}");
        }
    }
}

#[test]
fn a_slot_holds_a_copy_only_where_every_path_brings_it() {
    let micro = [
        MicroOp::JumpIfFalse { cond: 1, target: 3 },
        MicroOp::Move { dst: 2, src: 0 },
        MicroOp::Jump { target: 4 },
        MicroOp::LoadConst { dst: 2, value: 7 },
        MicroOp::Move { dst: 3, src: 0 },
        MicroOp::Return { src: 2 },
    ];
    let holds = entry_copies(&micro, &[0]);
    assert_eq!(holds[2][2], Some(0), "on the move's path, slot 2 holds the entry value");
    assert_eq!(holds[4][2], None, "where the paths meet, one brought a constant");
    assert_eq!(holds[5][3], Some(0));
    assert_eq!(holds[5][0], Some(0), "slot 0 is never written");
}

#[test]
fn a_parameter_overwritten_on_a_later_path_is_held_only_before_the_write() {
    let micro = [
        MicroOp::Move { dst: 1, src: 0 },
        MicroOp::LoadConst { dst: 0, value: 3 },
        MicroOp::Move { dst: 2, src: 0 },
        MicroOp::Return { src: 2 },
    ];
    let holds = entry_copies(&micro, &[0]);
    assert_eq!(holds[1][0], Some(0));
    assert_eq!(holds[2][0], None, "the constant replaced it");
    assert_eq!(holds[3][1], Some(0), "the copy made before the write still holds it");
    assert_eq!(holds[3][2], None);
}

#[test]
fn an_op_no_path_reaches_holds_nothing() {
    let micro = [MicroOp::Return { src: 0 }, MicroOp::Move { dst: 1, src: 0 }, MicroOp::Return { src: 1 }];
    let holds = entry_copies(&micro, &[0]);
    assert_eq!(holds[0][0], Some(0));
    assert_eq!(holds[1], vec![None, None]);
    assert_eq!(holds[2], vec![None, None]);
}

#[test]
fn a_loop_that_rewrites_its_divisor_with_another_value_keeps_its_division() {
    let micro = vec![
        MicroOp::Div { dst: 2, lhs: 1, rhs: 0 },
        MicroOp::Add { dst: 0, lhs: 0, rhs: 0 },
        MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 4 },
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 2 },
    ];
    let mut specialized = micro.clone();
    let guards = specialize_invariant_divisors(&mut specialized, &[(0, SlotKind::Int)], &[Some(ScalarValue::Int(3))]);
    assert_eq!(guards, vec![]);
    assert_eq!(specialized, micro, "the divisor doubles on every pass");
}
