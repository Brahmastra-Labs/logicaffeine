use super::*;

// r1 may already contain a parameter sequence on the branch that skips NewRange.
// Adjacency in the bytecode is not dominance of the iterator's entry.
#[test]
fn a_branch_around_range_creation_must_keep_the_selected_iterable() {
    let ops = vec![
        Op::JumpIfFalse { cond: 0, target: 2 },
        Op::NewRange {
            dst: 1,
            start: 2,
            end: 3,
        },
        Op::IterPrepare { iterable: 1 },
        Op::IterNext { dst: 4, exit: 5 },
        Op::Jump { target: 3 },
        Op::IterPop,
        Op::Halt,
    ];
    assert!(
        !range_loop_registers(&ops, &[]).contains_key(&1),
        "a path into IterPrepare bypasses NewRange; the selected sequence must be preserved"
    );
}

#[test]
fn an_exclusively_consumed_range_with_one_entry_can_use_constant_storage() {
    let ops = vec![
        Op::NewRange {
            dst: 1,
            start: 2,
            end: 3,
        },
        Op::IterPrepare { iterable: 1 },
        Op::IterNext { dst: 4, exit: 4 },
        Op::Jump { target: 2 },
        Op::IterPop,
        Op::Halt,
    ];
    assert_eq!(range_loop_registers(&ops, &[]).get(&1), Some(&(2, 3)));
}

#[test]
fn a_range_that_is_also_returned_must_remain_a_sequence() {
    let ops = vec![
        Op::NewRange {
            dst: 1,
            start: 2,
            end: 3,
        },
        Op::IterPrepare { iterable: 1 },
        Op::IterNext { dst: 4, exit: 4 },
        Op::Jump { target: 2 },
        Op::IterPop,
        Op::Return { src: 1 },
    ];
    assert!(!range_loop_registers(&ops, &[]).contains_key(&1));
}
