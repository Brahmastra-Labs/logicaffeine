//! Phase 8.5: Escape Analysis for Zone Safety
//!
//! Implements the "Hotel California" containment rule: values can enter
//! zones but cannot escape. This pass checks for obvious violations before
//! codegen, providing Socratic error messages.
//!
//! More complex escape patterns are caught by Rust's borrow checker at
//! compile time, but this pass catches the common cases with better errors.

use std::collections::HashMap;
use crate::ast::stmt::{Stmt, Expr, Block};
use crate::intern::{Interner, Symbol};
use crate::token::Span;

/// Error type for escape violations
#[derive(Debug, Clone)]
pub struct EscapeError {
    pub kind: EscapeErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum EscapeErrorKind {
    /// Variable cannot escape zone via return
    ReturnEscape {
        variable: String,
        zone_name: String,
    },
    /// Variable cannot escape zone via assignment to outer variable
    AssignmentEscape {
        variable: String,
        target: String,
        zone_name: String,
    },
}

impl std::fmt::Display for EscapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            EscapeErrorKind::ReturnEscape { variable, zone_name } => {
                write!(
                    f,
                    "Reference '{}' cannot escape zone '{}'.\n\n\
                    Variables allocated inside a zone are deallocated when the zone ends.\n\
                    Returning them would create a dangling reference.\n\n\
                    Tip: Copy the data if you need it outside the zone.",
                    variable, zone_name
                )
            }
            EscapeErrorKind::AssignmentEscape { variable, target, zone_name } => {
                write!(
                    f,
                    "Reference '{}' cannot escape zone '{}' via assignment to '{}'.\n\n\
                    Variables allocated inside a zone are deallocated when the zone ends.\n\
                    Assigning them to outer scope variables would create a dangling reference.\n\n\
                    Tip: Copy the data if you need it outside the zone.",
                    variable, zone_name, target
                )
            }
        }
    }
}

impl std::error::Error for EscapeError {}

/// Tracks the "zone depth" of variables for escape analysis
pub struct EscapeChecker<'a> {
    /// Maps variable symbols to their zone depth (0 = global/outside all zones)
    zone_depth: HashMap<Symbol, usize>,
    /// Current zone depth (increases when entering zones)
    current_depth: usize,
    /// Stack of zone names for error messages
    zone_stack: Vec<Symbol>,
    /// String interner for resolving symbols
    interner: &'a Interner,
}

impl<'a> EscapeChecker<'a> {
    /// Create a new escape checker
    pub fn new(interner: &'a Interner) -> Self {
        Self {
            zone_depth: HashMap::new(),
            current_depth: 0,
            zone_stack: Vec::new(),
            interner,
        }
    }

