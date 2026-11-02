use super::*;
use crate::codegen_sva::unified_placement::{exec_dram_ambit, exec_reference, Prim, Row, PRIMS};

fn desc_dram_ambit(kind: PrimKind) -> bool {
    PRIMS
        .iter()
        .find(|d| d.kind == kind)
        .expect("kind in PRIMS")
        .dram_ambit
}

/// Every OnPlane op maps to a DRAM-tier primitive whose in-DRAM execution equals the VM
/// reference over fuzzed rows — the ≡VM heart of the coverage ratchet, self-contained here.
#[test]
fn every_onplane_op_has_a_computedram_realization_eq_vm() {
    let mut x = 0x1234_5678_9abc_def0u64;
    let mut rng = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut saw_onplane = false;
    for op in Op::all_variants() {
        match fragment(&op) {
            DramFragment::OnPlane => {
                saw_onplane = true;
                let kind = dram_prim(&op)
                    .unwrap_or_else(|| panic!("OnPlane op `{}` has no dram_prim", op.name()));
                assert!(
                    desc_dram_ambit(kind),
                    "OnPlane op `{}` maps to a non-DRAM primitive {kind:?}",
                    op.name()
                );
                assert!(
                    dram_tier(&op).is_some(),
                    "OnPlane op `{}` has no dram_tier",
                    op.name()
                );
                // Behavioral ≡VM: in-DRAM result == VM reference over a fuzzed 8-word operand.
                let a: Row = (0..8).map(|_| rng()).collect();
                let b: Row = (0..8).map(|_| rng()).collect();
                let prim = match kind {
                    PrimKind::And => Prim::And { dst: 2, a: 0, b: 1 },
                    PrimKind::Or => Prim::Or { dst: 2, a: 0, b: 1 },
                    PrimKind::Xor => Prim::Xor { dst: 2, a: 0, b: 1 },
                    PrimKind::Copy => Prim::Copy { dst: 2, src: 0 },
                    other => panic!("OnPlane op `{}` unexpected prim {other:?}", op.name()),
                };
                let mut heap_d = vec![a.clone(), b.clone(), vec![0u64; 8]];
                let mut heap_r = heap_d.clone();
                exec_dram_ambit(&mut heap_d, &prim).expect("DRAM realizes an OnPlane prim");
                exec_reference(&mut heap_r, &prim);
                assert_eq!(
                    heap_d[2],
                    heap_r[2],
                    "in-DRAM `{}` ({kind:?}) diverged from the VM reference",
                    op.name()
                );
            }
            _ => {
                assert!(
                    dram_prim(&op).is_none(),
                    "non-OnPlane op `{}` has a dram_prim (biconditional broken)",
                    op.name()
                );
                assert!(
                    dram_tier(&op).is_none(),
                    "non-OnPlane op `{}` has a dram_tier (biconditional broken)",
                    op.name()
                );
            }
        }
    }
    assert!(
        saw_onplane,
        "no OnPlane op — the DRAM tier realizes nothing (vacuous)"
    );
}
