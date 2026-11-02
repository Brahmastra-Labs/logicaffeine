//! Arithmetic, logical, and bitwise operators.

use std::rc::Rc;

use logicaffeine_base::{BigInt, Complex, ComplexF, Decimal, Modular, Rational, WordVal};

use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::RuntimeValue;

use super::compare::{compare, values_equal};
use super::exact;
use super::temporal::date_add_span;

/// View an exact number — `Int`, `BigInt`, `Rational`, or `Decimal` — as a `Rational`,
/// for the exact-arithmetic path that integer division "overflows" into. `Float` is
/// inexact by choice and returns `None`.
fn rat_of(v: &RuntimeValue) -> Option<Rational> {
    match v {
        RuntimeValue::Int(n) => Some(Rational::from_i64(*n)),
        RuntimeValue::BigInt(b) => Some(Rational::from_bigint((**b).clone())),
        RuntimeValue::Rational(r) => Some((**r).clone()),
        RuntimeValue::Decimal(d) => Some(d.to_rational()),
        _ => None,
    }
}

/// View an integer-or-decimal value as a `Decimal`, for the decimal-PRESERVING path:
/// `Decimal ∘ {Decimal, Int, BigInt}` stays an exact `Decimal` (money keeps its scale).
/// `Rational`/`Float` return `None` so those operands route to the rational/float paths.
fn dec_of(v: &RuntimeValue) -> Option<Decimal> {
    match v {
        RuntimeValue::Int(n) => Some(Decimal::from_i64(*n)),
        RuntimeValue::BigInt(b) => Some(Decimal::from_bigint((**b).clone())),
        RuntimeValue::Decimal(d) => Some((**d).clone()),
        _ => None,
    }
}

/// View any number — exact or `Float` — as an `f64`, for the inexact path a `Float`
/// operand forces. `None` for non-numbers.
fn num_f64(v: &RuntimeValue) -> Option<f64> {
    match v {
        RuntimeValue::Int(n) => Some(*n as f64),
        RuntimeValue::BigInt(b) => Some(b.to_f64()),
        RuntimeValue::Rational(r) => Some(r.to_f64()),
        RuntimeValue::Decimal(d) => Some(d.to_rational().to_f64()),
        RuntimeValue::Float(f) => Some(*f),
        _ => None,
    }
}

/// View an exact number — `Int`, `BigInt`, `Rational`, `Decimal`, or `Complex` — as a
/// `Complex`. `Float` is inexact and returns `None` (an exact `Complex` does not absorb a
/// float), so `Complex ∘ Float` is a typed error rather than a silent lossy coercion.
fn complex_of(v: &RuntimeValue) -> Option<Complex> {
    match v {
        RuntimeValue::Int(n) => Some(Complex::from_i64(*n)),
        RuntimeValue::BigInt(b) => {
            Some(Complex::from_rational(Rational::from_bigint((**b).clone())))
        }
        RuntimeValue::Rational(r) => Some(Complex::from_rational((**r).clone())),
        RuntimeValue::Decimal(d) => Some(Complex::from_rational(d.to_rational())),
        RuntimeValue::Complex(c) => Some((**c).clone()),
        _ => None,
    }
}

/// Modular-operand dispatch shared by `add`/`subtract`/`multiply`: a `Modular` combines ONLY
/// with another `Modular` of the same modulus (no auto-lift — a bare integer has no modulus).
/// `None` when neither side is a `Modular`; `Some(Err)` on a non-Modular operand or a modulus
/// mismatch (`f` returns `None` for a mismatch, like a Word width mismatch).
fn modular_binop(
    left: &RuntimeValue,
    right: &RuntimeValue,
    op_name: &str,
    f: impl Fn(&Modular, &Modular) -> Option<Modular>,
) -> Option<Result<RuntimeValue, String>> {
    if !matches!(left, RuntimeValue::Modular(_)) && !matches!(right, RuntimeValue::Modular(_)) {
        return None;
    }
    Some(match (left, right) {
        (RuntimeValue::Modular(a), RuntimeValue::Modular(b)) => match f(a, b) {
            Some(r) => Ok(RuntimeValue::Modular(Rc::new(r))),
            None => Err(logicaffeine_data::failure::modular_rings_differ(op_name)),
        },
        _ => Err(format!(
            "Cannot {} {} and {} (modular arithmetic needs two ℤ/nℤ values of the same modulus)",
            op_name,
            left.type_name(),
            right.type_name()
        )),
    })
}

/// Complex-operand dispatch shared by `add`/`subtract`/`multiply`: when either side is a
/// `Complex`, the result is `Complex` (a real embeds as `re + 0i`). `None` when no operand
/// is a `Complex` (fall through); `Some(Err)` when an operand is inexact (`Float`).
fn complex_binop(
    left: &RuntimeValue,
    right: &RuntimeValue,
    op_name: &str,
    f: impl Fn(&Complex, &Complex) -> Complex,
) -> Option<Result<RuntimeValue, String>> {
    if !matches!(left, RuntimeValue::Complex(_)) && !matches!(right, RuntimeValue::Complex(_)) {
        return None;
    }
    Some(match (complex_of(left), complex_of(right)) {
        (Some(a), Some(b)) => Ok(RuntimeValue::Complex(Rc::new(f(&a, &b)))),
        _ => Err(format!(
            "Cannot {} {} and {} (Complex combines only with exact numbers)",
            op_name,
            left.type_name(),
            right.type_name()
        )),
    })
}

/// View any real or complex value as a floating-point `ComplexF`: the whole real tower
/// (Int/BigInt/Rational/Decimal/Float) plus exact `Complex` and `ComplexF` itself promote here.
fn complexf_of(v: &RuntimeValue) -> Option<ComplexF> {
    match v {
        RuntimeValue::Float(f) => Some(ComplexF::from_f64(*f)),
        RuntimeValue::ComplexF(c) => Some(**c),
        RuntimeValue::Complex(c) => Some(ComplexF::new(c.re().to_f64(), c.im().to_f64())),
        other => complex_of(other).map(|c| ComplexF::new(c.re().to_f64(), c.im().to_f64())),
    }
}

/// Floating-point-complex dispatch shared by `add`/`subtract`/`multiply`/`divide`: the result is
/// `ComplexF` when either side already is one, OR when an exact `Complex` meets a `Float` — the
/// promotion `Rational + Float → Float` extended to the complex plane. `None` (fall through to the
/// exact `complex_binop`) otherwise, so two exact complexes stay exact. `f` returns `None` only for
/// division by zero.
fn complexf_binop(
    left: &RuntimeValue,
    right: &RuntimeValue,
    op_name: &str,
    f: impl Fn(&ComplexF, &ComplexF) -> Option<ComplexF>,
) -> Option<Result<RuntimeValue, String>> {
    let is_cf = |v: &RuntimeValue| matches!(v, RuntimeValue::ComplexF(_));
    let is_c = |v: &RuntimeValue| matches!(v, RuntimeValue::Complex(_));
    let is_f = |v: &RuntimeValue| matches!(v, RuntimeValue::Float(_));
    let fires =
        is_cf(left) || is_cf(right) || (is_c(left) && is_f(right)) || (is_f(left) && is_c(right));
    if !fires {
        return None;
    }
    Some(match (complexf_of(left), complexf_of(right)) {
        (Some(a), Some(b)) => match f(&a, &b) {
            Some(r) => Ok(RuntimeValue::ComplexF(Rc::new(r))),
            None => Err(format!(
                "Cannot {} {} and {} (division by zero)",
                op_name,
                left.type_name(),
                right.type_name()
            )),
        },
        _ => Err(format!(
            "Cannot {} {} and {} (complex combines only with numbers)",
            op_name,
            left.type_name(),
            right.type_name()
        )),
    })
}

