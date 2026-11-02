//! The VM's 64-bit integer cells on narrow lanes: every integer cell routine, at every lane shape, gives
//! exactly the checked 64-bit semantics (`AluOp`/`FuncUnit::eval_checked`, `CmpKind::eval`), which
//! `fabric_checked_semantics_lock` holds to the VM's arithmetic kernel. Where the checked semantics names a
//! divergence, the routine's fault word carries that divergence's code; otherwise the fault word is 0 and the
//! 64-bit result is the VM's. No routine may touch a unit its shape lacks: the lane interpreter refuses one.

use logicaffeine_compile::accel::cells::{all_ops, routine, CellOp};
use logicaffeine_compile::accel::lane::{join_u64, run, split_u64, ExactLanes, LaneShape};
use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, CmpKind, Divergence, FuncUnit};

/// A shape and the operand battery its routines are swept over: wide shapes take the whole battery, narrow
/// ones (whose routines run hundreds of times more lane operations) take the edges and a few random values.
fn shapes() -> Vec<(LaneShape, usize)> {
    vec![
        (LaneShape::new(32, 16), 64),
        (LaneShape::new(32, 8), 48),
        (LaneShape::new(16, 8), 28),
        (LaneShape::new(8, 4), 20),
        (LaneShape::new(4, 2), 14),
        (LaneShape::new(2, 1), 11),
    ]
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The first `count` of: the edges every 64-bit datapath has, then random words of every magnitude.
fn ints(count: usize) -> Vec<i64> {
    let mut v: Vec<i64> = vec![
        0,
        1,
        -1,
        i64::MAX,
        i64::MIN,
        2,
        -2,
        i64::MIN + 1,
        3_037_000_500,
        -3_037_000_499,
        63,
        64,
        7,
        -7,
        1 << 32,
        (1 << 32) - 1,
        i32::MIN as i64,
        i64::MAX - 1,
        1 << 62,
        -(1 << 62),
    ];
    let mut s = 0x1357_9BDF_2468_ACE0;
    while v.len() < count.max(20) {
        v.push(splitmix(&mut s) as i64);
        let k = splitmix(&mut s) % 62;
        v.push((splitmix(&mut s) as i64) >> k);
    }
    v.truncate(count);
    v
}

/// The exponents a power is swept over: negative, small, the overflow boundaries, and past `u32`.
fn exponents(count: usize) -> Vec<i64> {
    let all = [
        0i64,
        1,
        2,
        -1,
        63,
        3,
        62,
        64,
        i64::from(u32::MAX) + 1,
        5,
        -7,
        10,
        31,
        32,
        40,
        4,
        6,
        16,
        61,
        65,
    ];
    all.iter()
        .copied()
        .take(count.clamp(9, all.len()))
        .collect()
}

/// The checked 64-bit meaning of `op`, as the routine must report it.
fn checked(op: CellOp, a: i64, b: i64) -> Result<i64, Divergence> {
    match op {
        CellOp::Alu(alu) => alu.eval_checked(a, b),
        CellOp::Unit(u) => u.eval_checked(a, b),
        CellOp::Select(c) => Ok(i64::from(c.eval(a, b))),
    }
}

/// Run `op`'s routine on `(a, b)`; a select runs with the arms `1` and `0`, so its value is the predicate.
fn device(shape: LaneShape, op: CellOp, a: i64, b: i64) -> Result<i64, Divergence> {
    let r = routine(shape, op);
    let n = shape.words_for(64);
    let mut args: Vec<u32> = split_u64(shape, a as u64, n);
    args.extend(split_u64(shape, b as u64, n));
    if let CellOp::Select(_) = op {
        args.extend(split_u64(shape, 1, n));
        args.extend(split_u64(shape, 0, n));
    }
    let mut lanes = ExactLanes { shape };
    let (out, _) =
        run(&r, &mut lanes, &args).unwrap_or_else(|e| panic!("{shape:?} {}: {e}", r.name));
    let value = join_u64(shape, &out[..n]) as i64;
    if !op.faults() {
        return Ok(value);
    }
    let code = join_u64(shape, &out[n..]) as u32;
    match code {
        0 => Ok(value),
        c => Err(Divergence::from_code(c)
            .unwrap_or_else(|| panic!("{shape:?} {}: fault code {c} names no divergence", r.name))),
    }
}

fn integer_ops() -> Vec<CellOp> {
    all_ops()
        .into_iter()
        .filter(|op| match op {
            CellOp::Alu(_) | CellOp::Select(_) => true,
            CellOp::Unit(u) => matches!(
                u,
                FuncUnit::DivS
                    | FuncUnit::ModS
                    | FuncUnit::FloorDivS
                    | FuncUnit::Pow
                    | FuncUnit::MulHiU
                    | FuncUnit::MulOvfS
                    | FuncUnit::PopCount
            ),
        })
        .collect()
}

#[test]
fn every_integer_cell_routine_is_the_checked_semantics_at_every_shape() {
    let mut bad = Vec::new();
    for (shape, count) in shapes() {
        let xs = ints(count);
        for op in integer_ops() {
            let ys = if op == CellOp::Unit(FuncUnit::Pow) {
                exponents(count)
            } else {
                xs.clone()
            };
            let ys = if op == CellOp::Alu(AluOp::UShr)
                || op == CellOp::Alu(AluOp::Shl)
                || op == CellOp::Alu(AluOp::Shr)
            {
                ys.into_iter()
                    .chain([0, 1, 31, 32, 63, 64, -1, 127])
                    .collect()
            } else {
                ys
            };
            for &a in &xs {
                for &b in &ys {
                    let want = checked(op, a, b);
                    let got = device(shape, op, a, b);
                    let agree = match (got, want) {
                        (Ok(g), Ok(w)) => g == w,
                        (Err(g), Err(w)) => g == w,
                        _ => false,
                    };
                    if !agree {
                        bad.push(format!(
                            "{shape:?} {}({a:#x}, {b:#x}): lanes {got:?}, checked {want:?}",
                            op.name()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} integer cell runs differ from the checked semantics:\n{}",
        bad.len(),
        bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// A select chooses its arm by the integer comparison, for arms that are not the predicate itself.
#[test]
fn a_select_routes_the_chosen_arm() {
    for (shape, _) in shapes() {
        let n = shape.words_for(64);
        for c in [
            CmpKind::Lt,
            CmpKind::Gt,
            CmpKind::Le,
            CmpKind::Ge,
            CmpKind::Eq,
            CmpKind::Ne,
        ] {
            let r = routine(shape, CellOp::Select(c));
            for (a, b) in [(1i64, 2i64), (2, 1), (5, 5), (-3, 3), (i64::MIN, i64::MAX)] {
                let (t, e) = (0x1122_3344_5566_7788u64, 0x8877_6655_4433_2211u64);
                let args: Vec<u32> = [a as u64, b as u64, t, e]
                    .iter()
                    .flat_map(|v| split_u64(shape, *v, n))
                    .collect();
                let mut lanes = ExactLanes { shape };
                let (out, _) = run(&r, &mut lanes, &args).expect("the select runs");
                let want = if c.eval(a, b) { t } else { e };
                assert_eq!(
                    join_u64(shape, &out),
                    want,
                    "{shape:?} {c:?}({a}, {b}) picks its arm"
                );
            }
        }
    }
}

/// Every functional unit's place in declaration order: a total match, so a new unit does not compile here
/// until the catalog test knows it.
fn unit_ordinal(u: FuncUnit) -> usize {
    match u {
        FuncUnit::DivS => 0,
        FuncUnit::ModS => 1,
        FuncUnit::FloorDivS => 2,
        FuncUnit::Pow => 3,
        FuncUnit::MulHiU => 4,
        FuncUnit::ApproxEqF => 5,
        FuncUnit::FAddF => 6,
        FuncUnit::FSubF => 7,
        FuncUnit::FMulF => 8,
        FuncUnit::FDivF => 9,
        FuncUnit::FSqrtF => 10,
        FuncUnit::FFloorF => 11,
        FuncUnit::MulOvfS => 12,
        FuncUnit::IntToF => 13,
        FuncUnit::FLtF => 14,
        FuncUnit::FGtF => 15,
        FuncUnit::FLeF => 16,
        FuncUnit::FGeF => 17,
        FuncUnit::FEqF => 18,
        FuncUnit::FNeF => 19,
        FuncUnit::PopCount => 20,
    }
}

fn alu_ordinal(op: AluOp) -> usize {
    match op {
        AluOp::Add => 0,
        AluOp::Sub => 1,
        AluOp::Mul => 2,
        AluOp::And => 3,
        AluOp::Or => 4,
        AluOp::Xor => 5,
        AluOp::Shl => 6,
        AluOp::Shr => 7,
        AluOp::UShr => 8,
    }
}

fn cmp_ordinal(c: CmpKind) -> usize {
    match c {
        CmpKind::Lt => 0,
        CmpKind::Gt => 1,
        CmpKind::Le => 2,
        CmpKind::Ge => 3,
        CmpKind::Eq => 4,
        CmpKind::Ne => 5,
    }
}

/// The catalog names every cell op exactly once: all 9 ALU ops, all 21 functional units, all 6 comparisons.
#[test]
fn the_catalog_has_every_cell_op() {
    let ops = all_ops();
    let mut alu: Vec<usize> = Vec::new();
    let mut units: Vec<usize> = Vec::new();
    let mut cmps: Vec<usize> = Vec::new();
    for op in &ops {
        match op {
            CellOp::Alu(a) => alu.push(alu_ordinal(*a)),
            CellOp::Unit(u) => units.push(unit_ordinal(*u)),
            CellOp::Select(c) => cmps.push(cmp_ordinal(*c)),
        }
    }
    for v in [&mut alu, &mut units, &mut cmps] {
        v.sort_unstable();
    }
    assert_eq!(alu, (0..9).collect::<Vec<_>>(), "every ALU op once");
    assert_eq!(
        units,
        (0..21).collect::<Vec<_>>(),
        "every functional unit once"
    );
    assert_eq!(cmps, (0..6).collect::<Vec<_>>(), "every comparison once");
}
