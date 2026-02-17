//! Native ownership analysis for use-after-move detection.
//!
//! Lightweight data-flow analysis that catches the 90% common ownership errors
//! at check-time (milliseconds), before Rust compilation. This pass tracks
//! `Owned`, `Moved`, and `Borrowed` states through control flow.
//!
//! # State Transitions
//!
//! ```text
//!          Let x be value
//!               │
//!               ▼
//!           [Owned]
//!          /        \
//!    Give x       Show x
//!        │            │
//!        ▼            ▼
//!    [Moved]     [Borrowed]
//!        │            │
//!    use x?      use x? ✓
//!        │
//!     ERROR: use-after-move
//! ```
//!
//! # Control Flow Awareness
//!
//! The checker handles branches by merging states:
//! - `Moved` in one branch + `Owned` in other = `MaybeMoved`
//! - Using a `MaybeMoved` variable produces an error
//!
//! # Example
//!
//! ```text
//! Let x be 5.
//! Give x to y.
//! Show x to show.  ← Error: Cannot use 'x' after giving it away
//! ```

use std::collections::HashMap;
use crate::ast::stmt::{BinaryOpKind, Literal, Stmt, Expr, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::token::Span;

/// Ownership state for a variable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarState {
    /// Variable is owned and can be used
    Owned,
    /// Variable has been moved (Give)
    Moved,
    /// Variable might be moved (conditional branch)
    MaybeMoved,
    /// Variable is borrowed (Show) - still usable
    Borrowed,
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
    /// Use after potential move (in conditional)
    UseAfterMaybeMove { variable: String, branch: String },
    /// Double move
    DoubleMoved { variable: String },
}

impl std::fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            OwnershipErrorKind::UseAfterMove { variable } => {
                write!(f, "Cannot use '{}' after giving it away.\n\n\
                    You transferred ownership of '{}' with Give.\n\
                    Once given, you cannot use it anymore.\n\n\
                    Tip: Use Show instead to lend without giving up ownership.",
                    variable, variable)
            }
            OwnershipErrorKind::UseAfterMaybeMove { variable, branch } => {
                write!(f, "Cannot use '{}' - it might have been given away in {}.\n\n\
                    If the {} branch executes, '{}' will be moved.\n\
                    Using it afterward is not safe.\n\n\
                    Tip: Move the usage inside the branch, or restructure to ensure ownership.",
                    variable, branch, branch, variable)
            }
            OwnershipErrorKind::DoubleMoved { variable } => {
                write!(f, "Cannot give '{}' twice.\n\n\
                    You already transferred ownership of '{}' with Give.\n\
                    You cannot give it again.\n\n\
                    Tip: Consider using Copy to duplicate the value.",
                    variable, variable)
            }
        }
    }
}

impl std::error::Error for OwnershipError {}

/// Ownership checker - tracks variable states through control flow
pub struct OwnershipChecker<'a> {
    /// Maps variable symbols to their current ownership state
    state: HashMap<Symbol, VarState>,
    /// Tracks whether each variable is a Copy type (true = Copy, absent = unknown/Copy)
    types: HashMap<Symbol, bool>,
    /// String interner for resolving symbols
    interner: &'a Interner,
}

