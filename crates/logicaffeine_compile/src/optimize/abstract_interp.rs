use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, MatchArm, Pattern, Stmt};
use crate::intern::Symbol;

/// Per-collection ELEMENT-TYPE tracking: a read `item k of arr` carries `arr`'s
/// proven element type (the join of every value written into it). A
/// homogeneously-typed collection's reads gain a concrete scalar kind, which the
/// magic-reciprocal modulo gate (`(... + ...) % c`) consumes to replace idiv.
/// PROMOTED 2026-06-21: default ON (kill-switch LOGOS_ELEM_TYPE=0) — coins -11%
/// on the faithful interleaved A/B, all 33 benchmarks bit-identical, no regression
/// (a spurious histogram +2.4% on best-of-min was confirmed noise — identical raw
/// distributions). With it off `eval_type` treats an index read as `Top` exactly
/// as before, so the byte output is unchanged.
fn elem_type_enabled() -> bool {
    crate::optimize::active_config().is_on(crate::optimization::Opt::ElemType)
}

#[derive(Clone, Debug, PartialEq)]
enum Bound {
    NegInf,
    Finite(i64),
    PosInf,
}

impl Bound {
    fn add(&self, other: &Bound) -> Bound {
        match (self, other) {
            (Bound::Finite(a), Bound::Finite(b)) => {
                match a.checked_add(*b) {
                    Some(r) => Bound::Finite(r),
                    None => if *a > 0 { Bound::PosInf } else { Bound::NegInf },
                }
            }
            (Bound::PosInf, Bound::NegInf) | (Bound::NegInf, Bound::PosInf) => Bound::NegInf,
            (Bound::PosInf, _) | (_, Bound::PosInf) => Bound::PosInf,
            (Bound::NegInf, _) | (_, Bound::NegInf) => Bound::NegInf,
        }
    }

    fn sub(&self, other: &Bound) -> Bound {
        match (self, other) {
            (Bound::Finite(a), Bound::Finite(b)) => {
                match a.checked_sub(*b) {
                    Some(r) => Bound::Finite(r),
                    None => if *a > 0 { Bound::PosInf } else { Bound::NegInf },
                }
            }
            (Bound::PosInf, Bound::PosInf) | (Bound::NegInf, Bound::NegInf) => Bound::NegInf,
            (Bound::PosInf, _) | (_, Bound::NegInf) => Bound::PosInf,
            (Bound::NegInf, _) | (_, Bound::PosInf) => Bound::NegInf,
        }
    }

    fn mul(&self, other: &Bound) -> Bound {
        // An exact-zero endpoint annihilates: every product with it is 0
        // (this keeps `0 * [c, +inf]` at 0 rather than collapsing to top).
        if matches!(self, Bound::Finite(0)) || matches!(other, Bound::Finite(0)) {
            return Bound::Finite(0);
        }
        if let (Bound::Finite(a), Bound::Finite(b)) = (self, other) {
            return match a.checked_mul(*b) {
                Some(r) => Bound::Finite(r),
                None => {
                    if (*a > 0) == (*b > 0) {
                        Bound::PosInf
                    } else {
                        Bound::NegInf
                    }
                }
            };
        }
        // At least one infinite, neither zero — the result's sign is the
        // product of the operands' signs.
        let positive = |x: &Bound| match x {
            Bound::PosInf => true,
            Bound::NegInf => false,
            Bound::Finite(v) => *v > 0,
        };
        if positive(self) == positive(other) {
            Bound::PosInf
        } else {
            Bound::NegInf
        }
    }

    /// Truncating division by a nonzero constant `k` (toward zero, matching
    /// Rust `i64` `/`). An infinite endpoint divided by a finite divisor stays
    /// infinite, its sign flipping when `k < 0`.
    fn div_by(&self, k: i64) -> Bound {
        match self {
            Bound::Finite(a) => Bound::Finite(a / k),
            Bound::NegInf => {
                if k > 0 {
                    Bound::NegInf
                } else {
                    Bound::PosInf
                }
            }
            Bound::PosInf => {
                if k > 0 {
                    Bound::PosInf
                } else {
                    Bound::NegInf
                }
            }
        }
    }

    /// Arithmetic right shift by a constant `k ∈ [0, 63]` — floor-division by
    /// `2^k`, MONOTONE non-decreasing, so an interval's endpoints map straight
    /// through. Infinite endpoints stay infinite (their magnitude only shrinks).
    fn shr_by(&self, k: u32) -> Bound {
        match self {
            Bound::Finite(a) => Bound::Finite(a >> k),
            Bound::NegInf => Bound::NegInf,
            Bound::PosInf => Bound::PosInf,
        }
    }

    fn cmp_bound(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Bound::NegInf, Bound::NegInf) => std::cmp::Ordering::Equal,
            (Bound::NegInf, _) => std::cmp::Ordering::Less,
            (_, Bound::NegInf) => std::cmp::Ordering::Greater,
            (Bound::PosInf, Bound::PosInf) => std::cmp::Ordering::Equal,
            (Bound::PosInf, _) => std::cmp::Ordering::Greater,
            (_, Bound::PosInf) => std::cmp::Ordering::Less,
            (Bound::Finite(a), Bound::Finite(b)) => a.cmp(b),
        }
    }

    fn min_bound(a: &Bound, b: &Bound) -> Bound {
        if a.cmp_bound(b) == std::cmp::Ordering::Less { a.clone() } else { b.clone() }
    }

    fn max_bound(a: &Bound, b: &Bound) -> Bound {
        if a.cmp_bound(b) == std::cmp::Ordering::Greater { a.clone() } else { b.clone() }
    }
}

/// A bounded lattice with the operations abstract interpretation needs.
///
/// Each abstract domain of the Oracle (intervals, types, collection shapes,
/// nullability, aliasing) implements this. The product lattice combines them
/// componentwise. `widen` accelerates ascending chains to a fixpoint so loop
/// analysis terminates; `leq` is the lattice order `⊑` (more precise ⊑ less
/// precise, i.e. `γ`-subset).
trait AbstractDomain: Clone {
    /// Greatest element `⊤` — "no information".
    fn top() -> Self;
    /// Least element `⊥` — "unreachable / empty".
    fn bottom() -> Self;
    /// Least upper bound `⊔` (merge at control-flow joins).
    fn join(&self, other: &Self) -> Self;
    /// Greatest lower bound `⊓` (refine under a known fact).
    fn meet(&self, other: &Self) -> Self;
    /// Widening `▽` — over-approximates the join to force loop convergence.
    fn widen(&self, other: &Self) -> Self;
    /// Lattice order: `self ⊑ other`.
    fn leq(&self, other: &Self) -> bool;
}

#[derive(Clone, Debug)]
struct Interval {
    lo: Bound,
    hi: Bound,
}

impl Interval {
    fn exact(n: i64) -> Self {
        Interval { lo: Bound::Finite(n), hi: Bound::Finite(n) }
    }

    fn top() -> Self {
        Interval { lo: Bound::NegInf, hi: Bound::PosInf }
    }

    fn non_negative() -> Self {
        Interval { lo: Bound::Finite(0), hi: Bound::PosInf }
    }

    fn is_exact(&self) -> Option<i64> {
        if let (Bound::Finite(a), Bound::Finite(b)) = (&self.lo, &self.hi) {
            if a == b { return Some(*a); }
        }
        None
    }

    /// The empty interval `⊥`. Represented as `lo > hi`, which no reachable
    /// program point ever constructs, so it is unambiguous.
    fn bottom() -> Self {
        Interval { lo: Bound::PosInf, hi: Bound::NegInf }
    }

    fn is_bottom(&self) -> bool {
        self.lo.cmp_bound(&self.hi) == std::cmp::Ordering::Greater
    }

    fn join(&self, other: &Interval) -> Interval {
        if self.is_bottom() { return other.clone(); }
        if other.is_bottom() { return self.clone(); }
        Interval {
            lo: Bound::min_bound(&self.lo, &other.lo),
            hi: Bound::max_bound(&self.hi, &other.hi),
        }
    }

    /// Intersection `⊓`. Disjoint inputs collapse to `⊥`.
    fn meet(&self, other: &Interval) -> Interval {
        if self.is_bottom() || other.is_bottom() { return Interval::bottom(); }
        let r = Interval {
            lo: Bound::max_bound(&self.lo, &other.lo),
            hi: Bound::min_bound(&self.hi, &other.hi),
        };
        if r.is_bottom() { Interval::bottom() } else { r }
    }

    /// Widening `self ▽ other` (self = old, other = new). An unstable bound is
    /// thrown to the corresponding infinity so ascending chains converge.
    fn widen(&self, other: &Interval) -> Interval {
        if self.is_bottom() { return other.clone(); }
        if other.is_bottom() { return self.clone(); }
        // EXODIA's threshold ladder: an unstable bound snaps to the next
        // rung before falling off to ±∞ — loop counters converge to finite
        // bounds (e.g. `While i < 10` keeps i ⊑ [entry, 10ish]) instead of
        // instantly losing all precision. Still a finite ascent → terminates.
        const WIDENING_THRESHOLDS: &[i64] = &[-1000, -100, -10, -1, 0, 1, 10, 100, 1000];
        let lo = if other.lo.cmp_bound(&self.lo) == std::cmp::Ordering::Less {
            match other.lo {
                Bound::Finite(v) => WIDENING_THRESHOLDS
                    .iter()
                    .rev()
                    .find(|&&t| t <= v)
                    .map(|&t| Bound::Finite(t))
                    .unwrap_or(Bound::NegInf),
                _ => Bound::NegInf,
            }
        } else {
            self.lo.clone()
        };
        let hi = if other.hi.cmp_bound(&self.hi) == std::cmp::Ordering::Greater {
            match other.hi {
                Bound::Finite(v) => WIDENING_THRESHOLDS
                    .iter()
                    .find(|&&t| t >= v)
                    .map(|&t| Bound::Finite(t))
                    .unwrap_or(Bound::PosInf),
                _ => Bound::PosInf,
            }
        } else {
            self.hi.clone()
        };
        Interval { lo, hi }
    }

    /// Lattice order `self ⊑ other`, i.e. `self ⊆ other` as concrete sets.
    fn leq(&self, other: &Interval) -> bool {
        if self.is_bottom() { return true; }
        if other.is_bottom() { return false; }
        other.lo.cmp_bound(&self.lo) != std::cmp::Ordering::Greater
            && self.hi.cmp_bound(&other.hi) != std::cmp::Ordering::Greater
    }

    fn add(&self, other: &Interval) -> Interval {
        Interval {
            lo: self.lo.add(&other.lo),
            hi: self.hi.add(&other.hi),
        }
    }

    fn sub(&self, other: &Interval) -> Interval {
        Interval {
            lo: self.lo.sub(&other.hi),
            hi: self.hi.sub(&other.lo),
        }
    }

    fn mul(&self, other: &Interval) -> Interval {
        // Standard interval product: the extremes lie at the four corner
        // products. With the `0 * inf = 0` convention in `Bound::mul`, this
        // stays precise for sign-definite operands (e.g. `i * i` with `i >= 2`
        // gives `[4, +inf)`, not top).
        let corners = [
            self.lo.mul(&other.lo),
            self.lo.mul(&other.hi),
            self.hi.mul(&other.lo),
            self.hi.mul(&other.hi),
        ];
        let mut lo = corners[0].clone();
        let mut hi = corners[0].clone();
        for c in &corners[1..] {
            lo = Bound::min_bound(&lo, c);
            hi = Bound::max_bound(&hi, c);
        }
        Interval { lo, hi }
    }

    fn div(&self, other: &Interval) -> Interval {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            if b != 0 {
                // `wrapping_div` matches the runtime (semantics/arith.rs) and
                // avoids the `i64::MIN / -1` overflow panic of raw `/`.
                return Interval::exact(a.wrapping_div(b));
            }
        }
        // Constant nonzero divisor, arbitrary dividend range. Truncating
        // division (toward zero, matching Rust `/`) is monotone in the dividend
        // for a positive divisor, so both bounds map through; a negative divisor
        // additionally swaps them. This generalizes the exact/exact case so a
        // fixpoint can bound `seed / 65536`, `hi / 2`, etc.
        if let Some(k) = other.is_exact() {
            if k > 0 {
                return Interval { lo: self.lo.div_by(k), hi: self.hi.div_by(k) };
            } else if k < 0 {
                return Interval { lo: self.hi.div_by(k), hi: self.lo.div_by(k) };
            }
        }
        Interval::top()
    }

    fn modulo(&self, other: &Interval) -> Interval {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            if b != 0 {
                // `wrapping_rem` matches the runtime (semantics/arith.rs) and
                // avoids the `i64::MIN % -1` overflow panic of raw `%`.
                return Interval::exact(a.wrapping_rem(b));
            }
        }
        // Constant nonzero divisor, arbitrary dividend range. LOGOS `%` is the
        // TRUNCATED remainder: `|x % k| <= |k| - 1` and the sign follows the
        // DIVIDEND. So a provably non-negative dividend gives `[0, |k|-1]`
        // (the counting_sort / LCG case: `seed % 2147483648 ∈ [0, 2^31-1]`,
        // `(seed/65536) % 1000 ∈ [0, 999]`), a non-positive one `[-(|k|-1), 0]`,
        // and a sign-spanning one `[-(|k|-1), |k|-1]`.
        if let Some(k) = other.is_exact() {
            if k != 0 {
                let m = k.checked_abs().unwrap_or(i64::MAX).saturating_sub(1).max(0);
                let nonneg = matches!(self.lo, Bound::Finite(v) if v >= 0);
                let nonpos = matches!(self.hi, Bound::Finite(v) if v <= 0);
                let lo = if nonneg { Bound::Finite(0) } else { Bound::Finite(-m) };
                let hi = if nonpos { Bound::Finite(0) } else { Bound::Finite(m) };
                return Interval { lo, hi };
            }
        }
        // VARIABLE (or otherwise non-constant) divisor. The truncated
        // remainder's SIGN still follows the dividend, so a provably
        // non-negative dividend yields a non-negative result (magnitude
        // unbounded without a concrete divisor) — the `x % n >= 0` element
        // LOWER bound the symbolic prover pairs with the symbolic upper
        // `x % n <= n - 1` (graph_bfs: `adj` filled with `(...) % n`). A
        // zero divisor errors at runtime, so the result's range is moot.
        let nonneg = matches!(self.lo, Bound::Finite(v) if v >= 0);
        let nonpos = matches!(self.hi, Bound::Finite(v) if v <= 0);
        if nonneg {
            return Interval::non_negative();
        }
        if nonpos {
            return Interval { lo: Bound::NegInf, hi: Bound::Finite(0) };
        }
        Interval::top()
    }

    /// Arithmetic right shift `x >> k`. For a constant, in-range amount this is
    /// floor(x / 2^k): monotone non-decreasing in `x`, so both endpoints map
    /// straight through (mirrors `div` by a positive power of two). Keeps
    /// value/element bounds alive across the e-graph's strength reduction
    /// `x / 2^k → x >> k` — e.g. `(seed >> 16) % 1000 ∈ [0, 999]` survives,
    /// exactly as `(seed / 65536) % 1000` did.
    fn shr(&self, other: &Interval) -> Interval {
        if let Some(k) = other.is_exact() {
            if (0..64).contains(&k) {
                let k = k as u32;
                return Interval { lo: self.lo.shr_by(k), hi: self.hi.shr_by(k) };
            }
        }
        Interval::top()
    }

    fn definitely_gt(&self, other: &Interval) -> Option<bool> {
        if self.lo.cmp_bound(&other.hi) == std::cmp::Ordering::Greater {
            return Some(true);
        }
        if self.hi.cmp_bound(&other.lo) != std::cmp::Ordering::Greater {
            return Some(false);
        }
        None
    }

    fn definitely_lt(&self, other: &Interval) -> Option<bool> {
        if self.hi.cmp_bound(&other.lo) == std::cmp::Ordering::Less {
            return Some(true);
        }
        if self.lo.cmp_bound(&other.hi) != std::cmp::Ordering::Less {
            return Some(false);
        }
        None
    }

    fn definitely_gteq(&self, other: &Interval) -> Option<bool> {
        if self.lo.cmp_bound(&other.hi) != std::cmp::Ordering::Less {
            return Some(true);
        }
        if self.hi.cmp_bound(&other.lo) == std::cmp::Ordering::Less {
            return Some(false);
        }
        None
    }

    fn definitely_lteq(&self, other: &Interval) -> Option<bool> {
        if self.hi.cmp_bound(&other.lo) != std::cmp::Ordering::Greater {
            return Some(true);
        }
        if self.lo.cmp_bound(&other.hi) == std::cmp::Ordering::Greater {
            return Some(false);
        }
        None
    }

    fn definitely_eq(&self, other: &Interval) -> Option<bool> {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            return Some(a == b);
        }
        None
    }

    fn definitely_neq(&self, other: &Interval) -> Option<bool> {
        if self.hi.cmp_bound(&other.lo) == std::cmp::Ordering::Less
            || self.lo.cmp_bound(&other.hi) == std::cmp::Ordering::Greater
        {
            return Some(true);
        }
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            return Some(a != b);
        }
        None
    }
}

impl AbstractDomain for Interval {
    fn top() -> Self { Interval::top() }
    fn bottom() -> Self { Interval::bottom() }
    fn join(&self, other: &Self) -> Self { Interval::join(self, other) }
    fn meet(&self, other: &Self) -> Self { Interval::meet(self, other) }
    fn widen(&self, other: &Self) -> Self { Interval::widen(self, other) }
    fn leq(&self, other: &Self) -> bool { Interval::leq(self, other) }
}

/// Concrete runtime type of a value, the atoms of the type lattice. Mirrors the
/// scalar/temporal arms of `RuntimeValue`; aggregate/struct tags are added as the
/// transfer functions that produce them land.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TypeTag {
    Int,
    Float,
    Bool,
    Text,
    Char,
    Nothing,
    Duration,
    Date,
    Moment,
    Span,
    Time,
}

/// The type domain: a value is exactly one type (`Concrete`), one of several
/// (`Union`), unknown (`Top`), or unreachable (`Bottom`). Equality is by the
/// canonical tag-set, so `Union({Int})` and `Concrete(Int)` compare equal.
#[derive(Clone, Debug)]
enum TypeAbstraction {
    Bottom,
    Concrete(TypeTag),
    Union(std::collections::HashSet<TypeTag>),
    Top,
}

impl TypeAbstraction {
    /// Maximum union cardinality before collapsing to `Top` — keeps the lattice
    /// finite-height so `widen` (= `join` here) always converges.
    const UNION_CAP: usize = 6;

    /// Canonical view: `None` is `Top`; `Some(set)` is the set of possible tags
    /// (`Bottom` → empty set, `Concrete(t)` → `{t}`).
    fn tag_set(&self) -> Option<std::collections::HashSet<TypeTag>> {
        match self {
            TypeAbstraction::Top => None,
            TypeAbstraction::Bottom => Some(std::collections::HashSet::new()),
            TypeAbstraction::Concrete(t) => {
                let mut s = std::collections::HashSet::new();
                s.insert(t.clone());
                Some(s)
            }
            TypeAbstraction::Union(s) => Some(s.clone()),
        }
    }

    fn from_tags(s: std::collections::HashSet<TypeTag>) -> Self {
        if s.is_empty() {
            TypeAbstraction::Bottom
        } else if s.len() == 1 {
            TypeAbstraction::Concrete(s.into_iter().next().unwrap())
        } else if s.len() > Self::UNION_CAP {
            TypeAbstraction::Top
        } else {
            TypeAbstraction::Union(s)
        }
    }
}

impl PartialEq for TypeAbstraction {
    fn eq(&self, other: &Self) -> bool {
        self.tag_set() == other.tag_set()
    }
}

impl AbstractDomain for TypeAbstraction {
    fn top() -> Self { TypeAbstraction::Top }
    fn bottom() -> Self { TypeAbstraction::Bottom }

    fn join(&self, other: &Self) -> Self {
        match (self.tag_set(), other.tag_set()) {
            (None, _) | (_, None) => TypeAbstraction::Top,
            (Some(a), Some(b)) => TypeAbstraction::from_tags(a.union(&b).cloned().collect()),
        }
    }

    fn meet(&self, other: &Self) -> Self {
        match (self.tag_set(), other.tag_set()) {
            (None, _) => other.clone(),
            (_, None) => self.clone(),
            (Some(a), Some(b)) => TypeAbstraction::from_tags(a.intersection(&b).cloned().collect()),
        }
    }

    // Finite-height lattice (union capped at UNION_CAP): widening is just join.
    fn widen(&self, other: &Self) -> Self { self.join(other) }

    fn leq(&self, other: &Self) -> bool {
        match (self.tag_set(), other.tag_set()) {
            (_, None) => true,            // everything ⊑ Top
            (None, Some(_)) => false,     // Top ⊑ only Top
            (Some(a), Some(b)) => a.is_subset(&b),
        }
    }
}

fn literal_type(lit: &Literal) -> TypeTag {
    match lit {
        Literal::Number(_) => TypeTag::Int,
        Literal::Float(_) => TypeTag::Float,
        Literal::Text(_) => TypeTag::Text,
        Literal::Boolean(_) => TypeTag::Bool,
        Literal::Nothing => TypeTag::Nothing,
        Literal::Char(_) => TypeTag::Char,
        Literal::Duration(_) => TypeTag::Duration,
        Literal::Date(_) => TypeTag::Date,
        Literal::Moment(_) => TypeTag::Moment,
        Literal::Span { .. } => TypeTag::Span,
        Literal::Time(_) => TypeTag::Time,
    }
}

fn binop_type(op: BinaryOpKind, l: &TypeAbstraction, r: &TypeAbstraction) -> TypeAbstraction {
    use BinaryOpKind::*;
    let conc = |t: TypeTag| TypeAbstraction::Concrete(t);
    let is = |a: &TypeAbstraction, t: TypeTag| matches!(a, TypeAbstraction::Concrete(x) if *x == t);
    match op {
        // Relational and equality always yield Bool.
        Lt | Gt | LtEq | GtEq | Eq | NotEq => conc(TypeTag::Bool),
        // Explicit concatenation is always Text.
        Concat => conc(TypeTag::Text),
        Add => {
            if is(l, TypeTag::Int) && is(r, TypeTag::Int) { conc(TypeTag::Int) }
            else if is(l, TypeTag::Float) && is(r, TypeTag::Float) { conc(TypeTag::Float) }
            else if is(l, TypeTag::Text) || is(r, TypeTag::Text) { conc(TypeTag::Text) }
            else { TypeAbstraction::Top }
        }
        // ExactDivide produces a Rational, never a known Int — `Top` keeps a
        // Rational-derived value out of the integer-only strength reductions.
        ExactDivide => TypeAbstraction::Top,
        Subtract | Multiply | Divide | Modulo => {
            if is(l, TypeTag::Int) && is(r, TypeTag::Int) { conc(TypeTag::Int) }
            else if is(l, TypeTag::Float) && is(r, TypeTag::Float) { conc(TypeTag::Float) }
            else { TypeAbstraction::Top }
        }
        And | Or => {
            if is(l, TypeTag::Bool) && is(r, TypeTag::Bool) { conc(TypeTag::Bool) }
            else if is(l, TypeTag::Int) && is(r, TypeTag::Int) { conc(TypeTag::Int) }
            else { TypeAbstraction::Top }
        }
        BitXor | Shl | Shr => {
            if is(l, TypeTag::Int) && is(r, TypeTag::Int) { conc(TypeTag::Int) }
            else { TypeAbstraction::Top }
        }
    }
}

fn eval_type(
    expr: &Expr,
    types: &HashMap<Symbol, TypeAbstraction>,
    fn_returns: &HashMap<Symbol, TypeTag>,
    elem_type: &HashMap<Symbol, TypeAbstraction>,
) -> TypeAbstraction {
    match expr {
        Expr::Literal(lit) => TypeAbstraction::Concrete(literal_type(lit)),
        Expr::Identifier(sym) => types.get(sym).cloned().unwrap_or(TypeAbstraction::Top),
        Expr::BinaryOp { op, left, right } => {
            let l = eval_type(left, types, fn_returns, elem_type);
            let r = eval_type(right, types, fn_returns, elem_type);
            binop_type(*op, &l, &r)
        }
        Expr::Not { operand } => match eval_type(operand, types, fn_returns, elem_type) {
            TypeAbstraction::Concrete(TypeTag::Bool) => TypeAbstraction::Concrete(TypeTag::Bool),
            TypeAbstraction::Concrete(TypeTag::Int) => TypeAbstraction::Concrete(TypeTag::Int),
            _ => TypeAbstraction::Top,
        },
        Expr::Length { .. } => TypeAbstraction::Concrete(TypeTag::Int),
        Expr::Contains { .. } => TypeAbstraction::Concrete(TypeTag::Bool),
        // A call's DECLARED primitive return type is a sound fact: the
        // kernel enforces it dynamically, native signatures statically.
        Expr::Call { function, .. } => fn_returns
            .get(function)
            .map(|t| TypeAbstraction::Concrete(t.clone()))
            .unwrap_or(TypeAbstraction::Top),
        // A read `item k of arr` carries `arr`'s proven ELEMENT TYPE — the join
        // over every value written into it. Sound because every element IS one
        // of those written values, so they all satisfy the join. Gated OFF by
        // default (`Top`, the prior behaviour) for byte-identical A/B.
        Expr::Index { collection: Expr::Identifier(sym), .. } if elem_type_enabled() => {
            let t = elem_type.get(sym).cloned().unwrap_or(TypeAbstraction::Top);
            if !matches!(t, TypeAbstraction::Top) {
                crate::optimize::mark_fired(crate::optimization::Opt::ElemType);
            }
            t
        }
        _ => TypeAbstraction::Top,
    }
}

/// The collection-size domain — an interval over the (non-negative) length of a
/// collection, named for the cases that drive guard elimination. Equality and
/// the lattice ops work on the canonical `(lo, hi)` size bounds, so e.g.
/// `KnownSize(1)` and `Singleton` are the same element.
#[derive(Clone, Debug)]
enum CollectionShape {
    /// Unreachable — the `⊥` of the lattice.
    Bottom,
    /// length == 0
    Empty,
    /// length == 1
    Singleton,
    /// length == n
    KnownSize(u64),
    /// length ∈ [lo, hi]
    SizeRange(u64, u64),
    /// length >= 1
    NonEmpty,
    /// length >= 0 — the `⊤` of the lattice.
    Top,
}

impl CollectionShape {
    /// A freshly constructed (`a new Seq/Set/Map`) collection is empty.
    fn empty_collection() -> Self { CollectionShape::Empty }

    /// Canonical size bounds `(lo, hi)`; `hi == None` is unbounded above,
    /// `None` overall is `Bottom`.
    fn bounds(&self) -> Option<(u64, Option<u64>)> {
        match self {
            CollectionShape::Bottom => None,
            CollectionShape::Empty => Some((0, Some(0))),
            CollectionShape::Singleton => Some((1, Some(1))),
            CollectionShape::KnownSize(n) => Some((*n, Some(*n))),
            CollectionShape::SizeRange(lo, hi) => Some((*lo, Some(*hi))),
            CollectionShape::NonEmpty => Some((1, None)),
            CollectionShape::Top => Some((0, None)),
        }
    }

    /// Reconstruct the most specific named shape from size bounds. An unbounded
    /// lower bound `>= 2` is sound-but-lossily folded to `NonEmpty`.
    fn from_bounds(lo: u64, hi: Option<u64>) -> Self {
        match hi {
            Some(h) if lo > h => CollectionShape::Bottom,
            Some(0) => CollectionShape::Empty,
            Some(1) if lo == 1 => CollectionShape::Singleton,
            Some(h) if lo == h => CollectionShape::KnownSize(h),
            Some(h) => CollectionShape::SizeRange(lo, h),
            None if lo == 0 => CollectionShape::Top,
            None => CollectionShape::NonEmpty,
        }
    }

    /// Effect of `Push` — size grows by one.
    fn pushed(&self) -> Self {
        match self.bounds() {
            None => CollectionShape::Bottom,
            Some((lo, hi)) => CollectionShape::from_bounds(lo + 1, hi.map(|h| h + 1)),
        }
    }

    /// Effect of `Pop`/`Remove` — size shrinks by one (saturating at 0).
    fn popped(&self) -> Self {
        match self.bounds() {
            None => CollectionShape::Bottom,
            Some((lo, hi)) => {
                CollectionShape::from_bounds(lo.saturating_sub(1), hi.map(|h| h.saturating_sub(1)))
            }
        }
    }

    fn is_definitely_nonempty(&self) -> bool {
        matches!(self.bounds(), Some((lo, _)) if lo >= 1)
    }

    fn is_definitely_empty(&self) -> bool {
        matches!(self.bounds(), Some((_, Some(0))))
    }
}

impl PartialEq for CollectionShape {
    fn eq(&self, other: &Self) -> bool {
        self.bounds() == other.bounds()
    }
}

impl AbstractDomain for CollectionShape {
    fn top() -> Self { CollectionShape::Top }
    fn bottom() -> Self { CollectionShape::Bottom }

    fn join(&self, other: &Self) -> Self {
        match (self.bounds(), other.bounds()) {
            (None, _) => other.clone(),
            (_, None) => self.clone(),
            (Some((l1, h1)), Some((l2, h2))) => {
                let lo = l1.min(l2);
                let hi = match (h1, h2) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
                CollectionShape::from_bounds(lo, hi)
            }
        }
    }

    fn meet(&self, other: &Self) -> Self {
        match (self.bounds(), other.bounds()) {
            (None, _) | (_, None) => CollectionShape::Bottom,
            (Some((l1, h1)), Some((l2, h2))) => {
                let lo = l1.max(l2);
                let hi = match (h1, h2) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };
                CollectionShape::from_bounds(lo, hi)
            }
        }
    }

    fn widen(&self, other: &Self) -> Self {
        match (self.bounds(), other.bounds()) {
            (None, _) => other.clone(),
            (_, None) => self.clone(),
            (Some((l1, h1)), Some((l2, h2))) => {
                let lo = if l2 < l1 { 0 } else { l1 };
                let hi = match (h1, h2) {
                    (Some(a), Some(b)) if b <= a => Some(a),
                    _ => None,
                };
                CollectionShape::from_bounds(lo, hi)
            }
        }
    }

    fn leq(&self, other: &Self) -> bool {
        match (self.bounds(), other.bounds()) {
            (None, _) => true,
            (_, None) => false,
            (Some((l1, h1)), Some((l2, h2))) => {
                let lo_ok = l2 <= l1;
                let hi_ok = match (h1, h2) {
                    (_, None) => true,
                    (None, Some(_)) => false,
                    (Some(a), Some(b)) => a <= b,
                };
                lo_ok && hi_ok
            }
        }
    }
}

