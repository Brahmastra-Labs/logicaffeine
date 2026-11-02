use super::*;
use crate::jit::{reference_eval, ChainOutcome};
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn run(ops: &[MicroOp], frame: &[i64]) -> (ChainOutcome, Vec<i64>) {
    let status = Arc::new(AtomicI64::new(0));
    let chain = compile_region_regalloc(ops, Some(status), &[]).expect("supported region compiles");
    let mut f = frame.to_vec();
    let out = chain.run_with_frame(&mut f);
    (out, f)
}

#[test]
fn live_intervals_span_first_to_last_touch() {
    // LoadConst s0; Move s1<-s0; Return s1.
    let ops = vec![
        MicroOp::LoadConst { dst: 0, value: 7 },
        MicroOp::Move { dst: 1, src: 0 },
        MicroOp::Return { src: 1 },
    ];
    let iv = live_intervals(&ops, 1);
    assert_eq!(
        iv,
        vec![
            LiveInterval {
                slot: 0,
                start: 0,
                end: 1
            }, // defined @0, last read @1
            LiveInterval {
                slot: 1,
                start: 1,
                end: 2
            }, // defined @1, returned @2
        ]
    );
}

#[test]
fn linscan_reuses_a_register_across_disjoint_ranges() {
    // Five float temporaries with strictly DISJOINT ranges. The whole-region
    // per-slot model would give each its own home (spilling 4 of 5 with a
    // single register); linear scan reuses the one register for all → 0 spill.
    let class = vec![Class::Float; 5];
    let iv: Vec<LiveInterval> = (0..5)
        .map(|s| LiveInterval {
            slot: s,
            start: (s as usize) * 2,
            end: (s as usize) * 2 + 1,
        })
        .collect();
    assert_eq!(
        linscan_spills(&iv, &class, Class::Float, 1),
        0,
        "disjoint ranges share 1 reg"
    );
    assert_eq!(
        linscan_spills(&iv, &class, Class::Float, 0),
        5,
        "0 regs spills all"
    );
}

#[test]
fn linscan_spills_only_the_overlap_excess() {
    // Three fully-overlapping float intervals need 3 registers; with 2 exactly
    // one spills, with 3 none. This is the pressure the per-slot model can't
    // see — it would spill by rank, not by overlap.
    let class = vec![Class::Float; 3];
    let iv: Vec<LiveInterval> = (0..3)
        .map(|s| LiveInterval {
            slot: s,
            start: 0,
            end: 9,
        })
        .collect();
    assert_eq!(linscan_spills(&iv, &class, Class::Float, 3), 0);
    assert_eq!(linscan_spills(&iv, &class, Class::Float, 2), 1);
    assert_eq!(linscan_spills(&iv, &class, Class::Float, 1), 2);
}

/// A float accumulation loop: `acc` is loop-carried; `x` and `y` are disjoint
/// within-iteration float temps. Returns (ops, max_slot). slots: 0=i 1=N
/// 2=acc(f) 3=x(f) 4=y(f) 5=one.
fn float_loop_two_temps() -> (Vec<MicroOp>, usize) {
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 8,
        }, // loop guard (ctrl + target@8)
        MicroOp::IntToFloat { dst: 3, src: 0 }, // x = (f)i
        MicroOp::AddF {
            dst: 2,
            lhs: 2,
            rhs: 3,
        }, // acc += x   (x dead after)
        MicroOp::IntToFloat { dst: 4, src: 0 }, // y = (f)i
        MicroOp::AddF {
            dst: 2,
            lhs: 2,
            rhs: 4,
        }, // acc += y   (y dead after)
        MicroOp::LoadConst { dst: 5, value: 1 },
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 5,
        }, // i += 1
        MicroOp::Jump { target: 0 }, // back-edge (ctrl + target@0)
        MicroOp::Return { src: 2 },
    ];
    (ops, 5)
}

#[test]
fn live_ranges_are_loop_aware() {
    let (ops, max_slot) = float_loop_two_temps();
    let r = live_ranges(&ops, max_slot);
    // acc (loop-carried) spans the whole loop, returned at op 8.
    assert_eq!(r[2], Some((0, 8)), "carried acc must span the loop");
    // x / y are within-iteration temps with disjoint ranges.
    assert_eq!(r[3], Some((1, 2)));
    assert_eq!(r[4], Some((3, 4)));
}

