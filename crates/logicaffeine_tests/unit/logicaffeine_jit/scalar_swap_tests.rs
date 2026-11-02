use super::*;

#[test]
fn scalar_swap_preserves_kinds_at_the_following_deopt_point() {
    let ops = [Op::RevSwap { a: 0, b: 1 }, Op::ReturnNothing];
    let kinds = kind_flow(&ops, 40, &[], vec![Kind::Int, Kind::Float], None, &[])
        .expect("two scalars are representable");
    assert_eq!(kinds[1], Some(vec![Kind::Float, Kind::Int]));
}

#[test]
fn scalar_swap_preserves_word_widths() {
    let ops = [Op::RevSwap { a: 0, b: 1 }, Op::ReturnNothing];
    let kinds = kind_flow(&ops, 0, &[], vec![Kind::Word32, Kind::Word64], None, &[])
        .expect("word slots carry their raw bits");
    assert_eq!(kinds[1], Some(vec![Kind::Word64, Kind::Word32]));
}

#[test]
fn scalar_swap_rejects_pinned_or_boxed_values() {
    let ops = [Op::RevSwap { a: 0, b: 1 }, Op::ReturnNothing];
    for kind in [
        Kind::IntList,
        Kind::IntListI32,
        Kind::FloatList,
        Kind::BoolList,
        Kind::Word32List,
        Kind::IntMap,
        Kind::TextBytes,
        Kind::Nothing,
        Kind::Unknown,
        Kind::Mixed,
    ] {
        for entry in [vec![kind, Kind::Int], vec![Kind::Int, kind]] {
            assert!(
                kind_flow(&ops, 0, &[], entry.clone(), None, &[]).is_none(),
                "cannot swap frame slots for {entry:?}"
            );
        }
    }
}

#[test]
fn list_discovery_follows_swap_in_both_directions() {
    let ops = [Op::RevSwap { a: 0, b: 1 }, Op::Move { dst: 2, src: 1 }];
    for root in [0, 1] {
        assert_eq!(
            discover_list_regs(&ops, &[root], false, 0, 4),
            vec![true, true, true, false]
        );
    }
}

#[test]
fn swap_inputs_stay_live_even_when_only_one_result_is_returned() {
    let ops = [Op::RevSwap { a: 0, b: 1 }, Op::Return { src: 1 }];
    let live = liveness(&ops, 0, 3).expect("scalar swap is supported");
    assert_eq!(live[0], vec![true, true, false]);
    assert_eq!(live[1], vec![false, true, false]);
}

#[test]
fn a_self_swap_preserves_its_kind_and_input_liveness() {
    let ops = [Op::RevSwap { a: 0, b: 0 }, Op::Return { src: 0 }];
    let kinds = kind_flow(&ops, 0, &[], vec![Kind::Int, Kind::Float], None, &[])
        .expect("self swap is a scalar no-op");
    assert_eq!(kinds[1], Some(vec![Kind::Int, Kind::Float]));
    assert_eq!(liveness(&ops, 0, 2).unwrap()[0], vec![true, false]);
}
