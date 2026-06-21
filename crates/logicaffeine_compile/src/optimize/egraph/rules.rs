//! Rewrite rules (EXODIA Groups 1–3) with KERNEL-CHECKED soundness
//! certificates (D11b): the compiler's e-graph is a sibling of the proof
//! engine, not a stranger.
//!
//! # Soundness discipline
//!
//! Three obligations gate every rule, and all three fail closed:
//!
//! 1. **Type**: a rule fires only when the operand kinds are PROVEN
//!    (literal nodes or Oracle facts). Float operands never rewrite —
//!    `-0.0 + 0.0 == +0.0` already breaks `x + 0 → x` bit-for-bit.
//! 2. **Error/effect preservation**: a rule that DELETES a subterm (stops
//!    it being evaluated) requires [`CompilerEGraph::provably_total`] —
//!    `(a / b) * 0 → 0` would erase the divide-by-zero. Short-circuit
//!    forms (`false ∧ x → false`) are exempt for the RIGHT operand only,
//!    because the runtime never evaluates it either.
//! 3. **Value**: the identity itself carries a certificate checked through
//!    `logicaffeine_kernel`:
//!    - [`Certificate::Ring`] — polynomial identity via the kernel `ring`
//!      tactic. Ring identities with integer coefficients hold in EVERY
//!      commutative ring, in particular ℤ/2⁶⁴ = wrapping `i64`.
//!    - [`Certificate::BoolCases`] — exhaustive Bool case analysis: both
//!      sides are built as CIC terms over Match-defined `and/or/not` and
//!      normalized BY THE KERNEL for every valuation.
//!    - [`Certificate::Bitvector`] — wrapping/shift identities outside the
//!      ring fragment, exhaustively checked on the adversarial i64
//!      boundary grid. The full Z3 bitvector theorem lands with the Forge
//!      SMT layer (M15); this grid is the standing mechanical check.

use logicaffeine_kernel::{normalize, ring, Context, Literal as KLiteral, Term};

use super::{CompilerEGraph, CompilerENode, NodeId};
use crate::optimize::ScalarKind;

pub struct Rewrite {
    pub name: &'static str,
    pub apply: fn(&mut CompilerEGraph, NodeId) -> Option<NodeId>,
    pub certificate: Certificate,
}

// =====================================================================
// Certificates
// =====================================================================

/// A tiny polynomial expression language reified into the kernel's
/// `ring` tactic syntax.
#[derive(Debug, Clone)]
pub enum RingExpr {
    Var(u8),
    Const(i64),
    Add(Box<RingExpr>, Box<RingExpr>),
    Sub(Box<RingExpr>, Box<RingExpr>),
    Mul(Box<RingExpr>, Box<RingExpr>),
}

/// A tiny boolean expression language evaluated through kernel
/// normalization (Match-defined connectives over the prelude's Bool).
#[derive(Debug, Clone)]
pub enum BoolExpr {
    Var(u8),
    Const(bool),
    And(Box<BoolExpr>, Box<BoolExpr>),
    Or(Box<BoolExpr>, Box<BoolExpr>),
    Not(Box<BoolExpr>),
}

pub enum Certificate {
    /// `lhs = rhs` as polynomials — kernel `ring` tactic.
    Ring { lhs: RingExpr, rhs: RingExpr },
    /// `lhs = rhs` for all Bool valuations of `vars` variables — each side
    /// normalized by the kernel.
    BoolCases { vars: u8, lhs: BoolExpr, rhs: BoolExpr },
    /// Wrapping-i64 identity checked exhaustively on the boundary grid by
    /// the named check function (upgraded to a Z3 proof in M15).
    Bitvector { check: fn() -> Result<(), String> },
    /// An executable LOGOS property program (D11b — the language is its
    /// own test substrate): the program generates a deterministic corpus,
    /// compares the rule's two sides over it, and Shows its failure
    /// count. Certification = the tree-walker printing `expected`.
    LogosProperty { program: &'static str, expected: &'static str },
}

// ----- Ring reification ------------------------------------------------

fn s_lit(n: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SLit".to_string())),
        Box::new(Term::Lit(KLiteral::Int(n))),
    )
}

fn s_var(i: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SVar".to_string())),
        Box::new(Term::Lit(KLiteral::Int(i))),
    )
}

fn s_name(name: &str) -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(KLiteral::Text(name.to_string()))),
    )
}

fn s_app(f: Term, x: Term) -> Term {
    Term::App(
        Box::new(Term::App(Box::new(Term::Global("SApp".to_string())), Box::new(f))),
        Box::new(x),
    )
}

fn s_binop(op: &str, a: Term, b: Term) -> Term {
    s_app(s_app(s_name(op), a), b)
}

