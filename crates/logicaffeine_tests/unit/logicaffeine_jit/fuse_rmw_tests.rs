use super::*;
use std::collections::HashSet;

// register_count = 8 (slots 0..7 are VM registers); slots 6/7 stand in for
// the pinned array's ptr/len cells. `named` marks slot 5 (the index) live;
// the scratch temps 3 (loaded value) and 4 (sum) are un-named.
fn named8(live: &[u16]) -> Vec<bool> {
    let mut n = vec![false; 8];
    for &s in live {
        n[s as usize] = true;
    }
    n
}

/// The canonical histogram idiom `arr[i] = arr[i] + operand` — ArrLoad into
/// a single-use scratch, Add with the operand, ArrStore the result back to
/// the SAME array+index — collapses to one ArrRMW.
#[test]
fn load_add_store_to_same_cell_fuses_to_rmw() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Add {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut micro, &named8(&[2, 5]), 8, &HashSet::new());
    assert_eq!(
        micro.len(),
        2,
        "the 3-op idiom must collapse to 1 op + Return"
    );
    match micro[0] {
        MicroOp::ArrRMW {
            idx,
            operand,
            ptr_slot,
            len_slot,
            op,
            checked,
        } => {
            assert_eq!((idx, operand, ptr_slot, len_slot), (5, 2, 6, 7));
            assert_eq!(op, RmwOp::Add);
            assert!(checked);
        }
        ref other => panic!("expected ArrRMW, got {other:?}"),
    }
}

/// Subtraction is non-commutative: it fuses ONLY when the loaded element is
/// the left operand (`buf[i] - operand`), and the operand is the right.
#[test]
fn sub_fuses_only_with_loaded_value_on_the_left() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::Sub {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut micro, &named8(&[2, 5]), 8, &HashSet::new());
    match micro[0] {
        MicroOp::ArrRMW {
            op: RmwOp::Sub,
            operand: 2,
            checked: false,
            ..
        } => {}
        ref other => panic!("expected unchecked ArrRMW Sub operand=2, got {other:?}"),
    }

    // operand - buf[i] is NOT an RMW (the loaded value is on the right).
    let mut rev = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::Sub {
            dst: 4,
            lhs: 2,
            rhs: 3,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut rev, &named8(&[2, 5]), 8, &HashSet::new());
    assert!(
        matches!(rev[0], MicroOp::ArrLoad { .. }),
        "operand - buf[i] must NOT fuse: {rev:?}"
    );
}

/// A bounds-checked load with an unchecked store (or vice versa) fuses to a
/// CHECKED rmw — a single pre-write check covers both (same index).
#[test]
fn mixed_checked_load_unchecked_store_fuses_checked() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::BitOr {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut micro, &named8(&[2, 5]), 8, &HashSet::new());
    match micro[0] {
        MicroOp::ArrRMW {
            op: RmwOp::Or,
            checked: true,
            ..
        } => {}
        ref other => panic!("expected checked ArrRMW Or, got {other:?}"),
    }
}

/// The loaded value must be single-use: if `t` is read again (here also
/// stored elsewhere), folding it away would lose that read — no fusion.
#[test]
fn multi_use_loaded_value_blocks_fusion() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Add {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Move { dst: 1, src: 3 }, // second use of the loaded value
        MicroOp::Return { src: 1 },
    ];
    fuse_array_rmw(&mut micro, &named8(&[1, 2, 5]), 8, &HashSet::new());
    assert!(
        matches!(micro[0], MicroOp::ArrLoad { .. }),
        "multi-use t must block fusion"
    );
}

/// Load and store to DIFFERENT arrays (or different indices) is not an
/// in-place RMW and must never fuse.
#[test]
fn different_array_or_index_blocks_fusion() {
    // different ptr_slot
    let mut a = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Add {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 1,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut a, &named8(&[1, 2, 5]), 8, &HashSet::new());
    assert!(
        matches!(a[0], MicroOp::ArrLoad { .. }),
        "different array must block fusion"
    );
    // different index
    let mut b = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Add {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 1,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut b, &named8(&[1, 2, 5]), 8, &HashSet::new());
    assert!(
        matches!(b[0], MicroOp::ArrLoad { .. }),
        "different index must block fusion"
    );
}

/// A jump landing ON the store (or the ALU) means another path enters
/// mid-idiom — fusing would skip the load on that entry. No fusion.
#[test]
fn jump_target_inside_idiom_blocks_fusion() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Add {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::JumpIfTrue { cond: 2, target: 2 }, // jumps onto the store
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut micro, &named8(&[2, 5]), 8, &HashSet::new());
    assert!(
        matches!(micro[0], MicroOp::ArrLoad { .. }),
        "a jump target inside the idiom must block fusion"
    );
}

