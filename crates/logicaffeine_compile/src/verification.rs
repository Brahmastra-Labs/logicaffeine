//! Z3-based static verification pass.
//!
//! This module bridges the LOGOS AST to the Z3-based verification system,
//! mapping `Stmt`, `Expr`, `LogicExpr` and `Term` to the verification IR
//! which is then encoded into Z3 constraints.
//!
//! # The contract: nothing unchecked is counted as verified
//!
//! `largo verify` reports a program verified only when every claim the program makes was checked:
//! each `Assert`, each refinement type (at its `Let`, at every later `Set` of that binding, and at
//! every `Return` of a function whose return type is refined), and each `decreasing` loop. A claim
//! the pass cannot model is REFUSED by name, never skipped, so "passed" always means "proved".
//! `Require` and `Assume` are enforced when the program runs, so execution continues past one only
//! when it held: the pass assumes them.
//!
//! Soundness rests on these rules:
//!
//! | Rule | Why |
//! |------|-----|
//! | Every binding is a fresh solver name per assignment (SSA) | `Set x to 6` after `Let x be 5` is a new value, not a second equation that contradicts the first |
//! | A branch's facts hold only under its condition, and the two arms join through a guarded merge | a branch that does not run decides nothing |
//! | A branch arm that always leaves (`Return`, `Break`, `Fail`) puts the rest of the block under the other arm's condition | an early return narrows what follows it |
//! | A declaration inside a block shadows only inside it | the code after the block reads the outer binding |
//! | Whatever a loop, a call or an unmodelled statement may write is forgotten; a loop that cannot break out leaves its exit condition behind | the pass never keeps a fact the program may have changed |
//! | A call stands for its result only when the function is pure; any other value the pass cannot model (an impure call, text, a field, division, a float) is unknown | an impure function may read the console, a clock or a global it changes, and `/` truncates where Z3's integer division floors |
//!
//! Each function body is checked in its own session (its parameters are unknown integers), so a
//! function's facts never leak into `## Main`. `## Test` blocks are not part of the program `largo
//! verify` checks. Integers are unbounded in the model, as LOGOS `Int` is.
//!
//! # Verification Flow
//!
//! ```text
//! LOGOS AST
//!     │
//!     ▼
//! ┌────────────────────┐
//! │  VerificationPass  │ ─── SSA names, guarded branches, refusals
//! └──────────┬─────────┘
//!            ▼
//!     VerifyExpr (IR)
//!            │
//!            ▼
//! ┌────────────────────┐
//! │ VerificationSession│ ─── assume/check
//! └──────────┬─────────┘
//!            ▼
//!     Z3 Result (Sat/Unsat)
//! ```
//!
//! # Feature Flag
//!
//! This module requires the `verification` feature to be enabled.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use logicaffeine_base::{Interner, Symbol};
use logicaffeine_language::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use logicaffeine_language::ast::{AspectOperator, TemporalOperator};
use logicaffeine_language::ast::{LogicExpr, ModalDomain, NumberKind, QuantifierKind, Term};
use logicaffeine_language::token::TokenType;

use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyOp, VerifyType};

use crate::optimize::purity::Purity;
use crate::optimize::writes::{pattern_symbols, CalleeGlobals, WriteSet};

/// The verification pass that maps LOGOS AST to Z3 constraints.
///
/// It reads the program as a sequence of facts about versions of its variables. Every assignment makes a new
/// version, so a fact about an earlier value never speaks about a later one; a branch's facts hold under its
/// condition and the two sides join after it; a loop forgets every name its body may write; an assertion is checked
/// under the conditions it runs under. Anything the pass does not model is forgotten rather than guessed: a name a
/// statement may change gets a fresh, unconstrained version, so every fact the pass keeps is true of the run.
pub struct VerificationPass<'a> {
    interner: &'a Interner,
}

impl<'a> VerificationPass<'a> {
    /// Create a new verification pass.
    pub fn new(interner: &'a Interner) -> Self {
        Self { interner }
    }

    /// Verify every claim `stmts` makes, or say which one could not be verified and why.
    ///
    /// Every `Assert` must follow from what precedes it on its path; refinements and `decreasing` clauses are
    /// checked where they stand. `Require` and `Assume` are enforced at run time, so execution continues past one
    /// only when it held: the pass assumes them. A crash inside the solver bridge is reported as a refusal, never as
    /// a pass and never as a crash of the compiler that called it.
    pub fn verify_program(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        crate::analysis::callgraph::install_lending_parameters(stmts);
        let program = Program::of(stmts, self.interner);
        let mut walk = Walk::new(self.interner, &program);
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| walk.block(stmts))) {
            Ok(verdict) => verdict.map(|_| ()),
            Err(panic) => {
                let message = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "no message".to_string());
                Err(format!(
                    "the verifier stopped on an internal error ({message}), so this program is NOT \
                     verified. This is a bug in `largo verify`, not in the program"
                ))
            }
        }
    }
}

/// The solver sort of a value the pass models. A binding whose value it cannot model is
/// `Opaque`: it has a solver name, and nothing is ever assumed or concluded about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    Int,
    Bool,
    Opaque,
}

impl Sort {
    fn verify_type(self) -> VerifyType {
        match self {
            Sort::Int => VerifyType::Int,
            Sort::Bool => VerifyType::Bool,
            Sort::Opaque => VerifyType::Object,
        }
    }

    /// The sort a declared type gives a binding before anything is known about its value.
    fn of_type(ty: &TypeExpr, interner: &Interner) -> Sort {
        match ty {
            TypeExpr::Refinement { base, .. } => Sort::of_type(base, interner),
            TypeExpr::Primitive(name) | TypeExpr::Named(name) => match interner.resolve(*name) {
                "Int" | "Nat" => Sort::Int,
                "Bool" | "Boolean" => Sort::Bool,
                _ => Sort::Opaque,
            },
            _ => Sort::Opaque,
        }
    }
}

/// What every body's walk asks of the whole program: the names a call may write, which functions a call computes
/// from its arguments alone, and the signatures such a call is stated with.
struct Program {
    globals: CalleeGlobals,
    /// The program's pure functions: a call to one computes its answer from its arguments alone.
    purity: Purity,
    /// The program functions whose parameters are all `Int` and whose result is `Int` or `Bool`: the calls the
    /// encoder can state as an uninterpreted application of the right sort.
    signatures: HashMap<Symbol, (usize, Sort)>,
}

impl Program {
    fn of(stmts: &[Stmt], interner: &Interner) -> Self {
        let mut signatures = HashMap::new();
        for stmt in stmts {
            if let Stmt::FunctionDef { name, params, return_type: Some(ret), .. } = stmt {
                let ret = Sort::of_type(ret, interner);
                let int_params = params.iter().all(|(_, ty)| Sort::of_type(ty, interner) == Sort::Int);
                if int_params && matches!(ret, Sort::Int | Sort::Bool) {
                    signatures.insert(*name, (params.len(), ret));
                }
            }
        }
        Self {
            globals: CalleeGlobals::of_program(stmts),
            purity: Purity::of_program(stmts, interner),
            signatures,
        }
    }
}

