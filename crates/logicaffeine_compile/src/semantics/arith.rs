//! Arithmetic, logical, and bitwise operators.

use std::rc::Rc;

use logicaffeine_base::{BigInt, Complex, Decimal, Modular, Rational, WordVal};

use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::RuntimeValue;

use super::compare::{compare, values_equal};
use super::temporal::date_add_span;

/// View an integer value — narrow `Int` or wide `BigInt` — as a `BigInt`, for the
/// exact-arithmetic path that the overflow-safe operators promote into.
fn big_of(v: &RuntimeValue) -> Option<BigInt> {
    match v {
        RuntimeValue::Int(n) => Some(BigInt::from_i64(*n)),
        RuntimeValue::BigInt(b) => Some((**b).clone()),
        _ => None,
    }
}

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
        RuntimeValue::BigInt(b) => Some(Complex::from_rational(Rational::from_bigint((**b).clone()))),
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
            None => Err(format!("cannot {op_name} values in different modular rings")),
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
fn quantity_binop(left: &RuntimeValue, right: &RuntimeValue, op: char) -> Option<Result<RuntimeValue, String>> {
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
                format!("cannot add quantities of different dimensions ({} vs {})", a.q.dimension(), b.q.dimension())
            }),
            '-' => a.q.sub(&b.q).map(|q| mk(q, a.unit.clone())).ok_or_else(|| {
                format!("cannot subtract quantities of different dimensions ({} vs {})", a.q.dimension(), b.q.dimension())
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
                None => Err("cannot divide by a zero quantity".to_string()),
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
                        None => return Some(Err("cannot divide a quantity by zero".to_string())),
                    }
                };
                Ok(mk(Quantity::si(mag, a.q.dimension()), a.unit.clone()))
            }
            None => return None,
        },
        // `k * q` — scalar on the left (multiplication commutes).
        (scalar, RuntimeValue::Quantity(b)) if op == '*' => match rat_of(scalar) {
            Some(k) => Ok(mk(Quantity::si(b.q.magnitude_si().mul(&k), b.q.dimension()), b.unit.clone())),
            None => return None,
        },
        _ => return None,
    })
}

/// Money arithmetic. `+ −` require the SAME currency (a typed error otherwise, like a dimension
/// mismatch); `×` and `÷` scale by an exact number (Int/Decimal), re-quantised to the currency's
/// minor unit; a same-currency `Money ÷ Money` is the dimensionless ratio. Returns `None` when
/// neither operand is Money (so the normal numeric dispatch runs); `Some(Err)` on a currency mismatch.
fn money_binop(left: &RuntimeValue, right: &RuntimeValue, op: char) -> Option<Result<RuntimeValue, String>> {
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
        format!("cannot {verb} money of different currencies ({} vs {})", a.currency.code, b.currency.code)
    };
    Some(match (left, right) {
        (RuntimeValue::Money(a), RuntimeValue::Money(b)) => match op {
            '+' => a.add(b).map(mk).ok_or_else(|| mismatch(a, b, "add")),
            '-' => a.sub(b).map(mk).ok_or_else(|| mismatch(a, b, "subtract")),
            '/' => match a.ratio(b) {
                Some(r) => Ok(RuntimeValue::from_rational(r)),
                None if a.currency != b.currency => Err(mismatch(a, b, "compare")),
                None => Err("cannot divide money by a zero amount".to_string()),
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
                None => Err("cannot divide money by zero".to_string()),
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
    format!("cannot combine Word{} and Word{} — width mismatch", a.width(), b.width())
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
            let n = shift_count(right).ok_or_else(|| "shift count must be an integer".to_string())?;
            let r = if matches!(op, Shl) { a.shl(n) } else { a.shr(n) };
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
        And => a.bitand(*b),
        Or => a.bitor(*b),
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
        BinaryOpKind::Eq => Ok(RuntimeValue::Bool(values_equal(&left, &right))),
        BinaryOpKind::NotEq => Ok(RuntimeValue::Bool(!values_equal(&left, &right))),
        BinaryOpKind::ApproxEq => approx_eq(left, right),
        BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq => {
            compare(op, &left, &right)
        }
        // Logical words: truthiness in, Bool out. The bitwise spellings are `&`/`|` (BitAnd/BitOr).
        BinaryOpKind::And => Ok(RuntimeValue::Bool(left.is_truthy() && right.is_truthy())),
        BinaryOpKind::Or => Ok(RuntimeValue::Bool(left.is_truthy() || right.is_truthy())),
        BinaryOpKind::Concat => concat(left, right),
        BinaryOpKind::SeqConcat => seq_concat(left, right),
        BinaryOpKind::BitXor => match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a ^ b)),
            // Boolean XOR (`a ≠ b`) — matches the codegen's `a ^ b` on Rust `bool`, so the tiers
            // agree. (Word and lane XOR are taken on the fast paths above.)
            (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => Ok(RuntimeValue::Bool(a ^ b)),
            // On Sets, `^` is the symmetric difference.
            (RuntimeValue::Set(_), RuntimeValue::Set(_)) => set_binop(&left, &right, SetOp::SymmetricDifference),
            _ => Err("Bitwise XOR requires integer, boolean, or Set operands".to_string()),
        },
        // `&` — bitwise AND on Int, intersection on Sets.
        BinaryOpKind::BitAnd => match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a & b)),
            (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => Ok(RuntimeValue::Bool(a & b)),
            (RuntimeValue::Set(_), RuntimeValue::Set(_)) => set_binop(&left, &right, SetOp::Intersection),
            _ => Err("`&` requires integer, boolean, or Set operands".to_string()),
        },
        // `|` — bitwise OR on Int, union on Sets.
        BinaryOpKind::BitOr => match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a | b)),
            (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => Ok(RuntimeValue::Bool(a | b)),
            (RuntimeValue::Set(_), RuntimeValue::Set(_)) => set_binop(&left, &right, SetOp::Union),
            _ => Err("`|` requires integer, boolean, or Set operands".to_string()),
        },
        // Shift counts are truncated to u32 and masked mod 64 (the wrapping spec).
        BinaryOpKind::Shl => match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                Ok(RuntimeValue::Int(a.wrapping_shl(b as u32)))
            }
            _ => Err("Left shift requires integer operands".to_string()),
        },
        BinaryOpKind::Shr => match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                Ok(RuntimeValue::Int(a.wrapping_shr(b as u32)))
            }
            _ => Err("Right shift requires integer operands".to_string()),
        },
    }
}

pub fn add(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    // Words add in the ring ℤ/2ᵏ (wrapping). Here as well as in `binary_op` so the VM, which
    // calls `add` directly, stays byte-identical to the tree-walker on the Word path.
    if let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (&left, &right) {
        return a.add(*b).map(RuntimeValue::Word).ok_or_else(|| word_width_err(*a, *b));
    }
    // SIMD lane vectors add lane-wise in ℤ/2³² (the ChaCha quarter-round's `a += b`).
    if let (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) = (&left, &right) {
        return a.add(**b).map(|v| RuntimeValue::Lanes(std::rc::Rc::new(v))).ok_or_else(|| {
            format!("cannot add {} and {} — lane-config mismatch", a.type_name(), b.type_name())
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
    if let Some(r) = complex_binop(&left, &right, "add", |a, b| a.add(b)) {
        return r;
    }
    if let Some(r) = decimal_binop(&left, &right, |a, b| a.add(b), |a, b| a.add(b), |a, b| a + b) {
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
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::from_bigint(a.add(b))),
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            Ok(RuntimeValue::from_bigint(a.add(&BigInt::from_i64(*b))))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(BigInt::from_i64(*a).add(b)))
        }
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a + b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 + b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a + *b as f64)),
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a.to_f64() + b)),
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::Float(a + b.to_f64())),
        // Rational keeps arithmetic EXACT: any Int/BigInt/Rational mix involving a
        // Rational stays exact (downsized to Int when it reduces to a whole number); a
        // Float operand makes the result Float (floats are inexact by choice).
        (RuntimeValue::Rational(a), RuntimeValue::Rational(b)) => {
            Ok(RuntimeValue::from_rational(a.add(b)))
        }
        (RuntimeValue::Rational(_), RuntimeValue::Int(_))
        | (RuntimeValue::Int(_), RuntimeValue::Rational(_))
        | (RuntimeValue::Rational(_), RuntimeValue::BigInt(_))
        | (RuntimeValue::BigInt(_), RuntimeValue::Rational(_)) => {
            Ok(RuntimeValue::from_rational(rat_of(&left).unwrap().add(&rat_of(&right).unwrap())))
        }
        (RuntimeValue::Rational(r), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(r.to_f64() + b)),
        (RuntimeValue::Float(a), RuntimeValue::Rational(r)) => Ok(RuntimeValue::Float(a + r.to_f64())),
        (RuntimeValue::Text(a), RuntimeValue::Text(b)) => {
            Ok(RuntimeValue::Text(Rc::new(format!("{}{}", a, b))))
        }
        (RuntimeValue::Text(a), other) => {
            Ok(RuntimeValue::Text(Rc::new(format!("{}{}", a, other.to_display_string()))))
        }
        (other, RuntimeValue::Text(b)) => {
            Ok(RuntimeValue::Text(Rc::new(format!("{}{}", other.to_display_string(), b))))
        }
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => {
            Ok(RuntimeValue::Duration(a.wrapping_add(*b)))
        }
        (RuntimeValue::Date(days), RuntimeValue::Span { months, days: span_days }) => {
            Ok(RuntimeValue::Date(date_add_span(*days, *months, *span_days)))
        }
        // A Moment plus a CIVIL Span is calendar arithmetic (`m + 1 month` clamps end-of-month,
        // respects leap years, keeps the wall time); commutes. Contrast the physical Duration below.
        (RuntimeValue::Moment(nanos), RuntimeValue::Span { months, days: span_days })
        | (RuntimeValue::Span { months, days: span_days }, RuntimeValue::Moment(nanos)) => Ok(
            RuntimeValue::Moment(super::temporal::moment_add_span(*nanos, *months, *span_days)),
        ),
        // A Moment plus a physical Duration is a later Moment (e.g. `m + 90 seconds`); commutes.
        (RuntimeValue::Moment(nanos), RuntimeValue::Duration(d))
        | (RuntimeValue::Duration(d), RuntimeValue::Moment(nanos)) => {
            Ok(RuntimeValue::Moment(nanos.wrapping_add(*d)))
        }
        // `xs + ys` concatenates sequences — exactly `xs followed by ys`
        // (a fresh sequence; neither operand is mutated).
        (RuntimeValue::List(_), RuntimeValue::List(_)) => {
            seq_concat(left.clone(), right.clone())
        }
        _ => Err(format!("Cannot add {} and {}", left.type_name(), right.type_name())),
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
        (Some(a), Some(b)) => Ok(RuntimeValue::Bool(logicaffeine_data::ops::logos_approx_eq(a, b))),
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
            Ok(RuntimeValue::List(Rc::new(std::cell::RefCell::new(ListRepr::from_values(items)))))
        }
        _ => Err("`followed by` requires two sequences (merge two sequences into one)".to_string()),
    }
}

