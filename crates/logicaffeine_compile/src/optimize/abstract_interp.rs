use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, MatchArm, Pattern, Stmt};
use crate::intern::Symbol;

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
        let lo = if other.lo.cmp_bound(&self.lo) == std::cmp::Ordering::Less {
            Bound::NegInf
        } else {
            self.lo.clone()
        };
        let hi = if other.hi.cmp_bound(&self.hi) == std::cmp::Ordering::Greater {
            Bound::PosInf
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
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            if let Some(r) = a.checked_mul(b) {
                return Interval::exact(r);
            }
        }
        Interval::top()
    }

    fn div(&self, other: &Interval) -> Interval {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            if b != 0 {
                return Interval::exact(a / b);
            }
        }
        Interval::top()
    }

    fn modulo(&self, other: &Interval) -> Interval {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            if b != 0 {
                return Interval::exact(a % b);
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

fn eval_type(expr: &Expr, types: &HashMap<Symbol, TypeAbstraction>) -> TypeAbstraction {
    match expr {
        Expr::Literal(lit) => TypeAbstraction::Concrete(literal_type(lit)),
        Expr::Identifier(sym) => types.get(sym).cloned().unwrap_or(TypeAbstraction::Top),
        Expr::BinaryOp { op, left, right } => {
            let l = eval_type(left, types);
            let r = eval_type(right, types);
            binop_type(*op, &l, &r)
        }
        Expr::Not { operand } => match eval_type(operand, types) {
            TypeAbstraction::Concrete(TypeTag::Bool) => TypeAbstraction::Concrete(TypeTag::Bool),
            TypeAbstraction::Concrete(TypeTag::Int) => TypeAbstraction::Concrete(TypeTag::Int),
            _ => TypeAbstraction::Top,
        },
        Expr::Length { .. } => TypeAbstraction::Concrete(TypeTag::Int),
        Expr::Contains { .. } => TypeAbstraction::Concrete(TypeTag::Bool),
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
#[derive(Clone, Default)]
struct AliasGraph {
    edges: HashMap<Symbol, HashSet<Symbol>>,
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

    /// Rebinding `a` to a fresh allocation severs its old alias edges.
    fn unlink(&mut self, a: Symbol) {
        if let Some(ns) = self.edges.remove(&a) {
            for n in ns {
                if let Some(s) = self.edges.get_mut(&n) {
                    s.remove(&a);
                }
            }
        }
    }
}

#[derive(Clone)]
struct AbstractState {
    vars: HashMap<Symbol, Interval>,
    lengths: HashMap<Symbol, Interval>,
}

impl AbstractState {
    fn new() -> Self {
        AbstractState {
            vars: HashMap::new(),
            lengths: HashMap::new(),
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
                BinaryOpKind::Modulo => l.modulo(&r),
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
}

impl RichAbstractState {
    fn new() -> Self {
        RichAbstractState {
            intervals: AbstractState::new(),
            types: HashMap::new(),
            shapes: HashMap::new(),
            nullability: HashMap::new(),
            aliases: AliasGraph::new(),
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
    /// or produced by an operation we do not model).
    fn invalidate_var(&mut self, sym: Symbol) {
        self.intervals.set_var(sym, Interval::top());
        self.types.insert(sym, TypeAbstraction::Top);
        self.shapes.insert(sym, CollectionShape::Top);
        self.nullability.insert(sym, Nullability::Maybe);
    }
}

/// Run the Oracle's rich abstract interpretation, returning the (unchanged)
/// statements together with the per-variable abstract facts.
pub(crate) fn rich_abstract_interp_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    _expr_arena: &'a Arena<Expr<'a>>,
    _stmt_arena: &'a Arena<Stmt<'a>>,
) -> (Vec<Stmt<'a>>, RichAbstractState) {
    let mut st = RichAbstractState::new();
    rich_walk_block(&stmts, &mut st);
    (stmts, st)
}

fn rich_walk_block(block: &[Stmt], st: &mut RichAbstractState) {
    for stmt in block {
        rich_walk_stmt(stmt, st);
    }
}

fn rich_walk_stmt(stmt: &Stmt, st: &mut RichAbstractState) {
    match stmt {
        Stmt::Let { var, value, .. } => rich_bind(*var, value, st),
        Stmt::Set { target, value } => rich_bind(*target, value, st),
        Stmt::Push { collection, .. } | Stmt::Add { collection, .. } => rich_grow(collection, st),
        Stmt::Pop { collection, into } => {
            rich_shrink(collection, st);
            if let Some(v) = into {
                st.invalidate_var(*v);
                st.aliases.unlink(*v);
            }
        }
        Stmt::Remove { collection, .. } => rich_shrink(collection, st),
        Stmt::SetIndex { .. } => { /* size unchanged; no per-element tracking */ }
        Stmt::SetField { object, .. } => {
            if let Expr::Identifier(s) = *object {
                for a in st.aliases.may_alias(*s) {
                    st.shapes.insert(a, CollectionShape::Top);
                }
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            rich_walk_if(cond, then_block, *else_block, st);
        }
        Stmt::While { cond, body, .. } => rich_walk_loop(Some(cond), body, st, None),
        Stmt::Repeat { pattern, body, .. } => {
            rich_walk_loop(None, body, st, pattern_loop_var(pattern));
        }
        Stmt::Inspect { target, arms, .. } => rich_walk_inspect(target, arms, st),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => rich_walk_block(tasks, st),
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
fn rich_bind(var: Symbol, value: &Expr, st: &mut RichAbstractState) {
    st.aliases.unlink(var);

    let iv = eval_expr(value, &st.intervals);
    st.intervals.set_var(var, iv);
    st.types.insert(var, eval_type(value, &st.types));
    st.nullability.insert(var, nullability_of_expr(value, st));

    match value {
        Expr::New { .. } => {
            st.shapes.insert(var, CollectionShape::Empty);
            st.intervals.set_length(var, Interval::exact(0));
        }
        Expr::List(items) | Expr::Tuple(items) => {
            let n = items.len() as u64;
            st.shapes.insert(var, CollectionShape::from_bounds(n, Some(n)));
            st.intervals.set_length(var, Interval::exact(items.len() as i64));
        }
        Expr::Identifier(src) => {
            // `v` and `src` are now two names for one allocation.
            st.aliases.link(var, *src);
            let shape = st.shapes.get(src).cloned().unwrap_or(CollectionShape::Top);
            st.shapes.insert(var, shape);
            let len = st.intervals.get_length(src);
            st.intervals.set_length(var, len);
        }
        _ => {
            // Includes `copy of ...`, which is a fresh unaliased value.
            st.shapes.insert(var, CollectionShape::Top);
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
) {
    match eval_condition(cond, &st.intervals) {
        Some(true) => {
            narrow_state(cond, &mut st.intervals);
            rich_walk_block(then_block, st);
        }
        Some(false) => {
            if let Some(eb) = else_block {
                narrow_state_negated(cond, &mut st.intervals);
                rich_walk_block(eb, st);
            }
        }
        None => {
            let mut then_st = st.clone();
            narrow_state(cond, &mut then_st.intervals);
            rich_walk_block(then_block, &mut then_st);

            let mut else_st = st.clone();
            narrow_state_negated(cond, &mut else_st.intervals);
            if let Some(eb) = else_block {
                rich_walk_block(eb, &mut else_st);
            }

            *st = rich_join(&then_st, &else_st);
        }
    }
}

/// Sound conservative loop handling: any variable mutated in the body has its
/// facts forgotten after the loop. (Precise loop-carried analysis is a later
/// precision upgrade; over-approximating to `⊤` is sound.)
fn rich_walk_loop(
    cond: Option<&Expr>,
    body: &[Stmt],
    st: &mut RichAbstractState,
    loop_var: Option<Symbol>,
) {
    let mut mutated = collect_mutations(body);
    if let Some(lv) = loop_var {
        if !mutated.contains(&lv) {
            mutated.push(lv);
        }
    }
    if let Some(c) = cond {
        narrow_state_negated(c, &mut st.intervals);
    }
    for m in mutated {
        st.intervals.set_var(m, Interval::top());
        st.intervals.set_length(m, Interval::non_negative());
        st.types.insert(m, TypeAbstraction::Top);
        st.shapes.insert(m, CollectionShape::Top);
        st.nullability.insert(m, Nullability::Maybe);
    }
}

fn rich_walk_inspect(target: &Expr, arms: &[MatchArm], st: &mut RichAbstractState) {
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
        rich_walk_block(arm.body, &mut arm_st);
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
    }
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
    for (k, ns) in &b.edges {
        let e = out.edges.entry(*k).or_default();
        for n in ns {
            e.insert(*n);
        }
    }
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
    fn interval_widen_accelerates_unstable_bounds_to_infinity() {
        // rising upper bound → +∞, stable lower bound preserved
        let w = iv(0, 5).widen(&iv(0, 10));
        assert_eq!(w.lo, Bound::Finite(0));
        assert!(matches!(w.hi, Bound::PosInf));
        // falling lower bound → -∞, stable upper bound preserved
        let w2 = iv(0, 5).widen(&iv(-3, 5));
        assert!(matches!(w2.lo, Bound::NegInf));
        assert_eq!(w2.hi, Bound::Finite(5));
        // stable interval is a fixpoint (no spurious widening)
        let w3 = iv(0, 5).widen(&iv(0, 5));
        assert_eq!(w3.lo, Bound::Finite(0));
        assert_eq!(w3.hi, Bound::Finite(5));
        // widening from ⊥ yields the new value
        let w4 = Interval::bottom().widen(&iv(2, 4));
        assert_eq!(w4.lo, Bound::Finite(2));
        assert_eq!(w4.hi, Bound::Finite(4));
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
        assert_eq!(eval_type(&Expr::Literal(Literal::Number(5)), &empty_types()),
                   TypeAbstraction::Concrete(TypeTag::Int));
        assert_eq!(eval_type(&Expr::Literal(Literal::Float(1.5)), &empty_types()),
                   TypeAbstraction::Concrete(TypeTag::Float));
        assert_eq!(eval_type(&Expr::Literal(Literal::Boolean(true)), &empty_types()),
                   TypeAbstraction::Concrete(TypeTag::Bool));
        assert_eq!(eval_type(&Expr::Literal(Literal::Nothing), &empty_types()),
                   TypeAbstraction::Concrete(TypeTag::Nothing));
        assert_eq!(eval_type(&Expr::Literal(Literal::Char('a')), &empty_types()),
                   TypeAbstraction::Concrete(TypeTag::Char));
        assert_eq!(eval_type(&Expr::Literal(Literal::Duration(60)), &empty_types()),
                   TypeAbstraction::Concrete(TypeTag::Duration));
    }

    #[test]
    fn arithmetic_and_comparison_result_types() {
        let five = Expr::Literal(Literal::Number(5));
        let three = Expr::Literal(Literal::Number(3));
        // Int + Int : Int
        let add = Expr::BinaryOp { op: BinaryOpKind::Add, left: &five, right: &three };
        assert_eq!(eval_type(&add, &empty_types()), TypeAbstraction::Concrete(TypeTag::Int));
        // Int < Int : Bool
        let lt = Expr::BinaryOp { op: BinaryOpKind::Lt, left: &five, right: &three };
        assert_eq!(eval_type(&lt, &empty_types()), TypeAbstraction::Concrete(TypeTag::Bool));
        // Float + Float : Float
        let f1 = Expr::Literal(Literal::Float(1.0));
        let f2 = Expr::Literal(Literal::Float(2.0));
        let fadd = Expr::BinaryOp { op: BinaryOpKind::Add, left: &f1, right: &f2 };
        assert_eq!(eval_type(&fadd, &empty_types()), TypeAbstraction::Concrete(TypeTag::Float));
        // Int + Float : Top (we do not assume a coercion — conservative)
        let mixed = Expr::BinaryOp { op: BinaryOpKind::Add, left: &five, right: &f1 };
        assert_eq!(eval_type(&mixed, &empty_types()), TypeAbstraction::Top);
        // Text + Int : Text (interpolating concat)
        let txt = Expr::Literal(Literal::Text(Symbol::EMPTY));
        let concat = Expr::BinaryOp { op: BinaryOpKind::Add, left: &txt, right: &three };
        assert_eq!(eval_type(&concat, &empty_types()), TypeAbstraction::Concrete(TypeTag::Text));
    }

    #[test]
    fn let_binds_value_type_in_env() {
        // `Let x be 5.` ⇒ env[x] = Concrete(Int); a later use of x reads Int.
        let x = Symbol::EMPTY;
        let mut env = empty_types();
        let value = Expr::Literal(Literal::Number(5));
        env.insert(x, eval_type(&value, &env));
        assert_eq!(env.get(&x), Some(&TypeAbstraction::Concrete(TypeTag::Int)));
        // and an identifier reference resolves through the env
        assert_eq!(eval_type(&Expr::Identifier(x), &env),
                   TypeAbstraction::Concrete(TypeTag::Int));
        // an unbound identifier is Top (no information)
        let unbound = empty_types();
        assert_eq!(eval_type(&Expr::Identifier(x), &unbound), TypeAbstraction::Top);
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
        // widen each component (interval bound rises → +∞)
        let b = av((1, 9), TypeTag::Int, CollectionShape::KnownSize(9), Nullability::Definite);
        let w = a.widen(&b);
        assert!(matches!(w.interval.hi, Bound::PosInf));
        assert_eq!(w.ty, TypeAbstraction::Concrete(TypeTag::Int));
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