/// Whether `block` can leave its loop early: a `Break` that is not inside a nested loop or a nested definition.
fn breaks_out(block: &[Stmt]) -> bool {
    block.iter().any(|stmt| match stmt {
        Stmt::Break => true,
        Stmt::If { then_block, else_block, .. } => {
            breaks_out(then_block) || else_block.is_some_and(|b| breaks_out(b))
        }
        Stmt::Inspect { arms, .. } => arms.iter().any(|arm| breaks_out(&arm.body)),
        Stmt::Select { branches } => branches.iter().any(|branch| match branch {
            logicaffeine_language::ast::stmt::SelectBranch::Receive { body, .. }
            | logicaffeine_language::ast::stmt::SelectBranch::Timeout { body, .. } => breaks_out(body),
        }),
        Stmt::Zone { body, .. }
        | Stmt::Splice { body }
        | Stmt::Reversibly { body }
        | Stmt::During { body, .. }
        | Stmt::ExpectFail { body, .. } => breaks_out(body),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => breaks_out(tasks),
        _ => false,
    })
}

/// Whether control continues past a statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    /// The next statement runs.
    Continues,
    /// The statement always leaves the block: it returns, fails or breaks.
    Exits,
}

/// What is known at one program point: the solver name each binding's CURRENT value has, the
/// refinement each binding must keep, and the refinement the enclosing function's return value
/// must meet.
#[derive(Clone, Default)]
struct Scope<'s> {
    names: HashMap<Symbol, String>,
    refinements: HashMap<Symbol, (Symbol, &'s LogicExpr<'s>)>,
    returns: Option<(Symbol, Symbol, &'s LogicExpr<'s>)>,
}

/// A binding a declaration shadowed: the solver name and refinement its symbol had before the
/// declaration, put back when the declaring block ends.
struct Shadowed<'s> {
    sym: Symbol,
    name: Option<String>,
    refinement: Option<(Symbol, &'s LogicExpr<'s>)>,
}

/// One walk over a program: the solver session of the body being checked, its scope, and the
/// conditions the statement being visited runs under.
struct Walk<'i, 'p, 's> {
    interner: &'i Interner,
    program: &'p Program,
    session: VerificationSession,
    scope: Scope<'s>,
    /// The conditions under which the statement being visited runs.
    path: Vec<VerifyExpr>,
    /// For each block being visited, the bindings its declarations shadow, restored when the block ends.
    frames: Vec<Vec<Shadowed<'s>>>,
    /// The sort of every solver name minted so far in this session.
    sorts: HashMap<String, Sort>,
    /// Every solver name minted in this session, so no two values ever share one.
    used: HashSet<String>,
    fresh: Cell<u32>,
    /// Names a predicate binds (a refinement's `it`, a quantifier's or a lambda's variable), with
    /// the solver name each stands for while that predicate is mapped.
    bound: RefCell<Vec<(Symbol, String)>>,
}

impl<'i, 'p, 's> Walk<'i, 'p, 's> {
    fn new(interner: &'i Interner, program: &'p Program) -> Self {
        Self {
            interner,
            program,
            session: VerificationSession::new(),
            scope: Scope::default(),
            path: Vec::new(),
            frames: Vec::new(),
            sorts: HashMap::new(),
            used: HashSet::new(),
            fresh: Cell::new(0),
            bound: RefCell::new(Vec::new()),
        }
    }

    fn next(&self) -> u32 {
        let n = self.fresh.get();
        self.fresh.set(n + 1);
        n
    }

    /// A solver name no other value in this session has, declared with `sort`.
    fn mint(&mut self, base: &str, sort: Sort) -> String {
        let name = if self.used.contains(base) {
            loop {
                let candidate = format!("{base}#{}", self.next());
                if !self.used.contains(&candidate) {
                    break candidate;
                }
            }
        } else {
            base.to_string()
        };
        self.used.insert(name.clone());
        self.session.declare(&name, sort.verify_type());
        self.sorts.insert(name.clone(), sort);
        name
    }

    /// Give `sym` a NEW value of `sort`, about which nothing is known yet.
    fn rebind(&mut self, sym: Symbol, sort: Sort) -> String {
        let base = self.interner.resolve(sym).to_string();
        let name = self.mint(&base, sort);
        self.scope.names.insert(sym, name.clone());
        name
    }

    /// Declare `sym` in the innermost block: a new value of `sort` held to `refinement`, and the
    /// binding it shadows restored when the block ends.
    fn declare(&mut self, sym: Symbol, sort: Sort, refinement: Option<(Symbol, &'s LogicExpr<'s>)>) -> String {
        let shadowed = Shadowed {
            sym,
            name: self.scope.names.get(&sym).cloned(),
            refinement: self.scope.refinements.get(&sym).copied(),
        };
        if let Some(frame) = self.frames.last_mut() {
            frame.push(shadowed);
        }
        match refinement {
            Some(refinement) => {
                self.scope.refinements.insert(sym, refinement);
            }
            None => {
                self.scope.refinements.remove(&sym);
            }
        }
        self.rebind(sym, sort)
    }

    /// Open a block's scope.
    fn open_frame(&mut self) {
        self.frames.push(Vec::new());
    }

    /// Close a block's scope: every binding a declaration in it shadowed is the one its name
    /// denotes again, and a name the block introduced is gone.
    fn close_frame(&mut self) {
        let Some(frame) = self.frames.pop() else { return };
        for Shadowed { sym, name, refinement } in frame.into_iter().rev() {
            match name {
                Some(name) => {
                    self.scope.names.insert(sym, name);
                }
                None => {
                    self.scope.names.remove(&sym);
                }
            }
            match refinement {
                Some(refinement) => {
                    self.scope.refinements.insert(sym, refinement);
                }
                None => {
                    self.scope.refinements.remove(&sym);
                }
            }
        }
    }

    /// The sort of `sym`'s current value, `Opaque` when it has none yet.
    fn sort_of(&self, sym: Symbol) -> Sort {
        self.scope
            .names
            .get(&sym)
            .and_then(|n| self.sorts.get(n))
            .copied()
            .unwrap_or(Sort::Opaque)
    }

    /// The solver name of `sym`'s current value; a fresh unknown one when the walk has not met it.
    fn current(&mut self, sym: Symbol) -> String {
        match self.scope.names.get(&sym) {
            Some(name) => name.clone(),
            None => self.rebind(sym, Sort::Opaque),
        }
    }

