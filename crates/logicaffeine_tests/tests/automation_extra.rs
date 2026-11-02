//! AUTOMATION AND THE CERTIFIER — the tests the conformance corpus does not contain.
//!
//! Three of this bunch's cases (T08-001, T08-024, T11-021) are decided by corpus tests that
//! live elsewhere. This file holds the assertions those cases DO NOT make, and which decide
//! whether the fixes behind them are a prover or a liar:
//!
//! * **Every goal newly CLOSED gets a false sibling that must still be REFUSED.** Teaching
//!   the certified contradiction finder modus ponens is worth nothing if it also closes a
//!   consistent set; reaching excluded middle through `dne` is worth nothing if `p ∧ ¬p`
//!   comes along with it. Every positive here is paired with the neighbouring negative.
//! * **`verified == true` is the only predicate.** `Ok(_)` means a derivation was BUILT.
//!   The kernel's word is what a proof is, so every assertion below reads `verified`, and
//!   every negative reads `!verified` — never `is_err()`.
//! * **A dead `InferenceRule` must fail deliberately, not fall off the end of a match.**
//!   T11-021 has no reproduction anywhere in the repo; this file is its first one.
//!
//! The two `omega` tests at the bottom pin a diagnostic rather than a capability: a decision
//! procedure may decline, but it may never call an unsatisfiable system satisfiable. They
//! were written RED against `omega_solve::omega_close`'s early return and went green while
//! this file was being written, when that routing was fixed. They stay, because the wrong
//! MESSAGE was the part nothing else was watching.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, Context, Term};
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::verify::{check_derivation, prove_certify_check};
use logicaffeine_proof::{
    DerivationTree, InductionArg, InductionCase, InferenceRule, ProofExpr, ProofTerm,
};
use std::collections::BTreeSet;

// ============================================================================================
// HELPERS — per-file, as house style requires. Nothing here is shared with another test.
// ============================================================================================

fn c(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn app(name: &str, args: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(name.to_string(), args)
}
fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn not(p: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(p))
}
/// The engine's absurdity.
fn falsum() -> ProofExpr {
    ProofExpr::Atom("⊥".to_string())
}
/// `a ≤ b` in the system's canonical encoding: `Identity(le(a, b), true)`.
fn le(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    eq(app("le", vec![a, b]), c("true"))
}
/// `a < b` in the same encoding.
fn lt(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    eq(app("lt", vec![a, b]), c("true"))
}
/// The corpus's propositional variables `(p q r : Prop)`. Unary predicates over one fixed
/// entity rather than `ProofExpr::Atom`s, because the certifier resolves an `Atom` as a
/// GLOBAL NAME — an unregistered bare atom cannot be a premise at all.
fn p_() -> ProofExpr {
    pred("p", vec![c("X")])
}
fn q_() -> ProofExpr {
    pred("q", vec![c("X")])
}

/// Did the KERNEL certify `premises ⊢ goal`? `Err` carries the reason, so a failed
/// assertion says where the chain broke.
fn proves(premises: &[ProofExpr], goal: &ProofExpr) -> Result<(), String> {
    let r = prove_certify_check(premises, goal);
    if r.verified {
        Ok(())
    } else {
        Err(r
            .verification_error
            .unwrap_or_else(|| "no error recorded".to_string()))
    }
}

/// True iff `tree` (or some node below it) was justified by `rule`'s discriminant.
fn uses_rule(tree: &DerivationTree, want: &InferenceRule) -> bool {
    if std::mem::discriminant(&tree.rule) == std::mem::discriminant(want) {
        return true;
    }
    tree.premises.iter().any(|p| uses_rule(p, want))
}

fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

// ============================================================================================
// A · A DERIVED ⊥ IS A PROOF — and a consistent set still has none.  (T08-001's second half)
// ============================================================================================

/// The gap T08-001 dies on one step after `intro`: with `hp : p` and `h : p → False` in
/// scope, `auto` on the goal `⊥` refuses to do the modus ponens.
///
/// It is not that the step cannot be built — `cert_saturate` builds it, records `⊥` among
/// the known facts, and then `cert_derive_falsum` throws it away, because every closer below
/// the saturation looks for a `P`/`¬P` PAIR and nothing looks for `⊥` itself.
///
/// The paired negative is the whole point: a contradiction finder that returns a proof of
/// `False` for a set that HAS a model is not an improvement, it is the end of the system.
/// `p → False` with `q` in scope is consistent (take `p` false, `q` true), and so is
/// `p → False` alone.
#[test]
fn falsum_follows_by_modus_ponens_and_a_consistent_set_still_yields_none() {
    let premises = vec![implies(p_(), falsum()), p_()];
    assert!(
        proves(&premises, &falsum()).is_ok(),
        "`p → False` and `p` prove `False` by ONE modus ponens; the certified contradiction \
         finder derives it and must not discard it: {:?}",
        proves(&premises, &falsum())
    );

    // …and the proof is the modus ponens, not a relabelled premise.
    let r = prove_certify_check(&premises, &falsum());
    let tree = r.derivation.as_ref().expect("a derivation was found");
    assert!(
        uses_rule(tree, &InferenceRule::ModusPonens),
        "the ⊥-derivation must RECORD the modus ponens it did: {:?}",
        tree.rule
    );

    // THE NEGATIVES. Each of these sets has a model, so `False` must not follow.
    for (name, premises) in [
        (
            "antecedent never established",
            vec![implies(p_(), falsum()), q_()],
        ),
        ("the rule alone", vec![implies(p_(), falsum())]),
        ("the fact alone", vec![p_()]),
        (
            "unrelated rule and fact",
            vec![implies(q_(), falsum()), p_()],
        ),
    ] {
        assert!(
            proves(&premises, &falsum()).is_err(),
            "{name}: this premise set has a model — proving `False` from it would make every \
             proposition provable"
        );
    }
}