pub fn subtract(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    // `a - b` on Sets is the difference (`a without b` is the English form).
    if matches!((&left, &right), (RuntimeValue::Set(_), RuntimeValue::Set(_))) {
        return set_binop(&left, &right, SetOp::Difference);
    }
    if let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (&left, &right) {
        return a.sub(*b).map(RuntimeValue::Word).ok_or_else(|| word_width_err(*a, *b));
    }
    // SIMD lane vectors subtract lane-wise (the NTT butterfly's `a - t` over Word16 lanes).
    if let (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) = (&left, &right) {
        return a.sub(**b).map(|v| RuntimeValue::Lanes(std::rc::Rc::new(v))).ok_or_else(|| {
            format!("cannot subtract {} and {} — lane-config mismatch", a.type_name(), b.type_name())
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
    if let Some(r) = complex_binop(&left, &right, "subtract", |a, b| a.sub(b)) {
        return r;
    }
    if let Some(r) = decimal_binop(&left, &right, |a, b| a.sub(b), |a, b| a.sub(b), |a, b| a - b) {
        return Ok(r);
    }
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(match a.checked_sub(*b) {
            Some(s) => RuntimeValue::Int(s),
            None => RuntimeValue::from_bigint(BigInt::from_i64(*a).sub(&BigInt::from_i64(*b))),
        }),
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::from_bigint(a.sub(b))),
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            Ok(RuntimeValue::from_bigint(a.sub(&BigInt::from_i64(*b))))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(BigInt::from_i64(*a).sub(b)))
        }
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a - b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 - b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a - *b as f64)),
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a.to_f64() - b)),
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::Float(a - b.to_f64())),
        (RuntimeValue::Rational(a), RuntimeValue::Rational(b)) => {
            Ok(RuntimeValue::from_rational(a.sub(b)))
        }
        (RuntimeValue::Rational(_), RuntimeValue::Int(_))
        | (RuntimeValue::Int(_), RuntimeValue::Rational(_))
        | (RuntimeValue::Rational(_), RuntimeValue::BigInt(_))
        | (RuntimeValue::BigInt(_), RuntimeValue::Rational(_)) => {
            Ok(RuntimeValue::from_rational(rat_of(&left).unwrap().sub(&rat_of(&right).unwrap())))
        }
        (RuntimeValue::Rational(r), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(r.to_f64() - b)),
        (RuntimeValue::Float(a), RuntimeValue::Rational(r)) => Ok(RuntimeValue::Float(a - r.to_f64())),
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => {
            Ok(RuntimeValue::Duration(a.wrapping_sub(*b)))
        }
        (RuntimeValue::Date(days), RuntimeValue::Span { months, days: span_days }) => {
            Ok(RuntimeValue::Date(date_add_span(*days, -*months, -*span_days)))
        }
        // A Moment minus a CIVIL Span steps the calendar backward (`m - 1 month`), mirroring the add.
        (RuntimeValue::Moment(nanos), RuntimeValue::Span { months, days: span_days }) => Ok(
            RuntimeValue::Moment(super::temporal::moment_add_span(*nanos, -*months, -*span_days)),
        ),
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
    // Any Float operand → inexact f64 power.
    if matches!(base, RuntimeValue::Float(_)) || matches!(exp, RuntimeValue::Float(_)) {
        let b = num_f64(&base)
            .ok_or_else(|| format!("cannot raise {} to a power", base.type_name()))?;
        let e = num_f64(&exp)
            .ok_or_else(|| format!("cannot raise to a {} power", exp.type_name()))?;
        return Ok(RuntimeValue::Float(b.powf(e)));
    }
    match (&base, &exp) {
        (RuntimeValue::Int(b), RuntimeValue::Int(e)) => int_power(*b, *e),
        (RuntimeValue::BigInt(b), RuntimeValue::Int(e)) => {
            let ue = u32::try_from(*e)
                .map_err(|_| "negative or too-large exponent on an integer (use a Float base)".to_string())?;
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
    if let (RuntimeValue::Word(a), RuntimeValue::Word(b)) = (&left, &right) {
        return a.mul(*b).map(RuntimeValue::Word).ok_or_else(|| word_width_err(*a, *b));
    }
    // SIMD lane vectors multiply lane-wise low-16 (`vpmullw`, the NTT's `*` over Word16 lanes).
    if let (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) = (&left, &right) {
        return a.mullo(**b).map(|v| RuntimeValue::Lanes(std::rc::Rc::new(v))).ok_or_else(|| {
            format!("cannot multiply {} and {} — lane op undefined", a.type_name(), b.type_name())
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
    if let Some(r) = complex_binop(&left, &right, "multiply", |a, b| a.mul(b)) {
        return r;
    }
    if let Some(r) = decimal_binop(&left, &right, |a, b| a.mul(b), |a, b| a.mul(b), |a, b| a * b) {
        return Ok(r);
    }
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(match a.checked_mul(*b) {
            Some(p) => RuntimeValue::Int(p),
            None => RuntimeValue::from_bigint(BigInt::from_i64(*a).mul(&BigInt::from_i64(*b))),
        }),
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::from_bigint(a.mul(b))),
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            Ok(RuntimeValue::from_bigint(a.mul(&BigInt::from_i64(*b))))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(BigInt::from_i64(*a).mul(b)))
        }
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a * b)),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 * b)),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a * *b as f64)),
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a.to_f64() * b)),
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::Float(a * b.to_f64())),
        (RuntimeValue::Rational(a), RuntimeValue::Rational(b)) => {
            Ok(RuntimeValue::from_rational(a.mul(b)))
        }
        (RuntimeValue::Rational(_), RuntimeValue::Int(_))
        | (RuntimeValue::Int(_), RuntimeValue::Rational(_))
        | (RuntimeValue::Rational(_), RuntimeValue::BigInt(_))
        | (RuntimeValue::BigInt(_), RuntimeValue::Rational(_)) => {
            Ok(RuntimeValue::from_rational(rat_of(&left).unwrap().mul(&rat_of(&right).unwrap())))
        }
        (RuntimeValue::Rational(r), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(r.to_f64() * b)),
        (RuntimeValue::Float(a), RuntimeValue::Rational(r)) => Ok(RuntimeValue::Float(a * r.to_f64())),
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
            Ok(RuntimeValue::List(Rc::new(std::cell::RefCell::new(ListRepr::from_values(out)))))
        }
        _ => Err(format!(
            "Cannot multiply {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

pub fn divide(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
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
                    Err("cannot divide values in different modular rings".to_string())
                } else {
                    a.div(b).map(|r| RuntimeValue::Modular(Rc::new(r))).ok_or_else(|| {
                        "modular divisor has no inverse (not coprime to the modulus)".to_string()
                    })
                }
            }
            _ => Err(format!("Cannot divide {} by {}", left.type_name(), right.type_name())),
        };
    }
    // Complex division stays Complex (the field is closed); `None` on a zero divisor.
    if matches!(left, RuntimeValue::Complex(_)) || matches!(right, RuntimeValue::Complex(_)) {
        return match (complex_of(&left), complex_of(&right)) {
            (Some(a), Some(b)) => a
                .div(&b)
                .map(|c| RuntimeValue::Complex(Rc::new(c)))
                .ok_or_else(|| "Division by zero".to_string()),
            _ => Err(format!("Cannot divide {} by {}", left.type_name(), right.type_name())),
        };
    }
    // A Decimal divides EXACTLY into the Rational tower (base-10 division need not
    // terminate, so it does not stay a Decimal); a Float operand divides as Float.
    if matches!(left, RuntimeValue::Decimal(_)) || matches!(right, RuntimeValue::Decimal(_)) {
        if let (Some(a), Some(b)) = (rat_of(&left), rat_of(&right)) {
            return a
                .div(&b)
                .map(RuntimeValue::from_rational)
                .ok_or_else(|| "Division by zero".to_string());
        }
        if let (Some(a), Some(b)) = (num_f64(&left), num_f64(&right)) {
            if b == 0.0 {
                return Err("Division by zero".to_string());
            }
            return Ok(RuntimeValue::Float(a / b));
        }
    }
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err("Division by zero".to_string());
            }
            // checked_div is None only for i64::MIN / -1, whose true quotient
            // (2^63) overflows i64 → promote rather than wrap.
            Ok(match a.checked_div(*b) {
                Some(q) => RuntimeValue::Int(q),
                None => RuntimeValue::from_bigint(int_div(*a, *b).0),
            })
        }
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(a.div_rem(b).expect("BigInt divisor is never zero").0))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::from_bigint(a.div_rem(&BigInt::from_i64(*b)).unwrap().0))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(BigInt::from_i64(*a).div_rem(b).unwrap().0))
        }
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Float(a / b))
        }
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Float(*a as f64 / b))
        }
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Float(a / *b as f64))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Float(a.to_f64() / b))
        }
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::Float(a / b.to_f64())),
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
            .ok_or_else(|| "Division by zero".to_string()),
        (RuntimeValue::Rational(r), RuntimeValue::Float(b)) => {
            if *b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Ok(RuntimeValue::Float(r.to_f64() / b))
        }
        (RuntimeValue::Float(a), RuntimeValue::Rational(r)) => Ok(RuntimeValue::Float(a / r.to_f64())),
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
    BigInt::from_i64(a).div_rem(&BigInt::from_i64(b)).expect("nonzero divisor")
}

