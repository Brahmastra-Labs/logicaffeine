//! Equality and relational comparison.

use logicaffeine_base::numeric;
use logicaffeine_base::{BigInt, Rational};

use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::RuntimeValue;

/// Value equality for the `equals`/`==` operator, set/list membership, and
/// map keys (`RuntimeValue`'s `PartialEq` delegates here — ONE equality).
///
/// - Floats compare by IEEE `==` (`NaN != NaN`, `-0.0 == 0.0`) — identical to
///   what compiled Rust emits, on every engine.
/// - Cross-type numeric equality is EXACT (mathematical value, never a lossy
///   cast): `1 == 1.0`, but `9007199254740993 != 9007199254740993.0` because
///   that float literal IS `2^53`. Coheres with `compare` and with the
///   unified numeric hash (`base::numeric`).
/// - Collections, tuples, and structs compare STRUCTURALLY (same shape, all
///   parts equal), with an `Rc` identity fast path and a depth cap against
///   cyclic values.
/// - Decimal/Complex/Modular keep their documented within-type equality
///   (they have no cross-type ordering, so there is no coherence to break).
/// `left = right` (or `≠` when `negate`) as a program asks it: decided exactly where a theorem
/// decides it, residue-dependent where the answer depends on `⌈1/dt⌉`, and a loud error where two
/// computable reals agree to the budget and nothing decides them. Every other value compares by
/// [`values_equal`].
pub fn equality(left: &RuntimeValue, right: &RuntimeValue, negate: bool) -> Result<RuntimeValue, String> {
    let rel = if negate { super::exact::Rel::NotEq } else { super::exact::Rel::Eq };
    if let Some(r) = super::exact::relation(rel, left, right, &exact_relation_recurse) {
        return r;
    }
    Ok(RuntimeValue::Bool(values_equal(left, right) != negate))
}

/// The relational entry the exact types re-enter to lift a comparison over residue branches.
fn exact_relation_recurse(
    rel: super::exact::Rel,
    a: RuntimeValue,
    b: RuntimeValue,
) -> Result<RuntimeValue, String> {
    use super::exact::Rel;
    match rel {
        Rel::Eq => equality(&a, &b, false),
        Rel::NotEq => equality(&a, &b, true),
        Rel::Lt => compare(BinaryOpKind::Lt, &a, &b),
        Rel::Gt => compare(BinaryOpKind::Gt, &a, &b),
        Rel::LtEq => compare(BinaryOpKind::LtEq, &a, &b),
        Rel::GtEq => compare(BinaryOpKind::GtEq, &a, &b),
    }
}

pub fn values_equal(left: &RuntimeValue, right: &RuntimeValue) -> bool {
    values_equal_depth(left, right, 0)
}

/// Cyclic values (a list pushed into itself) bottom out here instead of
/// overflowing the stack; 256 levels is far beyond any real data shape.
const EQ_MAX_DEPTH: usize = 256;

/// A CRDT counter (`ConvergentCount`/`Tally`) reads as its integer value everywhere a scalar is
/// expected — comparison and equality see the number, not the CRDT wrapper. Non-counter CRDTs
/// (sets/sequences/registers) return `None` and keep their own structural comparison.
fn counter_as_int(v: &RuntimeValue) -> Option<RuntimeValue> {
    match v {
        RuntimeValue::Crdt(c) => c.borrow().counter_value().map(RuntimeValue::Int),
        _ => None,
    }
}