/// The same step through the REAL door a negation goal takes: `¬p` is `p → False`, so
/// `p → False ⊢ ¬p` is implication introduction over exactly the modus ponens above. The
/// negative is its converse, which does not hold.
#[test]
fn a_negation_goal_is_discharged_by_the_implication_it_unfolds_to() {
    assert!(
        proves(&[implies(p_(), falsum())], &not(p_())).is_ok(),
        "`p → False ⊢ ¬p`: the certifier lowers both to `Π(_:p). False`, so this is the \
         identity and must certify: {:?}",
        proves(&[implies(p_(), falsum())], &not(p_()))
    );
    assert!(
        proves(&[implies(q_(), falsum())], &not(p_())).is_err(),
        "`q → False` says nothing about `p` — a negation goal must not be discharged by \
         SOME implication into `False`, only by the one whose antecedent is the negatum"
    );
}

// ============================================================================================
// B · EXCLUDED MIDDLE — and the four neighbours that must stay unprovable.  (T08-024)
// ============================================================================================

/// `p ∨ ¬p` is the tautology none of our automation reaches, even though `dne` is the
/// trusted base's classical axiom and `ClassicalReductio` is a live strategy. The reason is
/// a missing De Morgan step: `ClassicalReductio` assumes `¬(p ∨ ¬p)` and hands it to the
/// certified contradiction finder, whose saturation refutes each conjunct of a negated
/// CONJUNCTION and has no dual for a negated DISJUNCTION. With the dual, `¬(p ∨ ¬p)` yields
/// `¬p` and `¬¬p`, and the contradiction is read straight off them.
///
/// The axiom footprint is part of the theorem, so the derivation must RECORD that it went
/// classical — a constructive-looking certificate for a classically-proved theorem is a lie
/// about what the theorem rests on.
#[test]
fn excluded_middle_certifies_and_records_that_it_went_classical() {
    let goal = or(p_(), not(p_()));
    assert!(
        proves(&[], &goal).is_ok(),
        "`⊢ p ∨ ¬p` must certify — `dne` is in the trusted base and `ClassicalReductio` is a \
         live strategy, so refusing it is an automation gap, not a foundational one: {:?}",
        proves(&[], &goal)
    );

    let r = prove_certify_check(&[], &goal);
    let tree = r.derivation.as_ref().expect("a derivation was found");
    assert!(
        uses_rule(tree, &InferenceRule::ClassicalReductio)
            || uses_rule(tree, &InferenceRule::DoubleNegation),
        "the derivation must record the classical step it took: {:?}",
        tree.rule
    );
}

/// The false siblings. Each is one connective away from excluded middle, and each is
/// unprovable — three of them in EVERY model, and `p ∨ q` in some. A search that reaches
/// `p ∨ ¬p` by widening until something closes would close these too.
#[test]
fn the_neighbours_of_excluded_middle_stay_unprovable() {
    for (name, goal) in [
        ("the contradiction", and(p_(), not(p_()))),
        ("the denial of excluded middle", not(or(p_(), not(p_())))),
        ("a disjunction of two unrelated atoms", or(p_(), q_())),
        ("the bare atom", p_()),
        ("the bare negation", not(p_())),
    ] {
        let r = prove_certify_check(&[], &goal);
        assert!(
            !r.verified,
            "{name} must NOT be provable from no premises — `verified == true` here would \
             mean the automation proves whatever it is pointed at"
        );
    }
}

/// `p ∨ ¬p` is classical; `¬¬p → p` is the same axiom wearing a different face, and
/// `p → ¬¬p` is the intuitionistic half that needs no axiom at all. Pinning all three keeps
/// the classical door exactly one door wide.
#[test]
fn the_classical_and_constructive_halves_of_double_negation_are_both_reachable() {
    assert!(
        proves(&[p_()], &not(not(p_()))).is_ok(),
        "`p ⊢ ¬¬p` is INTUITIONISTIC — it must certify without reaching for `dne`: {:?}",
        proves(&[p_()], &not(not(p_())))
    );
    assert!(
        proves(&[not(not(p_()))], &p_()).is_ok(),
        "`¬¬p ⊢ p` is the classical direction and `dne` is in the trusted base: {:?}",
        proves(&[not(not(p_()))], &p_())
    );
    assert!(
        proves(&[not(p_())], &p_()).is_err(),
        "`¬p ⊢ p` is not double negation — one `¬` too few must not be collapsed by the same \
         rule, which is exactly the `cert_neg` `¬¬P ↦ P` trap"
    );
}

// ============================================================================================
// C · DE MORGAN ON A NEGATED DISJUNCTION — refutes each disjunct and claims NO MORE.
// ============================================================================================