/// THE HISTOGRAM SHAPE: the index `v + 1` is computed TWICE (separate
/// slots, as the bytecode lowers `Set item (v+1) of A to (item (v+1) of A)
/// + 1`). `copy_propagate`'s value-numbering must merge the two `v + 1` into
/// one slot so the load and store share an index, and only then does
/// `fuse_array_rmw` collapse the idiom. This pins the CSE→fusion handoff.
#[test]
fn duplicated_index_expr_is_cse_d_then_fused() {
    // v=1 (named loop var); c=2 the `1`; 3/5/6 scratch; 8/9 ptr/len.
    let mut micro = vec![
        MicroOp::LoadConst { dst: 2, value: 1 },
        MicroOp::Add {
            dst: 3,
            lhs: 1,
            rhs: 2,
        }, // idx_a = v + 1
        MicroOp::ArrLoad {
            dst: 4,
            idx: 3,
            ptr_slot: 8,
            len_slot: 9,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Add {
            dst: 5,
            lhs: 4,
            rhs: 2,
        }, // t2 = arr[idx_a] + 1
        MicroOp::Add {
            dst: 6,
            lhs: 1,
            rhs: 2,
        }, // idx_b = v + 1  (recomputed)
        MicroOp::ArrStore {
            src: 5,
            idx: 6,
            ptr_slot: 8,
            len_slot: 9,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 1 },
    ];
    let named = named8_n(&[1], 10);
    copy_propagate(&mut micro, &named, 10, &HashSet::new());
    fuse_array_rmw(&mut micro, &named, 10, &HashSet::new());
    // The dead recomputed index is gone; the idiom is one ArrRMW.
    let rmw = micro.iter().find(|op| matches!(op, MicroOp::ArrRMW { .. }));
    match rmw {
        Some(&MicroOp::ArrRMW {
            idx, operand, op, ..
        }) => {
            assert_eq!(op, RmwOp::Add);
            assert_eq!(operand, 2, "operand is the deduped `1`");
            // idx is whichever surviving slot holds v+1 (the load's index).
            assert!(
                idx == 3,
                "load+store must share the CSE'd index slot; got {idx}"
            );
        }
        _ => panic!("CSE+fusion failed to collapse the duplicated-index idiom: {micro:?}"),
    }
    assert!(
        !micro
            .iter()
            .any(|op| matches!(op, MicroOp::ArrStore { .. })),
        "the store must be folded into the ArrRMW"
    );
}

fn named8_n(live: &[u16], n: usize) -> Vec<bool> {
    let mut v = vec![false; n];
    for &s in live {
        v[s as usize] = true;
    }
    v
}

/// FLOAT RMW (nbody `v[i] = v[i] + dx*mag`): an ArrLoad feeding an AddF whose
/// result stores back to the same cell collapses to a float ArrRMW. The
/// operand (the product) stays live; commutative AddF takes `t` on either
/// side, SubF only on the left.
#[test]
fn float_load_addf_store_fuses_to_float_rmw() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 8,
            len_slot: 9,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::AddF {
            dst: 4,
            lhs: 3,
            rhs: 2,
        }, // v[i] + product
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 8,
            len_slot: 9,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut micro, &named8(&[2, 5]), 10, &HashSet::new());
    match micro[0] {
        MicroOp::ArrRMW {
            op: RmwOp::AddF,
            operand: 2,
            idx: 5,
            ..
        } => {}
        ref other => panic!("expected float ArrRMW AddF operand=2, got {other:?}"),
    }

    // operand - v[i] must NOT fuse (loaded value on the right of SubF).
    let mut rev = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 8,
            len_slot: 9,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::SubF {
            dst: 4,
            lhs: 2,
            rhs: 3,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 8,
            len_slot: 9,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 0 },
    ];
    fuse_array_rmw(&mut rev, &named8(&[2, 5]), 10, &HashSet::new());
    assert!(
        matches!(rev[0], MicroOp::ArrLoad { .. }),
        "operand - v[i] must not fuse: {rev:?}"
    );
}

/// FLOAT MULTIPLY-ADD: `MulF(t = a*b); AddF(d = t + c)` with `t` single-use
/// collapses to one FmaF. Commutative — the product may be either AddF
/// operand; the addend `c` stays live.
#[test]
fn mulf_addf_fuses_to_fma() {
    for swap in [false, true] {
        let add = if swap {
            MicroOp::AddF {
                dst: 4,
                lhs: 2,
                rhs: 3,
            } // c + t
        } else {
            MicroOp::AddF {
                dst: 4,
                lhs: 3,
                rhs: 2,
            } // t + c
        };
        let mut micro = vec![
            MicroOp::MulF {
                dst: 3,
                lhs: 0,
                rhs: 1,
            }, // t = a*b
            add,
            MicroOp::Return { src: 4 },
        ];
        fuse_fma(&mut micro, &named8(&[0, 1, 2, 4]), 8, &HashSet::new());
        match micro[0] {
            MicroOp::FmaF {
                dst: 4,
                a: 0,
                b: 1,
                c: 2,
            } => {}
            ref other => panic!("expected FmaF(dst4,a0,b1,c2), got {other:?} (swap={swap})"),
        }
        assert!(
            !micro.iter().any(|op| matches!(op, MicroOp::AddF { .. })),
            "AddF folded away"
        );
    }
}