fn ring_to_syntax(e: &RingExpr) -> Term {
    match e {
        RingExpr::Var(i) => s_var(*i as i64),
        RingExpr::Const(k) => s_lit(*k),
        RingExpr::Add(a, b) => s_binop("add", ring_to_syntax(a), ring_to_syntax(b)),
        RingExpr::Sub(a, b) => s_binop("sub", ring_to_syntax(a), ring_to_syntax(b)),
        RingExpr::Mul(a, b) => s_binop("mul", ring_to_syntax(a), ring_to_syntax(b)),
    }
}

fn check_ring(lhs: &RingExpr, rhs: &RingExpr) -> Result<(), String> {
    let pl = ring::reify(&ring_to_syntax(lhs))
        .map_err(|e| format!("ring reify (lhs): {e:?}"))?;
    let pr = ring::reify(&ring_to_syntax(rhs))
        .map_err(|e| format!("ring reify (rhs): {e:?}"))?;
    if pl.canonical_eq(&pr) {
        Ok(())
    } else {
        Err("polynomials differ".to_string())
    }
}

// ----- Bool case analysis through kernel normalization ------------------

fn k_bool(b: bool) -> Term {
    Term::Global(if b { "true" } else { "false" }.to_string())
}

fn match_bool(disc: Term, on_true: Term, on_false: Term) -> Term {
    Term::Match {
        discriminant: Box::new(disc),
        motive: Box::new(Term::Lambda {
            param: "_b".to_string(),
            param_type: Box::new(Term::Global("Bool".to_string())),
            body: Box::new(Term::Global("Bool".to_string())),
        }),
        cases: vec![on_true, on_false],
    }
}

/// Build the kernel term for `e` under a concrete valuation. The
/// connectives are the textbook Match definitions:
/// `and a b = match a with true ⇒ b | false ⇒ false`, etc. — evaluation
/// is entirely the kernel's iota reduction.
fn bool_to_term(e: &BoolExpr, valuation: &[bool]) -> Term {
    match e {
        BoolExpr::Var(i) => k_bool(valuation[*i as usize]),
        BoolExpr::Const(b) => k_bool(*b),
        BoolExpr::And(a, b) => match_bool(
            bool_to_term(a, valuation),
            bool_to_term(b, valuation),
            k_bool(false),
        ),
        BoolExpr::Or(a, b) => match_bool(
            bool_to_term(a, valuation),
            k_bool(true),
            bool_to_term(b, valuation),
        ),
        BoolExpr::Not(a) => match_bool(bool_to_term(a, valuation), k_bool(false), k_bool(true)),
    }
}

fn check_bool_cases(vars: u8, lhs: &BoolExpr, rhs: &BoolExpr) -> Result<(), String> {
    let mut ctx = Context::new();
    logicaffeine_kernel::prelude::StandardLibrary::register(&mut ctx);
    for bits in 0..(1u32 << vars) {
        let valuation: Vec<bool> = (0..vars).map(|i| bits & (1 << i) != 0).collect();
        let nl = normalize(&ctx, &bool_to_term(lhs, &valuation));
        let nr = normalize(&ctx, &bool_to_term(rhs, &valuation));
        if nl != nr {
            return Err(format!("valuation {valuation:?}: {nl:?} ≠ {nr:?}"));
        }
    }
    Ok(())
}

// ----- Bitvector boundary grid ------------------------------------------

/// Adversarial i64 boundary values — overflow rims, sign boundaries,
/// shift-width rims, small primes.
const GRID: &[i64] = &[
    i64::MIN,
    i64::MIN + 1,
    i64::MIN / 2,
    -4_294_967_296,
    -65_537,
    -255,
    -65,
    -64,
    -63,
    -9,
    -8,
    -7,
    -3,
    -2,
    -1,
    0,
    1,
    2,
    3,
    7,
    8,
    9,
    63,
    64,
    65,
    255,
    65_537,
    4_294_967_296,
    i64::MAX / 2,
    i64::MAX - 1,
    i64::MAX,
];

const POWERS: &[u32] = &[0, 1, 2, 3, 5, 16, 31, 32, 62];

fn bv_mul_pow2_is_shl() -> Result<(), String> {
    for &x in GRID {
        for &n in POWERS {
            let p = 1i64.wrapping_shl(n);
            let mul = x.wrapping_mul(p);
            let shl = x.wrapping_shl(n);
            if mul != shl {
                return Err(format!("{x} * 2^{n}: mul {mul} ≠ shl {shl}"));
            }
        }
    }
    Ok(())
}

fn bv_div_pow2_is_shr_nonneg() -> Result<(), String> {
    for &x in GRID.iter().filter(|&&x| x >= 0) {
        for &n in POWERS {
            let p = 1i64.wrapping_shl(n);
            if p <= 0 {
                continue;
            }
            let div = x.wrapping_div(p);
            let shr = x.wrapping_shr(n);
            if div != shr {
                return Err(format!("{x} / 2^{n}: div {div} ≠ shr {shr}"));
            }
        }
    }
    // The guard is REQUIRED: witness that negatives disagree.
    if (-7i64).wrapping_div(4) == (-7i64).wrapping_shr(2) {
        return Err("guard witness failed: -7/4 should differ from -7>>2".to_string());
    }
    Ok(())
}

