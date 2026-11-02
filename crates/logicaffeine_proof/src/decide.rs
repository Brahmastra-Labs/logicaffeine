//! `decide` — proof by evaluation for closed decidable goals.
//!
//! The router below is untrusted: it evaluates the goal with exact integer
//! arithmetic and, only when the goal is TRUE, builds the derivation whose
//! leaves re-derive that truth through certified channels — `ArithDecision`
//! (the proof-producing arithmetic oracle) for ground `Int` identities,
//! `NativeDecide` (the kernel's trusted-evaluator route via the `reduceBool`
//! hook and a `Decidable` instance) for ground comparisons and Bool
//! equalities, and `GroundRefutation` (the `Bool` no-confusion discriminator,
//! after transport for `Int`) for a NEGATION whose subject computes false.
//! Propositional structure recurses through the ordinary intro rules. A false,
//! open, or unsupported goal yields `Verdict::Undecided` — decide DECLINES,
//! it never guesses — and a goal it evaluated and found FALSE says so, which
//! is a different answer and must not be conflated with the first.
//!
//! The evaluator is arbitrary-precision, through `crate::numeral`: both
//! kernels compute integers exactly, so the router's own window must not be
//! the system's arithmetic ceiling.

use logicaffeine_base::BigInt;

use crate::numeral::int_of_constant;
use crate::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

/// What `decide` found. The old single `Option` conflated "I cannot evaluate
/// this" with "I evaluated it and it is FALSE" — and a caller that treats the
/// second as the first keeps searching past a goal that is not true, until some
/// unrelated contradiction discharges it by ex falso. The distinction is the
/// capability, so it is in the type.
#[derive(Debug, Clone)]
pub(crate) enum Verdict {
    /// The goal evaluates TRUE and here is the certificate for it.
    True(DerivationTree),
    /// The goal was fully evaluated and is FALSE. Nothing to build, and nothing
    /// else will prove it either.
    False,
    /// Not a closed goal in the decidable fragment — no verdict either way.
    Undecided,
}

/// `div`/`mod` at arbitrary precision, in Lean's EUCLIDEAN convention, and TOTAL.
///
/// THE SPECIFICATION is the one written out in
/// `logicaffeine_kernel::reduction::ediv_emod_i64`'s doc comment, which every engine that
/// folds division on literals implements FROM THE TEXT:
///
/// ```text
///   b ≠ 0  ⟹  a = b·q + r   ∧   0 ≤ r < |b|      (the unique such pair)
///   b = 0  ⟹  q = 0  ∧  r = a
/// ```
///
/// This router is the FIFTH such engine — beside `reduction::try_primitive_reduce`, the second
/// kernel's `recheck::try_builtin`, `eval::prim_arith` and `simp::try_arithmetic` — and it is
/// written by its OWN derivation, never by copying one of the other four, so a slip in one
/// derivation cannot hide in another.
///
/// `BigInt::div_rem` is TRUNCATED (the quotient rounds toward zero and the remainder carries
/// the dividend's sign), so it is used only where the two conventions provably cannot differ:
/// on the MAGNITUDES, which are non-negative. The signs are then reconstructed from the
/// division identity itself. Writing `|a| = q₀·|b| + r₀` with `0 ≤ r₀ < |b|`:
///
/// * `a ≥ 0` — `r₀` is already in range, and `a = q₀·|b|`, so `q = q₀·sgn b`.
/// * `a < 0` and `r₀ = 0` — `a = -q₀·|b|` exactly, so `r = 0` and `q = -q₀·sgn b`.
/// * `a < 0` and `r₀ > 0` — negating `r₀` would leave the range, so take the COMPLEMENT
///   `r = |b| - r₀`, which lies in `(0, |b|)`; then
///   `a = -q₀|b| - r₀ = -(q₀+1)·|b| + (|b| - r₀)`, so `q = -(q₀+1)·sgn b`.
///
/// The truncating router this replaces answered `div(-7, 2) = -3` where all four kernel engines
/// answer `-4`, and got STUCK at a zero divisor where they answer `0` — so it called a true
/// goal [`Verdict::False`], whose documented meaning is "nothing else will prove it either",
/// and handed a false one to the trust door as an `ArithDecision` the arithmetic oracle then
/// refused, which is a tactic reporting success without a checkable object.
fn ediv_emod(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    if b.is_zero() {
        return (BigInt::zero(), a.clone());
    }
    let magnitude = b.abs();
    let (q0, r0) = a
        .abs()
        .div_rem(&magnitude)
        .expect("a non-zero divisor has a quotient");
    let (unsigned_q, remainder) = if !a.is_negative() {
        (q0, r0)
    } else if r0.is_zero() {
        (q0.negated(), r0)
    } else {
        (q0.add(&BigInt::from_i64(1)).negated(), magnitude.sub(&r0))
    };
    let quotient = if b.is_negative() {
        unsigned_q.negated()
    } else {
        unsigned_q
    };
    (quotient, remainder)
}

/// Evaluate a ground integer term: numeric constants and `add`/`sub`/`mul`/
/// `div`/`mod` trees over them, at ARBITRARY precision. Division is
/// [`ediv_emod`] — Euclidean and total, the answer both kernels give.
fn eval_int(t: &ProofTerm) -> Option<BigInt> {
    match t {
        ProofTerm::Constant(s) => int_of_constant(s),
        ProofTerm::Function(op, args) if args.len() == 2 => {
            let a = eval_int(&args[0])?;
            let b = eval_int(&args[1])?;
            match op.as_str() {
                "add" | "Add" => Some(a.add(&b)),
                "sub" | "Sub" => Some(a.sub(&b)),
                "mul" | "Mul" => Some(a.mul(&b)),
                "div" | "Div" => Some(ediv_emod(&a, &b).0),
                "mod" => Some(ediv_emod(&a, &b).1),
                _ => None,
            }
        }
        _ => None,
    }
}

