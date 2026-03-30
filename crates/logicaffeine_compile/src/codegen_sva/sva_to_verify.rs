//! SVA → Bounded Verification IR Translation
//!
//! Translates SvaExpr to a bounded timestep model suitable for Z3 equivalence checking.
//! Each signal at timestep t becomes a variable "signal@t".
//! Temporal operators are unrolled to bounded disjunctions/conjunctions.

use super::sva_model::SvaExpr;
use std::collections::HashSet;

/// A verification expression in the bounded timestep model.
/// This is a Z3-ready IR — each node maps directly to a Z3 AST construct.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundedExpr {
    /// Boolean variable: "signal@timestep"
    Var(String),
    /// Boolean literal
    Bool(bool),
    /// Integer literal
    Int(i64),
    /// Conjunction
    And(Box<BoundedExpr>, Box<BoundedExpr>),
    /// Disjunction
    Or(Box<BoundedExpr>, Box<BoundedExpr>),
    /// Negation
    Not(Box<BoundedExpr>),
    /// Implication: a → b
    Implies(Box<BoundedExpr>, Box<BoundedExpr>),
    /// Equality: a == b
    Eq(Box<BoundedExpr>, Box<BoundedExpr>),
}

/// Result of translating an SVA expression to bounded verification IR.
pub struct TranslateResult {
    pub expr: BoundedExpr,
    pub declarations: Vec<String>, // signal@t variable names
}

/// Translator that converts SvaExpr to bounded timestep verification IR.
pub struct SvaTranslator {
    pub bound: u32,
    declarations: HashSet<String>,
}

impl SvaTranslator {
    pub fn new(bound: u32) -> Self {
        Self {
            bound,
            declarations: HashSet::new(),
        }
    }

    /// Translate an SVA expression at a specific timestep.
    pub fn translate(&mut self, expr: &SvaExpr, t: u32) -> BoundedExpr {
        match expr {
            SvaExpr::Signal(name) => {
                let var_name = format!("{}@{}", name, t);
                self.declarations.insert(var_name.clone());
                BoundedExpr::Var(var_name)
            }

            SvaExpr::Const(value, _width) => BoundedExpr::Int(*value as i64),

            SvaExpr::Rose(inner) => {
                if t == 0 {
                    // At t=0, rising edge = signal is high (no prior state)
                    self.translate(inner, 0)
                } else {
                    let current = self.translate(inner, t);
                    let previous = self.translate(inner, t - 1);
                    BoundedExpr::And(
                        Box::new(current),
                        Box::new(BoundedExpr::Not(Box::new(previous))),
                    )
                }
            }

            SvaExpr::Fell(inner) => {
                if t == 0 {
                    BoundedExpr::Not(Box::new(self.translate(inner, 0)))
                } else {
                    let current = self.translate(inner, t);
                    let previous = self.translate(inner, t - 1);
                    BoundedExpr::And(
                        Box::new(BoundedExpr::Not(Box::new(current))),
                        Box::new(previous),
                    )
                }
            }

            SvaExpr::Past(inner, n) => {
                if t >= *n {
                    self.translate(inner, t - n)
                } else {
                    BoundedExpr::Bool(false) // no prior state available
                }
            }

            SvaExpr::And(left, right) => {
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::And(Box::new(l), Box::new(r))
            }

            SvaExpr::Or(left, right) => {
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::Or(Box::new(l), Box::new(r))
            }

            SvaExpr::Not(inner) => {
                let i = self.translate(inner, t);
                BoundedExpr::Not(Box::new(i))
            }

            SvaExpr::Eq(left, right) => {
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::Eq(Box::new(l), Box::new(r))
            }

            SvaExpr::Implication {
                antecedent,
                consequent,
                overlapping,
            } => {
                let ante = self.translate(antecedent, t);
                let cons_t = if *overlapping { t } else { t + 1 };
                let cons = self.translate(consequent, cons_t);
                BoundedExpr::Implies(Box::new(ante), Box::new(cons))
            }

            SvaExpr::Delay { body, min, max } => match max {
                Some(max_val) => {
                    // ##[min:max] body → body@{t+min} ∨ body@{t+min+1} ∨ ... ∨ body@{t+max}
                    let mut result: Option<BoundedExpr> = None;
                    for offset in *min..=*max_val {
                        let step = t + offset;
                        if step > t + self.bound {
                            break;
                        }
                        let b = self.translate(body, step);
                        result = Some(match result {
                            None => b,
                            Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(b)),
                        });
                    }
                    result.unwrap_or(BoundedExpr::Bool(false))
                }
                None => {
                    // ##N body → body@{t+N} (exact delay)
                    self.translate(body, t + min)
                }
            },

            SvaExpr::SEventually(inner) => {
                // s_eventually(body) → body@{t+1} ∨ body@{t+2} ∨ ... ∨ body@{t+bound}
                let mut result: Option<BoundedExpr> = None;
                for offset in 1..=self.bound {
                    let b = self.translate(inner, t + offset);
                    result = Some(match result {
                        None => b,
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(b)),
                    });
                }
                result.unwrap_or(BoundedExpr::Bool(false))
            }
        }
    }

    /// Translate a top-level SVA property: conjoin over all timesteps [0, bound).
    /// This models G(property) — the property must hold at every reachable state.
    pub fn translate_property(&mut self, expr: &SvaExpr) -> TranslateResult {
        let mut result: Option<BoundedExpr> = None;
        for t in 0..self.bound {
            let step = self.translate(expr, t);
            result = Some(match result {
                None => step,
                Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(step)),
            });
        }
        let expr = result.unwrap_or(BoundedExpr::Bool(true));
        let declarations: Vec<String> = self.declarations.iter().cloned().collect();
        TranslateResult {
            expr,
            declarations,
        }
    }
}

/// Count the number of Or-leaves in a BoundedExpr tree.
pub fn count_or_leaves(e: &BoundedExpr) -> usize {
    match e {
        BoundedExpr::Or(left, right) => count_or_leaves(left) + count_or_leaves(right),
        _ => 1,
    }
}

/// Count the number of And-leaves in a BoundedExpr tree.
pub fn count_and_leaves(e: &BoundedExpr) -> usize {
    match e {
        BoundedExpr::And(left, right) => count_and_leaves(left) + count_and_leaves(right),
        _ => 1,
    }
}
