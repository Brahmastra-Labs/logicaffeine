//! FOL → Bounded Verification IR Translation
//!
//! Translates Kripke-lowered LogicExpr to a bounded timestep model.
//! World variables (w0, w1, ...) become timestep indices.
//! Temporal accessibility predicates control the unrolling.

use logicaffeine_language::ast::logic::{LogicExpr, QuantifierKind, TemporalOperator, BinaryTemporalOp, Term, ThematicRole};
use logicaffeine_language::Interner;
use logicaffeine_language::token::TokenType;
use super::sva_to_verify::BoundedExpr;
use super::hw_pipeline::SignalMap;
use std::collections::{HashMap, HashSet};
use logicaffeine_base::Symbol;

/// Translator from Kripke-lowered FOL to bounded timestep model.
pub struct FolTranslator<'a> {
    interner: &'a Interner,
    bound: u32,
    /// Maps world variable symbols to fixed timesteps
    world_map: HashMap<Symbol, u32>,
    /// Accumulated signal declarations
    declarations: HashSet<String>,
    /// Optional signal map for FOL arg → SVA signal name mapping
    signal_map: Option<&'a SignalMap>,
    /// When true, collapse ∀x(P(x) → TruthPred(x)) to just P even without a signal map.
    /// Used by the consistency checker to ensure contradictory specs produce conflicting variables.
    collapse_truth_predicates: bool,
}

impl<'a> FolTranslator<'a> {
    pub fn new(interner: &'a Interner, bound: u32) -> Self {
        Self {
            interner,
            bound,
            world_map: HashMap::new(),
            declarations: HashSet::new(),
            signal_map: None,
            collapse_truth_predicates: false,
        }
    }

    /// Enable truth predicate collapsing for consistency checking.
    pub fn set_collapse_truth_predicates(&mut self, collapse: bool) {
        self.collapse_truth_predicates = collapse;
    }

    /// Set a signal map for translating FOL argument names to SVA signal names.
    pub fn set_signal_map(&mut self, map: &'a SignalMap) {
        self.signal_map = Some(map);
    }