fn ground_bool(t: &ProofTerm) -> Option<bool> {
    match t {
        ProofTerm::Constant(s) if s == "true" => Some(true),
        ProofTerm::Constant(s) if s == "false" => Some(false),
        _ => None,
    }
}

/// Evaluate a ground comparison term `le(a, b)`/`lt`/`ge`/`gt` to its truth.
fn eval_comparison(t: &ProofTerm) -> Option<bool> {
    let ProofTerm::Function(name, args) = t else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    let a = eval_int(&args[0])?;
    let b = eval_int(&args[1])?;
    match name.as_str() {
        "le" => Some(a <= b),
        "lt" => Some(a < b),
        "ge" => Some(a >= b),
        "gt" => Some(a > b),
        _ => None,
    }
}

/// The truth value of a ground `Identity`, when every operand evaluates —
/// `None` when the identity is not closed in the fragment. Returns the rule
/// that re-derives it too, because which certified channel applies is decided
/// by the same case split that decides the truth.
fn eval_identity(l: &ProofTerm, r: &ProofTerm) -> Option<(bool, InferenceRule)> {
    // Ground comparison in the canonical encoding `le(a, b) = true` (or
    // `= false`): the kernel evaluator re-derives it.
    if let (Some(truth), Some(claimed)) = (eval_comparison(l), ground_bool(r)) {
        return Some((truth == claimed, InferenceRule::NativeDecide));
    }
    // …and the mirrored encoding `true = le(a, b)`, which a translator writes
    // just as readily and which the old router declined outright.
    if let (Some(claimed), Some(truth)) = (ground_bool(l), eval_comparison(r)) {
        return Some((truth == claimed, InferenceRule::NativeDecide));
    }
    // Ground Int identity: the arithmetic oracle re-derives it.
    if let (Some(a), Some(b)) = (eval_int(l), eval_int(r)) {
        return Some((a == b, InferenceRule::ArithDecision));
    }
    // Ground Bool identity: the kernel evaluator re-derives it.
    if let (Some(a), Some(b)) = (ground_bool(l), ground_bool(r)) {
        return Some((a == b, InferenceRule::NativeDecide));
    }
    None
}

/// Decide a closed goal. See [`Verdict`] — `True` carries a derivation whose
/// leaves certify through `ArithDecision`/`NativeDecide`/`GroundRefutation` and
/// whose structure goes through the intro rules.
pub(crate) fn decide_verdict(goal: &ProofExpr) -> Verdict {
    match goal {
        ProofExpr::Identity(l, r) => match eval_identity(l, r) {
            Some((true, rule)) => Verdict::True(DerivationTree::leaf(goal.clone(), rule)),
            Some((false, _)) => Verdict::False,
            None => Verdict::Undecided,
        },
        // ¬P for a P that COMPUTES FALSE. The dual `Decidable` instances give
        // only through `of_decide_eq_true`; this goes through `Bool`'s
        // no-confusion discriminator instead, which the trusted base already
        // has — see `InferenceRule::GroundRefutation`.
        ProofExpr::Not(inner) => match decide_verdict(inner) {
            Verdict::False => {
                // Only a ground (dis)equality has a refutation channel; the
                // certifier re-checks that, but declining here keeps the
                // tactic's answer honest instead of deferring it to `qed`.
                if matches!(inner.as_ref(), ProofExpr::Identity(_, _)) {
                    Verdict::True(DerivationTree::leaf(
                        goal.clone(),
                        InferenceRule::GroundRefutation,
                    ))
                } else {
                    Verdict::Undecided
                }
            }
            // ¬P where P is TRUE: the goal is false, decidedly.
            Verdict::True(_) => Verdict::False,
            Verdict::Undecided => Verdict::Undecided,
        },
        ProofExpr::And(l, r) => match (decide_verdict(l), decide_verdict(r)) {
            (Verdict::True(lt), Verdict::True(rt)) => Verdict::True(DerivationTree::new(
                goal.clone(),
                InferenceRule::ConjunctionIntro,
                vec![lt, rt],
            )),
            (Verdict::False, _) | (_, Verdict::False) => Verdict::False,
            _ => Verdict::Undecided,
        },
        ProofExpr::Or(l, r) => match (decide_verdict(l), decide_verdict(r)) {
            (Verdict::True(side), _) | (Verdict::Undecided, Verdict::True(side)) => Verdict::True(
                DerivationTree::new(goal.clone(), InferenceRule::DisjunctionIntro, vec![side]),
            ),
            (Verdict::False, Verdict::True(side)) => Verdict::True(DerivationTree::new(
                goal.clone(),
                InferenceRule::DisjunctionIntro,
                vec![side],
            )),
            (Verdict::False, Verdict::False) => Verdict::False,
            _ => Verdict::Undecided,
        },
        // An implication holds by weakening whenever its consequent decides
        // true. A FALSE consequent is not a false implication — the antecedent
        // may be unsatisfiable — so that case is `Undecided`, not `False`.
        ProofExpr::Implies(_, r) => match decide_verdict(r) {
            Verdict::True(rt) => Verdict::True(DerivationTree::new(
                goal.clone(),
                InferenceRule::ImpliesIntro,
                vec![rt],
            )),
            _ => Verdict::Undecided,
        },
        _ => Verdict::Undecided,
    }
}

/// The historical shape: `Some(tree)` iff the goal decides TRUE. Kept because
/// callers that only want the certificate should not have to match three ways.
pub(crate) fn decide_expr(goal: &ProofExpr) -> Option<DerivationTree> {
    match decide_verdict(goal) {
        Verdict::True(tree) => Some(tree),
        _ => None,
    }
}
