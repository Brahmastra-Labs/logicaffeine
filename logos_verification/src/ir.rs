//! Verification IR (Intermediate Representation)
//!
//! A lightweight AST for Z3 verification that decouples from the main LOGOS AST.
//! This avoids circular dependencies: logos depends on logos_verification,
//! so logos_verification cannot depend on logos.
//!
//! **Strategy: Smart Full Mapping with Uninterpreted Functions**
//!
//! Complex types (Modals, Temporals, Predicates) become uninterpreted functions.
//! Z3 can reason about their structure without semantic understanding.
//! E.g., if `Possible(A) -> Possible(B)` and `Possible(A)`, Z3 deduces `Possible(B)`.

/// Type declarations for verification variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyType {
    /// Integer type (maps to Z3 Int sort)
    Int,
    /// Boolean type (maps to Z3 Bool sort)
    Bool,
    /// Opaque object type for entities (maps to uninterpreted sort)
    Object,
}

/// Binary operations in the verification IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOp {
    // Arithmetic (Int -> Int)
    Add,
    Sub,
    Mul,
    Div,

    // Comparison (Int -> Bool)
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,

    // Logic (Bool -> Bool)
    And,
    Or,
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
    pub fn var(name: impl Into<String>) -> Self {
        VerifyExpr::Var(name.into())
    }

    /// Create an integer literal.
    pub fn int(n: i64) -> Self {
        VerifyExpr::Int(n)
    }

    /// Create a boolean literal.
    pub fn bool(b: bool) -> Self {
        VerifyExpr::Bool(b)
    }

    /// Create a binary operation.
    pub fn binary(op: VerifyOp, left: VerifyExpr, right: VerifyExpr) -> Self {
        VerifyExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Create a negation.
    pub fn not(expr: VerifyExpr) -> Self {
        VerifyExpr::Not(Box::new(expr))
    }

    /// Create an uninterpreted function application.
    pub fn apply(name: impl Into<String>, args: Vec<VerifyExpr>) -> Self {
        VerifyExpr::Apply {
            name: name.into(),
            args,
        }
    }

    /// Create a universal quantifier.
    pub fn forall(vars: Vec<(String, VerifyType)>, body: VerifyExpr) -> Self {
        VerifyExpr::ForAll {
            vars,
            body: Box::new(body),
        }
    }

    /// Create an existential quantifier.
    pub fn exists(vars: Vec<(String, VerifyType)>, body: VerifyExpr) -> Self {
        VerifyExpr::Exists {
            vars,
            body: Box::new(body),
        }
    }

    // Convenience methods for common operations

    /// x == y
    pub fn eq(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Eq, left, right)
    }

    /// x > y
    pub fn gt(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Gt, left, right)
    }

    /// x < y
    pub fn lt(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Lt, left, right)
    }

    /// x >= y
    pub fn gte(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Gte, left, right)
    }

    /// x <= y
    pub fn lte(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Lte, left, right)
    }

    /// x != y
    pub fn neq(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Neq, left, right)
    }

    /// x && y
    pub fn and(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::And, left, right)
    }

    /// x || y
    pub fn or(left: VerifyExpr, right: VerifyExpr) -> Self {
        Self::binary(VerifyOp::Or, left, right)
    }

    /// x -> y (implication)
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
