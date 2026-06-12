//! Standard translation: modal / temporal / counterfactual [`ProofExpr`]s
//! become world-indexed first-order formulas for the Z3 oracle.
//!
//! World-indexing happens at the **predicate level** — every non-arithmetic
//! predicate gains a trailing world argument (`P(x)` ↦ `P(x, w)`), and every
//! modal operator becomes quantification over a named accessibility relation.
//! Propositions are never passed as function arguments (the verify IR encodes
//! every uninterpreted function as `Int^n → Bool`, so a Bool-sorted argument
//! would be ill-sorted).
//!
//! Relations are named per (domain, flavor) — `Acc_Alethic_Root`,
//! `Acc_Deontic_Bouletic`, … — so evidential evidence-worlds never leak into
//! deontic ideality. Counterfactuals get a per-antecedent `Closest_<i>`
//! relation (two counterfactuals share a relation iff their antecedents are
//! structurally identical), which is exactly what validates consequent
//! weakening while refuting antecedent strengthening.
//!
//! Frame axioms are emitted **lazily** — only for relations the current
//! problem actually uses — so non-modal proofs see no axioms at all:
//!
//! | Relation               | Axiom        | Effect                          |
//! |------------------------|--------------|---------------------------------|
//! | `Acc_Alethic_Root`     | T (reflexive)| `□P ⊢ P`                        |
//! | `Acc_Alethic_Epistemic`| D (serial)   | `must(P) ⊬ P`                   |
//! | `Acc_Alethic_Evidential`| D (serial)  | `Seem(P) ⊬ P` (no T — that's it)|
//! | `Acc_Deontic_*`        | D (serial)   | `O(P) ⊢ ¬O(¬P)`, `O(P) ⊬ P`     |
//! | `Closest_<i>` (ant. A) | success: closest worlds satisfy A — **no weak centering** (v1), so `(P □→ Q)` and `(P → Q)` are independent in both directions |
//!
//! Verdicts built on this translation are Z3-side only and are **never**
//! kernel-certified.

use std::collections::BTreeSet;

use crate::oracle::proof_term_to_verify_expr;
use crate::ProofExpr;

use logicaffeine_verify::ir::{VerifyExpr, VerifyType};

/// Does this expression require the standard translation (any modal,
/// counterfactual, or temporal construct, at any depth)?
pub(crate) fn contains_modal_constructs(expr: &ProofExpr) -> bool {
    match expr {
        ProofExpr::Modal { .. }
        | ProofExpr::Counterfactual { .. }
        | ProofExpr::Temporal { .. }
        | ProofExpr::TemporalBinary { .. } => true,

        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => contains_modal_constructs(l) || contains_modal_constructs(r),

        ProofExpr::Not(inner) => contains_modal_constructs(inner),

        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            contains_modal_constructs(body)
        }

        _ => false,
    }
}

/// The world-indexing translator. One instance per verification problem; it
/// records which accessibility relations the problem touched so
/// [`WorldTranslation::finalize`] can emit exactly the needed frame axioms.
pub(crate) struct WorldTranslation {
    /// (domain, flavor) pairs whose `Acc_<domain>_<flavor>` relation was used.
    used_relations: BTreeSet<(String, String)>,
    /// Temporal relations used (`Time_<operator>`).
    used_temporal: BTreeSet<String>,
    /// Counterfactual antecedents, deduplicated by structural identity
    /// (display form); index `i` owns relation `Closest_<i>`.
    cf_antecedents: Vec<(String, ProofExpr)>,
    /// Fresh world-variable counter.
    world_counter: u32,
}

impl WorldTranslation {
    pub(crate) fn new() -> Self {
        WorldTranslation {
            used_relations: BTreeSet::new(),
            used_temporal: BTreeSet::new(),
            cf_antecedents: Vec::new(),
            world_counter: 0,
        }
    }

    fn fresh_world(&mut self) -> String {
        self.world_counter += 1;
        format!("w{}", self.world_counter)
    }

    fn cf_relation(&mut self, antecedent: &ProofExpr) -> String {
        let key = format!("{}", antecedent);
        let idx = match self
            .cf_antecedents
            .iter()
            .position(|(k, _)| *k == key)
        {
            Some(i) => i,
            None => {
                self.cf_antecedents.push((key, antecedent.clone()));
                self.cf_antecedents.len() - 1
            }
        };
        format!("Closest_{}", idx)
    }

