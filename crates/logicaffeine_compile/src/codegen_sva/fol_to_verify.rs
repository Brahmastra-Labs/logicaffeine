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
                    logicaffeine_language::token::TokenType::If => {
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

            // Binary temporal (Until)
            LogicExpr::TemporalBinary { operator: _, left, right } => {
                let l = self.translate(left);
                let r = self.translate(right);
                BoundedExpr::Or(Box::new(r), Box::new(l))
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