    /// Try to extract an accessibility predicate pattern from a quantifier body.
    ///
    /// For existential: body is `And(Reachable_Temporal(w_source, w_target), actual_body)`
    /// Returns (source_world_symbol, actual_body, is_strictly_future)
    fn extract_accessibility_from_existential<'b>(
        &self,
        body: &'b LogicExpr<'b>,
        quantified_var: Symbol,
    ) -> Option<(Symbol, &'b LogicExpr<'b>, bool)> {
        if let LogicExpr::BinaryOp { left, op, right } = body {
            if matches!(op, TokenType::And) {
                if let LogicExpr::Predicate { name, args, world: None } = *left {
                    let pred_name = self.interner.resolve(*name);
                    if pred_name == "Reachable_Temporal" || pred_name == "Accessible_Temporal" {
                        if args.len() >= 2 {
                            if let (Term::Variable(source), Term::Variable(target)) = (&args[0], &args[1]) {
                                if *target == quantified_var {
                                    let strictly_future = pred_name == "Reachable_Temporal";
                                    return Some((*source, right, strictly_future));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Try to extract an accessibility predicate pattern from a universal quantifier body.
    ///
    /// For universal: body is `Implies(Accessible_Temporal(w_source, w_target), actual_body)`
    /// Returns (source_world_symbol, actual_body, predicate_name)
    fn extract_accessibility_from_universal<'b>(
        &self,
        body: &'b LogicExpr<'b>,
        quantified_var: Symbol,
    ) -> Option<(Symbol, &'b LogicExpr<'b>, &'a str)> {
        if let LogicExpr::BinaryOp { left, op, right } = body {
            if matches!(op, TokenType::If | TokenType::Implies) {
                if let LogicExpr::Predicate { name, args, world: None } = *left {
                    let pred_name = self.interner.resolve(*name);
                    if pred_name == "Accessible_Temporal"
                        || pred_name == "Reachable_Temporal"
                        || pred_name == "Next_Temporal"
                    {
                        if args.len() >= 2 {
                            if let (Term::Variable(source), Term::Variable(target)) = (&args[0], &args[1]) {
                                if *target == quantified_var {
                                    return Some((*source, right, pred_name));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Translate a Kripke-lowered LogicExpr to bounded verification IR.
    pub fn translate(&mut self, expr: &LogicExpr<'_>) -> BoundedExpr {
        match expr {
            // Predicates with world arguments → timestamped variables
            LogicExpr::Predicate { name, args, world } => {
                let pred_name = self.interner.resolve(*name).to_string();

                // Accessibility predicates — evaluate ordering constraint
                if pred_name == "Accessible_Temporal"
                    || pred_name == "Reachable_Temporal"
                    || pred_name == "Next_Temporal"
                {
                    // Extract source and target world timesteps from args
                    if args.len() >= 2 {
                        if let (Term::Variable(source), Term::Variable(target)) = (&args[0], &args[1]) {
                            let source_t = self.world_map.get(source).copied().unwrap_or(0);
                            let target_t = self.world_map.get(target).copied().unwrap_or(0);

                            return match pred_name.as_str() {
                                "Accessible_Temporal" => BoundedExpr::Bool(target_t >= source_t),
                                "Reachable_Temporal" => BoundedExpr::Bool(target_t > source_t),
                                "Next_Temporal" => BoundedExpr::Bool(target_t == source_t + 1),
                                _ => BoundedExpr::Bool(true),
                            };
                        }
                    }
                    // Fallback if args don't match expected structure
                    return BoundedExpr::Bool(true);
                }

                // Regular predicate with world → timestamped variable
                if let Some(w) = world {
                    let timestep = self.world_map.get(w).copied().unwrap_or(0);

                    // Check if the predicate name itself matches a signal declaration
                    if let Some(signal_map) = self.signal_map {
                        if let Some(sva_name) = signal_map.resolve(&pred_name) {
                            let var_name = format!("{}@{}", sva_name, timestep);
                            self.declarations.insert(var_name.clone());
                            return BoundedExpr::Var(var_name);
                        }
                    }

                    if args.is_empty() {
                        let var_name = format!("{}@{}", pred_name, timestep);
                        self.declarations.insert(var_name.clone());
                        return BoundedExpr::Var(var_name);
                    }
                    // Multi-arg predicate: use first arg as signal name
                    if let Some(arg) = args.first() {
                        let arg_name = self.term_to_string(arg);

                        // If signal map has this argument (constant/proper noun),
                        // use the mapped signal name
                        if let Some(signal_map) = self.signal_map {
                            if let Some(sva_name) = signal_map.resolve(&arg_name) {
                                let var_name = format!("{}@{}", sva_name, timestep);
                                self.declarations.insert(var_name.clone());
                                return BoundedExpr::Var(var_name);
                            }
                        }

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
                    // Check for accessibility predicate pattern in body
                    if let Some((source_world, actual_body, pred_kind)) =
                        self.extract_accessibility_from_universal(body, *variable)
                    {
                        let source_t = self.world_map.get(&source_world).copied().unwrap_or(0);

                        // Next_Temporal: exactly one timestep (source + 1)
                        if pred_kind == "Next_Temporal" {
                            let next_t = source_t + 1;
                            self.world_map.insert(*variable, next_t);
                            let step = self.translate(actual_body);
                            self.world_map.remove(variable);
                            return step;
                        }

                        let (start, end) = match pred_kind {
                            "Accessible_Temporal" => (source_t, source_t + self.bound),
                            "Reachable_Temporal" => (source_t + 1, source_t + 1 + self.bound),
                            _ => (0, self.bound),
                        };

                        let mut result: Option<BoundedExpr> = None;
                        for t in start..end {
                            self.world_map.insert(*variable, t);
                            let step = self.translate(actual_body);
                            result = Some(match result {
                                None => step,
                                Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(step)),
                            });
                        }
                        self.world_map.remove(variable);
                        return result.unwrap_or(BoundedExpr::Bool(true));
                    }

                    // Fallback: generic world quantifier — iterate 0..bound
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
                    // Regular variable quantifier: check for signal collapsing pattern
                    // ∀x(Restrictor(x,w) → TruthPredicate(x,w)) → just Restrictor@t
                    // ∀x(Restrictor(x,w) → ¬TruthPredicate(x,w)) → ¬Restrictor@t
                    if self.signal_map.is_some() || self.collapse_truth_predicates {
                        if let LogicExpr::BinaryOp { left, right, op } = body {
                            if matches!(op, TokenType::If | TokenType::Implies) {
                                if self.is_truth_expr(right) {
                                    // When collapsing, use just the restrictor predicate name
                                    // (without variable suffix) for cross-sentence consistency.
                                    let restrictor = if self.signal_map.is_some() {
                                        // With signal map: normal translation (uses mapped name)
                                        self.translate(left)
                                    } else {
                                        // Without signal map (consistency mode): use bare predicate name
                                        self.translate_predicate_bare(left)
                                    };
                                    if matches!(right, LogicExpr::UnaryOp { .. }) {
                                        return BoundedExpr::Not(Box::new(restrictor));
                                    }
                                    return restrictor;
                                }
                            }
                        }
                    }
                    self.translate(body)
                }
            }

            // Existential quantifier over worlds → disjunction over timesteps
            LogicExpr::Quantifier { kind: QuantifierKind::Existential, variable, body, .. } => {
                let var_name = self.interner.resolve(*variable).to_string();
                if var_name.starts_with('w') {
                    // Check for accessibility predicate pattern in body
                    if let Some((source_world, actual_body, strictly_future)) =
                        self.extract_accessibility_from_existential(body, *variable)
                    {
                        let source_t = self.world_map.get(&source_world).copied().unwrap_or(0);
                        let start = if strictly_future { source_t + 1 } else { source_t };
                        let end = start + self.bound;

                        let mut result: Option<BoundedExpr> = None;
                        for t in start..end {
                            self.world_map.insert(*variable, t);
                            let step = self.translate(actual_body);
                            result = Some(match result {
                                None => step,
                                Some(acc) => BoundedExpr::Or(Box::new(acc), Box::new(step)),
                            });
                        }
                        self.world_map.remove(variable);
                        return result.unwrap_or(BoundedExpr::Bool(false));
                    }

                    // Fallback: generic world quantifier — iterate 0..bound
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
                    TokenType::And => {
                        BoundedExpr::And(Box::new(l), Box::new(r))
                    }
                    TokenType::Or => {
                        BoundedExpr::Or(Box::new(l), Box::new(r))
                    }
                    TokenType::If
                    | TokenType::Implies => {
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
                        for _t in 0..self.bound {
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
                        for _t in 0..self.bound {
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
                        self.unroll_until(&l, &r, 0)
                    }
                    BinaryTemporalOp::Release => {
                        self.unroll_release(&l, &r, 0)
                    }
                    BinaryTemporalOp::WeakUntil => {
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

            // Neo-Davidsonian event: extract verb + agent as signal name
            LogicExpr::NeoEvent(data) => {
                let verb_name = self.interner.resolve(data.verb).to_string();
                let timestep = data.world
                    .and_then(|w| self.world_map.get(&w).copied())
                    .unwrap_or(0);

                // Extract agent from roles for signal naming
                let agent_name = data.roles.iter()
                    .find(|(role, _)| matches!(role, ThematicRole::Agent))
                    .map(|(_, term)| self.term_to_string(term));

                if let Some(ref arg_name) = agent_name {
                    if let Some(signal_map) = self.signal_map {
                        if let Some(sva_name) = signal_map.resolve(arg_name) {
                            let var_name = format!("{}@{}", sva_name, timestep);
                            self.declarations.insert(var_name.clone());
                            return BoundedExpr::Var(var_name);
                        }
                        if let Some(sva_name) = signal_map.resolve(&verb_name) {
                            let var_name = format!("{}@{}", sva_name, timestep);
                            self.declarations.insert(var_name.clone());
                            return BoundedExpr::Var(var_name);
                        }
                    }
                    let var_name = format!("{}_{}_@{}", verb_name, arg_name, timestep);
                    self.declarations.insert(var_name.clone());
                    BoundedExpr::Var(var_name)
                } else {
                    if let Some(signal_map) = self.signal_map {
                        if let Some(sva_name) = signal_map.resolve(&verb_name) {
                            let var_name = format!("{}@{}", sva_name, timestep);
                            self.declarations.insert(var_name.clone());
                            return BoundedExpr::Var(var_name);
                        }
                    }
                    let var_name = format!("{}@{}", verb_name, timestep);
                    self.declarations.insert(var_name.clone());
                    BoundedExpr::Var(var_name)
                }
            }

            // Modal: unwrap
            LogicExpr::Modal { operand, .. } => {
                self.translate(operand)
            }

            // Catch-all: fail closed (false, not true) for unhandled constructs.
            // Unsupported constructs must NOT silently become vacuously true.
            _ => BoundedExpr::Bool(false),
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
    fn unroll_until(&self, phi: &BoundedExpr, psi: &BoundedExpr, depth: u32) -> BoundedExpr {
        if depth >= self.bound {
            psi.clone()
        } else {
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
    fn unroll_release(&self, phi: &BoundedExpr, psi: &BoundedExpr, depth: u32) -> BoundedExpr {
        if depth >= self.bound {
            psi.clone()
        } else {
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

    /// Unroll G(φ) (Always) to bounded depth.
    fn unroll_always(&self, phi: &BoundedExpr, depth: u32) -> BoundedExpr {
        if depth >= self.bound {
            phi.clone()
        } else {
            let rest = self.unroll_always(phi, depth + 1);
            BoundedExpr::And(Box::new(phi.clone()), Box::new(rest))
        }
    }

    /// Check if a LogicExpr is a truth predicate (hold/have/valid/active)
    /// or a negation of one. Used for quantifier collapsing:
    /// ∀x(Signal(x) → TruthPred(x)) → Signal
    /// ∀x(Signal(x) → ¬TruthPred(x)) → ¬Signal
    fn is_truth_expr(&self, expr: &LogicExpr<'_>) -> bool {
        match expr {
            LogicExpr::Predicate { name, .. } => {
                let pred_name = self.interner.resolve(*name).to_string();
                is_truth_predicate(&pred_name)
            }
            LogicExpr::NeoEvent(data) => {
                let verb_name = self.interner.resolve(data.verb).to_string();
                is_truth_predicate(&verb_name)
            }
            LogicExpr::UnaryOp { operand, .. } => self.is_truth_expr(operand),
            _ => false,
        }
    }

    /// Translate a restrictor predicate using just its name (no variable suffix).
    /// Used in consistency mode to ensure cross-sentence variable consistency.
    fn translate_predicate_bare(&mut self, expr: &LogicExpr<'_>) -> BoundedExpr {
        match expr {
            LogicExpr::Predicate { name, world, .. } => {
                let pred_name = self.interner.resolve(*name).to_string();
                let timestep = world
                    .and_then(|w| self.world_map.get(&w).copied())
                    .unwrap_or(0);
                let var_name = format!("{}@{}", pred_name, timestep);
                self.declarations.insert(var_name.clone());
                BoundedExpr::Var(var_name)
            }
            _ => self.translate(expr),
        }
    }

    fn term_to_string(&self, term: &Term<'_>) -> String {
        match term {
            Term::Constant(sym) | Term::Variable(sym) => {
                self.interner.resolve(*sym).to_string()
            }
            Term::Function(sym, _) => self.interner.resolve(*sym).to_string(),
            _ => "unknown".to_string(),
        }
    }

    fn term_to_bounded(&self, term: &Term<'_>) -> BoundedExpr {
        let name = self.term_to_string(term);
        BoundedExpr::Var(name)
    }
}

/// Check if a predicate name is a "truth predicate" — a copula-like verb
/// that means "the signal is true" in hardware context.
/// When a signal map is present and the restrictor maps to a signal,
/// truth predicates should be elided (the signal itself carries the boolean).
fn is_truth_predicate(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(lower.as_str(),
        "hold" | "holds" | "have" | "has" | "had"
        | "valid" | "active" | "true" | "assert" | "asserted"
    )
}
