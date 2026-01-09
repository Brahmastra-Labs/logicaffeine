// =============================================================================
// PROOF ENGINE - THE BACKWARD CHAINER
// =============================================================================
// "The machine that crawls backward from the Conclusion to the Axioms."
//
// This is the brain of the proof engine. It takes the InferenceRule definitions
// and actually *hunts* for the proof using Backward Chaining and Unification.
//
// Backward Chaining Strategy:
// 1. Start with the goal we want to prove
// 2. Find rules that conclude our goal
// 3. Recursively prove the premises of those rules
// 4. Build the derivation tree as we succeed

use crate::proof::error::{ProofError, ProofResult};
use crate::proof::unify::{
    apply_subst_to_expr, beta_reduce, compose_substitutions, unify_exprs, unify_pattern,
    unify_terms, Substitution,
};
use crate::proof::{DerivationTree, InferenceRule, ProofExpr, ProofGoal, ProofTerm};

/// Default maximum depth for proof search.
const DEFAULT_MAX_DEPTH: usize = 100;

/// The backward chaining proof engine.
///
/// Searches for proofs by working backwards from the goal, finding rules
/// whose conclusions match, and recursively proving their premises.
pub struct BackwardChainer {
    /// Knowledge base: facts and rules available to the prover.
    knowledge_base: Vec<ProofExpr>,

    /// Maximum proof depth (prevents infinite loops).
    max_depth: usize,

    /// Counter for generating fresh variable names.
    var_counter: usize,
}

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Convert a ProofTerm to a ProofExpr for reduction.
///
/// Terms embed into expressions as atoms or constructors.
fn term_to_expr(term: &ProofTerm) -> ProofExpr {
    match term {
        ProofTerm::Constant(s) => ProofExpr::Atom(s.clone()),
        ProofTerm::Variable(s) => ProofExpr::Atom(s.clone()),
        ProofTerm::BoundVarRef(s) => ProofExpr::Atom(s.clone()),
        ProofTerm::Function(name, args) => {
            // Check if this is a known constructor
            if matches!(name.as_str(), "Zero" | "Succ" | "Nil" | "Cons") {
                ProofExpr::Ctor {
                    name: name.clone(),
                    args: args.iter().map(term_to_expr).collect(),
                }
            } else {
                // Otherwise it's a predicate/function
                ProofExpr::Predicate {
                    name: name.clone(),
                    args: args.clone(),
                    world: None,
                }
            }
        }
        ProofTerm::Group(terms) => {
            // Groups become nested predicates or just the single element
            if terms.len() == 1 {
                term_to_expr(&terms[0])
            } else {
                // Multi-term groups - convert to predicate
                ProofExpr::Predicate {
                    name: "Group".into(),
                    args: terms.clone(),
                    world: None,
                }
            }
        }
    }
}

/// Check if two expressions are structurally equal.
///
/// This is syntactic equality after normalization - no unification needed.
fn exprs_structurally_equal(left: &ProofExpr, right: &ProofExpr) -> bool {
    match (left, right) {
        (ProofExpr::Atom(a), ProofExpr::Atom(b)) => a == b,

        (ProofExpr::Ctor { name: n1, args: a1 }, ProofExpr::Ctor { name: n2, args: a2 }) => {
            n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| exprs_structurally_equal(x, y))
        }

        (
            ProofExpr::Predicate { name: n1, args: a1, .. },
            ProofExpr::Predicate { name: n2, args: a2, .. },
        ) => n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| terms_structurally_equal(x, y)),

        (ProofExpr::Identity(l1, r1), ProofExpr::Identity(l2, r2)) => {
            terms_structurally_equal(l1, l2) && terms_structurally_equal(r1, r2)
        }

        (ProofExpr::And(l1, r1), ProofExpr::And(l2, r2))
        | (ProofExpr::Or(l1, r1), ProofExpr::Or(l2, r2))
        | (ProofExpr::Implies(l1, r1), ProofExpr::Implies(l2, r2))
        | (ProofExpr::Iff(l1, r1), ProofExpr::Iff(l2, r2)) => {
            exprs_structurally_equal(l1, l2) && exprs_structurally_equal(r1, r2)
        }

        (ProofExpr::Not(a), ProofExpr::Not(b)) => exprs_structurally_equal(a, b),

        (
            ProofExpr::ForAll { variable: v1, body: b1 },
            ProofExpr::ForAll { variable: v2, body: b2 },
        )
        | (
            ProofExpr::Exists { variable: v1, body: b1 },
            ProofExpr::Exists { variable: v2, body: b2 },
        ) => v1 == v2 && exprs_structurally_equal(b1, b2),

        (
            ProofExpr::Lambda { variable: v1, body: b1 },
            ProofExpr::Lambda { variable: v2, body: b2 },
        ) => v1 == v2 && exprs_structurally_equal(b1, b2),

        (ProofExpr::App(f1, a1), ProofExpr::App(f2, a2)) => {
            exprs_structurally_equal(f1, f2) && exprs_structurally_equal(a1, a2)
        }

        (
            ProofExpr::TypedVar { name: n1, typename: t1 },
            ProofExpr::TypedVar { name: n2, typename: t2 },
        ) => n1 == n2 && t1 == t2,

        (
            ProofExpr::Fixpoint { name: n1, body: b1 },
            ProofExpr::Fixpoint { name: n2, body: b2 },
        ) => n1 == n2 && exprs_structurally_equal(b1, b2),

        _ => false,
    }
}

/// Check if two terms are structurally equal.
fn terms_structurally_equal(left: &ProofTerm, right: &ProofTerm) -> bool {
    match (left, right) {
        (ProofTerm::Constant(a), ProofTerm::Constant(b)) => a == b,
        (ProofTerm::Variable(a), ProofTerm::Variable(b)) => a == b,
        (ProofTerm::BoundVarRef(a), ProofTerm::BoundVarRef(b)) => a == b,
        (ProofTerm::Function(n1, a1), ProofTerm::Function(n2, a2)) => {
            n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| terms_structurally_equal(x, y))
        }
        (ProofTerm::Group(a1), ProofTerm::Group(a2)) => {
            a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| terms_structurally_equal(x, y))
        }
        _ => false,
    }
}

impl BackwardChainer {
    /// Create a new proof engine with empty knowledge base.
    pub fn new() -> Self {
        Self {
            knowledge_base: Vec::new(),
            max_depth: DEFAULT_MAX_DEPTH,
            var_counter: 0,
        }
    }

    /// Set the maximum proof search depth.
    pub fn set_max_depth(&mut self, depth: usize) {
        self.max_depth = depth;
    }

    /// Get a reference to the knowledge base (for debugging).
    pub fn knowledge_base(&self) -> &[ProofExpr] {
        &self.knowledge_base
    }

    /// Add an axiom/fact/rule to the knowledge base.
    ///
    /// Event semantics are automatically abstracted to simple predicates for efficient proof search.
    pub fn add_axiom(&mut self, expr: ProofExpr) {
        // Pre-process: abstract event semantics to simple predicates
        let abstracted = self.abstract_all_events(&expr);
        // Simplify definite description conjunctions (e.g., butler(butler) ∧ P → P)
        let simplified = self.simplify_definite_description_conjunction(&abstracted);
        self.knowledge_base.push(simplified);
    }

    /// Attempt to prove a goal.
    ///
    /// Returns a derivation tree if successful, explaining how the proof was constructed.
    /// Event semantics in the goal are automatically abstracted (but De Morgan is not applied
    /// to preserve goal pattern matching for reductio strategies).
    pub fn prove(&mut self, goal: ProofExpr) -> ProofResult<DerivationTree> {
        // Pre-process: unify definite descriptions across all axioms
        // This handles Russell's theory of definite descriptions, where multiple
        // "the X" references should refer to the same entity.
        self.unify_definite_descriptions();

        // Pre-process: abstract event semantics in the goal
        // Use abstract_events_only which doesn't apply De Morgan (to preserve ¬∃ pattern)
        let abstracted_goal = self.abstract_events_only(&goal);
        // Simplify definite description conjunctions
        let normalized_goal = self.simplify_definite_description_conjunction(&abstracted_goal);
        self.prove_goal(ProofGoal::new(normalized_goal), 0)
    }

    /// Unify definite descriptions across axioms.
    ///
    /// When multiple axioms contain the same definite description pattern
    /// (e.g., "the barber" creates `∃x ((barber(x) ∧ ∀y (barber(y) → y=x)) ∧ P(x))`),
    /// this function:
    /// 1. Identifies all axioms with the same defining predicate
    /// 2. Extracts the properties attributed to the definite description
    /// 3. Replaces them with a unified Skolem constant and extracted properties
    fn unify_definite_descriptions(&mut self) {
        // Collect definite descriptions by their defining predicate
        let mut definite_descs: std::collections::HashMap<String, Vec<(usize, String, ProofExpr)>> = std::collections::HashMap::new();

        for (idx, axiom) in self.knowledge_base.iter().enumerate() {
            if let Some((pred_name, var_name, property)) = self.extract_definite_description(axiom) {
                definite_descs.entry(pred_name).or_default().push((idx, var_name, property));
            }
        }

        // For each group of definite descriptions with the same predicate
        for (pred_name, descs) in definite_descs {
            if descs.is_empty() {
                continue;
            }

            // Create a unified Skolem constant for this definite description
            let skolem_name = format!("the_{}", pred_name);
            let skolem_const = ProofTerm::Constant(skolem_name.clone());

            // Add the defining property: pred(skolem)
            let defining_fact = ProofExpr::Predicate {
                name: pred_name.clone(),
                args: vec![skolem_const.clone()],
                world: None,
            };
            self.knowledge_base.push(defining_fact);

            // CRITICAL: Add uniqueness constraint: ∀y (pred(y) → y = skolem)
            // This is essential for proofs that assume ∃x pred(x) - they need to
            // unify their Skolem constant with our unified constant.
            let uniqueness = ProofExpr::ForAll {
                variable: "_u".to_string(),
                body: Box::new(ProofExpr::Implies(
                    Box::new(ProofExpr::Predicate {
                        name: pred_name.clone(),
                        args: vec![ProofTerm::Variable("_u".to_string())],
                        world: None,
                    }),
                    Box::new(ProofExpr::Identity(
                        ProofTerm::Variable("_u".to_string()),
                        skolem_const.clone(),
                    )),
                )),
            };
            self.knowledge_base.push(uniqueness);

            // Replace axioms with the extracted properties
            let mut indices_to_remove: Vec<usize> = Vec::new();
            for (idx, var_name, property) in descs {
                // Substitute the original variable with the Skolem constant
                let substituted = self.substitute_term_in_expr(
                    &property,
                    &ProofTerm::Variable(var_name),
                    &skolem_const,
                );
                // Normalize the property (especially for ∀x ¬(P ∧ Q) → ∀x (P → ¬Q))
                let normalized = self.normalize_for_proof(&substituted);
                self.knowledge_base.push(normalized);
                indices_to_remove.push(idx);
            }

            // Remove the original existential axioms (in reverse order to preserve indices)
            indices_to_remove.sort_unstable_by(|a, b| b.cmp(a));
            for idx in indices_to_remove {
                self.knowledge_base.remove(idx);
            }
        }
    }