/// The nullability domain: is an (optional-typed) value definitely present,
/// definitely `nothing`, or unknown. A diamond lattice
/// `Bottom ⊏ {Definite, Null} ⊏ Maybe`. Used to drop `nothing`-checks the Oracle
/// can prove can never fail.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Nullability {
    /// Unreachable — the `⊥` of the lattice.
    Bottom,
    /// Definitely present (not `nothing`).
    Definite,
    /// Definitely `nothing`.
    Null,
    /// Unknown — the `⊤` of the lattice.
    Maybe,
}

impl Nullability {
    /// The nullability a literal establishes.
    fn for_literal(lit: &Literal) -> Nullability {
        match lit {
            Literal::Nothing => Nullability::Null,
            _ => Nullability::Definite,
        }
    }

    /// Inside a concrete-variant `Inspect` arm (and for its field bindings) the
    /// matched value is present — the fact that retires the unwrap guard.
    fn for_matched_variant() -> Nullability {
        Nullability::Definite
    }
}

impl AbstractDomain for Nullability {
    fn top() -> Self { Nullability::Maybe }
    fn bottom() -> Self { Nullability::Bottom }

    fn join(&self, other: &Self) -> Self {
        use Nullability::*;
        match (self, other) {
            (Bottom, x) | (x, Bottom) => x.clone(),
            (Maybe, _) | (_, Maybe) => Maybe,
            (Definite, Definite) => Definite,
            (Null, Null) => Null,
            (Definite, Null) | (Null, Definite) => Maybe,
        }
    }

    fn meet(&self, other: &Self) -> Self {
        use Nullability::*;
        match (self, other) {
            (Maybe, x) | (x, Maybe) => x.clone(),
            (Bottom, _) | (_, Bottom) => Bottom,
            (Definite, Definite) => Definite,
            (Null, Null) => Null,
            (Definite, Null) | (Null, Definite) => Bottom,
        }
    }

    // Four-element lattice → widening is join.
    fn widen(&self, other: &Self) -> Self { self.join(other) }

    fn leq(&self, other: &Self) -> bool {
        *self == Nullability::Bottom || *other == Nullability::Maybe || self == other
    }
}

/// The aliasing domain (a may-alias set). LOGOS collections are
/// `Rc<RefCell<_>>`, so `Let a be items.` makes `a` and `items` two names for
/// one allocation: a mutation through either must invalidate facts about both.
/// `Unique` (aliases nobody) `⊏ MayAlias(set) ⊏ Top` (aliases anything).
#[derive(Clone, Debug)]
enum AliasInfo {
    /// Unreachable — the `⊥` of the lattice.
    Bottom,
    /// No other name refers to this allocation — may-alias `∅`.
    Unique,
    /// May share its allocation with any of these variables.
    MayAlias(HashSet<Symbol>),
    /// May alias anything — the `⊤` of the lattice.
    Top,
}

/// Canonical three-state view of an `AliasInfo` (`Bottom` / finite may-set / `⊤`).
enum AliasReach {
    Bottom,
    Finite(HashSet<Symbol>),
    Anything,
}

impl AliasInfo {
    fn reach(&self) -> AliasReach {
        match self {
            AliasInfo::Bottom => AliasReach::Bottom,
            AliasInfo::Unique => AliasReach::Finite(HashSet::new()),
            AliasInfo::MayAlias(s) => AliasReach::Finite(s.clone()),
            AliasInfo::Top => AliasReach::Anything,
        }
    }

    fn from_reach(r: AliasReach) -> Self {
        match r {
            AliasReach::Bottom => AliasInfo::Bottom,
            AliasReach::Anything => AliasInfo::Top,
            AliasReach::Finite(s) => {
                if s.is_empty() { AliasInfo::Unique } else { AliasInfo::MayAlias(s) }
            }
        }
    }
}

impl PartialEq for AliasInfo {
    fn eq(&self, other: &Self) -> bool {
        match (self.reach(), other.reach()) {
            (AliasReach::Bottom, AliasReach::Bottom) => true,
            (AliasReach::Anything, AliasReach::Anything) => true,
            (AliasReach::Finite(a), AliasReach::Finite(b)) => a == b,
            _ => false,
        }
    }
}

impl AbstractDomain for AliasInfo {
    fn top() -> Self { AliasInfo::Top }
    fn bottom() -> Self { AliasInfo::Bottom }

    fn join(&self, other: &Self) -> Self {
        match (self.reach(), other.reach()) {
            (AliasReach::Bottom, _) => other.clone(),
            (_, AliasReach::Bottom) => self.clone(),
            (AliasReach::Anything, _) | (_, AliasReach::Anything) => AliasInfo::Top,
            (AliasReach::Finite(a), AliasReach::Finite(b)) => {
                AliasInfo::from_reach(AliasReach::Finite(a.union(&b).cloned().collect()))
            }
        }
    }

    fn meet(&self, other: &Self) -> Self {
        match (self.reach(), other.reach()) {
            (AliasReach::Bottom, _) | (_, AliasReach::Bottom) => AliasInfo::Bottom,
            (AliasReach::Anything, _) => other.clone(),
            (_, AliasReach::Anything) => self.clone(),
            (AliasReach::Finite(a), AliasReach::Finite(b)) => {
                AliasInfo::from_reach(AliasReach::Finite(a.intersection(&b).cloned().collect()))
            }
        }
    }

    // May-alias sets are bounded by the program's variables → widen is join.
    fn widen(&self, other: &Self) -> Self { self.join(other) }

    fn leq(&self, other: &Self) -> bool {
        match (self.reach(), other.reach()) {
            (AliasReach::Bottom, _) => true,
            (_, AliasReach::Anything) => true,
            (AliasReach::Anything, _) => false,
            (_, AliasReach::Bottom) => false,
            (AliasReach::Finite(a), AliasReach::Finite(b)) => a.is_subset(&b),
        }
    }
}

/// The may-alias relation over a scope: an undirected graph whose connected
/// components are sets of variables that share one `Rc<RefCell<_>>`. A mutation
/// through any vertex invalidates abstract facts for its whole component.
///
/// `tainted` marks handles of UNKNOWN PROVENANCE — produced by calls,
/// extracted from containers (Index/Slice/FieldAccess/Pop/Repeat/Inspect
/// bindings), or received as function parameters. A tainted handle may alias
/// anything; only a fresh rebinding clears the mark. The invariant the
/// distinctness query rests on: an UNtainted handle's component is a
/// complete account of its possible aliases.
#[derive(Clone, Default)]
struct AliasGraph {
    edges: HashMap<Symbol, HashSet<Symbol>>,
    tainted: HashSet<Symbol>,
}

impl AliasGraph {
    fn new() -> Self {
        AliasGraph::default()
    }

    /// `Let a be b.` / `Set a to b.` where `b` is an identifier: `a` and `b`
    /// alias the same allocation.
    fn link(&mut self, a: Symbol, b: Symbol) {
        if a == b {
            return;
        }
        self.edges.entry(a).or_default().insert(b);
        self.edges.entry(b).or_default().insert(a);
        // Aliasing a tainted handle spreads its unknown provenance.
        if self.tainted.contains(&a) {
            self.tainted.insert(b);
        }
        if self.tainted.contains(&b) {
            self.tainted.insert(a);
        }
    }

    /// Mark `a` as a handle of unknown provenance.
    fn taint(&mut self, a: Symbol) {
        self.tainted.insert(a);
    }

    /// Union the edges and taint of `other` into `self`; true if anything grew.
    fn union_from(&mut self, other: &AliasGraph) -> bool {
        let mut grew = false;
        for (k, ns) in &other.edges {
            let e = self.edges.entry(*k).or_default();
            for n in ns {
                if e.insert(*n) {
                    grew = true;
                }
            }
        }
        for t in &other.tainted {
            if self.tainted.insert(*t) {
                grew = true;
            }
        }
        grew
    }

    /// PROOF of distinctness: both handles have fully tracked provenance and
    /// their components do not meet. Anything tainted refuses.
    fn definitely_distinct(&self, a: Symbol, b: Symbol) -> bool {
        if a == b || self.tainted.contains(&a) || self.tainted.contains(&b) {
            return false;
        }
        !self.may_alias(a).contains(&b)
    }

    /// Every variable that may alias `v` (its connected component, including `v`).
    fn may_alias(&self, v: Symbol) -> HashSet<Symbol> {
        let mut seen = HashSet::new();
        let mut stack = vec![v];
        while let Some(x) = stack.pop() {
            if seen.insert(x) {
                if let Some(ns) = self.edges.get(&x) {
                    for &n in ns {
                        if !seen.contains(&n) {
                            stack.push(n);
                        }
                    }
                }
            }
        }
        seen
    }

    /// The facts to invalidate when a mutation flows through `v`.
    fn invalidated_by_mutation(&self, v: Symbol) -> HashSet<Symbol> {
        self.may_alias(v)
    }

    /// Rebinding `a` to a fresh allocation severs its old alias edges and
    /// clears its taint: the new value's provenance is known again.
    fn unlink(&mut self, a: Symbol) {
        if let Some(ns) = self.edges.remove(&a) {
            for n in ns {
                if let Some(s) = self.edges.get_mut(&n) {
                    s.remove(&a);
                }
            }
        }
        self.tainted.remove(&a);
    }
}

#[derive(Clone)]
struct AbstractState {
    vars: HashMap<Symbol, Interval>,
    lengths: HashMap<Symbol, Interval>,
    /// Per-collection ELEMENT interval: a bound satisfied by EVERY element of
    /// the collection, joined over the values written into it. Lets a read
    /// `item k of arr` carry a value range (`(seed/65536) % 1000 ∈ [0,999]`
    /// pushed into `arr` ⟹ `item i of arr ∈ [0,999]`), which in turn proves a
    /// scatter index `counts[v+1]` in bounds. `None`/absent = unknown (`top`).
    elem: HashMap<Symbol, Interval>,
}

impl AbstractState {
    fn new() -> Self {
        AbstractState {
            vars: HashMap::new(),
            lengths: HashMap::new(),
            elem: HashMap::new(),
        }
    }

    fn get_var(&self, sym: &Symbol) -> Interval {
        self.vars.get(sym).cloned().unwrap_or(Interval::top())
    }

    fn set_var(&mut self, sym: Symbol, range: Interval) {
        self.vars.insert(sym, range);
    }

    fn get_length(&self, sym: &Symbol) -> Interval {
        self.lengths.get(sym).cloned().unwrap_or(Interval::non_negative())
    }

    fn set_length(&mut self, sym: Symbol, range: Interval) {
        self.lengths.insert(sym, range);
    }

    /// The proven element interval of a collection, or `top` if unknown.
    fn get_elem(&self, sym: &Symbol) -> Interval {
        self.elem.get(sym).cloned().unwrap_or(Interval::top())
    }

    /// Record an element written into `sym`: JOIN with the existing element
    /// bound (every element must satisfy the union). Absent means "no element
    /// yet" — the first write seeds it exactly.
    fn observe_elem(&mut self, sym: Symbol, range: Interval) {
        let joined = match self.elem.get(&sym) {
            Some(existing) => existing.join(&range),
            None => range,
        };
        self.elem.insert(sym, joined);
    }

    /// Forget the element bound (collection rebound, aliased, or written with
    /// an unknown value).
    fn clear_elem(&mut self, sym: &Symbol) {
        self.elem.remove(sym);
    }
}

fn eval_expr(expr: &Expr, state: &AbstractState) -> Interval {
    match expr {
        Expr::Literal(Literal::Number(n)) => Interval::exact(*n),
        Expr::Literal(Literal::Boolean(_)) => Interval::top(),
        Expr::Literal(Literal::Float(_)) => Interval::top(),
        Expr::Identifier(sym) => state.get_var(sym),
        Expr::BinaryOp { op, left, right } => {
            let l = eval_expr(left, state);
            let r = eval_expr(right, state);
            match op {
                BinaryOpKind::Add => l.add(&r),
                BinaryOpKind::Subtract => l.sub(&r),
                BinaryOpKind::Multiply => l.mul(&r),
                BinaryOpKind::Divide => l.div(&r),
                // ExactDivide yields a Rational — it has NO integer interval, so report
                // `top`. This is also the defense that keeps a Rational-derived value
                // from being "proven Int": the integer-only division strength reductions
                // (MagicDivU / DivPow2) require a proven non-negative Int and so never
                // misfire on it.
                BinaryOpKind::ExactDivide => Interval::top(),
                BinaryOpKind::Modulo => l.modulo(&r),
                BinaryOpKind::Shr => l.shr(&r),
                _ => Interval::top(),
            }
        }
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                state.get_length(sym)
            } else {
                Interval::non_negative()
            }
        }
        // A read `item k of arr` carries `arr`'s proven ELEMENT interval — the
        // value-range-through-memory step that turns an element into a usable
        // scatter index (`item i of arr ∈ [0,999]` ⟹ `counts[that + 1]` in
        // bounds). Unknown element bound ⟹ `top`, exactly as before.
        Expr::Index { collection, .. } => {
            if let Expr::Identifier(sym) = collection {
                state.get_elem(sym)
            } else {
                Interval::top()
            }
        }
        _ => Interval::top(),
    }
}

fn eval_condition(cond: &Expr, state: &AbstractState) -> Option<bool> {
    match cond {
        Expr::Literal(Literal::Boolean(b)) => Some(*b),
        Expr::BinaryOp { op, left, right } => {
            let l = eval_expr(left, state);
            let r = eval_expr(right, state);
            match op {
                BinaryOpKind::Gt => l.definitely_gt(&r),
                BinaryOpKind::Lt => l.definitely_lt(&r),
                BinaryOpKind::GtEq => l.definitely_gteq(&r),
                BinaryOpKind::LtEq => l.definitely_lteq(&r),
                BinaryOpKind::Eq => l.definitely_eq(&r),
                BinaryOpKind::NotEq => l.definitely_neq(&r),
                _ => None,
            }
        }
        Expr::Not { operand } => eval_condition(operand, state).map(|b| !b),
        _ => None,
    }
}

fn narrow_state(cond: &Expr, state: &mut AbstractState) {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::Gt, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    if let Some(new_lo) = n.checked_add(1) {
                        state.set_var(*sym, Interval {
                            lo: Bound::max_bound(&cur.lo, &Bound::Finite(new_lo)),
                            hi: cur.hi,
                        });
                    }
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    if let Some(new_hi) = n.checked_sub(1) {
                        state.set_var(*sym, Interval {
                            lo: cur.lo,
                            hi: Bound::min_bound(&cur.hi, &Bound::Finite(new_hi)),
                        });
                    }
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::GtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    state.set_var(*sym, Interval {
                        lo: Bound::max_bound(&cur.lo, &Bound::Finite(n)),
                        hi: cur.hi,
                    });
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    state.set_var(*sym, Interval {
                        lo: cur.lo,
                        hi: Bound::min_bound(&cur.hi, &Bound::Finite(n)),
                    });
                }
            }
        }
        _ => {}
    }
}

fn narrow_state_negated(cond: &Expr, state: &mut AbstractState) {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::Gt, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    state.set_var(*sym, Interval {
                        lo: cur.lo,
                        hi: Bound::min_bound(&cur.hi, &Bound::Finite(n)),
                    });
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    state.set_var(*sym, Interval {
                        lo: Bound::max_bound(&cur.lo, &Bound::Finite(n)),
                        hi: cur.hi,
                    });
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::GtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    if let Some(new_hi) = n.checked_sub(1) {
                        state.set_var(*sym, Interval {
                            lo: cur.lo,
                            hi: Bound::min_bound(&cur.hi, &Bound::Finite(new_hi)),
                        });
                    }
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    if let Some(new_lo) = n.checked_add(1) {
                        state.set_var(*sym, Interval {
                            lo: Bound::max_bound(&cur.lo, &Bound::Finite(new_lo)),
                            hi: cur.hi,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

pub fn abstract_interp_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut state = AbstractState::new();
    interp_block(stmts, &mut state, expr_arena, stmt_arena)
}

fn interp_block<'a>(
    stmts: Vec<Stmt<'a>>,
    state: &mut AbstractState,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::Let { var, ty, value, mutable } => {
                let range = eval_expr(value, state);
                state.set_var(var, range);
                if matches!(value, Expr::New { .. }) {
                    state.set_length(var, Interval::exact(0));
                }
                result.push(Stmt::Let { var, ty, value, mutable });
            }

            Stmt::Set { target, value } => {
                let range = eval_expr(value, state);
                state.set_var(target, range);
                result.push(Stmt::Set { target, value });
            }

            Stmt::Push { value, collection } => {
                if let Expr::Identifier(sym) = collection {
                    let cur_len = state.get_length(sym);
                    state.set_length(*sym, cur_len.add(&Interval::exact(1)));
                }
                result.push(Stmt::Push { value, collection });
            }

            Stmt::If { cond, then_block, else_block } => {
                if let Some(val) = eval_condition(cond, state) {
                    let new_cond = expr_arena.alloc(Expr::Literal(Literal::Boolean(val)));
                    if val {
                        let mut then_state = state.clone();
                        narrow_state(cond, &mut then_state);
                        let new_then = interp_nested_block(then_block, &mut then_state, expr_arena, stmt_arena);
                        *state = then_state;
                        result.push(Stmt::If {
                            cond: new_cond,
                            then_block: new_then,
                            else_block: None,
                        });
                    } else {
                        if let Some(eb) = else_block {
                            let mut else_state = state.clone();
                            narrow_state_negated(cond, &mut else_state);
                            let new_else = interp_nested_block(eb, &mut else_state, expr_arena, stmt_arena);
                            *state = else_state;
                            result.push(Stmt::If {
                                cond: new_cond,
                                then_block: stmt_arena.alloc_slice(vec![]),
                                else_block: Some(new_else),
                            });
                        } else {
                            result.push(Stmt::If {
                                cond: new_cond,
                                then_block: stmt_arena.alloc_slice(vec![]),
                                else_block: None,
                            });
                        }
                    }
                } else {
                    let mut then_state = state.clone();
                    narrow_state(cond, &mut then_state);
                    let new_then = interp_nested_block(then_block, &mut then_state, expr_arena, stmt_arena);

                    let (new_else, else_state) = if let Some(eb) = else_block {
                        let mut es = state.clone();
                        narrow_state_negated(cond, &mut es);
                        let ne = interp_nested_block(eb, &mut es, expr_arena, stmt_arena);
                        (Some(ne), Some(es))
                    } else {
                        (None, None)
                    };

                    if let Some(es) = else_state {
                        join_states(state, &then_state, &es);
                    } else {
                        let orig = state.clone();
                        join_states(state, &then_state, &orig);
                    }

                    result.push(Stmt::If { cond, then_block: new_then, else_block: new_else });
                }
            }

            Stmt::While { cond, body, decreasing } => {
                let mut loop_state = state.clone();

                let loop_writes = collect_writes(body);
                let bounded_var = extract_bounded_var(cond);

                // Widen all loop-written variables (including the counter)
                // to their full possible range before analyzing the body.
                for w in &loop_writes {
                    loop_state.set_var(*w, Interval::top());
                }

                // Now narrow the loop state based on the condition.
                // For the bounded variable (counter), this gives it the range
                // from its initial value (widened to top above) narrowed by the
                // condition, resulting in [-inf, bound].
                narrow_state(cond, &mut loop_state);

                let new_body = interp_nested_block(body, &mut loop_state, expr_arena, stmt_arena);

                // After loop: condition is false (loop exited)
                narrow_state_negated(cond, state);
                // Variables written in loop body get widened to top
                for w in &loop_writes {
                    if Some(*w) != bounded_var {
                        state.set_var(*w, Interval::top());
                    }
                }

                result.push(Stmt::While { cond, body: new_body, decreasing });
            }

            Stmt::Repeat { pattern, iterable, body } => {
                let mut loop_state = state.clone();

                if let Pattern::Identifier(var) = &pattern {
                    loop_state.set_var(*var, Interval::top());
                }

                let loop_writes = collect_writes(body);
                for w in &loop_writes {
                    loop_state.set_var(*w, Interval::top());
                }

                let new_body = interp_nested_block(body, &mut loop_state, expr_arena, stmt_arena);

                for w in &loop_writes {
                    state.set_var(*w, Interval::top());
                }

                result.push(Stmt::Repeat { pattern, iterable, body: new_body });
            }

            Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
                let mut func_state = AbstractState::new();
                let new_body = interp_nested_block(body, &mut func_state, expr_arena, stmt_arena);
                result.push(Stmt::FunctionDef {
                    name, params, generics,
                    body: new_body,
                    return_type, is_native, native_path, is_exported, export_target, opt_flags,
                });
            }

            Stmt::Inspect { target, arms, has_otherwise } => {
                let new_arms: Vec<_> = arms.into_iter().map(|arm| {
                    let mut arm_state = state.clone();
                    let new_body = interp_nested_block(arm.body, &mut arm_state, expr_arena, stmt_arena);
                    crate::ast::stmt::MatchArm {
                        enum_name: arm.enum_name,
                        variant: arm.variant,
                        bindings: arm.bindings,
                        body: new_body,
                    }
                }).collect();
                result.push(Stmt::Inspect { target, arms: new_arms, has_otherwise });
            }

            Stmt::Zone { .. } => {
                // Don't analyze inside zones — zone-scoped bindings must be
                // preserved for escape analysis (same as propagation pass).
                result.push(stmt);
            }

            Stmt::Concurrent { tasks } => {
                let mut sub_state = state.clone();
                let new_tasks = interp_nested_block(tasks, &mut sub_state, expr_arena, stmt_arena);
                result.push(Stmt::Concurrent { tasks: new_tasks });
            }

            Stmt::Parallel { tasks } => {
                let mut sub_state = state.clone();
                let new_tasks = interp_nested_block(tasks, &mut sub_state, expr_arena, stmt_arena);
                result.push(Stmt::Parallel { tasks: new_tasks });
            }

            other => {
                result.push(other);
            }
        }
    }

    result
}

fn interp_nested_block<'a>(
    block: &'a [Stmt<'a>],
    state: &mut AbstractState,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> &'a [Stmt<'a>] {
    let stmts: Vec<Stmt<'a>> = block.iter().cloned().collect();
    let result = interp_block(stmts, state, expr_arena, stmt_arena);
    stmt_arena.alloc_slice(result)
}

fn join_states(out: &mut AbstractState, a: &AbstractState, b: &AbstractState) {
    let mut all_keys: std::collections::HashSet<Symbol> = a.vars.keys().cloned().collect();
    all_keys.extend(b.vars.keys().cloned());

    for key in all_keys {
        let a_range = a.vars.get(&key).cloned().unwrap_or(Interval::top());
        let b_range = b.vars.get(&key).cloned().unwrap_or(Interval::top());
        out.set_var(key, a_range.join(&b_range));
    }

    let mut len_keys: std::collections::HashSet<Symbol> = a.lengths.keys().cloned().collect();
    len_keys.extend(b.lengths.keys().cloned());

    for key in len_keys {
        let a_len = a.lengths.get(&key).cloned().unwrap_or(Interval::non_negative());
        let b_len = b.lengths.get(&key).cloned().unwrap_or(Interval::non_negative());
        out.set_length(key, a_len.join(&b_len));
    }

    // Element bounds: absent means UNKNOWN (`top`), so a key tracked on only
    // one path joins to `top` (sound — the other path could hold anything). An
    // EMPTY fresh collection is seeded `⊥` (no elements), so the 0-iteration
    // branch of a build loop's exit join preserves the filled branch's bound.
    let mut el_keys: std::collections::HashSet<Symbol> = a.elem.keys().cloned().collect();
    el_keys.extend(b.elem.keys().cloned());
    for key in el_keys {
        let a_el = a.get_elem(&key);
        let b_el = b.get_elem(&key);
        out.elem.insert(key, a_el.join(&b_el));
    }
}

fn collect_writes(block: &[Stmt]) -> Vec<Symbol> {
    let mut writes = Vec::new();
    for stmt in block {
        collect_writes_stmt(stmt, &mut writes);
    }
    writes
}

