//! Native ownership analysis for use-after-move detection.
//!
//! Lightweight data-flow analysis that catches use-after-move at check-time
//! (milliseconds), before Rust compilation. This pass tracks `Owned`, `Moved`,
//! and `Borrowed` states through control flow.
//!
//! Collections have value semantics in every tier: a call's plain argument and
//! `Let b be a` each receive a copy, and the caller's value stays usable. Only
//! GIVING moves — the `Give x to y.` statement, and `give x` as a call argument
//! (`Call consume with give data.`), which the backend emits as a real move.
//!
//! # State Transitions
//!
//! ```text
//!          Let x be value
//!               │
//!               ▼
//!           [Owned] ── f(x) · Let y be x ──▶ [Owned]
//!          /        \
//!    Give x       Show x
//!  … with give x      │
//!        │            ▼
//!        ▼       [Borrowed]
//!    [Moved]          │
//!        │        use x? ✓
//!    use x?
//!        │
//!     ERROR: use-after-move
//! ```
//!
//! # Control Flow Awareness
//!
//! The checker handles branches and loops by merging states:
//! - `Moved` on one path + `Owned` on another = `MaybeMoved`, remembering whether a branch or a
//!   loop made the move uncertain
//! - A loop may run no pass at all, so its body's moves merge with the state before it
//! - Using a `MaybeMoved` variable produces an error, wherever it is read: an `If` or `While`
//!   condition, the collection a `Repeat` walks and the value an `Inspect` examines included
//!
//! # The Program as Written
//!
//! Ownership is a property of the program its author wrote, so every entry point runs this
//! checker on the freshly-parsed statements, before any rewrite. Once `x` is a known constant,
//! the optimizer's propagation turns `Give x to p.` into `Give 5 to p.` and `Show x.` into
//! `Show 5.`, and folding flattens a constant `If` into its body: a checker fed that program
//! would pass a use after a possible move, or call a possible move certain.
//!
//! # Example
//!
//! ```text
//! Let x be 5.
//! Give x to y.
//! Show x to show.  ← Error: Cannot use 'x' after giving it away
//! ```

use crate::ast::stmt::{Expr, Stmt, StmtId};
use crate::intern::{Interner, Symbol};
use crate::token::Span;
use std::collections::HashMap;

/// Ownership state for a variable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarState {
    /// Variable is owned and can be used
    Owned,
    /// Variable has been moved (Give)
    Moved,
    /// Variable might be moved: some path through a branch or a loop gave it away
    MaybeMoved(MaybeGivenIn),
    /// Variable is borrowed (Show) - still usable
    Borrowed,
}

/// Where a value may have been given away: the construct whose paths disagree about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaybeGivenIn {
    /// One branch of an `If`, or one arm of an `Inspect`, gives it away and another does not.
    Branch,
    /// A loop's body gives it away, and the loop may run no pass at all.
    Loop,
}

impl std::fmt::Display for MaybeGivenIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MaybeGivenIn::Branch => "a conditional branch",
            MaybeGivenIn::Loop => "a pass through the loop",
        })
    }
}

/// Error type for ownership violations
#[derive(Debug, Clone)]
pub struct OwnershipError {
    pub kind: OwnershipErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum OwnershipErrorKind {
    /// Use after move
    UseAfterMove { variable: String },
    /// Use after potential move (in a branch or a loop)
    UseAfterMaybeMove {
        variable: String,
        given_in: MaybeGivenIn,
    },
    /// Double move
    DoubleMoved { variable: String },
}

impl std::fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            OwnershipErrorKind::UseAfterMove { variable } => {
                write!(
                    f,
                    "Cannot use '{}' after giving it away.\n\n\
                    You transferred ownership of '{}' with Give.\n\
                    Once given, you cannot use it anymore.\n\n\
                    Tip: Use Show instead to lend without giving up ownership.",
                    variable, variable
                )
            }
            OwnershipErrorKind::UseAfterMaybeMove { variable, given_in } => {
                write!(
                    f,
                    "Cannot use '{variable}' - it might have been given away in {given_in}.\n\n"
                )?;
                match given_in {
                    MaybeGivenIn::Branch => write!(f,
                        "If that branch runs, '{variable}' is moved, so using it afterward is not safe.\n\n\
                        Tip: Move the usage inside the branch, or restructure to ensure ownership."),
                    MaybeGivenIn::Loop => write!(f,
                        "If that pass runs, '{variable}' is moved, so using it afterward is not safe.\n\n\
                        Tip: Give a copy on each pass ('Give a copy of {variable} to ...'), or bind a \
                        fresh value inside the loop."),
                }
            }
            OwnershipErrorKind::DoubleMoved { variable } => {
                write!(
                    f,
                    "Cannot give '{}' twice.\n\n\
                    You already transferred ownership of '{}' with Give.\n\
                    You cannot give it again.\n\n\
                    Tip: Consider using Copy to duplicate the value.",
                    variable, variable
                )
            }
        }
    }
}

