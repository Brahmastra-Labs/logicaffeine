use super::*;

/// `{:?}` of each op — `Op` is not `PartialEq`, and the renaming is entirely about register
/// fields, which the Debug rendering captures exactly.
fn fmt(ops: &[Op]) -> Vec<String> {
    ops.iter().map(|o| format!("{o:?}")).collect()
}

/// Split, and hold the placement the pass reports to what it did: every original op lands at its
/// reported index as the same instruction (its registers may be renamed), and in its original order.
fn split(ops: &[Op], num_regs: u32, num_params: u32) -> (Vec<Op>, u32) {
    let (out, regs, placed) = split_registers(ops, num_regs, num_params, &[]);
    assert_eq!(placed.len(), ops.len(), "one placement per original op");
    for (pc, &at) in placed.iter().enumerate() {
        assert_eq!(
            std::mem::discriminant(&ops[pc]),
            std::mem::discriminant(&out[at]),
            "op {pc} ({:?}) is reported at {at}, which holds {:?}",
            ops[pc],
            out[at]
        );
    }
    assert!(
        placed.windows(2).all(|w| w[0] < w[1]),
        "placement keeps the original op order: {placed:?}"
    );
    (out, regs)
}

#[test]
fn identity_when_every_register_is_single_web() {
    // A straight-line program: each register is defined once and used in one connected range, so
    // there is nothing to split — the pass must be the exact identity (same ops, same count).
    let ops = vec![
        Op::LoadConst { dst: 0, idx: 0 },
        Op::LoadConst { dst: 1, idx: 1 },
        Op::Add {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        Op::Show { src: 2 },
        Op::Halt,
    ];
    let (out, n) = split(&ops, 3, 0);
    assert_eq!(fmt(&out), fmt(&ops), "single-web program must be untouched");
    assert_eq!(n, 3, "no locals added");
}

#[test]
fn disjoint_ranges_of_one_register_split_into_separate_locals() {
    // reg1 is written+read in two disjoint ranges: [pc0 def, pc1 use] then [pc2 def, pc3 use].
    // The second range must move to a fresh local (3); the first keeps the number.
    let ops = vec![
        Op::LoadConst { dst: 1, idx: 0 },
        Op::Show { src: 1 },
        Op::LoadConst { dst: 1, idx: 1 },
        Op::Show { src: 1 },
        Op::Halt,
    ];
    let (out, n) = split(&ops, 2, 0);
    assert_eq!(n, 3, "one extra local for the second web");
    let got = fmt(&out);
    assert_eq!(
        got[0],
        format!("{:?}", Op::LoadConst { dst: 1, idx: 0 }),
        "first web keeps reg 1"
    );
    assert_eq!(got[1], format!("{:?}", Op::Show { src: 1 }));
    assert_eq!(
        got[2],
        format!("{:?}", Op::LoadConst { dst: 2, idx: 1 }),
        "second web → local 2"
    );
    assert_eq!(
        got[3],
        format!("{:?}", Op::Show { src: 2 }),
        "its use follows"
    );
}

#[test]
fn loop_carried_accumulator_is_one_web_not_split() {
    // `acc` is initialized then incremented across a back-edge — the body use is reached by both
    // the init and the back-edge def, so they are one web (a single local), never split.
    let ops = vec![
        Op::LoadConst { dst: 0, idx: 0 }, // acc = 0
        Op::LoadConst { dst: 1, idx: 1 }, // bound
        Op::Lt {
            dst: 2,
            lhs: 0,
            rhs: 1,
        }, // acc < bound  (header, pc2)
        Op::JumpIfFalse { cond: 2, target: 6 },
        Op::AddAssign { dst: 0, src: 1 }, // acc += bound
        Op::Jump { target: 2 },           // back-edge
        Op::Show { src: 0 },              // pc6
        Op::Halt,
    ];
    let (out, n) = split(&ops, 3, 0);
    assert_eq!(
        fmt(&out),
        fmt(&ops),
        "loop-carried accumulator must not split"
    );
    assert_eq!(n, 3);
}

#[test]
fn parameters_are_pinned_and_never_split() {
    // reg0 is a parameter reused across two disjoint ranges. Parameters are pinned (they ARE the
    // function signature), so even a genuine conflict leaves them on their declared number.
    let ops = vec![
        Op::Show { src: 0 },
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Show { src: 0 },
        Op::Return { src: 0 },
    ];
    let (out, n) = split(&ops, 1, 1); // num_params = 1 ⇒ reg0 pinned
    assert_eq!(
        fmt(&out),
        fmt(&ops),
        "a parameter register is never renamed"
    );
    assert_eq!(n, 1);
}

#[test]
fn split_is_deterministic_across_runs() {
    // A diamond merges two definitions of reg1 into one web (so the union has multiple reaching
    // defs — the exact shape whose representative once depended on `HashSet` iteration order),
    // then a disjoint third definition forms a second web that must split out. Running the pass
    // many times must yield byte-identical ops AND a stable new register count.
    let ops = vec![
        Op::LoadConst { dst: 0, idx: 0 }, // cond
        Op::JumpIfFalse { cond: 0, target: 4 },
        Op::LoadConst { dst: 1, idx: 1 }, // def reg1 (branch A) — web W
        Op::Jump { target: 5 },
        Op::LoadConst { dst: 1, idx: 2 }, // def reg1 (branch B) — web W
        Op::Move { dst: 2, src: 1 },      // use reg1: reached by BOTH A and B
        Op::LoadConst { dst: 1, idx: 3 }, // def reg1 — web W2 (disjoint)
        Op::Return { src: 1 },            // use reg1: reached by W2 only
    ];
    let (first, first_n) = split(&ops, 3, 0);
    assert_eq!(first_n, 4, "the second web of reg1 splits to one new local");
    // The disjoint third range moved to local 3; the merged-diamond range kept reg 1.
    let got = fmt(&first);
    assert_eq!(
        got[6],
        format!("{:?}", Op::LoadConst { dst: 3, idx: 3 }),
        "W2 def → local 3"
    );
    assert_eq!(
        got[7],
        format!("{:?}", Op::Return { src: 3 }),
        "W2 use → local 3"
    );
    assert_eq!(
        got[2],
        format!("{:?}", Op::LoadConst { dst: 1, idx: 1 }),
        "W def keeps reg 1"
    );
    assert_eq!(
        got[5],
        format!("{:?}", Op::Move { dst: 2, src: 1 }),
        "W use keeps reg 1"
    );
    for _ in 0..64 {
        let (again, again_n) = split(&ops, 3, 0);
        assert_eq!(
            fmt(&again),
            got,
            "splitting must be deterministic run-to-run"
        );
        assert_eq!(again_n, first_n);
    }
}

/// The MD5 pattern: a register the allocator reused across two disjoint live ranges where EACH
/// range is a member of a DIFFERENT multi-argument call (once a `Rotl` shift-count on `i64`, once
/// a `Rotl` word value on `i32`). Both anchored uses want the same slot number, so the plain split
/// cannot separate them. Argument materialization copies each call's arguments into a fresh
/// dedicated block, turning the reused member into a plain scalar the def-use split then splits.
#[test]
fn recycled_range_member_across_two_calls_is_materialized_and_split() {
    let ops = vec![
        Op::LoadConst { dst: 2, idx: 0 }, // reg2 web A def
        Op::LoadConst { dst: 3, idx: 1 },
        Op::Call {
            dst: 0,
            func: 0,
            args_start: 2,
            arg_count: 2,
        }, // reads reg2 as member 0
        Op::LoadConst { dst: 1, idx: 2 },
        Op::LoadConst { dst: 2, idx: 3 }, // reg2 web B def (reused slot)
        Op::Call {
            dst: 0,
            func: 0,
            args_start: 1,
            arg_count: 2,
        }, // reads reg2 as member 1
        Op::Halt,
    ];
    let (out, n) = split(&ops, 4, 0);
    // Each call's argument block was relocated to a fresh contiguous block (>= the original 4),
    // populated by `arg_count` Moves in the immediately preceding slots.
    let calls: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, o)| matches!(o, Op::Call { .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(calls.len(), 2, "both calls survive");
    for &ci in &calls {
        let Op::Call {
            args_start,
            arg_count,
            ..
        } = out[ci]
        else {
            unreachable!()
        };
        assert!(
            args_start >= 4,
            "call args relocated to a fresh block, got {args_start}"
        );
        for j in 0..arg_count {
            let mv = &out[ci - arg_count as usize + j as usize];
            assert!(
                matches!(mv, Op::Move { dst, .. } if *dst == args_start + j),
                "arg slot {} populated by a preceding Move, got {mv:?}",
                args_start + j
            );
        }
    }
    // reg2's two disjoint definitions now write DISTINCT locals — the reuse was split.
    let dsts: Vec<Reg> = out
        .iter()
        .filter_map(|o| match o {
            Op::LoadConst { dst, idx } if *idx == 0 || *idx == 3 => Some(*dst),
            _ => None,
        })
        .collect();
    assert_eq!(dsts.len(), 2);
    assert_ne!(
        dsts[0], dsts[1],
        "the reused register was split into two locals"
    );
    assert!(
        n > 4,
        "register file grew for the fresh blocks and the split web"
    );
}

/// A range member reused by ONE other, free web — a list literal's element slot that an earlier
/// scalar also used — is separated by moving the free web, not by copying the range: the range's
/// web keeps its number, so the literal still reads its elements where they were defined. Copying
/// the whole range would cost a `Move` per element — code per ITEM for a literal of any length.
#[test]
fn a_range_member_reused_by_one_free_web_is_split_without_copying_the_range() {
    let ops = vec![
        Op::LoadConst { dst: 2, idx: 0 }, // reg2 web A: a scalar
        Op::Show { src: 2 },              // its only use
        Op::LoadConst { dst: 1, idx: 1 },
        Op::LoadConst { dst: 2, idx: 2 }, // reg2 web B: a list element
        Op::LoadConst { dst: 3, idx: 3 },
        Op::NewList {
            dst: 0,
            start: 1,
            count: 3,
        }, // reads regs 1..=3
        Op::Halt,
    ];
    let (out, n) = split(&ops, 4, 0);
    assert!(
        !out.iter()
            .any(|o| matches!(o, Op::Move { dst, .. } if *dst >= 4)),
        "the range was copied into a fresh block: {:?}",
        fmt(&out)
    );
    let list = out
        .iter()
        .find(|o| matches!(o, Op::NewList { .. }))
        .expect("the literal survives");
    assert_eq!(
        format!("{list:?}"),
        format!(
            "{:?}",
            Op::NewList {
                dst: 0,
                start: 1,
                count: 3
            }
        )
    );
    let scalar_def = out.iter().find_map(|o| match o {
        Op::LoadConst { dst, idx: 0 } => Some(*dst),
        _ => None,
    });
    assert_eq!(scalar_def, Some(4), "the free web moved to one fresh local");
    assert_eq!(n, 5, "exactly one local was added");
}

/// A multi-argument call whose members are each single-web (defined once, used once) has no reuse
/// to resolve — materialization must be the exact identity, so a program the backend already
/// compiles is never perturbed.
#[test]
fn single_web_range_members_are_not_materialized() {
    let ops = vec![
        Op::LoadConst { dst: 1, idx: 0 },
        Op::LoadConst { dst: 2, idx: 1 },
        Op::Call {
            dst: 0,
            func: 0,
            args_start: 1,
            arg_count: 2,
        },
        Op::Halt,
    ];
    let (out, n) = split(&ops, 3, 0);
    assert_eq!(
        fmt(&out),
        fmt(&ops),
        "no reused range member ⇒ no materialization"
    );
    assert_eq!(n, 3);
}

/// Materialization inserts copy ops, shifting every later instruction, so a jump whose target sits
/// past an inserted block must be remapped or it lands on the wrong op. The remapped target must
/// resolve to the argument setup of the same call it named before the transform.
#[test]
fn materialization_remaps_jump_targets() {
    let ops = vec![
        Op::LoadConst { dst: 2, idx: 0 }, // reg2 web A def
        Op::LoadConst { dst: 3, idx: 1 },
        Op::Call {
            dst: 0,
            func: 0,
            args_start: 2,
            arg_count: 2,
        }, // materialized (reg2 reused)
        Op::LoadConst { dst: 2, idx: 2 }, // reg2 web B def
        Op::Jump { target: 6 },           // → the second call
        Op::LoadConst { dst: 9, idx: 3 }, // skipped
        Op::Call {
            dst: 0,
            func: 0,
            args_start: 2,
            arg_count: 2,
        }, // materialized; the jump target
        Op::Halt,
    ];
    let (out, _n) = split(&ops, 4, 0);
    let Some(Op::Jump { target }) = out.iter().find(|o| matches!(o, Op::Jump { .. })).copied()
    else {
        panic!("jump survived materialization");
    };
    // The target lands on the relocated call's argument setup (a Move), and a call reading a
    // materialized block (fresh args_start) follows — the same call the jump originally named.
    assert!(
        matches!(out[target], Op::Move { .. }),
        "jump lands on the call's arg setup, got {:?}",
        out[target]
    );
    let next_call = out[target..].iter().find(|o| matches!(o, Op::Call { .. }));
    assert!(
        matches!(next_call, Some(Op::Call { args_start, .. }) if *args_start >= 4),
        "the targeted call reads a materialized block, got {next_call:?}"
    );
}

/// Materialization + the follow-on split must be byte-identical run-to-run (fresh blocks assigned
/// in pc order, webs canonicalized deterministically) — a nondeterministic pass would break the
/// tier cache keyed by source.
#[test]
fn materialization_is_deterministic() {
    let ops = vec![
        Op::LoadConst { dst: 2, idx: 0 },
        Op::LoadConst { dst: 3, idx: 1 },
        Op::Call {
            dst: 0,
            func: 0,
            args_start: 2,
            arg_count: 2,
        },
        Op::LoadConst { dst: 1, idx: 2 },
        Op::LoadConst { dst: 2, idx: 3 },
        Op::Call {
            dst: 0,
            func: 0,
            args_start: 1,
            arg_count: 2,
        },
        Op::Halt,
    ];
    let (first, first_n) = split(&ops, 4, 0);
    let got = fmt(&first);
    for _ in 0..64 {
        let (again, again_n) = split(&ops, 4, 0);
        assert_eq!(
            fmt(&again),
            got,
            "materialization must be deterministic run-to-run"
        );
        assert_eq!(again_n, first_n);
    }
}

#[test]
fn nothing_clear_is_unobserved_only_before_an_unconditional_replacement() {
    use crate::vm::instruction::Constant;
    let constants = [Constant::Nothing, Constant::Int(7)];
    let ops = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::LoadConst { dst: 0, idx: 1 },
        Op::Show { src: 0 },
        Op::ReturnNothing,
    ];
    assert_eq!(
        unobserved_nothing_writes(&ops, &constants, &[], &[0]),
        HashSet::from([0])
    );
    let observed = [
        ops[0].clone(),
        Op::Show { src: 0 },
        ops[1].clone(),
        Op::ReturnNothing,
    ];
    assert!(unobserved_nothing_writes(&observed, &constants, &[], &[0]).is_empty());
}

#[test]
fn a_branch_or_backedge_read_preserves_nothings_identity() {
    use crate::vm::instruction::Constant;
    let constants = [Constant::Nothing, Constant::Int(7)];
    let branch = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::JumpIfFalse { cond: 1, target: 3 },
        Op::LoadConst { dst: 0, idx: 1 },
        Op::Show { src: 0 },
        Op::ReturnNothing,
    ];
    assert!(unobserved_nothing_writes(&branch, &constants, &[], &[]).is_empty());
    let backedge = [
        Op::Show { src: 0 },
        Op::LoadConst { dst: 0, idx: 0 },
        Op::JumpIfTrue { cond: 1, target: 0 },
        Op::ReturnNothing,
    ];
    assert!(unobserved_nothing_writes(&backedge, &constants, &[], &[]).is_empty());
}

