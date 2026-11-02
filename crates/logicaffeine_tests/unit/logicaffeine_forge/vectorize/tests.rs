use super::*;

// `c[i] = a[i] + b[i]` — the canonical map. Slots: i=0, n=1, a=2, b=3, c=4,
// alen=5, blen=6, clen=7, ta=8, tb=9, tc=10, one=11.
fn load(dst: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot) -> MicroOp {
    MicroOp::ArrLoad {
        dst,
        idx,
        ptr_slot,
        len_slot,
        byte: false,
        narrow32: false,
        checked: false,
    }
}
fn store(src: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot) -> MicroOp {
    MicroOp::ArrStore {
        src,
        idx,
        ptr_slot,
        len_slot,
        byte: false,
        narrow32: false,
        checked: false,
    }
}
fn load_checked(dst: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot) -> MicroOp {
    MicroOp::ArrLoad {
        dst,
        idx,
        ptr_slot,
        len_slot,
        byte: false,
        narrow32: false,
        checked: true,
    }
}

// Wrap a load/compute/store `body` in the top-tested while-loop shape the
// recognizer expects: guard / body / LoadConst(step) / increment / back-Jump /
// Return. Induction = slot 0, limit = slot 1, step slot = 90.
fn loop_region(body: Vec<MicroOp>, cmp: Cmp, step_val: i64) -> Vec<MicroOp> {
    let mut ops = vec![MicroOp::Branch {
        cmp,
        lhs: 0,
        rhs: 1,
        target: 0,
    }];
    ops.extend(body);
    ops.push(MicroOp::LoadConst {
        dst: 90,
        value: step_val,
    });
    ops.push(MicroOp::Add {
        dst: 0,
        lhs: 0,
        rhs: 90,
    });
    let jump_idx = ops.len();
    ops.push(MicroOp::Jump { target: 0 });
    ops.push(MicroOp::Return { src: 0 });
    if let MicroOp::Branch { target, .. } = &mut ops[0] {
        *target = jump_idx + 1; // exit -> the Return
    }
    ops
}

fn map_add_region() -> Vec<MicroOp> {
    loop_region(
        vec![
            load(8, 0, 2, 5),
            load(9, 0, 3, 6),
            MicroOp::AddF {
                dst: 10,
                lhs: 8,
                rhs: 9,
            },
            store(10, 0, 4, 7),
        ],
        Cmp::LtEq,
        1,
    )
}

#[test]
fn recognizes_elementwise_add_map() {
    let ops = map_add_region();
    let plan = recognize_elementwise_map(&ops).expect("c[i]=a[i]+b[i] is a map");
    assert_eq!(plan.induction, 0);
    assert_eq!(plan.limit, 1);
    assert_eq!(plan.cmp, Cmp::LtEq);
    // body = the 4 load/arith/store ops at [1..5] (guard at 0, control after).
    assert_eq!((plan.body_start, plan.body_end), (1, 5));
}

#[test]
fn recognizes_multiop_map() {
    // c[i] = a[i]*b[i] + a[i] — still a pure map (all reads at i, no carry).
    let ops = loop_region(
        vec![
            load(8, 0, 2, 5),
            load(9, 0, 3, 6),
            MicroOp::MulF {
                dst: 10,
                lhs: 8,
                rhs: 9,
            },
            MicroOp::AddF {
                dst: 10,
                lhs: 10,
                rhs: 8,
            },
            store(10, 0, 4, 7),
        ],
        Cmp::Lt,
        1,
    );
    assert!(recognize_elementwise_map(&ops).is_some());
}

#[test]
fn rejects_reduction_loop_carried_accumulator() {
    // `sum` (slot 9) is read before being defined in the body — loop-carried,
    // NOT bit-exactly vectorizable. Must reject.
    let ops = loop_region(
        vec![
            load(8, 0, 2, 5),
            MicroOp::AddF {
                dst: 9,
                lhs: 9,
                rhs: 8,
            }, // slot 9 loop-carried
            store(9, 0, 4, 7),
        ],
        Cmp::Lt,
        1,
    );
    assert_eq!(recognize_elementwise_map(&ops), None);
}

#[test]
fn rejects_cross_index_access_scan() {
    // out[i] = a[j] where j != i (a scan/stencil read at a different index).
    let ops = loop_region(
        vec![load(8, 9, 2, 5), store(8, 0, 4, 7)], // idx=9 != i(0)
        Cmp::Lt,
        1,
    );
    assert_eq!(recognize_elementwise_map(&ops), None);
}

#[test]
fn rejects_body_with_load_only_no_store() {
    // A body that never stores is not a map (e.g. it feeds a reduction).
    let ops = loop_region(vec![load(8, 0, 2, 5)], Cmp::Lt, 1);
    assert_eq!(recognize_elementwise_map(&ops), None);
}