/// Decimal-operand dispatch shared by `add`/`subtract`/`multiply`: when either side is a
/// `Decimal`, `Decimal ∘ {Decimal,Int,BigInt}` stays exact `Decimal` (via `dec`), a
/// `Rational` operand promotes to exact `Rational`, and a `Float` operand yields `Float`.
/// Returns `None` when no operand is a `Decimal` (fall through to the existing paths).
fn decimal_binop(
    left: &RuntimeValue,
    right: &RuntimeValue,
    dec: impl Fn(&Decimal, &Decimal) -> Decimal,
    rat: impl Fn(&Rational, &Rational) -> Rational,
    flt: impl Fn(f64, f64) -> f64,
) -> Option<RuntimeValue> {
    if !matches!(left, RuntimeValue::Decimal(_)) && !matches!(right, RuntimeValue::Decimal(_)) {
        return None;
    }
    if let (Some(a), Some(b)) = (dec_of(left), dec_of(right)) {
        return Some(RuntimeValue::Decimal(Rc::new(dec(&a, &b))));
    }
    if let (Some(a), Some(b)) = (rat_of(left), rat_of(right)) {
        return Some(RuntimeValue::from_rational(rat(&a, &b)));
    }
    let (a, b) = (num_f64(left)?, num_f64(right)?);
    Some(RuntimeValue::Float(flt(a, b)))
}

/// Physical-quantity arithmetic. `+ −` require equal dimensions (the result keeps the LEFT operand's
/// display unit), `× ÷` combine dimensions (the result is shown in SI/dimension form), and a quantity
/// may be scaled by a dimensionless number under `× ÷` (its unit preserved). Returns `None` when
/// neither operand is a `Quantity` (so the normal numeric dispatch runs); `Some(Err)` on a dimension
/// mismatch or division by zero. The magnitude rides the exact rational tower, so it never drifts.
fn quantity_binop(
    left: &RuntimeValue,
    right: &RuntimeValue,
    op: char,
) -> Option<Result<RuntimeValue, String>> {
    use crate::interpreter::QuantityValue;
    use logicaffeine_base::{Quantity, Unit};
    if !matches!(left, RuntimeValue::Quantity(_)) && !matches!(right, RuntimeValue::Quantity(_)) {
        return None;
    }
    let mk = |q: Quantity, unit: Unit| RuntimeValue::Quantity(Rc::new(QuantityValue { q, unit }));
    // A synthetic SI-base unit (empty symbol) for a combined dimension — `display` renders it as the
    // magnitude plus the dimension signature until a named compound unit is chosen.
    let si_unit = |q: &Quantity| Unit::linear("", q.dimension(), Rational::one());
    Some(match (left, right) {
        (RuntimeValue::Quantity(a), RuntimeValue::Quantity(b)) => match op {
            '+' => a.q.add(&b.q).map(|q| mk(q, a.unit.clone())).ok_or_else(|| {
                logicaffeine_data::failure::quantities_differ(
                    "add",
                    a.q.dimension(),
                    b.q.dimension(),
                )
            }),
            '-' => a.q.sub(&b.q).map(|q| mk(q, a.unit.clone())).ok_or_else(|| {
                logicaffeine_data::failure::quantities_differ(
                    "subtract",
                    a.q.dimension(),
                    b.q.dimension(),
                )
            }),
            '*' => {
                let q = a.q.mul(&b.q);
                let u = si_unit(&q);
                Ok(mk(q, u))
            }
            '/' => match a.q.div(&b.q) {
                Some(q) => {
                    let u = si_unit(&q);
                    Ok(mk(q, u))
                }
                None => Err(logicaffeine_data::failure::DIVIDE_BY_ZERO_QUANTITY.to_string()),
            },
            _ => unreachable!("quantity_binop only handles + - * /"),
        },
        // Scale a quantity by a dimensionless number, preserving its unit: `q * k`, `q / k`.
        (RuntimeValue::Quantity(a), scalar) if matches!(op, '*' | '/') => match rat_of(scalar) {
            Some(k) => {
                let mag = if op == '*' {
                    a.q.magnitude_si().mul(&k)
                } else {
                    match a.q.magnitude_si().div(&k) {
                        Some(m) => m,
                        None => {
                            return Some(Err(
                                logicaffeine_data::failure::DIVIDE_QUANTITY_BY_ZERO.to_string()
                            ))
                        }
                    }
                };
                Ok(mk(Quantity::si(mag, a.q.dimension()), a.unit.clone()))
            }
            None => return None,
        },
        // `k * q` — scalar on the left (multiplication commutes).
        (scalar, RuntimeValue::Quantity(b)) if op == '*' => match rat_of(scalar) {
            Some(k) => Ok(mk(
                Quantity::si(b.q.magnitude_si().mul(&k), b.q.dimension()),
                b.unit.clone(),
            )),
            None => return None,
        },
        _ => return None,
    })
}

/// Money arithmetic. `+ −` require the SAME currency (a typed error otherwise, like a dimension
/// mismatch); `×` and `÷` scale by an exact number (Int/Decimal), re-quantised to the currency's
/// minor unit; a same-currency `Money ÷ Money` is the dimensionless ratio. Returns `None` when
/// neither operand is Money (so the normal numeric dispatch runs); `Some(Err)` on a currency mismatch.
fn money_binop(
    left: &RuntimeValue,
    right: &RuntimeValue,
    op: char,
) -> Option<Result<RuntimeValue, String>> {
    use logicaffeine_base::{Decimal, Money, RoundingMode};
    if !matches!(left, RuntimeValue::Money(_)) && !matches!(right, RuntimeValue::Money(_)) {
        return None;
    }
    // An exact base-10 scalar (Int or Decimal); a Float or non-terminating Rational is refused.
    let dec_of = |v: &RuntimeValue| -> Option<Decimal> {
        match v {
            RuntimeValue::Int(n) => Some(Decimal::from_i64(*n)),
            RuntimeValue::Decimal(d) => Some((**d).clone()),
            _ => None,
        }
    };
    let mk = |m: Money| RuntimeValue::Money(Rc::new(m));
    let mismatch = |a: &Money, b: &Money, verb: &str| {
        logicaffeine_data::failure::currencies_differ(verb, a.currency.code, b.currency.code)
    };
    Some(match (left, right) {
        (RuntimeValue::Money(a), RuntimeValue::Money(b)) => match op {
            '+' => a.add(b).map(mk).ok_or_else(|| mismatch(a, b, "add")),
            '-' => a.sub(b).map(mk).ok_or_else(|| mismatch(a, b, "subtract")),
            '/' => match a.ratio(b) {
                Some(r) => Ok(RuntimeValue::from_rational(r)),
                None if a.currency != b.currency => Err(mismatch(a, b, "compare")),
                None => Err(logicaffeine_data::failure::DIVIDE_MONEY_BY_ZERO_AMOUNT.to_string()),
            },
            '*' => Err("cannot multiply money by money — scale by a number instead".to_string()),
            _ => unreachable!("money_binop only handles + - * /"),
        },
        // Scale money by an exact number (`19.99 USD × 3`, `price × 1.5`), re-quantised; `×` commutes.
        (RuntimeValue::Money(a), scalar) | (scalar, RuntimeValue::Money(a)) if op == '*' => {
            match dec_of(scalar) {
                Some(s) => Ok(mk(Money::of(a.amount.mul(&s), a.currency))),
                None => Err(format!("cannot multiply money by {}", scalar.type_name())),
            }
        }
        // Divide money by an exact number (e.g. split a bill), re-quantised to the currency.
        (RuntimeValue::Money(a), scalar) if op == '/' => match dec_of(scalar) {
            Some(s) => match a.amount.div(&s, a.currency.scale, RoundingMode::HalfEven) {
                Some(d) => Ok(mk(Money::of(d, a.currency))),
                None => Err(logicaffeine_data::failure::DIVIDE_MONEY_BY_ZERO.to_string()),
            },
            None => Err(format!("cannot divide money by {}", scalar.type_name())),
        },
        _ => return None,
    })
}

/// Apply a binary operator to two already-evaluated values.
///
/// NOTE on `And`/`Or`: these are the *eager* semantics (both operands already
/// evaluated) — bitwise for Int×Int, truthiness otherwise. Short-circuit
/// evaluation order is the engine's responsibility: evaluate the left operand,
/// and only consult the right when the left is an Int (bitwise) or when
/// truthiness requires it.
/// The error for combining two words of different widths — a type error, never a coercion.
fn word_width_err(a: WordVal, b: WordVal) -> String {
    format!(
        "cannot combine Word{} and Word{} — width mismatch",
        a.width(),
        b.width()
    )
}

