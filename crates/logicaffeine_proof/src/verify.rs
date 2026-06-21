//! Unified theorem verification — the single door.
//!
//! Every public theorem entry point in the workspace funnels through
//! [`prove_certify_check`], so they all share one trust guarantee:
//!
//! > A proof is `verified` **iff** the backward chainer produced a derivation,
//! > the certifier turned it into a kernel term, **and** the kernel type-checked
//! > that term.
//!
//! A derivation alone never counts as verified — `verified == true` is always
//! backed by a [`Term`] that re-checks under [`infer_type`] in the returned
//! [`Context`]. This core lives in the proof crate (not the compile crate) so
//! both `logicaffeine_language` and `logicaffeine_compile` can reach it without
//! a dependency cycle.

use std::collections::{HashMap, HashSet};

use logicaffeine_kernel::{infer_type, is_subtype, prelude::StandardLibrary, Context, Term, Universe};

use crate::certifier::{certify, proof_expr_to_type, CertificationContext};
use crate::{BackwardChainer, DerivationTree, ProofExpr, ProofTerm};

/// Outcome of running a goal through prove → certify → kernel type-check.
///
/// The invariant: `verified == true` **iff** `proof_term.is_some()` and that
/// term type-checks in `kernel_ctx`. When verification fails, `derivation` may
/// still be present (the chainer found *a* derivation) but it is never reported
/// as verified — `verification_error` explains where the chain broke.
pub struct VerifiedProof {
    /// The derivation found by backward chaining, if any.
    pub derivation: Option<DerivationTree>,
    /// The certified kernel proof term, present only when `verified`.
    pub proof_term: Option<Term>,
    /// The kernel context the proof term was checked in (predicates,
    /// constants, and premise hypotheses registered).
    pub kernel_ctx: Context,
    /// True iff a proof term was certified and kernel type-checked.
    pub verified: bool,
    /// Where the chain broke (search, certification, or type-check), if it did.
    pub verification_error: Option<String>,
}

/// Prove `goal` from `premises`, certify the derivation, and kernel-check it.
///
/// This is the canonical pipeline. Symbols are extracted from the premises and
/// goal and registered in a fresh kernel context (predicates as `Entity → Prop`,
/// constants as `Entity`); each premise is registered as a hypothesis using the
/// **same** conversion the certifier uses for hypothesis lookup, so a registered
/// premise is guaranteed to match.
pub fn prove_certify_check(premises: &[ProofExpr], goal: &ProofExpr) -> VerifiedProof {
    prove_certify_check_bounded(premises, goal, 100)
}

/// Like [`prove_certify_check`] but caps the backward-chainer search depth, so a
/// goal the kernel cannot reach fails FAST instead of exhausting the default depth.
/// Keeps "prove-with-ours-first" cheap when answering a grid cell by cell: a
/// shallow kernel attempt certifies what it can, then falls through to the oracle.
pub fn prove_certify_check_bounded(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    max_depth: usize,
) -> VerifiedProof {
    let (kernel_ctx, flat_premises, abstracted_goal) = prepare_ctx(premises, goal);

    // === Prove ===
    let mut engine = BackwardChainer::new();
    engine.set_max_depth(max_depth);
    for premise in &flat_premises {
        engine.add_axiom(premise.clone());
    }
    let derivation = match engine.prove(goal.clone()) {
        Ok(d) => d,
        Err(e) => {
            return VerifiedProof {
                derivation: None,
                proof_term: None,
                kernel_ctx,
                verified: false,
                verification_error: Some(format!("Proof search failed: {}", e)),
            };
        }
    };

    finish_check(kernel_ctx, &abstracted_goal, derivation)
}

/// Certify and kernel-check a derivation built EXTERNALLY (e.g. by the fast grid
/// solver) against `premises ⊢ goal`, WITHOUT running the backward chainer. The
/// trust guarantee is identical to [`prove_certify_check`]: `verified` is true only
/// when the certifier produced a kernel term whose inferred type IS the goal type.
/// So an external solver sits OUTSIDE the trusted base — it hands us a
/// `DerivationTree`, the kernel re-checks it, and a wrong tree yields
/// `verified == false`, never a false claim.
pub fn check_derivation(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    derivation: DerivationTree,
) -> VerifiedProof {
    let (kernel_ctx, _flat_premises, abstracted_goal) = prepare_ctx(premises, goal);
    finish_check(kernel_ctx, &abstracted_goal, derivation)
}