/// The new saturation step, and the line it must not cross. From `¬(A ∨ B)` the certified
/// step is `¬A` and `¬B` — assume the disjunct, `∨`-introduce, contradict. It is NOT `A` or
/// `B`, and it is not `¬(A ∨ B) ⊢ ⊥`: the premise set is perfectly satisfiable on its own.
///
/// The `¬¬p` case is the one that decides whether the implementation is honest. Writing the
/// conclusion through `cert_neg` (which collapses `¬¬P` to `P`) would record `p` as PROVED
/// when only `¬¬p` is — a classical step the intuitionistic reductio does not take, and a
/// fact whose recorded proposition is not the kernel type of its own proof.
#[test]
fn a_negated_disjunction_refutes_each_disjunct_and_nothing_more() {
    let neg_or = not(or(p_(), q_()));

    // The positives: each disjunct is refuted, so pairing it with the disjunct closes ⊥.
    assert!(
        proves(&[neg_or.clone(), p_()], &falsum()).is_ok(),
        "`¬(p ∨ q)` with `p` is contradictory: {:?}",
        proves(&[neg_or.clone(), p_()], &falsum())
    );
    assert!(
        proves(&[neg_or.clone(), q_()], &falsum()).is_ok(),
        "`¬(p ∨ q)` with `q` is contradictory: {:?}",
        proves(&[neg_or.clone(), q_()], &falsum())
    );

    // The double-negation shape, which is where excluded middle actually comes from.
    let neg_em = not(or(p_(), not(p_())));
    assert!(
        proves(&[neg_em], &falsum()).is_ok(),
        "`¬(p ∨ ¬p)` is contradictory in ONE De Morgan step plus one `P`/`¬P` close — this \
         is the whole of excluded middle: {:?}",
        proves(&[not(or(p_(), not(p_())))], &falsum())
    );

    // THE NEGATIVES. `¬(p ∨ q)` entails `¬p` and `¬q`; it entails neither disjunct, and it
    // is not itself absurd.
    for (name, goal) in [
        ("the left disjunct", p_()),
        ("the right disjunct", q_()),
        ("falsum from a satisfiable set", falsum()),
        ("the disjunction it denies", or(p_(), q_())),
    ] {
        assert!(
            proves(&[neg_or.clone()], &goal).is_err(),
            "{name}: `¬(p ∨ q)` does not entail it. De Morgan gives the NEGATIONS of the \
             disjuncts and must claim nothing beyond them"
        );
    }

    // And the entailments it DOES have, so the step is not merely safe but useful.
    assert!(
        proves(&[neg_or.clone()], &not(p_())).is_ok(),
        "`¬(p ∨ q) ⊢ ¬p`: {:?}",
        proves(&[neg_or.clone()], &not(p_()))
    );
    assert!(
        proves(&[neg_or.clone()], &not(q_())).is_ok(),
        "`¬(p ∨ q) ⊢ ¬q`: {:?}",
        proves(&[neg_or], &not(q_()))
    );
}

/// **A latent mislabelling, pinned where it would surface.** Saturation's MODUS TOLLENS
/// branch records the refuted antecedent as `cert_neg(ante)`, and `cert_neg` collapses
/// `¬¬P` to `P`. When the antecedent is itself a negation the branch therefore files `p` as
/// PROVED while the reductio it built proves only `¬¬p` — a fact whose recorded proposition
/// is not the kernel type of its own proof. Writing the De Morgan step for excluded middle
/// meant walking past exactly this trap, which is how it was found.
///
/// It is not unsoundness and it is not, today, a lost proof: the branch only fires when
/// `¬cons` is already known, so the moment `ante` becomes derivable modus ponens yields
/// `cons` and the `cons`/`¬cons` pair closes the goal by a route that never touches the
/// mislabelled node. Every shape below is therefore GREEN — and every one of them would go
/// RED the day that masking stops, because each is a set whose only refutation runs through
/// a negated antecedent. The kernel would catch it (`¬p : p → False` applied to a term of
/// type `¬¬p` does not type-check), so the failure mode is a lost proof, not a false one.
///
/// `¬p ⊬ p`, pinned above, is the assertion that stops the eventual fix from being "collapse
/// the negation and hope".
#[test]
fn modus_tollens_on_a_negated_antecedent_proves_the_double_negation_not_the_atom() {
    let rule = implies(not(p_()), q_());
    let premises = vec![rule.clone(), not(q_()), not(p_())];
    assert!(
        proves(&premises, &falsum()).is_ok(),
        "`¬p → q`, `¬q` refute `¬p`; with `¬p` also assumed the set is contradictory and the \
         refutation is `¬¬p`, never `p`: {:?}",
        proves(&premises, &falsum())
    );

    // The negative: drop the assumed `¬p` and the set has a model (`p` true, `q` false).
    assert!(
        proves(&[rule.clone(), not(q_())], &falsum()).is_err(),
        "`¬p → q` with `¬q` is satisfiable — the refutation of `¬p` is a FACT, not a \
         contradiction"
    );
    // And what it entails is the double negation, not the atom. Both are separately pinned
    // because a fix that simply collapsed `¬¬p` would satisfy the first and break the second
    // only on a set where the collapse is not licensed.
    assert!(
        proves(&[rule.clone(), not(q_())], &not(not(p_()))).is_ok(),
        "`¬p → q`, `¬q ⊢ ¬¬p` is the intuitionistic modus tollens: {:?}",
        proves(&[rule.clone(), not(q_())], &not(not(p_())))
    );

    // THE SET THAT ONLY SATURATION CAN CLOSE. Two rules, each refuted at its consequent:
    // `¬q` refutes `¬p` (giving `¬¬p`) and `¬r` refutes `p` (giving `¬p`) — and `¬¬p` with
    // `¬p` is the contradiction. Neither the goal nor any single hypothesis is a negation
    // the direct `try_modus_tollens` can drive, so the whole refutation happens inside
    // `cert_saturate`, which is where the mislabelling lives. Filing `p` instead of `¬¬p`
    // makes the closing `Contradiction` apply `¬p : p → False` to a term of type `¬¬p`, and
    // the kernel refuses the certificate.
    let two_rules = vec![
        rule,
        not(q_()),
        implies(p_(), pred("r", vec![c("X")])),
        not(pred("r", vec![c("X")])),
    ];
    assert!(
        proves(&two_rules, &falsum()).is_ok(),
        "`¬p → q`, `¬q`, `p → r`, `¬r` is contradictory through `¬¬p` and `¬p`, and the \
         refutation of a NEGATED antecedent is the double negation — recording it as the \
         bare atom builds a certificate the kernel cannot check: {:?}",
        proves(&two_rules, &falsum())
    );

    // The negative for that set: drop one refutation and the remaining three have a model
    // (`p` false, `q` true, `r` false).
    assert!(
        proves(
            &[
                implies(not(p_()), q_()),
                implies(p_(), pred("r", vec![c("X")])),
                not(pred("r", vec![c("X")])),
            ],
            &falsum()
        )
        .is_err(),
        "without `¬q` the set is satisfiable and must stay so"
    );
}