/// Extract a shift count as `u32` from an `Int` or `Word`.
fn shift_count(v: &RuntimeValue) -> Option<u32> {
    match v {
        RuntimeValue::Int(n) => Some(*n as u32),
        RuntimeValue::Word(w) => Some(w.to_u64() as u32),
        _ => None,
    }
}

/// Word-operand dispatch for [`binary_op`]: `Some(result)` when handled as a ring-of-ℤ/2ᵏ op,
/// `None` to fall through to the generic numeric/comparison path, `Err` on a width mismatch.
/// Arithmetic/bitwise require both operands at the same width; shifts take an integer count.
fn word_binary_op(
    op: BinaryOpKind,
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Result<Option<RuntimeValue>, String> {
    use BinaryOpKind::*;
    if matches!(op, Shl | Shr) {
        if let RuntimeValue::Word(a) = left {
            let n =
                shift_count(right).ok_or_else(|| "shift count must be an integer".to_string())?;
            let r = if matches!(op, Shl) {
                a.shl(n)
            } else {
                a.shr(n)
            };
            return Ok(Some(RuntimeValue::Word(r)));
        }
        return Ok(None);
    }
    let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (left, right) else {
        return Ok(None);
    };
    let combined = match op {
        // Add/Subtract/Multiply are handled in `add`/`subtract`/`multiply` (the VM calls those
        // directly), so both tiers share ONE Word path; fall through to them here.
        And | BitAnd => a.bitand(*b),
        Or | BitOr => a.bitor(*b),
        BitXor => a.bitxor(*b),
        _ => return Ok(None),
    };
    match combined {
        Some(w) => Ok(Some(RuntimeValue::Word(w))),
        None => Err(word_width_err(*a, *b)),
    }
}

/// Lane-operand dispatch for [`binary_op`]: a SIMD lane vector op is the same operator applied to
/// every lane (the scalar-lane spec; AOT lowers it to the matching AVX2 intrinsic). `None` falls
/// through, `Err` on a lane-config mismatch.
fn lanes_binary_op(
    op: BinaryOpKind,
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Result<Option<RuntimeValue>, String> {
    let (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) = (left, right) else {
        return Ok(None);
    };
    let combined = match op {
        BinaryOpKind::BitXor => a.bitxor(**b),
        _ => return Ok(None),
    };
    match combined {
        Some(v) => Ok(Some(RuntimeValue::Lanes(std::rc::Rc::new(v)))),
        None => Err(format!(
            "cannot combine {} and {} — lane-config mismatch",
            a.type_name(),
            b.type_name()
        )),
    }
}

/// The arithmetic entry the exact types re-enter to lift an operation over residue branches and
/// provisional values.
fn exact_recurse(op: exact::Op, a: RuntimeValue, b: RuntimeValue) -> Result<RuntimeValue, String> {
    match op {
        exact::Op::Add => add(a, b),
        exact::Op::Sub => subtract(a, b),
        exact::Op::Mul => multiply(a, b),
        exact::Op::Div => divide(a, b),
        exact::Op::ExactDiv => exact_divide(a, b),
        exact::Op::FloorDiv => floor_divide(a, b),
        exact::Op::Mod => modulo(a, b),
        exact::Op::Pow => power(a, b),
    }
}

pub fn binary_op(
    op: BinaryOpKind,
    left: RuntimeValue,
    right: RuntimeValue,
) -> Result<RuntimeValue, String> {
    // Fixed-width wrapping fast path: an op on words stays in the ring ℤ/2ᵏ (wrapping, never
    // promoting). Equality/comparison fall through to the generic path below.
    if let Some(result) = word_binary_op(op, &left, &right)? {
        return Ok(result);
    }
    // SIMD lane vectors: the op applies lane-wise (the scalar-lane spec).
    if let Some(result) = lanes_binary_op(op, &left, &right)? {
        return Ok(result);
    }
    match op {
        BinaryOpKind::Add => add(left, right),
        BinaryOpKind::Subtract => subtract(left, right),
        BinaryOpKind::Multiply => multiply(left, right),
        BinaryOpKind::Pow => power(left, right),
        BinaryOpKind::Divide => divide(left, right),
        BinaryOpKind::ExactDivide => exact_divide(left, right),
        BinaryOpKind::FloorDivide => floor_divide(left, right),
        BinaryOpKind::Modulo => modulo(left, right),
        BinaryOpKind::Eq => super::compare::equality(&left, &right, false),
        BinaryOpKind::NotEq => super::compare::equality(&left, &right, true),
        BinaryOpKind::ApproxEq => approx_eq(left, right),
        BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq => {
            compare(op, &left, &right)
        }
        // Logical words: truthiness in, Bool out. The bitwise spellings are `&`/`|` (BitAnd/BitOr).
        BinaryOpKind::And => Ok(RuntimeValue::Bool(left.decided_truth()? && right.decided_truth()?)),
        BinaryOpKind::Or => Ok(RuntimeValue::Bool(left.decided_truth()? || right.decided_truth()?)),
        BinaryOpKind::Concat => concat(left, right),
        BinaryOpKind::SeqConcat => seq_concat(left, right),
        BinaryOpKind::BitXor => match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a ^ b)),
            // Boolean XOR (`a ≠ b`) — matches the codegen's `a ^ b` on Rust `bool`, so the tiers
            // agree. (Word and lane XOR are taken on the fast paths above.)
            (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => Ok(RuntimeValue::Bool(a ^ b)),
            // On Sets, `^` is the symmetric difference.
            (RuntimeValue::Set(_), RuntimeValue::Set(_)) => {
                set_binop(&left, &right, SetOp::SymmetricDifference)
            }
            _ => Err(logicaffeine_data::failure::BITXOR_OPERANDS.to_string()),
        },
        // `&` — bitwise AND on Int, intersection on Sets.
        BinaryOpKind::BitAnd => match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a & b)),
            // A low-bit mask `2^k - 1` AND a NON-NEGATIVE BigInt: this is exactly the
            // `x % 2^k → x & (2^k-1)` lowering firing on a widening-loop bignum. For `x >= 0`,
            // `x & (2^k-1) == x mod 2^k`, computed bignum-exact so the optimized VM matches the
            // tree-walker's `Op::Mod`. (A negative bignum, or a non-mask AND, has no i64-bit meaning
            // and still falls through to the loud error, exactly as before.)
            (RuntimeValue::BigInt(big), RuntimeValue::Int(m))
            | (RuntimeValue::Int(m), RuntimeValue::BigInt(big))
                if *m >= 0
                    && (*m as u64).wrapping_add(1).is_power_of_two()
                    && !big.is_negative() =>
            {
                let modulus = BigInt::from_i64(*m).add(&BigInt::from_i64(1));
                match big.div_rem(&modulus) {
                    Some((_, r)) => Ok(RuntimeValue::from_bigint(r)),
                    None => Ok(RuntimeValue::Int(0)),
                }
            }
            (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => Ok(RuntimeValue::Bool(a & b)),
            (RuntimeValue::Set(_), RuntimeValue::Set(_)) => {
                set_binop(&left, &right, SetOp::Intersection)
            }
            _ => Err(logicaffeine_data::failure::BITAND_OPERANDS.to_string()),
        },
        // `|` — bitwise OR on Int, union on Sets.
        BinaryOpKind::BitOr => match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a | b)),
            (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => Ok(RuntimeValue::Bool(a | b)),
            (RuntimeValue::Set(_), RuntimeValue::Set(_)) => set_binop(&left, &right, SetOp::Union),
            _ => Err(logicaffeine_data::failure::BITOR_OPERANDS.to_string()),
        },
        // Shift counts are truncated to u32 and masked mod 64 (the wrapping spec).
        BinaryOpKind::Shl => match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                Ok(RuntimeValue::Int(a.wrapping_shl(b as u32)))
            }
            _ => Err(logicaffeine_data::failure::SHL_OPERANDS.to_string()),
        },
        BinaryOpKind::Shr => match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                Ok(RuntimeValue::Int(a.wrapping_shr(b as u32)))
            }
            _ => Err(logicaffeine_data::failure::SHR_OPERANDS.to_string()),
        },
    }
}