#[test]
fn linscan_prefers_fresh_then_reuses_under_pressure() {
    let (ops, max_slot) = float_loop_two_temps();
    let class = classify_slots(&ops, max_slot);
    let ranges = live_ranges(&ops, max_slot);
    // AMPLE XMM pool: disjoint temps x and y each get their OWN register — no
    // reuse, hence no spill-at-end overhead when the region already fits.
    let ample = linscan_assign(&ops, max_slot, &class, &CALLER_SAVED, &FLOAT_REGS);
    assert_ne!(ample[3], Loc::Frame, "temp must get a register");
    assert_ne!(
        ample[3], ample[4],
        "with registers to spare, disjoint temps do NOT share"
    );
    // PRESSURE: only acc + one scratch XMM. acc (loop-carried) overlaps both
    // temps and keeps its register; x and y must then SHARE the single scratch.
    let tight = linscan_assign(
        &ops,
        max_slot,
        &class,
        &CALLER_SAVED,
        &[Xmm::Xmm0, Xmm::Xmm1],
    );
    assert_eq!(
        tight[3], tight[4],
        "under pressure, disjoint gap-clear temps reuse one register"
    );
    assert_ne!(
        tight[2], tight[3],
        "carried acc cannot share a live temp's register"
    );
    // INVARIANT (both regimes): no two slots with OVERLAPPING ranges share a register.
    for assign in [&ample, &tight] {
        for a in 0..=max_slot {
            for b in (a + 1)..=max_slot {
                if let (Some((sa, ea)), Some((sb, eb))) = (ranges[a], ranges[b]) {
                    let overlap = sa <= eb && sb <= ea;
                    if overlap && assign[a] != Loc::Frame {
                        assert_ne!(
                            assign[a], assign[b],
                            "overlapping slots {a},{b} share a register"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn linscan_no_reuse_across_a_control_point() {
    // Two disjoint int temps separated by a jump TARGET: reuse would cross a
    // control join, so they must NOT share a register.
    // 0:LoadConst t0  1:Add a=t0+t0 (t0 dead@1)  2:Jump 4  3:(unreached)  4:(target) LoadConst t1  5:Add b=t1+t1  6:Return b
    let ops = vec![
        MicroOp::LoadConst { dst: 0, value: 1 }, // t0 @0
        MicroOp::Add {
            dst: 1,
            lhs: 0,
            rhs: 0,
        }, // a=t0+t0 ; t0 dead after @1
        MicroOp::Jump { target: 4 },             // ctrl; target 4 is a jump target
        MicroOp::LoadConst { dst: 9, value: 0 }, // filler (unreached) keeps idx 3 occupied
        MicroOp::LoadConst { dst: 2, value: 2 }, // t1 @4 (a jump target — control join)
        MicroOp::Add {
            dst: 3,
            lhs: 2,
            rhs: 2,
        }, // b=t1+t1
        MicroOp::Return { src: 3 },
    ];
    let max_slot = 9;
    let class = classify_slots(&ops, max_slot);
    let assign = linscan_assign(&ops, max_slot, &class, &CALLER_SAVED, &FLOAT_REGS);
    // t0 (range ~[0,1]) freed before t1 (range ~[4,5]) but op 4 is a jump
    // target in the gap → t1 must take a FRESH register, not t0's.
    assert_ne!(assign[0], assign[2], "reuse must not cross a control point");
}

#[test]
fn straightline_arith_matches_reference() {
    let ops = vec![
        MicroOp::Add {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Mul {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::Sub {
            dst: 5,
            lhs: 4,
            rhs: 0,
        },
        MicroOp::Return { src: 5 },
    ];
    let frame = vec![7i64, 11, 13, 0, 0, 0];
    let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
    let (out, _) = run(&ops, &frame);
    assert_eq!(out, ChainOutcome::Return(expected));
}

#[test]
fn counting_loop_matches_reference() {
    // sum 0..N via a branch-back loop.
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 2,
            target: 5,
        }, // while i<N
        MicroOp::Add {
            dst: 1,
            lhs: 1,
            rhs: 0,
        }, // acc += i
        MicroOp::LoadConst { dst: 3, value: 1 },
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 3,
        }, // i += 1
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 1 },
    ];
    let frame = vec![0i64, 0, 1000, 0];
    let expected = reference_eval(&ops, &mut frame.clone(), 10_000_000).unwrap();
    let (out, _) = run(&ops, &frame);
    assert_eq!(out, ChainOutcome::Return(expected));
    assert_eq!(out, ChainOutcome::Return(499_500));
}

#[test]
fn div_by_zero_side_exits() {
    let status = Arc::new(AtomicI64::new(0));
    let ops = vec![
        MicroOp::Div {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Return { src: 2 },
    ];
    let chain = compile_region_regalloc(&ops, Some(status), &[]).unwrap();
    let mut frame = vec![100i64, 0, 0];
    let out = chain.run_with_frame(&mut frame);
    assert!(out.is_deopt(), "div by zero must side-exit, got {out:?}");
}

#[test]
fn div_min_neg_one_side_exits() {
    // Exact arithmetic: the overflowing quotient i64::MIN / -1 side-exits
    // in BOTH the reference and the machine code (deopt → promotion),
    // never wraps — and it must not trap the process.
    let ops = vec![
        MicroOp::Div {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Return { src: 2 },
    ];
    let frame = vec![i64::MIN, -1, 0];
    assert!(
        reference_eval(&ops, &mut frame.clone(), 1000).is_none(),
        "reference must side-exit MIN / -1"
    );
    let (out, _) = run(&ops, &frame);
    assert!(
        out.is_deopt(),
        "machine code must side-exit MIN / -1, got {out:?}"
    );
}

#[test]
fn mod_min_neg_one_zero() {
    let ops = vec![
        MicroOp::Mod {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Return { src: 2 },
    ];
    let frame = vec![i64::MIN, -1, 0];
    let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
    let (out, _) = run(&ops, &frame);
    assert_eq!(out, ChainOutcome::Return(expected)); // MIN % -1 == 0
    assert_eq!(out, ChainOutcome::Return(0));
}

#[test]
fn unsupported_op_returns_none() {
    // A `MapGet` (hash-map probe via a runtime helper) is outside the
    // supported subset, so the region declines and the caller falls back to
    // the per-piece stencil tier. (Byte / `Seq of Bool` array loads/stores
    // ARE now supported — see `byte_array_*` — so they no longer decline.)
    let ops = vec![
        MicroOp::MapGet {
            dst: 1,
            key: 0,
            map_slot: 2,
            helper_addr: 0,
        },
        MicroOp::Return { src: 1 },
    ];
    assert!(compile_region_regalloc(&ops, None, &[]).is_none());
}

// =================================================================
// WAVE 25: the INLINED `ArrPush` fast path. Each push lowers to an
// INLINE `len < cap ? buffer[len++] = v` test that calls the runtime
// helper ONLY on the (cold) realloc boundary. These forge-level tests
// drive a REAL `Vec` through the compiled region with an instrumented
// helper, proving (1) bit-identical contents to `Vec::push` across the
// fast path AND the realloc cold path, and (2) that the per-iteration
// call vanished on the fast path (the helper's call counter).
// =================================================================

/// Call counter for the instrumented test push helper: a fast-path push
/// (`len < cap`) must NOT touch the helper, so this counts only the realloc
/// cold-path entries.
static TEST_PUSH_CALLS: AtomicI64 = AtomicI64::new(0);

/// A drop-in stand-in for `logos_rt_push_i64` with the SAME SysV ABI and the
/// SAME `Vec::push` + ptr/len-refresh semantics, plus a call counter. The
/// inline fast path must reproduce its effect EXACTLY when `len < cap`.
///
/// # Safety
/// `frame` slots must hold a live `*mut Vec<i64>` (vec_slot) and the mirrored
/// ptr/len cells, exactly as the region pin convention requires.
unsafe extern "C" fn test_push_i64(
    frame: *mut i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
    value: i64,
) {
    TEST_PUSH_CALLS.fetch_add(1, Ordering::Relaxed);
    let vec = *frame.add(vec_slot as usize) as *mut Vec<i64>;
    (*vec).push(value);
    *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = (*vec).len() as i64;
}

/// Build a region that pushes `i` for `i` in `[0, n)` into a pinned Int list,
/// then returns the final length. Frame layout:
///   0 = i (induction), 1 = n (bound), 2 = const 1,
///   8 = vec handle, 9 = buffer ptr, 10 = length.
fn push_loop_ops(helper_addr: i64) -> Vec<MicroOp> {
    vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 5,
        }, // while i < n
        MicroOp::ArrPush {
            src: 0,
            vec_slot: 8,
            ptr_slot: 9,
            len_slot: 10,
            helper_addr,
            byte: false,
            narrow32: false,
        },
        MicroOp::LoadConst { dst: 2, value: 1 },
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 2,
        }, // i += 1
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 10 }, // final length
    ]
}

/// RED: pushes that ALL fit the pre-allocated capacity touch the runtime
/// helper ZERO times — every push took the inline fast path — and the buffer
/// holds exactly `Vec::push`'s result (same contents, same final pointer,
/// since no realloc moved it).
#[test]
fn inline_push_fast_path_never_calls_helper_within_capacity() {
    let n = 5000i64;
    let mut vec: Vec<i64> = Vec::with_capacity(n as usize);
    let vec_ptr = &mut vec as *mut Vec<i64>;
    let buf_before = vec.as_ptr() as i64;

    let ops = push_loop_ops(test_push_i64 as usize as i64);
    let status = Arc::new(AtomicI64::new(0));
    let chain = compile_region_regalloc(&ops, Some(status), &[]).expect("push region compiles");

    // frame[8] = vec handle, [9] = buffer ptr, [10] = length (all 0 initially).
    let mut frame = vec![0i64; 11];
    frame[1] = n;
    frame[8] = vec_ptr as i64;
    frame[9] = buf_before;
    frame[10] = 0;

    TEST_PUSH_CALLS.store(0, Ordering::Relaxed);
    let out = chain.run_with_frame(&mut frame);

    assert_eq!(out, ChainOutcome::Return(n), "final length must be n");
    assert_eq!(
        TEST_PUSH_CALLS.load(Ordering::Relaxed),
        0,
        "every push fit within capacity — the inline fast path must call the helper ZERO times"
    );
    // Bit-identical to Vec::push: contents and (no-realloc) pointer.
    assert_eq!(vec.len(), n as usize, "real Vec length");
    assert_eq!(
        vec.as_ptr() as i64,
        buf_before,
        "no realloc moved the buffer"
    );
    for (i, &v) in vec.iter().enumerate() {
        assert_eq!(v, i as i64, "buffer[{i}] must be the pushed value");
    }
    // The frame's mirrored len/ptr stay coherent with the real Vec.
    assert_eq!(frame[10], n, "mirrored length");
    assert_eq!(
        frame[9], buf_before,
        "mirrored pointer unchanged (no realloc)"
    );
}

/// RED: pushes that GROW past the initial capacity stay bit-identical to
/// `Vec::push` — the cold helper path reallocates and refreshes ptr/len, and
/// the inline path bumps len in place between reallocs. The helper is called
/// only on the realloc boundaries (far fewer than `n` times), proving the
/// per-iteration call is gone.
#[test]
fn inline_push_realloc_cold_path_matches_vec_push() {
    let n = 4000i64;
    // Start with a TINY capacity so the cold realloc path fires repeatedly.
    let mut vec: Vec<i64> = Vec::with_capacity(1);
    let vec_ptr = &mut vec as *mut Vec<i64>;

    // The reference: a plain Vec::push of the same sequence from cap 1.
    let mut reference: Vec<i64> = Vec::with_capacity(1);
    for i in 0..n {
        reference.push(i);
    }

    let ops = push_loop_ops(test_push_i64 as usize as i64);
    let status = Arc::new(AtomicI64::new(0));
    let chain = compile_region_regalloc(&ops, Some(status), &[]).expect("push region compiles");

    let mut frame = vec![0i64; 11];
    frame[1] = n;
    frame[8] = vec_ptr as i64;
    frame[9] = vec.as_ptr() as i64;
    frame[10] = 0;

    TEST_PUSH_CALLS.store(0, Ordering::Relaxed);
    let out = chain.run_with_frame(&mut frame);

    assert_eq!(out, ChainOutcome::Return(n), "final length must be n");
    assert_eq!(
        vec, reference,
        "compiled push must be bit-identical to Vec::push"
    );
    assert_eq!(frame[10], n, "mirrored length matches");
    assert_eq!(
        frame[9],
        vec.as_ptr() as i64,
        "mirrored pointer matches the (post-realloc) buffer"
    );
    let calls = TEST_PUSH_CALLS.load(Ordering::Relaxed);
    // Geometric growth from cap 1: O(log n) reallocs, vastly fewer than n.
    assert!(
        calls < n / 2,
        "the helper must fire only on realloc boundaries (got {calls} of {n} pushes)"
    );
    assert!(calls >= 1, "at least one realloc must have occurred");
}

/// RED: a `Seq of Bool` (1-byte element) push stores the BOOLEAN
/// NORMALIZATION `(v != 0) as u8` inline — bit-identical to
/// `logos_rt_push_bool`'s `(*vec).push(value != 0)`.
#[test]
fn inline_push_byte_normalizes_like_helper() {
    unsafe extern "C" fn test_push_bool(
        frame: *mut i64,
        vec_slot: i64,
        ptr_slot: i64,
        len_slot: i64,
        value: i64,
    ) {
        let vec = *frame.add(vec_slot as usize) as *mut Vec<bool>;
        (*vec).push(value != 0);
        *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
        *frame.add(len_slot as usize) = (*vec).len() as i64;
    }

    let n = 300i64;
    let mut vec: Vec<bool> = Vec::with_capacity(n as usize);
    let vec_ptr = &mut vec as *mut Vec<bool>;
    let buf_before = vec.as_ptr() as i64;

    // Push `i % 3` (0, 1, 2, 0, 1, 2, …): zero stores false, nonzero true.
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 7,
        }, // while i < n
        MicroOp::LoadConst { dst: 3, value: 3 },
        MicroOp::Mod {
            dst: 4,
            lhs: 0,
            rhs: 3,
        }, // v = i % 3
        MicroOp::ArrPush {
            src: 4,
            vec_slot: 8,
            ptr_slot: 9,
            len_slot: 10,
            helper_addr: test_push_bool as usize as i64,
            byte: true,
            narrow32: false,
        },
        MicroOp::LoadConst { dst: 2, value: 1 },
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 2,
        }, // i += 1
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 10 },
    ];
    let status = Arc::new(AtomicI64::new(0));
    let chain = compile_region_regalloc(&ops, Some(status), &[]).expect("byte push region compiles");

    let mut frame = vec![0i64; 11];
    frame[1] = n;
    frame[8] = vec_ptr as i64;
    frame[9] = buf_before;
    frame[10] = 0;
    let out = chain.run_with_frame(&mut frame);

    assert_eq!(out, ChainOutcome::Return(n));
    let reference: Vec<bool> = (0..n).map(|i| (i % 3) != 0).collect();
    assert_eq!(
        vec, reference,
        "byte push must normalize (v != 0) like the helper"
    );
    assert_eq!(
        vec.as_ptr() as i64,
        buf_before,
        "no realloc within capacity"
    );
}

