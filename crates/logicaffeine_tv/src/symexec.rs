//! Big-step symbolic execution of the LOGOS Verifiable Core into `VerifyExpr`.
//!
//! The executor walks the AST and produces a [`SymSummary`]: the ordered sequence of
//! `Show` outputs and a sticky "an observable error occurred" condition, each expressed
//! as a `VerifyExpr` over the program's free inputs. Two summaries can then be compared
//! for observable equivalence by the SMT backend ([`crate::equiv`]).
//!
//! ## Verifiable Core (this phase)
//!
//! Straight-line `Int`/`Bool` programs: integer literals, booleans, `+ - *`, signed
//! comparisons, `== !=`, `and`/`or` (logical on `Bool`, bitwise on `Int`), `not`,
//! `Let`, `Set`, and `Show … to show`. `Int` is modeled as a 64-bit bitvector so
//! wrapping/overflow matches the interpreter's native `i64`. Anything outside the
//! fragment yields [`Unsupported`] — never a silent or wrong result.

use std::collections::HashMap;

use logicaffeine_compile::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use logicaffeine_compile::Interner;
use logicaffeine_verify::{BitVecOp, VerifyExpr};

/// Bit width used to model LOGOS `Int` (native `i64`).
pub const INT_WIDTH: u32 = 64;

/// A symbolic LOGOS value: either an integer (64-bit bitvector) or a boolean.
#[derive(Debug, Clone)]
pub enum SymValue {
    /// An integer value, as a width-64 bitvector `VerifyExpr`.
    Int(VerifyExpr),
    /// A boolean value, as a `Bool`-sorted `VerifyExpr`.
    Bool(VerifyExpr),
}

/// The observable summary of a program: the ordered `Show` outputs plus the condition
/// under which the program raised an observable error.
#[derive(Debug, Clone)]
pub struct SymSummary {
    /// Ordered `Show` emissions, in execution order.
    pub outputs: Vec<SymValue>,
    /// Sticky condition (a `Bool` `VerifyExpr`) that is true exactly when the program
    /// raised an observable error (e.g. division by zero).
    pub errored: VerifyExpr,
}

/// A construct outside the currently-supported Verifiable Core. Carries a human reason
/// so the validator can report `Unverified(reason)` rather than ever guessing.
#[derive(Debug, Clone)]
pub struct Unsupported(pub String);

fn unsupported<T>(reason: impl Into<String>) -> Result<T, Unsupported> {
    Err(Unsupported(reason.into()))
}

/// Mutable symbolic-execution state threaded through a straight-line block.
struct State {
    env: HashMap<String, SymValue>,
    outputs: Vec<SymValue>,
    errored: VerifyExpr,
}

impl State {
    fn new() -> Self {
        State {
            env: HashMap::new(),
            outputs: Vec::new(),
            errored: VerifyExpr::bool(false),
        }
    }
}

/// Symbolically execute a program (a sequence of statements) into a [`SymSummary`].
pub fn execute(stmts: &[Stmt], interner: &Interner) -> Result<SymSummary, Unsupported> {
    let mut state = State::new();
    exec_block(&mut state, stmts, interner)?;
    Ok(SymSummary {
        outputs: state.outputs,
        errored: state.errored,
    })
}

fn exec_block(state: &mut State, stmts: &[Stmt], interner: &Interner) -> Result<(), Unsupported> {
    for stmt in stmts {
        exec_stmt(state, stmt, interner)?;
    }
    Ok(())
}

fn exec_stmt(state: &mut State, stmt: &Stmt, interner: &Interner) -> Result<(), Unsupported> {
    match stmt {
        Stmt::Let { var, value, .. } => {
            let v = eval(state, value, interner)?;
            state.env.insert(interner.resolve(*var).to_string(), v);
            Ok(())
        }
        Stmt::Set { target, value } => {
            let name = interner.resolve(*target).to_string();
            if !state.env.contains_key(&name) {
                return unsupported(format!("Set to variable '{name}' not in scope"));
            }
            let v = eval(state, value, interner)?;
            state.env.insert(name, v);
            Ok(())
        }
        Stmt::Show { object, recipient } => {
            // Only `Show <expr> to show` (console) is in-fragment; showing to a
            // function is an effectful call we do not model yet.
            if let Expr::Identifier(sym) = recipient {
                if interner.resolve(*sym) == "show" {
                    let v = eval(state, object, interner)?;
                    state.outputs.push(v);
                    return Ok(());
                }
            }
            unsupported("Show to a non-console recipient")
        }
        other => unsupported(format!("statement {}", stmt_kind(other))),
    }
}