pub fn add(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::arith(exact::Op::Add, &left, &right, &exact_recurse) {
        return r;
    }
    // Words add in the ring ℤ/2ᵏ (wrapping). Here as well as in `binary_op` so the VM, which
    // calls `add` directly, stays byte-identical to the tree-walker on the Word path.
    if let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (&left, &right) {
        return a
            .add(*b)
            .map(RuntimeValue::Word)
            .ok_or_else(|| word_width_err(*a, *b));
    }
    // SIMD lane vectors add lane-wise in ℤ/2³² (the ChaCha quarter-round's `a += b`).
    if let (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) = (&left, &right) {
        return a
            .add(**b)
            .map(|v| RuntimeValue::Lanes(std::rc::Rc::new(v)))
            .ok_or_else(|| {
                format!(
                    "cannot add {} and {} — lane-config mismatch",
                    a.type_name(),
                    b.type_name()
                )
            });
    }
    if let Some(r) = quantity_binop(&left, &right, '+') {
        return r;
    }
    if let Some(r) = money_binop(&left, &right, '+') {
        return r;
    }
    if let Some(r) = modular_binop(&left, &right, "add", |a, b| a.add(b)) {
        return r;
    }
    if let Some(r) = complexf_binop(&left, &right, "add", |a, b| Some(a.add(b))) {
        return r;
    }
    if let Some(r) = complex_binop(&left, &right, "add", |a, b| a.add(b)) {
        return r;
    }
    if let Some(r) = decimal_binop(
        &left,
        &right,
        |a, b| a.add(b),
        |a, b| a.add(b),
        |a, b| a + b,
    ) {
        return Ok(r);
    }
    match (&left, &right) {
        // Integer addition is EXACT: on i64 overflow it promotes to BigInt instead of
        // wrapping (the silent `i64::MAX + 1 == i64::MIN` corruption is the bug this
        // fixes). The result downsizes back to Int whenever it fits (`from_bigint`).
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(match a.checked_add(*b) {
            Some(s) => RuntimeValue::Int(s),
            None => RuntimeValue::from_bigint(BigInt::from_i64(*a).add(&BigInt::from_i64(*b))),
        }),
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(a.add(b)))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            Ok(RuntimeValue::from_bigint(a.add(&BigInt::from_i64(*b))))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(BigInt::from_i64(*a).add(b)))
        }
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a + b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 + b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a + *b as f64)),
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Float(a.to_f64() + b))
        }
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::Float(a + b.to_f64()))
        }
        // Rational keeps arithmetic EXACT: any Int/BigInt/Rational mix involving a
        // Rational stays exact (downsized to Int when it reduces to a whole number); a
        // Float operand makes the result Float (floats are inexact by choice).
        (RuntimeValue::Rational(a), RuntimeValue::Rational(b)) => {
            Ok(RuntimeValue::from_rational(a.add(b)))
        }
        (RuntimeValue::Rational(_), RuntimeValue::Int(_))
        | (RuntimeValue::Int(_), RuntimeValue::Rational(_))
        | (RuntimeValue::Rational(_), RuntimeValue::BigInt(_))
        | (RuntimeValue::BigInt(_), RuntimeValue::Rational(_)) => Ok(RuntimeValue::from_rational(
            rat_of(&left).unwrap().add(&rat_of(&right).unwrap()),
        )),
        (RuntimeValue::Rational(r), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Float(r.to_f64() + b))
        }
        (RuntimeValue::Float(a), RuntimeValue::Rational(r)) => {
            Ok(RuntimeValue::Float(a + r.to_f64()))
        }
        (RuntimeValue::Text(a), RuntimeValue::Text(b)) => {
            Ok(RuntimeValue::Text(Rc::new(format!("{}{}", a, b))))
        }
        (RuntimeValue::Text(a), other) => Ok(RuntimeValue::Text(Rc::new(format!(
            "{}{}",
            a,
            other.to_display_string()
        )))),
        (other, RuntimeValue::Text(b)) => Ok(RuntimeValue::Text(Rc::new(format!(
            "{}{}",
            other.to_display_string(),
            b
        )))),
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => {
            Ok(RuntimeValue::Duration(a.wrapping_add(*b)))
        }
        (
            RuntimeValue::Date(days),
            RuntimeValue::Span {
                months,
                days: span_days,
            },
        ) => Ok(RuntimeValue::Date(date_add_span(
            *days, *months, *span_days,
        ))),
        // A Moment plus a CIVIL Span is calendar arithmetic (`m + 1 month` clamps end-of-month,
        // respects leap years, keeps the wall time); commutes. Contrast the physical Duration below.
        (
            RuntimeValue::Moment(nanos),
            RuntimeValue::Span {
                months,
                days: span_days,
            },
        )
        | (
            RuntimeValue::Span {
                months,
                days: span_days,
            },
            RuntimeValue::Moment(nanos),
        ) => Ok(RuntimeValue::Moment(super::temporal::moment_add_span(
            *nanos, *months, *span_days,
        ))),
        // A Moment plus a physical Duration is a later Moment (e.g. `m + 90 seconds`); commutes.
        (RuntimeValue::Moment(nanos), RuntimeValue::Duration(d))
        | (RuntimeValue::Duration(d), RuntimeValue::Moment(nanos)) => {
            Ok(RuntimeValue::Moment(nanos.wrapping_add(*d)))
        }
        // `xs + ys` concatenates sequences — exactly `xs followed by ys`
        // (a fresh sequence; neither operand is mutated).
        (RuntimeValue::List(_), RuntimeValue::List(_)) => seq_concat(left.clone(), right.clone()),
        _ => Err(format!(
            "Cannot add {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

pub fn concat(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    Ok(RuntimeValue::Text(Rc::new(format!(
        "{}{}",
        left.to_display_string(),
        right.to_display_string()
    ))))
}

/// The Set operations behind `| & ^ -` on Set operands. Results are FRESH
/// sets in first-operand-then-second insertion order, deduped by
/// `values_equal` — the same equality the language uses everywhere.
enum SetOp {
    Union,
    Intersection,
    Difference,
    SymmetricDifference,
}

fn set_binop(left: &RuntimeValue, right: &RuntimeValue, op: SetOp) -> Result<RuntimeValue, String> {
    let (RuntimeValue::Set(a), RuntimeValue::Set(b)) = (left, right) else {
        return Err("set operation requires two Sets".to_string());
    };
    let (a, b) = (a.borrow(), b.borrow());
    let contains = |xs: &[RuntimeValue], v: &RuntimeValue| xs.iter().any(|x| values_equal(x, v));
    let mut out: Vec<RuntimeValue> = Vec::new();
    match op {
        SetOp::Union => {
            out.extend(a.iter().cloned());
            for v in b.iter() {
                if !contains(&out, v) {
                    out.push(v.clone());
                }
            }
        }
        SetOp::Intersection => {
            for v in a.iter() {
                if contains(&b, v) {
                    out.push(v.clone());
                }
            }
        }
        SetOp::Difference => {
            for v in a.iter() {
                if !contains(&b, v) {
                    out.push(v.clone());
                }
            }
        }
        SetOp::SymmetricDifference => {
            for v in a.iter() {
                if !contains(&b, v) {
                    out.push(v.clone());
                }
            }
            for v in b.iter() {
                if !contains(&a, v) {
                    out.push(v.clone());
                }
            }
        }
    }
    Ok(RuntimeValue::Set(Rc::new(std::cell::RefCell::new(out))))
}

/// `a is approximately b` — the TOLERANT numeric comparison (`==` is IEEE
/// bit-exact). Both operands coerce to f64 (approximation is inherently
/// tolerant, so the lossy view is correct here) and compare with the ONE
/// shared isclose definition (`logicaffeine_data::ops::logos_approx_eq`).
pub fn approx_eq(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    let as_f64 = |v: &RuntimeValue| -> Option<f64> {
        match v {
            RuntimeValue::Float(f) => Some(*f),
            RuntimeValue::Int(n) => Some(*n as f64),
            RuntimeValue::BigInt(b) => Some(b.to_f64()),
            RuntimeValue::Rational(r) => Some(r.to_f64()),
            _ => None,
        }
    };
    match (as_f64(&left), as_f64(&right)) {
        (Some(a), Some(b)) => Ok(RuntimeValue::Bool(logicaffeine_data::ops::logos_approx_eq(
            a, b,
        ))),
        _ => Err(format!(
            "`is approximately` compares numbers, got {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// `a followed by b` — merge two sequences into one fresh sequence. Element order is preserved:
/// all of `a`, then all of `b`. Operands must both be sequences.
pub fn seq_concat(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    use crate::interpreter::ListRepr;
    match (&left, &right) {
        (RuntimeValue::List(a), RuntimeValue::List(b)) => {
            let mut items = a.borrow().to_values();
            items.extend(b.borrow().to_values());
            Ok(RuntimeValue::List(Rc::new(std::cell::RefCell::new(
                ListRepr::from_values(items),
            ))))
        }
        _ => Err("`followed by` requires two sequences (merge two sequences into one)".to_string()),
    }
}

pub fn subtract(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::arith(exact::Op::Sub, &left, &right, &exact_recurse) {
        return r;
    }
    // `a - b` on Sets is the difference (`a without b` is the English form).
    if matches!(
        (&left, &right),
        (RuntimeValue::Set(_), RuntimeValue::Set(_))
    ) {
        return set_binop(&left, &right, SetOp::Difference);
    }
    if let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (&left, &right) {
        return a
            .sub(*b)
            .map(RuntimeValue::Word)
            .ok_or_else(|| word_width_err(*a, *b));
    }
    // SIMD lane vectors subtract lane-wise (the NTT butterfly's `a - t` over Word16 lanes).
    if let (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) = (&left, &right) {
        return a
            .sub(**b)
            .map(|v| RuntimeValue::Lanes(std::rc::Rc::new(v)))
            .ok_or_else(|| {
                format!(
                    "cannot subtract {} and {} — lane-config mismatch",
                    a.type_name(),
                    b.type_name()
                )
            });
    }
    if let Some(r) = quantity_binop(&left, &right, '-') {
        return r;
    }
    if let Some(r) = money_binop(&left, &right, '-') {
        return r;
    }
    if let Some(r) = modular_binop(&left, &right, "subtract", |a, b| a.sub(b)) {
        return r;
    }
    if let Some(r) = complexf_binop(&left, &right, "subtract", |a, b| Some(a.sub(b))) {
        return r;
    }
    if let Some(r) = complex_binop(&left, &right, "subtract", |a, b| a.sub(b)) {
        return r;
    }
    if let Some(r) = decimal_binop(
        &left,
        &right,
        |a, b| a.sub(b),
        |a, b| a.sub(b),
        |a, b| a - b,
    ) {
        return Ok(r);
    }
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(match a.checked_sub(*b) {
            Some(s) => RuntimeValue::Int(s),
            None => RuntimeValue::from_bigint(BigInt::from_i64(*a).sub(&BigInt::from_i64(*b))),
        }),
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(a.sub(b)))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            Ok(RuntimeValue::from_bigint(a.sub(&BigInt::from_i64(*b))))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(BigInt::from_i64(*a).sub(b)))
        }
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a - b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 - b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a - *b as f64)),
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Float(a.to_f64() - b))
        }
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::Float(a - b.to_f64()))
        }
        (RuntimeValue::Rational(a), RuntimeValue::Rational(b)) => {
            Ok(RuntimeValue::from_rational(a.sub(b)))
        }
        (RuntimeValue::Rational(_), RuntimeValue::Int(_))
        | (RuntimeValue::Int(_), RuntimeValue::Rational(_))
        | (RuntimeValue::Rational(_), RuntimeValue::BigInt(_))
        | (RuntimeValue::BigInt(_), RuntimeValue::Rational(_)) => Ok(RuntimeValue::from_rational(
            rat_of(&left).unwrap().sub(&rat_of(&right).unwrap()),
        )),
        (RuntimeValue::Rational(r), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Float(r.to_f64() - b))
        }
        (RuntimeValue::Float(a), RuntimeValue::Rational(r)) => {
            Ok(RuntimeValue::Float(a - r.to_f64()))
        }
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => {
            Ok(RuntimeValue::Duration(a.wrapping_sub(*b)))
        }
        (
            RuntimeValue::Date(days),
            RuntimeValue::Span {
                months,
                days: span_days,
            },
        ) => Ok(RuntimeValue::Date(date_add_span(
            *days,
            -*months,
            -*span_days,
        ))),
        // A Moment minus a CIVIL Span steps the calendar backward (`m - 1 month`), mirroring the add.
        (
            RuntimeValue::Moment(nanos),
            RuntimeValue::Span {
                months,
                days: span_days,
            },
        ) => Ok(RuntimeValue::Moment(super::temporal::moment_add_span(
            *nanos,
            -*months,
            -*span_days,
        ))),
        // A Moment minus a Duration is an earlier Moment. (Moment − Moment → elapsed Duration is
        // deferred until Duration has a signed i64-nanos AOT representation, so the tiers stay in
        // lock-step; `the seconds between a and b` already covers elapsed time naturally.)
        (RuntimeValue::Moment(nanos), RuntimeValue::Duration(d)) => {
            Ok(RuntimeValue::Moment(nanos.wrapping_sub(*d)))
        }
        _ => Err(format!(
            "Cannot subtract {} from {}",
            right.type_name(),
            left.type_name()
        )),
    }
}

