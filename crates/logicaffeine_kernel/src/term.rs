//! Unified term representation for the Calculus of Constructions.
//!
//! In CoC, there is no distinction between terms and types.
//! Everything is a Term in an infinite hierarchy of universes.

use std::fmt;

/// Primitive literal values.
///
/// These are opaque values that compute via hardware ALU, not recursion.
///
/// `PartialEq` is written by hand rather than derived, because it is THE KERNEL'S EQUALITY:
/// `type_checker::types_equal` and `recheck::def_eq` both compare two `Term::Lit` through this
/// one impl, so it decides literal conversion in BOTH kernels at once and the de Bruijn
/// criterion gives it no protection whatever. See the impl below for what the derive got wrong.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Literal {
    /// 64-bit signed integer
    Int(i64),
    /// 64-bit floating point
    Float(f64),
    /// UTF-8 string
    Text(String),
    /// Duration in nanoseconds (signed for negative offsets like "5 min early")
    Duration(i64),
    /// Calendar date as days since Unix epoch (i32 gives ±5.8 million year range)
    Date(i32),
    /// Instant in time as nanoseconds since Unix epoch (UTC)
    Moment(i64),
    /// Arbitrary-precision integer — the `Int` values that overflow `i64` (K6). This is a
    /// PARALLEL representation, appended so existing certificates' `Int`/`Float`/… encoding
    /// is byte-unchanged. A `BigInt` is CANONICAL: it holds only values `to_i64()` cannot,
    /// so every integer has a unique `Literal` (small → `Int`, huge → `BigInt`) and
    /// definitional equality stays sound. Serialized as a decimal string, stable across
    /// `BigInt`'s internal limb layout. Produced by `int_lit`; never constructed directly
    /// for a value that fits `i64`.
    BigInt(#[cfg_attr(feature = "serde", serde(with = "bigint_dec"))] logicaffeine_base::BigInt),
    /// Arbitrary-precision NATURAL-number literal — a compact, accelerated form of the
    /// unary Peano numeral `Succ^n Zero` (K6). `Nat(n)` is DEFINITIONALLY EQUAL to `Succ`
    /// applied `n` times to `Zero`: the kernel bridges the two in `extract_constructor`
    /// (so a `match`/recursor computes on it, peeling one `Succ` per step) and in `def_eq`
    /// (so `Nat(n)` and `Succ^n Zero` are interchangeable), in BOTH kernels. It stores the
    /// count as one `BigInt` instead of `n` heap nodes. Serialized as a decimal string;
    /// the value is non-negative.
    Nat(#[cfg_attr(feature = "serde", serde(with = "bigint_dec"))] logicaffeine_base::BigInt),
}

/// The canonical `Literal` for an integer: `Int(i64)` when it fits (the fast, common path),
/// otherwise the arbitrary-precision `BigInt`. This is the ONLY sanctioned way to build an
/// integer literal from a `BigInt` result, guaranteeing the one-representation-per-value
/// invariant on which definitional equality of literals rests.
pub fn int_lit(n: logicaffeine_base::BigInt) -> Literal {
    match n.to_i64() {
        Some(x) => Literal::Int(x),
        None => Literal::BigInt(n),
    }
}

/// The `BigInt` value of an integer literal, promoting a machine `Int` — for arithmetic
/// that must run in arbitrary precision. `None` for non-integer literals.
pub fn lit_bigint(lit: &Literal) -> Option<logicaffeine_base::BigInt> {
    match lit {
        Literal::Int(x) => Some(logicaffeine_base::BigInt::from_i64(*x)),
        Literal::BigInt(n) => Some(n.clone()),
        _ => None,
    }
}

/// Serialize a [`logicaffeine_base::BigInt`] as its decimal string — a representation
/// independent of the internal limb layout, so certificates stay portable and stable.
#[cfg(feature = "serde")]
mod bigint_dec {
    use logicaffeine_base::BigInt;

    pub fn serialize<S: serde::Serializer>(v: &BigInt, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BigInt, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(d)?;
        BigInt::parse_decimal(&s).ok_or_else(|| serde::de::Error::custom("invalid BigInt literal"))
    }
}

/// Literal equality — identical to the derive on every arm but `Float`.
///
/// **Why `Float` is not compared with `==`.** IEEE-754 equality is not an equivalence relation:
/// it is not REFLEXIVE at `NaN`, and it identifies `0.0` with `-0.0`, which have distinct
/// encodings. A kernel whose conversion is not reflexive cannot retype `refl Float x : x = x`
/// for a `NaN`, and one that identifies two distinct values proves an equation Lean refuses
/// (`(0.0 : Float) = -0.0` is not closed by `rfl` there). Both are wrong, in opposite
/// directions, and [`impl Eq for Literal`](Literal) is a written claim that neither happens.
///
/// `f64::to_bits` is the independent model the comparison is stated against: total, reflexive
/// at every bit pattern including each `NaN` payload, and injective, so two literals are equal
/// exactly when they are the same IEEE value. It is also what `term_store`'s interner already
/// keys floats on, so id equality and `Term` equality now agree — the `float_free` caveat that
/// module carries was about exactly this disagreement.
impl PartialEq for Literal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Literal::Int(a), Literal::Int(b)) => a == b,
            (Literal::Float(a), Literal::Float(b)) => a.to_bits() == b.to_bits(),
            (Literal::Text(a), Literal::Text(b)) => a == b,
            (Literal::Duration(a), Literal::Duration(b)) => a == b,
            (Literal::Date(a), Literal::Date(b)) => a == b,
            (Literal::Moment(a), Literal::Moment(b)) => a == b,
            (Literal::BigInt(a), Literal::BigInt(b)) => a == b,
            (Literal::Nat(a), Literal::Nat(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Literal {}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Int(n) => write!(f, "{}", n),
            Literal::Float(x) => write!(f, "{}", x),
            Literal::Text(s) => write!(f, "{:?}", s),
            Literal::Duration(nanos) => {
                // Display in most human-readable unit
                let abs = nanos.unsigned_abs();
                let sign = if *nanos < 0 { "-" } else { "" };
                if abs >= 3_600_000_000_000 {
                    write!(f, "{}{}h", sign, abs / 3_600_000_000_000)
                } else if abs >= 60_000_000_000 {
                    write!(f, "{}{}min", sign, abs / 60_000_000_000)
                } else if abs >= 1_000_000_000 {
                    write!(f, "{}{}s", sign, abs / 1_000_000_000)
                } else if abs >= 1_000_000 {
                    write!(f, "{}{}ms", sign, abs / 1_000_000)
                } else if abs >= 1_000 {
                    write!(f, "{}{}μs", sign, abs / 1_000)
                } else {
                    write!(f, "{}{}ns", sign, abs)
                }
            }
            Literal::Date(days) => {
                // Convert days since epoch to ISO-8601 date
                // Unix epoch is 1970-01-01 (day 0)
                // We use a simple algorithm for display purposes
                let days = *days as i64;
                let (year, month, day) = days_to_ymd(days);
                write!(f, "{:04}-{:02}-{:02}", year, month, day)
            }
            Literal::Moment(nanos) => {
                // Convert to ISO-8601 datetime
                let secs = nanos / 1_000_000_000;
                let days = secs / 86400;
                let time_secs = secs % 86400;
                let hours = time_secs / 3600;
                let mins = (time_secs % 3600) / 60;
                let secs_rem = time_secs % 60;
                let (year, month, day) = days_to_ymd(days);
                write!(
                    f,
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                    year, month, day, hours, mins, secs_rem
                )
            }
            Literal::BigInt(n) => write!(f, "{}", n),
            Literal::Nat(n) => write!(f, "{}", n),
        }
    }
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: i64) -> (i64, u8, u8) {
    // Civil date from days since epoch using the algorithm from Howard Hinnant
    // https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 {
        z / 146097
    } else {
        (z - 146096) / 146097
    };
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u8, d as u8)
}

