//! Fixed-capacity allocation is a real heap operation on each fabric path.
#![cfg(not(target_arch = "wasm32"))]
#[path = "common/fabric_witnesses.rs"]
mod fabric_witnesses;

use fabric_witnesses::{hand_bytecode_tapes, run_corpus_fn, CorpusFn};
use logicaffeine_compile::codegen_sva::fabric_rtl::function_op_names;
use logicaffeine_compile::codegen_sva::fabric_target::FabricTarget;
use logicaffeine_compile::codegen_sva::fabric_tile::{fabric_tile, hosted_ops, FabricTile};
use logicaffeine_compile::vm::{Constant, Op};

#[test]
fn fixed_lists_are_real_heap_operations_on_both_rdu_envelopes() {
    let op = Op::NewEmptyListFixed {
        dst: 0,
        capacity: 8,
    };
    assert!(
        matches!(fabric_tile(&op), Some(FabricTile::Mem)),
        "an allocation needs a realized memory tile"
    );
    for target in [FabricTarget::rdu_shaped(), FabricTarget::sn50_shaped()] {
        assert!(hosted_ops(&target).contains(&"NewEmptyListFixed"));
    }
}

#[test]
fn two_fixed_lists_keep_separate_storage_when_their_pushes_interleave() {
    for a in [-100, 0, 37] {
        let (fabric, vm) = hand_bytecode_tapes(
            vec![Constant::Int(a), Constant::Int(99), Constant::Int(1)],
            vec![
                Op::NewEmptyListFixed {
                    dst: 0,
                    capacity: 2,
                },
                Op::NewEmptyListFixed {
                    dst: 1,
                    capacity: 2,
                },
                Op::LoadConst { dst: 2, idx: 0 },
                Op::LoadConst { dst: 3, idx: 1 },
                Op::LoadConst { dst: 4, idx: 2 },
                Op::ListPush { list: 0, value: 2 },
                Op::ListPush { list: 1, value: 3 },
                Op::ListPush { list: 0, value: 3 },
                Op::ListPush { list: 1, value: 2 },
                Op::Index {
                    dst: 5,
                    collection: 0,
                    index: 4,
                },
                Op::Show { src: 5 },
                Op::Index {
                    dst: 5,
                    collection: 1,
                    index: 4,
                },
                Op::Show { src: 5 },
                Op::Length {
                    dst: 5,
                    collection: 0,
                },
                Op::Show { src: 5 },
                Op::Length {
                    dst: 5,
                    collection: 1,
                },
                Op::Show { src: 5 },
                Op::Halt,
            ],
            6,
        );
        assert_eq!(vm, [a.to_string(), "99".into(), "2".into(), "2".into()]);
        assert_eq!(fabric, vm, "fixed buffers must not overwrite one another");
    }
}

#[test]
fn a_source_authored_fixed_list_function_matches_the_vm() {
    let source = r#"## No Inline
## To measure (n: Int) -> Int:
    Let mutable values be a new Seq of Int.
    Push n + 1 to values.
    Push n + 2 to values.
    Push n + 3 to values.
    Push n + 4 to values.
    Push n + 5 to values.
    Push n + 6 to values.
    Push n + 7 to values.
    Push n + 8 to values.
    Let answer be item 8 of values.
    Return answer.

"#;
    assert!(
        function_op_names(source, "measure").contains(&"NewEmptyListFixed"),
        "the witness must exercise the fixed-capacity opcode"
    );
    run_corpus_fn(&CorpusFn {
        src: source,
        fn_name: "measure",
        argc: 1,
    });
}

#[test]
fn two_source_fixed_lists_keep_separate_function_heap_storage() {
    let source = r#"## No Inline
## To measure (n: Int) -> Int:
    Let mutable left be a new Seq of Int.
    Push n + 1 to left.
    Push n + 2 to left.
    Push n + 3 to left.
    Push n + 4 to left.
    Push n + 5 to left.
    Push n + 6 to left.
    Push n + 7 to left.
    Push n + 8 to left.
    Let mutable right be a new Seq of Int.
    Push n - 1 to right.
    Push n - 2 to right.
    Push n - 3 to right.
    Push n - 4 to right.
    Push n - 5 to right.
    Push n - 6 to right.
    Push n - 7 to right.
    Push n - 8 to right.
    Let answer be (item 8 of left) * 17 + (item 8 of right).
    Return answer.

"#;
    assert!(
        function_op_names(source, "measure").contains(&"NewEmptyListFixed"),
        "the function must exercise fixed-capacity allocation"
    );
    run_corpus_fn(&CorpusFn {
        src: source,
        fn_name: "measure",
        argc: 1,
    });
}