/// `base ** exp` — exponentiation. Integer power is EXACT (i64 fast path,
/// promoting to BigInt on overflow); a Float operand takes the `f64::powf`
/// path; a Rational base with an integer exponent stays exact; a NEGATIVE
/// integer exponent on an integer base is a loud error (an Int can't hold the
/// fractional result — use a Float base). ONE definition, shared by tw/VM/JIT.
pub fn power(base: RuntimeValue, exp: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::arith(exact::Op::Pow, &base, &exp, &exact_recurse) {
        return r;
    }
    // Any Float operand → inexact f64 power.
    if matches!(base, RuntimeValue::Float(_)) || matches!(exp, RuntimeValue::Float(_)) {
        let b = num_f64(&base)
            .ok_or_else(|| format!("cannot raise {} to a power", base.type_name()))?;
        let e =
            num_f64(&exp).ok_or_else(|| format!("cannot raise to a {} power", exp.type_name()))?;
        return Ok(RuntimeValue::Float(b.powf(e)));
    }
    match (&base, &exp) {
        (RuntimeValue::Int(b), RuntimeValue::Int(e)) => int_power(*b, *e),
        (RuntimeValue::BigInt(b), RuntimeValue::Int(e)) => {
            let ue = u32::try_from(*e).map_err(|_| {
                "negative or too-large exponent on an integer (use a Float base)".to_string()
            })?;
            Ok(RuntimeValue::from_bigint(b.pow(ue)))
        }
        (RuntimeValue::Rational(b), RuntimeValue::Int(e)) => {
            let ie = i32::try_from(*e).map_err(|_| "exponent too large".to_string())?;
            b.pow(ie)
                .map(RuntimeValue::from_rational)
                .ok_or_else(|| "zero raised to a negative power".to_string())
        }
        _ => Err(format!(
            "cannot raise {} to the {} power",
            base.type_name(),
            exp.type_name()
        )),
    }
}

/// Exact `base^exp` for non-negative integer exponents; promotes to BigInt on
/// i64 overflow. A negative exponent is a loud error.
fn int_power(base: i64, exp: i64) -> Result<RuntimeValue, String> {
    if exp < 0 {
        return Err(
            "negative exponent on an integer (an Int can't hold a fraction — use a Float base)"
                .to_string(),
        );
    }
    let e = u32::try_from(exp).map_err(|_| "exponent too large".to_string())?;
    match base.checked_pow(e) {
        Some(r) => Ok(RuntimeValue::Int(r)),
        None => Ok(RuntimeValue::from_bigint(BigInt::from_i64(base).pow(e))),
    }
}

