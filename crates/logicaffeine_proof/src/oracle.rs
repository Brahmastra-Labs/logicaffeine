//! Z3 Oracle integration for proof search fallback.
//!
//! When the structural backward chainer cannot derive a proof, the oracle
//! delegates to Z3, an SMT solver, for arithmetic, comparisons, and
//! uninterpreted function reasoning.
//!
//! # Architecture
//!
//! 1. Convert [`ProofExpr`] → [`VerifyExpr`](logicaffeine_verify::ir::VerifyExpr)
//! 2. Add assumptions from the knowledge base
//! 3. Ask Z3 to verify the goal
//! 4. Return [`DerivationTree`] with `OracleVerification` rule if successful
//!
//! # Limitations
//!
//! The oracle cannot reason about:
//! - Inductive constructs (`Ctor`, `Match`, `Fixpoint`, `TypedVar`)
//! - Lambda calculus (`Lambda`, `App`)
//! - Event semantics (`NeoEvent`)
//!
//! These constructs are detected by [`contains_inductive_constructs`] and cause
//! the oracle to return `Ok(None)` rather than attempting verification.
//!
//! # Feature Gate
//!
//! This module requires the `verification` feature flag:
//!
//! ```toml
//! [dependencies]
//! logicaffeine_proof = { version = "...", features = ["verification"] }
//! ```
//!
//! # Example
//!
//! ```ignore
//! use logicaffeine_proof::{ProofGoal, ProofExpr};
//! use logicaffeine_proof::oracle::try_oracle;
//!
//! let goal = ProofGoal::new(ProofExpr::Atom("P".into()));
//! let kb = vec![ProofExpr::Atom("P".into())];
//!
//! match try_oracle(&goal, &kb) {
//!     Ok(Some(tree)) => println!("Z3 verified: {}", tree),
//!     Ok(None) => println!("Z3 cannot verify"),
//!     Err(e) => println!("Error: {}", e),
//! }
//! ```

use crate::error::ProofResult;
use crate::{DerivationTree, InferenceRule, ProofExpr, ProofGoal, ProofTerm};

use crate::modal_translation::{contains_modal_constructs, WorldTranslation};
use logicaffeine_verify::ir::{VerifyExpr, VerifyOp, VerifyType};
use logicaffeine_verify::solver::VerificationSession;
use logicaffeine_verify::VerificationErrorKind;

// =============================================================================
// SMT VERDICTS (semantic entailment over the standard translation)
// =============================================================================

/// A three-valued Z3 verdict on a semantic entailment question.
///
/// This is the oracle's answer over the standard translation of modal,
/// mereological, and defeasible constructs with their frame/lattice axioms.
/// It is **NOT kernel-certified**: never conflate an [`SmtVerdict::Entailed`]
/// with a [`crate::verify::VerifiedProof`] whose `verified` flag is true. The
/// kernel path stays monotonic and modal-free by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtVerdict {
    /// Z3 proved the goal follows from the premises (plus emitted axioms).
    Entailed,
    /// Z3 found a countermodel: the premises are satisfiable together with
    /// the negated goal.
    NotEntailed,
    /// Z3 returned unknown or the construct is not yet translatable. Never
    /// treated as success in either direction.
    Unknown,
}

/// A three-valued Z3 verdict on the joint satisfiability of premises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtConsistency {
    /// Z3 found a model satisfying every premise (plus emitted axioms).
    Consistent,
    /// Z3 proved the premises jointly unsatisfiable.
    Inconsistent,
    /// Z3 returned unknown or the construct is not yet translatable.
    Unknown,
}

/// Lexicon-derived facts the SMT layer cannot know on its own (the proof
/// crate has no language dependency). Threaded in by the compile-side doors.
#[derive(Debug, Clone, Default)]
pub struct SmtTheory {
    /// Predicates closed under the lattice sum (lexicon-tagged MASS nouns):
    /// `M(x) ∧ M(y) → M(x ⊕ y)`, and a portion of M counts as consuming the
    /// M-kind (`M(x) ∧ Theme(e, x) → Theme(e, ^M)`).
    pub cumulative_predicates: Vec<String>,
}

/// Ask Z3 whether `premises ⊨ goal` under the standard translation.
///
/// Modal operators are expanded to world-quantified accessibility relations
/// with per-(domain, flavor) frame axioms; counterfactuals use a similarity
/// (`Closest`) relation; group terms use the Link-lattice sum. Non-modal,
/// non-mereological inputs take the same encoding as [`try_oracle`].
///
/// The verdict is **not kernel-certified** — see [`SmtVerdict`].
pub fn oracle_entails(premises: &[ProofExpr], goal: &ProofExpr) -> SmtVerdict {
    oracle_entails_with_theory(premises, goal, &SmtTheory::default())
}