/// Universe levels in the type hierarchy — a level EXPRESSION, so the kernel can be
/// universe-POLYMORPHIC (R3). The concrete hierarchy is `Prop : Type 1 : Type 2 : …`
/// with `Prop ≤ Type i`; on top of it, a level may mention universe VARIABLES, so one
/// definition (`id.{u} : Π(A : Sort u). A → A`) is reusable at every level instead of
/// duplicated per level.
///
/// - `Prop` is the universe of propositions (the impredicative bottom; `Prop ≤` all)
/// - `Type(n)` is the concrete universe at level n
/// - `Var(u)` is a universe variable; it ranges over `Prop` and the `Type` levels (never `SProp`,
///   which is a sort and not a level), so a level that mentions one may be `Prop`
/// - `Succ(ℓ)` is `ℓ + 1`
/// - `Max(ℓ₁, ℓ₂)` is the least upper bound (used in Π-type formation)
///
/// The algebra (`succ`/`max`/`equiv`/`is_subtype_of`) is decided over a canonical
/// normal form, NOT by the derived structural equality — `max(u,u) ≡ u`,
/// `max(succ u, u) ≡ succ u`, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Universe {
    /// SProp — the DEFINITIONALLY-proof-irrelevant sort (S). The bottom of the hierarchy
    /// (`SProp ≤ Prop ≤ Type n`): any two terms of a type in `SProp` are definitionally
    /// equal, and it is impredicative (`Π` into `SProp` is `SProp`). It collapses out of
    /// `max`/`imax`/`succ` so the `Prop=0` level encoding is never disturbed.
    SProp,
    /// Prop - the universe of propositions
    Prop,
    /// Type n - the universe of types at level n
    Type(u32),
    /// A universe variable (universe polymorphism).
    Var(String),
    /// The successor level `ℓ + 1`.
    Succ(Box<Universe>),
    /// The least upper bound of two levels.
    Max(Box<Universe>, Box<Universe>),
    /// The IMPREDICATIVE maximum, `imax(a, b)` = `b` if `b` is `Prop`, else
    /// `max(a, b)`. It is the sort of `Π(x:A). B` where `A : Sort a`, `B : Sort b`:
    /// a Π into a proposition is a proposition (Prop is impredicative), no matter
    /// the domain. When `b` is a variable this stays symbolic — the level may be
    /// `Prop` or not depending on the instantiation.
    IMax(Box<Universe>, Box<Universe>),
}

impl Universe {
    /// Get the successor universe: `Type n → Type (n+1)`, `Prop → Type 0`, and a
    /// symbolic `Succ(ℓ)` for a level that mentions variables.
    ///
    /// `Prop.succ()` is `Type 0`, NOT `Type 1`. This function and `level_leq` must agree, and
    /// `level_leq` decides everything over `level_order`'s ℕ-model, in which `Prop ↦ 0` and
    /// `Type n ↦ n+1`. In that model `succ(Prop)` is level 1, which is spelled `Type 0`. Returning
    /// `Type 1` made the successor operation and the level ORDERING disagree by one about `Prop`
    /// alone — the algebra was not self-consistent, and it also put us one level off Lean, which
    /// says `Prop : Type 0`.
    ///
    /// `SProp` moves with it for the same reason: `to_lnat` puts it at 0 as well, so its successor
    /// is `Type 0` by the identical argument. The discriminator is narrow, which is why this
    /// survived: `Prop → Type` agrees either way because the codomain's level already dominates;
    /// only `Prop → Nat` (`Type 0` in Lean, `Type 1` here) told them apart.
    pub fn succ(&self) -> Universe {
        match self {
            Universe::SProp | Universe::Prop => Universe::Type(0),
            // AT THE CEILING of the machine representation the successor stays SYMBOLIC. Both
            // machine answers to `Type(u32::MAX) + 1` are catastrophic and neither is a
            // judgement: wrapping gives `Type u32::MAX : Type 0`, and `Type 0 ≤ Type u32::MAX`
            // by cumulativity, so a universe contains itself — Girard's paradox, from which a
            // closed term of type `False` follows; and overflowing makes the kernel's answer to
            // a malformed input "the process is gone", which cannot be part of a trusted base.
            // `to_lnat` decides the ordering in `u64`, where `Succ(Type u32::MAX)` is an
            // ordinary level strictly above `Type u32::MAX`, so the symbolic answer is total,
            // sound and fully comparable — and it costs the ordinary levels nothing.
            Universe::Type(n) => match n.checked_add(1) {
                Some(next) => Universe::Type(next),
                None => Universe::Succ(Box::new(self.clone())),
            },
            other => Universe::Succ(Box::new(other.clone())),
        }
    }

    /// Get the maximum of two universes (for Pi type formation). Concrete operands
    /// collapse immediately; otherwise a symbolic `Max(…)` is formed (its algebra is
    /// resolved by `normalize`).
    pub fn max(&self, other: &Universe) -> Universe {
        match (self, other) {
            // SProp and Prop are absorbed (they are `≤` everything), SProp first (it is `≤ Prop`).
            (Universe::SProp, u) | (u, Universe::SProp) => u.clone(),
            (Universe::Prop, u) | (u, Universe::Prop) => u.clone(),
            (Universe::Type(a), Universe::Type(b)) => Universe::Type((*a).max(*b)),
            _ => Universe::Max(Box::new(self.clone()), Box::new(other.clone())),
        }
    }