fn bv_mod_pow2_is_and_nonneg() -> Result<(), String> {
    for &x in GRID.iter().filter(|&&x| x >= 0) {
        for &n in POWERS {
            let p = 1i64.wrapping_shl(n);
            if p <= 0 {
                continue;
            }
            let md = x.wrapping_rem(p);
            let masked = x & (p - 1);
            if md != masked {
                return Err(format!("{x} % 2^{n}: rem {md} ≠ mask {masked}"));
            }
        }
    }
    if (-7i64).wrapping_rem(4) == (-7i64 & 3) {
        return Err("guard witness failed: -7%4 should differ from -7&3".to_string());
    }
    Ok(())
}

fn bv_div_one_is_identity() -> Result<(), String> {
    for &x in GRID {
        if x.wrapping_div(1) != x {
            return Err(format!("{x} / 1 ≠ {x}"));
        }
    }
    Ok(())
}

fn bv_not_not_is_identity() -> Result<(), String> {
    for &x in GRID {
        if !!x != x {
            return Err(format!("!!{x} ≠ {x}"));
        }
    }
    Ok(())
}

/// The constant-folder's evaluator, differentially checked against an
/// independent transcription of the kernel's wrapping arithmetic over the
/// full grid × grid, INCLUDING the error cases (zero divisors must refuse
/// to fold on both sides).
fn bv_fold_evaluator_matches_kernel() -> Result<(), String> {
    for &a in GRID {
        for &b in GRID {
            let cases: &[(&str, Option<i64>, Option<i64>)] = &[
                ("add", fold_binop(FoldOp::Add, a, b), Some(a.wrapping_add(b))),
                ("sub", fold_binop(FoldOp::Sub, a, b), Some(a.wrapping_sub(b))),
                ("mul", fold_binop(FoldOp::Mul, a, b), Some(a.wrapping_mul(b))),
                (
                    "div",
                    fold_binop(FoldOp::Div, a, b),
                    if b == 0 { None } else { Some(a.wrapping_div(b)) },
                ),
                (
                    "mod",
                    fold_binop(FoldOp::Mod, a, b),
                    if b == 0 { None } else { Some(a.wrapping_rem(b)) },
                ),
                ("shl", fold_binop(FoldOp::Shl, a, b), Some(a.wrapping_shl(b as u32))),
                ("shr", fold_binop(FoldOp::Shr, a, b), Some(a.wrapping_shr(b as u32))),
                ("xor", fold_binop(FoldOp::Xor, a, b), Some(a ^ b)),
                ("and", fold_binop(FoldOp::And, a, b), Some(a & b)),
                ("or", fold_binop(FoldOp::Or, a, b), Some(a | b)),
            ];
            for (name, got, want) in cases {
                if got != want {
                    return Err(format!("fold {name}({a}, {b}): {got:?} ≠ {want:?}"));
                }
            }
        }
    }
    Ok(())
}

// =====================================================================
// The fold evaluator (shared by the const-fold rule and its certificate)
// =====================================================================

#[derive(Clone, Copy)]
pub(crate) enum FoldOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Xor,
    And,
    Or,
}

/// Kernel-exact wrapping arithmetic. `None` = refuses to fold (the
/// runtime error is the program's meaning).
pub(crate) fn fold_binop(op: FoldOp, a: i64, b: i64) -> Option<i64> {
    Some(match op {
        FoldOp::Add => a.wrapping_add(b),
        FoldOp::Sub => a.wrapping_sub(b),
        FoldOp::Mul => a.wrapping_mul(b),
        FoldOp::Div => {
            if b == 0 {
                return None;
            }
            a.wrapping_div(b)
        }
        FoldOp::Mod => {
            if b == 0 {
                return None;
            }
            a.wrapping_rem(b)
        }
        FoldOp::Shl => a.wrapping_shl(b as u32),
        FoldOp::Shr => a.wrapping_shr(b as u32),
        FoldOp::Xor => a ^ b,
        FoldOp::And => a & b,
        FoldOp::Or => a | b,
    })
}

// =====================================================================
// Rule guards
// =====================================================================

fn is_int(eg: &mut CompilerEGraph, id: NodeId) -> bool {
    eg.scalar_of(id) == Some(ScalarKind::Int)
}

fn is_bool(eg: &mut CompilerEGraph, id: NodeId) -> bool {
    eg.scalar_of(id) == Some(ScalarKind::Bool)
}

/// May this operand be DELETED from the residual?
fn removable(eg: &mut CompilerEGraph, id: NodeId) -> bool {
    eg.provably_total(id)
}

/// The class is a proven zero / one (point interval — literals seed these,
/// and the Oracle can prove them for variables too).
fn proven(eg: &mut CompilerEGraph, id: NodeId, k: i64) -> bool {
    eg.int_value(id) == Some(k)
}

