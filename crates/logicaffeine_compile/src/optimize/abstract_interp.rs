use std::collections::{HashMap, HashSet};

use super::writes::{count_writes_of, rebound_names, CalleeGlobals, WriteSet};
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
            (Bound::Finite(a), Bound::Finite(b)) => match a.checked_add(*b) {
                Some(r) => Bound::Finite(r),
                None => {
                    if *a > 0 {
                        Bound::PosInf
                    } else {
                        Bound::NegInf
                    }
                }
            },
            (Bound::PosInf, Bound::NegInf) | (Bound::NegInf, Bound::PosInf) => Bound::NegInf,
            (Bound::PosInf, _) | (_, Bound::PosInf) => Bound::PosInf,
            (Bound::NegInf, _) | (_, Bound::NegInf) => Bound::NegInf,
        }
    }

    fn sub(&self, other: &Bound) -> Bound {
        match (self, other) {
            (Bound::Finite(a), Bound::Finite(b)) => match a.checked_sub(*b) {
                Some(r) => Bound::Finite(r),
                None => {
                    if *a > 0 {
                        Bound::PosInf
                    } else {
                        Bound::NegInf
                    }
                }
            },
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

    /// How `self` orders against `other` for every integer the two bounds stand for, when that is decided. An
    /// infinite bound stands for integers past the word on its side — an endpoint that overflowed i64 is one — so
    /// two of the same sign are not ordered by their bounds.
    fn decided(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Bound::PosInf, Bound::PosInf) | (Bound::NegInf, Bound::NegInf) => None,
            _ => Some(self.cmp_bound(other)),
        }
    }

    fn min_bound(a: &Bound, b: &Bound) -> Bound {
        if a.cmp_bound(b) == std::cmp::Ordering::Less {
            a.clone()
        } else {
            b.clone()
        }
    }

    fn max_bound(a: &Bound, b: &Bound) -> Bound {
        if a.cmp_bound(b) == std::cmp::Ordering::Greater {
            a.clone()
        } else {
            b.clone()
        }
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
        Interval {
            lo: Bound::Finite(n),
            hi: Bound::Finite(n),
        }
    }

    fn top() -> Self {
        Interval {
            lo: Bound::NegInf,
            hi: Bound::PosInf,
        }
    }

    fn non_negative() -> Self {
        Interval {
            lo: Bound::Finite(0),
            hi: Bound::PosInf,
        }
    }

    fn is_exact(&self) -> Option<i64> {
        if let (Bound::Finite(a), Bound::Finite(b)) = (&self.lo, &self.hi) {
            if a == b {
                return Some(*a);
            }
        }
        None
    }

    /// The empty interval `⊥`. Represented as `lo > hi`, which no reachable
    /// program point ever constructs, so it is unambiguous.
    fn bottom() -> Self {
        Interval {
            lo: Bound::PosInf,
            hi: Bound::NegInf,
        }
    }

    fn is_bottom(&self) -> bool {
        self.lo.cmp_bound(&self.hi) == std::cmp::Ordering::Greater
    }

    fn join(&self, other: &Interval) -> Interval {
        if self.is_bottom() {
            return other.clone();
        }
        if other.is_bottom() {
            return self.clone();
        }
        Interval {
            lo: Bound::min_bound(&self.lo, &other.lo),
            hi: Bound::max_bound(&self.hi, &other.hi),
        }
    }

    /// Intersection `⊓`. Disjoint inputs collapse to `⊥`.
    fn meet(&self, other: &Interval) -> Interval {
        if self.is_bottom() || other.is_bottom() {
            return Interval::bottom();
        }
        let r = Interval {
            lo: Bound::max_bound(&self.lo, &other.lo),
            hi: Bound::min_bound(&self.hi, &other.hi),
        };
        if r.is_bottom() {
            Interval::bottom()
        } else {
            r
        }
    }

    /// Widening `self ▽ other` (self = old, other = new). An unstable bound is
    /// thrown to the corresponding infinity so ascending chains converge.
    fn widen(&self, other: &Interval) -> Interval {
        if self.is_bottom() {
            return other.clone();
        }
        if other.is_bottom() {
            return self.clone();
        }
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

    /// Narrowing `self △ other` (self = the widened post-fixpoint bound, other =
    /// the body transfer recomputed from it). Only an INFINITE bound is replaced,
    /// by the recomputed value; a finite bound is kept. Descending from a
    /// post-fixpoint (`self ⊒ lfp`, so `other = F(self) ⊒ lfp`) every result stays
    /// a sound over-approximation, and each step fills at least one ±∞ — so a
    /// bounded run terminates. This is what recovers a modulo-clamped
    /// accumulator's finite bound (`b = (a + b) % M` re-clamps `b` to `[0, M-1]`
    /// even though the widener threw its upper to `+∞`): the license the raw-i64
    /// arithmetic lowering needs to drop the overflow guard.
    fn narrow(&self, other: &Interval) -> Interval {
        if self.is_bottom() || other.is_bottom() {
            return self.clone();
        }
        let lo = match self.lo {
            Bound::NegInf => other.lo.clone(),
            _ => self.lo.clone(),
        };
        let hi = match self.hi {
            Bound::PosInf => other.hi.clone(),
            _ => self.hi.clone(),
        };
        Interval { lo, hi }
    }

    /// Lattice order `self ⊑ other`, i.e. `self ⊆ other` as concrete sets.
    fn leq(&self, other: &Interval) -> bool {
        if self.is_bottom() {
            return true;
        }
        if other.is_bottom() {
            return false;
        }
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
                return Interval {
                    lo: self.lo.div_by(k),
                    hi: self.hi.div_by(k),
                };
            } else if k < 0 {
                return Interval {
                    lo: self.hi.div_by(k),
                    hi: self.lo.div_by(k),
                };
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
                let lo = if nonneg {
                    Bound::Finite(0)
                } else {
                    Bound::Finite(-m)
                };
                let hi = if nonpos {
                    Bound::Finite(0)
                } else {
                    Bound::Finite(m)
                };
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
            return Interval {
                lo: Bound::NegInf,
                hi: Bound::Finite(0),
            };
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
                return Interval {
                    lo: self.lo.shr_by(k),
                    hi: self.hi.shr_by(k),
                };
            }
        }
        Interval::top()
    }

    /// The finite non-negative `[lo, hi]` this interval is, if it is one.
    fn natural(&self) -> Option<(i64, i64)> {
        match (&self.lo, &self.hi) {
            (Bound::Finite(lo), Bound::Finite(hi)) if *lo >= 0 && lo <= hi => Some((*lo, *hi)),
            _ => None,
        }
    }

    /// `x & y`. A non-negative operand bounds the result to `[0, it]`, whatever the other: on two words the
    /// result has no sign bit and no bit the non-negative one lacks, and a big integer takes a mask only when it
    /// is not negative, leaving its low bits — any other big operand is refused, giving no value at all.
    fn bitand(&self, other: &Interval) -> Interval {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            return Interval::exact(a & b);
        }
        let cap = |i: &Interval| match (&i.lo, &i.hi) {
            (Bound::Finite(lo), hi) if *lo >= 0 => Some(hi.clone()),
            _ => None,
        };
        match (cap(self), cap(other)) {
            (Some(a), Some(b)) => Interval {
                lo: Bound::Finite(0),
                hi: Bound::min_bound(&a, &b),
            },
            (Some(hi), None) | (None, Some(hi)) => Interval {
                lo: Bound::Finite(0),
                hi,
            },
            (None, None) => Interval::top(),
        }
    }

    /// `x | y` and `x ^ y` on two non-negative words: below the next power of two past both, and for `|` at
    /// least the larger. A big operand is refused, so it gives no value.
    fn bitor_or_xor(&self, other: &Interval, or: bool) -> Interval {
        let (Some((al, ah)), Some((bl, bh))) = (self.natural(), other.natural()) else {
            return Interval::top();
        };
        let width = 64 - (ah.max(bh) as u64).leading_zeros();
        let ceiling = ((1u128 << width) - 1) as i64;
        Interval {
            lo: Bound::Finite(if or { al.max(bl) } else { 0 }),
            hi: Bound::Finite(ceiling),
        }
    }

    /// `x << s` for a constant `s`: the shift is by `s mod 64` (`i64::wrapping_shl`), which is the product by
    /// `2^(s mod 64)` exactly when no endpoint's product leaves the word; otherwise it wraps, and nothing is known.
    fn shl(&self, other: &Interval) -> Interval {
        let (Some(s), Bound::Finite(lo), Bound::Finite(hi)) = (other.is_exact(), &self.lo, &self.hi) else {
            return Interval::top();
        };
        let k = (s as u32) & 63;
        let scale = |v: i64| i64::try_from((v as i128) << k).ok();
        match (scale(*lo), scale(*hi)) {
            (Some(lo), Some(hi)) => Interval {
                lo: Bound::Finite(lo),
                hi: Bound::Finite(hi),
            },
            _ => Interval::top(),
        }
    }

    fn definitely_gt(&self, other: &Interval) -> Option<bool> {
        use std::cmp::Ordering::{Equal, Greater, Less};
        if self.lo.decided(&other.hi) == Some(Greater) {
            return Some(true);
        }
        if matches!(self.hi.decided(&other.lo), Some(Less | Equal)) {
            return Some(false);
        }
        None
    }

    fn definitely_lt(&self, other: &Interval) -> Option<bool> {
        use std::cmp::Ordering::{Equal, Greater, Less};
        if self.hi.decided(&other.lo) == Some(Less) {
            return Some(true);
        }
        if matches!(self.lo.decided(&other.hi), Some(Greater | Equal)) {
            return Some(false);
        }
        None
    }

    fn definitely_gteq(&self, other: &Interval) -> Option<bool> {
        use std::cmp::Ordering::{Equal, Greater, Less};
        if matches!(self.lo.decided(&other.hi), Some(Greater | Equal)) {
            return Some(true);
        }
        if self.hi.decided(&other.lo) == Some(Less) {
            return Some(false);
        }
        None
    }

    fn definitely_lteq(&self, other: &Interval) -> Option<bool> {
        use std::cmp::Ordering::{Equal, Greater, Less};
        if matches!(self.hi.decided(&other.lo), Some(Less | Equal)) {
            return Some(true);
        }
        if self.lo.decided(&other.hi) == Some(Greater) {
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
        if self.hi.decided(&other.lo) == Some(std::cmp::Ordering::Less)
            || self.lo.decided(&other.hi) == Some(std::cmp::Ordering::Greater)
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
    fn top() -> Self {
        Interval::top()
    }
    fn bottom() -> Self {
        Interval::bottom()
    }
    fn join(&self, other: &Self) -> Self {
        Interval::join(self, other)
    }
    fn meet(&self, other: &Self) -> Self {
        Interval::meet(self, other)
    }
    fn widen(&self, other: &Self) -> Self {
        Interval::widen(self, other)
    }
    fn leq(&self, other: &Self) -> bool {
        Interval::leq(self, other)
    }
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
    fn top() -> Self {
        TypeAbstraction::Top
    }
    fn bottom() -> Self {
        TypeAbstraction::Bottom
    }

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
    fn widen(&self, other: &Self) -> Self {
        self.join(other)
    }

    fn leq(&self, other: &Self) -> bool {
        match (self.tag_set(), other.tag_set()) {
            (_, None) => true,        // everything ⊑ Top
            (None, Some(_)) => false, // Top ⊑ only Top
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
        Lt | Gt | LtEq | GtEq | Eq | NotEq | ApproxEq => conc(TypeTag::Bool),
        // Bitwise on ints; on Sets these are set ops — Top keeps them out of
        // numeric reductions.
        BitAnd | BitOr => TypeAbstraction::Top,
        // Explicit concatenation is always Text.
        Concat => conc(TypeTag::Text),
        // Sequence concatenation yields a sequence — `Top` keeps it out of numeric reductions.
        SeqConcat => TypeAbstraction::Top,
        Add => {
            if is(l, TypeTag::Int) && is(r, TypeTag::Int) {
                conc(TypeTag::Int)
            } else if is(l, TypeTag::Float) && is(r, TypeTag::Float) {
                conc(TypeTag::Float)
            } else if is(l, TypeTag::Text) || is(r, TypeTag::Text) {
                conc(TypeTag::Text)
            } else {
                TypeAbstraction::Top
            }
        }
        // ExactDivide produces a Rational, never a known Int — `Top` keeps a
        // Rational-derived value out of the integer-only strength reductions.
        // Pow can promote to BigInt (or a Float) — `Top` likewise.
        ExactDivide | Pow => TypeAbstraction::Top,
        Subtract | Multiply | Divide | FloorDivide | Modulo => {
            if is(l, TypeTag::Int) && is(r, TypeTag::Int) {
                conc(TypeTag::Int)
            } else if is(l, TypeTag::Float) && is(r, TypeTag::Float) {
                conc(TypeTag::Float)
            } else {
                TypeAbstraction::Top
            }
        }
        And | Or => {
            if is(l, TypeTag::Bool) && is(r, TypeTag::Bool) {
                conc(TypeTag::Bool)
            } else if is(l, TypeTag::Int) && is(r, TypeTag::Int) {
                conc(TypeTag::Int)
            } else {
                TypeAbstraction::Top
            }
        }
        BitXor | Shl | Shr => {
            if is(l, TypeTag::Int) && is(r, TypeTag::Int) {
                conc(TypeTag::Int)
            } else {
                TypeAbstraction::Top
            }
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
        // A capacity is an allocation hint, not part of the value's type.
        // Keep every type-driven optimization identical to the unhinted form.
        Expr::WithCapacity { value, .. } => eval_type(value, types, fn_returns, elem_type),
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
        Expr::Index {
            collection: Expr::Identifier(sym),
            ..
        } if elem_type_enabled() => {
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
    fn empty_collection() -> Self {
        CollectionShape::Empty
    }

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
    fn top() -> Self {
        CollectionShape::Top
    }
    fn bottom() -> Self {
        CollectionShape::Bottom
    }

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
    fn top() -> Self {
        Nullability::Maybe
    }
    fn bottom() -> Self {
        Nullability::Bottom
    }

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
    fn widen(&self, other: &Self) -> Self {
        self.join(other)
    }

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
                if s.is_empty() {
                    AliasInfo::Unique
                } else {
                    AliasInfo::MayAlias(s)
                }
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
    fn top() -> Self {
        AliasInfo::Top
    }
    fn bottom() -> Self {
        AliasInfo::Bottom
    }

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
    fn widen(&self, other: &Self) -> Self {
        self.join(other)
    }

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
    /// What any call in this program may write — shared by every state cloned from the entry.
    globals: std::rc::Rc<CalleeGlobals>,
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
        AbstractState::with_globals(std::rc::Rc::default())
    }

    fn with_globals(globals: std::rc::Rc<CalleeGlobals>) -> Self {
        AbstractState {
            globals,
            vars: HashMap::new(),
            lengths: HashMap::new(),
            elem: HashMap::new(),
        }
    }

    /// What `stmt` may write, with the program's callee-written globals folded in when it calls.
    fn effects_of(&self, stmt: &Stmt) -> WriteSet {
        WriteSet::of_stmt(stmt).with_callee_globals(&self.globals)
    }

    /// What EVALUATING `expr` may write (through the calls it makes).
    fn effects_of_expr(&self, expr: &Expr) -> WriteSet {
        WriteSet::of_expr(expr).with_callee_globals(&self.globals)
    }

    /// Forget every fact a statement with these effects may have made stale: the written variables
    /// and lengths go to `top` — everything, after foreign code.
    fn forget(&mut self, effects: &WriteSet) {
        if effects.is_opaque() {
            self.forget_all();
            return;
        }
        for sym in effects.symbols() {
            self.set_var(sym, Interval::top());
            self.set_length(sym, Interval::top());
        }
    }

    fn forget_all(&mut self) {
        self.vars.clear();
        self.lengths.clear();
        self.elem.clear();
    }

    fn get_var(&self, sym: &Symbol) -> Interval {
        self.vars.get(sym).cloned().unwrap_or(Interval::top())
    }

    fn set_var(&mut self, sym: Symbol, range: Interval) {
        self.vars.insert(sym, range);
    }

    fn get_length(&self, sym: &Symbol) -> Interval {
        self.lengths
            .get(sym)
            .cloned()
            .unwrap_or(Interval::non_negative())
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
                BinaryOpKind::Shl => l.shl(&r),
                BinaryOpKind::BitAnd => l.bitand(&r),
                BinaryOpKind::BitOr => l.bitor_or_xor(&r, true),
                BinaryOpKind::BitXor => l.bitor_or_xor(&r, false),
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
    narrow_by_comparison(cond, state, false);
}

fn narrow_state_negated(cond: &Expr, state: &mut AbstractState) {
    narrow_by_comparison(cond, state, true);
}

/// Narrow the names a comparison compares by what its holding (or, `negated`, its failing) says, over intervals. The
/// relation that holds is put as `small < big` or `small <= big`: a name on the small side is at most the big side's
/// largest value (one less when strict), a name on the big side at least the small side's smallest (one more when
/// strict), and an equality that holds puts each name inside the other side's interval. Sound whatever the other
/// side's interval is, since a fact that holds only discards values it excludes: `j <= limit` bounds `j` by `limit`'s
/// own bound, where only a constant `limit` used to.
fn narrow_by_comparison(cond: &Expr, state: &mut AbstractState, negated: bool) {
    let Expr::BinaryOp { op, left, right } = cond else {
        return;
    };
    let holds = match (op, negated) {
        (BinaryOpKind::Lt, false) | (BinaryOpKind::GtEq, true) => BinaryOpKind::Lt,
        (BinaryOpKind::LtEq, false) | (BinaryOpKind::Gt, true) => BinaryOpKind::LtEq,
        (BinaryOpKind::Gt, false) | (BinaryOpKind::LtEq, true) => BinaryOpKind::Gt,
        (BinaryOpKind::GtEq, false) | (BinaryOpKind::Lt, true) => BinaryOpKind::GtEq,
        (BinaryOpKind::Eq, false) | (BinaryOpKind::NotEq, true) => BinaryOpKind::Eq,
        _ => return,
    };
    let (l, r) = (eval_expr(left, state), eval_expr(right, state));
    let cap = |state: &mut AbstractState, e: &Expr, hi: &Bound| {
        if let Expr::Identifier(sym) = e {
            let cur = state.get_var(sym);
            state.set_var(
                *sym,
                Interval {
                    lo: cur.lo.clone(),
                    hi: Bound::min_bound(&cur.hi, hi),
                },
            );
        }
    };
    let floor = |state: &mut AbstractState, e: &Expr, lo: &Bound| {
        if let Expr::Identifier(sym) = e {
            let cur = state.get_var(sym);
            state.set_var(
                *sym,
                Interval {
                    lo: Bound::max_bound(&cur.lo, lo),
                    hi: cur.hi.clone(),
                },
            );
        }
    };
    // A strict bound moves by one; at the very edge of i64 there is no such value, and nothing is narrowed.
    let step = |b: &Bound, by: i64| -> Option<Bound> {
        match b {
            Bound::Finite(n) => n.checked_add(by).map(Bound::Finite),
            other => Some(other.clone()),
        }
    };
    let (small, big, small_iv, big_iv, strict) = match holds {
        BinaryOpKind::Lt => (left, right, &l, &r, true),
        BinaryOpKind::LtEq => (left, right, &l, &r, false),
        BinaryOpKind::Gt => (right, left, &r, &l, true),
        BinaryOpKind::GtEq => (right, left, &r, &l, false),
        _ => {
            cap(state, left, &r.hi);
            floor(state, left, &r.lo);
            cap(state, right, &l.hi);
            floor(state, right, &l.lo);
            return;
        }
    };
    let by = if strict { 1 } else { 0 };
    if let Some(hi) = step(&big_iv.hi, -by) {
        cap(state, small, &hi);
    }
    if let Some(lo) = step(&small_iv.lo, by) {
        floor(state, big, &lo);
    }
}

pub fn abstract_interp_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut state =
        AbstractState::with_globals(std::rc::Rc::new(CalleeGlobals::of_program(&stmts)));
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
        // Everything this statement may write — its nested blocks and the globals its calls reach included.
        let effects = state.effects_of(&stmt);
        match stmt {
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => {
                // A call in the value runs before `var` takes it: what it may write is unknown from here on.
                let value_effects = state.effects_of_expr(value);
                state.forget(&value_effects);
                let range = eval_expr(value, state);
                state.set_var(var, range);
                if matches!(value, Expr::New { .. }) {
                    state.set_length(var, Interval::exact(0));
                }
                result.push(Stmt::Let {
                    var,
                    ty,
                    value,
                    mutable,
                });
            }

            Stmt::Set { target, value } => {
                let value_effects = state.effects_of_expr(value);
                state.forget(&value_effects);
                let range = eval_expr(value, state);
                state.set_var(target, range);
                result.push(Stmt::Set { target, value });
            }

            Stmt::Push { value, collection } => {
                let value_effects = state.effects_of_expr(value);
                state.forget(&value_effects);
                if let Expr::Identifier(sym) = collection {
                    let cur_len = state.get_length(sym);
                    state.set_length(*sym, cur_len.add(&Interval::exact(1)));
                }
                result.push(Stmt::Push { value, collection });
            }

            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let cond_effects = state.effects_of_expr(cond);
                state.forget(&cond_effects);
                if let Some(val) = eval_condition(cond, state) {
                    let new_cond = expr_arena.alloc(Expr::Literal(Literal::Boolean(val)));
                    if val {
                        let mut then_state = state.clone();
                        narrow_state(cond, &mut then_state);
                        let new_then = interp_nested_block(
                            then_block,
                            &mut then_state,
                            expr_arena,
                            stmt_arena,
                        );
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
                            let new_else =
                                interp_nested_block(eb, &mut else_state, expr_arena, stmt_arena);
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
                    let new_then =
                        interp_nested_block(then_block, &mut then_state, expr_arena, stmt_arena);

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

                    result.push(Stmt::If {
                        cond,
                        then_block: new_then,
                        else_block: new_else,
                    });
                }
            }

            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                let mut loop_state = state.clone();

                // Widen everything the loop may write — variables (the counter included), lengths, and
                // the globals any call in it reaches — to their full range before analyzing the body: a
                // loop carries them across iterations, so every pre-loop fact about them is stale from
                // the first iteration on (the Oracle must not fold a `(length of xs) > 0` guard against
                // a stale pre-loop length). Foreign code in the body forgets everything.
                loop_state.forget(&effects);

                // Now narrow the loop state based on the condition.
                // For the bounded variable (counter), this gives it the range
                // from its initial value (widened to top above) narrowed by the
                // condition, resulting in [-inf, bound].
                narrow_state(cond, &mut loop_state);

                let new_body = interp_nested_block(body, &mut loop_state, expr_arena, stmt_arena);

                // After the loop: whatever it may have written holds what its last iteration left, so it is
                // unknown here; and the loop ended because its condition failed, unless a `Break` of its own
                // ended it with the condition still holding.
                state.forget(&effects);
                if !crate::loop_shape::leaves_by_break(body) {
                    narrow_state_negated(cond, state);
                }

                result.push(Stmt::While {
                    cond,
                    body: new_body,
                    decreasing,
                });
            }

            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                let mut loop_state = state.clone();

                if let Pattern::Identifier(var) = &pattern {
                    loop_state.set_var(*var, Interval::top());
                }

                loop_state.forget(&effects);

                let new_body = interp_nested_block(body, &mut loop_state, expr_arena, stmt_arena);

                state.forget(&effects);

                result.push(Stmt::Repeat {
                    pattern,
                    iterable,
                    body: new_body,
                });
            }

            Stmt::FunctionDef {
                name,
                params,
                generics,
                body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            } => {
                let mut func_state = AbstractState::with_globals(state.globals.clone());
                let new_body = interp_nested_block(body, &mut func_state, expr_arena, stmt_arena);
                result.push(Stmt::FunctionDef {
                    name,
                    params,
                    generics,
                    body: new_body,
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                });
            }

            Stmt::Inspect {
                target,
                arms,
                has_otherwise,
            } => {
                let new_arms: Vec<_> = arms
                    .into_iter()
                    .map(|arm| {
                        let mut arm_state = state.clone();
                        let new_body =
                            interp_nested_block(arm.body, &mut arm_state, expr_arena, stmt_arena);
                        crate::ast::stmt::MatchArm {
                            enum_name: arm.enum_name,
                            variant: arm.variant,
                            bindings: arm.bindings,
                            body: new_body,
                        }
                    })
                    .collect();
                // Whatever any arm may have written is unknown after the `Inspect`.
                state.forget(&effects);
                result.push(Stmt::Inspect {
                    target,
                    arms: new_arms,
                    has_otherwise,
                });
            }

            Stmt::Zone { .. } => {
                // Don't analyze inside zones — zone-scoped bindings must be
                // preserved for escape analysis (same as propagation pass).
                state.forget(&effects);
                result.push(stmt);
            }

            Stmt::Concurrent { tasks } => {
                let mut sub_state = state.clone();
                let new_tasks = interp_nested_block(tasks, &mut sub_state, expr_arena, stmt_arena);
                state.forget(&effects);
                result.push(Stmt::Concurrent { tasks: new_tasks });
            }

            Stmt::Parallel { tasks } => {
                let mut sub_state = state.clone();
                let new_tasks = interp_nested_block(tasks, &mut sub_state, expr_arena, stmt_arena);
                state.forget(&effects);
                result.push(Stmt::Parallel { tasks: new_tasks });
            }

            // Every other statement passes through; whatever it may write is unknown afterwards.
            other => {
                state.forget(&effects);
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
        let a_len = a
            .lengths
            .get(&key)
            .cloned()
            .unwrap_or(Interval::non_negative());
        let b_len = b
            .lengths
            .get(&key)
            .cloned()
            .unwrap_or(Interval::non_negative());
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
    /// What any call in this program may write — the names a `Call` makes unknown.
    callee_globals: std::rc::Rc<CalleeGlobals>,
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
    /// The entry-guarded function's precondition lower bounds `(param, lb)` — `lo>=1`,
    /// `hi>=1`, which `codegen::entry_guard`'s `assert!` enforces at runtime. Carried
    /// ON THE STATE (like `aot_entry_guard`) so it reaches EVERY proof context: the
    /// loop fixpoint's `inside = st.clone()` copies it into the loop's record-state,
    /// where a thread-local was empty. Injected into every affine proof system; sound
    /// because the runtime guard aborts if a caller violates the precondition.
    entry_guard_lowers: Vec<(Symbol, i64)>,
    /// The affine equalities among the function's integers and sequence lengths here ([`super::karr`]).
    karr: super::karr::AffineSpace,
    /// Names bound to a sequence (`a new Seq`, a list literal, a sequence parameter, a copy of one, or a call that
    /// returns one as long as its argument): the collections `karr` tracks a length for, since a store into a
    /// sequence keeps its length and a store into a map may not.
    seq_vars: std::collections::HashSet<Symbol>,
    /// The type names `a new …` builds a sequence from, when the walk has an interner.
    seq_types: std::rc::Rc<std::collections::HashSet<Symbol>>,
    /// Functions that return a sequence as long as the one handed to them at a parameter position
    /// ([`length_preserving_functions`]).
    len_summaries: std::rc::Rc<HashMap<Symbol, usize>>,
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
            callee_globals: std::rc::Rc::new(CalleeGlobals::default()),
            length_def: HashMap::new(),
            param_colls: std::collections::HashSet::new(),
            scalar_def: HashMap::new(),
            scalar_upper: HashMap::new(),
            elem_upper: HashMap::new(),
            elem_type: HashMap::new(),
            scalar_lower: HashMap::new(),
            aot_entry_guard: false,
            entry_guard_lowers: Vec::new(),
            karr: super::karr::AffineSpace::top(),
            seq_vars: std::collections::HashSet::new(),
            seq_types: std::rc::Rc::new(std::collections::HashSet::new()),
            len_summaries: std::rc::Rc::new(HashMap::new()),
        }
    }

    /// Every equality through `sym`'s value or its length is gone.
    fn karr_forget(&mut self, sym: Symbol) {
        self.karr.forget(super::affine::var_index(sym));
        self.karr.forget(super::affine::len_index(sym));
    }

    /// The collection `c` may have changed length: what any other name for its allocation says about its own length
    /// is gone. The names for one allocation are the alias graph's linked names (as the length intervals take them,
    /// [`rich_grow`]); a resize through a handle of unknown provenance reaches every other such handle too.
    fn karr_resized(&mut self, c: Symbol) {
        let names = self.aliases.may_alias(c);
        let tainted = &self.aliases.tainted;
        let unknown = tainted.contains(&c);
        let stale: Vec<i64> = self
            .karr
            .tracked()
            .iter()
            .copied()
            .filter(|&d| {
                super::affine::len_owner(d).is_some_and(|a| {
                    a != c && (names.contains(&a) || (unknown && tainted.contains(&a)))
                })
            })
            .collect();
        for d in stale {
            self.karr.forget(d);
        }
    }

    /// `sym` may hold anything now, a collection of any length sharing any allocation it shared: nothing `karr`
    /// says through it holds.
    fn karr_unknown(&mut self, sym: Symbol) {
        self.karr_resized(sym);
        self.karr_forget(sym);
        self.seq_vars.remove(&sym);
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
            ty: self
                .types
                .get(&sym)
                .cloned()
                .unwrap_or(TypeAbstraction::Top),
            shape: self
                .shapes
                .get(&sym)
                .cloned()
                .unwrap_or(CollectionShape::Top),
            nullability: self
                .nullability
                .get(&sym)
                .cloned()
                .unwrap_or(Nullability::Maybe),
            alias,
        }
    }

    /// Forget every scalar fact about `sym` (used when its value is overwritten
    /// or produced by an operation we do not model). Rebinding can change
    /// KIND too, so the collection proof drops with it.
    /// A call ran: every global some callee assigns is unknown now (everything is, if a callee holds
    /// raw foreign code).
    fn forget_callee_globals(&mut self) {
        let globals = self.callee_globals.clone();
        if globals.opaque {
            self.forget_everything();
            return;
        }
        for g in &globals.symbols {
            self.karr_unknown(*g);
            self.invalidate_var(*g);
            self.aliases.unlink(*g);
            self.intervals.set_length(*g, Interval::non_negative());
            self.length_def.remove(g);
        }
    }

    /// Raw foreign code ran: no fact about any name survives. Declared function return types, the
    /// parameter-collection set, and the entry-guard mode are structural and stay.
    fn forget_everything(&mut self) {
        self.intervals.forget_all();
        self.types.clear();
        self.shapes.clear();
        self.nullability.clear();
        self.aliases = AliasGraph::new();
        self.coll_vars.clear();
        self.length_def.clear();
        self.scalar_def.clear();
        self.scalar_upper.clear();
        self.elem_upper.clear();
        self.elem_type.clear();
        self.scalar_lower.clear();
        self.entry_guard_lowers.clear();
        self.karr = super::karr::AffineSpace::top();
        self.seq_vars.clear();
    }

    /// Nothing this state says about the names in `written` holds any longer: each may hold anything, alias
    /// anything, have any length, and no affine definition names or mentions it.
    fn forget_written(&mut self, written: &[Symbol]) {
        for &m in written {
            self.karr_unknown(m);
            self.invalidate_var(m);
            self.aliases.unlink(m);
            self.intervals.set_length(m, Interval::non_negative());
            self.length_def.remove(&m);
            let mi = m.index() as i64;
            self.scalar_def.remove(&m);
            self.scalar_def
                .retain(|_, e| !e.coefficients.contains_key(&mi));
        }
    }

    fn invalidate_var(&mut self, sym: Symbol) {
        self.karr_forget(sym);
        self.seq_vars.remove(&sym);
        self.intervals.set_var(sym, Interval::top());
        self.types.insert(sym, TypeAbstraction::Top);
        self.shapes.insert(sym, CollectionShape::Top);
        self.nullability.insert(sym, Nullability::Maybe);
        self.coll_vars.remove(&sym);
        self.intervals.clear_elem(&sym);
        let si = sym.index() as i64;
        self.scalar_upper.remove(&sym);
        self.scalar_upper
            .retain(|_, e| !e.coefficients.contains_key(&si));
        self.elem_upper.remove(&sym);
        self.elem_upper
            .retain(|_, e| !e.coefficients.contains_key(&si));
        self.elem_type.remove(&sym);
        self.scalar_lower.remove(&sym);
        self.scalar_lower
            .retain(|_, e| !e.coefficients.contains_key(&si));
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

/// A variable's program-wide PROVEN facts — the statically-verified summary the
/// debugger shows beside its live value (distinct from the dynamic, observed-this-run
/// invariants). All fields are sound over-approximations: present ⇒ proven.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VarProvenFacts {
    /// Proven scalar type, if concrete across the program.
    pub scalar: Option<ScalarKind>,
    /// Proven finite integer range `[lo, hi]` (the value was in it at every occurrence).
    pub int_range: Option<(i64, i64)>,
    /// Proven non-negative everywhere (even if unbounded above).
    pub nonneg: bool,
}

/// EXODIA Phase 1's DELIVERY layer: a product fact for every expression
/// occurrence at its program point, keyed by ARENA ADDRESS (stable for the
/// analyzed snapshot — record facts immediately before consuming them, and
/// never across a pass that re-allocates expressions). Shared subtrees that
/// appear at several program points JOIN their facts (sound; precision is
/// lost exactly where sharing occurs).
#[derive(Default)]
pub struct OracleFacts {
    /// How many loop fixpoints are open on the current walk — [`rich_walk_loop`] bounds the nesting
    /// at which a loop still gets its own fixpoint.
    loop_depth: usize,
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
    /// The affine equalities ([`super::karr`]) holding where each index expression (by address) is evaluated,
    /// joined over every recorded visit: what relates a store's index to the lengths around it.
    karr_at: HashMap<usize, super::karr::AffineSpace>,
    /// While a function is walked for its length summary ([`length_preserving_functions`]): each `Return`
    /// statement's address, the name it returns, and the equalities where it returns.
    return_probe: Option<Vec<(usize, Option<Symbol>, super::karr::AffineSpace)>>,
}

/// A region-entry bounds hoist in SYMBOL terms (the compiler maps to
/// registers): `length(array) >= bound + add_max` and `iv + add_min >= 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        self.hoist_descs
            .get(&loop_ptr)
            .map_or(&[], |v| v.as_slice())
    }

    /// Is this `index` sub-expression proven in bounds only SPECULATIVELY
    /// (needs a region-entry guard)? The compiler elides it solely when it
    /// emitted that guard.
    pub fn index_is_speculative(&self, index: &Expr) -> bool {
        self.speculative_inbounds
            .contains(&(index as *const Expr as usize))
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
        self.expr_proven_at_least(e, 0)
    }

    /// Is this occurrence's proven LOWER bound a finite `>= k`, whatever its upper bound? A 1-based index proven
    /// `>= 1` names its item from the front.
    pub fn expr_proven_at_least(&self, e: &Expr, k: i64) -> bool {
        match self.exprs.get(&(e as *const Expr as usize)) {
            Some(av) => matches!(av.interval.lo, Bound::Finite(v) if v >= k),
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

    /// **Debugger-only** by-name rollup: walk `stmts` and JOIN every `Identifier(sym)`
    /// occurrence's already-recorded abstract value into one proven summary per variable.
    /// Sound by construction (joining widens), so a returned `int_range` of `[0, 9]` means
    /// the variable was within it at *every* use — a program-wide proven invariant.
    ///
    /// This re-reads the facts that the production walk already computed; it adds NOTHING
    /// to `record_expr`/`oracle_analyze`, so it is **zero cost** unless the debugger calls
    /// it (no production path does). Reused from the same module so it can read `exprs`.
    pub fn summarize_variables(&self, stmts: &[Stmt]) -> Vec<(Symbol, VarProvenFacts)> {
        let mut by_sym: HashMap<Symbol, AbstractValue> = HashMap::new();
        collect_idents_block(stmts, &mut |sym, addr| {
            if let Some(av) = self.exprs.get(&addr) {
                match by_sym.get_mut(&sym) {
                    None => {
                        by_sym.insert(sym, av.clone());
                    }
                    Some(prev) => *prev = prev.join(av),
                }
            }
        });
        by_sym
            .into_iter()
            .map(|(sym, av)| {
                let int_range = match (&av.interval.lo, &av.interval.hi) {
                    (Bound::Finite(lo), Bound::Finite(hi)) if lo <= hi => Some((*lo, *hi)),
                    _ => None,
                };
                let scalar = match &av.ty {
                    TypeAbstraction::Concrete(TypeTag::Int) => Some(ScalarKind::Int),
                    TypeAbstraction::Concrete(TypeTag::Float) => Some(ScalarKind::Float),
                    TypeAbstraction::Concrete(TypeTag::Bool) => Some(ScalarKind::Bool),
                    TypeAbstraction::Concrete(TypeTag::Text) => Some(ScalarKind::Text),
                    _ => None,
                };
                let nonneg = matches!(av.interval.lo, Bound::Finite(v) if v >= 0);
                (
                    sym,
                    VarProvenFacts {
                        scalar,
                        int_range,
                        nonneg,
                    },
                )
            })
            .collect()
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
    pub fn loop_handles_definitely_distinct(&self, loop_stmt: &Stmt, a: Symbol, b: Symbol) -> bool {
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
        self.relational_inbounds
            .contains(&(index as *const Expr as usize))
    }

    /// The guard-elision query (EXODIA 1.3): is `index` PROVABLY within the
    /// 1-based bounds of `collection` at this program point? True only when
    /// the index interval sits inside [1, len.lo].
    pub fn index_provably_in_bounds(&self, collection: &Expr, index: &Expr) -> bool {
        // Relational induction-variable bound (SCEV-style): the loop guard
        // already proved `1 <= index <= length(collection)` for this exact
        // read. Intervals alone cannot express it.
        if self
            .relational_inbounds
            .contains(&(index as *const Expr as usize))
        {
            return true;
        }
        // Non-relational interval check: a concrete index interval inside
        // `[1, proven-minimum-length]`.
        let Some(len) = self.lengths.get(&(collection as *const Expr as usize)) else {
            return false;
        };
        let Some((ilo, ihi)) = self.expr_int_range(index) else {
            return false;
        };
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

    /// Whether `index` names an item of `collection` on EVERY run: a construct of
    /// the compile proved it ([`super::facts::Fact::InBounds`]), or
    /// [`Self::index_provably_in_bounds`] holds with no precondition. The one
    /// question an access that nothing checks rests on.
    pub fn index_always_in_bounds(&self, collection: &Expr, index: &Expr) -> bool {
        super::facts::holds_expr(super::facts::Fact::InBounds, index)
            || (self.index_provably_in_bounds(collection, index)
                && self.index_positivity_guards(index).is_empty())
    }
}

/// Record the equalities holding where the index expression `index` is evaluated, joined with any visit recorded
/// before.
fn record_karr(index: &Expr, st: &RichAbstractState, facts: &mut OracleFacts) {
    if facts.suppress_exprs {
        return;
    }
    let key = index as *const Expr as usize;
    match facts.karr_at.entry(key) {
        std::collections::hash_map::Entry::Occupied(mut e) => {
            let joined = e.get().join(&st.karr);
            e.insert(joined);
        }
        std::collections::hash_map::Entry::Vacant(v) => {
            v.insert(st.karr.clone());
        }
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
        | Expr::Range {
            start: left,
            end: right,
        } => {
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
            record_karr(index, st, facts);
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
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
            Expr::Identifier(sym) => st.shapes.get(sym).cloned().unwrap_or(CollectionShape::Top),
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

/// **Debugger-only** traversal: invoke `f(sym, addr)` for every `Identifier(sym)`
/// occurrence in `stmts`, mirroring the production walk's coverage so the addresses
/// line up with the recorded `exprs`. Never called on a production compile path —
/// only [`OracleFacts::summarize_variables`] uses it. Incompleteness here would only
/// narrow the debugger's summary (still sound), never affect codegen.
fn collect_idents_block(stmts: &[Stmt], f: &mut dyn FnMut(Symbol, usize)) {
    for s in stmts {
        collect_idents_stmt(s, f);
    }
}

fn collect_idents_stmt(stmt: &Stmt, f: &mut dyn FnMut(Symbol, usize)) {
    use crate::ast::stmt::ReadSource;
    // Direct expressions a statement holds (mirror `record_stmt_exprs`).
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => collect_idents_expr(value, f),
        Stmt::Return { value: Some(e) } => collect_idents_expr(e, f),
        Stmt::Call { args, .. } => {
            for a in args {
                collect_idents_expr(a, f);
            }
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => collect_idents_expr(cond, f),
        Stmt::Repeat { iterable, .. } => collect_idents_expr(iterable, f),
        Stmt::Show { object, .. } | Stmt::Give { object, .. } => collect_idents_expr(object, f),
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            collect_idents_expr(value, f);
            collect_idents_expr(collection, f);
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            collect_idents_expr(collection, f);
            collect_idents_expr(index, f);
            collect_idents_expr(value, f);
        }
        Stmt::SetField { object, value, .. } => {
            collect_idents_expr(object, f);
            collect_idents_expr(value, f);
        }
        Stmt::Inspect { target, .. } => collect_idents_expr(target, f),
        Stmt::RuntimeAssert { condition, .. } => collect_idents_expr(condition, f),
        Stmt::Sleep { milliseconds } => collect_idents_expr(milliseconds, f),
        Stmt::ReadFrom {
            source: ReadSource::File(p),
            ..
        } => collect_idents_expr(p, f),
        _ => {}
    }
    // Nested blocks (mirror the block recursion the production walk does).
    match stmt {
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            collect_idents_block(then_block, f);
            if let Some(eb) = else_block {
                collect_idents_block(eb, f);
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => collect_idents_block(body, f),
        Stmt::FunctionDef { body, .. } => collect_idents_block(body, f),
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                collect_idents_block(arm.body, f);
            }
        }
        Stmt::Zone { body, .. } => collect_idents_block(body, f),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => collect_idents_block(tasks, f),
        _ => {}
    }
}

fn collect_idents_expr(e: &Expr, f: &mut dyn FnMut(Symbol, usize)) {
    use crate::ast::stmt::StringPart;
    if let Expr::Identifier(sym) = e {
        f(*sym, e as *const Expr as usize);
    }
    match e {
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            collect_idents_expr(left, f);
            collect_idents_expr(right, f);
        }
        Expr::Not { operand } => collect_idents_expr(operand, f),
        Expr::Call { args, .. } => {
            for a in args {
                collect_idents_expr(a, f);
            }
        }
        Expr::CallExpr { callee, args } => {
            collect_idents_expr(callee, f);
            for a in args {
                collect_idents_expr(a, f);
            }
        }
        Expr::Index { collection, index } => {
            collect_idents_expr(collection, f);
            collect_idents_expr(index, f);
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            collect_idents_expr(collection, f);
            collect_idents_expr(start, f);
            collect_idents_expr(end, f);
        }
        Expr::Copy { expr } => collect_idents_expr(expr, f),
        Expr::Give { value } => collect_idents_expr(value, f),
        Expr::Length { collection } => collect_idents_expr(collection, f),
        Expr::Contains { collection, value } => {
            collect_idents_expr(collection, f);
            collect_idents_expr(value, f);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                collect_idents_expr(i, f);
            }
        }
        Expr::FieldAccess { object, .. } => collect_idents_expr(object, f),
        Expr::New { init_fields, .. } => {
            for (_, fe) in init_fields {
                collect_idents_expr(fe, f);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, fe) in fields {
                collect_idents_expr(fe, f);
            }
        }
        Expr::OptionSome { value } => collect_idents_expr(value, f),
        Expr::WithCapacity { value, capacity } => {
            collect_idents_expr(value, f);
            collect_idents_expr(capacity, f);
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let StringPart::Expr { value, .. } = p {
                    collect_idents_expr(value, f);
                }
            }
        }
        _ => {}
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
    record_proven_reads(
        body,
        &guards,
        &state.length_def,
        &mut clobbered,
        facts,
        &state.callee_globals,
    );
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
    /// The sequences a guard may read `length of` from: tracked sequences the loop neither resizes nor rebinds, so
    /// the length a guard reads at the loop head is the length at every point of the body.
    lengths: std::rc::Rc<HashSet<Symbol>>,
    /// The walk that proves accesses from the equalities recorded where they are evaluated
    /// ([`try_prove_affine_karr`]), which runs once the loop body's record pass has recorded them; the other proofs
    /// run on the walk before it.
    karr_pass: bool,
}

fn record_affine_index_bounds(
    cond: &Expr,
    body: &[Stmt],
    state: &RichAbstractState,
    entry: &RichAbstractState,
    facts: &mut OracleFacts,
    karr_pass: bool,
) {
    if !body_is_index_proof_safe(body) {
        return;
    }
    let mut clobbered: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
    collect_resized_arrays(body, &mut clobbered);
    let mutated: std::collections::HashSet<Symbol> =
        collect_mutations_in(body, &state.callee_globals)
            .into_iter()
            .collect();
    // The loop fixpoint merges `scalar_upper`/`scalar_lower` across the back-edge
    // but NOT `scalar_def`, so a mid-body `Let complement be target - x` is gone
    // by the record state. Re-establish the body's TOP-LEVEL affine definitions
    // (their RHS is straight-line before every later use, and the bound var is not
    // reassigned), so a key proof can follow the chain `complement -> target - x
    // -> x`'s element bounds`. Sound: each added equality holds at every use site.
    let state = {
        let mut st = state.clone();
        // The entry-guard precondition lowers (`lo>=1`,`hi>=1`) hold throughout the
        // entry-guarded function body, but the loop's record-state may not carry them
        // through the fixpoint — copy them from the loop-ENTRY state, which does.
        if st.entry_guard_lowers.is_empty() {
            st.entry_guard_lowers = entry.entry_guard_lowers.clone();
        }
        augment_body_scalar_defs(body, &mut st);
        st
    };
    let inits: HashMap<Symbol, i64> = HashMap::new();
    let mut amb = AffineFacts {
        lengths: std::rc::Rc::new(
            state
                .seq_vars
                .iter()
                .copied()
                .filter(|c| !clobbered.contains(c) && !mutated.contains(c))
                .collect(),
        ),
        karr_pass,
        ..AffineFacts::default()
    };
    affine_add_loop_guards(cond, body, &state, Some(entry), &inits, &mut amb);
    affine_walk(
        body, body, &state, &clobbered, &mutated, &amb, &inits, facts,
    );
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
    let reassigned = reassigned_scalars(body, &st.callee_globals);
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

/// Every scalar `body` may REASSIGN (`Set`, `Swap`, a global some callee it calls assigns) — the
/// names whose body-local definitions are not stable iteration invariants. After foreign code every
/// name the body binds or mentions is suspect.
fn reassigned_scalars(body: &[Stmt], globals: &CalleeGlobals) -> std::collections::HashSet<Symbol> {
    let writes = WriteSet::of_block(body).with_callee_globals(globals);
    if writes.is_opaque() {
        return collect_mutations_in(body, globals).into_iter().collect();
    }
    writes.assigned().clone()
}

/// `a <op> b` (optionally negated) as a kernel constraint, when both sides are
/// affine.
fn affine_cmp(
    op: BinaryOpKind,
    a: &Expr,
    b: &Expr,
    negate: bool,
    lengths: &HashSet<Symbol>,
) -> Option<super::affine::Constraint> {
    use super::affine::{ge, gt, le, lin_of_with_lengths, lt};
    let fixed = |c: Symbol| lengths.contains(&c);
    let (la, lb) = (
        lin_of_with_lengths(a, &fixed)?,
        lin_of_with_lengths(b, &fixed)?,
    );
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
        Expr::BinaryOp {
            op: BinaryOpKind::And,
            left,
            right,
        } if !negate => {
            affine_add_cmp(left, false, out);
            affine_add_cmp(right, false, out);
        }
        Expr::BinaryOp {
            op: op @ (BinaryOpKind::Lt | BinaryOpKind::LtEq | BinaryOpKind::Gt | BinaryOpKind::GtEq),
            left,
            right,
        } => {
            if let Some(c) = affine_cmp(*op, left, right, negate, &out.lengths) {
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
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
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
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = value
    {
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

/// `Set v to v - c` (literal `c >= 0`) → `Some(c)` — the decreasing counterpart of
/// [`self_increment`], used to recognize a monotonically-shrinking loop bound.
pub(crate) fn self_decrement(v: Symbol, value: &Expr) -> Option<i64> {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Subtract,
        left,
        right,
    } = value
    {
        if let (Expr::Identifier(s), Expr::Literal(Literal::Number(c))) = (&**left, &**right) {
            if *s == v && *c >= 0 {
                return Some(*c);
            }
        }
    }
    None
}

/// Variables that have a TOP-LEVEL (direct child of `body`, not nested in any
/// `If`/`While`) `Set v to v + dj` (`dj > 0`) that is `v`'s ONLY write in the
/// whole body — so each iteration increments `v` by exactly `dj`.
fn top_level_unconditional_incs(body: &[Stmt], globals: &CalleeGlobals) -> Vec<(Symbol, i64)> {
    let mut out = Vec::new();
    for s in body {
        if let Stmt::Set { target, value } = s {
            if let Some(c) = self_increment(*target, value) {
                if c > 0 && count_writes_of(body, *target, globals) == 1 {
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
fn monotone_inc_within(
    stmts: &[Stmt],
    v: Symbol,
    in_loop: bool,
    total: &mut i64,
    count: &mut usize,
) -> bool {
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
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
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
    for (j, dj) in top_level_unconditional_incs(body, &entry.callee_globals) {
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
            // SYMBOLIC lower: `i`/`j` start at their shared entry def `U` (parameter
            // `lo`) and only INCREASE, so `i >= U`, `j >= U`. Chained with the
            // entry-guard's `scalar_lower[lo] >= 1` (pushed into the system for U's
            // symbols), this discharges `1 <= E` for a SYMBOLIC `lo` — the partition
            // swap's `item i`. Sound: a monotone-increasing var never drops below its
            // initial value. Add U's symbols so `1 <= lo` (its scalar_lower) enters.
            // `i`/`j` start at their shared entry def `U` (parameter `lo`) and only
            // INCREASE, so `i >= U`, `j >= U`. Chained with the function-wide entry-guard
            // ambient `lo >= 1` (injected in every proof system), this discharges
            // `1 <= i,j` for the partition swap. Sound: monotone-increasing from init.
            if let Some(u) = entry.scalar_def.get(&i) {
                amb.constraints.push(ge(&var(i), u));
                amb.constraints.push(ge(&var(j), u));
                for &si in u.coefficients.keys() {
                    let s = Symbol::from_index(si as usize);
                    if !amb.syms.contains(&s) {
                        amb.syms.push(s);
                    }
                }
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
        collect_mutations_in(body, &state.callee_globals)
            .into_iter()
            .collect();
    let mut ivs: Vec<Symbol> = Vec::new();
    affine_collect_syms(cond, &mut ivs);
    for iv in ivs {
        if let Some(lo) = iv_lower_bound(iv, body, state, &mutated, inits) {
            amb.constraints.push(super::affine::ge(
                &super::affine::var(iv),
                &super::affine::konst(lo),
            ));
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
        derive_monotone_dec_invariants(body, entry, &mutated, amb);
    }
}

/// A variable whose ONLY write in `body` is a single top-level `Set v to v - c`
/// (`c >= 0`) monotonically DECREASES, so `v <= v_entry` holds throughout the loop.
/// Its loop-entry def (`entry.scalar_def[v]` = `n - 1`, captured before the loop's
/// first decrement) then discharges the UPPER bound of an `item v of arr` access
/// when a length lower bound relates `arr` to that def (`len >= n`) — e.g.
/// heap_sort's main-phase `arr[end]` swap. The decreasing mirror of
/// [`derive_iv_le_invariants`]. SOUND: a value that only shrinks never exceeds its
/// loop-entry value, and the entry def is used only when its own variables are
/// loop-invariant (so it names the same fixed value at every access).
fn derive_monotone_dec_invariants(
    body: &[Stmt],
    entry: &RichAbstractState,
    mutated: &std::collections::HashSet<Symbol>,
    amb: &mut AffineFacts,
) {
    use super::affine::{le, var};
    for s in body {
        if let Stmt::Set { target, value } = s {
            if self_decrement(*target, value).is_some()
                && count_writes_of(body, *target, &entry.callee_globals) == 1
            {
                if let Some(e0) = entry.scalar_def.get(target) {
                    let e0_stable = e0
                        .coefficients
                        .keys()
                        .all(|&si| !mutated.contains(&Symbol::from_index(si as usize)));
                    if e0_stable {
                        amb.constraints.push(le(&var(*target), e0));
                        for &si in e0.coefficients.keys() {
                            let sym = Symbol::from_index(si as usize);
                            if !amb.syms.contains(&sym) {
                                amb.syms.push(sym);
                            }
                        }
                        if !amb.syms.contains(target) {
                            amb.syms.push(*target);
                        }
                    }
                }
            }
        }
    }
}

/// Drop ambient facts mentioning any symbol in `set` (a write reaches earlier
/// reads across loop iterations, or a sequential reassignment invalidates a
/// guard).
fn affine_drop_mentioning(amb: &mut AffineFacts, set: &std::collections::HashSet<Symbol>) {
    amb.constraints.retain(|c| {
        !set.iter()
            .any(|s| c.expr.coefficients.contains_key(&(s.index() as i64)))
    });
    amb.syms.retain(|s| !set.contains(s));
}

fn affine_drop_one(amb: &mut AffineFacts, sym: Symbol) {
    let k = sym.index() as i64;
    amb.constraints
        .retain(|c| !c.expr.coefficients.contains_key(&k));
    amb.syms.retain(|s| *s != sym);
}

/// Prove every `Index` node in an expression in place (no ref storage —
/// avoids threading the arena lifetime through a collecting `Vec`).
fn affine_prove_in_expr(
    e: &Expr,
    body: &[Stmt],
    state: &RichAbstractState,
    clobbered: &std::collections::HashSet<Symbol>,
    mutated: &std::collections::HashSet<Symbol>,
    amb: &AffineFacts,
    facts: &mut OracleFacts,
) {
    if let Expr::Index { collection, index } = e {
        if amb.karr_pass {
            try_prove_affine_karr(collection, index, state, mutated, amb, facts);
        } else {
            // Dense `Map of Int to Int` get: `item k of m` → prove `0 <= k <= cap(m)`.
            if let Expr::Identifier(m) = collection {
                if facts.map_caps.contains_key(m) {
                    try_prove_dense_key(*m, index, state, mutated, amb, facts);
                }
            }
            try_prove_affine(
                collection, index, body, state, clobbered, mutated, amb, facts,
            );
        }
    }
    // Dense map membership: `m contains k` → prove `0 <= k <= cap(m)`.
    if let (Expr::Contains { collection, value }, false) = (e, amb.karr_pass) {
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
        | Expr::Range {
            start: left,
            end: right,
        } => {
            affine_prove_in_expr(left, body, state, clobbered, mutated, amb, facts);
            affine_prove_in_expr(right, body, state, clobbered, mutated, amb, facts);
        }
        Expr::Not { operand } => {
            affine_prove_in_expr(operand, body, state, clobbered, mutated, amb, facts)
        }
        Expr::Contains { collection, value } => {
            affine_prove_in_expr(collection, body, state, clobbered, mutated, amb, facts);
            affine_prove_in_expr(value, body, state, clobbered, mutated, amb, facts);
        }
        Expr::Index { collection, index } => {
            affine_prove_in_expr(collection, body, state, clobbered, mutated, amb, facts);
            affine_prove_in_expr(index, body, state, clobbered, mutated, amb, facts);
        }
        Expr::Call { args, .. } => {
            for a in args {
                affine_prove_in_expr(a, body, state, clobbered, mutated, amb, facts);
            }
        }
        Expr::Length { collection } => {
            affine_prove_in_expr(collection, body, state, clobbered, mutated, amb, facts)
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
    full_body: &[Stmt],
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
            affine_prove_in_expr(e, full_body, state, clobbered, mutated, &local, facts);
        });
        if let Stmt::SetIndex {
            collection, index, ..
        } = stmt
        {
            if local.karr_pass {
                try_prove_affine_karr(collection, index, state, mutated, &local, facts);
            } else {
                // Dense `Map of Int to Int` insert: `Set item k of m to v` → prove
                // `0 <= k <= cap(m)`.
                if let Expr::Identifier(m) = collection {
                    if facts.map_caps.contains_key(m) {
                        try_prove_dense_key(*m, index, state, mutated, &local, facts);
                    }
                }
                try_prove_affine(
                    collection, index, full_body, state, clobbered, mutated, &local, facts,
                );
            }
        }
        match stmt {
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let mut then_facts = local.clone();
                affine_add_cmp(cond, false, &mut then_facts);
                affine_walk(
                    then_block,
                    full_body,
                    state,
                    clobbered,
                    mutated,
                    &then_facts,
                    &inits,
                    facts,
                );
                if let Some(eb) = else_block {
                    let mut else_facts = local.clone();
                    affine_add_cmp(cond, true, &mut else_facts);
                    affine_walk(
                        eb,
                        full_body,
                        state,
                        clobbered,
                        mutated,
                        &else_facts,
                        &inits,
                        facts,
                    );
                }
            }
            Stmt::While { cond, body, .. } => {
                // A nested loop's body may rewrite a variable an ambient fact
                // rests on (a later iteration reaching an earlier read), so drop
                // facts naming anything it mutates before adding its own guard.
                let nested: std::collections::HashSet<Symbol> =
                    collect_mutations_in(body, &state.callee_globals)
                        .into_iter()
                        .collect();
                let mut inner = local.clone();
                affine_drop_mentioning(&mut inner, &nested);
                affine_add_loop_guards(cond, body, state, None, &inits, &mut inner);
                affine_walk(
                    body, full_body, state, clobbered, mutated, &inner, &inits, facts,
                );
            }
            Stmt::Repeat { body, .. } => {
                affine_walk(
                    body, full_body, state, clobbered, mutated, &local, &inits, facts,
                );
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

/// Discharge the index accesses a single TOP-LEVEL statement holds, at `st` (the
/// statement's pre-state) — the same affine proof `affine_walk` runs over a loop
/// body, applied to a statement OUTSIDE any loop. The entry-guarded partition's
/// post-loop pivot swap (`item i of arr`) rests on the loop-EXIT invariant
/// `lo <= i <= hi` that `rich_walk_loop` records into `st`; the function-wide
/// entry-guard lowers (`lo>=1`,`hi>=1`) reach the proof from `st.entry_guard_lowers`
/// inside `try_prove_affine`, so the ambient system is empty here. `body` is the
/// whole function body (the `sym_ge_lo`/`sym_le_sym` walk domain).
fn prove_toplevel_accesses(
    stmt: &Stmt,
    body: &[Stmt],
    st: &RichAbstractState,
    clobbered: &std::collections::HashSet<Symbol>,
    mutated: &std::collections::HashSet<Symbol>,
    facts: &mut OracleFacts,
) {
    let amb = AffineFacts::default();
    for_each_direct_expr(stmt, &mut |e| {
        affine_prove_in_expr(e, body, st, clobbered, mutated, &amb, facts);
    });
    if let Stmt::SetIndex {
        collection, index, ..
    } = stmt
    {
        try_prove_affine(collection, index, body, st, clobbered, mutated, &amb, facts);
    }
}

/// Prove `value >= lo` under the assume-guarantee hypothesis that every symbol in
/// `assumed` is already `>= lo` at the point of assignment.
///
/// **Soundness (co-induction over the `>= lo` safety invariant).** The `assumed`
/// shortcut is the co-inductive hypothesis; it is sound because the invariant's BASE
/// holds: every LOGOS variable has a grounded first definition (use-before-def is
/// rejected by scope analysis *before* the optimizer runs), so every recursion
/// bottoms out in a literal, a parameter interval / entry-guard lower, or an
/// unconditional `>= lo` assignment not depending on the hypothesis. A variable is
/// proven `>= lo` only when EVERY value assigned to it in `body` is `>= lo`. The
/// index arithmetic stays overflow-CHECKED, so the runtime index equals this value
/// (or the program panics first) — sound for `get_unchecked`.
fn value_ge_lo(
    value: &Expr,
    lo: i64,
    body: &[Stmt],
    state: &RichAbstractState,
    mutated: &std::collections::HashSet<Symbol>,
    assumed: &std::collections::HashSet<Symbol>,
    fuel: u32,
) -> bool {
    if fuel == 0 {
        return false;
    }
    match value {
        Expr::Literal(Literal::Number(n)) => *n >= lo,
        Expr::Identifier(s) => {
            if assumed.contains(s) {
                return true;
            }
            if !mutated.contains(s) {
                if matches!(state.intervals.get_var(s).lo, Bound::Finite(l) if l >= lo) {
                    return true;
                }
            }
            if state
                .entry_guard_lowers
                .iter()
                .any(|(g, l)| g == s && *l >= lo)
            {
                return true;
            }
            let mut a2 = assumed.clone();
            a2.insert(*s);
            sym_ge_lo(*s, lo, body, state, mutated, &a2, fuel - 1)
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            (value_ge_lo(left, lo, body, state, mutated, assumed, fuel - 1)
                && value_ge_lo(right, 0, body, state, mutated, assumed, fuel - 1))
                || (value_ge_lo(left, 0, body, state, mutated, assumed, fuel - 1)
                    && value_ge_lo(right, lo, body, state, mutated, assumed, fuel - 1))
        }
        // `k * v` with both factors `>= 0` is `>= 0 >= lo` (only claimed for `lo <= 0`).
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left,
            right,
        } => {
            lo <= 0
                && value_ge_lo(left, 0, body, state, mutated, assumed, fuel - 1)
                && value_ge_lo(right, 0, body, state, mutated, assumed, fuel - 1)
        }
        _ => false,
    }
}

/// `sym >= lo` whenever it has at least one definition in `body` and EVERY value
/// assigned to it (its `Let` init and all `Set`s) is `>= lo` (via [`value_ge_lo`]).
fn sym_ge_lo(
    sym: Symbol,
    lo: i64,
    body: &[Stmt],
    state: &RichAbstractState,
    mutated: &std::collections::HashSet<Symbol>,
    assumed: &std::collections::HashSet<Symbol>,
    fuel: u32,
) -> bool {
    if fuel == 0 {
        return false;
    }
    fn walk(
        sym: Symbol,
        stmts: &[Stmt],
        lo: i64,
        body: &[Stmt],
        state: &RichAbstractState,
        mutated: &std::collections::HashSet<Symbol>,
        assumed: &std::collections::HashSet<Symbol>,
        fuel: u32,
        seen: &mut bool,
    ) -> bool {
        stmts.iter().all(|s| match s {
            Stmt::Set { target, value } if *target == sym => {
                *seen = true;
                value_ge_lo(value, lo, body, state, mutated, assumed, fuel)
            }
            Stmt::Let { var, value, .. } if *var == sym => {
                *seen = true;
                value_ge_lo(value, lo, body, state, mutated, assumed, fuel)
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                walk(
                    sym, then_block, lo, body, state, mutated, assumed, fuel, seen,
                ) && else_block.map_or(true, |eb| {
                    walk(sym, eb, lo, body, state, mutated, assumed, fuel, seen)
                })
            }
            Stmt::While { body: b, .. } | Stmt::Repeat { body: b, .. } => {
                walk(sym, b, lo, body, state, mutated, assumed, fuel, seen)
            }
            _ => true,
        })
    }
    let mut seen = false;
    let ok = walk(
        sym, body, lo, body, state, mutated, assumed, fuel, &mut seen,
    );
    ok && seen
}

/// Prove `value <= bound_sym` under `base` plus the affine `path` guards enclosing
/// the assignment (each as `(cond, negate)`), plus the scalar definitions of the
/// value's own symbols (so `child = 2*root+1` reaches the loop guard `2*root+1 <=
/// end`). Path-sensitive: `Set s to child+1` inside `if child+1 <= end` uses that
/// guard.
fn prove_le_bound(
    value: &Expr,
    bound_sym: Symbol,
    base: &[super::affine::Constraint],
    path: &[(&Expr, bool)],
    state: &RichAbstractState,
    local_defs: &HashMap<Symbol, super::affine::LinExpr>,
) -> bool {
    use super::affine::{consistent, le, lin_of, prove, var};
    let Some(v_lin) = lin_of(value) else {
        return false;
    };
    let mut facts = AffineFacts {
        constraints: base.to_vec(),
        ..AffineFacts::default()
    };
    for &(c, neg) in path {
        affine_add_cmp(c, neg, &mut facts);
    }
    let mut vsyms = Vec::new();
    affine_collect_syms(value, &mut vsyms);
    for s in vsyms {
        // A body scalar-def (`cols = capacity + 1`) OR a loop-local single-`Let`
        // affine binding (`child = 2*root+1`). The latter is dropped from the
        // shared `scalar_def` when its RHS (`root`) is reassigned, but `child = E`
        // still holds at every use inside the iteration — and is SOUND here because
        // it only combines with the loop guard, which is itself dropped from `base`
        // once `root` is reassigned, so the pair is available only before the change.
        if let Some(def) = state.scalar_def.get(&s).or_else(|| local_defs.get(&s)) {
            facts.constraints.push(le(&var(s), def));
            facts.constraints.push(le(def, &var(s)));
        }
    }
    consistent(&facts.constraints) && prove(&facts.constraints, &var(bound_sym).sub(&v_lin))
}

/// Single-`Let`, never-`Set` affine bindings of `body` (`child = 2*root+1`),
/// INCLUDING those whose RHS is later reassigned — sound for [`prove_le_bound`]'s
/// guard-gated use (see there). The permissive counterpart of
/// [`augment_body_scalar_defs`].
fn collect_local_affine_defs(
    body: &[Stmt],
    globals: &CalleeGlobals,
) -> HashMap<Symbol, super::affine::LinExpr> {
    let set_targets = reassigned_scalars(body, globals);
    let mut let_count: HashMap<Symbol, u32> = HashMap::new();
    fn count(stmts: &[Stmt], lc: &mut HashMap<Symbol, u32>) {
        for s in stmts {
            match s {
                Stmt::Let { var, .. } => *lc.entry(*var).or_insert(0) += 1,
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    count(then_block, lc);
                    if let Some(e) = else_block {
                        count(e, lc);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => count(body, lc),
                _ => {}
            }
        }
    }
    count(body, &mut let_count);
    let mut defs: HashMap<Symbol, super::affine::LinExpr> = HashMap::new();
    fn walk(
        stmts: &[Stmt],
        set_targets: &std::collections::HashSet<Symbol>,
        let_count: &HashMap<Symbol, u32>,
        defs: &mut HashMap<Symbol, super::affine::LinExpr>,
    ) {
        for s in stmts {
            match s {
                Stmt::Let { var, value, .. }
                    if !set_targets.contains(var) && let_count.get(var) == Some(&1) =>
                {
                    if let Some(e) = super::affine::lin_of(value) {
                        if !e.coefficients.is_empty()
                            && !e.coefficients.contains_key(&(var.index() as i64))
                        {
                            defs.insert(*var, e);
                        }
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, set_targets, let_count, defs);
                    if let Some(e) = else_block {
                        walk(e, set_targets, let_count, defs);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                    walk(body, set_targets, let_count, defs)
                }
                _ => {}
            }
        }
    }
    walk(body, &set_targets, &let_count, &mut defs);
    defs
}

/// `s <= bound_sym` when it has at least one definition and EVERY value assigned to
/// it is `<= bound_sym` under its enclosing path guards (via [`prove_le_bound`]).
/// The upper counterpart to [`sym_ge_lo`]; unlike the lower bound it needs no
/// co-induction because a heap index's assignments (`root`, `2*root+1`, `child+1`)
/// bound above by the loop / path guards, never by the index itself.
fn sym_le_sym(
    s: Symbol,
    bound_sym: Symbol,
    body: &[Stmt],
    base: &[super::affine::Constraint],
    state: &RichAbstractState,
) -> bool {
    #[allow(clippy::too_many_arguments)]
    fn walk<'a>(
        s: Symbol,
        stmts: &'a [Stmt<'a>],
        bound_sym: Symbol,
        base: &[super::affine::Constraint],
        state: &RichAbstractState,
        local_defs: &HashMap<Symbol, super::affine::LinExpr>,
        path: &mut Vec<(&'a Expr<'a>, bool)>,
        seen: &mut bool,
        fuel: &mut u32,
    ) -> bool {
        if *fuel == 0 {
            return false;
        }
        *fuel -= 1;
        stmts.iter().all(|stmt| match stmt {
            Stmt::Set { target, value } if *target == s => {
                *seen = true;
                prove_le_bound(value, bound_sym, base, path, state, local_defs)
            }
            Stmt::Let { var, value, .. } if *var == s => {
                *seen = true;
                prove_le_bound(value, bound_sym, base, path, state, local_defs)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                path.push((cond, false));
                let ok_t = walk(
                    s, then_block, bound_sym, base, state, local_defs, path, seen, fuel,
                );
                path.pop();
                let ok_e = else_block.map_or(true, |eb| {
                    path.push((cond, true));
                    let r = walk(s, eb, bound_sym, base, state, local_defs, path, seen, fuel);
                    path.pop();
                    r
                });
                ok_t && ok_e
            }
            Stmt::While { body: b, .. } | Stmt::Repeat { body: b, .. } => {
                walk(s, b, bound_sym, base, state, local_defs, path, seen, fuel)
            }
            _ => true,
        })
    }
    let local_defs = collect_local_affine_defs(body, &state.callee_globals);
    let mut path: Vec<(&Expr, bool)> = Vec::new();
    let mut seen = false;
    let mut fuel = 400u32;
    let ok = walk(
        s,
        body,
        bound_sym,
        base,
        state,
        &local_defs,
        &mut path,
        &mut seen,
        &mut fuel,
    );
    ok && seen
}

/// Discharge `1 <= E <= length(arr)` for one access from the affine equalities holding where it is evaluated
/// ([`OracleFacts::karr_at`]) and the guards in scope, against the sequence's own length. A merge's store
/// `item k of result` is the case: `k = li + ri - 1` under `li <= length of left` and `ri <= length of right`, with
/// the builds before the loop making `length of left + length of right` equal to `length of result`. A name the body
/// writes keeps no header interval here; the guards and the equalities speak for it.
///
/// The equalities never reach the prover as pairs of inequalities: each one solves for a variable of its own
/// ([`super::karr::AffineSpace::solved`]), which is substituted out of the goal and every guard exactly. What is left
/// is narrowed to the constraints that share a variable, directly or through others, with the goal, and the prover
/// runs only on a system of at most [`KARR_MAX_CONSTRAINTS`] constraints over [`KARR_MAX_VARS`] variables:
/// Fourier-Motzkin can square its constraint count with each variable it eliminates, and a large generated program
/// holds dozens of equalities at one access. A system past the bound proves nothing, and the access keeps its check.
fn try_prove_affine_karr(
    collection: &Expr,
    index: &Expr,
    state: &RichAbstractState,
    mutated: &HashSet<Symbol>,
    amb: &AffineFacts,
    facts: &mut OracleFacts,
) {
    use super::affine::{
        consistent, konst, le, len_index, len_var, lin_of, prove, var, Constraint, LinExpr,
    };
    let Expr::Identifier(arr) = collection else {
        return;
    };
    let key = index as *const Expr as usize;
    if facts.relational_inbounds.contains(&key) {
        return;
    }
    let Some(karr) = facts.karr_at.get(&key) else {
        return;
    };
    if !karr.tracked().contains(&len_index(*arr)) {
        return;
    }
    let Some(e) = lin_of(index) else { return };

    // The guards and the bounds of every name they or the goal mention.
    let mut system: Vec<Constraint> = amb.constraints.clone();
    let mut mentioned: std::collections::BTreeSet<i64> = e.coefficients.keys().copied().collect();
    mentioned.insert(len_index(*arr));
    for c in &system {
        mentioned.extend(c.expr.coefficients.keys().copied());
    }
    let solved = karr.solved();
    for (_, v) in &solved {
        mentioned.extend(v.coefficients.keys().copied());
    }
    for &(d, _) in &solved {
        mentioned.insert(d);
    }
    for &k in &mentioned {
        if k < 0 {
            system.push(le(&konst(0), &LinExpr::var(k)));
            continue;
        }
        let s = Symbol::from_index(k as usize);
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
    }
    for &(s, lb) in &state.entry_guard_lowers {
        system.push(le(&konst(lb), &var(s)));
    }

    // Substitute every solved equality out of the goals and the system.
    let substitute = |x: &LinExpr| -> LinExpr {
        let mut out = x.clone();
        for (d, v) in &solved {
            if let Some(c) = out.coefficients.remove(d) {
                out = out.add(&v.scale(&c));
            }
        }
        out
    };
    let low = substitute(&e.add(&konst(-1)));
    let high = substitute(&len_var(*arr).sub(&e));
    let system: Vec<Constraint> = system
        .iter()
        .map(|c| Constraint {
            expr: substitute(&c.expr),
            strict: c.strict,
        })
        .filter(|c| !c.expr.coefficients.is_empty() || !c.is_satisfied_constant())
        .collect();

    // The constraints connected to the goals through shared variables.
    let mut reach: std::collections::BTreeSet<i64> = low.coefficients.keys().copied().collect();
    reach.extend(high.coefficients.keys().copied());
    let mut kept = vec![false; system.len()];
    loop {
        let mut grew = false;
        for (k, c) in system.iter().enumerate() {
            if kept[k] {
                continue;
            }
            if c.expr.coefficients.is_empty()
                || c.expr.coefficients.keys().any(|v| reach.contains(v))
            {
                kept[k] = true;
                reach.extend(c.expr.coefficients.keys().copied());
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let relevant: Vec<Constraint> = system
        .into_iter()
        .zip(kept)
        .filter_map(|(c, k)| k.then_some(c))
        .collect();
    if relevant.len() > KARR_MAX_CONSTRAINTS || reach.len() > KARR_MAX_VARS {
        return;
    }
    if consistent(&relevant) && prove(&relevant, &low) && prove(&relevant, &high) {
        facts.relational_inbounds.insert(key);
    }
}

/// The most constraints the equality proof hands Fourier-Motzkin for one access.
const KARR_MAX_CONSTRAINTS: usize = 48;

/// The most variables the equality proof's system may hold for one access.
const KARR_MAX_VARS: usize = 12;

/// Discharge `1 <= E <= length(arr)` for a single access via the kernel LIA
/// prover, recording the index address on success.
fn try_prove_affine(
    collection: &Expr,
    index: &Expr,
    body: &[Stmt],
    state: &RichAbstractState,
    clobbered: &std::collections::HashSet<Symbol>,
    mutated: &std::collections::HashSet<Symbol>,
    amb: &AffineFacts,
    facts: &mut OracleFacts,
) {
    use super::affine::{ge, konst, le, lin_of, prove, var};
    let Expr::Identifier(arr) = collection else {
        return;
    };
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

    // Entry-guarded function's precondition (`lo>=1`, `hi>=1`): a FUNCTION-WIDE
    // ambient fact, injected into every proof regardless of the per-state widening
    // that drops the header interval in the swap's recording pass. Enforced by the
    // `codegen::entry_guard` `assert!` at runtime, so sound at every access past the
    // base-case guard. This is what discharges `1 <= i,j` for the partition swap.
    for &(s, lb) in &state.entry_guard_lowers {
        system.push(le(&konst(lb), &var(s)));
    }

    // Derived bounds for a conditionally-assigned index variable — a heap sift's
    // `swapIdx` (`Set swapIdx to root | child | child+1`) has no `scalar_def` and is
    // no loop IV, so nothing above bounds it. Add `s >= 0` (co-induction: every value
    // assigned to it is `>= 0`, grounded by the seeded counter lowers) and `s <=
    // <len-bound-sym>` (each assignment proven `<= end` under its path guards), both
    // SOUND, closing its `[0, end] ⊆ [0, len)` accesses. `base_system`
    // (pre-augmentation) is the fixed premise set for the upper proof — non-circular.
    let base_system = system.clone();
    let bound_sym = state.length_def.get(arr).map(|(b, _)| *b);
    for &s in &syms {
        if !mutated.contains(&s) || state.scalar_def.contains_key(&s) {
            continue;
        }
        let assumed = std::collections::HashSet::new();
        if sym_ge_lo(s, 0, body, state, mutated, &assumed, 64) {
            system.push(ge(&var(s), &konst(0)));
        }
        if let Some(b) = bound_sym {
            if s != b && sym_le_sym(s, b, body, &base_system, state) {
                system.push(le(&var(s), &var(b)));
            }
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
            facts
                .positivity_guards
                .entry(key)
                .or_default()
                .extend(mod_divisors);
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
        Expr::BinaryOp {
            op: BinaryOpKind::And,
            left,
            right,
        } => {
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
                Expr::Identifier(v) => IvBound::Var {
                    var: *v,
                    headroom: 0,
                },
                // `n - t1 - t2 - …` with each subtracted term provably ≥ 0.
                _ => match var_minus_bound(bnd_e, state) {
                    Some((v, headroom)) => IvBound::Var { var: v, headroom },
                    None => return,
                },
            };
            out.push(IvGuard {
                iv: *iv,
                bound,
                strict,
                iv_lo,
            });
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
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => {
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
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => match (&**left, &**right) {
            (Expr::Identifier(s), Expr::Literal(Literal::Number(k))) => Some((*s, *k)),
            (Expr::Literal(Literal::Number(k)), Expr::Identifier(s)) => Some((*s, *k)),
            _ => None,
        },
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => match (&**left, &**right) {
            (Expr::Identifier(s), Expr::Literal(Literal::Number(k))) => Some((*s, -*k)),
            _ => None,
        },
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
    let Expr::Identifier(arr) = collection else {
        return;
    };
    if clobbered.contains(arr) {
        return;
    }
    let Some((iv, k)) = affine_of(index) else {
        return;
    };
    let proven = guards
        .iter()
        .any(|g| g.iv == iv && !clobbered.contains(&g.iv) && guard_proves(g, *arr, k, length_def));
    if proven {
        facts
            .relational_inbounds
            .insert(index as *const Expr as usize);
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
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            var_rebound_in(sym, then_block)
                || matches!(else_block, Some(eb) if var_rebound_in(sym, eb))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => var_rebound_in(sym, body),
        _ => false,
    })
}

/// `value` is `iv + c` or `c + iv` with `c` a literal `>= 1`.
fn is_positive_increment(value: &Expr, iv: Symbol) -> bool {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = value
    {
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
    let Some((last, rest)) = body.split_last() else {
        return false;
    };
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
    let Expr::Identifier(arr) = collection else {
        return;
    };
    // The array must be a proven collection, stable (never rebound, never
    // resized — a realloc would stale the pinned pointer), and the index
    // affine in THIS loop's induction variable.
    if !state.coll_vars.contains(arr) || var_rebound_in(*arr, body) || array_resized_in(*arr, body)
    {
        return;
    }
    // Speculate ONLY on a function-PARAMETER collection — its length is
    // fundamentally unknown, so a region-entry runtime check is the right
    // tool. A locally-built array has a statically determinable length and is
    // left to the static path (speculating there would only ever deopt).
    if !state.param_colls.contains(arr) {
        return;
    }
    let Some((aiv, k)) = affine_of(index) else {
        return;
    };
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
        if let Stmt::SetIndex {
            collection, index, ..
        } = s
        {
            consider_hoist_access(collection, index, iv, state, body, facts, per_array);
        }
        match s {
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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
        | Expr::Range {
            start: left,
            end: right,
        } => {
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
        Expr::Slice {
            collection,
            start,
            end,
        } => {
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

/// One region-entry guard per array a hoisted loop reads, from the covered offsets `(kmin, kmax)` of each,
/// ordered by the array's symbol: the guards the compiler emits are a function of the loop, whatever order
/// the accesses were collected in.
fn hoist_descs(
    per_array: impl IntoIterator<Item = (Symbol, (i64, i64))>,
    bound: Symbol,
    iv: Symbol,
    strict: i64,
) -> Vec<HoistDesc> {
    let mut descs: Vec<HoistDesc> = per_array
        .into_iter()
        .filter_map(|(array, (kmin, kmax))| {
            Some(HoistDesc {
                array,
                bound,
                iv,
                add_max: i32::try_from(kmax - strict).ok()?,
                add_min: i32::try_from(kmin).ok()?,
            })
        })
        .collect();
    descs.sort_by_key(|d| d.array);
    descs
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
    let descs = hoist_descs(per_array, bound, g.iv, strict);
    if !descs.is_empty() {
        facts.hoist_descs.entry(loop_key).or_default().extend(descs);
    }
}

/// The length fact a counted build loop establishes for the array it fills.
enum BuildLength {
    /// Variable bound: `length(arr) >= n + off`.
    Symbolic(Symbol, Symbol, i64),
    /// Literal bound: `length(arr)` lies in the interval — exact when the array's
    /// entry length is (the interval check then proves reads directly).
    Concrete(Symbol, Interval),
}

/// Recognize a counted build loop `while c </<= B: <fill arr once per
/// iteration>`, where `B` is a variable (→ a symbolic length lower bound) or an
/// integer literal (→ a concrete length) — the standard allocation-size fact.
/// The loop appends one item per trip to whatever the array held at entry, so a
/// table seeded before its fill loop (`Push 1 to dp`, then `n` zeros) ends with
/// its seed counted in. Strict by construction: the body is flat with exactly
/// one `push to arr`, exactly one `c := c + 1`, `c` starting at a known
/// constant, and nothing else that resizes a collection or branches (a
/// conditional push would make the length too small).
fn infer_build_length(cond: &Expr, body: &[Stmt], entry: &RichAbstractState) -> Vec<BuildLength> {
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
    let Expr::Identifier(c) = c_e else {
        return Vec::new();
    };
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
        // The items the array holds at loop entry stay; the loop appends after
        // them. The symbolic fact is a LOWER bound, so it takes the entry
        // length's lower end, and it holds for a zero-trip loop too: no trips
        // means `n + off <= 0`, so `n + off + seed <= seed <= length`.
        let entry_len = entry.intervals.get_length(&arr);
        let Bound::Finite(seed) = entry_len.lo else {
            continue;
        };
        match (bound_var, b_e) {
            (Some(n), _) => {
                if let Some(total) = off.checked_add(seed) {
                    out.push(BuildLength::Symbolic(arr, n, total));
                }
            }
            (None, Expr::Literal(Literal::Number(bn))) => {
                if let Some(trips) = bn.checked_add(off) {
                    let filled = Interval::exact(trips.max(0)).add(&entry_len);
                    out.push(BuildLength::Concrete(arr, filled));
                }
            }
            _ => {}
        }
    }
    out
}

/// `value` is exactly `c + 1` or `1 + c`.
fn is_increment_by_one(value: &Expr, c: Symbol) -> bool {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = value
    {
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
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
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
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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
        Stmt::Pop { collection, into } => hits(collection) || matches!(into, Some(v) if *v == sym),
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
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
    globals: &CalleeGlobals,
) {
    for stmt in stmts {
        for_each_direct_expr(stmt, &mut |e| {
            record_index_reads(e, guards, length_def, clobbered, facts)
        });
        // A store `Set item E of arr to v` — prove the STORE index too (its
        // index is a direct expression, not an `Index` node a read walk sees).
        if let Stmt::SetIndex {
            collection, index, ..
        } = stmt
        {
            try_record_index(collection, index, guards, length_def, clobbered, facts);
        }
        match stmt {
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                let mut c_then = clobbered.clone();
                record_proven_reads(then_block, guards, length_def, &mut c_then, facts, globals);
                let mut c_else = clobbered.clone();
                if let Some(eb) = else_block {
                    record_proven_reads(eb, guards, length_def, &mut c_else, facts, globals);
                }
                clobbered.extend(c_then);
                clobbered.extend(c_else);
            }
            Stmt::While { body, .. } => {
                let mut inner = clobbered.clone();
                for m in collect_mutations_in(body, globals) {
                    inner.insert(m);
                }
                record_proven_reads(body, guards, length_def, &mut inner.clone(), facts, globals);
                *clobbered = inner;
            }
            Stmt::Repeat { pattern, body, .. } => {
                if let Some(v) = pattern_loop_var(pattern) {
                    clobbered.insert(v);
                }
                for m in collect_mutations_in(body, globals) {
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
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
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
        Stmt::While {
            cond, decreasing, ..
        } => {
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
        | Expr::Range {
            start: left,
            end: right,
        } => {
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
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            record_index_reads(collection, guards, length_def, clobbered, facts);
            record_index_reads(start, guards, length_def, clobbered, facts);
            record_index_reads(end, guards, length_def, clobbered, facts);
        }
        Expr::Copy { expr } => record_index_reads(expr, guards, length_def, clobbered, facts),
        Expr::Give { value } => record_index_reads(value, guards, length_def, clobbered, facts),
        Expr::Length { collection } => {
            record_index_reads(collection, guards, length_def, clobbered, facts)
        }
        Expr::Contains { collection, value } => {
            record_index_reads(collection, guards, length_def, clobbered, facts);
            record_index_reads(value, guards, length_def, clobbered, facts);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for it in items {
                record_index_reads(it, guards, length_def, clobbered, facts);
            }
        }
        Expr::FieldAccess { object, .. } => {
            record_index_reads(object, guards, length_def, clobbered, facts)
        }
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
        Expr::OptionSome { value } => {
            record_index_reads(value, guards, length_def, clobbered, facts)
        }
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
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
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
            if let Expr::WithCapacity {
                value: inner,
                capacity,
            } = value
            {
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
        each_child_block(s, &mut |b| {
            gather_map_caps(b, interner, reassigned, out, poisoned)
        });
    }
}

/// The inclusive loop bound `B` of a counted condition `iv </<= B` whose counter
/// is a bare identifier — the candidate key-domain capacity for a map filled by
/// the loop. `None` for any other condition shape.
fn counted_loop_bound<'a>(cond: &'a Expr<'a>) -> Option<&'a Expr<'a>> {
    let Expr::BinaryOp { op, left, right } = cond else {
        return None;
    };
    if !matches!(op, BinaryOpKind::Lt | BinaryOpKind::LtEq) {
        return None;
    }
    let Expr::Identifier(_) = *left else {
        return None;
    };
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
        if let Stmt::SetIndex {
            collection: Expr::Identifier(m),
            ..
        } = s
        {
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
pub fn lin_to_rust(
    e: &super::affine::LinExpr,
    interner: &crate::intern::Interner,
) -> Option<String> {
    let mut coeffs: Vec<(i64, i64)> = Vec::new();
    for (idx, c) in e.coefficients.iter() {
        let ci = c.to_i64()?;
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
    let k = e.constant.to_i64()?;
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
        if let Stmt::SetIndex {
            collection: Expr::Identifier(m),
            ..
        } = s
        {
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
    let Expr::BinaryOp { op, left, right } = cond else {
        return None;
    };
    let Expr::Identifier(iv) = left else {
        return None;
    };
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
            Stmt::SetIndex {
                collection: Expr::Identifier(m),
                index: Expr::Identifier(ix),
                ..
            } if ix == iv && map_caps.contains_key(m) => {
                if found_map.is_some() {
                    return None;
                }
                found_map = Some(*m);
            }
            // The unit increment `Set iv to iv + 1` (either operand order).
            Stmt::Set {
                target,
                value:
                    Expr::BinaryOp {
                        op: BinaryOpKind::Add,
                        left: l,
                        right: r,
                    },
            } if target == iv => {
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
            Stmt::Let {
                var,
                value: Expr::Literal(Literal::Number(n)),
                ..
            } => {
                inits.insert(*var, *n);
            }
            Stmt::Let { var, .. } => {
                inits.remove(var);
            }
            Stmt::Set {
                target,
                value: Expr::Literal(Literal::Number(n)),
            } => {
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
    let callee_globals = std::rc::Rc::new(CalleeGlobals::of_program(stmts));
    let mut st = RichAbstractState::new();
    st.callee_globals = callee_globals.clone();
    rich_walk_block(stmts, &mut st, &mut facts);
    // Function bodies: parameters carry their declared scalar types.
    for s in stmts {
        if let Stmt::FunctionDef { params, body, .. } = s {
            let mut fst = RichAbstractState::new();
            fst.callee_globals = callee_globals.clone();
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

/// The functions every return of which hands back a sequence as long as the one handed to them at a parameter
/// position: `function -> position`. A candidate (a sequence parameter, a sequence return type, and a body whose
/// statements the walk enters, so no return hides from it) is walked with that parameter's entry length named
/// ([`super::affine::entry_len_index`]); every `Return` it holds must be recorded handing back a sequence whose length
/// equals it. Calls assume the summaries still standing, and a refuted candidate is dropped and the rest walked again
/// until none is refuted. The greatest fixpoint is sound for what a call returns: a call that returns made only
/// calls that returned, so the summaries hold by induction on the depth of the calls.
fn length_preserving_functions(
    stmts: &[Stmt],
    fn_returns: &std::rc::Rc<HashMap<Symbol, TypeTag>>,
    callee_globals: &std::rc::Rc<CalleeGlobals>,
    seq_types: &std::rc::Rc<HashSet<Symbol>>,
) -> HashMap<Symbol, usize> {
    use super::affine::{entry_len_index, len_index, len_var, LinExpr};
    use crate::ast::stmt::TypeExpr;
    let is_seq =
        |ty: &TypeExpr| matches!(ty, TypeExpr::Generic { base, .. } if seq_types.contains(base));
    let mut defs = Vec::new();
    let mut candidates: HashMap<Symbol, Vec<usize>> = HashMap::new();
    for s in stmts {
        let Stmt::FunctionDef {
            name,
            params,
            body,
            return_type: Some(ret),
            ..
        } = s
        else {
            continue;
        };
        let mut sites = Vec::new();
        if !is_seq(ret) || !return_sites(body, &mut sites) || sites.is_empty() {
            continue;
        }
        let positions: Vec<usize> = (0..params.len()).filter(|&i| is_seq(params[i].1)).collect();
        if !positions.is_empty() {
            candidates.insert(*name, positions);
            defs.push((*name, params, *body, sites));
        }
    }
    loop {
        let assumed: std::rc::Rc<HashMap<Symbol, usize>> = std::rc::Rc::new(
            candidates
                .iter()
                .filter_map(|(f, ps)| ps.first().map(|p| (*f, *p)))
                .collect(),
        );
        let mut refuted = false;
        for (name, params, body, sites) in &defs {
            let positions = candidates[name].clone();
            if positions.is_empty() {
                continue;
            }
            let mut fst = RichAbstractState::new();
            fst.fn_returns = fn_returns.clone();
            fst.callee_globals = callee_globals.clone();
            fst.seq_types = seq_types.clone();
            fst.len_summaries = assumed.clone();
            for (i, (p, ty)) in params.iter().enumerate() {
                fst.aliases.taint(*p);
                if is_seq(ty) {
                    fst.seq_vars.insert(*p);
                    fst.coll_vars.insert(*p);
                    fst.karr
                        .assign(len_index(*p), Some(&LinExpr::var(entry_len_index(i))));
                }
            }
            let mut facts = OracleFacts {
                return_probe: Some(Vec::new()),
                ..OracleFacts::default()
            };
            rich_walk_block(body, &mut fst, &mut facts);
            let returns = facts.return_probe.take().unwrap_or_default();
            let kept: Vec<usize> = positions
                .iter()
                .copied()
                .filter(|&p| {
                    let entry = LinExpr::var(entry_len_index(p));
                    sites.iter().all(|&site| {
                        let mut at = returns.iter().filter(|(a, _, _)| *a == site).peekable();
                        at.peek().is_some()
                            && at.all(|(_, named, karr)| {
                                named.is_some_and(|c| karr.entails_zero(&len_var(c).sub(&entry)))
                            })
                    })
                })
                .collect();
            if kept.len() != positions.len() {
                refuted = true;
                candidates.insert(*name, kept);
            }
        }
        if !refuted {
            break;
        }
    }
    candidates
        .into_iter()
        .filter_map(|(f, ps)| ps.first().map(|p| (f, *p)))
        .collect()
}

/// Collect the address of every `Return` in `block` that hands back a value, through the blocks the walk enters.
/// False when the block holds a `Return` without a value, or a statement whose blocks the walk does not enter: a
/// return could hide there, so no summary may rest on the ones found.
fn return_sites(block: &[Stmt], out: &mut Vec<usize>) -> bool {
    block.iter().all(|s| match s {
        Stmt::Return { value: Some(_) } => {
            out.push(s as *const Stmt as usize);
            true
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => return_sites(then_block, out) && else_block.map_or(true, |eb| return_sites(eb, out)),
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => return_sites(body, out),
        Stmt::Inspect { arms, .. } => arms.iter().all(|a| return_sites(a.body, out)),
        Stmt::Let { .. }
        | Stmt::Set { .. }
        | Stmt::SetIndex { .. }
        | Stmt::SetField { .. }
        | Stmt::Push { .. }
        | Stmt::Pop { .. }
        | Stmt::Add { .. }
        | Stmt::Remove { .. }
        | Stmt::Swap { .. }
        | Stmt::Show { .. }
        | Stmt::Call { .. }
        | Stmt::Break
        | Stmt::RuntimeAssert { .. } => true,
        _ => false,
    })
}

/// Shared body of the two public entry points. `aot_entry_guard` gates the
/// entry-guard-precondition facts to the AOT path that enforces them.
fn oracle_analyze_with_opts(
    stmts: &[Stmt],
    interner: &crate::intern::Interner,
    aot_entry_guard: bool,
) -> OracleFacts {
    let mut facts = OracleFacts::default();
    let callee_globals = std::rc::Rc::new(CalleeGlobals::of_program(stmts));
    // Dense-map gate (precondition): record each `Map of Int to Int … with
    // capacity CAP`'s invariant affine capacity BEFORE the loop walks, so
    // `try_prove_dense_key` can relate a key to it while the loop guards are live.
    {
        let reassigned = rebound_names(stmts, &callee_globals);
        let mut poisoned = std::collections::HashSet::new();
        gather_map_caps(
            stmts,
            interner,
            &reassigned,
            &mut facts.map_caps,
            &mut poisoned,
        );
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
        gather_insert_coverage(
            stmts,
            &facts.map_caps,
            &insert_counts,
            &mut inits,
            &mut facts.map_insert_cover,
        );
    }
    // DECLARED primitive return types: a call to one of these functions
    // (native or user-defined) produces a typed value — a static fact the
    // kernel enforces dynamically.
    let mut fn_returns: HashMap<Symbol, TypeTag> = HashMap::new();
    for s in stmts {
        if let Stmt::FunctionDef {
            name,
            return_type: Some(ty),
            ..
        } = s
        {
            if let crate::ast::stmt::TypeExpr::Primitive(t) | crate::ast::stmt::TypeExpr::Named(t) =
                ty
            {
                let tag = match interner.resolve(*t) {
                    "Int" | "Element" | "Event" | "View" => Some(TypeTag::Int),
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
    let seq_types: std::rc::Rc<HashSet<Symbol>> = std::rc::Rc::new(
        ["Seq", "List"]
            .iter()
            .filter_map(|n| interner.lookup(n))
            .collect(),
    );
    let len_summaries = std::rc::Rc::new(length_preserving_functions(
        stmts,
        &fn_returns,
        &callee_globals,
        &seq_types,
    ));
    let mut st = RichAbstractState::new();
    st.fn_returns = fn_returns.clone();
    st.callee_globals = callee_globals.clone();
    st.seq_types = seq_types.clone();
    st.len_summaries = len_summaries.clone();
    rich_walk_block(stmts, &mut st, &mut facts);
    for s in stmts {
        if let Stmt::FunctionDef { params, body, .. } = s {
            let mut fst = RichAbstractState::new();
            fst.fn_returns = fn_returns.clone();
            fst.callee_globals = callee_globals.clone();
            fst.seq_types = seq_types.clone();
            fst.len_summaries = len_summaries.clone();
            fst.aot_entry_guard = aot_entry_guard;
            for (psym, ty) in params.iter() {
                match ty {
                    crate::ast::stmt::TypeExpr::Primitive(t) => {
                        let tag = match interner.resolve(*t) {
                            "Int" | "Element" | "Event" | "View" => {
                                TypeAbstraction::Concrete(TypeTag::Int)
                            }
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
                        if seq_types.contains(base) {
                            fst.seq_vars.insert(*psym);
                        }
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
            let mut partition_guarded = false;
            if aot_entry_guard {
                if let Some(g) =
                    crate::codegen::entry_guard::detect_entry_guard(params, body, interner)
                {
                    fst.intervals.set_var(
                        g.lo,
                        Interval {
                            lo: Bound::Finite(1),
                            hi: Bound::PosInf,
                        },
                    );
                    fst.intervals.set_var(
                        g.hi,
                        Interval {
                            lo: Bound::Finite(1),
                            hi: Bound::PosInf,
                        },
                    );
                    fst.length_def.insert(g.arr, (g.hi, 0));
                    // `lo>=1`/`hi>=1` as function-wide ambient facts carried ON THE
                    // STATE — they flow (via clone) into the loop fixpoint's record
                    // state, where the header interval is widened away and a
                    // thread-local was empty. Enforced by the entry-guard `assert!`.
                    fst.entry_guard_lowers = vec![(g.lo, 1), (g.hi, 1)];
                    partition_guarded = true;
                } else if let Some(g) =
                    crate::codegen::entry_guard::detect_heap_sift_guard(params, body, interner)
                {
                    // Affine-index descent (heap sift): `end < arr.len()` (length_def)
                    // and `start,end >= 0` (enforced by the codegen entry guard). LIFT:
                    // the whole function body is in scope HERE, so verify the loop
                    // `counter` is `>= 0` (grounded by `Let counter be start`, start>=0)
                    // and seed that as a function-wide lower — no per-access whole-body
                    // context needed. The conditionally-assigned indices (`swapIdx`)
                    // then ground off this seeded lower in `try_prove_affine`.
                    fst.intervals.set_var(
                        g.start,
                        Interval {
                            lo: Bound::Finite(0),
                            hi: Bound::PosInf,
                        },
                    );
                    fst.intervals.set_var(
                        g.end,
                        Interval {
                            lo: Bound::Finite(0),
                            hi: Bound::PosInf,
                        },
                    );
                    fst.length_def.insert(g.arr, (g.end, 1));
                    fst.entry_guard_lowers = vec![(g.start, 0), (g.end, 0)];
                    let mutated: std::collections::HashSet<Symbol> =
                        collect_mutations_in(body, &callee_globals)
                            .into_iter()
                            .collect();
                    let assumed = std::collections::HashSet::new();
                    if sym_ge_lo(g.counter, 0, body, &fst, &mutated, &assumed, 64) {
                        fst.entry_guard_lowers.push((g.counter, 0));
                    }
                }
            }
            if partition_guarded {
                // Inlined top-level walk for the entry-guarded partition (quicksort):
                // the affine index proof runs only for LOOP bodies, so a post-loop
                // access — the Lomuto partition's `item i of arr` pivot swap, sitting
                // after the `while j < hi` scan — is never proven and keeps its bounds
                // branch. This mirrors `rich_walk_block`'s transfer exactly
                // (`record_stmt_exprs` + `rich_walk_stmt`), interleaving the affine
                // proof at each top-level statement's PRE-state — so the loop-exit
                // invariant `lo <= i <= hi` that `rich_walk_loop` records into the
                // post-loop state discharges the pivot swap. Scoped to the entry-guarded
                // partition functions (no other program reaches this branch), so it
                // cannot change any non-partition program's oracle output.
                let tl_mutated: std::collections::HashSet<Symbol> =
                    collect_mutations_in(body, &callee_globals)
                        .into_iter()
                        .collect();
                let mut tl_clobbered: std::collections::HashSet<Symbol> =
                    std::collections::HashSet::new();
                collect_resized_arrays(body, &mut tl_clobbered);
                for stmt in body.iter() {
                    record_stmt_exprs(stmt, &fst, &mut facts);
                    prove_toplevel_accesses(
                        stmt,
                        body,
                        &fst,
                        &tl_clobbered,
                        &tl_mutated,
                        &mut facts,
                    );
                    rich_walk_stmt(stmt, &mut fst, &mut facts);
                }
            } else {
                rich_walk_block(body, &mut fst, &mut facts);
            }
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
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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
    st.callee_globals = std::rc::Rc::new(CalleeGlobals::of_program(&stmts));
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
        Stmt::Return { value: Some(e) } => {
            record_expr(e, st, facts);
            if let Some(probe) = &mut facts.return_probe {
                let named = match e {
                    Expr::Identifier(s) if st.seq_vars.contains(s) => Some(*s),
                    _ => None,
                };
                probe.push((stmt as *const Stmt as usize, named, st.karr.clone()));
            }
        }
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
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            record_expr(collection, st, facts);
            record_expr(index, st, facts);
            record_expr(value, st, facts);
            record_karr(index, st, facts);
        }
        Stmt::SetField { object, value, .. } => {
            record_expr(object, st, facts);
            record_expr(value, st, facts);
        }
        Stmt::Inspect { target, .. } => record_expr(target, st, facts),
        Stmt::RuntimeAssert { condition, .. } => record_expr(condition, st, facts),
        Stmt::Sleep { milliseconds } => record_expr(milliseconds, st, facts),
        Stmt::ReadFrom {
            source: ReadSource::File(p),
            ..
        } => record_expr(p, st, facts),
        _ => {}
    }
}

fn rich_walk_stmt(stmt: &Stmt, st: &mut RichAbstractState, facts: &mut OracleFacts) {
    // The statement's OWN effects (its arms below recurse into nested blocks, whose statements get
    // their own turn here): foreign code forgets everything, a call forgets the callee-written globals.
    let own = WriteSet::of_stmt_shallow(stmt).with_callee_globals(&st.callee_globals);
    if own.is_opaque() {
        st.forget_everything();
        return;
    }
    if own.has_calls() {
        st.forget_callee_globals();
    }
    karr_step(stmt, &own, st);
    match stmt {
        Stmt::Let { var, value, .. } => rich_bind(*var, value, st),
        Stmt::Swap { a, b } => {
            for sym in [*a, *b] {
                st.invalidate_var(sym);
                st.aliases.unlink(sym);
            }
        }
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
        Stmt::SetIndex {
            collection, value, ..
        } => observe_written_elem(collection, value, st),
        Stmt::SetField { object, .. } => {
            if let Expr::Identifier(s) = *object {
                for a in st.aliases.may_alias(*s) {
                    st.shapes.insert(a, CollectionShape::Top);
                }
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
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
            // Tasks interleave: an equality one establishes another may break between any two statements.
            st.karr = super::karr::AffineSpace::top();
            rich_walk_block(tasks, st, facts);
            st.karr = super::karr::AffineSpace::top();
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
        // Every other statement is not descended into: everything it (or any block inside it) binds,
        // mutates, or reaches through a call is unknown afterwards; foreign code anywhere in it forgets
        // everything.
        _ => {
            let effects = WriteSet::of_stmt(stmt).with_callee_globals(&st.callee_globals);
            if effects.is_opaque() {
                st.forget_everything();
                return;
            }
            if effects.has_calls() {
                st.forget_callee_globals();
            }
            for sym in effects.symbols() {
                st.invalidate_var(sym);
                st.aliases.unlink(sym);
            }
        }
    }
}

/// A counted loop's exit value. `While x <= B` (or `x < B`) whose one write of `x` is an unconditional top-level
/// `Set x to x + 1`, with `B` affine over names the body leaves alone and sequences it does not resize, ends with
/// `x = B + 1` (`x = B`) when `x` starts at most there: every pass begins with the guard holding and adds at most
/// one, so `x` never passes that value, and the loop ends only once `x` exceeds `B`. The equality joins `after`'s,
/// along with everything that moved in step with `x` (a counted build's length).
fn karr_counted_exit(
    cond: &Expr,
    body: &[Stmt],
    entry: &RichAbstractState,
    after: &mut RichAbstractState,
) {
    use super::affine::{konst, lin_of_with_lengths, var};
    let Expr::BinaryOp { op, left, right } = cond else {
        return;
    };
    let Expr::Identifier(x) = left else { return };
    let past = match op {
        BinaryOpKind::LtEq => 1,
        BinaryOpKind::Lt => 0,
        _ => return,
    };
    let writes = WriteSet::of_block(body).with_callee_globals(&entry.callee_globals);
    if writes.is_opaque()
        || !top_level_unconditional_incs(body, &entry.callee_globals)
            .iter()
            .any(|(v, d)| v == x && *d == 1)
    {
        return;
    }
    let mut resized = HashSet::new();
    collect_resized_arrays(body, &mut resized);
    let fixed =
        |c: Symbol| entry.seq_vars.contains(&c) && !resized.contains(&c) && !writes.may_write(c);
    let Some(bound) = lin_of_with_lengths(right, &fixed) else {
        return;
    };
    let int = |s: Symbol| {
        matches!(
            entry.types.get(&s),
            Some(TypeAbstraction::Concrete(TypeTag::Int))
        )
    };
    let names = bound
        .coefficients
        .keys()
        .filter(|&&k| k >= 0)
        .map(|&k| Symbol::from_index(k as usize));
    let names_ok = names.clone().all(|s| int(s) && !writes.may_write(s));
    if !int(*x) || !names_ok || !entry_proves(entry, &bound.add(&konst(past)).sub(&var(*x))) {
        return;
    }
    after.karr.meet_eq(&var(*x).sub(&bound).sub(&konst(past)));
}

/// Whether `goal >= 0` follows at `st` from its equalities, the intervals and symbolic bounds of the names they and
/// `goal` mention, and every length being non-negative.
fn entry_proves(st: &RichAbstractState, goal: &super::affine::LinExpr) -> bool {
    use super::affine::{consistent, konst, le, prove, var};
    let mut system = Vec::new();
    for eq in st.karr.equalities() {
        system.push(le(&eq, &konst(0)));
        system.push(le(&konst(0), &eq));
    }
    let mut mentioned: std::collections::BTreeSet<i64> =
        goal.coefficients.keys().copied().collect();
    for c in &system {
        mentioned.extend(c.expr.coefficients.keys().copied());
    }
    for &k in &mentioned {
        if k < 0 {
            system.push(le(&konst(0), &super::affine::LinExpr::var(k)));
            continue;
        }
        let s = Symbol::from_index(k as usize);
        let iv = st.intervals.get_var(&s);
        if let Bound::Finite(lo) = iv.lo {
            system.push(le(&konst(lo), &var(s)));
        }
        if let Bound::Finite(hi) = iv.hi {
            system.push(le(&var(s), &konst(hi)));
        }
        if let Some(u) = st.scalar_upper.get(&s) {
            system.push(le(&var(s), u));
        }
        if let Some(l) = st.scalar_lower.get(&s) {
            system.push(le(l, &var(s)));
        }
    }
    consistent(&system) && prove(&system, goal)
}

/// What a statement other than a binding does to the equalities: a push or a pop moves its sequence's length by
/// one, a store into a sequence keeps it, and every other name the statement writes (or lends to a callee's
/// `mutable` parameter) may come back as anything. A binding's transfer is [`karr_after_bind`]; a compound
/// statement's blocks are walked statement by statement.
fn karr_step(stmt: &Stmt, own: &WriteSet, st: &mut RichAbstractState) {
    use super::affine::{konst, len_index, len_var};
    let named = |e: &Expr| match e {
        Expr::Identifier(s) => Some(*s),
        _ => None,
    };
    let moved: Option<(Symbol, i64)> = match stmt {
        Stmt::Let { .. } | Stmt::Set { .. } => return,
        Stmt::Push { collection, .. } => named(collection).map(|c| (c, 1)),
        Stmt::Pop { collection, .. } => named(collection).map(|c| (c, -1)),
        _ => None,
    };
    let kept = match stmt {
        Stmt::SetIndex { collection, .. } => named(collection).filter(|c| st.seq_vars.contains(c)),
        _ => None,
    };
    let unknown: Vec<Symbol> = own
        .symbols()
        .filter(|s| Some(*s) != moved.map(|(c, _)| c) && Some(*s) != kept)
        .collect();
    for name in unknown {
        st.karr_unknown(name);
    }
    if let Some((c, by)) = moved {
        st.karr_resized(c);
        let next = st.seq_vars.contains(&c).then(|| len_var(c).add(&konst(by)));
        st.karr.assign(len_index(c), next.as_ref());
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
        Expr::BinaryOp {
            op: BinaryOpKind::Modulo,
            right,
            ..
        } => match right {
            Expr::Identifier(n) => Some(var(*n).sub(&konst(1))),
            _ => None,
        },
        // A non-negative `y` divided by a literal `k >= 1` is at most `y`.
        Expr::BinaryOp {
            op: BinaryOpKind::Divide,
            left,
            right,
        } if divides_nonnegative(left, right, st) => super::affine::lin_of(left),
        _ => None,
    }
}

/// Whether `left / right` divides a value the intervals prove non-negative by a literal of at least one: its quotient
/// lies in `[0, left]`.
fn divides_nonnegative(left: &Expr, right: &Expr, st: &RichAbstractState) -> bool {
    matches!(right, Expr::Literal(Literal::Number(k)) if *k >= 1)
        && matches!(eval_expr(left, &st.intervals).lo, Bound::Finite(lo) if lo >= 0)
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
        Expr::BinaryOp {
            op: BinaryOpKind::Divide,
            left,
            right,
        } if divides_nonnegative(left, right, st) => Some(konst(0)),
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
    if e.coefficients.len() != 1 || e.constant.to_i64() != Some(-1) {
        return None;
    }
    let (&idx, coeff) = e.coefficients.iter().next()?;
    (coeff.to_i64() == Some(1)).then(|| Symbol::from_index(idx as usize))
}

/// Is `e` a constant `<= 0` (no variables, non-positive)?
fn is_nonpos_const(e: &super::affine::LinExpr) -> bool {
    e.coefficients.is_empty() && !e.constant.is_positive()
}

/// Is the linear expression `e` provably `>= 0` under the current per-variable
/// intervals? Evaluates `const + Σ coeff·interval(var)` and tests the lower
/// bound is a finite non-negative. A non-integer rational or a variable whose
/// interval is unbounded below (for a positive coefficient) fails closed.
fn lin_ge_zero(e: &super::affine::LinExpr, st: &RichAbstractState) -> bool {
    let Some(k) = e.constant.to_i64() else {
        return false;
    };
    let mut acc = Interval::exact(k);
    for (idx, coeff) in &e.coefficients {
        let Some(c) = coeff.to_i64() else {
            return false;
        };
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
    let karr = karr_after_bind(var, value, st);
    let is_seq = sequence_length(value, st).is_some();
    rich_bind_facts(var, value, st);
    st.karr = karr;
    if is_seq {
        st.seq_vars.insert(var);
    } else {
        st.seq_vars.remove(&var);
    }
}

/// The equalities after `var := value`, from the state before it: `var`'s new value when it is an Int affine over
/// the integers and sequence lengths, and its new length when it is a sequence. A name the value lends to a
/// callee's `mutable` parameter may come back as anything.
fn karr_after_bind(var: Symbol, value: &Expr, st: &RichAbstractState) -> super::karr::AffineSpace {
    let int_valued = matches!(
        eval_type(value, &st.types, &st.fn_returns, &st.elem_type),
        TypeAbstraction::Concrete(TypeTag::Int)
    );
    let lent: Vec<Symbol> = WriteSet::of_expr(value).symbols().collect();
    if lent.is_empty() {
        return karr_assigned(var, value, int_valued, st.karr.clone(), st);
    }
    let mut probe = RichAbstractState {
        karr: st.karr.clone(),
        aliases: st.aliases.clone(),
        seq_vars: st.seq_vars.clone(),
        seq_types: st.seq_types.clone(),
        len_summaries: st.len_summaries.clone(),
        ..RichAbstractState::new()
    };
    for name in lent {
        probe.karr_unknown(name);
    }
    let karr = std::mem::replace(&mut probe.karr, super::karr::AffineSpace::top());
    karr_assigned(var, value, int_valued, karr, &probe)
}

/// `karr` after `var := value`, reading lengths through the sequences `lengths` tracks.
fn karr_assigned(
    var: Symbol,
    value: &Expr,
    int_valued: bool,
    mut karr: super::karr::AffineSpace,
    lengths: &RichAbstractState,
) -> super::karr::AffineSpace {
    use super::affine::{len_index, lin_of_with_lengths, var_index};
    let scalar = if int_valued {
        lin_of_with_lengths(value, &|c| lengths.seq_vars.contains(&c))
    } else {
        None
    };
    let length = sequence_length(value, lengths);
    karr.assign(var_index(var), scalar.as_ref());
    karr.assign(len_index(var), length.as_ref());
    karr
}

/// The length of the sequence `value` makes, affine over the state before it, when it is one this walk tracks: a
/// new sequence (0), a list literal, a copy of or another name for a tracked sequence, or a call that returns a
/// sequence as long as a tracked one it is handed.
fn sequence_length(value: &Expr, st: &RichAbstractState) -> Option<super::affine::LinExpr> {
    use super::affine::{konst, len_var};
    let tracked = |e: &Expr| match e {
        Expr::Identifier(s) if st.seq_vars.contains(s) => Some(len_var(*s)),
        _ => None,
    };
    match value {
        Expr::New { type_name, .. } if st.seq_types.contains(type_name) => Some(konst(0)),
        Expr::List(items) => Some(konst(items.len() as i64)),
        Expr::Identifier(_) => tracked(value),
        Expr::Copy { expr } => tracked(expr),
        Expr::WithCapacity { value, .. } => sequence_length(value, st),
        Expr::Call { function, args } => tracked(args.get(*st.len_summaries.get(function)?)?),
        _ => None,
    }
}

fn rich_bind_facts(var: Symbol, value: &Expr, st: &mut RichAbstractState) {
    // `WithCapacity` changes only the allocation strategy. Bind every semantic
    // fact (freshness, shape, length, element type and aliases) from the inner
    // value so an allocation hint cannot hide optimizations from this tier.
    // Capacity metadata itself is collected independently by `gather_map_caps`
    // and the backend-specific lowering.
    if let Expr::WithCapacity { value, .. } = value {
        rich_bind_facts(var, value, st);
        return;
    }
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
    st.scalar_def
        .retain(|_, e| !e.coefficients.contains_key(&vi));
    // Symbolic bounds that name `var` (its own scalar upper, the element upper
    // of a rebound collection, or any bound whose RHS mentions `var`) are now
    // stale — drop them exactly like the affine scalar definitions above.
    st.scalar_upper.remove(&var);
    st.scalar_upper
        .retain(|_, e| !e.coefficients.contains_key(&vi));
    st.elem_upper.remove(&var);
    st.elem_upper
        .retain(|_, e| !e.coefficients.contains_key(&vi));
    // The element TYPE of a rebound collection is stale; its values can't name
    // another variable, so only its own entry needs dropping (re-seeded below
    // for a fresh/aliased collection by the element-write observers).
    st.elem_type.remove(&var);
    st.scalar_lower.remove(&var);
    st.scalar_lower
        .retain(|_, e| !e.coefficients.contains_key(&vi));
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
            st.shapes
                .insert(var, CollectionShape::from_bounds(n, Some(n)));
            st.intervals
                .set_length(var, Interval::exact(items.len() as i64));
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
        Expr::Copy { expr } => {
            // A fresh unaliased value of the OPERAND's kind.
            st.shapes.insert(var, CollectionShape::Top);
            let from_coll =
                matches!(
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
        let new_shape = st
            .shapes
            .get(sym)
            .cloned()
            .unwrap_or(CollectionShape::Top)
            .pushed();
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
        let new_shape = st
            .shapes
            .get(sym)
            .cloned()
            .unwrap_or(CollectionShape::Top)
            .popped();
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
    if let Expr::BinaryOp {
        op: BinaryOpKind::And,
        left,
        right,
    } = cond
    {
        refine_invariant_bound_from_guard(left, pre, mutated, out);
        refine_invariant_bound_from_guard(right, pre, mutated, out);
        return;
    }
    let Expr::BinaryOp { op, left, right } = cond else {
        return;
    };
    let (iv_e, bnd_e, strict) = match op {
        BinaryOpKind::Lt => (left, right, true),
        BinaryOpKind::LtEq => (left, right, false),
        BinaryOpKind::Gt => (right, left, true),
        BinaryOpKind::GtEq => (right, left, false),
        _ => return,
    };
    let (Expr::Identifier(iv), Expr::Identifier(bnd)) = (iv_e, bnd_e) else {
        return;
    };
    if mutated.contains(bnd) {
        return; // the bound must be loop-invariant for the relation to persist
    }
    let Bound::Finite(i0) = pre.intervals.get_var(iv).lo else {
        return;
    };
    let Some(new_lo) = (if strict { i0.checked_add(1) } else { Some(i0) }) else {
        return;
    };
    let cur = out.get_var(bnd);
    out.set_var(
        *bnd,
        Interval {
            lo: Bound::max_bound(&cur.lo, &Bound::Finite(new_lo)),
            hi: cur.hi,
        },
    );
}

/// Widening-to-fixpoint loop analysis (EXODIA 1.1): iterate the body
/// transfer, widening the mutated variables' facts between passes through
/// the threshold ladder; on convergence the recorded body facts hold at the
/// loop invariant. A non-converging loop (cap hit) falls back to the sound
/// invalidate-everything treatment for whatever is still unstable.
/// How deep a loop may sit inside enclosing loops' fixpoints and still get a fixpoint of its own.
/// Each fixpoint walks its body four times (two ascent passes, a narrowing probe, the recording pass),
/// and every walk re-analyzes the loops nested inside — 4^depth on a deep nest, which the recursive
/// inliner produces routinely (eight unfolded levels of `solve`). Beyond this depth a loop is walked
/// once with everything it may write widened: sound, linear, and it only forfeits precision deep in a
/// nest.
const MAX_FIXPOINT_NESTING: usize = 4;

fn rich_walk_loop(
    cond: Option<&Expr>,
    body: &[Stmt],
    st: &mut RichAbstractState,
    loop_var: Option<Symbol>,
    facts: &mut OracleFacts,
    loop_key: Option<usize>,
) {
    let body_writes = WriteSet::of_block(body).with_callee_globals(&st.callee_globals);
    if body_writes.is_opaque() {
        // Foreign code in the body may rewrite anything: no fact survives the loop.
        st.forget_everything();
        return;
    }
    let mut mutated: Vec<Symbol> = body_writes.symbols().collect();
    if let Some(lv) = loop_var {
        if !mutated.contains(&lv) {
            mutated.push(lv);
        }
    }
    // A loop a `Break` of its own can end leaves with its condition still holding and with what its last
    // iteration wrote before breaking: neither the negated condition nor the loop head's facts hold after it.
    let breaks = crate::loop_shape::leaves_by_break(body);
    if facts.loop_depth >= MAX_FIXPOINT_NESTING {
        rich_walk_loop_widened(cond, body, st, &mutated, breaks, facts, loop_key);
        return;
    }
    facts.loop_depth += 1;
    rich_walk_loop_fixpoint(cond, body, st, loop_var, &mutated, breaks, facts, loop_key);
    facts.loop_depth -= 1;
}

/// The bounded-nesting path: everything the loop may write is unknown from its first iteration on,
/// then ONE pass over the body under that widened state records the facts inside it (a fact that
/// holds with every written name at ⊤ holds on every iteration).
fn rich_walk_loop_widened(
    cond: Option<&Expr>,
    body: &[Stmt],
    st: &mut RichAbstractState,
    mutated: &[Symbol],
    breaks: bool,
    facts: &mut OracleFacts,
    loop_key: Option<usize>,
) {
    for &m in mutated {
        st.karr_unknown(m);
        st.invalidate_var(m);
        st.aliases.unlink(m);
        st.intervals.set_length(m, Interval::non_negative());
        st.length_def.remove(&m);
    }
    let mut record_state = st.clone();
    if let Some(c) = cond {
        narrow_state(c, &mut record_state.intervals);
    }
    // Record the SAME facts the converged path records, from the widened state. Every one of them is
    // a statement about all iterations, and a fact that survives having each written name widened to
    // `top` survives every iteration — so the facts are weaker here, never wrong. They must still be
    // recorded: codegen's borrow-hoist reads `loop_aliases` KEYED BY THIS LOOP'S ADDRESS, and a loop
    // that contributes no entry makes codegen plan against a different picture than the one the
    // walk built (an aliased buffer hoisted to a slice at the loop head while a statement inside it
    // still emits the `Rc<RefCell<…>>` form — Rust that does not compile).
    if let Some(key) = loop_key {
        match facts.loop_aliases.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                e.get_mut().union_from(&record_state.aliases);
            }
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(record_state.aliases.clone());
            }
        }
    }
    let header = record_state.clone();
    if let Some(c) = cond {
        record_loop_index_bounds(c, body, &record_state, facts);
        record_affine_index_bounds(c, body, &record_state, st, facts, false);
        if let Some(key) = loop_key {
            record_hoist_speculation(c, body, &record_state, facts, key);
        }
    }
    rich_walk_block(body, &mut record_state, facts);
    if let Some(c) = cond {
        record_expr(c, &record_state, facts);
        record_affine_index_bounds(c, body, &header, st, facts, true);
    }
    if let Some(c) = cond.filter(|_| !breaks) {
        narrow_state_negated(c, &mut st.intervals);
    }
}

fn rich_walk_loop_fixpoint(
    cond: Option<&Expr>,
    body: &[Stmt],
    st: &mut RichAbstractState,
    loop_var: Option<Symbol>,
    mutated: &[Symbol],
    breaks: bool,
    facts: &mut OracleFacts,
    loop_key: Option<usize>,
) {
    let mutated: Vec<Symbol> = mutated.to_vec();
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
    sink.loop_depth = facts.loop_depth;
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
            // A length still growing is not a converged loop: a list the body pushes onto reaches a new
            // length every pass, and the pass that stops looking is not the last one the program runs.
            let cur_len = inside.intervals.get_length(&m);
            let nxt_len = next.intervals.get_length(&m);
            let wid_len = cur_len.widen(&nxt_len);
            if !wid_len.leq(&cur_len) || !cur_len.leq(&wid_len) {
                stable = false;
            }
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
                match (
                    inside.elem_upper.get(&m).cloned(),
                    next.elem_upper.get(&m).cloned(),
                ) {
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
                match (
                    inside.elem_type.get(&m).cloned(),
                    next.elem_type.get(&m).cloned(),
                ) {
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

            let cur_ty = inside
                .types
                .get(&m)
                .cloned()
                .unwrap_or(TypeAbstraction::Top);
            let nxt_ty = next.types.get(&m).cloned().unwrap_or(TypeAbstraction::Top);
            let j = cur_ty.join(&nxt_ty);
            if !(j.leq(&cur_ty) && cur_ty.leq(&j)) {
                stable = false;
            }
            inside.types.insert(m, j);

            let cur_sh = inside
                .shapes
                .get(&m)
                .cloned()
                .unwrap_or(CollectionShape::Top);
            let nxt_sh = next.shapes.get(&m).cloned().unwrap_or(CollectionShape::Top);
            inside.shapes.insert(m, cur_sh.widen(&nxt_sh));

            // Kind survives growth, but a rebinding inside the body
            // (absent from `next`) drops the proof.
            if !next.coll_vars.contains(&m) {
                inside.coll_vars.remove(&m);
            }

            let cur_n = inside
                .nullability
                .get(&m)
                .cloned()
                .unwrap_or(Nullability::Maybe);
            let nxt_n = next
                .nullability
                .get(&m)
                .cloned()
                .unwrap_or(Nullability::Maybe);
            inside.nullability.insert(m, cur_n.join(&nxt_n));
        }
        // Loop-CARRIED aliasing: an edge created at the end of one iteration
        // (`Set prev to curr`) holds at the top of the next, so the alias
        // graph joins by union across the back-edge. Monotone over a finite
        // symbol set — converges. Growth participates in the stability check.
        if inside.aliases.union_from(&next.aliases) {
            stable = false;
        }
        // The equalities at the header hold on entry and after every pass: the hull of both. Each growth raises its
        // dimension, so it settles within a pass per quantity it relates.
        let hull = inside.karr.join(&next.karr);
        if !hull.leq(&inside.karr) {
            stable = false;
        }
        inside.karr = hull;
        let seqs_before = inside.seq_vars.len();
        inside.seq_vars.retain(|s| next.seq_vars.contains(s));
        if inside.seq_vars.len() != seqs_before {
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
        // NARROWING (Cousot descending iteration). The widening above accelerates
        // ascending chains by throwing a still-growing bound to ±∞ — sound, but it
        // discards the finite bound a MODULO re-establishes every iteration
        // (`b = (a + b) % M` re-clamps `b` to `[0, M-1]` no matter the entry). Re-
        // applying the body from the widened post-fixpoint and keeping every
        // finite bound while replacing only ±∞ descends back toward the true
        // invariant, STAYING a sound over-approximation (`inside ⊒ lfp` ⟹
        // `F(inside) ⊒ lfp`). It is the license the raw-i64 lowering needs:
        // recovering `a, b ∈ [0, M-1]` proves `a + b` fits i64, so the overflow
        // guard is dropped and the hot loop matches C. Scoped to scalar var
        // intervals (the arithmetic-guard input); element/length/shape facts,
        // already converged, are untouched. Bounded — each pass fills ≥1 ±∞.
        const NARROW_PASSES: usize = 4;
        for _ in 0..NARROW_PASSES {
            let mut probe = inside.clone();
            let mut narrow_sink = OracleFacts::default();
            narrow_sink.loop_depth = facts.loop_depth;
            rich_walk_block(body, &mut probe, &mut narrow_sink);
            if let Some(c) = cond {
                narrow_state(c, &mut probe.intervals);
            }
            let mut tightened = false;
            for &m in &mutated {
                let cur = inside.intervals.get_var(&m);
                let narrowed = cur.narrow(&probe.intervals.get_var(&m));
                if !(narrowed.leq(&cur) && cur.leq(&narrowed)) {
                    tightened = true;
                }
                inside.intervals.set_var(m, narrowed);
            }
            if !tightened {
                break;
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
        let header = record_state.clone();
        if let Some(c) = cond {
            record_loop_index_bounds(c, body, &record_state, facts);
            // GENERAL multi-variable proof (kernel LIA): catches the affine
            // indices the single-var recognizer above cannot — knapsack's
            // `prev[w-wi+1]` under a path guard, nested-loop windows, scatter
            // indices bounded through element ranges. Idempotent with the
            // single-var path (both feed the same `relational_inbounds` set).
            record_affine_index_bounds(c, body, &record_state, st, facts, false);
            if let Some(key) = loop_key {
                record_hoist_speculation(c, body, &record_state, facts, key);
            }
        }
        rich_walk_block(body, &mut record_state, facts);
        // The condition is evaluated again after every iteration, at what the body leaves: its facts join that
        // state to the entry state `record_stmt_exprs` recorded them at.
        if let Some(c) = cond {
            record_expr(c, &record_state, facts);
        }
        // The equalities each access is evaluated under are recorded now: prove from them.
        if let Some(c) = cond {
            record_affine_index_bounds(c, body, &header, st, facts, true);
        }
        let mut after = rich_join(st, &inside);
        if breaks {
            after.forget_written(&mutated);
        } else if let Some(c) = cond {
            narrow_state_negated(c, &mut after.intervals);
            karr_counted_exit(c, body, st, &mut after);
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
                        after.intervals.set_length(arr, len);
                    }
                }
            }
        }
        // Loop-EXIT induction bound for the entry-guarded partition's pivot swap.
        // For `while j < hi` where `j` is a top-level unconditional `+1` counter and
        // `i` a monotone-increasing var sharing `j`'s loop-entry def, the strict
        // guard with `Δj = 1` lands `j` on exactly `hi` at exit, and the loop
        // invariant `i <= j` (equal start, `Δi <= Δj`) then gives `i <= hi` AFTER the
        // loop; `i >= lo` holds since `i` only rises from its init `lo`. Record
        // `lo <= i <= hi` as symbolic bounds so the following top-level `item i of
        // arr` (proven by the inlined entry-guard walk) discharges. Gated to the
        // entry-guarded functions (`entry_guard_lowers` non-empty) so no other
        // program's loops are affected; SOUND — both bounds hold throughout the
        // post-loop region, where `i` is not written (a later write clears them).
        if !breaks && !st.entry_guard_lowers.is_empty() {
            if let Some(Expr::BinaryOp {
                op: BinaryOpKind::Lt,
                left: Expr::Identifier(j),
                right: Expr::Identifier(hi),
            }) = cond
            {
                if top_level_unconditional_incs(body, &st.callee_globals)
                    .iter()
                    .any(|(v, d)| v == j && *d == 1)
                {
                    for i in monotone_inc_vars(body, 1) {
                        if i != *j {
                            if let (Some(di), Some(dj)) =
                                (st.scalar_def.get(&i), st.scalar_def.get(j))
                            {
                                if di == dj {
                                    let lo_def = di.clone();
                                    after.scalar_lower.entry(i).or_insert(lo_def);
                                    after
                                        .scalar_upper
                                        .entry(i)
                                        .or_insert_with(|| super::affine::var(*hi));
                                }
                            }
                        }
                    }
                }
            }
        }
        *st = after;
    } else {
        // Sound fallback: forget everything the body can write.
        if let Some(c) = cond.filter(|_| !breaks) {
            narrow_state_negated(c, &mut st.intervals);
        }
        for m in mutated {
            st.karr_unknown(m);
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
        callee_globals: a.callee_globals.clone(),
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
        entry_guard_lowers: if a.entry_guard_lowers.is_empty() {
            b.entry_guard_lowers.clone()
        } else {
            a.entry_guard_lowers.clone()
        },
        karr: a.karr.join(&b.karr),
        seq_vars: a.seq_vars.intersection(&b.seq_vars).copied().collect(),
        seq_types: a.seq_types.clone(),
        len_summaries: a.len_summaries.clone(),
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
    let keys: HashSet<Symbol> = a
        .elem_upper
        .keys()
        .chain(b.elem_upper.keys())
        .cloned()
        .collect();
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
    let keys: HashSet<Symbol> = a
        .elem_type
        .keys()
        .chain(b.elem_type.keys())
        .cloned()
        .collect();
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

/// Every name `block` may write, as the shared analysis sees it (nested blocks and every binder kind
/// included). Use [`collect_mutations_in`] when the program's callee-written globals are at hand.
fn collect_mutations(block: &[Stmt]) -> Vec<Symbol> {
    collect_mutations_in(block, &CalleeGlobals::default())
}

/// [`collect_mutations`] with the globals any call in `block` may assign folded in. After foreign
/// code every name the block mentions is suspect, so all of them are returned.
fn collect_mutations_in(block: &[Stmt], globals: &CalleeGlobals) -> Vec<Symbol> {
    let writes = WriteSet::of_block(block).with_callee_globals(globals);
    let mut out: Vec<Symbol> = writes.symbols().collect();
    if writes.is_opaque() {
        for s in block {
            collect_idents_stmt(s, &mut |sym, _| {
                if !out.contains(&sym) {
                    out.push(sym);
                }
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/abstract_interp/domain_tests.rs"]
mod domain_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/abstract_interp/type_domain_tests.rs"]
mod type_domain_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/abstract_interp/shape_domain_tests.rs"]
mod shape_domain_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/abstract_interp/nullability_domain_tests.rs"]
mod nullability_domain_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/abstract_interp/alias_domain_tests.rs"]
mod alias_domain_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/abstract_interp/product_lattice_tests.rs"]
mod product_lattice_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/abstract_interp/rich_walk_tests.rs"]
mod rich_walk_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/abstract_interp/hoist_order_tests.rs"]
mod hoist_order_tests;
