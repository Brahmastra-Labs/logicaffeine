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

use logicaffeine_kernel::{
    double_check, infer_type, is_subtype, prelude::StandardLibrary, Context, DoubleCheck, Term,
    Universe,
};

use crate::certifier::{certify, proof_expr_to_type, proof_expr_to_type_ctx, CertificationContext};
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

/// A user-introduced predicate definition (Rung 0a), expressed in the proof
/// layer's own vocabulary so `logicaffeine_proof` keeps its
/// no-language-dependency invariant. Read as `name(params) :↔ definiens`.
///
/// Registered as a δ-unfoldable kernel definition (not an inlined expansion), so
/// the definiendum stays a first-class, citable node: `bachelor(x)` remains
/// `bachelor(x)` in the proposition, defeq to its meaning by δ-reduction.
#[derive(Debug, Clone)]
pub struct Definition {
    /// The definiendum predicate name (e.g. `"bachelor"`).
    pub name: String,
    /// The parameter names the definiendum binds (e.g. `["x"]`).
    pub params: Vec<String>,
    /// The definiens — the body the definiendum abbreviates.
    pub definiens: ProofExpr,
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

/// One theorem in a dependency-ordered library: proved from its own `premises` plus
/// the conclusions of the theorems it `cites`. This is the unit the multi-theorem
/// driver discharges in citation order (the Euclid-graph engine).
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryTheorem {
    pub name: String,
    pub premises: Vec<ProofExpr>,
    pub goal: ProofExpr,
    /// Names of earlier theorems whose conclusions this proof relies on.
    pub cites: Vec<String>,
}

/// The outcome of proving one theorem in a library.
pub struct LibraryResult {
    pub name: String,
    pub verified: bool,
    pub verification_error: Option<String>,
}

/// Order theorems so every citation precedes its citer. A theorem is ready once all
/// the theorems it cites (by name; unknown names are treated as external givens) are
/// already ordered. Any theorems left in a citation cycle are appended last (they
/// will simply fail to find their cyclic lemma). Stable: independent theorems keep
/// their input order.
pub(crate) fn citation_order(theorems: &[LibraryTheorem]) -> Vec<usize> {
    use std::collections::HashSet;
    let known: HashSet<&str> = theorems.iter().map(|t| t.name.as_str()).collect();
    let mut placed: HashSet<usize> = HashSet::new();
    let mut placed_names: HashSet<&str> = HashSet::new();
    let mut order = Vec::with_capacity(theorems.len());
    loop {
        let mut progressed = false;
        for (i, t) in theorems.iter().enumerate() {
            if placed.contains(&i) {
                continue;
            }
            let ready = t
                .cites
                .iter()
                .all(|c| !known.contains(c.as_str()) || placed_names.contains(c.as_str()));
            if ready {
                order.push(i);
                placed.insert(i);
                placed_names.insert(t.name.as_str());
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    for i in 0..theorems.len() {
        if !placed.contains(&i) {
            order.push(i);
        }
    }
    order
}

/// Discharge a library of theorems in citation order. Each theorem is proved from
/// its own premises plus the conclusions of the (already-proved) theorems it cites,
/// so a citation graph is walked exactly like the scraped Euclid dependency graph.
/// Results are returned in the INPUT order. A citation of an unproved/failed theorem
/// simply isn't available as a premise (so the dependent proof fails too).
pub fn prove_library(theorems: &[LibraryTheorem]) -> Vec<LibraryResult> {
    prove_library_with_axioms(&[], theorems)
}

/// Like [`prove_library`] but with a shared `axioms` base in scope for every
/// theorem — a named theory (e.g. the Tarski geometry axioms) on which the whole
/// dependency graph is discharged.
pub fn prove_library_with_axioms(
    axioms: &[ProofExpr],
    theorems: &[LibraryTheorem],
) -> Vec<LibraryResult> {
    use std::collections::HashMap;
    let mut proved: HashMap<&str, &ProofExpr> = HashMap::new();
    let mut by_name: HashMap<&str, LibraryResult> = HashMap::new();

    for &i in &citation_order(theorems) {
        let t = &theorems[i];
        let mut premises = axioms.to_vec();
        premises.extend(t.premises.iter().cloned());
        for cite in &t.cites {
            if let Some(goal) = proved.get(cite.as_str()) {
                premises.push((*goal).clone());
            }
        }
        let vp = prove_certify_check(&premises, &t.goal);
        if vp.verified {
            proved.insert(t.name.as_str(), &t.goal);
        }
        by_name.insert(
            t.name.as_str(),
            LibraryResult {
                name: t.name.clone(),
                verified: vp.verified,
                verification_error: vp.verification_error,
            },
        );
    }

    theorems
        .iter()
        .map(|t| by_name.remove(t.name.as_str()).expect("every theorem produces a result"))
        .collect()
}

/// Like [`prove_certify_check`] but with user [`Definition`]s in scope (Rung 0a).
pub fn prove_certify_check_with_defs(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    definitions: &[Definition],
) -> VerifiedProof {
    prove_certify_check_bounded_with_defs(premises, goal, definitions, 100)
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
    prove_certify_check_bounded_with_defs(premises, goal, &[], max_depth)
}

/// The depth-bounded pipeline with user definitions (Rung 0a). Definitions are
/// validated (recursion + lowering) up front, registered as δ-unfoldable kernel
/// definitions in the context, then the goal is proved and kernel-checked. The
/// engine still treats the definiendum opaquely during *search* (Stride 3 adds
/// expand-for-search); δ reconciles the certified term with the folded goal type
/// at the [`finish_check`] root.
pub fn prove_certify_check_bounded_with_defs(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    definitions: &[Definition],
    max_depth: usize,
) -> VerifiedProof {
    // The entire search + certify + kernel-check pipeline runs on ONE large-stack
    // thread, so a legitimately deep proof never overflows the native stack and the
    // common (shallow) case pays a single thread spawn.
    on_big_stack(|| {
        prove_certify_check_bounded_with_defs_inner(premises, goal, definitions, max_depth)
    })
}

fn prove_certify_check_bounded_with_defs_inner(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    definitions: &[Definition],
    max_depth: usize,
) -> VerifiedProof {
    let abstracted = abstract_definitions(definitions);
    let definitions = &abstracted[..];
    if let Err(e) = validate_definitions(definitions) {
        return definition_error(e);
    }

    // Expand-for-search: the backward chainer treats a definiendum opaquely, so
    // δ-expand defined predicates in the premises and goal before search — the
    // engine then proves over ordinary predicates. The goal *type* stays FOLDED
    // (abstracted from the ORIGINAL goal), so the certified proposition is still
    // `glorp(A)`, reconciled to its unfolding by δ at the `finish_check` root.
    //
    // FAST PATH: with no definitions, the originals pass through untouched — no
    // expansion walk, no clone — so ordinary theorems pay nothing for Rung 0a.
    let expanded: Option<(Vec<ProofExpr>, ProofExpr)> = if definitions.is_empty() {
        None
    } else {
        let defmap: HashMap<&str, &Definition> =
            definitions.iter().map(|d| (d.name.as_str(), d)).collect();
        let ep = premises
            .iter()
            .map(|p| expand_defs_in_expr(p, &defmap, 0))
            .collect();
        let eg = expand_defs_in_expr(goal, &defmap, 0);
        Some((ep, eg))
    };
    let (search_premises, search_goal): (&[ProofExpr], &ProofExpr) = match &expanded {
        Some((ep, eg)) => (ep, eg),
        None => (premises, goal),
    };

    // Build the context and hypotheses from the (expanded) search premises so the
    // certifier's PremiseMatch leaves resolve.
    let (kernel_ctx, flat_premises, prepared_goal) =
        prepare_ctx_with_defs(search_premises, search_goal, definitions);
    // Folded goal type: when nothing was expanded, `prepared_goal` is already the
    // folded form; otherwise re-abstract the ORIGINAL goal (the definiendum stays
    // a citable node).
    let abstracted_goal = if expanded.is_none() {
        prepared_goal
    } else {
        BackwardChainer::new().abstract_all_events(goal)
    };

    // === Prove ===
    let mut engine = BackwardChainer::new();
    engine.set_max_depth(max_depth);
    for premise in &flat_premises {
        engine.add_axiom(premise.clone());
    }
    // The engine searches over event-ABSTRACTED premises, so the goal must be
    // abstracted to the same form (`Alice admires Bob` ⇒ `admire(Alice, Bob)`).
    let search_goal = engine.abstract_all_events(search_goal);
    let derivation = match engine.prove(search_goal) {
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
    check_derivation_with_defs(premises, goal, &[], derivation)
}

/// Like [`check_derivation`] but with user [`Definition`]s in scope (Rung 0a), so
/// an externally-built derivation of a definiens can certify against a goal
/// stated with the definiendum — δ reconciles them at the root.
pub fn check_derivation_with_defs(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    definitions: &[Definition],
    derivation: DerivationTree,
) -> VerifiedProof {
    on_big_stack(|| check_derivation_with_defs_inner(premises, goal, definitions, derivation))
}

fn check_derivation_with_defs_inner(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    definitions: &[Definition],
    derivation: DerivationTree,
) -> VerifiedProof {
    let abstracted = abstract_definitions(definitions);
    let definitions = &abstracted[..];
    if let Err(e) = validate_definitions(definitions) {
        return definition_error(e);
    }
    let (kernel_ctx, _flat_premises, abstracted_goal) =
        prepare_ctx_with_defs(premises, goal, definitions);
    finish_check(kernel_ctx, &abstracted_goal, derivation)
}

/// Build the kernel context shared by [`prove_certify_check_bounded`] and
/// [`check_derivation`]: register the standard library, any user [`Definition`]s
/// (Rung 0a), the event-ABSTRACTED, conjunction-SPLIT premises as hypotheses
/// `h1, h2, …`, and every predicate/constant referenced. Returns the context, the
/// flattened premises, and the abstracted goal.
fn prepare_ctx_with_defs(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    definitions: &[Definition],
) -> (Context, Vec<ProofExpr>, ProofExpr) {
    let mut kernel_ctx = Context::new();
    StandardLibrary::register(&mut kernel_ctx);

    // Rung 0a: register user definitions FIRST, so a definiendum becomes a
    // δ-unfoldable definition (with a body) rather than an opaque axiom. The
    // opaque `register_predicate` pass below then skips it (idempotent on
    // `get_global`). `validate_definitions` already rejected recursive /
    // unlowerable definitions, so registration here cannot loop.
    for def in definitions {
        register_definition(&mut kernel_ctx, def);
    }

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
        let abstracted = engine_for_abstraction.abstract_all_events(premise);
        // Biconditional elimination: rewrite `P ↔ Q` premises into `(P→Q) ∧ (Q→P)`
        // (the kernel has no `Iff` type), so either direction is usable by the
        // existing implication + modus-ponens machinery after conjunction split.
        split_conjuncts(&expand_iff(&abstracted), &mut flat_premises);
    }
    let abstracted_goal = engine_for_abstraction.abstract_all_events(goal);

    // Register predicates and constants referenced by premises and goal.
    let mut collector = SymbolCollector::new();
    for premise in &flat_premises {
        collector.collect(premise);
    }
    collector.collect(&abstracted_goal);
    // Definiens bodies reference atomic predicates (and constants) that must be
    // registered opaquely too, or kernel type-checking of the definition body
    // fails. The definiendum itself is already a global (registered above), so
    // `register_predicate` skips it.
    for def in definitions {
        collector.collect(&def.definiens);
    }
    // Inductive-domain predicates first (the induction motive needs `P : Ind → Prop`,
    // e.g. `Nat → Prop` or `List → Prop`); the generic `Entity` registration below is
    // idempotent and skips them.
    for (name, ind) in collector.inductive_predicates() {
        if kernel_ctx.get_global(name).is_none() {
            kernel_ctx.add_declaration(
                name,
                Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(Term::Global(ind.to_string())),
                    body_type: Box::new(Term::Sort(Universe::Prop)),
                },
            );
        }
    }
    for (name, arity) in collector.predicates() {
        register_predicate(&mut kernel_ctx, name, arity);
    }
    for (name, arity) in collector.functions() {
        register_function(&mut kernel_ctx, name, arity);
    }
    // Temporal modalities `Op : Prop → Prop` (`Past`, `Future`, …), so a tensed
    // proposition `Past(P)` type-checks as a distinct proposition from `P`.
    for name in collector.modalities() {
        register_modality(&mut kernel_ctx, name);
    }
    // Atoms in arithmetic/comparison positions are `Int`, declared before the
    // generic `Entity` constants so `register_constant` skips them.
    for name in collector.int_atoms() {
        if kernel_ctx.get_global(name).is_none() {
            kernel_ctx.add_declaration(name, Term::Global("Int".to_string()));
        }
    }
    // Definition parameters are λ-bound in the registered body, not constants;
    // skip them so the definiens does not leak a spurious `p : Entity` global.
    let param_names: HashSet<&str> = definitions
        .iter()
        .flat_map(|d| d.params.iter().map(String::as_str))
        .collect();
    for name in collector.constants() {
        if param_names.contains(name.as_str()) {
            continue;
        }
        register_constant(&mut kernel_ctx, name);
    }

    for (i, premise) in flat_premises.iter().enumerate() {
        if let Ok(hyp_type) = proof_expr_to_type_ctx(premise, &kernel_ctx) {
            let hyp_name = format!("h{}", i + 1);
            kernel_ctx.add_declaration(&hyp_name, hyp_type);
        }
    }

    // Also register each original CONJUNCTIVE premise as a WHOLE hypothesis, in
    // addition to its split conjuncts. A derivation that references the conjunction
    // itself — e.g. a tactic `cases`/`ConjunctionElim` projecting `A` out of `A ∧ B`
    // — then resolves, while the search keeps using the flat conjuncts it prefers.
    for (i, premise) in premises.iter().enumerate() {
        let whole = expand_iff(&engine_for_abstraction.abstract_all_events(premise));
        if matches!(whole, ProofExpr::And(_, _)) {
            if let Ok(hyp_type) = proof_expr_to_type(&whole) {
                kernel_ctx.add_declaration(&format!("hw{}", i + 1), hyp_type);
            }
        }
    }

    (kernel_ctx, flat_premises, abstracted_goal)
}

/// Certify `derivation` and require its inferred kernel type to be the goal type —
/// the trust core shared by the prove and check entries.
/// Run `f` on a thread with a large stack. Proof search AND certification both
/// recurse to a depth proportional to the proof: a long chain re-derived from the
/// axioms is legitimately deep, so the default ~8 MB native stack is not enough.
/// The `max_depth` bound still terminates search and the loop detector still prunes
/// regress — this only removes the *native*-stack ceiling so a deep, finite proof
/// completes instead of aborting (the standard recursive-descent remedy).
pub(crate) fn on_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    // wasm has no threads, so `spawn_scoped` would return `Unsupported` and abort every in-browser
    // proof. Run inline there — the `max_depth` bound and loop detector still terminate search; only
    // the *native* ~8 MB stack ceiling needs the worker thread, and wasm sizes its stack at build
    // time instead. The browser links this crate, so this path must never spawn.
    #[cfg(target_arch = "wasm32")]
    {
        f()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(s, f)
                .expect("spawn proof thread")
                .join()
                .expect("proof thread panicked")
        })
    }
}