pub fn multiply(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::arith(exact::Op::Mul, &left, &right, &exact_recurse) {
        return r;
    }
    if let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (&left, &right) {
        return a
            .mul(*b)
            .map(RuntimeValue::Word)
            .ok_or_else(|| word_width_err(*a, *b));
    }
    // SIMD lane vectors multiply lane-wise low-16 (`vpmullw`, the NTT's `*` over Word16 lanes).
    if let (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) = (&left, &right) {
        return a
            .mullo(**b)
            .map(|v| RuntimeValue::Lanes(std::rc::Rc::new(v)))
            .ok_or_else(|| {
                format!(
                    "cannot multiply {} and {} — lane op undefined",
                    a.type_name(),
                    b.type_name()
                )
            });
    }
    if let Some(r) = quantity_binop(&left, &right, '*') {
        return r;
    }
    if let Some(r) = money_binop(&left, &right, '*') {
        return r;
    }
    if let Some(r) = modular_binop(&left, &right, "multiply", |a, b| a.mul(b)) {
        return r;
    }
    if let Some(r) = complexf_binop(&left, &right, "multiply", |a, b| Some(a.mul(b))) {
        return r;
    }
    if let Some(r) = complex_binop(&left, &right, "multiply", |a, b| a.mul(b)) {
        return r;
    }
    if let Some(r) = decimal_binop(
        &left,
        &right,
        |a, b| a.mul(b),
        |a, b| a.mul(b),
        |a, b| a * b,
    ) {
        return Ok(r);
    }
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(match a.checked_mul(*b) {
            Some(p) => RuntimeValue::Int(p),
            None => RuntimeValue::from_bigint(BigInt::from_i64(*a).mul(&BigInt::from_i64(*b))),
        }),
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(a.mul(b)))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            Ok(RuntimeValue::from_bigint(a.mul(&BigInt::from_i64(*b))))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(BigInt::from_i64(*a).mul(b)))
        }
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a * b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 * b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a * *b as f64)),
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Float(a.to_f64() * b))
        }
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::Float(a * b.to_f64()))
        }
        (RuntimeValue::Rational(a), RuntimeValue::Rational(b)) => {
            Ok(RuntimeValue::from_rational(a.mul(b)))
        }
        (RuntimeValue::Rational(_), RuntimeValue::Int(_))
        | (RuntimeValue::Int(_), RuntimeValue::Rational(_))
        | (RuntimeValue::Rational(_), RuntimeValue::BigInt(_))
        | (RuntimeValue::BigInt(_), RuntimeValue::Rational(_)) => Ok(RuntimeValue::from_rational(
            rat_of(&left).unwrap().mul(&rat_of(&right).unwrap()),
        )),
        (RuntimeValue::Rational(r), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Float(r.to_f64() * b))
        }
        (RuntimeValue::Float(a), RuntimeValue::Rational(r)) => {
            Ok(RuntimeValue::Float(a * r.to_f64()))
        }
        // `xs * n` / `n * xs` — repeat a sequence n times into a FRESH
        // sequence. Each slot deep-copies its element (a repeated inner
        // collection is n INDEPENDENT rows, never n aliases of one — the
        // classic `[[0]]*3` footgun is designed out). n ≤ 0 is empty.
        (RuntimeValue::List(items), RuntimeValue::Int(n))
        | (RuntimeValue::Int(n), RuntimeValue::List(items)) => {
            use crate::interpreter::ListRepr;
            let src = items.borrow().to_values();
            let count = (*n).max(0) as usize;
            let mut out = Vec::with_capacity(src.len() * count);
            for _ in 0..count {
                out.extend(src.iter().map(|v| v.deep_clone()));
            }
            Ok(RuntimeValue::List(Rc::new(std::cell::RefCell::new(
                ListRepr::from_values(out),
            ))))
        }
        _ => Err(format!(
            "Cannot multiply {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

pub fn divide(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::arith(exact::Op::Div, &left, &right, &exact_recurse) {
        return r;
    }
    if let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (&left, &right) {
        if a.width() != b.width() {
            return Err(word_width_err(*a, *b));
        }
        if b.to_u64() == 0 {
            return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
        }
        return Ok(RuntimeValue::Word(a.div(*b).expect("width checked above")));
    }
    if let Some(r) = quantity_binop(&left, &right, '/') {
        return r;
    }
    if let Some(r) = money_binop(&left, &right, '/') {
        return r;
    }
    // Modular division = multiply by the modular inverse: the operands must share a modulus
    // and the divisor must be a unit (coprime to the modulus), else there is no inverse.
    if matches!(left, RuntimeValue::Modular(_)) || matches!(right, RuntimeValue::Modular(_)) {
        return match (&left, &right) {
            (RuntimeValue::Modular(a), RuntimeValue::Modular(b)) => {
                if a.modulus() != b.modulus() {
                    Err(logicaffeine_data::failure::modular_rings_differ("divide"))
                } else {
                    a.div(b)
                        .map(|r| RuntimeValue::Modular(Rc::new(r)))
                        .ok_or_else(|| logicaffeine_data::failure::NO_MODULAR_INVERSE.to_string())
                }
            }
            _ => Err(format!(
                "Cannot divide {} by {}",
                left.type_name(),
                right.type_name()
            )),
        };
    }
    // Floating-point-complex division (promotes when a float meets a complex); `None` on a zero
    // divisor. Checked before the exact path so two exact complexes still divide exactly.
    if let Some(r) = complexf_binop(&left, &right, "divide", |a, b| a.div(b)) {
        return r;
    }
    // Complex division stays Complex (the field is closed); `None` on a zero divisor.
    if matches!(left, RuntimeValue::Complex(_)) || matches!(right, RuntimeValue::Complex(_)) {
        return match (complex_of(&left), complex_of(&right)) {
            (Some(a), Some(b)) => a
                .div(&b)
                .map(|c| RuntimeValue::Complex(Rc::new(c)))
                .ok_or_else(|| logicaffeine_data::failure::DIVISION_BY_ZERO.to_string()),
            _ => Err(format!(
                "Cannot divide {} by {}",
                left.type_name(),
                right.type_name()
            )),
        };
    }
    // A Decimal divides EXACTLY into the Rational tower (base-10 division need not
    // terminate, so it does not stay a Decimal); a Float operand divides as Float.
    if matches!(left, RuntimeValue::Decimal(_)) || matches!(right, RuntimeValue::Decimal(_)) {
        if let (Some(a), Some(b)) = (rat_of(&left), rat_of(&right)) {
            return a
                .div(&b)
                .map(RuntimeValue::from_rational)
                .ok_or_else(|| logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
        }
        if let (Some(a), Some(b)) = (num_f64(&left), num_f64(&right)) {
            if b == 0.0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            return Ok(RuntimeValue::Float(a / b));
        }
    }
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            // checked_div is None only for i64::MIN / -1, whose true quotient
            // (2^63) overflows i64 → promote rather than wrap.
            Ok(match a.checked_div(*b) {
                Some(q) => RuntimeValue::Int(q),
                None => RuntimeValue::from_bigint(int_div(*a, *b).0),
            })
        }
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::from_bigint(
            a.div_rem(b).expect("BigInt divisor is never zero").0,
        )),
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            Ok(RuntimeValue::from_bigint(
                a.div_rem(&BigInt::from_i64(*b)).unwrap().0,
            ))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::from_bigint(
            BigInt::from_i64(*a).div_rem(b).unwrap().0,
        )),
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            Ok(RuntimeValue::Float(a / b))
        }
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            Ok(RuntimeValue::Float(*a as f64 / b))
        }
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            Ok(RuntimeValue::Float(a / *b as f64))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            Ok(RuntimeValue::Float(a.to_f64() / b))
        }
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::Float(a / b.to_f64()))
        }
        // Exact division on Rational operands: the quotient is exact (downsized to an
        // Int when it reduces to a whole number). A Float operand makes it Float.
        (RuntimeValue::Rational(_), RuntimeValue::Rational(_))
        | (RuntimeValue::Rational(_), RuntimeValue::Int(_))
        | (RuntimeValue::Int(_), RuntimeValue::Rational(_))
        | (RuntimeValue::Rational(_), RuntimeValue::BigInt(_))
        | (RuntimeValue::BigInt(_), RuntimeValue::Rational(_)) => rat_of(&left)
            .unwrap()
            .div(&rat_of(&right).unwrap())
            .map(RuntimeValue::from_rational)
            .ok_or_else(|| logicaffeine_data::failure::DIVISION_BY_ZERO.to_string()),
        (RuntimeValue::Rational(r), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            Ok(RuntimeValue::Float(r.to_f64() / b))
        }
        (RuntimeValue::Float(a), RuntimeValue::Rational(r)) => {
            Ok(RuntimeValue::Float(a / r.to_f64()))
        }
        _ => Err(format!(
            "Cannot divide {} by {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Exact integer division of two `i64`s as a BigInt `(quotient, remainder)` — used on
/// the one overflowing case, `i64::MIN / -1`.
fn int_div(a: i64, b: i64) -> (BigInt, BigInt) {
    BigInt::from_i64(a)
        .div_rem(&BigInt::from_i64(b))
        .expect("nonzero divisor")
}

/// FLOOR division — the runtime of [`BinaryOpKind::FloorDivide`] (`a // b`), the quotient
/// rounded toward NEGATIVE INFINITY. On exact operands (Int/BigInt/Rational/Decimal) the
/// result is the exact quotient's floor as an integer (`-7 // 2 → -4`, `10^30 // 7` exact);
/// a Float operand floors the float quotient but stays Float (`7.5 // 2 → 3.0`, Python
/// semantics). Distinct from [`divide`], which truncates toward zero. Zero divisor is loud.
pub fn floor_divide(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::arith(exact::Op::FloorDiv, &left, &right, &exact_recurse) {
        return r;
    }
    // Float domain: floor the float quotient, staying inexact.
    if matches!(left, RuntimeValue::Float(_)) || matches!(right, RuntimeValue::Float(_)) {
        if let (Some(a), Some(b)) = (num_f64(&left), num_f64(&right)) {
            if b == 0.0 {
                return Err(logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
            }
            return Ok(RuntimeValue::Float((a / b).floor()));
        }
    }
    // Exact domain: the exact rational quotient, floored to an integer (BigInt-promoting,
    // downsized to Int when it fits). `rat_of` covers Int/BigInt/Rational/Decimal.
    if let (Some(a), Some(b)) = (rat_of(&left), rat_of(&right)) {
        return a
            .div(&b)
            .map(|q| RuntimeValue::from_bigint(q.floor()))
            .ok_or_else(|| logicaffeine_data::failure::DIVISION_BY_ZERO.to_string());
    }
    Err(format!(
        "Cannot floor-divide {} by {}",
        left.type_name(),
        right.type_name()
    ))
}

/// EXACT division — the runtime of [`BinaryOpKind::ExactDivide`], the type-directed
/// sibling of [`divide`]. An evenly-dividing integer pair stays an `Int`/`BigInt`;
/// otherwise the quotient is an exact `Rational` (`7 / 2 → 7/2`) — it NEVER truncates.
/// A `Float` operand divides as `Float` (floats are inexact by choice). This only ever
/// runs where the type says the result is a `Rational`, so floor code is untouched.
pub fn exact_divide(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::arith(exact::Op::ExactDiv, &left, &right, &exact_recurse) {
        return r;
    }
    match (&left, &right) {
        // Every Int/BigInt/Rational combination divides to an EXACT value — downsized to
        // an Int/BigInt when it reduces to a whole number, else an exact Rational.
        (a, b) if rat_of(a).is_some() && rat_of(b).is_some() => rat_of(&left)
            .unwrap()
            .div(&rat_of(&right).unwrap())
            .map(RuntimeValue::from_rational)
            .ok_or_else(|| logicaffeine_data::failure::DIVISION_BY_ZERO.to_string()),
        // A Float operand makes it Float — defer to the Float-aware `divide` (whose Float
        // arms don't truncate anyway).
        (RuntimeValue::Float(_), _) | (_, RuntimeValue::Float(_)) => divide(left, right),
        _ => Err(format!(
            "Cannot divide {} by {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

pub fn modulo(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::arith(exact::Op::Mod, &left, &right, &exact_recurse) {
        return r;
    }
    if let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (&left, &right) {
        if a.width() != b.width() {
            return Err(word_width_err(*a, *b));
        }
        if b.to_u64() == 0 {
            return Err(logicaffeine_data::failure::MODULO_BY_ZERO.to_string());
        }
        return Ok(RuntimeValue::Word(a.rem(*b).expect("width checked above")));
    }
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err(logicaffeine_data::failure::MODULO_BY_ZERO.to_string());
            }
            // checked_rem is None only for i64::MIN % -1, whose true remainder is 0.
            Ok(RuntimeValue::Int(a.checked_rem(*b).unwrap_or(0)))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::from_bigint(
            a.div_rem(b).expect("BigInt divisor is never zero").1,
        )),
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err(logicaffeine_data::failure::MODULO_BY_ZERO.to_string());
            }
            Ok(RuntimeValue::from_bigint(
                a.div_rem(&BigInt::from_i64(*b)).unwrap().1,
            ))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::from_bigint(
            BigInt::from_i64(*a).div_rem(b).unwrap().1,
        )),
        _ => Err(format!(
            "Cannot compute modulo of {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// One CRDT counter step: `current + amount`. A CRDT grow-counter is intentionally
/// `wrapping` (modular) — convergence is defined over the cyclic group, so overflow
/// is a feature here, NOT the silent-corruption footgun that ordinary integer math
/// now promotes away. An absent or Nothing field starts from zero.
pub fn crdt_counter_bump(
    current: RuntimeValue,
    amount: i64,
    field_name: &str,
) -> Result<RuntimeValue, String> {
    match current {
        // A live per-replica counter (`ConvergentCount`/`Tally`): bump THIS replica's slot, returning
        // a fresh replica so the field never aliases. The per-replica accounting is what makes a
        // later merge idempotent — the same remote state re-merged never double-counts.
        RuntimeValue::Crdt(rc) => {
            let mut cv = rc.borrow().clone();
            cv.bump(amount)?;
            Ok(RuntimeValue::Crdt(std::rc::Rc::new(
                std::cell::RefCell::new(cv),
            )))
        }
        RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n.wrapping_add(amount))),
        RuntimeValue::Nothing => Ok(RuntimeValue::Int(amount)),
        _ => Err(format!("Field '{}' is not a counter", field_name)),
    }
}

/// Merge one CRDT field by its convergence rule:
/// - `Int` + `Int` — a grow-counter, so add (intentionally `wrapping`/modular);
/// - `Set` + `Set` — a grow-set, so union (dedup by value);
/// - anything else (LWW register: Bool/Text/Float/…) — last writer wins ⇒ take
///   the incoming value.
/// The struct type tag the interpreter uses for a state-based, gossip-safe G-Counter: a
/// map of `replica id → that replica's monotonic count`. Distinct from a plain counter
/// (a bare `Int` whose op-based `add` merge is NOT idempotent under redelivery).
pub const GCOUNTER_TAG: &str = "__GCounter";

/// The total of a [`GCOUNTER_TAG`] counter — the sum of every replica's count. `None` for
/// a value that is not a G-Counter.
pub fn gcounter_value(v: &RuntimeValue) -> Option<i64> {
    match v {
        RuntimeValue::Struct(s) if s.type_name == GCOUNTER_TAG => Some(
            s.fields
                .values()
                .map(|c| if let RuntimeValue::Int(n) = c { *n } else { 0 })
                .fold(0i64, i64::wrapping_add),
        ),
        _ => None,
    }
}

pub fn crdt_merge_field(current: &RuntimeValue, incoming: RuntimeValue) -> RuntimeValue {
    match (current, &incoming) {
        // A state-based G-Counter: per-replica MAX. Unlike the bare-Int `add` below, MAX is
        // IDEMPOTENT (max of a value with itself is itself), so a redelivered or duplicated
        // counter state never double-counts — gossip/lossy-network safe. Commutative and
        // associative too, so all replicas converge to the same total.
        (RuntimeValue::Struct(a), RuntimeValue::Struct(b))
            if a.type_name == GCOUNTER_TAG && b.type_name == GCOUNTER_TAG =>
        {
            let mut fields = a.fields.clone();
            for (replica, count) in &b.fields {
                let next = match (fields.get(replica), count) {
                    (Some(RuntimeValue::Int(x)), RuntimeValue::Int(y)) => {
                        RuntimeValue::Int((*x).max(*y))
                    }
                    _ => count.clone(),
                };
                fields.insert(replica.clone(), next);
            }
            RuntimeValue::Struct(Box::new(crate::interpreter::StructValue {
                type_name: GCOUNTER_TAG.to_string(),
                fields,
            }))
        }
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => RuntimeValue::Int(a.wrapping_add(*b)),
        (RuntimeValue::Set(_), RuntimeValue::Set(_)) => {
            crate::semantics::collections::union(current, &incoming).unwrap_or(incoming)
        }
        // A CRDT MAP — "shared memory" over the network: the union of keys, each shared
        // key's values merged RECURSIVELY through this same join. The merge inherits the
        // value type's laws: a map of sets (or of nested maps) is commutative, associative,
        // AND idempotent, so replicas converge no matter the order or duplication of merges.
        (RuntimeValue::Map(a), RuntimeValue::Map(b)) => {
            let mut out = a.borrow().clone();
            for (k, v) in b.borrow().iter() {
                let merged = match out.get(k) {
                    Some(cur) => crdt_merge_field(cur, v.clone()),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            RuntimeValue::Map(std::rc::Rc::new(std::cell::RefCell::new(out)))
        }
        // A live CRDT (OR-Set / RGA / MV-register) converges through its own state-based
        // join. The result is a FRESH replica (the current state deep-copied, then the
        // incoming state merged into it), so neither operand is aliased into the result —
        // the join inherits the data-crate type's commutativity/associativity/idempotence.
        (RuntimeValue::Crdt(a), RuntimeValue::Crdt(b)) => {
            let mut merged = a.borrow().clone();
            let _ = merged.merge(&b.borrow());
            RuntimeValue::Crdt(std::rc::Rc::new(std::cell::RefCell::new(merged)))
        }
        // A live COUNTER CRDT receiving an incoming Int-projected total from the readable relay wire
        // (`{"points":3}`): a state-based counter cannot reconstruct a peer's per-replica breakdown
        // from a scalar total, so the local per-replica counter is authoritative and STANDS — folding a
        // bare total would be non-idempotent (double-count on redelivery) and would clobber the live
        // CRDT. Cross-replica counter convergence uses the in-process `Merge` (real per-replica MAX);
        // the Int wire form is the READABLE projection, not the merge channel.
        (RuntimeValue::Crdt(a), _) if a.borrow().counter_value().is_some() => {
            RuntimeValue::Crdt(a.clone())
        }
        _ => incoming,
    }
}

/// The state-based CRDT LATTICE JOIN of one struct field — the IDEMPOTENT sibling of `crdt_merge_field`:
/// identical EXCEPT a bare-`Int` field takes per-field MAX (not ADD), so applying a redelivered state twice
/// never double-counts. Counter/other fields defer to `crdt_merge_field` (already idempotent / LWW).
pub fn crdt_join_field(current: &RuntimeValue, incoming: RuntimeValue) -> RuntimeValue {
    match (current, &incoming) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => RuntimeValue::Int((*a).max(*b)),
        _ => crdt_merge_field(current, incoming),
    }
}

/// One CRDT field value → its JSON wire form. Scalars map directly; a `Set`
/// becomes a JSON array of its (scalar) members. Returns `None` for values that
/// are not CRDT-syncable.
fn field_to_json(value: &RuntimeValue) -> Option<serde_json::Value> {
    use serde_json::{json, Value};
    Some(match value {
        RuntimeValue::Int(n) => json!(n),
        RuntimeValue::Bool(b) => json!(b),
        RuntimeValue::Float(f) => json!(f),
        RuntimeValue::Text(s) => json!(s.as_str()),
        RuntimeValue::Nothing => Value::Null,
        RuntimeValue::Set(items) => {
            Value::Array(items.borrow().iter().filter_map(field_to_json).collect())
        }
        // A map ships as a TAGGED array of [key, value] pairs so it is unambiguous from a
        // plain Set array, and any key type (not just strings) round-trips.
        RuntimeValue::Map(m) => {
            let pairs: Vec<Value> = m
                .borrow()
                .iter()
                .filter_map(|(k, v)| Some(Value::Array(vec![field_to_json(k)?, field_to_json(v)?])))
                .collect();
            json!({ "__map": pairs })
        }
        // A live COUNTER CRDT (`ConvergentCount`/`Tally`) projects to its Int TOTAL — the readable
        // relay wire form `{"points":3}` (Spec A: a counter reads/serializes as its scalar total). A
        // non-counter CRDT (OR-Set / RGA / MV-register) has no scalar projection and ships its full
        // state through the δ-state `Sync` path instead, so it is `None` here.
        RuntimeValue::Crdt(rc) => match rc.borrow().counter_value() {
            Some(n) => json!(n),
            None => return None,
        },
        _ => return None,
    })
}

/// A JSON wire value → a CRDT field value. The inverse of [`field_to_json`]; a
/// JSON array reconstructs a `Set`.
fn field_from_json(value: &serde_json::Value) -> RuntimeValue {
    use serde_json::Value;
    match value {
        Value::Bool(b) => RuntimeValue::Bool(*b),
        Value::String(s) => RuntimeValue::Text(std::rc::Rc::new(s.clone())),
        Value::Number(n) => match n.as_i64() {
            Some(i) => RuntimeValue::Int(i),
            None => RuntimeValue::Float(n.as_f64().unwrap_or(0.0)),
        },
        Value::Array(items) => RuntimeValue::Set(std::rc::Rc::new(std::cell::RefCell::new(
            items.iter().map(field_from_json).collect(),
        ))),
        // A tagged `{"__map": [[k,v],…]}` reconstructs a map (the inverse of `field_to_json`).
        Value::Object(o) if o.contains_key("__map") => {
            let mut map = crate::interpreter::MapStorage::default();
            if let Some(Value::Array(pairs)) = o.get("__map") {
                for p in pairs {
                    if let Value::Array(kv) = p {
                        if kv.len() == 2 {
                            map.insert(field_from_json(&kv[0]), field_from_json(&kv[1]));
                        }
                    }
                }
            }
            RuntimeValue::Map(std::rc::Rc::new(std::cell::RefCell::new(map)))
        }
        _ => RuntimeValue::Nothing,
    }
}

/// Encode a CRDT value for the relay wire: a JSON object mapping each Int field
/// to its value. A bare `Int` counter uses the empty field name. `Nothing` (and
/// any non-counter value) has nothing to publish. The format is browser-friendly
/// and field-addressable, so structs merge field-by-field on the other side.
pub fn crdt_to_wire(value: &RuntimeValue) -> Option<Vec<u8>> {
    use serde_json::{Map, Value};
    let mut map = Map::new();
    match value {
        RuntimeValue::Nothing => return None,
        RuntimeValue::Struct(s) => {
            for (k, v) in &s.fields {
                if let Some(j) = field_to_json(v) {
                    map.insert(k.clone(), j);
                }
            }
        }
        // A bare counter / register / set uses the unnamed field.
        other => match field_to_json(other) {
            Some(j) => {
                map.insert(String::new(), j);
            }
            None => return None,
        },
    }
    serde_json::to_vec(&Value::Object(map)).ok()
}

/// Merge a wire-encoded CRDT value (from [`crdt_to_wire`]) into `local`, field by
/// field through [`crdt_merge_field`] — counters add, sets union, registers take
/// the latest. A struct merges each named field; a bare value the unnamed field.
/// Malformed bytes leave `local` unchanged.
pub fn crdt_merge_wire(local: RuntimeValue, bytes: &[u8]) -> RuntimeValue {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_slice::<serde_json::Value>(bytes)
    else {
        return local;
    };
    match local {
        RuntimeValue::Struct(mut s) => {
            for (k, v) in map {
                let incoming = field_from_json(&v);
                let current = s.fields.get(&k).cloned().unwrap_or(RuntimeValue::Nothing);
                s.fields.insert(k, crdt_merge_field(&current, incoming));
            }
            RuntimeValue::Struct(s)
        }
        other => match map.get("") {
            Some(v) => crdt_merge_field(&other, field_from_json(v)),
            None => other,
        },
    }
}

/// `not x` — logical negation of truthiness (Bool out). The bitwise complement is `~`.
pub fn not_value(val: RuntimeValue) -> Result<RuntimeValue, String> {
    if let Some(r) = exact::not(&val, &not_value) {
        return r;
    }
    Ok(RuntimeValue::Bool(!val.is_truthy()))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/arith/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/arith/bigint_exact_arithmetic.rs"]
mod bigint_exact_arithmetic;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/arith/rational_exact_arithmetic.rs"]
mod rational_exact_arithmetic;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/arith/word_tests.rs"]
mod word_tests;