/// FLOOR division — the runtime of [`BinaryOpKind::FloorDivide`] (`a // b`), the quotient
/// rounded toward NEGATIVE INFINITY. On exact operands (Int/BigInt/Rational/Decimal) the
/// result is the exact quotient's floor as an integer (`-7 // 2 → -4`, `10^30 // 7` exact);
/// a Float operand floors the float quotient but stays Float (`7.5 // 2 → 3.0`, Python
/// semantics). Distinct from [`divide`], which truncates toward zero. Zero divisor is loud.
pub fn floor_divide(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    // Float domain: floor the float quotient, staying inexact.
    if matches!(left, RuntimeValue::Float(_)) || matches!(right, RuntimeValue::Float(_)) {
        if let (Some(a), Some(b)) = (num_f64(&left), num_f64(&right)) {
            if b == 0.0 {
                return Err("Division by zero".to_string());
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
            .ok_or_else(|| "Division by zero".to_string());
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
    match (&left, &right) {
        // Every Int/BigInt/Rational combination divides to an EXACT value — downsized to
        // an Int/BigInt when it reduces to a whole number, else an exact Rational.
        (a, b) if rat_of(a).is_some() && rat_of(b).is_some() => rat_of(&left)
            .unwrap()
            .div(&rat_of(&right).unwrap())
            .map(RuntimeValue::from_rational)
            .ok_or_else(|| "Division by zero".to_string()),
        // A Float operand makes it Float — defer to the Float-aware `divide` (whose Float
        // arms don't truncate anyway).
        (RuntimeValue::Float(_), _) | (_, RuntimeValue::Float(_)) => divide(left, right),
        _ => Err(format!("Cannot divide {} by {}", left.type_name(), right.type_name())),
    }
}

pub fn modulo(left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
    match (&left, &right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err("Modulo by zero".to_string());
            }
            // checked_rem is None only for i64::MIN % -1, whose true remainder is 0.
            Ok(RuntimeValue::Int(a.checked_rem(*b).unwrap_or(0)))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(a.div_rem(b).expect("BigInt divisor is never zero").1))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => {
            if *b == 0 {
                return Err("Modulo by zero".to_string());
            }
            Ok(RuntimeValue::from_bigint(a.div_rem(&BigInt::from_i64(*b)).unwrap().1))
        }
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::from_bigint(BigInt::from_i64(*a).div_rem(b).unwrap().1))
        }
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
                    (Some(RuntimeValue::Int(x)), RuntimeValue::Int(y)) => RuntimeValue::Int((*x).max(*y)),
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
        _ => incoming,
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
    Ok(RuntimeValue::Bool(!val.is_truthy()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::StructValue;
    use std::collections::HashMap;

    #[test]
    fn money_arithmetic_and_comparison_are_exact_and_currency_safe() {
        use crate::ast::stmt::BinaryOpKind;
        use crate::semantics::builtins::{call_builtin, BuiltinId};
        use crate::semantics::compare::compare;
        let money = |s: &str, code: &str| {
            call_builtin(
                BuiltinId::Money,
                vec![
                    RuntimeValue::Decimal(Rc::new(logicaffeine_base::Decimal::parse(s).unwrap())),
                    RuntimeValue::Text(Rc::new(code.to_string())),
                ],
            )
            .unwrap()
        };
        let show = |v: &RuntimeValue| v.to_display_string();

        // Construction + display at the currency's minor unit.
        assert_eq!(show(&money("19.99", "USD")), "19.99 USD");
        // Same-currency add/sub are EXACT (no float drift) and keep the currency.
        assert_eq!(show(&add(money("0.10", "USD"), money("0.20", "USD")).unwrap()), "0.30 USD");
        assert_eq!(show(&add(money("19.99", "USD"), money("5.00", "USD")).unwrap()), "24.99 USD");
        assert_eq!(show(&subtract(money("24.99", "USD"), money("5.00", "USD")).unwrap()), "19.99 USD");
        // Cross-currency add/sub are a typed error (no common meaning).
        assert!(add(money("5.00", "USD"), money("1.00", "EUR")).is_err());
        assert!(subtract(money("5.00", "USD"), money("1.00", "EUR")).is_err());
        // Scale by a number (commutes); divide a bill by a number.
        assert_eq!(show(&multiply(money("19.99", "USD"), RuntimeValue::Int(3)).unwrap()), "59.97 USD");
        assert_eq!(show(&multiply(RuntimeValue::Int(3), money("19.99", "USD")).unwrap()), "59.97 USD");
        assert_eq!(show(&divide(money("10.00", "USD"), RuntimeValue::Int(4)).unwrap()), "2.50 USD");
        // Same-currency ratio (30/10 = 3, narrows to Int); money × money is refused.
        assert!(matches!(
            divide(money("30.00", "USD"), money("10.00", "USD")).unwrap(),
            RuntimeValue::Int(3) | RuntimeValue::Rational(_)
        ));
        assert!(multiply(money("2.00", "USD"), money("2.00", "USD")).is_err());
        // Ordering within a currency; ordering across currencies is a typed error.
        assert_eq!(
            compare(BinaryOpKind::Gt, &money("5.00", "USD"), &money("1.00", "USD")).unwrap(),
            RuntimeValue::Bool(true)
        );
        assert!(compare(BinaryOpKind::Lt, &money("5.00", "USD"), &money("1.00", "EUR")).is_err());
        // `money()` refuses an inexact amount and an unknown currency.
        assert!(call_builtin(
            BuiltinId::Money,
            vec![RuntimeValue::Float(1.5), RuntimeValue::Text(Rc::new("USD".to_string()))]
        )
        .is_err());
        assert!(call_builtin(
            BuiltinId::Money,
            vec![RuntimeValue::Int(5), RuntimeValue::Text(Rc::new("XYZ".to_string()))]
        )
        .is_err());
    }

    #[test]
    fn crdt_wire_int_into_nothing_takes_value() {
        let bytes = crdt_to_wire(&RuntimeValue::Int(7)).unwrap();
        assert!(matches!(crdt_merge_wire(RuntimeValue::Nothing, &bytes), RuntimeValue::Int(7)));
    }

    #[test]
    fn crdt_wire_int_merge_adds() {
        let bytes = crdt_to_wire(&RuntimeValue::Int(5)).unwrap();
        assert!(matches!(crdt_merge_wire(RuntimeValue::Int(3), &bytes), RuntimeValue::Int(8)));
    }

    #[test]
    fn crdt_wire_struct_merges_fieldwise() {
        let mut fields = HashMap::new();
        fields.insert("a".to_string(), RuntimeValue::Int(2));
        fields.insert("b".to_string(), RuntimeValue::Int(4));
        let incoming = RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Counter".into(),
            fields,
        }));
        let bytes = crdt_to_wire(&incoming).unwrap();

        let mut local_fields = HashMap::new();
        local_fields.insert("a".to_string(), RuntimeValue::Int(1)); // b absent ⇒ 0
        let local = RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Counter".into(),
            fields: local_fields,
        }));
        match crdt_merge_wire(local, &bytes) {
            RuntimeValue::Struct(s) => {
                assert!(matches!(s.fields.get("a"), Some(RuntimeValue::Int(3))), "1 + 2");
                assert!(matches!(s.fields.get("b"), Some(RuntimeValue::Int(4))), "0 + 4");
            }
            other => panic!("expected a struct, got {other:?}"),
        }
    }

    #[test]
    fn crdt_wire_nothing_has_nothing_to_publish() {
        assert!(crdt_to_wire(&RuntimeValue::Nothing).is_none());
    }

    #[test]
    fn crdt_wire_bool_lww_takes_incoming() {
        let bytes = crdt_to_wire(&RuntimeValue::Bool(true)).unwrap();
        assert!(matches!(
            crdt_merge_wire(RuntimeValue::Bool(false), &bytes),
            RuntimeValue::Bool(true)
        ));
    }

    #[test]
    fn crdt_wire_text_lww() {
        let bytes = crdt_to_wire(&RuntimeValue::Text(std::rc::Rc::new("hi".into()))).unwrap();
        match crdt_merge_wire(RuntimeValue::Nothing, &bytes) {
            RuntimeValue::Text(s) => assert_eq!(&*s, "hi"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn crdt_wire_float_roundtrips() {
        let bytes = crdt_to_wire(&RuntimeValue::Float(2.5)).unwrap();
        match crdt_merge_wire(RuntimeValue::Nothing, &bytes) {
            RuntimeValue::Float(f) => assert_eq!(f, 2.5),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    fn set_of(ns: &[i64]) -> RuntimeValue {
        RuntimeValue::Set(std::rc::Rc::new(std::cell::RefCell::new(
            ns.iter().map(|n| RuntimeValue::Int(*n)).collect(),
        )))
    }

    #[test]
    fn crdt_wire_set_unions() {
        let bytes = crdt_to_wire(&set_of(&[2, 3])).unwrap();
        match crdt_merge_wire(set_of(&[1, 2]), &bytes) {
            RuntimeValue::Set(items) => {
                let v = items.borrow();
                assert_eq!(v.len(), 3, "{{1,2}} ∪ {{2,3}} = {{1,2,3}}");
                for n in [1, 2, 3] {
                    assert!(
                        v.iter().any(|x| matches!(x, RuntimeValue::Int(m) if *m == n)),
                        "missing {n}"
                    );
                }
            }
            other => panic!("expected Set, got {other:?}"),
        }
    }

    #[test]
    fn crdt_wire_struct_mixed_types_merge_each_by_its_rule() {
        let mut fields = HashMap::new();
        fields.insert("hits".to_string(), RuntimeValue::Int(3));
        fields.insert("title".to_string(), RuntimeValue::Text(std::rc::Rc::new("v2".into())));
        fields.insert("tags".to_string(), set_of(&[9]));
        let incoming = RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Page".into(),
            fields,
        }));
        let bytes = crdt_to_wire(&incoming).unwrap();

        let mut local = HashMap::new();
        local.insert("hits".to_string(), RuntimeValue::Int(1));
        local.insert("title".to_string(), RuntimeValue::Text(std::rc::Rc::new("v1".into())));
        local.insert("tags".to_string(), set_of(&[7]));
        let local = RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Page".into(),
            fields: local,
        }));

        match crdt_merge_wire(local, &bytes) {
            RuntimeValue::Struct(s) => {
                assert!(matches!(s.fields.get("hits"), Some(RuntimeValue::Int(4))), "counter 1+3");
                assert!(
                    matches!(s.fields.get("title"), Some(RuntimeValue::Text(t)) if &***t == "v2"),
                    "LWW register"
                );
                match s.fields.get("tags") {
                    Some(RuntimeValue::Set(items)) => {
                        assert_eq!(items.borrow().len(), 2, "set union {{7}} ∪ {{9}}")
                    }
                    other => panic!("tags not a set: {other:?}"),
                }
            }
            other => panic!("expected struct, got {other:?}"),
        }
    }

    #[test]
    fn error_messages_are_canonical() {
        let e = add(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
        assert_eq!(e, "Cannot add Bool and Nothing");
        let e = subtract(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
        assert_eq!(e, "Cannot subtract Nothing from Bool");
        let e = multiply(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
        assert_eq!(e, "Cannot multiply Bool and Nothing");
        let e = divide(RuntimeValue::Int(1), RuntimeValue::Int(0)).unwrap_err();
        assert_eq!(e, "Division by zero");
        let e = divide(RuntimeValue::Float(1.0), RuntimeValue::Float(0.0)).unwrap_err();
        assert_eq!(e, "Division by zero");
        let e = modulo(RuntimeValue::Int(1), RuntimeValue::Int(0)).unwrap_err();
        assert_eq!(e, "Modulo by zero");
        // `not` is total — logical negation of truthiness, never an error.
        let r = not_value(RuntimeValue::Nothing).unwrap();
        assert!(matches!(r, RuntimeValue::Bool(true)));
    }

    #[test]
    fn text_add_stringifies_either_side() {
        let r = add(
            RuntimeValue::Text(Rc::new("n=".to_string())),
            RuntimeValue::Int(4),
        )
        .unwrap();
        assert!(matches!(&r, RuntimeValue::Text(s) if **s == "n=4"));
        let r = add(
            RuntimeValue::Int(4),
            RuntimeValue::Text(Rc::new("!".to_string())),
        )
        .unwrap();
        assert!(matches!(&r, RuntimeValue::Text(s) if **s == "4!"));
    }

    #[test]
    fn date_plus_span_is_calendar_aware() {
        // 2024-01-31 (day 19753) + 1 month = 2024-02-29.
        let r = add(
            RuntimeValue::Date(19753),
            RuntimeValue::Span { months: 1, days: 0 },
        )
        .unwrap();
        assert!(matches!(r, RuntimeValue::Date(19782)));
        // And subtraction inverts the span sign.
        let r = subtract(
            RuntimeValue::Date(19782),
            RuntimeValue::Span { months: 0, days: 1 },
        )
        .unwrap();
        assert!(matches!(r, RuntimeValue::Date(19781)));
    }

    #[test]
    fn decimal_arithmetic_stays_exact_and_promotes_correctly() {
        let d = |s: &str| RuntimeValue::Decimal(Rc::new(Decimal::parse(s).unwrap()));
        // +,-,* stay exact Decimal — money keeps its scale, no f64 drift.
        assert_eq!(add(d("19.99"), d("0.01")).unwrap().to_display_string(), "20.00");
        assert_eq!(subtract(d("20.00"), d("0.01")).unwrap().to_display_string(), "19.99");
        assert_eq!(multiply(d("1.1"), d("1.1")).unwrap().to_display_string(), "1.21");
        assert_eq!(add(d("0.1"), d("0.2")).unwrap().to_display_string(), "0.3");
        // Decimal ∘ Int stays Decimal (a count scales money).
        assert_eq!(multiply(d("19.99"), RuntimeValue::Int(3)).unwrap().to_display_string(), "59.97");
        assert!(matches!(add(d("1.50"), RuntimeValue::Int(1)).unwrap(), RuntimeValue::Decimal(_)));
        // Division promotes to the EXACT Rational tower (base-10 division need not terminate).
        let q = divide(d("1"), d("3")).unwrap();
        assert!(matches!(q, RuntimeValue::Rational(_)));
        assert_eq!(q.to_display_string(), "1/3");
        // A Rational operand promotes to exact Rational; a Float operand yields Float.
        let third = RuntimeValue::from_rational(Rational::from_ratio_i64(1, 3).unwrap());
        assert!(matches!(add(d("0.5"), third).unwrap(), RuntimeValue::Rational(_)));
        assert!(matches!(add(d("0.5"), RuntimeValue::Float(0.25)).unwrap(), RuntimeValue::Float(_)));
        // Cross-type ordering: 19.99 (Decimal) > 10 (Int).
        assert!(matches!(
            compare(BinaryOpKind::Gt, &d("19.99"), &RuntimeValue::Int(10)).unwrap(),
            RuntimeValue::Bool(true)
        ));
    }

    #[test]
    fn complex_arithmetic_is_exact_closed_and_unordered() {
        let c = |re: i64, im: i64| {
            RuntimeValue::Complex(Rc::new(Complex::new(Rational::from_i64(re), Rational::from_i64(im))))
        };
        let cq = |rn: i64, rd: i64, in_: i64, id: i64| {
            RuntimeValue::Complex(Rc::new(Complex::new(
                Rational::from_ratio_i64(rn, rd).unwrap(),
                Rational::from_ratio_i64(in_, id).unwrap(),
            )))
        };
        let i = c(0, 1);
        // The headline: i·i = −1, exact.
        assert_eq!(multiply(i.clone(), i.clone()).unwrap(), c(-1, 0));
        // Addition / subtraction.
        assert_eq!(add(c(2, 3), c(1, -1)).unwrap(), c(3, 2));
        assert_eq!(subtract(c(5, 2), c(1, 7)).unwrap(), c(4, -5));
        // (1+i)(1−i) = 2.
        assert_eq!(multiply(c(1, 1), c(1, -1)).unwrap(), c(2, 0));
        // (2+3i)(4+5i) = −7 + 22i.
        assert_eq!(multiply(c(2, 3), c(4, 5)).unwrap(), c(-7, 22));
        // Division stays Complex (closed field): (3+4i)/(1+2i) = (11−2i)/5; z/z = 1.
        assert_eq!(divide(c(3, 4), c(1, 2)).unwrap(), cq(11, 5, -2, 5));
        assert_eq!(divide(c(2, 3), c(2, 3)).unwrap(), c(1, 0));
        // A real embeds (re + 0i): Complex ∘ Int / BigInt / Rational stays Complex.
        assert_eq!(add(c(2, 3), RuntimeValue::Int(5)).unwrap(), c(7, 3));
        assert_eq!(multiply(i.clone(), RuntimeValue::Int(3)).unwrap(), c(0, 3)); // 3i
        let half = RuntimeValue::from_rational(Rational::from_ratio_i64(1, 2).unwrap());
        assert_eq!(add(c(1, 0), half).unwrap(), cq(3, 2, 0, 1));
        // Equality is exact and structural; reduced parts compare equal.
        assert!(matches!(
            binary_op(BinaryOpKind::Eq, c(3, 4), c(3, 4)).unwrap(),
            RuntimeValue::Bool(true)
        ));
        assert!(matches!(
            binary_op(BinaryOpKind::Eq, c(3, 4), c(3, -4)).unwrap(),
            RuntimeValue::Bool(false)
        ));
        // An inexact Float operand is REFUSED (an exact Complex never silently absorbs a float).
        assert!(add(c(1, 1), RuntimeValue::Float(0.5)).is_err());
        assert!(multiply(RuntimeValue::Float(2.0), i.clone()).is_err());
        // Division by zero is a clean error, never a panic.
        assert!(divide(c(1, 1), c(0, 0)).is_err());
        // Complex has NO total order: every relational comparison is a typed error.
        for op in [BinaryOpKind::Lt, BinaryOpKind::Gt, BinaryOpKind::LtEq, BinaryOpKind::GtEq] {
            assert!(compare(op, &c(1, 1), &c(2, 2)).is_err(), "complex is unordered under {op:?}");
        }
        // Display forms.
        assert_eq!(c(3, 4).to_display_string(), "3+4i");
        assert_eq!(c(0, 1).to_display_string(), "i");
        assert_eq!(c(0, -1).to_display_string(), "-i");
        assert_eq!(c(-1, 0).to_display_string(), "-1");
    }

    /// The variant kind (NOT `type_name`, which folds BigInt into "Int").
    fn kind(v: &RuntimeValue) -> &'static str {
        match v {
            RuntimeValue::Int(_) => "Int",
            RuntimeValue::BigInt(_) => "BigInt",
            RuntimeValue::Rational(_) => "Rational",
            RuntimeValue::Decimal(_) => "Decimal",
            RuntimeValue::Complex(_) => "Complex",
            RuntimeValue::Modular(_) => "Modular",
            RuntimeValue::Float(_) => "Float",
            _ => "other", // the gauntlet only uses numeric values; this keeps the return 'static
        }
    }

    /// THE GAUNTLET: every numeric type × every type × {+, −, ×, ÷}, asserting the promotion
    /// KIND and the exact VALUE (for the exact types) or a typed ERROR for the incompatible
    /// cells. The promotion precedence is: Float and Complex are mutually exclusive (an exact
    /// Complex refuses a Float); otherwise Complex > Rational > Decimal > Int, and Decimal ÷
    /// widens to Rational. Modular combines only with Modular of the same modulus.
    #[test]
    fn numeric_tower_cross_type_promotion_gauntlet() {
        let int = || RuntimeValue::Int(2);
        let rat = || RuntimeValue::from_rational(Rational::from_ratio_i64(1, 3).unwrap());
        let dec = || RuntimeValue::Decimal(Rc::new(Decimal::parse("0.5").unwrap()));
        let cpx = || RuntimeValue::Complex(Rc::new(Complex::new(Rational::from_i64(2), Rational::from_i64(3))));
        let flt = || RuntimeValue::Float(2.0);

        // ---- ADD: kind + exact value where the result is an exact type ----
        let g = |a: RuntimeValue, b: RuntimeValue| add(a, b);
        assert_eq!(kind(&g(int(), int()).unwrap()), "Int");
        assert_eq!(g(int(), rat()).unwrap().to_display_string(), "7/3"); // 2 + 1/3
        assert_eq!(kind(&g(int(), rat()).unwrap()), "Rational");
        assert_eq!(g(int(), dec()).unwrap().to_display_string(), "2.5"); // 2 + 0.5
        assert_eq!(kind(&g(int(), dec()).unwrap()), "Decimal");
        assert_eq!(g(int(), cpx()).unwrap().to_display_string(), "4+3i"); // 2 + (2+3i)
        assert_eq!(kind(&g(int(), cpx()).unwrap()), "Complex");
        assert_eq!(kind(&g(int(), flt()).unwrap()), "Float");
        assert_eq!(g(rat(), dec()).unwrap().to_display_string(), "5/6"); // 1/3 + 1/2
        assert_eq!(kind(&g(rat(), dec()).unwrap()), "Rational"); // Rational beats Decimal
        assert_eq!(g(rat(), cpx()).unwrap().to_display_string(), "7/3+3i"); // 1/3 + (2+3i)
        assert_eq!(kind(&g(rat(), cpx()).unwrap()), "Complex");
        assert_eq!(kind(&g(rat(), flt()).unwrap()), "Float");
        assert_eq!(g(dec(), dec()).unwrap().to_display_string(), "1.0"); // 0.5 + 0.5
        assert_eq!(kind(&g(dec(), dec()).unwrap()), "Decimal");
        assert_eq!(g(dec(), cpx()).unwrap().to_display_string(), "5/2+3i"); // 0.5 + (2+3i)
        assert_eq!(kind(&g(dec(), cpx()).unwrap()), "Complex");
        assert_eq!(kind(&g(dec(), flt()).unwrap()), "Float");
        assert_eq!(g(cpx(), cpx()).unwrap().to_display_string(), "4+6i");
        assert_eq!(kind(&g(flt(), flt()).unwrap()), "Float");
        // The one incompatible cell: an exact Complex refuses an inexact Float (both orders).
        assert!(g(cpx(), flt()).is_err(), "Complex + Float must be a typed error");
        assert!(g(flt(), cpx()).is_err(), "Float + Complex must be a typed error");

        // ---- Commutativity of the PROMOTION across the matrix (a∘b kind == b∘a kind) ----
        for a in [int(), rat(), dec(), cpx(), flt()] {
            for b in [int(), rat(), dec(), cpx(), flt()] {
                match (add(a.clone(), b.clone()), add(b.clone(), a.clone())) {
                    (Ok(ab), Ok(ba)) => assert_eq!(kind(&ab), kind(&ba), "promotion commutes: {} {}", kind(&a), kind(&b)),
                    (Err(_), Err(_)) => {} // both refuse (the Complex/Float cell)
                    other => panic!("promotion asymmetry for {} and {}: {other:?}", kind(&a), kind(&b)),
                }
            }
        }

        // ---- DIVIDE: Decimal widens to Rational; Complex stays Complex ----
        assert_eq!(divide(dec(), int()).unwrap().to_display_string(), "1/4"); // 0.5 / 2
        assert_eq!(kind(&divide(dec(), int()).unwrap()), "Rational");
        assert_eq!(divide(dec(), dec()).unwrap().to_display_string(), "1"); // 0.5 / 0.5 → 1 (Int)
        assert_eq!(divide(cpx(), int()).unwrap().to_display_string(), "1+3/2i"); // (2+3i)/2
        assert_eq!(kind(&divide(cpx(), int()).unwrap()), "Complex");
        assert_eq!(divide(cpx(), cpx()).unwrap().to_display_string(), "1"); // z/z
        assert_eq!(divide(rat(), int()).unwrap().to_display_string(), "1/6"); // (1/3)/2
        // Division by zero is a clean error across the exact types.
        assert!(divide(dec(), RuntimeValue::Int(0)).is_err());
        assert!(divide(cpx(), RuntimeValue::from_rational(Rational::zero())).is_err());
    }

    #[test]
    fn modular_arithmetic_wraps_and_requires_a_shared_ring() {
        let m = |v: i64, n: i64| {
            RuntimeValue::Modular(Rc::new(Modular::from_i64(v, n).unwrap()))
        };
        // Add/sub/mul wrap in ℤ/nℤ.
        assert_eq!(add(m(5, 7), m(4, 7)).unwrap(), m(2, 7)); // 9 ≡ 2
        assert_eq!(subtract(m(3, 7), m(5, 7)).unwrap(), m(5, 7)); // −2 ≡ 5
        assert_eq!(multiply(m(4, 7), m(5, 7)).unwrap(), m(6, 7)); // 20 ≡ 6
        // Division is by the modular inverse: 1/3 ≡ 5 (mod 7).
        assert_eq!(divide(m(1, 7), m(3, 7)).unwrap(), m(5, 7));
        // A non-invertible divisor (gcd ≠ 1) is a clean error, not a panic.
        assert!(divide(m(1, 4), m(2, 4)).is_err());
        // A modulus mismatch is refused on every op (no silent cross-ring math).
        assert!(add(m(3, 7), m(3, 5)).is_err());
        assert!(multiply(m(3, 7), m(3, 5)).is_err());
        assert!(divide(m(3, 7), m(3, 5)).is_err());
        // Mixing a Modular with a bare Int is refused (an Int has no modulus).
        assert!(add(m(3, 7), RuntimeValue::Int(5)).is_err());
        // Equality is per-ring; ℤ/nℤ is unordered so comparison is a typed error.
        assert!(matches!(binary_op(BinaryOpKind::Eq, m(3, 7), m(10, 7)).unwrap(), RuntimeValue::Bool(true)));
        assert!(matches!(binary_op(BinaryOpKind::Eq, m(3, 7), m(3, 5)).unwrap(), RuntimeValue::Bool(false)));
        assert!(compare(BinaryOpKind::Lt, &m(1, 7), &m(2, 7)).is_err());
        assert_eq!(m(3, 7).to_display_string(), "3 (mod 7)");
    }

    #[test]
    fn int_arithmetic_is_exact_in_every_build_profile() {
        // The LOGOS Int spec: EXACT (promoting) i64 — identical in debug AND release.
        let r = add(RuntimeValue::Int(i64::MAX), RuntimeValue::Int(1)).unwrap();
        assert_eq!(r.to_display_string(), "9223372036854775808"); // not wrapped i64::MIN
        let r = subtract(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(1)).unwrap();
        assert_eq!(r.to_display_string(), "-9223372036854775809");
        let r = multiply(RuntimeValue::Int(i64::MAX), RuntimeValue::Int(2)).unwrap();
        assert_eq!(r.to_display_string(), "18446744073709551614");
        // MIN / -1 = 2^63 (the division-overflow edge) promotes exactly; MIN % -1 = 0.
        let r = divide(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(-1)).unwrap();
        assert_eq!(r.to_display_string(), "9223372036854775808");
        let r = modulo(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(-1)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(0)));
        // Duration is a temporal quantity, NOT part of the integer tower — it still
        // wraps (its arithmetic is modular by construction).
        let r = add(RuntimeValue::Duration(i64::MAX), RuntimeValue::Duration(1)).unwrap();
        assert!(matches!(r, RuntimeValue::Duration(i64::MIN)));
        let r = subtract(RuntimeValue::Duration(i64::MIN), RuntimeValue::Duration(1)).unwrap();
        assert!(matches!(r, RuntimeValue::Duration(i64::MAX)));
    }

    #[test]
    fn shifts_mask_their_count_modulo_64() {
        // wrapping_shl/shr(b as u32): the count is truncated to u32 then masked
        // mod 64, so `1 << 64 == 1` and a negative count becomes (b as u32) & 63.
        let r = binary_op(BinaryOpKind::Shl, RuntimeValue::Int(1), RuntimeValue::Int(64)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(1)));
        let r = binary_op(BinaryOpKind::Shl, RuntimeValue::Int(1), RuntimeValue::Int(63)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
        // -1 as u32 == u32::MAX; masked mod 64 → 63.
        let r = binary_op(BinaryOpKind::Shl, RuntimeValue::Int(1), RuntimeValue::Int(-1)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
        let r = binary_op(BinaryOpKind::Shr, RuntimeValue::Int(i64::MIN), RuntimeValue::Int(63)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(-1)));
        let r = binary_op(BinaryOpKind::Shr, RuntimeValue::Int(8), RuntimeValue::Int(64)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(8)));
    }

    #[test]
    fn crdt_counter_bump_wraps() {
        let r = crdt_counter_bump(RuntimeValue::Int(i64::MAX), 1, "n").unwrap();
        assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
        let r = crdt_counter_bump(RuntimeValue::Nothing, 5, "n").unwrap();
        assert!(matches!(r, RuntimeValue::Int(5)));
        let e = crdt_counter_bump(RuntimeValue::Bool(true), 1, "score").unwrap_err();
        assert_eq!(e, "Field 'score' is not a counter");
    }

    #[test]
    fn eager_and_or_are_logical_truthiness_to_bool() {
        // `and`/`or` are LOGICAL: truthiness in, Bool out — `&`/`|` are the
        // bitwise spellings (BitAnd/BitOr below).
        let r = binary_op(BinaryOpKind::And, RuntimeValue::Int(6), RuntimeValue::Int(3)).unwrap();
        assert!(matches!(r, RuntimeValue::Bool(true)));
        let r = binary_op(BinaryOpKind::Or, RuntimeValue::Int(0), RuntimeValue::Int(7)).unwrap();
        assert!(matches!(r, RuntimeValue::Bool(true)));
        let r = binary_op(BinaryOpKind::And, RuntimeValue::Int(1), RuntimeValue::Bool(false)).unwrap();
        assert!(matches!(r, RuntimeValue::Bool(false)));
        let r = binary_op(BinaryOpKind::Or, RuntimeValue::Bool(false), RuntimeValue::Bool(true)).unwrap();
        assert!(matches!(r, RuntimeValue::Bool(true)));
        let r = binary_op(BinaryOpKind::BitAnd, RuntimeValue::Int(6), RuntimeValue::Int(3)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(2)));
        let r = binary_op(BinaryOpKind::BitOr, RuntimeValue::Int(6), RuntimeValue::Int(3)).unwrap();
        assert!(matches!(r, RuntimeValue::Int(7)));
    }
}