/// Build the kernel context shared by [`prove_certify_check_bounded`] and
/// [`check_derivation`]: register the standard library, the event-ABSTRACTED,
/// conjunction-SPLIT premises as hypotheses `h1, h2, …`, and every predicate/constant
/// referenced. Returns the context, the flattened premises, and the abstracted goal.
fn prepare_ctx(
    premises: &[ProofExpr],
    goal: &ProofExpr,
) -> (Context, Vec<ProofExpr>, ProofExpr) {
    let mut kernel_ctx = Context::new();
    StandardLibrary::register(&mut kernel_ctx);

    // The search, the hypotheses, and the goal type must all live in ONE
    // language: the engine's event-ABSTRACTED form (∃e(Lie(e) ∧ Agent(e, m))
    // ↦ lie(m)). Hypotheses registered in the raw event form could never be
    // found by the certifier, because derivation leaves cite abstracted
    // conclusions. Premises are also split at top-level conjunctions (the
    // standard sequent move: Γ, A ∧ B ⊢ φ iff Γ, A, B ⊢ φ) so a projected
    // presupposition conjunct is an exactly-matchable hypothesis.
    fn split_conjuncts(expr: &ProofExpr, out: &mut Vec<ProofExpr>) {
        if let ProofExpr::And(l, r) = expr {
            split_conjuncts(l, out);
            split_conjuncts(r, out);
        } else {
            out.push(expr.clone());
        }
    }
    let engine_for_abstraction = BackwardChainer::new();
    let mut flat_premises = Vec::new();
    for premise in premises {
        split_conjuncts(&engine_for_abstraction.abstract_all_events(premise), &mut flat_premises);
    }
    let abstracted_goal = engine_for_abstraction.abstract_all_events(goal);

    // Register predicates and constants referenced by premises and goal.
    let mut collector = SymbolCollector::new();
    for premise in &flat_premises {
        collector.collect(premise);
    }
    collector.collect(&abstracted_goal);
    for (name, arity) in collector.predicates() {
        register_predicate(&mut kernel_ctx, name, arity);
    }
    for name in collector.constants() {
        register_constant(&mut kernel_ctx, name);
    }

    for (i, premise) in flat_premises.iter().enumerate() {
        if let Ok(hyp_type) = proof_expr_to_type(premise) {
            let hyp_name = format!("h{}", i + 1);
            kernel_ctx.add_declaration(&hyp_name, hyp_type);
        }
    }

    (kernel_ctx, flat_premises, abstracted_goal)
}

