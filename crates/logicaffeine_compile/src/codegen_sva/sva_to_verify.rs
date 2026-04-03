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
    /// Less than: a < b
    Lt(Box<BoundedExpr>, Box<BoundedExpr>),
    /// Greater than: a > b
    Gt(Box<BoundedExpr>, Box<BoundedExpr>),
    /// Less than or equal: a <= b
    Lte(Box<BoundedExpr>, Box<BoundedExpr>),
    /// Greater than or equal: a >= b
    Gte(Box<BoundedExpr>, Box<BoundedExpr>),
    /// Unsupported construct (fail closed, not silently true)
    Unsupported(String),
}

/// Result of translating an SVA expression to bounded verification IR.
pub struct TranslateResult {
    pub expr: BoundedExpr,
    pub declarations: Vec<String>, // signal@t variable names
}

/// Translator that converts SvaExpr to bounded timestep verification IR.
pub struct SvaTranslator {
    pub bound: u32,
    pub(crate) declarations: HashSet<String>,
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
                    // No prior state available — return current value (vacuous identity)
                    // This ensures $stable(sig) ≡ sig == $past(sig,1) at t=0:
                    // $stable@0 = true, $past(sig,1)@0 = sig@0, so sig@0 == sig@0 = true ✓
                    self.translate(inner, t)
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

            SvaExpr::Stable(inner) => {
                // $stable(sig) → sig@t == sig@(t-1)
                // At t=0, no previous state → vacuously stable
                if t == 0 {
                    BoundedExpr::Bool(true)
                } else {
                    let current = self.translate(inner, t);
                    let previous = self.translate(inner, t - 1);
                    BoundedExpr::Eq(Box::new(current), Box::new(previous))
                }
            }

            SvaExpr::Changed(inner) => {
                // $changed(sig) → !(sig@t == sig@(t-1))
                // At t=0, no previous state → vacuously not changed
                if t == 0 {
                    BoundedExpr::Bool(false)
                } else {
                    let current = self.translate(inner, t);
                    let previous = self.translate(inner, t - 1);
                    BoundedExpr::Not(Box::new(BoundedExpr::Eq(
                        Box::new(current),
                        Box::new(previous),
                    )))
                }
            }

            SvaExpr::Nexttime(inner, n) => {
                // nexttime[N](body) → body@(t+N)
                self.translate(inner, t + n)
            }

            SvaExpr::DisableIff { condition, body } => {
                // disable iff (cond) body → ¬cond@t → body@t
                // When disable condition is active, property is vacuously true
                let cond = self.translate(condition, t);
                let prop = self.translate(body, t);
                BoundedExpr::Implies(
                    Box::new(BoundedExpr::Not(Box::new(cond))),
                    Box::new(prop),
                )
            }

