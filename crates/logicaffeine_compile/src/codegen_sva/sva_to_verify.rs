//! SVA → Bounded Verification IR Translation
//!
//! Translates SvaExpr to a bounded timestep model suitable for Z3 equivalence checking.
//! Each signal at timestep t becomes a variable "signal@t".
//! Temporal operators are unrolled to bounded disjunctions/conjunctions.

use super::sva_model::SvaExpr;
use std::collections::{HashMap, HashSet};

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

    // ---- Multi-sorted extensions (SUPERCRUSH S0C) ----

    /// Bitvector constant with explicit width
    BitVecConst { width: u32, value: u64 },
    /// Bitvector variable with known width
    BitVecVar(String, u32),
    /// Bitvector binary operation
    BitVecBinary { op: BitVecBoundedOp, left: Box<BoundedExpr>, right: Box<BoundedExpr> },
    /// Bitvector extraction: operand[high:low]
    BitVecExtract { high: u32, low: u32, operand: Box<BoundedExpr> },
    /// Bitvector concatenation
    BitVecConcat(Box<BoundedExpr>, Box<BoundedExpr>),
    /// Array select: array[index]
    ArraySelect { array: Box<BoundedExpr>, index: Box<BoundedExpr> },
    /// Array store: array[index] := value
    ArrayStore { array: Box<BoundedExpr>, index: Box<BoundedExpr>, value: Box<BoundedExpr> },
    /// Integer arithmetic binary operation
    IntBinary { op: ArithBoundedOp, left: Box<BoundedExpr>, right: Box<BoundedExpr> },
    /// Comparison returning Bool from Int/BV operands
    Comparison { op: CmpBoundedOp, left: Box<BoundedExpr>, right: Box<BoundedExpr> },
    /// Universal quantifier
    ForAll { var: String, sort: BoundedSort, body: Box<BoundedExpr> },
    /// Existential quantifier
    Exists { var: String, sort: BoundedSort, body: Box<BoundedExpr> },
    /// Uninterpreted function application (system functions like $onehot0, $bits, $clog2)
    Apply { name: String, args: Vec<BoundedExpr> },
}

/// Bitvector operations in bounded IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitVecBoundedOp {
    And, Or, Xor, Not, Shl, Shr, AShr, Add, Sub, Mul, ULt, SLt, Eq,
}

/// Arithmetic operations in bounded IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithBoundedOp {
    Add, Sub, Mul, Div,
}

/// Comparison operations in bounded IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpBoundedOp {
    Gt, Lt, Gte, Lte,
}

/// Sort annotation for bounded quantifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedSort {
    Bool,
    Int,
    BitVec(u32),
}

/// A single match endpoint for a sequence expression.
///
/// In IEEE 1800, sequences can have multiple possible match endpoints.
/// For example, `##[1:3] ack` has three possible match points (at offsets 1, 2, 3).
/// Each match has a boolean condition (what must hold) and a length (cycles from start).
///
/// Used by `translate_sequence()` to compute proper sequence-level AND, OR, intersect,
/// first_match, throughout, and within semantics.
#[derive(Debug, Clone)]
pub struct SequenceMatch {
    /// The boolean condition that must hold for this match to occur
    pub condition: BoundedExpr,
    /// Number of cycles from the sequence start tick to the match endpoint
    pub length: u32,
}

/// Result of translating an SVA expression to bounded verification IR.
pub struct TranslateResult {
    pub expr: BoundedExpr,
    pub declarations: Vec<String>, // signal@t variable names
}

/// Result of translating a directive — includes the semantic role.
pub struct DirectiveResult {
    pub expr: BoundedExpr,
    pub declarations: Vec<String>,
    pub role: DirectiveRole,
}

/// Translator that converts SvaExpr to bounded timestep verification IR.
pub struct SvaTranslator {
    pub bound: u32,
    pub(crate) declarations: HashSet<String>,
    /// Local variable bindings: maps variable name to the BoundedExpr captured
    /// at assignment time. When a SequenceAction assigns `v = data_in` at timestep t,
    /// we store `v → translate(data_in, t)`. When LocalVar("v") is later referenced
    /// at any timestep, we return the captured value, not a fresh `v@t`.
    local_bindings: HashMap<String, BoundedExpr>,
    /// Queue timestep: set when entering an implication's consequent to the
    /// antecedent's evaluation timestep. Used by `const'(expr)` to freeze
    /// values at the assertion trigger time rather than the consequent's time.
    queue_timestep: Option<u32>,
}

impl SvaTranslator {
    pub fn new(bound: u32) -> Self {
        Self {
            bound,
            declarations: HashSet::new(),
            local_bindings: HashMap::new(),
            queue_timestep: None,
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
                // Set queue_timestep to the antecedent's time for const' freeze
                let prev_queue = self.queue_timestep;
                self.queue_timestep = Some(t);
                let cons = self.translate(consequent, cons_t);
                self.queue_timestep = prev_queue;
                BoundedExpr::Implies(Box::new(ante), Box::new(cons))
            }

            SvaExpr::Delay { body, min, max } => match max {
                // Unified convention: None = unbounded ($), Some(n) = bounded
                Some(max_val) if max_val == min => {
                    // Exact delay ##N: body@{t+N}
                    self.translate(body, t + min)
                }
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
                    // ##[min:$] body → unbounded, clamp to bound
                    let mut result: Option<BoundedExpr> = None;
                    for offset in *min..=self.bound {
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
                // IEEE 16.9.9: sig throughout seq ≡ (sig[*0:$]) intersect seq
                // The signal must hold at EVERY cycle of the sequence.
                // Desugar: get sequence matches, and for each match at length L,
                // conjoin signal at every tick from t to t+L.
                let seq_matches = self.translate_sequence(sequence, t);
                let mut outer: Option<BoundedExpr> = None;
                for sm in &seq_matches {
                    // Signal must hold at every tick from t to t+sm.length
                    let mut sig_conj: Option<BoundedExpr> = None;
                    for offset in 0..=sm.length {
                        let s = self.translate(signal, t + offset);
                        sig_conj = Some(match sig_conj {
                            None => s,
                            Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(s)),
                        });
                    }
                    let sig_all = sig_conj.unwrap_or(BoundedExpr::Bool(true));
                    // Both signal-at-every-tick AND sequence condition must hold
                    let case = BoundedExpr::And(
                        Box::new(sig_all),
                        Box::new(sm.condition.clone()),
                    );
                    outer = Some(match outer {
                        None => case,
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(case)),
                    });
                }
                outer.unwrap_or(BoundedExpr::Bool(false))
            }