/// A multi-use product must NOT fuse (the second reader would lose it), and a
/// pinned operand blocks fusion (mem-form FmaF would need a spill).
#[test]
fn fma_blocks_multiuse_product_and_pinned_operands() {
    // product read twice
    let mut multi = vec![
        MicroOp::MulF {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::AddF {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::AddF {
            dst: 5,
            lhs: 3,
            rhs: 2,
        }, // second use of the product
        MicroOp::Return { src: 4 },
    ];
    fuse_fma(&mut multi, &named8(&[0, 1, 2, 4, 5]), 8, &HashSet::new());
    assert!(
        matches!(multi[0], MicroOp::MulF { .. }),
        "multi-use product must block FMA"
    );

    // a pinned operand (slot 0) blocks the spill-free fusion
    let mut pinned_in = vec![
        MicroOp::MulF {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::AddF {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::Return { src: 4 },
    ];
    let pins: HashSet<u16> = [0u16].into_iter().collect();
    fuse_fma(&mut pinned_in, &named8(&[0, 1, 2, 4]), 8, &pins);
    assert!(
        matches!(pinned_in[0], MicroOp::MulF { .. }),
        "pinned operand must block FMA"
    );
}

/// LEVER 1a — a pinned DESTINATION `d` (the accumulator/result XMM pin) must
/// NOT block fusion: `FmaF` is mem-form, so a pinned `d` is spilled/reloaded
/// by `compile_straightline_pinned_float`'s mem-form path exactly as it would
/// be by the unfused `AddF`'s register-form result — fusing adds no extra
/// spill on the INPUTS (which stay frame-resident). The nbody distance-sum's
/// `MulF(t = dz*dz); AddF(d = sum + t)` with `d` float-pinned is the case the
/// guard previously refused. Inputs `a`, `b`, the addend `c` stay frame-resident.
#[test]
fn fma_fuses_with_pinned_destination() {
    for swap in [false, true] {
        let add = if swap {
            MicroOp::AddF {
                dst: 4,
                lhs: 2,
                rhs: 3,
            } // c + t
        } else {
            MicroOp::AddF {
                dst: 4,
                lhs: 3,
                rhs: 2,
            } // t + c
        };
        let mut micro = vec![
            MicroOp::MulF {
                dst: 3,
                lhs: 0,
                rhs: 1,
            }, // t = a*b (a,b,t frame-resident)
            add, // d = t + c, d pinned
            MicroOp::Return { src: 4 },
        ];
        let pins: HashSet<u16> = [4u16].into_iter().collect(); // ONLY the dst is pinned
        fuse_fma(&mut micro, &named8(&[0, 1, 2, 4]), 8, &pins);
        match micro[0] {
            MicroOp::FmaF {
                dst: 4,
                a: 0,
                b: 1,
                c: 2,
            } => {}
            ref other => {
                panic!("expected FmaF(dst4,a0,b1,c2) with pinned dst, got {other:?} (swap={swap})")
            }
        }
        assert!(
            !micro.iter().any(|op| matches!(op, MicroOp::AddF { .. })),
            "AddF must fold into the FmaF even when the dst is pinned"
        );
    }
}

/// LEVER 1a soundness boundary — a pinned ADDEND `c` (the loop-carried
/// accumulator read on the FmaF's frame-input side, the `s = s + a*b` shape
/// where `c == d == s`) must STILL block: spilling the pinned accumulator
/// around the mem-form FmaF would cost more pieces than the register-form
/// `AddF` it replaces. Only a pinned `d` that is NOT also an input is relaxed.
#[test]
fn fma_pinned_addend_still_blocks() {
    // accumulator shape: t = a*b ; s = s + t  (c == d == slot 4, pinned)
    let mut micro = vec![
        MicroOp::MulF {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::AddF {
            dst: 4,
            lhs: 4,
            rhs: 3,
        }, // s = s + t, s is both c and d
        MicroOp::Return { src: 4 },
    ];
    let pins: HashSet<u16> = [4u16].into_iter().collect();
    fuse_fma(&mut micro, &named8(&[0, 1, 4]), 8, &pins);
    assert!(
        matches!(micro[0], MicroOp::MulF { .. }),
        "a pinned addend (loop-carried accumulator) must block FMA"
    );
}

/// Jump targets AFTER the idiom are remapped down by the two removed ops.
#[test]
fn fusion_remaps_later_jump_targets() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Add {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::JumpIfTrue { cond: 2, target: 5 }, // -> the Return
        MicroOp::Add {
            dst: 1,
            lhs: 1,
            rhs: 2,
        },
        MicroOp::Return { src: 1 },
    ];
    fuse_array_rmw(&mut micro, &named8(&[1, 2, 5]), 8, &HashSet::new());
    // [ArrRMW, JumpIfTrue->3, Add, Return] — the target slid 5 -> 3.
    assert!(matches!(micro[0], MicroOp::ArrRMW { .. }));
    match micro[1] {
        MicroOp::JumpIfTrue { target, .. } => assert_eq!(target, 3, "target must remap 5->3"),
        ref other => panic!("expected JumpIfTrue, got {other:?}"),
    }
    assert!(matches!(micro[3], MicroOp::Return { .. }));
}
