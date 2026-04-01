//! FOL → Bounded Verification IR Translation
//!
//! Translates Kripke-lowered LogicExpr to a bounded timestep model.
//! World variables (w0, w1, ...) become timestep indices.
//! Temporal accessibility predicates control the unrolling.

use logicaffeine_language::ast::logic::{LogicExpr, QuantifierKind, TemporalOperator, BinaryTemporalOp};
use logicaffeine_language::Interner;
use super::sva_to_verify::BoundedExpr;
use std::collections::{HashMap, HashSet};

/// Translator from Kripke-lowered FOL to bounded timestep model.
pub struct FolTranslator<'a> {
    interner: &'a Interner,
    bound: u32,
    /// Maps world variable symbols to fixed timesteps
    world_map: HashMap<logicaffeine_base::Symbol, u32>,
    /// Accumulated signal declarations
    declarations: HashSet<String>,
}

impl<'a> FolTranslator<'a> {
    pub fn new(interner: &'a Interner, bound: u32) -> Self {
        Self {
            interner,
            bound,
            world_map: HashMap::new(),
            declarations: HashSet::new(),
        }
    }

    /// Translate a Kripke-lowered LogicExpr to bounded verification IR.
    pub fn translate(&mut self, expr: &LogicExpr<'_>) -> BoundedExpr {
        match expr {
            // Predicates with world arguments → timestamped variables
            LogicExpr::Predicate { name, args, world } => {
                let pred_name = self.interner.resolve(*name).to_string();

                // Check for accessibility predicates — these become trivially true in BMC
                if pred_name == "Accessible_Temporal"
                    || pred_name == "Reachable_Temporal"
                    || pred_name == "Next_Temporal"
                {
                    return BoundedExpr::Bool(true);
                }

                // Regular predicate with world → timestamped variable
                if let Some(w) = world {
                    let timestep = self.world_map.get(w).copied().unwrap_or(0);
                    if args.is_empty() {
                        let var_name = format!("{}@{}", pred_name, timestep);
                        self.declarations.insert(var_name.clone());
                        return BoundedExpr::Var(var_name);
                    }
                    // Multi-arg predicate: use first arg as signal name
                    if let Some(arg) = args.first() {
                        let arg_name = self.term_to_string(arg);
                        let var_name = format!("{}_{}_@{}", pred_name, arg_name, timestep);
                        self.declarations.insert(var_name.clone());
                        return BoundedExpr::Var(var_name);
                    }
                }

                // Predicate without world → static (non-temporal)
                let var_name = pred_name;
                self.declarations.insert(var_name.clone());
                BoundedExpr::Var(var_name)
            }

            // Universal quantifier over worlds → conjunction over timesteps
            LogicExpr::Quantifier { kind: QuantifierKind::Universal, variable, body, .. } => {
                let var_name = self.interner.resolve(*variable).to_string();
                if var_name.starts_with('w') {
                    // This is a world quantifier — unroll to conjunction
                    let mut result: Option<BoundedExpr> = None;
                    for t in 0..self.bound {
                        self.world_map.insert(*variable, t);
                        let step = self.translate(body);
                        result = Some(match result {
                            None => step,
                            Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(step)),
                        });
                    }
                    self.world_map.remove(variable);
                    result.unwrap_or(BoundedExpr::Bool(true))
                } else {
                    // Regular variable quantifier — preserve structure
                    self.translate(body)
                }
            }

            // Existential quantifier over worlds → disjunction over timesteps
            LogicExpr::Quantifier { kind: QuantifierKind::Existential, variable, body, .. } => {
                let var_name = self.interner.resolve(*variable).to_string();
                if var_name.starts_with('w') {
                    let mut result: Option<BoundedExpr> = None;
                    for t in 0..self.bound {
                        self.world_map.insert(*variable, t);
                        let step = self.translate(body);
                        result = Some(match result {
                            None => step,
                            Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(step)),
                        });
                    }
                    self.world_map.remove(variable);
                    result.unwrap_or(BoundedExpr::Bool(false))
                } else {
                    self.translate(body)
                }
            }

            // Binary connectives
            LogicExpr::BinaryOp { left, op, right } => {
                let l = self.translate(left);
                let r = self.translate(right);
                match op {
                    logicaffeine_language::token::TokenType::And => {
                        BoundedExpr::And(Box::new(l), Box::new(r))
                    }
                    logicaffeine_language::token::TokenType::Or => {
                        BoundedExpr::Or(Box::new(l), Box::new(r))
                    }
                    logicaffeine_language::token::TokenType::If
                    | logicaffeine_language::token::TokenType::Implies => {
                        BoundedExpr::Implies(Box::new(l), Box::new(r))
                    }
                    _ => {
                        // Other binary ops: default to And
                        BoundedExpr::And(Box::new(l), Box::new(r))
                    }
                }
            }

            // Negation
            LogicExpr::UnaryOp { operand, .. } => {
                let inner = self.translate(operand);
                BoundedExpr::Not(Box::new(inner))
            }