fn finish_check(
    kernel_ctx: Context,
    abstracted_goal: &ProofExpr,
    derivation: DerivationTree,
) -> VerifiedProof {
    let trace = std::env::var("LOGOS_TRACE").is_ok();
    // === Certify ===
    let t_cert = trace.then(std::time::Instant::now);
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
    if let Some(t_cert) = t_cert {
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
    let t_infer = trace.then(std::time::Instant::now);
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
    if let Some(t_infer) = t_infer {
        eprintln!("[cert] infer_type(check) {:.2?}", t_infer.elapsed());
    }

    let goal_type = match proof_expr_to_type_ctx(abstracted_goal, &kernel_ctx) {
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

    // === Independent re-check (R1, the de Bruijn criterion) ===
    // A second, independently-written kernel (a de Bruijn checker, distinct from
    // `infer_type` above) must concur. `Agreed` is two-kernel verification; an honest
    // `Incomplete` (a fragment the re-checker does not cover, e.g. a δ-conversion it keeps
    // opaque) leaves the main kernel authoritative. Only a genuine `Disagree` — one kernel
    // accepts, the other rejects, or they infer different types — rejects the proof: that
    // is exactly the soundness alarm a second kernel exists to raise.
    if let DoubleCheck::Disagree(why) = double_check(&kernel_ctx, &proof_term) {
        return VerifiedProof {
            derivation: Some(derivation),
            proof_term: None,
            kernel_ctx,
            verified: false,
            verification_error: Some(format!("Independent re-checker disagreement: {}", why)),
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
    functions: HashMap<String, usize>,
    constants: HashSet<String>,
    /// Lowercase `Constant` names — candidate individual constants. The pipeline
    /// capitalizes proper names, but a definite description ("the butler") yields
    /// a lowercase `Constant("butler")`. Such a name IS an individual constant
    /// unless the SAME name is also used as a genuine variable (a quantifier
    /// binder or a `Variable` occurrence), in which case the lowercase `Constant`
    /// is a stray variable spelling and must stay unregistered.
    weak_constants: HashSet<String>,
    /// Names that occur as a genuine variable — a quantifier binder or a
    /// `ProofTerm::Variable`. A `weak_constants` candidate colliding with one of
    /// these is NOT registered as a constant.
    variables: HashSet<String>,
    /// Atoms that occur in an arithmetic/comparison argument position, so must be
    /// typed `Int` (not the `Entity` default) — `le`/`lt`/… need `Int` operands.
    int_atoms: HashSet<String>,
    /// Unary predicates applied to an inductive-type constructor (`Zero`/`Succ` →
    /// `Nat`, `Nil`/`Cons` → `List`), mapped to that inductive type's name, so they
    /// are typed `<Ind> → Prop` (not the `Entity` default) — the motive of a
    /// structural-`induction` proof is `λx:Ind. P(x)`, so `P` must accept `Ind`.
    inductive_predicates: HashMap<String, &'static str>,
    /// Temporal modality operators (`Past`, `Future`, …) seen wrapping a
    /// proposition, each registered as an opaque `Op : Prop → Prop`.
    modalities: HashSet<String>,
}

impl SymbolCollector {
    fn new() -> Self {
        SymbolCollector {
            predicates: HashMap::new(),
            functions: HashMap::new(),
            constants: HashSet::new(),
            weak_constants: HashSet::new(),
            variables: HashSet::new(),
            int_atoms: HashSet::new(),
            inductive_predicates: HashMap::new(),
            modalities: HashSet::new(),
        }
    }

    /// Mark a term that sits in an Int position. Atoms become `Int`; nested
    /// arithmetic propagates the Int-ness to its own operands.
    fn mark_int(&mut self, term: &ProofTerm) {
        match term {
            // Numeric literals lower to `Lit(Int)` (certifier) — no declaration.
            ProofTerm::Constant(s) if s.parse::<i64>().is_ok() => {}
            ProofTerm::Constant(s) | ProofTerm::Variable(s) | ProofTerm::BoundVarRef(s) => {
                self.int_atoms.insert(s.clone());
            }
            ProofTerm::Function(name, args)
                if matches!(name.as_str(), "add" | "sub" | "mul" | "div" | "mod") =>
            {
                for a in args {
                    self.mark_int(a);
                }
            }
            _ => {}
        }
    }

    fn note_predicate(&mut self, name: &str, arity: usize) {
        self.predicates
            .entry(name.to_string())
            .and_modify(|a| *a = (*a).max(arity))
            .or_insert(arity);
    }

    /// A `ProofTerm::Function` is Entity-valued (a function on entities), never a
    /// proposition — only `ProofExpr::Predicate` is propositional. Registering it
    /// as `Entity → … → Entity` (not `… → Prop`) lets `F(a)` appear inside terms
    /// like `Eq Entity (F a) (F b)`, the basis for congruence.
    fn note_function(&mut self, name: &str, arity: usize) {
        self.functions
            .entry(name.to_string())
            .and_modify(|a| *a = (*a).max(arity))
            .or_insert(arity);
    }

    fn collect(&mut self, expr: &ProofExpr) {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                self.note_predicate(name, args.len());
                // A unary predicate applied to an inductive-type constructor is
                // `<Ind> → Prop`, so it can be the motive of a structural induction.
                if args.len() == 1 {
                    if let Some(ind) = constructor_domain(&args[0]) {
                        self.inductive_predicates.insert(name.clone(), ind);
                    }
                }
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
            ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
                self.variables.insert(variable.clone());
                self.collect(body);
            }
            // A temporal modality `Op(P)`: note the operator (registered as
            // `Op : Prop → Prop`) and recurse so `P`'s own predicates register.
            ProofExpr::Temporal { operator, body } => {
                self.modalities.insert(operator.clone());
                self.collect(body);
            }
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
                // Proper names (capitalized) and numeric labels (a year like `2004`)
                // are unambiguously Entity constants. A lowercase name — a definite
                // description's referent ("the butler" → `Constant("butler")`) — is a
                // constant TOO, but only if it is never used as a genuine variable
                // (a binder or `Variable` occurrence); that collision is resolved in
                // `constants()`, once every symbol has been seen.
                if name
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase() || c.is_ascii_digit())
                    .unwrap_or(false)
                {
                    self.constants.insert(name.clone());
                } else {
                    self.weak_constants.insert(name.clone());
                }
            }
            ProofTerm::Variable(name) | ProofTerm::BoundVarRef(name) => {
                self.variables.insert(name.clone());
            }
            ProofTerm::Function(name, args) => {
                // Arithmetic / comparison builtins are prelude globals (typed
                // `Int → … → Int`/`Bool`); don't re-declare them, but type their
                // operands as `Int`. Other functions are uninterpreted (Entity).
                if matches!(
                    name.as_str(),
                    "le" | "lt" | "ge" | "gt" | "add" | "sub" | "mul" | "div" | "mod"
                ) {
                    for arg in args {
                        self.mark_int(arg);
                    }
                } else {
                    self.note_function(name, args.len());
                }
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

    fn functions(&self) -> impl Iterator<Item = (&String, usize)> {
        self.functions.iter().map(|(n, a)| (n, *a))
    }

    fn int_atoms(&self) -> impl Iterator<Item = &String> {
        self.int_atoms.iter()
    }

    fn inductive_predicates(&self) -> impl Iterator<Item = (&String, &'static str)> {
        self.inductive_predicates.iter().map(|(n, d)| (n, *d))
    }

    fn constants(&self) -> Vec<&String> {
        // Capitalized/numeric constants always register; a lowercase candidate
        // registers only when the name is never a genuine variable.
        self.constants
            .iter()
            .chain(
                self.weak_constants
                    .iter()
                    .filter(|n| !self.variables.contains(*n)),
            )
            .collect()
    }

    fn modalities(&self) -> impl Iterator<Item = &String> {
        self.modalities.iter()
    }
}

/// The inductive type whose constructor `t` is — `Zero`/`Succ` ⇒ `Nat`,
/// `ENil`/`ECons` ⇒ `EList` — or `None` if `t` is not a recognized prelude
/// constructor. The signal that a unary predicate applied to `t` ranges over that
/// inductive type, not `Entity`, so it can serve as a structural-induction motive
/// `λx:Ind. P(x)`. (The `E`-prefixed list names are the prelude's monomorphic
/// `EList`, distinct from a user program's parametric `List`/`Nil`/`Cons`.)
fn constructor_domain(t: &ProofTerm) -> Option<&'static str> {
    let name = match t {
        ProofTerm::Constant(s) => s.as_str(),
        ProofTerm::Function(n, _) => n.as_str(),
        _ => return None,
    };
    match name {
        "Zero" | "Succ" => Some("Nat"),
        "ENil" | "ECons" => Some("EList"),
        _ => None,
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

/// Register a function symbol `f : Entity → … → Entity → Entity` of the given arity
/// (idempotent). Unlike a predicate, a function is Entity-valued, so `f(a)` is a
/// term that can be compared by equality and rewritten under congruence.
fn register_function(ctx: &mut Context, name: &str, arity: usize) {
    if ctx.get_global(name).is_some() {
        return;
    }
    let mut ty = Term::Global("Entity".to_string());
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

/// Register a temporal modality `Op : Prop → Prop` (idempotent). A tensed
/// proposition `Past(P)` lowers to `(Op P)`, distinct from `P`, so a modus-tollens
/// chain over tensed premises certifies.
fn register_modality(ctx: &mut Context, name: &str) {
    if ctx.get_global(name).is_some() {
        return;
    }
    ctx.add_declaration(
        name,
        Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Sort(Universe::Prop)),
        },
    );
}

/// A failed-verification result carrying a definition-level error message.
fn definition_error(message: String) -> VerifiedProof {
    VerifiedProof {
        derivation: None,
        proof_term: None,
        kernel_ctx: Context::new(),
        verified: false,
        verification_error: Some(message),
    }
}

/// Abstract neo-Davidsonian events in each definiens into first-order form — the
/// SAME transformation premises and goals get in [`prepare_ctx_with_defs`]. A
/// definiens over a verb (`someone admires x` ⇒ a `NeoEvent`) becomes an ordinary
/// predicate the kernel can type and the engine can match against the theorem's
/// (also-abstracted) events.
fn abstract_definitions(definitions: &[Definition]) -> Vec<Definition> {
    let abstractor = BackwardChainer::new();
    definitions
        .iter()
        .map(|d| Definition {
            name: d.name.clone(),
            params: d.params.clone(),
            definiens: abstractor.abstract_all_events(&d.definiens),
        })
        .collect()
}

/// Reject definitions that cannot be soundly registered (Rung 0a guards):
/// self-recursive definienda (δ-unfolding would not terminate) and definiens
/// bodies the kernel lowering cannot type. Surfacing these up front gives a
/// clear message instead of a silent opaque fallback or a fuel-capped failure.
///
/// Mutual recursion *across* definitions is a Rung 0b (library DAG) concern and
/// is not yet detected here; the kernel's normalize fuel cap is the backstop —
/// an unguarded cycle fails the proof, it never hangs or returns a false proof.
fn validate_definitions(definitions: &[Definition]) -> Result<(), String> {
    if definitions.is_empty() {
        return Ok(());
    }
    for def in definitions {
        if let Err(e) = proof_expr_to_type(&def.definiens) {
            return Err(format!("cannot lower definition `{}`: {:?}", def.name, e));
        }
    }
    // δ-unfolding a cycle (self or mutual) would not terminate. The def→def graph
    // catches both; `find_definition_cycle` returns the offending names.
    if let Some(cycle) = find_definition_cycle(definitions) {
        return Err(format!(
            "circular definition among {{{}}}: a definition may not recursively refer \
             to itself, directly or transitively",
            cycle.join(", ")
        ));
    }
    Ok(())
}

/// Register one definition as a δ-unfoldable kernel definition:
/// `name : Entity → … → Prop := λ(p₁:Entity)…λ(pₙ:Entity). <definiens>`.
///
/// The definiens lowers its parameters to `Global(p)` (constants), but a kernel
/// `Lambda` binds `Var(p)` — so we rewrite `Global(p) → Var(p)` for each
/// parameter before abstracting. Assumes [`validate_definitions`] has passed.
fn register_definition(ctx: &mut Context, def: &Definition) {
    if ctx.get_global(&def.name).is_some() {
        return;
    }
    let mut body = match proof_expr_to_type(&def.definiens) {
        Ok(b) => b,
        Err(_) => {
            // Defensive: validation should have caught this. Keep the context
            // well-formed by registering the definiendum as an opaque predicate.
            register_predicate(ctx, &def.name, def.params.len());
            return;
        }
    };
    for p in &def.params {
        body = subst_global_to_var(body, p);
    }
    // Abstract innermost-last so parameters bind left-to-right.
    for p in def.params.iter().rev() {
        body = Term::Lambda {
            param: p.clone(),
            param_type: Box::new(Term::Global("Entity".to_string())),
            body: Box::new(body),
        };
    }
    let mut ty = Term::Sort(Universe::Prop);
    for _ in &def.params {
        ty = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Global("Entity".to_string())),
            body_type: Box::new(ty),
        };
    }
    ctx.add_definition(def.name.clone(), ty, body);
}

/// Rewrite every free `Global(name)` in `term` to `Var(name)` — used to turn a
/// definiens parameter (lowered as a global constant) into a bindable variable
/// before λ-abstraction.
fn subst_global_to_var(term: Term, name: &str) -> Term {
    match term {
        Term::Global(n) => {
            if n == name {
                Term::Var(n)
            } else {
                Term::Global(n)
            }
        }
        Term::App(f, a) => Term::App(
            Box::new(subst_global_to_var(*f, name)),
            Box::new(subst_global_to_var(*a, name)),
        ),
        Term::Pi { param, param_type, body_type } => Term::Pi {
            param,
            param_type: Box::new(subst_global_to_var(*param_type, name)),
            body_type: Box::new(subst_global_to_var(*body_type, name)),
        },
        Term::Lambda { param, param_type, body } => Term::Lambda {
            param,
            param_type: Box::new(subst_global_to_var(*param_type, name)),
            body: Box::new(subst_global_to_var(*body, name)),
        },
        Term::Match { discriminant, motive, cases } => Term::Match {
            discriminant: Box::new(subst_global_to_var(*discriminant, name)),
            motive: Box::new(subst_global_to_var(*motive, name)),
            cases: cases.into_iter().map(|c| subst_global_to_var(c, name)).collect(),
        },
        Term::Fix { name: fix_name, body } => Term::Fix {
            name: fix_name,
            body: Box::new(subst_global_to_var(*body, name)),
        },
        other => other,
    }
}

/// Collect, into `out`, every predicate name in `expr` that is one of the
/// `defined` names — i.e. the definitions this expression directly *uses*. Walks
/// the propositional structure (predicates are δ-expanded only in predicate
/// position, so that is exactly what we scan). Deduplicated, insertion-ordered.
fn collect_defined_predicates(expr: &ProofExpr, defined: &HashSet<&str>, out: &mut Vec<String>) {
    match expr {
        ProofExpr::Predicate { name, .. } => {
            if defined.contains(name.as_str()) && !out.iter().any(|n| n == name) {
                out.push(name.clone());
            }
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_defined_predicates(l, defined, out);
            collect_defined_predicates(r, defined, out);
        }
        ProofExpr::Not(p) => collect_defined_predicates(p, defined, out),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            collect_defined_predicates(body, defined, out)
        }
        _ => {}
    }
}

