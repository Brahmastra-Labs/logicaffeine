//! Unified term representation for the Calculus of Constructions.
//!
//! In CoC, there is no distinction between terms and types.
//! Everything is a Term in an infinite hierarchy of universes.

use std::fmt;

/// Primitive literal values.
///
/// These are opaque values that compute via hardware ALU, not recursion.
#[derive(Debug, Clone, PartialEq)]
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
    BigInt(
        #[cfg_attr(feature = "serde", serde(with = "bigint_dec"))] logicaffeine_base::BigInt,
    ),
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
                write!(f, "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                       year, month, day, hours, mins, secs_rem)
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
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
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
/// - `Var(u)` is a universe variable (ranges over `Type` levels, `≥ Type 0`)
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
    /// Get the successor universe: `Type n → Type (n+1)`, `Prop → Type 1`, and a
    /// symbolic `Succ(ℓ)` for a level that mentions variables.
    pub fn succ(&self) -> Universe {
        match self {
            Universe::SProp | Universe::Prop => Universe::Type(1),
            Universe::Type(n) => Universe::Type(n + 1),
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
                if self.equiv(other) {
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

    /// Definitional equality of two level expressions, accounting for the algebra
    /// (`max`, `imax`, `succ`, variables) — NOT the derived structural equality.
    /// Decided as mutual subtyping over all variable instantiations.
    pub fn equiv(&self, other: &Universe) -> bool {
        level_leq(self, other) && level_leq(other, self)
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

/// A level in the ℕ-ENCODING used by the decision core: `Prop = 0`, `Type n = n+1`,
/// and every universe VARIABLE ranges over all of ℕ (so a variable may be `Prop`,
/// which is what closes the `Sort u := Nat` unsoundness — `Type 0 ≤ u` is false, since
/// `u` could be `Prop`). This mirrors Lean's level model.
#[derive(Clone, Debug)]
enum LNat {
    Const(u64),
    Var(String),
    Succ(Box<LNat>),
    Max(Box<LNat>, Box<LNat>),
    /// `imax(a, b) = 0` when `b = 0`, else `max(a, b)`.
    IMax(Box<LNat>, Box<LNat>),
}

fn to_lnat(u: &Universe) -> LNat {
    match u {
        // SProp is handled directly in `level_leq` and never survives inside a compound
        // level (max/imax/succ collapse it), so this is a never-hit fallback.
        Universe::SProp => LNat::Const(0),
        Universe::Prop => LNat::Const(0),
        Universe::Type(n) => LNat::Const(*n as u64 + 1),
        Universe::Var(v) => LNat::Var(v.clone()),
        Universe::Succ(l) => LNat::Succ(Box::new(to_lnat(l))),
        Universe::Max(a, b) => LNat::Max(Box::new(to_lnat(a)), Box::new(to_lnat(b))),
        Universe::IMax(a, b) => LNat::IMax(Box::new(to_lnat(a)), Box::new(to_lnat(b))),
    }
}

/// Simplify an [`LNat`], resolving every `imax` whose right argument's Prop-ness is
/// determined and pushing `imax` down over `max`/`imax` until its right argument is a
/// bare variable (or a constant). After this, an unresolved `imax` has the form
/// `imax(_, Var v)`, so the remaining case analysis is on those `v`.
fn simp_lnat(t: &LNat) -> LNat {
    match t {
        LNat::Const(_) | LNat::Var(_) => t.clone(),
        LNat::Succ(l) => LNat::Succ(Box::new(simp_lnat(l))),
        LNat::Max(a, b) => LNat::Max(Box::new(simp_lnat(a)), Box::new(simp_lnat(b))),
        LNat::IMax(a, b) => {
            let a = simp_lnat(a);
            let b = simp_lnat(b);
            match &b {
                // b = 0 ⇒ imax = 0.
                LNat::Const(0) => LNat::Const(0),
                // b ≥ 1 (a positive constant or a successor) ⇒ imax = max(a, b).
                LNat::Const(_) | LNat::Succ(_) => simp_lnat(&LNat::Max(Box::new(a), Box::new(b))),
                // imax(a, max(b1,b2)) = max(imax(a,b1), imax(a,b2)).
                LNat::Max(b1, b2) => simp_lnat(&LNat::Max(
                    Box::new(LNat::IMax(Box::new(a.clone()), b1.clone())),
                    Box::new(LNat::IMax(Box::new(a), b2.clone())),
                )),
                // imax(a, imax(b1,b2)) = imax(max(a,b1), b2).
                LNat::IMax(b1, b2) => simp_lnat(&LNat::IMax(
                    Box::new(LNat::Max(Box::new(a), b1.clone())),
                    b2.clone(),
                )),
                // imax(a, v): stays symbolic (v may be 0 or ≥ 1).
                LNat::Var(_) => LNat::IMax(Box::new(a), Box::new(b)),
            }
        }
    }
}

/// A variable that still appears as the right argument of an `imax` — the pivot to
/// case-split on. `None` when the term is imax-free (a pure `max`/`succ`/`const`/`var`).
fn imax_pivot(t: &LNat) -> Option<String> {
    match t {
        LNat::Const(_) | LNat::Var(_) => None,
        LNat::Succ(l) => imax_pivot(l),
        LNat::Max(a, b) => imax_pivot(a).or_else(|| imax_pivot(b)),
        LNat::IMax(a, b) => match &**b {
            LNat::Var(v) => Some(v.clone()),
            _ => imax_pivot(a).or_else(|| imax_pivot(b)),
        },
    }
}

/// Substitute `Var(v)` by `repl` throughout an [`LNat`].
fn subst_lnat(t: &LNat, v: &str, repl: &LNat) -> LNat {
    match t {
        LNat::Const(_) => t.clone(),
        LNat::Var(x) => {
            if x == v {
                repl.clone()
            } else {
                t.clone()
            }
        }
        LNat::Succ(l) => LNat::Succ(Box::new(subst_lnat(l, v, repl))),
        LNat::Max(a, b) => {
            LNat::Max(Box::new(subst_lnat(a, v, repl)), Box::new(subst_lnat(b, v, repl)))
        }
        LNat::IMax(a, b) => {
            LNat::IMax(Box::new(subst_lnat(a, v, repl)), Box::new(subst_lnat(b, v, repl)))
        }
    }
}

/// A flat atom of an imax-free level: `var + offset` (`var = None` ⇒ a constant
/// `offset`). A level is a `max` of atoms.
fn lnat_atoms(t: &LNat, off: u64, out: &mut Vec<(Option<String>, u64)>) {
    match t {
        LNat::Const(c) => out.push((None, c + off)),
        LNat::Var(v) => out.push((Some(v.clone()), off)),
        LNat::Succ(l) => lnat_atoms(l, off + 1, out),
        LNat::Max(a, b) => {
            lnat_atoms(a, off, out);
            lnat_atoms(b, off, out);
        }
        // An imax-free term never contains IMax (guaranteed by the pivot loop).
        LNat::IMax(..) => unreachable!("lnat_atoms on an unresolved imax"),
    }
}

/// Decide `a ≤ b` for an IMAX-FREE pair, over ALL variable assignments (each variable
/// ranges over ℕ ≥ 0). `max(A) ≤ max(B)` holds iff every atom of `A` is dominated by
/// `B`: a constant `c` needs `c ≤ min max(B)` (all variables at 0); a `var+off` atom
/// needs `B` to contain the SAME variable with offset `≥ off` (else driving it to ∞
/// breaks the bound).
fn leq_linear(a: &LNat, b: &LNat) -> bool {
    let mut atoms_a = Vec::new();
    lnat_atoms(a, 0, &mut atoms_a);
    let mut atoms_b = Vec::new();
    lnat_atoms(b, 0, &mut atoms_b);
    // `min max(B)`: every variable at 0, so each atom contributes its offset.
    let b_min = atoms_b.iter().map(|(_, off)| *off).max().unwrap_or(0);
    atoms_a.iter().all(|(v, off)| match v {
        None => *off <= b_min,
        Some(name) => atoms_b
            .iter()
            .any(|(bv, boff)| bv.as_deref() == Some(name.as_str()) && *boff >= *off),
    })
}

/// Decide cumulative `a ≤ b`, SOUND over all instantiations of the universe variables
/// (each ranges over ℕ, `Prop = 0`). Unresolved `imax(_, v)` is handled by splitting
/// `v` into the `Prop` case (`v := 0`) and the positive case (`v := succ v′`); each
/// split removes a pivot, so the recursion terminates.
fn level_leq(a: &Universe, b: &Universe) -> bool {
    // SProp is the bottom sort — `SProp ≤ everything`, and only `SProp ≤ SProp`. Handled
    // BEFORE the `Prop=0` level encoding, which never sees `SProp` (max/imax/succ collapse
    // it away, so it only ever appears bare here).
    if matches!(a, Universe::SProp) {
        return true;
    }
    if matches!(b, Universe::SProp) {
        return false;
    }
    lnat_leq(&simp_lnat(&to_lnat(a)), &simp_lnat(&to_lnat(b)))
}

fn lnat_leq(a: &LNat, b: &LNat) -> bool {
    match imax_pivot(a).or_else(|| imax_pivot(b)) {
        Some(v) => {
            // v = Prop (0).
            let zero = LNat::Const(0);
            let a0 = simp_lnat(&subst_lnat(a, &v, &zero));
            let b0 = simp_lnat(&subst_lnat(b, &v, &zero));
            // v ≥ 1: v := succ(v′) for a fresh v′ (bakes the `≥ 1` into an offset).
            let vpos = LNat::Succ(Box::new(LNat::Var(format!("{v}✦"))));
            let ap = simp_lnat(&subst_lnat(a, &v, &vpos));
            let bp = simp_lnat(&subst_lnat(b, &v, &vpos));
            lnat_leq(&a0, &b0) && lnat_leq(&ap, &bp)
        }
        None => leq_linear(a, b),
    }
}

/// Unified term representation.
///
/// Every expression in CoC is a Term:
/// - `Sort(u)` - universes (Type 0, Type 1, Prop)
/// - `Var(x)` - variables
/// - `Pi` - dependent function types: Π(x:A). B
/// - `Lambda` - functions: λ(x:A). t
/// - `App` - application: f x
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

impl fmt::Display for Universe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Universe::SProp => write!(f, "SProp"),
            Universe::Prop => write!(f, "Prop"),
            Universe::Type(n) => write!(f, "Type{}", n),
            Universe::Var(v) => write!(f, "{}", v),
            Universe::Succ(l) => write!(f, "({}+1)", l),
            Universe::Max(a, b) => write!(f, "max({}, {})", a, b),
            Universe::IMax(a, b) => write!(f, "imax({}, {})", a, b),
        }
    }
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Term::Sort(u) => write!(f, "{}", u),
            Term::Var(name) => write!(f, "{}", name),
            Term::Global(name) => write!(f, "{}", name),
            Term::Const { name, levels } => {
                write!(f, "{}.{{", name)?;
                for (i, l) in levels.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", l)?;
                }
                write!(f, "}}")
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                // Use arrow notation for non-dependent functions (param = "_")
                if param == "_" {
                    write!(f, "{} -> {}", param_type, body_type)
                } else {
                    write!(f, "Π({}:{}). {}", param, param_type, body_type)
                }
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                write!(f, "λ({}:{}). {}", param, param_type, body)
            }
            Term::App(func, arg) => {
                // Arrow types (Pi with _) need inner parens when used as args
                let arg_needs_inner_parens =
                    matches!(arg.as_ref(), Term::Pi { param, .. } if param == "_");

                if arg_needs_inner_parens {
                    write!(f, "({} ({}))", func, arg)
                } else {
                    write!(f, "({} {})", func, arg)
                }
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                write!(f, "match {} return {} with ", discriminant, motive)?;
                for (i, case) in cases.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", case)?;
                }
                Ok(())
            }
            Term::Fix { name, body } => {
                write!(f, "fix {}. {}", name, body)
            }
            Term::MutualFix { defs, index } => {
                write!(f, "mutualfix {{ ")?;
                for (i, (name, body)) in defs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} := {}", name, body)?;
                }
                write!(f, " }}.{}", index)
            }
            Term::Let { name, ty, value, body } => {
                write!(f, "let {}:{} := {} in {}", name, ty, value, body)
            }
            Term::Lit(lit) => {
                write!(f, "{}", lit)
            }
            Term::Hole => {
                write!(f, "_")
            }
        }
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
        Term::Pi { param, param_type, body_type } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(instantiate_universes(param_type, subst)),
            body_type: Box::new(instantiate_universes(body_type, subst)),
        },
        Term::Lambda { param, param_type, body } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(instantiate_universes(param_type, subst)),
            body: Box::new(instantiate_universes(body, subst)),
        },
        Term::App(f, a) => Term::App(
            Box::new(instantiate_universes(f, subst)),
            Box::new(instantiate_universes(a, subst)),
        ),
        Term::Match { discriminant, motive, cases } => Term::Match {
            discriminant: Box::new(instantiate_universes(discriminant, subst)),
            motive: Box::new(instantiate_universes(motive, subst)),
            cases: cases.iter().map(|c| instantiate_universes(c, subst)).collect(),
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
        Term::Let { name, ty, value, body } => Term::Let {
            name: name.clone(),
            ty: Box::new(instantiate_universes(ty, subst)),
            value: Box::new(instantiate_universes(value, subst)),
            body: Box::new(instantiate_universes(body, subst)),
        },
    }
}