// =====================================================================
// Group 1 — algebraic identities (int-guarded)
// =====================================================================

fn r_add_zero(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Add(l, r) = eg.canonical_node(id) {
        if proven(eg, r, 0) && is_int(eg, l) && removable(eg, r) {
            return Some(l);
        }
        if proven(eg, l, 0) && is_int(eg, r) && removable(eg, l) {
            return Some(r);
        }
    }
    None
}

fn r_mul_one(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        if proven(eg, r, 1) && is_int(eg, l) && removable(eg, r) {
            return Some(l);
        }
        if proven(eg, l, 1) && is_int(eg, r) && removable(eg, l) {
            return Some(r);
        }
    }
    None
}

fn r_mul_zero(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        let zero_side = if proven(eg, r, 0) {
            Some((r, l))
        } else if proven(eg, l, 0) {
            Some((l, r))
        } else {
            None
        };
        if let Some((zero, other)) = zero_side {
            // BOTH operands stop being evaluated.
            if is_int(eg, other) && removable(eg, other) && removable(eg, zero) {
                let z = eg.add(CompilerENode::Int(0));
                return Some(z);
            }
        }
    }
    None
}

fn r_sub_zero(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Sub(l, r) = eg.canonical_node(id) {
        if proven(eg, r, 0) && is_int(eg, l) && removable(eg, r) {
            return Some(l);
        }
    }
    None
}

fn r_sub_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Sub(l, r) = eg.canonical_node(id) {
        if eg.find(l) == eg.find(r) && is_int(eg, l) && removable(eg, l) {
            let z = eg.add(CompilerENode::Int(0));
            return Some(z);
        }
    }
    None
}

fn r_div_one(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Div(l, r) = eg.canonical_node(id) {
        if proven(eg, r, 1) && is_int(eg, l) && removable(eg, r) {
            return Some(l);
        }
    }
    None
}

fn r_not_not_bool(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Not(inner) = eg.canonical_node(id) {
        for m in eg.class_members(inner) {
            if let CompilerENode::Not(x) = eg.canonical_node(m) {
                if is_bool(eg, x) {
                    return Some(x);
                }
            }
        }
    }
    None
}

fn r_not_not_int(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Not(inner) = eg.canonical_node(id) {
        for m in eg.class_members(inner) {
            if let CompilerENode::Not(x) = eg.canonical_node(m) {
                if is_int(eg, x) {
                    return Some(x);
                }
            }
        }
    }
    None
}

// =====================================================================
// Group 2 — boolean simplification (bool-guarded; `And`/`Or` are
// short-circuit in the language, so left-constant forms delete only the
// never-evaluated RIGHT operand)
// =====================================================================

fn r_true_and(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::And(l, r) = eg.canonical_node(id) {
        if eg.class_has_bool(l, true) && is_bool(eg, r) && removable(eg, l) {
            return Some(r);
        }
    }
    None
}

fn r_false_and(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::And(l, r) = eg.canonical_node(id) {
        // Short-circuit: the runtime never evaluates r either, so only
        // the deleted LEFT operand needs the totality proof.
        if eg.class_has_bool(l, false) && is_bool(eg, r) && removable(eg, l) {
            let f = eg.add(CompilerENode::Bool(false));
            return Some(f);
        }
    }
    None
}

fn r_true_or(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Or(l, r) = eg.canonical_node(id) {
        if eg.class_has_bool(l, true) && is_bool(eg, r) && removable(eg, l) {
            let t = eg.add(CompilerENode::Bool(true));
            return Some(t);
        }
    }
    None
}

fn r_false_or(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Or(l, r) = eg.canonical_node(id) {
        if eg.class_has_bool(l, false) && is_bool(eg, r) && removable(eg, l) {
            return Some(r);
        }
    }
    None
}

fn r_and_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::And(l, r) = eg.canonical_node(id) {
        // The kept copy evaluates identically (same class, same first
        // error) — no totality requirement.
        if eg.find(l) == eg.find(r) && is_bool(eg, l) {
            return Some(l);
        }
    }
    None
}

fn r_or_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Or(l, r) = eg.canonical_node(id) {
        if eg.find(l) == eg.find(r) && is_bool(eg, l) {
            return Some(l);
        }
    }
    None
}

/// Is some member of `maybe_not`'s class `Not(x)` with x ≡ `base`?
fn class_negates(eg: &mut CompilerEGraph, maybe_not: NodeId, base: NodeId) -> bool {
    let broot = eg.find(base);
    for m in eg.class_members(maybe_not) {
        if let CompilerENode::Not(x) = eg.canonical_node(m) {
            if eg.find(x) == broot {
                return true;
            }
        }
    }
    false
}

fn r_and_not_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::And(l, r) = eg.canonical_node(id) {
        if is_bool(eg, l)
            && (class_negates(eg, r, l) || class_negates(eg, l, r))
            && removable(eg, l)
            && removable(eg, r)
        {
            let f = eg.add(CompilerENode::Bool(false));
            return Some(f);
        }
    }
    None
}