fn collect_writes_stmt(stmt: &Stmt, writes: &mut Vec<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => {
            if !writes.contains(target) {
                writes.push(*target);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block { collect_writes_stmt(s, writes); }
            if let Some(eb) = else_block {
                for s in *eb { collect_writes_stmt(s, writes); }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            for s in *body { collect_writes_stmt(s, writes); }
        }
        _ => {}
    }
}

fn extract_bounded_var(cond: &Expr) -> Option<Symbol> {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::Lt | BinaryOpKind::LtEq | BinaryOpKind::Gt | BinaryOpKind::GtEq, left, .. } => {
            if let Expr::Identifier(sym) = left {
                Some(*sym)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ============================================================================
// The Oracle: product lattice + rich, fact-returning analysis (EXODIA Phase 1).
//
// `abstract_interp_stmts` above transforms the program (interval-driven dead
// branch elimination). `rich_abstract_interp_stmts` below is additive: it leaves
// the program unchanged and hands back the five-domain abstract facts the VM and
// copy-and-patch JIT consult to choose typed stencils and drop guards.
// ============================================================================

/// The product abstract value — the five domains combined componentwise.
#[derive(Clone, Debug)]
pub(crate) struct AbstractValue {
    interval: Interval,
    ty: TypeAbstraction,
    shape: CollectionShape,
    nullability: Nullability,
    alias: AliasInfo,
}

impl AbstractDomain for AbstractValue {
    fn top() -> Self {
        AbstractValue {
            interval: Interval::top(),
            ty: TypeAbstraction::Top,
            shape: CollectionShape::Top,
            nullability: Nullability::Maybe,
            alias: AliasInfo::Top,
        }
    }

    fn bottom() -> Self {
        AbstractValue {
            interval: Interval::bottom(),
            ty: TypeAbstraction::Bottom,
            shape: CollectionShape::Bottom,
            nullability: Nullability::Bottom,
            alias: AliasInfo::Bottom,
        }
    }

    fn join(&self, o: &Self) -> Self {
        AbstractValue {
            interval: self.interval.join(&o.interval),
            ty: self.ty.join(&o.ty),
            shape: self.shape.join(&o.shape),
            nullability: self.nullability.join(&o.nullability),
            alias: self.alias.join(&o.alias),
        }
    }

    fn meet(&self, o: &Self) -> Self {
        AbstractValue {
            interval: self.interval.meet(&o.interval),
            ty: self.ty.meet(&o.ty),
            shape: self.shape.meet(&o.shape),
            nullability: self.nullability.meet(&o.nullability),
            alias: self.alias.meet(&o.alias),
        }
    }

    fn widen(&self, o: &Self) -> Self {
        AbstractValue {
            interval: self.interval.widen(&o.interval),
            ty: self.ty.widen(&o.ty),
            shape: self.shape.widen(&o.shape),
            nullability: self.nullability.widen(&o.nullability),
            alias: self.alias.widen(&o.alias),
        }
    }

    fn leq(&self, o: &Self) -> bool {
        self.interval.leq(&o.interval)
            && self.ty.leq(&o.ty)
            && self.shape.leq(&o.shape)
            && self.nullability.leq(&o.nullability)
            && self.alias.leq(&o.alias)
    }
}

/// The Oracle's per-program-point state: every domain tracked per variable, plus
/// the may-alias relation. `value_of` assembles the product fact on demand.
#[derive(Clone)]
pub(crate) struct RichAbstractState {
    intervals: AbstractState,
    types: HashMap<Symbol, TypeAbstraction>,
    shapes: HashMap<Symbol, CollectionShape>,
    nullability: HashMap<Symbol, Nullability>,
    aliases: AliasGraph,
    /// Variables PROVEN collection-kinded (created as collections). Kind
    /// is binding-level: pushes, pops and callees resize a collection but
    /// can never rebind it to a scalar — only reassignment removes a
    /// variable here. Size widening (shapes → Top) does NOT erase kind.
    coll_vars: std::collections::HashSet<Symbol>,
    /// DECLARED primitive return types per function — calls produce facts.
    fn_returns: std::rc::Rc<HashMap<Symbol, TypeTag>>,
    /// Symbolic length LOWER bound per collection: `arr -> (n, off)` means
    /// `length(arr) >= n + off`, where `n` is a program variable. Established
    /// by a counted build loop (`while c < n: push to arr` once per iteration
    /// from an empty array — the standard allocation-size fact) and consumed
    /// by the relational bounds recognizer to prove `item i of arr` reads
    /// whose guard bounds `i` by the SAME variable `n`. Invalidated when the
    /// array is resized/rebound or when `n` is reassigned.
    length_def: HashMap<Symbol, (Symbol, i64)>,
    /// Function-PARAMETER collections (`arr: Seq of Int`): their length is
    /// fundamentally unknown, so they are the targets of SPECULATIVE
    /// region-entry hoisting (a locally-built array, by contrast, has a
    /// statically determinable length and is left to the static path).
    param_colls: std::collections::HashSet<Symbol>,
    /// Affine scalar DEFINITIONS: `X -> linear(E)` for a binding `Let X be E`
    /// whose value is affine over other variables (`Let cols be capacity + 1`).
    /// Fed to the kernel LIA prover as an EQUALITY `X = E` so a multi-variable
    /// bounds proof can relate a length variable to a loop bound (knapsack:
    /// `length(prev) = cols = capacity + 1`, guard `w <= capacity`). Invalidated
    /// when `X` or any variable in `E` is reassigned.
    scalar_def: HashMap<Symbol, super::affine::LinExpr>,
    /// SYMBOLIC scalar UPPER bound: `X -> L` means `X <= L` for a linear `L`
    /// over program variables. The variable-divisor sibling of the concrete
    /// interval upper — `Let neighbor be (...) % n` records `neighbor <= n - 1`,
    /// which no concrete `Interval` can hold (`n` is symbolic). Flows into the
    /// per-collection element upper through a store and back out through a read,
    /// then feeds the Fourier–Motzkin prover so `dist[u]` (u an element of a
    /// `% n`-filled array) proves in bounds. Invalidated like `scalar_def`.
    scalar_upper: HashMap<Symbol, super::affine::LinExpr>,
    /// SYMBOLIC per-collection ELEMENT upper bound: `A -> L` means EVERY element
    /// of `A` is `<= L`. Absent means ⊤ (unknown) when the concrete element
    /// interval is non-⊥, or ⊥ (fresh, no elements) when it is — the concrete
    /// `is_bottom()` disambiguates, exactly as the fixpoint's freshness test
    /// does. Seeded by stores/pushes whose value carries a `scalar_upper`, read
    /// back as a `scalar_upper` on `Let u be item _ of A`.
    elem_upper: HashMap<Symbol, super::affine::LinExpr>,
    /// Per-collection ELEMENT TYPE: `A -> T` means EVERY element of `A` is of
    /// scalar type `T` (the join over every value written into `A`). The type
    /// sibling of the concrete `elem` interval — seeded by the first
    /// store/push, joined on every later one, and read back as the proven type
    /// of `item k of A`. Lets a read of a homogeneously-typed collection carry
    /// a concrete scalar kind (`item k of dp ∈ Int` when `dp` is built only
    /// from `Int` writes), which the magic-reciprocal modulo gate needs to fire
    /// on `(... + ...) % c`. Absent means ⊤ (unknown) when the concrete `elem`
    /// interval is non-⊥, or ⊥ (fresh, no elements) when it is — the same
    /// `is_bottom()` freshness test the `elem`/`elem_upper` siblings use.
    elem_type: HashMap<Symbol, TypeAbstraction>,
    /// SYMBOLIC scalar LOWER bound — the mirror of `scalar_upper`, the relation
    /// that makes `dist[u+1]`'s LOWER half (`u >= 0`) provable when `u`'s raw
    /// interval is widened to ⊤ (a loop-local read variable's undefined entry
    /// value floods the fixpoint join). Captured at bind time from the source's
    /// proven lower — an element read of `A` carries `A`'s concrete element
    /// interval lower; a `(...) % n` of a non-negative dividend carries `0`.
    /// Sound for a MUTATED scalar (re-established by its in-body `Let`).
    scalar_lower: HashMap<Symbol, super::affine::LinExpr>,
    /// AOT-only: this state belongs to a function body analyzed for the
    /// `largo build` codegen, which emits the recursive-1-based-partition entry
    /// precondition guard (`assert!(lo >= 1 && hi <= len)`). Only then is it
    /// sound to (a) seed that precondition as a fact and (b) propagate a
    /// `length_def` across an alias bind — both rest on a runtime check the VM/
    /// JIT bytecode path does NOT emit, so they stay OFF for it. Set by
    /// [`oracle_analyze_with_entry_guards`]; copied by `clone()` into branch
    /// states, so it survives the whole body walk.
    aot_entry_guard: bool,
}

impl RichAbstractState {
    fn new() -> Self {
        RichAbstractState {
            intervals: AbstractState::new(),
            types: HashMap::new(),
            shapes: HashMap::new(),
            nullability: HashMap::new(),
            aliases: AliasGraph::new(),
            coll_vars: std::collections::HashSet::new(),
            fn_returns: std::rc::Rc::new(HashMap::new()),
            length_def: HashMap::new(),
            param_colls: std::collections::HashSet::new(),
            scalar_def: HashMap::new(),
            scalar_upper: HashMap::new(),
            elem_upper: HashMap::new(),
            elem_type: HashMap::new(),
            scalar_lower: HashMap::new(),
            aot_entry_guard: false,
        }
    }

    /// The product fact known about `sym` at this point.
    pub(crate) fn value_of(&self, sym: Symbol) -> AbstractValue {
        let mut others = self.aliases.may_alias(sym);
        others.remove(&sym);
        let alias = if others.is_empty() {
            AliasInfo::Unique
        } else {
            AliasInfo::MayAlias(others)
        };
        AbstractValue {
            interval: self.intervals.get_var(&sym),
            ty: self.types.get(&sym).cloned().unwrap_or(TypeAbstraction::Top),
            shape: self.shapes.get(&sym).cloned().unwrap_or(CollectionShape::Top),
            nullability: self.nullability.get(&sym).cloned().unwrap_or(Nullability::Maybe),
            alias,
        }
    }

    /// Forget every scalar fact about `sym` (used when its value is overwritten
    /// or produced by an operation we do not model). Rebinding can change
    /// KIND too, so the collection proof drops with it.
    fn invalidate_var(&mut self, sym: Symbol) {
        self.intervals.set_var(sym, Interval::top());
        self.types.insert(sym, TypeAbstraction::Top);
        self.shapes.insert(sym, CollectionShape::Top);
        self.nullability.insert(sym, Nullability::Maybe);
        self.coll_vars.remove(&sym);
        self.intervals.clear_elem(&sym);
        let si = sym.index() as i64;
        self.scalar_upper.remove(&sym);
        self.scalar_upper.retain(|_, e| !e.coefficients.contains_key(&si));
        self.elem_upper.remove(&sym);
        self.elem_upper.retain(|_, e| !e.coefficients.contains_key(&si));
        self.elem_type.remove(&sym);
        self.scalar_lower.remove(&sym);
        self.scalar_lower.retain(|_, e| !e.coefficients.contains_key(&si));
    }
}

/// Run the Oracle's rich abstract interpretation, returning the (unchanged)
/// statements together with the per-variable abstract facts.
/// The scalar kinds downstream consumers (typed bytecode, JIT guard
/// elision) act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarKind {
    Int,
    Float,
    Bool,
    Text,
}

/// EXODIA Phase 1's DELIVERY layer: a product fact for every expression
/// occurrence at its program point, keyed by ARENA ADDRESS (stable for the
/// analyzed snapshot — record facts immediately before consuming them, and
/// never across a pass that re-allocates expressions). Shared subtrees that
/// appear at several program points JOIN their facts (sound; precision is
/// lost exactly where sharing occurs).
#[derive(Default)]
pub struct OracleFacts {
    exprs: HashMap<usize, AbstractValue>,
    /// Length interval of the collection an Identifier expression denotes,
    /// at that occurrence — the bounds-elision query's other half.
    lengths: HashMap<usize, Interval>,
    /// Identifier occurrences PROVEN collection-kinded (binding-level —
    /// kind survives growth and callees; only rebinding erases it).
    collections: std::collections::HashSet<usize>,
    /// Converged loop-invariant alias graphs, keyed by the While/Repeat
    /// statement's arena address. Borrow hoisting's distinctness queries
    /// read these; loops the fixpoint never converged on (or that run under
    /// concurrent blocks) have no entry and refuse.
    loop_aliases: HashMap<usize, AliasGraph>,
    /// When set, per-expression facts (`exprs`/`lengths`/`collections`) are
    /// NOT recorded, but state updates and `loop_aliases` snapshots still
    /// are. Used to walk Zone interiors for borrow hoisting's alias
    /// snapshots WITHOUT feeding new expr facts to the EXODIA VM/JIT region
    /// compiler — whose pinned-chain emitter assumes the oracle records
    /// nothing inside zones.
    suppress_exprs: bool,
    /// `index` sub-expression addresses of `item i of arr` reads PROVEN
    /// in bounds by RELATIONAL induction-variable reasoning (V8/LLVM SCEV
    /// style): a loop guard `i </<= length(arr)` with `i >= 1` and `arr`
    /// not resized in the body bounds the index by the array's length — a
    /// relation the interval domain cannot represent. `index_provably_in_
    /// bounds` consults this in addition to the interval check.
    relational_inbounds: std::collections::HashSet<usize>,
    /// `index` addresses proven in bounds only SPECULATIVELY — they rely on a
    /// region-entry runtime check (`hoist_descs`). Kept SEPARATE from
    /// `relational_inbounds` so the compiler elides them ONLY when it actually
    /// emits the matching `RegionBoundsGuard` (elision ⟺ guard ⟺ VM check).
    speculative_inbounds: std::collections::HashSet<usize>,
    /// Per-loop region-entry bounds hoists (V8 loop bound-check elimination),
    /// keyed by the `While`/`Repeat` statement's arena address. When a loop's
    /// array length cannot be proven statically (e.g. a function parameter),
    /// but the induction is monotone, the bound loop-invariant, and the array
    /// stable, the covered accesses are recorded in `relational_inbounds`
    /// (speculatively) and one descriptor here justifies them with a runtime
    /// check the bytecode compiler lowers to `RegionBoundsGuard`.
    hoist_descs: HashMap<usize, Vec<HoistDesc>>,
    /// Per-access POSITIVITY guards: an `index` (by `*const Expr`) elided only
    /// because a `% m` element bound `m - 1` closed the proof carries the
    /// divisors `m` whose `m >= 1` precondition the codegen must discharge with
    /// a (nonemptiness-guarded, invariant-hoisted) `assert!(m > 0)` at the
    /// access. Keeps the lenient symbolic element join sound for ALL inputs:
    /// `m <= 0` panics before the would-be out-of-bounds read instead of UB.
    positivity_guards: HashMap<usize, Vec<Symbol>>,
    /// `Map of Int to Int` locals declared `… with capacity CAP` whose capacity is
    /// an affine expression of program-INVARIANT variables — stored as a kernel
    /// `LinearExpr` so the dense-key bound proof can relate a key to it. Keyed by
    /// the map symbol. The invariance check (no capacity variable is ever
    /// reassigned) is what makes the stored expression equal the runtime
    /// allocation size at every key site. Populated once, before the loop walks.
    map_caps: HashMap<Symbol, super::affine::LinExpr>,
    /// `key` sub-expression addresses (insert/get/contains) PROVEN to satisfy
    /// `0 <= key <= capacity(map)` under the loop guards, mapped to the map they
    /// were proven against. The dense gate lowers a `Map of Int to Int` to a
    /// direct-addressed array iff EVERY one of its key sites appears here for it.
    dense_map_key_proven: HashMap<usize, Symbol>,
    /// `Map of Int to Int` locals whose SINGLE insert loop provably writes EVERY
    /// integer in a contiguous range `[A, B]` (unit-stride induction variable,
    /// unconditional `Set item iv of m`), keyed to that range as `(A, B)` kernel
    /// `LinearExpr`s. The precondition for presence elision: a key proven inside
    /// `[A, B]` was definitely inserted, so `get` can skip its presence check.
    map_insert_cover: HashMap<Symbol, (super::affine::LinExpr, super::affine::LinExpr)>,
    /// `key` addresses additionally PROVEN inside their map's fully-covered insert
    /// range `[A, B]` (so the key was definitely written). When every key site of
    /// a dense map is here, the map needs no presence bitset (`…NoPresence`).
    map_key_covered: HashMap<usize, Symbol>,
}

/// A region-entry bounds hoist in SYMBOL terms (the compiler maps to
/// registers): `length(array) >= bound + add_max` and `iv + add_min >= 1`.
#[derive(Clone, Copy, Debug)]
pub struct HoistDesc {
    pub array: Symbol,
    pub bound: Symbol,
    pub iv: Symbol,
    pub add_max: i32,
    pub add_min: i32,
}

impl OracleFacts {
    /// Region-entry bounds hoists for the loop at `loop_ptr` (the `While`/
    /// `Repeat` statement's arena address). The compiler lowers each to a
    /// `RegionBoundsGuard` at the loop head.
    pub fn hoist_descs_for(&self, loop_ptr: usize) -> &[HoistDesc] {
        self.hoist_descs.get(&loop_ptr).map_or(&[], |v| v.as_slice())
    }

    /// Is this `index` sub-expression proven in bounds only SPECULATIVELY
    /// (needs a region-entry guard)? The compiler elides it solely when it
    /// emitted that guard.
    pub fn index_is_speculative(&self, index: &Expr) -> bool {
        self.speculative_inbounds.contains(&(index as *const Expr as usize))
    }

    fn record(&mut self, e: &Expr, av: AbstractValue) {
        if self.suppress_exprs {
            return;
        }
        let key = e as *const Expr as usize;
        match self.exprs.get_mut(&key) {
            None => {
                self.exprs.insert(key, av);
            }
            Some(prev) => *prev = prev.join(&av),
        }
    }

    fn record_length(&mut self, e: &Expr, len: Interval) {
        if self.suppress_exprs {
            return;
        }
        let key = e as *const Expr as usize;
        match self.lengths.get_mut(&key) {
            None => {
                self.lengths.insert(key, len);
            }
            Some(prev) => *prev = prev.join(&len),
        }
    }

    /// The proven scalar kind of this expression occurrence, if concrete.
    pub fn expr_scalar(&self, e: &Expr) -> Option<ScalarKind> {
        let av = self.exprs.get(&(e as *const Expr as usize))?;
        match &av.ty {
            TypeAbstraction::Concrete(TypeTag::Int) => Some(ScalarKind::Int),
            TypeAbstraction::Concrete(TypeTag::Float) => Some(ScalarKind::Float),
            TypeAbstraction::Concrete(TypeTag::Bool) => Some(ScalarKind::Bool),
            TypeAbstraction::Concrete(TypeTag::Text) => Some(ScalarKind::Text),
            _ => None,
        }
    }

    /// The finite integer range proven for this occurrence, if both bounds
    /// are finite.
    pub fn expr_int_range(&self, e: &Expr) -> Option<(i64, i64)> {
        self.expr_int_range_addr(e as *const Expr as usize)
    }

    /// Is this occurrence PROVEN non-negative — its abstract interval's LOWER
    /// bound is a finite `>= 0`? Unlike [`Self::expr_int_range`] this ignores
    /// the upper bound, so it holds even when the value's magnitude is unbounded
    /// above (the LCG `seed * 1103515245 + 12345` dividend: both factors
    /// non-negative, so `lo = 0`, but the product's `hi` may saturate). This is
    /// the soundness gate for `x % 2^k → x & (2^k - 1)`, whose identity needs
    /// only `x >= 0`, not a finite ceiling.
    pub fn expr_proven_nonneg(&self, e: &Expr) -> bool {
        match self.exprs.get(&(e as *const Expr as usize)) {
            Some(av) => matches!(av.interval.lo, Bound::Finite(v) if v >= 0),
            None => false,
        }
    }

    /// As [`Self::expr_int_range`], keyed by the expression's arena address (the
    /// form the i32-narrowing gate collects its key/value sites in).
    pub fn expr_int_range_addr(&self, addr: usize) -> Option<(i64, i64)> {
        let av = self.exprs.get(&addr)?;
        match (&av.interval.lo, &av.interval.hi) {
            (Bound::Finite(lo), Bound::Finite(hi)) if lo <= hi => Some((*lo, *hi)),
            _ => None,
        }
    }

    /// Does this `Map of Int to Int` have a recorded invariant affine capacity
    /// (the precondition for any dense-key proof against it)?
    pub fn has_dense_map_capacity(&self, map: Symbol) -> bool {
        self.map_caps.contains_key(&map)
    }

    /// The recorded invariant affine capacity for a dense map — the bound every
    /// key was proven `<= cap` against. Codegen renders it (see [`lin_to_rust`])
    /// to size the direct-addressed array `with_bounds(0, cap + 1)`.
    pub fn map_cap_lin(&self, map: Symbol) -> Option<&super::affine::LinExpr> {
        self.map_caps.get(&map)
    }

    /// Was this key occurrence PROVEN within `[0, capacity(map)]` (so a
    /// direct-addressed array of `capacity + 1` slots indexes it safely)?
    pub fn dense_map_key_proven(&self, key: &Expr, map: Symbol) -> bool {
        self.dense_map_key_proven_addr(key as *const Expr as usize, map)
    }

    /// As [`Self::dense_map_key_proven`], keyed by the expression's arena address
    /// directly (the form the dense gate collects its key sites in).
    pub fn dense_map_key_proven_addr(&self, key_addr: usize, map: Symbol) -> bool {
        self.dense_map_key_proven.get(&key_addr) == Some(&map)
    }

    /// Does this dense map's single insert loop provably cover a contiguous range
    /// fully (the structural precondition for presence elision)?
    pub fn dense_map_has_full_coverage(&self, map: Symbol) -> bool {
        self.map_insert_cover.contains_key(&map)
    }

    /// Was this key occurrence PROVEN inside its map's fully-covered insert range
    /// (so the key was definitely written, and `get` may skip the presence bit)?
    pub fn dense_key_covered_addr(&self, key_addr: usize, map: Symbol) -> bool {
        self.map_key_covered.get(&key_addr) == Some(&map)
    }

    /// Is this occurrence PROVEN collection-kinded? Seeded at collection
    /// creations and preserved through growth, shrinkage and calls —
    /// kind is a binding-level fact only rebinding can erase.
    pub fn expr_is_tracked_collection(&self, e: &Expr) -> bool {
        self.collections.contains(&(e as *const Expr as usize))
    }

    /// PROOF that two handles denote distinct allocations at every iteration
    /// of `loop_stmt` (a While/Repeat analyzed to convergence). This is the
    /// soundness gate for borrow hoisting: `true` only when both handles
    /// have fully tracked provenance and their may-alias components do not
    /// meet at the loop invariant — including loop-CARRIED edges. Anything
    /// else (tainted handle, non-converged loop, unknown statement,
    /// concurrent context) refuses.
    pub fn loop_handles_definitely_distinct(
        &self,
        loop_stmt: &Stmt,
        a: Symbol,
        b: Symbol,
    ) -> bool {
        let key = loop_stmt as *const Stmt as usize;
        match self.loop_aliases.get(&key) {
            Some(g) => g.definitely_distinct(a, b),
            None => false,
        }
    }

    /// Finite proven LENGTH bounds for this collection occurrence.
    pub fn expr_len_range(&self, e: &Expr) -> Option<(i64, i64)> {
        let iv = self.lengths.get(&(e as *const Expr as usize))?;
        match (&iv.lo, &iv.hi) {
            (Bound::Finite(lo), Bound::Finite(hi)) if lo <= hi => Some((*lo, *hi)),
            _ => None,
        }
    }

    /// True when `index` is proven in bounds RELATIONALLY (multi-variable LIA /
    /// SCEV / entry-guard), as opposed to a plain interval bound. The swap
    /// peephole uses this to keep partition swaps (quicksort's `i`/`j`, related
    /// by `i <= j < hi <= len`) as the unchecked indexed form instead of
    /// downgrading them to a bounds-checked `.swap()`; literal-index swaps
    /// (interval-proven) still become `.swap()`/`__swap_tmp`.
    pub fn index_proven_relational(&self, index: &Expr) -> bool {
        self.relational_inbounds.contains(&(index as *const Expr as usize))
    }

    /// The guard-elision query (EXODIA 1.3): is `index` PROVABLY within the
    /// 1-based bounds of `collection` at this program point? True only when
    /// the index interval sits inside [1, len.lo].
    pub fn index_provably_in_bounds(&self, collection: &Expr, index: &Expr) -> bool {
        // Relational induction-variable bound (SCEV-style): the loop guard
        // already proved `1 <= index <= length(collection)` for this exact
        // read. Intervals alone cannot express it.
        if self.relational_inbounds.contains(&(index as *const Expr as usize)) {
            return true;
        }
        // Non-relational interval check: a concrete index interval inside
        // `[1, proven-minimum-length]`.
        let Some(len) = self.lengths.get(&(collection as *const Expr as usize)) else {
            return false;
        };
        let Some((ilo, ihi)) = self.expr_int_range(index) else { return false };
        let len_lo = match len.lo {
            Bound::Finite(v) => v,
            _ => return false,
        };
        ilo >= 1 && ihi <= len_lo
    }

    /// The `% m` divisors whose `m >= 1` precondition an elision of this exact
    /// `index` relied on (empty for a purely affine/interval elision). Codegen
    /// emits `assert!(m > 0)` for each at the access, discharging the symbolic
    /// element bound's precondition so the elision is sound for every input.
    pub fn index_positivity_guards(&self, index: &Expr) -> &[Symbol] {
        self.positivity_guards
            .get(&(index as *const Expr as usize))
            .map_or(&[], |v| v.as_slice())
    }
}

/// Compute a bottom-up product fact for `e` and every subexpression, at the
/// state `st` (the pre-state of the statement that contains it).
fn record_expr(e: &Expr, st: &RichAbstractState, facts: &mut OracleFacts) {
    use crate::ast::stmt::StringPart;
    match e {
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            record_expr(left, st, facts);
            record_expr(right, st, facts);
        }
        Expr::Not { operand } => record_expr(operand, st, facts),
        Expr::Call { args, .. } => {
            for a in args {
                record_expr(a, st, facts);
            }
        }
        Expr::CallExpr { callee, args } => {
            record_expr(callee, st, facts);
            for a in args {
                record_expr(a, st, facts);
            }
        }
        Expr::Index { collection, index } => {
            record_expr(collection, st, facts);
            record_expr(index, st, facts);
        }
        Expr::Slice { collection, start, end } => {
            record_expr(collection, st, facts);
            record_expr(start, st, facts);
            record_expr(end, st, facts);
        }
        Expr::Copy { expr } => record_expr(expr, st, facts),
        Expr::Give { value } => record_expr(value, st, facts),
        Expr::Length { collection } => record_expr(collection, st, facts),
        Expr::Contains { collection, value } => {
            record_expr(collection, st, facts);
            record_expr(value, st, facts);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                record_expr(i, st, facts);
            }
        }
        Expr::FieldAccess { object, .. } => record_expr(object, st, facts),
        Expr::New { init_fields, .. } => {
            for (_, fe) in init_fields {
                record_expr(fe, st, facts);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, fe) in fields {
                record_expr(fe, st, facts);
            }
        }
        Expr::OptionSome { value } => record_expr(value, st, facts),
        Expr::WithCapacity { value, capacity } => {
            record_expr(value, st, facts);
            record_expr(capacity, st, facts);
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let StringPart::Expr { value, .. } = p {
                    record_expr(value, st, facts);
                }
            }
        }
        _ => {}
    }
    let av = AbstractValue {
        interval: eval_expr(e, &st.intervals),
        ty: eval_type(e, &st.types, &st.fn_returns, &st.elem_type),
        shape: match e {
            Expr::Identifier(sym) => {
                st.shapes.get(sym).cloned().unwrap_or(CollectionShape::Top)
            }
            _ => CollectionShape::Top,
        },
        nullability: nullability_of_expr(e, st),
        alias: AliasInfo::top(),
    };
    facts.record(e, av);
    if let Expr::Identifier(sym) = e {
        facts.record_length(e, st.intervals.get_length(sym));
        if !facts.suppress_exprs && st.coll_vars.contains(sym) {
            facts.collections.insert(e as *const Expr as usize);
        }
    }
}

/// What a loop guard bounds an induction variable by.
enum IvBound {
    /// `length of arr`.
    LenOf(Symbol),
    /// A program variable `n` (related to an array length by a build-loop fact
    /// `length(arr) >= n + off`), optionally minus a provably-nonnegative
    /// amount: the guard is `iv <op> n - headroom_or_more`, so `iv` clears the
    /// length by at least `headroom` — extra room for a positive affine offset
    /// (e.g. `j <= n - 1 - i` lets `item (j + 1) of arr` stay in bounds).
    Var { var: Symbol, headroom: i64 },
}

/// A loop guard `iv </<= bound` bounding the induction variable from above.
struct IvGuard {
    iv: Symbol,
    bound: IvBound,
    /// `true` for `<` (so `iv <= bound - 1`), `false` for `<=`.
    strict: bool,
    /// Proven lower bound of `iv` at the loop body (its 1-based start).
    iv_lo: i64,
}

/// RELATIONAL induction-variable bound elimination (V8 TurboFan / LLVM SCEV).
///
/// The interval domain proves `iv ∈ [lo, …]` but cannot represent the
/// relation `iv <= length(arr)` a loop guard establishes between two
/// variables. This recognizer reconstructs it: from a guard `iv </<= B`
/// (where `B` is `length of arr` directly, or a variable `n` tied to the
/// array's length by a build-loop fact `length(arr) >= n + off`), every
/// in-body read `item E of arr` with `E = iv + k` affine in `iv` is proven
/// `1 <= E <= length(arr)` — exactly what V8/LLVM hoist. Each proven read's
/// `index` address is recorded so the bytecode compiler emits
/// `IndexUnchecked` and the JIT drops the bounds branch.
fn record_loop_index_bounds(
    cond: &Expr,
    body: &[Stmt],
    state: &RichAbstractState,
    facts: &mut OracleFacts,
) {
    let mut guards: Vec<IvGuard> = Vec::new();
    collect_iv_guards(cond, state, &mut guards);
    if guards.is_empty() {
        return;
    }
    // A call (a callee may resize) or any unmodeled statement could change a
    // length or alias an array — refuse the whole proof. Resizes of SPECIFIC
    // arrays are tolerated: those arrays start clobbered, so their own reads
    // stay checked, while a stable array read alongside them still elides.
    if !body_is_index_proof_safe(body) {
        return;
    }
    let mut clobbered: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
    collect_resized_arrays(body, &mut clobbered);
    record_proven_reads(body, &guards, &state.length_def, &mut clobbered, facts);
}

// ===================================================================
// GENERAL multi-variable bounds elision via the kernel LIA prover.
//
// Where the single-variable recognizer above proves `item (iv+k) of arr` and
// the interval check proves a constant index into a constant-length array,
// this proves ANY affine index `item E of arr` whose `1 <= E <= length(arr)`
// follows from the enclosing loop guard, the path guards on the way in, the
// loop-invariant scalar ranges (INCLUDING element bounds reached through
// `Let v be item k of B` — A2), the affine scalar definitions (`cols =
// capacity + 1`), and a symbolic length lower bound. Each `1 <= E` and
// `E <= length` obligation is discharged by `logicaffeine_kernel::lia`
// (Fourier–Motzkin) and recorded into `relational_inbounds`, shared with the
// single-var path and consumed by codegen and the VM. Knapsack's
// `prev[w-wi+1]` (guard `w <= capacity`, path guard `w >= wi`, `length(prev) =
// cols = capacity + 1`, element bound `wi ∈ [1,50]`) is the canonical case.
// ===================================================================

/// The affine fact context threaded down a loop nest: the kernel constraints
/// in scope (loop and path guards, monotone-IV lower bounds) plus the symbols
/// they mention, so a proof can pull in those variables' interval bounds and
/// scalar definitions.
#[derive(Default, Clone)]
struct AffineFacts {
    constraints: Vec<super::affine::Constraint>,
    syms: Vec<Symbol>,
}

fn record_affine_index_bounds(
    cond: &Expr,
    body: &[Stmt],
    state: &RichAbstractState,
    entry: &RichAbstractState,
    facts: &mut OracleFacts,
) {
    if !body_is_index_proof_safe(body) {
        return;
    }
    let mut clobbered: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
    collect_resized_arrays(body, &mut clobbered);
    let mutated: std::collections::HashSet<Symbol> =
        collect_mutations(body).into_iter().collect();
    // The loop fixpoint merges `scalar_upper`/`scalar_lower` across the back-edge
    // but NOT `scalar_def`, so a mid-body `Let complement be target - x` is gone
    // by the record state. Re-establish the body's TOP-LEVEL affine definitions
    // (their RHS is straight-line before every later use, and the bound var is not
    // reassigned), so a key proof can follow the chain `complement -> target - x
    // -> x`'s element bounds`. Sound: each added equality holds at every use site.
    let state = {
        let mut st = state.clone();
        augment_body_scalar_defs(body, &mut st);
        st
    };
    let inits: HashMap<Symbol, i64> = HashMap::new();
    let mut amb = AffineFacts::default();
    affine_add_loop_guards(cond, body, &state, Some(entry), &inits, &mut amb);
    affine_walk(body, &state, &clobbered, &mutated, &amb, &inits, facts);
}

/// Re-establish the body's TOP-LEVEL affine scalar definitions into a proof
/// state. Only straight-line `Let v be <affine>` where `v` is not reassigned in
/// the body (so the equality holds at every later use in the iteration) and the
/// RHS is variable-bearing and non-self-referential — exactly the form the LIA
/// prover consumes. Conditional (nested-block) defs are skipped: their equality
/// is not unconditional. Never overwrites an existing `scalar_def`.
fn augment_body_scalar_defs(body: &[Stmt], st: &mut RichAbstractState) {
    // Scalars reassigned via `Set` anywhere in the body: a def naming any of them
    // (as the bound var OR in its RHS) is not a stable iteration invariant.
    let mut reassigned = std::collections::HashSet::new();
    collect_set_targets(body, &mut reassigned);
    // Only a single, unshadowed top-level `Let` has an unambiguous definition.
    let mut let_count: HashMap<Symbol, u32> = HashMap::new();
    for s in body {
        if let Stmt::Let { var, .. } = s {
            *let_count.entry(*var).or_insert(0) += 1;
        }
    }
    for s in body {
        if let Stmt::Let { var, value, .. } = s {
            if reassigned.contains(var)
                || let_count.get(var) != Some(&1)
                || st.scalar_def.contains_key(var)
            {
                continue;
            }
            if let Some(e) = super::affine::lin_of(value) {
                // Every RHS variable must be single-assignment within the body
                // (not `Set`-reassigned and not re-`Let`/shadowed) so its value is
                // fixed for the iteration — then `var = E` holds at every later use.
                // An outer variable (absent from `let_count`) is stable unless it is
                // reassigned in the body.
                let rhs_stable = e.coefficients.keys().all(|&i| {
                    let s = Symbol::from_index(i as usize);
                    !reassigned.contains(&s) && let_count.get(&s).map_or(true, |&c| c <= 1)
                });
                if rhs_stable
                    && !e.coefficients.is_empty()
                    && !e.coefficients.contains_key(&(var.index() as i64))
                {
                    st.scalar_def.insert(*var, e);
                }
            }
        }
    }
}

/// Collect every scalar `Set` reassignment target in `stmts` (recursing into
/// nested blocks). Distinct from `collect_mutations`, which also counts `Let`
/// bindings — here only true reassignments matter.
fn collect_set_targets(stmts: &[Stmt], out: &mut std::collections::HashSet<Symbol>) {
    for s in stmts {
        if let Stmt::Set { target, .. } = s {
            out.insert(*target);
        }
        each_child_block(s, &mut |b| collect_set_targets(b, out));
    }
}

/// `a <op> b` (optionally negated) as a kernel constraint, when both sides are
/// affine.
fn affine_cmp(
    op: BinaryOpKind,
    a: &Expr,
    b: &Expr,
    negate: bool,
) -> Option<super::affine::Constraint> {
    use super::affine::{ge, gt, le, lin_of, lt};
    let (la, lb) = (lin_of(a)?, lin_of(b)?);
    Some(match (op, negate) {
        (BinaryOpKind::Lt, false) | (BinaryOpKind::GtEq, true) => lt(&la, &lb),
        (BinaryOpKind::LtEq, false) | (BinaryOpKind::Gt, true) => le(&la, &lb),
        (BinaryOpKind::Gt, false) | (BinaryOpKind::LtEq, true) => gt(&la, &lb),
        (BinaryOpKind::GtEq, false) | (BinaryOpKind::Lt, true) => ge(&la, &lb),
        _ => return None,
    })
}

/// Add a comparison (and the symbols it mentions) to a fact set, descending
/// through `and`. A disjunctive (`or`) guard cannot be soundly negated
/// termwise, so a negated `or`/anything non-comparison is dropped.
fn affine_add_cmp(cond: &Expr, negate: bool, out: &mut AffineFacts) {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::And, left, right } if !negate => {
            affine_add_cmp(left, false, out);
            affine_add_cmp(right, false, out);
        }
        Expr::BinaryOp {
            op:
                op @ (BinaryOpKind::Lt
                | BinaryOpKind::LtEq
                | BinaryOpKind::Gt
                | BinaryOpKind::GtEq),
            left,
            right,
        } => {
            if let Some(c) = affine_cmp(*op, left, right, negate) {
                out.constraints.push(c);
                affine_collect_syms(left, &mut out.syms);
                affine_collect_syms(right, &mut out.syms);
            }
        }
        _ => {}
    }
}