    /// The impredicative maximum `imax(a, b)` — the sort of a `Π` whose codomain
    /// lives in `b`. Collapses when `b`'s Prop-ness is known: `imax(a, Prop) = Prop`,
    /// `imax(a, Type n) = max(a, Type n)`; `imax(a, a) = a`. Otherwise (a variable or
    /// other symbolic `b`) it stays a symbolic `IMax`, whose algebra `equiv`/
    /// `is_subtype_of` decide by case-splitting on whether `b` is `Prop`.
    pub fn imax(&self, other: &Universe) -> Universe {
        match other {
            // A Π into an S-proposition is an S-proposition (impredicative SProp).
            Universe::SProp => Universe::SProp,
            // A Π into a proposition is a proposition.
            Universe::Prop => Universe::Prop,
            // A concrete non-Prop codomain: the Π is predicative.
            Universe::Type(_) | Universe::Succ(_) => self.max(other),
            _ => {
                // Construction preserves symbolic levels. Semantic equivalence is
                // decided when checking conversion, not each time a Pi is formed.
                if self == other {
                    // imax(a, a) = a (a = 0 ⇒ 0; a ≥ 1 ⇒ max(a,a) = a).
                    self.clone()
                } else {
                    Universe::IMax(Box::new(self.clone()), Box::new(other.clone()))
                }
            }
        }
    }

    /// Cumulative subtyping `self ≤ other`, decided over the level normal form. Sound
    /// for variables: `u ≤ u`, `Type 0 ≤ u`, `u ≤ succ u` hold, but `Type 1 ≤ u` and
    /// `u ≤ v` do NOT (they fail for some instantiation).
    pub fn is_subtype_of(&self, other: &Universe) -> bool {
        level_leq(self, other)
    }

    /// Whether every instantiation of this level's variables puts it at `Type 0` or above: a
    /// family in `Sort ℓ` is then never a proposition. `Sort u` may be `Prop` (at `u := 0`);
    /// `Sort (u+1)` and `Sort (max 1 u)` never are.
    pub fn is_never_prop(&self) -> bool {
        level_leq(&Universe::Type(0), self)
    }

    /// The least universe LEVEL at or above this one: itself, with `SProp` raised to `Prop`. `SProp`
    /// is a sort and never a level, and `SProp ≤ Prop`, so a level variable solved from a type in
    /// `SProp` takes `Prop`, and cumulativity carries the type into `Sort Prop`.
    pub fn least_level(&self) -> Universe {
        match self {
            Universe::SProp => Universe::Prop,
            Universe::Prop | Universe::Type(_) | Universe::Var(_) => self.clone(),
            Universe::Succ(l) => l.least_level().succ(),
            Universe::Max(a, b) => a.least_level().max(&b.least_level()),
            Universe::IMax(a, b) => a.least_level().imax(&b.least_level()),
        }
    }

    /// Whether `SProp` occurs anywhere in this level expression.
    pub fn mentions_sprop(&self) -> bool {
        match self {
            Universe::SProp => true,
            Universe::Prop | Universe::Type(_) | Universe::Var(_) => false,
            Universe::Succ(l) => l.mentions_sprop(),
            Universe::Max(a, b) | Universe::IMax(a, b) => a.mentions_sprop() || b.mentions_sprop(),
        }
    }

    /// Definitional equality of two level expressions, accounting for the algebra
    /// (`max`, `imax`, `succ`, variables) — NOT the derived structural equality.
    /// Decided as mutual subtyping over all variable instantiations.
    pub fn equiv(&self, other: &Universe) -> bool {
        level_leq(self, other) && level_leq(other, self)
    }

    /// The same rigid-level equivalence as `equiv`, charged to a shared construction budget.
    pub(crate) fn equiv_bounded(
        &self,
        other: &Universe,
        budget: &mut crate::recursor::bounded::RecursorBudget,
    ) -> crate::recursor::bounded::GenerationResult<bool> {
        level_order::equivalent(self, other, budget)
    }

    /// Substitute universe variables (replacing each `Var(v)` by `subst[v]`) throughout
    /// this level expression — the heart of instantiating a universe-polymorphic global.
    pub fn substitute(&self, subst: &std::collections::HashMap<String, Universe>) -> Universe {
        match self {
            Universe::SProp | Universe::Prop | Universe::Type(_) => self.clone(),
            Universe::Var(v) => subst.get(v).cloned().unwrap_or_else(|| self.clone()),
            Universe::Succ(l) => l.substitute(subst).succ(),
            Universe::Max(a, b) => a.substitute(subst).max(&b.substitute(subst)),
            Universe::IMax(a, b) => a.substitute(subst).imax(&b.substitute(subst)),
        }
    }
}

#[path = "term_level_order.rs"]
mod level_order;