/// Exhaustive coverage of integer arithmetic at and beyond the i64 boundary: math is
/// EXACT on the tree-walker (overflow promotes to BigInt; in-range results downsize
/// to Int), every operand mix (Int/BigInt/Float) is covered, and a dense differential
/// against i128 proves we equal the machine's exact answer wherever it fits.
#[cfg(test)]
mod bigint_exact_arithmetic {
    use super::*;
    use logicaffeine_base::BigInt;

    fn int(n: i64) -> RuntimeValue {
        RuntimeValue::Int(n)
    }
    /// `i64::MAX + 1` = 2^63, the smallest value that does not fit i64 — our canonical
    /// "just past the boundary" BigInt.
    fn two_pow_63() -> RuntimeValue {
        add(int(i64::MAX), int(1)).unwrap()
    }
    fn disp(v: &RuntimeValue) -> String {
        v.to_display_string()
    }
    fn is_big(v: &RuntimeValue) -> bool {
        matches!(v, RuntimeValue::BigInt(_))
    }

    #[test]
    fn add_at_the_boundary_promotes_not_wraps() {
        // Arrange: the classic overflow.  Act:
        let r = add(int(i64::MAX), int(1)).unwrap();
        // Assert: the EXACT value, never the wrapped i64::MIN.
        assert!(is_big(&r), "i64::MAX + 1 must be a BigInt, not a wrapped Int");
        assert_eq!(disp(&r), "9223372036854775808");
        assert_ne!(r, int(i64::MIN), "the JSON/2's-complement footgun must be gone");
    }