    /// Forget every name `writes` may change: each one in scope keeps its sort and loses every
    /// fact. Foreign code may write anything, so it forgets every name in scope.
    fn havoc(&mut self, writes: &WriteSet) {
        let written: Vec<Symbol> = if writes.is_opaque() {
            self.scope.names.keys().copied().collect()
        } else {
            writes.symbols().filter(|sym| self.scope.names.contains_key(sym)).collect()
        };
        for sym in written {
            let sort = self.sort_of(sym);
            self.rebind(sym, sort);
        }
    }

    /// Forget whatever evaluating `expr` may write, the globals its calls may write included.
    fn havoc_expr(&mut self, expr: &Expr) {
        let writes = WriteSet::of_expr(expr).with_callee_globals(&self.program.globals);
        self.havoc(&writes);
    }

    /// Forget everything `stmt` may write, the globals its calls may write included.
    fn havoc_stmt(&mut self, stmt: &Stmt) {
        let writes = WriteSet::of_stmt(stmt).with_callee_globals(&self.program.globals);
        self.havoc(&writes);
    }

    /// The conjunction of the conditions the current statement runs under.
    fn path_condition(&self) -> Option<VerifyExpr> {
        self.path.iter().cloned().reduce(VerifyExpr::and)
    }

    /// `e` as it must hold here: under every condition that leads to this point.
    fn guarded(&self, e: VerifyExpr) -> VerifyExpr {
        match self.path_condition() {
            Some(pc) => VerifyExpr::implies(pc, e),
            None => e,
        }
    }

    /// Record that `e` holds on the current path.
    fn assume(&mut self, e: VerifyExpr) {
        let fact = self.guarded(e);
        self.session.assume(&fact);
    }

    /// A branch or loop condition: modelled when the pass can, otherwise a fresh unknown truth
    /// value, so neither arm is ever assumed to be the one that runs.
    fn condition(&mut self, cond: &Expr) -> VerifyExpr {
        match self.map_expr(cond) {
            Some((c, Sort::Bool)) => c,
            _ => self.unknown_condition(),
        }
    }

    /// A fresh truth value the pass knows nothing about: whether a branch it cannot select runs.
    fn unknown_condition(&mut self) -> VerifyExpr {
        VerifyExpr::var(self.mint("__condition", Sort::Bool))
    }

    /// Whether a call is to a pure function of the program or a pure builtin: its answer depends on its arguments
    /// alone, so two calls with equal arguments have equal answers.
    fn call_is_pure(&self, function: Symbol) -> bool {
        self.program.purity.is_pure(function)
            || crate::semantics::builtins::builtin_from_name(self.interner.resolve(function))
                .is_some_and(|b| b.effect() == crate::semantics::builtins::BuiltinEffect::Pure)
    }

    /// A call to a pure function of known signature, as an uninterpreted application of its result's sort; `None`
    /// for any other call (its value is unknown). The symbol is prefixed so it never meets a predicate of a
    /// proposition that happens to share the name.
    fn pure_call(&self, function: Symbol, args: Vec<(VerifyExpr, Sort)>) -> Option<(VerifyExpr, Sort)> {
        if !self.call_is_pure(function) {
            return None;
        }
        let (arity, ret) = *self.program.signatures.get(&function)?;
        if args.len() != arity || args.iter().any(|(_, sort)| *sort != Sort::Int) {
            return None;
        }
        let name = format!("call:{}", self.interner.resolve(function));
        let args = args.into_iter().map(|(arg, _)| arg).collect();
        match ret {
            Sort::Int => Some((VerifyExpr::apply_int(name, args), Sort::Int)),
            Sort::Bool => Some((VerifyExpr::apply(name, args), Sort::Bool)),
            Sort::Opaque => None,
        }
    }

