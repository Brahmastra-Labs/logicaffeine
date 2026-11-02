//! Escape analysis for zone safety.
//!
//! Implements the zone containment rule: values can enter
//! zones but cannot escape. This pass checks for obvious violations before
//! code generation, providing Socratic error messages.
//!
//! # The Zone Rule
//!
//! Variables defined inside a zone are deallocated when the zone ends.
//! Returning or assigning them to outer variables would create dangling
//! references. This pass catches these common patterns with friendly errors.
//!
//! # Example
//!
//! ```text
//! Zone "temp":
//!     Let x be 5.
//!     Return x.        ← Error: x cannot escape zone "temp"
//! ```
//!
//! # Limitations
//!
//! More complex escape patterns (e.g., through closures or indirect references)
//! are caught by Rust's borrow checker at compile time. This pass catches the
//! common cases with better LOGOS-specific error messages.

use crate::ast::stmt::{Expr, Stmt, StmtId};
use crate::intern::{Interner, Symbol};
use crate::token::Span;
use std::collections::HashMap;

/// Error type for escape violations
#[derive(Debug, Clone)]
pub struct EscapeError {
    pub kind: EscapeErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum EscapeErrorKind {
    /// Variable cannot escape zone via return
    ReturnEscape { variable: String, zone_name: String },
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
            EscapeErrorKind::ReturnEscape {
                variable,
                zone_name,
            } => {
                write!(
                    f,
                    "Reference '{}' cannot escape zone '{}'.\n\n\
                    Variables allocated inside a zone are deallocated when the zone ends.\n\
                    Returning them would create a dangling reference.\n\n\
                    Tip: Copy the data if you need it outside the zone.",
                    variable, zone_name
                )
            }
            EscapeErrorKind::AssignmentEscape {
                variable,
                target,
                zone_name,
            } => {
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
/// One escape finding: the top-level statement it belongs to (mapping onto
/// `Parser::stmt_spans()`), and the erring statement with each one around it,
/// innermost first (for `Parser::nested_stmt_spans()`).
#[derive(Debug)]
pub struct EscapeFinding {
    pub stmt_index: usize,
    pub trail: Vec<StmtId>,
    pub error: EscapeError,
}

/// An escape error on its way out of the statements it was found in, naming
/// each one as it passes, innermost first.
struct Traced {
    error: EscapeError,
    trail: Vec<StmtId>,
}

impl From<EscapeError> for Traced {
    fn from(error: EscapeError) -> Self {
        Traced {
            error,
            trail: Vec::new(),
        }
    }
}

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
        self.check_block(stmts).map_err(|traced| traced.error)
    }

    /// Collect every escape finding with its statements, continuing past failures.
    pub fn check_program_collect(&mut self, stmts: &[Stmt<'_>]) -> Vec<EscapeFinding> {
        stmts
            .iter()
            .enumerate()
            .filter_map(|(index, stmt)| {
                self.check_stmt(stmt)
                    .err()
                    .map(|Traced { error, trail }| EscapeFinding {
                        stmt_index: index,
                        trail,
                        error,
                    })
            })
            .collect()
    }

    /// Check a block of statements
    fn check_block(&mut self, stmts: &[Stmt<'_>]) -> Result<(), Traced> {
        for stmt in stmts {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    /// Check a single statement for escape violations
    fn check_stmt(&mut self, stmt: &Stmt<'_>) -> Result<(), Traced> {
        self.check_stmt_kind(stmt).map_err(|mut traced| {
            traced.trail.push(StmtId::of(stmt));
            traced
        })
    }

    fn check_stmt_kind(&mut self, stmt: &Stmt<'_>) -> Result<(), Traced> {
        match stmt {
            Stmt::Zone { name, body, .. } => {
                // Enter zone: increase depth
                self.current_depth += 1;
                self.zone_stack.push(*name);

                // Check body statements
                let body_result = self.check_block(body);

                // Exit zone: decrease depth — on a finding too, so the
                // statements after it are checked at their own depth.
                self.zone_stack.pop();
                self.current_depth -= 1;
                body_result?;
            }

            // A function body is its own scope: its zones and their values
            // are checked where they are written, and its names end with it.
            Stmt::FunctionDef { body, .. } => {
                let outer = self.zone_depth.clone();
                let body_result = self.check_block(body);
                self.zone_depth = outer;
                body_result?;
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
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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

            // A `During` span is scope-transparent: its body runs in place, where it is written.
            Stmt::During { body, .. } => {
                self.check_block(body)?;
            }

            // Escape blocks are opaque — the Rust compiler handles zone safety for raw code
            Stmt::Escape { .. } => {}

            // Other statements don't introduce escape risks
            _ => {}
        }
        Ok(())
    }

    /// Check that an expression doesn't escape to a shallower depth
    fn check_no_escape(&self, expr: &Expr<'_>, max_depth: usize) -> Result<(), EscapeError> {
        match expr {
            // A view's holes are evaluated where the view renders; its handler blocks are
            // closures (checked when called, like a block closure).
            // A page query reads the DOM; only its selector expression can escape a zone.
            Expr::Dom(q) => {
                if let Some(sel) = q.selector() {
                    self.check_no_escape(sel, max_depth)?;
                }
            }
            Expr::View(v) => {
                for e in v.exprs_and_blocks().0 {
                    self.check_no_escape(e, max_depth)?;
                }
            }
            Expr::Identifier(sym) => {
                if let Some(&depth) = self.zone_depth.get(sym) {
                    if depth > max_depth && depth > 0 {
                        // This variable was defined in a deeper zone
                        let zone_name = self
                            .zone_stack
                            .get(depth - 1)
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

            Expr::Slice {
                collection,
                start,
                end,
            } => {
                self.check_no_escape(collection, max_depth)?;
                self.check_no_escape(start, max_depth)?;
                self.check_no_escape(end, max_depth)?;
            }

            Expr::Copy { expr }
            | Expr::Give { value: expr }
            | Expr::Length { collection: expr }
            | Expr::Not { operand: expr } => {
                self.check_no_escape(expr, max_depth)?;
            }

            Expr::List(items) | Expr::Tuple(items) => {
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

            Expr::WithCapacity { value, capacity } => {
                self.check_no_escape(value, max_depth)?;
                self.check_no_escape(capacity, max_depth)?;
            }

            Expr::OptionSome { value } => {
                self.check_no_escape(value, max_depth)?;
            }
            Expr::OptionNone => {}

            // Escape expressions are opaque — the Rust compiler handles zone safety for raw code
            Expr::Escape { .. } => {}

            Expr::Closure { body, .. } => {
                match body {
                    crate::ast::stmt::ClosureBody::Expression(expr) => {
                        self.check_no_escape(expr, max_depth)?;
                    }
                    crate::ast::stmt::ClosureBody::Block(_) => {
                        // Block closures don't escape zone values through their definition.
                        // The closure body is a separate scope; zone safety for the body
                        // is checked when the closure is called, not when it's defined.
                    }
                }
            }

            Expr::CallExpr { callee, args } => {
                self.check_no_escape(callee, max_depth)?;
                for arg in args {
                    self.check_no_escape(arg, max_depth)?;
                }
            }

            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let crate::ast::stmt::StringPart::Expr { value, .. } = part {
                        self.check_no_escape(value, max_depth)?;
                    }
                }
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
                        let zone_name = self
                            .zone_stack
                            .get(depth - 1)
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/escape/tests.rs"]
mod tests;