/// RED: an XMM-resident (float-list) push bit-copies the value to the buffer
/// inline — the raw 8 bytes travel identically to the helper's
/// `f64::from_bits` round-trip.
#[test]
fn inline_push_float_value_bitcopies() {
    unsafe extern "C" fn test_push_f64(
        frame: *mut i64,
        vec_slot: i64,
        ptr_slot: i64,
        len_slot: i64,
        value: i64,
    ) {
        let vec = *frame.add(vec_slot as usize) as *mut Vec<f64>;
        (*vec).push(f64::from_bits(value as u64));
        *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
        *frame.add(len_slot as usize) = (*vec).len() as i64;
    }

    let n = 200i64;
    let mut vec: Vec<f64> = Vec::with_capacity(n as usize);
    let vec_ptr = &mut vec as *mut Vec<f64>;
    let buf_before = vec.as_ptr() as i64;

    // src is a FLOAT slot produced by IntToFloat (lands in XMM): push i as f64.
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 6,
        }, // while i < n
        MicroOp::IntToFloat { dst: 4, src: 0 }, // v = i as f64 (XMM-resident)
        MicroOp::ArrPush {
            src: 4,
            vec_slot: 8,
            ptr_slot: 9,
            len_slot: 10,
            helper_addr: test_push_f64 as usize as i64,
            byte: false,
            narrow32: false,
        },
        MicroOp::LoadConst { dst: 2, value: 1 },
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 2,
        }, // i += 1
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 10 },
    ];
    let status = Arc::new(AtomicI64::new(0));
    let chain = compile_region_regalloc(&ops, Some(status), &[]).expect("float push region compiles");

    let mut frame = vec![0i64; 11];
    frame[1] = n;
    frame[8] = vec_ptr as i64;
    frame[9] = buf_before;
    frame[10] = 0;
    let out = chain.run_with_frame(&mut frame);

    assert_eq!(out, ChainOutcome::Return(n));
    let reference: Vec<f64> = (0..n).map(|i| i as f64).collect();
    assert_eq!(
        vec, reference,
        "float push must bit-copy the value like the helper"
    );
    assert_eq!(
        vec.as_ptr() as i64,
        buf_before,
        "no realloc within capacity"
    );
}

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