    /// Visit a block's statements in its own scope. A statement that always leaves the block ends
    /// it; the block's declarations and the conditions an early exit adds to the path end with it.
    fn block(&mut self, stmts: &[Stmt<'s>]) -> Result<Flow, String> {
        let depth = self.path.len();
        self.open_frame();
        let flow = self.statements(stmts);
        self.close_frame();
        self.path.truncate(depth);
        flow
    }

    /// Visit `stmts` in order, up to the first that always leaves.
    fn statements(&mut self, stmts: &[Stmt<'s>]) -> Result<Flow, String> {
        for stmt in stmts {
            if self.stmt(stmt)? == Flow::Exits {
                return Ok(Flow::Exits);
            }
        }
        Ok(Flow::Continues)
    }

    /// After a branch whose other arm always leaves, each binding visible before it holds the
    /// value `arm` left it.
    fn take_arm(&mut self, before: &Scope<'s>, arm: &Scope<'s>) {
        for (sym, was) in &before.names {
            let name = arm.names.get(sym).unwrap_or(was);
            self.scope.names.insert(*sym, name.clone());
        }
    }

    /// After the two arms of a branch on `condition`, each binding visible before it holds
    /// whichever arm's value ran.
    fn join(&mut self, condition: &VerifyExpr, before: &Scope<'s>, after_then: &Scope<'s>, after_else: &Scope<'s>) {
        for (sym, was) in &before.names {
            let then_name = after_then.names.get(sym).unwrap_or(was);
            let else_name = after_else.names.get(sym).unwrap_or(was);
            if then_name == was && else_name == was {
                continue;
            }
            if then_name == else_name {
                self.scope.names.insert(*sym, then_name.clone());
                continue;
            }
            let then_sort = self.sorts.get(then_name).copied().unwrap_or(Sort::Opaque);
            let else_sort = self.sorts.get(else_name).copied().unwrap_or(Sort::Opaque);
            let sort = if then_sort == else_sort { then_sort } else { Sort::Opaque };
            let joined = self.rebind(*sym, sort);
            if sort != Sort::Opaque {
                let merged = VerifyExpr::var(joined);
                self.assume(VerifyExpr::and(
                    VerifyExpr::implies(
                        condition.clone(),
                        VerifyExpr::eq(merged.clone(), VerifyExpr::var(then_name.clone())),
                    ),
                    VerifyExpr::implies(
                        VerifyExpr::not(condition.clone()),
                        VerifyExpr::eq(merged, VerifyExpr::var(else_name.clone())),
                    ),
                ));
            }
        }
    }

    fn stmt(&mut self, stmt: &Stmt<'s>) -> Result<Flow, String> {
        match stmt {
            Stmt::Let { var, ty, value, .. } => {
                let mapped = self.map_expr(value);
                let refinement = match ty {
                    Some(TypeExpr::Refinement { var: bound, predicate, .. }) => Some((*bound, *predicate)),
                    _ => None,
                };
                if let Some((bound, predicate)) = refinement {
                    let label = self.interner.resolve(*var).to_string();
                    self.check_refinement(&label, bound, predicate, mapped.clone())?;
                }
                self.havoc_expr(value);
                let sort = match (ty.map(|t| Sort::of_type(t, self.interner)), &mapped) {
                    (Some(declared), _) if declared != Sort::Opaque => declared,
                    (_, Some((_, sort))) => *sort,
                    _ => Sort::Opaque,
                };
                let name = self.declare(*var, sort, refinement);
                if let Some((value, value_sort)) = mapped {
                    if value_sort == sort {
                        self.assume(VerifyExpr::eq(VerifyExpr::var(name), value));
                    }
                }
                Ok(Flow::Continues)
            }

            Stmt::Set { target, value } => {
                let mapped = self.map_expr(value);
                if let Some((bound, predicate)) = self.scope.refinements.get(target).copied() {
                    let label = self.interner.resolve(*target).to_string();
                    self.check_refinement(&label, bound, predicate, mapped.clone())?;
                }
                self.havoc_expr(value);
                let sort = match self.sort_of(*target) {
                    Sort::Opaque => mapped.as_ref().map_or(Sort::Opaque, |(_, s)| *s),
                    known => known,
                };
                let name = self.rebind(*target, sort);
                if let Some((value, value_sort)) = mapped {
                    if value_sort == sort {
                        self.assume(VerifyExpr::eq(VerifyExpr::var(name), value));
                    }
                }
                Ok(Flow::Continues)
            }

            Stmt::Swap { a, b } => {
                let (old_a, old_b) = (self.current(*a), self.current(*b));
                let (sort_a, sort_b) = (self.sort_of(*a), self.sort_of(*b));
                let new_a = self.rebind(*a, sort_a);
                let new_b = self.rebind(*b, sort_b);
                if sort_a == sort_b && sort_a != Sort::Opaque {
                    self.assume(VerifyExpr::eq(VerifyExpr::var(new_a), VerifyExpr::var(old_b)));
                    self.assume(VerifyExpr::eq(VerifyExpr::var(new_b), VerifyExpr::var(old_a)));
                }
                Ok(Flow::Continues)
            }

            Stmt::RuntimeAssert { condition, hard, .. } => {
                let claim = match self.map_expr(condition) {
                    Some((claim, Sort::Bool)) => Some(claim),
                    _ => None,
                };
                if !*hard {
                    let Some(goal) = claim.clone() else {
                        return Err(
                            "`largo verify` cannot check an Assert in this program: its condition uses \
                             something the verifier does not model (a call whose answer it does not know, \
                             text, a field, a list, division or a decimal). The Assert is still checked when \
                             the program runs, but the program is not verified"
                                .to_string(),
                        );
                    };
                    self.session
                        .verify(&self.guarded(goal))
                        .map_err(|e| format!("An Assert does not hold on every run: {e}"))?;
                }
                if let Some(claim) = claim {
                    self.assume(claim);
                }
                self.havoc_expr(condition);
                Ok(Flow::Continues)
            }

            Stmt::Assert { proposition } => {
                let claim = self.map_logic_expr(proposition);
                // A proposition that maps to `true` holds wherever it stands.
                if matches!(&claim, VerifyExpr::Bool(true)) {
                    return Ok(Flow::Continues);
                }
                self.session.verify(&self.guarded(claim.clone())).map_err(|e| format!("{}", e))?;
                self.assume(claim);
                Ok(Flow::Continues)
            }

            Stmt::Trust { proposition, justification } => {
                let claim = self.map_logic_expr(proposition);
                if matches!(&claim, VerifyExpr::Bool(true)) {
                    return Ok(Flow::Continues);
                }
                let reason = self.interner.resolve(*justification);
                self.session.verify(&self.guarded(claim.clone())).map_err(|e| {
                    format!("Trust verification failed (justification: {}): {}", reason, e)
                })?;
                self.assume(claim);
                Ok(Flow::Continues)
            }

            Stmt::If { cond, then_block, else_block } => {
                let condition = self.condition(cond);
                self.havoc_expr(cond);
                let before = self.scope.clone();

                self.path.push(condition.clone());
                let then_flow = self.block(*then_block)?;
                self.path.pop();
                let after_then = std::mem::replace(&mut self.scope, before.clone());

                self.path.push(VerifyExpr::not(condition.clone()));
                let else_flow = match else_block {
                    Some(else_block) => self.block(*else_block)?,
                    None => Flow::Continues,
                };
                self.path.pop();
                let after_else = std::mem::replace(&mut self.scope, before.clone());

                // An arm that always leaves the block (`Return`, `Break`, `Fail`) does not reach the
                // statements after the `If`: those run only when the other arm ran.
                match (then_flow, else_flow) {
                    (Flow::Exits, Flow::Exits) => return Ok(Flow::Exits),
                    (Flow::Exits, Flow::Continues) => {
                        self.take_arm(&before, &after_else);
                        self.path.push(VerifyExpr::not(condition));
                    }
                    (Flow::Continues, Flow::Exits) => {
                        self.take_arm(&before, &after_then);
                        self.path.push(condition);
                    }
                    (Flow::Continues, Flow::Continues) => {
                        self.join(&condition, &before, &after_then, &after_else)
                    }
                }
                Ok(Flow::Continues)
            }

            Stmt::While { cond, body, decreasing } => {
                // The loop head is reached with whatever the previous iterations left behind.
                self.havoc_stmt(stmt);
                let variant = match decreasing {
                    None => None,
                    Some(Expr::Identifier(sym)) => Some(*sym),
                    Some(_) => return Err("Decreasing clause must be a simple variable".to_string()),
                };
                let condition = self.condition(cond);
                self.havoc_expr(cond);
                let before = self.scope.clone();
                let depth = self.path.len();
                self.path.push(condition);
                let start = variant.and_then(|sym| self.scope.names.get(&sym).cloned());
                self.open_frame();
                let flow = self.statements(*body);
                self.close_frame();
                let iteration = match (flow, variant) {
                    (Ok(_), Some(sym)) => self.check_termination(sym, start),
                    (flow, _) => flow.map(|_| ()),
                };
                self.path.truncate(depth);
                iteration?;
                self.scope = before;
                self.havoc_stmt(stmt);
                if !breaks_out(body) {
                    let after = self.condition(cond);
                    self.assume(VerifyExpr::not(after));
                    self.havoc_expr(cond);
                }
                Ok(Flow::Continues)
            }

            Stmt::Repeat { pattern, body, .. } => {
                self.havoc_stmt(stmt);
                let before = self.scope.clone();
                self.open_frame();
                for binder in pattern_symbols(pattern) {
                    self.declare(binder, Sort::Opaque, None);
                }
                let visited = self.block(*body);
                self.close_frame();
                visited?;
                self.scope = before;
                self.havoc_stmt(stmt);
                Ok(Flow::Continues)
            }

            Stmt::Inspect { target, arms, .. } => {
                self.havoc_expr(target);
                let before = self.scope.clone();
                for arm in arms {
                    self.scope = before.clone();
                    let arm_runs = self.unknown_condition();
                    self.path.push(arm_runs);
                    self.open_frame();
                    for (_, binding) in &arm.bindings {
                        self.declare(*binding, Sort::Opaque, None);
                    }
                    let visited = self.block(arm.body);
                    self.close_frame();
                    self.path.pop();
                    visited?;
                }
                self.scope = before;
                self.havoc_stmt(stmt);
                Ok(Flow::Continues)
            }

            Stmt::Zone { body, .. } => self.block(*body),

            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                let writes = WriteSet::of_block(tasks).with_callee_globals(&self.program.globals);
                for task in tasks.iter() {
                    self.havoc(&writes);
                    self.stmt(task)?;
                }
                self.havoc(&writes);
                Ok(Flow::Continues)
            }

            Stmt::FunctionDef { name, params, body, return_type, is_native, .. } => {
                if *is_native {
                    return Ok(Flow::Continues);
                }
                let session = std::mem::replace(&mut self.session, VerificationSession::new());
                let scope = std::mem::take(&mut self.scope);
                let path = std::mem::take(&mut self.path);
                let sorts = std::mem::take(&mut self.sorts);
                let used = std::mem::take(&mut self.used);
                for (param, ty) in params {
                    let sort = Sort::of_type(ty, self.interner);
                    self.rebind(*param, sort);
                }
                if let Some(TypeExpr::Refinement { var: bound, predicate, .. }) = return_type {
                    self.scope.returns = Some((*name, *bound, *predicate));
                }
                let verdict = self.block(*body);
                self.session = session;
                self.scope = scope;
                self.path = path;
                self.sorts = sorts;
                self.used = used;
                verdict.map(|_| Flow::Continues)
            }

            Stmt::Return { value } => {
                if let Some((function, bound, predicate)) = self.scope.returns {
                    let mapped = value.and_then(|v| self.map_expr(v));
                    let label = format!("the return value of {}", self.interner.resolve(function));
                    self.check_refinement(&label, bound, predicate, mapped)?;
                }
                Ok(Flow::Exits)
            }

            Stmt::Fail { .. } | Stmt::Break => Ok(Flow::Exits),

            // A test is not part of the program `largo verify` checks.
            Stmt::TestDef { .. } => Ok(Flow::Continues),

            // Any other statement: refuse a claim it hides, and forget every name it may write.
            _ => {
                if let Some(claim) = self.unchecked_claim_in(stmt) {
                    return Err(format!(
                        "`largo verify` cannot check {claim} inside this statement, because it does \
                         not model the statement around it, so the program is not verified"
                    ));
                }
                self.havoc_stmt(stmt);
                Ok(Flow::Continues)
            }
        }
    }

    /// The first claim nested anywhere inside `stmt` that this pass would have to check, named
    /// the way a refusal names it. A statement the pass does not model may not hide one.
    fn unchecked_claim_in(&self, stmt: &Stmt) -> Option<&'static str> {
        match stmt {
            Stmt::RuntimeAssert { .. } | Stmt::Assert { .. } | Stmt::Trust { .. } => {
                return Some("an Assert")
            }
            Stmt::Let { ty: Some(TypeExpr::Refinement { .. }), .. } => {
                return Some("a refinement type")
            }
            Stmt::Set { target, .. } if self.scope.refinements.contains_key(target) => {
                return Some("a refinement type")
            }
            Stmt::While { decreasing: Some(_), .. } => return Some("a `decreasing` loop"),
            Stmt::Return { .. } if self.scope.returns.is_some() => {
                return Some("a refined return value")
            }
            Stmt::FunctionDef { .. } | Stmt::TestDef { .. } => return None,
            _ => {}
        }
        crate::analysis::tower::nested_blocks(stmt)
            .into_iter()
            .flat_map(|block| block.iter())
            .find_map(|inner| self.unchecked_claim_in(inner))
    }