fn values_equal_depth(left: &RuntimeValue, right: &RuntimeValue, depth: usize) -> bool {
    if depth > EQ_MAX_DEPTH {
        return false;
    }
    if let Some(same) = super::exact::structurally_equal(left, right) {
        return same;
    }
    let (lc, rc) = (counter_as_int(left), counter_as_int(right));
    let (left, right) = (lc.as_ref().unwrap_or(left), rc.as_ref().unwrap_or(right));
    match (left, right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => a == b,
        // BigInt holds only out-of-i64 values, so a BigInt never equals an Int; two
        // BigInts compare by exact magnitude (the `_ => false` arm would be wrong).
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => a == b,
        // A Rational is never whole, so it never equals an Int/BigInt; Rational==Rational
        // is exact (reduced form is canonical).
        (RuntimeValue::Rational(a), RuntimeValue::Rational(b)) => a == b,
        // Decimal equality is by VALUE (`1.0 == 1.00`); like Int vs Float, a Decimal is
        // never `==` a value of a different numeric type.
        (RuntimeValue::Decimal(a), RuntimeValue::Decimal(b)) => a == b,
        // Complex equality is exact and structural; complex numbers have no ordering, so
        // `compare` (`< > …`) deliberately falls through to a type error.
        (RuntimeValue::Complex(a), RuntimeValue::Complex(b)) => a == b,
        // Floating-point complex: IEEE equality on both parts (like `Float`; NaN parts unequal).
        // Also unordered, so `compare` falls through to a type error.
        (RuntimeValue::ComplexF(a), RuntimeValue::ComplexF(b)) => a == b,
        // Modular equality is per-ring (same residue AND modulus); ℤ/nℤ has no total order,
        // so `compare` falls through to a type error.
        (RuntimeValue::Modular(a), RuntimeValue::Modular(b)) => a == b,
        // IEEE float equality — what the compiled backend emits.
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => a == b,
        // Exact cross-type numeric equality (mathematical values).
        (RuntimeValue::Int(a), RuntimeValue::Float(b))
        | (RuntimeValue::Float(b), RuntimeValue::Int(a)) => {
            numeric::cmp_i64_f64_exact(*a, *b) == Some(std::cmp::Ordering::Equal)
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b))
        | (RuntimeValue::Float(b), RuntimeValue::BigInt(a)) => {
            numeric::cmp_bigint_f64_exact(a, *b) == Some(std::cmp::Ordering::Equal)
        }
        (RuntimeValue::Rational(a), RuntimeValue::Float(b))
        | (RuntimeValue::Float(b), RuntimeValue::Rational(a)) => {
            numeric::cmp_rational_f64_exact(a, *b) == Some(std::cmp::Ordering::Equal)
        }
        (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => a == b,
        (RuntimeValue::Text(a), RuntimeValue::Text(b)) => **a == **b,
        (RuntimeValue::Char(a), RuntimeValue::Char(b)) => a == b,
        (RuntimeValue::Nothing, RuntimeValue::Nothing) => true,
        // ★ R2 — ABSENCE IS ONE THING, however it was spelled.
        //
        // `nothing` and `none` are different AST nodes (`Literal::Nothing` vs `Expr::OptionNone`)
        // and, now that an Option carries a real tag, different runtime values. They must still
        // compare equal: `x is equal to nothing` is the spelling every `Try to receive`, `Pop` and
        // `storageGet` result is tested with, and the wasm tier compiles it to a handle test. A
        // present Option is never equal to `nothing` — that is the whole point of the change.
        (RuntimeValue::Nothing, RuntimeValue::Inductive(i))
        | (RuntimeValue::Inductive(i), RuntimeValue::Nothing) => {
            i.inductive_type == "Option" && i.constructor == "None"
        }
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => a == b,
        (RuntimeValue::Date(a), RuntimeValue::Date(b)) => a == b,
        (RuntimeValue::Moment(a), RuntimeValue::Moment(b)) => a == b,
        (
            RuntimeValue::Span {
                months: m1,
                days: d1,
            },
            RuntimeValue::Span {
                months: m2,
                days: d2,
            },
        ) => m1 == m2 && d1 == d2,
        (RuntimeValue::Time(a), RuntimeValue::Time(b)) => a == b,
        // Words are equal only at the same width and value (`WordVal`'s derived `Eq`).
        (RuntimeValue::Word(a), RuntimeValue::Word(b)) => a == b,
        // Money is value-equal by currency + amount; a Quantity by physical magnitude (display unit
        // ignored, so `2 inches == 5.08 cm`); a Uuid by its 128 bits.
        (RuntimeValue::Money(a), RuntimeValue::Money(b)) => a == b,
        (RuntimeValue::Quantity(a), RuntimeValue::Quantity(b)) => a.q == b.q,
        (RuntimeValue::Uuid(a), RuntimeValue::Uuid(b)) => a == b,
        (RuntimeValue::Inductive(a), RuntimeValue::Inductive(b)) => {
            a.inductive_type == b.inductive_type
                && a.constructor == b.constructor
                && a.args.len() == b.args.len()
                && a.args
                    .iter()
                    .zip(b.args.iter())
                    .all(|(x, y)| values_equal_depth(x, y, depth + 1))
        }
        // ── Structural equality ─────────────────────────────────────────
        (RuntimeValue::List(a), RuntimeValue::List(b)) => {
            if std::rc::Rc::ptr_eq(a, b) {
                return true;
            }
            let (a, b) = (a.borrow(), b.borrow());
            a.len() == b.len()
                && (0..a.len()).all(|i| match (a.get(i), b.get(i)) {
                    (Some(x), Some(y)) => values_equal_depth(&x, &y, depth + 1),
                    _ => false,
                })
        }
        (RuntimeValue::Tuple(a), RuntimeValue::Tuple(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| values_equal_depth(x, y, depth + 1))
        }
        // Sets compare by CONTENT, order-insensitive (both sides are deduped,
        // so equal length + one-sided containment is a bijection).
        (RuntimeValue::Set(a), RuntimeValue::Set(b)) => {
            if std::rc::Rc::ptr_eq(a, b) {
                return true;
            }
            let (a, b) = (a.borrow(), b.borrow());
            a.len() == b.len()
                && a.iter()
                    .all(|x| b.iter().any(|y| values_equal_depth(x, y, depth + 1)))
        }
        // Maps compare by CONTENT, insertion order ignored (like Python dicts).
        (RuntimeValue::Map(a), RuntimeValue::Map(b)) => {
            if std::rc::Rc::ptr_eq(a, b) {
                return true;
            }
            let (a, b) = (a.borrow(), b.borrow());
            a.len() == b.len()
                && a.iter().all(|(k, va)| {
                    b.get(k)
                        .is_some_and(|vb| values_equal_depth(va, vb, depth + 1))
                })
        }
        (RuntimeValue::Struct(a), RuntimeValue::Struct(b)) => {
            a.type_name == b.type_name
                && a.fields.len() == b.fields.len()
                && a.fields.iter().all(|(name, va)| {
                    b.fields
                        .get(name)
                        .is_some_and(|vb| values_equal_depth(va, vb, depth + 1))
                })
        }
        // ── Identity-style values (moved from the old `PartialEq` impl so
        //    this function is the TOTAL equality it delegates to) ─────────
        (RuntimeValue::Chan(a), RuntimeValue::Chan(b)) => a == b,
        (RuntimeValue::TaskHandle(a), RuntimeValue::TaskHandle(b)) => a == b,
        (RuntimeValue::Peer(a), RuntimeValue::Peer(b)) => **a == **b,
        (RuntimeValue::Function(a), RuntimeValue::Function(b)) => a.body_index == b.body_index,
        // Two CRDTs are equal when they observe the same elements (a sequence also
        // compares order) — the convergence-relevant view, ignoring internal tags.
        (RuntimeValue::Crdt(a), RuntimeValue::Crdt(b)) => {
            crate::semantics::crdt::crdt_values_equal(&a.borrow(), &b.borrow())
        }
        // Lane vectors compare by value (all lanes equal), consistent with Hash.
        (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) => a == b,
        _ => false,
    }
}