#[test]
fn inout_results_read_absence_at_explicit_and_fallthrough_exits() {
    use crate::vm::instruction::Constant;
    for last in [
        None,
        Some(Op::ReturnNothing),
        Some(Op::Return { src: 1 }),
        Some(Op::Halt),
    ] {
        let mut ops = vec![Op::LoadConst { dst: 0, idx: 0 }];
        ops.extend(last);
        assert!(unobserved_nothing_writes(&ops, &[Constant::Nothing], &[], &[0]).is_empty());
        assert_eq!(
            unobserved_nothing_writes(&ops, &[Constant::Nothing], &[], &[]),
            HashSet::from([0])
        );
    }
}

#[test]
fn a_call_observes_its_argument_before_writing_back() {
    use crate::vm::instruction::Constant;
    let ops = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Call {
            dst: 2,
            func: 0,
            args_start: 0,
            arg_count: 1,
        },
        Op::LoadConst { dst: 0, idx: 1 },
        Op::ReturnNothing,
    ];
    assert!(
        unobserved_nothing_writes(&ops, &[Constant::Nothing, Constant::Int(7)], &[], &[])
            .is_empty()
    );
}

#[test]
fn nothing_liveness_covers_more_than_one_bitmap_word() {
    use crate::vm::instruction::Constant;
    let mut ops: Vec<_> = (0..130).map(|dst| Op::LoadConst { dst, idx: 0 }).collect();
    ops.extend([
        Op::Show { src: 64 },
        Op::Show { src: 129 },
        Op::ReturnNothing,
    ]);
    let expected: HashSet<_> = (0..130).filter(|r| ![63, 64, 129].contains(r)).collect();
    assert_eq!(
        unobserved_nothing_writes(&ops, &[Constant::Nothing], &[], &[63]),
        expected
    );
}

#[test]
fn a_final_conditional_edge_can_return_an_inout_value() {
    use crate::vm::instruction::Constant;
    let ops = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::JumpIfTrue { cond: 1, target: 0 },
    ];
    assert!(unobserved_nothing_writes(&ops, &[Constant::Nothing], &[], &[0]).is_empty());
}