/// The direct def→def `uses` edges: for each definition, the OTHER definitions
/// its definiens references. O(total definiens size); membership is O(1).
fn def_edges(definitions: &[Definition]) -> Vec<(String, Vec<String>)> {
    let defined: HashSet<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    definitions
        .iter()
        .map(|d| {
            let mut uses = Vec::new();
            collect_defined_predicates(&d.definiens, &defined, &mut uses);
            (d.name.clone(), uses)
        })
        .collect()
}

/// Detect a cycle (self-loop or mutual) in the def→def `uses` graph via Kahn's
/// topological elimination: if not every node can be peeled to zero in-degree,
/// the residual nodes form/feed cycles. Returns those names (sorted), or `None`
/// for a DAG. O(V + E).
fn find_definition_cycle(definitions: &[Definition]) -> Option<Vec<String>> {
    let edges = def_edges(definitions);
    let adj: HashMap<&str, &[String]> =
        edges.iter().map(|(n, u)| (n.as_str(), u.as_slice())).collect();

    let mut indeg: HashMap<&str, usize> = adj.keys().map(|n| (*n, 0usize)).collect();
    for deps in adj.values() {
        for d in deps.iter() {
            if let Some(c) = indeg.get_mut(d.as_str()) {
                *c += 1;
            }
        }
    }

    let mut queue: Vec<&str> = indeg
        .iter()
        .filter(|(_, &c)| c == 0)
        .map(|(n, _)| *n)
        .collect();
    let mut removed = 0usize;
    while let Some(n) = queue.pop() {
        removed += 1;
        if let Some(deps) = adj.get(n) {
            for d in deps.iter() {
                if let Some(c) = indeg.get_mut(d.as_str()) {
                    *c -= 1;
                    if *c == 0 {
                        queue.push(d.as_str());
                    }
                }
            }
        }
    }

    if removed == adj.len() {
        None
    } else {
        let mut cyclic: Vec<String> = indeg
            .iter()
            .filter(|(_, &c)| c > 0)
            .map(|(n, _)| n.to_string())
            .collect();
        cyclic.sort();
        Some(cyclic)
    }
}