impl std::error::Error for OwnershipError {}

/// Ownership checker - tracks variable states through control flow
pub struct OwnershipChecker<'a> {
    /// Maps variable symbols to their current ownership state
    state: HashMap<Symbol, VarState>,
    /// String interner for resolving symbols
    interner: &'a Interner,
    /// Which top-level statement moved each variable, and the statement that
    /// did the moving — the cause link that
    /// [`OwnershipChecker::check_program_collect`] reports for diagnostics.
    moved_at: HashMap<Symbol, (usize, StmtId)>,
    /// The top-level statement currently being checked (collect mode).
    current_top_stmt: usize,
    /// The innermost statement currently being checked.
    current_stmt: Option<StmtId>,
}

/// One ownership finding with its statement coordinates: the top-level
/// statement that ERRED, and (when known) the one that CAUSED it by moving
/// the variable — both map 1:1 onto `Parser::stmt_spans()` — plus the exact
/// statements, nested ones included, for `Parser::nested_stmt_spans()`:
/// `trail` is the erring statement and each one around it, innermost first,
/// and `cause` is the statement that gave the value away.
#[derive(Debug)]
pub struct OwnershipFinding {
    pub stmt_index: usize,
    pub cause_stmt_index: Option<usize>,
    pub trail: Vec<StmtId>,
    pub cause: Option<StmtId>,
    pub error: OwnershipError,
}

/// An ownership error on its way out of the statements it was found in,
/// naming each one as it passes, innermost first.
struct Traced {
    error: OwnershipError,
    trail: Vec<StmtId>,
}

impl From<OwnershipError> for Traced {
    fn from(error: OwnershipError) -> Self {
        Traced {
            error,
            trail: Vec::new(),
        }
    }
}