    /// Check that `value` meets the refinement `predicate` (which binds `bound`) at this point.
    fn check_refinement(
        &mut self,
        label: &str,
        bound: Symbol,
        predicate: &LogicExpr,
        value: Option<(VerifyExpr, Sort)>,
    ) -> Result<(), String> {
        let Some((value, sort)) = value.filter(|(_, sort)| *sort != Sort::Opaque) else {
            return Err(format!(
                "Cannot verify refinement for '{}': its value is not something `largo verify` \
                 models (a call whose answer it does not know, text, a field, a list, division or \
                 a decimal), so the refinement was not checked",
                label
            ));
        };
        let placeholder = format!("{}@{}", self.interner.resolve(bound), self.next());
        self.bound.borrow_mut().push((bound, placeholder.clone()));
        let claim = self.map_logic_expr(predicate);
        self.bound.borrow_mut().pop();
        let goal = self.guarded(claim);
        self.session
            .verify_with_binding(&placeholder, sort.verify_type(), &value, &goal)
            .map_err(|e| format!("Refinement type verification failed for '{}': {}", label, e))
    }

    /// Check that one pass of a loop's body makes its variant `sym` strictly smaller and that the
    /// variant starts it at zero or more, wherever the loop condition admits another iteration:
    /// `start` is the variant's solver name at the top of the body, its current one what the body
    /// leaves it.
    fn check_termination(&self, sym: Symbol, start: Option<String>) -> Result<(), String> {
        let name = self.interner.resolve(sym).to_string();
        let end = self.scope.names.get(&sym).cloned();
        let (Some(start), Some(end)) = (start, end) else {
            return Err(format!("Variant '{}' is not modified in loop body", name));
        };
        if start == end {
            return Err(format!("Variant '{}' is not modified in loop body", name));
        }
        let int = |n: &String| self.sorts.get(n).copied() == Some(Sort::Int);
        if !int(&start) || !int(&end) {
            return Err(format!(
                "Termination verification failed for '{}': the verifier does not \
                 model its value as an integer",
                name
            ));
        }
        let (start, end) = (VerifyExpr::var(start), VerifyExpr::var(end));
        let decreases = VerifyExpr::and(
            VerifyExpr::lt(end, start.clone()),
            VerifyExpr::gte(start, VerifyExpr::int(0)),
        );
        self.session
            .verify(&self.guarded(decreases))
            .map_err(|e| format!("Termination verification failed for '{}': {}", name, e))
    }

    /// The solver name a program name stands for here.
    fn solver_name(&self, sym: Symbol) -> String {
        if let Some((_, name)) = self.bound.borrow().iter().rev().find(|(s, _)| *s == sym) {
            return name.clone();
        }
        match self.scope.names.get(&sym) {
            Some(name) => name.clone(),
            None => self.interner.resolve(sym).to_string(),
        }
    }