/// Exhaustive differential: every random straight-line program over the
/// supported families must equal the (location-independent) reference, at
/// EVERY slot count — small (all resident) and large (forces spills).
#[test]
fn random_straightline_matches_reference_all_pressures() {
    for slots in [3u16, 6, 10, 20] {
        for seed in 1..=80u64 {
            let mut s = seed.wrapping_mul(2654435761);
            let len = 8 + (lcg(&mut s) % 24) as usize;
            let mut ops = Vec::with_capacity(len + 1);
            for _ in 0..len {
                let dst = (lcg(&mut s) % slots as u64) as u16;
                let lhs = (lcg(&mut s) % slots as u64) as u16;
                let rhs = (lcg(&mut s) % slots as u64) as u16;
                let op = match lcg(&mut s) % 13 {
                    0 => MicroOp::Add { dst, lhs, rhs },
                    1 => MicroOp::Sub { dst, lhs, rhs },
                    2 => MicroOp::Mul { dst, lhs, rhs },
                    3 => MicroOp::BitAnd { dst, lhs, rhs },
                    4 => MicroOp::BitOr { dst, lhs, rhs },
                    5 => MicroOp::BitXor { dst, lhs, rhs },
                    6 => MicroOp::Lt { dst, lhs, rhs },
                    7 => MicroOp::Gt { dst, lhs, rhs },
                    8 => MicroOp::LtEq { dst, lhs, rhs },
                    9 => MicroOp::Eq { dst, lhs, rhs },
                    10 => MicroOp::Neq { dst, lhs, rhs },
                    11 => MicroOp::Move { dst, src: lhs },
                    _ => MicroOp::LoadConst {
                        dst,
                        value: (lcg(&mut s) as i64).wrapping_sub(1 << 40),
                    },
                };
                ops.push(op);
            }
            let ret = (lcg(&mut s) % slots as u64) as u16;
            ops.push(MicroOp::Return { src: ret });

            let mut frame = vec![0i64; slots as usize];
            for (i, f) in frame.iter_mut().enumerate() {
                *f = (i as i64 + 1).wrapping_mul(1_000_003) - 7;
            }
            // Exact integer math: `reference_eval` returns None on i64 overflow,
            // which the native chain matches with a side-exit (deopt).
            let expected = reference_eval(&ops, &mut frame.clone(), 100_000);
            let (out, post) = run(&ops, &frame);
            match expected {
                Some(e) => {
                    assert_eq!(
                        out,
                        ChainOutcome::Return(e),
                        "slots={slots} seed={seed}: return diverged"
                    );
                    // The full frame must also match the reference's full frame.
                    let mut ref_frame = frame.clone();
                    reference_eval(&ops, &mut ref_frame, 100_000);
                    assert_eq!(post, ref_frame, "slots={slots} seed={seed}: frame diverged");
                }
                None => assert!(
                    out.is_deopt(),
                    "slots={slots} seed={seed}: overflow must deopt, got {out:?}"
                ),
            }
        }
    }
}

/// Shifts and unary ops bit-exact against the reference, at spill pressure.
#[test]
fn shifts_and_unary_match_reference() {
    let ops = vec![
        MicroOp::Shl {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Shr {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::NotInt { dst: 5, src: 4 },
        MicroOp::NotBool { dst: 6, src: 2 },
        MicroOp::DivPow2 {
            dst: 7,
            lhs: 0,
            k: 3,
        },
        MicroOp::Add {
            dst: 8,
            lhs: 5,
            rhs: 7,
        },
        MicroOp::Return { src: 8 },
    ];
    for &a in &[1i64, -1, 1024, i64::MIN, -255] {
        let frame = vec![a, 5, 2, 0, 0, 0, 0, 0, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
        let (out, _) = run(&ops, &frame);
        assert_eq!(out, ChainOutcome::Return(expected), "a={a}");
    }
}

/// The Granlund–Montgomery magic constants for an unsigned divisor `d`
/// (test-local; mirrors the canonical `LogosDivU64::new` the compiler ships).
fn magic_parts(d: u64) -> (u64, u8) {
    if d & (d - 1) == 0 {
        return (0, (d.trailing_zeros() as u8) | 0x80);
    }
    let l = 63 - d.leading_zeros();
    let numer = (1u128 << l) << 64;
    let m = (numer / d as u128) as u64;
    let rem = (numer % d as u128) as u64;
    if d - rem < (1u64 << l) {
        (m + 1, l as u8)
    } else {
        let twice = rem.wrapping_mul(2);
        let bump = (twice >= d || twice < rem) as u64;
        (m.wrapping_mul(2).wrapping_add(bump) + 1, (l as u8) | 0x40)
    }
}

/// `MagicDivU` (the W24 constant-divisor magic reciprocal) is bit-exact with
/// `reference_eval` — which itself is bit-exact with the kernel's
/// `wrapping_div`/`wrapping_rem` for the non-negative dividend it gates on —
/// for both the quotient and the remainder, across several divisors (the
/// 64-bit-magic and 65-bit add-marker paths) and a boundary value grid,
/// emitted into a frame with enough live slots to force spill pressure on
/// the dst/lhs.
#[test]
fn magic_div_matches_reference_div_and_mod() {
    for &c in &[3i64, 7, 100, 1000, 1_000_000_007, 65521] {
        let (magic, more) = magic_parts(c as u64);
        // Two ops (div into 1, mod into 2) plus padding slots 3..16 so the
        // allocator must spill — exercises both register- and frame-resident
        // dst/lhs in emit_magic_div.
        let ops = vec![
            MicroOp::MagicDivU {
                dst: 1,
                lhs: 0,
                magic,
                more,
                mul_back: 0,
            },
            MicroOp::MagicDivU {
                dst: 2,
                lhs: 0,
                magic,
                more,
                mul_back: c,
            },
            MicroOp::Add {
                dst: 3,
                lhs: 1,
                rhs: 2,
            },
            MicroOp::Return { src: 3 },
        ];
        for n in [
            0i64,
            1,
            c - 1,
            c,
            c + 1,
            2 * c + 3,
            123_456_789,
            i64::MAX,
            i64::MAX - 1,
            (i64::MAX / c) * c,
            (i64::MAX / c).saturating_sub(1) * c,
        ] {
            assert!(n >= 0);
            let frame = vec![n, 0, 0, 0];
            let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
            let (out, _) = run(&ops, &frame);
            assert_eq!(out, ChainOutcome::Return(expected), "c={c} n={n}");
            // And the reference itself equals the kernel for these.
            assert_eq!(
                expected,
                n.wrapping_div(c).wrapping_add(n.wrapping_rem(c)),
                "reference vs kernel c={c} n={n}"
            );
        }
    }
}

/// A nested loop with many live slots (forces some spills) — the polynomial
/// accumulation the ceiling harness times — matches the reference exactly.
#[test]
fn poly_loop_matches_reference_under_spill() {
    // s = s*A + i*B - C across i in 0..N, with A,B,C,N,i,s in distinct slots.
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 11,
        }, // i<N
        MicroOp::Mul {
            dst: 7,
            lhs: 2,
            rhs: 3,
        }, // t = s*A
        MicroOp::Mul {
            dst: 8,
            lhs: 0,
            rhs: 4,
        }, // u = i*B
        MicroOp::Add {
            dst: 7,
            lhs: 7,
            rhs: 8,
        }, // t += u
        MicroOp::Sub {
            dst: 2,
            lhs: 7,
            rhs: 5,
        }, // s = t - C
        MicroOp::BitXor {
            dst: 2,
            lhs: 2,
            rhs: 6,
        }, // s ^= MASK
        MicroOp::Add {
            dst: 9,
            lhs: 2,
            rhs: 0,
        }, // (extra live)
        MicroOp::Mul {
            dst: 2,
            lhs: 2,
            rhs: 3,
        }, // s *= A again
        MicroOp::LoadConst { dst: 10, value: 1 },
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 10,
        }, // i += 1
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 2 },
    ];
    // slots: 0=i 1=N 2=s 3=A 4=B 5=C 6=MASK 7=t 8=u 9=extra 10=one
    let frame = vec![
        0i64,
        5000,
        1,
        6364136223846793005u64 as i64,
        1442695040888963407u64 as i64,
        12345,
        0x5DEECE66D,
        0,
        0,
        0,
        0,
    ];
    // These LCG-sized multipliers overflow i64 on the first `s*A`. Under exact
    // integer math the reference returns None and the native chain side-exits
    // (deopt) — the two must agree that this leaves the i64 fast path.
    let expected = reference_eval(&ops, &mut frame.clone(), 100_000_000);
    let (out, _) = run(&ops, &frame);
    match expected {
        Some(e) => assert_eq!(out, ChainOutcome::Return(e)),
        None => assert!(out.is_deopt(), "overflow must deopt, got {out:?}"),
    }
}