/// Relational comparison (`< > <= >=`).
///
/// Floats use IEEE 754 semantics — NaN is unordered (every relational
/// comparison with a NaN is `false`) and `-0.0 == 0.0` — matching Rust's `f64`
/// ordering, which is what the compile-to-Rust path emits. Integer and
/// temporal types use the natural total order; a Moment compares against a
/// Time by its time-of-day.
pub fn compare(
    op: BinaryOpKind,
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Result<RuntimeValue, String> {
    use std::cmp::Ordering;

    let rel = match op {
        BinaryOpKind::Lt => Some(super::exact::Rel::Lt),
        BinaryOpKind::Gt => Some(super::exact::Rel::Gt),
        BinaryOpKind::LtEq => Some(super::exact::Rel::LtEq),
        BinaryOpKind::GtEq => Some(super::exact::Rel::GtEq),
        _ => None,
    };
    if let Some(rel) = rel {
        if let Some(r) = super::exact::relation(rel, left, right, &exact_relation_recurse) {
            return r;
        }
    }

    // A counter compares by its integer value (the CRDT wrapper is transparent to `< > <= >=`).
    let (lc, rc) = (counter_as_int(left), counter_as_int(right));
    let (left, right) = (lc.as_ref().unwrap_or(left), rc.as_ref().unwrap_or(right));

    // Map an `Ordering` (or `None` for an unordered/NaN pair) to the operator.
    let rel = |ord: Option<Ordering>| -> bool {
        match ord {
            None => false,
            Some(o) => match op {
                BinaryOpKind::Lt => o == Ordering::Less,
                BinaryOpKind::Gt => o == Ordering::Greater,
                BinaryOpKind::LtEq => o != Ordering::Greater,
                BinaryOpKind::GtEq => o != Ordering::Less,
                _ => false,
            },
        }
    };
    let int_rel = |a: i64, b: i64| rel(Some(a.cmp(&b)));

    match (left, right) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Bool(int_rel(*a, *b))),
        // Exact integer ordering across the narrow/wide boundary: compare as BigInts.
        (RuntimeValue::BigInt(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::Bool(rel(Some((**a).cmp(b)))))
        }
        (RuntimeValue::BigInt(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Bool(rel(Some(
            (**a).cmp(&BigInt::from_i64(*b)),
        )))),
        (RuntimeValue::Int(a), RuntimeValue::BigInt(b)) => {
            Ok(RuntimeValue::Bool(rel(Some(BigInt::from_i64(*a).cmp(b)))))
        }
        // Cross-type numeric ordering is EXACT — mathematical values, never a
        // lossy as-f64 view (which rounds above 2^53). NaN stays unordered.
        (RuntimeValue::BigInt(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Bool(rel(
            numeric::cmp_bigint_f64_exact(a, *b),
        ))),
        (RuntimeValue::Float(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::Bool(rel(
            numeric::cmp_bigint_f64_exact(b, *a).map(std::cmp::Ordering::reverse),
        ))),
        // Exact rational ordering (cross-multiply, no rounding) including the
        // narrow/wide boundary; vs Float uses IEEE partial order on the f64 view.
        (RuntimeValue::Rational(a), RuntimeValue::Rational(b)) => {
            Ok(RuntimeValue::Bool(rel(Some((**a).cmp(b)))))
        }
        (RuntimeValue::Rational(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Bool(rel(Some(
            (**a).cmp(&Rational::from_i64(*b)),
        )))),
        (RuntimeValue::Int(a), RuntimeValue::Rational(b)) => {
            Ok(RuntimeValue::Bool(rel(Some(Rational::from_i64(*a).cmp(b)))))
        }
        (RuntimeValue::Rational(a), RuntimeValue::BigInt(b)) => Ok(RuntimeValue::Bool(rel(Some(
            (**a).cmp(&Rational::from_bigint((**b).clone())),
        )))),
        (RuntimeValue::BigInt(a), RuntimeValue::Rational(b)) => Ok(RuntimeValue::Bool(rel(Some(
            Rational::from_bigint((**a).clone()).cmp(b),
        )))),
        (RuntimeValue::Rational(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Bool(rel(
            numeric::cmp_rational_f64_exact(a, *b),
        ))),
        (RuntimeValue::Float(a), RuntimeValue::Rational(b)) => Ok(RuntimeValue::Bool(rel(
            numeric::cmp_rational_f64_exact(b, *a).map(std::cmp::Ordering::reverse),
        ))),
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Bool(rel(a.partial_cmp(b))))
        }
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => {
            Ok(RuntimeValue::Bool(rel(numeric::cmp_i64_f64_exact(*a, *b))))
        }
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Bool(rel(
            numeric::cmp_i64_f64_exact(*b, *a).map(std::cmp::Ordering::reverse),
        ))),
        (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => {
            Ok(RuntimeValue::Bool(int_rel(*a, *b)))
        }
        (RuntimeValue::Date(a), RuntimeValue::Date(b)) => {
            Ok(RuntimeValue::Bool(int_rel(*a as i64, *b as i64)))
        }
        (RuntimeValue::Moment(a), RuntimeValue::Moment(b)) => {
            Ok(RuntimeValue::Bool(int_rel(*a, *b)))
        }
        (RuntimeValue::Time(a), RuntimeValue::Time(b)) => Ok(RuntimeValue::Bool(int_rel(*a, *b))),
        // Moment vs Time: extract time-of-day from Moment. Use Euclidean
        // remainder so a pre-epoch (negative) Moment yields a 0..86399 ns
        // time-of-day, not a negative one.
        (RuntimeValue::Moment(m), RuntimeValue::Time(t)) => {
            let nanos_per_day = 86_400_000_000_000i64;
            Ok(RuntimeValue::Bool(int_rel(m.rem_euclid(nanos_per_day), *t)))
        }
        (RuntimeValue::Time(t), RuntimeValue::Moment(m)) => {
            let nanos_per_day = 86_400_000_000_000i64;
            Ok(RuntimeValue::Bool(int_rel(*t, m.rem_euclid(nanos_per_day))))
        }
        // Two physical quantities order by their EXACT SI magnitude when their dimensions match;
        // ordering across dimensions (Length vs Mass) is a typed error, mirroring the AOT `PartialOrd`
        // (cross-dimension → `None` → not orderable). `==`/`!=` go through value equality elsewhere.
        (RuntimeValue::Quantity(a), RuntimeValue::Quantity(b)) => {
            if a.q.dimension() != b.q.dimension() {
                return Err(logicaffeine_data::failure::quantities_uncomparable(
                    a.q.dimension(),
                    b.q.dimension(),
                ));
            }
            Ok(RuntimeValue::Bool(rel(Some(
                a.q.magnitude_si().cmp(b.q.magnitude_si()),
            ))))
        }
        // Money orders by amount within the SAME currency; ordering across currencies is meaningless
        // without a rate context, so it is a typed error (the dimension-mismatch precedent).
        (RuntimeValue::Money(a), RuntimeValue::Money(b)) => {
            if a.currency != b.currency {
                return Err(logicaffeine_data::failure::currencies_differ(
                    "compare",
                    a.currency.code,
                    b.currency.code,
                ));
            }
            Ok(RuntimeValue::Bool(rel(Some(
                a.amount.to_rational().cmp(&b.amount.to_rational()),
            ))))
        }
        // UUIDs order by their 128 bits — so v6/v7 (time-ordered) ids sort chronologically.
        (RuntimeValue::Uuid(a), RuntimeValue::Uuid(b)) => {
            Ok(RuntimeValue::Bool(rel(Some(a.cmp(b)))))
        }
        // Machine words order by UNSIGNED magnitude when their widths match — exactly the
        // AOT newtype's `u32`/`u64` `PartialOrd`. Mixing widths (Word32 vs Word64) is a typed
        // error, mirroring Word arithmetic (`Word32 + Word64` errors, never silently coerces).
        (RuntimeValue::Word(a), RuntimeValue::Word(b)) => {
            if a.width() != b.width() {
                return Err(format!(
                    "cannot compare Word{} and Word{} — width mismatch",
                    a.width(),
                    b.width()
                ));
            }
            Ok(RuntimeValue::Bool(rel(Some(a.to_u64().cmp(&b.to_u64())))))
        }
        // A Decimal orders by EXACT value against any exact number (Int/BigInt/Rational/
        // Decimal); against a Float it compares on the f64 view (IEEE partial order).
        (l, r)
            if matches!(l, RuntimeValue::Decimal(_)) || matches!(r, RuntimeValue::Decimal(_)) =>
        {
            let rat_view = |v: &RuntimeValue| -> Option<Rational> {
                match v {
                    RuntimeValue::Int(n) => Some(Rational::from_i64(*n)),
                    RuntimeValue::BigInt(b) => Some(Rational::from_bigint((**b).clone())),
                    RuntimeValue::Rational(r) => Some((**r).clone()),
                    RuntimeValue::Decimal(d) => Some(d.to_rational()),
                    _ => None,
                }
            };
            if let (Some(a), Some(b)) = (rat_view(l), rat_view(r)) {
                Ok(RuntimeValue::Bool(rel(Some(a.cmp(&b)))))
            } else {
                let f64_view = |v: &RuntimeValue| -> Option<f64> {
                    match v {
                        RuntimeValue::Float(f) => Some(*f),
                        other => rat_view(other).map(|x| x.to_f64()),
                    }
                };
                match (f64_view(l), f64_view(r)) {
                    (Some(a), Some(b)) => Ok(RuntimeValue::Bool(rel(a.partial_cmp(&b)))),
                    _ => Err(format!(
                        "Cannot compare {} and {}",
                        l.type_name(),
                        r.type_name()
                    )),
                }
            }
        }
        _ => Err(format!(
            "Cannot compare {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/compare/tests.rs"]
mod tests;
