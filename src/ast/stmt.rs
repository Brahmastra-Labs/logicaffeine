use super::logic::LogicExpr;
use crate::intern::Symbol;

/// Type expression for explicit type annotations.
///
/// Represents type syntax like:
/// - `Int` → Primitive(Int)
/// - `User` → Named(User)
/// - `List of Int` → Generic { base: List, params: [Primitive(Int)] }
/// - `List of List of Int` → Generic { base: List, params: [Generic { base: List, params: [Primitive(Int)] }] }
/// - `Result of Int and Text` → Generic { base: Result, params: [Primitive(Int), Primitive(Text)] }
#[derive(Debug, Clone)]
pub enum TypeExpr<'a> {
    /// Primitive type: Int, Nat, Text, Bool
    Primitive(Symbol),
    /// Named type (user-defined): User, Point
    Named(Symbol),
    /// Generic type: List of Int, Option of Text, Result of Int and Text
    Generic {
        base: Symbol,
        params: &'a [TypeExpr<'a>],
    },
    /// Function type: fn(A, B) -> C (for higher-order functions)
    Function {
        inputs: &'a [TypeExpr<'a>],
        output: &'a TypeExpr<'a>,
    },
    /// Phase 43C: Refinement type with predicate constraint
    /// Example: `Int where it > 0`
    Refinement {
        /// The base type being refined
        base: &'a TypeExpr<'a>,
        /// The bound variable (usually "it")
        var: Symbol,
        /// The predicate constraint (from Logic Kernel)
        predicate: &'a LogicExpr<'a>,
    },
}

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
    // Grand Challenge: Logical operators for compound conditions
    And,
    Or,
}

/// Block is a sequence of statements.
pub type Block<'a> = &'a [Stmt<'a>];

/// Phase 33: Match arm for Inspect statement
#[derive(Debug)]
pub struct MatchArm<'a> {
    pub enum_name: Option<Symbol>,          // The enum type (e.g., Shape)
    pub variant: Option<Symbol>,            // None = Otherwise (wildcard)
    pub bindings: Vec<(Symbol, Symbol)>,    // (field_name, binding_name)
    pub body: Block<'a>,
}

/// Imperative statement AST (LOGOS §15.0.0).
///
/// Stmt is the primary AST node for imperative code blocks like `## Main`
/// and function bodies. The Assert variant bridges to the Logic Kernel.
#[derive(Debug)]
pub enum Stmt<'a> {
    /// Variable binding: `Let x be 5.` or `Let x: Int be 5.`
    Let {
        var: Symbol,
        ty: Option<&'a TypeExpr<'a>>,
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

    /// Iteration: `Repeat for x in list: ...` or `Repeat for i from 1 to 10: ...`
    Repeat {
        var: Symbol,
        iterable: &'a Expr<'a>,
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

    /// Phase 35: Documented assertion with justification
    /// `Trust that P because "reason".`
    /// Semantics: Documented runtime check that could be verified statically.
    Trust {
        proposition: &'a LogicExpr<'a>,
        justification: Symbol,
    },

    /// Ownership transfer (move): `Give x to processor.`
    /// Semantics: Move ownership of `object` to `recipient`.
    Give {
        object: &'a Expr<'a>,
        recipient: &'a Expr<'a>,
    },

    /// Immutable borrow: `Show x to console.`
    /// Semantics: Immutable borrow of `object` passed to `recipient`.
    Show {
        object: &'a Expr<'a>,
        recipient: &'a Expr<'a>,
    },

    /// Phase 31: Field mutation: `Set p's x to 10.`
    SetField {
        object: &'a Expr<'a>,
        field: Symbol,
        value: &'a Expr<'a>,
    },

    /// Phase 31: Struct definition for codegen
    StructDef {
        name: Symbol,
        fields: Vec<(Symbol, Symbol, bool)>, // (name, type_name, is_public)
    },

    /// Phase 32/38: Function definition
    /// Phase 38: Updated for native functions and TypeExpr types
    FunctionDef {
        name: Symbol,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>, // Phase 38: Changed to TypeExpr
        body: Block<'a>,
        return_type: Option<&'a TypeExpr<'a>>,   // Phase 38: Changed to TypeExpr
        is_native: bool,                          // Phase 38: Native function flag
    },

    /// Phase 33: Pattern matching on sum types
    Inspect {
        target: &'a Expr<'a>,
        arms: Vec<MatchArm<'a>>,
        has_otherwise: bool,            // For exhaustiveness tracking
    },

    /// Phase 43D: Push to collection: `Push x to items.`
    Push {
        value: &'a Expr<'a>,
        collection: &'a Expr<'a>,
    },

    /// Phase 43D: Pop from collection: `Pop from items.` or `Pop from items into y.`
    Pop {
        collection: &'a Expr<'a>,
        into: Option<Symbol>,
    },

    /// Index assignment: `Set item N of X to Y.`
    SetIndex {
        collection: &'a Expr<'a>,
        index: &'a Expr<'a>,
        value: &'a Expr<'a>,
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

    /// Function call as expression: f(x, y)
    Call {
        function: Symbol,
        args: Vec<&'a Expr<'a>>,
    },

    /// Phase 43D: Dynamic index access: `items at i` (1-indexed)
    Index {
        collection: &'a Expr<'a>,
        index: &'a Expr<'a>,
    },

    /// Phase 43D: Dynamic slice access: `items 1 through mid` (1-indexed, inclusive)
    Slice {
        collection: &'a Expr<'a>,
        start: &'a Expr<'a>,
        end: &'a Expr<'a>,
    },

    /// Phase 43D: Copy expression: `copy of slice` → slice.to_vec()
    Copy {
        expr: &'a Expr<'a>,
    },

    /// Phase 43D: Length expression: `length of items` → items.len()
    Length {
        collection: &'a Expr<'a>,
    },

    /// List literal: [1, 2, 3]
    List(Vec<&'a Expr<'a>>),

    /// Range: 1 to 10 (inclusive)
    Range {
        start: &'a Expr<'a>,
        end: &'a Expr<'a>,
    },

    /// Phase 31: Field access: `p's x` or `the x of p`
    FieldAccess {
        object: &'a Expr<'a>,
        field: Symbol,
    },

    /// Phase 31: Constructor: `a new Point` or `a new Point with x 10 and y 20`
    /// Phase 34: Extended for generics: `a new Box of Int`
    New {
        type_name: Symbol,
        type_args: Vec<Symbol>,  // Empty for non-generic types
        init_fields: Vec<(Symbol, &'a Expr<'a>)>,  // Optional field initialization
    },

    /// Phase 33: Enum variant constructor: `a new Circle with radius 10`
    NewVariant {
        enum_name: Symbol,                      // Shape (resolved from registry)
        variant: Symbol,                        // Circle
        fields: Vec<(Symbol, &'a Expr<'a>)>,    // [(radius, 10)]
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
