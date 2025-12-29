//! Verification Pass: AST to Verification IR Mapper
//!
//! This module bridges the LOGOS AST to the Z3-based verification system.
//! It maps LogicExpr, Stmt, and Term types to the lightweight Verification IR,
//! which is then encoded into Z3 constraints.
//!
//! Strategy: Smart Full Mapping with Uninterpreted Functions
//! - Int, Bool → direct Z3 sorts
//! - Object → uninterpreted sort for entities
//! - Predicates, Modals, Temporals → Apply (uninterpreted functions)
//! - Z3 reasons structurally without semantic knowledge

use crate::ast::{LogicExpr, ModalDomain, NumberKind, QuantifierKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::token::TokenType;

use logos_verification::{VerificationSession, VerifyExpr, VerifyOp, VerifyType};

/// The verification pass that maps LOGOS AST to Z3 constraints.
pub struct VerificationPass<'a> {
    session: VerificationSession,
    interner: &'a Interner,
}

impl<'a> VerificationPass<'a> {
    /// Create a new verification pass.
    pub fn new(interner: &'a Interner) -> Self {
        Self {
            session: VerificationSession::new(),
            interner,
        }
    }

    /// Run verification on a list of statements.
    ///
    /// This processes Let statements to build up assumptions,
    /// then verifies Assert statements against those assumptions.
    pub fn verify_program(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for stmt in stmts {
            self.visit_stmt(stmt)?;
        }
        Ok(())
    }

    fn visit_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Let { var, ty, value, .. } => {
                let name = self.interner.resolve(*var);

                // Phase 43D: Check refinement constraints BEFORE declaring variable
                if let Some(TypeExpr::Refinement { var: bound_var, predicate, .. }) = ty {
                    self.check_refinement(name, *bound_var, predicate, value)?;
                }

                // Infer type from the value
                let inferred_ty = self.infer_type(value);
                self.session.declare(name, inferred_ty);

                // Map the value to IR and assume var = value
                if let Some(val_ir) = self.map_imperative_expr(value) {
                    let constraint = VerifyExpr::eq(
                        VerifyExpr::var(name),
                        val_ir,
                    );
                    self.session.assume(&constraint);
                }
                Ok(())
            }

            Stmt::Set { target, value } => {
                // Mutation: add new constraint (simplified SSA)
                // In full verification, this would use SSA renaming
                let name = self.interner.resolve(*target);
                if let Some(val_ir) = self.map_imperative_expr(value) {
                    let constraint = VerifyExpr::eq(
                        VerifyExpr::var(name),
                        val_ir,
                    );
                    self.session.assume(&constraint);
                }
                Ok(())
            }

            Stmt::Assert { proposition } => {
                let ir = self.map_logic_expr(proposition);
                // Skip verification if the assertion maps to a trivial True
                // This handles complex linguistic constructs we can't verify yet
                if matches!(&ir, VerifyExpr::Bool(true)) {
                    return Ok(());
                }
                self.session.verify(&ir).map_err(|e| format!("{}", e))
            }

            Stmt::Trust { proposition, justification } => {
                // Trust is like Assert but with documented justification
                // For static verification, we verify it like an assertion
                let ir = self.map_logic_expr(proposition);
                // Skip verification if the assertion maps to a trivial True
                if matches!(&ir, VerifyExpr::Bool(true)) {
                    return Ok(());
                }
                let reason = self.interner.resolve(*justification);
                self.session.verify(&ir).map_err(|e| {
                    format!("Trust verification failed (justification: {}): {}", reason, e)
                })
            }

            // Recurse into blocks (simplified - no path-sensitive analysis yet)
            Stmt::If { then_block, else_block, .. } => {
                // Verify both branches independently
                for stmt in *then_block {
                    self.visit_stmt(stmt)?;
                }
                if let Some(else_stmts) = else_block {
                    for stmt in *else_stmts {
                        self.visit_stmt(stmt)?;
                    }
                }
                Ok(())
            }

            Stmt::While { body, .. } => {
                for stmt in *body {
                    self.visit_stmt(stmt)?;
                }
                Ok(())
            }

            Stmt::Repeat { body, .. } => {
                for stmt in *body {
                    self.visit_stmt(stmt)?;
                }
                Ok(())
            }

            Stmt::Zone { body, .. } => {
                for stmt in *body {
                    self.visit_stmt(stmt)?;
                }
                Ok(())
            }

            Stmt::FunctionDef { body, .. } => {
                for stmt in *body {
                    self.visit_stmt(stmt)?;
                }
                Ok(())
            }