#[test]
fn rejects_non_unit_stride() {
    // i += 2 — the kernel visits every index, so a stride-2 loop (a subset)
    // must be rejected (the step LoadConst is not 1).
    let ops = loop_region(vec![load(8, 0, 2, 5), store(8, 0, 4, 7)], Cmp::Lt, 2);
    assert_eq!(recognize_elementwise_map(&ops), None);
}

#[test]
fn rejects_bounds_checked_access() {
    // A `checked: true` access — the Oracle did NOT prove length >= n, so the
    // kernel (which elides per-element checks) would be memory-unsafe. Reject.
    let ops = loop_region(
        vec![load_checked(8, 0, 2, 5), store(8, 0, 4, 7)],
        Cmp::Lt,
        1,
    );
    assert_eq!(recognize_elementwise_map(&ops), None);
}

#[test]
fn rejects_non_back_edge_region() {
    // No back-edge `Jump { target: 0 }` -> not a recognized loop region.
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 5,
        },
        load(8, 0, 2, 5),
        store(8, 0, 4, 7),
        MicroOp::LoadConst { dst: 90, value: 1 },
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 90,
        },
    ];
    assert_eq!(recognize_elementwise_map(&ops), None);
}

#[cfg(target_arch = "x86_64")]
fn run_frame(code: &[u8], frame: &mut [i64]) -> i64 {
    let page = crate::JitPage::new(code).unwrap();
    let f: extern "C" fn(*mut i64) -> i64 = unsafe { std::mem::transmute(page.as_ptr()) };
    f(frame.as_mut_ptr())
}