// ============================================================================================
// D · T11-021 · THE SEVEN DEAD `InferenceRule` VARIANTS — the corpus case's first repro.
// ============================================================================================

/// The seven variants of [`InferenceRule`] that `certify` has never had an arm for. The
/// list is PINNED here rather than derived, because deriving it from the same source that
/// decides it would make the test agree with any drift.
const DEAD_RULES: [&str; 7] = [
    "ModalAccess",
    "ModalGeneralization",
    "TemporalTransitivity",
    "TemporalInduction",
    "TemporalUnfolding",
    "EventualityProgress",
    "UntilInduction",
];

/// Every variant of `InferenceRule`, one representative each. The count is pinned: the enum
/// is a TRUSTED surface, and a variant added without a decision about its certifier arm is
/// exactly the trap this case is about.
fn every_inference_rule() -> Vec<InferenceRule> {
    vec![
        InferenceRule::PremiseMatch,
        InferenceRule::ModusPonens,
        InferenceRule::ModusTollens,
        InferenceRule::ConjunctionIntro,
        InferenceRule::ConjunctionElim,
        InferenceRule::DisjunctionIntro,
        InferenceRule::DisjunctionElim,
        InferenceRule::ExFalso,
        InferenceRule::ImpliesIntro,
        InferenceRule::BicondIntro,
        InferenceRule::DoubleNegation,
        InferenceRule::ClassicalReductio,
        InferenceRule::UniversalInst("t".to_string()),
        InferenceRule::UniversalInstTerm(c("t")),
        InferenceRule::UniversalIntro {
            variable: "x".to_string(),
            var_type: "Entity".to_string(),
        },
        InferenceRule::ExistentialIntro {
            witness: "w".to_string(),
            witness_type: "Entity".to_string(),
        },
        InferenceRule::ModalAccess,
        InferenceRule::ModalGeneralization,
        InferenceRule::TemporalTransitivity,
        InferenceRule::TemporalInduction,
        InferenceRule::TemporalUnfolding,
        InferenceRule::EventualityProgress,
        InferenceRule::UntilInduction,
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        InferenceRule::InductionScheme {
            variable: "t".to_string(),
            ind_type: "Nat".to_string(),
            cases: vec![InductionCase {
                constructor: "Zero".to_string(),
                args: vec![InductionArg {
                    name: "k".to_string(),
                    recursive: true,
                }],
            }],
        },
        InferenceRule::LeTrans,
        InferenceRule::LeRefl,
        InferenceRule::LeAddMono,
        InferenceRule::LinFalse,
        InferenceRule::LeMulNonneg,
        InferenceRule::LeSub,
        InferenceRule::LtSuccLe,
        InferenceRule::LtAdd1Le,
        InferenceRule::Rewrite {
            from: c("a"),
            to: c("b"),
        },
        InferenceRule::EqualitySymmetry,
        InferenceRule::EqualityTransitivity,
        InferenceRule::Reflexivity,
        InferenceRule::ArithDecision,
        InferenceRule::NativeDecide,
        InferenceRule::Axiom,
        InferenceRule::OracleVerification("z3".to_string()),
        InferenceRule::ReductioAdAbsurdum,
        InferenceRule::Contradiction,
        InferenceRule::ExistentialElim {
            witness: "w".to_string(),
        },
        InferenceRule::CaseAnalysis {
            case_formula: Box::new(p_()),
        },
        InferenceRule::DisjunctionCases,
    ]
}

/// **T11-021 · MUST.** `TemporalTransitivity` is the most plausible of the seven: a proof
/// that `t₁ < t₂` and `t₂ < t₃` give `t₁ < t₃` is ordinary mathematics, the variant EXISTS
/// with a doc comment stating exactly that logic, and picking it by name builds a
/// `DerivationTree` that is well-formed, well-typed Rust — and is not a proof of anything.
///
/// The failure must therefore be DELIBERATE and LEGIBLE: an error that names the variant,
/// says the certifier has no arm for it, and points at the live translation. Falling through
/// a catch-all into "Certification not implemented for TemporalTransitivity" reads like a
/// kernel limitation and invites the three wrong responses — retry, widen the search, or
/// report the user's input as unsupported. The tree was never certifiable.
#[test]
fn a_dead_inference_rule_is_refused_deliberately_and_names_itself() {
    let t1 = c("t1");
    let t2 = c("t2");
    let t3 = c("t3");
    let tree = DerivationTree::new(
        lt(t1.clone(), t3.clone()),
        InferenceRule::TemporalTransitivity,
        vec![
            DerivationTree::leaf(lt(t1.clone(), t2.clone()), InferenceRule::PremiseMatch),
            DerivationTree::leaf(lt(t2, t3.clone()), InferenceRule::PremiseMatch),
        ],
    );

    // Through the certifier directly.
    let ctx = std_ctx();
    let err = certify(&tree, &CertificationContext::new(&ctx))
        .expect_err("a dead rule can never produce a kernel term");
    let msg = format!("{err}");
    assert!(
        msg.contains("TemporalTransitivity"),
        "the refusal must NAME the variant that caused it: {msg}"
    );
    assert!(
        msg.contains("E_DEAD_INFERENCE_RULE"),
        "the refusal must be the dead-rule diagnostic, not a catch-all — a translator has to \
         be able to tell 'this tree was never certifiable' from 'the kernel rejected this \
         proof': {msg}"
    );

    // …and through the door an external producer actually uses.
    let premises = vec![lt(t1.clone(), c("t2")), lt(c("t2"), t3.clone())];
    let goal = lt(t1, t3);
    let r = check_derivation(&premises, &goal, tree);
    assert!(
        !r.verified,
        "a tree carrying a dead rule is never a proof, whatever else it looks like"
    );
    let verr = r
        .verification_error
        .expect("an unverified proof states why");
    assert!(
        verr.contains("E_DEAD_INFERENCE_RULE") && verr.contains("TemporalTransitivity"),
        "`VerifiedProof.verification_error` is what a caller reads; it must carry the same \
         deliberate diagnostic: {verr}"
    );
}