/// Collect the identifier symbols of an affine-shaped expression.
fn affine_collect_syms(e: &Expr, out: &mut Vec<Symbol>) {
    match e {
        Expr::Identifier(s) => {
            if !out.contains(s) {
                out.push(*s);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            affine_collect_syms(left, out);
            affine_collect_syms(right, out);
        }
        _ => {}
    }
}

/// A write `Set iv to value` keeps `iv >= lo` (given the induction hypothesis
/// `iv >= lo` before it): an increment by a non-negative literal, a literal
/// `>= lo`, or assignment from a LOOP-INVARIANT variable whose interval lower
/// is `>= lo`. The loop-invariant requirement makes `state`'s interval for that
/// variable a sound bound (a body-mutated source could differ mid-iteration).
fn write_keeps_ge(
    value: &Expr,
    iv: Symbol,
    lo: i64,
    state: &RichAbstractState,
    mutated: &std::collections::HashSet<Symbol>,
) -> bool {
    match value {
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            let is_iv = |e: &&Expr| matches!(e, Expr::Identifier(s) if *s == iv);
            let nonneg = |e: &&Expr| matches!(e, Expr::Literal(Literal::Number(n)) if *n >= 0);
            (is_iv(&left) && nonneg(&right)) || (nonneg(&left) && is_iv(&right))
        }
        Expr::Literal(Literal::Number(n)) => *n >= lo,
        Expr::Identifier(s) if !mutated.contains(s) => {
            matches!(state.intervals.get_var(s).lo, Bound::Finite(l) if l >= lo)
        }
        _ => false,
    }
}

/// A SOUND lower bound for a loop variable throughout the body: its entry value
/// `lo`, provided every write to it keeps it `>= lo` (induction). The entry
/// value comes from an explicit init `Let j be 0` in the ENCLOSING body
/// (`inits`) when available — essential for a NESTED loop whose counter is
/// reset each outer iteration (so the outer state widens it to top) — else the
/// loop-invariant state interval. More general than pure monotonicity: it
/// survives the `Set j to needleLen` of a break loop (needleLen loop-invariant,
/// `>= 0`), where a strict increment-only check gives up.
fn iv_lower_bound(
    iv: Symbol,
    body: &[Stmt],
    state: &RichAbstractState,
    mutated: &std::collections::HashSet<Symbol>,
    inits: &HashMap<Symbol, i64>,
) -> Option<i64> {
    let lo = match inits.get(&iv) {
        Some(c) => *c,
        None => match state.intervals.get_var(&iv).lo {
            Bound::Finite(l) => l,
            _ => return None,
        },
    };
    fn preserved(
        iv: Symbol,
        stmts: &[Stmt],
        lo: i64,
        state: &RichAbstractState,
        mutated: &std::collections::HashSet<Symbol>,
    ) -> bool {
        stmts.iter().all(|s| match s {
            Stmt::Set { target, value } if *target == iv => {
                write_keeps_ge(value, iv, lo, state, mutated)
            }
            Stmt::Let { var, value, .. } if *var == iv => {
                write_keeps_ge(value, iv, lo, state, mutated)
            }
            Stmt::If { then_block, else_block, .. } => {
                preserved(iv, then_block, lo, state, mutated)
                    && else_block.map_or(true, |eb| preserved(iv, eb, lo, state, mutated))
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                preserved(iv, body, lo, state, mutated)
            }
            _ => true,
        })
    }
    preserved(iv, body, lo, state, mutated).then_some(lo)
}

/// Add a loop's guard constraints, plus a sound lower bound for each induction
/// variable, to the fact set. `inits` carries explicit counter initializations
/// from the enclosing body (`Let j be 0`) so a reset nested counter still gets
/// its entry lower bound.
/// `Set v to v + c` or `Set v to c + v` (literal `c`) → `Some(c)`.
pub(crate) fn self_increment(v: Symbol, value: &Expr) -> Option<i64> {
    if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = value {
        if let (Expr::Identifier(s), Expr::Literal(Literal::Number(c))) = (&**left, &**right) {
            if *s == v {
                return Some(*c);
            }
        }
        if let (Expr::Literal(Literal::Number(c)), Expr::Identifier(s)) = (&**left, &**right) {
            if *s == v {
                return Some(*c);
            }
        }
    }
    None
}