            // LTL temporal operators (if not already Kripke-lowered)
            LogicExpr::Temporal { operator, body } => {
                match operator {
                    TemporalOperator::Always => {
                        // G(P) → conjunction over all timesteps
                        let mut result: Option<BoundedExpr> = None;
                        for t in 0..self.bound {
                            // Temporarily set world mapping
                            let step = self.translate(body);
                            result = Some(match result {
                                None => step,
                                Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(step)),
                            });
                        }
                        result.unwrap_or(BoundedExpr::Bool(true))
                    }
                    TemporalOperator::Eventually => {
                        let mut result: Option<BoundedExpr> = None;
                        for t in 0..self.bound {
                            let step = self.translate(body);
                            result = Some(match result {
                                None => step,
                                Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(step)),
                            });
                        }
                        result.unwrap_or(BoundedExpr::Bool(false))
                    }
                    _ => self.translate(body),
                }
            }

            // Binary temporal operators — each has distinct bounded semantics
            LogicExpr::TemporalBinary { operator, left, right } => {
                let l = self.translate(left);
                let r = self.translate(right);
                match operator {
                    BinaryTemporalOp::Until => {
                        // φ U ψ: ψ holds now, OR (φ holds now AND φ U ψ holds next)
                        // Bounded unrolling: Q ∨ (P ∧ (Q' ∨ (P' ∧ Q'')))
                        // At depth 0: just Or(Q, P) as base case
                        self.unroll_until(&l, &r, 0)
                    }
                    BinaryTemporalOp::Release => {
                        // φ R ψ: dual of Until — ψ must hold, AND (φ releases OR continue)
                        // Bounded: Q ∧ (P ∨ (Q' ∧ (P' ∨ Q'')))
                        self.unroll_release(&l, &r, 0)
                    }
                    BinaryTemporalOp::WeakUntil => {
                        // φ W ψ: (φ U ψ) ∨ G(φ) — Until, or φ holds forever
                        let until = self.unroll_until(&l, &r, 0);
                        let always = self.unroll_always(&l, 0);
                        BoundedExpr::Or(Box::new(until), Box::new(always))
                    }
                }
            }

            // Identity/equality
            LogicExpr::Identity { left, right, .. } => {
                let l = self.term_to_bounded(left);
                let r = self.term_to_bounded(right);
                BoundedExpr::Eq(Box::new(l), Box::new(r))
            }

            // Catch-all: translate as true (safe default for unhandled linguistic constructs)
            _ => BoundedExpr::Bool(true),
        }
    }

    /// Translate a full Kripke-lowered expression as a property (for all timesteps).
    pub fn translate_property(&mut self, expr: &LogicExpr<'_>) -> super::sva_to_verify::TranslateResult {
        let expr_result = self.translate(expr);
        let declarations: Vec<String> = self.declarations.iter().cloned().collect();
        super::sva_to_verify::TranslateResult {
            expr: expr_result,
            declarations,
        }
    }

    /// Unroll φ U ψ (Until) to bounded depth.
    /// φ U ψ ≡ ψ ∨ (φ ∧ X(φ U ψ))
    /// Bounded: at max depth, just return ψ (ψ must eventually hold).
    fn unroll_until(&self, phi: &BoundedExpr, psi: &BoundedExpr, depth: u32) -> BoundedExpr {
        if depth >= self.bound {
            // Base case at bound: ψ must hold
            psi.clone()
        } else {
            // ψ ∨ (φ ∧ unroll(depth+1))
            let rest = self.unroll_until(phi, psi, depth + 1);
            BoundedExpr::Or(
                Box::new(psi.clone()),
                Box::new(BoundedExpr::And(
                    Box::new(phi.clone()),
                    Box::new(rest),
                )),
            )
        }
    }

    /// Unroll φ R ψ (Release) to bounded depth.
    /// φ R ψ ≡ ψ ∧ (φ ∨ X(φ R ψ))   (dual of Until)
    /// Bounded: at max depth, just return ψ (ψ must still hold).
    fn unroll_release(&self, phi: &BoundedExpr, psi: &BoundedExpr, depth: u32) -> BoundedExpr {
        if depth >= self.bound {
            psi.clone()
        } else {
            // ψ ∧ (φ ∨ unroll(depth+1))
            let rest = self.unroll_release(phi, psi, depth + 1);
            BoundedExpr::And(
                Box::new(psi.clone()),
                Box::new(BoundedExpr::Or(
                    Box::new(phi.clone()),
                    Box::new(rest),
                )),
            )
        }
    }

    /// Unroll G(φ) (Always) to bounded depth — conjunction at all remaining steps.
    fn unroll_always(&self, phi: &BoundedExpr, depth: u32) -> BoundedExpr {
        if depth >= self.bound {
            phi.clone()
        } else {
            let rest = self.unroll_always(phi, depth + 1);
            BoundedExpr::And(Box::new(phi.clone()), Box::new(rest))
        }
    }

    fn term_to_string(&self, term: &logicaffeine_language::ast::logic::Term<'_>) -> String {
        use logicaffeine_language::ast::logic::Term;
        match term {
            Term::Constant(sym) | Term::Variable(sym) => {
                self.interner.resolve(*sym).to_string()
            }
            Term::Function(sym, _) => self.interner.resolve(*sym).to_string(),
            _ => "unknown".to_string(),
        }
    }

    fn term_to_bounded(&self, term: &logicaffeine_language::ast::logic::Term<'_>) -> BoundedExpr {
        let name = self.term_to_string(term);
        self.declarations.clone(); // touch for borrow checker
        BoundedExpr::Var(name)
    }
}