    /// Map an imperative expression to the IR with its sort, or `None` when the pass does not
    /// model it. `None` is the honest answer for every construct whose value the solver would
    /// otherwise invent: a call it cannot state (an impure function may read the console, a clock
    /// or a global), text, a field, a list, a decimal, and `/` (LOGOS truncates toward zero, Z3
    /// does not). A call to a pure function is a function of its arguments, the same answer for
    /// the same arguments.
    fn map_expr(&mut self, expr: &Expr) -> Option<(VerifyExpr, Sort)> {
        match expr {
            Expr::Literal(Literal::Number(n)) => Some((VerifyExpr::int(*n), Sort::Int)),
            Expr::Literal(Literal::Boolean(b)) => Some((VerifyExpr::bool(*b), Sort::Bool)),
            Expr::Literal(Literal::Duration(nanos)) => Some((VerifyExpr::int(*nanos), Sort::Int)),
            Expr::Literal(Literal::Date(days)) => Some((VerifyExpr::int(*days as i64), Sort::Int)),
            Expr::Literal(Literal::Moment(nanos)) => Some((VerifyExpr::int(*nanos), Sort::Int)),
            Expr::Literal(_) => None,

            Expr::Identifier(sym) => {
                if !self.scope.names.contains_key(sym) {
                    self.rebind(*sym, Sort::Opaque);
                }
                match self.sort_of(*sym) {
                    Sort::Opaque => None,
                    sort => Some((VerifyExpr::var(self.solver_name(*sym)), sort)),
                }
            }

            Expr::Not { operand } => match self.map_expr(operand)? {
                (e, Sort::Bool) => Some((VerifyExpr::not(e), Sort::Bool)),
                _ => None,
            },

            Expr::BinaryOp { op, left, right } => {
                let (l, ls) = self.map_expr(left)?;
                let (r, rs) = self.map_expr(right)?;
                let (verify_op, operands, result) = match op {
                    BinaryOpKind::Add => (VerifyOp::Add, Sort::Int, Sort::Int),
                    BinaryOpKind::Subtract => (VerifyOp::Sub, Sort::Int, Sort::Int),
                    BinaryOpKind::Multiply => (VerifyOp::Mul, Sort::Int, Sort::Int),
                    // `//` floors toward -inf, modelled EXACTLY in the IR (`to_int(a/b)` over reals).
                    BinaryOpKind::FloorDivide => (VerifyOp::FloorDiv, Sort::Int, Sort::Int),
                    BinaryOpKind::Gt => (VerifyOp::Gt, Sort::Int, Sort::Bool),
                    BinaryOpKind::Lt => (VerifyOp::Lt, Sort::Int, Sort::Bool),
                    BinaryOpKind::GtEq => (VerifyOp::Gte, Sort::Int, Sort::Bool),
                    BinaryOpKind::LtEq => (VerifyOp::Lte, Sort::Int, Sort::Bool),
                    BinaryOpKind::And => (VerifyOp::And, Sort::Bool, Sort::Bool),
                    BinaryOpKind::Or => (VerifyOp::Or, Sort::Bool, Sort::Bool),
                    BinaryOpKind::Eq | BinaryOpKind::NotEq => {
                        if ls != rs {
                            return None;
                        }
                        let verify_op = if matches!(op, BinaryOpKind::Eq) {
                            VerifyOp::Eq
                        } else {
                            VerifyOp::Neq
                        };
                        return Some((VerifyExpr::binary(verify_op, l, r), Sort::Bool));
                    }
                    BinaryOpKind::Divide
                    | BinaryOpKind::ExactDivide
                    | BinaryOpKind::Modulo
                    | BinaryOpKind::Pow
                    | BinaryOpKind::Concat
                    | BinaryOpKind::SeqConcat
                    | BinaryOpKind::ApproxEq
                    | BinaryOpKind::BitXor
                    | BinaryOpKind::BitAnd
                    | BinaryOpKind::BitOr
                    | BinaryOpKind::Shl
                    | BinaryOpKind::Shr => return None,
                };
                (ls == operands && rs == operands)
                    .then(|| (VerifyExpr::binary(verify_op, l, r), result))
            }

            Expr::Call { function, args } => {
                let mut mapped = Vec::with_capacity(args.len());
                for arg in args {
                    mapped.push(self.map_expr(arg)?);
                }
                self.pure_call(*function, mapped)
            }

            _ => None,
        }
    }

    /// A fresh, distinct uninterpreted proposition standing for a construct the
    /// verifier cannot interpret.
    ///
    /// This is the sound stand-in for "we don't know": it is Bool-sorted (an
    /// uninterpreted predicate application), and because each call is a *fresh*
    /// symbol it is never entailed by anything. As an assertion *goal* it
    /// therefore fails verification (we must not claim to have proved what we
    /// cannot interpret); as an *assumption* it is a harmless free proposition.
    /// Crucially it is **not** `Bool(true)`, so it can never be vacuously
    /// discharged.
    fn unverifiable(&self) -> VerifyExpr {
        let n = self.next();
        VerifyExpr::apply(
            "__Unverifiable",
            vec![VerifyExpr::var(format!("__unverifiable_{}", n))],
        )
    }