            SvaExpr::Within { inner, outer } => {
                // IEEE 16.9.10: seq1 within seq2
                // Inner must start at or after outer start and end at or before outer end.
                // For each outer match at length L_outer, try each possible inner start
                // offset s in [0, L_outer], get inner matches, and only keep those where
                // inner_start + inner_length <= L_outer.
                let outer_matches = self.translate_sequence(outer, t);
                let mut result: Option<BoundedExpr> = None;
                for om in &outer_matches {
                    // Try starting inner at each offset within the outer's span
                    for inner_start in 0..=om.length {
                        let inner_matches = self.translate_sequence(inner, t + inner_start);
                        for im in &inner_matches {
                            // Inner must end within outer's span
                            if inner_start + im.length <= om.length {
                                let case = BoundedExpr::And(
                                    Box::new(om.condition.clone()),
                                    Box::new(im.condition.clone()),
                                );
                                result = Some(match result {
                                    None => case,
                                    Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(case)),
                                });
                            }
                        }
                    }
                }
                result.unwrap_or(BoundedExpr::Bool(false))
            }

            SvaExpr::FirstMatch(inner) => {
                // IEEE 16.9.8: first_match(seq) → only the earliest-completing match.
                // Get all matches, find the minimum length, and only keep those.
                // For priority encoding: shortest match condition AND NOT any shorter.
                let matches = self.translate_sequence(inner, t);
                if matches.is_empty() {
                    return BoundedExpr::Bool(false);
                }
                let min_length = matches.iter().map(|m| m.length).min().unwrap_or(0);
                let mut result: Option<BoundedExpr> = None;
                for m in &matches {
                    if m.length == min_length {
                        result = Some(match result {
                            None => m.condition.clone(),
                            Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(m.condition.clone())),
                        });
                    }
                }
                result.unwrap_or(BoundedExpr::Bool(false))
            }

            SvaExpr::Intersect { left, right } => {
                // IEEE 16.9.6: intersect requires both sequences to match AND have
                // the SAME match length. Only pairs where left.length == right.length
                // contribute to the result.
                let left_matches = self.translate_sequence(left, t);
                let right_matches = self.translate_sequence(right, t);
                let mut outer: Option<BoundedExpr> = None;
                for lm in &left_matches {
                    for rm in &right_matches {
                        if lm.length == rm.length {
                            // Same length — this pair is valid
                            let both = BoundedExpr::And(
                                Box::new(lm.condition.clone()),
                                Box::new(rm.condition.clone()),
                            );
                            outer = Some(match outer {
                                None => both,
                                Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(both)),
                            });
                        }
                    }
                }
                // If no common lengths exist, the intersect can NEVER match
                outer.unwrap_or(BoundedExpr::Bool(false))
            }

            // ── IEEE 1800 System Functions (Audit) ──

            SvaExpr::OneHot0(inner) => {
                let sig = self.translate(inner, t);
                BoundedExpr::Apply {
                    name: "onehot0".to_string(),
                    args: vec![sig],
                }
            }

            SvaExpr::OneHot(inner) => {
                let sig = self.translate(inner, t);
                BoundedExpr::Apply {
                    name: "onehot".to_string(),
                    args: vec![sig],
                }
            }

            SvaExpr::CountOnes(inner) => {
                let sig = self.translate(inner, t);
                BoundedExpr::Apply {
                    name: "countones".to_string(),
                    args: vec![sig],
                }
            }

            SvaExpr::IsUnknown(_) => {
                // In 2-state formal verification, X/Z don't exist
                BoundedExpr::Bool(false)
            }

            SvaExpr::Sampled(inner) => {
                // In synchronous single-clock formal, $sampled == identity
                self.translate(inner, t)
            }

            SvaExpr::Bits(inner) => {
                let sig = self.translate(inner, t);
                BoundedExpr::Apply {
                    name: "bits".to_string(),
                    args: vec![sig],
                }
            }

            SvaExpr::Clog2(inner) => {
                let val = self.translate(inner, t);
                if let BoundedExpr::Int(n) = &val {
                    let n = *n;
                    let result = if n <= 1 {
                        0i64
                    } else {
                        // IEEE 1800 $clog2: ceiling of log base 2
                        // $clog2(n) = ceil(log2(n))
                        let u = n as u64;
                        // For power of 2: log2(u) exactly
                        // For non-power: round up
                        // u.next_power_of_two() gives the next power of 2 >= u
                        // trailing_zeros gives log2 of that power
                        u.next_power_of_two().trailing_zeros() as i64
                    };
                    BoundedExpr::Int(result)
                } else {
                    BoundedExpr::Apply {
                        name: "clog2".to_string(),
                        args: vec![val],
                    }
                }
            }

            // ── Sprint 13 System Functions ──

            SvaExpr::CountBits(inner, control_chars) => {
                let sig = self.translate(inner, t);
                BoundedExpr::Apply {
                    name: format!("countbits_{}", control_chars.iter().collect::<String>()),
                    args: vec![sig],
                }
            }

            SvaExpr::IsUnbounded(_) => {
                // $isunbounded evaluates to a boolean constant at compile time
                // In bounded model checking, parameters are always bounded
                BoundedExpr::Bool(false)
            }

            // ── Advanced Sequences (Audit) ──

            SvaExpr::GotoRepetition { body, count } => {
                if *count == 0 {
                    BoundedExpr::Bool(true)
                } else if *count > self.bound {
                    BoundedExpr::Bool(false)
                } else {
                    // Disjunction over all C(bound, count) subsets of timestep positions
                    let combos = combinations(self.bound, *count);
                    let mut disj: Option<BoundedExpr> = None;
                    for combo in combos {
                        let mut conj: Option<BoundedExpr> = None;
                        for &pos in &combo {
                            let b = self.translate(body, t + pos);
                            conj = Some(match conj {
                                None => b,
                                Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(b)),
                            });
                        }
                        let term = conj.unwrap_or(BoundedExpr::Bool(true));
                        disj = Some(match disj {
                            None => term,
                            Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(term)),
                        });
                    }
                    disj.unwrap_or(BoundedExpr::Bool(false))
                }
            }

            SvaExpr::NonConsecRepetition { body, min, max } => {
                let effective_max = match max {
                    Some(m) => *m,
                    None => self.bound,
                };
                if *min > self.bound {
                    BoundedExpr::Bool(false)
                } else {
                    let capped_max = effective_max.min(self.bound);
                    let all_positions: Vec<u32> = (1..=self.bound).collect();
                    let mut outer_disj: Option<BoundedExpr> = None;
                    for count in *min..=capped_max {
                        let combos = combinations(self.bound, count);
                        for combo in combos {
                            // body true at chosen positions, false at all others
                            let mut conj: Option<BoundedExpr> = None;
                            for &pos in &all_positions {
                                let b = self.translate(body, t + pos);
                                let term = if combo.contains(&pos) {
                                    b
                                } else {
                                    BoundedExpr::Not(Box::new(b))
                                };
                                conj = Some(match conj {
                                    None => term,
                                    Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(term)),
                                });
                            }
                            let term = conj.unwrap_or(BoundedExpr::Bool(true));
                            outer_disj = Some(match outer_disj {
                                None => term,
                                Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(term)),
                            });
                        }
                    }
                    outer_disj.unwrap_or(BoundedExpr::Bool(false))
                }
            }

            // ── Property Abort Operators (Audit) ──

            SvaExpr::AcceptOn { condition, body } => {
                // accept_on(C) P ≡ C ∨ P
                let c = self.translate(condition, t);
                let p = self.translate(body, t);
                BoundedExpr::Or(Box::new(c), Box::new(p))
            }

            SvaExpr::RejectOn { condition, body } => {
                // reject_on(C) P ≡ ¬C ∧ P
                let c = self.translate(condition, t);
                let p = self.translate(body, t);
                BoundedExpr::And(
                    Box::new(BoundedExpr::Not(Box::new(c))),
                    Box::new(p),
                )
            }

            // ── Property Connectives (Sprint 1, IEEE 16.12.3-8) ──

            SvaExpr::PropertyNot(inner) => {
                let i = self.translate(inner, t);
                BoundedExpr::Not(Box::new(i))
            }

            SvaExpr::PropertyImplies(left, right) => {
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::Implies(Box::new(l), Box::new(r))
            }

            SvaExpr::PropertyIff(left, right) => {
                // p iff q → And(Implies(p, q), Implies(q, p))
                let l = self.translate(left, t);
                let r = self.translate(right, t);
                BoundedExpr::And(
                    Box::new(BoundedExpr::Implies(Box::new(l.clone()), Box::new(r.clone()))),
                    Box::new(BoundedExpr::Implies(Box::new(r), Box::new(l))),
                )
            }

            // ── LTL Temporal Operators (Sprint 2, IEEE 16.12.11-13) ──

            SvaExpr::Always(inner) => {
                // always p → ∀t ∈ [0, bound). p@t (weak: passes if trace ends)
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

            SvaExpr::AlwaysBounded { body, min, max } => {
                // always [m:n] p @ t → ∀i ∈ [m, n]. p@(t+i)
                let effective_max = match max {
                    Some(m) => *m,
                    None => self.bound.saturating_sub(t), // $ clamped to bound
                };
                let mut result: Option<BoundedExpr> = None;
                for i in *min..=effective_max {
                    if t + i >= self.bound + t { break; } // clamp
                    let b = self.translate(body, t + i);
                    result = Some(match result {
                        None => b,
                        Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(b)),
                    });
                }
                result.unwrap_or(BoundedExpr::Bool(true)) // weak: vacuously true if no ticks
            }

            SvaExpr::SAlwaysBounded { body, min, max } => {
                // s_always [m:n] p @ t → ∀i ∈ [m, n]. p@(t+i) (strong: ticks must exist)
                let mut result: Option<BoundedExpr> = None;
                for i in *min..=*max {
                    let b = self.translate(body, t + i);
                    result = Some(match result {
                        None => b,
                        Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(b)),
                    });
                }
                result.unwrap_or(BoundedExpr::Bool(true))
            }

            SvaExpr::EventuallyBounded { body, min, max } => {
                // eventually [m:n] p @ t → ∃i ∈ [m, n]. p@(t+i)
                let mut result: Option<BoundedExpr> = None;
                for i in *min..=*max {
                    let b = self.translate(body, t + i);
                    result = Some(match result {
                        None => b,
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(b)),
                    });
                }
                result.unwrap_or(BoundedExpr::Bool(false))
            }

            SvaExpr::SEventuallyBounded { body, min, max } => {
                // s_eventually [m:n] p @ t → ∃i ∈ [m, min(n, bound)]. p@(t+i)
                let effective_max = match max {
                    Some(m) => *m,
                    None => self.bound.saturating_sub(t),
                };
                let mut result: Option<BoundedExpr> = None;
                for i in *min..=effective_max {
                    let b = self.translate(body, t + i);
                    result = Some(match result {
                        None => b,
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(b)),
                    });
                }
                result.unwrap_or(BoundedExpr::Bool(false))
            }

            SvaExpr::Until { lhs, rhs, strong, inclusive } => {
                // Bounded until encoding:
                // ∃k ∈ [t, t+bound). rhs@k ∧ (∀j ∈ [t, k). lhs@j)
                //   For inclusive (until_with): ∀j ∈ [t, k]. lhs@j (includes k)
                //   For strong: rhs MUST appear within bound
                //   For weak: if no rhs within bound, passes if lhs holds throughout
                let max_k = t + self.bound;
                let mut outer: Option<BoundedExpr> = None;
                for k in t..max_k {
                    let rhs_at_k = self.translate(rhs, k);
                    // lhs holds at all j in [t, k) or [t, k] for inclusive
                    let end = if *inclusive { k + 1 } else { k };
                    let mut lhs_conj: Option<BoundedExpr> = None;
                    for j in t..end {
                        let l = self.translate(lhs, j);
                        lhs_conj = Some(match lhs_conj {
                            None => l,
                            Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(l)),
                        });
                    }
                    let lhs_all = lhs_conj.unwrap_or(BoundedExpr::Bool(true));
                    let case = BoundedExpr::And(Box::new(rhs_at_k), Box::new(lhs_all));
                    outer = Some(match outer {
                        None => case,
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(case)),
                    });
                }
                if !strong {
                    // Weak: also passes if lhs holds at ALL ticks and rhs never appears
                    let mut all_lhs: Option<BoundedExpr> = None;
                    for j in t..max_k {
                        let l = self.translate(lhs, j);
                        all_lhs = Some(match all_lhs {
                            None => l,
                            Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(l)),
                        });
                    }
                    let fallback = all_lhs.unwrap_or(BoundedExpr::Bool(true));
                    outer = Some(match outer {
                        None => fallback,
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(fallback)),
                    });
                }
                outer.unwrap_or(BoundedExpr::Bool(false))
            }

            // ── Sprint 3: Strong/Weak, Advanced Temporal, Sync Abort ──

            SvaExpr::Strong(inner) => {
                // strong(seq): existential — at least one match endpoint MUST exist
                // within bound. Translate as disjunction over sequence match endpoints.
                // If no match exists → property FAILS.
                let matches = self.translate_sequence(inner, t);
                if matches.is_empty() {
                    return BoundedExpr::Bool(false);
                }
                let mut result: Option<BoundedExpr> = None;
                for m in &matches {
                    let cond = m.condition.clone();
                    result = Some(match result {
                        None => cond,
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(cond)),
                    });
                }
                result.unwrap_or(BoundedExpr::Bool(false))
            }

            SvaExpr::Weak(inner) => {
                // weak(seq): if no match endpoint exists within bound, property
                // PASSES (vacuously). Otherwise, at least one match must hold.
                // Translate as: (exists a match) OR (no match possible within bound).
                // In bounded model checking the "no match possible" case is when
                // the trace ends before the sequence can complete — which is
                // captured by allowing vacuous true when all match conditions are false.
                let matches = self.translate_sequence(inner, t);
                if matches.is_empty() {
                    return BoundedExpr::Bool(true);
                }
                // any_match = disjunction of all match conditions
                let mut any_match: Option<BoundedExpr> = None;
                for m in &matches {
                    let cond = m.condition.clone();
                    any_match = Some(match any_match {
                        None => cond,
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(cond)),
                    });
                }
                // weak: if none of the match conditions hold, pass anyway
                // This is equivalent to: any_match OR (NOT any_match) = true
                // BUT that would be trivially true. The correct semantics is:
                // if the sequence CAN complete (all required ticks exist), then
                // at least one match must hold. If it CANNOT complete (bound too
                // small), then pass. We approximate this in bounded BMC by checking
                // whether the max match length exceeds remaining bound.
                let max_length = matches.iter().map(|m| m.length).max().unwrap_or(0);
                if t + max_length > self.bound {
                    // Sequence cannot complete within bound — weak passes
                    BoundedExpr::Bool(true)
                } else {
                    // Sequence can complete — must have at least one match
                    any_match.unwrap_or(BoundedExpr::Bool(true))
                }
            }

            SvaExpr::SNexttime(inner, n) => {
                // s_nexttime[N] p → p@(t+N), strong: t+N must exist within bound
                // IEEE 16.12.10: strong nexttime FAILS if the required timestep
                // is at or beyond the bound (the tick must exist)
                if t + n >= self.bound {
                    BoundedExpr::Bool(false)
                } else {
                    self.translate(inner, t + n)
                }
            }

            SvaExpr::FollowedBy { antecedent, consequent, overlapping } => {
                // seq #-# prop ≡ not (seq |-> not prop) (IEEE p.430)
                // seq #=# prop ≡ not (seq |=> not prop)
                let impl_expr = SvaExpr::Implication {
                    antecedent: antecedent.clone(),
                    consequent: Box::new(SvaExpr::Not(consequent.clone())),
                    overlapping: *overlapping,
                };
                let impl_result = self.translate(&impl_expr, t);
                BoundedExpr::Not(Box::new(impl_result))
            }

            SvaExpr::PropertyCase { expression, items, default } => {
                // case(expr) val: prop; ... → nested if-else
                let expr_val = self.translate(expression, t);
                let mut result = match default {
                    Some(d) => self.translate(d, t),
                    None => BoundedExpr::Bool(true), // no default → vacuously true
                };
                // Build from last to first (nested if-else chain)
                for (vals, prop) in items.iter().rev() {
                    let prop_translated = self.translate(prop, t);
                    // OR of all value matches
                    let mut cond: Option<BoundedExpr> = None;
                    for v in vals {
                        let v_translated = self.translate(v, t);
                        let eq = BoundedExpr::Eq(Box::new(expr_val.clone()), Box::new(v_translated));
                        cond = Some(match cond {
                            None => eq,
                            Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(eq)),
                        });
                    }
                    let condition = cond.unwrap_or(BoundedExpr::Bool(false));
                    // if condition then prop else previous
                    result = BoundedExpr::And(
                        Box::new(BoundedExpr::Implies(Box::new(condition.clone()), Box::new(prop_translated))),
                        Box::new(BoundedExpr::Implies(
                            Box::new(BoundedExpr::Not(Box::new(condition))),
                            Box::new(result),
                        )),
                    );
                }
                result
            }

            SvaExpr::SyncAcceptOn { condition, body } => {
                // sync_accept_on(C) P — like accept_on but C sampled at clock ticks only
                // In single-clock bounded model: same as accept_on
                let c = self.translate(condition, t);
                let p = self.translate(body, t);
                BoundedExpr::Or(Box::new(c), Box::new(p))
            }

            SvaExpr::SyncRejectOn { condition, body } => {
                // sync_reject_on(C) P — like reject_on but C sampled at clock ticks only
                let c = self.translate(condition, t);
                let p = self.translate(body, t);
                BoundedExpr::And(
                    Box::new(BoundedExpr::Not(Box::new(c))),
                    Box::new(p),
                )
            }

            // ── Sprint 5: Sequence-level AND & OR ──

            SvaExpr::SequenceAnd(left, right) => {
                // IEEE 16.9.5 Thread semantics: both start at t, both must match,
                // composite ends at whichever finishes LAST (max endpoint).
                let left_matches = self.translate_sequence(left, t);
                let right_matches = self.translate_sequence(right, t);
                let mut outer: Option<BoundedExpr> = None;
                for lm in &left_matches {
                    for rm in &right_matches {
                        // Both conditions must hold; composite length = max
                        let _composite_length = lm.length.max(rm.length);
                        let both = BoundedExpr::And(
                            Box::new(lm.condition.clone()),
                            Box::new(rm.condition.clone()),
                        );
                        outer = Some(match outer {
                            None => both,
                            Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(both)),
                        });
                    }
                }
                outer.unwrap_or(BoundedExpr::Bool(false))
            }

            SvaExpr::SequenceOr(left, right) => {
                // IEEE 16.9.7 Union semantics: at least one matches,
                // composite match set is the union of both.
                let left_matches = self.translate_sequence(left, t);
                let right_matches = self.translate_sequence(right, t);
                let mut outer: Option<BoundedExpr> = None;
                for m in left_matches.iter().chain(right_matches.iter()) {
                    outer = Some(match outer {
                        None => m.condition.clone(),
                        Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(m.condition.clone())),
                    });
                }
                outer.unwrap_or(BoundedExpr::Bool(false))
            }

            // ── Sprint 7: Assertion Directives ──

            SvaExpr::ImmediateAssert { expression, .. } => {
                // Immediate assert → combinational check at each timestep
                self.translate(expression, t)
            }

            // ── Sprint 13: Complex Data Types ──

            SvaExpr::FieldAccess { signal, field } => {
                let sig = self.translate(signal, t);
                // Field access → create a derived variable name
                match &sig {
                    BoundedExpr::Var(name) => {
                        let field_var = format!("{}.{}", name.split('@').next().unwrap_or(name), field);
                        let var_name = format!("{}@{}", field_var, t);
                        self.declarations.insert(var_name.clone());
                        BoundedExpr::Var(var_name)
                    }
                    _ => sig, // fallback
                }
            }

            SvaExpr::EnumLiteral { value, .. } => {
                // Enum literal → integer constant or uninterpreted
                BoundedExpr::Var(value.clone())
            }

            // ── Sprint 14: Endpoint Methods ──

            SvaExpr::Triggered(name) => {
                let var_name = format!("{}.triggered@{}", name, t);
                self.declarations.insert(var_name.clone());
                BoundedExpr::Var(var_name)
            }

            SvaExpr::Matched(name) => {
                let var_name = format!("{}.matched@{}", name, t);
                self.declarations.insert(var_name.clone());
                BoundedExpr::Var(var_name)
            }

            // ── Sprint 15: Bitwise Operators ──

            SvaExpr::BitAnd(l, r) => {
                let left = self.translate(l, t);
                let right = self.translate(r, t);
                BoundedExpr::BitVecBinary { op: BitVecBoundedOp::And, left: Box::new(left), right: Box::new(right) }
            }

            SvaExpr::BitOr(l, r) => {
                let left = self.translate(l, t);
                let right = self.translate(r, t);
                BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Or, left: Box::new(left), right: Box::new(right) }
            }

            SvaExpr::BitXor(l, r) => {
                let left = self.translate(l, t);
                let right = self.translate(r, t);
                BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Xor, left: Box::new(left), right: Box::new(right) }
            }

            SvaExpr::BitNot(inner) => {
                let i = self.translate(inner, t);
                BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Not, left: Box::new(i.clone()), right: Box::new(i) }
            }

            SvaExpr::ReductionAnd(inner) => {
                let sig = self.translate(inner, t);
                BoundedExpr::Apply { name: "reduction_and".to_string(), args: vec![sig] }
            }

            SvaExpr::ReductionOr(inner) => {
                let sig = self.translate(inner, t);
                BoundedExpr::Apply { name: "reduction_or".to_string(), args: vec![sig] }
            }

            SvaExpr::ReductionXor(inner) => {
                let sig = self.translate(inner, t);
                BoundedExpr::Apply { name: "reduction_xor".to_string(), args: vec![sig] }
            }

            SvaExpr::BitSelect { signal, index } => {
                let s = self.translate(signal, t);
                let i = self.translate(index, t);
                BoundedExpr::ArraySelect { array: Box::new(s), index: Box::new(i) }
            }

            SvaExpr::PartSelect { signal, high, low } => {
                let s = self.translate(signal, t);
                BoundedExpr::BitVecExtract { high: *high, low: *low, operand: Box::new(s) }
            }

            SvaExpr::Concat(items) => {
                if items.len() < 2 {
                    return items.first().map(|i| self.translate(i, t)).unwrap_or(BoundedExpr::Bool(false));
                }
                let mut result = self.translate(&items[0], t);
                for item in &items[1..] {
                    let r = self.translate(item, t);
                    result = BoundedExpr::BitVecConcat(Box::new(result), Box::new(r));
                }
                result
            }

            // ── Sprint 10: Local Variables ──

            SvaExpr::SequenceAction { expression, assignments } => {
                // The expression must hold, and all assignments capture values at this tick.
                // IEEE 16.10: assignments bind local variables to values at this timestep.
                // v = data_in at tick t → v resolves to data_in@t in subsequent references.
                let expr_cond = self.translate(expression, t);
                for (name, rhs) in assignments {
                    let bound_value = self.translate(rhs, t);
                    self.local_bindings.insert(name.clone(), bound_value);
                }
                expr_cond
            }

            SvaExpr::LocalVar(name) => {
                // IEEE 16.10: Local variable reference resolves to the value captured
                // at assignment time, not the current timestep.
                if let Some(bound_value) = self.local_bindings.get(name) {
                    bound_value.clone()
                } else {
                    // Fallback: if no binding exists, create a timestep-specific variable
                    let var_name = format!("{}@{}", name, t);
                    self.declarations.insert(var_name.clone());
                    BoundedExpr::Var(var_name)
                }
            }

            // ── Sprint 18: const' cast ──

            SvaExpr::ConstCast(inner) => {
                // IEEE 16.14.6.1: const'(expr) freezes the value at assertion queue time.
                // In an implication, the queue time is the antecedent's timestep.
                let freeze_t = self.queue_timestep.unwrap_or(t);
                self.translate(inner, freeze_t)
            }

            // ── Sprint 12: Multi-Clock ──
            SvaExpr::Clocked { body, clock, .. } => {
                // In multi-clock BMC, each clock gets its own timestep domain.
                // For now, tag the variables with the clock domain name.
                // This enables downstream multi-clock analysis to distinguish domains.
                let inner = self.translate(body, t);
                // Wrap with clock domain context — variables in this subtree
                // are in the clock domain. We tag this by prefixing declarations.
                for decl in self.declarations.clone() {
                    if !decl.contains("__clk_") {
                        let tagged = format!("__clk_{}__{}", clock, decl);
                        self.declarations.insert(tagged);
                    }
                }
                inner
            }
        }
    }

    /// Translate an SvaExpr as a SEQUENCE, returning all possible match endpoints.
    ///
    /// Unlike `translate()` which returns a single BoundedExpr, this returns the
    /// set of (condition, length) pairs representing all possible matches.
    /// This is the foundation for proper IEEE 1800 sequence-level operators:
    /// - SequenceAnd: both match, composite at max endpoint
    /// - SequenceOr: union of match sets
    /// - Intersect: both match at SAME length
    /// - first_match: only shortest match
    /// - throughout / within: desugared via intersect
    pub fn translate_sequence(&mut self, expr: &SvaExpr, t: u32) -> Vec<SequenceMatch> {
        match expr {
            // --- Delay: ##N or ##[min:max] ---
            SvaExpr::Delay { body, min, max } => {
                match max {
                    // Unified convention: None = unbounded ($), Some(n) = bounded
                    Some(max_val) if max_val == min => {
                        // Exact delay ##N: single offset at min
                        let body_matches = self.translate_sequence(body, t + min);
                        body_matches.into_iter().map(|bm| SequenceMatch {
                            condition: bm.condition,
                            length: min + bm.length,
                        }).collect()
                    }
                    Some(max_val) => {
                        // Range delay ##[min:max]: one possible match per offset
                        let mut matches = Vec::new();
                        for offset in *min..=*max_val {
                            if t + offset > t + self.bound { break; }
                            let body_matches = self.translate_sequence(body, t + offset);
                            for bm in body_matches {
                                matches.push(SequenceMatch {
                                    condition: bm.condition,
                                    length: offset + bm.length,
                                });
                            }
                        }
                        if matches.is_empty() {
                            matches.push(SequenceMatch {
                                condition: BoundedExpr::Bool(false),
                                length: *min,
                            });
                        }
                        matches
                    }
                    None => {
                        // Unbounded delay ##[min:$]: clamp to bound
                        let mut matches = Vec::new();
                        for offset in *min..=self.bound {
                            if t + offset > t + self.bound { break; }
                            let body_matches = self.translate_sequence(body, t + offset);
                            for bm in body_matches {
                                matches.push(SequenceMatch {
                                    condition: bm.condition,
                                    length: offset + bm.length,
                                });
                            }
                        }
                        if matches.is_empty() {
                            matches.push(SequenceMatch {
                                condition: BoundedExpr::Bool(false),
                                length: *min,
                            });
                        }
                        matches
                    }
                }
            }

            // --- Repetition: [*N] or [*min:max] ---
            SvaExpr::Repetition { body, min, max } => {
                let effective_max = match max {
                    Some(m) => *m,
                    None => (*min).max(1) + self.bound,
                };
                let mut all_matches = Vec::new();
                for count in *min..=effective_max {
                    if count > self.bound + 1 { break; }
                    if count == 0 {
                        // [*0] matches immediately (empty sequence)
                        all_matches.push(SequenceMatch {
                            condition: BoundedExpr::Bool(true),
                            length: 0,
                        });
                        continue;
                    }
                    // For count N: body must match N consecutive times.
                    // The existing translate() evaluates body at offsets 0..N.
                    // Length = N - 1 (last match at offset N-1).
                    let total_length = count - 1;
                    if t + total_length > t + self.bound { break; }
                    let mut cond: Option<BoundedExpr> = None;
                    for offset in 0..count {
                        let b = self.translate(body, t + offset);
                        cond = Some(match cond {
                            None => b,
                            Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(b)),
                        });
                    }
                    all_matches.push(SequenceMatch {
                        condition: cond.unwrap_or(BoundedExpr::Bool(true)),
                        length: total_length,
                    });
                }
                if all_matches.is_empty() {
                    all_matches.push(SequenceMatch {
                        condition: BoundedExpr::Bool(false),
                        length: 0,
                    });
                }
                all_matches
            }

            // --- Sequence concatenation via Implication (a ##N b) ---
            // In the parser, `a ##N b` → Implication { ante: a, cons: Delay{b,N,None}, overlapping: true }
            // In sequence context, this is conjunction, not material implication.
            SvaExpr::Implication { antecedent, consequent, overlapping } => {
                let ante_matches = self.translate_sequence(antecedent, t);
                let mut all_matches = Vec::new();
                for am in &ante_matches {
                    let gap = if *overlapping { 0u32 } else { 1u32 };
                    let cons_start = t + am.length + gap;
                    if cons_start > t + self.bound { continue; }
                    let cons_matches = self.translate_sequence(consequent, cons_start);
                    for cm in &cons_matches {
                        let total_length = am.length + gap + cm.length;
                        if total_length > self.bound { continue; }
                        all_matches.push(SequenceMatch {
                            condition: BoundedExpr::And(
                                Box::new(am.condition.clone()),
                                Box::new(cm.condition.clone()),
                            ),
                            length: total_length,
                        });
                    }
                }
                if all_matches.is_empty() {
                    all_matches.push(SequenceMatch {
                        condition: BoundedExpr::Bool(false),
                        length: 0,
                    });
                }
                all_matches
            }

            // --- Goto repetition: [->N] ---
            SvaExpr::GotoRepetition { body, count } => {
                if *count == 0 {
                    return vec![SequenceMatch { condition: BoundedExpr::Bool(true), length: 0 }];
                }
                if *count > self.bound {
                    return vec![SequenceMatch { condition: BoundedExpr::Bool(false), length: 0 }];
                }
                let combos = combinations(self.bound, *count);
                let mut matches = Vec::new();
                for combo in combos {
                    let end_pos = combo.last().copied().unwrap_or(0);
                    let mut conj: Option<BoundedExpr> = None;
                    for &pos in &combo {
                        let b = self.translate(body, t + pos);
                        conj = Some(match conj {
                            None => b,
                            Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(b)),
                        });
                    }
                    matches.push(SequenceMatch {
                        condition: conj.unwrap_or(BoundedExpr::Bool(true)),
                        length: end_pos,
                    });
                }
                matches
            }

            // --- Non-consecutive repetition: [=N] ---
            SvaExpr::NonConsecRepetition { body, min, max } => {
                let effective_max = match max {
                    Some(m) => *m,
                    None => self.bound,
                };
                if *min > self.bound {
                    return vec![SequenceMatch { condition: BoundedExpr::Bool(false), length: 0 }];
                }
                let capped_max = effective_max.min(self.bound);
                let all_positions: Vec<u32> = (1..=self.bound).collect();
                let mut all_matches = Vec::new();
                for count in *min..=capped_max {
                    let combos = combinations(self.bound, count);
                    for combo in combos {
                        let mut conj: Option<BoundedExpr> = None;
                        for &pos in &all_positions {
                            let b = self.translate(body, t + pos);
                            let term = if combo.contains(&pos) {
                                b
                            } else {
                                BoundedExpr::Not(Box::new(b))
                            };
                            conj = Some(match conj {
                                None => term,
                                Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(term)),
                            });
                        }
                        // Non-consecutive can end anywhere within the bound
                        all_matches.push(SequenceMatch {
                            condition: conj.unwrap_or(BoundedExpr::Bool(true)),
                            length: self.bound,
                        });
                    }
                }
                if all_matches.is_empty() {
                    all_matches.push(SequenceMatch {
                        condition: BoundedExpr::Bool(true),
                        length: 0,
                    });
                }
                all_matches
            }

            // --- Strong/Weak: forward to inner ---
            SvaExpr::Strong(inner) | SvaExpr::Weak(inner) => {
                self.translate_sequence(inner, t)
            }

            // --- FirstMatch: only the shortest-length match (IEEE 16.9.8) ---
            SvaExpr::FirstMatch(inner) => {
                let matches = self.translate_sequence(inner, t);
                if matches.is_empty() {
                    return vec![SequenceMatch {
                        condition: BoundedExpr::Bool(false),
                        length: 0,
                    }];
                }
                let min_length = matches.iter().map(|m| m.length).min().unwrap_or(0);
                matches.into_iter().filter(|m| m.length == min_length).collect()
            }

            // --- Intersect: length-matching (IEEE 16.9.6) ---
            SvaExpr::Intersect { left, right } => {
                let left_matches = self.translate_sequence(left, t);
                let right_matches = self.translate_sequence(right, t);
                let mut combined = Vec::new();
                for lm in &left_matches {
                    for rm in &right_matches {
                        if lm.length == rm.length {
                            combined.push(SequenceMatch {
                                condition: BoundedExpr::And(
                                    Box::new(lm.condition.clone()),
                                    Box::new(rm.condition.clone()),
                                ),
                                length: lm.length,
                            });
                        }
                    }
                }
                if combined.is_empty() {
                    combined.push(SequenceMatch {
                        condition: BoundedExpr::Bool(false),
                        length: 0,
                    });
                }
                combined
            }

            // --- SequenceAnd: thread semantics (IEEE 16.9.5) ---
            SvaExpr::SequenceAnd(left, right) => {
                let left_matches = self.translate_sequence(left, t);
                let right_matches = self.translate_sequence(right, t);
                let mut combined = Vec::new();
                for lm in &left_matches {
                    for rm in &right_matches {
                        combined.push(SequenceMatch {
                            condition: BoundedExpr::And(
                                Box::new(lm.condition.clone()),
                                Box::new(rm.condition.clone()),
                            ),
                            length: lm.length.max(rm.length),
                        });
                    }
                }
                if combined.is_empty() {
                    combined.push(SequenceMatch {
                        condition: BoundedExpr::Bool(false),
                        length: 0,
                    });
                }
                combined
            }

            // --- SequenceOr: union semantics (IEEE 16.9.7) ---
            SvaExpr::SequenceOr(left, right) => {
                let mut left_matches = self.translate_sequence(left, t);
                let right_matches = self.translate_sequence(right, t);
                left_matches.extend(right_matches);
                if left_matches.is_empty() {
                    left_matches.push(SequenceMatch {
                        condition: BoundedExpr::Bool(false),
                        length: 0,
                    });
                }
                left_matches
            }

            // --- All other expressions: atomic match at length 0 ---
            _ => {
                vec![SequenceMatch {
                    condition: self.translate(expr, t),
                    length: 0,
                }]
            }
        }
    }

    /// Estimate the timestep span of a sequence expression (how many cycles it covers).
    fn sequence_span(&self, expr: &SvaExpr) -> u32 {
        match expr {
            SvaExpr::Delay { min, max, body } => {
                // None = unbounded ($) → use bound as estimate
                let delay_span = max.unwrap_or(self.bound);
                delay_span + self.sequence_span(body)
            }
            SvaExpr::Repetition { min, max, body } => {
                // None = unbounded ($) → use bound as estimate
                let rep_count = max.unwrap_or((*min).max(1) + self.bound);
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
            SvaExpr::GotoRepetition { count, body } => {
                *count * self.sequence_span(body).max(1)
            }
            SvaExpr::NonConsecRepetition { min, body, .. } => {
                *min * self.sequence_span(body).max(1)
            }
            SvaExpr::AcceptOn { body, .. } | SvaExpr::RejectOn { body, .. } => {
                self.sequence_span(body)
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

    /// Translate a concurrent assertion directive (IEEE 16.14).
    ///
    /// Returns a `DirectiveResult` with the translated expression and the directive's
    /// semantic role (check, constraint, or reachability query).
    pub fn translate_directive(&mut self, directive: &super::sva_model::SvaDirective) -> DirectiveResult {
        use super::sva_model::SvaDirectiveKind;

        // First apply disable_iff if present
        let effective_property = if let Some(ref disable_cond) = directive.disable_iff {
            SvaExpr::DisableIff {
                condition: Box::new(disable_cond.clone()),
                body: Box::new(directive.property.clone()),
            }
        } else {
            directive.property.clone()
        };

        let translated = self.translate_property(&effective_property);

        match directive.kind {
            SvaDirectiveKind::Assert => DirectiveResult {
                expr: translated.expr,
                declarations: translated.declarations,
                role: DirectiveRole::Check,
            },
            SvaDirectiveKind::Assume | SvaDirectiveKind::Restrict => DirectiveResult {
                expr: translated.expr,
                declarations: translated.declarations,
                role: DirectiveRole::Constraint,
            },
            SvaDirectiveKind::Cover => DirectiveResult {
                expr: translated.expr,
                declarations: translated.declarations,
                role: DirectiveRole::Reachability,
            },
            SvaDirectiveKind::CoverSequence => DirectiveResult {
                expr: translated.expr,
                declarations: translated.declarations,
                role: DirectiveRole::ReachabilityMultiple,
            },
        }
    }
}

/// The semantic role of a directive in formal verification.
#[derive(Debug, Clone, PartialEq)]
pub enum DirectiveRole {
    /// Assert: check property holds (negate and check UNSAT)
    Check,
    /// Assume/Restrict: add as solver constraint
    Constraint,
    /// Cover property: check reachability (SAT check, not UNSAT)
    Reachability,
    /// Cover sequence: count ALL matches (multiplicity)
    ReachabilityMultiple,
}

/// Generate all k-element subsets of {1, 2, ..., n}.
/// Used for combinatorial expansion of goto/non-consecutive repetitions.
fn combinations(n: u32, k: u32) -> Vec<Vec<u32>> {
    if k == 0 {
        return vec![vec![]];
    }
    if k > n {
        return vec![];
    }
    let mut result = Vec::new();
    fn helper(start: u32, n: u32, k: u32, current: &mut Vec<u32>, result: &mut Vec<Vec<u32>>) {
        if current.len() == k as usize {
            result.push(current.clone());
            return;
        }
        for i in start..=n {
            current.push(i);
            helper(i + 1, n, k, current, result);
            current.pop();
        }
    }
    helper(1, n, k, &mut Vec::new(), &mut result);
    result
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
        | BoundedExpr::Lte(l, r) | BoundedExpr::Gte(l, r)
        | BoundedExpr::BitVecConcat(l, r) => {
            collect_vars_from_bounded(l, vars);
            collect_vars_from_bounded(r, vars);
        }
        BoundedExpr::Not(inner) => collect_vars_from_bounded(inner, vars),
        BoundedExpr::BitVecVar(name, _) => { vars.insert(name.clone()); }
        BoundedExpr::BitVecBinary { left, right, .. }
        | BoundedExpr::IntBinary { left, right, .. }
        | BoundedExpr::Comparison { left, right, .. } => {
            collect_vars_from_bounded(left, vars);
            collect_vars_from_bounded(right, vars);
        }
        BoundedExpr::BitVecExtract { operand, .. } => collect_vars_from_bounded(operand, vars),
        BoundedExpr::ArraySelect { array, index } => {
            collect_vars_from_bounded(array, vars);
            collect_vars_from_bounded(index, vars);
        }
        BoundedExpr::ArrayStore { array, index, value } => {
            collect_vars_from_bounded(array, vars);
            collect_vars_from_bounded(index, vars);
            collect_vars_from_bounded(value, vars);
        }
        BoundedExpr::ForAll { body, .. } | BoundedExpr::Exists { body, .. } => {
            collect_vars_from_bounded(body, vars);
        }
        BoundedExpr::Apply { args, .. } => {
            for arg in args {
                collect_vars_from_bounded(arg, vars);
            }
        }
        BoundedExpr::Bool(_) | BoundedExpr::Int(_)
        | BoundedExpr::BitVecConst { .. } | BoundedExpr::Unsupported(_) => {}
    }
}

/// Public wrapper for collect_vars_from_bounded (for testing).
pub fn collect_vars_from_bounded_pub(expr: &BoundedExpr, vars: &mut std::collections::HashSet<String>) {
    collect_vars_from_bounded(expr, vars);
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

        // ---- Multi-sorted extensions ----
        BoundedExpr::BitVecConst { width, value } => VerifyExpr::bv_const(*width, *value),
        BoundedExpr::BitVecVar(name, _width) => VerifyExpr::Var(name.clone()),
        BoundedExpr::BitVecBinary { op, left, right } => {
            use logicaffeine_verify::BitVecOp;
            let vop = match op {
                BitVecBoundedOp::And => BitVecOp::And,
                BitVecBoundedOp::Or => BitVecOp::Or,
                BitVecBoundedOp::Xor => BitVecOp::Xor,
                BitVecBoundedOp::Not => BitVecOp::Not,
                BitVecBoundedOp::Shl => BitVecOp::Shl,
                BitVecBoundedOp::Shr => BitVecOp::Shr,
                BitVecBoundedOp::AShr => BitVecOp::AShr,
                BitVecBoundedOp::Add => BitVecOp::Add,
                BitVecBoundedOp::Sub => BitVecOp::Sub,
                BitVecBoundedOp::Mul => BitVecOp::Mul,
                BitVecBoundedOp::ULt => BitVecOp::ULt,
                BitVecBoundedOp::SLt => BitVecOp::SLt,
                BitVecBoundedOp::Eq => BitVecOp::Eq,
            };
            VerifyExpr::bv_binary(vop, bounded_to_verify(left), bounded_to_verify(right))
        }
        BoundedExpr::BitVecExtract { high, low, operand } => VerifyExpr::BitVecExtract {
            high: *high,
            low: *low,
            operand: Box::new(bounded_to_verify(operand)),
        },
        BoundedExpr::BitVecConcat(l, r) => VerifyExpr::BitVecConcat(
            Box::new(bounded_to_verify(l)),
            Box::new(bounded_to_verify(r)),
        ),
        BoundedExpr::ArraySelect { array, index } => VerifyExpr::Select {
            array: Box::new(bounded_to_verify(array)),
            index: Box::new(bounded_to_verify(index)),
        },
        BoundedExpr::ArrayStore { array, index, value } => VerifyExpr::Store {
            array: Box::new(bounded_to_verify(array)),
            index: Box::new(bounded_to_verify(index)),
            value: Box::new(bounded_to_verify(value)),
        },
        BoundedExpr::IntBinary { op, left, right } => {
            let vop = match op {
                ArithBoundedOp::Add => VerifyOp::Add,
                ArithBoundedOp::Sub => VerifyOp::Sub,
                ArithBoundedOp::Mul => VerifyOp::Mul,
                ArithBoundedOp::Div => VerifyOp::Div,
            };
            VerifyExpr::binary(vop, bounded_to_verify(left), bounded_to_verify(right))
        }
        BoundedExpr::Comparison { op, left, right } => {
            let vop = match op {
                CmpBoundedOp::Gt => VerifyOp::Gt,
                CmpBoundedOp::Lt => VerifyOp::Lt,
                CmpBoundedOp::Gte => VerifyOp::Gte,
                CmpBoundedOp::Lte => VerifyOp::Lte,
            };
            VerifyExpr::binary(vop, bounded_to_verify(left), bounded_to_verify(right))
        }
        BoundedExpr::ForAll { var, sort, body } => {
            use logicaffeine_verify::VerifyType;
            let ty = match sort {
                BoundedSort::Bool => VerifyType::Bool,
                BoundedSort::Int => VerifyType::Int,
                BoundedSort::BitVec(w) => VerifyType::BitVector(*w),
            };
            VerifyExpr::forall(vec![(var.clone(), ty)], bounded_to_verify(body))
        }
        BoundedExpr::Exists { var, sort, body } => {
            use logicaffeine_verify::VerifyType;
            let ty = match sort {
                BoundedSort::Bool => VerifyType::Bool,
                BoundedSort::Int => VerifyType::Int,
                BoundedSort::BitVec(w) => VerifyType::BitVector(*w),
            };
            VerifyExpr::exists(vec![(var.clone(), ty)], bounded_to_verify(body))
        }
        BoundedExpr::Apply { name, args } => {
            VerifyExpr::Apply {
                name: name.clone(),
                args: args.iter().map(bounded_to_verify).collect(),
            }
        }
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