fn eval(state: &mut State, expr: &Expr, interner: &Interner) -> Result<SymValue, Unsupported> {
    match expr {
        Expr::Literal(Literal::Number(n)) => {
            Ok(SymValue::Int(VerifyExpr::bv_const(INT_WIDTH, *n as u64)))
        }
        Expr::Literal(Literal::Boolean(b)) => Ok(SymValue::Bool(VerifyExpr::bool(*b))),
        Expr::Literal(_) => unsupported("non-Int/Bool literal"),
        Expr::Identifier(sym) => {
            let name = interner.resolve(*sym);
            state
                .env
                .get(name)
                .cloned()
                .ok_or_else(|| Unsupported(format!("reference to unbound variable '{name}'")))
        }
        Expr::BinaryOp { op, left, right } => {
            let l = eval(state, left, interner)?;
            let r = eval(state, right, interner)?;
            match op {
                // Division / modulo: by-zero is an observable error in the interpreter,
                // so record `divisor == 0` into the sticky error condition. The result
                // value past that point is irrelevant (observable equivalence only
                // compares outputs when neither side errored).
                BinaryOpKind::Divide | BinaryOpKind::Modulo => {
                    let (a, b) = match (l, r) {
                        (SymValue::Int(a), SymValue::Int(b)) => (a, b),
                        _ => return unsupported("division on non-Int operands"),
                    };
                    let div_by_zero = VerifyExpr::bv_binary(
                        BitVecOp::Eq,
                        b.clone(),
                        VerifyExpr::bv_const(INT_WIDTH, 0),
                    );
                    state.errored = VerifyExpr::or(state.errored.clone(), div_by_zero);
                    let bvop = if matches!(op, BinaryOpKind::Divide) {
                        BitVecOp::SDiv
                    } else {
                        BitVecOp::SRem
                    };
                    Ok(SymValue::Int(VerifyExpr::bv_binary(bvop, a, b)))
                }
                _ => apply_binop(*op, l, r),
            }
        }
        Expr::Not { operand } => match eval(state, operand, interner)? {
            SymValue::Bool(e) => Ok(SymValue::Bool(VerifyExpr::not(e))),
            // Bitwise NOT on an i64: ~x = x XOR 0xFFFF_FFFF_FFFF_FFFF.
            SymValue::Int(e) => Ok(SymValue::Int(VerifyExpr::bv_binary(
                BitVecOp::Xor,
                e,
                VerifyExpr::bv_const(INT_WIDTH, u64::MAX),
            ))),
        },
        other => unsupported(format!("expression {}", expr_kind(other))),
    }
}

fn apply_binop(op: BinaryOpKind, l: SymValue, r: SymValue) -> Result<SymValue, Unsupported> {
    use BinaryOpKind::*;
    use SymValue::{Bool, Int};
    match (op, l, r) {
        // ---- Integer arithmetic (wrapping, two's complement) ----
        (Add, Int(a), Int(b)) => Ok(Int(VerifyExpr::bv_binary(BitVecOp::Add, a, b))),
        (Subtract, Int(a), Int(b)) => Ok(Int(VerifyExpr::bv_binary(BitVecOp::Sub, a, b))),
        (Multiply, Int(a), Int(b)) => Ok(Int(VerifyExpr::bv_binary(BitVecOp::Mul, a, b))),

        // ---- Signed integer comparison ----
        (Lt, Int(a), Int(b)) => Ok(Bool(VerifyExpr::bv_binary(BitVecOp::SLt, a, b))),
        (Gt, Int(a), Int(b)) => Ok(Bool(VerifyExpr::bv_binary(BitVecOp::SLt, b, a))),
        (LtEq, Int(a), Int(b)) => Ok(Bool(VerifyExpr::bv_binary(BitVecOp::SLe, a, b))),
        (GtEq, Int(a), Int(b)) => Ok(Bool(VerifyExpr::bv_binary(BitVecOp::SLe, b, a))),

        // ---- Equality (Int via bv, Bool via iff) ----
        (Eq, Int(a), Int(b)) => Ok(Bool(VerifyExpr::bv_binary(BitVecOp::Eq, a, b))),
        (Eq, Bool(a), Bool(b)) => Ok(Bool(VerifyExpr::iff(a, b))),
        (NotEq, Int(a), Int(b)) => Ok(Bool(VerifyExpr::not(VerifyExpr::bv_binary(
            BitVecOp::Eq,
            a,
            b,
        )))),
        (NotEq, Bool(a), Bool(b)) => Ok(Bool(VerifyExpr::not(VerifyExpr::iff(a, b)))),

        // ---- And / Or (logical on Bool, bitwise on Int — matches the interpreter) ----
        (And, Bool(a), Bool(b)) => Ok(Bool(VerifyExpr::and(a, b))),
        (And, Int(a), Int(b)) => Ok(Int(VerifyExpr::bv_binary(BitVecOp::And, a, b))),
        (Or, Bool(a), Bool(b)) => Ok(Bool(VerifyExpr::or(a, b))),
        (Or, Int(a), Int(b)) => Ok(Int(VerifyExpr::bv_binary(BitVecOp::Or, a, b))),

        (op, _, _) => unsupported(format!("operator {op:?} on these operand types")),
    }
}

fn stmt_kind(s: &Stmt) -> &'static str {
    match s {
        Stmt::If { .. } => "If",
        Stmt::While { .. } => "While",
        Stmt::Repeat { .. } => "Repeat",
        Stmt::Return { .. } => "Return",
        Stmt::Inspect { .. } => "Inspect",
        Stmt::FunctionDef { .. } => "FunctionDef",
        Stmt::Call { .. } => "Call",
        _ => "<other>",
    }
}

fn expr_kind(e: &Expr) -> &'static str {
    match e {
        Expr::Call { .. } => "Call",
        Expr::Index { .. } => "Index",
        Expr::FieldAccess { .. } => "FieldAccess",
        Expr::List(_) => "List",
        Expr::InterpolatedString(_) => "InterpolatedString",
        _ => "<other>",
    }
}
