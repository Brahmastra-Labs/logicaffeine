//! The fabric's fixed-width cells held to the VM's arithmetic kernel, value for value.
//!
//! A fabric cell computes a 64-bit word; the VM computes an exact value that promotes past `i64`, fails on a
//! zero divisor and refuses a negative integer exponent. `eval_checked` is the reconciliation: for every
//! operand pair it either returns exactly the word the kernel's result is (an `Int`, a `Bool` as 0/1, a
//! `Float` as its bit pattern), or names the [`Divergence`] the kernel's result shows. Each unit is paired with
//! the kernel entry the VM itself calls for that op (`semantics::arith`, `semantics::compare`, the builtins),
//! through total matches over `AluOp`, `FuncUnit` and `CmpKind`, so a new unit does not compile here until it
//! has a kernel counterpart and a verdict.

use logicaffeine_compile::ast::stmt::BinaryOpKind;
use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, CmpKind, Divergence, FuncUnit};
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::builtins::{call_builtin, BuiltinId};
use logicaffeine_compile::semantics::{arith, compare};

/// What the kernel produced, as the word a cell would carry or the reason it cannot carry one.
#[derive(Debug)]
enum Kernel {
    Word(i64),
    Promoted,
    Failed(String),
}

fn kernel(r: Result<RuntimeValue, String>) -> Kernel {
    match r {
        Ok(RuntimeValue::Int(n)) => Kernel::Word(n),
        Ok(RuntimeValue::Bool(b)) => Kernel::Word(i64::from(b)),
        Ok(RuntimeValue::Float(f)) => Kernel::Word(f.to_bits() as i64),
        Ok(RuntimeValue::Word(w)) => Kernel::Word(w.to_u64() as i64),
        Ok(RuntimeValue::BigInt(_)) => Kernel::Promoted,
        Ok(other) => panic!(
            "the kernel returned a {} where a cell carries a word",
            other.type_name()
        ),
        Err(e) => Kernel::Failed(e),
    }
}