/// [`oracle_entails`] with lexicon-derived theory (mass/cumulative tags).
pub fn oracle_entails_with_theory(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    theory: &SmtTheory,
) -> SmtVerdict {
    if contains_inductive_constructs(goal)
        || premises.iter().any(contains_inductive_constructs)
    {
        return SmtVerdict::Unknown;
    }
    let mut session = VerificationSession::new();
    let goal_expr = match build_smt_problem(premises, Some(goal), &mut session, theory) {
        Some(g) => g,
        None => return SmtVerdict::Unknown,
    };
    match session.verify(&goal_expr) {
        Ok(()) => SmtVerdict::Entailed,
        Err(e) => match e.kind {
            VerificationErrorKind::ContradictoryAssertion => SmtVerdict::NotEntailed,
            _ => SmtVerdict::Unknown,
        },
    }
}

/// Ask Z3 whether the premises are jointly satisfiable under the standard
/// translation (with the same axiom emission as [`oracle_entails`]).
///
/// Every non-entailment claim in the test suite is paired with a consistency
/// check so an over-axiomatized (inconsistent) theory cannot fake a
/// [`SmtVerdict::NotEntailed`] via vacuity.
pub fn oracle_consistent(premises: &[ProofExpr]) -> SmtConsistency {
    oracle_consistent_with_theory(premises, &SmtTheory::default())
}

/// [`oracle_consistent`] with lexicon-derived theory (mass/cumulative tags).
pub fn oracle_consistent_with_theory(
    premises: &[ProofExpr],
    theory: &SmtTheory,
) -> SmtConsistency {
    if premises.iter().any(contains_inductive_constructs) {
        return SmtConsistency::Unknown;
    }
    let mut session = VerificationSession::new();
    if build_smt_problem(premises, None, &mut session, theory).is_none() {
        return SmtConsistency::Unknown;
    }
    match session.check_sat() {
        Ok(true) => SmtConsistency::Consistent,
        Ok(false) => SmtConsistency::Inconsistent,
        Err(_) => SmtConsistency::Unknown,
    }
}

/// Does the problem mention the lattice sum (a `sum(...)` function term or a
/// multi-member plural group)?
fn contains_sum_term(expr: &ProofExpr) -> bool {
    fn in_term(term: &ProofTerm) -> bool {
        match term {
            ProofTerm::Function(name, args) => {
                name == "sum" || args.iter().any(in_term)
            }
            ProofTerm::Group(terms) => terms.len() > 1 || terms.iter().any(in_term),
            _ => false,
        }
    }
    fn walk(expr: &ProofExpr, found: &mut bool) {
        if *found {
            return;
        }
        match expr {
            ProofExpr::Predicate { args, .. } => *found = args.iter().any(in_term),
            ProofExpr::Identity(l, r) => *found = in_term(l) || in_term(r),
            ProofExpr::NeoEvent { roles, .. } => {
                *found = roles.iter().any(|(_, t)| in_term(t))
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                walk(l, found);
                walk(r, found);
            }
            ProofExpr::Not(i) => walk(i, found),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                walk(body, found)
            }
            ProofExpr::Modal { body, .. } | ProofExpr::Temporal { body, .. } => {
                walk(body, found)
            }
            ProofExpr::Counterfactual {
                antecedent,
                consequent,
            } => {
                walk(antecedent, found);
                walk(consequent, found);
            }
            _ => {}
        }
    }
    let mut found = false;
    walk(expr, &mut found);
    found
}

/// Every predicate name mentioned across the given expressions. The
/// compile-side doors use this to look up lexicon facts (e.g. mass tags)
/// when assembling an [`SmtTheory`].
pub fn predicate_names(exprs: &[ProofExpr]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for expr in exprs {
        collect_predicate_names(expr, &mut out);
    }
    out
}

/// Collect every predicate name mentioned in the problem.
fn collect_predicate_names(expr: &ProofExpr, out: &mut std::collections::BTreeSet<String>) {
    match expr {
        ProofExpr::Predicate { name, .. } => {
            out.insert(name.clone());
        }
        ProofExpr::NeoEvent { verb, .. } => {
            out.insert(verb.to_lowercase());
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_predicate_names(l, out);
            collect_predicate_names(r, out);
        }
        ProofExpr::Not(i) => collect_predicate_names(i, out),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            collect_predicate_names(body, out)
        }
        ProofExpr::Modal { body, .. } | ProofExpr::Temporal { body, .. } => {
            collect_predicate_names(body, out)
        }
        ProofExpr::Counterfactual {
            antecedent,
            consequent,
        } => {
            collect_predicate_names(antecedent, out);
            collect_predicate_names(consequent, out);
        }
        _ => {}
    }
}

