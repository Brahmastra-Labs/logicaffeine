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

use std::collections::{HashMap, VecDeque};

use logicaffeine_compile::ast::stmt::{BinaryOpKind, Block, Expr, Literal, SelectBranch, Stmt};
use logicaffeine_compile::Interner;
use logicaffeine_verify::{BitVecOp, VerifyExpr};

/// The non-native functions a program defines, keyed by name → (parameter names, body).
/// Used to inline a `Launch a task to f(args)` at its spawn point in the determinate model.
type FuncTable<'a> = HashMap<String, (Vec<String>, Block<'a>)>;

/// SplitMix64 — a byte-for-byte mirror of `logicaffeine_runtime::seed::SeededRng`, so a
/// `Select` winner the encoder draws matches the winner the interpreter's scheduler drew at
/// the same seed. (Kept private and re-derived here to honor invariant I6: the TV crate
/// shares the *spec* of the choice function, not a code path linked into any binary.)
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform index in `[0, n)`. Returns 0 *without drawing* when `n <= 1` — matching the
    /// runtime exactly, so the entropy stream stays in lockstep.
    fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
}

/// Bit width used to model LOGOS `Int` (native `i64`).
pub const INT_WIDTH: u32 = 64;

/// A symbolic LOGOS value: an integer (64-bit bitvector), a boolean, or a channel handle.
#[derive(Debug, Clone)]
pub enum SymValue {
    /// An integer value, as a width-64 bitvector `VerifyExpr`.
    Int(VerifyExpr),
    /// A boolean value, as a `Bool`-sorted `VerifyExpr`.
    Bool(VerifyExpr),
    /// A channel handle — an opaque id into [`State::channels`]. Not an SMT value; it is
    /// the dataflow conduit the determinate concurrency fragment threads `Send`/`Receive`
    /// through (and passes as a task argument).
    Chan(usize),
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
    /// Channel buffers, keyed by the opaque id a [`SymValue::Chan`] carries (FIFO histories).
    channels: HashMap<usize, VecDeque<SymValue>>,
    /// Allocates the next channel id, so distinct `Pipe`s never alias.
    chan_counter: usize,
    /// Present in *seeded* mode: resolves `Select` winners from the same SplitMix64 stream
    /// the interpreter's scheduler uses. `None` ⇒ `Select` is `Unsupported` (no entropy).
    rng: Option<SplitMix64>,
}

impl State {
    fn new() -> Self {
        State {
            env: HashMap::new(),
            outputs: Vec::new(),
            errored: VerifyExpr::bool(false),
            channels: HashMap::new(),
            chan_counter: 0,
            rng: None,
        }
    }
}

/// Symbolically execute a program (a sequence of statements) into a [`SymSummary`].
pub fn execute(stmts: &[Stmt], interner: &Interner) -> Result<SymSummary, Unsupported> {
    let funcs = collect_funcs(stmts, interner);
    let mut state = State::new();
    exec_block(&mut state, stmts, interner, &funcs)?;
    Ok(SymSummary {
        outputs: state.outputs,
        errored: state.errored,
    })
}

/// Symbolically execute a *nondeterministic* program under a fixed `seed`, resolving every
/// `Select` winner from a SplitMix64 mirror of the scheduler's choice function. The result
/// is cross-checked per-seed against `run_treewalker_concurrent_seeded` at the same seed,
/// so a misaligned encoding surfaces as a disagreement — never a false proof.
pub fn execute_seeded(stmts: &[Stmt], interner: &Interner, seed: u64) -> Result<SymSummary, Unsupported> {
    let funcs = collect_funcs(stmts, interner);
    let mut state = State::new();
    state.rng = Some(SplitMix64::new(seed));
    exec_block(&mut state, stmts, interner, &funcs)?;
    Ok(SymSummary {
        outputs: state.outputs,
        errored: state.errored,
    })
}

/// Index the program's non-native function definitions for task inlining.
fn collect_funcs<'a>(stmts: &'a [Stmt<'a>], interner: &Interner) -> FuncTable<'a> {
    let mut table = FuncTable::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, is_native: false, .. } = stmt {
            let param_names = params.iter().map(|(p, _)| interner.resolve(*p).to_string()).collect();
            table.insert(interner.resolve(*name).to_string(), (param_names, *body));
        }
    }
    table
}

fn exec_block(
    state: &mut State,
    stmts: &[Stmt],
    interner: &Interner,
    funcs: &FuncTable,
) -> Result<(), Unsupported> {
    for stmt in stmts {
        exec_stmt(state, stmt, interner, funcs)?;
    }
    Ok(())
}

/// Resolve a pipe expression to its channel id, or report why it is not a channel.
fn resolve_chan(state: &mut State, expr: &Expr, interner: &Interner) -> Result<usize, Unsupported> {
    match eval(state, expr, interner)? {
        SymValue::Chan(id) => Ok(id),
        _ => unsupported("pipe operand is not a channel"),
    }
}