#[cfg(test)]
thread_local! { static LEVEL_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn level_leq(a: &Universe, b: &Universe) -> bool {
    #[cfg(test)]
    LEVEL_COMPARISONS.with(|n| n.set(n.get() + 1));
    level_order::leq(a, b)
}

/// Unified term representation.
///
/// Every expression in CoC is a Term:
/// - `Sort(u)` - universes (Type 0, Type 1, Prop)
/// - `Var(x)` - variables
/// - `Pi` - dependent function types: Π(x:A). B
/// - `Lambda` - functions: λ(x:A). t
/// - `App` - application: f x
///
/// `==` is the kernel's identity: it compares kernel meaning and does not see
/// [`Term::Annotated`]. [`Term::identical`] is the comparison that does.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Term {
    /// Universe: Type n or Prop
    Sort(Universe),

    /// Local variable reference (bound by λ or Π)
    Var(String),

    /// Global definition (inductive type or constructor)
    Global(String),

    /// A universe-polymorphic global referenced at explicit levels — `name.{ℓ₀, ℓ₁, …}`.
    /// `Global` is the monomorphic case (no level arguments); `Const` instantiates a
    /// stored universe-polymorphic definition's universe parameters with `levels`.
    Const { name: String, levels: Vec<Universe> },

    /// Dependent function type: Π(x:A). B
    ///
    /// When B doesn't mention x, this is just A → B.
    /// When B mentions x, this is a dependent type.
    Pi {
        param: String,
        param_type: Box<Term>,
        body_type: Box<Term>,
    },

    /// Lambda abstraction: λ(x:A). t
    Lambda {
        param: String,
        param_type: Box<Term>,
        body: Box<Term>,
    },

    /// Application: f x
    App(Box<Term>, Box<Term>),

    /// Pattern matching on inductive types.
    ///
    /// `match discriminant return motive with cases`
    ///
    /// - discriminant: the term being matched (must have inductive type)
    /// - motive: λx:I. T — describes the return type
    /// - cases: one case per constructor, in definition order
    Match {
        discriminant: Box<Term>,
        motive: Box<Term>,
        cases: Vec<Term>,
    },

    /// Fixpoint (recursive function).
    ///
    /// `fix name. body` binds `name` to itself within `body`.
    /// Used for recursive definitions like addition.
    Fix {
        /// Name for self-reference within the body
        name: String,
        /// The body of the fixpoint (typically a lambda)
        body: Box<Term>,
    },

    /// MUTUAL fixpoint — a block of mutually-recursive functions (K3).
    ///
    /// `mutualfix { f₀ := b₀, …, fₙ := bₙ }.index` binds ALL of `f₀ … fₙ` within EVERY
    /// body `bᵢ` (that is the mutual part), and the whole term reduces to the
    /// `index`-th definition. It is the body of a mutual inductive block's recursor:
    /// `Even.rec`'s fixpoint calls `Odd.rec`'s on the smaller sub-proof and vice
    /// versa. Each body's type is inferred structurally from its λ-telescope (like the
    /// single `Fix`); termination is the MUTUAL Giménez guard — a call to ANY member
    /// must pass a structurally-smaller argument at that member's recursive position.
    MutualFix {
        /// The mutually-recursive definitions, `(name, body)`, in block order. Every
        /// name is in scope in every body.
        defs: Vec<(String, Term)>,
        /// Which definition this occurrence denotes (and reduces to).
        index: usize,
    },

    /// Local definition: `let name : ty := value in body`.
    ///
    /// `name` is bound to `value` (of type `ty`) transparently within `body` —
    /// so the body is type-checked and reduced with `name ≡ value` (ZETA), not
    /// as an opaque hypothesis. The surface `let`; the sharing seam for the
    /// elaborator.
    Let {
        name: String,
        ty: Box<Term>,
        value: Box<Term>,
        body: Box<Term>,
    },

    /// Primitive literal value.
    ///
    /// Hardware-native values like i64, f64, String.
    /// These compute via CPU ALU, not recursion.
    Lit(Literal),

    /// Hole (implicit argument).
    ///
    /// Represents an argument that should be inferred by the type checker.
    /// Used in Literate syntax like `X equals Y` where the type of X/Y is implicit.
    Hole,

    /// `term`, carrying an [`Annotation`](crate::Annotation): a binder's kind, or where `term` came
    /// from. TRANSPARENT — no judgment of the kernel reads it, `==` does not see it, and a
    /// certificate cannot carry it (it has no serialized form). See `annotation.rs`.
    #[cfg_attr(feature = "serde", serde(skip))]
    Annotated {
        annotation: crate::annotation::Annotation,
        term: Box<Term>,
    },
}

/// Equality of KERNEL MEANING: annotations are peeled at every node, so a term equals its
/// annotated copy. Every other variant compares exactly as the derive did, field by field in
/// declaration order.
impl PartialEq for Term {
    fn eq(&self, other: &Term) -> bool {
        match (self.peel_annotations(), other.peel_annotations()) {
            (Term::Sort(a), Term::Sort(b)) => a == b,
            (Term::Var(a), Term::Var(b)) => a == b,
            (Term::Global(a), Term::Global(b)) => a == b,
            (Term::Const { name: m, levels: a }, Term::Const { name: n, levels: b }) => {
                m == n && a == b
            }
            (
                Term::Pi {
                    param: p,
                    param_type: s,
                    body_type: t,
                },
                Term::Pi {
                    param: q,
                    param_type: u,
                    body_type: v,
                },
            ) => p == q && s == u && t == v,
            (
                Term::Lambda {
                    param: p,
                    param_type: s,
                    body: t,
                },
                Term::Lambda {
                    param: q,
                    param_type: u,
                    body: v,
                },
            ) => p == q && s == u && t == v,
            (Term::App(f, a), Term::App(g, b)) => f == g && a == b,
            (
                Term::Match {
                    discriminant: d,
                    motive: m,
                    cases: cs,
                },
                Term::Match {
                    discriminant: e,
                    motive: n,
                    cases: ds,
                },
            ) => d == e && m == n && cs == ds,
            (Term::Fix { name: m, body: s }, Term::Fix { name: n, body: t }) => m == n && s == t,
            (Term::MutualFix { defs: a, index: i }, Term::MutualFix { defs: b, index: j }) => {
                a == b && i == j
            }
            (
                Term::Let {
                    name: m,
                    ty: s1,
                    value: s2,
                    body: s3,
                },
                Term::Let {
                    name: n,
                    ty: t1,
                    value: t2,
                    body: t3,
                },
            ) => m == n && s1 == t1 && s2 == t2 && s3 == t3,
            (Term::Lit(a), Term::Lit(b)) => a == b,
            (Term::Hole, Term::Hole) => true,
            _ => false,
        }
    }
}

impl Eq for Term {}