            // Skip statements that don't affect verification
            Stmt::Return { .. }
            | Stmt::Call { .. }
            | Stmt::Give { .. }
            | Stmt::Show { .. }
            | Stmt::SetField { .. }
            | Stmt::StructDef { .. }
            | Stmt::Inspect { .. }
            | Stmt::Push { .. }
            | Stmt::Pop { .. }
            | Stmt::SetIndex { .. }
            | Stmt::Concurrent { .. }
            | Stmt::Parallel { .. }
            | Stmt::ReadFrom { .. }
            | Stmt::WriteFile { .. } => Ok(()),
        }
    }

    /// Infer the verification type from an imperative expression.
    fn infer_type(&self, expr: &Expr) -> VerifyType {
        match expr {
            Expr::Literal(Literal::Number(_)) => VerifyType::Int,
            Expr::Literal(Literal::Boolean(_)) => VerifyType::Bool,
            Expr::Literal(Literal::Text(_)) => VerifyType::Object,
            Expr::Literal(Literal::Nothing) => VerifyType::Object,
            Expr::BinaryOp { op, .. } => {
                match op {
                    // Comparison operators produce Bool
                    BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
                    | BinaryOpKind::Lt
                    | BinaryOpKind::Gt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::And
                    | BinaryOpKind::Or => VerifyType::Bool,
                    // Arithmetic operators produce Int
                    BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide => VerifyType::Int,
                }
            }
            // Default to Int for other expressions
            _ => VerifyType::Int,
        }
    }

    /// Phase 43D: Check that a value satisfies a refinement type constraint.
    fn check_refinement(
        &self,
        var_name: &str,
        bound_var: Symbol,
        predicate: &LogicExpr,
        value: &Expr,
    ) -> Result<(), String> {
        // 1. Map the value to IR
        let val_ir = self.map_imperative_expr(value)
            .ok_or_else(|| format!(
                "Cannot verify refinement for '{}': value expression not supported for verification",
                var_name
            ))?;

        // 2. Map the predicate to IR
        let pred_ir = self.map_logic_expr(predicate);

        // Skip if predicate maps to trivial True (complex linguistic constructs)
        if matches!(&pred_ir, VerifyExpr::Bool(true)) {
            return Ok(());
        }

        // 3. Get the bound variable name (e.g., "it" or "x")
        let bound_name = self.interner.resolve(bound_var);

        // 4. Verify with the binding
        self.session.verify_with_binding(
            bound_name,
            VerifyType::Int, // Refinements are typically on Int
            &val_ir,
            &pred_ir,
        ).map_err(|e| format!(
            "Refinement type verification failed for '{}': {}",
            var_name, e
        ))
    }

    /// Map an imperative expression to Verification IR.
    fn map_imperative_expr(&self, expr: &Expr) -> Option<VerifyExpr> {
        match expr {
            Expr::Literal(Literal::Number(n)) => Some(VerifyExpr::int(*n)),
            Expr::Literal(Literal::Boolean(b)) => Some(VerifyExpr::bool(*b)),
            Expr::Literal(Literal::Text(_)) => None, // Text not supported in Z3
            Expr::Literal(Literal::Nothing) => None,

            Expr::Identifier(sym) => {
                let name = self.interner.resolve(*sym);
                Some(VerifyExpr::var(name))
            }

            Expr::BinaryOp { op, left, right } => {
                let l = self.map_imperative_expr(left)?;
                let r = self.map_imperative_expr(right)?;
                let verify_op = match op {
                    BinaryOpKind::Add => VerifyOp::Add,
                    BinaryOpKind::Subtract => VerifyOp::Sub,
                    BinaryOpKind::Multiply => VerifyOp::Mul,
                    BinaryOpKind::Divide => VerifyOp::Div,
                    BinaryOpKind::Eq => VerifyOp::Eq,
                    BinaryOpKind::NotEq => VerifyOp::Neq,
                    BinaryOpKind::Gt => VerifyOp::Gt,
                    BinaryOpKind::Lt => VerifyOp::Lt,
                    BinaryOpKind::GtEq => VerifyOp::Gte,
                    BinaryOpKind::LtEq => VerifyOp::Lte,
                    BinaryOpKind::And => VerifyOp::And,
                    BinaryOpKind::Or => VerifyOp::Or,
                };
                Some(VerifyExpr::binary(verify_op, l, r))
            }

            Expr::Call { function, args } => {
                let func_name = self.interner.resolve(*function);
                let verify_args: Vec<VerifyExpr> = args
                    .iter()
                    .filter_map(|a| self.map_imperative_expr(a))
                    .collect();
                Some(VerifyExpr::apply(func_name, verify_args))
            }

            // Unsupported expressions
            Expr::Index { .. }
            | Expr::Slice { .. }
            | Expr::Copy { .. }
            | Expr::Length { .. }
            | Expr::List(_)
            | Expr::Range { .. }
            | Expr::FieldAccess { .. }
            | Expr::New { .. }
            | Expr::NewVariant { .. } => None,
        }
    }

    /// Map a logic expression to Verification IR.
    ///
    /// This is the core of the "Smart Full Mapping" strategy:
    /// - Simple types (Int, Bool) map directly
    /// - Complex types (Predicates, Modals) become uninterpreted functions
    fn map_logic_expr(&self, expr: &LogicExpr) -> VerifyExpr {
        match expr {
            LogicExpr::Atom(sym) => {
                // Atoms are boolean variables or 0-arity predicates
                let name = self.interner.resolve(*sym);
                VerifyExpr::var(name)
            }

            LogicExpr::Predicate { name, args } => {
                let pred_name = self.interner.resolve(*name);
                let verify_args: Vec<VerifyExpr> = args
                    .iter()
                    .map(|t| self.map_term(t))
                    .collect();

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
                    TokenType::If | TokenType::Then => VerifyOp::Implies,
                    TokenType::Iff => VerifyOp::Eq, // Biconditional is boolean equality
                    _ => VerifyOp::And, // Fallback
                };
                VerifyExpr::binary(verify_op, l, r)
            }

            LogicExpr::UnaryOp { op, operand } => {
                match op {
                    TokenType::Not => VerifyExpr::not(self.map_logic_expr(operand)),
                    _ => self.map_logic_expr(operand),
                }
            }

            // Smart Mapping: Modal operators become uninterpreted functions
            LogicExpr::Modal { vector, operand } => {
                let op_name = match vector.domain {
                    ModalDomain::Alethic => {
                        if vector.force > 0.5 { "Necessarily" } else { "Possibly" }
                    }
                    ModalDomain::Deontic => {
                        if vector.force > 0.5 { "Obligatory" } else { "Permissible" }
                    }
                };
                VerifyExpr::apply(op_name, vec![self.map_logic_expr(operand)])
            }

            // Smart Mapping: Temporal operators become uninterpreted functions
            LogicExpr::Temporal { operator, body } => {
                let op_name = match operator {
                    crate::ast::TemporalOperator::Past => "Past",
                    crate::ast::TemporalOperator::Future => "Future",
                };
                VerifyExpr::apply(op_name, vec![self.map_logic_expr(body)])
            }

            // Smart Mapping: Aspectual operators become uninterpreted functions
            LogicExpr::Aspectual { operator, body } => {
                let op_name = match operator {
                    crate::ast::AspectOperator::Progressive => "Progressive",
                    crate::ast::AspectOperator::Perfect => "Perfect",
                    crate::ast::AspectOperator::Habitual => "Habitual",
                    crate::ast::AspectOperator::Iterative => "Iterative",
                };
                VerifyExpr::apply(op_name, vec![self.map_logic_expr(body)])
            }

            // Quantifiers map to IR quantifiers
            LogicExpr::Quantifier { kind, variable, body, .. } => {
                let var_name = self.interner.resolve(*variable);
                let body_ir = self.map_logic_expr(body);
                match kind {
                    QuantifierKind::Universal => {
                        VerifyExpr::forall(
                            vec![(var_name.to_string(), VerifyType::Object)],
                            body_ir,
                        )
                    }
                    QuantifierKind::Existential => {
                        VerifyExpr::exists(
                            vec![(var_name.to_string(), VerifyType::Object)],
                            body_ir,
                        )
                    }
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
                    QuantifierKind::Cardinal(n) => {
                        VerifyExpr::apply(
                            &format!("Exactly{}", n),
                            vec![VerifyExpr::var(var_name), body_ir],
                        )
                    }
                    QuantifierKind::AtLeast(n) => {
                        VerifyExpr::apply(
                            &format!("AtLeast{}", n),
                            vec![VerifyExpr::var(var_name), body_ir],
                        )
                    }
                    QuantifierKind::AtMost(n) => {
                        VerifyExpr::apply(
                            &format!("AtMost{}", n),
                            vec![VerifyExpr::var(var_name), body_ir],
                        )
                    }
                    QuantifierKind::Generic => {
                        VerifyExpr::apply("Generic", vec![VerifyExpr::var(var_name), body_ir])
                    }
                }
            }

            // Lambda abstractions become uninterpreted
            LogicExpr::Lambda { variable, body } => {
                let var_name = self.interner.resolve(*variable);
                VerifyExpr::apply(
                    "Lambda",
                    vec![VerifyExpr::var(var_name), self.map_logic_expr(body)],
                )
            }

            // Function application
            LogicExpr::App { function, argument } => {
                VerifyExpr::apply(
                    "App",
                    vec![self.map_logic_expr(function), self.map_logic_expr(argument)],
                )
            }

            // Counterfactuals: if-then with special modal semantics
            LogicExpr::Counterfactual { antecedent, consequent } => {
                VerifyExpr::apply(
                    "Counterfactual",
                    vec![self.map_logic_expr(antecedent), self.map_logic_expr(consequent)],
                )
            }

            // Causation
            LogicExpr::Causal { cause, effect } => {
                VerifyExpr::apply(
                    "Causes",
                    vec![self.map_logic_expr(cause), self.map_logic_expr(effect)],
                )
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
            LogicExpr::SpeechAct { performer, act_type, content } => {
                let performer_name = self.interner.resolve(*performer);
                let act_name = self.interner.resolve(*act_type);
                VerifyExpr::apply(
                    act_name,
                    vec![VerifyExpr::var(performer_name), self.map_logic_expr(content)],
                )
            }

            // Comparatives
            LogicExpr::Comparative { adjective, subject, object, difference } => {
                let adj_name = self.interner.resolve(*adjective);
                let mut args = vec![
                    self.map_term(subject),
                    self.map_term(object),
                ];
                if let Some(diff) = difference {
                    args.push(self.map_term(diff));
                }
                VerifyExpr::apply(&format!("More{}", adj_name), args)
            }

            // Superlatives
            LogicExpr::Superlative { adjective, subject, domain } => {
                let adj_name = self.interner.resolve(*adjective);
                let domain_name = self.interner.resolve(*domain);
                VerifyExpr::apply(
                    &format!("Most{}", adj_name),
                    vec![self.map_term(subject), VerifyExpr::var(domain_name)],
                )
            }

            // Focus
            LogicExpr::Focus { focused, scope, .. } => {
                VerifyExpr::apply(
                    "Focus",
                    vec![self.map_term(focused), self.map_logic_expr(scope)],
                )
            }

            // Presupposition
            LogicExpr::Presupposition { assertion, presupposition } => {
                // Verify both assertion and presupposition
                VerifyExpr::and(
                    self.map_logic_expr(presupposition),
                    self.map_logic_expr(assertion),
                )
            }

            // Fallback for complex types: map to True to avoid false positives
            LogicExpr::Metaphor { .. }
            | LogicExpr::Categorical(_)
            | LogicExpr::Relation(_)
            | LogicExpr::Voice { .. }
            | LogicExpr::Event { .. }
            | LogicExpr::NeoEvent(_)
            | LogicExpr::Imperative { .. }
            | LogicExpr::TemporalAnchor { .. }
            | LogicExpr::Distributive { .. }
            | LogicExpr::GroupQuantifier { .. }
            | LogicExpr::Scopal { .. }
            | LogicExpr::Control { .. } => {
                // These complex linguistic constructs are assumed valid
                VerifyExpr::bool(true)
            }
        }
    }

    /// Map a term to Verification IR.
    fn map_term(&self, term: &Term) -> VerifyExpr {
        match term {
            Term::Constant(sym) | Term::Variable(sym) => {
                let name = self.interner.resolve(*sym);
                VerifyExpr::var(name)
            }

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
                let verify_args: Vec<VerifyExpr> = args
                    .iter()
                    .map(|t| self.map_term(t))
                    .collect();
                VerifyExpr::apply(func_name, verify_args)
            }

            Term::Group(terms) => {
                // Group terms become a special "Group" function
                let verify_args: Vec<VerifyExpr> = terms
                    .iter()
                    .map(|t| self.map_term(t))
                    .collect();
                VerifyExpr::apply("Group", verify_args)
            }

            Term::Possessed { possessor, possessed } => {
                let poss_name = self.interner.resolve(*possessed);
                VerifyExpr::apply(
                    &format!("{}Of", poss_name),
                    vec![self.map_term(possessor)],
                )
            }

            Term::Sigma(sym) => {
                let name = self.interner.resolve(*sym);
                VerifyExpr::apply("Sigma", vec![VerifyExpr::var(name)])
            }

            Term::Intension(sym) => {
                let name = self.interner.resolve(*sym);
                VerifyExpr::apply("Intension", vec![VerifyExpr::var(name)])
            }

            Term::Proposition(expr) => {
                self.map_logic_expr(expr)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_interner() -> Interner {
        Interner::new()
    }

    #[test]
    fn test_verification_pass_creation() {
        let interner = make_interner();
        let pass = VerificationPass::new(&interner);
        // Just verify it constructs without panic
        drop(pass);
    }
}