/// Total number of writes (`Set`/`Let`) to `v` anywhere in `stmts`.
pub(crate) fn count_writes_of(stmts: &[Stmt], v: Symbol) -> usize {
    let mut n = 0;
    for s in stmts {
        match s {
            Stmt::Set { target, .. } if *target == v => n += 1,
            Stmt::Let { var, .. } if *var == v => n += 1,
            Stmt::If { then_block, else_block, .. } => {
                n += count_writes_of(then_block, v);
                if let Some(eb) = else_block {
                    n += count_writes_of(eb, v);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => n += count_writes_of(body, v),
            _ => {}
        }
    }
    n
}

/// Variables that have a TOP-LEVEL (direct child of `body`, not nested in any
/// `If`/`While`) `Set v to v + dj` (`dj > 0`) that is `v`'s ONLY write in the
/// whole body — so each iteration increments `v` by exactly `dj`.
fn top_level_unconditional_incs(body: &[Stmt]) -> Vec<(Symbol, i64)> {
    let mut out = Vec::new();
    for s in body {
        if let Stmt::Set { target, value } = s {
            if let Some(c) = self_increment(*target, value) {
                if c > 0 && count_writes_of(body, *target) == 1 {
                    out.push((*target, c));
                }
            }
        }
    }
    out
}

/// Is every write to `v` in `stmts` a `Set v to v + ci` (`ci >= 0`), none inside
/// a nested loop, with the writes summing to `<= limit` and at least one present?
/// Such a `v` increases by at most `limit` per outer iteration.
fn monotone_inc_within(stmts: &[Stmt], v: Symbol, in_loop: bool, total: &mut i64, count: &mut usize) -> bool {
    for s in stmts {
        match s {
            Stmt::Set { target, value } if *target == v => match self_increment(v, value) {
                Some(c) if c >= 0 && !in_loop => {
                    *total += c;
                    *count += 1;
                }
                _ => return false,
            },
            Stmt::Let { var, .. } if *var == v => return false,
            Stmt::If { then_block, else_block, .. } => {
                if !monotone_inc_within(then_block, v, in_loop, total, count) {
                    return false;
                }
                if let Some(eb) = else_block {
                    if !monotone_inc_within(eb, v, in_loop, total, count) {
                        return false;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if !monotone_inc_within(body, v, true, total, count) {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// Candidate `i` variables for an `i <= j` invariant: monotone increments
/// summing to `<= limit`, at least one increment, none inside a nested loop.
fn monotone_inc_vars(body: &[Stmt], limit: i64) -> Vec<Symbol> {
    let mut cands: Vec<Symbol> = Vec::new();
    fn collect(stmts: &[Stmt], out: &mut Vec<Symbol>) {
        for s in stmts {
            match s {
                Stmt::Set { target, value } if self_increment(*target, value).is_some() => {
                    if !out.contains(target) {
                        out.push(*target);
                    }
                }
                Stmt::If { then_block, else_block, .. } => {
                    collect(then_block, out);
                    if let Some(eb) = else_block {
                        collect(eb, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => collect(body, out),
                _ => {}
            }
        }
    }
    collect(body, &mut cands);
    cands
        .into_iter()
        .filter(|v| {
            let (mut total, mut count) = (0, 0);
            monotone_inc_within(body, *v, false, &mut total, &mut count)
                && count >= 1
                && total <= limit
        })
        .collect()
}

/// Derive `i <= j` loop invariants (the quicksort/Lomuto partition shape).
///
/// `j` increments unconditionally by `dj` at the top of the body; `i`'s only
/// writes are `Set i to i + ci` (`ci >= 0`) summing to `<= dj`, none inside a
/// nested loop; and `i` and `j` start from the SAME value (equal `scalar_def` at
/// loop entry — covers `i=1,j=1` and `i=lo,j=lo` alike). Then `i0 = j0` and
/// `Δi <= Δj` each iteration, so `i <= j` holds at the loop head. `affine_walk`
/// drops the fact when `i` (or `j`) is mutated, so it only proves accesses
/// BEFORE the increment — exactly the partition's `item i of arr` read/store.
fn derive_iv_le_invariants(
    body: &[Stmt],
    entry: &RichAbstractState,
    state: &RichAbstractState,
    mutated: &std::collections::HashSet<Symbol>,
    inits: &HashMap<Symbol, i64>,
    amb: &mut AffineFacts,
) {
    use super::affine::{ge, konst, le, var};
    for (j, dj) in top_level_unconditional_incs(body) {
        for i in monotone_inc_vars(body, dj) {
            if i == j {
                continue;
            }
            // i0 == j0: `i` and `j` are defined by the same linear expression at
            // loop entry (constant or symbolic `lo`).
            let equal_start = match (entry.scalar_def.get(&i), entry.scalar_def.get(&j)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };
            if !equal_start {
                continue;
            }
            amb.constraints.push(le(&var(i), &var(j)));
            // `1 <= i` obligation needs `i`'s sound lower bound (it is not a
            // guard variable, so the guard pass would not add it).
            if let Some(lo) = iv_lower_bound(i, body, state, mutated, inits) {
                amb.constraints.push(ge(&var(i), &konst(lo)));
            }
            for s in [i, j] {
                if !amb.syms.contains(&s) {
                    amb.syms.push(s);
                }
            }
        }
    }
}

fn affine_add_loop_guards(
    cond: &Expr,
    body: &[Stmt],
    state: &RichAbstractState,
    entry: Option<&RichAbstractState>,
    inits: &HashMap<Symbol, i64>,
    amb: &mut AffineFacts,
) {
    affine_add_cmp(cond, false, amb);
    let mutated: std::collections::HashSet<Symbol> =
        collect_mutations(body).into_iter().collect();
    let mut ivs: Vec<Symbol> = Vec::new();
    affine_collect_syms(cond, &mut ivs);
    for iv in ivs {
        if let Some(lo) = iv_lower_bound(iv, body, state, &mutated, inits) {
            amb.constraints
                .push(super::affine::ge(&super::affine::var(iv), &super::affine::konst(lo)));
            if !amb.syms.contains(&iv) {
                amb.syms.push(iv);
            }
        }
    }
    // Two-induction-variable `i <= j` relations need the true loop-ENTRY state to
    // establish `i0 = j0`; only the top-level call threads it (nested loops pass
    // `None`, since the widened body state cannot witness the entry equality).
    if let Some(entry) = entry {
        derive_iv_le_invariants(body, entry, state, &mutated, inits, amb);
    }
}

/// Drop ambient facts mentioning any symbol in `set` (a write reaches earlier
/// reads across loop iterations, or a sequential reassignment invalidates a
/// guard).
fn affine_drop_mentioning(amb: &mut AffineFacts, set: &std::collections::HashSet<Symbol>) {
    amb.constraints.retain(|c| {
        !set.iter().any(|s| c.expr.coefficients.contains_key(&(s.index() as i64)))
    });
    amb.syms.retain(|s| !set.contains(s));
}

fn affine_drop_one(amb: &mut AffineFacts, sym: Symbol) {
    let k = sym.index() as i64;
    amb.constraints.retain(|c| !c.expr.coefficients.contains_key(&k));
    amb.syms.retain(|s| *s != sym);
}

/// Prove every `Index` node in an expression in place (no ref storage —
/// avoids threading the arena lifetime through a collecting `Vec`).
fn affine_prove_in_expr(
    e: &Expr,
    state: &RichAbstractState,
    clobbered: &std::collections::HashSet<Symbol>,
    mutated: &std::collections::HashSet<Symbol>,
    amb: &AffineFacts,
    facts: &mut OracleFacts,
) {
    if let Expr::Index { collection, index } = e {
        // Dense `Map of Int to Int` get: `item k of m` → prove `0 <= k <= cap(m)`.
        if let Expr::Identifier(m) = collection {
            if facts.map_caps.contains_key(m) {
                try_prove_dense_key(*m, index, state, mutated, amb, facts);
            }
        }
        try_prove_affine(collection, index, state, clobbered, mutated, amb, facts);
    }
    // Dense map membership: `m contains k` → prove `0 <= k <= cap(m)`.
    if let Expr::Contains { collection, value } = e {
        if let Expr::Identifier(m) = collection {
            if facts.map_caps.contains_key(m) {
                try_prove_dense_key(*m, value, state, mutated, amb, facts);
            }
        }
    }
    match e {
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            affine_prove_in_expr(left, state, clobbered, mutated, amb, facts);
            affine_prove_in_expr(right, state, clobbered, mutated, amb, facts);
        }
        Expr::Not { operand } => affine_prove_in_expr(operand, state, clobbered, mutated, amb, facts),
        Expr::Contains { collection, value } => {
            affine_prove_in_expr(collection, state, clobbered, mutated, amb, facts);
            affine_prove_in_expr(value, state, clobbered, mutated, amb, facts);
        }
        Expr::Index { collection, index } => {
            affine_prove_in_expr(collection, state, clobbered, mutated, amb, facts);
            affine_prove_in_expr(index, state, clobbered, mutated, amb, facts);
        }
        Expr::Call { args, .. } => {
            for a in args {
                affine_prove_in_expr(a, state, clobbered, mutated, amb, facts);
            }
        }
        Expr::Length { collection } => {
            affine_prove_in_expr(collection, state, clobbered, mutated, amb, facts)
        }
        _ => {}
    }
}

/// A symbolic LOWER bound on `length(arr)` (the `var(n) + off` from a counted
/// build loop, else a finite concrete length), with the variables it names.
fn affine_length_lb(
    arr: Symbol,
    state: &RichAbstractState,
) -> Option<(super::affine::LinExpr, Vec<Symbol>)> {
    use super::affine::{konst, var};
    if let Some((n, off)) = state.length_def.get(&arr) {
        return Some((var(*n).add(&konst(*off)), vec![*n]));
    }
    if let Bound::Finite(l) = state.intervals.get_length(&arr).lo {
        return Some((konst(l), vec![]));
    }
    None
}

/// Walk a loop body in execution order, accumulating path guards and nested
/// loop guards, and prove each affine index access in scope.
fn affine_walk(
    stmts: &[Stmt],
    state: &RichAbstractState,
    clobbered: &std::collections::HashSet<Symbol>,
    mutated: &std::collections::HashSet<Symbol>,
    ambient: &AffineFacts,
    inits: &HashMap<Symbol, i64>,
    facts: &mut OracleFacts,
) {
    let mut local = ambient.clone();
    let mut inits = inits.clone();
    for stmt in stmts {
        for_each_direct_expr(stmt, &mut |e| {
            affine_prove_in_expr(e, state, clobbered, mutated, &local, facts);
        });
        if let Stmt::SetIndex { collection, index, .. } = stmt {
            // Dense `Map of Int to Int` insert: `Set item k of m to v` → prove
            // `0 <= k <= cap(m)`.
            if let Expr::Identifier(m) = collection {
                if facts.map_caps.contains_key(m) {
                    try_prove_dense_key(*m, index, state, mutated, &local, facts);
                }
            }
            try_prove_affine(collection, index, state, clobbered, mutated, &local, facts);
        }
        match stmt {
            Stmt::If { cond, then_block, else_block } => {
                let mut then_facts = local.clone();
                affine_add_cmp(cond, false, &mut then_facts);
                affine_walk(then_block, state, clobbered, mutated, &then_facts, &inits, facts);
                if let Some(eb) = else_block {
                    let mut else_facts = local.clone();
                    affine_add_cmp(cond, true, &mut else_facts);
                    affine_walk(eb, state, clobbered, mutated, &else_facts, &inits, facts);
                }
            }
            Stmt::While { cond, body, .. } => {
                // A nested loop's body may rewrite a variable an ambient fact
                // rests on (a later iteration reaching an earlier read), so drop
                // facts naming anything it mutates before adding its own guard.
                let nested: std::collections::HashSet<Symbol> =
                    collect_mutations(body).into_iter().collect();
                let mut inner = local.clone();
                affine_drop_mentioning(&mut inner, &nested);
                affine_add_loop_guards(cond, body, state, None, &inits, &mut inner);
                affine_walk(body, state, clobbered, mutated, &inner, &inits, facts);
            }
            Stmt::Repeat { body, .. } => {
                affine_walk(body, state, clobbered, mutated, &local, &inits, facts);
            }
            _ => {}
        }
        // Track counter initializations (`Let j be 0`) so a following nested
        // loop sees the reset counter's true entry lower bound; a reassignment
        // to anything else clears it. Also drop ambient facts naming the target.
        match stmt {
            Stmt::Set { target, value } => {
                if let Expr::Literal(Literal::Number(n)) = value {
                    inits.insert(*target, *n);
                } else {
                    inits.remove(target);
                }
                affine_drop_one(&mut local, *target);
            }
            Stmt::Let { var, value, .. } => {
                if let Expr::Literal(Literal::Number(n)) = value {
                    inits.insert(*var, *n);
                } else {
                    inits.remove(var);
                }
                affine_drop_one(&mut local, *var);
            }
            _ => {}
        }
    }
}

/// Discharge `1 <= E <= length(arr)` for a single access via the kernel LIA
/// prover, recording the index address on success.
fn try_prove_affine(
    collection: &Expr,
    index: &Expr,
    state: &RichAbstractState,
    clobbered: &std::collections::HashSet<Symbol>,
    mutated: &std::collections::HashSet<Symbol>,
    amb: &AffineFacts,
    facts: &mut OracleFacts,
) {
    use super::affine::{konst, le, lin_of, prove, var};
    let Expr::Identifier(arr) = collection else { return };
    if clobbered.contains(arr) {
        return;
    }
    let key = index as *const Expr as usize;
    if facts.relational_inbounds.contains(&key) {
        return; // already proven (single-var path or a prior visit)
    }
    let Some(e_lin) = lin_of(index) else { return };
    let Some((len_lb, len_syms)) = affine_length_lb(*arr, state) else {
        return;
    };

    let mut system = amb.constraints.clone();
    let mut syms = amb.syms.clone();
    affine_collect_syms(index, &mut syms);
    for s in len_syms {
        if !syms.contains(&s) {
            syms.push(s);
        }
    }
    for &s in &syms {
        // Affine scalar definition `s = E` (relates a length var to a loop
        // bound: `cols = capacity + 1`).
        if let Some(def) = state.scalar_def.get(&s) {
            let sv = var(s);
            system.push(le(&sv, def));
            system.push(le(def, &sv));
        }
        // Loop-invariant scalar's interval bounds (the element bound `wi ∈
        // [1,50]` reached through `Let wi be item _ of weights`). A MUTATED
        // variable's raw header interval is NOT sound mid-body — those get
        // bounds only from guards / monotone-IV lowers.
        if !mutated.contains(&s) {
            let iv = state.intervals.get_var(&s);
            if let Bound::Finite(lo) = iv.lo {
                system.push(le(&konst(lo), &var(s)));
            }
            if let Bound::Finite(hi) = iv.hi {
                system.push(le(&var(s), &konst(hi)));
            }
        }
        // Symbolic element-source UPPER bound: a scalar read from a `% n`-filled
        // array carries `s <= n - 1` — the variable-divisor relation no concrete
        // `Interval` can hold (`n` is symbolic). Unlike the raw header interval
        // above this is sound for a MUTATED `s`: the in-body `Let u be item _ of
        // adj` re-establishes it every iteration, so EACH value of `u` (not just
        // the entry one) satisfies it. The bound's own variables (`n`) reach the
        // system through the length lower bound that put them in `syms`.
        // The element LOWER bound (`u >= 0` for an element of a `% n`-filled
        // array) joins the BASE system: it holds regardless of the divisor's
        // sign (`X % n` of a non-negative dividend is `>= 0` for any `n != 0`),
        // so it needs NO positivity guard. Sound for a MUTATED `s`: a value
        // re-read from a fixed array each iteration lands in its proven range.
        if let Some(l) = state.scalar_lower.get(&s) {
            system.push(le(l, &var(s)));
        }
    }

    // Defensive: a contradictory fact set proves any goal. Never elide from
    // inconsistency — if a stale/false hypothesis poisoned the system, refuse.
    if !super::affine::consistent(&system) {
        return;
    }
    // 1-based bounds: lower `E - 1 >= 0` (E >= 1), upper `len_lb - E >= 0`.
    let lower = e_lin.add(&konst(-1));
    let upper = len_lb.sub(&e_lin);
    if !prove(&system, &lower) {
        return;
    }
    // A purely AFFINE upper carries no positivity obligation — try it first.
    if prove(&system, &upper) {
        facts.relational_inbounds.insert(key);
        return;
    }
    // The upper needs the symbolic element UPPER `u <= m - 1` (the variable-
    // divisor modulo bound a concrete interval cannot hold). Adding it closes
    // the proof under `m >= 1`; record the `% m` divisors so codegen discharges
    // that precondition with a nonemptiness-guarded `assert!(m > 0)`, keeping
    // the lenient symbolic element join sound for every input.
    let mut mod_divisors: Vec<Symbol> = Vec::new();
    for &s in &syms {
        if let Some(u) = state.scalar_upper.get(&s) {
            system.push(le(&var(s), u));
            if let Some(m) = mod_upper_divisor(u) {
                mod_divisors.push(m);
            }
        }
    }
    if super::affine::consistent(&system) && prove(&system, &upper) {
        facts.relational_inbounds.insert(key);
        if !mod_divisors.is_empty() {
            facts.positivity_guards.entry(key).or_default().extend(mod_divisors);
        }
    }
}

/// Discharge `0 <= key <= capacity(map)` for one access on a `Map of Int to Int`
/// proven dense-eligible, via the SAME kernel LIA prover as `try_prove_affine`.
/// The map's capacity is an invariant affine expression captured at its
/// declaration (`facts.map_caps`); proving both bounds means a direct-addressed
/// array of `capacity + 1` slots indexes every key safely at offset 0
/// (`data[key]`), since `key ∈ [0, capacity]`. Records the key address on success;
/// the dense gate fires for a map only when EVERY key site is recorded for it.
fn try_prove_dense_key(
    map: Symbol,
    key: &Expr,
    state: &RichAbstractState,
    mutated: &std::collections::HashSet<Symbol>,
    amb: &AffineFacts,
    facts: &mut OracleFacts,
) {
    use super::affine::{consistent, konst, le, lin_of, prove, var};
    let key_addr = key as *const Expr as usize;
    if facts.dense_map_key_proven.contains_key(&key_addr) {
        return;
    }
    let Some(cap) = facts.map_caps.get(&map).cloned() else {
        return;
    };
    let Some(k_lin) = lin_of(key) else { return };

    // Mirror `try_prove_affine`'s system: ambient loop/path guards plus each
    // mentioned symbol's sound scalar facts. Collection is TRANSITIVE over the
    // scalar-definition and symbolic-bound chains, so an element-derived key
    // (`complement = target - x`, `x = item i of arr`) pulls in `target`'s
    // definition (`= n`) and `x`'s symbolic element bounds (`0 <= x <= n-1`) —
    // closing `0 <= complement <= n`. Each added fact is individually sound, so
    // the (consistency-guarded) system only ever proves MORE, never unsoundly.
    let mut system = amb.constraints.clone();
    let mut syms = amb.syms.clone();
    affine_collect_syms(key, &mut syms);
    let mut reached: std::collections::HashSet<Symbol> = syms.iter().copied().collect();
    let mut w = 0;
    while w < syms.len() {
        let s = syms[w];
        w += 1;
        for chained in [
            state.scalar_def.get(&s),
            state.scalar_upper.get(&s),
            state.scalar_lower.get(&s),
        ] {
            if let Some(e) = chained {
                for &idx in e.coefficients.keys() {
                    let t = Symbol::from_index(idx as usize);
                    if reached.insert(t) {
                        syms.push(t);
                    }
                }
            }
        }
    }
    for &s in &syms {
        if let Some(def) = state.scalar_def.get(&s) {
            let sv = var(s);
            system.push(le(&sv, def));
            system.push(le(def, &sv));
        }
        if !mutated.contains(&s) {
            let iv = state.intervals.get_var(&s);
            if let Bound::Finite(lo) = iv.lo {
                system.push(le(&konst(lo), &var(s)));
            }
            if let Bound::Finite(hi) = iv.hi {
                system.push(le(&var(s), &konst(hi)));
            }
        }
        if let Some(l) = state.scalar_lower.get(&s) {
            system.push(le(l, &var(s)));
        }
        if let Some(u) = state.scalar_upper.get(&s) {
            system.push(le(&var(s), u));
        }
    }

    // Never elide from an inconsistent system (a false hypothesis proves anything).
    if !consistent(&system) {
        return;
    }
    // Lower `key >= 0` and upper `capacity - key >= 0`.
    if prove(&system, &k_lin) && prove(&system, &cap.sub(&k_lin)) {
        facts.dense_map_key_proven.insert(key_addr, map);
        // Presence elision: if the map's inserts fully cover `[A, B]`, prove this
        // key is also inside `[A, B]` — then it was definitely written, so `get`
        // needs no presence check. `A <= key` and `key <= B`.
        if let Some((a, b)) = facts.map_insert_cover.get(&map).cloned() {
            if prove(&system, &k_lin.sub(&a)) && prove(&system, &b.sub(&k_lin)) {
                facts.map_key_covered.insert(key_addr, map);
            }
        }
    }
}

/// Collect upper-bound induction guards from a loop condition, descending
/// through `and`: `iv </<= B` and the flipped `B >/>= iv`, where `B` is
/// `length of arr` or a plain variable. `iv`'s lower bound must be a known
/// constant (its loop-start value).
fn collect_iv_guards(cond: &Expr, state: &RichAbstractState, out: &mut Vec<IvGuard>) {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::And, left, right } => {
            collect_iv_guards(left, state, out);
            collect_iv_guards(right, state, out);
        }
        Expr::BinaryOp { op, left, right } => {
            let (iv_e, bnd_e, strict): (&Expr, &Expr, bool) = match op {
                BinaryOpKind::Lt => (*left, *right, true),
                BinaryOpKind::LtEq => (*left, *right, false),
                BinaryOpKind::Gt => (*right, *left, true),
                BinaryOpKind::GtEq => (*right, *left, false),
                _ => return,
            };
            let Expr::Identifier(iv) = iv_e else { return };
            // A non-constant start (e.g. an induction var seeded from a
            // parameter) gives no static lower bound — `i64::MIN` makes the
            // STATIC proof fail closed, while the SPECULATIVE hoist (which
            // checks the lower bound at runtime) still collects the guard.
            let iv_lo = match state.intervals.get_var(iv).lo {
                Bound::Finite(lo) => lo,
                _ => i64::MIN,
            };
            let bound = match bnd_e {
                Expr::Length { collection } => match &**collection {
                    Expr::Identifier(arr) => IvBound::LenOf(*arr),
                    _ => return,
                },
                Expr::Identifier(v) => IvBound::Var { var: *v, headroom: 0 },
                // `n - t1 - t2 - …` with each subtracted term provably ≥ 0.
                _ => match var_minus_bound(bnd_e, state) {
                    Some((v, headroom)) => IvBound::Var { var: v, headroom },
                    None => return,
                },
            };
            out.push(IvGuard { iv: *iv, bound, strict, iv_lo });
        }
        _ => {}
    }
}

/// Recognize a bound expression `n - t1 - t2 - …` rooted at a variable `n`,
/// where every subtracted term is provably `>= 0`. Returns `(n, headroom)`
/// with `headroom = Σ lower-bounds of the subtracted terms` — a sound lower
/// bound on `n - bound`, i.e. how far the guard keeps `iv` below `n`.
fn var_minus_bound(b: &Expr, state: &RichAbstractState) -> Option<(Symbol, i64)> {
    match b {
        Expr::Identifier(v) => Some((*v, 0)),
        Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right } => {
            let (v, head) = var_minus_bound(left, state)?;
            // The subtracted term must be provably non-negative (else `n -
            // term` could exceed `n` and the bound would be unsound).
            let r_lo = match eval_expr(right, &state.intervals).lo {
                Bound::Finite(lo) => lo,
                _ => return None,
            };
            if r_lo < 0 {
                return None;
            }
            Some((v, head.saturating_add(r_lo)))
        }
        _ => None,
    }
}

/// Extract `(var, offset)` from an affine index expression: `v`, `v + k`,
/// `k + v`, or `v - k` (`k` an integer literal).
fn affine_of(e: &Expr) -> Option<(Symbol, i64)> {
    match e {
        Expr::Identifier(s) => Some((*s, 0)),
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            match (&**left, &**right) {
                (Expr::Identifier(s), Expr::Literal(Literal::Number(k))) => Some((*s, *k)),
                (Expr::Literal(Literal::Number(k)), Expr::Identifier(s)) => Some((*s, *k)),
                _ => None,
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right } => {
            match (&**left, &**right) {
                (Expr::Identifier(s), Expr::Literal(Literal::Number(k))) => Some((*s, -*k)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// If `collection` is an array and `index` is affine in a live guard's
/// induction variable that proves it in bounds, record the index's address
/// as proven (consumed by the bytecode compiler for both `IndexUnchecked`
/// reads and `SetIndexUnchecked` stores).
fn try_record_index(
    collection: &Expr,
    index: &Expr,
    guards: &[IvGuard],
    length_def: &HashMap<Symbol, (Symbol, i64)>,
    clobbered: &std::collections::HashSet<Symbol>,
    facts: &mut OracleFacts,
) {
    let Expr::Identifier(arr) = collection else { return };
    if clobbered.contains(arr) {
        return;
    }
    let Some((iv, k)) = affine_of(index) else { return };
    let proven = guards
        .iter()
        .any(|g| g.iv == iv && !clobbered.contains(&g.iv) && guard_proves(g, *arr, k, length_def));
    if proven {
        facts.relational_inbounds.insert(index as *const Expr as usize);
    }
}

/// Does guard `g` prove `1 <= (iv + k) <= length(arr)` for an access on
/// `arr` with affine offset `k`?
fn guard_proves(
    g: &IvGuard,
    arr: Symbol,
    k: i64,
    length_def: &HashMap<Symbol, (Symbol, i64)>,
) -> bool {
    // Lower bound: iv >= iv_lo ⇒ index = iv + k >= iv_lo + k, need >= 1.
    if g.iv_lo.saturating_add(k) < 1 {
        return false;
    }
    // Upper bound: index_max = iv_max + k, where iv_max = bound - (strict?1:0).
    let slack = if g.strict { 1 } else { 0 };
    match &g.bound {
        // bound == length(arr): need index_max <= bound ⇒ k - slack <= 0.
        IvBound::LenOf(g_arr) => *g_arr == arr && k - slack <= 0,
        // bound == n - headroom, length(arr) >= n + off: iv_max = n - headroom
        // - slack, so index_max = iv_max + k; need <= n + off
        // ⇒ k - slack - headroom <= off.
        IvBound::Var { var, headroom } => match length_def.get(&arr) {
            Some((n, off)) => *n == *var && k - slack - headroom <= *off,
            None => false,
        },
    }
}

/// True if `sym` is rebound (target of `Set`/`Let`) anywhere in `body`.
fn var_rebound_in(sym: Symbol, body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Set { target, .. } => *target == sym,
        Stmt::Let { var, .. } => *var == sym,
        Stmt::If { then_block, else_block, .. } => {
            var_rebound_in(sym, then_block)
                || matches!(else_block, Some(eb) if var_rebound_in(sym, eb))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => var_rebound_in(sym, body),
        _ => false,
    })
}

/// `value` is `iv + c` or `c + iv` with `c` a literal `>= 1`.
fn is_positive_increment(value: &Expr, iv: Symbol) -> bool {
    if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = value {
        let is_iv = |e: &Expr| matches!(e, Expr::Identifier(s) if *s == iv);
        let amt = match (&**left, &**right) {
            (l, Expr::Literal(Literal::Number(c))) if is_iv(l) => Some(*c),
            (Expr::Literal(Literal::Number(c)), r) if is_iv(r) => Some(*c),
            _ => None,
        };
        return amt.is_some_and(|c| c >= 1);
    }
    false
}

/// The loop's LAST top-level statement is `iv = iv + c` (c >= 1) and `iv` is
/// written NOWHERE else. Then every access before it sees the un-incremented
/// `iv`, so no per-access clobber tracking is needed and the induction is
/// monotone — the cleanest sound shape for region-entry hoisting.
fn iv_increment_is_last(iv: Symbol, body: &[Stmt]) -> bool {
    let Some((last, rest)) = body.split_last() else { return false };
    let last_is_inc = matches!(last, Stmt::Set { target, value }
        if *target == iv && is_positive_increment(value, iv));
    last_is_inc && !var_rebound_in(iv, rest)
}

/// Record a single covered store/read `item (iv+k) of arr` as speculatively
/// in bounds and fold its offset into the per-array `(kmin, kmax)`.
fn consider_hoist_access(
    collection: &Expr,
    index: &Expr,
    iv: Symbol,
    state: &RichAbstractState,
    body: &[Stmt],
    facts: &mut OracleFacts,
    per_array: &mut HashMap<Symbol, (i64, i64)>,
) {
    let Expr::Identifier(arr) = collection else { return };
    // The array must be a proven collection, stable (never rebound, never
    // resized — a realloc would stale the pinned pointer), and the index
    // affine in THIS loop's induction variable.
    if !state.coll_vars.contains(arr) || var_rebound_in(*arr, body) || array_resized_in(*arr, body) {
        return;
    }
    // Speculate ONLY on a function-PARAMETER collection — its length is
    // fundamentally unknown, so a region-entry runtime check is the right
    // tool. A locally-built array has a statically determinable length and is
    // left to the static path (speculating there would only ever deopt).
    if !state.param_colls.contains(arr) {
        return;
    }
    let Some((aiv, k)) = affine_of(index) else { return };
    if aiv != iv {
        return;
    }
    let key = index as *const Expr as usize;
    if facts.relational_inbounds.contains(&key) {
        return; // already proven statically — no runtime guard needed
    }
    facts.speculative_inbounds.insert(key);
    let entry = per_array.entry(*arr).or_insert((k, k));
    entry.0 = entry.0.min(k);
    entry.1 = entry.1.max(k);
}

/// Walk statements (and their expressions) collecting covered hoist accesses.
fn walk_hoist_accesses(
    stmts: &[Stmt],
    iv: Symbol,
    state: &RichAbstractState,
    body: &[Stmt],
    facts: &mut OracleFacts,
    per_array: &mut HashMap<Symbol, (i64, i64)>,
) {
    for s in stmts {
        for_each_direct_expr(s, &mut |e| {
            walk_hoist_in_expr(e, iv, state, body, facts, per_array)
        });
        if let Stmt::SetIndex { collection, index, .. } = s {
            consider_hoist_access(collection, index, iv, state, body, facts, per_array);
        }
        match s {
            Stmt::If { then_block, else_block, .. } => {
                walk_hoist_accesses(then_block, iv, state, body, facts, per_array);
                if let Some(eb) = else_block {
                    walk_hoist_accesses(eb, iv, state, body, facts, per_array);
                }
            }
            Stmt::While { body: b, .. } | Stmt::Repeat { body: b, .. } => {
                walk_hoist_accesses(b, iv, state, body, facts, per_array);
            }
            _ => {}
        }
    }
}

/// Find `item (iv+k) of arr` reads anywhere in an expression tree.
fn walk_hoist_in_expr(
    e: &Expr,
    iv: Symbol,
    state: &RichAbstractState,
    body: &[Stmt],
    facts: &mut OracleFacts,
    per_array: &mut HashMap<Symbol, (i64, i64)>,
) {
    use crate::ast::stmt::StringPart;
    if let Expr::Index { collection, index } = e {
        consider_hoist_access(collection, index, iv, state, body, facts, per_array);
    }
    let mut go = |x: &Expr| walk_hoist_in_expr(x, iv, state, body, facts, per_array);
    match e {
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            go(left);
            go(right);
        }
        Expr::Not { operand } => go(operand),
        Expr::Call { args, .. } => args.iter().for_each(|a| go(a)),
        Expr::CallExpr { callee, args } => {
            go(callee);
            args.iter().for_each(|a| go(a));
        }
        Expr::Index { collection, index } => {
            go(collection);
            go(index);
        }
        Expr::Slice { collection, start, end } => {
            go(collection);
            go(start);
            go(end);
        }
        Expr::Copy { expr } => go(expr),
        Expr::Give { value } => go(value),
        Expr::Length { collection } => go(collection),
        Expr::Contains { collection, value } => {
            go(collection);
            go(value);
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().for_each(|i| go(i)),
        Expr::FieldAccess { object, .. } => go(object),
        Expr::New { init_fields, .. } => init_fields.iter().for_each(|(_, fe)| go(fe)),
        Expr::NewVariant { fields, .. } => fields.iter().for_each(|(_, fe)| go(fe)),
        Expr::OptionSome { value } => go(value),
        Expr::WithCapacity { value, capacity } => {
            go(value);
            go(capacity);
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let StringPart::Expr { value, .. } = p {
                    go(value);
                }
            }
        }
        _ => {}
    }
}

/// SPECULATIVE region-entry hoist (V8 loop bound-check elimination): when a
/// loop's array length is not statically provable (e.g. a function parameter)
/// but the induction is monotone, the bound loop-invariant, and the array
/// stable, record the covered `item (iv+k) of arr` accesses as in-bounds and
/// emit one descriptor per array — justified at runtime by a single
/// region-entry check (`RegionBoundsGuard`), which the VM verifies before
/// entering native code (declining the region, and so the elision, on miss).
fn record_hoist_speculation(
    cond: &Expr,
    body: &[Stmt],
    state: &RichAbstractState,
    facts: &mut OracleFacts,
    loop_key: usize,
) {
    // Same body restriction as static elision: no resize, no call, nothing
    // unmodeled (any of which could change the array's length).
    if !body_is_index_proof_safe(body) {
        return;
    }
    let mut guards: Vec<IvGuard> = Vec::new();
    collect_iv_guards(cond, state, &mut guards);
    // One plain-variable guard `iv </<= bound`, bound loop-invariant, the
    // increment last (so monotone, no clobber tracking needed).
    let Some(g) = guards.iter().find(|g| {
        matches!(g.bound, IvBound::Var { headroom: 0, var } if !var_rebound_in(var, body))
            && iv_increment_is_last(g.iv, body)
    }) else {
        return;
    };
    let IvBound::Var { var: bound, .. } = g.bound else {
        return;
    };
    if bound == g.iv {
        return;
    }
    let mut per_array: HashMap<Symbol, (i64, i64)> = HashMap::new();
    walk_hoist_accesses(body, g.iv, state, body, facts, &mut per_array);
    if per_array.is_empty() {
        return;
    }
    let strict = if g.strict { 1 } else { 0 };
    let descs: Vec<HoistDesc> = per_array
        .into_iter()
        .filter_map(|(array, (kmin, kmax))| {
            Some(HoistDesc {
                array,
                bound,
                iv: g.iv,
                add_max: i32::try_from(kmax - strict).ok()?,
                add_min: i32::try_from(kmin).ok()?,
            })
        })
        .collect();
    if !descs.is_empty() {
        facts.hoist_descs.entry(loop_key).or_default().extend(descs);
    }
}

/// The length fact a counted build loop establishes for the array it fills.
enum BuildLength {
    /// Variable bound: `length(arr) >= n + off`.
    Symbolic(Symbol, Symbol, i64),
    /// Literal bound: `length(arr) == len` exactly (the interval check then
    /// proves reads directly).
    Concrete(Symbol, i64),
}

/// Recognize a counted build loop `while c </<= B: <fill arr once per
/// iteration>` from an EMPTY array, where `B` is a variable (→ a symbolic
/// length lower bound) or an integer literal (→ a concrete exact length) —
/// the standard allocation-size fact. Strict by construction: the body is
/// flat with exactly one `push to arr`, exactly one `c := c + 1`, `c`
/// starting at a known constant, and nothing else that resizes a collection
/// or branches (a conditional push would make the length too small).
fn infer_build_length(
    cond: &Expr,
    body: &[Stmt],
    entry: &RichAbstractState,
) -> Vec<BuildLength> {
    let (c_e, b_e, strict) = match cond {
        Expr::BinaryOp { op, left, right } => {
            let (l, r) = (&**left, &**right);
            match op {
                BinaryOpKind::Lt => (l, r, true),
                BinaryOpKind::LtEq => (l, r, false),
                _ => return Vec::new(),
            }
        }
        _ => return Vec::new(),
    };
    let Expr::Identifier(c) = c_e else { return Vec::new() };
    let c = *c;
    // The bound is a variable (symbolic) or an integer literal (concrete).
    let bound_var = match b_e {
        Expr::Identifier(n) => Some(*n),
        Expr::Literal(Literal::Number(_)) => None,
        _ => return Vec::new(),
    };
    if bound_var == Some(c) {
        return Vec::new();
    }
    // c starts at a known constant c0 at loop entry.
    let iv = entry.intervals.get_var(&c);
    let c0 = match (iv.lo, iv.hi) {
        (Bound::Finite(lo), Bound::Finite(hi)) if lo == hi => lo,
        _ => return Vec::new(),
    };
    // Pushes PER ARRAY: a MULTI-array build loop (graph_bfs's `Push adjStarts;
    // Push adjCounts; Push adj x5`) gives every array pushed EXACTLY once a
    // length equal to the trip count. An array pushed `K > 1` times has length
    // `K * n`, which `length_def`'s `(var, off)` form cannot hold, so it is
    // skipped (no false length fact). The body is still otherwise restricted to
    // pushes + the single increment — a branch or call undercounts.
    let mut push_counts: HashMap<Symbol, u32> = HashMap::new();
    let mut increments = 0;
    for s in body {
        match s {
            Stmt::Push { collection, .. } => {
                let Expr::Identifier(a) = &**collection else {
                    return Vec::new();
                };
                *push_counts.entry(*a).or_insert(0) += 1;
            }
            Stmt::Set { target, value } => {
                if *target == c {
                    if !is_increment_by_one(value, c) {
                        return Vec::new();
                    }
                    increments += 1;
                } else if bound_var == Some(*target) {
                    return Vec::new(); // the bound is mutated inside the loop
                }
            }
            Stmt::Let { var, .. } => {
                if *var == c || bound_var == Some(*var) {
                    return Vec::new();
                }
            }
            // Anything that could resize the array, branch (a conditional
            // push undercounts), or call out disqualifies the inference.
            Stmt::Show { .. }
            | Stmt::RuntimeAssert { .. }
            | Stmt::Assert { .. }
            | Stmt::Trust { .. } => {}
            _ => return Vec::new(),
        }
    }
    if increments != 1 {
        return Vec::new();
    }
    // trip count = B - c0 (for `<`) or B - c0 + 1 (for `<=`).
    let off = if strict { -c0 } else { -c0 + 1 };
    // Deterministic order (HashMap iteration is not) — sort by dense index.
    let mut arrays: Vec<(Symbol, u32)> = push_counts.into_iter().collect();
    arrays.sort_by_key(|(a, _)| a.index());
    let mut out = Vec::new();
    for (arr, count) in arrays {
        if count != 1 || arr == c || bound_var == Some(arr) {
            continue;
        }
        // The array must be empty at loop entry — a fresh allocation filled here.
        let entry_len = entry.intervals.get_length(&arr);
        if !matches!(
            (entry_len.lo, entry_len.hi),
            (Bound::Finite(0), Bound::Finite(0))
        ) {
            continue;
        }
        match (bound_var, b_e) {
            (Some(n), _) => out.push(BuildLength::Symbolic(arr, n, off)),
            (None, Expr::Literal(Literal::Number(bn))) => {
                out.push(BuildLength::Concrete(arr, (*bn + off).max(0)))
            }
            _ => {}
        }
    }
    out
}

/// `value` is exactly `c + 1` or `1 + c`.
fn is_increment_by_one(value: &Expr, c: Symbol) -> bool {
    if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = value {
        let is_c = |e: &Expr| matches!(e, Expr::Identifier(s) if *s == c);
        let is_one = |e: &Expr| matches!(e, Expr::Literal(Literal::Number(1)));
        return (is_c(&**left) && is_one(&**right)) || (is_one(&**left) && is_c(&**right));
    }
    false
}

/// True when the loop body contains only statements the recognizer can
/// reason about. Resizes (`Push`/`Pop`/`Add`/`Remove`) ARE allowed — a read
/// on a stable array stays sound while a DIFFERENT array grows (the common
/// "build B from A" loop); the per-array `array_resized_in` check keeps the
/// resized array's own reads checked. A CALL (which could resize anything) or
/// an unmodeled statement still disqualifies.
fn body_is_index_proof_safe(body: &[Stmt]) -> bool {
    body.iter().all(|s| match s {
        Stmt::Let { .. }
        | Stmt::Set { .. }
        | Stmt::SetIndex { .. }
        | Stmt::Push { .. }
        | Stmt::Pop { .. }
        | Stmt::Add { .. }
        | Stmt::Remove { .. }
        | Stmt::Show { .. }
        | Stmt::Return { .. }
        | Stmt::Break
        | Stmt::RuntimeAssert { .. }
        | Stmt::Assert { .. }
        | Stmt::Trust { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            body_is_index_proof_safe(then_block)
                && match else_block {
                    Some(eb) => body_is_index_proof_safe(eb),
                    None => true,
                }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => body_is_index_proof_safe(body),
        _ => false,
    })
}

/// Collect every collection grown/shrunk anywhere in `body`. Reads on these
/// stay CHECKED: a resize can realloc the buffer, and the JIT region pins the
/// pointer/length once at entry.
/// Arrays whose length could DECREASE in the loop (so an `i <= length(arr)`
/// guard, checked at the top of each iteration, may no longer hold at a read).
/// GROW-only ops (`Push`/`Add`) leave the length monotonically non-decreasing,
/// so the guard keeps holding — those do NOT make an array unsafe. Only SHRINK
/// (`Pop`/`Remove`) and ALIASING (a second live handle that could itself be
/// shrunk) can drop the length below the guard.
fn collect_resized_arrays(body: &[Stmt], out: &mut std::collections::HashSet<Symbol>) {
    for s in body {
        match s {
            // Grow-only: length never decreases — the bound survives. NOT
            // clobbered (this is the relaxation that proves a growing FIFO's
            // own cursor reads, e.g. graph_bfs's queue).
            Stmt::Push { .. } | Stmt::Add { .. } => {}
            // Shrink: length can drop below the guard.
            Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(c) = &**collection {
                    out.insert(*c);
                }
            }
            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(c) = &**collection {
                    out.insert(*c);
                }
                if let Some(v) = into {
                    out.insert(*v);
                }
            }
            // Aliasing: `Let b be a` / `Set b to a` makes `b` a second handle
            // on `a`'s allocation; a later `Pop b` would shrink `a` without
            // naming it, so the source `a` is no longer provably grow-only.
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => {
                if let Expr::Identifier(src) = &**value {
                    out.insert(*src);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                collect_resized_arrays(then_block, out);
                if let Some(eb) = else_block {
                    collect_resized_arrays(eb, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_resized_arrays(body, out);
            }
            _ => {}
        }
    }
}

/// True if `sym` is grown or shrunk (`Push`/`Pop`/`Add`/`Remove`) anywhere in
/// `body` — its length is not loop-stable, so its reads stay checked.
fn array_resized_in(sym: Symbol, body: &[Stmt]) -> bool {
    let hits = |c: &Expr| matches!(c, Expr::Identifier(s) if *s == sym);
    body.iter().any(|s| match s {
        Stmt::Push { collection, .. }
        | Stmt::Add { collection, .. }
        | Stmt::Remove { collection, .. } => hits(collection),
        Stmt::Pop { collection, into } => {
            hits(collection) || matches!(into, Some(v) if *v == sym)
        }
        Stmt::If { then_block, else_block, .. } => {
            array_resized_in(sym, then_block)
                || matches!(else_block, Some(eb) if array_resized_in(sym, eb))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => array_resized_in(sym, body),
        _ => false,
    })
}

/// Walk the body in execution order, recording each proven `item i of arr`
/// read up to the point a guard variable is clobbered. A nested loop
/// pre-marks every variable it clobbers (it repeats, so a later-iteration
/// write reaches an earlier-source read); a `Repeat`'s reads are not
/// recorded (its pattern rebinds the iteration variable).
fn record_proven_reads(
    stmts: &[Stmt],
    guards: &[IvGuard],
    length_def: &HashMap<Symbol, (Symbol, i64)>,
    clobbered: &mut std::collections::HashSet<Symbol>,
    facts: &mut OracleFacts,
) {
    for stmt in stmts {
        for_each_direct_expr(stmt, &mut |e| {
            record_index_reads(e, guards, length_def, clobbered, facts)
        });
        // A store `Set item E of arr to v` — prove the STORE index too (its
        // index is a direct expression, not an `Index` node a read walk sees).
        if let Stmt::SetIndex { collection, index, .. } = stmt {
            try_record_index(collection, index, guards, length_def, clobbered, facts);
        }
        match stmt {
            Stmt::If { then_block, else_block, .. } => {
                let mut c_then = clobbered.clone();
                record_proven_reads(then_block, guards, length_def, &mut c_then, facts);
                let mut c_else = clobbered.clone();
                if let Some(eb) = else_block {
                    record_proven_reads(eb, guards, length_def, &mut c_else, facts);
                }
                clobbered.extend(c_then);
                clobbered.extend(c_else);
            }
            Stmt::While { body, .. } => {
                let mut inner = clobbered.clone();
                for m in collect_mutations(body) {
                    inner.insert(m);
                }
                record_proven_reads(body, guards, length_def, &mut inner.clone(), facts);
                *clobbered = inner;
            }
            Stmt::Repeat { pattern, body, .. } => {
                if let Some(v) = pattern_loop_var(pattern) {
                    clobbered.insert(v);
                }
                for m in collect_mutations(body) {
                    clobbered.insert(m);
                }
            }
            _ => {}
        }
        // Only a rebinding of a variable invalidates subsequent proofs — an
        // element store (`SetIndex`) leaves the length untouched.
        match stmt {
            Stmt::Set { target, .. } => {
                clobbered.insert(*target);
            }
            Stmt::Let { var, .. } => {
                clobbered.insert(*var);
            }
            _ => {}
        }
    }
}

/// Apply `f` to each expression a statement evaluates directly (not those in
/// nested blocks — those are walked separately by `record_proven_reads`).
fn for_each_direct_expr(stmt: &Stmt, f: &mut impl FnMut(&Expr)) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => f(value),
        Stmt::SetIndex { collection, index, value } => {
            f(collection);
            f(index);
            f(value);
        }
        Stmt::Show { object, .. } => f(object),
        // The pushed/added value is an expression that may read other arrays
        // (e.g. `Push item li of left to result` — the "build B from A" loop).
        Stmt::Push { value, .. } | Stmt::Add { value, .. } | Stmt::Remove { value, .. } => f(value),
        Stmt::Return { value: Some(v) } => f(v),
        Stmt::RuntimeAssert { condition, .. } => f(condition),
        Stmt::If { cond, .. } => f(cond),
        Stmt::While { cond, decreasing, .. } => {
            f(cond);
            if let Some(d) = decreasing {
                f(d);
            }
        }
        Stmt::Repeat { iterable, .. } => f(iterable),
        _ => {}
    }
}

/// Walk `e`, recording the arena address of every `item E of arr` index
/// sub-expression a live guard proves in bounds (`E` affine in the guard's
/// induction variable, `arr` un-clobbered).
fn record_index_reads(
    e: &Expr,
    guards: &[IvGuard],
    length_def: &HashMap<Symbol, (Symbol, i64)>,
    clobbered: &std::collections::HashSet<Symbol>,
    facts: &mut OracleFacts,
) {
    use crate::ast::stmt::StringPart;
    if let Expr::Index { collection, index } = e {
        try_record_index(collection, index, guards, length_def, clobbered, facts);
    }
    match e {
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            record_index_reads(left, guards, length_def, clobbered, facts);
            record_index_reads(right, guards, length_def, clobbered, facts);
        }
        Expr::Not { operand } => record_index_reads(operand, guards, length_def, clobbered, facts),
        Expr::Call { args, .. } => {
            for a in args {
                record_index_reads(a, guards, length_def, clobbered, facts);
            }
        }
        Expr::CallExpr { callee, args } => {
            record_index_reads(callee, guards, length_def, clobbered, facts);
            for a in args {
                record_index_reads(a, guards, length_def, clobbered, facts);
            }
        }
        Expr::Index { collection, index } => {
            record_index_reads(collection, guards, length_def, clobbered, facts);
            record_index_reads(index, guards, length_def, clobbered, facts);
        }
        Expr::Slice { collection, start, end } => {
            record_index_reads(collection, guards, length_def, clobbered, facts);
            record_index_reads(start, guards, length_def, clobbered, facts);
            record_index_reads(end, guards, length_def, clobbered, facts);
        }
        Expr::Copy { expr } => record_index_reads(expr, guards, length_def, clobbered, facts),
        Expr::Give { value } => record_index_reads(value, guards, length_def, clobbered, facts),
        Expr::Length { collection } => record_index_reads(collection, guards, length_def, clobbered, facts),
        Expr::Contains { collection, value } => {
            record_index_reads(collection, guards, length_def, clobbered, facts);
            record_index_reads(value, guards, length_def, clobbered, facts);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for it in items {
                record_index_reads(it, guards, length_def, clobbered, facts);
            }
        }
        Expr::FieldAccess { object, .. } => record_index_reads(object, guards, length_def, clobbered, facts),
        Expr::New { init_fields, .. } => {
            for (_, fe) in init_fields {
                record_index_reads(fe, guards, length_def, clobbered, facts);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, fe) in fields {
                record_index_reads(fe, guards, length_def, clobbered, facts);
            }
        }
        Expr::OptionSome { value } => record_index_reads(value, guards, length_def, clobbered, facts),
        Expr::WithCapacity { value, capacity } => {
            record_index_reads(value, guards, length_def, clobbered, facts);
            record_index_reads(capacity, guards, length_def, clobbered, facts);
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let StringPart::Expr { value, .. } = p {
                    record_index_reads(value, guards, length_def, clobbered, facts);
                }
            }
        }
        _ => {}
    }
}

/// EXODIA Phase 1 entry point: analyze a parsed program and return the
/// per-expression fact table (Main statements AND every function body,
/// seeded from declared parameter types).
/// Apply `f` to each child statement block of `s` that shares (or nests under)
/// its scope — used by the program-wide scans the dense-map gate needs.
fn each_child_block(s: &Stmt, f: &mut impl FnMut(&[Stmt])) {
    match s {
        Stmt::If { then_block, else_block, .. } => {
            f(then_block);
            if let Some(eb) = else_block {
                f(eb);
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => f(body),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => f(tasks),
        Stmt::Inspect { arms, .. } => {
            for a in arms {
                f(a.body);
            }
        }
        Stmt::FunctionDef { body, .. } => f(body),
        _ => {}
    }
}

/// Symbols that are NOT program-invariant: every `Set` target, plus any symbol
/// bound by more than one `Let` (a shadow/rebind). A `with capacity` expression
/// built only from symbols OUTSIDE this set holds the same value at the map's
/// declaration as at every key site, so the dense array's runtime size equals
/// the capacity the bound proof reasons about. Conservative across scopes (a
/// name reused in two scopes counts as reassigned) — sound, at worst it declines
/// to optimize.
fn collect_reassigned(stmts: &[Stmt]) -> std::collections::HashSet<Symbol> {
    fn walk(
        stmts: &[Stmt],
        set_targets: &mut std::collections::HashSet<Symbol>,
        let_counts: &mut HashMap<Symbol, u32>,
    ) {
        for s in stmts {
            match s {
                Stmt::Set { target, .. } => {
                    set_targets.insert(*target);
                }
                Stmt::Let { var, .. } => {
                    *let_counts.entry(*var).or_insert(0) += 1;
                }
                _ => {}
            }
            each_child_block(s, &mut |b| walk(b, set_targets, let_counts));
        }
    }
    let mut set_targets = std::collections::HashSet::new();
    let mut let_counts: HashMap<Symbol, u32> = HashMap::new();
    walk(stmts, &mut set_targets, &mut let_counts);
    for (sym, count) in let_counts {
        if count > 1 {
            set_targets.insert(sym);
        }
    }
    set_targets
}

/// Record, per `Map`/`HashMap` local declared `… with capacity CAP`, the capacity
/// as a kernel `LinearExpr` — but only when CAP is affine and every variable it
/// names is program-invariant (`reassigned` excludes it). A map symbol defined by
/// two such `Let`s has an ambiguous capacity and is poisoned (never recorded).
/// Recording for non-`Int` maps is harmless: their keys are not affine integers,
/// so `try_prove_dense_key` never proves them and the (Int-only) codegen gate
/// never consults them.
fn gather_map_caps(
    stmts: &[Stmt],
    interner: &crate::intern::Interner,
    reassigned: &std::collections::HashSet<Symbol>,
    out: &mut HashMap<Symbol, super::affine::LinExpr>,
    poisoned: &mut std::collections::HashSet<Symbol>,
) {
    for s in stmts {
        if let Stmt::Let { var, value, .. } = s {
            if let Expr::WithCapacity { value: inner, capacity } = value {
                if let Expr::New { type_name, .. } = inner {
                    if matches!(interner.resolve(*type_name), "Map" | "HashMap") {
                        if let Some(cap_lin) = super::affine::lin_of(capacity) {
                            let mut cap_syms = Vec::new();
                            affine_collect_syms(capacity, &mut cap_syms);
                            let invariant = cap_syms.iter().all(|s| !reassigned.contains(s));
                            if poisoned.contains(var) {
                                // already ambiguous — leave unrecorded
                            } else if !invariant {
                                out.remove(var);
                                poisoned.insert(*var);
                            } else if out.insert(*var, cap_lin).is_some() {
                                // second WithCapacity definition of the same map
                                out.remove(var);
                                poisoned.insert(*var);
                            }
                        }
                    }
                }
            }
        }
        each_child_block(s, &mut |b| gather_map_caps(b, interner, reassigned, out, poisoned));
    }
}

/// The inclusive loop bound `B` of a counted condition `iv </<= B` whose counter
/// is a bare identifier — the candidate key-domain capacity for a map filled by
/// the loop. `None` for any other condition shape.
fn counted_loop_bound<'a>(cond: &'a Expr<'a>) -> Option<&'a Expr<'a>> {
    let Expr::BinaryOp { op, left, right } = cond else { return None };
    if !matches!(op, BinaryOpKind::Lt | BinaryOpKind::LtEq) {
        return None;
    }
    let Expr::Identifier(_) = *left else { return None };
    Some(*right)
}

/// Collect every `new Map`/`new HashMap` local created WITHOUT an explicit
/// capacity (a bare `Expr::New`) — the maps whose capacity, if any, must be
/// INFERRED from a fill loop rather than read off the declaration.
fn collect_bare_new_maps(
    stmts: &[Stmt],
    interner: &crate::intern::Interner,
    out: &mut std::collections::HashSet<Symbol>,
) {
    for s in stmts {
        if let Stmt::Let { var, value, .. } = s {
            if let Expr::New { type_name, .. } = value {
                if matches!(interner.resolve(*type_name), "Map" | "HashMap") {
                    out.insert(*var);
                }
            }
        }
        each_child_block(s, &mut |b| collect_bare_new_maps(b, interner, out));
    }
}

/// Record every bare-`new` map inserted into within `stmts` (a `Set item _ of m`),
/// restricted to the `candidates` set.
fn collect_loop_inserted_maps(
    stmts: &[Stmt],
    candidates: &std::collections::HashSet<Symbol>,
    out: &mut Vec<Symbol>,
) {
    for s in stmts {
        if let Stmt::SetIndex { collection: Expr::Identifier(m), .. } = s {
            if candidates.contains(m) {
                out.push(*m);
            }
        }
        each_child_block(s, &mut |b| collect_loop_inserted_maps(b, candidates, out));
    }
}

/// Capacity inference for a `Map of Int to Int` created with a BARE `new Map`
/// (no `with capacity`) but filled inside a counted loop: the loop bound `B` of a
/// `While iv </<= B` whose body inserts into the map is a candidate key-domain
/// capacity. The dense gate still PROVES every key `<= B` before lowering, so a
/// loose or unrelated `B` simply fails the proof and the map stays a hash table —
/// recording it is never a miscompile, only an opportunity (this is what unlocks
/// two_sum's `seen`, whose keys `x = item i of arr` and `complement = n - x` are
/// element-derived, not loop counters). `B` must be affine and program-invariant.
/// `or_insert` so an explicit `with capacity` already in `out` always wins, and
/// `poisoned` (ambiguous explicit caps) are left untouched.
fn gather_implicit_map_caps(
    stmts: &[Stmt],
    interner: &crate::intern::Interner,
    reassigned: &std::collections::HashSet<Symbol>,
    poisoned: &std::collections::HashSet<Symbol>,
    out: &mut HashMap<Symbol, super::affine::LinExpr>,
) {
    let mut bare = std::collections::HashSet::new();
    collect_bare_new_maps(stmts, interner, &mut bare);
    if bare.is_empty() {
        return;
    }
    fn walk(
        stmts: &[Stmt],
        bare: &std::collections::HashSet<Symbol>,
        reassigned: &std::collections::HashSet<Symbol>,
        poisoned: &std::collections::HashSet<Symbol>,
        out: &mut HashMap<Symbol, super::affine::LinExpr>,
    ) {
        for s in stmts {
            if let Stmt::While { cond, body, .. } = s {
                if let Some(bound) = counted_loop_bound(cond) {
                    if let Some(b_lin) = super::affine::lin_of(bound) {
                        let mut syms = Vec::new();
                        affine_collect_syms(bound, &mut syms);
                        if syms.iter().all(|x| !reassigned.contains(x)) {
                            let mut inserted = Vec::new();
                            collect_loop_inserted_maps(body, bare, &mut inserted);
                            for m in inserted {
                                if !poisoned.contains(&m) && !out.contains_key(&m) {
                                    out.insert(m, b_lin.clone());
                                }
                            }
                        }
                    }
                }
            }
            each_child_block(s, &mut |b| walk(b, bare, reassigned, poisoned, out));
        }
    }
    walk(stmts, &bare, reassigned, poisoned, out);
}

/// Render an affine capacity `LinExpr` as a Rust `i64` expression string — used to
/// size a dense map's direct-addressed array (`with_bounds(0, (cap) + 1)`) at its
/// constructor site. `None` if any coefficient or the constant is non-integer (no
/// clean rendering), in which case the dense gate declines and the map stays a
/// hash table. Terms are emitted in symbol-index order for determinism.
pub fn lin_to_rust(e: &super::affine::LinExpr, interner: &crate::intern::Interner) -> Option<String> {
    let int_of = |num: i64, den: i64| -> Option<i64> { (den == 1).then_some(num) };
    let mut coeffs: Vec<(i64, i64)> = Vec::new();
    for (idx, c) in e.coefficients.iter() {
        let ci = int_of(c.numerator, c.denominator)?;
        if ci != 0 {
            coeffs.push((*idx, ci));
        }
    }
    coeffs.sort_by_key(|&(idx, _)| idx);
    let mut terms: Vec<String> = Vec::new();
    for (idx, ci) in coeffs {
        let name = interner.resolve(Symbol::from_index(idx as usize));
        terms.push(if ci == 1 {
            name.to_string()
        } else {
            format!("{} * {}", ci, name)
        });
    }
    let k = int_of(e.constant.numerator, e.constant.denominator)?;
    if k != 0 || terms.is_empty() {
        terms.push(k.to_string());
    }
    Some(terms.join(" + "))
}

/// Tally how many `Set item _ of m to _` insert sites each map symbol has across
/// the program. A map with exactly one insert loop is eligible for the
/// full-coverage recognizer; more than one (or zero) is not.
fn count_insert_sites(stmts: &[Stmt], out: &mut HashMap<Symbol, u32>) {
    for s in stmts {
        if let Stmt::SetIndex { collection: Expr::Identifier(m), .. } = s {
            *out.entry(*m).or_insert(0) += 1;
        }
        each_child_block(s, &mut |b| count_insert_sites(b, out));
    }
}

/// Recognize a CONTIGUOUS, UNIT-STRIDE, UNCONDITIONAL insert loop over a map in
/// `map_caps`: `While iv </<= UB:` whose body is EXACTLY `Set item iv of m to _`
/// and `Set iv to iv + 1`. Such a loop writes `m[iv]` for every integer `iv` in
/// `[entry(iv), B]`, so the inserted key set is that whole contiguous range —
/// the gap-free coverage presence elision needs. Returns `(m, iv, B)` with `B`
/// the inclusive upper bound (`UB - 1` for `<`, `UB` for `<=`). Deliberately
/// strict: a conditional insert, a non-unit stride, a `break`, or any extra
/// statement leaves gaps and is rejected (no elision, presence bit kept).
fn match_insert_loop(
    cond: &Expr,
    body: &[Stmt],
    map_caps: &HashMap<Symbol, super::affine::LinExpr>,
) -> Option<(Symbol, Symbol, super::affine::LinExpr)> {
    use super::affine::{konst, lin_of};
    // cond: `iv < UB` (B = UB - 1) or `iv <= UB` (B = UB), iv a bare identifier.
    let Expr::BinaryOp { op, left, right } = cond else { return None };
    let Expr::Identifier(iv) = left else { return None };
    let b_lin = match op {
        BinaryOpKind::Lt => lin_of(right)?.sub(&konst(1)),
        BinaryOpKind::LtEq => lin_of(right)?,
        _ => return None,
    };
    if body.len() != 2 {
        return None;
    }
    let mut found_map: Option<Symbol> = None;
    let mut found_incr = false;
    for s in body {
        match s {
            // The unconditional insert `Set item iv of m to _`.
            Stmt::SetIndex { collection: Expr::Identifier(m), index: Expr::Identifier(ix), .. }
                if ix == iv && map_caps.contains_key(m) =>
            {
                if found_map.is_some() {
                    return None;
                }
                found_map = Some(*m);
            }
            // The unit increment `Set iv to iv + 1` (either operand order).
            Stmt::Set { target, value: Expr::BinaryOp { op: BinaryOpKind::Add, left: l, right: r } }
                if target == iv =>
            {
                let iv_plus_one = matches!((l, r),
                    (Expr::Identifier(a), Expr::Literal(Literal::Number(1))) if a == iv)
                    || matches!((l, r),
                        (Expr::Literal(Literal::Number(1)), Expr::Identifier(a)) if a == iv);
                if !iv_plus_one {
                    return None;
                }
                found_incr = true;
            }
            _ => return None,
        }
    }
    match (found_map, found_incr) {
        (Some(m), true) => Some((m, *iv, b_lin)),
        _ => None,
    }
}

/// Record, per map whose single insert loop fully covers a contiguous range, the
/// covered interval `[A, B]` — `A` the induction variable's constant entry value
/// (the most recent literal assignment in scope before the loop), `B` from the
/// loop guard. Only maps with EXACTLY one insert site are considered (so the one
/// loop accounts for every inserted key).
fn gather_insert_coverage(
    stmts: &[Stmt],
    map_caps: &HashMap<Symbol, super::affine::LinExpr>,
    insert_counts: &HashMap<Symbol, u32>,
    inits: &mut HashMap<Symbol, i64>,
    out: &mut HashMap<Symbol, (super::affine::LinExpr, super::affine::LinExpr)>,
) {
    use super::affine::konst;
    for s in stmts {
        if let Stmt::While { cond, body, .. } = s {
            if let Some((m, iv, b_lin)) = match_insert_loop(cond, body, map_caps) {
                if insert_counts.get(&m) == Some(&1) {
                    if let Some(&a) = inits.get(&iv) {
                        out.entry(m).or_insert((konst(a), b_lin));
                    }
                }
            }
        }
        each_child_block(s, &mut |b| {
            gather_insert_coverage(b, map_caps, insert_counts, &mut inits.clone(), out)
        });
        // Track constant inits in execution order so a loop sees its IV's entry
        // value; any non-literal (re)assignment clears the fact.
        match s {
            Stmt::Let { var, value: Expr::Literal(Literal::Number(n)), .. } => {
                inits.insert(*var, *n);
            }
            Stmt::Let { var, .. } => {
                inits.remove(var);
            }
            Stmt::Set { target, value: Expr::Literal(Literal::Number(n)) } => {
                inits.insert(*target, *n);
            }
            Stmt::Set { target, .. } => {
                inits.remove(target);
            }
            _ => {}
        }
    }
}

pub fn oracle_analyze(stmts: &[Stmt]) -> OracleFacts {
    let mut facts = OracleFacts::default();
    let mut st = RichAbstractState::new();
    rich_walk_block(stmts, &mut st, &mut facts);
    // Function bodies: parameters carry their declared scalar types.
    for s in stmts {
        if let Stmt::FunctionDef { params, body, .. } = s {
            let mut fst = RichAbstractState::new();
            for (psym, ty) in params.iter() {
                if let crate::ast::stmt::TypeExpr::Primitive(t) = ty {
                    // Declared types arrive as interned names; tag the ones
                    // the scalar lattice models.
                    fst.types.insert(*psym, type_tag_for_name(*t));
                }
                // Two parameters may be handed the same allocation by a
                // caller — unknown provenance for the alias domain.
                fst.aliases.taint(*psym);
            }
            rich_walk_block(body, &mut fst, &mut facts);
        }
    }
    strip_concurrent_loop_snapshots(stmts, &mut facts, false);
    facts
}

/// Map a declared primitive type NAME to a type-domain fact (Top when the
/// name is outside the scalar lattice). Resolved structurally — the interner
/// is unavailable here, so compare against the few candidate tags by trying
/// each known name through the type registry convention.
fn type_tag_for_name(_name: Symbol) -> TypeAbstraction {
    // Without the interner, names cannot be resolved to strings here;
    // declared-type seeding happens in oracle_analyze_with via the interner.
    TypeAbstraction::Top
}

/// [`oracle_analyze`] with the interner, enabling declared-parameter type
/// seeds inside function bodies. Used by every NON-AOT consumer (VM bytecode,
/// copy-and-patch JIT, e-graph, UI) — these get NO entry-guard precondition, so
/// the partition's accesses stay behind a runtime `RegionBoundsGuard` rather
/// than being statically elided on a precondition the bytecode never enforces.
pub fn oracle_analyze_with(stmts: &[Stmt], interner: &crate::intern::Interner) -> OracleFacts {
    oracle_analyze_with_opts(stmts, interner, false)
}

/// [`oracle_analyze_with`] for the `largo build` AOT codegen ONLY. Enables the
/// recursive-1-based-partition entry-guard precondition (`1 <= lo`,
/// `hi <= len`) and alias `length_def` propagation — sound here because the AOT
/// codegen emits the matching runtime `assert!` at the function entry. The
/// VM/JIT path emits no such assert, so it must use [`oracle_analyze_with`].
pub fn oracle_analyze_with_entry_guards(
    stmts: &[Stmt],
    interner: &crate::intern::Interner,
) -> OracleFacts {
    oracle_analyze_with_opts(stmts, interner, true)
}

/// Shared body of the two public entry points. `aot_entry_guard` gates the
/// entry-guard-precondition facts to the AOT path that enforces them.
fn oracle_analyze_with_opts(
    stmts: &[Stmt],
    interner: &crate::intern::Interner,
    aot_entry_guard: bool,
) -> OracleFacts {
    let mut facts = OracleFacts::default();
    // Dense-map gate (precondition): record each `Map of Int to Int … with
    // capacity CAP`'s invariant affine capacity BEFORE the loop walks, so
    // `try_prove_dense_key` can relate a key to it while the loop guards are live.
    {
        let reassigned = collect_reassigned(stmts);
        let mut poisoned = std::collections::HashSet::new();
        gather_map_caps(stmts, interner, &reassigned, &mut facts.map_caps, &mut poisoned);
        // Implicit capacity: a bare `new Map` filled by a counted loop takes that
        // loop's bound as a candidate key-domain cap (proof-gated downstream) —
        // so a `Map of Int to Int` written without `with capacity` can still go
        // dense (two_sum's `seen`). Explicit caps above already win via `or_insert`.
        gather_implicit_map_caps(stmts, interner, &reassigned, &poisoned, &mut facts.map_caps);
        // Presence elision precondition: which dense maps have a single insert
        // loop that fully covers a contiguous key range.
        let mut insert_counts = HashMap::new();
        count_insert_sites(stmts, &mut insert_counts);
        let mut inits = HashMap::new();
        gather_insert_coverage(stmts, &facts.map_caps, &insert_counts, &mut inits, &mut facts.map_insert_cover);
    }
    // DECLARED primitive return types: a call to one of these functions
    // (native or user-defined) produces a typed value — a static fact the
    // kernel enforces dynamically.
    let mut fn_returns: HashMap<Symbol, TypeTag> = HashMap::new();
    for s in stmts {
        if let Stmt::FunctionDef { name, return_type: Some(ty), .. } = s {
            if let crate::ast::stmt::TypeExpr::Primitive(t)
            | crate::ast::stmt::TypeExpr::Named(t) = ty
            {
                let tag = match interner.resolve(*t) {
                    "Int" => Some(TypeTag::Int),
                    "Float" => Some(TypeTag::Float),
                    "Bool" => Some(TypeTag::Bool),
                    "Text" => Some(TypeTag::Text),
                    _ => None,
                };
                if let Some(tag) = tag {
                    fn_returns.insert(*name, tag);
                }
            }
        }
    }
    let fn_returns = std::rc::Rc::new(fn_returns);
    let mut st = RichAbstractState::new();
    st.fn_returns = fn_returns.clone();
    rich_walk_block(stmts, &mut st, &mut facts);
    for s in stmts {
        if let Stmt::FunctionDef { params, body, .. } = s {
            let mut fst = RichAbstractState::new();
            fst.fn_returns = fn_returns.clone();
            fst.aot_entry_guard = aot_entry_guard;
            for (psym, ty) in params.iter() {
                match ty {
                    crate::ast::stmt::TypeExpr::Primitive(t) => {
                        let tag = match interner.resolve(*t) {
                            "Int" => TypeAbstraction::Concrete(TypeTag::Int),
                            "Float" => TypeAbstraction::Concrete(TypeTag::Float),
                            "Bool" => TypeAbstraction::Concrete(TypeTag::Bool),
                            "Text" => TypeAbstraction::Concrete(TypeTag::Text),
                            _ => TypeAbstraction::Top,
                        };
                        fst.types.insert(*psym, tag);
                    }
                    // An ordered-collection parameter (`arr: Seq of Int`): a
                    // proven collection of unknown length — the speculative
                    // region-entry hoist's target.
                    crate::ast::stmt::TypeExpr::Generic { base, .. }
                        if matches!(interner.resolve(*base), "Seq" | "List" | "Array") =>
                    {
                        fst.coll_vars.insert(*psym);
                        fst.param_colls.insert(*psym);
                    }
                    _ => {}
                }
                // Two parameters may be handed the same allocation by a
                // caller — unknown provenance for the alias domain.
                fst.aliases.taint(*psym);
            }
            // Recursive 1-based partition precondition (quicksort/Lomuto): the
            // function is only ever entered with `1 <= lo` and `hi <= length(arr)`
            // — the contract `codegen::entry_guard` asserts at runtime for these
            // pure functions. Seeding those facts lets the relational BCE
            // discharge the partition's `item j of arr` / `item i of arr` accesses
            // (`1 <= i <= j < hi <= len`), a relation the interval domain alone
            // cannot express. Sound: every access sits past the `lo < hi` base
            // case, exactly where the runtime guard has already enforced these.
            // AOT ONLY — the VM/JIT bytecode path emits no entry guard, so it
            // must not see these unenforced facts (else it would drop a
            // `RegionBoundsGuard` it actually needs).
            if aot_entry_guard {
                if let Some(g) =
                    crate::codegen::entry_guard::detect_entry_guard(params, body, interner)
                {
                    fst.intervals
                        .set_var(g.lo, Interval { lo: Bound::Finite(1), hi: Bound::PosInf });
                    fst.intervals
                        .set_var(g.hi, Interval { lo: Bound::Finite(1), hi: Bound::PosInf });
                    fst.length_def.insert(g.arr, (g.hi, 0));
                }
            }
            rich_walk_block(body, &mut fst, &mut facts);
        }
    }
    strip_concurrent_loop_snapshots(stmts, &mut facts, false);
    facts
}

/// Loops under Concurrent/Parallel blocks run interleaved: the sequential
/// alias walk is not a sound model of their entry states, so their
/// snapshots are withheld and the distinctness queries refuse.
fn strip_concurrent_loop_snapshots(stmts: &[Stmt], facts: &mut OracleFacts, in_concurrent: bool) {
    for stmt in stmts {
        match stmt {
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if in_concurrent {
                    facts.loop_aliases.remove(&(stmt as *const Stmt as usize));
                }
                strip_concurrent_loop_snapshots(body, facts, in_concurrent);
            }
            Stmt::If { then_block, else_block, .. } => {
                strip_concurrent_loop_snapshots(then_block, facts, in_concurrent);
                if let Some(eb) = else_block {
                    strip_concurrent_loop_snapshots(eb, facts, in_concurrent);
                }
            }
            Stmt::Zone { body, .. } => {
                strip_concurrent_loop_snapshots(body, facts, in_concurrent);
            }
            Stmt::FunctionDef { body, .. } => {
                strip_concurrent_loop_snapshots(body, facts, in_concurrent);
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    strip_concurrent_loop_snapshots(arm.body, facts, in_concurrent);
                }
            }
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                strip_concurrent_loop_snapshots(tasks, facts, true);
            }
            _ => {}
        }
    }
}

pub(crate) fn rich_abstract_interp_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    _expr_arena: &'a Arena<Expr<'a>>,
    _stmt_arena: &'a Arena<Stmt<'a>>,
) -> (Vec<Stmt<'a>>, RichAbstractState) {
    let mut st = RichAbstractState::new();
    let mut facts = OracleFacts::default();
    rich_walk_block(&stmts, &mut st, &mut facts);
    (stmts, st)
}

fn rich_walk_block(block: &[Stmt], st: &mut RichAbstractState, facts: &mut OracleFacts) {
    for stmt in block {
        record_stmt_exprs(stmt, st, facts);
        rich_walk_stmt(stmt, st, facts);
    }
}

/// Record facts for every expression a statement holds, at the PRE-state
/// (the state its expressions evaluate in).
fn record_stmt_exprs(stmt: &Stmt, st: &RichAbstractState, facts: &mut OracleFacts) {
    use crate::ast::stmt::ReadSource;
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => record_expr(value, st, facts),
        Stmt::Return { value: Some(e) } => record_expr(e, st, facts),
        Stmt::Call { args, .. } => {
            for a in args {
                record_expr(a, st, facts);
            }
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => record_expr(cond, st, facts),
        Stmt::Repeat { iterable, .. } => record_expr(iterable, st, facts),
        Stmt::Show { object, .. } | Stmt::Give { object, .. } => record_expr(object, st, facts),
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            record_expr(value, st, facts);
            record_expr(collection, st, facts);
        }
        Stmt::SetIndex { collection, index, value } => {
            record_expr(collection, st, facts);
            record_expr(index, st, facts);
            record_expr(value, st, facts);
        }
        Stmt::SetField { object, value, .. } => {
            record_expr(object, st, facts);
            record_expr(value, st, facts);
        }
        Stmt::Inspect { target, .. } => record_expr(target, st, facts),
        Stmt::RuntimeAssert { condition, .. } => record_expr(condition, st, facts),
        Stmt::Sleep { milliseconds } => record_expr(milliseconds, st, facts),
        Stmt::ReadFrom { source: ReadSource::File(p), .. } => record_expr(p, st, facts),
        _ => {}
    }
}

fn rich_walk_stmt(stmt: &Stmt, st: &mut RichAbstractState, facts: &mut OracleFacts) {
    match stmt {
        Stmt::Let { var, value, .. } => rich_bind(*var, value, st),
        Stmt::Set { target, value } => rich_bind(*target, value, st),
        Stmt::Push { collection, value } | Stmt::Add { collection, value } => {
            rich_grow(collection, st);
            observe_written_elem(collection, value, st);
        }
        Stmt::Pop { collection, into } => {
            rich_shrink(collection, st);
            if let Some(v) = into {
                st.invalidate_var(*v);
                st.aliases.unlink(*v);
                // A popped element may be any handle ever stored in the
                // container — unknown provenance.
                st.aliases.taint(*v);
            }
        }
        Stmt::Remove { collection, .. } => rich_shrink(collection, st),
        // A store leaves the length unchanged but adds an element value: join
        // it into the collection's element bound (the scatter `Set item k of
        // arr to V` case).
        Stmt::SetIndex { collection, value, .. } => observe_written_elem(collection, value, st),
        Stmt::SetField { object, .. } => {
            if let Expr::Identifier(s) = *object {
                for a in st.aliases.may_alias(*s) {
                    st.shapes.insert(a, CollectionShape::Top);
                }
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            rich_walk_if(cond, then_block, *else_block, st, facts);
        }
        Stmt::While { cond, body, .. } => {
            let key = stmt as *const Stmt as usize;
            rich_walk_loop(Some(cond), body, st, None, facts, Some(key));
        }
        Stmt::Repeat { pattern, body, .. } => {
            let key = stmt as *const Stmt as usize;
            rich_walk_loop(None, body, st, pattern_loop_var(pattern), facts, Some(key));
        }
        Stmt::Inspect { target, arms, .. } => rich_walk_inspect(target, arms, st, facts),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            rich_walk_block(tasks, st, facts)
        }
        Stmt::Zone { body, .. } => {
            // Walk the zone body so aliasing established inside it is visible
            // and loops within it get borrow-hoist snapshots — but suppress
            // per-expression fact recording: the EXODIA region/JIT compiler
            // consumes those and assumes none exist inside zones. State
            // updates and loop_aliases snapshots stay live; only the
            // exprs/lengths/collections tables are held back.
            let prev = facts.suppress_exprs;
            facts.suppress_exprs = true;
            rich_walk_block(body, st, facts);
            facts.suppress_exprs = prev;
        }
        Stmt::Call { args, .. } => {
            // A callee may resize any collection it is handed: forget their sizes.
            for arg in args {
                if let Expr::Identifier(s) = *arg {
                    for a in st.aliases.may_alias(*s) {
                        st.shapes.insert(a, CollectionShape::Top);
                        st.intervals.set_length(a, Interval::non_negative());
                    }
                }
            }
        }
        Stmt::Give { object, .. } => {
            if let Expr::Identifier(s) = *object {
                st.invalidate_var(*s);
                st.aliases.unlink(*s);
            }
        }
        Stmt::ReadFrom { var, .. } => {
            st.invalidate_var(*var);
            st.aliases.unlink(*var);
        }
        _ => {}
    }
}

/// `Let v be value.` / `Set v to value.` — rebinds `v`, so its old aliasing is
/// severed and all five facts are recomputed from `value`.
/// Record an element written into a collection (`Push`/`Add`/`SetIndex`):
/// join the value's interval into the element bound of every handle that
/// aliases the collection (a write through one alias is visible through all).
/// The SYMBOLIC upper bound of `value` as a linear expression over program
/// variables, or `None` if no useful one is known. This is the variable-divisor
/// sibling of `eval_expr`'s concrete interval upper: it exists precisely for the
/// bounds a concrete `Interval` cannot hold. A `(...) % n` with a VARIABLE `n`
/// is `<= n - 1` (the truncated remainder satisfies this whenever `n >= 1`,
/// which the `while i < n` fill loop guarantees and an empty array makes
/// vacuous); a scalar carries its `scalar_upper`; an element read carries its
/// array's `elem_upper`; a numeric literal is its own bound.
fn value_upper(value: &Expr, st: &RichAbstractState) -> Option<super::affine::LinExpr> {
    use super::affine::{konst, var};
    match value {
        Expr::Literal(Literal::Number(k)) => Some(konst(*k)),
        Expr::Identifier(s) => st.scalar_upper.get(s).cloned(),
        Expr::Index { collection, .. } => match collection {
            Expr::Identifier(arr) => st.elem_upper.get(arr).cloned(),
            _ => None,
        },
        Expr::BinaryOp { op: BinaryOpKind::Modulo, right, .. } => match right {
            Expr::Identifier(n) => Some(var(*n).sub(&konst(1))),
            _ => None,
        },
        _ => None,
    }
}

/// The symbolic LOWER bound of `value`, the mirror of [`value_upper`]. An
/// element read carries its array's proven concrete element interval lower (the
/// `% n`-filled array's `0`); a scalar carries its `scalar_lower`; a literal is
/// its own bound. Captured here because a loop-local read variable's raw
/// interval is flooded to ⊤ by its undefined entry value, losing this lower.
fn value_lower(value: &Expr, st: &RichAbstractState) -> Option<super::affine::LinExpr> {
    use super::affine::konst;
    match value {
        Expr::Literal(Literal::Number(k)) => Some(konst(*k)),
        Expr::Identifier(s) => st.scalar_lower.get(s).cloned(),
        Expr::Index { collection, .. } => match collection {
            Expr::Identifier(arr) => match st.intervals.get_elem(arr).lo {
                Bound::Finite(lo) => Some(konst(lo)),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Merge two symbolic element-upper bounds under the "EVERY element satisfies
/// it" reading: the result must dominate BOTH. Equal bounds (the uniform fill —
/// every store is the same `% n`) are kept; otherwise the one provably the
/// larger in the current interval state wins (so a constant zero-init folds into
/// the symbolic `n - 1`), and the bound drops to ⊤ when the two are unordered.
fn join_sym_upper(
    a: &super::affine::LinExpr,
    b: &super::affine::LinExpr,
    st: &RichAbstractState,
) -> Option<super::affine::LinExpr> {
    if a == b {
        return Some(a.clone());
    }
    if lin_ge_zero(&b.sub(a), st) {
        return Some(b.clone()); // b - a >= 0 ⟹ b is the weaker (larger) upper
    }
    if lin_ge_zero(&a.sub(b), st) {
        return Some(a.clone());
    }
    // LENIENT: a non-positive constant element (the zero-init of a build-then-
    // scatter array) joins into a `% m` element upper `m - 1`. Sound under
    // `m >= 1`: then `c <= 0 <= m - 1`, so every element satisfies `m - 1`
    // (vacuous when the array is empty). The `m >= 1` precondition is discharged
    // by a nonemptiness-guarded `assert!(m > 0)` at any elision that consumes the
    // bound (see `try_prove_affine` / `positivity_guards`). Restricted to the
    // `m - 1` shape so the divisor to guard is unambiguous.
    if is_nonpos_const(a) && mod_upper_divisor(b).is_some() {
        return Some(b.clone());
    }
    if is_nonpos_const(b) && mod_upper_divisor(a).is_some() {
        return Some(a.clone());
    }
    None
}

/// `Some(m)` when `e` is the variable-divisor modulo upper `m - 1` (a single
/// variable with coefficient `+1` and constant `-1`) — the bound `value_upper`
/// emits for `(...) % m`. Its `m >= 1` precondition is what the positivity guard
/// discharges.
fn mod_upper_divisor(e: &super::affine::LinExpr) -> Option<Symbol> {
    if e.coefficients.len() != 1 || e.constant.numerator != -1 || e.constant.denominator != 1 {
        return None;
    }
    let (&idx, coeff) = e.coefficients.iter().next()?;
    (coeff.numerator == 1 && coeff.denominator == 1).then(|| Symbol::from_index(idx as usize))
}

/// Is `e` a constant `<= 0` (no variables, non-positive)?
fn is_nonpos_const(e: &super::affine::LinExpr) -> bool {
    e.coefficients.is_empty() && e.constant.numerator <= 0
}

/// Is the linear expression `e` provably `>= 0` under the current per-variable
/// intervals? Evaluates `const + Σ coeff·interval(var)` and tests the lower
/// bound is a finite non-negative. A non-integer rational or a variable whose
/// interval is unbounded below (for a positive coefficient) fails closed.
fn lin_ge_zero(e: &super::affine::LinExpr, st: &RichAbstractState) -> bool {
    let as_int = |num: i64, den: i64| -> Option<i64> { (den == 1).then_some(num) };
    let Some(k) = as_int(e.constant.numerator, e.constant.denominator) else { return false };
    let mut acc = Interval::exact(k);
    for (idx, coeff) in &e.coefficients {
        let Some(c) = as_int(coeff.numerator, coeff.denominator) else { return false };
        let sym = Symbol::from_index(*idx as usize);
        acc = acc.add(&Interval::exact(c).mul(&st.intervals.get_var(&sym)));
    }
    matches!(acc.lo, Bound::Finite(v) if v >= 0)
}

fn observe_written_elem(collection: &Expr, value: &Expr, st: &mut RichAbstractState) {
    if let Expr::Identifier(sym) = collection {
        let v = eval_expr(value, &st.intervals);
        // The symbolic element upper of the written value, computed BEFORE the
        // mutations below (it reads other variables' bounds, never `sym`'s).
        let sym_up = value_upper(value, st);
        // The scalar TYPE of the written value, likewise BEFORE the mutations
        // (it reads `types`/`elem_type`, never the array's own future state).
        let sym_ty = eval_type(value, &st.types, &st.fn_returns, &st.elem_type);
        for a in st.aliases.may_alias(*sym) {
            // Freshness BEFORE the concrete update: a fresh array (⊥ element
            // interval) is SEEDED by the first write; a non-fresh one JOINS.
            let was_fresh = st.intervals.get_elem(&a).is_bottom();
            st.intervals.observe_elem(a, v.clone());
            // Element TYPE: every element must satisfy the join over all writes.
            // A fresh array is seeded; a non-fresh one joins. An unknown write
            // type (`Top`) forces the element type to ⊤, exactly as the symbolic
            // upper does — once an element's type is unknown, all reads are ⊤.
            if was_fresh {
                st.elem_type.insert(a, sym_ty.clone());
            } else {
                let joined = match st.elem_type.get(&a) {
                    Some(existing) => existing.join(&sym_ty),
                    None => TypeAbstraction::Top,
                };
                st.elem_type.insert(a, joined);
            }
            match &sym_up {
                // An unbounded written value forces the array's element upper to
                // ⊤ — every element must satisfy the bound, and this one doesn't.
                None => {
                    st.elem_upper.remove(&a);
                }
                Some(nb) => {
                    if was_fresh {
                        st.elem_upper.insert(a, nb.clone());
                    } else if let Some(existing) = st.elem_upper.get(&a).cloned() {
                        match join_sym_upper(&existing, nb, st) {
                            Some(m) => {
                                st.elem_upper.insert(a, m);
                            }
                            None => {
                                st.elem_upper.remove(&a);
                            }
                        }
                    }
                    // existing absent + non-fresh = ⊤ (a prior element was
                    // unbounded); a bounded write cannot recover it.
                }
            }
        }
    }
}

fn rich_bind(var: Symbol, value: &Expr, st: &mut RichAbstractState) {
    st.aliases.unlink(var);
    // Writing `var` invalidates any length fact it participates in: the array
    // itself is rebound, or a size variable `n` in `length(arr) >= n + off`
    // is reassigned (so the old relation no longer holds).
    st.length_def.remove(&var);
    st.length_def.retain(|_, (n, _)| *n != var);
    // Likewise any affine scalar definition that names `var` (as the defined
    // variable or anywhere in another's right-hand side) is now stale.
    st.scalar_def.remove(&var);
    let vi = var.index() as i64;
    st.scalar_def.retain(|_, e| !e.coefficients.contains_key(&vi));
    // Symbolic bounds that name `var` (its own scalar upper, the element upper
    // of a rebound collection, or any bound whose RHS mentions `var`) are now
    // stale — drop them exactly like the affine scalar definitions above.
    st.scalar_upper.remove(&var);
    st.scalar_upper.retain(|_, e| !e.coefficients.contains_key(&vi));
    st.elem_upper.remove(&var);
    st.elem_upper.retain(|_, e| !e.coefficients.contains_key(&vi));
    // The element TYPE of a rebound collection is stale; its values can't name
    // another variable, so only its own entry needs dropping (re-seeded below
    // for a fresh/aliased collection by the element-write observers).
    st.elem_type.remove(&var);
    st.scalar_lower.remove(&var);
    st.scalar_lower.retain(|_, e| !e.coefficients.contains_key(&vi));
    // Rebinding resets `var`'s extern-length status; the producer arms below
    // re-establish it for aliases/copies/slices of an extern-length array.
    st.param_colls.remove(&var);

    let iv = eval_expr(value, &st.intervals);
    st.intervals.set_var(var, iv);
    let value_ty = eval_type(value, &st.types, &st.fn_returns, &st.elem_type);
    st.types.insert(var, value_ty);
    st.nullability.insert(var, nullability_of_expr(value, st));
    // Rebinding discards the old binding's element bound; the producer arms
    // below re-establish it for fresh/aliased collections.
    st.intervals.clear_elem(&var);
    // Record an affine scalar definition `var = E` (E linear over ≥1 variable)
    // for the LIA bounds prover — a constant value is already in the interval
    // domain, so only variable-bearing forms are worth an equality fact. NEVER
    // record a SELF-REFERENTIAL update (`Set n to n + 10`): as an equality
    // `n = n + 10` it is the contradiction `0 = 10`, which would make the LIA
    // system inconsistent and prove every bound vacuously (unsound).
    if let Some(e) = super::affine::lin_of(value) {
        if !e.coefficients.is_empty() && !e.coefficients.contains_key(&(var.index() as i64)) {
            st.scalar_def.insert(var, e);
        }
    }
    // Symbolic UPPER bound for the new binding — `Let u be item _ of adj` carries
    // `adj`'s element upper, `Let neighbor be (...) % n` carries `n - 1`. A
    // self-referential bound (`Set x to x % n`) would be unsound to keep, so a
    // result naming `var` is dropped.
    if let Some(u) = value_upper(value, st) {
        if !u.coefficients.contains_key(&(var.index() as i64)) {
            st.scalar_upper.insert(var, u);
        }
    }
    if let Some(l) = value_lower(value, st) {
        if !l.coefficients.contains_key(&(var.index() as i64)) {
            st.scalar_lower.insert(var, l);
        }
    }
    // A3 LENGTH BINDING: `Let n be length of A` ⟹ `length(A) = n`, recorded as
    // the symbolic lower bound `length(A) >= n + 0` — the symmetric fact to a
    // build loop's `length_def`. Lets the bounds prover relate an index bounded
    // by `n` back to `A`'s length (string_search: `text[i+j]` with
    // `i+j <= textLen = length(text)`). Invalidated when `A` is resized/rebound
    // or `n` is reassigned, exactly like the build-loop fact. Holds for a String
    // too — its byte length is what the `as_bytes()[..]` access checks.
    if let Expr::Length { collection } = value {
        if let Expr::Identifier(a) = &**collection {
            if *a != var {
                st.length_def.insert(*a, (var, 0));
            }
        }
    }

    match value {
        Expr::New { .. } => {
            st.shapes.insert(var, CollectionShape::Empty);
            st.intervals.set_length(var, Interval::exact(0));
            st.coll_vars.insert(var);
            // A fresh empty collection has NO elements: element bound `⊥`, so
            // the first `Push` seeds an exact bound and a 0-trip build loop's
            // exit join keeps the filled branch's bound.
            st.intervals.observe_elem(var, Interval::bottom());
        }
        Expr::List(items) | Expr::Tuple(items) => {
            let n = items.len() as u64;
            st.shapes.insert(var, CollectionShape::from_bounds(n, Some(n)));
            st.intervals.set_length(var, Interval::exact(items.len() as i64));
            st.coll_vars.insert(var);
            // Element bound = join of the literal items (`⊥` for an empty list).
            let mut el = Interval::bottom();
            for it in items.iter() {
                el = el.join(&eval_expr(it, &st.intervals));
            }
            st.intervals.observe_elem(var, el);
            // Element TYPE = join of the item types; an empty list stays fresh
            // (absent), so its later writes seed it exactly.
            let mut item_iter = items.iter();
            if let Some(first) = item_iter.next() {
                let mut ty = eval_type(first, &st.types, &st.fn_returns, &st.elem_type);
                for it in item_iter {
                    ty = ty.join(&eval_type(it, &st.types, &st.fn_returns, &st.elem_type));
                }
                st.elem_type.insert(var, ty);
            }
        }
        Expr::Identifier(src) => {
            // `v` and `src` are now two names for one allocation.
            st.aliases.link(var, *src);
            let shape = st.shapes.get(src).cloned().unwrap_or(CollectionShape::Top);
            st.shapes.insert(var, shape);
            let len = st.intervals.get_length(src);
            st.intervals.set_length(var, len);
            // The alias shares the element bound of its source.
            if st.intervals.elem.contains_key(src) {
                let e = st.intervals.get_elem(src);
                st.intervals.observe_elem(var, e);
            }
            // …and the source's element TYPE (one allocation, same elements).
            if let Some(t) = st.elem_type.get(src).cloned() {
                st.elem_type.insert(var, t);
            }
            if st.coll_vars.contains(src) {
                st.coll_vars.insert(var);
                // An alias of an extern-length array is itself extern-length.
                if st.param_colls.contains(src) {
                    st.param_colls.insert(var);
                }
            } else {
                st.coll_vars.remove(&var);
            }
            // An alias shares its source's symbolic length lower bound (one
            // allocation): `length(src) >= n` ⟹ `length(var) >= n`. The quicksort
            // partition reads `result` after `Let mutable result be arr`, so the
            // entry-guard fact on `arr` must flow to `result`. A later resize of
            // either name drops it (`rich_grow`/`rich_shrink` walk `may_alias`).
            // AOT-only: this rides on the entry-guard precondition, which only the
            // AOT codegen enforces — gated so the VM/JIT oracle is unchanged.
            if st.aot_entry_guard {
                if let Some(&(n, off)) = st.length_def.get(src) {
                    if n != var {
                        st.length_def.insert(var, (n, off));
                    }
                }
            }
        }
        Expr::Copy { expr } | Expr::WithCapacity { value: expr, .. } => {
            // A fresh unaliased value of the OPERAND's kind.
            st.shapes.insert(var, CollectionShape::Top);
            let from_coll = matches!(
                expr,
                Expr::Identifier(s) if st.coll_vars.contains(s)
            ) || matches!(expr, Expr::New { .. } | Expr::List(_) | Expr::Slice { .. });
            if from_coll {
                st.coll_vars.insert(var);
            } else {
                st.coll_vars.remove(&var);
            }
        }
        Expr::Slice { .. } => {
            // A slice that evaluates is always a fresh list.
            st.shapes.insert(var, CollectionShape::Top);
            st.coll_vars.insert(var);
        }
        _ => {
            // Unmodeled producer — kind unknown.
            st.shapes.insert(var, CollectionShape::Top);
            st.coll_vars.remove(&var);
            // Provenance: scalar and definitely-fresh producers stay
            // alias-tracked; anything that can RETURN AN EXISTING HANDLE —
            // calls, container extraction (Index/FieldAccess), opaque
            // escapes — taints the binding (it may alias anything).
            match value {
                Expr::Literal(_)
                | Expr::BinaryOp { .. }
                | Expr::Not { .. }
                | Expr::Length { .. }
                | Expr::Contains { .. }
                | Expr::Range { .. }
                | Expr::Union { .. }
                | Expr::Intersection { .. }
                | Expr::InterpolatedString(_)
                | Expr::OptionNone => {}
                _ => st.aliases.taint(var),
            }
        }
    }
}

/// `Push`/`Add` — one element added to a shared allocation: grow the shape and
/// length of the collection and every variable that aliases it.
fn rich_grow(collection: &Expr, st: &mut RichAbstractState) {
    if let Expr::Identifier(sym) = collection {
        let new_shape = st.shapes.get(sym).cloned().unwrap_or(CollectionShape::Top).pushed();
        let new_len = st.intervals.get_length(sym).add(&Interval::exact(1));
        for a in st.aliases.may_alias(*sym) {
            st.shapes.insert(a, new_shape.clone());
            st.intervals.set_length(a, new_len.clone());
            // A resize moves length off its build-time relation — drop it
            // (conservative; a grow keeps `>=` valid but we re-derive cleanly).
            st.length_def.remove(&a);
        }
    }
}

/// `Pop`/`Remove` — one element removed from a shared allocation.
fn rich_shrink(collection: &Expr, st: &mut RichAbstractState) {
    if let Expr::Identifier(sym) = collection {
        let new_shape = st.shapes.get(sym).cloned().unwrap_or(CollectionShape::Top).popped();
        let new_len = st.intervals.get_length(sym).sub(&Interval::exact(1));
        for a in st.aliases.may_alias(*sym) {
            st.shapes.insert(a, new_shape.clone());
            st.intervals.set_length(a, new_len.clone());
            // A shrink can break the `length(arr) >= n` lower bound — drop it.
            st.length_def.remove(&a);
        }
    }
}

fn nullability_of_expr(expr: &Expr, st: &RichAbstractState) -> Nullability {
    match expr {
        Expr::Literal(lit) => Nullability::for_literal(lit),
        Expr::Identifier(s) => st.nullability.get(s).cloned().unwrap_or(Nullability::Maybe),
        Expr::New { .. }
        | Expr::NewVariant { .. }
        | Expr::List(_)
        | Expr::Tuple(_)
        | Expr::Range { .. }
        | Expr::BinaryOp { .. }
        | Expr::Not { .. }
        | Expr::Length { .. }
        | Expr::Contains { .. } => Nullability::Definite,
        // Index/Call/FieldAccess may yield `nothing`: stay conservative.
        _ => Nullability::Maybe,
    }
}

fn pattern_loop_var(p: &Pattern) -> Option<Symbol> {
    match p {
        Pattern::Identifier(s) => Some(*s),
        _ => None,
    }
}

fn rich_walk_if(
    cond: &Expr,
    then_block: &[Stmt],
    else_block: Option<&[Stmt]>,
    st: &mut RichAbstractState,
    facts: &mut OracleFacts,
) {
    match eval_condition(cond, &st.intervals) {
        Some(true) => {
            narrow_state(cond, &mut st.intervals);
            rich_walk_block(then_block, st, facts);
        }
        Some(false) => {
            if let Some(eb) = else_block {
                narrow_state_negated(cond, &mut st.intervals);
                rich_walk_block(eb, st, facts);
            }
        }
        None => {
            let mut then_st = st.clone();
            narrow_state(cond, &mut then_st.intervals);
            rich_walk_block(then_block, &mut then_st, facts);

            let mut else_st = st.clone();
            narrow_state_negated(cond, &mut else_st.intervals);
            if let Some(eb) = else_block {
                rich_walk_block(eb, &mut else_st, facts);
            }

            *st = rich_join(&then_st, &else_st);
        }
    }
}

/// Refine a loop-INVARIANT bound variable's interval lower from the guard the
/// body entry implies: `i < n` with `i` at its pre-loop value `i0` proves
/// `n > i0` (`n >= i0 + 1`); `i <= n` proves `n >= i0`. Sound because `n` is not
/// mutated, so the relation established at first entry holds for the whole body.
/// Descends through `and` and accepts the flipped `n > i` / `n >= i` forms.
fn refine_invariant_bound_from_guard(
    cond: &Expr,
    pre: &RichAbstractState,
    mutated: &[Symbol],
    out: &mut AbstractState,
) {
    if let Expr::BinaryOp { op: BinaryOpKind::And, left, right } = cond {
        refine_invariant_bound_from_guard(left, pre, mutated, out);
        refine_invariant_bound_from_guard(right, pre, mutated, out);
        return;
    }
    let Expr::BinaryOp { op, left, right } = cond else { return };
    let (iv_e, bnd_e, strict) = match op {
        BinaryOpKind::Lt => (left, right, true),
        BinaryOpKind::LtEq => (left, right, false),
        BinaryOpKind::Gt => (right, left, true),
        BinaryOpKind::GtEq => (right, left, false),
        _ => return,
    };
    let (Expr::Identifier(iv), Expr::Identifier(bnd)) = (iv_e, bnd_e) else { return };
    if mutated.contains(bnd) {
        return; // the bound must be loop-invariant for the relation to persist
    }
    let Bound::Finite(i0) = pre.intervals.get_var(iv).lo else { return };
    let Some(new_lo) = (if strict { i0.checked_add(1) } else { Some(i0) }) else { return };
    let cur = out.get_var(bnd);
    out.set_var(
        *bnd,
        Interval { lo: Bound::max_bound(&cur.lo, &Bound::Finite(new_lo)), hi: cur.hi },
    );
}

/// Widening-to-fixpoint loop analysis (EXODIA 1.1): iterate the body
/// transfer, widening the mutated variables' facts between passes through
/// the threshold ladder; on convergence the recorded body facts hold at the
/// loop invariant. A non-converging loop (cap hit) falls back to the sound
/// invalidate-everything treatment for whatever is still unstable.
fn rich_walk_loop(
    cond: Option<&Expr>,
    body: &[Stmt],
    st: &mut RichAbstractState,
    loop_var: Option<Symbol>,
    facts: &mut OracleFacts,
    loop_key: Option<usize>,
) {
    let mut mutated = collect_mutations(body);
    if let Some(lv) = loop_var {
        if !mutated.contains(&lv) {
            mutated.push(lv);
        }
    }
    // Repeat loop variables hold unknown element values inside the body —
    // and the element handle could be ANY handle stored in the iterable.
    let mut inside = st.clone();
    if let Some(lv) = loop_var {
        inside.invalidate_var(lv);
        inside.aliases.unlink(lv);
        inside.aliases.taint(lv);
    }
    if let Some(c) = cond {
        narrow_state(c, &mut inside.intervals);
        // The loop body implies its guard at first entry: `i < n` with `i` at
        // its pre-loop value `i0` proves `n > i0`, i.e. `n >= i0 + 1`. Since `n`
        // is loop-invariant (not mutated), this lower holds throughout the body
        // — it is what lets a `% n` element bound `n - 1` dominate a constant
        // zero-init in the symbolic element join (graph_bfs's adjacency array).
        refine_invariant_bound_from_guard(c, st, &mutated, &mut inside.intervals);
    }

    // Fixpoint over the mutated set: facts recorded into a SINK during the
    // ascent (intermediate states are not loop invariants).
    let mut sink = OracleFacts::default();
    let mut converged = false;
    for pass in 0..12 {
        let mut next = inside.clone();
        rich_walk_block(body, &mut next, &mut sink);
        if let Some(c) = cond {
            narrow_state(c, &mut next.intervals);
        }
        // Widen the mutated variables; everything else keeps the entry fact
        // (sound: un-mutated vars are loop-invariant).
        let mut stable = true;
        for &m in &mutated {
            // Captured BEFORE the concrete element merge below overwrites the
            // ⊥-ness: a fresh entry seeds the symbolic element upper from `next`.
            let inside_fresh_elem = inside.intervals.get_elem(&m).is_bottom();
            let cur_iv = inside.intervals.get_var(&m);
            let nxt_iv = next.intervals.get_var(&m);
            let wid = cur_iv.widen(&nxt_iv);
            if !wid.leq(&cur_iv) || !cur_iv.leq(&wid) {
                stable = false;
            }
            inside.intervals.set_var(m, wid);
            let cur_len = inside.intervals.get_length(&m);
            let nxt_len = next.intervals.get_length(&m);
            let wid_len = cur_len.widen(&nxt_len);
            inside.intervals.set_length(m, wid_len);

            // Element bound: DELAYED widening. Join for the first few passes so
            // a bound fed by an external converging value settles EXACTLY
            // (`(seed/65536) % 1000 → [0,999]`, which a threshold widen would
            // round up to 1000 and lose by one), then widen so a SELF-feeding
            // accumulator (`counts[v] += 1`, growing by 1 each pass) escapes to
            // ±∞ and converges instead of pinning the loop to the iteration cap
            // (which forces the blunt invalidate-everything fallback, losing the
            // length fact). A wider element bound is always sound.
            if inside.intervals.elem.contains_key(&m) || next.intervals.elem.contains_key(&m) {
                let cur_el = inside.intervals.get_elem(&m);
                let nxt_el = next.intervals.get_elem(&m);
                let merged = if pass < 3 {
                    cur_el.join(&nxt_el)
                } else {
                    cur_el.widen(&nxt_el)
                };
                if !(merged.leq(&cur_el) && cur_el.leq(&merged)) {
                    stable = false;
                }
                inside.intervals.elem.insert(m, merged);
            }

            // Symbolic element upper — the variable-divisor sibling of the
            // concrete elem above. Seeded from `next` on a fresh entry, then an
            // equality/domination merge (a symbolic LinExpr is its own fixed
            // point, so no widening); drops to ⊤ on disagreement.
            let merged_sym = if inside_fresh_elem {
                next.elem_upper.get(&m).cloned()
            } else {
                match (inside.elem_upper.get(&m).cloned(), next.elem_upper.get(&m).cloned()) {
                    (Some(e), Some(n)) => join_sym_upper(&e, &n, &next),
                    _ => None,
                }
            };
            if inside.elem_upper.get(&m) != merged_sym.as_ref() {
                stable = false;
            }
            match merged_sym {
                Some(l) => {
                    inside.elem_upper.insert(m, l);
                }
                None => {
                    inside.elem_upper.remove(&m);
                }
            }
            // Element TYPE — the type sibling of the concrete elem above. Seeded
            // from `next` on a fresh entry, then JOINED across the back-edge. The
            // type lattice is finite-height (`widen == join`, capped union), so a
            // plain join converges; a divergent type lands at ⊤ (sound: every
            // element must satisfy the join, so a wider type only forfeits the
            // elision). Stability participates in the fixpoint check.
            let merged_et = if inside_fresh_elem {
                next.elem_type.get(&m).cloned()
            } else {
                match (inside.elem_type.get(&m).cloned(), next.elem_type.get(&m).cloned()) {
                    (Some(e), Some(n)) => Some(e.join(&n)),
                    _ => None,
                }
            };
            if inside.elem_type.get(&m) != merged_et.as_ref() {
                stable = false;
            }
            match merged_et {
                Some(t) => {
                    inside.elem_type.insert(m, t);
                }
                None => {
                    inside.elem_type.remove(&m);
                }
            }
            // A mutated SCALAR's `scalar_upper` (the element/modulo bound on `u =
            // item _ of adj`, re-established by its in-body `Let` every pass) IS
            // a loop invariant when stable: build it up from the first defining
            // pass, keep it while it holds, drop it the moment it diverges or
            // vanishes (so the fixpoint converges instead of oscillating).
            let merged_su = match (
                inside.scalar_upper.get(&m).cloned(),
                next.scalar_upper.get(&m).cloned(),
            ) {
                (Some(a), Some(b)) if a == b => Some(a),
                (None, Some(b)) => Some(b),
                _ => None,
            };
            if inside.scalar_upper.get(&m) != merged_su.as_ref() {
                stable = false;
            }
            match merged_su {
                Some(l) => {
                    inside.scalar_upper.insert(m, l);
                }
                None => {
                    inside.scalar_upper.remove(&m);
                }
            }
            let merged_sl = match (
                inside.scalar_lower.get(&m).cloned(),
                next.scalar_lower.get(&m).cloned(),
            ) {
                (Some(a), Some(b)) if a == b => Some(a),
                (None, Some(b)) => Some(b),
                _ => None,
            };
            if inside.scalar_lower.get(&m) != merged_sl.as_ref() {
                stable = false;
            }
            match merged_sl {
                Some(l) => {
                    inside.scalar_lower.insert(m, l);
                }
                None => {
                    inside.scalar_lower.remove(&m);
                }
            }

            let cur_ty = inside.types.get(&m).cloned().unwrap_or(TypeAbstraction::Top);
            let nxt_ty = next.types.get(&m).cloned().unwrap_or(TypeAbstraction::Top);
            let j = cur_ty.join(&nxt_ty);
            if !(j.leq(&cur_ty) && cur_ty.leq(&j)) {
                stable = false;
            }
            inside.types.insert(m, j);

            let cur_sh = inside.shapes.get(&m).cloned().unwrap_or(CollectionShape::Top);
            let nxt_sh = next.shapes.get(&m).cloned().unwrap_or(CollectionShape::Top);
            inside.shapes.insert(m, cur_sh.widen(&nxt_sh));

            // Kind survives growth, but a rebinding inside the body
            // (absent from `next`) drops the proof.
            if !next.coll_vars.contains(&m) {
                inside.coll_vars.remove(&m);
            }

            let cur_n = inside.nullability.get(&m).cloned().unwrap_or(Nullability::Maybe);
            let nxt_n = next.nullability.get(&m).cloned().unwrap_or(Nullability::Maybe);
            inside.nullability.insert(m, cur_n.join(&nxt_n));
        }
        // Loop-CARRIED aliasing: an edge created at the end of one iteration
        // (`Set prev to curr`) holds at the top of the next, so the alias
        // graph joins by union across the back-edge. Monotone over a finite
        // symbol set — converges. Growth participates in the stability check.
        if inside.aliases.union_from(&next.aliases) {
            stable = false;
        }
        if stable {
            converged = true;
            break;
        }
    }

    if converged {
        // The converged alias graph IS the loop invariant — snapshot it for
        // the distinctness queries (borrow hoisting). Non-converged loops
        // record nothing: queries refuse by default.
        if let Some(key) = loop_key {
            match facts.loop_aliases.entry(key) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    e.get_mut().union_from(&inside.aliases);
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(inside.aliases.clone());
                }
            }
        }
        // Record body facts at the invariant UNDER the loop condition (the
        // body only runs when it holds), then build the after-state: the
        // loop may run zero times (entry state) or exit from the invariant
        // — join, then apply the negated condition.
        let mut record_state = inside.clone();
        if let Some(c) = cond {
            narrow_state(c, &mut record_state.intervals);
        }
        // RELATIONAL induction-variable bound (V8/LLVM SCEV): a guard that
        // bounds an index variable by an array's length lets every
        // `item i of arr` read in the body skip its bounds check. Then the
        // SPECULATIVE hoist: arrays whose length is not statically known get a
        // single region-entry runtime check instead.
        if let Some(c) = cond {
            record_loop_index_bounds(c, body, &record_state, facts);
            // GENERAL multi-variable proof (kernel LIA): catches the affine
            // indices the single-var recognizer above cannot — knapsack's
            // `prev[w-wi+1]` under a path guard, nested-loop windows, scatter
            // indices bounded through element ranges. Idempotent with the
            // single-var path (both feed the same `relational_inbounds` set).
            record_affine_index_bounds(c, body, &record_state, st, facts);
            if let Some(key) = loop_key {
                record_hoist_speculation(c, body, &record_state, facts, key);
            }
        }
        rich_walk_block(body, &mut record_state, facts);
        let mut after = rich_join(st, &inside);
        if let Some(c) = cond {
            narrow_state_negated(c, &mut after.intervals);
            // Allocation-size fact: a counted build loop establishes a length
            // bound on the array it fills (`st` is the pre-loop entry state —
            // the array's emptiness and the counter's start are read there).
            // A variable bound gives a symbolic lower bound (consumed by the
            // relational recognizer); a literal bound gives a concrete exact
            // length (consumed by the interval bounds check directly).
            for bl in infer_build_length(c, body, st) {
                match bl {
                    BuildLength::Symbolic(arr, n, off) => {
                        after.length_def.insert(arr, (n, off));
                    }
                    BuildLength::Concrete(arr, len) => {
                        after.intervals.set_length(arr, Interval::exact(len));
                    }
                }
            }
        }
        *st = after;
    } else {
        // Sound fallback: forget everything the body can write.
        if let Some(c) = cond {
            narrow_state_negated(c, &mut st.intervals);
        }
        for m in mutated {
            st.intervals.set_var(m, Interval::top());
            st.intervals.set_length(m, Interval::non_negative());
            st.types.insert(m, TypeAbstraction::Top);
            st.shapes.insert(m, CollectionShape::Top);
            st.nullability.insert(m, Nullability::Maybe);
            st.coll_vars.remove(&m);
        }
    }
}

fn rich_walk_inspect(
    target: &Expr,
    arms: &[MatchArm],
    st: &mut RichAbstractState,
    facts: &mut OracleFacts,
) {
    if arms.is_empty() {
        return;
    }
    let mut acc: Option<RichAbstractState> = None;
    for arm in arms {
        let mut arm_st = st.clone();
        // A concrete-variant arm proves the scrutinee (and its field bindings)
        // present — this is what lets the unwrap/`nothing` guard be removed.
        if arm.variant.is_some() {
            if let Expr::Identifier(sym) = target {
                arm_st.nullability.insert(*sym, Nullability::Definite);
            }
            for (_field, binding) in &arm.bindings {
                arm_st.nullability.insert(*binding, Nullability::Definite);
            }
        }
        // Field bindings extract handles from the scrutinee — unknown
        // provenance for the alias domain.
        for (_field, binding) in &arm.bindings {
            arm_st.aliases.unlink(*binding);
            arm_st.aliases.taint(*binding);
        }
        rich_walk_block(arm.body, &mut arm_st, facts);
        acc = Some(match acc {
            None => arm_st,
            Some(prev) => rich_join(&prev, &arm_st),
        });
    }
    if let Some(joined) = acc {
        *st = joined;
    }
}

fn rich_join(a: &RichAbstractState, b: &RichAbstractState) -> RichAbstractState {
    let mut intervals = AbstractState::new();
    join_states(&mut intervals, &a.intervals, &b.intervals);
    RichAbstractState {
        intervals,
        types: join_maps(&a.types, &b.types),
        shapes: join_maps(&a.shapes, &b.shapes),
        nullability: join_maps(&a.nullability, &b.nullability),
        aliases: alias_union(&a.aliases, &b.aliases),
        // Kind proven only where BOTH paths prove it.
        coll_vars: a.coll_vars.intersection(&b.coll_vars).cloned().collect(),
        fn_returns: a.fn_returns.clone(),
        // A length fact survives a join only where both paths agree exactly.
        length_def: a
            .length_def
            .iter()
            .filter(|(k, v)| b.length_def.get(k) == Some(v))
            .map(|(k, v)| (*k, *v))
            .collect(),
        // Parameter-collection membership is a function-entry constant.
        param_colls: a.param_colls.union(&b.param_colls).copied().collect(),
        // A scalar definition survives a join only where both paths define it
        // to the SAME affine expression.
        scalar_def: a
            .scalar_def
            .iter()
            .filter(|(k, v)| b.scalar_def.get(*k) == Some(*v))
            .map(|(k, v)| (*k, v.clone()))
            .collect(),
        // Symbolic upper bounds survive a join only where both paths carry the
        // SAME bound — a divergent (or absent-on-one-side) bound is ⊤. Sound:
        // dropping a bound only forfeits an elision, never licenses one.
        scalar_upper: a
            .scalar_upper
            .iter()
            .filter(|(k, v)| b.scalar_upper.get(*k) == Some(*v))
            .map(|(k, v)| (*k, v.clone()))
            .collect(),
        elem_upper: join_elem_upper_maps(a, b),
        elem_type: join_elem_type_maps(a, b),
        scalar_lower: a
            .scalar_lower
            .iter()
            .filter(|(k, v)| b.scalar_lower.get(*k) == Some(*v))
            .map(|(k, v)| (*k, v.clone()))
            .collect(),
        // A function-entry constant (both joined states are the same body), so
        // it survives every merge and reaches the post-branch alias binds.
        aot_entry_guard: a.aot_entry_guard || b.aot_entry_guard,
    }
}

/// Join the per-collection symbolic element uppers under the "EVERY element
/// satisfies it" reading, mirroring the concrete element interval's ⊥-seed: a
/// side whose array is ⊥ (a fresh `new Seq`, no elements) contributes nothing,
/// so the OTHER side's bound carries across the 0-iteration exit branch of a
/// build loop. When both sides hold elements the bounds must agree (or one
/// dominate); otherwise the bound is lost to ⊤.
fn join_elem_upper_maps(
    a: &RichAbstractState,
    b: &RichAbstractState,
) -> HashMap<Symbol, super::affine::LinExpr> {
    let mut out = HashMap::new();
    let keys: HashSet<Symbol> = a.elem_upper.keys().chain(b.elem_upper.keys()).cloned().collect();
    for key in keys {
        let a_bot = a.intervals.get_elem(&key).is_bottom();
        let b_bot = b.intervals.get_elem(&key).is_bottom();
        let v = if a_bot {
            b.elem_upper.get(&key).cloned()
        } else if b_bot {
            a.elem_upper.get(&key).cloned()
        } else {
            match (a.elem_upper.get(&key), b.elem_upper.get(&key)) {
                (Some(x), Some(y)) => join_sym_upper(x, y, b),
                _ => None,
            }
        };
        if let Some(l) = v {
            out.insert(key, l);
        }
    }
    out
}

/// Join the per-collection element TYPES under the "EVERY element satisfies it"
/// reading, mirroring [`join_elem_upper_maps`]'s ⊥-seed: a side whose array is ⊥
/// (a fresh `new Seq`, no elements) contributes nothing, so the OTHER side's type
/// carries across the 0-iteration exit branch of a build loop. When both sides
/// hold elements the types JOIN (a divergent pair lands at ⊤, sound).
fn join_elem_type_maps(
    a: &RichAbstractState,
    b: &RichAbstractState,
) -> HashMap<Symbol, TypeAbstraction> {
    let mut out = HashMap::new();
    let keys: HashSet<Symbol> = a.elem_type.keys().chain(b.elem_type.keys()).cloned().collect();
    for key in keys {
        let a_bot = a.intervals.get_elem(&key).is_bottom();
        let b_bot = b.intervals.get_elem(&key).is_bottom();
        let v = if a_bot {
            b.elem_type.get(&key).cloned()
        } else if b_bot {
            a.elem_type.get(&key).cloned()
        } else {
            match (a.elem_type.get(&key), b.elem_type.get(&key)) {
                (Some(x), Some(y)) => Some(x.join(y)),
                _ => None,
            }
        };
        if let Some(t) = v {
            out.insert(key, t);
        }
    }
    out
}

fn join_maps<V: AbstractDomain>(
    a: &HashMap<Symbol, V>,
    b: &HashMap<Symbol, V>,
) -> HashMap<Symbol, V> {
    let mut out = HashMap::new();
    let keys: HashSet<Symbol> = a.keys().chain(b.keys()).cloned().collect();
    for k in keys {
        let top = V::top();
        let va = a.get(&k).unwrap_or(&top);
        let vb = b.get(&k).unwrap_or(&top);
        out.insert(k, va.join(vb));
    }
    out
}

fn alias_union(a: &AliasGraph, b: &AliasGraph) -> AliasGraph {
    let mut out = a.clone();
    out.union_from(b);
    out
}

fn collect_mutations(block: &[Stmt]) -> Vec<Symbol> {
    let mut out = Vec::new();
    for s in block {
        collect_mut_stmt(s, &mut out);
    }
    out
}

fn add_unique(sym: Symbol, out: &mut Vec<Symbol>) {
    if !out.contains(&sym) {
        out.push(sym);
    }
}

fn collect_mut_stmt(s: &Stmt, out: &mut Vec<Symbol>) {
    match s {
        Stmt::Set { target, .. } => add_unique(*target, out),
        Stmt::Let { var, .. } => add_unique(*var, out),
        Stmt::Push { collection, .. }
        | Stmt::Add { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::SetIndex { collection, .. } => {
            if let Expr::Identifier(sym) = *collection {
                add_unique(*sym, out);
            }
        }
        Stmt::Pop { collection, into } => {
            if let Expr::Identifier(sym) = *collection {
                add_unique(*sym, out);
            }
            if let Some(v) = into {
                add_unique(*v, out);
            }
        }
        Stmt::SetField { object, .. } => {
            if let Expr::Identifier(sym) = *object {
                add_unique(*sym, out);
            }
        }
        Stmt::ReadFrom { var, .. } => add_unique(*var, out),
        Stmt::If { then_block, else_block, .. } => {
            for st in *then_block {
                collect_mut_stmt(st, out);
            }
            if let Some(eb) = else_block {
                for st in *eb {
                    collect_mut_stmt(st, out);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            for st in *body {
                collect_mut_stmt(st, out);
            }
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                for st in arm.body {
                    collect_mut_stmt(st, out);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod domain_tests {
    use super::*;

    fn iv(lo: i64, hi: i64) -> Interval {
        Interval { lo: Bound::Finite(lo), hi: Bound::Finite(hi) }
    }

    #[test]
    fn interval_top_and_bottom() {
        let t = <Interval as AbstractDomain>::top();
        assert!(matches!(t.lo, Bound::NegInf));
        assert!(matches!(t.hi, Bound::PosInf));
        assert!(!t.is_bottom());

        let b = <Interval as AbstractDomain>::bottom();
        assert!(b.is_bottom());
    }

    #[test]
    fn interval_join_is_least_upper_bound() {
        // [1,3] ⊔ [5,7] = [1,7]
        let j = iv(1, 3).join(&iv(5, 7));
        assert_eq!(j.lo, Bound::Finite(1));
        assert_eq!(j.hi, Bound::Finite(7));
        // bottom is the identity for join: ⊥ ⊔ x = x
        let jb = Interval::bottom().join(&iv(2, 4));
        assert_eq!(jb.lo, Bound::Finite(2));
        assert_eq!(jb.hi, Bound::Finite(4));
        let jb2 = iv(2, 4).join(&Interval::bottom());
        assert_eq!(jb2.lo, Bound::Finite(2));
        assert_eq!(jb2.hi, Bound::Finite(4));
    }

    #[test]
    fn interval_meet_is_greatest_lower_bound() {
        // [1,5] ⊓ [3,7] = [3,5]
        let m = iv(1, 5).meet(&iv(3, 7));
        assert_eq!(m.lo, Bound::Finite(3));
        assert_eq!(m.hi, Bound::Finite(5));
        // disjoint intervals meet to ⊥
        assert!(iv(1, 2).meet(&iv(5, 6)).is_bottom());
        // top is the identity for meet: ⊤ ⊓ x = x
        let t = Interval::top().meet(&iv(2, 4));
        assert_eq!(t.lo, Bound::Finite(2));
        assert_eq!(t.hi, Bound::Finite(4));
    }

    #[test]
    fn interval_widen_climbs_the_threshold_ladder_then_to_infinity() {
        // CONTRACT UPDATE (EXODIA M9, plan-approved): widening no longer
        // jumps straight to ±∞ — an unstable bound snaps to the next rung
        // of the threshold ladder [-1000..1000], and only past the last
        // rung falls off to ±∞. Loop counters keep finite bounds.
        // rising upper bound snaps to the next rung, stable lower preserved
        let w = iv(0, 5).widen(&iv(0, 10));
        assert_eq!(w.lo, Bound::Finite(0));
        assert_eq!(w.hi, Bound::Finite(10));
        let w_mid = iv(0, 5).widen(&iv(0, 11));
        assert_eq!(w_mid.hi, Bound::Finite(100));
        // beyond the last rung → +∞
        let w_inf = iv(0, 5).widen(&iv(0, 1001));
        assert!(matches!(w_inf.hi, Bound::PosInf));
        // falling lower bound snaps to the next rung down, upper preserved
        let w2 = iv(0, 5).widen(&iv(-3, 5));
        assert_eq!(w2.lo, Bound::Finite(-10));
        assert_eq!(w2.hi, Bound::Finite(5));
        // below the last rung → -∞
        let w2_inf = iv(0, 5).widen(&iv(-1001, 5));
        assert!(matches!(w2_inf.lo, Bound::NegInf));
        // stable interval is a fixpoint (no spurious widening)
        let w3 = iv(0, 5).widen(&iv(0, 5));
        assert_eq!(w3.lo, Bound::Finite(0));
        assert_eq!(w3.hi, Bound::Finite(5));
        // widening from ⊥ yields the new value
        let w4 = Interval::bottom().widen(&iv(2, 4));
        assert_eq!(w4.lo, Bound::Finite(2));
        assert_eq!(w4.hi, Bound::Finite(4));
        // the ascent is FINITE: iterated widening against ever-growing
        // inputs stabilizes within ladder-length steps (termination).
        let mut cur = iv(0, 1);
        let mut steps = 0;
        loop {
            let grown = Interval {
                lo: cur.lo.clone(),
                hi: match cur.hi {
                    Bound::Finite(v) => Bound::Finite(v.saturating_add(v.abs() + 1)),
                    ref b => b.clone(),
                },
            };
            let next = cur.widen(&grown);
            steps += 1;
            if next.leq(&cur) && cur.leq(&next) {
                break;
            }
            cur = next;
            assert!(steps <= 12, "widening must terminate within the ladder");
        }
        assert!(matches!(cur.hi, Bound::PosInf));
    }

    #[test]
    fn interval_leq_is_the_subset_order() {
        assert!(iv(3, 5).leq(&iv(1, 7)));
        assert!(!iv(1, 7).leq(&iv(3, 5)));
        // ⊥ ⊑ everything; everything ⊑ ⊤
        assert!(Interval::bottom().leq(&iv(1, 2)));
        assert!(iv(1, 2).leq(&<Interval as AbstractDomain>::top()));
        // reflexive
        assert!(iv(1, 2).leq(&iv(1, 2)));
    }

    #[test]
    fn interval_lattice_laws_hold() {
        let a = iv(1, 5);
        let b = iv(3, 9);
        // a ⊑ a⊔b and b ⊑ a⊔b (join is an upper bound)
        assert!(a.leq(&a.join(&b)));
        assert!(b.leq(&a.join(&b)));
        // a⊓b ⊑ a and a⊓b ⊑ b (meet is a lower bound)
        assert!(a.meet(&b).leq(&a));
        assert!(a.meet(&b).leq(&b));
        // commutativity of join and meet
        assert!(a.join(&b).leq(&b.join(&a)) && b.join(&a).leq(&a.join(&b)));
        assert!(a.meet(&b).leq(&b.meet(&a)) && b.meet(&a).leq(&a.meet(&b)));
    }
}

#[cfg(test)]
mod type_domain_tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_types() -> HashMap<Symbol, TypeAbstraction> {
        HashMap::new()
    }

    fn union(tags: &[TypeTag]) -> TypeAbstraction {
        TypeAbstraction::Union(tags.iter().cloned().collect())
    }

    #[test]
    fn literal_types_are_concrete() {
        assert_eq!(eval_type(&Expr::Literal(Literal::Number(5)), &empty_types(), &HashMap::new(), &HashMap::new()),
                   TypeAbstraction::Concrete(TypeTag::Int));
        assert_eq!(eval_type(&Expr::Literal(Literal::Float(1.5)), &empty_types(), &HashMap::new(), &HashMap::new()),
                   TypeAbstraction::Concrete(TypeTag::Float));
        assert_eq!(eval_type(&Expr::Literal(Literal::Boolean(true)), &empty_types(), &HashMap::new(), &HashMap::new()),
                   TypeAbstraction::Concrete(TypeTag::Bool));
        assert_eq!(eval_type(&Expr::Literal(Literal::Nothing), &empty_types(), &HashMap::new(), &HashMap::new()),
                   TypeAbstraction::Concrete(TypeTag::Nothing));
        assert_eq!(eval_type(&Expr::Literal(Literal::Char('a')), &empty_types(), &HashMap::new(), &HashMap::new()),
                   TypeAbstraction::Concrete(TypeTag::Char));
        assert_eq!(eval_type(&Expr::Literal(Literal::Duration(60)), &empty_types(), &HashMap::new(), &HashMap::new()),
                   TypeAbstraction::Concrete(TypeTag::Duration));
    }

    #[test]
    fn arithmetic_and_comparison_result_types() {
        let five = Expr::Literal(Literal::Number(5));
        let three = Expr::Literal(Literal::Number(3));
        // Int + Int : Int
        let add = Expr::BinaryOp { op: BinaryOpKind::Add, left: &five, right: &three };
        assert_eq!(eval_type(&add, &empty_types(), &HashMap::new(), &HashMap::new()), TypeAbstraction::Concrete(TypeTag::Int));
        // Int < Int : Bool
        let lt = Expr::BinaryOp { op: BinaryOpKind::Lt, left: &five, right: &three };
        assert_eq!(eval_type(&lt, &empty_types(), &HashMap::new(), &HashMap::new()), TypeAbstraction::Concrete(TypeTag::Bool));
        // Float + Float : Float
        let f1 = Expr::Literal(Literal::Float(1.0));
        let f2 = Expr::Literal(Literal::Float(2.0));
        let fadd = Expr::BinaryOp { op: BinaryOpKind::Add, left: &f1, right: &f2 };
        assert_eq!(eval_type(&fadd, &empty_types(), &HashMap::new(), &HashMap::new()), TypeAbstraction::Concrete(TypeTag::Float));
        // Int + Float : Top (we do not assume a coercion — conservative)
        let mixed = Expr::BinaryOp { op: BinaryOpKind::Add, left: &five, right: &f1 };
        assert_eq!(eval_type(&mixed, &empty_types(), &HashMap::new(), &HashMap::new()), TypeAbstraction::Top);
        // Text + Int : Text (interpolating concat)
        let txt = Expr::Literal(Literal::Text(Symbol::EMPTY));
        let concat = Expr::BinaryOp { op: BinaryOpKind::Add, left: &txt, right: &three };
        assert_eq!(eval_type(&concat, &empty_types(), &HashMap::new(), &HashMap::new()), TypeAbstraction::Concrete(TypeTag::Text));
    }

    #[test]
    fn let_binds_value_type_in_env() {
        // `Let x be 5.` ⇒ env[x] = Concrete(Int); a later use of x reads Int.
        let x = Symbol::EMPTY;
        let mut env = empty_types();
        let value = Expr::Literal(Literal::Number(5));
        env.insert(x, eval_type(&value, &env, &HashMap::new(), &HashMap::new()));
        assert_eq!(env.get(&x), Some(&TypeAbstraction::Concrete(TypeTag::Int)));
        // and an identifier reference resolves through the env
        assert_eq!(eval_type(&Expr::Identifier(x), &env, &HashMap::new(), &HashMap::new()),
                   TypeAbstraction::Concrete(TypeTag::Int));
        // an unbound identifier is Top (no information)
        let unbound = empty_types();
        assert_eq!(eval_type(&Expr::Identifier(x), &unbound, &HashMap::new(), &HashMap::new()), TypeAbstraction::Top);
    }

    #[test]
    fn type_join_merges_distinct_into_union_and_collapses() {
        let i = TypeAbstraction::Concrete(TypeTag::Int);
        let b = TypeAbstraction::Concrete(TypeTag::Bool);
        // distinct concretes join to a union
        assert_eq!(i.join(&b), union(&[TypeTag::Int, TypeTag::Bool]));
        // identical concretes stay concrete
        assert_eq!(i.join(&i), i);
        // a singleton union normalizes back to a concrete
        assert_eq!(union(&[TypeTag::Int]), i);
        // join with Top is Top; join with Bottom is identity
        assert_eq!(i.join(&TypeAbstraction::Top), TypeAbstraction::Top);
        assert_eq!(i.join(&TypeAbstraction::Bottom), i);
        assert_eq!(TypeAbstraction::Bottom.join(&i), i);
    }

    #[test]
    fn type_meet_intersects() {
        let i = TypeAbstraction::Concrete(TypeTag::Int);
        let b = TypeAbstraction::Concrete(TypeTag::Bool);
        let ib = union(&[TypeTag::Int, TypeTag::Bool]);
        // Concrete ⊓ matching Union = Concrete
        assert_eq!(i.meet(&ib), i);
        // disjoint concretes meet to Bottom
        assert_eq!(i.meet(&b), TypeAbstraction::Bottom);
        // Top is identity for meet
        assert_eq!(TypeAbstraction::Top.meet(&i), i);
        // Bottom annihilates
        assert_eq!(TypeAbstraction::Bottom.meet(&i), TypeAbstraction::Bottom);
    }

    #[test]
    fn type_lattice_order_and_laws() {
        let i = TypeAbstraction::Concrete(TypeTag::Int);
        let ib = union(&[TypeTag::Int, TypeTag::Bool]);
        // Bottom ⊑ everything ⊑ Top
        assert!(TypeAbstraction::Bottom.leq(&i));
        assert!(i.leq(&TypeAbstraction::Top));
        // Concrete ⊑ Union containing it
        assert!(i.leq(&ib));
        assert!(!ib.leq(&i));
        // reflexive
        assert!(i.leq(&i));
        // join is an upper bound; widen over-approximates join (finite lattice → widen == join)
        let b = TypeAbstraction::Concrete(TypeTag::Bool);
        assert!(i.leq(&i.join(&b)));
        assert!(b.leq(&i.join(&b)));
        assert_eq!(i.widen(&b), i.join(&b));
    }
}

#[cfg(test)]
mod shape_domain_tests {
    use super::*;

    #[test]
    fn new_collection_is_empty_and_push_pop_track_size() {
        assert_eq!(CollectionShape::empty_collection(), CollectionShape::Empty);
        // Push grows the size precisely.
        assert_eq!(CollectionShape::Empty.pushed(), CollectionShape::Singleton);
        assert_eq!(CollectionShape::Singleton.pushed(), CollectionShape::KnownSize(2));
        assert_eq!(CollectionShape::KnownSize(2).pushed(), CollectionShape::KnownSize(3));
        // Pop shrinks it.
        assert_eq!(CollectionShape::KnownSize(3).popped(), CollectionShape::KnownSize(2));
        assert_eq!(CollectionShape::Singleton.popped(), CollectionShape::Empty);
        // Pop on a definitely-empty collection stays Empty (saturating).
        assert_eq!(CollectionShape::Empty.popped(), CollectionShape::Empty);
        // Pushing an unknown (Top) collection proves it non-empty.
        assert_eq!(CollectionShape::Top.pushed(), CollectionShape::NonEmpty);
    }

    #[test]
    fn nonempty_and_empty_predicates() {
        assert!(CollectionShape::Singleton.is_definitely_nonempty());
        assert!(CollectionShape::KnownSize(5).is_definitely_nonempty());
        assert!(CollectionShape::NonEmpty.is_definitely_nonempty());
        assert!(!CollectionShape::Empty.is_definitely_nonempty());
        assert!(!CollectionShape::Top.is_definitely_nonempty());
        assert!(!CollectionShape::KnownSize(0).is_definitely_nonempty());

        assert!(CollectionShape::Empty.is_definitely_empty());
        assert!(CollectionShape::KnownSize(0).is_definitely_empty());
        assert!(!CollectionShape::Singleton.is_definitely_empty());
        assert!(!CollectionShape::Top.is_definitely_empty());
    }

    #[test]
    fn shape_canonical_equality() {
        // Named variants that denote the same size set compare equal.
        assert_eq!(CollectionShape::KnownSize(0), CollectionShape::Empty);
        assert_eq!(CollectionShape::KnownSize(1), CollectionShape::Singleton);
        assert_eq!(CollectionShape::SizeRange(1, 1), CollectionShape::Singleton);
    }

    #[test]
    fn shape_lattice_join_meet_leq() {
        // join is union of size ranges
        assert_eq!(CollectionShape::Empty.join(&CollectionShape::Singleton),
                   CollectionShape::SizeRange(0, 1));
        assert_eq!(CollectionShape::Singleton.join(&CollectionShape::KnownSize(3)),
                   CollectionShape::SizeRange(1, 3));
        assert_eq!(CollectionShape::Singleton.join(&CollectionShape::Top),
                   CollectionShape::Top);
        // meet is intersection; disjoint sizes collapse to Bottom
        assert_eq!(CollectionShape::Empty.meet(&CollectionShape::Singleton),
                   CollectionShape::Bottom);
        assert_eq!(CollectionShape::Top.meet(&CollectionShape::Singleton),
                   CollectionShape::Singleton);
        // order
        assert!(CollectionShape::Singleton.leq(&CollectionShape::NonEmpty));
        assert!(CollectionShape::KnownSize(2).leq(&CollectionShape::SizeRange(0, 5)));
        assert!(CollectionShape::Empty.leq(&CollectionShape::Top));
        assert!(CollectionShape::Bottom.leq(&CollectionShape::Empty));
        assert!(!CollectionShape::Top.leq(&CollectionShape::NonEmpty));
    }

    #[test]
    fn shape_widening_converges_growing_loops() {
        // A push-only loop keeps growing the upper bound → +∞, stays NonEmpty.
        let w = CollectionShape::KnownSize(1).widen(&CollectionShape::KnownSize(2));
        assert_eq!(w, CollectionShape::NonEmpty);
        // A collection that might shrink widens its lower bound toward 0.
        let w2 = CollectionShape::KnownSize(2).widen(&CollectionShape::KnownSize(1));
        assert!(w2.leq(&CollectionShape::Top));
    }
}

#[cfg(test)]
mod nullability_domain_tests {
    use super::*;

    #[test]
    fn top_and_bottom() {
        assert_eq!(<Nullability as AbstractDomain>::top(), Nullability::Maybe);
        assert_eq!(<Nullability as AbstractDomain>::bottom(), Nullability::Bottom);
    }

    #[test]
    fn literal_nullability() {
        // `nothing` is definitely absent; everything else is definitely present.
        assert_eq!(Nullability::for_literal(&Literal::Nothing), Nullability::Null);
        assert_eq!(Nullability::for_literal(&Literal::Number(5)), Nullability::Definite);
        assert_eq!(Nullability::for_literal(&Literal::Boolean(false)), Nullability::Definite);
    }

    #[test]
    fn matched_variant_binding_is_definite() {
        // `Inspect x: When Some(v): ...` ⇒ inside the arm, the match is present.
        // This is the fact that lets the unwrap guard be eliminated.
        assert_eq!(Nullability::for_matched_variant(), Nullability::Definite);
    }

    #[test]
    fn nullability_join_meet() {
        // join merges Definite and Null into Maybe (the diamond top)
        assert_eq!(Nullability::Definite.join(&Nullability::Null), Nullability::Maybe);
        assert_eq!(Nullability::Definite.join(&Nullability::Definite), Nullability::Definite);
        assert_eq!(Nullability::Bottom.join(&Nullability::Null), Nullability::Null);
        assert_eq!(Nullability::Maybe.join(&Nullability::Definite), Nullability::Maybe);
        // meet refines; Definite ⊓ Null is unreachable
        assert_eq!(Nullability::Definite.meet(&Nullability::Null), Nullability::Bottom);
        assert_eq!(Nullability::Maybe.meet(&Nullability::Definite), Nullability::Definite);
        assert_eq!(Nullability::Definite.meet(&Nullability::Definite), Nullability::Definite);
    }

    #[test]
    fn nullability_order_and_widen() {
        assert!(Nullability::Bottom.leq(&Nullability::Definite));
        assert!(Nullability::Definite.leq(&Nullability::Maybe));
        assert!(Nullability::Null.leq(&Nullability::Maybe));
        assert!(Nullability::Definite.leq(&Nullability::Definite));
        assert!(!Nullability::Definite.leq(&Nullability::Null));
        assert!(!Nullability::Maybe.leq(&Nullability::Definite));
        // finite lattice → widen is join
        assert_eq!(Nullability::Definite.widen(&Nullability::Null),
                   Nullability::Definite.join(&Nullability::Null));
    }
}

#[cfg(test)]
mod alias_domain_tests {
    use super::*;
    use crate::intern::Interner;
    use std::collections::HashSet;

    fn set(syms: &[Symbol]) -> HashSet<Symbol> {
        syms.iter().cloned().collect()
    }

    #[test]
    fn alias_lattice_ops() {
        let mut it = Interner::new();
        let s = it.intern("s");
        let t = it.intern("t");
        assert_eq!(<AliasInfo as AbstractDomain>::top(), AliasInfo::Top);
        assert_eq!(<AliasInfo as AbstractDomain>::bottom(), AliasInfo::Bottom);
        // Unique is "may-alias nobody" = MayAlias(∅)
        assert_eq!(AliasInfo::MayAlias(HashSet::new()), AliasInfo::Unique);
        // join unions the may-alias sets
        assert_eq!(AliasInfo::MayAlias(set(&[s])).join(&AliasInfo::MayAlias(set(&[t]))),
                   AliasInfo::MayAlias(set(&[s, t])));
        assert_eq!(AliasInfo::Unique.join(&AliasInfo::MayAlias(set(&[s]))),
                   AliasInfo::MayAlias(set(&[s])));
        assert_eq!(AliasInfo::Top.join(&AliasInfo::Unique), AliasInfo::Top);
        // meet intersects
        assert_eq!(AliasInfo::MayAlias(set(&[s, t])).meet(&AliasInfo::MayAlias(set(&[s]))),
                   AliasInfo::MayAlias(set(&[s])));
        assert_eq!(AliasInfo::Top.meet(&AliasInfo::MayAlias(set(&[s]))),
                   AliasInfo::MayAlias(set(&[s])));
        // order: fewer possible aliases is more precise
        assert!(AliasInfo::Unique.leq(&AliasInfo::MayAlias(set(&[s]))));
        assert!(AliasInfo::MayAlias(set(&[s])).leq(&AliasInfo::MayAlias(set(&[s, t]))));
        assert!(AliasInfo::MayAlias(set(&[s])).leq(&AliasInfo::Top));
        assert!(AliasInfo::Bottom.leq(&AliasInfo::Unique));
        assert!(!AliasInfo::Top.leq(&AliasInfo::Unique));
        // widen = join (alias sets are bounded by the program's variables)
        assert_eq!(AliasInfo::MayAlias(set(&[s])).widen(&AliasInfo::MayAlias(set(&[t]))),
                   AliasInfo::MayAlias(set(&[s])).join(&AliasInfo::MayAlias(set(&[t]))));
    }

    #[test]
    fn aliasing_mutation_invalidates_shared_allocation() {
        let mut it = Interner::new();
        let a = it.intern("a");
        let items = it.intern("items");
        let c = it.intern("c"); // unrelated

        let mut g = AliasGraph::new();
        g.link(a, items); // `Let a be items.` — they share one Rc<RefCell>

        // `Push 1 to a.` mutates the shared allocation → invalidate `items` too.
        let inval = g.invalidated_by_mutation(a);
        assert!(inval.contains(&items));
        assert!(inval.contains(&a));
        assert!(!inval.contains(&c));

        // symmetric: mutating `items` invalidates `a`
        assert!(g.invalidated_by_mutation(items).contains(&a));

        // an unrelated variable aliases only itself
        assert_eq!(g.invalidated_by_mutation(c), set(&[c]));

        // rebinding `a` to a fresh allocation breaks the alias
        g.unlink(a);
        assert!(!g.invalidated_by_mutation(items).contains(&a));
    }

    #[test]
    fn alias_transitivity() {
        let mut it = Interner::new();
        let a = it.intern("a");
        let b = it.intern("b");
        let c = it.intern("c");
        let mut g = AliasGraph::new();
        g.link(a, b); // Let b be a
        g.link(b, c); // Let c be b
        // mutation through a reaches c via b
        let inval = g.invalidated_by_mutation(a);
        assert!(inval.contains(&b) && inval.contains(&c));
    }
}

#[cfg(test)]
mod product_lattice_tests {
    use super::*;

    fn av(iv: (i64, i64), t: TypeTag, sh: CollectionShape, n: Nullability) -> AbstractValue {
        AbstractValue {
            interval: Interval { lo: Bound::Finite(iv.0), hi: Bound::Finite(iv.1) },
            ty: TypeAbstraction::Concrete(t),
            shape: sh,
            nullability: n,
            alias: AliasInfo::Unique,
        }
    }

    #[test]
    fn product_top_is_all_top() {
        let t = <AbstractValue as AbstractDomain>::top();
        assert_eq!(t.ty, TypeAbstraction::Top);
        assert_eq!(t.shape, CollectionShape::Top);
        assert_eq!(t.nullability, Nullability::Maybe);
        assert_eq!(t.alias, AliasInfo::Top);
        assert!(t.interval.lo == Bound::NegInf && t.interval.hi == Bound::PosInf);
    }

    #[test]
    fn product_join_is_componentwise() {
        let a = av((1, 1), TypeTag::Int, CollectionShape::Singleton, Nullability::Definite);
        let b = av((3, 3), TypeTag::Int, CollectionShape::KnownSize(3), Nullability::Definite);
        let j = a.join(&b);
        // interval joins to [1,3]
        assert_eq!(j.interval.lo, Bound::Finite(1));
        assert_eq!(j.interval.hi, Bound::Finite(3));
        // same type stays Concrete(Int)
        assert_eq!(j.ty, TypeAbstraction::Concrete(TypeTag::Int));
        // shapes join to a size range
        assert_eq!(j.shape, CollectionShape::SizeRange(1, 3));
        // both Definite stays Definite
        assert_eq!(j.nullability, Nullability::Definite);
    }

    #[test]
    fn product_join_mixed_types_widens_each_component() {
        let a = av((0, 0), TypeTag::Int, CollectionShape::Empty, Nullability::Null);
        let b = av((5, 5), TypeTag::Bool, CollectionShape::Singleton, Nullability::Definite);
        let j = a.join(&b);
        assert_eq!(j.interval.lo, Bound::Finite(0));
        assert_eq!(j.interval.hi, Bound::Finite(5));
        // distinct types → union
        assert_eq!(j.ty, TypeAbstraction::Union([TypeTag::Int, TypeTag::Bool].into_iter().collect()));
        assert_eq!(j.shape, CollectionShape::SizeRange(0, 1));
        // Null ⊔ Definite → Maybe
        assert_eq!(j.nullability, Nullability::Maybe);
    }

    #[test]
    fn product_leq_and_widen_componentwise() {
        let a = av((1, 1), TypeTag::Int, CollectionShape::Singleton, Nullability::Definite);
        let top = <AbstractValue as AbstractDomain>::top();
        assert!(a.leq(&top));
        assert!(!top.leq(&a));
        // widen each component: the rising interval bound snaps to the
        // next THRESHOLD rung (EXODIA M9 ladder), not straight to +∞ —
        let b = av((1, 9), TypeTag::Int, CollectionShape::KnownSize(9), Nullability::Definite);
        let w = a.widen(&b);
        assert_eq!(w.interval.hi, Bound::Finite(10));
        assert_eq!(w.ty, TypeAbstraction::Concrete(TypeTag::Int));
        // — and falls off to +∞ only past the last rung.
        let c = av((1, 5000), TypeTag::Int, CollectionShape::KnownSize(9), Nullability::Definite);
        let w2 = a.widen(&c);
        assert!(matches!(w2.interval.hi, Bound::PosInf));
    }
}

#[cfg(test)]
mod rich_walk_tests {
    use super::*;
    use crate::arena::Arena;
    use crate::intern::Interner;

    fn lit_num<'a>(ea: &'a Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
        ea.alloc(Expr::Literal(Literal::Number(n)))
    }
    fn ident<'a>(ea: &'a Arena<Expr<'a>>, s: Symbol) -> &'a Expr<'a> {
        ea.alloc(Expr::Identifier(s))
    }
    fn add<'a>(ea: &'a Arena<Expr<'a>>, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
        ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: l, right: r })
    }
    fn new_seq<'a>(ea: &'a Arena<Expr<'a>>, name: Symbol) -> &'a Expr<'a> {
        ea.alloc(Expr::New { type_name: name, type_args: vec![], init_fields: vec![] })
    }
    fn let_stmt<'a>(var: Symbol, value: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Let { var, ty: None, value, mutable: true }
    }

    #[test]
    fn let_chain_tracks_type_and_interval() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let y = it.intern("y");
        // Let x be 5. Let y be x + 7.
        let five = lit_num(&ea, 5);
        let xref = ident(&ea, x);
        let seven = lit_num(&ea, 7);
        let sum = add(&ea, xref, seven);
        let stmts = vec![let_stmt(x, five), let_stmt(y, sum)];

        let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
        let vx = st.value_of(x);
        let vy = st.value_of(y);
        assert_eq!(vx.ty, TypeAbstraction::Concrete(TypeTag::Int));
        assert_eq!(vx.interval.is_exact(), Some(5));
        assert_eq!(vy.ty, TypeAbstraction::Concrete(TypeTag::Int));
        assert_eq!(vy.interval.is_exact(), Some(12));
        assert_eq!(vx.nullability, Nullability::Definite);
    }

    #[test]
    fn new_and_push_track_shape() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let items = it.intern("items");
        let seq_ty = it.intern("Seq");
        // Let items be a new Seq. Push 10 to items.
        let new_e = new_seq(&ea, seq_ty);
        let ten = lit_num(&ea, 10);
        let items_ref = ident(&ea, items);
        let stmts = vec![
            let_stmt(items, new_e),
            Stmt::Push { value: ten, collection: items_ref },
        ];
        let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
        let v = st.value_of(items);
        assert_eq!(v.shape, CollectionShape::Singleton);
        assert!(v.shape.is_definitely_nonempty());
    }

    #[test]
    fn alias_push_keeps_aliases_consistent() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let items = it.intern("items");
        let a = it.intern("a");
        let seq_ty = it.intern("Seq");
        // Let items be a new Seq. Push 1 to items. Let a be items. Push 2 to a.
        let new_e = new_seq(&ea, seq_ty);
        let one = lit_num(&ea, 1);
        let two = lit_num(&ea, 2);
        let items_ref1 = ident(&ea, items);
        let items_ref2 = ident(&ea, items);
        let a_ref = ident(&ea, a);
        let stmts = vec![
            let_stmt(items, new_e),
            Stmt::Push { value: one, collection: items_ref1 },
            let_stmt(a, items_ref2), // a aliases items
            Stmt::Push { value: two, collection: a_ref }, // mutate through alias
        ];
        let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
        let vitems = st.value_of(items);
        let va = st.value_of(a);
        // pushing through `a` updated `items` (shared Rc) — both see size 2.
        assert!(vitems.shape.is_definitely_nonempty());
        assert_eq!(vitems.shape, va.shape);
        assert_eq!(vitems.shape, CollectionShape::KnownSize(2));
    }

    #[test]
    fn if_branches_join() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let n = it.intern("n"); // unbound → dynamic condition
        // Let mutable x be 0. If n > 5: Set x to 10. Otherwise: Set x to 20.
        let zero = lit_num(&ea, 0);
        let nref = ident(&ea, n);
        let five = lit_num(&ea, 5);
        let cond = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Gt, left: nref, right: five });
        let ten = lit_num(&ea, 10);
        let twenty = lit_num(&ea, 20);
        let then_block: &[Stmt] = sa.alloc_slice(vec![Stmt::Set { target: x, value: ten }]);
        let else_block: &[Stmt] = sa.alloc_slice(vec![Stmt::Set { target: x, value: twenty }]);
        let stmts = vec![
            let_stmt(x, zero),
            Stmt::If { cond, then_block, else_block: Some(else_block) },
        ];
        let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
        let vx = st.value_of(x);
        // x is 10 or 20 after the join → interval [10, 20], type Int
        assert_eq!(vx.ty, TypeAbstraction::Concrete(TypeTag::Int));
        assert_eq!(vx.interval.lo, Bound::Finite(10));
        assert_eq!(vx.interval.hi, Bound::Finite(20));
    }
}