/// Certify `derivation` and require its inferred kernel type to be the goal type —
/// the trust core shared by the prove and check entries.
fn finish_check(
    kernel_ctx: Context,
    abstracted_goal: &ProofExpr,
    derivation: DerivationTree,
) -> VerifiedProof {
    let trace = std::env::var("LOGOS_TRACE").is_ok();
    // === Certify ===
    let t_cert = std::time::Instant::now();
    let proof_term = {
        let cert_ctx = CertificationContext::new(&kernel_ctx);
        match certify(&derivation, &cert_ctx) {
            Ok(t) => t,
            Err(e) => {
                return VerifiedProof {
                    derivation: Some(derivation),
                    proof_term: None,
                    kernel_ctx,
                    verified: false,
                    verification_error: Some(format!("Certification failed: {}", e)),
                };
            }
        }
    };
    if trace {
        eprintln!(
            "[cert] certify(build) {:.2?} → {} kernel-term nodes",
            t_cert.elapsed(),
            count_term_nodes(&proof_term)
        );
    }

    // === Kernel type-check ===
    // The term must not merely be well-typed — its type must be the goal.
    // Otherwise a certifier that produced a well-formed proof of the *wrong*
    // proposition would be wrongly accepted. We compute the goal's kernel type
    // and require the inferred type to match it (up to definitional equality).
    let t_infer = std::time::Instant::now();
    let inferred = match infer_type(&kernel_ctx, &proof_term) {
        Ok(t) => t,
        Err(e) => {
            return VerifiedProof {
                derivation: Some(derivation),
                proof_term: None,
                kernel_ctx,
                verified: false,
                verification_error: Some(format!("Type check failed: {}", e)),
            };
        }
    };
    if trace {
        eprintln!("[cert] infer_type(check) {:.2?}", t_infer.elapsed());
    }

    let goal_type = match proof_expr_to_type(abstracted_goal) {
        Ok(t) => t,
        Err(e) => {
            return VerifiedProof {
                derivation: Some(derivation),
                proof_term: None,
                kernel_ctx,
                verified: false,
                verification_error: Some(format!(
                    "Cannot express the goal as a kernel type: {}",
                    e
                )),
            };
        }
    };

    if !is_subtype(&kernel_ctx, &inferred, &goal_type) {
        return VerifiedProof {
            derivation: Some(derivation),
            proof_term: None,
            kernel_ctx,
            verified: false,
            verification_error: Some(format!(
                "Proof term proves a different proposition: inferred {:?}, goal {:?}",
                inferred, goal_type
            )),
        };
    }

    VerifiedProof {
        derivation: Some(derivation),
        proof_term: Some(proof_term),
        kernel_ctx,
        verified: true,
        verification_error: None,
    }
}

/// The result of checking a rule set for an internal contradiction.
///
/// `inconsistent == true` is never a bare verdict: it is backed by `proof_term`,
/// a kernel term of type `False` that re-checks under [`infer_type`] in
/// `kernel_ctx`. `conflicting_premises` names the input premises (by index) that
/// the proof of ⊥ actually used — the rules that clash.
pub struct ConflictReport {
    /// True iff the premises are jointly inconsistent *and* a kernel-checked
    /// proof of `False` was produced.
    pub inconsistent: bool,
    /// The kernel proof of `False`, present only when `inconsistent`.
    pub proof_term: Option<Term>,
    /// The kernel context the proof term checks in.
    pub kernel_ctx: Context,
    /// Indices (into the input `premises`) of the premises the proof used.
    pub conflicting_premises: Vec<usize>,
    /// Why detection failed to certify, if a contradiction was sketched but not
    /// kernel-checked (or none was found).
    pub error: Option<String>,
}

/// Detect whether `premises` are jointly inconsistent, returning a
/// kernel-checked proof of `False` and the indices of the clashing premises.
///
/// This is verified conflict detection: where an SMT-only tool returns "unsat",
/// this returns a certificate anyone can re-check, plus *which* rules conflict.
/// A consistent rule set yields `inconsistent == false` with no proof — no false
/// alarms.
pub fn detect_conflict(premises: &[ProofExpr]) -> ConflictReport {
    let falsum = ProofExpr::Atom("⊥".to_string());
    let outcome = prove_certify_check(premises, &falsum);

    if !outcome.verified {
        return ConflictReport {
            inconsistent: false,
            proof_term: None,
            kernel_ctx: outcome.kernel_ctx,
            conflicting_premises: Vec::new(),
            error: outcome.verification_error,
        };
    }

    // Collect which premises the derivation actually referenced. Every premise
    // enters the proof as a `PremiseMatch` leaf (directly, or as the source of a
    // `UniversalInst`); we match those leaf conclusions back to the inputs.
    let mut used: Vec<usize> = Vec::new();
    if let Some(derivation) = &outcome.derivation {
        let mut leaves: Vec<&ProofExpr> = Vec::new();
        collect_premise_leaves(derivation, &mut leaves);
        for (i, premise) in premises.iter().enumerate() {
            if leaves.iter().any(|leaf| *leaf == premise) && !used.contains(&i) {
                used.push(i);
            }
        }
    }

    ConflictReport {
        inconsistent: true,
        proof_term: outcome.proof_term,
        kernel_ctx: outcome.kernel_ctx,
        conflicting_premises: used,
        error: None,
    }
}

