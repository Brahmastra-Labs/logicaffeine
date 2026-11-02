//! The lane machine: each lane operation does exactly what its definition says, at every lane shape a device
//! declares; a fixed-count `Repeat` runs its body that many times; the interpreter refuses a routine that
//! would use a unit the shape lacks (a multiplicand wider than the multiplier, a word wider than the lane) or
//! a carry that is not a bit; and every operation is charged once to the cost counts.

use logicaffeine_compile::accel::lane::{
    join_u128, join_u64, run, split_u128, split_u64, ExactLanes, LInst, LOp, LStmt, LaneCounts,
    LaneFault, LaneShape, Routine,
};

/// Every shape the three devices declare, and the narrow ones between.
fn shapes() -> Vec<LaneShape> {
    vec![
        LaneShape::new(2, 1),
        LaneShape::new(4, 2),
        LaneShape::new(8, 4),
        LaneShape::new(16, 8),
        LaneShape::new(32, 16),
        LaneShape::new(32, 8),
    ]
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Words of `shape`: 0, 1, the top word, the half word, and random words.
fn words(shape: LaneShape) -> Vec<u32> {
    let m = shape.mask();
    let mut v = vec![0, 1, m, m >> 1, (m >> 1) + 1, m - 1];
    let mut s = 0xA5A5_5A5A_1234_5678 ^ shape.word as u64;
    for _ in 0..12 {
        v.push(splitmix(&mut s) as u32 & m);
    }
    v.sort_unstable();
    v.dedup();
    v
}

/// A one-instruction routine over three inputs `r0, r1, r2`, writing `r3` (and `r4` for a carry).
fn one(shape: LaneShape, op: LOp, carry: bool) -> Routine {
    let dst2 = carry.then_some(4);
    Routine {
        name: format!("{op:?}"),
        shape,
        regs: 5,
        inputs: vec![0, 1, 2],
        outputs: if carry { vec![3, 4] } else { vec![3] },
        body: vec![LStmt::Inst(LInst { dst: 3, dst2, op })],
    }
}

fn run_one(shape: LaneShape, op: LOp, carry: bool, a: u32, b: u32, c: u32) -> Vec<u32> {
    let mut lanes = ExactLanes { shape };
    run(&one(shape, op, carry), &mut lanes, &[a, b, c])
        .unwrap_or_else(|e| panic!("{op:?} at {shape:?}: {e}"))
        .0
}

#[test]
fn each_lane_op_runs_as_defined_at_every_shape() {
    let mut bad = Vec::new();
    for shape in shapes() {
        let (w, m) = (shape.word, shape.mask());
        let ws = words(shape);
        let digit = (1u32 << shape.mul) - 1;
        for &a in &ws {
            for &b in &ws {
                for cin in [0u32, 1] {
                    let s = a as u64 + b as u64 + cin as u64;
                    let want = vec![(s & m as u64) as u32, (s >> w) as u32];
                    let got = run_one(shape, LOp::Add { a: 0, b: 1, cin: 2 }, true, a, b, cin);
                    if got != want {
                        bad.push(format!(
                            "{shape:?} Add({a:#x}, {b:#x}, {cin}): got {got:x?}, want {want:x?}"
                        ));
                    }
                    let d = a as i64 - b as i64 - cin as i64;
                    let want = vec![d.rem_euclid(1i64 << w) as u32, u32::from(d < 0)];
                    let got = run_one(shape, LOp::Sub { a: 0, b: 1, bin: 2 }, true, a, b, cin);
                    if got != want {
                        bad.push(format!(
                            "{shape:?} Sub({a:#x}, {b:#x}, {cin}): got {got:x?}, want {want:x?}"
                        ));
                    }
                }
                let (da, db) = (a & digit, b & digit);
                let checks: [(LOp, u32); 8] = [
                    (LOp::Mul { a: 0, b: 1 }, da * db),
                    (LOp::And { a: 0, b: 1 }, a & b),
                    (LOp::Or { a: 0, b: 1 }, a | b),
                    (LOp::Xor { a: 0, b: 1 }, a ^ b),
                    (LOp::Not { a: 0 }, !a & m),
                    (LOp::Ltu { a: 0, b: 1 }, u32::from(a < b)),
                    (LOp::Eq { a: 0, b: 1 }, u32::from(a == b)),
                    (LOp::Mov(1), b),
                ];
                for (op, want) in checks {
                    let (x, y) = if matches!(op, LOp::Mul { .. }) {
                        (da, db)
                    } else {
                        (a, b)
                    };
                    let got = run_one(shape, op, false, x, y, 0)[0];
                    if got != want {
                        bad.push(format!(
                            "{shape:?} {op:?} on ({x:#x}, {y:#x}): got {got:#x}, want {want:#x}"
                        ));
                    }
                }
                for c in [0u32, 1, m] {
                    let got = run_one(shape, LOp::Sel { c: 2, t: 0, f: 1 }, false, a, b, c)[0];
                    let want = if c != 0 { a } else { b };
                    if got != want {
                        bad.push(format!(
                            "{shape:?} Sel({c:#x}, {a:#x}, {b:#x}): got {got:#x}, want {want:#x}"
                        ));
                    }
                }
            }
            for k in 1..w {
                let got = run_one(shape, LOp::Shl { a: 0, k }, false, a, 0, 0)[0];
                if got != (a << k) & m {
                    bad.push(format!("{shape:?} Shl({a:#x}, {k}): got {got:#x}"));
                }
                let got = run_one(shape, LOp::Shr { a: 0, k }, false, a, 0, 0)[0];
                if got != a >> k {
                    bad.push(format!("{shape:?} Shr({a:#x}, {k}): got {got:#x}"));
                }
            }
            let got = run_one(shape, LOp::Const(a), false, 0, 0, 0)[0];
            if got != a {
                bad.push(format!("{shape:?} Const({a:#x}): got {got:#x}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} lane operations differ from their definitions:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn a_repeat_runs_its_body_the_stated_number_of_times() {
    let shape = LaneShape::new(32, 16);
    for times in [0u32, 1, 2, 7, 64, 1000] {
        // r0 counts; r1 is the constant 1; r2 is the constant 0 carry.
        let routine = Routine {
            name: "count".into(),
            shape,
            regs: 4,
            inputs: vec![],
            outputs: vec![0],
            body: vec![
                LStmt::Inst(LInst {
                    dst: 0,
                    dst2: None,
                    op: LOp::Const(0),
                }),
                LStmt::Inst(LInst {
                    dst: 1,
                    dst2: None,
                    op: LOp::Const(1),
                }),
                LStmt::Inst(LInst {
                    dst: 2,
                    dst2: None,
                    op: LOp::Const(0),
                }),
                LStmt::Repeat {
                    times,
                    body: vec![LStmt::Inst(LInst {
                        dst: 0,
                        dst2: Some(3),
                        op: LOp::Add { a: 0, b: 1, cin: 2 },
                    })],
                },
            ],
        };
        let mut lanes = ExactLanes { shape };
        let (out, counts) = run(&routine, &mut lanes, &[]).expect("the counter runs");
        assert_eq!(
            out,
            vec![times],
            "a {times}-trip repeat adds one {times} times"
        );
        assert_eq!(counts.add, times as u64, "each trip charges one add");
    }
}

#[test]
fn the_interpreter_refuses_what_the_shape_lacks() {
    let shape = LaneShape::new(8, 4);
    let mut lanes = ExactLanes { shape };
    let mul = one(shape, LOp::Mul { a: 0, b: 1 }, false);
    assert_eq!(
        run(&mul, &mut lanes, &[16, 3, 0]),
        Err(LaneFault::WideMultiplicand { value: 16 }),
        "a 5-bit operand on a 4-bit multiplier"
    );
    let add = one(shape, LOp::Add { a: 0, b: 1, cin: 2 }, true);
    assert_eq!(
        run(&add, &mut lanes, &[1, 1, 2]),
        Err(LaneFault::NotABit { value: 2 }),
        "a carry-in of 2"
    );
    assert_eq!(
        run(&add, &mut lanes, &[256, 1, 0]),
        Err(LaneFault::WideWord {
            op: "input",
            value: 256
        }),
        "a 9-bit input word"
    );
    assert_eq!(
        run(&add, &mut lanes, &[1, 1]),
        Err(LaneFault::Arity {
            expected: 3,
            got: 2
        }),
        "two words for three inputs"
    );
    let wide_const = Routine {
        name: "wide".into(),
        shape,
        regs: 1,
        inputs: vec![],
        outputs: vec![0],
        body: vec![LStmt::Inst(LInst {
            dst: 0,
            dst2: None,
            op: LOp::Const(0x1FF),
        })],
    };
    assert_eq!(
        run(&wide_const, &mut lanes, &[]),
        Err(LaneFault::WideWord {
            op: "const",
            value: 0x1FF
        }),
        "a 9-bit constant"
    );
}

#[test]
fn every_operation_is_charged_once() {
    let shape = LaneShape::new(16, 8);
    let ops: [(LOp, bool, fn(&LaneCounts) -> u64); 13] = [
        (LOp::Const(3), false, |c| c.moves),
        (LOp::Mov(0), false, |c| c.moves),
        (LOp::Add { a: 0, b: 1, cin: 2 }, true, |c| c.add),
        (LOp::Sub { a: 0, b: 1, bin: 2 }, true, |c| c.sub),
        (LOp::Mul { a: 0, b: 1 }, false, |c| c.mul),
        (LOp::And { a: 0, b: 1 }, false, |c| c.logic),
        (LOp::Or { a: 0, b: 1 }, false, |c| c.logic),
        (LOp::Xor { a: 0, b: 1 }, false, |c| c.logic),
        (LOp::Not { a: 0 }, false, |c| c.logic),
        (LOp::Shl { a: 0, k: 3 }, false, |c| c.shift),
        (LOp::Ltu { a: 0, b: 1 }, false, |c| c.compare),
        (LOp::Eq { a: 0, b: 1 }, false, |c| c.compare),
        (LOp::Sel { c: 2, t: 0, f: 1 }, false, |c| c.select),
    ];
    for (op, carry, field) in ops {
        let mut lanes = ExactLanes { shape };
        let (_, counts) = run(&one(shape, op, carry), &mut lanes, &[5, 7, 1]).expect("runs");
        assert_eq!(field(&counts), 1, "{op:?} charges its own counter once");
        assert_eq!(counts.total(), 1, "{op:?} charges nothing else");
    }
}

#[test]
fn values_split_into_lane_words_and_join_back() {
    let mut s = 0x5EED_u64;
    for shape in shapes() {
        let n = shape.words_for(64);
        for _ in 0..200 {
            let x = splitmix(&mut s);
            let parts = split_u64(shape, x, n);
            assert_eq!(parts.len(), n);
            assert!(
                parts.iter().all(|p| p & !shape.mask() == 0),
                "{shape:?}: every part fits a word"
            );
            assert_eq!(join_u64(shape, &parts), x, "{shape:?}: {x:#x} round-trips");
            let y = (x as u128) << 64 | splitmix(&mut s) as u128;
            let n2 = shape.words_for(128);
            assert_eq!(
                join_u128(shape, &split_u128(shape, y, n2)),
                y,
                "{shape:?}: {y:#x} round-trips"
            );
        }
        assert_eq!(split_u64(shape, 1, n)[0], 1, "{shape:?}: little-endian");
    }
}