    /// Map a logic expression to Verification IR.
    ///
    /// This is the core of the "Smart Full Mapping" strategy:
    /// - Simple types (Int, Bool) map directly
    /// - Complex types (Predicates, Modals) become uninterpreted functions
    fn map_logic_expr(&self, expr: &LogicExpr) -> VerifyExpr {
        match expr {
            // Atoms are boolean variables or 0-arity predicates
            LogicExpr::Atom(sym) => VerifyExpr::var(self.solver_name(*sym)),

            LogicExpr::Predicate { name, args, .. } => {
                let pred_name = self.interner.resolve(*name);
                let verify_args: Vec<VerifyExpr> = args.iter().map(|t| self.map_term(t)).collect();

                // Phase 43D: Handle comparison predicates from refinement types
                // The parser creates predicates like "Greater(it, 0)" for "it > 0"
                if verify_args.len() == 2 {
                    let left = verify_args[0].clone();
                    let right = verify_args[1].clone();
                    match pred_name {
                        "Greater" => return VerifyExpr::gt(left, right),
                        "Less" => return VerifyExpr::lt(left, right),
                        "GreaterEqual" => return VerifyExpr::gte(left, right),
                        "LessEqual" => return VerifyExpr::lte(left, right),
                        "Equal" => return VerifyExpr::eq(left, right),
                        "NotEqual" => return VerifyExpr::neq(left, right),
                        _ => {}
                    }
                }

                // Default: treat as uninterpreted function
                VerifyExpr::apply(pred_name, verify_args)
            }

            LogicExpr::Identity { left, right } => {
                let l = self.map_term(left);
                let r = self.map_term(right);
                VerifyExpr::eq(l, r)
            }

            LogicExpr::BinaryOp { left, op, right } => {
                let l = self.map_logic_expr(left);
                let r = self.map_logic_expr(right);
                let verify_op = match op {
                    TokenType::And => VerifyOp::And,
                    TokenType::Or => VerifyOp::Or,
                    TokenType::If | TokenType::Implies | TokenType::Then => VerifyOp::Implies,
                    TokenType::Iff => VerifyOp::Eq, // Biconditional is boolean equality
                    _ => VerifyOp::And,             // Fallback
                };
                VerifyExpr::binary(verify_op, l, r)
            }

            LogicExpr::UnaryOp { op, operand } => match op {
                TokenType::Not => VerifyExpr::not(self.map_logic_expr(operand)),
                _ => self.map_logic_expr(operand),
            },

            // Smart Mapping: Modal operators become uninterpreted functions
            LogicExpr::Modal { vector, operand } => {
                let op_name = match vector.domain {
                    ModalDomain::Alethic => {
                        if vector.force > 0.5 {
                            "Necessarily"
                        } else {
                            "Possibly"
                        }
                    }
                    ModalDomain::Deontic => {
                        if vector.force > 0.5 {
                            "Obligatory"
                        } else {
                            "Permissible"
                        }
                    }
                    ModalDomain::Temporal => {
                        if vector.force > 0.5 {
                            "Always"
                        } else {
                            "Eventually"
                        }
                    }
                };
                VerifyExpr::apply(op_name, vec![self.map_logic_expr(operand)])
            }

            // Smart Mapping: Temporal operators become uninterpreted functions
            LogicExpr::Temporal { operator, body } => {
                let op_name = match operator {
                    TemporalOperator::Past => "Past",
                    TemporalOperator::Future => "Future",
                    TemporalOperator::Always => "Always",
                    TemporalOperator::Eventually | TemporalOperator::BoundedEventually(_) => {
                        "Eventually"
                    }
                    TemporalOperator::Next => "Next",
                };
                VerifyExpr::apply(op_name, vec![self.map_logic_expr(body)])
            }

            // Binary temporal operators (Until, Release, WeakUntil)
            LogicExpr::TemporalBinary {
                operator,
                left,
                right,
            } => {
                let op_name = match operator {
                    logicaffeine_language::ast::logic::BinaryTemporalOp::Until => "Until",
                    logicaffeine_language::ast::logic::BinaryTemporalOp::Release => "Release",
                    logicaffeine_language::ast::logic::BinaryTemporalOp::WeakUntil => "WeakUntil",
                };
                VerifyExpr::apply(
                    op_name,
                    vec![self.map_logic_expr(left), self.map_logic_expr(right)],
                )
            }

            // Smart Mapping: Aspectual operators become uninterpreted functions
            LogicExpr::Aspectual { operator, body } => {
                let op_name = match operator {
                    AspectOperator::Progressive => "Progressive",
                    AspectOperator::Perfect => "Perfect",
                    AspectOperator::Habitual => "Habitual",
                    AspectOperator::Iterative => "Iterative",
                };
                VerifyExpr::apply(op_name, vec![self.map_logic_expr(body)])
            }

            // Quantifiers map to IR quantifiers
            LogicExpr::Quantifier {
                kind,
                variable,
                body,
                ..
            } => {
                let var_name = self.interner.resolve(*variable);
                self.bound.borrow_mut().push((*variable, var_name.to_string()));
                let body_ir = self.map_logic_expr(body);
                self.bound.borrow_mut().pop();
                match kind {
                    QuantifierKind::Universal => VerifyExpr::forall(
                        vec![(var_name.to_string(), VerifyType::Object)],
                        body_ir,
                    ),
                    QuantifierKind::Existential => VerifyExpr::exists(
                        vec![(var_name.to_string(), VerifyType::Object)],
                        body_ir,
                    ),
                    // Generalized quantifiers become uninterpreted
                    QuantifierKind::Most => {
                        VerifyExpr::apply("Most", vec![VerifyExpr::var(var_name), body_ir])
                    }
                    QuantifierKind::Few => {
                        VerifyExpr::apply("Few", vec![VerifyExpr::var(var_name), body_ir])
                    }
                    QuantifierKind::Many => {
                        VerifyExpr::apply("Many", vec![VerifyExpr::var(var_name), body_ir])
                    }
                    QuantifierKind::Cardinal(n) => VerifyExpr::apply(
                        &format!("Exactly{}", n),
                        vec![VerifyExpr::var(var_name), body_ir],
                    ),
                    QuantifierKind::AtLeast(n) => VerifyExpr::apply(
                        &format!("AtLeast{}", n),
                        vec![VerifyExpr::var(var_name), body_ir],
                    ),
                    QuantifierKind::AtMost(n) => VerifyExpr::apply(
                        &format!("AtMost{}", n),
                        vec![VerifyExpr::var(var_name), body_ir],
                    ),
                    QuantifierKind::Generic => {
                        VerifyExpr::apply("Generic", vec![VerifyExpr::var(var_name), body_ir])
                    }
                    QuantifierKind::InfinitelyMany => VerifyExpr::apply(
                        "InfinitelyMany",
                        vec![VerifyExpr::var(var_name), body_ir],
                    ),
                    QuantifierKind::FinitelyMany => {
                        VerifyExpr::apply("FinitelyMany", vec![VerifyExpr::var(var_name), body_ir])
                    }
                    QuantifierKind::AllButFinitelyMany => VerifyExpr::apply(
                        "AllButFinitelyMany",
                        vec![VerifyExpr::var(var_name), body_ir],
                    ),
                }
            }

            // Lambda abstractions become uninterpreted
            LogicExpr::Lambda { variable, body } => {
                let var_name = self.interner.resolve(*variable);
                self.bound.borrow_mut().push((*variable, var_name.to_string()));
                let body_ir = self.map_logic_expr(body);
                self.bound.borrow_mut().pop();
                VerifyExpr::apply("Lambda", vec![VerifyExpr::var(var_name), body_ir])
            }

            // Function application
            LogicExpr::App { function, argument } => VerifyExpr::apply(
                "App",
                vec![self.map_logic_expr(function), self.map_logic_expr(argument)],
            ),

            // Counterfactuals: if-then with special modal semantics
            LogicExpr::Counterfactual {
                antecedent,
                consequent,
            } => VerifyExpr::apply(
                "Counterfactual",
                vec![
                    self.map_logic_expr(antecedent),
                    self.map_logic_expr(consequent),
                ],
            ),

            // Causation
            LogicExpr::Causal { cause, effect } => VerifyExpr::apply(
                "Causes",
                vec![self.map_logic_expr(cause), self.map_logic_expr(effect)],
            ),

            LogicExpr::Concessive { main, concession } => {
                // main ∧ Concessive(concession): the main clause is asserted.
                VerifyExpr::and(
                    self.map_logic_expr(main),
                    VerifyExpr::apply("Concessive", vec![self.map_logic_expr(concession)]),
                )
            }

            LogicExpr::Implicature { assertion, .. } => {
                // Only the literal assertion is truth-conditional; the implicature is
                // defeasible and excluded from the verification core.
                self.map_logic_expr(assertion)
            }

            // Questions become uninterpreted (for query semantics)
            LogicExpr::Question { wh_variable, body } => {
                let var_name = self.interner.resolve(*wh_variable);
                VerifyExpr::apply(
                    "Question",
                    vec![VerifyExpr::var(var_name), self.map_logic_expr(body)],
                )
            }

            LogicExpr::YesNoQuestion { body } => {
                VerifyExpr::apply("YesNo", vec![self.map_logic_expr(body)])
            }

            // Intensional contexts
            LogicExpr::Intensional { operator, content } => {
                let op_name = self.interner.resolve(*operator);
                VerifyExpr::apply(op_name, vec![self.map_logic_expr(content)])
            }

            // Speech acts
            LogicExpr::SpeechAct {
                performer,
                act_type,
                content,
            } => {
                let performer_name = self.interner.resolve(*performer);
                let act_name = self.interner.resolve(*act_type);
                VerifyExpr::apply(
                    act_name,
                    vec![
                        VerifyExpr::var(performer_name),
                        self.map_logic_expr(content),
                    ],
                )
            }

            // Comparatives
            LogicExpr::Comparative {
                adjective,
                subject,
                object,
                difference,
                ..
            } => {
                let adj_name = self.interner.resolve(*adjective);
                let mut args = vec![self.map_term(subject), self.map_term(object)];
                if let Some(diff) = difference {
                    args.push(self.map_term(diff));
                }
                VerifyExpr::apply(&format!("More{}", adj_name), args)
            }

            // Superlatives
            LogicExpr::Superlative {
                adjective,
                subject,
                domain,
            } => {
                let adj_name = self.interner.resolve(*adjective);
                let domain_name = self.interner.resolve(*domain);
                VerifyExpr::apply(
                    &format!("Most{}", adj_name),
                    vec![self.map_term(subject), VerifyExpr::var(domain_name)],
                )
            }

            // Focus
            LogicExpr::Focus { focused, scope, .. } => VerifyExpr::apply(
                "Focus",
                vec![self.map_term(focused), self.map_logic_expr(scope)],
            ),

            // Presupposition
            LogicExpr::Presupposition {
                assertion,
                presupposition,
            } => {
                // Verify both assertion and presupposition
                VerifyExpr::and(
                    self.map_logic_expr(presupposition),
                    self.map_logic_expr(assertion),
                )
            }

            // Constructs the program-refinement verifier cannot interpret.
            // Mapping these to `true` would *vacuously discharge* any assertion
            // or refinement that touches them — a soundness hole (we'd claim to
            // have verified what we never modelled). Instead we mint a fresh
            // uninterpreted proposition: unprovable as a goal (so such
            // assertions honestly fail), harmless as an assumption. The
            // LINGUISTIC reasoning path is separate: events, Distributive, and
            // GroupQuantifier lower to real first-order forms in
            // `proof_convert` and reason via the oracle's Link-lattice axioms.
            // An optative wish quantifies over bouletically ideal worlds: its
            // complement is NEVER entailed as fact, yet an identical wish entails
            // itself. Encode it as a DETERMINISTIC uninterpreted predicate of the
            // (mapped) wish content — structurally stable so `Wish ⊢ Wish`, opaque
            // so `Wish ⊬ content`. (The fresh-symbol `unverifiable()` would break
            // the self-entailment, minting a distinct symbol per occurrence.)
            LogicExpr::Optative { wish } => {
                VerifyExpr::apply("__Optative", vec![self.map_logic_expr(wish)])
            }

            LogicExpr::Metaphor { .. }
            | LogicExpr::Categorical(_)
            | LogicExpr::Relation(_)
            | LogicExpr::Voice { .. }
            | LogicExpr::Event { .. }
            | LogicExpr::NeoEvent(_)
            | LogicExpr::Imperative { .. }
            | LogicExpr::Exclamative { .. }
            | LogicExpr::TemporalAnchor { .. }
            | LogicExpr::Distributive { .. }
            | LogicExpr::GroupQuantifier { .. }
            | LogicExpr::Scopal { .. }
            | LogicExpr::Control { .. } => self.unverifiable(),
        }
    }