fn r_or_not_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Or(l, r) = eg.canonical_node(id) {
        if is_bool(eg, l)
            && (class_negates(eg, r, l) || class_negates(eg, l, r))
            && removable(eg, l)
            && removable(eg, r)
        {
            let t = eg.add(CompilerENode::Bool(true));
            return Some(t);
        }
    }
    None
}

// =====================================================================
// Group 3 — strength reduction (Oracle-conditional)
// =====================================================================

fn pow2_log(k: i64) -> Option<u32> {
    if k > 0 && (k as u64).is_power_of_two() {
        Some(k.trailing_zeros())
    } else {
        None
    }
}

fn r_mul_two_add(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        let x = if proven(eg, r, 2) && removable(eg, r) {
            Some(l)
        } else if proven(eg, l, 2) && removable(eg, l) {
            Some(r)
        } else {
            None
        };
        if let Some(x) = x {
            if is_int(eg, x) {
                let sum = eg.add(CompilerENode::Add(x, x));
                return Some(sum);
            }
        }
    }
    None
}

fn r_mul_pow2_shl(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        let hit = if let Some(n) = eg.int_value(r).and_then(pow2_log) {
            if removable(eg, r) { Some((l, n)) } else { None }
        } else if let Some(n) = eg.int_value(l).and_then(pow2_log) {
            if removable(eg, l) { Some((r, n)) } else { None }
        } else {
            None
        };
        if let Some((x, n)) = hit {
            if is_int(eg, x) {
                let shift = eg.add(CompilerENode::Int(n as i64));
                let shl = eg.add(CompilerENode::Shl(x, shift));
                return Some(shl);
            }
        }
    }
    None
}

fn r_div_pow2_shr(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Div(l, r) = eg.canonical_node(id) {
        if let Some(n) = eg.int_value(r).and_then(pow2_log) {
            // ORACLE GATE: truncating ÷ and arithmetic shift agree only
            // for proven non-negative dividends.
            if is_int(eg, l) && eg.proven_nonneg(l) && removable(eg, r) {
                let shift = eg.add(CompilerENode::Int(n as i64));
                let shr = eg.add(CompilerENode::Shr(l, shift));
                return Some(shr);
            }
        }
    }
    None
}

fn r_mod_pow2_and(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mod(l, r) = eg.canonical_node(id) {
        if let Some(k) = eg.int_value(r) {
            if pow2_log(k).is_some() && is_int(eg, l) && eg.proven_nonneg(l) && removable(eg, r) {
                let mask = eg.add(CompilerENode::Int(k - 1));
                let masked = eg.add(CompilerENode::And(l, mask));
                return Some(masked);
            }
        }
    }
    None
}

// =====================================================================
// Group 5: deforestation / fusion — the Len/Slice/Copy algebra
// (Wadler 1988 at expression grain: intermediate collections that exist
// only to be measured or read once never materialize)
// =====================================================================

/// A member of `class` matching `pred`, canonicalized.
fn member_matching(
    eg: &mut CompilerEGraph,
    class: NodeId,
    pred: fn(&CompilerENode) -> bool,
) -> Option<CompilerENode> {
    let members = eg.class_members(class);
    members.into_iter().map(|m| eg.canonical_node(m)).find(pred)
}

/// `len(copy(xs))` → `len(xs)`: over a PROVEN collection the copy cannot
/// raise and preserves length — the O(n) materialization disappears.
fn r_len_copy(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Len(c) = eg.canonical_node(id) {
        if let Some(CompilerENode::Copy(inner)) =
            member_matching(eg, c, |n| matches!(n, CompilerENode::Copy(_)))
        {
            if eg.proven_collection(inner) {
                return Some(eg.add(CompilerENode::Len(inner)));
            }
        }
    }
    None
}

/// `index(copy(xs), i)` → `index(xs, i)`: same contents, same bounds,
/// same error — one read through a fresh copy is unobservable.
fn r_index_copy(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Index(c, i) = eg.canonical_node(id) {
        if let Some(CompilerENode::Copy(inner)) =
            member_matching(eg, c, |n| matches!(n, CompilerENode::Copy(_)))
        {
            if eg.proven_collection(inner) {
                return Some(eg.add(CompilerENode::Index(inner, i)));
            }
        }
    }
    None
}

/// `copy(copy(x))` ≡ `copy(x)` extensionally: both are fresh values with
/// identical contents, and an erroring operand raises the SAME first
/// error on both sides — unconditional.
fn r_copy_copy(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Copy(c) = eg.canonical_node(id) {
        if member_matching(eg, c, |n| matches!(n, CompilerENode::Copy(_))).is_some() {
            return Some(eg.find(c));
        }
    }
    None
}