impl<'a> OwnershipChecker<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Self {
            state: HashMap::new(),
            interner,
            moved_at: HashMap::new(),
            current_top_stmt: 0,
            current_stmt: None,
        }
    }

    /// Record a move, remembering which statement performed it.
    fn mark_moved(&mut self, sym: Symbol) {
        self.state.insert(sym, VarState::Moved);
        if let Some(stmt) = self.current_stmt {
            self.moved_at.insert(sym, (self.current_top_stmt, stmt));
        }
    }

    /// The interned symbol behind an error's variable name, if still tracked.
    fn symbol_for(&self, error: &OwnershipError) -> Option<Symbol> {
        let name = match &error.kind {
            OwnershipErrorKind::UseAfterMove { variable }
            | OwnershipErrorKind::UseAfterMaybeMove { variable, .. }
            | OwnershipErrorKind::DoubleMoved { variable } => variable,
        };
        self.state
            .keys()
            .copied()
            .find(|s| self.interner.resolve(*s) == name)
    }

    /// Collect EVERY ownership finding instead of bailing at the first.
    ///
    /// After each finding the offending variable resets to `Owned`, so one
    /// use-after-move does not cascade into errors on every later use.
    /// [`OwnershipChecker::check_program`] keeps the strict fail-fast
    /// contract for compile paths.
    pub fn check_program_collect(&mut self, stmts: &[Stmt<'_>]) -> Vec<OwnershipFinding> {
        let mut findings = Vec::new();
        for (index, stmt) in stmts.iter().enumerate() {
            self.current_top_stmt = index;
            if let Err(Traced { error, trail }) = self.check_stmt(stmt) {
                let sym = self.symbol_for(&error);
                let cause = sym.and_then(|s| self.moved_at.get(&s).copied());
                if let Some(s) = sym {
                    self.state.insert(s, VarState::Owned);
                }
                findings.push(OwnershipFinding {
                    stmt_index: index,
                    cause_stmt_index: cause.map(|(top, _)| top),
                    trail,
                    cause: cause.map(|(_, stmt)| stmt),
                    error,
                });
            }
        }
        findings
    }

    /// Access the current variable ownership states.
    pub fn var_states(&self) -> &HashMap<Symbol, VarState> {
        &self.state
    }

    /// Mark every value GIVEN inside an expression as Moved: `give x` as a call
    /// argument hands `x` over (the backend emits a move, not a copy). A plain
    /// argument is a copy under value semantics and moves nothing.
    fn mark_gives_in_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Give { value } => {
                if let Expr::Identifier(sym) = value {
                    self.mark_moved(*sym);
                }
                self.mark_gives_in_expr(value);
            }
            Expr::Call { args, .. } | Expr::CallExpr { args, .. } => {
                for arg in args.iter() {
                    self.mark_gives_in_expr(arg);
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.mark_gives_in_expr(left);
                self.mark_gives_in_expr(right);
            }
            Expr::Index { collection, index } => {
                self.mark_gives_in_expr(collection);
                self.mark_gives_in_expr(index);
            }
            Expr::FieldAccess { object, .. } => {
                self.mark_gives_in_expr(object);
            }
            _ => {}
        }
    }

    /// Check a program for ownership violations
    pub fn check_program(&mut self, stmts: &[Stmt<'_>]) -> Result<(), OwnershipError> {
        self.check_block(stmts).map_err(|traced| traced.error)
    }

    fn check_block(&mut self, stmts: &[Stmt<'_>]) -> Result<(), Traced> {
        for stmt in stmts {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt<'_>) -> Result<(), Traced> {
        let id = StmtId::of(stmt);
        let enclosing = self.current_stmt.replace(id);
        let result = self.check_stmt_kind(stmt);
        self.current_stmt = enclosing;
        result.map_err(|mut traced| {
            traced.trail.push(id);
            traced
        })
    }

    fn check_stmt_kind(&mut self, stmt: &Stmt<'_>) -> Result<(), Traced> {
        match stmt {
            Stmt::Let { var, value, .. } => {
                self.check_not_moved(value)?;
                self.mark_gives_in_expr(value);
                self.state.insert(*var, VarState::Owned);
            }

            Stmt::Give { object, .. } => {
                // Check if object is already moved
                if let Expr::Identifier(sym) = object {
                    let current = self.state.get(sym).copied().unwrap_or(VarState::Owned);
                    match current {
                        VarState::Moved => {
                            return Err(OwnershipError {
                                kind: OwnershipErrorKind::DoubleMoved {
                                    variable: self.interner.resolve(*sym).to_string(),
                                },
                                span: Span::default(),
                            }
                            .into());
                        }
                        VarState::MaybeMoved(given_in) => {
                            return Err(OwnershipError {
                                kind: OwnershipErrorKind::UseAfterMaybeMove {
                                    variable: self.interner.resolve(*sym).to_string(),
                                    given_in,
                                },
                                span: Span::default(),
                            }
                            .into());
                        }
                        _ => {
                            self.mark_moved(*sym);
                        }
                    }
                } else {
                    // For complex expressions, just check they're not moved
                    self.check_not_moved(object)?;
                }
            }

            Stmt::Show { object, .. } => {
                // Check if object is moved before borrowing
                self.check_not_moved(object)?;
                // Mark as borrowed (still usable)
                if let Expr::Identifier(sym) = object {
                    let current = self.state.get(sym).copied();
                    if current == Some(VarState::Owned) || current.is_none() {
                        self.state.insert(*sym, VarState::Borrowed);
                    }
                }
            }

            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                self.check_not_moved(cond)?;
                let state_before = self.state.clone();

                self.check_block(then_block)?;
                let state_after_then = std::mem::replace(&mut self.state, state_before);

                if let Some(else_b) = else_block {
                    self.check_block(else_b)?;
                }

                // Merge states: MaybeMoved if moved in any branch
                self.state =
                    Self::merge_states(&state_after_then, &self.state, MaybeGivenIn::Branch);
            }

            Stmt::While { cond, body, .. } => {
                self.check_not_moved(cond)?;
                self.check_loop_body(body)?;
            }

            Stmt::Repeat { iterable, body, .. } => {
                self.check_not_moved(iterable)?;
                self.check_loop_body(body)?;
            }

            Stmt::Zone { body, .. } => {
                self.check_block(body)?;
            }

            // A `During` span is scope-transparent: its body runs once, in place.
            Stmt::During { body, .. } => {
                self.check_block(body)?;
            }

            Stmt::Inspect { target, arms, .. } => {
                self.check_not_moved(target)?;
                if arms.is_empty() {
                    return Ok(());
                }

                // Clone state before branches
                let state_before = self.state.clone();
                let mut branch_states = Vec::new();

                for arm in arms {
                    self.state = state_before.clone();
                    self.check_block(arm.body)?;
                    branch_states.push(self.state.clone());
                }

                // Merge all branch states
                if let Some(first) = branch_states.first() {
                    let mut merged = first.clone();
                    for state in branch_states.iter().skip(1) {
                        merged = Self::merge_states(&merged, state, MaybeGivenIn::Branch);
                    }
                    self.state = merged;
                }
            }

            Stmt::Return { value: Some(expr) } => {
                self.check_not_moved(expr)?;
                self.mark_gives_in_expr(expr);
            }

            Stmt::Return { value: None } => {}

            Stmt::Set { value, .. } => {
                self.check_not_moved(value)?;
                self.mark_gives_in_expr(value);
            }

            Stmt::Call { args, .. } => {
                for arg in args.iter() {
                    self.check_not_moved(arg)?;
                }
                for arg in args.iter() {
                    self.mark_gives_in_expr(arg);
                }
            }

            Stmt::FunctionDef { params, body, .. } => {
                // Save state — function body is a separate scope
                let saved_state = self.state.clone();
                for (param_sym, _) in params.iter() {
                    self.state.insert(*param_sym, VarState::Owned);
                }
                self.check_block(body)?;
                self.state = saved_state;
            }

            // Escape blocks are opaque to ownership analysis — the Rust compiler
            // catches use-after-move in the generated code
            Stmt::Escape { .. } => {}

            // Other statements don't affect ownership
            _ => {}
        }
        Ok(())
    }

    /// Check that an expression doesn't reference a moved variable
    fn check_not_moved(&self, expr: &Expr<'_>) -> Result<(), OwnershipError> {
        match expr {
            // A view's holes read where it renders; handler blocks are closures (checked when called).
            Expr::Dom(q) => match q.selector() {
                Some(sel) => self.check_not_moved(sel),
                None => Ok(()),
            },
            Expr::View(v) => {
                for e in v.exprs_and_blocks().0 {
                    self.check_not_moved(e)?;
                }
                Ok(())
            }
            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let crate::ast::stmt::StringPart::Expr { value, .. } = part {
                        self.check_not_moved(value)?;
                    }
                }
                Ok(())
            }
            Expr::Identifier(sym) => match self.state.get(sym).copied() {
                Some(VarState::Moved) => Err(OwnershipError {
                    kind: OwnershipErrorKind::UseAfterMove {
                        variable: self.interner.resolve(*sym).to_string(),
                    },
                    span: Span::default(),
                }),
                Some(VarState::MaybeMoved(given_in)) => Err(OwnershipError {
                    kind: OwnershipErrorKind::UseAfterMaybeMove {
                        variable: self.interner.resolve(*sym).to_string(),
                        given_in,
                    },
                    span: Span::default(),
                }),
                _ => Ok(()),
            },
            Expr::BinaryOp { left, right, .. } => {
                self.check_not_moved(left)?;
                self.check_not_moved(right)?;
                Ok(())
            }
            Expr::FieldAccess { object, .. } => self.check_not_moved(object),
            Expr::Index { collection, index } => {
                self.check_not_moved(collection)?;
                self.check_not_moved(index)?;
                Ok(())
            }
            Expr::Slice {
                collection,
                start,
                end,
            } => {
                self.check_not_moved(collection)?;
                self.check_not_moved(start)?;
                self.check_not_moved(end)?;
                Ok(())
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    self.check_not_moved(arg)?;
                }
                Ok(())
            }
            Expr::List(items) | Expr::Tuple(items) => {
                for item in items {
                    self.check_not_moved(item)?;
                }
                Ok(())
            }
            Expr::Range { start, end } => {
                self.check_not_moved(start)?;
                self.check_not_moved(end)?;
                Ok(())
            }
            Expr::New { init_fields, .. } => {
                for (_, field_expr) in init_fields {
                    self.check_not_moved(field_expr)?;
                }
                Ok(())
            }
            Expr::NewVariant { fields, .. } => {
                for (_, field_expr) in fields {
                    self.check_not_moved(field_expr)?;
                }
                Ok(())
            }
            Expr::Copy { expr }
            | Expr::Give { value: expr }
            | Expr::Length { collection: expr }
            | Expr::Not { operand: expr } => self.check_not_moved(expr),
            Expr::ManifestOf { zone } => self.check_not_moved(zone),
            Expr::ChunkAt { index, zone } => {
                self.check_not_moved(index)?;
                self.check_not_moved(zone)
            }
            Expr::Contains { collection, value } => {
                self.check_not_moved(collection)?;
                self.check_not_moved(value)
            }
            Expr::Union { left, right } | Expr::Intersection { left, right } => {
                self.check_not_moved(left)?;
                self.check_not_moved(right)
            }
            Expr::WithCapacity { value, capacity } => {
                self.check_not_moved(value)?;
                self.check_not_moved(capacity)
            }
            Expr::OptionSome { value } => self.check_not_moved(value),
            Expr::OptionNone => Ok(()),

            // Escape expressions are opaque — the Rust compiler handles ownership for raw code
            Expr::Escape { .. } => Ok(()),

            // Closures capture by cloning — no ownership transfer at creation time.
            // We only check the expression body for moved variables; block bodies
            // create their own scope so ownership is handled there.
            Expr::Closure { body, .. } => match body {
                crate::ast::stmt::ClosureBody::Expression(expr) => self.check_not_moved(expr),
                crate::ast::stmt::ClosureBody::Block(_) => Ok(()),
            },

            Expr::CallExpr { callee, args } => {
                self.check_not_moved(callee)?;
                for arg in args {
                    self.check_not_moved(arg)?;
                }
                Ok(())
            }

            // Literals are always safe
            Expr::Literal(_) => Ok(()),
        }
    }

    /// A loop may run no pass at all, so what its body gives away may still be there after it:
    /// the state after the loop is the state before it merged with the state one pass leaves.
    fn check_loop_body(&mut self, body: &[Stmt<'_>]) -> Result<(), Traced> {
        let state_before = self.state.clone();
        self.check_block(body)?;
        self.state = Self::merge_states(&state_before, &self.state, MaybeGivenIn::Loop);
        Ok(())
    }

    /// Merge the states two paths reach where they meet — if moved on one path but not the
    /// other, the value is MaybeMoved, given away in the construct `given_in` names. A variable
    /// only one path declares keeps that path's state.
    fn merge_states(
        state_a: &HashMap<Symbol, VarState>,
        state_b: &HashMap<Symbol, VarState>,
        given_in: MaybeGivenIn,
    ) -> HashMap<Symbol, VarState> {
        let mut merged = state_a.clone();

        for (sym, state_b_val) in state_b {
            let state_a_val = state_a.get(sym).copied().unwrap_or(VarState::Owned);

            let merged_val = match (state_a_val, *state_b_val) {
                // Both moved = definitely moved
                (VarState::Moved, VarState::Moved) => VarState::Moved,
                // Already maybe moved = still maybe moved, where it first became uncertain
                (VarState::MaybeMoved(earlier), _) | (_, VarState::MaybeMoved(earlier)) => {
                    VarState::MaybeMoved(earlier)
                }
                // One moved, one not = maybe moved, here
                (VarState::Moved, _) | (_, VarState::Moved) => VarState::MaybeMoved(given_in),
                // Both borrowed = borrowed
                (VarState::Borrowed, VarState::Borrowed) => VarState::Borrowed,
                // Borrowed + Owned = Borrowed (conservative)
                (VarState::Borrowed, _) | (_, VarState::Borrowed) => VarState::Borrowed,
                // Both owned = owned
                (VarState::Owned, VarState::Owned) => VarState::Owned,
            };

            merged.insert(*sym, merged_val);
        }

        merged
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/ownership/tests.rs"]
mod tests;