/// The finite ground-term universe the lattice axioms are instantiated over:
/// every ground leaf term (constant/variable) appearing inside a `sum`/group
/// or as the argument of a cumulative predicate, capped to keep the cubic
/// associativity instantiation small.
fn ground_sum_universe(
    premises: &[ProofExpr],
    goal: Option<&ProofExpr>,
    theory: &SmtTheory,
) -> Vec<VerifyExpr> {
    fn leaves(term: &ProofTerm, out: &mut Vec<ProofTerm>) {
        match term {
            ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
                for arg in args {
                    leaves(arg, out);
                }
            }
            other => {
                if !out.contains(other) {
                    out.push(other.clone());
                }
            }
        }
    }
    fn walk(expr: &ProofExpr, theory: &SmtTheory, out: &mut Vec<ProofTerm>) {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                let relevant = theory.cumulative_predicates.contains(name)
                    || args.iter().any(|t| {
                        matches!(t, ProofTerm::Function(n, _) if n == "sum")
                            || matches!(t, ProofTerm::Group(g) if g.len() > 1)
                    });
                if relevant {
                    for arg in args {
                        leaves(arg, out);
                    }
                }
            }
            ProofExpr::Identity(l, r) => {
                let relevant = [l, r].iter().any(|t| {
                    matches!(t, ProofTerm::Function(n, _) if n == "sum")
                        || matches!(t, ProofTerm::Group(g) if g.len() > 1)
                });
                if relevant {
                    leaves(l, out);
                    leaves(r, out);
                }
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                walk(l, theory, out);
                walk(r, theory, out);
            }
            ProofExpr::Not(i) => walk(i, theory, out),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                walk(body, theory, out)
            }
            ProofExpr::Modal { body, .. } | ProofExpr::Temporal { body, .. } => {
                walk(body, theory, out)
            }
            ProofExpr::Counterfactual {
                antecedent,
                consequent,
            } => {
                walk(antecedent, theory, out);
                walk(consequent, theory, out);
            }
            ProofExpr::NeoEvent { roles, .. } => {
                for (_, term) in roles {
                    if matches!(term, ProofTerm::Function(n, _) if n == "sum")
                        || matches!(term, ProofTerm::Group(g) if g.len() > 1)
                    {
                        leaves(term, out);
                    }
                }
            }
            _ => {}
        }
    }
    let mut terms = Vec::new();
    for premise in premises {
        walk(premise, theory, &mut terms);
    }
    if let Some(g) = goal {
        walk(g, theory, &mut terms);
    }
    terms.truncate(6);
    terms
        .iter()
        .filter_map(proof_term_to_verify_expr)
        .collect()
}

/// The Link-lattice axiom pack, emitted demand-driven: the core ⊕ axioms
/// only when a sum term appears, CUM / kind-Theme lifting only for the
/// theory's cumulative (mass) predicates that the problem actually mentions.
fn lattice_axioms(
    premises: &[ProofExpr],
    goal: Option<&ProofExpr>,
    theory: &SmtTheory,
) -> Vec<VerifyExpr> {
    let mut axioms = Vec::new();
    let has_sum = premises.iter().any(contains_sum_term)
        || goal.map(contains_sum_term).unwrap_or(false);

    let mut mentioned = std::collections::BTreeSet::new();
    for premise in premises {
        collect_predicate_names(premise, &mut mentioned);
    }
    if let Some(g) = goal {
        collect_predicate_names(g, &mut mentioned);
    }

    if has_sum {
        // The axioms are GROUND-INSTANTIATED over the problem's finite term
        // universe rather than ∀-quantified: ground instances prove exactly
        // the same lattice facts here, and they keep the encoding
        // quantifier-free so Z3 can also FIND COUNTERMODELS (a quantified
        // Int-function axiom set makes the SAT direction return unknown).
        let universe = ground_sum_universe(premises, goal, theory);
        let sum_of = |a: &VerifyExpr, b: &VerifyExpr| {
            VerifyExpr::apply_int("sum", vec![a.clone(), b.clone()])
        };

        for a in &universe {
            // Idempotence: a ⊕ a = a
            axioms.push(VerifyExpr::eq(sum_of(a, a), a.clone()));
            for b in &universe {
                // Commutativity: a ⊕ b = b ⊕ a
                axioms.push(VerifyExpr::eq(sum_of(a, b), sum_of(b, a)));
                // Parthood: Part(x, y) ↔ x ⊕ y = y, with y ranging over
                // atoms AND pairwise sums (so Part(a, a⊕b) is decidable).
                axioms.push(VerifyExpr::Iff(
                    Box::new(VerifyExpr::apply("Part", vec![a.clone(), b.clone()])),
                    Box::new(VerifyExpr::eq(sum_of(a, b), b.clone())),
                ));
                for c in &universe {
                    let bc = sum_of(b, c);
                    axioms.push(VerifyExpr::Iff(
                        Box::new(VerifyExpr::apply("Part", vec![a.clone(), bc.clone()])),
                        Box::new(VerifyExpr::eq(sum_of(a, &bc), bc.clone())),
                    ));
                    // Associativity: (a ⊕ b) ⊕ c = a ⊕ (b ⊕ c)
                    axioms.push(VerifyExpr::eq(
                        VerifyExpr::apply_int("sum", vec![sum_of(a, b), c.clone()]),
                        VerifyExpr::apply_int("sum", vec![a.clone(), bc]),
                    ));
                }
            }
        }

        // CUM(M): M(x) ∧ M(y) → M(x ⊕ y), per mass predicate mentioned.
        for mass in &theory.cumulative_predicates {
            if !mentioned.contains(mass) {
                continue;
            }
            for a in &universe {
                for b in &universe {
                    axioms.push(VerifyExpr::implies(
                        VerifyExpr::and(
                            VerifyExpr::apply(mass, vec![a.clone()]),
                            VerifyExpr::apply(mass, vec![b.clone()]),
                        ),
                        VerifyExpr::apply(mass, vec![sum_of(a, b)]),
                    ));
                }
            }
        }
    }

    // Kind-Theme lifting: consuming a portion of M is consuming the M-kind —
    // M(x) ∧ Theme(e, x) → Theme(e, ^M), with ^M the capitalized kind
    // constant the bare-mass-object parse produces.
    for mass in &theory.cumulative_predicates {
        if !mentioned.contains(mass) {
            continue;
        }
        let mut kind = mass.clone();
        if let Some(first) = kind.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        axioms.push(VerifyExpr::forall(
            vec![
                ("le".to_string(), VerifyType::Int),
                ("lx".to_string(), VerifyType::Int),
            ],
            VerifyExpr::implies(
                VerifyExpr::and(
                    VerifyExpr::apply(mass, vec![VerifyExpr::var("lx")]),
                    VerifyExpr::apply(
                        "Theme",
                        vec![VerifyExpr::var("le"), VerifyExpr::var("lx")],
                    ),
                ),
                VerifyExpr::apply(
                    "Theme",
                    vec![VerifyExpr::var("le"), VerifyExpr::var(&kind)],
                ),
            ),
        ));
    }

    axioms
}