/// `slice(xs, 1, len(xs))` ≡ `copy(xs)` for a PROVEN LIST: the full
/// slice's clamps are no-ops and both sides are fresh. Gated on listness
/// because slicing is list-shaped while copy accepts any collection.
fn r_slice_full_copy(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Slice(xs, a, b) = eg.canonical_node(id) {
        if eg.int_value(a) == Some(1) && eg.proven_list(xs) {
            let len_match = member_matching(eg, b, |n| matches!(n, CompilerENode::Len(_)));
            if let Some(CompilerENode::Len(c2)) = len_match {
                if eg.find(c2) == eg.find(xs) {
                    return Some(eg.add(CompilerENode::Copy(xs)));
                }
            }
        }
    }
    None
}

/// `len(slice(xs, a, b))` → `b − a + 1` under PROOFS: constant bounds
/// with 1 ≤ a, a ≤ b + 1 and b ≤ len(xs) guaranteed — the clamps cannot
/// engage, so the length is exact and the whole slice (xs included)
/// disappears. Deleting xs requires its tree total.
fn r_len_slice_bounds(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Len(s) = eg.canonical_node(id) {
        let slice = member_matching(eg, s, |n| matches!(n, CompilerENode::Slice(..)));
        if let Some(CompilerENode::Slice(xs, a, b)) = slice {
            let (Some(av), Some(bv)) = (eg.int_value(a), eg.int_value(b)) else {
                return None;
            };
            if av < 1 || av > bv.saturating_add(1) || !eg.proven_list(xs) {
                return None;
            }
            let len_class = eg.add(CompilerENode::Len(xs));
            let Some((len_lo, _)) = eg.int_range(len_class) else {
                return None;
            };
            if bv <= len_lo && removable(eg, xs) {
                return Some(eg.add(CompilerENode::Int(bv - av + 1)));
            }
        }
    }
    None
}

// =====================================================================
// Commutativity / associativity (int-only — float reassociation is
// unsound, and `And`/`Or` short-circuit so they are NOT commutative
// with respect to errors)
// =====================================================================

fn r_add_comm(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Add(l, r) = eg.canonical_node(id) {
        // Swapping evaluation order is observable through error precedence
        // unless both operands are total.
        if is_int(eg, l) && is_int(eg, r) && removable(eg, l) && removable(eg, r) {
            let flipped = eg.add(CompilerENode::Add(r, l));
            return Some(flipped);
        }
    }
    None
}

fn r_mul_comm(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        if is_int(eg, l) && is_int(eg, r) && removable(eg, l) && removable(eg, r) {
            let flipped = eg.add(CompilerENode::Mul(r, l));
            return Some(flipped);
        }
    }
    None
}

fn r_add_assoc(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Add(l, c) = eg.canonical_node(id) {
        if !is_int(eg, c) {
            return None;
        }
        for m in eg.class_members(l) {
            if let CompilerENode::Add(a, b) = eg.canonical_node(m) {
                if is_int(eg, a) && is_int(eg, b) {
                    let bc = eg.add(CompilerENode::Add(b, c));
                    let abc = eg.add(CompilerENode::Add(a, bc));
                    return Some(abc);
                }
            }
        }
    }
    None
}

fn r_mul_assoc(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, c) = eg.canonical_node(id) {
        if !is_int(eg, c) {
            return None;
        }
        for m in eg.class_members(l) {
            if let CompilerENode::Mul(a, b) = eg.canonical_node(m) {
                if is_int(eg, a) && is_int(eg, b) {
                    let bc = eg.add(CompilerENode::Mul(b, c));
                    let abc = eg.add(CompilerENode::Mul(a, bc));
                    return Some(abc);
                }
            }
        }
    }
    None
}

// =====================================================================
// Constant folding (point intervals — subsumes literal folding and
// extends to Oracle-proven single values)
// =====================================================================

fn r_const_fold(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    let node = eg.canonical_node(id);
    let op = match node {
        CompilerENode::Add(..) => FoldOp::Add,
        CompilerENode::Sub(..) => FoldOp::Sub,
        CompilerENode::Mul(..) => FoldOp::Mul,
        CompilerENode::Div(..) => FoldOp::Div,
        CompilerENode::Mod(..) => FoldOp::Mod,
        CompilerENode::Shl(..) => FoldOp::Shl,
        CompilerENode::Shr(..) => FoldOp::Shr,
        CompilerENode::BitXor(..) => FoldOp::Xor,
        _ => return None,
    };
    let (l, r) = match node {
        CompilerENode::Add(l, r)
        | CompilerENode::Sub(l, r)
        | CompilerENode::Mul(l, r)
        | CompilerENode::Div(l, r)
        | CompilerENode::Mod(l, r)
        | CompilerENode::Shl(l, r)
        | CompilerENode::Shr(l, r)
        | CompilerENode::BitXor(l, r) => (l, r),
        _ => unreachable!(),
    };
    if eg.find(id) == eg.find(l) || eg.find(id) == eg.find(r) {
        // Cyclic class (id ≡ one of its own children) — folding would
        // self-justify; leave it to extraction.
        return None;
    }
    let a = eg.int_value(l)?;
    let b = eg.int_value(r)?;
    if !(removable(eg, l) && removable(eg, r)) {
        return None;
    }
    let v = fold_binop(op, a, b)?;
    let lit = eg.add(CompilerENode::Int(v));
    Some(lit)
}