enum SyntaxDisplayPart<'a> {
    Term(&'a Term),
    Universe(&'a Universe),
    Text(&'a str),
    Index(usize),
    Levels(&'a [Universe]),
    Cases(&'a [Term]),
    Definitions(&'a [(String, Term)]),
}

// Formatting is also used on unfinished reductions in diagnostics. Such terms
// may exceed the kernel's input depth, so both term and universe walks use a
// borrowed work stack. Sequence continuations keep memory proportional to depth.
fn display_syntax(part: SyntaxDisplayPart<'_>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    use SyntaxDisplayPart as Part;
    let mut pending = vec![part];
    while let Some(part) = pending.pop() {
        match part {
            Part::Text(text) => f.write_str(text)?,
            Part::Index(index) => write!(f, "{}", index)?,
            Part::Levels(levels) => {
                if let Some((first, rest)) = levels.split_first() {
                    if !rest.is_empty() {
                        pending.extend([Part::Levels(rest), Part::Text(", ")]);
                    }
                    pending.push(Part::Universe(first));
                }
            }
            Part::Cases(cases) => {
                if let Some((first, rest)) = cases.split_first() {
                    if !rest.is_empty() {
                        pending.extend([Part::Cases(rest), Part::Text(" | ")]);
                    }
                    pending.push(Part::Term(first));
                }
            }
            Part::Definitions(definitions) => {
                if let Some(((name, body), rest)) = definitions.split_first() {
                    write!(f, "{} := ", name)?;
                    if !rest.is_empty() {
                        pending.extend([Part::Definitions(rest), Part::Text(", ")]);
                    }
                    pending.push(Part::Term(body));
                }
            }
            Part::Universe(universe) => match universe {
                Universe::SProp => f.write_str("SProp")?,
                Universe::Prop => f.write_str("Prop")?,
                Universe::Type(n) => write!(f, "Type{}", n)?,
                Universe::Var(name) => f.write_str(name)?,
                Universe::Succ(inner) => {
                    f.write_str("(")?;
                    pending.extend([Part::Text("+1)"), Part::Universe(inner)]);
                }
                Universe::Max(left, right) | Universe::IMax(left, right) => {
                    f.write_str(if matches!(universe, Universe::Max(..)) {
                        "max("
                    } else {
                        "imax("
                    })?;
                    pending.extend([
                        Part::Text(")"),
                        Part::Universe(right),
                        Part::Text(", "),
                        Part::Universe(left),
                    ]);
                }
            },
            Part::Term(term) => match term {
                Term::Sort(universe) => pending.push(Part::Universe(universe)),
                Term::Var(name) | Term::Global(name) => f.write_str(name)?,
                Term::Const { name, levels } => {
                    write!(f, "{}.{{", name)?;
                    pending.extend([Part::Text("}"), Part::Levels(levels)]);
                }
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                } => {
                    if param == "_" {
                        pending.extend([
                            Part::Term(body_type),
                            Part::Text(" -> "),
                            Part::Term(param_type),
                        ]);
                    } else {
                        write!(f, "Π({}:", param)?;
                        pending.extend([
                            Part::Term(body_type),
                            Part::Text("). "),
                            Part::Term(param_type),
                        ]);
                    }
                }
                Term::Lambda {
                    param,
                    param_type,
                    body,
                } => {
                    write!(f, "λ({}:", param)?;
                    pending.extend([Part::Term(body), Part::Text("). "), Part::Term(param_type)]);
                }
                Term::App(func, arg) => {
                    f.write_str("(")?;
                    let arrow = matches!(arg.as_ref(), Term::Pi { param, .. } if param == "_");
                    pending.extend([
                        Part::Text(if arrow { "))" } else { ")" }),
                        Part::Term(arg),
                        Part::Text(if arrow { " (" } else { " " }),
                        Part::Term(func),
                    ]);
                }
                Term::Match {
                    discriminant,
                    motive,
                    cases,
                } => {
                    f.write_str("match ")?;
                    pending.extend([
                        Part::Cases(cases),
                        Part::Text(" with "),
                        Part::Term(motive),
                        Part::Text(" return "),
                        Part::Term(discriminant),
                    ]);
                }
                Term::Fix { name, body } => {
                    write!(f, "fix {}. ", name)?;
                    pending.push(Part::Term(body));
                }
                Term::MutualFix { defs, index } => {
                    f.write_str("mutualfix { ")?;
                    pending.extend([
                        Part::Index(*index),
                        Part::Text(" }."),
                        Part::Definitions(defs),
                    ]);
                }
                Term::Let {
                    name,
                    ty,
                    value,
                    body,
                } => {
                    write!(f, "let {}:", name)?;
                    pending.extend([
                        Part::Term(body),
                        Part::Text(" in "),
                        Part::Term(value),
                        Part::Text(" := "),
                        Part::Term(ty),
                    ]);
                }
                Term::Lit(literal) => write!(f, "{}", literal)?,
                Term::Hole => f.write_str("_")?,
                Term::Annotated { annotation, term } => {
                    write!(f, "@[{}] ", annotation)?;
                    pending.push(Part::Term(term));
                }
            },
        }
    }
    Ok(())
}

impl fmt::Display for Universe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display_syntax(SyntaxDisplayPart::Universe(self), f)
    }
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display_syntax(SyntaxDisplayPart::Term(self), f)
    }
}

/// Instantiate universe variables throughout a term: substitute every `Sort`'s level by
/// `subst`. This specializes a universe-POLYMORPHIC term (`λA:Sort u. …`) to a concrete
/// level (`u := Type 0`), yielding an ordinary term the kernel checks as-is — so one
/// definition is reused at every level instead of duplicated.
pub fn instantiate_universes(
    term: &Term,
    subst: &std::collections::HashMap<String, Universe>,
) -> Term {
    match term {
        Term::Sort(u) => Term::Sort(u.substitute(subst)),
        Term::Const { name, levels } => Term::Const {
            name: name.clone(),
            levels: levels.iter().map(|l| l.substitute(subst)).collect(),
        },
        Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => term.clone(),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(instantiate_universes(param_type, subst)),
            body_type: Box::new(instantiate_universes(body_type, subst)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(instantiate_universes(param_type, subst)),
            body: Box::new(instantiate_universes(body, subst)),
        },
        Term::App(f, a) => Term::App(
            Box::new(instantiate_universes(f, subst)),
            Box::new(instantiate_universes(a, subst)),
        ),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(instantiate_universes(discriminant, subst)),
            motive: Box::new(instantiate_universes(motive, subst)),
            cases: cases
                .iter()
                .map(|c| instantiate_universes(c, subst))
                .collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(instantiate_universes(body, subst)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), instantiate_universes(b, subst)))
                .collect(),
            index: *index,
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(instantiate_universes(ty, subst)),
            value: Box::new(instantiate_universes(value, subst)),
            body: Box::new(instantiate_universes(body, subst)),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(instantiate_universes(term, subst)),
        },
    }
}