/// Build one SMT problem: declare variables, assume every premise (and the
/// needed frame axioms), and return the translated goal (or a trivially-true
/// placeholder when only consistency is asked, signalled by `goal = None` —
/// the return is then `Some(true)`-shaped only to signal success).
///
/// Modal problems take the standard translation with ONE shared
/// [`WorldTranslation`] across premises and goal, so counterfactuals with
/// identical antecedents share their `Closest` relation. Non-modal problems
/// take the byte-identical legacy encoding. In BOTH paths a premise that
/// fails to convert aborts the build (`None`) — a silently dropped premise
/// could turn a real entailment into `NotEntailed`.
fn build_smt_problem(
    premises: &[ProofExpr],
    goal: Option<&ProofExpr>,
    session: &mut VerificationSession,
    theory: &SmtTheory,
) -> Option<VerifyExpr> {
    for axiom in lattice_axioms(premises, goal, theory) {
        session.assume(&axiom);
    }
    let modal = goal.map(contains_modal_constructs).unwrap_or(false)
        || premises.iter().any(contains_modal_constructs);

    let mut types = TypeInference::new();
    if let Some(g) = goal {
        types.infer_from_expr(g);
    }
    for premise in premises {
        types.infer_from_expr(premise);
    }
    for (name, ty) in types.variables.iter() {
        // In the world-indexed encoding a propositional atom is a unary
        // predicate over worlds, not a Bool constant — don't declare it.
        if modal && matches!(ty, VerifyType::Bool) {
            continue;
        }
        session.declare(name, ty.clone());
    }

    if modal {
        let mut translation = WorldTranslation::new();
        let mut assumed = Vec::with_capacity(premises.len());
        for premise in premises {
            assumed.push(translation.translate(premise, "w0")?);
        }
        let goal_expr = match goal {
            Some(g) => Some(translation.translate(g, "w0")?),
            None => None,
        };
        for axiom in translation.finalize()? {
            session.assume(&axiom);
        }
        for assumption in assumed {
            session.assume(&assumption);
        }
        Some(goal_expr.unwrap_or_else(|| VerifyExpr::bool(true)))
    } else {
        for premise in premises {
            let v = proof_expr_to_verify_expr(premise)?;
            session.assume(&v);
        }
        match goal {
            Some(g) => proof_expr_to_verify_expr(g),
            None => Some(VerifyExpr::bool(true)),
        }
    }
}

// =============================================================================
// INDUCTIVE CONSTRUCT DETECTION
// =============================================================================

