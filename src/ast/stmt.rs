use super::logic::LogicExpr;
use crate::intern::Symbol;

/// Binary operation kinds for imperative expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOpKind {
    Add,
    Subtract,
    Multiply,
    Divide,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
}

/// Block is a sequence of statements.
pub type Block<'a> = &'a [Stmt<'a>];

/// Imperative statement AST (LOGOS §15.0.0).
///
/// Stmt is the primary AST node for imperative code blocks like `## Main`
/// and function bodies. The Assert variant bridges to the Logic Kernel.
#[derive(Debug)]
pub enum Stmt<'a> {
    /// Variable binding: `Let x be 5.`
    Let {
        var: Symbol,
        value: &'a Expr<'a>,
        mutable: bool,
    },

    /// Mutation: `Set x to 10.`
    Set {
        target: Symbol,
        value: &'a Expr<'a>,
    },

    /// Function call as statement: `Call process with data.`
    Call {
        function: Symbol,
        args: Vec<&'a Expr<'a>>,
    },

    /// Conditional: `If condition: ... Otherwise: ...`
    If {
        cond: &'a Expr<'a>,
        then_block: Block<'a>,
        else_block: Option<Block<'a>>,
    },

    /// Loop: `While condition: ...`
    While {
        cond: &'a Expr<'a>,
        body: Block<'a>,
    },

    /// Return: `Return x.` or `Return.`
    Return {
        value: Option<&'a Expr<'a>>,
    },

    /// Bridge to Logic Kernel: `Assert that P.`
    Assert {
        proposition: &'a LogicExpr<'a>,
    },
}

/// Shared expression type for pure computations (LOGOS §15.0.0).
///
/// Expr is used by both LogicExpr (as terms) and Stmt (as values).
/// These are pure computations without side effects.
#[derive(Debug)]
pub enum Expr<'a> {
    /// Literal value: 42, "hello", true, nothing
    Literal(Literal),

    /// Variable reference: x
    Identifier(Symbol),

    /// Binary operation: x plus y
    BinaryOp {
        op: BinaryOpKind,
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
    },

    /// Function call as expression: the factorial of n
    Call {
        function: Symbol,
        args: &'a [&'a Expr<'a>],
    },

    /// Index access: item 1 of list (1-indexed)
    Index {
        collection: &'a Expr<'a>,
        index: usize,
    },

    /// Slice access: items 2 through 5 of list (1-indexed, inclusive)
    Slice {
        collection: &'a Expr<'a>,
        start: usize,
        end: usize,
    },
}

/// Literal values in LOGOS.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Integer literal
    Number(i64),
    /// Text literal
    Text(Symbol),
    /// Boolean literal
    Boolean(bool),
    /// The nothing literal (unit type)
    Nothing,
}