/// The generalisation, which is the lesson the case actually teaches: not "avoid seven
/// names", but that a producer must be able to ask, per rule, whether `certify` has an arm —
/// and that the answer cannot drift away from the certifier.
///
/// `uncertifiable_reason` is that question. It is answered by an EXHAUSTIVE match, as is
/// `certify`'s dispatch, so a forty-seventh variant is a compile error in both places rather
/// than a tree that fails at the user.
#[test]
fn every_inference_rule_declares_whether_the_certifier_can_build_it() {
    let all = every_inference_rule();
    assert_eq!(
        all.len(),
        46,
        "`InferenceRule` has 46 variants and every one needs a representative here; a new \
         variant must be classified, not merely added"
    );

    let mut dead: Vec<String> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for rule in &all {
        let name = format!("{rule:?}");
        let name = name
            .split(|ch: char| !ch.is_alphanumeric())
            .next()
            .unwrap_or("")
            .to_string();
        match rule.uncertifiable_reason() {
            None => {}
            Some(why) if why.starts_with("no certifier arm") => dead.push(name),
            Some(_) => refused.push(name),
        }
    }

    assert_eq!(
        dead, DEAD_RULES,
        "the dead set must be exactly the seven modal/temporal variants, in enum order — a \
         rule that left the set was implemented (raise the win) and one that joined it is a \
         capability that silently died"
    );
    assert_eq!(
        refused,
        vec!["OracleVerification".to_string()],
        "the only rule refused BY DESIGN is the SMT oracle, which attests satisfiability and \
         produces no checkable term. Anything else here is an unimplemented rule wearing a \
         principled excuse"
    );

    for rule in &all {
        let live = rule.is_certifiable();
        assert_eq!(
            live,
            rule.uncertifiable_reason().is_none(),
            "`is_certifiable` must be exactly the negation of `uncertifiable_reason`"
        );
    }
}

/// The case's decisive assertion is STRUCTURAL and happens BEFORE the tree is handed to
/// `check_derivation`: no emitted `DerivationTree` may carry any of the seven, checked on the
/// tree rather than inferred from the verification result. A producer needs that as one
/// question about a whole tree, not a walk it writes itself — the dead rule is usually deep
/// in a subtree, under a root whose own rule is perfectly live.
#[test]
fn a_tree_can_be_asked_for_its_dead_rules_before_anyone_tries_to_certify_it() {
    // A live root over a dead child: exactly the shape a rule-by-name translator emits.
    let deep = DerivationTree::new(
        pred("Q", vec![c("A")]),
        InferenceRule::ExFalso,
        vec![DerivationTree::new(
            falsum(),
            InferenceRule::Contradiction,
            vec![
                DerivationTree::leaf(p_(), InferenceRule::PremiseMatch),
                DerivationTree::leaf(not(p_()), InferenceRule::TemporalUnfolding),
            ],
        )],
    );
    let dead = deep.uncertifiable_rules();
    assert_eq!(
        dead.len(),
        1,
        "one dead rule, found wherever it sits in the tree — the root's own rule being live \
         proves nothing about the subtree: {dead:?}"
    );
    assert!(
        dead[0].0.contains("TemporalUnfolding") && dead[0].1.starts_with("no certifier arm"),
        "the report must name the variant AND why it cannot be built: {dead:?}"
    );

    // A tree of live rules reports nothing, so the check is usable as a gate rather than a
    // warning nobody can clear.
    let live = DerivationTree::new(
        falsum(),
        InferenceRule::Contradiction,
        vec![
            DerivationTree::leaf(p_(), InferenceRule::PremiseMatch),
            DerivationTree::leaf(not(p_()), InferenceRule::PremiseMatch),
        ],
    );
    assert!(
        live.uncertifiable_rules().is_empty(),
        "a tree built from live rules must clear the gate: {:?}",
        live.uncertifiable_rules()
    );

    // The SMT oracle is reported too — it is equally uncertifiable, and a producer that must
    // hand back a kernel certificate needs to know before it tries, not after.
    let oracular = DerivationTree::leaf(p_(), InferenceRule::OracleVerification("sat".to_string()));
    let reported = oracular.uncertifiable_rules();
    assert_eq!(
        reported.len(),
        1,
        "the oracle leaf is uncertifiable: {reported:?}"
    );
    assert!(
        reported[0].1.starts_with("refused by design"),
        "…but for a DIFFERENT reason than a dead rule, and the report must keep them apart: \
         {reported:?}"
    );
}