/// Check if an expression contains inductive constructs (Ctor, TypedVar, etc.)
/// that Z3 cannot handle without explicit axioms.
fn contains_inductive_constructs(expr: &ProofExpr) -> bool {
    match expr {
        // Inductive constructors - Z3 doesn't understand these
        ProofExpr::Ctor { .. } => true,
        ProofExpr::TypedVar { .. } => true,
        ProofExpr::Match { .. } => true,
        ProofExpr::Fixpoint { .. } => true,

        // Check sub-expressions
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            contains_inductive_constructs(l) || contains_inductive_constructs(r)
        }

        ProofExpr::Not(inner) => contains_inductive_constructs(inner),

        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            contains_inductive_constructs(body)
        }

        // Modal/temporal/counterfactual wrappers: the gate must see through
        // them so a Peano construct inside a modal body is still refused.
        ProofExpr::Modal { body, .. } | ProofExpr::Temporal { body, .. } => {
            contains_inductive_constructs(body)
        }
        ProofExpr::Counterfactual { antecedent, consequent } => {
            contains_inductive_constructs(antecedent)
                || contains_inductive_constructs(consequent)
        }
        ProofExpr::TemporalBinary { left, right, .. } => {
            contains_inductive_constructs(left) || contains_inductive_constructs(right)
        }
        ProofExpr::NeoEvent { roles, .. } => roles
            .iter()
            .any(|(_, term)| contains_inductive_constructs_term(term)),

        ProofExpr::Identity(l, r) => {
            contains_inductive_constructs_term(l) || contains_inductive_constructs_term(r)
        }

        ProofExpr::Predicate { args, .. } => {
            args.iter().any(contains_inductive_constructs_term)
        }

        // Other expressions are fine
        _ => false,
    }
}

/// Check if a term contains inductive constructs.
fn contains_inductive_constructs_term(term: &ProofTerm) -> bool {
    match term {
        ProofTerm::Function(name, args) => {
            // Check for known inductive constructors
            matches!(name.as_str(), "Zero" | "Succ" | "Nil" | "Cons")
                || args.iter().any(contains_inductive_constructs_term)
        }
        ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) => {
            // Check for TypedVar pattern "name:Type"
            v.contains(':')
        }
        ProofTerm::Group(terms) => terms.iter().any(contains_inductive_constructs_term),
        ProofTerm::Constant(_) => false,
    }
}

// =============================================================================
// PUBLIC API
// =============================================================================

/// Attempt to prove a goal using Z3 as an oracle.
///
/// This is the fallback when structural backward chaining fails. Z3 will verify
/// arithmetic, comparisons, and uninterpreted function reasoning.
///
/// # Arguments
///
/// * `goal` - The proof goal to verify
/// * `knowledge_base` - Facts and rules available as assumptions
///
/// # Returns
///
/// * `Ok(Some(tree))` - Z3 verified the goal; returns a [`DerivationTree`] with
///   [`InferenceRule::OracleVerification`]
/// * `Ok(None)` - Z3 cannot verify (unknown, unsat, or unsupported constructs)
/// * `Err(e)` - Internal error during verification
///
/// # Behavior
///
/// The function performs these steps:
/// 1. Check for inductive constructs (returns `None` if found)
/// 2. Infer types for all variables in goal and KB
/// 3. Declare variables in Z3 session
/// 4. Add context and KB as assumptions
/// 5. Convert goal to [`VerifyExpr`](logicaffeine_verify::ir::VerifyExpr)
/// 6. Ask Z3 to verify
///
/// # See Also
///
/// * [`proof_expr_to_verify_expr`] - Conversion from proof to verification expressions
/// * [`BackwardChainer`](crate::BackwardChainer) - The main proof engine that calls this
pub fn try_oracle(
    goal: &ProofGoal,
    knowledge_base: &[ProofExpr],
) -> ProofResult<Option<DerivationTree>> {
    // Skip oracle for goals containing inductive constructs
    // Z3 cannot reason about Peano arithmetic without explicit axioms
    if contains_inductive_constructs(&goal.target) {
        return Ok(None);
    }

    // Also skip if KB contains inductive constructs (they would corrupt Z3 context)
    for kb_expr in knowledge_base {
        if contains_inductive_constructs(kb_expr) {
            return Ok(None);
        }
    }

    // Modal/temporal/counterfactual goals take the standard translation with
    // frame axioms. (The result is still uncertifiable — the kernel rejects
    // OracleVerification leaves — but the engine's answer becomes sound.)
    if contains_modal_constructs(&goal.target)
        || goal.context.iter().any(contains_modal_constructs)
        || knowledge_base.iter().any(contains_modal_constructs)
    {
        let premises: Vec<ProofExpr> = goal
            .context
            .iter()
            .chain(knowledge_base.iter())
            .cloned()
            .collect();
        let mut session = VerificationSession::new();
        let goal_expr = match build_smt_problem(
            &premises,
            Some(&goal.target),
            &mut session,
            &SmtTheory::default(),
        ) {
            Some(g) => g,
            None => return Ok(None),
        };
        return Ok(match session.verify(&goal_expr) {
            Ok(()) => Some(DerivationTree::leaf(
                goal.target.clone(),
                InferenceRule::OracleVerification(
                    "Verified by Z3 (standard modal translation)".into(),
                ),
            )),
            Err(_) => None,
        });
    }

    // Collect all variables and their types
    let mut session = VerificationSession::new();
    let mut types = TypeInference::new();

    // Infer types from goal
    types.infer_from_expr(&goal.target);

    // Infer types from context and KB
    for ctx_expr in &goal.context {
        types.infer_from_expr(ctx_expr);
    }
    for kb_expr in knowledge_base {
        types.infer_from_expr(kb_expr);
    }

    // Declare all inferred variables
    for (name, ty) in types.variables.iter() {
        session.declare(name, ty.clone());
    }

    // Add context assumptions
    for ctx_expr in &goal.context {
        if let Some(verify_expr) = proof_expr_to_verify_expr(ctx_expr) {
            session.assume(&verify_expr);
        }
    }

    // Add KB as assumptions (simplified - in full version, would be more selective)
    for kb_expr in knowledge_base {
        if let Some(verify_expr) = proof_expr_to_verify_expr(kb_expr) {
            session.assume(&verify_expr);
        }
    }

    // Convert goal to VerifyExpr
    let goal_expr = match proof_expr_to_verify_expr(&goal.target) {
        Some(e) => e,
        None => return Ok(None), // Cannot convert, oracle can't help
    };

    // Ask Z3 to verify the goal
    match session.verify(&goal_expr) {
        Ok(()) => {
            // Z3 verified it!
            let tree = DerivationTree::leaf(
                goal.target.clone(),
                InferenceRule::OracleVerification("Verified by Z3".into()),
            );
            Ok(Some(tree))
        }
        Err(_) => {
            // Z3 could not verify (either invalid or unknown)
            Ok(None)
        }
    }
}