/// `loop_depths` (the spill-weight heuristic's loop detector): a back-edge
/// `[target, idx]` raises the depth of every op in its span by one, and
/// nesting accumulates. Straight-line code stays depth 0.
#[test]
fn loop_depths_counts_back_edge_nesting() {
    // No back-edge: all depth 0.
    let flat = vec![
        MicroOp::Add {
            dst: 1,
            lhs: 0,
            rhs: 0,
        },
        MicroOp::Return { src: 1 },
    ];
    assert_eq!(loop_depths(&flat), vec![0, 0]);

    // One loop: a Jump at idx 3 back to 1 wraps ops 1..=3 (depth 1); the
    // guard at 0 and the tail at 4 stay depth 0.
    let single = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 4,
        }, // 0
        MicroOp::Add {
            dst: 2,
            lhs: 2,
            rhs: 0,
        }, // 1
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 3,
        }, // 2
        MicroOp::Jump { target: 1 }, // 3
        MicroOp::Return { src: 2 },  // 4
    ];
    assert_eq!(loop_depths(&single), vec![0, 1, 1, 1, 0]);

    // Nested: inner back-edge 3->2 (depth 2 over ops 2..=3) sits inside the
    // outer back-edge 5->1 (depth 1 over ops 1..=5).
    let nested = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 6,
        }, // 0
        MicroOp::Move { dst: 4, src: 0 }, // 1 outer
        MicroOp::Add {
            dst: 2,
            lhs: 2,
            rhs: 4,
        }, // 2 inner
        MicroOp::Jump { target: 2 },      // 3 inner back-edge
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 3,
        }, // 4 outer
        MicroOp::Jump { target: 1 },      // 5 outer back-edge
        MicroOp::Return { src: 2 },       // 6
    ];
    assert_eq!(loop_depths(&nested), vec![0, 1, 2, 2, 1, 1, 0]);
}

/// A loop-carried slot referenced ONCE per iteration but living inside a
/// loop must out-rank a colder slot referenced MANY times only in
/// straight-line setup, so it wins a register. We assert this through the
/// public `loc` assignment indirectly: build a region where slot `H` (hot,
/// loop-carried, few raw refs) and slot `C` (cold, setup-only, many raw refs)
/// compete for the SOLE remaining int register, and confirm the loop weight
/// flips the ranking so the result is still bit-identical to the reference
/// (the heuristic only moves residency, never correctness).
#[test]
fn loop_weight_keeps_carried_slot_resident_and_exact() {
    // Setup touches `C` (slot 5) four times in straight-line code; the loop
    // touches `H` (slot 2, the accumulator) once per iteration. Raw counts
    // would rank C above H; the loop weight (10^1 per loop ref) flips it.
    let ops = vec![
        MicroOp::Add {
            dst: 5,
            lhs: 0,
            rhs: 1,
        }, // 0 C = a+b   (setup)
        MicroOp::Add {
            dst: 5,
            lhs: 5,
            rhs: 0,
        }, // 1 C += a    (setup)
        MicroOp::Add {
            dst: 5,
            lhs: 5,
            rhs: 1,
        }, // 2 C += b    (setup)
        MicroOp::Add {
            dst: 2,
            lhs: 2,
            rhs: 5,
        }, // 3 seed acc with C (setup)
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 3,
            rhs: 4,
            target: 8,
        }, // 4 while i<N
        MicroOp::Add {
            dst: 2,
            lhs: 2,
            rhs: 3,
        }, // 5 acc += i  (HOT, depth 1)
        MicroOp::Add {
            dst: 3,
            lhs: 3,
            rhs: 6,
        }, // 6 i += 1    (HOT, depth 1)
        MicroOp::Jump { target: 4 }, // 7 back-edge
        MicroOp::Return { src: 2 },  // 8
    ];
    // slots: 0=a 1=b 2=acc(H) 3=i 4=N 5=C(cold) 6=one
    let frame = vec![3i64, 4, 0, 0, 100_000, 0, 1];
    let depths = loop_depths(&ops);
    assert_eq!(depths[5], 1, "the accumulator update is inside the loop");
    let expected = reference_eval(&ops, &mut frame.clone(), 10_000_000).unwrap();
    let (out, post) = run(&ops, &frame);
    assert_eq!(
        out,
        ChainOutcome::Return(expected),
        "loop-weighted ranking must stay bit-identical"
    );
    let mut ref_frame = frame.clone();
    reference_eval(&ops, &mut ref_frame, 10_000_000).unwrap();
    assert_eq!(
        post, ref_frame,
        "frame diverged under loop-weighted ranking"
    );
}

/// Fix 1 (call-weight tie-break): two slots with the SAME primary loop-weight
/// must be ordered by call-survival — the slot LIVE ACROSS a SysV call ranks
/// first, so it wins a callee-saved register (paying no per-call spill/reload)
/// while the call-dead slot of equal weight takes a caller-saved one. The
/// call-weight is a SECONDARY key: it breaks the tie WITHOUT changing the
/// primary weights (so it can never evict a hotter loop slot).
#[test]
fn call_weight_breaks_tie_toward_live_across_call_slot() {
    // `survivor` (slot 1) and `transient` (slot 2) are each defined once and
    // read once — identical primary loop-weight. `survivor` is read AFTER the
    // call (op 3) → live across it; `transient` is consumed BEFORE the call
    // and never read after → dead across it. Only the secondary key differs.
    // 0: survivor = a + one    (def slot 1)
    // 1: transient = a + one   (def slot 2)
    // 2: sink = transient + a  (last read of transient, def slot 8 — BEFORE call)
    // 3: ListClear             (the SysV call, op 3)
    // 4: r = survivor + one    (reads slot 1 AFTER the call, def slot 4)
    // 5: Return r
    let ops = vec![
        MicroOp::Add {
            dst: 1,
            lhs: 0,
            rhs: 3,
        },
        MicroOp::Add {
            dst: 2,
            lhs: 0,
            rhs: 3,
        },
        MicroOp::Add {
            dst: 8,
            lhs: 2,
            rhs: 0,
        },
        MicroOp::ListClear {
            vec_slot: 5,
            ptr_slot: 6,
            len_slot: 7,
            helper_addr: 0,
        },
        MicroOp::Add {
            dst: 4,
            lhs: 1,
            rhs: 3,
        },
        MicroOp::Return { src: 4 },
    ];
    let max_slot = max_slot_of(&ops);
    let la = liveness_after(&ops, max_slot);
    assert!(la[3][1], "survivor must be live across the call");
    assert!(!la[3][2], "transient must be dead across the call");
    // PRIMARY weights must be equal (both: 1 def-ref + 1 use-ref outside any
    // loop). Slot 1: def@0 + use@4 = 2 refs. Slot 2: def@1 + use@2 = 2 refs.
    let (order, _) = rank_slots(&ops, 16, 4, 4, Some(&la));
    let pos = |slot: Slot| order.iter().position(|&(s, _)| s == slot).unwrap();
    let key = |slot: Slot| order.iter().find(|&&(s, _)| s == slot).unwrap().1;
    assert_eq!(
        key(1),
        key(2),
        "survivor and transient must share the SAME primary weight (got {} vs {})",
        key(1),
        key(2)
    );
    assert!(
        pos(1) < pos(2),
        "on equal primary weight the call-survivor must rank ahead of the call-dead slot"
    );
}