// =================================================================================================
// Universe-polymorphic references — the specification BOTH kernels implement
// =================================================================================================
//
// `Term::Const { name, levels }` is the only instantiating form, and three places have to agree
// about what it MEANS: the main kernel's typing rule, the de Bruijn re-checker's, and the
// reducer's δ step. When they are written three times they disagree three ways — one of them
// silently, by handing an ill-formed reference back as a settled normal form. So the decision
// lives HERE, once, as pure syntax over `(declared params, stored type, level arguments)`, and
// each kernel calls it with its own lookup. Sharing the SPECIFICATION is what makes two
// independent implementations meaningful; sharing the arithmetic is what makes a bug invisible
// in both.
//
// The rules, in the order they are decided:
//
// 1. **The level telescope.** An explicitly universe-polymorphic definition carries its own
//    parameter list. Every other declaration — inductive, constructor, definition, axiom —
//    AUTO-BINDS the free level variables of its stored type, in FIRST-OCCURRENCE order. That is
//    Lean's auto-bound-implicit rule, and deriving it rather than storing it is why one
//    registration path cannot disagree with another about what a name's telescope is.
// 2. **Closure.** `free_level_vars(ty) ⊆ params`. A declaration carrying a level variable no
//    parameter binds is refused at every reference — never instantiated with a partial
//    substitution, which silently retains the free level and leaves the declaration usable at
//    exactly one instantiation.
// 3. **Arity.** `|levels| = |params|`, zipped POSITIONALLY. A level list on a monomorphic name
//    is a statement about a declaration that does not exist, so it is refused too.
// 4. **Bare references.** A bare `Term::Global` naming a declaration with a level telescope is an
//    instance of it. At an application's head its levels are solved from the arguments
//    (`type_checker::implicit_levels_for_head`); anywhere else it stands at the declaration's own
//    parameters, as its type is read there. A stored term writes the second kind out
//    ([`bare_references_at_own_levels`]), so rule 1 sees those levels and binds them, and an
//    instantiation of the declaration around the reference moves them with everything else. Left
//    implicit, `P2 : Sort u := B2` over `B2 : Sort u` would be checked where its own `u` and
//    `B2`'s coincide and used where they do not: `P2.{0}` a proposition holding `B2`'s two
//    constructors.

/// Why a `Term::Const` reference could not be resolved. Every kernel and the reducer report the
/// same three failures, so a reference refused by typing is never a normal form to the reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstError {
    /// The level list does not match the declaration's telescope.
    Arity {
        name: String,
        expected: usize,
        got: usize,
    },
    /// The declaration's stored type mentions a level variable its parameter list does not bind.
    /// Instantiating it would leave that level free, so the declaration is refused outright.
    LevelNotClosed { name: String, level: String },
    /// A level argument is or contains `SProp`, which is a sort and never a level.
    SPropLevel { name: String },
}

impl fmt::Display for ConstError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstError::Arity { name, expected, got } => write!(
                f,
                "universe-polymorphic '{name}' expects {expected} level argument(s), got {got}",
            ),
            ConstError::LevelNotClosed { name, level } => write!(
                f,
                "declaration '{name}' carries the level variable '{level}', which none of its \
                 universe parameters binds — it could never be instantiated, so no reference to \
                 it is well-formed",
            ),
            ConstError::SPropLevel { name } => write!(
                f,
                "'{name}' is instantiated at SProp, which is a sort and not a universe level: a level \
                 ranges over Prop and the Type levels, so a polymorphic family checked as possibly \
                 relevant can never become definitionally irrelevant",
            ),
        }
    }
}

/// The free universe VARIABLES of a level expression, appended in first-occurrence order
/// (duplicates suppressed).
fn level_vars_of_universe(u: &Universe, out: &mut Vec<String>) {
    match u {
        Universe::SProp | Universe::Prop | Universe::Type(_) => {}
        Universe::Var(v) => {
            if !out.iter().any(|x| x == v) {
                out.push(v.clone());
            }
        }
        Universe::Succ(l) => level_vars_of_universe(l, out),
        Universe::Max(a, b) | Universe::IMax(a, b) => {
            level_vars_of_universe(a, out);
            level_vars_of_universe(b, out);
        }
    }
}

/// The free universe variables of a TERM, in first-occurrence order under a left-to-right
/// pre-order walk. This order is the positional contract a `Const`'s level list is zipped
/// against, so it is part of the specification rather than an implementation detail: a
/// declaration written `Π(A : Sort u). Π(B : Sort v). Sort v` has telescope `[u, v]`, and a
/// swapped instantiation is therefore observable wherever the level expression is asymmetric.
///
/// Levels written INSIDE a nested `Const`'s own level arguments count: they are levels this
/// declaration must supply, so they must be bound here.
pub fn free_level_vars(term: &Term) -> Vec<String> {
    #[cfg(test)]
    LEVEL_COLLECTION_CALLS.with(|n| n.set(n.get() + 1));
    let mut out = Vec::new();
    collect_level_vars(term, &mut out);
    out
}

fn collect_level_vars(term: &Term, out: &mut Vec<String>) {
    // Pre-order, left to right, on a stack of its own: the order is the positional contract, and a
    // door reads a term's levels before its depth limit has been checked.
    let mut work = vec![term];
    while let Some(t) = work.pop() {
        match t {
            Term::Sort(u) => level_vars_of_universe(u, out),
            Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => {}
            Term::Const { levels, .. } => {
                for l in levels {
                    level_vars_of_universe(l, out);
                }
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                work.push(body_type);
                work.push(param_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                work.push(body);
                work.push(param_type);
            }
            Term::App(f, a) => {
                work.push(a);
                work.push(f);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.extend(cases.iter().rev());
                work.push(motive);
                work.push(discriminant);
            }
            Term::Fix { body, .. } => work.push(body),
            Term::MutualFix { defs, .. } => work.extend(defs.iter().rev().map(|(_, b)| b)),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push(body);
                work.push(value);
                work.push(ty);
            }
            Term::Annotated { term, .. } => work.push(term),
        }
    }
}

/// The level telescope rule 1 gives a definition whose body mentions levels its type does not: the
/// type's level variables, then the body's, each in first-occurrence order. `None` when the body
/// adds none, so the type alone derives the telescope.
pub fn definition_telescope(ty: &Term, body: &Term) -> Option<Vec<String>> {
    let mut levels = free_level_vars(ty);
    let declared = levels.len();
    collect_level_vars(body, &mut levels);
    (levels.len() > declared).then_some(levels)
}