// =============================================================================
// TYPE INFERENCE
// =============================================================================

/// Simple type inference for Z3 variable declaration.
struct TypeInference {
    variables: std::collections::HashMap<String, VerifyType>,
}

impl TypeInference {
    fn new() -> Self {
        Self {
            variables: std::collections::HashMap::new(),
        }
    }

    /// Infer types from a ProofExpr.
    fn infer_from_expr(&mut self, expr: &ProofExpr) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.infer_from_term(arg, VerifyType::Int);
                }
            }

            ProofExpr::Identity(left, right) => {
                self.infer_from_term(left, VerifyType::Int);
                self.infer_from_term(right, VerifyType::Int);
            }

            ProofExpr::Atom(name) => {
                // Atoms are boolean propositions
                self.variables.insert(name.clone(), VerifyType::Bool);
            }

            ProofExpr::And(left, right)
            | ProofExpr::Or(left, right)
            | ProofExpr::Implies(left, right)
            | ProofExpr::Iff(left, right) => {
                self.infer_from_expr(left);
                self.infer_from_expr(right);
            }

            ProofExpr::Not(inner) => {
                self.infer_from_expr(inner);
            }

            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.infer_from_expr(body);
            }

            _ => {}
        }
    }

    /// Infer type of a term.
    fn infer_from_term(&mut self, term: &ProofTerm, context_type: VerifyType) {
        match term {
            ProofTerm::Variable(name) | ProofTerm::BoundVarRef(name) => {
                // Use context type if not already declared
                if !self.variables.contains_key(name) {
                    self.variables.insert(name.clone(), context_type);
                }
            }

            ProofTerm::Function(_, args) => {
                for arg in args {
                    self.infer_from_term(arg, VerifyType::Int);
                }
            }

            ProofTerm::Group(terms) => {
                for t in terms {
                    self.infer_from_term(t, VerifyType::Int);
                }
            }

            ProofTerm::Constant(_) => {
                // Constants don't need declaration
            }
        }
    }
}

// =============================================================================
// CONVERSION: ProofExpr → VerifyExpr
// =============================================================================