/// Fix 1 soundness: the call-survival key NEVER changes the primary
/// loop-weighted ordering of two slots with DIFFERENT primary weights, so it
/// cannot evict a hotter loop slot. A loop-hot, call-DEAD slot must still
/// out-rank a cold, call-SURVIVING slot.
#[test]
fn call_weight_never_outranks_a_hotter_loop_slot() {
    // `hot` (slot 2) is referenced every iteration of a loop (high primary
    // weight) but is consumed before the call → dead across it. `cold` (slot
    // 1) is set once outside the loop and read once after the call → live
    // across the call but cold. The loop-hot slot must still win.
    let ops = vec![
        MicroOp::Add {
            dst: 1,
            lhs: 0,
            rhs: 3,
        }, // 0 cold = a+one (setup)
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 4,
            rhs: 5,
            target: 6,
        }, // 1 while i<N
        MicroOp::Add {
            dst: 2,
            lhs: 2,
            rhs: 4,
        }, // 2 hot += i (HOT)
        MicroOp::Add {
            dst: 4,
            lhs: 4,
            rhs: 3,
        }, // 3 i += one (HOT)
        MicroOp::ListClear {
            vec_slot: 7,
            ptr_slot: 8,
            len_slot: 9,
            helper_addr: 0,
        }, // 4 call
        MicroOp::Jump { target: 1 }, // 5 back-edge
        MicroOp::Add {
            dst: 10,
            lhs: 1,
            rhs: 2,
        }, // 6 use cold (live across) + hot
    ];
    // Append a Return so the region is well-formed.
    let mut ops = ops;
    ops.push(MicroOp::Return { src: 10 });
    let max_slot = max_slot_of(&ops);
    let la = liveness_after(&ops, max_slot);
    let (order, _) = rank_slots(&ops, 16, 4, 4, Some(&la));
    let pos = |slot: Slot| order.iter().position(|&(s, _)| s == slot).unwrap();
    assert!(
        pos(2) < pos(1),
        "the loop-hot slot must out-rank the cold call-survivor (primary key dominates)"
    );
}

/// Fix 1 soundness: the call bonus is INERT for a call-free region. With
/// `live_after = None` the ranking is exactly the pre-Fix-1 loop-weighted
/// ranking — no bonus exists because there is no call to survive.
#[test]
fn call_weight_is_inert_without_a_call() {
    let ops = vec![
        MicroOp::Add {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Mul {
            dst: 4,
            lhs: 3,
            rhs: 2,
        },
        MicroOp::Sub {
            dst: 5,
            lhs: 4,
            rhs: 0,
        },
        MicroOp::Return { src: 5 },
    ];
    // With no liveness (call-free) and with an EMPTY all-dead liveness, the
    // bonus contributes nothing, so the two rankings are identical.
    let (no_call, _) = rank_slots(&ops, 16, 4, 4, None);
    let max_slot = max_slot_of(&ops);
    let dead = vec![vec![false; max_slot + 1]; ops.len()];
    let (dead_la, _) = rank_slots(&ops, 16, 4, 4, Some(&dead));
    assert_eq!(
        no_call, dead_la,
        "the call bonus must vanish with no live-across-call slot"
    );
}

/// Fix 1 (call-weight) must not perturb the SELF-CALL window forcing or the
/// `max_slot` extent: a region with a `CallSelf` still compiles, and the
/// ranking ranks the call-surviving non-window slot ahead of a transient. The
/// runnable end-to-end (bit-identical + tiers) lives in the integration suite
/// (`jit_regalloc::call_weight_*`) where a real recursive program is built.
#[test]
fn call_weight_self_call_region_compiles_and_ranks_survivor() {
    // A function-shaped region: guard, a self-call, an accumulate that reads a
    // value live across the call, return. (Window slots are forced frame-
    // resident regardless; this only checks the ranking + that it compiles.)
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 5,
        }, // 0 if !(i<N) -> ret
        MicroOp::CallSelf {
            dst: 3,
            args_start: 8,
            depth_addr: 0,
            status_addr: 0,
            limit_slot: 2,
            frame_size: 4,
        }, // 1 call
        MicroOp::Add {
            dst: 4,
            lhs: 4,
            rhs: 3,
        }, // 2 acc += call result (acc live across? no — defined here)
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 4,
        }, // 3 i += acc (reads acc 4 → live after the call)
        MicroOp::Jump { target: 0 }, // 4 back-edge
        MicroOp::Return { src: 4 },  // 5
    ];
    let max_slot = max_slot_of(&ops);
    assert!(
        max_slot >= 11,
        "self-call window extent (8+4-1=11) must be covered, got {max_slot}"
    );
    let la = liveness_after(&ops, max_slot);
    // slot 4 (acc) is read by op 3 after the call at op 1 → live across it.
    assert!(la[1][4], "acc must be live across the self-call");
    let (order, _) = rank_slots(&ops, 16, 4, 4, Some(&la));
    let key = |slot: Slot| {
        order
            .iter()
            .find(|&&(s, _)| s == slot)
            .map(|&(_, k)| k)
            .unwrap_or(0)
    };
    // slot 4 (call-survivor, loop-carried) must out-rank the bound slot 1
    // (referenced only at the cold guard, not across the call).
    assert!(
        key(4) > key(1),
        "call-surviving loop slot must out-rank a cold bound"
    );
}

/// Wave 16 (call-site spill liveness): `liveness_after` is a sound backward
/// dataflow. A slot WRITTEN before an op and READ only before it (never on any
/// path AFTER) is DEAD after that op; a slot read by a LATER op (including a
/// loop back-edge) is LIVE after every op on the path to that read.
#[test]
fn liveness_after_is_a_sound_backward_dataflow() {
    // 0: t = a + b      (defines t=2)
    // 1: dead = a * a   (defines dead=3, never read again)
    // 2: r = t - dead   (reads t and dead, defines r=4)
    // 3: Return r
    let ops = vec![
        MicroOp::Add {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Mul {
            dst: 3,
            lhs: 0,
            rhs: 0,
        },
        MicroOp::Sub {
            dst: 4,
            lhs: 2,
            rhs: 3,
        },
        MicroOp::Return { src: 4 },
    ];
    let la = liveness_after(&ops, 4);
    // After op 1 (`dead = a*a`), `dead` (slot 3) is LIVE (read by op 2) and so
    // is `t` (slot 2, read by op 2).
    assert!(
        la[1][3],
        "dead must be live right after it is defined (read by op 2)"
    );
    assert!(la[1][2], "t must still be live after op 1 (read by op 2)");
    // After op 2 (`r = t - dead`), NEITHER `t` nor `dead` is read again — both DEAD.
    assert!(!la[2][2], "t is dead after its last read (op 2)");
    assert!(!la[2][3], "dead is dead after its last read (op 2)");
    // `r` (slot 4) is live after op 2 (the Return reads it).
    assert!(la[2][4], "r is live after op 2 (Return reads it)");
    // After the Return, nothing is live.
    assert!(la[3].iter().all(|&b| !b), "no slot is live after Return");
}

/// A loop-carried slot read across a back-edge stays LIVE after every op in
/// the loop body (the backward dataflow must reach a fixpoint over the
/// back-edge, not just the forward pass).
#[test]
fn liveness_after_propagates_across_loop_back_edge() {
    // counting loop: 0=i 1=acc 2=N 3=one
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 2,
            target: 5,
        }, // 0 while i<N
        MicroOp::Add {
            dst: 1,
            lhs: 1,
            rhs: 0,
        }, // 1 acc += i
        MicroOp::LoadConst { dst: 3, value: 1 }, // 2 one = 1
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 3,
        }, // 3 i += 1
        MicroOp::Jump { target: 0 },             // 4 back-edge
        MicroOp::Return { src: 1 },              // 5
    ];
    let la = liveness_after(&ops, 3);
    // `i` (slot 0) is read by the guard (op 0), `acc += i` (op 1), and `i += 1`
    // (op 3); via the back-edge it must be live after op 3 (the increment),
    // after op 1, and after the guard's true edge — i.e. live around the loop.
    assert!(
        la[3][0],
        "i must be live after the increment (back-edge re-reads it)"
    );
    assert!(
        la[1][0],
        "i must be live after acc += i (used again at op 3)"
    );
    // `acc` (slot 1) is read by op 1 and the final Return → live around the loop.
    assert!(
        la[3][1],
        "acc must be live after op 3 (read next iteration / return)"
    );
    assert!(la[1][1], "acc must be live after its own update");
}