#[test]
fn fixed_lists_own_their_capacity_even_with_zero_general_reserve() {
    for a in [-100, 0, 37] {
        let (fabric, vm) =
            logicaffeine_compile::codegen_sva::fabric_rtl::with_seq_reserve(0, || {
                hand_bytecode_tapes(
                    vec![Constant::Int(a), Constant::Int(99), Constant::Int(1)],
                    vec![
                        Op::NewEmptyListFixed {
                            dst: 0,
                            capacity: 2,
                        },
                        Op::NewEmptyListFixed {
                            dst: 1,
                            capacity: 2,
                        },
                        Op::LoadConst { dst: 2, idx: 0 },
                        Op::LoadConst { dst: 3, idx: 1 },
                        Op::LoadConst { dst: 4, idx: 2 },
                        Op::ListPush { list: 0, value: 2 },
                        Op::ListPush { list: 1, value: 3 },
                        Op::ListPush { list: 0, value: 3 },
                        Op::ListPush { list: 1, value: 2 },
                        Op::Index {
                            dst: 5,
                            collection: 0,
                            index: 4,
                        },
                        Op::Show { src: 5 },
                        Op::Index {
                            dst: 5,
                            collection: 1,
                            index: 4,
                        },
                        Op::Show { src: 5 },
                        Op::Length {
                            dst: 5,
                            collection: 0,
                        },
                        Op::Show { src: 5 },
                        Op::Length {
                            dst: 5,
                            collection: 1,
                        },
                        Op::Show { src: 5 },
                        Op::Halt,
                    ],
                    6,
                )
            });
        assert_eq!(vm, [a.to_string(), "99".into(), "2".into(), "2".into()]);
        assert_eq!(fabric, vm, "fixed buffers must not overwrite one another");
    }
}

#[test]
fn cfg_fixed_buffers_reserve_their_proven_capacity() {
    use logicaffeine_compile::codegen_sva::fabric_rtl::{
        eval_program_cfg_fabric, program_cfg_fabric, with_seq_reserve, LoopCell, Src,
    };
    let source = r#"## No Inline
## To measure (n: Int) -> Int:
    Let mutable left be a new Seq of Int.
    Push n + 1 to left.
    Push n + 2 to left.
    Push n + 3 to left.
    Push n + 4 to left.
    Push n + 5 to left.
    Push n + 6 to left.
    Push n + 7 to left.
    Push n + 8 to left.
    Let mutable right be a new Seq of Int.
    Push n - 1 to right.
    Push n - 2 to right.
    Push n - 3 to right.
    Push n - 4 to right.
    Push n - 5 to right.
    Push n - 6 to right.
    Push n - 7 to right.
    Push n - 8 to right.
    Let answer be (item 8 of left) * 17 + (item 8 of right).
    Return answer.

"#;
    assert_eq!(
        function_op_names(source, "measure")
            .iter()
            .filter(|&&op| op == "NewEmptyListFixed")
            .count(),
        2
    );
    for reserve in [0, 1, 2048] {
        let cfg = with_seq_reserve(reserve, || program_cfg_fabric(source, "measure"));
        let capacities: Vec<_> = cfg
            .blocks
            .iter()
            .flat_map(|b| &b.body)
            .filter_map(|cell| {
                if let LoopCell::HeapAlloc {
                    size: Src::Const(size),
                } = cell
                {
                    Some(*size)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            capacities,
            [9, 9],
            "each fixed buffer owns eight elements plus its header regardless of general reserve"
        );
        for n in [-100, 0, 37] {
            let vm_source = format!("{source}\n## Main\nShow measure({n}).\n");
            let vm = logicaffeine_compile::compile::vm_outcome(&vm_source);
            assert_eq!(vm.error, None);
            let expected = 18 * n + 128;
            assert_eq!(vm.output, format!("{expected}\n"));
            assert_eq!(eval_program_cfg_fabric(&cfg, &[n]), expected);
        }
    }
}