    #[test]
    fn subtract_below_the_boundary_promotes() {
        let r = subtract(int(i64::MIN), int(1)).unwrap();
        assert!(is_big(&r));
        assert_eq!(disp(&r), "-9223372036854775809");
    }

    #[test]
    fn multiply_overflow_promotes() {
        let r = multiply(int(i64::MAX), int(2)).unwrap();
        assert_eq!(disp(&r), "18446744073709551614");
        assert!(is_big(&r));
    }

    #[test]
    fn results_that_fit_downsize_back_to_int() {
        // Arrange a BigInt, then bring it back into range.
        let big = two_pow_63(); // 2^63
        // 2^63 - 1 == i64::MAX, which fits → must be a narrow Int again.
        let back = subtract(big.clone(), int(1)).unwrap();
        assert_eq!(back, int(i64::MAX), "must downsize to Int");
        assert!(!is_big(&back));
        // big + (-big) == 0 → Int(0).
        let zero = add(big.clone(), subtract(int(0), big).unwrap()).unwrap();
        assert_eq!(zero, int(0));
    }

    #[test]
    fn every_operand_mix_is_handled_for_add_sub_mul() {
        let big = two_pow_63(); // 2^63 = 9223372036854775808
        // Int ∘ BigInt, BigInt ∘ Int, BigInt ∘ BigInt — add.
        assert_eq!(disp(&add(int(1), big.clone()).unwrap()), "9223372036854775809");
        assert_eq!(disp(&add(big.clone(), int(1)).unwrap()), "9223372036854775809");
        assert_eq!(disp(&add(big.clone(), big.clone()).unwrap()), "18446744073709551616"); // 2^64
        // subtract.
        assert_eq!(disp(&subtract(big.clone(), big.clone()).unwrap()), "0");
        // multiply (2^63 * 2 = 2^64).
        assert_eq!(disp(&multiply(big.clone(), int(2)).unwrap()), "18446744073709551616");
        assert_eq!(disp(&multiply(int(2), big.clone()).unwrap()), "18446744073709551616");
    }