fn exec_stmt(
    state: &mut State,
    stmt: &Stmt,
    interner: &Interner,
    funcs: &FuncTable,
) -> Result<(), Unsupported> {
    match stmt {
        // Definitions are indexed up front by `collect_funcs`; nothing to execute here.
        Stmt::FunctionDef { .. } => Ok(()),
        // ---- Determinate concurrency fragment ----
        Stmt::CreatePipe { var, .. } => {
            let id = state.chan_counter;
            state.chan_counter += 1;
            state.channels.insert(id, VecDeque::new());
            state.env.insert(interner.resolve(*var).to_string(), SymValue::Chan(id));
            Ok(())
        }
        Stmt::SendPipe { value, pipe } => {
            let v = eval(state, value, interner)?;
            let id = resolve_chan(state, pipe, interner)?;
            state.channels.get_mut(&id).expect("channel id is allocated").push_back(v);
            Ok(())
        }
        Stmt::ReceivePipe { var, pipe } => {
            let id = resolve_chan(state, pipe, interner)?;
            let v = match state.channels.get_mut(&id).and_then(|q| q.pop_front()) {
                Some(v) => v,
                // A receive on an empty channel cannot be statically resolved under the
                // single modeled schedule — bail honestly rather than guess.
                None => return unsupported("receive on an empty channel (not statically resolvable)"),
            };
            state.env.insert(interner.resolve(*var).to_string(), v);
            Ok(())
        }
        // A launched task runs to completion at its spawn point: in the determinate fragment
        // the output is schedule-independent (Kahn), so this single schedule is canonical.
        // Its `Send`s fill shared channels; a `Receive` it cannot satisfy bails Unsupported.
        Stmt::LaunchTask { function, args } => {
            let fname = interner.resolve(*function).to_string();
            let (params, body) = match funcs.get(&fname) {
                Some(f) => f.clone(),
                None => return unsupported(format!("launch of unknown task '{fname}'")),
            };
            if params.len() != args.len() {
                return unsupported(format!("task '{fname}' arity mismatch"));
            }
            let arg_vals = args
                .iter()
                .map(|a| eval(state, a, interner))
                .collect::<Result<Vec<_>, _>>()?;
            let saved = std::mem::take(&mut state.env);
            for (p, v) in params.iter().zip(arg_vals) {
                state.env.insert(p.clone(), v);
            }
            let r = exec_block(state, body, interner, funcs);
            state.env = saved;
            r
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            // Sequential in the determinate spec — each task runs without a block scope.
            exec_block(state, tasks, interner, funcs)
        }
        // `Await the first of:` — nondeterministic, so only modeled in seeded mode. Ready
        // arms are the `Receive`s whose channel already holds a value (readiness is static
        // once prior tasks have run eagerly); the winner is `below(ready_count)` drawn from
        // the same SplitMix64 the scheduler uses. With no ready receive, the timeout arm
        // fires (deterministic, no draw). The per-seed cross-check is the soundness net.
        Stmt::Select { branches } => {
            if state.rng.is_none() {
                return unsupported("Select requires seeded mode (nondeterministic)");
            }
            // Indices of receive arms whose modeled channel is non-empty.
            let mut ready: Vec<usize> = Vec::new();
            for (i, b) in branches.iter().enumerate() {
                if let SelectBranch::Receive { pipe, .. } = b {
                    let id = resolve_chan(state, pipe, interner)?;
                    if state.channels.get(&id).map(|q| !q.is_empty()).unwrap_or(false) {
                        ready.push(i);
                    }
                }
            }
            let winner = if !ready.is_empty() {
                let k = state.rng.as_mut().unwrap().below(ready.len());
                ready[k]
            } else {
                // No receive ready ⇒ the timeout arm fires. Find the first one.
                match branches.iter().position(|b| matches!(b, SelectBranch::Timeout { .. })) {
                    Some(i) => i,
                    None => return unsupported("Select with no ready arm and no timeout would block"),
                }
            };
            match &branches[winner] {
                SelectBranch::Receive { var, pipe, body } => {
                    let id = resolve_chan(state, pipe, interner)?;
                    let v = state
                        .channels
                        .get_mut(&id)
                        .and_then(|q| q.pop_front())
                        .ok_or_else(|| Unsupported("Select winner channel empty".into()))?;
                    state.env.insert(interner.resolve(*var).to_string(), v);
                    exec_block(state, body, interner, funcs)
                }
                SelectBranch::Timeout { body, .. } => exec_block(state, body, interner, funcs),
            }
        }
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
            SymValue::Chan(_) => unsupported("`not` on a channel"),
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