            SvaExpr::Repetition { body, min, max } => {
                let effective_max = match max {
                    Some(m) => *m,
                    None => (*min).max(1) + self.bound,
                };
                if *min == effective_max {
                    // Exact repetition [*N]: body@t ∧ body@{t+1} ∧ ... ∧ body@{t+N-1}
                    let mut result: Option<BoundedExpr> = None;
                    for offset in 0..*min {
                        let b = self.translate(body, t + offset);
                        result = Some(match result {
                            None => b,
                            Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(b)),
                        });
                    }
                    result.unwrap_or(BoundedExpr::Bool(true))
                } else {
                    // Range repetition [*min:max]: ∨ over lengths, each a conjunction
                    let mut outer: Option<BoundedExpr> = None;
                    for len in *min..=effective_max {
                        if len > self.bound + t {
                            break;
                        }
                        let mut inner_conj: Option<BoundedExpr> = None;
                        for offset in 0..len {
                            let b = self.translate(body, t + offset);
                            inner_conj = Some(match inner_conj {
                                None => b,
                                Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(b)),
                            });
                        }
                        let conj = inner_conj.unwrap_or(BoundedExpr::Bool(true));
                        outer = Some(match outer {
                            None => conj,
                            Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(conj)),
                        });
                    }
                    outer.unwrap_or(BoundedExpr::Bool(false))
                }
            }

            SvaExpr::SAlways(inner) => {
                // s_always(body) at t → body@t ∧ body@{t+1} ∧ ... ∧ body@{bound-1}
                let remaining = if self.bound > t { self.bound - t } else { 1 };
                let mut result: Option<BoundedExpr> = None;
                for offset in 0..remaining {
                    let b = self.translate(inner, t + offset);
                    result = Some(match result {
                        None => b,
                        Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(b)),
                    });
                }
                result.unwrap_or(BoundedExpr::Bool(true))
            }

            SvaExpr::IfElse { condition, then_expr, else_expr } => {
                // if (C) P else Q → (C@t → P@t) ∧ (¬C@t → Q@t)
                let c = self.translate(condition, t);
                let p = self.translate(then_expr, t);
                let q = self.translate(else_expr, t);
                BoundedExpr::And(
                    Box::new(BoundedExpr::Implies(Box::new(c.clone()), Box::new(p))),
                    Box::new(BoundedExpr::Implies(
                        Box::new(BoundedExpr::Not(Box::new(c))),
                        Box::new(q),
                    )),
                )
            }

            // ── IEEE 1800 Extended (Sprint 1B) ──

            SvaExpr::NotEq(left, right) => {
                // a != b → ¬(a == b) at timestep t
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::Not(Box::new(BoundedExpr::Eq(Box::new(l), Box::new(r))))
            }

            SvaExpr::LessThan(left, right) => {
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::Lt(Box::new(l), Box::new(r))
            }

            SvaExpr::GreaterThan(left, right) => {
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::Gt(Box::new(l), Box::new(r))
            }

            SvaExpr::LessEqual(left, right) => {
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::Lte(Box::new(l), Box::new(r))
            }

            SvaExpr::GreaterEqual(left, right) => {
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::Gte(Box::new(l), Box::new(r))
            }

            SvaExpr::Ternary { condition, then_expr, else_expr } => {
                // cond ? a : b → (cond@t ∧ a@t) ∨ (¬cond@t ∧ b@t)
                let c = self.translate(condition, t);
                let a = self.translate(then_expr, t);
                let b = self.translate(else_expr, t);
                BoundedExpr::Or(
                    Box::new(BoundedExpr::And(Box::new(c.clone()), Box::new(a))),
                    Box::new(BoundedExpr::And(
                        Box::new(BoundedExpr::Not(Box::new(c))),
                        Box::new(b),
                    )),
                )
            }

            SvaExpr::Throughout { signal, sequence } => {
                // sig throughout seq → sig holds at every timestep during seq's span
                // Determine sequence span by examining the sequence structure
                let span = self.sequence_span(sequence);
                let mut result: Option<BoundedExpr> = None;
                // Conjoin signal at every timestep in [t, t+span]
                for offset in 0..=span {
                    let s = self.translate(signal, t + offset);
                    result = Some(match result {
                        None => s,
                        Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(s)),
                    });
                }
                // Also conjoin the sequence itself at t
                let seq = self.translate(sequence, t);
                let sig_conj = result.unwrap_or(BoundedExpr::Bool(true));
                BoundedExpr::And(Box::new(sig_conj), Box::new(seq))
            }

            SvaExpr::Within { inner, outer } => {
                // seq1 within seq2 → inner completes within outer's span
                // Translate outer across its span, inner at each possible start
                let outer_span = self.sequence_span(outer);
                let mut result: Option<BoundedExpr> = None;
                // Outer must hold across its span
                for offset in 0..=outer_span {
                    let o = self.translate(outer, t + offset);
                    result = Some(match result {
                        None => o,
                        Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(o)),
                    });
                }
                // Inner starts somewhere within the outer span
                let inner_at_t = self.translate(inner, t);
                let outer_conj = result.unwrap_or(BoundedExpr::Bool(true));
                BoundedExpr::And(Box::new(outer_conj), Box::new(inner_at_t))
            }

            SvaExpr::FirstMatch(inner) => {
                // first_match(seq) → first matching instance of sequence
                self.translate(inner, t)
            }

            SvaExpr::Intersect { left, right } => {
                // seq1 intersect seq2 → both complete, conjoin at each timestep
                let span = self.sequence_span(left).max(self.sequence_span(right));
                let mut result: Option<BoundedExpr> = None;
                for offset in 0..=span {
                    let l = self.translate(left, t + offset);
                    let r = self.translate(right, t + offset);
                    let both = BoundedExpr::And(Box::new(l), Box::new(r));
                    result = Some(match result {
                        None => both,
                        Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(both)),
                    });
                }
                result.unwrap_or(BoundedExpr::Bool(true))
            }
        }
    }

    /// Estimate the timestep span of a sequence expression (how many cycles it covers).
    fn sequence_span(&self, expr: &SvaExpr) -> u32 {
        match expr {
            SvaExpr::Delay { min, max, body } => {
                let delay_span = max.unwrap_or(*min);
                delay_span + self.sequence_span(body)
            }
            SvaExpr::Repetition { min, max, body } => {
                let rep_count = max.unwrap_or(*min);
                rep_count * self.sequence_span(body).max(1)
            }
            SvaExpr::And(l, r) | SvaExpr::Or(l, r) => {
                self.sequence_span(l).max(self.sequence_span(r))
            }
            SvaExpr::Implication { antecedent, consequent, overlapping } => {
                let ante_span = self.sequence_span(antecedent);
                let cons_span = self.sequence_span(consequent);
                ante_span + cons_span + if *overlapping { 0 } else { 1 }
            }
            _ => 1, // atomic signal = 1 cycle
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

/// Collect all signal names from declarations in a BoundedExpr tree.
fn collect_vars_from_bounded(expr: &BoundedExpr, vars: &mut std::collections::HashSet<String>) {
    match expr {
        BoundedExpr::Var(name) => { vars.insert(name.clone()); }
        BoundedExpr::And(l, r) | BoundedExpr::Or(l, r)
        | BoundedExpr::Implies(l, r) | BoundedExpr::Eq(l, r)
        | BoundedExpr::Lt(l, r) | BoundedExpr::Gt(l, r)
        | BoundedExpr::Lte(l, r) | BoundedExpr::Gte(l, r) => {
            collect_vars_from_bounded(l, vars);
            collect_vars_from_bounded(r, vars);
        }
        BoundedExpr::Not(inner) => collect_vars_from_bounded(inner, vars),
        BoundedExpr::Bool(_) | BoundedExpr::Int(_) | BoundedExpr::Unsupported(_) => {}
    }
}

/// Translate a BoundedExpr (timestep-unrolled) into a VerifyExpr (Z3-ready).
///
/// This is the bridge from the compile crate to the verify crate.
/// Both SVA and FOL translate to BoundedExpr first; this function makes
/// them consumable by the Z3 solver for semantic equivalence checking.
#[cfg(feature = "verification")]
pub fn bounded_to_verify(expr: &BoundedExpr) -> logicaffeine_verify::VerifyExpr {
    use logicaffeine_verify::{VerifyExpr, VerifyOp};
    match expr {
        BoundedExpr::Var(name) => VerifyExpr::Var(name.clone()),
        BoundedExpr::Bool(b) => VerifyExpr::Bool(*b),
        BoundedExpr::Int(i) => VerifyExpr::Int(*i),
        BoundedExpr::And(l, r) => VerifyExpr::binary(
            VerifyOp::And,
            bounded_to_verify(l),
            bounded_to_verify(r),
        ),
        BoundedExpr::Or(l, r) => VerifyExpr::binary(
            VerifyOp::Or,
            bounded_to_verify(l),
            bounded_to_verify(r),
        ),
        BoundedExpr::Not(e) => VerifyExpr::not(bounded_to_verify(e)),
        BoundedExpr::Implies(l, r) => VerifyExpr::binary(
            VerifyOp::Implies,
            bounded_to_verify(l),
            bounded_to_verify(r),
        ),
        BoundedExpr::Eq(l, r) => VerifyExpr::binary(
            VerifyOp::Eq,
            bounded_to_verify(l),
            bounded_to_verify(r),
        ),
        BoundedExpr::Lt(l, r) => VerifyExpr::binary(
            VerifyOp::Lt,
            bounded_to_verify(l),
            bounded_to_verify(r),
        ),
        BoundedExpr::Gt(l, r) => VerifyExpr::binary(
            VerifyOp::Gt,
            bounded_to_verify(l),
            bounded_to_verify(r),
        ),
        BoundedExpr::Lte(l, r) => VerifyExpr::binary(
            VerifyOp::Lte,
            bounded_to_verify(l),
            bounded_to_verify(r),
        ),
        BoundedExpr::Gte(l, r) => VerifyExpr::binary(
            VerifyOp::Gte,
            bounded_to_verify(l),
            bounded_to_verify(r),
        ),
        BoundedExpr::Unsupported(_) => VerifyExpr::Bool(false),
    }
}

/// Extract signal names from a BoundedExpr by collecting all Var names
/// and stripping the @timestep suffix.
pub fn extract_signal_names(result: &TranslateResult) -> Vec<String> {
    let mut signals: HashSet<String> = HashSet::new();
    for decl in &result.declarations {
        if let Some(at_pos) = decl.find('@') {
            signals.insert(decl[..at_pos].to_string());
        } else {
            signals.insert(decl.clone());
        }
    }
    signals.into_iter().collect()
}