    #[test]
    fn divide_and_modulo_cover_the_overflow_and_big_operands() {
        // i64::MIN / -1 = 2^63 (the one i64 division overflow) → promotes.
        let q = divide(int(i64::MIN), int(-1)).unwrap();
        assert_eq!(disp(&q), "9223372036854775808");
        assert_eq!(modulo(int(i64::MIN), int(-1)).unwrap(), int(0));
        // BigInt / Int, BigInt % Int.
        let big = two_pow_63(); // 9223372036854775808
        assert_eq!(divide(big.clone(), int(2)).unwrap(), int(4611686018427387904));
        assert_eq!(modulo(big.clone(), int(2)).unwrap(), int(0));
        // Int / BigInt: a small number over a huge one truncates to 0.
        assert_eq!(divide(int(5), big.clone()).unwrap(), int(0));
        assert_eq!(modulo(int(5), big).unwrap(), int(5));
        // Division by zero is still an error, never a panic.
        assert!(divide(int(1), int(0)).is_err());
        assert!(modulo(int(1), int(0)).is_err());
    }

    #[test]
    fn mixing_a_bigint_with_a_float_yields_a_float() {
        let big = two_pow_63();
        match add(big.clone(), RuntimeValue::Float(1.0)).unwrap() {
            RuntimeValue::Float(f) => assert!((f - 9223372036854775809.0).abs() < 1e9),
            other => panic!("expected Float, got {}", other.type_name()),
        }
        assert!(matches!(multiply(RuntimeValue::Float(2.0), big).unwrap(), RuntimeValue::Float(_)));
    }

    #[test]
    fn comparison_orders_across_the_narrow_wide_boundary() {
        let big = two_pow_63(); // > every i64
        let neg_big = subtract(int(i64::MIN), int(1)).unwrap(); // < every i64
        let lt = |a: &RuntimeValue, b: &RuntimeValue| {
            matches!(
                super::super::compare::compare(BinaryOpKind::Lt, a, b).unwrap(),
                RuntimeValue::Bool(true)
            )
        };
        assert!(lt(&int(i64::MAX), &big), "i64::MAX < 2^63");
        assert!(lt(&neg_big, &int(i64::MIN)), "-(2^63+1) < i64::MIN");
        assert!(lt(&neg_big, &big), "huge negative < huge positive");
        // equal BigInts are not less-than each other.
        assert!(!lt(&big.clone(), &big));
    }

    #[test]
    fn equality_and_hashing_are_consistent_for_bigints() {
        use std::collections::HashSet;
        let a = two_pow_63();
        let b = add(int(1), int(i64::MAX)).unwrap(); // same value, different path
        assert_eq!(a, b, "equal BigInts compare equal");
        assert_ne!(a, int(0), "a BigInt is never equal to an Int");
        // Hash agreement: a HashSet dedups two equal BigInts to one entry.
        let mut set = HashSet::new();
        set.insert(a);
        set.insert(b);
        assert_eq!(set.len(), 1, "equal BigInts must hash-collapse");
    }

    #[test]
    fn dense_differential_against_i128_through_the_arith_layer() {
        // For every pair whose true result fits i128, our Int/BigInt arithmetic — with
        // promotion AND downsizing — must equal the machine's exact answer.
        let xs: [i64; 9] = [0, 1, -1, 7, -7, i32::MAX as i64, i32::MIN as i64, i64::MAX, i64::MIN];
        for &x in &xs {
            for &y in &xs {
                assert_eq!(disp(&add(int(x), int(y)).unwrap()), (x as i128 + y as i128).to_string(), "{x}+{y}");
                assert_eq!(disp(&subtract(int(x), int(y)).unwrap()), (x as i128 - y as i128).to_string(), "{x}-{y}");
                assert_eq!(disp(&multiply(int(x), int(y)).unwrap()), (x as i128 * y as i128).to_string(), "{x}*{y}");
                if y != 0 {
                    assert_eq!(disp(&divide(int(x), int(y)).unwrap()), (x as i128 / y as i128).to_string(), "{x}/{y}");
                    assert_eq!(disp(&modulo(int(x), int(y)).unwrap()), (x as i128 % y as i128).to_string(), "{x}%{y}");
                }
            }
        }
    }