    /// Translate `expr` as evaluated at world `w`. `None` means the
    /// construct is outside the translatable fragment — the caller must
    /// answer `Unknown`, never drop the formula.
    pub(crate) fn translate(&mut self, expr: &ProofExpr, w: &str) -> Option<VerifyExpr> {
        match expr {
            // A propositional atom is a unary world predicate.
            ProofExpr::Atom(name) => {
                Some(VerifyExpr::apply(name, vec![VerifyExpr::var(w)]))
            }

            ProofExpr::Predicate { name, args, .. } => {
                // Arithmetic comparisons are world-invariant.
                if args.len() == 2 {
                    let builtin = matches!(
                        name.as_str(),
                        "Gt" | "Lt" | "Gte" | "Lte" | "Eq" | "Neq"
                    );
                    if builtin {
                        let left = proof_term_to_verify_expr(&args[0])?;
                        let right = proof_term_to_verify_expr(&args[1])?;
                        return Some(match name.as_str() {
                            "Gt" => VerifyExpr::gt(left, right),
                            "Lt" => VerifyExpr::lt(left, right),
                            "Gte" => VerifyExpr::gte(left, right),
                            "Lte" => VerifyExpr::lte(left, right),
                            "Eq" => VerifyExpr::eq(left, right),
                            _ => VerifyExpr::neq(left, right),
                        });
                    }
                }
                let mut verify_args = Vec::with_capacity(args.len() + 1);
                for arg in args {
                    verify_args.push(proof_term_to_verify_expr(arg)?);
                }
                verify_args.push(VerifyExpr::var(w));
                Some(VerifyExpr::apply(name, verify_args))
            }

            // Identity is world-invariant (rigid terms).
            ProofExpr::Identity(left, right) => {
                let l = proof_term_to_verify_expr(left)?;
                let r = proof_term_to_verify_expr(right)?;
                Some(VerifyExpr::eq(l, r))
            }

            ProofExpr::And(l, r) => Some(VerifyExpr::and(
                self.translate(l, w)?,
                self.translate(r, w)?,
            )),
            ProofExpr::Or(l, r) => Some(VerifyExpr::or(
                self.translate(l, w)?,
                self.translate(r, w)?,
            )),
            ProofExpr::Implies(l, r) => Some(VerifyExpr::implies(
                self.translate(l, w)?,
                self.translate(r, w)?,
            )),
            ProofExpr::Iff(l, r) => {
                let lv = self.translate(l, w)?;
                let rv = self.translate(r, w)?;
                Some(VerifyExpr::and(
                    VerifyExpr::implies(lv.clone(), rv.clone()),
                    VerifyExpr::implies(rv, lv),
                ))
            }
            ProofExpr::Not(inner) => Some(VerifyExpr::not(self.translate(inner, w)?)),

            ProofExpr::ForAll { variable, body } => Some(VerifyExpr::forall(
                vec![(variable.clone(), VerifyType::Int)],
                self.translate(body, w)?,
            )),
            ProofExpr::Exists { variable, body } => Some(VerifyExpr::exists(
                vec![(variable.clone(), VerifyType::Int)],
                self.translate(body, w)?,
            )),

            // □/◇ — quantify over the (domain, flavor) accessibility relation.
            ProofExpr::Modal {
                domain,
                force,
                flavor,
                body,
            } => {
                let relation = format!("Acc_{}_{}", domain, flavor);
                self.used_relations
                    .insert((domain.clone(), flavor.clone()));
                let w2 = self.fresh_world();
                let access = VerifyExpr::apply(
                    &relation,
                    vec![VerifyExpr::var(w), VerifyExpr::var(&w2)],
                );
                let body_v = self.translate(body, &w2)?;
                // Same force split as the Kripke display lowering.
                Some(if *force > 0.5 {
                    VerifyExpr::forall(
                        vec![(w2, VerifyType::Int)],
                        VerifyExpr::implies(access, body_v),
                    )
                } else {
                    VerifyExpr::exists(
                        vec![(w2, VerifyType::Int)],
                        VerifyExpr::and(access, body_v),
                    )
                })
            }

            // P □→ Q — universal over the closest P-worlds.
            ProofExpr::Counterfactual {
                antecedent,
                consequent,
            } => {
                let relation = self.cf_relation(antecedent);
                let w2 = self.fresh_world();
                let access = VerifyExpr::apply(
                    &relation,
                    vec![VerifyExpr::var(w), VerifyExpr::var(&w2)],
                );
                let body_v = self.translate(consequent, &w2)?;
                Some(VerifyExpr::forall(
                    vec![(w2, VerifyType::Int)],
                    VerifyExpr::implies(access, body_v),
                ))
            }

            // Tense/aspect over a temporal accessibility relation:
            // Always is universal; Past/Future/Eventually/Next existential.
            ProofExpr::Temporal { operator, body } => {
                let relation = format!("Time_{}", operator);
                self.used_temporal.insert(operator.clone());
                let w2 = self.fresh_world();
                let access = VerifyExpr::apply(
                    &relation,
                    vec![VerifyExpr::var(w), VerifyExpr::var(&w2)],
                );
                let body_v = self.translate(body, &w2)?;
                Some(if operator == "Always" {
                    VerifyExpr::forall(
                        vec![(w2, VerifyType::Int)],
                        VerifyExpr::implies(access, body_v),
                    )
                } else {
                    VerifyExpr::exists(
                        vec![(w2, VerifyType::Int)],
                        VerifyExpr::and(access, body_v),
                    )
                })
            }

            // Until/Release v1: a witness time where both sides hold.
            ProofExpr::TemporalBinary {
                operator,
                left,
                right,
            } => {
                let relation = format!("Time_{}", operator);
                self.used_temporal.insert(operator.clone());
                let w2 = self.fresh_world();
                let access = VerifyExpr::apply(
                    &relation,
                    vec![VerifyExpr::var(w), VerifyExpr::var(&w2)],
                );
                let l = self.translate(left, &w2)?;
                let r = self.translate(right, &w2)?;
                Some(VerifyExpr::exists(
                    vec![(w2, VerifyType::Int)],
                    VerifyExpr::and(access, VerifyExpr::and(l, r)),
                ))
            }

            // ∃e(Verb(e, w) ∧ Role(e, t) ∧ …) — the world rides on the verb.
            ProofExpr::NeoEvent {
                event_var,
                verb,
                roles,
            } => {
                let mut body = VerifyExpr::apply(
                    verb,
                    vec![VerifyExpr::var(event_var), VerifyExpr::var(w)],
                );
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

            // Outside the fragment: inductive types, lambda calculus, holes.
            ProofExpr::Ctor { .. }
            | ProofExpr::Match { .. }
            | ProofExpr::Fixpoint { .. }
            | ProofExpr::TypedVar { .. }
            | ProofExpr::Lambda { .. }
            | ProofExpr::App(_, _)
            | ProofExpr::Hole(_)
            | ProofExpr::Term(_)
            | ProofExpr::Unsupported(_) => None,
        }
    }

    /// Emit the frame axioms for every relation this problem used. Consumes
    /// the translator: counterfactual success axioms may themselves use
    /// accessibility relations (modal antecedents), so they are generated
    /// first and the domain-frame axioms read the final relation set.
    pub(crate) fn finalize(mut self) -> Option<Vec<VerifyExpr>> {
        let mut axioms = Vec::new();

        // Success axiom per Closest relation: closest A-worlds satisfy A.
        // Deliberately NO weak centering and NO seriality: an impossible
        // antecedent yields a vacuous counterfactual (Lewis), and the actual
        // world is not assumed among the closest (v1; CF-modus-ponens off).
        let antecedents = std::mem::take(&mut self.cf_antecedents);
        for (idx, (_, antecedent)) in antecedents.iter().enumerate() {
            let relation = format!("Closest_{}", idx);
            let access = VerifyExpr::apply(
                &relation,
                vec![VerifyExpr::var("wa"), VerifyExpr::var("wb")],
            );
            let holds = self.translate(antecedent, "wb")?;
            axioms.push(VerifyExpr::forall(
                vec![
                    ("wa".to_string(), VerifyType::Int),
                    ("wb".to_string(), VerifyType::Int),
                ],
                VerifyExpr::implies(access, holds),
            ));
        }

        for (domain, flavor) in &self.used_relations {
            let relation = format!("Acc_{}_{}", domain, flavor);
            if domain == "Alethic" && flavor == "Root" {
                // T (reflexivity): □P → P. T entails D, so nothing else.
                axioms.push(VerifyExpr::forall(
                    vec![("wt".to_string(), VerifyType::Int)],
                    VerifyExpr::apply(
                        &relation,
                        vec![VerifyExpr::var("wt"), VerifyExpr::var("wt")],
                    ),
                ));
            } else {
                // D (seriality): every world sees some world — obligations /
                // evidence / wishes are satisfiable but never factive.
                axioms.push(VerifyExpr::forall(
                    vec![("wd".to_string(), VerifyType::Int)],
                    VerifyExpr::exists(
                        vec![("we".to_string(), VerifyType::Int)],
                        VerifyExpr::apply(
                            &relation,
                            vec![VerifyExpr::var("wd"), VerifyExpr::var("we")],
                        ),
                    ),
                ));
            }
        }

        // Temporal relations carry no frame axioms in v1: identity and
        // K-style reasoning hold; ordering axioms come with the full tense
        // semantics later.

        Some(axioms)
    }
}