/// Convert a [`ProofExpr`] to [`VerifyExpr`](logicaffeine_verify::ir::VerifyExpr) for Z3 verification.
///
/// Transforms proof-level expressions into the verification IR that Z3 understands.
///
/// # Returns
///
/// * `Some(expr)` - Successfully converted expression
/// * `None` - Expression contains unsupported constructs
///
/// # Supported Constructs
///
/// | ProofExpr | VerifyExpr |
/// |-----------|------------|
/// | `Atom(name)` | `Var(name)` |
/// | `Predicate { Gt, [x, y] }` | `Binary(Gt, x, y)` |
/// | `And(l, r)` | `And(l, r)` |
/// | `Implies(l, r)` | `Implies(l, r)` |
/// | `ForAll { var, body }` | `ForAll([(var, Int)], body)` |
/// | `Identity(l, r)` | `Eq(l, r)` |
///
/// # Unsupported (returns `None`)
///
/// * `Lambda`, `App` - Higher-order functions
/// * `Ctor`, `Match`, `Fixpoint` - Inductive types
/// * `NeoEvent` - Event semantics
/// * `Hole`, `Term`, `Unsupported` - Meta-constructs
pub fn proof_expr_to_verify_expr(expr: &ProofExpr) -> Option<VerifyExpr> {
    match expr {
        ProofExpr::Atom(name) => Some(VerifyExpr::var(name)),

        ProofExpr::Predicate { name, args, .. } => {
            // Check for built-in comparison predicates
            if args.len() == 2 {
                let left = proof_term_to_verify_expr(&args[0])?;
                let right = proof_term_to_verify_expr(&args[1])?;

                match name.as_str() {
                    "Gt" => return Some(VerifyExpr::gt(left, right)),
                    "Lt" => return Some(VerifyExpr::lt(left, right)),
                    "Gte" => return Some(VerifyExpr::gte(left, right)),
                    "Lte" => return Some(VerifyExpr::lte(left, right)),
                    "Eq" => return Some(VerifyExpr::eq(left, right)),
                    "Neq" => return Some(VerifyExpr::neq(left, right)),
                    _ => {}
                }
            }

            // General predicate → uninterpreted function
            let verify_args: Vec<VerifyExpr> = args
                .iter()
                .filter_map(proof_term_to_verify_expr)
                .collect();
            Some(VerifyExpr::apply(name, verify_args))
        }

        ProofExpr::Identity(left, right) => {
            let l = proof_term_to_verify_expr(left)?;
            let r = proof_term_to_verify_expr(right)?;
            Some(VerifyExpr::eq(l, r))
        }

        ProofExpr::And(left, right) => {
            let l = proof_expr_to_verify_expr(left)?;
            let r = proof_expr_to_verify_expr(right)?;
            Some(VerifyExpr::and(l, r))
        }

        ProofExpr::Or(left, right) => {
            let l = proof_expr_to_verify_expr(left)?;
            let r = proof_expr_to_verify_expr(right)?;
            Some(VerifyExpr::or(l, r))
        }

        ProofExpr::Implies(left, right) => {
            let l = proof_expr_to_verify_expr(left)?;
            let r = proof_expr_to_verify_expr(right)?;
            Some(VerifyExpr::implies(l, r))
        }

        ProofExpr::Iff(left, right) => {
            // A ↔ B is (A → B) ∧ (B → A)
            let l = proof_expr_to_verify_expr(left)?;
            let r = proof_expr_to_verify_expr(right)?;
            Some(VerifyExpr::and(
                VerifyExpr::implies(l.clone(), r.clone()),
                VerifyExpr::implies(r, l),
            ))
        }

        ProofExpr::Not(inner) => {
            let i = proof_expr_to_verify_expr(inner)?;
            Some(VerifyExpr::not(i))
        }

        ProofExpr::ForAll { variable, body } => {
            let b = proof_expr_to_verify_expr(body)?;
            Some(VerifyExpr::forall(
                vec![(variable.clone(), VerifyType::Int)],
                b,
            ))
        }

        ProofExpr::Exists { variable, body } => {
            let b = proof_expr_to_verify_expr(body)?;
            Some(VerifyExpr::exists(
                vec![(variable.clone(), VerifyType::Int)],
                b,
            ))
        }

        // Modal, temporal, and counterfactual operators need the standard
        // translation (world-indexed predicates + accessibility relations) —
        // this legacy encoding cannot express them (an uninterpreted function
        // over a Bool argument would be ill-sorted: `Int^n → Bool` only).
        ProofExpr::Modal { .. }
        | ProofExpr::Counterfactual { .. }
        | ProofExpr::Temporal { .. }
        | ProofExpr::TemporalBinary { .. } => None,

        // Inductive types - unsupported for now
        ProofExpr::Ctor { .. }
        | ProofExpr::Match { .. }
        | ProofExpr::Fixpoint { .. }
        | ProofExpr::TypedVar { .. } => None,

        // Neo-Davidsonian event: ∃e(Verb(e) ∧ Role(e, t) ∧ …)
        ProofExpr::NeoEvent {
            event_var,
            verb,
            roles,
        } => {
            let mut body = VerifyExpr::apply(verb, vec![VerifyExpr::var(event_var)]);
            for (role, term) in roles {
                let t = proof_term_to_verify_expr(term)?;
                body = VerifyExpr::and(
                    body,
                    VerifyExpr::apply(role, vec![VerifyExpr::var(event_var), t]),
                );
            }
            Some(VerifyExpr::exists(
                vec![(event_var.clone(), VerifyType::Int)],
                body,
            ))
        }

        // Others - not representable in Z3
        ProofExpr::Lambda { .. }
        | ProofExpr::App(_, _)
        | ProofExpr::Hole(_)
        | ProofExpr::Term(_)
        | ProofExpr::Unsupported(_) => None,
    }
}