/// The `uses` dependency graph among definitions and from a theorem to its
/// definitions — the Rung 0b graph seed (each node a definition/theorem, each
/// edge a `uses`). Direct edges only; transitive use is a query over this. This
/// is the structure a `mathscrapes` node/edge compiles into.
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// definiendum name → the defined names its definiens directly uses.
    pub def_uses: Vec<(String, Vec<String>)>,
    /// the defined names the theorem's premises + goal directly use.
    pub theorem_uses: Vec<String>,
}

/// Build the [`DependencyGraph`] for a set of definitions plus a theorem
/// (premises + goal), recording which definitions each definiens uses and which
/// the theorem uses. Pure, allocation-light, O(total expression size).
pub fn dependency_graph(
    definitions: &[Definition],
    premises: &[ProofExpr],
    goal: &ProofExpr,
) -> DependencyGraph {
    let defined: HashSet<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    let def_uses = def_edges(definitions);
    let mut theorem_uses = Vec::new();
    for p in premises {
        collect_defined_predicates(p, &defined, &mut theorem_uses);
    }
    collect_defined_predicates(goal, &defined, &mut theorem_uses);
    DependencyGraph {
        def_uses,
        theorem_uses,
    }
}

/// Maximum δ-expansion depth for expand-for-search — a backstop against an
/// accidental cross-definition cycle (self-recursion is already rejected by
/// [`validate_definitions`]; cross-def cycle detection is a Rung 0b concern).
const MAX_EXPANSION_DEPTH: usize = 64;