#[cfg(target_arch = "x86_64")]
#[test]
fn packed_body_lowering_matches_scalar_both_lanes() {
    use crate::x64asm::{Asm, Reg, Xmm};
    // Lane assignment for body slots: ta=xmm0, tb=xmm1, tc=xmm2.
    let lane = |s: Slot| match s {
        0 => Xmm::Xmm0,
        1 => Xmm::Xmm1,
        2 => Xmm::Xmm2,
        _ => unreachable!(),
    };
    // Body: tc = ta * tb ; tc = tc + ta  (a pure map expression, 2 ops).
    let body = [
        MicroOp::MulF {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::AddF {
            dst: 2,
            lhs: 2,
            rhs: 0,
        },
    ];
    let cases: [(f64, f64, f64, f64); 3] = [
        (2.5, 0.5, 1.0, 3.0),
        (-1.0, 7.25, 4.0, -2.5),
        (0.1, 0.2, 1.0 / 3.0, 9.0),
    ];
    for (a0, a1, b0, b1) in cases {
        let mut asm = Asm::new();
        asm.movupd_rm(Xmm::Xmm0, Reg::Rdi, 0); // ta = {a0,a1}
        asm.movupd_rm(Xmm::Xmm1, Reg::Rdi, 16); // tb = {b0,b1}
        for op in &body {
            emit_packed_arith(&mut asm, op, lane, Xmm::Xmm15);
        }
        asm.movupd_mr(Reg::Rdi, 32, Xmm::Xmm2); // store tc
        asm.ret();
        let mut frame = [
            a0.to_bits() as i64,
            a1.to_bits() as i64,
            b0.to_bits() as i64,
            b1.to_bits() as i64,
            0,
            0,
        ];
        run_frame(&asm.resolve(), &mut frame);
        assert_eq!(
            f64::from_bits(frame[4] as u64).to_bits(),
            (a0 * b0 + a0).to_bits(),
            "lane0"
        );
        assert_eq!(
            f64::from_bits(frame[5] as u64).to_bits(),
            (a1 * b1 + a1).to_bits(),
            "lane1"
        );
    }
}

#[cfg(target_arch = "x86_64")]
fn run_map(body: &[MicroOp], plan: &MapPlan, a: &[f64], b: &[f64]) -> Vec<f64> {
    // 1-based loop `for i in 1..=n` (LtEq), accessing buffer[i-1] — the shape
    // the recognizer fires on. i starts at 1; n = the length.
    let code = emit_map_kernel(body, plan).expect("kernel emits");
    let mut c = vec![0.0f64; a.len()];
    let mut frame = [
        1i64,
        a.len() as i64,
        a.as_ptr() as i64,
        b.as_ptr() as i64,
        c.as_mut_ptr() as i64,
        0,
        0,
        0,
    ];
    run_frame(&code, &mut frame);
    // The kernel leaves i at the scalar terminal value: n+1 for `i <= n`.
    assert_eq!(frame[0], a.len() as i64 + 1, "induction terminal n+1");
    c
}

#[cfg(target_arch = "x86_64")]
#[test]
fn map_kernel_add_matches_scalar_all_lengths() {
    // c[i] = a[i] + b[i] over various n (even, odd, 1, 0) — exercises the pair
    // loop AND the scalar tail.
    let body = vec![
        load(5, 0, 2, 0),
        load(6, 0, 3, 0),
        MicroOp::AddF {
            dst: 7,
            lhs: 5,
            rhs: 6,
        },
        store(7, 0, 4, 0),
    ];
    let plan = MapPlan {
        induction: 0,
        limit: 1,
        cmp: Cmp::LtEq,
        body_start: 0,
        body_end: 4,
    };
    for n in [0usize, 1, 2, 3, 7, 8, 15] {
        let a: Vec<f64> = (0..n).map(|i| i as f64 * 1.5 - 3.0).collect();
        let b: Vec<f64> = (0..n).map(|i| i as f64 * -0.25 + 1.0).collect();
        let got = run_map(&body, &plan, &a, &b);
        for i in 0..n {
            assert_eq!(got[i].to_bits(), (a[i] + b[i]).to_bits(), "n={n} i={i}");
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn map_kernel_multiop_matches_scalar() {
    // c[i] = a[i]*b[i] + a[i] — multi-op map, pair loop + tail.
    let body = vec![
        load(5, 0, 2, 0),
        load(6, 0, 3, 0),
        MicroOp::MulF {
            dst: 7,
            lhs: 5,
            rhs: 6,
        },
        MicroOp::AddF {
            dst: 7,
            lhs: 7,
            rhs: 5,
        },
        store(7, 0, 4, 0),
    ];
    let plan = MapPlan {
        induction: 0,
        limit: 1,
        cmp: Cmp::LtEq,
        body_start: 0,
        body_end: 5,
    };
    for n in [1usize, 4, 5, 16, 17] {
        let a: Vec<f64> = (0..n).map(|i| 0.1 * i as f64 + 0.3).collect();
        let b: Vec<f64> = (0..n).map(|i| 2.0 - 0.07 * i as f64).collect();
        let got = run_map(&body, &plan, &a, &b);
        for i in 0..n {
            assert_eq!(
                got[i].to_bits(),
                (a[i] * b[i] + a[i]).to_bits(),
                "n={n} i={i}"
            );
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn map_kernel_bails_on_unsupported_loadconst() {
    // A body with a LoadConst is not lowered by the kernel -> None (the caller
    // falls back to the scalar region, staying sound).
    let body = vec![
        load(5, 0, 2, 0),
        MicroOp::LoadConst { dst: 6, value: 2 },
        MicroOp::MulF {
            dst: 7,
            lhs: 5,
            rhs: 6,
        },
        store(7, 0, 4, 0),
    ];
    let plan = MapPlan {
        induction: 0,
        limit: 1,
        cmp: Cmp::LtEq,
        body_start: 0,
        body_end: 4,
    };
    assert!(emit_map_kernel(&body, &plan).is_none());
}

#[cfg(target_arch = "x86_64")]
#[test]
fn packed_body_lowering_non_commutative_dst_eq_rhs_uses_scratch() {
    use crate::x64asm::{Asm, Reg, Xmm};
    // SubF dst==rhs: td(xmm1) = ta(xmm0) - td(xmm1). The 2-operand subpd would
    // compute xmm1 = xmm1 - xmm0 (wrong order); emit_bin must route through the
    // scratch to preserve `lhs - rhs`.
    let lane = |s: Slot| match s {
        0 => Xmm::Xmm0,
        1 => Xmm::Xmm1,
        _ => unreachable!(),
    };
    let op = MicroOp::SubF {
        dst: 1,
        lhs: 0,
        rhs: 1,
    };
    let cases: [(f64, f64, f64, f64); 2] = [(2.5, 0.5, 1.0, 3.0), (-1.0, 3.0, 4.0, -2.5)];
    for (a0, a1, b0, b1) in cases {
        let mut asm = Asm::new();
        asm.movupd_rm(Xmm::Xmm0, Reg::Rdi, 0); // ta
        asm.movupd_rm(Xmm::Xmm1, Reg::Rdi, 16); // td (the rhs, gets overwritten)
        emit_packed_arith(&mut asm, &op, lane, Xmm::Xmm15);
        asm.movupd_mr(Reg::Rdi, 32, Xmm::Xmm1);
        asm.ret();
        let mut frame = [
            a0.to_bits() as i64,
            a1.to_bits() as i64,
            b0.to_bits() as i64,
            b1.to_bits() as i64,
            0,
            0,
        ];
        run_frame(&asm.resolve(), &mut frame);
        assert_eq!(
            f64::from_bits(frame[4] as u64).to_bits(),
            (a0 - b0).to_bits(),
            "lane0 a-b"
        );
        assert_eq!(
            f64::from_bits(frame[5] as u64).to_bits(),
            (a1 - b1).to_bits(),
            "lane1 a-b"
        );
    }
}
