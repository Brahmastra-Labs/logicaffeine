use super::*;

fn lcg(seed: u64) -> impl FnMut() -> u64 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

/// Every set operation's DRAM dataflow kernel equals its set-theoretic reference over fuzzed
/// bitset rows — the complete finite set algebra (∩ ∪ \ △ ¬) proven ≡ in-DRAM.
#[test]
fn every_set_op_lowers_to_a_dram_kernel_eq_reference() {
    let width = 8; // 512-bit universe
    let mut rng = lcg(0x5e7a_1_9eb_1234_5678);
    for _ in 0..40 {
        let a: Row = (0..width).map(|_| rng()).collect();
        let b: Row = (0..width).map(|_| rng()).collect();
        for &op in SetOp::ALL {
            let mut df = DramDataflow::new(2);
            let out = lower(op, &mut df, 0, 1);
            let heap = df.run(&[a.clone(), b.clone()], width);
            assert_eq!(
                heap[out],
                reference(op, &a, &b),
                "set op {op:?} diverged from reference"
            );
        }
    }
}

/// `recognize` maps exactly the VM's first-class set ops and nothing else.
#[test]
fn recognize_maps_the_vm_set_ops() {
    assert_eq!(
        recognize(&Op::IntersectOp {
            dst: 0,
            lhs: 1,
            rhs: 2
        }),
        Some(SetOp::Intersection)
    );
    assert_eq!(
        recognize(&Op::UnionOp {
            dst: 0,
            lhs: 1,
            rhs: 2
        }),
        Some(SetOp::Union)
    );
    // A non-set op is not recognized.
    assert_eq!(
        recognize(&Op::Add {
            dst: 0,
            lhs: 1,
            rhs: 2
        }),
        None
    );
    // Difference/symmetric_difference/complement have no VM ISA op yet.
    assert_eq!(
        recognize(&Op::BitXor {
            dst: 0,
            lhs: 1,
            rhs: 2
        }),
        None
    );
}

/// The recognized VM set ops lower to the SAME in-DRAM result as their single-op `dram_prim`
/// realization — the set-algebra library and the op-coverage ratchet agree.
#[test]
fn recognized_set_ops_agree_with_dram_prim() {
    use crate::codegen_dram::dram_prim;
    use crate::codegen_sva::unified_placement::{exec_dram_ambit, Prim, PrimKind};
    let mut rng = lcg(0xa9ac_0011_2233_4455);
    for _ in 0..40 {
        let a: Row = vec![rng()];
        let b: Row = vec![rng()];
        for op in [
            Op::IntersectOp {
                dst: 2,
                lhs: 0,
                rhs: 1,
            },
            Op::UnionOp {
                dst: 2,
                lhs: 0,
                rhs: 1,
            },
        ] {
            let set_op = recognize(&op).expect("recognized set op");
            // set-algebra dataflow lowering
            let mut df = DramDataflow::new(2);
            let out = lower(set_op, &mut df, 0, 1);
            let heap = df.run(&[a.clone(), b.clone()], 1);
            // single-op dram_prim realization
            let kind = dram_prim(&op).expect("dram_prim for a set op");
            let prim = match kind {
                PrimKind::And => Prim::And { dst: 2, a: 0, b: 1 },
                PrimKind::Or => Prim::Or { dst: 2, a: 0, b: 1 },
                other => panic!("unexpected prim {other:?}"),
            };
            let mut hp = vec![a.clone(), b.clone(), vec![0u64]];
            exec_dram_ambit(&mut hp, &prim).expect("realizable");
            assert_eq!(
                heap[out], hp[2],
                "set-algebra kernel disagrees with dram_prim for {op:?}"
            );
        }
    }
}