/// `term`, in place, with each bare reference that is not an application's head written at the
/// level parameters `own_levels` gives its declaration (rule 4): `QUnit` becomes `QUnit.{u}`, and
/// `QUnit_rec P c x` keeps its bare head. A term with no such reference is read once and left as
/// it is. Both walks keep a stack of their own, since a door writes a term before its depth limit
/// has been checked.
pub fn bare_references_at_own_levels(
    term: &mut Term,
    own_levels: &dyn Fn(&str) -> Option<Vec<String>>,
) {
    if !mentions_bare_reference(term, own_levels) {
        return;
    }
    let mut work: Vec<(&mut Term, bool)> = vec![(term, false)];
    while let Some((node, head)) = work.pop() {
        if let Term::Global(name) = node {
            if !head {
                if let Some(params) = own_levels(name) {
                    let name = std::mem::take(name);
                    *node = Term::Const {
                        name,
                        levels: params.into_iter().map(Universe::Var).collect(),
                    };
                }
            }
            continue;
        }
        match node {
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => {}
            Term::App(f, a) => {
                work.push((f, true));
                work.push((a, false));
            }
            Term::Pi {
                param_type,
                body_type: body,
                ..
            }
            | Term::Lambda {
                param_type, body, ..
            } => {
                work.push((param_type, false));
                work.push((body, false));
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push((discriminant, false));
                work.push((motive, false));
                work.extend(cases.iter_mut().map(|c| (c, false)));
            }
            Term::Fix { body, .. } => work.push((body, false)),
            Term::MutualFix { defs, .. } => work.extend(defs.iter_mut().map(|(_, b)| (b, false))),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push((ty, false));
                work.push((value, false));
                work.push((body, false));
            }
            Term::Annotated { term, .. } => work.push((term, head)),
        }
    }
}

/// Whether [`bare_references_at_own_levels`] has anything to write in `term`.
fn mentions_bare_reference(term: &Term, own_levels: &dyn Fn(&str) -> Option<Vec<String>>) -> bool {
    let mut work: Vec<(&Term, bool)> = vec![(term, false)];
    while let Some((node, head)) = work.pop() {
        match node {
            Term::Global(name) => {
                if !head && own_levels(name).is_some() {
                    return true;
                }
            }
            Term::Sort(_) | Term::Var(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => {}
            Term::App(f, a) => {
                work.push((f, true));
                work.push((a, false));
            }
            Term::Pi {
                param_type,
                body_type: body,
                ..
            }
            | Term::Lambda {
                param_type, body, ..
            } => {
                work.push((param_type, false));
                work.push((body, false));
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push((discriminant, false));
                work.push((motive, false));
                work.extend(cases.iter().map(|c| (c, false)));
            }
            Term::Fix { body, .. } => work.push((body, false)),
            Term::MutualFix { defs, .. } => work.extend(defs.iter().map(|(_, b)| (b, false))),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push((ty, false));
                work.push((value, false));
                work.push((body, false));
            }
            Term::Annotated { term, .. } => work.push((term, head)),
        }
    }
    false
}

/// Whether `t` mentions any free universe VARIABLE — allocation-free and short-circuiting.
///
/// This is the GUARD in front of `free_level_vars`, and it is load-bearing for speed rather than
/// for meaning: the application rule asks the question at every node of every application, and
/// collecting a `Vec` there made registering the standard library two orders of magnitude slower.
/// Almost every declaration is monomorphic, so the answer is almost always "no" and must cost
/// nothing to learn.
pub fn mentions_level_var(t: &Term) -> bool {
    level_var_scan(t, &|_| true)
}

/// Whether `t` mentions an unsolved level METAVARIABLE (a `?`-prefixed level variable) —
/// allocation-free and short-circuiting, for the same reason.
pub fn mentions_level_meta(t: &Term) -> bool {
    level_var_scan(t, &|v| is_level_meta(v))
}

/// Short-circuiting search for a level variable satisfying `pred`.
///
/// Recursive and ALLOCATION-FREE by design. This runs at every application node of every
/// type-check, so a `Vec` here — even one that is usually empty — is a malloc per node; the
/// recursion instead costs a stack frame that the kernel's depth door already bounds, and the
/// walk short-circuits on the first hit.
fn level_var_scan(t: &Term, pred: &dyn Fn(&str) -> bool) -> bool {
    match t {
        Term::Sort(u) => universe_mentions(u, pred),
        Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => false,
        Term::Const { levels, .. } => levels.iter().any(|l| universe_mentions(l, pred)),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => level_var_scan(param_type, pred) || level_var_scan(body_type, pred),
        Term::Lambda {
            param_type, body, ..
        } => level_var_scan(param_type, pred) || level_var_scan(body, pred),
        Term::App(f, a) => level_var_scan(f, pred) || level_var_scan(a, pred),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            level_var_scan(discriminant, pred)
                || level_var_scan(motive, pred)
                || cases.iter().any(|c| level_var_scan(c, pred))
        }
        Term::Fix { body, .. } => level_var_scan(body, pred),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| level_var_scan(b, pred)),
        Term::Let {
            ty, value, body, ..
        } => level_var_scan(ty, pred) || level_var_scan(value, pred) || level_var_scan(body, pred),
        Term::Annotated { term, .. } => level_var_scan(term, pred),
    }
}

/// Whether a level expression mentions a variable satisfying `pred` — allocation-free.
pub fn universe_mentions(u: &Universe, pred: &dyn Fn(&str) -> bool) -> bool {
    match u {
        Universe::SProp | Universe::Prop | Universe::Type(_) => false,
        Universe::Var(v) => pred(v),
        Universe::Succ(l) => universe_mentions(l, pred),
        Universe::Max(a, b) | Universe::IMax(a, b) => {
            universe_mentions(a, pred) || universe_mentions(b, pred)
        }
    }
}

/// The level telescope of a declaration (rule 1 above). `declared` is `Some` only for an
/// explicitly universe-polymorphic definition, which carries its own ordered parameter list;
/// every other declaration auto-binds what its type mentions.
pub fn level_telescope(declared: Option<&[String]>, ty: &Term) -> Vec<String> {
    match declared {
        Some(params) => params.to_vec(),
        None => free_level_vars(ty),
    }
}

/// Resolve a `Term::Const { name, levels }` against a declaration, producing the universe
/// substitution to apply to its type AND its body (rules 1-3 above). This is the ONE place the
/// decision is made; `instantiate_universes` applies the result.
pub fn resolve_const_levels(
    name: &str,
    declared: Option<&[String]>,
    ty: &Term,
    levels: &[Universe],
) -> Result<std::collections::HashMap<String, Universe>, ConstError> {
    // Explicit telescopes already own their ordered names. Validate closure
    // with the existing left-to-right short-circuiting scan; only a diagnostic
    // allocates an offending name. Inferred telescopes are closed by construction.
    let inferred;
    let params = match declared {
        Some(params) => {
            let missing = std::cell::RefCell::new(None);
            level_var_scan(ty, &|level| {
                if params.iter().any(|param| param == level) {
                    return false;
                }
                *missing.borrow_mut() = Some(level.to_owned());
                true
            });
            if let Some(level) = missing.into_inner() {
                return Err(ConstError::LevelNotClosed {
                    name: name.to_owned(),
                    level,
                });
            }
            params
        }
        None => {
            inferred = free_level_vars(ty);
            &inferred
        }
    };
    if params.len() != levels.len() {
        return Err(ConstError::Arity {
            name: name.to_string(),
            expected: params.len(),
            got: levels.len(),
        });
    }
    if levels.iter().any(Universe::mentions_sprop) {
        return Err(ConstError::SPropLevel {
            name: name.to_string(),
        });
    }
    Ok(params.iter().cloned().zip(levels.iter().cloned()).collect())
}