impl<'a> OwnershipChecker<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Self {
            state: HashMap::new(),
            types: HashMap::new(),
            interner,
        }
    }

    /// Access the current variable ownership states.
    pub fn var_states(&self) -> &HashMap<Symbol, VarState> {
        &self.state
    }

    /// Returns true if a symbol is known to be a Copy type.
    /// Unknown types conservatively return true (won't produce false positives).
    fn is_copy_sym(&self, sym: Symbol) -> bool {
        self.types.get(&sym).copied().unwrap_or(true)
    }

    /// Infer whether an expression produces a Copy type.
    /// Conservative: returns true (Copy) when uncertain.
    fn infer_copy_from_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Literal(Literal::Number(_)) => true,
            Expr::Literal(Literal::Float(_)) => true,
            Expr::Literal(Literal::Boolean(_)) => true,
            Expr::Literal(Literal::Nothing) => true,
            Expr::Literal(Literal::Text(_)) => false,
            Expr::Identifier(sym) => self.is_copy_sym(*sym),
            Expr::New { .. } => false,
            Expr::List(_) => false,
            Expr::InterpolatedString(_) => false,
            Expr::Copy { .. } => true,
            Expr::BinaryOp { op: BinaryOpKind::Concat, .. } => false,
            Expr::BinaryOp { .. } => true,
            Expr::Contains { .. } => true,
            Expr::Length { .. } => true,
            _ => true,
        }
    }

    /// After an expression has been validated, walk it to mark non-Copy
    /// function call arguments as Moved.
    fn mark_moves_in_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Call { args, .. } | Expr::CallExpr { args, .. } => {
                for arg in args.iter() {
                    if let Expr::Identifier(sym) = arg {
                        if !self.is_copy_sym(*sym) {
                            self.state.insert(*sym, VarState::Moved);
                        }
                    }
                    self.mark_moves_in_expr(arg);
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.mark_moves_in_expr(left);
                self.mark_moves_in_expr(right);
            }
            Expr::Index { collection, index } => {
                self.mark_moves_in_expr(collection);
                self.mark_moves_in_expr(index);
            }
            Expr::FieldAccess { object, .. } => {
                self.mark_moves_in_expr(object);
            }
            _ => {}
        }
    }

    /// Infer Copy-ness from a TypeExpr (function parameter type annotation).
    /// Conservative: returns true (Copy) when uncertain.
    fn infer_copy_from_type_name(&self, ty: &TypeExpr) -> bool {
        match ty {
            TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                let name = self.interner.resolve(*sym);
                matches!(name, "Int" | "Nat" | "Float" | "Bool" | "Char" | "Byte")
            }
            TypeExpr::Generic { .. } => false,
            TypeExpr::Function { .. } => true,
            _ => true,
        }
    }

    /// Check a program for ownership violations
    pub fn check_program(&mut self, stmts: &[Stmt<'_>]) -> Result<(), OwnershipError> {
        self.check_block(stmts)
    }

    fn check_block(&mut self, stmts: &[Stmt<'_>]) -> Result<(), OwnershipError> {
        for stmt in stmts {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt<'_>) -> Result<(), OwnershipError> {
        match stmt {
            Stmt::Let { var, value, .. } => {
                // Check the value expression first
                self.check_not_moved(value)?;
                // Mark non-Copy identifiers used as values as Moved
                if let Expr::Identifier(sym) = value {
                    if !self.is_copy_sym(*sym) {
                        self.state.insert(*sym, VarState::Moved);
                    }
                }
                // Mark non-Copy function call arguments as Moved
                self.mark_moves_in_expr(value);
                // Register variable as Owned and track its type
                let is_copy = self.infer_copy_from_expr(value);
                self.state.insert(*var, VarState::Owned);
                self.types.insert(*var, is_copy);
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
                            });
                        }
                        VarState::MaybeMoved => {
                            return Err(OwnershipError {
                                kind: OwnershipErrorKind::UseAfterMaybeMove {
                                    variable: self.interner.resolve(*sym).to_string(),
                                    branch: "a previous branch".to_string(),
                                },
                                span: Span::default(),
                            });
                        }
                        _ => {
                            self.state.insert(*sym, VarState::Moved);
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

            Stmt::If { then_block, else_block, .. } => {
                // Clone state before branching
                let state_before = self.state.clone();

                // Check then branch
                self.check_block(then_block)?;
                let state_after_then = self.state.clone();

                // Check else branch (if exists)
                let state_after_else = if let Some(else_b) = else_block {
                    self.state = state_before.clone();
                    self.check_block(else_b)?;
                    self.state.clone()
                } else {
                    state_before.clone()
                };

                // Merge states: MaybeMoved if moved in any branch
                self.state = self.merge_states(&state_after_then, &state_after_else);
            }

            Stmt::While { body, .. } => {
                // Clone state before loop
                let state_before = self.state.clone();

                // Check body once
                self.check_block(body)?;
                let state_after_body = self.state.clone();

                // Merge: if moved in body, mark as MaybeMoved
                // (loop might not execute, or might execute multiple times)
                self.state = self.merge_states(&state_before, &state_after_body);
            }

            Stmt::Repeat { body, .. } => {
                // Check body once
                self.check_block(body)?;
            }

            Stmt::Zone { body, .. } => {
                self.check_block(body)?;
            }

            Stmt::Inspect { arms, .. } => {
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
                        merged = self.merge_states(&merged, state);
                    }
                    self.state = merged;
                }
            }

            Stmt::Return { value: Some(expr) } => {
                self.check_not_moved(expr)?;
                self.mark_moves_in_expr(expr);
            }

            Stmt::Return { value: None } => {}

            Stmt::Set { value, .. } => {
                self.check_not_moved(value)?;
                // Mark non-Copy function call arguments as Moved
                self.mark_moves_in_expr(value);
            }

            Stmt::Call { args, .. } => {
                for arg in args.iter() {
                    self.check_not_moved(arg)?;
                }
                // Mark non-Copy identifier arguments as Moved
                for arg in args.iter() {
                    if let Expr::Identifier(sym) = arg {
                        if !self.is_copy_sym(*sym) {
                            self.state.insert(*sym, VarState::Moved);
                        }
                    }
                }
            }

            Stmt::FunctionDef { params, body, .. } => {
                // Save state — function body is a separate scope
                let saved_state = self.state.clone();
                let saved_types = self.types.clone();
                // Register parameters as Owned with inferred Copy-ness
                for (param_sym, param_type) in params.iter() {
                    self.state.insert(*param_sym, VarState::Owned);
                    let is_copy = self.infer_copy_from_type_name(param_type);
                    self.types.insert(*param_sym, is_copy);
                }
                self.check_block(body)?;
                self.state = saved_state;
                self.types = saved_types;
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
            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let crate::ast::stmt::StringPart::Expr { value, .. } = part {
                        self.check_not_moved(value)?;
                    }
                }
                Ok(())
            }
            Expr::Identifier(sym) => {
                match self.state.get(sym).copied() {
                    Some(VarState::Moved) => {
                        Err(OwnershipError {
                            kind: OwnershipErrorKind::UseAfterMove {
                                variable: self.interner.resolve(*sym).to_string(),
                            },
                            span: Span::default(),
                        })
                    }
                    Some(VarState::MaybeMoved) => {
                        Err(OwnershipError {
                            kind: OwnershipErrorKind::UseAfterMaybeMove {
                                variable: self.interner.resolve(*sym).to_string(),
                                branch: "a conditional branch".to_string(),
                            },
                            span: Span::default(),
                        })
                    }
                    _ => Ok(())
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.check_not_moved(left)?;
                self.check_not_moved(right)?;
                Ok(())
            }
            Expr::FieldAccess { object, .. } => {
                self.check_not_moved(object)
            }
            Expr::Index { collection, index } => {
                self.check_not_moved(collection)?;
                self.check_not_moved(index)?;
                Ok(())
            }
            Expr::Slice { collection, start, end } => {
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
            Expr::Copy { expr } | Expr::Give { value: expr } | Expr::Length { collection: expr } => {
                self.check_not_moved(expr)
            }
            Expr::ManifestOf { zone } => {
                self.check_not_moved(zone)
            }
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
            Expr::Closure { body, .. } => {
                match body {
                    crate::ast::stmt::ClosureBody::Expression(expr) => {
                        self.check_not_moved(expr)
                    }
                    crate::ast::stmt::ClosureBody::Block(_) => Ok(()),
                }
            }

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

    /// Merge two branch states - if moved in either, mark as MaybeMoved
    fn merge_states(
        &self,
        state_a: &HashMap<Symbol, VarState>,
        state_b: &HashMap<Symbol, VarState>,
    ) -> HashMap<Symbol, VarState> {
        let mut merged = state_a.clone();

        // Merge keys from state_b
        for (sym, state_b_val) in state_b {
            let state_a_val = state_a.get(sym).copied().unwrap_or(VarState::Owned);

            let merged_val = match (state_a_val, *state_b_val) {
                // Both moved = definitely moved
                (VarState::Moved, VarState::Moved) => VarState::Moved,
                // One moved, one not = maybe moved
                (VarState::Moved, _) | (_, VarState::Moved) => VarState::MaybeMoved,
                // Any maybe moved = maybe moved
                (VarState::MaybeMoved, _) | (_, VarState::MaybeMoved) => VarState::MaybeMoved,
                // Both borrowed = borrowed
                (VarState::Borrowed, VarState::Borrowed) => VarState::Borrowed,
                // Borrowed + Owned = Borrowed (conservative)
                (VarState::Borrowed, _) | (_, VarState::Borrowed) => VarState::Borrowed,
                // Both owned = owned
                (VarState::Owned, VarState::Owned) => VarState::Owned,
            };

            merged.insert(*sym, merged_val);
        }

        // Also check keys only in state_a
        for sym in state_a.keys() {
            if !state_b.contains_key(sym) {
                // Variable exists in one branch but not other - keep state_a value
                // (already in merged)
            }
        }

        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ownership_checker_basic() {
        let interner = Interner::new();
        let checker = OwnershipChecker::new(&interner);
        assert!(checker.state.is_empty());
    }
}
