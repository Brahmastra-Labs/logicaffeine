use super::*;
use logicaffeine_forge::jit::IOp;
use std::collections::HashSet;

fn named_n(live: &[u16], n: usize) -> Vec<bool> {
    let mut v = vec![false; n];
    for &s in live {
        v[s as usize] = true;
    }
    v
}

/// THE MATMUL DOT-PRODUCT SHAPE: `ArrLoad(t1 = a[ix]); ArrLoad(t2 = b[jx]);
/// Mul(d = t1 * t2)` — the two elements come from TWO DISTINCT pinned int
/// buffers (a at ptr 10/len 11, b at ptr 12/len 13). The two single-use
/// scratch loads collapse into ONE fused ArrLoad2 (two loads + multiply,
/// three dispatches → one), so the loaded i64s never round-trip the frame.
#[test]
fn two_buffer_load_mul_fuses_to_arrld2() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 1,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrLoad {
            dst: 4,
            idx: 2,
            ptr_slot: 12,
            len_slot: 13,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Mul {
            dst: 5,
            lhs: 3,
            rhs: 4,
        },
        MicroOp::Return { src: 5 },
    ];
    fuse_array_ld2(&mut micro, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
    assert_eq!(
        micro.len(),
        2,
        "the 3-op dot-product idiom must collapse to 1 op + Return"
    );
    match micro[0] {
        MicroOp::ArrLoad2 {
            dst,
            i,
            j,
            ptr_a,
            len_a,
            ptr_b,
            len_b,
            op,
            checked,
        } => {
            assert_eq!((dst, i, j), (5, 1, 2));
            assert_eq!((ptr_a, len_a, ptr_b, len_b), (10, 11, 12, 13));
            assert_eq!(op, IOp::Mul);
            assert!(checked);
        }
        ref other => panic!("expected ArrLoad2, got {other:?}"),
    }
}

/// Add commutes — the loads may feed the binop in either order, and the
/// fused op records the index order that reproduces `a[i] OP b[j]`.
#[test]
fn add_fuses_in_either_operand_order() {
    for swap in [false, true] {
        let add = if swap {
            MicroOp::Add {
                dst: 5,
                lhs: 4,
                rhs: 3,
            } // b[j] + a[i]
        } else {
            MicroOp::Add {
                dst: 5,
                lhs: 3,
                rhs: 4,
            } // a[i] + b[j]
        };
        let mut micro = vec![
            MicroOp::ArrLoad {
                dst: 3,
                idx: 1,
                ptr_slot: 10,
                len_slot: 11,
                byte: false,
                narrow32: false,
                checked: false,
            },
            MicroOp::ArrLoad {
                dst: 4,
                idx: 2,
                ptr_slot: 12,
                len_slot: 13,
                byte: false,
                narrow32: false,
                checked: false,
            },
            add,
            MicroOp::Return { src: 5 },
        ];
        fuse_array_ld2(&mut micro, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
        match micro[0] {
            MicroOp::ArrLoad2 {
                op: IOp::Add,
                checked: false,
                ..
            } => {}
            ref other => panic!("expected unchecked ArrLoad2 Add (swap={swap}), got {other:?}"),
        }
    }
}

/// Subtraction is non-commutative, but the fusion is anchored on the binop
/// and tracks WHICH load feeds the left vs. right operand, so it fuses BOTH
/// orderings correctly by recording the left load as `a`/`i` and the right
/// load as `b`/`j` (the stencil computes `a[i] - b[j]`).
#[test]
fn sub_fuses_in_binop_order() {
    // `X[1] - Y[2]`: left load = X (ptr 10), right load = Y (ptr 12).
    let mut fwd = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 1,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrLoad {
            dst: 4,
            idx: 2,
            ptr_slot: 12,
            len_slot: 13,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Sub {
            dst: 5,
            lhs: 3,
            rhs: 4,
        }, // X[1] - Y[2]
        MicroOp::Return { src: 5 },
    ];
    fuse_array_ld2(&mut fwd, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
    match fwd[0] {
        MicroOp::ArrLoad2 {
            op: IOp::Sub,
            i: 1,
            j: 2,
            ptr_a: 10,
            ptr_b: 12,
            ..
        } => {}
        ref other => {
            panic!("expected ArrLoad2 Sub (X-Y): i=1 j=2 ptr_a=10 ptr_b=12, got {other:?}")
        }
    }

    // `Y[2] - X[1]`: left load = Y (ptr 12), right load = X (ptr 10). The
    // fused op must SWAP which buffer is `a`/`b` so it still computes Y - X.
    let mut rev = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 1,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrLoad {
            dst: 4,
            idx: 2,
            ptr_slot: 12,
            len_slot: 13,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Sub {
            dst: 5,
            lhs: 4,
            rhs: 3,
        }, // Y[2] - X[1]
        MicroOp::Return { src: 5 },
    ];
    fuse_array_ld2(&mut rev, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
    match rev[0] {
        MicroOp::ArrLoad2 {
            op: IOp::Sub,
            i: 2,
            j: 1,
            ptr_a: 12,
            ptr_b: 10,
            ..
        } => {}
        ref other => {
            panic!("expected ArrLoad2 Sub (Y-X): i=2 j=1 ptr_a=12 ptr_b=10, got {other:?}")
        }
    }
}

/// A byte (Bool) buffer is not an 8-byte int load — the fused int stencil
/// reads raw i64s, so a byte load must never fuse.
#[test]
fn byte_buffer_blocks_fusion() {
    let mut micro = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 1,
            ptr_slot: 10,
            len_slot: 11,
            byte: true,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrLoad {
            dst: 4,
            idx: 2,
            ptr_slot: 12,
            len_slot: 13,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Mul {
            dst: 5,
            lhs: 3,
            rhs: 4,
        },
        MicroOp::Return { src: 5 },
    ];
    fuse_array_ld2(&mut micro, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
    assert!(
        matches!(micro[0], MicroOp::ArrLoad { byte: true, .. }),
        "byte load must block fusion"
    );
}

/// A multi-use loaded value must NOT be folded away (a later reader would
/// lose it), and a jump landing on the second load or the binop blocks it.
#[test]
fn multiuse_load_and_inner_jump_target_block_fusion() {
    let mut multi = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 1,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrLoad {
            dst: 4,
            idx: 2,
            ptr_slot: 12,
            len_slot: 13,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Mul {
            dst: 5,
            lhs: 3,
            rhs: 4,
        },
        MicroOp::Move { dst: 6, src: 3 }, // second use of a[i]
        MicroOp::Return { src: 5 },
    ];
    fuse_array_ld2(&mut multi, &named_n(&[1, 2, 5, 6], 14), 14, &HashSet::new());
    assert!(
        matches!(multi[0], MicroOp::ArrLoad { .. }),
        "multi-use load must block fusion"
    );

    let mut jt = vec![
        MicroOp::ArrLoad {
            dst: 3,
            idx: 1,
            ptr_slot: 10,
            len_slot: 11,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrLoad {
            dst: 4,
            idx: 2,
            ptr_slot: 12,
            len_slot: 13,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Mul {
            dst: 5,
            lhs: 3,
            rhs: 4,
        },
        MicroOp::JumpIfTrue { cond: 0, target: 1 }, // lands on the 2nd load
        MicroOp::Return { src: 5 },
    ];
    fuse_array_ld2(&mut jt, &named_n(&[0, 1, 2, 5], 14), 14, &HashSet::new());
    assert!(
        matches!(jt[0], MicroOp::ArrLoad { .. }),
        "inner jump target must block fusion"
    );
}