// =====================================================================
// The registry
// =====================================================================

fn rx(e: RingExpr) -> Box<RingExpr> {
    Box::new(e)
}

fn bx(e: BoolExpr) -> Box<BoolExpr> {
    Box::new(e)
}

pub fn all() -> Vec<Rewrite> {
    use BoolExpr as B;
    use RingExpr as R;
    vec![
        Rewrite {
            name: "add-zero",
            apply: r_add_zero,
            certificate: Certificate::Ring {
                lhs: R::Add(rx(R::Var(0)), rx(R::Const(0))),
                rhs: R::Var(0),
            },
        },
        Rewrite {
            name: "mul-one",
            apply: r_mul_one,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Var(0)), rx(R::Const(1))),
                rhs: R::Var(0),
            },
        },
        Rewrite {
            name: "mul-zero",
            apply: r_mul_zero,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Var(0)), rx(R::Const(0))),
                rhs: R::Const(0),
            },
        },
        Rewrite {
            name: "sub-zero",
            apply: r_sub_zero,
            certificate: Certificate::Ring {
                lhs: R::Sub(rx(R::Var(0)), rx(R::Const(0))),
                rhs: R::Var(0),
            },
        },
        Rewrite {
            name: "sub-self",
            apply: r_sub_self,
            certificate: Certificate::Ring {
                lhs: R::Sub(rx(R::Var(0)), rx(R::Var(0))),
                rhs: R::Const(0),
            },
        },
        Rewrite {
            name: "div-one",
            apply: r_div_one,
            certificate: Certificate::Bitvector { check: bv_div_one_is_identity },
        },
        Rewrite {
            name: "not-not-bool",
            apply: r_not_not_bool,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Not(bx(B::Not(bx(B::Var(0))))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "not-not-int",
            apply: r_not_not_int,
            certificate: Certificate::Bitvector { check: bv_not_not_is_identity },
        },
        Rewrite {
            name: "true-and",
            apply: r_true_and,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::And(bx(B::Const(true)), bx(B::Var(0))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "false-and",
            apply: r_false_and,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::And(bx(B::Const(false)), bx(B::Var(0))),
                rhs: B::Const(false),
            },
        },
        Rewrite {
            name: "true-or",
            apply: r_true_or,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Or(bx(B::Const(true)), bx(B::Var(0))),
                rhs: B::Const(true),
            },
        },
        Rewrite {
            name: "false-or",
            apply: r_false_or,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Or(bx(B::Const(false)), bx(B::Var(0))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "and-self",
            apply: r_and_self,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::And(bx(B::Var(0)), bx(B::Var(0))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "or-self",
            apply: r_or_self,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Or(bx(B::Var(0)), bx(B::Var(0))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "and-not-self",
            apply: r_and_not_self,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::And(bx(B::Var(0)), bx(B::Not(bx(B::Var(0))))),
                rhs: B::Const(false),
            },
        },
        Rewrite {
            name: "or-not-self",
            apply: r_or_not_self,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Or(bx(B::Var(0)), bx(B::Not(bx(B::Var(0))))),
                rhs: B::Const(true),
            },
        },
        Rewrite {
            name: "mul-two-add",
            apply: r_mul_two_add,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Var(0)), rx(R::Const(2))),
                rhs: R::Add(rx(R::Var(0)), rx(R::Var(0))),
            },
        },
        Rewrite {
            name: "mul-pow2-shl",
            apply: r_mul_pow2_shl,
            certificate: Certificate::Bitvector { check: bv_mul_pow2_is_shl },
        },
        Rewrite {
            name: "div-pow2-shr",
            apply: r_div_pow2_shr,
            certificate: Certificate::Bitvector { check: bv_div_pow2_is_shr_nonneg },
        },
        Rewrite {
            name: "mod-pow2-and",
            apply: r_mod_pow2_and,
            certificate: Certificate::Bitvector { check: bv_mod_pow2_is_and_nonneg },
        },
        Rewrite {
            name: "add-comm",
            apply: r_add_comm,
            certificate: Certificate::Ring {
                lhs: R::Add(rx(R::Var(0)), rx(R::Var(1))),
                rhs: R::Add(rx(R::Var(1)), rx(R::Var(0))),
            },
        },
        Rewrite {
            name: "add-assoc",
            apply: r_add_assoc,
            certificate: Certificate::Ring {
                lhs: R::Add(rx(R::Add(rx(R::Var(0)), rx(R::Var(1)))), rx(R::Var(2))),
                rhs: R::Add(rx(R::Var(0)), rx(R::Add(rx(R::Var(1)), rx(R::Var(2))))),
            },
        },
        Rewrite {
            name: "mul-comm",
            apply: r_mul_comm,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Var(0)), rx(R::Var(1))),
                rhs: R::Mul(rx(R::Var(1)), rx(R::Var(0))),
            },
        },
        Rewrite {
            name: "mul-assoc",
            apply: r_mul_assoc,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Mul(rx(R::Var(0)), rx(R::Var(1)))), rx(R::Var(2))),
                rhs: R::Mul(rx(R::Var(0)), rx(R::Mul(rx(R::Var(1)), rx(R::Var(2))))),
            },
        },
        Rewrite {
            name: "const-fold",
            apply: r_const_fold,
            certificate: Certificate::Bitvector { check: bv_fold_evaluator_matches_kernel },
        },
        Rewrite {
            name: "len-copy",
            apply: r_len_copy,
            certificate: Certificate::LogosProperty {
                program: PROP_LEN_COPY,
                expected: "0",
            },
        },
        Rewrite {
            name: "index-copy",
            apply: r_index_copy,
            certificate: Certificate::LogosProperty {
                program: PROP_INDEX_COPY,
                expected: "0",
            },
        },
        Rewrite {
            name: "copy-copy",
            apply: r_copy_copy,
            certificate: Certificate::LogosProperty {
                program: PROP_COPY_COPY,
                expected: "0",
            },
        },
        Rewrite {
            name: "slice-full-copy",
            apply: r_slice_full_copy,
            certificate: Certificate::LogosProperty {
                program: PROP_SLICE_FULL_COPY,
                expected: "0",
            },
        },
        Rewrite {
            name: "len-slice-bounds",
            apply: r_len_slice_bounds,
            certificate: Certificate::LogosProperty {
                program: PROP_LEN_SLICE_BOUNDS,
                expected: "0",
            },
        },
    ]
}