/// Convert a [`ProofTerm`] to [`VerifyExpr`](logicaffeine_verify::ir::VerifyExpr).
///
/// Transforms proof-level terms into verification expressions for Z3.
///
/// # Conversion Rules
///
/// | ProofTerm | VerifyExpr |
/// |-----------|------------|
/// | `Constant("42")` | `Int(42)` |
/// | `Constant("foo")` | `Var("foo")` |
/// | `Variable(name)` | `Var(name)` |
/// | `BoundVarRef(name)` | `Var(name)` |
/// | `Function("Add", [x, y])` | `Binary(Add, x, y)` |
/// | `Function(name, args)` | `Apply(name, args)` |
///
/// Numeric string constants are parsed as integers; non-numeric become variables.
/// Arithmetic functions (`Add`, `Sub`, `Mul`, `Div`) are converted to binary operations.
pub fn proof_term_to_verify_expr(term: &ProofTerm) -> Option<VerifyExpr> {
    match term {
        ProofTerm::Constant(s) => {
            // Try to parse as integer
            if let Ok(n) = s.parse::<i64>() {
                Some(VerifyExpr::int(n))
            } else {
                // Non-numeric constant becomes a variable
                Some(VerifyExpr::var(s))
            }
        }

        ProofTerm::Variable(name) | ProofTerm::BoundVarRef(name) => Some(VerifyExpr::var(name)),

        ProofTerm::Function(name, args) => {
            // Check for built-in arithmetic functions
            if args.len() == 2 {
                let left = proof_term_to_verify_expr(&args[0])?;
                let right = proof_term_to_verify_expr(&args[1])?;

                match name.as_str() {
                    "Add" => {
                        return Some(VerifyExpr::binary(VerifyOp::Add, left, right))
                    }
                    "Sub" => {
                        return Some(VerifyExpr::binary(VerifyOp::Sub, left, right))
                    }
                    "Mul" => {
                        return Some(VerifyExpr::binary(VerifyOp::Mul, left, right))
                    }
                    "Div" => {
                        return Some(VerifyExpr::binary(VerifyOp::Div, left, right))
                    }
                    _ => {}
                }
            }

            // General function in TERM position → Int-valued uninterpreted
            // function (Bool-ranged Apply would be ill-sorted here).
            let verify_args: Vec<VerifyExpr> = args
                .iter()
                .filter_map(proof_term_to_verify_expr)
                .collect();
            Some(VerifyExpr::apply_int(name, verify_args))
        }

        ProofTerm::Group(terms) => {
            // A plural group is the Link-lattice sum of its members:
            // [a, b, c] ↦ sum(a, sum(b, c)).
            let mut converted = terms.iter().filter_map(proof_term_to_verify_expr);
            let first = converted.next()?;
            Some(converted.fold(first, |acc, t| {
                VerifyExpr::apply_int("sum", vec![acc, t])
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_atom() {
        let expr = ProofExpr::Atom("P".into());
        let result = proof_expr_to_verify_expr(&expr);
        assert!(matches!(result, Some(VerifyExpr::Var(s)) if s == "P"));
    }

    #[test]
    fn test_convert_gt_predicate() {
        let expr = ProofExpr::Predicate {
            name: "Gt".into(),
            args: vec![
                ProofTerm::Variable("x".into()),
                ProofTerm::Constant("10".into()),
            ],
            world: None,
        };
        let result = proof_expr_to_verify_expr(&expr);
        assert!(matches!(
            result,
            Some(VerifyExpr::Binary {
                op: VerifyOp::Gt,
                ..
            })
        ));
    }

    #[test]
    fn test_convert_implication() {
        let expr = ProofExpr::Implies(
            Box::new(ProofExpr::Atom("P".into())),
            Box::new(ProofExpr::Atom("Q".into())),
        );
        let result = proof_expr_to_verify_expr(&expr);
        assert!(matches!(
            result,
            Some(VerifyExpr::Binary {
                op: VerifyOp::Implies,
                ..
            })
        ));
    }

    #[test]
    fn test_convert_arithmetic_function() {
        let term = ProofTerm::Function(
            "Add".into(),
            vec![
                ProofTerm::Variable("x".into()),
                ProofTerm::Constant("5".into()),
            ],
        );
        let result = proof_term_to_verify_expr(&term);
        assert!(matches!(
            result,
            Some(VerifyExpr::Binary {
                op: VerifyOp::Add,
                ..
            })
        ));
    }
}
