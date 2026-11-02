use super::*;
use logicaffeine_forge::jit::AffOp;
use std::collections::HashSet;

// slots 0..n are registers; named marks the live ones. The const-holding
// slot is left live (it is reused), the index temps are scratch (un-named).
fn named_n(live: &[u16], n: usize) -> Vec<bool> {
    let mut v = vec![false; n];
    for &s in live {
        v[s as usize] = true;
    }
    v
}

/// THE `w + 1` SHAPE — `LoadConst(kc = 1); Add(idx = w + kc); ArrLoad(arr[idx])`.
/// The index `idx` is single-use scratch, the const slot is reused; folds to
/// one `ArrLoadAffine{op: None, a: w, const_offset: 1}` and the `Add` vanishes
/// while the `LoadConst` survives.
#[test]
fn slot_plus_const_folds_to_affine_none() {
    // slot 4 = w (live index var), 8 = kc (live const), 5 = idx (scratch),
    // 6 = loaded value, 10/11 = ptr/len.
    let mut micro = vec![
        MicroOp::LoadConst { dst: 8, value: 1 },
        MicroOp::Add {
            dst: 5,
            lhs: 4,
            rhs: 8,
        },
        MicroOp::ArrLoad {
            dst: 6,
            idx: 5,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 6 },
    ];
    fuse_index_affine(&mut micro, &named_n(&[4, 8, 6], 12), 12, &HashSet::new());
    assert_eq!(micro.len(), 3, "Add folds into the load; LoadConst kept");
    match micro[1] {
        MicroOp::ArrLoadAffine {
            dst,
            a,
            op,
            const_offset,
            ptr_slot,
            len_slot,
            checked,
            ..
        } => {
            assert_eq!((dst, a, op, const_offset), (6, 4, AffOp::None, 1));
            assert_eq!((ptr_slot, len_slot, checked), (10, 11, true));
        }
        ref other => panic!("expected ArrLoadAffine None, got {other:?}"),
    }
    assert!(
        matches!(micro[0], MicroOp::LoadConst { dst: 8, value: 1 }),
        "shared const kept"
    );
}