/// The spill-elision keep-rule: a caller-saved resident is elided around a
/// call ONLY when it is READ-ONLY (never written → never flushed at exit) AND
/// dead after the call. A WRITTEN slot is kept even when dead-after-call
/// (its last value is flushed at every region exit), and a read-only slot
/// read after the call is kept (the reload feeds that read). This mirrors the
/// `keep` closure in `compile_impl` and is the soundness contract the
/// `*_matches_stencil` post-frame differentials enforce.
#[test]
fn spill_elision_keeps_written_and_live_drops_readonly_dead() {
    // Slot 5 = `cold`: a frame input READ only at op 0 (never written, never
    //   read after the call) → read-only + dead-after-call → ELIDED.
    // Slot 8 = `hot`: WRITTEN at op 1, dead after the call (op 3 re-reads it,
    //   so it is actually live-after too — but the WRITTEN rule alone keeps
    //   it; flushed at every exit). Slot 1 = the literal-`1` operand: read
    //   only (never written) but READ after the call (op 3) → kept via live.
    // 0: t0 = cold + 1   (reads slot 5 `cold`, defines slot 7)
    // 1: hot = hot + 1   (WRITTEN slot 8)
    // 2: ListClear       (the call op, index 2)
    // 3: keep = hot + 1  (reads slot 8 `hot` and slot 1 `1`, defines slot 9)
    // 4: Return keep
    let ops = vec![
        MicroOp::Add {
            dst: 7,
            lhs: 5,
            rhs: 1,
        }, // 0 t0 = cold + 1
        MicroOp::Add {
            dst: 8,
            lhs: 8,
            rhs: 1,
        }, // 1 hot += 1 (written)
        MicroOp::ListClear {
            vec_slot: 2,
            ptr_slot: 3,
            len_slot: 4,
            helper_addr: 0,
        }, // 2 call
        MicroOp::Add {
            dst: 9,
            lhs: 8,
            rhs: 1,
        }, // 3 keep = hot + 1
        MicroOp::Return { src: 9 }, // 4
    ];
    let max_slot = 9;
    let la = liveness_after(&ops, max_slot);
    let mut written = vec![false; max_slot + 1];
    for op in &ops {
        if let Some(d) = dest_of(op) {
            written[d as usize] = true;
        }
    }
    let keep = |s: Slot| {
        written.get(s as usize).copied().unwrap_or(true)
            || la[2].get(s as usize).copied().unwrap_or(true)
    };
    // `cold` (slot 5): read-only (only ever an operand) and NOT read after the
    // call (op 3 reads `hot`/`1`, not `cold`) → ELIDED.
    assert!(!written[5], "cold is read-only");
    assert!(!la[2][5], "cold is dead after the call");
    assert!(
        !keep(5),
        "a read-only, dead-after-call resident must be elided"
    );
    // `hot` (slot 8): WRITTEN → kept regardless (flushed at every exit). It is
    // also live-after here, but the WRITTEN rule alone must keep it.
    assert!(written[8], "hot is written");
    assert!(keep(8), "a written resident must be kept (flushed at exit)");
    // `frame[1]` (the literal `1` operand): read-only but read AFTER the call
    // (op 3 reads it) → kept via the live-after rule.
    assert!(!written[1], "the constant operand is read-only");
    assert!(la[2][1], "the constant operand is read after the call");
    assert!(
        keep(1),
        "a read-only resident read after the call must be kept"
    );
}

// =================================================================
// WAVE 21: precise REGION backend (in-place SetIndex + reallocating
// ArrPush). The fannkuch / graph_bfs worklist shape — a precise region
// that does NOT have the mode-B function prologue, so `mode_b_rc` is
// inferred only for FUNCTIONS, never regions.
// =================================================================

/// The precise REGION path compiles a push+SetIndex stream (a reallocating
/// `ArrPush` beside an in-place checked `ArrStore`) into ONE contiguous
/// register-allocated chain — even though the stream has NO mode-B function
/// prologue (`LoadConst {-1}`). The per-piece precise stencil tier no longer
/// monopolizes this shape.
#[test]
fn precise_region_with_push_and_setindex_compiles() {
    // 0: i < n?  -> exit (op 6)
    // 1: ArrStore arr[i] = v  (in-place, CHECKED — the precise side exit)
    // 2: ArrPush v -> q       (reallocates q; helper refreshes ptr/len)
    // 3: i += 1
    // 4: Jump head
    // 5: (exit) Return i
    let ops = vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 5,
        },
        MicroOp::ArrStore {
            src: 2,
            idx: 0,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::ArrPush {
            src: 2,
            vec_slot: 8,
            ptr_slot: 9,
            len_slot: 10,
            helper_addr: 0,
            byte: false,
            narrow32: false,
        },
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 3,
        },
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 0 },
    ];
    // Per-op precise resume codes (parallel to `ops`): the checked store and
    // the push carry a precise tag `(pc << 2) | 3`; the rest stay plain `1`.
    let codes: Vec<i64> = ops
        .iter()
        .enumerate()
        .map(|(i, op)| match op {
            MicroOp::ArrStore { checked: true, .. } | MicroOp::ArrPush { .. } => {
                ((i as i64) << 2) | 3
            }
            _ => 1,
        })
        .collect();
    let status = Arc::new(AtomicI64::new(0));
    let chain = compile_region_regalloc_precise(&ops, Some(status), 0, &codes, &[]);
    assert!(
        chain.is_some(),
        "a precise push+SetIndex REGION (no mode-B prologue) must compile through \
             the contiguous regalloc backend"
    );
}

/// The precise REGION path DECLINES when the per-op `deopt_codes` length does
/// not match the op stream — a malformed call falls back to the per-piece tier
/// rather than mis-indexing the resume table.
#[test]
fn precise_region_declines_on_codes_length_mismatch() {
    let ops = vec![
        MicroOp::ArrStore {
            src: 2,
            idx: 0,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 0 },
    ];
    let codes = vec![1i64]; // one short
    let status = Arc::new(AtomicI64::new(0));
    assert!(
        compile_region_regalloc_precise(&ops, Some(status), 0, &codes, &[]).is_none(),
        "a deopt-codes length mismatch must decline"
    );
}

