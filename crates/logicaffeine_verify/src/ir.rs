//! Verification IR (Intermediate Representation)
//!
//! A lightweight AST for Z3 verification that decouples from the main Logicaffeine AST.
//! This avoids circular dependencies: logicaffeine depends on logicaffeine_verify,
//! so logicaffeine_verify cannot depend on logicaffeine.
//!
//! ## Usage
//!
//! Build expressions using the [`VerifyExpr`] constructors:
//!
//! ```
//! use logicaffeine_verify::{VerifyExpr, VerifyOp, VerifyType};
//!
//! // Build: x > 5 && y < 10
//! let expr = VerifyExpr::and(
//!     VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5)),
//!     VerifyExpr::lt(VerifyExpr::var("y"), VerifyExpr::int(10)),
//! );
//! ```
//!
//! ## Encoding Strategy
//!
//! Complex types (modals, temporals, predicates) become uninterpreted functions.
//! Z3 reasons about their structure without semantic understanding.
//!
//! For example, given `Possible(A) -> Possible(B)` and `Possible(A)`, Z3 deduces `Possible(B)`.

/// Type declarations for verification variables.
///
/// Each type maps to a Z3 sort:
///
/// | VerifyType | Z3 Sort | Usage |
/// |------------|---------|-------|
/// | `Int` | `IntSort` | Numeric constraints, bounds checking |
/// | `Bool` | `BoolSort` | Logical propositions |
/// | `Object` | Uninterpreted | Entities (people, objects, propositions) |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyType {
    /// Integer type, maps to Z3 `IntSort`.
    Int,
    /// Boolean type, maps to Z3 `BoolSort`.
    Bool,
    /// Opaque object type for entities, maps to an uninterpreted sort.
    Object,
}

/// Binary operations in the verification IR.
///
/// Operations are grouped by category:
/// - **Arithmetic**: `Add`, `Sub`, `Mul`, `Div` (Int × Int → Int)
/// - **Comparison**: `Eq`, `Neq`, `Gt`, `Lt`, `Gte`, `Lte` (Int × Int → Bool)
/// - **Logic**: `And`, `Or`, `Implies` (Bool × Bool → Bool)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOp {
    // ---- Arithmetic (Int × Int → Int) ----

    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Mul,
    /// Integer division.
    Div,

    // ---- Comparison (Int × Int → Bool) ----

    /// Equality.
    Eq,
    /// Inequality.
    Neq,
    /// Greater than.
    Gt,
    /// Less than.
    Lt,
    /// Greater than or equal.
    Gte,
    /// Less than or equal.
    Lte,

    // ---- Logic (Bool × Bool → Bool) ----

    /// Conjunction.
    And,
    /// Disjunction.
    Or,
    /// Material implication.
    Implies,
}

/// Expression AST for verification.
///
/// This IR is designed to be easily encodable into Z3 ASTs.
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyExpr {
    /// Integer literal
    Int(i64),

    /// Boolean literal
    Bool(bool),

    /// Variable reference
    Var(String),

    /// Binary operation
    Binary {
        op: VerifyOp,
        left: Box<VerifyExpr>,
        right: Box<VerifyExpr>,
    },

    /// Logical negation
    Not(Box<VerifyExpr>),

    /// Universal quantifier: forall x: T. P(x)
    ForAll {
        vars: Vec<(String, VerifyType)>,
        body: Box<VerifyExpr>,
    },

    /// Existential quantifier: exists x: T. P(x)
    Exists {
        vars: Vec<(String, VerifyType)>,
        body: Box<VerifyExpr>,
    },

    /// Uninterpreted function application (the "catch-all")
    ///
    /// Used for predicates, modals, temporals, etc. that we can't
    /// directly encode semantically. Z3 treats these as opaque functions
    /// and reasons about them structurally.
    ///
    /// Examples:
    /// - `Mortal(socrates)` -> `Apply { name: "Mortal", args: [Var("socrates")] }`
    /// - `Possible(P)` -> `Apply { name: "Possible", args: [P] }`
    Apply {
        name: String,
        args: Vec<VerifyExpr>,
    },
}

impl VerifyExpr {
    /// Create a variable reference.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let x = VerifyExpr::var("x");
    /// let counter = VerifyExpr::var("counter");
    /// ```
    pub fn var(name: impl Into<String>) -> Self {
        VerifyExpr::Var(name.into())
    }

    /// Create an integer literal.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let five = VerifyExpr::int(5);
    /// let negative = VerifyExpr::int(-42);
    /// ```
    pub fn int(n: i64) -> Self {
        VerifyExpr::Int(n)
    }

    /// Create a boolean literal.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let truth = VerifyExpr::bool(true);
    /// let falsity = VerifyExpr::bool(false);
    /// ```
    pub fn bool(b: bool) -> Self {
        VerifyExpr::Bool(b)
    }