// =================================================================================================
// Level metavariables — implicit instantiation at a bare reference
// =================================================================================================
//
// A bare `Term::Global` naming a declaration with a level telescope carries NO levels, so the
// levels have to be solved from the use site — which is what Lean's elaborator does and what a
// Mathlib file relies on in almost every line. The representation reuses the elaborator's own
// convention rather than adding a `Universe` variant: a level metavariable is a `Universe::Var`
// whose name begins with `?`, a spelling no source-level universe parameter can have.
//
// That choice is load-bearing for SOUNDNESS, not only for compatibility: an UNSOLVED `?`-level
// is an ordinary level variable to `level_leq`, where `Type 0 ⋠ ?u` because a variable ranges
// over all of ℕ including `Prop`. So a metavariable that never gets solved makes the reference
// FAIL rather than succeed — the conservative direction. Defaulting it to `Type 0` (the tempting
// move) would do the opposite.

/// The marker a level metavariable's name begins with — the elaborator's convention for a term
/// metavariable, reused so the two never collide with a source-level universe parameter.
pub const LEVEL_META_MARKER: char = '?';

/// Whether a universe variable name denotes a metavariable awaiting a solution.
pub fn is_level_meta(name: &str) -> bool {
    name.starts_with(LEVEL_META_MARKER)
}

/// Fresh level metavariables for `params`, named after the declaration they came from.
///
/// The name is DERIVED (`?u@Box`) rather than drawn from a counter, so the kernel stays a pure
/// function of its input: two runs, two threads and two orders of evaluation produce the same
/// term and the same diagnostic. Two references to the SAME declaration therefore share a
/// metavariable name — which is harmless, because each application rule solves its own
/// substitution locally, and in the one case where two unsolved copies could meet in a single
/// level expression the shared name forces them EQUAL, which can only refuse a term, never
/// admit one.
pub fn fresh_level_metas(
    owner: &str,
    params: &[String],
) -> std::collections::HashMap<String, Universe> {
    params
        .iter()
        .map(|p| {
            (
                p.clone(),
                Universe::Var(format!("{LEVEL_META_MARKER}{p}@{owner}")),
            )
        })
        .collect()
}

/// Solve level metavariables by matching an expected type (the PATTERN, which may contain
/// metavariables) against an actual one, recording solutions in `subst`.
///
/// Only a BARE metavariable is solved, and only once: a first solution wins, and a compound
/// level containing a metavariable (`max ?u v`) is left alone rather than inverted. This is a
/// heuristic for CHOOSING an instantiation, never a substitute for checking it — the caller
/// still checks the argument against the substituted domain, so an unhelpful choice makes the
/// application fail rather than succeed.
pub fn unify_levels(
    pattern: &Term,
    actual: &Term,
    subst: &mut std::collections::HashMap<String, Universe>,
) {
    match (pattern.peel_annotations(), actual.peel_annotations()) {
        (Term::Sort(p), Term::Sort(a)) => unify_level(p, a, subst),
        (
            Term::Pi {
                param_type: p1,
                body_type: b1,
                ..
            },
            Term::Pi {
                param_type: p2,
                body_type: b2,
                ..
            },
        ) => {
            unify_levels(p1, p2, subst);
            unify_levels(b1, b2, subst);
        }
        (
            Term::Lambda {
                param_type: p1,
                body: b1,
                ..
            },
            Term::Lambda {
                param_type: p2,
                body: b2,
                ..
            },
        ) => {
            unify_levels(p1, p2, subst);
            unify_levels(b1, b2, subst);
        }
        (Term::App(f1, a1), Term::App(f2, a2)) => {
            unify_levels(f1, f2, subst);
            unify_levels(a1, a2, subst);
        }
        (
            Term::Const {
                name: n1,
                levels: l1,
            },
            Term::Const {
                name: n2,
                levels: l2,
            },
        ) if n1 == n2 && l1.len() == l2.len() => {
            for (p, a) in l1.iter().zip(l2.iter()) {
                unify_level(p, a, subst);
            }
        }
        _ => {}
    }
}

/// The level half of [`unify_levels`]: solve a bare metavariable, else look inside a matching
/// algebraic shape.
fn unify_level(
    pattern: &Universe,
    actual: &Universe,
    subst: &mut std::collections::HashMap<String, Universe>,
) {
    match pattern {
        Universe::Var(v) if is_level_meta(v) => {
            // An occurs check: solving `?u := succ ?u` would build a cyclic level.
            let mut occurring = Vec::new();
            level_vars_of_universe(actual, &mut occurring);
            if occurring.iter().any(|x| x == v) {
                return;
            }
            subst
                .entry(v.clone())
                .or_insert_with(|| actual.least_level());
        }
        // A concrete level is a successor too: `Type 0` is `succ Prop` and `Type n` is
        // `succ (Type (n-1))`, so `Type u` meets `Type 1` at `u := Type 0` exactly as it meets
        // `succ (Type 0)`.
        Universe::Succ(p) => match actual {
            Universe::Succ(a) => unify_level(p, a, subst),
            Universe::Type(0) => unify_level(p, &Universe::Prop, subst),
            Universe::Type(n) => unify_level(p, &Universe::Type(n - 1), subst),
            _ => {}
        },
        Universe::Max(p1, p2) => {
            if let Universe::Max(a1, a2) = actual {
                unify_level(p1, a1, subst);
                unify_level(p2, a2, subst);
            }
        }
        Universe::IMax(p1, p2) => {
            if let Universe::IMax(a1, a2) = actual {
                unify_level(p1, a1, subst);
                unify_level(p2, a2, subst);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term/universe_algebra_tests.rs"]
mod universe_algebra_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term/display_date_and_serde_tests.rs"]
mod display_date_and_serde_tests;

#[cfg(test)]
thread_local! {static LEVEL_COLLECTION_CALLS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term/borrowed_telescope_controls.rs"]
mod borrowed_telescope_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term/universe_constructor_controls.rs"]
mod universe_constructor_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term/level_order_controls.rs"]
mod level_order_controls;
