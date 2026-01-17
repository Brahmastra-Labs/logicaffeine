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
}

impl Eq for Literal {}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Int(n) => write!(f, "{}", n),
            Literal::Float(x) => write!(f, "{}", x),
            Literal::Text(s) => write!(f, "{:?}", s),
        }
    }
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
