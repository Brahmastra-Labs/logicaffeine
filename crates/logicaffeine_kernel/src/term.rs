//! Unified term representation for the Calculus of Constructions.
//!
//! In CoC, there is no distinction between terms and types.
//! Everything is a Term in an infinite hierarchy of universes.

use std::fmt;

/// Primitive literal values.
///
/// These are opaque values that compute via hardware ALU, not recursion.
#[derive(Debug, Clone, PartialEq)]
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

/// Universe levels in the type hierarchy.
///
/// The hierarchy is: Prop : Type 1 : Type 2 : Type 3 : ...
///
/// - `Prop` is the universe of propositions (proof-irrelevant in full CIC)
/// - `Type(n)` is the universe of types at level n
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Universe {
    /// Prop - the universe of propositions
    Prop,
    /// Type n - the universe of types at level n
    Type(u32),
}

impl Universe {
    /// Get the successor universe: Type n → Type (n+1)
    pub fn succ(&self) -> Universe {
        match self {
            Universe::Prop => Universe::Type(1),
            Universe::Type(n) => Universe::Type(n + 1),
        }
    }

    /// Get the maximum of two universes (for Pi type formation)
    pub fn max(&self, other: &Universe) -> Universe {
        match (self, other) {
            (Universe::Prop, u) | (u, Universe::Prop) => u.clone(),
            (Universe::Type(a), Universe::Type(b)) => Universe::Type((*a).max(*b)),
        }
    }

    /// Check if this universe is a subtype of another (cumulativity).
    ///
    /// Subtyping rules:
    /// - Prop ≤ Type(i) for all i
    /// - Type(i) ≤ Type(j) if i ≤ j
    /// - Type(i) is NOT ≤ Prop
    pub fn is_subtype_of(&self, other: &Universe) -> bool {
        match (self, other) {
            // Prop ≤ anything (Prop ≤ Prop, Prop ≤ Type(i))
            (Universe::Prop, _) => true,
            // Type(i) ≤ Type(j) if i ≤ j
            (Universe::Type(i), Universe::Type(j)) => i <= j,
            // Type(i) is NOT ≤ Prop
            (Universe::Type(_), Universe::Prop) => false,
        }
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
pub enum Term {
    /// Universe: Type n or Prop
    Sort(Universe),

    /// Local variable reference (bound by λ or Π)
    Var(String),

    /// Global definition (inductive type or constructor)
    Global(String),

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
            Universe::Prop => write!(f, "Prop"),
            Universe::Type(n) => write!(f, "Type{}", n),
        }
    }
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Term::Sort(u) => write!(f, "{}", u),
            Term::Var(name) => write!(f, "{}", name),
            Term::Global(name) => write!(f, "{}", name),
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
            Term::Lit(lit) => {
                write!(f, "{}", lit)
            }
            Term::Hole => {
                write!(f, "_")
            }
        }
    }
}