    /// Create a binary operation.
    ///
    /// For common operations, prefer the convenience methods like [`eq`](Self::eq),
    /// [`gt`](Self::gt), [`and`](Self::and), etc.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::{VerifyExpr, VerifyOp};
    ///
    /// // x + y
    /// let sum = VerifyExpr::binary(
    ///     VerifyOp::Add,
    ///     VerifyExpr::var("x"),
    ///     VerifyExpr::var("y"),
    /// );
    /// ```
    pub fn binary(op: VerifyOp, left: VerifyExpr, right: VerifyExpr) -> Self {
        VerifyExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Create a negation.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// // ¬p
    /// let not_p = VerifyExpr::not(VerifyExpr::var("p"));
    ///
    /// // ¬(x > 5)
    /// let not_gt = VerifyExpr::not(
    ///     VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5))
    /// );
    /// ```
    pub fn not(expr: VerifyExpr) -> Self {
        VerifyExpr::Not(Box::new(expr))
    }

    /// Create an uninterpreted function application.
    ///
    /// Use this for predicates, modals, temporals, and other constructs
    /// that cannot be directly encoded semantically. Z3 treats these as
    /// opaque functions and reasons about them structurally.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// // Mortal(socrates)
    /// let mortal = VerifyExpr::apply("Mortal", vec![VerifyExpr::var("socrates")]);
    ///
    /// // Possible(P) for modal logic
    /// let possible_p = VerifyExpr::apply("Possible", vec![VerifyExpr::var("P")]);
    ///
    /// // Before(e1, e2) for temporal relations
    /// let before = VerifyExpr::apply("Before", vec![
    ///     VerifyExpr::var("e1"),
    ///     VerifyExpr::var("e2"),
    /// ]);
    /// ```
    pub fn apply(name: impl Into<String>, args: Vec<VerifyExpr>) -> Self {
        VerifyExpr::Apply {
            name: name.into(),
            args,
        }
    }

    /// Create a universal quantifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::{VerifyExpr, VerifyType};
    ///
    /// // ∀x: Object. Mortal(x) → Human(x)
    /// let all_mortals_are_human = VerifyExpr::forall(
    ///     vec![("x".to_string(), VerifyType::Object)],
    ///     VerifyExpr::implies(
    ///         VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]),
    ///         VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]),
    ///     ),
    /// );
    /// ```
    pub fn forall(vars: Vec<(String, VerifyType)>, body: VerifyExpr) -> Self {
        VerifyExpr::ForAll {
            vars,
            body: Box::new(body),
        }
    }

    /// Create an existential quantifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::{VerifyExpr, VerifyType};
    ///
    /// // ∃x: Object. Mortal(x)
    /// let something_is_mortal = VerifyExpr::exists(
    ///     vec![("x".to_string(), VerifyType::Object)],
    ///     VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]),
    /// );
    /// ```
    pub fn exists(vars: Vec<(String, VerifyType)>, body: VerifyExpr) -> Self {
        VerifyExpr::Exists {
            vars,
            body: Box::new(body),
        }
    }

    // ---- Convenience methods for common operations ----

    /// Equality: `left == right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let x_equals_10 = VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10));
    /// ```
    pub fn eq(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Eq, left, right)
    }

    /// Greater than: `left > right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let x_gt_5 = VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5));
    /// ```
    pub fn gt(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Gt, left, right)
    }

    /// Less than: `left < right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let x_lt_100 = VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(100));
    /// ```
    pub fn lt(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Lt, left, right)
    }

    /// Greater than or equal: `left >= right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let x_gte_0 = VerifyExpr::gte(VerifyExpr::var("x"), VerifyExpr::int(0));
    /// ```
    pub fn gte(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Gte, left, right)
    }

    /// Less than or equal: `left <= right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let x_lte_max = VerifyExpr::lte(VerifyExpr::var("x"), VerifyExpr::var("max"));
    /// ```
    pub fn lte(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Lte, left, right)
    }

    /// Inequality: `left != right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// let x_neq_0 = VerifyExpr::neq(VerifyExpr::var("x"), VerifyExpr::int(0));
    /// ```
    pub fn neq(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Neq, left, right)
    }

    /// Conjunction: `left && right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// // x > 0 && x < 100
    /// let in_range = VerifyExpr::and(
    ///     VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    ///     VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(100)),
    /// );
    /// ```
    pub fn and(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::And, left, right)
    }

    /// Disjunction: `left || right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// // x < 0 || x > 100
    /// let out_of_range = VerifyExpr::or(
    ///     VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    ///     VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(100)),
    /// );
    /// ```
    pub fn or(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Or, left, right)
    }

    /// Material implication: `left → right`.
    ///
    /// # Examples
    ///
    /// ```
    /// use logicaffeine_verify::VerifyExpr;
    ///
    /// // Mortal(x) → Human(x)
    /// let mortals_are_human = VerifyExpr::implies(
    ///     VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]),
    ///     VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]),
    /// );
    /// ```
    pub fn implies(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Implies, left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_expr_construction() {
        // Test that we can construct expressions
        let x = VerifyExpr::var("x");
        let five = VerifyExpr::int(5);
        let ten = VerifyExpr::int(10);

        // x > 5
        let gt = VerifyExpr::gt(x.clone(), five);
        assert!(matches!(gt, VerifyExpr::Binary { op: VerifyOp::Gt, .. }));

        // x == 10
        let eq = VerifyExpr::eq(x.clone(), ten);
        assert!(matches!(eq, VerifyExpr::Binary { op: VerifyOp::Eq, .. }));
    }

    #[test]
    fn test_uninterpreted_function() {
        // Mortal(x)
        let mortal_x = VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]);
        assert!(matches!(mortal_x, VerifyExpr::Apply { name, args } if name == "Mortal" && args.len() == 1));
    }

    #[test]
    fn test_implication() {
        // Mortal(x) -> Human(x)
        let mortal = VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]);
        let human = VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]);
        let impl_expr = VerifyExpr::implies(mortal, human);

        assert!(matches!(impl_expr, VerifyExpr::Binary { op: VerifyOp::Implies, .. }));
    }

    #[test]
    fn test_quantifier() {
        // forall x: Object. Mortal(x) -> Human(x)
        let body = VerifyExpr::implies(
            VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]),
            VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]),
        );
        let forall = VerifyExpr::forall(
            vec![("x".to_string(), VerifyType::Object)],
            body,
        );

        assert!(matches!(forall, VerifyExpr::ForAll { vars, .. } if vars.len() == 1));
    }
}