/// Collect the conclusions of every `PremiseMatch` leaf in a derivation — the
/// premises (and instantiation sources) the proof draws on.
fn collect_premise_leaves<'a>(tree: &'a DerivationTree, out: &mut Vec<&'a ProofExpr>) {
    if matches!(tree.rule, crate::InferenceRule::PremiseMatch) {
        out.push(&tree.conclusion);
    }
    for premise in &tree.premises {
        collect_premise_leaves(premise, out);
    }
}

// =============================================================================
// Symbol collection & registration (proof + kernel only — no language dep)
// =============================================================================

/// Collects predicate and constant names from a `ProofExpr`. Predicates are
/// keyed to their arity (the max seen) so they register with the right type
/// `Entity → … → Entity → Prop`, which is what lets relations like `shaves(a,b)`
/// type-check.
struct SymbolCollector {
    predicates: HashMap<String, usize>,
    constants: HashSet<String>,
}

impl SymbolCollector {
    fn new() -> Self {
        SymbolCollector {
            predicates: HashMap::new(),
            constants: HashSet::new(),
        }
    }

    fn note_predicate(&mut self, name: &str, arity: usize) {
        self.predicates
            .entry(name.to_string())
            .and_modify(|a| *a = (*a).max(arity))
            .or_insert(arity);
    }

    fn collect(&mut self, expr: &ProofExpr) {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                self.note_predicate(name, args.len());
                for arg in args {
                    self.collect_term(arg);
                }
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.collect(l);
                self.collect(r);
            }
            ProofExpr::Not(inner) => self.collect(inner),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => self.collect(body),
            ProofExpr::Identity(l, r) => {
                self.collect_term(l);
                self.collect_term(r);
            }
            // Atoms are propositional constants, not FOL predicates.
            _ => {}
        }
    }

    fn collect_term(&mut self, term: &ProofTerm) {
        match term {
            ProofTerm::Constant(name) => {
                // Proper names (capitalized) and numeric labels (a year like `2004`) are
                // Entity constants; a bare lowercase name is treated as a variable and
                // left unregistered.
                if name
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase() || c.is_ascii_digit())
                    .unwrap_or(false)
                {
                    self.constants.insert(name.clone());
                }
            }
            ProofTerm::Function(name, args) => {
                self.note_predicate(name, args.len());
                for arg in args {
                    self.collect_term(arg);
                }
            }
            _ => {}
        }
    }

    fn predicates(&self) -> impl Iterator<Item = (&String, usize)> {
        self.predicates.iter().map(|(n, a)| (n, *a))
    }

    fn constants(&self) -> impl Iterator<Item = &String> {
        self.constants.iter()
    }
}

/// Register a predicate `P : Entity → … → Entity → Prop` of the given arity
/// (idempotent). Arity 0 registers a propositional constant `P : Prop`.
#[doc(hidden)]
fn count_term_nodes(t: &Term) -> usize {
    match t {
        Term::App(f, a) => 1 + count_term_nodes(f) + count_term_nodes(a),
        Term::Pi { param_type, body_type, .. } => 1 + count_term_nodes(param_type) + count_term_nodes(body_type),
        Term::Lambda { param_type, body, .. } => 1 + count_term_nodes(param_type) + count_term_nodes(body),
        Term::Match { discriminant, motive, cases } => {
            1 + count_term_nodes(discriminant) + count_term_nodes(motive)
                + cases.iter().map(count_term_nodes).sum::<usize>()
        }
        Term::Fix { body, .. } => 1 + count_term_nodes(body),
        _ => 1,
    }
}

fn register_predicate(ctx: &mut Context, name: &str, arity: usize) {
    if ctx.get_global(name).is_some() {
        return;
    }
    let mut ty = Term::Sort(Universe::Prop);
    for _ in 0..arity {
        ty = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Global("Entity".to_string())),
            body_type: Box::new(ty),
        };
    }
    ctx.add_declaration(name, ty);
}

/// Register a constant `c : Entity` (idempotent).
fn register_constant(ctx: &mut Context, name: &str) {
    if ctx.get_global(name).is_some() {
        return;
    }
    ctx.add_declaration(name, Term::Global("Entity".to_string()));
}