/// The refusal has to be SPECIFIC, or it teaches nothing. A LIVE rule handed a malformed
/// tree must fail with its own diagnostic — not with the dead-rule one — otherwise
/// "E_DEAD_INFERENCE_RULE" degrades into a second catch-all and a producer is back to
/// guessing.
#[test]
fn a_live_rule_misused_reports_its_own_error_not_the_dead_rule_one() {
    let ctx = std_ctx();
    let cert_ctx = CertificationContext::new(&ctx);

    let malformed = DerivationTree::new(
        p_(),
        InferenceRule::ModusPonens,
        vec![DerivationTree::leaf(p_(), InferenceRule::PremiseMatch)],
    );
    let msg = format!(
        "{}",
        certify(&malformed, &cert_ctx).expect_err("one premise is not a modus ponens")
    );
    assert!(
        !msg.contains("E_DEAD_INFERENCE_RULE"),
        "`ModusPonens` is implemented; a malformed one must say so in its own words: {msg}"
    );
    assert!(
        msg.contains("ModusPonens"),
        "and it must still name the rule that failed: {msg}"
    );

    // The control from the corpus case: the live translation of `TemporalTransitivity`.
    // `2 ≤ 5`, `5 ≤ 9` ⊢ `2 ≤ 9` by `le_trans` — same shape, live rule, real kernel term.
    let tree = DerivationTree::new(
        le(c("2"), c("9")),
        InferenceRule::LeTrans,
        vec![
            DerivationTree::leaf(le(c("2"), c("5")), InferenceRule::Axiom),
            DerivationTree::leaf(le(c("5"), c("9")), InferenceRule::Axiom),
        ],
    );
    let premises = vec![le(c("2"), c("5")), le(c("5"), c("9"))];
    let goal = le(c("2"), c("9"));
    let r = check_derivation(&premises, &goal, tree);
    assert!(
        r.verified,
        "the live translation of the dead rule must actually work, or the corpus case's \
         advice is empty: {:?}",
        r.verification_error
    );
}

// ============================================================================================
// E · THE AXIOM FOOTPRINT — what a certified proof actually rests on.
// ============================================================================================

/// Lean's classical base in LOGOS spellings, in the footprint's sorted order. Lean 4.14's
/// `#print axioms` for a proof by contradiction (`Classical.byContradiction`, `Classical.em`)
/// is `[propext, Classical.choice, Quot.sound]`.
const LEAN_CLASSICAL_BASE: [&str; 3] = ["Classical_choice", "Quot_sound", "propext"];

/// Lean's quotient constants in LOGOS spellings: `ConstantInfo.quotInfo`, whose `QuotKind` is
/// exactly type / ctor / lift / ind (`Lean/Declaration.lean:380-384`). `collect` records NOTHING
/// for them (`Lean/Util/CollectAxioms.lean:32`). `Quot.sound` is not among them: it is an axiom.
const LEAN_QUOT_FORMERS: [&str; 4] = ["Quot", "Quot_mk", "Quot_lift", "Quot_ind"];

/// `double_check(ctx, sorryAx (2 = 3))` returns `Agreed`: both kernels are asked the same
/// question against the same context, so two kernels give ZERO protection against an added
/// axiom. The only thing that catches one is rebuilding the trusted base and asking what the
/// proof term rests on — Lean's `#print axioms`.
///
/// This is that report at the proof crate's door. It separates the TRUSTED base a proof used
/// from what the theorem itself introduced (its premise hypotheses and symbols), because the
/// second list is where a smuggled axiom lands. A classical step shows up as Lean's own
/// classical axioms: `dne` is Lean's `Classical.byContradiction`, a THEOREM, so the report walks
/// through it and names what it rests on — `Classical_choice`, `propext`, `Quot_sound` — exactly
/// the list `#print axioms` gives for a Lean proof by contradiction.
#[test]
fn a_certified_proof_discloses_the_trusted_axioms_it_rests_on() {
    // The classical proof: its footprint is Lean's classical base, exactly.
    let classical = prove_certify_check(&[], &or(p_(), not(p_())));
    assert!(
        classical.verified,
        "precondition: {:?}",
        classical.verification_error
    );
    let term = classical
        .proof_term
        .as_ref()
        .expect("verified proofs carry their term");
    assert!(
        cites(term, "dne"),
        "precondition: the classical step is in the TERM, as an application of `dne`: {term}"
    );
    let fp = classical.axiom_footprint();
    assert!(
        fp.trusted.iter().any(|a| a == "Classical_choice"),
        "a proof by classical reductio RESTS ON `Classical_choice`; a footprint that does not \
         say so makes a classical theorem indistinguishable from a constructive one: {:?}",
        fp.trusted
    );
    assert_eq!(
        fp.trusted, LEAN_CLASSICAL_BASE,
        "the footprint of a proof by contradiction is Lean's `#print axioms` for one — \
         `[propext, Classical.choice, Quot.sound]` — and nothing else. `dne` is a theorem: it is \
         walked through, never listed. `Quot`, `Quot_mk`, `Quot_lift` and `Quot_ind` are Lean's \
         quotient constants, which `#print axioms` never names; a footprint that lists them \
         states an assumption Lean's kernel does not make"
    );

    // The constructive one: same shape of report, no classical axiom in it.
    let constructive = prove_certify_check(&[p_()], &p_());
    assert!(
        constructive.verified,
        "precondition: {:?}",
        constructive.verification_error
    );
    let fp = constructive.axiom_footprint();
    assert!(
        !fp.trusted
            .iter()
            .any(|a| a == "dne" || LEAN_CLASSICAL_BASE.contains(&a.as_str())),
        "`p ⊢ p` uses no classical axiom; the taint must be exact, not session-wide: {:?}",
        fp.trusted
    );
    assert!(
        fp.trusted.is_empty(),
        "`p ⊢ p` is its premise and nothing more, so it rests on nothing in the prelude: {:?}",
        fp.trusted
    );
    assert!(
        !fp.introduced.is_empty(),
        "the proof cites its premise, which is a declaration this theorem INTRODUCED — the \
         report must list it separately from the trusted base, because that is the list a \
         smuggled axiom lands in: {:?}",
        fp.introduced
    );

    // The report is over the TERM, not the derivation: a proof that cites nothing beyond
    // its own premises names nothing from the prelude that it did not use.
    assert!(
        !fp.trusted
            .iter()
            .any(|a| a == "le_trans" || a == "lt_succ_le"),
        "the footprint must be what the term CITES, not what the prelude offers: {:?}",
        fp.trusted
    );
}