// =====================================================================
// Group 5 property certificates: deterministic LCG corpora (lists of
// length 0..12 — the empty list and the a = b + 1 empty slice are in
// range by construction), each Showing its failure count.
// =====================================================================

const PROP_LEN_COPY: &str = "## Main\n\
Let mutable seed be 42.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 40:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 13.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   If length of (copy of xs) is not length of xs:\n\
\x20       Set failures to failures + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

const PROP_INDEX_COPY: &str = "## Main\n\
Let mutable seed be 99.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 40:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 13.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   If n is at least 1:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Let k be seed % n + 1.\n\
\x20       If item k of (copy of xs) is not item k of xs:\n\
\x20           Set failures to failures + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

const PROP_COPY_COPY: &str = "## Main\n\
Let mutable seed be 7.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 40:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 13.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   Let a be copy of (copy of xs).\n\
\x20   Let b be copy of xs.\n\
\x20   If length of a is not length of b:\n\
\x20       Set failures to failures + 1.\n\
\x20   Let mutable j be 1.\n\
\x20   While j is at most length of a:\n\
\x20       If item j of a is not item j of b:\n\
\x20           Set failures to failures + 1.\n\
\x20       Set j to j + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

const PROP_SLICE_FULL_COPY: &str = "## Main\n\
Let mutable seed be 1234.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 40:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 13.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   Let s be items 1 through length of xs of xs.\n\
\x20   Let c be copy of xs.\n\
\x20   If length of s is not length of c:\n\
\x20       Set failures to failures + 1.\n\
\x20   Let mutable j be 1.\n\
\x20   While j is at most length of s:\n\
\x20       If item j of s is not item j of c:\n\
\x20           Set failures to failures + 1.\n\
\x20       Set j to j + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

const PROP_LEN_SLICE_BOUNDS: &str = "## Main\n\
Let mutable seed be 5150.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 60:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 12 + 1.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let lo be seed % n + 1.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let hi be lo - 1 + (seed % (n - lo + 2)).\n\
\x20   Let s be items (lo) through (hi) of xs.\n\
\x20   If length of s is not hi - lo + 1:\n\
\x20       Set failures to failures + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

/// Check every rule's certificate through the kernel. Returns the number
/// of verified rules; the first failure aborts with the rule's name.
pub fn verify_all_with_kernel() -> Result<usize, String> {
    let rules = all();
    for rule in &rules {
        let outcome = match &rule.certificate {
            Certificate::Ring { lhs, rhs } => check_ring(lhs, rhs),
            Certificate::BoolCases { vars, lhs, rhs } => check_bool_cases(*vars, lhs, rhs),
            Certificate::Bitvector { check } => check(),
            Certificate::LogosProperty { program, expected } => {
                match crate::compile::interpret_program(program) {
                    Ok(out) if out.trim() == *expected => Ok(()),
                    Ok(out) => Err(format!(
                        "property program printed {:?}, expected {:?}",
                        out.trim(),
                        expected
                    )),
                    Err(e) => Err(format!("property program failed: {e:?}")),
                }
            }
        };
        outcome.map_err(|e| format!("rule '{}': {e}", rule.name))?;
    }
    Ok(rules.len())
}