    #[test]
    fn promoted_values_round_trip_through_a_negation_chain() {
        // A stress walk: repeatedly add 1 starting from i64::MAX-1, crossing the
        // boundary, then subtract back down — every value exact, Int↔BigInt seamless.
        let mut v = int(i64::MAX - 1);
        for _ in 0..4 {
            v = add(v, int(1)).unwrap();
        }
        assert_eq!(disp(&v), "9223372036854775810"); // i64::MAX-1 + 4 = 2^63 + 1
        assert!(is_big(&v));
        for _ in 0..4 {
            v = subtract(v, int(1)).unwrap();
        }
        assert_eq!(v, int(i64::MAX - 1), "back to the exact narrow value");
        assert!(!is_big(&v));
    }

    #[test]
    fn fuzz_promotion_layer_matches_i128_and_obeys_algebra() {
        // Deterministic SplitMix64 — reproducible, no external dependency.
        let mut state = 0x5DEE_CE66_1357_2468u64;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for _ in 0..4000 {
            let (x, y) = (next() as i64, next() as i64);
            // Differential against i128 THROUGH the promote/downsize layer.
            assert_eq!(disp(&add(int(x), int(y)).unwrap()), (x as i128 + y as i128).to_string(), "{x}+{y}");
            assert_eq!(disp(&subtract(int(x), int(y)).unwrap()), (x as i128 - y as i128).to_string(), "{x}-{y}");
            assert_eq!(disp(&multiply(int(x), int(y)).unwrap()), (x as i128 * y as i128).to_string(), "{x}*{y}");
            if y != 0 {
                assert_eq!(disp(&divide(int(x), int(y)).unwrap()), (x as i128 / y as i128).to_string(), "{x}/{y}");
                assert_eq!(disp(&modulo(int(x), int(y)).unwrap()), (x as i128 % y as i128).to_string(), "{x}%{y}");
            }
            // Commutativity survives promotion.
            assert_eq!(add(int(x), int(y)).unwrap(), add(int(y), int(x)).unwrap());
            assert_eq!(multiply(int(x), int(y)).unwrap(), multiply(int(y), int(x)).unwrap());
            // The downsizing invariant: a sum is a BigInt IFF it does not fit i64.
            let exact = x as i128 + y as i128;
            let fits = (i64::MIN as i128..=i64::MAX as i128).contains(&exact);
            assert_eq!(is_big(&add(int(x), int(y)).unwrap()), !fits, "BigInt iff out of i64 range for {x}+{y}");
        }
    }

    #[test]
    fn from_bigint_constructor_maintains_the_downsizing_invariant() {
        // A BigInt that fits i64 must NOT be wrapped in the BigInt variant.
        assert_eq!(RuntimeValue::from_bigint(BigInt::from_i64(42)), int(42));
        assert!(!is_big(&RuntimeValue::from_bigint(BigInt::from_i64(i64::MIN))));
        // One that does not fit stays BigInt.
        assert!(is_big(&RuntimeValue::from_bigint(BigInt::from_i64(i64::MAX).add(&BigInt::from_i64(1)))));
    }
}

/// Phase R1: `RuntimeValue::Rational` as a first-class exact value. Arithmetic on
/// Rational operands is exact (an integer-valued result downsizes back to `Int`),
/// a Float operand makes the expression Float, and eq/compare/display are exact.
/// (The `Int / Int → Rational` flip itself is Phase R2 — here `/` still truncates,
/// so these tests construct Rationals directly via `from_rational`.)
#[cfg(test)]
mod rational_exact_arithmetic {
    use super::*;
    use crate::interpreter::StructValue;
    use logicaffeine_base::Rational;
    use std::collections::HashMap;

    fn rat(n: i64, d: i64) -> RuntimeValue {
        RuntimeValue::from_rational(Rational::from_ratio_i64(n, d).unwrap())
    }
    fn int(n: i64) -> RuntimeValue {
        RuntimeValue::Int(n)
    }
    fn is_rat(v: &RuntimeValue) -> bool {
        matches!(v, RuntimeValue::Rational(_))
    }

    #[test]
    fn from_rational_downsizes_whole_values_to_int() {
        // 6/2 reduces to the whole number 3 → Int, NOT a Rational.
        assert_eq!(rat(6, 2), int(3));
        assert!(!is_rat(&rat(6, 2)));
        assert_eq!(rat(0, 5), int(0));
        // 7/2 is not whole → stays a Rational, displayed as a fraction.
        assert!(is_rat(&rat(7, 2)));
        assert_eq!(rat(7, 2).to_display_string(), "7/2");
        assert_eq!(rat(7, 2).type_name(), "Rational");
    }

    #[test]
    fn rational_arithmetic_is_exact_and_downsizes() {
        // 1/2 + 1/2 = 1 (downsizes to Int).
        assert_eq!(add(rat(1, 2), rat(1, 2)).unwrap(), int(1));
        // 1/3 + 1/6 = 1/2 (stays exact).
        assert_eq!(add(rat(1, 3), rat(1, 6)).unwrap().to_display_string(), "1/2");
        // 1/2 - 1/3 = 1/6 ; 2/3 * 3/4 = 1/2 ; (1/2)/(3/4) = 2/3.
        assert_eq!(subtract(rat(1, 2), rat(1, 3)).unwrap().to_display_string(), "1/6");
        assert_eq!(multiply(rat(2, 3), rat(3, 4)).unwrap().to_display_string(), "1/2");
        assert_eq!(divide(rat(1, 2), rat(3, 4)).unwrap().to_display_string(), "2/3");
    }

    #[test]
    fn rational_mixes_with_int_and_bigint_exactly() {
        // 1/2 + 1 = 3/2 ; 3 * (1/2) = 3/2 ; (3/2) / 3 = 1/2.
        assert_eq!(add(rat(1, 2), int(1)).unwrap().to_display_string(), "3/2");
        assert_eq!(multiply(int(3), rat(1, 2)).unwrap().to_display_string(), "3/2");
        assert_eq!(divide(rat(3, 2), int(3)).unwrap(), rat(1, 2));
        // 1/3 + 2/3 = 1 (Int).
        assert_eq!(add(rat(1, 3), rat(2, 3)).unwrap(), int(1));
    }

    #[test]
    fn rational_with_a_float_operand_becomes_float() {
        // 1/2 + 0.5 → Float 1.0 (a Float operand opts the whole expression into floats).
        assert!(matches!(add(rat(1, 2), RuntimeValue::Float(0.5)).unwrap(), RuntimeValue::Float(f) if (f - 1.0).abs() < 1e-12));
        assert!(matches!(divide(rat(1, 2), RuntimeValue::Float(2.0)).unwrap(), RuntimeValue::Float(f) if (f - 0.25).abs() < 1e-12));
    }

    #[test]
    fn rational_equality_and_ordering_are_exact() {
        // Never equal to an Int; equal to a structurally-equal Rational.
        assert!(!values_equal(&rat(7, 2), &int(3)));
        assert!(values_equal(&rat(7, 2), &rat(7, 2)));
        // 1/3 < 1/2 < 2/3 by exact cross-multiplication (no rounding).
        let lt = |a, b| matches!(compare(BinaryOpKind::Lt, &a, &b).unwrap(), RuntimeValue::Bool(true));
        assert!(lt(rat(1, 3), rat(1, 2)));
        assert!(lt(rat(1, 2), rat(2, 3)));
        // 1/2 < 1 (Int) and 0 < 1/2.
        assert!(lt(rat(1, 2), int(1)));
        assert!(lt(int(0), rat(1, 2)));
    }

    #[test]
    fn dividing_a_rational_by_zero_errors() {
        assert_eq!(divide(rat(1, 2), int(0)).unwrap_err(), "Division by zero");
        assert_eq!(divide(rat(1, 2), rat(0, 1)).unwrap_err(), "Division by zero");
    }