/// The footprint follows DEFINITIONS AND THEOREMS transitively, which is the half no
/// per-declaration warning can reach: a term that names a theorem rests on everything that
/// theorem rests on, and mentions none of it. Lean's rule, and this one: an axiom is a leaf;
/// definitions and theorems are walked through and never listed themselves.
///
/// Both directions are pinned. `le_trans` is a theorem proved from the integer constructors,
/// so a proof reaching it only through the `LeTrans` certificate rests on NOTHING. `dne` is a
/// theorem proved from Lean's classical axioms, so a proof naming it rests on `Classical_choice`,
/// which the term never mentions.
#[test]
fn the_axiom_footprint_is_transitive_through_the_terms_it_names() {
    let premises = vec![le(c("2"), c("5")), le(c("5"), c("9"))];
    let goal = le(c("2"), c("9"));
    let tree = DerivationTree::new(
        goal.clone(),
        InferenceRule::LeTrans,
        vec![
            DerivationTree::leaf(le(c("2"), c("5")), InferenceRule::Axiom),
            DerivationTree::leaf(le(c("5"), c("9")), InferenceRule::Axiom),
        ],
    );
    let r = check_derivation(&premises, &goal, tree);
    assert!(r.verified, "precondition: {:?}", r.verification_error);
    let term = r
        .proof_term
        .as_ref()
        .expect("verified proofs carry their term");
    assert!(
        cites(term, "le_trans"),
        "precondition: the `LeTrans` certificate names the `≤`-transitivity theorem: {term}"
    );
    assert!(
        r.kernel_ctx.get_definition_body("le_trans").is_some(),
        "precondition: `le_trans` is a checked theorem with a proof to walk through"
    );
    let fp = r.axiom_footprint();
    assert!(
        !fp.trusted
            .iter()
            .chain(&fp.introduced)
            .any(|a| a == "le_trans"),
        "`le_trans` is a THEOREM: the footprint walks through it and never lists it — Lean's \
         `#print axioms` names axioms, not the lemmas a proof cites: {fp:?}"
    );
    assert!(
        fp.trusted.is_empty(),
        "`le_trans` is proved from the integer constructors and rests on no axiom, so neither \
         does a proof that reaches the prelude only through it: {:?}",
        fp.trusted
    );
    assert!(
        !fp.trusted.iter().any(|a| a == "dne"),
        "and nothing else — a footprint that over-reports is as useless as one that \
         under-reports: {:?}",
        fp.trusted
    );

    // The other direction: an axiom reached ONLY through a theorem the term names.
    let classical = prove_certify_check(&[], &or(p_(), not(p_())));
    assert!(
        classical.verified,
        "precondition: {:?}",
        classical.verification_error
    );
    let term = classical
        .proof_term
        .as_ref()
        .expect("verified proofs carry their term");
    assert!(
        cites(term, "dne") && !cites(term, "Classical_choice"),
        "precondition: the term names `dne` and never `Classical_choice`: {term}"
    );
    assert!(
        classical
            .axiom_footprint()
            .trusted
            .iter()
            .any(|a| a == "Classical_choice"),
        "`Classical_choice` is reached only through the proof of `dne`, a theorem the term \
         names — a footprint that stops at the names the term mentions misses it: {:?}",
        classical.axiom_footprint().trusted
    );

    // An unverified proof has no footprint to report: there is no term.
    let bogus = DerivationTree::leaf(goal.clone(), InferenceRule::TemporalTransitivity);
    let bad = check_derivation(&premises, &goal, bogus);
    assert!(!bad.verified);
    let fp = bad.axiom_footprint();
    assert!(
        fp.trusted.is_empty() && fp.introduced.is_empty(),
        "there is no proof term, so there is nothing it rests on — reporting a footprint for \
         a failed proof would imply one exists: {fp:?}"
    );
}

/// The footprint is computed from the kernel term, so it is exactly as trustworthy as the
/// term — and the term is what `infer_type` accepted. Pinned against an INDEPENDENT reading of
/// that term: Lean's `collect`, transcribed below, walked from the constants the term itself
/// names. A footprint read off the DERIVATION (untrusted producer data) would name what the
/// derivation's rules suggest rather than what the term reaches, and the two would differ.
#[test]
fn the_footprint_is_read_off_the_kernel_term_the_kernel_accepted() {
    let r = prove_certify_check(&[], &or(p_(), not(p_())));
    let term = r
        .proof_term
        .as_ref()
        .expect("verified proofs carry their term");
    let inferred = infer_type(&r.kernel_ctx, term).expect("the term re-checks");
    assert!(
        matches!(inferred, Term::App(..)),
        "`p ∨ ¬p` is an application of `Or`: {inferred}"
    );
    let (lean_axioms, reached) = lean_collect(&r.kernel_ctx, term);
    let fp = r.axiom_footprint();
    let reported: BTreeSet<String> = fp.trusted.iter().chain(&fp.introduced).cloned().collect();
    for axiom in &reported {
        assert!(
            reached.contains(axiom),
            "`{axiom}` is reported as an axiom this proof rests on but the TERM never reaches \
             it — the footprint must be read off the term, not the derivation"
        );
    }
    assert_eq!(
        reported,
        lean_axioms,
        "the footprint must be exactly what Lean's `#print axioms` rule collects from the term: \
         reported beyond it {:?}; collected and not reported {:?}",
        reported.difference(&lean_axioms).collect::<Vec<_>>(),
        lean_axioms.difference(&reported).collect::<Vec<_>>()
    );
}

/// Does `term` name `name` anywhere?
fn cites(term: &Term, name: &str) -> bool {
    let mut names = Vec::new();
    used_constants(term, &mut names);
    names.iter().any(|n| n == name)
}