    /// Map a term to Verification IR.
    fn map_term(&self, term: &Term) -> VerifyExpr {
        match term {
            Term::Constant(sym) | Term::Variable(sym) => VerifyExpr::var(self.solver_name(*sym)),

            Term::Value { kind, .. } => {
                match kind {
                    NumberKind::Integer(n) => VerifyExpr::int(*n),
                    NumberKind::Real(r) => VerifyExpr::int(*r as i64), // Truncate for now
                    NumberKind::Symbolic(s) => {
                        let name = self.interner.resolve(*s);
                        VerifyExpr::var(name)
                    }
                }
            }

            Term::Function(name, args) => {
                let func_name = self.interner.resolve(*name);
                let verify_args: Vec<VerifyExpr> = args.iter().map(|t| self.map_term(t)).collect();
                VerifyExpr::apply(func_name, verify_args)
            }

            Term::Group(terms) => {
                // Group terms become a special "Group" function
                let verify_args: Vec<VerifyExpr> = terms.iter().map(|t| self.map_term(t)).collect();
                VerifyExpr::apply("Group", verify_args)
            }

            Term::Possessed {
                possessor,
                possessed,
            } => {
                let poss_name = self.interner.resolve(*possessed);
                VerifyExpr::apply(&format!("{}Of", poss_name), vec![self.map_term(possessor)])
            }

            Term::Sigma(sym) => {
                let name = self.interner.resolve(*sym);
                VerifyExpr::apply("Sigma", vec![VerifyExpr::var(name)])
            }

            Term::Intension(sym) => {
                let name = self.interner.resolve(*sym);
                VerifyExpr::apply("Intension", vec![VerifyExpr::var(name)])
            }

            Term::Kind(sym) => {
                // Kind terms are reified entities: a 0-ary kind constant the
                // solver treats as a fixed individual (e.g. ^Tooth).
                let name = self.interner.resolve(*sym);
                VerifyExpr::apply("Kind", vec![VerifyExpr::var(name)])
            }

            Term::Proposition(expr) => self.map_logic_expr(expr),
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/verification/tests.rs"]
mod tests;