    /// Check a program (list of statements) for escape violations
    pub fn check_program(&mut self, stmts: &[Stmt<'_>]) -> Result<(), EscapeError> {
        self.check_block(stmts)
    }

    /// Check a block of statements
    fn check_block(&mut self, stmts: &[Stmt<'_>]) -> Result<(), EscapeError> {
        for stmt in stmts {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    /// Check a single statement for escape violations
    fn check_stmt(&mut self, stmt: &Stmt<'_>) -> Result<(), EscapeError> {
        match stmt {
            Stmt::Zone { name, body, .. } => {
                // Enter zone: increase depth
                self.current_depth += 1;
                self.zone_stack.push(*name);

                // Check body statements
                self.check_block(body)?;

                // Exit zone: decrease depth
                self.zone_stack.pop();
                self.current_depth -= 1;
            }

            Stmt::Let { var, .. } => {
                // Register variable at current depth
                self.zone_depth.insert(*var, self.current_depth);
            }

            Stmt::Return { value: Some(expr) } => {
                // Return escapes all zones (target depth = 0)
                self.check_no_escape(expr, 0)?;
            }

            Stmt::Set { target, value } => {
                // Assignment: check if value escapes to target's depth
                let target_depth = self.zone_depth.get(target).copied().unwrap_or(0);
                self.check_no_escape_with_target(value, target_depth, *target)?;
            }

            // Recurse into nested blocks
            Stmt::If { then_block, else_block, .. } => {
                self.check_block(then_block)?;
                if let Some(else_b) = else_block {
                    self.check_block(else_b)?;
                }
            }

            Stmt::While { body, .. } => {
                self.check_block(body)?;
            }

            Stmt::Repeat { body, .. } => {
                self.check_block(body)?;
            }

            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    self.check_block(arm.body)?;
                }
            }

            // Other statements don't introduce escape risks
            _ => {}
        }
        Ok(())
    }

    /// Check that an expression doesn't escape to a shallower depth
    fn check_no_escape(&self, expr: &Expr<'_>, max_depth: usize) -> Result<(), EscapeError> {
        match expr {
            Expr::Identifier(sym) => {
                if let Some(&depth) = self.zone_depth.get(sym) {
                    if depth > max_depth && depth > 0 {
                        // This variable was defined in a deeper zone
                        let zone_name = self.zone_stack.get(depth - 1)
                            .map(|s| self.interner.resolve(*s).to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        let var_name = self.interner.resolve(*sym).to_string();
                        return Err(EscapeError {
                            kind: EscapeErrorKind::ReturnEscape {
                                variable: var_name,
                                zone_name,
                            },
                            span: Span::default(),
                        });
                    }
                }
            }

            // Recurse into compound expressions
            Expr::BinaryOp { left, right, .. } => {
                self.check_no_escape(left, max_depth)?;
                self.check_no_escape(right, max_depth)?;
            }

            Expr::Call { args, .. } => {
                for arg in args {
                    self.check_no_escape(arg, max_depth)?;
                }
            }

            Expr::FieldAccess { object, .. } => {
                self.check_no_escape(object, max_depth)?;
            }

            Expr::Index { collection, index } => {
                self.check_no_escape(collection, max_depth)?;
                self.check_no_escape(index, max_depth)?;
            }

            Expr::Slice { collection, start, end } => {
                self.check_no_escape(collection, max_depth)?;
                self.check_no_escape(start, max_depth)?;
                self.check_no_escape(end, max_depth)?;
            }

            Expr::Copy { expr } | Expr::Length { collection: expr } => {
                self.check_no_escape(expr, max_depth)?;
            }

            Expr::List(items) => {
                for item in items {
                    self.check_no_escape(item, max_depth)?;
                }
            }

            Expr::Range { start, end } => {
                self.check_no_escape(start, max_depth)?;
                self.check_no_escape(end, max_depth)?;
            }

            Expr::New { init_fields, .. } => {
                for (_, expr) in init_fields {
                    self.check_no_escape(expr, max_depth)?;
                }
            }

            Expr::NewVariant { fields, .. } => {
                for (_, expr) in fields {
                    self.check_no_escape(expr, max_depth)?;
                }
            }

            Expr::ManifestOf { zone } => {
                self.check_no_escape(zone, max_depth)?;
            }

            Expr::ChunkAt { index, zone } => {
                self.check_no_escape(index, max_depth)?;
                self.check_no_escape(zone, max_depth)?;
            }

            Expr::Contains { collection, value } => {
                self.check_no_escape(collection, max_depth)?;
                self.check_no_escape(value, max_depth)?;
            }

            Expr::Union { left, right } | Expr::Intersection { left, right } => {
                self.check_no_escape(left, max_depth)?;
                self.check_no_escape(right, max_depth)?;
            }

            // Literals are always safe
            Expr::Literal(_) => {}
        }
        Ok(())
    }

    /// Check that an expression doesn't escape via assignment
    fn check_no_escape_with_target(
        &self,
        expr: &Expr<'_>,
        max_depth: usize,
        target: Symbol,
    ) -> Result<(), EscapeError> {
        match expr {
            Expr::Identifier(sym) => {
                if let Some(&depth) = self.zone_depth.get(sym) {
                    if depth > max_depth && depth > 0 {
                        let zone_name = self.zone_stack.get(depth - 1)
                            .map(|s| self.interner.resolve(*s).to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        let var_name = self.interner.resolve(*sym).to_string();
                        let target_name = self.interner.resolve(target).to_string();
                        return Err(EscapeError {
                            kind: EscapeErrorKind::AssignmentEscape {
                                variable: var_name,
                                target: target_name,
                                zone_name,
                            },
                            span: Span::default(),
                        });
                    }
                }
            }
            // For compound expressions, use the simpler check (return style error)
            _ => self.check_no_escape(expr, max_depth)?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full integration tests are in tests/phase85_zones.rs
    // These unit tests verify the basic mechanics of the escape checker

    #[test]
    fn test_escape_checker_basic() {
        use crate::intern::Interner;

        let mut interner = Interner::new();
        let checker = EscapeChecker::new(&interner);

        // Just verify creation works
        assert_eq!(checker.current_depth, 0);
        assert!(checker.zone_depth.is_empty());
    }
}