/// The precise REGION path requires the shared status channel (every checked
/// op's side exit stores its tagged resume value through it); a `None` status
/// declines.
#[test]
fn precise_region_declines_without_status() {
    let ops = vec![
        MicroOp::ArrStore {
            src: 2,
            idx: 0,
            ptr_slot: 6,
            len_slot: 7,
            byte: false,
            narrow32: false,
            checked: true,
        },
        MicroOp::Return { src: 0 },
    ];
    let codes = vec![3i64, 1];
    assert!(
        compile_region_regalloc_precise(&ops, None, 0, &codes, &[]).is_none(),
        "a precise region with no status channel must decline"
    );
}

/// A precise region may carry a CROSS-FUNCTION `Call`: it dispatches through the
/// callee's function-table slot (entry address and published register count), and
/// a deopt inside the callee propagates through the shared status channel, so the
/// region gate admits it (`jit_region_call_deopt_lock` holds a callee's deopt inside
/// such a region to the VM's output). The JIT adapter still keeps a region that
/// mixes such a call with list mutation off this backend. Without the status
/// channel there is nowhere for the callee's exit to land, and it declines.
#[test]
fn precise_region_compiles_a_cross_call_through_its_callee_table() {
    let ops = vec![
        MicroOp::Call {
            dst: 1,
            args_start: 4,
            table_addr: 0,
            depth_addr: 0,
            status_addr: 0,
            limit_slot: 3,
            depth_limit: SELF_CALL_DEPTH_LIMIT,
        },
        MicroOp::Return { src: 1 },
    ];
    let codes = vec![3i64, 1];
    let status = Arc::new(AtomicI64::new(0));
    assert!(
        compile_region_regalloc_precise(&ops, Some(status), 0, &codes, &[]).is_some(),
        "a precise region with a cross-function Call compiles through its callee's table slot"
    );
    assert!(
        compile_region_regalloc_precise(&ops, None, 0, &codes, &[]).is_none(),
        "a precise region with a cross-function Call and no status channel must decline"
    );
}

#[path = "host_calls.rs"]
mod host_calls;

/// A loop whose body reads `k` before its one in-loop write (`k = 5`). On the
/// first iteration the read sees the value `k` entered the loop with, so the
/// write cannot move to the pre-header: hoisted, the first read would see 5.
/// frame: 0=i 1=n 2=acc 3=k 4=one
fn read_before_write_loop() -> Vec<MicroOp> {
    vec![
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 6,
        }, // 0: while i < n
        MicroOp::Add {
            dst: 2,
            lhs: 2,
            rhs: 3,
        }, // 1: acc += k
        MicroOp::LoadConst { dst: 3, value: 5 }, // 2: k = 5
        MicroOp::LoadConst { dst: 4, value: 1 }, // 3: one = 1
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 4,
        }, // 4: i += one
        MicroOp::Jump { target: 0 },             // 5: back-edge
        MicroOp::Return { src: 2 },              // 6
    ]
}

#[test]
fn a_constant_read_before_its_write_in_the_loop_is_not_hoisted() {
    let ops = read_before_write_loop();
    let hoisted: Vec<usize> = loop_invariant_const_hoists(&ops, &[])
        .iter()
        .map(|h| h.op)
        .collect();
    assert_eq!(
        hoisted,
        vec![3],
        "only `one` is dead at the loop head; `k` is read there with its entry value"
    );
}

#[test]
fn a_loop_reading_a_constant_before_rewriting_it_runs_as_the_reference_does() {
    let ops = read_before_write_loop();
    for n in [0, 1, 2, 3, 10] {
        let frame = vec![0, n, 0, 100, 0];
        let mut reference = frame.clone();
        let expected = reference_eval(&ops, &mut reference, 10_000).expect("reference runs");
        let (out, _) = run(&ops, &frame);
        assert_eq!(out, ChainOutcome::Return(expected), "n = {n}");
    }
    let (out, _) = run(&ops, &[0, 3, 0, 100, 0]);
    assert_eq!(out, ChainOutcome::Return(110), "100 on the first pass, then 5 and 5");
}

/// A search loop whose hit path sets a flag and leaves the loop. The flag write
/// sits inside the loop's textual span but on a path that never reaches the
/// back-edge, and the caller reads the flag after the region exits. A miss must
/// leave the flag as the region set it before the loop.
/// frame: 0=i 1=n 2=m 3=flag 4=square 5=one
fn flag_on_the_way_out_loop() -> Vec<MicroOp> {
    vec![
        MicroOp::LoadConst { dst: 3, value: 0 }, // 0: flag = 0
        MicroOp::Branch {
            cmp: Cmp::Lt,
            lhs: 0,
            rhs: 1,
            target: 9,
        }, // 1: while i < n
        MicroOp::Mul {
            dst: 4,
            lhs: 0,
            rhs: 0,
        }, // 2: square = i * i
        MicroOp::Branch {
            cmp: Cmp::Eq,
            lhs: 4,
            rhs: 2,
            target: 6,
        }, // 3: if square = m
        MicroOp::LoadConst { dst: 3, value: 1 }, // 4: flag = 1
        MicroOp::Jump { target: 9 },             // 5: leave the loop
        MicroOp::LoadConst { dst: 5, value: 1 }, // 6: one = 1
        MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 5,
        }, // 7: i += one
        MicroOp::Jump { target: 1 },             // 8: back-edge
        MicroOp::Return { src: 0 },              // 9
    ]
}

#[test]
fn a_constant_the_caller_reads_after_the_exit_is_not_hoisted_off_its_path() {
    let ops = flag_on_the_way_out_loop();
    let hoisted: Vec<usize> = loop_invariant_const_hoists(&ops, &[3])
        .iter()
        .map(|h| h.op)
        .collect();
    assert_eq!(
        hoisted,
        vec![6],
        "the flag is live out of the region on the miss path; only `one` hoists"
    );
}

#[test]
fn a_region_leaves_every_live_out_slot_as_the_reference_does() {
    let ops = flag_on_the_way_out_loop();
    let live_out = [0u16, 3];
    for (n, m) in [(10, 49), (10, 50), (0, 0), (1, 0), (100, 64), (100, 65)] {
        let frame = vec![0, n, m, 7, 0, 0];
        let mut reference = frame.clone();
        let expected = reference_eval(&ops, &mut reference, 100_000).expect("reference runs");
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status), &live_out)
            .expect("supported region compiles");
        let mut native = frame.clone();
        let out = chain.run_with_frame(&mut native);
        assert_eq!(out, ChainOutcome::Return(expected), "n = {n}, m = {m}");
        for &s in &live_out {
            assert_eq!(
                native[s as usize], reference[s as usize],
                "live-out slot {s} after n = {n}, m = {m}"
            );
        }
    }
}

/// Liveness with exit uses: a `Return` reads every live-out slot, so a slot the
/// caller reads after the region is live on every path that reaches the exit
/// without writing it.
#[test]
fn a_live_out_slot_is_live_wherever_the_exit_can_still_be_reached_without_a_write() {
    let ops = flag_on_the_way_out_loop();
    let (live_in, _) = liveness(&ops, 5, &[3]);
    assert!(!live_in[0][3], "op 0 writes the flag before any exit");
    assert!(live_in[1][3], "the loop head reaches the exit through the miss path");
    assert!(!live_in[4][3], "the hit path writes the flag before it leaves");
    let (live_in, _) = liveness(&ops, 5, &[]);
    assert!(!live_in[1][3], "with nothing live out, the flag is never read");
}