fn verdict_agrees(got: Result<i64, Divergence>, want: &Kernel) -> bool {
    match (got, want) {
        (Ok(g), Kernel::Word(w)) => g == *w,
        (Err(Divergence::IntOverflow), Kernel::Promoted) => true,
        (Err(Divergence::DivideByZero), Kernel::Failed(m)) => {
            m == "Division by zero" || m == "Modulo by zero"
        }
        (Err(Divergence::FloatDivideByZero), Kernel::Failed(m)) => m == "Division by zero",
        (Err(Divergence::NegativeExponent), Kernel::Failed(m)) => {
            m.starts_with("negative exponent")
        }
        (Err(Divergence::ExponentTooLarge), Kernel::Failed(m)) => {
            m.starts_with("exponent too large")
        }
        _ => false,
    }
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The integer operands: every edge a 64-bit datapath has (zero, the units, the i32 and i64 boundaries and
/// their neighbours, the square-root-of-overflow pair, shift counts around the word size), then random words
/// of every magnitude.
fn ints() -> Vec<i64> {
    let mut v: Vec<i64> = vec![
        0,
        1,
        -1,
        2,
        -2,
        3,
        -3,
        7,
        -7,
        10,
        31,
        32,
        33,
        63,
        64,
        65,
        -63,
        -64,
        100,
        -100,
        255,
        256,
        i32::MAX as i64,
        i32::MIN as i64,
        (1 << 31) + 1,
        1 << 32,
        (1 << 32) - 1,
        -(1 << 32),
        1 << 53,
        (1 << 53) + 1,
        1 << 62,
        -(1 << 62),
        3_037_000_499,
        3_037_000_500,
        -3_037_000_499,
        -3_037_000_500,
        i64::MAX,
        i64::MAX - 1,
        i64::MIN,
        i64::MIN + 1,
    ];
    let mut s = 0x0123_4567_89AB_CDEF;
    for _ in 0..24 {
        v.push(splitmix(&mut s) as i64);
    }
    for _ in 0..24 {
        let bits = splitmix(&mut s) % 40;
        v.push((splitmix(&mut s) as i64) >> (24 + bits));
    }
    v
}

/// The float operands: signed zeros, subnormals, the finite extremes, infinities, NaN, the integers past
/// 2^53, halves and thirds, then random bit patterns.
fn floats() -> Vec<f64> {
    let mut v: Vec<f64> = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        0.1,
        -0.1,
        1.0 / 3.0,
        2.0,
        2.5,
        -2.5,
        3.5,
        123_456.789,
        9_007_199_254_740_992.0,
        9_007_199_254_740_994.0,
        9_223_372_036_854_775_808.0,
        -9_223_372_036_854_775_808.0,
        1e300,
        -1e300,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        f64::MIN_POSITIVE / 2.0,
        5e-324,
        -5e-324,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    let mut s = 0xFEDC_BA98_7654_3210;
    for _ in 0..24 {
        v.push(f64::from_bits(splitmix(&mut s)));
    }
    v
}

fn int(n: i64) -> RuntimeValue {
    RuntimeValue::Int(n)
}

fn float(bits: i64) -> RuntimeValue {
    RuntimeValue::Float(f64::from_bits(bits as u64))
}

fn fbits(f: f64) -> i64 {
    f.to_bits() as i64
}

/// Every pair of `xs`, the unit's checked word against the kernel's verdict; the disagreements, named.
fn sweep(
    label: &str,
    xs: &[i64],
    ys: &[i64],
    checked: &dyn Fn(i64, i64) -> Result<i64, Divergence>,
    vm: &dyn Fn(i64, i64) -> Kernel,
) -> Vec<String> {
    let mut bad = Vec::new();
    for &a in xs {
        for &b in ys {
            let got = checked(a, b);
            let want = vm(a, b);
            if !verdict_agrees(got, &want) {
                bad.push(format!(
                    "{label}({a:#x}, {b:#x}): checked {got:?}, kernel {want:?}"
                ));
            }
        }
    }
    bad
}

fn alu_kernel(op: AluOp) -> (&'static str, Box<dyn Fn(i64, i64) -> Kernel>) {
    match op {
        AluOp::Add => ("Add", Box::new(|a, b| kernel(arith::add(int(a), int(b))))),
        AluOp::Sub => (
            "Sub",
            Box::new(|a, b| kernel(arith::subtract(int(a), int(b)))),
        ),
        AluOp::Mul => (
            "Mul",
            Box::new(|a, b| kernel(arith::multiply(int(a), int(b)))),
        ),
        AluOp::And => (
            "And",
            Box::new(|a, b| kernel(arith::binary_op(BinaryOpKind::BitAnd, int(a), int(b)))),
        ),
        AluOp::Or => (
            "Or",
            Box::new(|a, b| kernel(arith::binary_op(BinaryOpKind::BitOr, int(a), int(b)))),
        ),
        AluOp::Xor => (
            "Xor",
            Box::new(|a, b| kernel(arith::binary_op(BinaryOpKind::BitXor, int(a), int(b)))),
        ),
        AluOp::Shl => (
            "Shl",
            Box::new(|a, b| kernel(arith::binary_op(BinaryOpKind::Shl, int(a), int(b)))),
        ),
        AluOp::Shr => (
            "Shr",
            Box::new(|a, b| kernel(arith::binary_op(BinaryOpKind::Shr, int(a), int(b)))),
        ),
        // The Word64 lane's zero-filling shift: the kernel's `Word64` shift, over the counts a word holds.
        AluOp::UShr => (
            "UShr",
            Box::new(|a, b| {
                let w = logicaffeine_base::word::WordVal::from_u64(64, a as u64)
                    .expect("a 64-bit word");
                kernel(arith::binary_op(
                    BinaryOpKind::Shr,
                    RuntimeValue::Word(w),
                    int(b),
                ))
            }),
        ),
    }
}

/// Every ALU op, in declaration order; `alu_index` is total, so a new op does not compile until it has an
/// index, and `every_alu_op_is_swept` fails until this list carries it.
const ALU_OPS: [AluOp; 9] = [
    AluOp::Add,
    AluOp::Sub,
    AluOp::Mul,
    AluOp::And,
    AluOp::Or,
    AluOp::Xor,
    AluOp::Shl,
    AluOp::Shr,
    AluOp::UShr,
];

fn alu_index(op: AluOp) -> usize {
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

#[test]
fn every_alu_op_is_swept() {
    let mut seen: Vec<usize> = ALU_OPS.iter().map(|op| alu_index(*op)).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen,
        (0..ALU_OPS.len()).collect::<Vec<_>>(),
        "ALU_OPS names every op exactly once"
    );
}

#[test]
fn every_alu_op_is_the_kernel_or_names_its_divergence() {
    let xs = ints();
    let mut bad = Vec::new();
    for op in ALU_OPS {
        let (label, vm) = alu_kernel(op);
        let ys: Vec<i64> = if op == AluOp::UShr {
            (0..64).collect()
        } else {
            xs.clone()
        };
        bad.extend(sweep(label, &xs, &ys, &|a, b| op.eval_checked(a, b), &*vm));
    }
    assert!(
        bad.is_empty(),
        "{} ALU verdicts disagree with the kernel:\n{}",
        bad.len(),
        bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// The operand domain a unit reads: integers by value, or floats as bit patterns.
#[derive(Clone, Copy, PartialEq)]
enum Domain {
    Ints,
    Floats,
    /// An integer base and the exponents the kernel can raise it to without building an astronomically
    /// large `BigInt` (the promotion itself is what is under test, not its size).
    PowExponents,
}

fn unit_kernel(unit: FuncUnit) -> (&'static str, Domain, Box<dyn Fn(i64, i64) -> Kernel>) {
    let fcmp = |op: BinaryOpKind| -> Box<dyn Fn(i64, i64) -> Kernel> {
        Box::new(move |a, b| kernel(compare::compare(op, &float(a), &float(b))))
    };
    match unit {
        FuncUnit::DivS => (
            "DivS",
            Domain::Ints,
            Box::new(|a, b| kernel(arith::divide(int(a), int(b)))),
        ),
        FuncUnit::ModS => (
            "ModS",
            Domain::Ints,
            Box::new(|a, b| kernel(arith::modulo(int(a), int(b)))),
        ),
        FuncUnit::FloorDivS => (
            "FloorDivS",
            Domain::Ints,
            Box::new(|a, b| kernel(arith::floor_divide(int(a), int(b)))),
        ),
        FuncUnit::Pow => (
            "Pow",
            Domain::PowExponents,
            Box::new(|a, b| kernel(arith::power(int(a), int(b)))),
        ),
        // The optimizer's magic-division step has no source op of its own: it is the high word of the
        // unsigned 128-bit product, exact by definition.
        FuncUnit::MulHiU => (
            "MulHiU",
            Domain::Ints,
            Box::new(|a, b| Kernel::Word((((a as u64 as u128) * (b as u64 as u128)) >> 64) as i64)),
        ),
        FuncUnit::ApproxEqF => (
            "ApproxEqF",
            Domain::Ints,
            Box::new(|a, b| kernel(arith::approx_eq(int(a), int(b)))),
        ),
        FuncUnit::FAddF => (
            "FAddF",
            Domain::Floats,
            Box::new(|a, b| kernel(arith::add(float(a), float(b)))),
        ),
        FuncUnit::FSubF => (
            "FSubF",
            Domain::Floats,
            Box::new(|a, b| kernel(arith::subtract(float(a), float(b)))),
        ),
        FuncUnit::FMulF => (
            "FMulF",
            Domain::Floats,
            Box::new(|a, b| kernel(arith::multiply(float(a), float(b)))),
        ),
        FuncUnit::FDivF => (
            "FDivF",
            Domain::Floats,
            Box::new(|a, b| kernel(arith::divide(float(a), float(b)))),
        ),
        FuncUnit::FSqrtF => (
            "FSqrtF",
            Domain::Floats,
            Box::new(|a, _| kernel(call_builtin(BuiltinId::Sqrt, vec![float(a)]))),
        ),
        FuncUnit::FFloorF => (
            "FFloorF",
            Domain::Floats,
            Box::new(|a, _| kernel(call_builtin(BuiltinId::Floor, vec![float(a)]))),
        ),
        // The BigIntWide lane's promotion test: 1 exactly when the kernel's product is a BigInt.
        FuncUnit::MulOvfS => (
            "MulOvfS",
            Domain::Ints,
            Box::new(|a, b| {
                Kernel::Word(i64::from(matches!(
                    arith::multiply(int(a), int(b)),
                    Ok(RuntimeValue::BigInt(_))
                )))
            }),
        ),
        // Int → Float promotion is what the kernel does to the Int side of a mixed add; `x + -0.0 == x` for
        // every x, so adding negative zero exposes the promoted value alone.
        FuncUnit::IntToF => (
            "IntToF",
            Domain::Ints,
            Box::new(|a, _| kernel(arith::add(int(a), RuntimeValue::Float(-0.0)))),
        ),
        FuncUnit::FLtF => ("FLtF", Domain::Floats, fcmp(BinaryOpKind::Lt)),
        FuncUnit::FGtF => ("FGtF", Domain::Floats, fcmp(BinaryOpKind::Gt)),
        FuncUnit::FLeF => ("FLeF", Domain::Floats, fcmp(BinaryOpKind::LtEq)),
        FuncUnit::FGeF => ("FGeF", Domain::Floats, fcmp(BinaryOpKind::GtEq)),
        FuncUnit::FEqF => (
            "FEqF",
            Domain::Floats,
            Box::new(|a, b| kernel(arith::binary_op(BinaryOpKind::Eq, float(a), float(b)))),
        ),
        FuncUnit::FNeF => (
            "FNeF",
            Domain::Floats,
            Box::new(|a, b| kernel(arith::binary_op(BinaryOpKind::NotEq, float(a), float(b)))),
        ),
        FuncUnit::PopCount => (
            "PopCount",
            Domain::Ints,
            Box::new(|a, _| kernel(call_builtin(BuiltinId::CountOnes, vec![int(a)]))),
        ),
    }
}

/// Every functional unit, in declaration order. `unit_index` is a total match, so a new unit does not compile
/// until it has an index, and `every_unit_is_swept` fails until this list carries it.
const UNITS: [FuncUnit; 21] = [
    FuncUnit::DivS,
    FuncUnit::ModS,
    FuncUnit::FloorDivS,
    FuncUnit::Pow,
    FuncUnit::MulHiU,
    FuncUnit::ApproxEqF,
    FuncUnit::FAddF,
    FuncUnit::FSubF,
    FuncUnit::FMulF,
    FuncUnit::FDivF,
    FuncUnit::FSqrtF,
    FuncUnit::FFloorF,
    FuncUnit::MulOvfS,
    FuncUnit::IntToF,
    FuncUnit::FLtF,
    FuncUnit::FGtF,
    FuncUnit::FLeF,
    FuncUnit::FGeF,
    FuncUnit::FEqF,
    FuncUnit::FNeF,
    FuncUnit::PopCount,
];

fn unit_index(u: FuncUnit) -> usize {
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

#[test]
fn every_unit_is_swept() {
    let mut seen: Vec<usize> = UNITS.iter().map(|u| unit_index(*u)).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen,
        (0..UNITS.len()).collect::<Vec<_>>(),
        "UNITS names every functional unit exactly once"
    );
}

#[test]
fn every_functional_unit_is_the_kernel_or_names_its_divergence() {
    let xs = ints();
    let fs: Vec<i64> = floats().into_iter().map(fbits).collect();
    let exponents: Vec<i64> = (-3..=66)
        .chain([i64::from(u32::MAX) + 1, i64::MAX, i64::MIN])
        .collect();
    let mut bad = Vec::new();
    for unit in UNITS {
        let (label, domain, vm) = unit_kernel(unit);
        let (lhs, rhs) = match domain {
            Domain::Ints => (xs.clone(), xs.clone()),
            Domain::Floats => (fs.clone(), fs.clone()),
            Domain::PowExponents => (xs.clone(), exponents.clone()),
        };
        bad.extend(sweep(
            label,
            &lhs,
            &rhs,
            &|a, b| unit.eval_checked(a, b),
            &*vm,
        ));
    }
    assert!(
        bad.is_empty(),
        "{} unit verdicts disagree with the kernel:\n{}",
        bad.len(),
        bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}

fn cmp_op(c: CmpKind) -> BinaryOpKind {
    match c {
        CmpKind::Lt => BinaryOpKind::Lt,
        CmpKind::Gt => BinaryOpKind::Gt,
        CmpKind::Le => BinaryOpKind::LtEq,
        CmpKind::Ge => BinaryOpKind::GtEq,
        CmpKind::Eq => BinaryOpKind::Eq,
        CmpKind::Ne => BinaryOpKind::NotEq,
    }
}

#[test]
fn every_integer_comparison_is_the_kernel() {
    let xs = ints();
    let mut bad = Vec::new();
    for c in [
        CmpKind::Lt,
        CmpKind::Gt,
        CmpKind::Le,
        CmpKind::Ge,
        CmpKind::Eq,
        CmpKind::Ne,
    ] {
        let op = cmp_op(c);
        bad.extend(sweep(
            &format!("{c:?}"),
            &xs,
            &xs,
            &|a, b| Ok(i64::from(c.eval(a, b))),
            &|a, b| match op {
                BinaryOpKind::Eq | BinaryOpKind::NotEq => {
                    kernel(arith::binary_op(op, int(a), int(b)))
                }
                _ => kernel(compare::compare(op, &int(a), &int(b))),
            },
        ));
    }
    assert!(
        bad.is_empty(),
        "{} comparisons disagree with the kernel:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// Every divergence is reachable: each names a verdict the kernel actually gives for some operand pair, so no
/// variant exists only on paper.
#[test]
fn every_divergence_is_witnessed_by_the_kernel() {
    let witnesses: [(Divergence, Result<i64, Divergence>, Kernel); 5] = [
        (
            Divergence::IntOverflow,
            AluOp::Add.eval_checked(i64::MAX, 1),
            kernel(arith::add(int(i64::MAX), int(1))),
        ),
        (
            Divergence::DivideByZero,
            FuncUnit::DivS.eval_checked(7, 0),
            kernel(arith::divide(int(7), int(0))),
        ),
        (
            Divergence::NegativeExponent,
            FuncUnit::Pow.eval_checked(2, -1),
            kernel(arith::power(int(2), int(-1))),
        ),
        (
            Divergence::ExponentTooLarge,
            FuncUnit::Pow.eval_checked(2, i64::from(u32::MAX) + 1),
            kernel(arith::power(int(2), int(i64::from(u32::MAX) + 1))),
        ),
        (
            Divergence::FloatDivideByZero,
            FuncUnit::FDivF.eval_checked(fbits(1.0), fbits(-0.0)),
            kernel(arith::divide(
                RuntimeValue::Float(1.0),
                RuntimeValue::Float(-0.0),
            )),
        ),
    ];
    for (d, got, want) in witnesses {
        assert_eq!(got, Err(d), "{d} is reported by the checked evaluation");
        assert!(
            verdict_agrees(got, &want),
            "{d} is what the kernel shows: {want:?}"
        );
    }
    let covered: Vec<Divergence> = Divergence::ALL.to_vec();
    assert_eq!(covered.len(), 5, "a new divergence needs a witness here");
    for d in Divergence::ALL {
        assert_eq!(
            Divergence::from_code(d.code()),
            Some(d),
            "{d}'s device code round-trips"
        );
        assert_ne!(d.code(), 0, "code 0 means no divergence");
    }
}