/// THE `w - wi + 1` SHAPE — `Sub(t = w - wi); LoadConst(kc = 1); Add(idx = t + kc);
/// ArrLoad(arr[idx])`. Both `t` and `idx` are single-use scratch; folds to one
/// `ArrLoadAffine{op: Sub, a: w, b: wi, const_offset: 1}` (the inner Sub AND the
/// trailing Add vanish, the LoadConst survives).
#[test]
fn sub_plus_const_folds_to_affine_sub() {
    // 4 = w, 3 = wi, 8 = kc (live), 7 = t (scratch), 5 = idx (scratch).
    let mut micro = vec![
        MicroOp::Sub {
            dst: 7,
            lhs: 4,
            rhs: 3,
        },
        MicroOp::LoadConst { dst: 8, value: 1 },
        MicroOp::Add {
            dst: 5,
            lhs: 7,
            rhs: 8,
        },
        MicroOp::ArrLoad {
            dst: 6,
            idx: 5,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::Return { src: 6 },
    ];
    fuse_index_affine(&mut micro, &named_n(&[4, 3, 8, 6], 12), 12, &HashSet::new());
    assert_eq!(
        micro.len(),
        3,
        "Sub + Add fold into the load; LoadConst kept"
    );
    match micro[1] {
        MicroOp::ArrLoadAffine {
            dst,
            a,
            op,
            b,
            const_offset,
            checked,
            ..
        } => {
            assert_eq!(
                (dst, a, op, b, const_offset, checked),
                (6, 4, AffOp::Sub, 3, 1, false)
            );
        }
        ref other => panic!("expected ArrLoadAffine Sub, got {other:?}"),
    }
}

/// THE `i*n + j + 1` MATMUL READ SHAPE — `Mul(t1 = i*n)` is MULTI-USE (the
/// matmul reuses `i*n`), so it must NOT be removed, but the trailing
/// `Add(t2 = t1 + j); Add(idx = t2 + kc); ArrLoad` still folds: the inner
/// add `t2` is single-use scratch over the slot `t1`, giving
/// `ArrLoadAffine{op: Add, a: t1, b: j, const_offset: 1}`.
#[test]
fn matmul_read_folds_keeping_shared_mul() {
    // 6 = i, 1 = n, 14 = j, 8 = kc (live), 18 = t1 = i*n (MULTI-USE: read by
    // the inner add AND a later op), 17 = t2 (scratch), 16 = idx (scratch).
    let mut micro = vec![
        MicroOp::Mul {
            dst: 18,
            lhs: 6,
            rhs: 1,
        }, // i*n, reused
        MicroOp::Add {
            dst: 17,
            lhs: 18,
            rhs: 14,
        }, // i*n + j  (scratch)
        MicroOp::LoadConst { dst: 8, value: 1 },
        MicroOp::Add {
            dst: 16,
            lhs: 17,
            rhs: 8,
        }, // + 1      (scratch)
        MicroOp::ArrLoad {
            dst: 23,
            idx: 16,
            ptr_slot: 40,
            len_slot: 41,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Move { dst: 99, src: 18 }, // second use of i*n
        MicroOp::Return { src: 23 },
    ];
    fuse_index_affine(
        &mut micro,
        &named_n(&[6, 1, 14, 8, 23, 99], 100),
        100,
        &HashSet::new(),
    );
    // The two inner Adds collapse into the load; the Mul and LoadConst stay.
    match micro
        .iter()
        .find(|o| matches!(o, MicroOp::ArrLoadAffine { .. }))
    {
        Some(MicroOp::ArrLoadAffine {
            dst,
            a,
            op,
            b,
            const_offset,
            ..
        }) => {
            assert_eq!(
                (*dst, *a, *op, *b, *const_offset),
                (23, 18, AffOp::Add, 14, 1)
            );
        }
        other => panic!("expected ArrLoadAffine, got {other:?}"),
    }
    assert!(
        micro
            .iter()
            .any(|o| matches!(o, MicroOp::Mul { dst: 18, .. })),
        "shared i*n Mul kept"
    );
}

/// THE BARE `a + b` SHAPE (no const) — `Add(idx = a + b); ArrLoad` folds to
/// `ArrLoadAffine{op: Add, const_offset: 0}`.
#[test]
fn bare_two_slot_add_folds_with_zero_offset() {
    let mut micro = vec![
        MicroOp::Add {
            dst: 5,
            lhs: 4,
            rhs: 3,
        },
        MicroOp::ArrLoad {
            dst: 6,
            idx: 5,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 6 },
    ];
    fuse_index_affine(&mut micro, &named_n(&[4, 3, 6], 12), 12, &HashSet::new());
    assert_eq!(micro.len(), 2, "the Add folds into the load");
    match micro[0] {
        MicroOp::ArrLoadAffine {
            a,
            op,
            b,
            const_offset,
            ..
        } => {
            assert_eq!((a, op, b, const_offset), (4, AffOp::Add, 3, 0));
        }
        ref other => panic!("expected ArrLoadAffine Add, got {other:?}"),
    }
}

/// A MULTI-USE INDEX (the slot is read by a store too, as matmul's `c[idx]`)
/// must NOT fold — folding would drop the index the store still needs.
#[test]
fn multiuse_index_blocks_fusion() {
    let mut micro = vec![
        MicroOp::LoadConst { dst: 8, value: 1 },
        MicroOp::Add {
            dst: 5,
            lhs: 4,
            rhs: 8,
        },
        MicroOp::ArrLoad {
            dst: 6,
            idx: 5,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrStore {
            src: 6,
            idx: 5,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 6 },
    ];
    fuse_index_affine(&mut micro, &named_n(&[4, 8, 6], 12), 12, &HashSet::new());
    assert!(
        matches!(micro[2], MicroOp::ArrLoad { .. }),
        "an index reused by a store must block the fold: {micro:?}"
    );
}

/// A BYTE (Bool) load never folds — the affine stencils read raw i64s.
#[test]
fn byte_load_blocks_fusion() {
    let mut micro = vec![
        MicroOp::LoadConst { dst: 8, value: 1 },
        MicroOp::Add {
            dst: 5,
            lhs: 4,
            rhs: 8,
        },
        MicroOp::ArrLoad {
            dst: 6,
            idx: 5,
            ptr_slot: 10,
            len_slot: 11,
            byte: true,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 6 },
    ];
    fuse_index_affine(&mut micro, &named_n(&[4, 8, 6], 12), 12, &HashSet::new());
    assert!(
        matches!(micro[2], MicroOp::ArrLoad { byte: true, .. }),
        "byte load must not fold"
    );
}

/// A PINNED index temp must not fold (the fused op reads it from the frame;
/// folding past a pin would lose the pinned value's settling).
#[test]
fn pinned_index_temp_blocks_fusion() {
    let mut micro = vec![
        MicroOp::Add {
            dst: 5,
            lhs: 4,
            rhs: 3,
        },
        MicroOp::ArrLoad {
            dst: 6,
            idx: 5,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 6 },
    ];
    let mut pins = HashSet::new();
    pins.insert(5u16);
    fuse_index_affine(&mut micro, &named_n(&[4, 3, 6], 12), 12, &pins);
    assert!(
        matches!(micro[0], MicroOp::Add { .. }),
        "a pinned index must block the fold"
    );
}

/// A jump landing on the load (or between the index op and the load) must
/// block the fold — an entry there would skip the index computation.
#[test]
fn jump_target_on_load_blocks_fusion() {
    let mut micro = vec![
        MicroOp::Add {
            dst: 5,
            lhs: 4,
            rhs: 3,
        },
        MicroOp::ArrLoad {
            dst: 6,
            idx: 5,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::JumpIfTrue { cond: 4, target: 1 }, // lands on the load
        MicroOp::Return { src: 6 },
    ];
    fuse_index_affine(&mut micro, &named_n(&[4, 3, 6], 12), 12, &HashSet::new());
    assert!(
        matches!(micro[0], MicroOp::Add { .. }),
        "jump onto the load blocks the fold"
    );
}

/// COND-SWAP WITH A COMPUTED SECOND INDEX (Priority 2) — the sort inner-loop
/// compare-and-swap idiom `ArrLoad(a, i1); <index arith for i2>; ArrLoad(b, i2);
/// Branch(cmp, a, b); ArrStore(i1, b); ArrStore(i2, a)` must still fuse into one
/// `ArrCondSwap` when `Mul`/`Add`/`LoadConst` (the `2*root+2`-style index
/// computation of the SECOND access) sit BETWEEN the two loads. The
/// between-ops are pure and redefine neither the first loaded value, the
/// first index, nor the pointer/length, so they do not block the fusion.
#[test]
fn cond_swap_tolerates_computed_index_between_loads() {
    // i1 = slot 5 (live), root = slot 4 (live). i2 = 2*root + 2 computed
    // between the loads (slots 8 = kc2, 7 = 2*root, 6 = i2; all scratch).
    // a = slot 9, b = slot 10 (the two loaded values, dead after the swap).
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 9,
            idx: 5,
            ptr_slot: 20,
            len_slot: 21,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::LoadConst { dst: 8, value: 2 },
        MicroOp::Mul {
            dst: 7,
            lhs: 4,
            rhs: 8,
        }, // 2*root
        MicroOp::Add {
            dst: 6,
            lhs: 7,
            rhs: 8,
        }, // 2*root + 2  (the i2 index)
        MicroOp::ArrLoad {
            dst: 10,
            idx: 6,
            ptr_slot: 20,
            len_slot: 21,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 9,
            rhs: 10,
            target: 8,
        },
        MicroOp::ArrStore {
            src: 10,
            idx: 5,
            ptr_slot: 20,
            len_slot: 21,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrStore {
            src: 9,
            idx: 6,
            ptr_slot: 20,
            len_slot: 21,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 0 },
    ];
    // gap from load1 to load2 is 3 (LoadConst, Mul, Add) — raise the search
    // window by running with the same named/pin config the pipeline uses.
    fuse_cond_swap(&mut micro, &named_n(&[4, 5], 22), 22, &HashSet::new());
    assert!(
        micro.iter().any(|o| matches!(
            o,
            MicroOp::ArrCondSwap {
                idx1: 5,
                idx2: 6,
                ..
            }
        )),
        "the compare-and-swap must fuse despite the 2*root+2 index arith between \
             the loads: {micro:?}"
    );
}

/// EVAL PARITY — `AffOp::eval` reproduces the kernel's wrapping i64 index
/// arithmetic exactly (the differential gate's bit-for-bit contract).
#[test]
fn affop_eval_matches_wrapping_kernel_semantics() {
    assert_eq!(AffOp::None.eval(7, 999, 1), 8);
    assert_eq!(AffOp::Add.eval(40, 3, 1), 44);
    assert_eq!(AffOp::Sub.eval(10, 4, 1), 7);
    assert_eq!(AffOp::Mul.eval(6, 7, 1), 43);
    // Wrapping at the i64 edges, identical to the kernel.
    assert_eq!(AffOp::Add.eval(i64::MAX, 1, 0), i64::MAX.wrapping_add(1));
    assert_eq!(AffOp::Mul.eval(i64::MIN, -1, 0), i64::MIN.wrapping_mul(-1));
    assert_eq!(AffOp::Sub.eval(i64::MIN, 1, 0), i64::MIN.wrapping_sub(1));
}
