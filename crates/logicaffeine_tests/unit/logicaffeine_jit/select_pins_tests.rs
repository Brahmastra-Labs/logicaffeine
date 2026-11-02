use super::*;

/// A slot used as BOTH an f64 (a `MulF` result, float-valued) AND an i64
/// (an `ArrStore` index, integer-valued) is type-reused scratch. It must
/// NEVER be float-pinned: the mem-form `ArrStore` spills the float pin to
/// the frame cell, overwriting the integer index the `Add` wrote, so the
/// store indexes through an f64 bit-pattern → a wild write. This was the
/// spectral_norm SIGSEGV. The integer-role classification in `select_pins`
/// must keep the mixed slot out of the float-pin budget (frame-resident).
#[test]
fn float_result_reused_as_array_index_is_not_float_pinned() {
    // register_count = 8 → slots 0..7 are registers; 8/9 stand in for the
    // pinned array's ptr/len cells. Slot 5 is the type-reused cell.
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 6,
        },
        MicroOp::MulF {
            dst: 5,
            lhs: 2,
            rhs: 3,
        }, // slot 5 holds an f64 …
        MicroOp::AddF {
            dst: 4,
            lhs: 4,
            rhs: 5,
        }, // … consumed as a float
        MicroOp::Add {
            dst: 5,
            lhs: 0,
            rhs: 1,
        }, // slot 5 reused as an i64 index
        MicroOp::ArrStore {
            src: 4,
            idx: 5,
            ptr_slot: 8,
            len_slot: 9,
            byte: false,
            narrow32: false,
            checked: false,
        },
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 0 },
    ];
    let (_gp, fpins) = select_pins(&ops, 8, true);
    assert!(
        !fpins.contains(&5),
        "slot 5 (an f64 result reused as an integer array index) must NOT be \
             float-pinned — pinning it spills f64 bits over the integer index; got fpins={fpins:?}"
    );
}

/// LEVER 3 — a single-use float partial-sum that is the DESTINATION of an
/// `AddF` consuming an immediately-preceding single-use `MulF` product (the
/// nbody distance-sum's `partial = a*a + (b*b)` shape, `d` distinct from its
/// addend) is the result an FmaF will absorb. It must be KEPT OUT of the
/// float-pin budget so the (later) `fuse_fma` writes the frame directly with
/// NO reload — a 2-pieces-to-1 win — instead of pinning it and paying a reload
/// after the mem-form FmaF.
#[test]
fn fma_result_temp_is_not_float_pinned() {
    // partial = aa + b*b, where aa is a frame temp and b*b is single-use.
    // slot 4 = partial (the FmaF dst); slot 6 = b*b product (single-use);
    // slot 5 = aa (the addend, distinct from 4); slot 7 reads partial after.
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 6,
        },
        MicroOp::MulF {
            dst: 6,
            lhs: 2,
            rhs: 2,
        }, // b*b → single-use product
        MicroOp::AddF {
            dst: 4,
            lhs: 5,
            rhs: 6,
        }, // partial = aa + (b*b), 4 != 5
        MicroOp::SqrtF { dst: 7, src: 4 }, // partial's only reader
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 0 },
    ];
    let (_gp, fpins) = select_pins(&ops, 8, true);
    assert!(
        !fpins.contains(&4),
        "slot 4 (a single-use FmaF-candidate partial-sum, distinct from its \
             addend) must NOT be float-pinned (Lever 3); got fpins={fpins:?}"
    );
}

/// LEVER 3 boundary — a LOOP-CARRIED accumulator `s = s + a*b` (`c == d == s`)
/// is NOT an FmaF candidate (Lever 1a refuses to fuse a pinned addend), so
/// Lever 3 must NOT exclude it from the float-pin budget. (The accumulator is
/// read+written by the AddF as a float; the region returns an unrelated int so
/// the `Return`-blocks-float rule does not interfere with the assertion.)
#[test]
fn loop_carried_float_accumulator_not_excluded_by_lever3() {
    // s = s + a*b: slot 4 = s (read + written by the AddF), 6 = a*b product.
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 4,
        },
        MicroOp::MulF {
            dst: 6,
            lhs: 2,
            rhs: 3,
        }, // a*b → product
        MicroOp::AddF {
            dst: 4,
            lhs: 4,
            rhs: 6,
        }, // s = s + product (c == d == 4)
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 0 }, // returns an int, not the accumulator
    ];
    let (_gp, fpins) = select_pins(&ops, 8, true);
    assert!(
        fpins.contains(&4),
        "slot 4 (a loop-carried float accumulator, c == d) must STILL be \
             float-pin-eligible — it is not an FmaF candidate, so Lever 3 must not \
             exclude it; got fpins={fpins:?}"
    );
}