    /// Normalize an expression for proof search.
    ///
    /// Applies transformations like: ∀x ¬(P ∧ Q) → ∀x (P → ¬Q)
    fn normalize_for_proof(&self, expr: &ProofExpr) -> ProofExpr {
        match expr {
            ProofExpr::ForAll { variable, body } => {
                // Check for pattern: ∀x ¬(P ∧ Q) → ∀x (P → ¬Q)
                if let ProofExpr::Not(inner) = body.as_ref() {
                    if let ProofExpr::And(left, right) = inner.as_ref() {
                        return ProofExpr::ForAll {
                            variable: variable.clone(),
                            body: Box::new(ProofExpr::Implies(
                                Box::new(self.normalize_for_proof(left)),
                                Box::new(ProofExpr::Not(Box::new(self.normalize_for_proof(right)))),
                            )),
                        };
                    }
                }
                ProofExpr::ForAll {
                    variable: variable.clone(),
                    body: Box::new(self.normalize_for_proof(body)),
                }
            }
            ProofExpr::And(left, right) => ProofExpr::And(
                Box::new(self.normalize_for_proof(left)),
                Box::new(self.normalize_for_proof(right)),
            ),
            ProofExpr::Or(left, right) => ProofExpr::Or(
                Box::new(self.normalize_for_proof(left)),
                Box::new(self.normalize_for_proof(right)),
            ),
            ProofExpr::Implies(left, right) => ProofExpr::Implies(
                Box::new(self.normalize_for_proof(left)),
                Box::new(self.normalize_for_proof(right)),
            ),
            ProofExpr::Not(inner) => ProofExpr::Not(Box::new(self.normalize_for_proof(inner))),
            ProofExpr::Exists { variable, body } => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.normalize_for_proof(body)),
            },
            other => other.clone(),
        }
    }

    /// Extract a definite description from an axiom.
    ///
    /// Pattern: ∃x ((P(x) ∧ ∀y (P(y) → y = x)) ∧ Q(x))
    /// Returns: Some((predicate_name, variable_name, Q(x)))
    fn extract_definite_description(&self, expr: &ProofExpr) -> Option<(String, String, ProofExpr)> {
        // Match: ∃x (body)
        let (var, body) = match expr {
            ProofExpr::Exists { variable, body } => (variable.clone(), body.as_ref()),
            _ => return None,
        };

        // Match: (defining_part ∧ property)
        let (defining_part, property) = match body {
            ProofExpr::And(left, right) => (left.as_ref(), right.as_ref().clone()),
            _ => return None,
        };

        // Match defining_part: (P(x) ∧ ∀y (P(y) → y = x))
        let (type_pred, uniqueness) = match defining_part {
            ProofExpr::And(left, right) => (left.as_ref(), right.as_ref()),
            _ => return None,
        };

        // Extract predicate name from P(x)
        let pred_name = match type_pred {
            ProofExpr::Predicate { name, args, .. } if args.len() == 1 => {
                // Verify the arg is our variable
                if let ProofTerm::Variable(v) = &args[0] {
                    if v == &var {
                        name.clone()
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        // Verify uniqueness constraint: ∀y (P(y) → y = x)
        match uniqueness {
            ProofExpr::ForAll { variable: _, body: inner_body } => {
                match inner_body.as_ref() {
                    ProofExpr::Implies(ante, cons) => {
                        // Verify antecedent is P(y)
                        if let ProofExpr::Predicate { name, .. } = ante.as_ref() {
                            if name != &pred_name {
                                return None;
                            }
                        } else {
                            return None;
                        }
                        // Verify consequent is an identity (y = x)
                        if !matches!(cons.as_ref(), ProofExpr::Identity(_, _)) {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }

        Some((pred_name, var, property))
    }

    /// Internal proof search with depth tracking.
    fn prove_goal(&mut self, goal: ProofGoal, depth: usize) -> ProofResult<DerivationTree> {
        // Check depth limit
        if depth > self.max_depth {
            return Err(ProofError::DepthExceeded);
        }

        // PRIORITY: Check for inductive goals FIRST
        // Goals with TypedVar (e.g., n:Nat) require structural induction,
        // not direct unification which would incorrectly ground the variable.
        if let Some((_, typename)) = self.find_typed_var(&goal.target) {
            // For known inductive types, require induction to succeed
            // Falling back to direct matching would incorrectly unify the TypedVar
            let is_known_inductive = matches!(typename.as_str(), "Nat" | "List");

            if let Some(tree) = self.try_structural_induction(&goal, depth)? {
                return Ok(tree);
            }

            // For known inductive types, if induction fails, the proof fails
            // (don't allow incorrect direct unification)
            if is_known_inductive {
                return Err(ProofError::NoProofFound);
            }
            // For unknown types, fall through to other strategies
        }

        // Strategy 0: Reflexivity by computation
        // Try to prove a = b by normalizing both sides
        if let Some(tree) = self.try_reflexivity(&goal)? {
            return Ok(tree);
        }

        // Strategy 1: Direct fact matching
        if let Some(tree) = self.try_match_fact(&goal)? {
            return Ok(tree);
        }

        // Strategy 2: Introduction rules (structural decomposition)
        if let Some(tree) = self.try_intro_rules(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 3: Backward chaining on implications
        if let Some(tree) = self.try_backward_chain(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 3b: Modus Tollens (from P → Q and ¬Q, derive ¬P)
        if let Some(tree) = self.try_modus_tollens(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 4: Universal instantiation
        if let Some(tree) = self.try_universal_inst(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 5: Existential introduction
        if let Some(tree) = self.try_existential_intro(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 5b: Disjunction elimination (disjunctive syllogism)
        if let Some(tree) = self.try_disjunction_elimination(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 5c: Proof by contradiction (reductio ad absurdum)
        // For negation goals, assume the positive and derive contradiction
        if let Some(tree) = self.try_reductio_ad_absurdum(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 5d: Existential elimination from premises
        // Extract witnesses from ∃x P(x) premises and add to context
        if let Some(tree) = self.try_existential_elimination(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 6: Equality rewriting (Leibniz's Law)
        if let Some(tree) = self.try_equality_rewrite(&goal, depth)? {
            return Ok(tree);
        }

        // Strategy 7: Oracle fallback (Z3)
        #[cfg(feature = "verification")]
        if let Some(tree) = self.try_oracle_fallback(&goal)? {
            return Ok(tree);
        }

        // No proof found
        Err(ProofError::NoProofFound)
    }

    // =========================================================================
    // STRATEGY 0: REFLEXIVITY BY COMPUTATION
    // =========================================================================

    /// Try to prove an identity a = b by normalizing both sides.
    ///
    /// If both sides reduce to structurally identical expressions,
    /// the proof is by reflexivity (a = a).
    fn try_reflexivity(&self, goal: &ProofGoal) -> ProofResult<Option<DerivationTree>> {
        if let ProofExpr::Identity(left_term, right_term) = &goal.target {
            // Convert terms to expressions for reduction
            let left_expr = term_to_expr(left_term);
            let right_expr = term_to_expr(right_term);

            // Normalize both sides using full reduction (beta + iota + fix)
            let left_normal = beta_reduce(&left_expr);
            let right_normal = beta_reduce(&right_expr);

            // Check structural equality after normalization
            if exprs_structurally_equal(&left_normal, &right_normal) {
                return Ok(Some(DerivationTree::leaf(
                    goal.target.clone(),
                    InferenceRule::Reflexivity,
                )));
            }
        }
        Ok(None)
    }

    // =========================================================================
    // STRATEGY 1: DIRECT FACT MATCHING
    // =========================================================================

    /// Try to match the goal directly against a fact in the knowledge base.
    fn try_match_fact(&self, goal: &ProofGoal) -> ProofResult<Option<DerivationTree>> {
        // Also check local context
        for fact in goal.context.iter().chain(self.knowledge_base.iter()) {
            if let Ok(subst) = unify_exprs(&goal.target, fact) {
                return Ok(Some(
                    DerivationTree::leaf(goal.target.clone(), InferenceRule::PremiseMatch)
                        .with_substitution(subst),
                ));
            }
        }
        Ok(None)
    }

    // =========================================================================
    // STRATEGY 2: INTRODUCTION RULES
    // =========================================================================

    /// Try introduction rules based on the goal's structure.
    fn try_intro_rules(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        match &goal.target {
            // Conjunction Introduction: To prove A ∧ B, prove A and prove B
            ProofExpr::And(left, right) => {
                let left_goal = ProofGoal::with_context((**left).clone(), goal.context.clone());
                let right_goal = ProofGoal::with_context((**right).clone(), goal.context.clone());

                // Try to prove both sides
                if let (Ok(left_proof), Ok(right_proof)) = (
                    self.prove_goal(left_goal, depth + 1),
                    self.prove_goal(right_goal, depth + 1),
                ) {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::ConjunctionIntro,
                        vec![left_proof, right_proof],
                    )));
                }
            }

            // Disjunction Introduction: To prove A ∨ B, prove A or prove B
            ProofExpr::Or(left, right) => {
                // Try left side first
                let left_goal = ProofGoal::with_context((**left).clone(), goal.context.clone());
                if let Ok(left_proof) = self.prove_goal(left_goal, depth + 1) {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::DisjunctionIntro,
                        vec![left_proof],
                    )));
                }

                // Try right side
                let right_goal = ProofGoal::with_context((**right).clone(), goal.context.clone());
                if let Ok(right_proof) = self.prove_goal(right_goal, depth + 1) {
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::DisjunctionIntro,
                        vec![right_proof],
                    )));
                }
            }

            // Double Negation: To prove ¬¬P, prove P
            ProofExpr::Not(inner) => {
                if let ProofExpr::Not(core) = &**inner {
                    let core_goal = ProofGoal::with_context((**core).clone(), goal.context.clone());
                    if let Ok(core_proof) = self.prove_goal(core_goal, depth + 1) {
                        return Ok(Some(DerivationTree::new(
                            goal.target.clone(),
                            InferenceRule::DoubleNegation,
                            vec![core_proof],
                        )));
                    }
                }
            }

            _ => {}
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 3: BACKWARD CHAINING ON IMPLICATIONS
    // =========================================================================

    /// Try backward chaining: find P → Goal in KB, then prove P.
    fn try_backward_chain(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect implications from KB (we need to clone to avoid borrow issues)
        let implications: Vec<(usize, ProofExpr)> = self
            .knowledge_base
            .iter()
            .enumerate()
            .filter_map(|(idx, expr)| {
                if let ProofExpr::Implies(_, _) = expr {
                    Some((idx, expr.clone()))
                } else {
                    None
                }
            })
            .collect();

        for (idx, impl_expr) in implications {
            if let ProofExpr::Implies(antecedent, consequent) = &impl_expr {
                // Rename variables to avoid capture
                let renamed = self.rename_variables(&impl_expr);
                if let ProofExpr::Implies(ant, con) = renamed {
                    // Try to unify the consequent with our goal
                    if let Ok(subst) = unify_exprs(&goal.target, &con) {
                        // Apply substitution to the antecedent
                        let new_antecedent = apply_subst_to_expr(&ant, &subst);

                        // Try to prove the antecedent
                        let ant_goal =
                            ProofGoal::with_context(new_antecedent, goal.context.clone());

                        if let Ok(ant_proof) = self.prove_goal(ant_goal, depth + 1) {
                            // Success! Build the modus ponens tree
                            let impl_leaf = DerivationTree::leaf(
                                impl_expr.clone(),
                                InferenceRule::PremiseMatch,
                            );

                            return Ok(Some(
                                DerivationTree::new(
                                    goal.target.clone(),
                                    InferenceRule::ModusPonens,
                                    vec![impl_leaf, ant_proof],
                                )
                                .with_substitution(subst),
                            ));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 3b: MODUS TOLLENS
    // =========================================================================

    /// Try Modus Tollens: from P → Q and ¬Q, derive ¬P.
    ///
    /// If the goal is ¬P:
    /// 1. Look for implications P → Q in the KB
    /// 2. Check if ¬Q is known (in KB or context) OR can be proved
    /// 3. If so, derive ¬P by Modus Tollens
    fn try_modus_tollens(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Modus Tollens only applies when goal is a negation: ¬P
        let inner_goal = match &goal.target {
            ProofExpr::Not(inner) => (**inner).clone(),
            _ => return Ok(None),
        };

        // Collect all implications from KB, including those inside ForAll
        let implications: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .flat_map(|expr| {
                match expr {
                    ProofExpr::Implies(_, _) => vec![expr.clone()],
                    ProofExpr::ForAll { body, .. } => {
                        // Extract implications from inside universal quantifiers
                        if let ProofExpr::Implies(_, _) = body.as_ref() {
                            vec![*body.clone()]
                        } else {
                            vec![]
                        }
                    }
                    _ => vec![],
                }
            })
            .collect();

        // Collect all negations from KB and context (for direct matching)
        let negations: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|expr| {
                if let ProofExpr::Not(inner) = expr {
                    Some((**inner).clone())
                } else {
                    None
                }
            })
            .collect();

        // For each implication P → Q
        for impl_expr in &implications {
            if let ProofExpr::Implies(antecedent, consequent) = impl_expr {
                // Check if the antecedent P matches our inner goal (we want to prove ¬P)
                if let Ok(subst) = unify_exprs(&inner_goal, antecedent) {
                    // Apply substitution to the consequent Q
                    let q = apply_subst_to_expr(consequent, &subst);

                    // First, check if ¬Q is directly in our known facts
                    for negated in &negations {
                        if exprs_structurally_equal(negated, &q) {
                            // We have P → Q and ¬Q, so we can derive ¬P
                            let impl_leaf = DerivationTree::leaf(
                                impl_expr.clone(),
                                InferenceRule::PremiseMatch,
                            );
                            let neg_q_leaf = DerivationTree::leaf(
                                ProofExpr::Not(Box::new(q.clone())),
                                InferenceRule::PremiseMatch,
                            );

                            return Ok(Some(
                                DerivationTree::new(
                                    goal.target.clone(),
                                    InferenceRule::ModusTollens,
                                    vec![impl_leaf, neg_q_leaf],
                                )
                                .with_substitution(subst),
                            ));
                        }
                    }

                    // Second, try to prove ¬Q recursively (for chaining)
                    let neg_q_goal = ProofGoal::with_context(
                        ProofExpr::Not(Box::new(q.clone())),
                        goal.context.clone(),
                    );

                    if let Ok(neg_q_proof) = self.prove_goal(neg_q_goal, depth + 1) {
                        // We proved ¬Q, so we can derive ¬P
                        let impl_leaf = DerivationTree::leaf(
                            impl_expr.clone(),
                            InferenceRule::PremiseMatch,
                        );

                        return Ok(Some(
                            DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::ModusTollens,
                                vec![impl_leaf, neg_q_proof],
                            )
                            .with_substitution(subst),
                        ));
                    }
                }
            }
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 4: UNIVERSAL INSTANTIATION
    // =========================================================================

    /// Try universal instantiation: if KB has ∀x.P(x), try to prove P(t) for some term t.
    fn try_universal_inst(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Look for universal quantifiers in KB
        let universals: Vec<(usize, ProofExpr)> = self
            .knowledge_base
            .iter()
            .enumerate()
            .filter_map(|(idx, expr)| {
                if let ProofExpr::ForAll { .. } = expr {
                    Some((idx, expr.clone()))
                } else {
                    None
                }
            })
            .collect();

        for (idx, forall_expr) in universals {
            if let ProofExpr::ForAll { variable, body } = &forall_expr {
                // Rename to avoid capture
                let renamed = self.rename_variables(&forall_expr);
                if let ProofExpr::ForAll {
                    variable: var,
                    body: renamed_body,
                } = renamed
                {
                    // Try to unify the body with our goal
                    if let Ok(subst) = unify_exprs(&goal.target, &renamed_body) {
                        // Check if we found a value for the quantified variable
                        if let Some(witness) = subst.get(&var) {
                            let witness_str = format!("{}", witness);

                            // The universal premise
                            let universal_leaf = DerivationTree::leaf(
                                forall_expr.clone(),
                                InferenceRule::PremiseMatch,
                            );

                            return Ok(Some(
                                DerivationTree::new(
                                    goal.target.clone(),
                                    InferenceRule::UniversalInst(witness_str),
                                    vec![universal_leaf],
                                )
                                .with_substitution(subst),
                            ));
                        }
                    }

                    // Also try: if the body is an implication (∀x(P(x) → Q(x))),
                    // and our goal is Q(t), try to prove P(t)
                    if let ProofExpr::Implies(ant, con) = &*renamed_body {
                        if let Ok(subst) = unify_exprs(&goal.target, con) {
                            // Found a match! Now prove the antecedent
                            let new_antecedent = apply_subst_to_expr(ant, &subst);

                            let ant_goal =
                                ProofGoal::with_context(new_antecedent, goal.context.clone());

                            if let Ok(ant_proof) = self.prove_goal(ant_goal, depth + 1) {
                                // Get the witness from substitution
                                let witness_str = subst
                                    .get(&var)
                                    .map(|t| format!("{}", t))
                                    .unwrap_or_else(|| var.clone());

                                // Build the proof tree
                                let universal_leaf = DerivationTree::leaf(
                                    forall_expr.clone(),
                                    InferenceRule::PremiseMatch,
                                );

                                let inst_node = DerivationTree::new(
                                    apply_subst_to_expr(&renamed_body, &subst),
                                    InferenceRule::UniversalInst(witness_str),
                                    vec![universal_leaf],
                                );

                                return Ok(Some(
                                    DerivationTree::new(
                                        goal.target.clone(),
                                        InferenceRule::ModusPonens,
                                        vec![inst_node, ant_proof],
                                    )
                                    .with_substitution(subst),
                                ));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 5: EXISTENTIAL INTRODUCTION
    // =========================================================================

    /// Try existential introduction: to prove ∃x.P(x), find a witness t and prove P(t).
    fn try_existential_intro(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        if let ProofExpr::Exists { variable, body } = &goal.target {
            // We need to find a witness that makes the body true
            // Try each constant/ground term in our KB as a potential witness
            let witnesses = self.collect_witnesses();

            for witness in witnesses {
                // Create a substitution mapping the variable to the witness
                let mut subst = Substitution::new();
                subst.insert(variable.clone(), witness.clone());

                // Apply substitution to get the instantiated body
                let instantiated = apply_subst_to_expr(body, &subst);

                // Try to prove the instantiated body
                let inst_goal = ProofGoal::with_context(instantiated, goal.context.clone());

                if let Ok(body_proof) = self.prove_goal(inst_goal, depth + 1) {
                    let witness_str = format!("{}", witness);
                    // Extract witness type from body if available, otherwise default to Nat
                    let witness_type = extract_type_from_exists_body(body)
                        .unwrap_or_else(|| "Nat".to_string());
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::ExistentialIntro {
                            witness: witness_str,
                            witness_type,
                        },
                        vec![body_proof],
                    )));
                }
            }
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 5b: DISJUNCTION ELIMINATION (DISJUNCTIVE SYLLOGISM)
    // =========================================================================

    /// Try disjunction elimination: if KB has A ∨ B and ¬A, conclude B (and vice versa).
    ///
    /// Disjunctive syllogism:
    /// - From A ∨ B and ¬A, derive B
    /// - From A ∨ B and ¬B, derive A
    fn try_disjunction_elimination(
        &mut self,
        goal: &ProofGoal,
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect disjunctions from KB and context
        let disjunctions: Vec<(ProofExpr, ProofExpr, ProofExpr)> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|expr| {
                if let ProofExpr::Or(left, right) = expr {
                    Some((expr.clone(), (**left).clone(), (**right).clone()))
                } else {
                    None
                }
            })
            .collect();

        // Collect negations from KB and context
        let negations: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|expr| {
                if let ProofExpr::Not(inner) = expr {
                    Some((**inner).clone())
                } else {
                    None
                }
            })
            .collect();

        // For each disjunction A ∨ B
        for (disj_expr, left, right) in &disjunctions {
            // Check if ¬left is in KB (so right must be true)
            for negated in &negations {
                if exprs_structurally_equal(negated, left) {
                    // We have A ∨ B and ¬A, so B is true
                    // Check if B matches our goal
                    if let Ok(subst) = unify_exprs(&goal.target, right) {
                        let disj_leaf = DerivationTree::leaf(
                            disj_expr.clone(),
                            InferenceRule::PremiseMatch,
                        );
                        let neg_leaf = DerivationTree::leaf(
                            ProofExpr::Not(Box::new(left.clone())),
                            InferenceRule::PremiseMatch,
                        );
                        return Ok(Some(
                            DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::DisjunctionElim,
                                vec![disj_leaf, neg_leaf],
                            )
                            .with_substitution(subst),
                        ));
                    }
                }

                // Check if ¬right is in KB (so left must be true)
                if exprs_structurally_equal(negated, right) {
                    // We have A ∨ B and ¬B, so A is true
                    // Check if A matches our goal
                    if let Ok(subst) = unify_exprs(&goal.target, left) {
                        let disj_leaf = DerivationTree::leaf(
                            disj_expr.clone(),
                            InferenceRule::PremiseMatch,
                        );
                        let neg_leaf = DerivationTree::leaf(
                            ProofExpr::Not(Box::new(right.clone())),
                            InferenceRule::PremiseMatch,
                        );
                        return Ok(Some(
                            DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::DisjunctionElim,
                                vec![disj_leaf, neg_leaf],
                            )
                            .with_substitution(subst),
                        ));
                    }
                }
            }
        }

        Ok(None)
    }

    // =========================================================================
    // STRATEGY 5c: PROOF BY CONTRADICTION (REDUCTIO AD ABSURDUM)
    // =========================================================================

    /// Try proof by contradiction: to prove ¬P, assume P and derive a contradiction.
    ///
    /// This implements reductio ad absurdum:
    /// 1. To prove ¬∃x P(x), assume ∃x P(x), derive contradiction, conclude ¬∃x P(x)
    /// 2. To prove ¬P, assume P, derive contradiction, conclude ¬P
    ///
    /// A contradiction is detected when both Q and ¬Q are derivable.
    fn try_reductio_ad_absurdum(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Only apply to negation goals
        let assumed = match &goal.target {
            ProofExpr::Not(inner) => (**inner).clone(),
            _ => return Ok(None),
        };

        // Aggressive depth limit - reductio is expensive
        if depth > 5 {
            return Ok(None);
        }

        // Special handling for existence negation goals: ¬∃x P(x)
        // This is crucial for paradoxes like the Barber Paradox
        if let ProofExpr::Exists { .. } = &assumed {
            return self.try_existence_negation_proof(&goal, &assumed, depth);
        }

        // For non-existence goals, skip if they contain other quantifiers
        // (to avoid infinite loops with universal instantiation)
        if self.contains_quantifier(&assumed) {
            return Ok(None);
        }

        // Create a temporary context with the assumption added
        let mut extended_context = goal.context.clone();
        extended_context.push(assumed.clone());

        // Also Skolemize existentials from the assumption (but be careful)
        let skolemized = self.skolemize_existential(&assumed);
        for sk in &skolemized {
            extended_context.push(sk.clone());
        }

        // Look for contradiction in the extended context + KB
        // Note: find_contradiction does NOT call prove_goal recursively
        if let Some(contradiction_proof) = self.find_contradiction(&extended_context, depth)? {
            // Found a contradiction! Build the reductio proof
            let assumption_leaf = DerivationTree::leaf(
                assumed.clone(),
                InferenceRule::PremiseMatch,
            );

            return Ok(Some(DerivationTree::new(
                goal.target.clone(),
                InferenceRule::ReductioAdAbsurdum,
                vec![assumption_leaf, contradiction_proof],
            )));
        }

        Ok(None)
    }

    /// Try to prove ¬∃x P(x) by assuming ∃x P(x) and deriving contradiction.
    ///
    /// This is the core strategy for existence paradoxes like the Barber Paradox.
    /// Steps:
    /// 1. Assume ∃x P(x)
    /// 2. Skolemize to get P(c) for fresh constant c
    /// 3. Skolemize KB existentials (definite descriptions) to extract inner structure
    /// 4. Abstract event semantics to simple predicates
    /// 5. Instantiate universal premises with the Skolem constant
    /// 6. Extract uniqueness constraints and derive equalities
    /// 7. Look for contradiction (possibly via case analysis)
    fn try_existence_negation_proof(
        &mut self,
        goal: &ProofGoal,
        assumed_existence: &ProofExpr,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Skolemize the assumed existence: ∃x P(x) → P(c)
        let witness_facts = self.skolemize_existential(assumed_existence);

        if witness_facts.is_empty() {
            return Ok(None);
        }

        // Build extended context with witness facts
        let mut extended_context = goal.context.clone();
        extended_context.push(assumed_existence.clone());

        // Add witness facts, abstracting events
        for fact in &witness_facts {
            let abstracted = self.abstract_all_events(fact);
            if !extended_context.contains(&abstracted) {
                extended_context.push(abstracted);
            }
            if !extended_context.contains(fact) {
                extended_context.push(fact.clone());
            }
        }

        // Extract any Skolem constants from the witness facts
        let mut skolem_constants = self.extract_skolem_constants(&witness_facts);

        // CRITICAL: Skolemize KB existentials to extract definite description structure.
        // Natural language "The barber" creates:
        // ∃y ((barber(y) ∧ ∀z (barber(z) → z = y)) ∧ ∀x ...)
        // We need to Skolemize these to access the inner universals.
        let kb_skolemized = self.skolemize_kb_existentials();
        for fact in &kb_skolemized {
            let abstracted = self.abstract_all_events(fact);
            if !extended_context.contains(&abstracted) {
                extended_context.push(abstracted);
            }
            if !extended_context.contains(fact) {
                extended_context.push(fact.clone());
            }
        }

        // Extract additional Skolem constants from KB
        let kb_skolems = self.extract_skolem_constants(&kb_skolemized);
        for sk in kb_skolems {
            if !skolem_constants.contains(&sk) {
                skolem_constants.push(sk);
            }
        }

        // Also extract unified definite description constants (e.g., "the_barber")
        // These are created by unify_definite_descriptions and should be treated like Skolems
        for expr in &self.knowledge_base {
            self.collect_unified_constants(expr, &mut skolem_constants);
        }

        // Instantiate universal premises with Skolem constants
        let instantiated = self.instantiate_universals_with_constants(
            &extended_context,
            &skolem_constants,
        );
        for inst in &instantiated {
            let abstracted = self.abstract_all_events(inst);
            if !extended_context.contains(&abstracted) {
                extended_context.push(abstracted);
            }
        }

        // Also process KB universals
        let kb_instantiated = self.instantiate_kb_universals_with_constants(&skolem_constants);
        for inst in &kb_instantiated {
            let abstracted = self.abstract_all_events(inst);
            if !extended_context.contains(&abstracted) {
                extended_context.push(abstracted);
            }
        }

        // CRITICAL: Extract uniqueness constraints from definite descriptions
        // and derive equalities between Skolem constants and KB witnesses.
        // This handles Russell's definite descriptions: "The barber" creates
        // ∃y ((barber(y) ∧ ∀z (barber(z) → z = y)) ∧ ...)
        let derived_equalities = self.derive_equalities_from_uniqueness_constraints(
            &extended_context,
            &skolem_constants,
        );

        // Add derived equalities to context
        for eq in &derived_equalities {
            if !extended_context.contains(eq) {
                extended_context.push(eq.clone());
            }
        }

        // Apply derived equalities to substitute terms throughout context
        // This unifies facts about different barbers (sk_0, y, v) into a single entity
        let unified_context = self.apply_equalities_to_context(&extended_context, &derived_equalities);

        // Look for direct contradiction first (in unified context)
        if let Some(contradiction_proof) = self.find_contradiction(&unified_context, depth)? {
            let assumption_leaf = DerivationTree::leaf(
                assumed_existence.clone(),
                InferenceRule::PremiseMatch,
            );

            return Ok(Some(DerivationTree::new(
                goal.target.clone(),
                InferenceRule::ReductioAdAbsurdum,
                vec![assumption_leaf, contradiction_proof],
            )));
        }

        // Try case analysis for self-referential structures (like Barber Paradox)
        if let Some(case_proof) = self.try_case_analysis_contradiction(&unified_context, &skolem_constants, depth)? {
            let assumption_leaf = DerivationTree::leaf(
                assumed_existence.clone(),
                InferenceRule::PremiseMatch,
            );

            return Ok(Some(DerivationTree::new(
                goal.target.clone(),
                InferenceRule::ReductioAdAbsurdum,
                vec![assumption_leaf, case_proof],
            )));
        }

        Ok(None)
    }

    /// Skolemize all existential expressions in the KB.
    ///
    /// This is essential for definite descriptions from natural language.
    /// "The barber" creates `∃y ((barber(y) ∧ ∀z (barber(z) → z = y)) ∧ ...)`.
    /// We Skolemize to extract the inner structure.
    fn skolemize_kb_existentials(&mut self) -> Vec<ProofExpr> {
        let mut results = Vec::new();

        for expr in &self.knowledge_base.clone() {
            if let ProofExpr::Exists { .. } = expr {
                let skolemized = self.skolemize_existential(expr);
                results.extend(skolemized);
            }
        }

        results
    }

    // =========================================================================
    // EQUATIONAL REASONING FOR DEFINITE DESCRIPTIONS
    // =========================================================================

    /// Derive equalities from uniqueness constraints in definite descriptions.
    ///
    /// Given facts like `barber(sk_0)` and uniqueness constraints like
    /// `∀z (barber(z) → z = y)`, derive `sk_0 = y`.
    ///
    /// This is essential for Russell's definite descriptions where
    /// "The barber" creates `∃y ((barber(y) ∧ ∀z (barber(z) → z = y)) ∧ ...)`.
    fn derive_equalities_from_uniqueness_constraints(
        &self,
        context: &[ProofExpr],
        skolem_constants: &[String],
    ) -> Vec<ProofExpr> {
        let mut equalities = Vec::new();

        // Collect all uniqueness constraints from KB and context
        // Pattern: ∀z (P(z) → z = c) where c is a constant/variable
        let uniqueness_constraints = self.extract_uniqueness_constraints(context);

        // For each Skolem constant, check if it satisfies predicates
        // with uniqueness constraints
        for skolem in skolem_constants {
            for (predicate_name, unique_entity) in &uniqueness_constraints {
                // Check if we have predicate(skolem) in context
                let skolem_term = ProofTerm::Constant(skolem.clone());
                let skolem_satisfies_predicate = context.iter().any(|expr| {
                    self.predicate_matches(expr, predicate_name, &skolem_term)
                });

                if skolem_satisfies_predicate {
                    // Derive: skolem = unique_entity
                    let equality = ProofExpr::Identity(
                        skolem_term.clone(),
                        unique_entity.clone(),
                    );
                    if !equalities.contains(&equality) {
                        equalities.push(equality);
                    }

                    // Also add the symmetric version for easier matching
                    let sym_equality = ProofExpr::Identity(
                        unique_entity.clone(),
                        skolem_term.clone(),
                    );
                    if !equalities.contains(&sym_equality) {
                        equalities.push(sym_equality);
                    }
                }
            }
        }

        // Derive transitive equalities: if sk_0 = y and sk_0 = v, then y = v
        let mut transitive_equalities = Vec::new();
        for eq1 in &equalities {
            if let ProofExpr::Identity(t1, t2) = eq1 {
                for eq2 in &equalities {
                    if let ProofExpr::Identity(t3, t4) = eq2 {
                        // If t1 = t2 and t1 = t4, then t2 = t4
                        if t1 == t3 && t2 != t4 {
                            let trans_eq = ProofExpr::Identity(t2.clone(), t4.clone());
                            if !equalities.contains(&trans_eq) && !transitive_equalities.contains(&trans_eq) {
                                transitive_equalities.push(trans_eq);
                            }
                        }
                        // If t1 = t2 and t3 = t1, then t2 = t3
                        if t1 == t4 && t2 != t3 {
                            let trans_eq = ProofExpr::Identity(t2.clone(), t3.clone());
                            if !equalities.contains(&trans_eq) && !transitive_equalities.contains(&trans_eq) {
                                transitive_equalities.push(trans_eq);
                            }
                        }
                    }
                }
            }
        }
        equalities.extend(transitive_equalities);

        equalities
    }

    /// Extract uniqueness constraints from context and KB.
    ///
    /// Looks for patterns like `∀z (P(z) → z = c)` which establish
    /// that c is the unique entity satisfying P.
    fn extract_uniqueness_constraints(&self, context: &[ProofExpr]) -> Vec<(String, ProofTerm)> {
        let mut constraints = Vec::new();

        for expr in context.iter().chain(self.knowledge_base.iter()) {
            self.extract_uniqueness_from_expr(expr, &mut constraints);
        }

        constraints
    }

    /// Recursively extract uniqueness constraints from an expression.
    fn extract_uniqueness_from_expr(&self, expr: &ProofExpr, constraints: &mut Vec<(String, ProofTerm)>) {
        match expr {
            // Direct uniqueness pattern: ∀z (P(z) → z = c)
            ProofExpr::ForAll { variable, body } => {
                if let ProofExpr::Implies(ante, cons) = body.as_ref() {
                    if let ProofExpr::Identity(left, right) = cons.as_ref() {
                        // Check if it's "z = c" where z is the quantified variable
                        let var_term = ProofTerm::Variable(variable.clone());
                        if left == &var_term {
                            // Extract the predicate name from the antecedent
                            if let Some(pred_name) = self.extract_unary_predicate_name(ante, variable) {
                                // right is the unique entity
                                constraints.push((pred_name, right.clone()));
                            }
                        } else if right == &var_term {
                            // Check c = z form
                            if let Some(pred_name) = self.extract_unary_predicate_name(ante, variable) {
                                constraints.push((pred_name, left.clone()));
                            }
                        }
                    }
                }
                // Recurse into body for nested structures
                self.extract_uniqueness_from_expr(body, constraints);
            }

            // Conjunction: extract from both sides
            ProofExpr::And(left, right) => {
                self.extract_uniqueness_from_expr(left, constraints);
                self.extract_uniqueness_from_expr(right, constraints);
            }

            // Existential: extract from body (definite descriptions are wrapped in ∃)
            ProofExpr::Exists { body, .. } => {
                self.extract_uniqueness_from_expr(body, constraints);
            }

            _ => {}
        }
    }

    /// Extract the predicate name from a unary predicate application.
    ///
    /// Given P(z) where z is the variable, returns "P".
    fn extract_unary_predicate_name(&self, expr: &ProofExpr, var: &str) -> Option<String> {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                if args.len() == 1 {
                    if let ProofTerm::Variable(v) = &args[0] {
                        if v == var {
                            return Some(name.clone());
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check if an expression is a predicate with the given name applied to the term.
    fn predicate_matches(&self, expr: &ProofExpr, pred_name: &str, term: &ProofTerm) -> bool {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                name == pred_name && args.len() == 1 && &args[0] == term
            }
            _ => false,
        }
    }

    /// Apply derived equalities to substitute terms throughout context.
    ///
    /// This unifies facts about different entities (sk_0, y, v) by replacing
    /// all occurrences with a canonical representative (the first Skolem constant).
    fn apply_equalities_to_context(
        &self,
        context: &[ProofExpr],
        equalities: &[ProofExpr],
    ) -> Vec<ProofExpr> {
        if equalities.is_empty() {
            return context.to_vec();
        }

        // Build a substitution map from equalities
        // Use the first term as the canonical representative
        let mut substitutions: Vec<(&ProofTerm, &ProofTerm)> = Vec::new();
        for eq in equalities {
            if let ProofExpr::Identity(t1, t2) = eq {
                // Prefer Skolem constants as canonical (they're from our assumption)
                if let ProofTerm::Constant(c) = t1 {
                    if c.starts_with("sk_") {
                        substitutions.push((t2, t1)); // t2 → t1 (Skolem)
                        continue;
                    }
                }
                if let ProofTerm::Constant(c) = t2 {
                    if c.starts_with("sk_") {
                        substitutions.push((t1, t2)); // t1 → t2 (Skolem)
                        continue;
                    }
                }
                // Default: first term is canonical
                substitutions.push((t2, t1));
            }
        }

        // Apply substitutions to each expression in context
        let mut unified_context = Vec::new();
        for expr in context {
            let mut unified = expr.clone();
            for (from, to) in &substitutions {
                unified = self.substitute_term_in_expr(&unified, from, to);
            }
            // Add abstracted version too
            let abstracted = self.abstract_all_events(&unified);
            if !unified_context.contains(&unified) {
                unified_context.push(unified);
            }
            if !unified_context.contains(&abstracted) {
                unified_context.push(abstracted);
            }
        }

        // Also add implications with substituted terms
        // This ensures cyclic implications like P(sk,sk) → ¬P(sk,sk) are in context
        for expr in context {
            if let ProofExpr::ForAll { variable, body } = expr {
                if let ProofExpr::Implies(_, _) = body.as_ref() {
                    // Find any Skolem constants and instantiate
                    for (from, to) in &substitutions {
                        if let ProofTerm::Constant(c) = to {
                            if c.starts_with("sk_") {
                                // Instantiate this universal with the Skolem constant
                                let mut subst = Substitution::new();
                                subst.insert(variable.clone(), (*to).clone());
                                let instantiated = apply_subst_to_expr(body, &subst);
                                let abstracted = self.abstract_all_events(&instantiated);
                                if !unified_context.contains(&abstracted) {
                                    unified_context.push(abstracted);
                                }
                            }
                        }
                    }
                }
            }
        }

        unified_context
    }

    /// Extract Skolem constants from a list of expressions.
    fn extract_skolem_constants(&self, exprs: &[ProofExpr]) -> Vec<String> {
        let mut constants = Vec::new();
        for expr in exprs {
            self.collect_skolem_constants_from_expr(expr, &mut constants);
        }
        constants.sort();
        constants.dedup();
        constants
    }

    /// Helper to collect Skolem constants from an expression.
    fn collect_skolem_constants_from_expr(&self, expr: &ProofExpr, constants: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.collect_skolem_constants_from_term(arg, constants);
                }
            }
            ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) | ProofExpr::Iff(l, r) => {
                self.collect_skolem_constants_from_expr(l, constants);
                self.collect_skolem_constants_from_expr(r, constants);
            }
            ProofExpr::Not(inner) => {
                self.collect_skolem_constants_from_expr(inner, constants);
            }
            ProofExpr::Identity(l, r) => {
                self.collect_skolem_constants_from_term(l, constants);
                self.collect_skolem_constants_from_term(r, constants);
            }
            ProofExpr::NeoEvent { roles, .. } => {
                for (_, term) in roles {
                    self.collect_skolem_constants_from_term(term, constants);
                }
            }
            _ => {}
        }
    }

    /// Helper to collect Skolem constants from a term.
    /// Collect unified definite description constants (e.g., "the_barber")
    /// These are created by unify_definite_descriptions and start with "the_".
    fn collect_unified_constants(&self, expr: &ProofExpr, constants: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    if let ProofTerm::Constant(name) = arg {
                        if name.starts_with("the_") && !constants.contains(name) {
                            constants.push(name.clone());
                        }
                    }
                }
            }
            ProofExpr::And(left, right) | ProofExpr::Or(left, right) |
            ProofExpr::Implies(left, right) | ProofExpr::Iff(left, right) => {
                self.collect_unified_constants(left, constants);
                self.collect_unified_constants(right, constants);
            }
            ProofExpr::Not(inner) => self.collect_unified_constants(inner, constants),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.collect_unified_constants(body, constants);
            }
            ProofExpr::Identity(t1, t2) => {
                if let ProofTerm::Constant(name) = t1 {
                    if name.starts_with("the_") && !constants.contains(name) {
                        constants.push(name.clone());
                    }
                }
                if let ProofTerm::Constant(name) = t2 {
                    if name.starts_with("the_") && !constants.contains(name) {
                        constants.push(name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_skolem_constants_from_term(&self, term: &ProofTerm, constants: &mut Vec<String>) {
        match term {
            ProofTerm::Constant(name) if name.starts_with("sk_") => {
                constants.push(name.clone());
            }
            ProofTerm::Function(_, args) => {
                for arg in args {
                    self.collect_skolem_constants_from_term(arg, constants);
                }
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    self.collect_skolem_constants_from_term(t, constants);
                }
            }
            _ => {}
        }
    }

    /// Instantiate universal quantifiers in the context with given constants.
    fn instantiate_universals_with_constants(
        &self,
        context: &[ProofExpr],
        constants: &[String],
    ) -> Vec<ProofExpr> {
        let mut results = Vec::new();

        for expr in context {
            if let ProofExpr::ForAll { variable, body } = expr {
                for constant in constants {
                    let mut subst = Substitution::new();
                    subst.insert(variable.clone(), ProofTerm::Constant(constant.clone()));
                    let instantiated = apply_subst_to_expr(body, &subst);
                    results.push(instantiated);
                }
            }
        }

        results
    }

    /// Instantiate universal quantifiers in KB with given constants.
    fn instantiate_kb_universals_with_constants(&self, constants: &[String]) -> Vec<ProofExpr> {
        let mut results = Vec::new();

        for expr in &self.knowledge_base {
            if let ProofExpr::ForAll { variable, body } = expr {
                for constant in constants {
                    let mut subst = Substitution::new();
                    subst.insert(variable.clone(), ProofTerm::Constant(constant.clone()));
                    let instantiated = apply_subst_to_expr(body, &subst);
                    results.push(instantiated);
                }
            }
        }

        results
    }

    // =========================================================================
    // CASE ANALYSIS (TERTIUM NON DATUR)
    // =========================================================================

    /// Try case analysis to derive a contradiction.
    ///
    /// For self-referential structures like the Barber Paradox:
    /// - Split on a predicate P(c, c) where c is a Skolem constant
    /// - Case 1: Assume P(c, c), derive ¬P(c, c) → contradiction
    /// - Case 2: Assume ¬P(c, c), derive P(c, c) → contradiction
    /// Either way we get contradiction (law of excluded middle).
    fn try_case_analysis_contradiction(
        &mut self,
        context: &[ProofExpr],
        skolem_constants: &[String],
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Find candidate predicates for case splitting
        // Look for self-referential predicates: P(c, c) where c is a Skolem constant
        let candidates = self.find_case_split_candidates(context, skolem_constants);

        for candidate in candidates {
            // Case 1: Assume the candidate is true
            let mut context_with_pos = context.to_vec();
            if !context_with_pos.contains(&candidate) {
                context_with_pos.push(candidate.clone());
            }

            // Case 2: Assume the candidate is false
            let negated = ProofExpr::Not(Box::new(candidate.clone()));
            let mut context_with_neg = context.to_vec();
            if !context_with_neg.contains(&negated) {
                context_with_neg.push(negated.clone());
            }

            // Try to derive contradiction in both cases
            let case1_contradiction = self.find_contradiction(&context_with_pos, depth)?;
            let case2_contradiction = self.find_contradiction(&context_with_neg, depth)?;

            // If both cases lead to contradiction, we have a proof
            if let (Some(case1_proof), Some(case2_proof)) = (case1_contradiction, case2_contradiction) {
                // Build the case analysis proof tree
                let case1_tree = DerivationTree::new(
                    ProofExpr::Atom("⊥".into()),
                    InferenceRule::PremiseMatch,
                    vec![case1_proof],
                );
                let case2_tree = DerivationTree::new(
                    ProofExpr::Atom("⊥".into()),
                    InferenceRule::PremiseMatch,
                    vec![case2_proof],
                );

                return Ok(Some(DerivationTree::new(
                    ProofExpr::Atom("⊥".into()),
                    InferenceRule::CaseAnalysis {
                        case_formula: format!("{}", candidate),
                    },
                    vec![case1_tree, case2_tree],
                )));
            }
        }

        Ok(None)
    }

    /// Find candidate predicates for case splitting.
    ///
    /// Looks for:
    /// 1. Self-referential predicates: P(c, c) where c is a Skolem constant
    /// 2. Predicates that appear in contradictory implications: P → ¬P and ¬P → P
    fn find_case_split_candidates(
        &self,
        context: &[ProofExpr],
        skolem_constants: &[String],
    ) -> Vec<ProofExpr> {
        let mut candidates = Vec::new();

        // Strategy 1: Find self-referential predicates P(c, c)
        for expr in context {
            if let ProofExpr::Predicate { name, args, world } = expr {
                // Check if it's a binary predicate with the same Skolem constant twice
                if args.len() == 2 {
                    if let (ProofTerm::Constant(c1), ProofTerm::Constant(c2)) = (&args[0], &args[1]) {
                        if c1 == c2 && skolem_constants.contains(c1) {
                            candidates.push(expr.clone());
                        }
                    }
                }
            }
        }

        // Strategy 2: Find predicates involved in cyclic implications
        // Look for patterns like: (P → ¬P) ∧ (¬P → P)
        let implications: Vec<(ProofExpr, ProofExpr)> = context.iter()
            .chain(self.knowledge_base.iter())
            .filter_map(|e| {
                if let ProofExpr::Implies(ante, cons) = e {
                    Some(((**ante).clone(), (**cons).clone()))
                } else {
                    None
                }
            })
            .collect();

        for (ante, cons) in &implications {
            // Check for P → ¬P pattern
            if let ProofExpr::Not(inner) = cons {
                if exprs_structurally_equal(ante, inner) {
                    // Found P → ¬P, check if ¬P → P also exists
                    let neg_ante = ProofExpr::Not(Box::new(ante.clone()));
                    for (a2, c2) in &implications {
                        if exprs_structurally_equal(a2, &neg_ante) && exprs_structurally_equal(c2, ante) {
                            // Found the cyclic pair - ante is a good candidate
                            if !candidates.contains(ante) {
                                candidates.push(ante.clone());
                            }
                        }
                    }
                }
            }
        }

        // Strategy 3: Generate self-referential predicates for Skolem constants
        // For each Skolem constant sk_N, look for predicates P and create P(sk_N, sk_N)
        for const_name in skolem_constants {
            // Look for action predicates in implications
            for expr in context.iter().chain(self.knowledge_base.iter()) {
                if let ProofExpr::Implies(ante, cons) = expr {
                    // Extract predicate names from consequences
                    self.extract_predicate_template(cons, const_name, &mut candidates);
                }
            }
        }

        candidates
    }

    /// Extract a predicate template and instantiate with a Skolem constant.
    fn extract_predicate_template(
        &self,
        expr: &ProofExpr,
        skolem: &str,
        candidates: &mut Vec<ProofExpr>,
    ) {
        match expr {
            ProofExpr::Predicate { name, args, world } if args.len() == 2 => {
                // Create a self-referential version: P(sk, sk)
                let self_ref = ProofExpr::Predicate {
                    name: name.clone(),
                    args: vec![
                        ProofTerm::Constant(skolem.to_string()),
                        ProofTerm::Constant(skolem.to_string()),
                    ],
                    world: world.clone(),
                };
                if !candidates.contains(&self_ref) {
                    candidates.push(self_ref);
                }
            }
            ProofExpr::Not(inner) => {
                self.extract_predicate_template(inner, skolem, candidates);
            }
            ProofExpr::NeoEvent { verb, .. } => {
                // Create abstracted predicate version
                let self_ref = ProofExpr::Predicate {
                    name: verb.to_lowercase(),
                    args: vec![
                        ProofTerm::Constant(skolem.to_string()),
                        ProofTerm::Constant(skolem.to_string()),
                    ],
                    world: None,
                };
                if !candidates.contains(&self_ref) {
                    candidates.push(self_ref);
                }
            }
            _ => {}
        }
    }

    // =========================================================================
    // STRATEGY 5d: EXISTENTIAL ELIMINATION
    // =========================================================================

    /// Try to eliminate existential quantifiers from premises.
    ///
    /// For each ∃x P(x) in the KB or context:
    /// 1. Generate a fresh Skolem constant c
    /// 2. Add P(c) to the context
    /// 3. Abstract any event semantics to simple predicates
    /// 4. Try to prove the goal with the extended context
    fn try_existential_elimination(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Depth guard to prevent infinite loops
        if depth > 8 {
            return Ok(None);
        }

        // Find existential expressions in KB and context
        let existentials: Vec<ProofExpr> = self.knowledge_base.iter()
            .chain(goal.context.iter())
            .filter(|e| matches!(e, ProofExpr::Exists { .. }))
            .cloned()
            .collect();

        if existentials.is_empty() {
            return Ok(None);
        }

        // Try eliminating each existential
        for exist_expr in existentials {
            // Skolemize to get witness facts
            let witness_facts = self.skolemize_existential(&exist_expr);

            if witness_facts.is_empty() {
                continue;
            }

            // Abstract event semantics in witness facts
            let abstracted_facts: Vec<ProofExpr> = witness_facts.iter()
                .map(|f| self.abstract_all_events(f))
                .collect();

            // Build extended context with witness facts
            let mut extended_context = goal.context.clone();
            for fact in &abstracted_facts {
                if !extended_context.contains(fact) {
                    extended_context.push(fact.clone());
                }
            }

            // Also add the original witness facts (in case abstraction changes things)
            for fact in &witness_facts {
                if !extended_context.contains(fact) {
                    extended_context.push(fact.clone());
                }
            }

            // Try to prove the goal with the extended context
            let extended_goal = ProofGoal::with_context(goal.target.clone(), extended_context);

            // Use a fresh engine to avoid polluting our KB
            // But we need to be careful about depth to prevent loops
            if let Ok(inner_proof) = self.prove_goal(extended_goal, depth + 1) {
                // Build proof tree with existential elimination
                let witness_name = if let ProofExpr::Exists { variable, .. } = &exist_expr {
                    variable.clone()
                } else {
                    "witness".to_string()
                };

                return Ok(Some(DerivationTree::new(
                    goal.target.clone(),
                    InferenceRule::ExistentialElim { witness: witness_name },
                    vec![inner_proof],
                )));
            }
        }

        Ok(None)
    }

    /// Check if an expression contains quantifiers.
    fn contains_quantifier(&self, expr: &ProofExpr) -> bool {
        match expr {
            ProofExpr::ForAll { .. } | ProofExpr::Exists { .. } => true,
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => self.contains_quantifier(l) || self.contains_quantifier(r),
            ProofExpr::Not(inner) => self.contains_quantifier(inner),
            _ => false,
        }
    }

    /// Skolemize an existential expression.
    ///
    /// Given ∃x P(x), introduce a fresh Skolem constant c and return P(c).
    /// For nested structures like ∃x((type(x) ∧ unique(x)) ∧ prop(x)),
    /// we extract the predicates with the Skolem constant.
    fn skolemize_existential(&mut self, expr: &ProofExpr) -> Vec<ProofExpr> {
        let mut results = Vec::new();

        if let ProofExpr::Exists { variable, body } = expr {
            // Generate a fresh Skolem constant
            let skolem = format!("sk_{}", self.fresh_var());

            // Apply substitution to the body
            let mut subst = Substitution::new();
            subst.insert(variable.clone(), ProofTerm::Constant(skolem.clone()));

            let instantiated = apply_subst_to_expr(body, &subst);

            // Flatten conjunctions into separate facts
            self.flatten_conjunction(&instantiated, &mut results);

            // Handle nested existentials in the result
            let mut i = 0;
            while i < results.len() {
                if let ProofExpr::Exists { .. } = &results[i] {
                    let nested = results.remove(i);
                    let nested_skolem = self.skolemize_existential(&nested);
                    results.extend(nested_skolem);
                } else {
                    i += 1;
                }
            }
        }

        results
    }

    /// Flatten a conjunction into a list of its components.
    fn flatten_conjunction(&self, expr: &ProofExpr, results: &mut Vec<ProofExpr>) {
        match expr {
            ProofExpr::And(left, right) => {
                self.flatten_conjunction(left, results);
                self.flatten_conjunction(right, results);
            }
            other => results.push(other.clone()),
        }
    }

    // =========================================================================
    // DEFINITE DESCRIPTION SIMPLIFICATION
    // =========================================================================

    /// Check if a predicate is a tautological identity check: name(name)
    /// This occurs when parsing "the butler" creates butler(butler)
    fn is_tautological_identity(&self, expr: &ProofExpr) -> bool {
        if let ProofExpr::Predicate { name, args, .. } = expr {
            args.len() == 1 && matches!(
                &args[0],
                ProofTerm::Constant(c) | ProofTerm::BoundVarRef(c) | ProofTerm::Variable(c) if c == name
            )
        } else {
            false
        }
    }

    /// Simplify conjunction by removing tautological identity predicates.
    /// (butler(butler) ∧ P) → P when butler is a constant
    fn simplify_definite_description_conjunction(&self, expr: &ProofExpr) -> ProofExpr {
        match expr {
            ProofExpr::And(left, right) => {
                // First simplify children
                let left_simplified = self.simplify_definite_description_conjunction(left);
                let right_simplified = self.simplify_definite_description_conjunction(right);

                // Remove tautological identities from the conjunction
                if self.is_tautological_identity(&left_simplified) {
                    return right_simplified;
                }
                if self.is_tautological_identity(&right_simplified) {
                    return left_simplified;
                }

                ProofExpr::And(
                    Box::new(left_simplified),
                    Box::new(right_simplified),
                )
            }
            ProofExpr::Or(left, right) => ProofExpr::Or(
                Box::new(self.simplify_definite_description_conjunction(left)),
                Box::new(self.simplify_definite_description_conjunction(right)),
            ),
            ProofExpr::Implies(left, right) => ProofExpr::Implies(
                Box::new(self.simplify_definite_description_conjunction(left)),
                Box::new(self.simplify_definite_description_conjunction(right)),
            ),
            ProofExpr::Iff(left, right) => ProofExpr::Iff(
                Box::new(self.simplify_definite_description_conjunction(left)),
                Box::new(self.simplify_definite_description_conjunction(right)),
            ),
            ProofExpr::Not(inner) => ProofExpr::Not(
                Box::new(self.simplify_definite_description_conjunction(inner)),
            ),
            ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.simplify_definite_description_conjunction(body)),
            },
            ProofExpr::Exists { variable, body } => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.simplify_definite_description_conjunction(body)),
            },
            ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
                operator: operator.clone(),
                body: Box::new(self.simplify_definite_description_conjunction(body)),
            },
            _ => expr.clone(),
        }
    }

    // =========================================================================
    // EVENT SEMANTICS ABSTRACTION
    // =========================================================================

    /// Abstract Neo-Davidsonian event semantics to simple predicates.
    ///
    /// Converts: ∃e(Shave(e) ∧ Agent(e, x) ∧ Theme(e, y)) → shaves(x, y)
    ///
    /// This allows the proof engine to reason about events using simpler
    /// predicate logic, which is essential for paradoxes like the Barber Paradox.
    fn abstract_event_to_predicate(&self, expr: &ProofExpr) -> Option<ProofExpr> {
        match expr {
            // Direct NeoEvent abstraction
            ProofExpr::NeoEvent { verb, roles, .. } => {
                // Extract Agent and Theme/Patient roles
                let agent = roles.iter()
                    .find(|(role, _)| role == "Agent")
                    .map(|(_, term)| term.clone());

                let theme = roles.iter()
                    .find(|(role, _)| role == "Theme" || role == "Patient")
                    .map(|(_, term)| term.clone());

                // Build a simple predicate: verb(agent, theme) or verb(agent)
                let mut args = Vec::new();
                if let Some(a) = agent {
                    args.push(a);
                }
                if let Some(t) = theme {
                    args.push(t);
                }

                // Lowercase the verb for predicate naming convention
                let pred_name = verb.to_lowercase();

                Some(ProofExpr::Predicate {
                    name: pred_name,
                    args,
                    world: None,
                })
            }

            // Handle Exists wrapping an event expression
            ProofExpr::Exists { variable, body } => {
                // Check if this is an event quantification
                if !self.is_event_variable(variable) {
                    return None;
                }

                // Try direct NeoEvent abstraction
                if let Some(abstracted) = self.abstract_event_to_predicate(body) {
                    return Some(abstracted);
                }

                // Try to parse conjunction of event predicates
                // Pattern: ∃e(Verb(e) ∧ Agent(e, x) ∧ Theme(e, y)) → verb(x, y)
                if let Some(abstracted) = self.abstract_event_conjunction(variable, body) {
                    return Some(abstracted);
                }

                None
            }

            _ => None,
        }
    }

    /// Abstract a conjunction of event predicates to a simple predicate.
    ///
    /// Handles: Verb(e) ∧ Agent(e, x) ∧ Theme(e, y) → verb(x, y)
    fn abstract_event_conjunction(&self, event_var: &str, body: &ProofExpr) -> Option<ProofExpr> {
        // Flatten the conjunction to get all components
        let mut components = Vec::new();
        self.flatten_conjunction(body, &mut components);

        // Find verb predicate (single arg that matches event_var)
        let mut verb_name: Option<String> = None;
        let mut agent: Option<ProofTerm> = None;
        let mut theme: Option<ProofTerm> = None;

        for comp in &components {
            if let ProofExpr::Predicate { name, args, .. } = comp {
                // Check if first arg is the event variable
                let first_is_event = args.first().map_or(false, |arg| {
                    matches!(arg, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == event_var)
                });

                if !first_is_event && args.len() == 1 {
                    // Single arg predicate that's the event var
                    if let Some(ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v)) = args.first() {
                        if v == event_var {
                            verb_name = Some(name.clone());
                            continue;
                        }
                    }
                }

                if first_is_event {
                    match name.as_str() {
                        "Agent" if args.len() == 2 => {
                            agent = Some(args[1].clone());
                        }
                        "Theme" | "Patient" if args.len() == 2 => {
                            theme = Some(args[1].clone());
                        }
                        _ if args.len() == 1 && verb_name.is_none() => {
                            // This is probably the verb predicate: Verb(e)
                            verb_name = Some(name.clone());
                        }
                        _ => {}
                    }
                }
            }
        }

        // If we found a verb, construct the simple predicate
        if let Some(verb) = verb_name {
            let mut args = Vec::new();
            if let Some(a) = agent {
                args.push(a);
            }
            if let Some(t) = theme {
                args.push(t);
            }

            return Some(ProofExpr::Predicate {
                name: verb.to_lowercase(),
                args,
                world: None,
            });
        }

        None
    }

    /// Check if a variable name looks like an event variable.
    ///
    /// Event variables are typically named "e", "e1", "e2", etc.
    fn is_event_variable(&self, var: &str) -> bool {
        var == "e" || var.starts_with("e_") ||
        (var.starts_with('e') && var.len() == 2 && var.chars().nth(1).map_or(false, |c| c.is_ascii_digit()))
    }

    /// Recursively abstract all events in an expression.
    ///
    /// This transforms the entire expression tree, replacing event semantics
    /// with simple predicates wherever possible.
    fn abstract_all_events(&self, expr: &ProofExpr) -> ProofExpr {
        // First try direct abstraction
        if let Some(abstracted) = self.abstract_event_to_predicate(expr) {
            return abstracted;
        }

        // Otherwise recurse into the structure
        match expr {
            ProofExpr::And(left, right) => ProofExpr::And(
                Box::new(self.abstract_all_events(left)),
                Box::new(self.abstract_all_events(right)),
            ),
            ProofExpr::Or(left, right) => ProofExpr::Or(
                Box::new(self.abstract_all_events(left)),
                Box::new(self.abstract_all_events(right)),
            ),
            ProofExpr::Implies(left, right) => ProofExpr::Implies(
                Box::new(self.abstract_all_events(left)),
                Box::new(self.abstract_all_events(right)),
            ),
            ProofExpr::Iff(left, right) => ProofExpr::Iff(
                Box::new(self.abstract_all_events(left)),
                Box::new(self.abstract_all_events(right)),
            ),
            ProofExpr::Not(inner) => {
                // Apply De Morgan for quantifiers: ¬∃x.P ≡ ∀x.¬P
                // This normalization is crucial for efficient proof search
                // (Converting negated existentials to universals helps the prover)
                if let ProofExpr::Exists { variable, body } = inner.as_ref() {
                    return ProofExpr::ForAll {
                        variable: variable.clone(),
                        body: Box::new(self.abstract_all_events(&ProofExpr::Not(body.clone()))),
                    };
                }
                // Note: We do NOT convert ¬∀x.P to ∃x.¬P because the prover
                // works better with universal quantifiers for backward chaining.
                ProofExpr::Not(Box::new(self.abstract_all_events(inner)))
            }
            ProofExpr::ForAll { variable, body } => {
                // Check for pattern: ∀x ¬(P ∧ Q) → ∀x (P → ¬Q)
                // This converts to implication form for better backward chaining
                if let ProofExpr::Not(inner) = body.as_ref() {
                    if let ProofExpr::And(left, right) = inner.as_ref() {
                        return ProofExpr::ForAll {
                            variable: variable.clone(),
                            body: Box::new(ProofExpr::Implies(
                                Box::new(self.abstract_all_events(left)),
                                Box::new(self.abstract_all_events(&ProofExpr::Not(right.clone()))),
                            )),
                        };
                    }
                }
                ProofExpr::ForAll {
                    variable: variable.clone(),
                    body: Box::new(self.abstract_all_events(body)),
                }
            }
            ProofExpr::Exists { variable, body } => {
                // Check if this is an event quantification that should be abstracted
                if self.is_event_variable(variable) {
                    if let Some(abstracted) = self.abstract_event_to_predicate(body) {
                        return abstracted;
                    }
                }
                // Otherwise keep the existential and recurse
                ProofExpr::Exists {
                    variable: variable.clone(),
                    body: Box::new(self.abstract_all_events(body)),
                }
            }
            // For other expressions, return as-is
            other => other.clone(),
        }
    }

    /// Abstract event semantics WITHOUT applying De Morgan transformations.
    ///
    /// This is used for goals where we want to preserve the ¬∃ pattern
    /// for reductio ad absurdum strategies.
    fn abstract_events_only(&self, expr: &ProofExpr) -> ProofExpr {
        // First try direct abstraction
        if let Some(abstracted) = self.abstract_event_to_predicate(expr) {
            return abstracted;
        }

        // Otherwise recurse into the structure
        match expr {
            ProofExpr::And(left, right) => ProofExpr::And(
                Box::new(self.abstract_events_only(left)),
                Box::new(self.abstract_events_only(right)),
            ),
            ProofExpr::Or(left, right) => ProofExpr::Or(
                Box::new(self.abstract_events_only(left)),
                Box::new(self.abstract_events_only(right)),
            ),
            ProofExpr::Implies(left, right) => ProofExpr::Implies(
                Box::new(self.abstract_events_only(left)),
                Box::new(self.abstract_events_only(right)),
            ),
            ProofExpr::Iff(left, right) => ProofExpr::Iff(
                Box::new(self.abstract_events_only(left)),
                Box::new(self.abstract_events_only(right)),
            ),
            ProofExpr::Not(inner) => {
                // Just recurse, no De Morgan transformation
                ProofExpr::Not(Box::new(self.abstract_events_only(inner)))
            }
            ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.abstract_events_only(body)),
            },
            ProofExpr::Exists { variable, body } => {
                // Check if this is an event quantification that should be abstracted
                if self.is_event_variable(variable) {
                    if let Some(abstracted) = self.abstract_event_to_predicate(body) {
                        return abstracted;
                    }
                }
                // Otherwise keep the existential and recurse
                ProofExpr::Exists {
                    variable: variable.clone(),
                    body: Box::new(self.abstract_events_only(body)),
                }
            }
            // For other expressions, return as-is
            other => other.clone(),
        }
    }

    /// Look for a contradiction in the knowledge base and context.
    ///
    /// A contradiction exists when both P and ¬P are derivable.
    fn find_contradiction(
        &mut self,
        context: &[ProofExpr],
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect all expressions from KB and context
        let all_exprs: Vec<ProofExpr> = self.knowledge_base.iter()
            .chain(context.iter())
            .cloned()
            .collect();

        // Strategy 1: Look for direct P and ¬P pairs
        for expr in &all_exprs {
            if let ProofExpr::Not(inner) = expr {
                // We have ¬P, check if P exists directly
                for other in &all_exprs {
                    if exprs_structurally_equal(other, inner) {
                        // Found both P and ¬P directly
                        let pos_leaf = DerivationTree::leaf(
                            (**inner).clone(),
                            InferenceRule::PremiseMatch,
                        );
                        let neg_leaf = DerivationTree::leaf(
                            expr.clone(),
                            InferenceRule::PremiseMatch,
                        );
                        return Ok(Some(DerivationTree::new(
                            ProofExpr::Atom("⊥".into()),
                            InferenceRule::Contradiction,
                            vec![pos_leaf, neg_leaf],
                        )));
                    }
                }
            }
        }

        // Strategy 2: Look for implications that derive contradictory results
        // Check if context fact P triggers P → ¬P (immediate contradiction)
        // Or if P triggers P → Q where ¬Q is also in context
        // Note: Extract implications from both top-level and inside ForAll quantifiers
        let mut implications: Vec<(ProofExpr, ProofExpr)> = Vec::new();
        for e in &all_exprs {
            if let ProofExpr::Implies(ante, cons) = e {
                implications.push(((**ante).clone(), (**cons).clone()));
            }
            // Also extract from inside ForAll (important for barber paradox!)
            if let ProofExpr::ForAll { body, .. } = e {
                if let ProofExpr::Implies(ante, cons) = body.as_ref() {
                    implications.push(((**ante).clone(), (**cons).clone()));
                }
            }
        }

        // For each fact in the context, see if it triggers contradictory implications
        for fact in context {
            // Find all implications where fact matches the antecedent
            let mut derivable_consequences: Vec<ProofExpr> = Vec::new();

            for (ante, cons) in &implications {
                // Try to unify the antecedent with the fact
                if let Ok(subst) = unify_exprs(fact, ante) {
                    let instantiated_cons = apply_subst_to_expr(cons, &subst);
                    derivable_consequences.push(instantiated_cons);
                }

                // Also try matching conjunctive antecedents with multiple facts
                if let ProofExpr::And(left, right) = ante {
                    // Try to find facts matching both parts of the conjunction
                    if let Some(subst) = self.try_match_conjunction_antecedent(
                        left, right, &all_exprs
                    ) {
                        let instantiated_cons = apply_subst_to_expr(cons, &subst);
                        if !derivable_consequences.contains(&instantiated_cons) {
                            derivable_consequences.push(instantiated_cons);
                        }
                    }
                }
            }

            // Check if any derived consequence contradicts the triggering fact
            for cons in &derivable_consequences {
                // Check if cons = ¬fact (the classic barber structure: P → ¬P)
                if let ProofExpr::Not(inner) = cons {
                    if exprs_structurally_equal(inner, fact) {
                        // fact triggered an implication that derives ¬fact
                        // This is a contradiction: fact ∧ ¬fact
                        let pos_leaf = DerivationTree::leaf(
                            fact.clone(),
                            InferenceRule::PremiseMatch,
                        );
                        let neg_leaf = DerivationTree::leaf(
                            cons.clone(),
                            InferenceRule::ModusPonens,
                        );
                        return Ok(Some(DerivationTree::new(
                            ProofExpr::Atom("⊥".into()),
                            InferenceRule::Contradiction,
                            vec![pos_leaf, neg_leaf],
                        )));
                    }
                }

                // Check if cons contradicts any other fact in context
                for other in context {
                    if std::ptr::eq(fact as *const _, other as *const _) {
                        continue; // Skip the triggering fact itself
                    }
                    // Check if cons = ¬other
                    if let ProofExpr::Not(inner) = cons {
                        if exprs_structurally_equal(inner, other) {
                            let pos_leaf = DerivationTree::leaf(
                                other.clone(),
                                InferenceRule::PremiseMatch,
                            );
                            let neg_leaf = DerivationTree::leaf(
                                cons.clone(),
                                InferenceRule::ModusPonens,
                            );
                            return Ok(Some(DerivationTree::new(
                                ProofExpr::Atom("⊥".into()),
                                InferenceRule::Contradiction,
                                vec![pos_leaf, neg_leaf],
                            )));
                        }
                    }
                    // Check if other = ¬cons
                    if let ProofExpr::Not(inner_other) = other {
                        if exprs_structurally_equal(inner_other, cons) {
                            let pos_leaf = DerivationTree::leaf(
                                cons.clone(),
                                InferenceRule::ModusPonens,
                            );
                            let neg_leaf = DerivationTree::leaf(
                                other.clone(),
                                InferenceRule::PremiseMatch,
                            );
                            return Ok(Some(DerivationTree::new(
                                ProofExpr::Atom("⊥".into()),
                                InferenceRule::Contradiction,
                                vec![pos_leaf, neg_leaf],
                            )));
                        }
                    }
                }
            }

            // Check if any pair of consequences contradicts each other
            for i in 0..derivable_consequences.len() {
                for j in (i + 1)..derivable_consequences.len() {
                    let cons1 = &derivable_consequences[i];
                    let cons2 = &derivable_consequences[j];

                    // Check if cons1 = ¬cons2 or cons2 = ¬cons1
                    if let ProofExpr::Not(inner1) = cons1 {
                        if exprs_structurally_equal(inner1, cons2) {
                            // cons1 = ¬cons2, contradiction!
                            let pos_leaf = DerivationTree::leaf(
                                cons2.clone(),
                                InferenceRule::ModusPonens,
                            );
                            let neg_leaf = DerivationTree::leaf(
                                cons1.clone(),
                                InferenceRule::ModusPonens,
                            );
                            return Ok(Some(DerivationTree::new(
                                ProofExpr::Atom("⊥".into()),
                                InferenceRule::Contradiction,
                                vec![pos_leaf, neg_leaf],
                            )));
                        }
                    }
                    if let ProofExpr::Not(inner2) = cons2 {
                        if exprs_structurally_equal(inner2, cons1) {
                            // cons2 = ¬cons1, contradiction!
                            let pos_leaf = DerivationTree::leaf(
                                cons1.clone(),
                                InferenceRule::ModusPonens,
                            );
                            let neg_leaf = DerivationTree::leaf(
                                cons2.clone(),
                                InferenceRule::ModusPonens,
                            );
                            return Ok(Some(DerivationTree::new(
                                ProofExpr::Atom("⊥".into()),
                                InferenceRule::Contradiction,
                                vec![pos_leaf, neg_leaf],
                            )));
                        }
                    }
                }
            }
        }

        // Strategy 3: Try to find self-referential contradictions (like Barber Paradox)
        if let Some(proof) = self.find_self_referential_contradiction(context, depth)? {
            return Ok(Some(proof));
        }

        Ok(None)
    }

    /// Try to match a conjunctive antecedent with facts in the context.
    ///
    /// For an antecedent like (man(z) ∧ shave(z,z)), we need to find facts
    /// that match both parts with consistent variable bindings.
    fn try_match_conjunction_antecedent(
        &self,
        left: &ProofExpr,
        right: &ProofExpr,
        facts: &[ProofExpr],
    ) -> Option<Substitution> {
        // Try to find a fact that matches the left part
        for fact1 in facts {
            if let Ok(subst1) = unify_exprs(fact1, left) {
                // Apply this substitution to the right part
                let instantiated_right = apply_subst_to_expr(right, &subst1);
                // Now look for a fact that matches the instantiated right part
                for fact2 in facts {
                    if let Ok(subst2) = unify_exprs(fact2, &instantiated_right) {
                        // Combine substitutions
                        let mut combined = subst1.clone();
                        for (k, v) in subst2.iter() {
                            combined.insert(k.clone(), v.clone());
                        }
                        return Some(combined);
                    }
                }
            }
        }
        // Also try right then left
        for fact1 in facts {
            if let Ok(subst1) = unify_exprs(fact1, right) {
                let instantiated_left = apply_subst_to_expr(left, &subst1);
                for fact2 in facts {
                    if let Ok(subst2) = unify_exprs(fact2, &instantiated_left) {
                        let mut combined = subst1.clone();
                        for (k, v) in subst2.iter() {
                            combined.insert(k.clone(), v.clone());
                        }
                        return Some(combined);
                    }
                }
            }
        }
        None
    }

    /// Special case: find self-referential contradictions (like the Barber Paradox).
    ///
    /// Pattern: If we have ∀x(P(x) → Q(b, x)) and ∀x(P(x) → ¬Q(b, x)),
    /// then for x = b with P(b), we get Q(b, b) ∧ ¬Q(b, b).
    ///
    /// This uses direct pattern matching WITHOUT recursive prove_goal calls
    /// to avoid infinite recursion.
    fn find_self_referential_contradiction(
        &mut self,
        context: &[ProofExpr],
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect all expressions from KB and context
        let all_exprs: Vec<ProofExpr> = self.knowledge_base.iter()
            .chain(context.iter())
            .cloned()
            .collect();

        // Look for pairs of universal implications with contradictory conclusions
        // that can be instantiated with the same witness
        for expr1 in &all_exprs {
            if let ProofExpr::ForAll { variable: var1, body: body1 } = expr1 {
                if let ProofExpr::Implies(ante1, cons1) = body1.as_ref() {
                    for expr2 in &all_exprs {
                        if std::ptr::eq(expr1, expr2) {
                            continue; // Skip same expression
                        }
                        if let ProofExpr::ForAll { variable: var2, body: body2 } = expr2 {
                            if let ProofExpr::Implies(ante2, cons2) = body2.as_ref() {
                                // Check if cons2 = ¬cons1 (structurally)
                                if let ProofExpr::Not(neg_cons2) = cons2.as_ref() {
                                    // Check if cons1 and neg_cons2 have matching structure
                                    // For barber: cons1 = shaves(barber, x), neg_cons2 = shaves(barber, x)

                                    // Try instantiating with x = barber (the self-referential case)
                                    // We look for constant terms in cons1 that could be witnesses
                                    let witnesses = self.extract_constants_from_expr(cons1);

                                    for witness_name in &witnesses {
                                        let witness = ProofTerm::Constant(witness_name.clone());

                                        // Instantiate both antecedents and consequents with this witness
                                        let mut subst1 = Substitution::new();
                                        subst1.insert(var1.clone(), witness.clone());
                                        let ante1_inst = apply_subst_to_expr(ante1, &subst1);
                                        let cons1_inst = apply_subst_to_expr(cons1, &subst1);

                                        let mut subst2 = Substitution::new();
                                        subst2.insert(var2.clone(), witness.clone());
                                        let ante2_inst = apply_subst_to_expr(ante2, &subst2);
                                        let cons2_inst = apply_subst_to_expr(cons2, &subst2);

                                        // Check if cons1_inst and ¬cons2_inst contradict
                                        // cons2_inst should be ¬X where X = cons1_inst
                                        if let ProofExpr::Not(inner2) = &cons2_inst {
                                            if exprs_structurally_equal(&cons1_inst, inner2) {
                                                // Now check if both antecedents could hold
                                                // ante1 typically is ¬P(x,x) and ante2 is P(x,x)
                                                // These are complementary - one must hold
                                                // For the paradox, we consider BOTH cases

                                                // If ante1 = ¬P(x,x) and ante2 = P(x,x), and x = witness,
                                                // we have a tertium non datur case:
                                                // - Either P(w,w) holds → cons2_inst = ¬cons1_inst
                                                // - Or ¬P(w,w) holds → cons1_inst

                                                // Check if ante1 and ante2 are complements
                                                if self.are_complements(&ante1_inst, &ante2_inst) {
                                                    // By excluded middle, one antecedent holds
                                                    // If cons1_inst and cons2_inst = ¬cons1_inst,
                                                    // we have a contradiction
                                                    let pos_leaf = DerivationTree::leaf(
                                                        cons1_inst.clone(),
                                                        InferenceRule::ModusPonens,
                                                    );
                                                    let neg_leaf = DerivationTree::leaf(
                                                        cons2_inst,
                                                        InferenceRule::ModusPonens,
                                                    );
                                                    return Ok(Some(DerivationTree::new(
                                                        ProofExpr::Atom("⊥".into()),
                                                        InferenceRule::Contradiction,
                                                        vec![pos_leaf, neg_leaf],
                                                    )));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check if two expressions are complements (one is the negation of the other).
    fn are_complements(&self, expr1: &ProofExpr, expr2: &ProofExpr) -> bool {
        // Check if expr1 = ¬expr2
        if let ProofExpr::Not(inner1) = expr1 {
            if exprs_structurally_equal(inner1, expr2) {
                return true;
            }
        }
        // Check if expr2 = ¬expr1
        if let ProofExpr::Not(inner2) = expr2 {
            if exprs_structurally_equal(inner2, expr1) {
                return true;
            }
        }
        false
    }

    /// Extract constant names from an expression.
    fn extract_constants_from_expr(&self, expr: &ProofExpr) -> Vec<String> {
        let mut constants = Vec::new();
        self.extract_constants_recursive(expr, &mut constants);
        constants
    }

    fn extract_constants_recursive(&self, expr: &ProofExpr, constants: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.extract_constants_from_term_recursive(arg, constants);
                }
            }
            ProofExpr::Identity(l, r) => {
                self.extract_constants_from_term_recursive(l, constants);
                self.extract_constants_from_term_recursive(r, constants);
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.extract_constants_recursive(l, constants);
                self.extract_constants_recursive(r, constants);
            }
            ProofExpr::Not(inner) => {
                self.extract_constants_recursive(inner, constants);
            }
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.extract_constants_recursive(body, constants);
            }
            _ => {}
        }
    }

    fn extract_constants_from_term_recursive(&self, term: &ProofTerm, constants: &mut Vec<String>) {
        match term {
            ProofTerm::Constant(name) => {
                if !constants.contains(name) {
                    constants.push(name.clone());
                }
            }
            ProofTerm::Function(_, args) => {
                for arg in args {
                    self.extract_constants_from_term_recursive(arg, constants);
                }
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    self.extract_constants_from_term_recursive(t, constants);
                }
            }
            _ => {}
        }
    }

    // =========================================================================
    // STRATEGY 6: EQUALITY REWRITING (LEIBNIZ'S LAW)
    // =========================================================================

    /// Try rewriting using equalities in the knowledge base.
    ///
    /// Leibniz's Law: If a = b and P(a), then P(b).
    /// Also handles symmetry (a = b ⊢ b = a) and transitivity (a = b, b = c ⊢ a = c).
    fn try_equality_rewrite(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Collect equalities from KB and context
        let equalities: Vec<(ProofTerm, ProofTerm)> = self
            .knowledge_base
            .iter()
            .chain(goal.context.iter())
            .filter_map(|expr| {
                if let ProofExpr::Identity(l, r) = expr {
                    Some((l.clone(), r.clone()))
                } else {
                    None
                }
            })
            .collect();

        if equalities.is_empty() {
            return Ok(None);
        }

        // Handle special case: goal is itself an equality (symmetry/transitivity)
        if let ProofExpr::Identity(goal_l, goal_r) = &goal.target {
            // Try symmetry: a = b ⊢ b = a
            if let Some(tree) = self.try_equality_symmetry(goal_l, goal_r, &equalities, depth)? {
                return Ok(Some(tree));
            }

            // Try transitivity: a = b, b = c ⊢ a = c
            if let Some(tree) = self.try_equality_transitivity(goal_l, goal_r, &equalities, depth)? {
                return Ok(Some(tree));
            }

            // Try equational rewriting: use axioms to rewrite LHS step by step
            // Only if we have depth budget remaining (prevents infinite recursion)
            if depth + 3 < self.max_depth {
                if let Some(tree) = self.try_equational_identity_rewrite(goal, goal_l, goal_r, depth)? {
                    return Ok(Some(tree));
                }
            }

            return Ok(None);
        }

        // Try rewriting: substitute one term for another (for non-Identity goals)
        for (eq_from, eq_to) in &equalities {
            // Try forward: a = b, P(a) ⊢ P(b)
            if let Some(tree) = self.try_rewrite_with_equality(
                goal, eq_from, eq_to, depth,
            )? {
                return Ok(Some(tree));
            }

            // Try backward: a = b, P(b) ⊢ P(a)
            if let Some(tree) = self.try_rewrite_with_equality(
                goal, eq_to, eq_from, depth,
            )? {
                return Ok(Some(tree));
            }
        }

        Ok(None)
    }

    /// Try to prove goal by substituting `from` with `to` in some known fact.
    fn try_rewrite_with_equality(
        &mut self,
        goal: &ProofGoal,
        from: &ProofTerm,
        to: &ProofTerm,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Create the "source" expression by substituting `to` with `from` in the goal
        // If goal is P(b) and we have a = b, we want to find P(a)
        let source_goal = self.substitute_term_in_expr(&goal.target, to, from);

        // Check if source_goal differs from the goal (substitution had effect)
        if source_goal == goal.target {
            return Ok(None);
        }

        // Try to prove the source goal
        let source_proof_goal = ProofGoal::with_context(source_goal.clone(), goal.context.clone());
        if let Ok(source_proof) = self.prove_goal(source_proof_goal, depth + 1) {
            // Also need a proof of the equality
            let equality = ProofExpr::Identity(from.clone(), to.clone());
            let eq_proof_goal = ProofGoal::with_context(equality.clone(), goal.context.clone());

            if let Ok(eq_proof) = self.prove_goal(eq_proof_goal, depth + 1) {
                return Ok(Some(DerivationTree::new(
                    goal.target.clone(),
                    InferenceRule::Rewrite {
                        from: from.clone(),
                        to: to.clone(),
                    },
                    vec![eq_proof, source_proof],
                )));
            }
        }

        Ok(None)
    }

    /// Try equality symmetry: a = b ⊢ b = a
    fn try_equality_symmetry(
        &mut self,
        goal_l: &ProofTerm,
        goal_r: &ProofTerm,
        equalities: &[(ProofTerm, ProofTerm)],
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Check if we have r = l in KB (so we can derive l = r)
        for (eq_l, eq_r) in equalities {
            if eq_l == goal_r && eq_r == goal_l {
                // Found r = l, can derive l = r by symmetry
                let source = ProofExpr::Identity(goal_r.clone(), goal_l.clone());
                return Ok(Some(DerivationTree::new(
                    ProofExpr::Identity(goal_l.clone(), goal_r.clone()),
                    InferenceRule::EqualitySymmetry,
                    vec![DerivationTree::leaf(source, InferenceRule::PremiseMatch)],
                )));
            }
        }
        Ok(None)
    }

    /// Try equality transitivity: a = b, b = c ⊢ a = c
    fn try_equality_transitivity(
        &mut self,
        goal_l: &ProofTerm,
        goal_r: &ProofTerm,
        equalities: &[(ProofTerm, ProofTerm)],
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Look for a = b and b = c where we want a = c
        for (eq1_l, eq1_r) in equalities {
            if eq1_l == goal_l {
                // Found a = b, now look for b = c
                for (eq2_l, eq2_r) in equalities {
                    if eq2_l == eq1_r && eq2_r == goal_r {
                        // Found a = b and b = c, derive a = c
                        let premise1 = ProofExpr::Identity(eq1_l.clone(), eq1_r.clone());
                        let premise2 = ProofExpr::Identity(eq2_l.clone(), eq2_r.clone());
                        return Ok(Some(DerivationTree::new(
                            ProofExpr::Identity(goal_l.clone(), goal_r.clone()),
                            InferenceRule::EqualityTransitivity,
                            vec![
                                DerivationTree::leaf(premise1, InferenceRule::PremiseMatch),
                                DerivationTree::leaf(premise2, InferenceRule::PremiseMatch),
                            ],
                        )));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Try equational rewriting for Identity goals.
    ///
    /// For a goal `f(a) = b`, find an axiom `f(x) = g(x)` that matches,
    /// rewrite to get `g(a) = b`, and recursively prove that.
    fn try_equational_identity_rewrite(
        &mut self,
        goal: &ProofGoal,
        goal_l: &ProofTerm,
        goal_r: &ProofTerm,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // First, try congruence: if both sides have the same outermost function/ctor,
        // recursively prove the arguments are equal.
        if let (
            ProofTerm::Function(name_l, args_l),
            ProofTerm::Function(name_r, args_r),
        ) = (goal_l, goal_r)
        {
            if name_l == name_r && args_l.len() == args_r.len() {
                // All arguments must be equal
                let mut arg_proofs = Vec::new();
                let mut all_ok = true;
                for (arg_l, arg_r) in args_l.iter().zip(args_r.iter()) {
                    let arg_goal_expr = ProofExpr::Identity(arg_l.clone(), arg_r.clone());
                    let arg_goal = ProofGoal::with_context(arg_goal_expr, goal.context.clone());
                    match self.prove_goal(arg_goal, depth + 1) {
                        Ok(proof) => arg_proofs.push(proof),
                        Err(_) => {
                            all_ok = false;
                            break;
                        }
                    }
                }
                if all_ok {
                    // All arguments are equal, so the functions are equal by congruence
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::Reflexivity, // Using Reflexivity for congruence
                        arg_proofs,
                    )));
                }
            }
        }
        // Collect Identity axioms from KB
        let axioms: Vec<(ProofTerm, ProofTerm)> = self
            .knowledge_base
            .iter()
            .filter_map(|e| {
                if let ProofExpr::Identity(l, r) = e {
                    Some((l.clone(), r.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Try each axiom to rewrite the goal's LHS
        for (axiom_l, axiom_r) in &axioms {
            // Rename variables in axiom to avoid capture (use same map for both sides!)
            let mut var_map = std::collections::HashMap::new();
            let renamed_l = self.rename_term_vars_with_map(axiom_l, &mut var_map);
            let renamed_r = self.rename_term_vars_with_map(axiom_r, &mut var_map);

            // Try to unify axiom LHS with goal LHS
            // e.g., unify(Add(Succ(k), n), Add(Succ(Zero), Succ(Zero)))
            //       => {k: Zero, n: Succ(Zero)}
            if let Ok(subst) = unify_terms(&renamed_l, goal_l) {
                // Apply substitution to axiom RHS to get the rewritten term
                let rewritten = self.apply_subst_to_term(&renamed_r, &subst);

                // First check: does rewritten equal goal_r directly?
                if terms_structurally_equal(&rewritten, goal_r) {
                    // Direct match! Build the proof
                    let axiom_expr = ProofExpr::Identity(axiom_l.clone(), axiom_r.clone());
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::Rewrite {
                            from: goal_l.clone(),
                            to: rewritten,
                        },
                        vec![DerivationTree::leaf(axiom_expr, InferenceRule::PremiseMatch)],
                    )));
                }

                // Otherwise, create a new goal with the rewritten LHS
                let new_goal_expr = ProofExpr::Identity(rewritten.clone(), goal_r.clone());
                let new_goal = ProofGoal::with_context(new_goal_expr.clone(), goal.context.clone());

                // Recursively try to prove the new goal
                if let Ok(sub_proof) = self.prove_goal(new_goal, depth + 1) {
                    // Success! Build the full proof
                    let axiom_expr = ProofExpr::Identity(axiom_l.clone(), axiom_r.clone());
                    return Ok(Some(DerivationTree::new(
                        goal.target.clone(),
                        InferenceRule::Rewrite {
                            from: goal_l.clone(),
                            to: rewritten,
                        },
                        vec![
                            DerivationTree::leaf(axiom_expr, InferenceRule::PremiseMatch),
                            sub_proof,
                        ],
                    )));
                }
            }
        }

        Ok(None)
    }

    /// Rename variables in a term to fresh names (consistently).
    fn rename_term_vars(&mut self, term: &ProofTerm) -> ProofTerm {
        let mut var_map = std::collections::HashMap::new();
        self.rename_term_vars_with_map(term, &mut var_map)
    }

    fn rename_term_vars_with_map(
        &mut self,
        term: &ProofTerm,
        var_map: &mut std::collections::HashMap<String, String>,
    ) -> ProofTerm {
        match term {
            ProofTerm::Variable(name) => {
                // Check if we've already renamed this variable
                if let Some(fresh) = var_map.get(name) {
                    ProofTerm::Variable(fresh.clone())
                } else {
                    // Create fresh name and remember it
                    let fresh = format!("_v{}", self.var_counter);
                    self.var_counter += 1;
                    var_map.insert(name.clone(), fresh.clone());
                    ProofTerm::Variable(fresh)
                }
            }
            ProofTerm::Function(name, args) => {
                ProofTerm::Function(
                    name.clone(),
                    args.iter().map(|a| self.rename_term_vars_with_map(a, var_map)).collect(),
                )
            }
            ProofTerm::Group(terms) => {
                ProofTerm::Group(
                    terms.iter().map(|t| self.rename_term_vars_with_map(t, var_map)).collect(),
                )
            }
            other => other.clone(),
        }
    }

    /// Apply a substitution to a term.
    fn apply_subst_to_term(&self, term: &ProofTerm, subst: &Substitution) -> ProofTerm {
        match term {
            ProofTerm::Variable(name) => {
                if let Some(replacement) = subst.get(name) {
                    replacement.clone()
                } else {
                    term.clone()
                }
            }
            ProofTerm::Function(name, args) => {
                ProofTerm::Function(
                    name.clone(),
                    args.iter().map(|a| self.apply_subst_to_term(a, subst)).collect(),
                )
            }
            ProofTerm::Group(terms) => {
                ProofTerm::Group(terms.iter().map(|t| self.apply_subst_to_term(t, subst)).collect())
            }
            other => other.clone(),
        }
    }

    /// Substitute a term for another in an expression.
    fn substitute_term_in_expr(
        &self,
        expr: &ProofExpr,
        from: &ProofTerm,
        to: &ProofTerm,
    ) -> ProofExpr {
        match expr {
            ProofExpr::Predicate { name, args, world } => {
                let new_args: Vec<_> = args
                    .iter()
                    .map(|arg| self.substitute_in_term(arg, from, to))
                    .collect();
                ProofExpr::Predicate {
                    name: name.clone(),
                    args: new_args,
                    world: world.clone(),
                }
            }
            ProofExpr::Identity(l, r) => ProofExpr::Identity(
                self.substitute_in_term(l, from, to),
                self.substitute_in_term(r, from, to),
            ),
            ProofExpr::And(l, r) => ProofExpr::And(
                Box::new(self.substitute_term_in_expr(l, from, to)),
                Box::new(self.substitute_term_in_expr(r, from, to)),
            ),
            ProofExpr::Or(l, r) => ProofExpr::Or(
                Box::new(self.substitute_term_in_expr(l, from, to)),
                Box::new(self.substitute_term_in_expr(r, from, to)),
            ),
            ProofExpr::Implies(l, r) => ProofExpr::Implies(
                Box::new(self.substitute_term_in_expr(l, from, to)),
                Box::new(self.substitute_term_in_expr(r, from, to)),
            ),
            ProofExpr::Not(inner) => {
                ProofExpr::Not(Box::new(self.substitute_term_in_expr(inner, from, to)))
            }
            ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.substitute_term_in_expr(body, from, to)),
            },
            ProofExpr::Exists { variable, body } => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.substitute_term_in_expr(body, from, to)),
            },
            // For other expressions, return as-is
            other => other.clone(),
        }
    }

    /// Substitute a term for another in a ProofTerm.
    fn substitute_in_term(
        &self,
        term: &ProofTerm,
        from: &ProofTerm,
        to: &ProofTerm,
    ) -> ProofTerm {
        if term == from {
            return to.clone();
        }
        match term {
            ProofTerm::Function(name, args) => {
                let new_args: Vec<_> = args
                    .iter()
                    .map(|arg| self.substitute_in_term(arg, from, to))
                    .collect();
                ProofTerm::Function(name.clone(), new_args)
            }
            ProofTerm::Group(terms) => {
                let new_terms: Vec<_> = terms
                    .iter()
                    .map(|t| self.substitute_in_term(t, from, to))
                    .collect();
                ProofTerm::Group(new_terms)
            }
            other => other.clone(),
        }
    }

    // =========================================================================
    // STRATEGY 7: STRUCTURAL INDUCTION
    // =========================================================================

    /// Try structural induction on inductive types (Nat, List, etc.).
    ///
    /// Phase 68 Enhancement: First attempts to infer the motive using Miller
    /// pattern unification (?Motive(#n) = Goal → ?Motive = λn.Goal).
    /// Falls back to crude substitution if pattern unification fails.
    ///
    /// When the goal contains a TypedVar like `n:Nat`, we split into:
    /// - Base case: P(Zero)
    /// - Step case: ∀k. P(k) → P(Succ(k))
    fn try_structural_induction(
        &mut self,
        goal: &ProofGoal,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Look for TypedVar in the goal
        if let Some((var_name, typename)) = self.find_typed_var(&goal.target) {
            // Phase 68: Try motive inference via pattern unification first
            if let Some(motive) = self.try_infer_motive(&goal.target, &var_name) {
                match typename.as_str() {
                    "Nat" => {
                        if let Ok(Some(proof)) =
                            self.try_nat_induction_with_motive(goal, &var_name, &motive, depth)
                        {
                            return Ok(Some(proof));
                        }
                    }
                    "List" => {
                        // TODO: Add try_list_induction_with_motive
                    }
                    _ => {}
                }
            }

            // Fallback: crude substitution approach
            match typename.as_str() {
                "Nat" => self.try_nat_induction(goal, &var_name, depth),
                "List" => self.try_list_induction(goal, &var_name, depth),
                _ => Ok(None), // Unknown inductive type
            }
        } else {
            Ok(None)
        }
    }

    /// Try to infer the induction motive using Miller pattern unification.
    ///
    /// Given a goal like `Add(n:Nat, Zero) = n:Nat`, creates the pattern
    /// `?Motive(#n) = Goal` and solves for `?Motive = λn. Goal`.
    fn try_infer_motive(&self, goal: &ProofExpr, var_name: &str) -> Option<ProofExpr> {
        // Create the pattern: ?Motive(#var_name)
        let motive_hole = ProofExpr::Hole("Motive".to_string());
        let pattern = ProofExpr::App(
            Box::new(motive_hole),
            Box::new(ProofExpr::Term(ProofTerm::BoundVarRef(var_name.to_string()))),
        );

        // The body is the goal itself (with TypedVar replaced by Variable for unification)
        let body = self.convert_typed_var_to_variable(goal, var_name);

        // Unify: ?Motive(#n) = body
        match unify_pattern(&pattern, &body) {
            Ok(solution) => solution.get("Motive").cloned(),
            Err(_) => None,
        }
    }

    /// Convert TypedVar to regular Variable for pattern unification.
    ///
    /// Pattern unification expects Variable("n") in the body to match BoundVarRef("n")
    /// in the pattern, but our goals have TypedVar { name: "n", typename: "Nat" }.
    fn convert_typed_var_to_variable(&self, expr: &ProofExpr, var_name: &str) -> ProofExpr {
        match expr {
            ProofExpr::TypedVar { name, .. } if name == var_name => {
                // Convert to Atom so it becomes a Variable in terms
                ProofExpr::Atom(name.clone())
            }
            ProofExpr::Identity(l, r) => ProofExpr::Identity(
                self.convert_typed_var_in_term(l, var_name),
                self.convert_typed_var_in_term(r, var_name),
            ),
            ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| self.convert_typed_var_in_term(a, var_name))
                    .collect(),
                world: world.clone(),
            },
            ProofExpr::And(l, r) => ProofExpr::And(
                Box::new(self.convert_typed_var_to_variable(l, var_name)),
                Box::new(self.convert_typed_var_to_variable(r, var_name)),
            ),
            ProofExpr::Or(l, r) => ProofExpr::Or(
                Box::new(self.convert_typed_var_to_variable(l, var_name)),
                Box::new(self.convert_typed_var_to_variable(r, var_name)),
            ),
            ProofExpr::Not(inner) => {
                ProofExpr::Not(Box::new(self.convert_typed_var_to_variable(inner, var_name)))
            }
            _ => expr.clone(),
        }
    }

    /// Convert TypedVar to Variable in a ProofTerm.
    fn convert_typed_var_in_term(&self, term: &ProofTerm, var_name: &str) -> ProofTerm {
        match term {
            ProofTerm::Variable(v) => {
                // Check for "name:Type" pattern
                if v == var_name || v.starts_with(&format!("{}:", var_name)) {
                    ProofTerm::Variable(var_name.to_string())
                } else {
                    term.clone()
                }
            }
            ProofTerm::Function(name, args) => ProofTerm::Function(
                name.clone(),
                args.iter()
                    .map(|a| self.convert_typed_var_in_term(a, var_name))
                    .collect(),
            ),
            ProofTerm::Group(terms) => ProofTerm::Group(
                terms
                    .iter()
                    .map(|t| self.convert_typed_var_in_term(t, var_name))
                    .collect(),
            ),
            _ => term.clone(),
        }
    }

    /// Perform structural induction on Nat using pattern unification.
    ///
    /// This is the Phase 68 approach: infer the motive using Miller pattern
    /// unification, then apply it to constructors via beta reduction.
    ///
    /// Base case: P(Zero)
    /// Step case: ∀k. P(k) → P(Succ(k))
    fn try_nat_induction_with_motive(
        &mut self,
        goal: &ProofGoal,
        var_name: &str,
        motive: &ProofExpr,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Base case: P(Zero)
        // Apply the motive lambda to Zero constructor
        let zero_ctor = ProofExpr::Ctor {
            name: "Zero".into(),
            args: vec![],
        };
        let base_goal_expr = beta_reduce(&ProofExpr::App(
            Box::new(motive.clone()),
            Box::new(zero_ctor),
        ));

        let base_goal = ProofGoal::with_context(base_goal_expr, goal.context.clone());
        let base_proof = match self.prove_goal(base_goal, depth + 1) {
            Ok(proof) => proof,
            Err(_) => return Ok(None),
        };

        // Step case: ∀k. P(k) → P(Succ(k))
        let fresh_k = self.fresh_var();
        let k_var = ProofExpr::Atom(fresh_k.clone());

        // Induction hypothesis: P(k)
        let ih = beta_reduce(&ProofExpr::App(
            Box::new(motive.clone()),
            Box::new(k_var.clone()),
        ));

        // Step conclusion: P(Succ(k))
        let succ_k = ProofExpr::Ctor {
            name: "Succ".into(),
            args: vec![k_var],
        };
        let step_goal_expr = beta_reduce(&ProofExpr::App(
            Box::new(motive.clone()),
            Box::new(succ_k),
        ));

        // Add IH to context for step case
        let mut step_context = goal.context.clone();
        step_context.push(ih.clone());

        let step_goal = ProofGoal::with_context(step_goal_expr, step_context);
        let step_proof = match self.try_step_case_with_equational_reasoning(&step_goal, &ih, depth)
        {
            Ok(proof) => proof,
            Err(_) => return Ok(None),
        };

        Ok(Some(DerivationTree::new(
            goal.target.clone(),
            InferenceRule::StructuralInduction {
                variable: var_name.to_string(),
                ind_type: "Nat".to_string(),
                step_var: fresh_k,
            },
            vec![base_proof, step_proof],
        )))
    }

    /// Perform structural induction on Nat (legacy crude substitution).
    ///
    /// Base case: P(Zero)
    /// Step case: ∀k. P(k) → P(Succ(k))
    fn try_nat_induction(
        &mut self,
        goal: &ProofGoal,
        var_name: &str,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Create Zero constructor
        let zero = ProofExpr::Ctor {
            name: "Zero".into(),
            args: vec![],
        };

        // Base case: substitute Zero for the induction variable
        let base_goal_expr = self.substitute_typed_var(&goal.target, var_name, &zero);
        let base_goal = ProofGoal::with_context(base_goal_expr, goal.context.clone());

        // Try to prove base case
        let base_proof = match self.prove_goal(base_goal, depth + 1) {
            Ok(proof) => proof,
            Err(_) => return Ok(None), // Can't prove base case
        };

        // Step case: assume P(k), prove P(Succ(k))
        let fresh_k = self.fresh_var();

        // Create k as a variable
        let k_var = ProofExpr::Atom(fresh_k.clone());

        // Create Succ(k)
        let succ_k = ProofExpr::Ctor {
            name: "Succ".into(),
            args: vec![k_var.clone()],
        };

        // Induction hypothesis: P(k)
        let ih = self.substitute_typed_var(&goal.target, var_name, &k_var);

        // Step goal: P(Succ(k))
        let step_goal_expr = self.substitute_typed_var(&goal.target, var_name, &succ_k);

        // Add IH to context for step case
        let mut step_context = goal.context.clone();
        step_context.push(ih.clone());

        let step_goal = ProofGoal::with_context(step_goal_expr, step_context);

        // Try to prove step case with IH in context
        let step_proof = match self.try_step_case_with_equational_reasoning(&step_goal, &ih, depth)
        {
            Ok(proof) => proof,
            Err(_) => return Ok(None), // Can't prove step case
        };

        // Build the induction proof tree
        Ok(Some(DerivationTree::new(
            goal.target.clone(),
            InferenceRule::StructuralInduction {
                variable: var_name.to_string(),
                ind_type: "Nat".to_string(),
                step_var: fresh_k,
            },
            vec![base_proof, step_proof],
        )))
    }

    /// Perform structural induction on List.
    ///
    /// Base case: P(Nil)
    /// Step case: ∀h,t. P(t) → P(Cons(h,t))
    fn try_list_induction(
        &mut self,
        goal: &ProofGoal,
        var_name: &str,
        depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Create Nil constructor
        let nil = ProofExpr::Ctor {
            name: "Nil".into(),
            args: vec![],
        };

        // Base case: substitute Nil for the induction variable
        let base_goal_expr = self.substitute_typed_var(&goal.target, var_name, &nil);
        let base_goal = ProofGoal::with_context(base_goal_expr, goal.context.clone());

        // Try to prove base case
        let base_proof = match self.prove_goal(base_goal, depth + 1) {
            Ok(proof) => proof,
            Err(_) => return Ok(None),
        };

        // Step case: assume P(t), prove P(Cons(h, t))
        let fresh_h = self.fresh_var();
        let fresh_t = self.fresh_var();

        let h_var = ProofExpr::Atom(fresh_h);
        let t_var = ProofExpr::Atom(fresh_t.clone());

        let cons_ht = ProofExpr::Ctor {
            name: "Cons".into(),
            args: vec![h_var, t_var.clone()],
        };

        // Induction hypothesis: P(t)
        let ih = self.substitute_typed_var(&goal.target, var_name, &t_var);

        // Step goal: P(Cons(h, t))
        let step_goal_expr = self.substitute_typed_var(&goal.target, var_name, &cons_ht);

        let mut step_context = goal.context.clone();
        step_context.push(ih.clone());

        let step_goal = ProofGoal::with_context(step_goal_expr, step_context);

        let step_proof = match self.try_step_case_with_equational_reasoning(&step_goal, &ih, depth)
        {
            Ok(proof) => proof,
            Err(_) => return Ok(None),
        };

        Ok(Some(DerivationTree::new(
            goal.target.clone(),
            InferenceRule::StructuralInduction {
                variable: var_name.to_string(),
                ind_type: "List".to_string(),
                step_var: fresh_t,
            },
            vec![base_proof, step_proof],
        )))
    }

    /// Try to prove the step case, potentially using equational reasoning.
    ///
    /// The step case often requires:
    /// 1. Applying a recursive axiom to simplify the goal
    /// 2. Using the induction hypothesis
    /// 3. Congruence reasoning (e.g., Succ(x) = Succ(y) if x = y)
    fn try_step_case_with_equational_reasoning(
        &mut self,
        goal: &ProofGoal,
        ih: &ProofExpr,
        depth: usize,
    ) -> ProofResult<DerivationTree> {
        // First, try direct proof (might work for simple cases)
        if let Ok(proof) = self.prove_goal(goal.clone(), depth + 1) {
            return Ok(proof);
        }

        // For Identity goals, try equational reasoning
        if let ProofExpr::Identity(lhs, rhs) = &goal.target {
            // Try to rewrite LHS using axioms and see if we can reach RHS
            if let Some(proof) = self.try_equational_proof(goal, lhs, rhs, ih, depth)? {
                return Ok(proof);
            }
        }

        Err(ProofError::NoProofFound)
    }

    /// Try equational reasoning: rewrite LHS to match RHS using axioms and IH.
    ///
    /// For the step case of induction, we need to:
    /// 1. Find an axiom that matches the goal's LHS pattern
    /// 2. Use the axiom to rewrite LHS
    /// 3. Apply the induction hypothesis to simplify
    /// 4. Check if the result equals RHS
    fn try_equational_proof(
        &mut self,
        goal: &ProofGoal,
        lhs: &ProofTerm,
        rhs: &ProofTerm,
        ih: &ProofExpr,
        _depth: usize,
    ) -> ProofResult<Option<DerivationTree>> {
        // Find applicable equations from KB (Identity axioms)
        let equations: Vec<ProofExpr> = self
            .knowledge_base
            .iter()
            .filter(|e| matches!(e, ProofExpr::Identity(_, _)))
            .cloned()
            .collect();

        // Try each equation to rewrite LHS
        for eq_axiom in &equations {
            if let ProofExpr::Identity(_, _) = &eq_axiom {
                // Rename variables in the axiom to avoid capture
                let renamed_axiom = self.rename_variables(&eq_axiom);
                if let ProofExpr::Identity(renamed_lhs, renamed_rhs) = renamed_axiom {
                    // Unify axiom LHS with goal LHS
                    // This binds axiom variables to goal terms
                    // e.g., unify(Add(Succ(x), m), Add(Succ(k), Zero)) gives {x->k, m->Zero}
                    if let Ok(subst) = unify_terms(&renamed_lhs, lhs) {
                        // Apply the substitution to the axiom's RHS
                        // This gives us what LHS rewrites to
                        let rewritten = self.apply_subst_to_term_with(&renamed_rhs, &subst);

                        // Now check if rewritten equals RHS (possibly using IH)
                        if self.terms_equal_with_ih(&rewritten, rhs, ih) {
                            // Success! Build proof using the axiom and IH
                            let axiom_leaf =
                                DerivationTree::leaf(eq_axiom.clone(), InferenceRule::PremiseMatch);

                            let ih_leaf =
                                DerivationTree::leaf(ih.clone(), InferenceRule::PremiseMatch);

                            return Ok(Some(DerivationTree::new(
                                goal.target.clone(),
                                InferenceRule::PremiseMatch, // Equational step
                                vec![axiom_leaf, ih_leaf],
                            )));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check if two terms are equal, potentially using the induction hypothesis.
    fn terms_equal_with_ih(&self, t1: &ProofTerm, t2: &ProofTerm, ih: &ProofExpr) -> bool {
        // Direct equality
        if t1 == t2 {
            return true;
        }

        // Try using IH: if IH is `x = y`, and t1 contains x, replace with y
        if let ProofExpr::Identity(ih_lhs, ih_rhs) = ih {
            // Check if t1 can be transformed to t2 using IH
            let t1_with_ih = self.rewrite_term_with_equation(t1, ih_lhs, ih_rhs);
            if &t1_with_ih == t2 {
                return true;
            }

            // Also try the other direction
            let t2_with_ih = self.rewrite_term_with_equation(t2, ih_rhs, ih_lhs);
            if t1 == &t2_with_ih {
                return true;
            }
        }

        false
    }

    /// Rewrite occurrences of `from` to `to` in the term.
    fn rewrite_term_with_equation(
        &self,
        term: &ProofTerm,
        from: &ProofTerm,
        to: &ProofTerm,
    ) -> ProofTerm {
        // If term matches `from`, return `to`
        if term == from {
            return to.clone();
        }

        // Recursively rewrite in subterms
        match term {
            ProofTerm::Function(name, args) => {
                let new_args: Vec<ProofTerm> = args
                    .iter()
                    .map(|a| self.rewrite_term_with_equation(a, from, to))
                    .collect();
                ProofTerm::Function(name.clone(), new_args)
            }
            ProofTerm::Group(terms) => {
                let new_terms: Vec<ProofTerm> = terms
                    .iter()
                    .map(|t| self.rewrite_term_with_equation(t, from, to))
                    .collect();
                ProofTerm::Group(new_terms)
            }
            _ => term.clone(),
        }
    }

    /// Apply substitution to a ProofTerm with given substitution.
    fn apply_subst_to_term_with(&self, term: &ProofTerm, subst: &Substitution) -> ProofTerm {
        match term {
            ProofTerm::Variable(v) => subst.get(v).cloned().unwrap_or_else(|| term.clone()),
            ProofTerm::Function(name, args) => ProofTerm::Function(
                name.clone(),
                args.iter()
                    .map(|a| self.apply_subst_to_term_with(a, subst))
                    .collect(),
            ),
            ProofTerm::Group(terms) => ProofTerm::Group(
                terms
                    .iter()
                    .map(|t| self.apply_subst_to_term_with(t, subst))
                    .collect(),
            ),
            ProofTerm::Constant(_) => term.clone(),
            ProofTerm::BoundVarRef(_) => term.clone(),
        }
    }

    /// Find a TypedVar in the expression.
    fn find_typed_var(&self, expr: &ProofExpr) -> Option<(String, String)> {
        match expr {
            ProofExpr::TypedVar { name, typename } => Some((name.clone(), typename.clone())),
            ProofExpr::Identity(l, r) => {
                self.find_typed_var_in_term(l).or_else(|| self.find_typed_var_in_term(r))
            }
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    if let Some(tv) = self.find_typed_var_in_term(arg) {
                        return Some(tv);
                    }
                }
                None
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => self.find_typed_var(l).or_else(|| self.find_typed_var(r)),
            ProofExpr::Not(inner) => self.find_typed_var(inner),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.find_typed_var(body)
            }
            _ => None,
        }
    }

    /// Find a TypedVar embedded in a ProofTerm.
    fn find_typed_var_in_term(&self, term: &ProofTerm) -> Option<(String, String)> {
        match term {
            ProofTerm::Variable(v) => {
                // Check if this variable name is in our KB as a TypedVar
                // Actually, TypedVar should be in the expression, not the term
                // Let's check if the variable name contains type annotation
                if v.contains(':') {
                    let parts: Vec<&str> = v.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        return Some((parts[0].to_string(), parts[1].to_string()));
                    }
                }
                None
            }
            ProofTerm::Function(_, args) => {
                for arg in args {
                    if let Some(tv) = self.find_typed_var_in_term(arg) {
                        return Some(tv);
                    }
                }
                None
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    if let Some(tv) = self.find_typed_var_in_term(t) {
                        return Some(tv);
                    }
                }
                None
            }
            ProofTerm::Constant(_) => None,
            ProofTerm::BoundVarRef(_) => None, // Pattern-level, no TypedVar
        }
    }

    /// Substitute a TypedVar with a given expression throughout the goal.
    fn substitute_typed_var(
        &self,
        expr: &ProofExpr,
        var_name: &str,
        replacement: &ProofExpr,
    ) -> ProofExpr {
        match expr {
            ProofExpr::TypedVar { name, .. } if name == var_name => replacement.clone(),
            ProofExpr::Identity(l, r) => {
                let new_l = self.substitute_typed_var_in_term(l, var_name, replacement);
                let new_r = self.substitute_typed_var_in_term(r, var_name, replacement);
                ProofExpr::Identity(new_l, new_r)
            }
            ProofExpr::Predicate { name, args, world } => {
                let new_args: Vec<ProofTerm> = args
                    .iter()
                    .map(|a| self.substitute_typed_var_in_term(a, var_name, replacement))
                    .collect();
                ProofExpr::Predicate {
                    name: name.clone(),
                    args: new_args,
                    world: world.clone(),
                }
            }
            ProofExpr::And(l, r) => ProofExpr::And(
                Box::new(self.substitute_typed_var(l, var_name, replacement)),
                Box::new(self.substitute_typed_var(r, var_name, replacement)),
            ),
            ProofExpr::Or(l, r) => ProofExpr::Or(
                Box::new(self.substitute_typed_var(l, var_name, replacement)),
                Box::new(self.substitute_typed_var(r, var_name, replacement)),
            ),
            ProofExpr::Implies(l, r) => ProofExpr::Implies(
                Box::new(self.substitute_typed_var(l, var_name, replacement)),
                Box::new(self.substitute_typed_var(r, var_name, replacement)),
            ),
            ProofExpr::Iff(l, r) => ProofExpr::Iff(
                Box::new(self.substitute_typed_var(l, var_name, replacement)),
                Box::new(self.substitute_typed_var(r, var_name, replacement)),
            ),
            ProofExpr::Not(inner) => {
                ProofExpr::Not(Box::new(self.substitute_typed_var(inner, var_name, replacement)))
            }
            ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
                variable: variable.clone(),
                body: Box::new(self.substitute_typed_var(body, var_name, replacement)),
            },
            ProofExpr::Exists { variable, body } => ProofExpr::Exists {
                variable: variable.clone(),
                body: Box::new(self.substitute_typed_var(body, var_name, replacement)),
            },
            _ => expr.clone(),
        }
    }

    /// Substitute a TypedVar in a ProofTerm.
    fn substitute_typed_var_in_term(
        &self,
        term: &ProofTerm,
        var_name: &str,
        replacement: &ProofExpr,
    ) -> ProofTerm {
        match term {
            ProofTerm::Variable(v) => {
                // Check for TypedVar pattern "name:Type"
                if v == var_name || v.starts_with(&format!("{}:", var_name)) {
                    self.expr_to_term(replacement)
                } else {
                    term.clone()
                }
            }
            ProofTerm::Function(name, args) => ProofTerm::Function(
                name.clone(),
                args.iter()
                    .map(|a| self.substitute_typed_var_in_term(a, var_name, replacement))
                    .collect(),
            ),
            ProofTerm::Group(terms) => ProofTerm::Group(
                terms
                    .iter()
                    .map(|t| self.substitute_typed_var_in_term(t, var_name, replacement))
                    .collect(),
            ),
            ProofTerm::Constant(_) => term.clone(),
            ProofTerm::BoundVarRef(_) => term.clone(),
        }
    }

    /// Convert a ProofExpr to a ProofTerm (for use in substitution).
    fn expr_to_term(&self, expr: &ProofExpr) -> ProofTerm {
        match expr {
            ProofExpr::Atom(s) => ProofTerm::Variable(s.clone()),
            ProofExpr::Ctor { name, args } => {
                ProofTerm::Function(name.clone(), args.iter().map(|a| self.expr_to_term(a)).collect())
            }
            ProofExpr::TypedVar { name, .. } => ProofTerm::Variable(name.clone()),
            _ => ProofTerm::Constant(format!("{}", expr)),
        }
    }

    // =========================================================================
    // HELPER METHODS
    // =========================================================================

    /// Generate a fresh variable name.
    fn fresh_var(&mut self) -> String {
        self.var_counter += 1;
        format!("_G{}", self.var_counter)
    }

    /// Rename all variables in an expression to fresh names.
    fn rename_variables(&mut self, expr: &ProofExpr) -> ProofExpr {
        let vars = self.collect_variables(expr);
        let mut subst = Substitution::new();

        for var in vars {
            let fresh = self.fresh_var();
            subst.insert(var, ProofTerm::Variable(fresh));
        }

        apply_subst_to_expr(expr, &subst)
    }

    /// Collect all variable names in an expression.
    fn collect_variables(&self, expr: &ProofExpr) -> Vec<String> {
        let mut vars = Vec::new();
        self.collect_variables_recursive(expr, &mut vars);
        vars
    }

    fn collect_variables_recursive(&self, expr: &ProofExpr, vars: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.collect_term_variables(arg, vars);
                }
            }
            ProofExpr::Identity(l, r) => {
                self.collect_term_variables(l, vars);
                self.collect_term_variables(r, vars);
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.collect_variables_recursive(l, vars);
                self.collect_variables_recursive(r, vars);
            }
            ProofExpr::Not(inner) => self.collect_variables_recursive(inner, vars),
            ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
                if !vars.contains(variable) {
                    vars.push(variable.clone());
                }
                self.collect_variables_recursive(body, vars);
            }
            ProofExpr::Lambda { variable, body } => {
                if !vars.contains(variable) {
                    vars.push(variable.clone());
                }
                self.collect_variables_recursive(body, vars);
            }
            ProofExpr::App(f, a) => {
                self.collect_variables_recursive(f, vars);
                self.collect_variables_recursive(a, vars);
            }
            ProofExpr::NeoEvent { roles, .. } => {
                for (_, term) in roles {
                    self.collect_term_variables(term, vars);
                }
            }
            _ => {}
        }
    }

    fn collect_term_variables(&self, term: &ProofTerm, vars: &mut Vec<String>) {
        match term {
            ProofTerm::Variable(v) => {
                if !vars.contains(v) {
                    vars.push(v.clone());
                }
            }
            ProofTerm::Function(_, args) => {
                for arg in args {
                    self.collect_term_variables(arg, vars);
                }
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    self.collect_term_variables(t, vars);
                }
            }
            ProofTerm::Constant(_) => {}
            ProofTerm::BoundVarRef(_) => {} // Pattern-level, no variables
        }
    }

    /// Collect potential witnesses (constants) from the knowledge base.
    fn collect_witnesses(&self) -> Vec<ProofTerm> {
        let mut witnesses = Vec::new();

        for expr in &self.knowledge_base {
            self.collect_constants_from_expr(expr, &mut witnesses);
        }

        witnesses
    }

    fn collect_constants_from_expr(&self, expr: &ProofExpr, constants: &mut Vec<ProofTerm>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.collect_constants_from_term(arg, constants);
                }
            }
            ProofExpr::Identity(l, r) => {
                self.collect_constants_from_term(l, constants);
                self.collect_constants_from_term(r, constants);
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.collect_constants_from_expr(l, constants);
                self.collect_constants_from_expr(r, constants);
            }
            ProofExpr::Not(inner) => self.collect_constants_from_expr(inner, constants),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.collect_constants_from_expr(body, constants);
            }
            ProofExpr::NeoEvent { roles, .. } => {
                for (_, term) in roles {
                    self.collect_constants_from_term(term, constants);
                }
            }
            _ => {}
        }
    }

    fn collect_constants_from_term(&self, term: &ProofTerm, constants: &mut Vec<ProofTerm>) {
        match term {
            ProofTerm::Constant(_) => {
                if !constants.contains(term) {
                    constants.push(term.clone());
                }
            }
            ProofTerm::Function(_, args) => {
                // The function application itself could be a witness
                if !constants.contains(term) {
                    constants.push(term.clone());
                }
                for arg in args {
                    self.collect_constants_from_term(arg, constants);
                }
            }
            ProofTerm::Group(terms) => {
                for t in terms {
                    self.collect_constants_from_term(t, constants);
                }
            }
            ProofTerm::Variable(_) => {}
            ProofTerm::BoundVarRef(_) => {} // Pattern-level, not a constant
        }
    }

    // =========================================================================
    // STRATEGY 7: ORACLE FALLBACK (Z3)
    // =========================================================================

    /// Attempt to prove using Z3 as an oracle.
    ///
    /// This is the fallback when all structural proof strategies fail.
    /// Z3 will verify arithmetic, comparisons, and uninterpreted function reasoning.
    #[cfg(feature = "verification")]
    fn try_oracle_fallback(&self, goal: &ProofGoal) -> ProofResult<Option<DerivationTree>> {
        crate::proof::oracle::try_oracle(goal, &self.knowledge_base)
    }
}

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Extract the type from an existential body if it contains type information.
///
/// Looks for TypedVar patterns in the body that might indicate the type
/// of the existentially quantified variable. Returns None if no type
/// information is found.
fn extract_type_from_exists_body(body: &ProofExpr) -> Option<String> {
    match body {
        // Direct TypedVar in body
        ProofExpr::TypedVar { typename, .. } => Some(typename.clone()),

        // Recurse into conjunctions
        ProofExpr::And(l, r) => {
            extract_type_from_exists_body(l).or_else(|| extract_type_from_exists_body(r))
        }

        // Recurse into disjunctions
        ProofExpr::Or(l, r) => {
            extract_type_from_exists_body(l).or_else(|| extract_type_from_exists_body(r))
        }

        // Recurse into nested quantifiers
        ProofExpr::Exists { body, .. } | ProofExpr::ForAll { body, .. } => {
            extract_type_from_exists_body(body)
        }

        // No type information found
        _ => None,
    }
}

impl Default for BackwardChainer {
    fn default() -> Self {
        Self::new()
    }
}