/// δ-expand every defined predicate in `expr`: replace `def(args)` with its
/// definiens (parameters substituted by `args`), recursively, so the result is
/// stated purely in terms of undefined predicates the backward chainer can
/// search over. Non-defined predicates and all other nodes are walked
/// structurally. The identity when `defs` is empty.
fn expand_defs_in_expr(
    expr: &ProofExpr,
    defs: &HashMap<&str, &Definition>,
    depth: usize,
) -> ProofExpr {
    if depth >= MAX_EXPANSION_DEPTH {
        return expr.clone();
    }
    match expr {
        ProofExpr::Predicate { name, args, .. } => {
            if let Some(def) = defs.get(name.as_str()) {
                if def.params.len() == args.len() {
                    let substituted = substitute_params(&def.definiens, &def.params, args);
                    return expand_defs_in_expr(&substituted, defs, depth + 1);
                }
            }
            expr.clone()
        }
        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(expand_defs_in_expr(l, defs, depth)),
            Box::new(expand_defs_in_expr(r, defs, depth)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(expand_defs_in_expr(l, defs, depth)),
            Box::new(expand_defs_in_expr(r, defs, depth)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(expand_defs_in_expr(l, defs, depth)),
            Box::new(expand_defs_in_expr(r, defs, depth)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(expand_defs_in_expr(l, defs, depth)),
            Box::new(expand_defs_in_expr(r, defs, depth)),
        ),
        ProofExpr::Not(p) => ProofExpr::Not(Box::new(expand_defs_in_expr(p, defs, depth))),
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(expand_defs_in_expr(body, defs, depth)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(expand_defs_in_expr(body, defs, depth)),
        },
        other => other.clone(),
    }
}

/// Substitute a definition's parameters with the actual arguments throughout its
/// definiens, when δ-expanding a definition occurrence.
fn substitute_params(expr: &ProofExpr, params: &[String], args: &[ProofTerm]) -> ProofExpr {
    match expr {
        ProofExpr::Predicate { name, args: pargs, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: pargs.iter().map(|t| subst_term(t, params, args)).collect(),
            world: world.clone(),
        },
        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(substitute_params(l, params, args)),
            Box::new(substitute_params(r, params, args)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(substitute_params(l, params, args)),
            Box::new(substitute_params(r, params, args)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(substitute_params(l, params, args)),
            Box::new(substitute_params(r, params, args)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(substitute_params(l, params, args)),
            Box::new(substitute_params(r, params, args)),
        ),
        ProofExpr::Not(p) => ProofExpr::Not(Box::new(substitute_params(p, params, args))),
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(substitute_params(body, params, args)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(substitute_params(body, params, args)),
        },
        ProofExpr::Identity(l, r) => {
            ProofExpr::Identity(subst_term(l, params, args), subst_term(r, params, args))
        }
        other => other.clone(),
    }
}

/// Rewrite biconditionals into their two implications: `P ↔ Q` becomes
/// `(P → Q) ∧ (Q → P)`. Applied to premises so biconditional elimination falls
/// out of the existing implication + conjunction machinery (the kernel has no
/// `Iff` type). Recurses through the propositional structure.
fn expand_iff(expr: &ProofExpr) -> ProofExpr {
    match expr {
        ProofExpr::Iff(p, q) => {
            let p = expand_iff(p);
            let q = expand_iff(q);
            ProofExpr::And(
                Box::new(ProofExpr::Implies(Box::new(p.clone()), Box::new(q.clone()))),
                Box::new(ProofExpr::Implies(Box::new(q), Box::new(p))),
            )
        }
        ProofExpr::And(l, r) => {
            ProofExpr::And(Box::new(expand_iff(l)), Box::new(expand_iff(r)))
        }
        ProofExpr::Or(l, r) => ProofExpr::Or(Box::new(expand_iff(l)), Box::new(expand_iff(r))),
        ProofExpr::Implies(l, r) => {
            ProofExpr::Implies(Box::new(expand_iff(l)), Box::new(expand_iff(r)))
        }
        ProofExpr::Not(p) => ProofExpr::Not(Box::new(expand_iff(p))),
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(expand_iff(body)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(expand_iff(body)),
        },
        other => other.clone(),
    }
}

/// Substitute a parameter with its argument inside a single term. Parameters lower
/// to `Constant`s and quantifier-bound variables are `Variable`/`BoundVarRef`, so
/// substituting ONLY `Constant`s can never capture a bound variable — even when a
/// definiens' quantifier reuses a parameter's name (`∀x … admire(x, x_param)`).
fn subst_term(term: &ProofTerm, params: &[String], args: &[ProofTerm]) -> ProofTerm {
    match term {
        ProofTerm::Constant(n) => match params.iter().position(|p| p == n) {
            Some(i) => args[i].clone(),
            None => term.clone(),
        },
        ProofTerm::Function(name, fargs) => ProofTerm::Function(
            name.clone(),
            fargs.iter().map(|t| subst_term(t, params, args)).collect(),
        ),
        ProofTerm::Group(ts) => {
            ProofTerm::Group(ts.iter().map(|t| subst_term(t, params, args)).collect())
        }
        // Bound variables (∀/∃) are never parameters — leave them untouched.
        ProofTerm::Variable(_) | ProofTerm::BoundVarRef(_) => term.clone(),
    }
}