/// Every constant a term names — our reading of Lean's `Expr.getUsedConstants`
/// (`Lean/Util/FoldConsts.lean:50`). Exhaustive over `Term` with no `_` arm, so a new variant
/// is a compile error here rather than a field nobody walks.
fn used_constants(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Global(n) => out.push(n.clone()),
        Term::Const { name, .. } => out.push(name.clone()),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            used_constants(param_type, out);
            used_constants(body_type, out);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            used_constants(param_type, out);
            used_constants(body, out);
        }
        Term::App(f, a) => {
            used_constants(f, out);
            used_constants(a, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            used_constants(discriminant, out);
            used_constants(motive, out);
            for case in cases {
                used_constants(case, out);
            }
        }
        Term::Fix { body, .. } => used_constants(body, out),
        Term::MutualFix { defs, .. } => {
            for (_, body) in defs {
                used_constants(body, out);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            used_constants(ty, out);
            used_constants(value, out);
            used_constants(body, out);
        }
        Term::Annotated { term, .. } => used_constants(term, out),
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// `Lean/Util/CollectAxioms.lean:21-36`, transcribed onto a kernel `Context` and walked from
/// the constants `term` names: the axioms Lean's `collect` records, and every constant the walk
/// reached. Written here rather than borrowed from the kernel, so the footprint is held against
/// an independent reading of the term and not against a second call to itself. One deliberate
/// departure from Lean: a name that resolves to nothing is recorded (Lean's `:36` drops it),
/// because LOGOS reports trusted-base drift and so must its reference.
fn lean_collect(ctx: &Context, term: &Term) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut axioms = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut queue = vec![term.clone()];
    while let Some(t) = queue.pop() {
        let mut names = Vec::new();
        used_constants(&t, &mut names);
        for name in names {
            if !visited.insert(name.clone()) {
                continue;
            }
            if let Some((_, ty, body)) = ctx.get_universe_poly(&name) {
                // :29 defnInfo / :30 thmInfo — the type AND the value.
                queue.push(ty.clone());
                queue.push(body.clone());
            } else if let Some(body) = ctx.get_definition_body(&name) {
                queue.push(body.clone());
                if let Some(ty) = ctx.get_definition_type(&name) {
                    queue.push(ty.clone());
                }
            } else if LEAN_QUOT_FORMERS.contains(&name.as_str()) {
                // :32 quotInfo — nothing recorded, nothing walked.
            } else if ctx.is_inductive(&name) {
                // :35 inductInfo — the arity, then every constructor by name.
                if let Some(arity) = ctx.get_global(&name) {
                    queue.push(arity.clone());
                }
                for (ctor, _) in ctx.get_constructors(&name) {
                    queue.push(Term::Global(ctor.to_string()));
                }
            } else if ctx.is_constructor(&name) {
                // :33 ctorInfo — the constructor's type.
                if let Some(ty) = ctx.get_global(&name) {
                    queue.push(ty.clone());
                }
            } else {
                // :28 axiomInfo — the name, and never its type; and the unresolved name.
                axioms.insert(name);
            }
        }
    }
    (axioms, visited)
}

// ============================================================================================
// F · `omega`'s DIAGNOSTIC — a procedure may decline; it may not call UNSAT satisfiable.
// ============================================================================================

/// `5 ≤ x` and `x ≤ 3` are jointly unsatisfiable over ℤ AND over ℚ: the rational Farkas core
/// refutes them with the certificate `1·(5 − x) + 1·(x − 3) = 2 ≤ 0`. `BackwardChainer` calls
/// `cert_farkas` before it calls `omega_close`, so the engine door has always found it; that
/// asymmetry with the tactic door is what made the diagnostic below wrong.
#[test]
fn the_engine_refutes_a_pure_le_contradiction() {
    let premises = vec![le(c("5"), c("x")), le(c("x"), c("3"))];
    assert!(
        proves(&premises, &pred("Q", vec![c("A")])).is_ok(),
        "`5 ≤ x` and `x ≤ 3` have no model; every goal follows by ex falso: {:?}",
        proves(&premises, &pred("Q", vec![c("A")]))
    );
    // The control's control: drop one bound and the set has a model again.
    assert!(
        proves(&[le(c("5"), c("x"))], &pred("Q", vec![c("A")])).is_err(),
        "`5 ≤ x` alone is satisfiable and must entail nothing"
    );
}

/// The defect, stated as the assertion that decides it. `ProofState::omega` routes ONLY
/// through `omega_solve::omega_close`, which early-returns `None` unless at least one STRICT
/// `<` was tightened — on the reasoning that "the caller's own `cert_farkas` already tried
/// this exact set". That is true of `BackwardChainer`, which calls both. It is false of
/// `ProofState::omega`, which calls neither of the others.
///
/// So a PURE `≤` contradiction is declined, and the message says *"the integer hypotheses
/// are satisfiable (no refutation)"* about a system the test above just refuted. A wrong
/// answer is a gap; a wrong answer that asserts the opposite of the truth is a defect, and
/// it is the message a user would act on.
///
/// The routing has since been fixed, so this is green — but the assertion is the one worth
/// keeping, because it is about the MESSAGE and not the capability. `omega_refutes_a_contradictory_pure_le_system` in the corpus file already covers "it closes"; nothing covered
/// "and when it does not close, it does not lie".
#[test]
fn omega_must_not_report_an_unsatisfiable_system_as_satisfiable() {
    let premises = vec![le(c("5"), c("x")), le(c("x"), c("3"))];
    let mut st = ProofState::start(premises, pred("Q", vec![c("A")]));
    match st.omega() {
        Ok(_) => {
            let r = st.qed().expect("omega closed the goal");
            assert!(
                r.verified,
                "and the kernel must certify it: {:?}",
                r.verification_error
            );
        }
        Err(e) => panic!(
            "`omega` declined `5 ≤ x, x ≤ 3` — a system with no model over ℤ or ℚ — and said \
             so in words that are FALSE: {e:?}. A decision procedure may decline; it may not \
             call an unsatisfiable system satisfiable."
        ),
    }
}