    // ===== G7: the CRDT laws, fuzzed to absurdity =====
    // A state-based CRDT (CvRDT) converges iff its merge is COMMUTATIVE, ASSOCIATIVE, and
    // IDEMPOTENT. We prove all three for set-union and the map-of-sets join over thousands of
    // random states, then prove the corollary that matters on a real network: replicas reach
    // the same value no matter the ORDER or DUPLICATION of merges (a gossip/lossy link).

    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed)
        }
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn upto(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    /// Structural CRDT equality: sets compared AS SETS (order-independent), maps AS MAPS,
    /// scalars directly — the right notion of "converged to the same state".
    fn crdt_eq(a: &RuntimeValue, b: &RuntimeValue) -> bool {
        match (a, b) {
            (RuntimeValue::Int(x), RuntimeValue::Int(y)) => x == y,
            (RuntimeValue::Text(x), RuntimeValue::Text(y)) => x == y,
            (RuntimeValue::Bool(x), RuntimeValue::Bool(y)) => x == y,
            (RuntimeValue::Nothing, RuntimeValue::Nothing) => true,
            (RuntimeValue::Set(x), RuntimeValue::Set(y)) => {
                let (xb, yb) = (x.borrow(), y.borrow());
                xb.len() == yb.len() && xb.iter().all(|e| yb.iter().any(|f| crdt_eq(e, f)))
            }
            (RuntimeValue::Map(x), RuntimeValue::Map(y)) => {
                let (xb, yb) = (x.borrow(), y.borrow());
                xb.len() == yb.len() && xb.iter().all(|(k, v)| yb.get(k).map_or(false, |w| crdt_eq(v, w)))
            }
            (RuntimeValue::Struct(x), RuntimeValue::Struct(y)) => {
                x.type_name == y.type_name
                    && x.fields.len() == y.fields.len()
                    && x.fields.iter().all(|(k, v)| y.fields.get(k).map_or(false, |w| crdt_eq(v, w)))
            }
            _ => false,
        }
    }

    fn rand_gcounter(rng: &mut Rng) -> RuntimeValue {
        let mut fields: HashMap<String, RuntimeValue> = HashMap::new();
        for _ in 0..rng.upto(4) {
            let r = format!("r{}", rng.upto(4));
            let c = rng.upto(20) as i64;
            let cur = if let Some(RuntimeValue::Int(x)) = fields.get(&r) { *x } else { 0 };
            fields.insert(r, RuntimeValue::Int(cur.max(c)));
        }
        RuntimeValue::Struct(Box::new(StructValue { type_name: GCOUNTER_TAG.to_string(), fields }))
    }

    fn gcounter(pairs: &[(&str, i64)]) -> RuntimeValue {
        let mut fields = HashMap::new();
        for (r, c) in pairs {
            fields.insert(r.to_string(), RuntimeValue::Int(*c));
        }
        RuntimeValue::Struct(Box::new(StructValue { type_name: GCOUNTER_TAG.to_string(), fields }))
    }

    #[test]
    fn gcounter_is_gossip_safe_max_per_replica_and_idempotent_under_redelivery() {
        // The fix for the #1 gap: per-replica MAX, not op-based add. A REDELIVERED counter
        // state must not double-count — the crux the old `add` counter fails on a lossy link.
        let a = gcounter(&[("alice", 3), ("bob", 1)]);
        let b = gcounter(&[("alice", 2), ("carol", 5)]);
        let ab = crdt_merge_field(&a, b.clone());
        assert_eq!(gcounter_value(&ab), Some(3 + 1 + 5), "MAX per replica → alice=3, bob=1, carol=5");
        let abb = crdt_merge_field(&ab, b.clone());
        assert_eq!(gcounter_value(&abb), Some(9), "REDELIVERY is a no-op — never double-counts");
        assert!(crdt_eq(&ab, &abb), "the state is unchanged under redelivery (idempotent)");
    }

    #[test]
    fn gcounter_merge_obeys_the_crdt_laws() {
        assert_crdt_laws(&mut Rng::new(0x06C0_0117), rand_gcounter, "g-counter");
    }

    #[test]
    fn gcounter_replicas_converge_under_any_order_and_duplication() {
        // Replicas increment independently, gossip in shuffled+duplicated order, and every
        // one ends at the same total — a genuinely gossip-safe distributed counter.
        let mut rng = Rng::new(0x06C0_F1A6);
        for _ in 0..500 {
            let states: Vec<RuntimeValue> = (0..4).map(|_| rand_gcounter(&mut rng)).collect();
            let mut truth = states[0].clone();
            for s in &states[1..] {
                truth = crdt_merge_field(&truth, s.clone());
            }
            let mut deliveries: Vec<usize> = (0..states.len()).chain(0..states.len()).collect();
            for i in (1..deliveries.len()).rev() {
                let j = rng.upto((i + 1) as u64) as usize;
                deliveries.swap(i, j);
            }
            let mut replica = states[rng.upto(states.len() as u64) as usize].clone();
            for &d in &deliveries {
                replica = crdt_merge_field(&replica, states[d].clone());
            }
            assert!(crdt_eq(&replica, &truth), "g-counter replicas converge despite order/duplication");
            assert_eq!(gcounter_value(&replica), gcounter_value(&truth), "and agree on the total");
        }
    }

    fn set_of(items: &[i64]) -> RuntimeValue {
        RuntimeValue::Set(std::rc::Rc::new(std::cell::RefCell::new(
            items.iter().map(|&i| RuntimeValue::Int(i)).collect(),
        )))
    }

    fn rand_set(rng: &mut Rng) -> RuntimeValue {
        let mut s = set_of(&[]);
        for _ in 0..rng.upto(6) {
            s = crate::semantics::collections::union(&s, &set_of(&[rng.upto(8) as i64])).unwrap();
        }
        s
    }

    fn rand_map_of_sets(rng: &mut Rng) -> RuntimeValue {
        let mut m = crate::interpreter::MapStorage::default();
        for _ in 0..rng.upto(5) {
            m.insert(RuntimeValue::Int(rng.upto(5) as i64), rand_set(rng));
        }
        RuntimeValue::Map(std::rc::Rc::new(std::cell::RefCell::new(m)))
    }

    fn assert_crdt_laws(rng: &mut Rng, gen: impl Fn(&mut Rng) -> RuntimeValue, what: &str) {
        for _ in 0..2000 {
            let (a, b, c) = (gen(rng), gen(rng), gen(rng));
            assert!(
                crdt_eq(&crdt_merge_field(&a, b.clone()), &crdt_merge_field(&b, a.clone())),
                "{what} merge must be COMMUTATIVE"
            );
            assert!(crdt_eq(&crdt_merge_field(&a, a.clone()), &a), "{what} merge must be IDEMPOTENT");
            let ab = crdt_merge_field(&a, b.clone());
            assert!(crdt_eq(&crdt_merge_field(&ab, b.clone()), &ab), "{what}: re-merging a seen value is a no-op");
            let l = crdt_merge_field(&crdt_merge_field(&a, b.clone()), c.clone());
            let r = crdt_merge_field(&a, crdt_merge_field(&b, c.clone()));
            assert!(crdt_eq(&l, &r), "{what} merge must be ASSOCIATIVE");
        }
    }

    #[test]
    fn crdt_set_merge_obeys_the_crdt_laws() {
        assert_crdt_laws(&mut Rng::new(0x00C0_FFEE), rand_set, "set");
    }

    #[test]
    fn crdt_map_of_sets_merge_obeys_the_crdt_laws() {
        // A CRDT map of CRDT sets — "shared memory" over the network — is itself a CRDT.
        assert_crdt_laws(&mut Rng::new(0x0000_BEEF), rand_map_of_sets, "map-of-sets");
    }

    #[test]
    fn crdt_replicas_converge_under_any_order_and_duplication() {
        // The property that matters on a real gossip/lossy network: a replica that receives
        // every state in a SHUFFLED order, some delivered TWICE, still ends at the exact LUB.
        let mut rng = Rng::new(0x0000_5EED);
        for _ in 0..500 {
            let states: Vec<RuntimeValue> = (0..4).map(|_| rand_map_of_sets(&mut rng)).collect();
            let mut truth = states[0].clone();
            for s in &states[1..] {
                truth = crdt_merge_field(&truth, s.clone());
            }
            // Deliver each state once, then a duplicate of each, in a shuffled order.
            let mut deliveries: Vec<usize> = (0..states.len()).chain(0..states.len()).collect();
            for i in (1..deliveries.len()).rev() {
                let j = rng.upto((i + 1) as u64) as usize;
                deliveries.swap(i, j);
            }
            let mut replica = states[rng.upto(states.len() as u64) as usize].clone();
            for &d in &deliveries {
                replica = crdt_merge_field(&replica, states[d].clone());
            }
            assert!(crdt_eq(&replica, &truth), "every replica must converge to the LUB despite order/duplication");
        }
    }

    #[test]
    fn crdt_map_round_trips_and_merges_through_the_wire() {
        // The CRDT map survives the JSON relay wire (the tagged `__map` form) and the wire
        // merge equals the in-memory merge — sync-over-network for shared memory.
        let mut rng = Rng::new(0x0057_17E0);
        for _ in 0..400 {
            let local = rand_map_of_sets(&mut rng);
            let incoming = rand_map_of_sets(&mut rng);
            let expected = crdt_merge_field(&local, incoming.clone());
            let bytes = crdt_to_wire(&incoming).expect("a map publishes to the wire");
            let merged = crdt_merge_wire(local.clone(), &bytes);
            assert!(crdt_eq(&merged, &expected), "wire merge must equal the in-memory merge");
        }
    }
}

#[cfg(test)]
mod word_tests {
    use super::*;
    use logicaffeine_base::{Word32, Word64, WordVal};

    fn w32(n: u32) -> RuntimeValue {
        RuntimeValue::Word(WordVal::W32(Word32(n)))
    }
    fn w64(n: u64) -> RuntimeValue {
        RuntimeValue::Word(WordVal::W64(Word64(n)))
    }

    #[test]
    fn word_arithmetic_wraps_through_binary_op() {
        // The ring ℤ/2³²: MAX + 1 wraps to 0 and never promotes to BigInt.
        assert_eq!(binary_op(BinaryOpKind::Add, w32(0xFFFF_FFFF), w32(1)).unwrap(), w32(0));
        assert_eq!(binary_op(BinaryOpKind::Subtract, w32(0), w32(1)).unwrap(), w32(0xFFFF_FFFF));
        assert_eq!(binary_op(BinaryOpKind::Multiply, w32(0x1000_0000), w32(0x10)).unwrap(), w32(0));
        assert_eq!(binary_op(BinaryOpKind::BitXor, w32(0xFF00), w32(0x0FF0)).unwrap(), w32(0xF0F0));
        assert_eq!(binary_op(BinaryOpKind::And, w32(0xFF00), w32(0x0FF0)).unwrap(), w32(0x0F00));
        assert_eq!(binary_op(BinaryOpKind::Or, w32(0xFF00), w32(0x0FF0)).unwrap(), w32(0xFFF0));
    }

    #[test]
    fn word_shift_takes_an_integer_count() {
        assert_eq!(binary_op(BinaryOpKind::Shl, w32(1), RuntimeValue::Int(8)).unwrap(), w32(0x100));
        assert_eq!(binary_op(BinaryOpKind::Shr, w32(0x100), RuntimeValue::Int(8)).unwrap(), w32(1));
    }

    #[test]
    fn word_equality_and_width_mismatch_is_typed() {
        assert!(values_equal(&w32(5), &w32(5)));
        assert!(!values_equal(&w32(5), &w64(5)), "different widths are never equal");
        assert!(
            binary_op(BinaryOpKind::Add, w32(1), w64(1)).is_err(),
            "Word32 + Word64 must error, not silently coerce"
        );
    }
}
