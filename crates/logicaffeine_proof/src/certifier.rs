//! The Certifier: Converts DerivationTrees to Kernel Terms.
//!
//! This is the Great Bridge between the Proof Engine and the Kernel.
//! Via Curry-Howard, each proof rule maps to a term constructor:
//!
//! | DerivationTree Rule     | Kernel Term                           |
//! |-------------------------|---------------------------------------|
//! | Axiom / PremiseMatch    | Term::Global(name)                    |
//! | ModusPonens [impl, arg] | Term::App(certify(impl), certify(arg))|
//! | ConjunctionIntro [p, q] | conj P Q p_proof q_proof              |

use logicaffeine_kernel::{Context, KernelError, KernelResult, Term};

use crate::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// =============================================================================
// INDUCTION STATE - Tracks context during induction certification
// =============================================================================

/// State for tracking induction during certification.
/// When certifying the step case, IH references become recursive calls.
struct InductionState {
    /// Name for self-reference in the fixpoint (e.g., "rec_n")
    fix_name: String,
    /// The predecessor variable in the step case (e.g., "k")
    step_var: String,
    /// What the IH looks like in the derivation tree
    ih_conclusion: ProofExpr,
}

// =============================================================================
// CERTIFICATION CONTEXT
// =============================================================================

/// Context for certifying derivation trees into kernel terms.
///
/// Wraps a kernel [`Context`] and tracks additional state
/// needed during certification:
///
/// - **Local variables**: Variables bound by lambda abstractions during traversal
/// - **Induction state**: For resolving IH (inductive hypothesis) references in step cases
///
/// # Lifetime
///
/// The `'a` lifetime ties this context to the kernel context it wraps.
/// The certification context borrows the kernel context immutably.
///
/// # Example
///
/// ```
/// use logicaffeine_proof::certifier::CertificationContext;
/// use logicaffeine_kernel::Context;
///
/// let kernel_ctx = Context::new();
/// let cert_ctx = CertificationContext::new(&kernel_ctx);
/// ```
///
/// # See Also
///
/// * [`certify`] - The main function that uses this context
/// * [`Context`] - The underlying kernel context
pub struct CertificationContext<'a> {
    kernel_ctx: &'a Context,
    /// Local variables in scope (from lambda abstractions)
    locals: Vec<String>,
    /// Induction state for IH resolution (only set during step case)
    induction_state: Option<InductionState>,
}

impl<'a> CertificationContext<'a> {
    pub fn new(kernel_ctx: &'a Context) -> Self {
        Self {
            kernel_ctx,
            locals: Vec::new(),
            induction_state: None,
        }
    }

    /// Create a new context with an additional local variable.
    fn with_local(&self, name: &str) -> Self {
        let mut new_locals = self.locals.clone();
        new_locals.push(name.to_string());
        Self {
            kernel_ctx: self.kernel_ctx,
            locals: new_locals,
            induction_state: self.induction_state.clone(),
        }
    }

    /// Create a new context with induction state for step case certification.
    fn with_induction(&self, fix_name: &str, step_var: &str, ih: ProofExpr) -> Self {
        Self {
            kernel_ctx: self.kernel_ctx,
            locals: self.locals.clone(),
            induction_state: Some(InductionState {
                fix_name: fix_name.to_string(),
                step_var: step_var.to_string(),
                ih_conclusion: ih,
            }),
        }
    }

    /// Check if a name is a local variable.
    fn is_local(&self, name: &str) -> bool {
        self.locals.iter().any(|n| n == name)
    }

    /// Check if this conclusion matches the IH in current induction context.
    /// Returns the recursive call term if it matches.
    fn get_ih_term(&self, conclusion: &ProofExpr) -> Option<Term> {
        if let Some(state) = &self.induction_state {
            if conclusions_match(conclusion, &state.ih_conclusion) {
                // IH becomes: rec k (recursive call)
                return Some(Term::App(
                    Box::new(Term::Var(state.fix_name.clone())),
                    Box::new(Term::Var(state.step_var.clone())),
                ));
            }
        }
        None
    }
}

impl Clone for InductionState {
    fn clone(&self) -> Self {
        Self {
            fix_name: self.fix_name.clone(),
            step_var: self.step_var.clone(),
            ih_conclusion: self.ih_conclusion.clone(),
        }
    }
}

/// Check if two ProofExprs are structurally equivalent (for IH matching).
/// Uses PartialEq derivation.
fn conclusions_match(a: &ProofExpr, b: &ProofExpr) -> bool {
    a == b
}

/// Certify a derivation tree, producing a kernel term.
///
/// Converts a proof (derivation tree) into a typed lambda calculus term
/// via the Curry-Howard correspondence. The resulting term, when type-checked
/// by the kernel, should have the type corresponding to the tree's conclusion.
///
/// # Arguments
///
/// * `tree` - The derivation tree to certify
/// * `ctx` - The certification context (wraps kernel context)
///
/// # Returns
///
/// * `Ok(term)` - The kernel term representing this proof
/// * `Err(_)` - If certification fails (missing hypothesis, wrong premises, etc.)
///
/// # Curry-Howard Mapping
///
/// | Inference Rule | Kernel Term |
/// |----------------|-------------|
/// | `Axiom` / `PremiseMatch` | `Term::Global(name)` (hypothesis lookup) |
/// | `ModusPonens` | `Term::App(certify(impl), certify(arg))` |
/// | `ConjunctionIntro` | `conj P Q p_proof q_proof` |
/// | `UniversalInst(w)` | `Term::App(forall_proof, w)` |
/// | `UniversalIntro { var, type }` | `Term::Lambda { param: var, ... }` |
/// | `StructuralInduction` | `Term::Fix { name, body: λn. match n ... }` |
/// | `Rewrite { from, to }` | `Eq_rec A from P proof to eq_proof` |
/// | `ExistentialIntro` | `witness A P w proof` |
///
/// # Induction Handling
///
/// Structural induction produces fixpoint terms. The step case certification
/// tracks the induction variable so that IH references become recursive calls.
///
/// # See Also
///
/// * [`CertificationContext`] - The context passed to this function
/// * [`DerivationTree`] - The input proof structure
/// * [`InferenceRule`] - The rules being certified
pub fn certify(tree: &DerivationTree, ctx: &CertificationContext) -> KernelResult<Term> {
    match &tree.rule {
        // Axiom or direct hypothesis reference
        InferenceRule::Axiom | InferenceRule::PremiseMatch => {
            certify_hypothesis(&tree.conclusion, ctx)
        }

        // Modus Ponens: App(impl_proof, arg_proof)
        // P → Q, P ⊢ Q becomes (h1 h2) : Q
        InferenceRule::ModusPonens => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "ModusPonens requires exactly 2 premises".to_string(),
                ));
            }
            let impl_term = certify(&tree.premises[0], ctx)?;
            let arg_term = certify(&tree.premises[1], ctx)?;
            Ok(Term::App(Box::new(impl_term), Box::new(arg_term)))
        }

        // Conjunction Introduction: conj P Q p_proof q_proof
        // P, Q ⊢ P ∧ Q becomes (conj P Q p q) : And P Q
        InferenceRule::ConjunctionIntro => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "ConjunctionIntro requires exactly 2 premises".to_string(),
                ));
            }
            let (p_type, q_type) = extract_and_types(&tree.conclusion)?;
            let p_term = certify(&tree.premises[0], ctx)?;
            let q_term = certify(&tree.premises[1], ctx)?;

            // Build: conj P Q p_proof q_proof (fully curried)
            let conj = Term::Global("conj".to_string());
            let applied = Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(Box::new(conj), Box::new(p_type))),
                        Box::new(q_type),
                    )),
                    Box::new(p_term),
                )),
                Box::new(q_term),
            );
            Ok(applied)
        }

        // Universal Instantiation: forall x.P(x) |- P(t)
        // Curry-Howard: Apply the forall-proof (a function) to the witness term
        InferenceRule::UniversalInst(witness) => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "UniversalInst requires exactly 1 premise".to_string(),
                ));
            }
            let forall_proof = certify(&tree.premises[0], ctx)?;
            // Check if witness is a local variable (inside a lambda body)
            let witness_term = if ctx.is_local(witness) {
                Term::Var(witness.clone())
            } else {
                Term::Global(witness.clone())
            };
            Ok(Term::App(Box::new(forall_proof), Box::new(witness_term)))
        }

        // Universal Introduction: Γ, x:T ⊢ P(x) implies Γ ⊢ ∀x:T. P(x)
        // Curry-Howard: Wrap the body proof in a Lambda
        InferenceRule::UniversalIntro { variable, var_type } => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "UniversalIntro requires exactly 1 premise".to_string(),
                ));
            }

            // Build the type term
            let type_term = Term::Global(var_type.clone());

            // Certify body with variable in scope
            let extended_ctx = ctx.with_local(variable);
            let body_term = certify(&tree.premises[0], &extended_ctx)?;

            // Wrap in Lambda
            Ok(Term::Lambda {
                param: variable.clone(),
                param_type: Box::new(type_term),
                body: Box::new(body_term),
            })
        }

        // Structural Induction: P(0), ∀k(P(k) → P(S(k))) ⊢ ∀n P(n)
        // Curry-Howard: fix rec. λn. match n { Zero => base, Succ k => step }
        InferenceRule::StructuralInduction {
            variable: var_name,
            ind_type,
            step_var,
        } => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "StructuralInduction requires exactly 2 premises (base, step)".to_string(),
                ));
            }

            // Extract motive body from ForAll conclusion
            let motive_body = extract_motive_body(&tree.conclusion, var_name)?;

            // Generate fix name
            let fix_name = format!("rec_{}", var_name);

            // === Certify Base Case ===
            // Base case is certified in the current context (no IH)
            let base_term = certify(&tree.premises[0], ctx)?;

            // === Certify Step Case ===
            // IH: P(k) - compute what IH looks like by substituting n -> k
            let ih_conclusion = compute_ih_conclusion(&tree.conclusion, var_name, step_var)?;

            // Create context with:
            // 1. step_var as a local (bound by the Succ case lambda)
            // 2. induction state (for IH resolution)
            let step_ctx = ctx
                .with_local(step_var)
                .with_induction(&fix_name, step_var, ih_conclusion);

            let step_body = certify(&tree.premises[1], &step_ctx)?;

            // Wrap step body in lambda: λk. step_body
            let step_term = Term::Lambda {
                param: step_var.clone(),
                param_type: Box::new(Term::Global(ind_type.clone())),
                body: Box::new(step_body),
            };

            // === Build Match ===
            // match n return (λn:Nat. P(n)) with { Zero => base, Succ k => step }
            // The motive uses var_name so body references are properly bound
            let match_term = Term::Match {
                discriminant: Box::new(Term::Var(var_name.clone())),
                motive: Box::new(build_motive(ind_type, &motive_body, var_name)),
                cases: vec![base_term, step_term],
            };

            // === Build Lambda ===
            // λn:Nat. match n ...
            let lambda_term = Term::Lambda {
                param: var_name.clone(),
                param_type: Box::new(Term::Global(ind_type.clone())),
                body: Box::new(match_term),
            };

            // === Build Fixpoint ===
            // fix rec_n. λn. match n ...
            Ok(Term::Fix {
                name: fix_name,
                body: Box::new(lambda_term),
            })
        }

        // Existential Introduction: P(w) ⊢ ∃x.P(x)
        // Curry-Howard: witness A P w proof
        InferenceRule::ExistentialIntro {
            witness: witness_str,
            witness_type,
        } => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "ExistentialIntro requires exactly 1 premise".to_string(),
                ));
            }

            // Extract variable and body from Exists conclusion
            let (variable, body) = match &tree.conclusion {
                ProofExpr::Exists { variable, body } => (variable.clone(), body.as_ref().clone()),
                _ => {
                    return Err(KernelError::CertificationError(
                        "ExistentialIntro conclusion must be Exists".to_string(),
                    ))
                }
            };

            // Determine witness term (local or global)
            let witness_term = if ctx.is_local(witness_str) {
                Term::Var(witness_str.clone())
            } else {
                Term::Global(witness_str.clone())
            };

            // Certify the premise (proof of P(witness))
            let proof_term = certify(&tree.premises[0], ctx)?;

            // The type A comes from the InferenceRule (EXPLICIT INTENT)
            let type_a = Term::Global(witness_type.clone());

            // Build predicate P
            // If body is just "P(x)" where x is the bound variable, use P directly
            // Otherwise, build λvar. body_type
            let predicate = match &body {
                // Simple case: P(x) where x is the existential variable
                ProofExpr::Predicate { name, args, .. }
                    if args.len() == 1
                        && matches!(&args[0], ProofTerm::Variable(v) if v == &variable) =>
                {
                    Term::Global(name.clone())
                }
                // General case: wrap in lambda
                _ => {
                    let body_type = proof_expr_to_type(&body)?;
                    Term::Lambda {
                        param: variable.clone(),
                        param_type: Box::new(type_a.clone()),
                        body: Box::new(body_type),
                    }
                }
            };

            // Build: witness A P w proof
            // witness : Π(A:Type). Π(P:A→Prop). Π(x:A). P(x) → Ex A P
            let witness_ctor = Term::Global("witness".to_string());

            let applied = Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(Box::new(witness_ctor), Box::new(type_a))),
                        Box::new(predicate),
                    )),
                    Box::new(witness_term),
                )),
                Box::new(proof_term),
            );

            Ok(applied)
        }

        // Equality Rewriting (Leibniz's Law)
        // a = b, P(a) ⊢ P(b)
        // Uses: Eq_rec A x P proof y eq_proof
        InferenceRule::Rewrite { from, to } => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "Rewrite requires exactly 2 premises (equality, source)".to_string(),
                ));
            }

            // Certify both premises
            let eq_proof = certify(&tree.premises[0], ctx)?;
            let source_proof = certify(&tree.premises[1], ctx)?;

            // Convert terms
            let from_term = proof_term_to_kernel_term(from)?;
            let to_term = proof_term_to_kernel_term(to)?;

            // Build the predicate P as a lambda that wraps the goal
            // P = λz. (goal with 'to' replaced by z)
            let predicate = build_equality_predicate(&tree.conclusion, to)?;

            // Eq_rec : Π(A:Type). Π(x:A). Π(P:A→Prop). P x → Π(y:A). Eq A x y → P y
            // Build: Eq_rec Entity from P source_proof to eq_proof
            let eq_rec = Term::Global("Eq_rec".to_string());
            let entity = Term::Global("Entity".to_string());

            let applied = Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::App(Box::new(eq_rec), Box::new(entity))),
                                Box::new(from_term),
                            )),
                            Box::new(predicate),
                        )),
                        Box::new(source_proof),
                    )),
                    Box::new(to_term),
                )),
                Box::new(eq_proof),
            );

            Ok(applied)
        }

        // Equality Symmetry: a = b ⊢ b = a
        // Uses: Eq_sym A x y proof
        InferenceRule::EqualitySymmetry => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "EqualitySymmetry requires exactly 1 premise".to_string(),
                ));
            }

            let premise_proof = certify(&tree.premises[0], ctx)?;

            // Extract x and y from the premise conclusion (x = y)
            let (x, y) = match &tree.premises[0].conclusion {
                ProofExpr::Identity(l, r) => {
                    (proof_term_to_kernel_term(l)?, proof_term_to_kernel_term(r)?)
                }
                _ => {
                    return Err(KernelError::CertificationError(
                        "EqualitySymmetry premise must be an Identity".to_string(),
                    ))
                }
            };

            // Eq_sym : Π(A:Type). Π(x:A). Π(y:A). Eq A x y → Eq A y x
            // Build: Eq_sym Entity x y proof
            let eq_sym = Term::Global("Eq_sym".to_string());
            let entity = Term::Global("Entity".to_string());

            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(Box::new(eq_sym), Box::new(entity))),
                        Box::new(x),
                    )),
                    Box::new(y),
                )),
                Box::new(premise_proof),
            ))
        }

        // Equality Transitivity: a = b, b = c ⊢ a = c
        // Uses: Eq_trans A x y z proof1 proof2
        InferenceRule::EqualityTransitivity => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "EqualityTransitivity requires exactly 2 premises".to_string(),
                ));
            }

            let proof1 = certify(&tree.premises[0], ctx)?;
            let proof2 = certify(&tree.premises[1], ctx)?;

            // Extract x, y from first premise (x = y)
            let (x, y) = match &tree.premises[0].conclusion {
                ProofExpr::Identity(l, r) => {
                    (proof_term_to_kernel_term(l)?, proof_term_to_kernel_term(r)?)
                }
                _ => {
                    return Err(KernelError::CertificationError(
                        "EqualityTransitivity first premise must be Identity".to_string(),
                    ))
                }
            };

            // Extract z from second premise (y = z)
            let z = match &tree.premises[1].conclusion {
                ProofExpr::Identity(_, r) => proof_term_to_kernel_term(r)?,
                _ => {
                    return Err(KernelError::CertificationError(
                        "EqualityTransitivity second premise must be Identity".to_string(),
                    ))
                }
            };

            // Eq_trans : Π(A:Type). Π(x:A). Π(y:A). Π(z:A). Eq A x y → Eq A y z → Eq A x z
            // Build: Eq_trans Entity x y z proof1 proof2
            let eq_trans = Term::Global("Eq_trans".to_string());
            let entity = Term::Global("Entity".to_string());

            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::App(Box::new(eq_trans), Box::new(entity))),
                                Box::new(x),
                            )),
                            Box::new(y),
                        )),
                        Box::new(z),
                    )),
                    Box::new(proof1),
                )),
                Box::new(proof2),
            ))
        }

        // Fallback for unimplemented rules
        rule => Err(KernelError::CertificationError(format!(
            "Certification not implemented for {:?}",
            rule
        ))),
    }
}

/// Build the equality predicate for rewriting.
/// Given goal P(y) and term y, extracts P.
///
/// For simple predicates like `mortal(Superman)`, returns `mortal` directly.
/// For complex goals, builds `λz. (goal with y replaced by z)`.
fn build_equality_predicate(goal: &ProofExpr, replace_term: &ProofTerm) -> KernelResult<Term> {
    // Simple case: if the goal is P(y), just use P as the predicate
    // This avoids the beta-reduction issue with (λz. P(z)) x
    if let ProofExpr::Predicate { name, args, .. } = goal {
        if args.len() == 1 && &args[0] == replace_term {
            return Ok(Term::Global(name.clone()));
        }
    }

    // General case: build λz. (goal with replace_term replaced by z)
    let goal_type = proof_expr_to_type(goal)?;
    let param_name = "_eq_var".to_string();
    let substituted = substitute_term_in_kernel(
        &goal_type,
        &proof_term_to_kernel_term(replace_term)?,
        &Term::Var(param_name.clone()),
    );

    Ok(Term::Lambda {
        param: param_name,
        param_type: Box::new(Term::Global("Entity".to_string())),
        body: Box::new(substituted),
    })
}

/// Substitute a kernel term for another in a kernel Term.
fn substitute_term_in_kernel(term: &Term, from: &Term, to: &Term) -> Term {
    if term == from {
        return to.clone();
    }
    match term {
        Term::App(f, a) => Term::App(
            Box::new(substitute_term_in_kernel(f, from, to)),
            Box::new(substitute_term_in_kernel(a, from, to)),
        ),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(substitute_term_in_kernel(param_type, from, to)),
            body_type: Box::new(substitute_term_in_kernel(body_type, from, to)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(substitute_term_in_kernel(param_type, from, to)),
            body: Box::new(substitute_term_in_kernel(body, from, to)),
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(substitute_term_in_kernel(discriminant, from, to)),
            motive: Box::new(substitute_term_in_kernel(motive, from, to)),
            cases: cases
                .iter()
                .map(|c| substitute_term_in_kernel(c, from, to))
                .collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(substitute_term_in_kernel(body, from, to)),
        },
        other => other.clone(),
    }
}

/// Certify a hypothesis reference (Axiom or PremiseMatch).
fn certify_hypothesis(conclusion: &ProofExpr, ctx: &CertificationContext) -> KernelResult<Term> {
    // Check if this is an IH reference (MUST check first!)
    if let Some(ih_term) = ctx.get_ih_term(conclusion) {
        return Ok(ih_term);
    }

    match conclusion {
        ProofExpr::Atom(name) => {
            // Check locals first (for lambda-bound variables)
            if ctx.is_local(name) {
                return Ok(Term::Var(name.clone()));
            }
            // Then check globals
            if ctx.kernel_ctx.get_global(name).is_some() {
                Ok(Term::Global(name.clone()))
            } else {
                Err(KernelError::CertificationError(format!(
                    "Unknown hypothesis: {}",
                    name
                )))
            }
        }
        // For predicate hypotheses, look up by type in kernel context
        ProofExpr::Predicate { name, args, .. } => {
            // Build the target type: P(a, b, ...) as nested application
            let mut target_type = Term::Global(name.clone());
            for arg in args {
                let arg_term = proof_term_to_kernel_term(arg)?;
                target_type = Term::App(Box::new(target_type), Box::new(arg_term));
            }

            // Search declarations for one with matching type
            for (decl_name, decl_type) in ctx.kernel_ctx.iter_declarations() {
                if types_structurally_match(&target_type, decl_type) {
                    return Ok(Term::Global(decl_name.to_string()));
                }
            }

            Err(KernelError::CertificationError(format!(
                "Cannot find hypothesis with type: {:?}",
                conclusion
            )))
        }
        // For ForAll, Implies, and Identity hypotheses, look up by type in kernel context
        ProofExpr::ForAll { .. } | ProofExpr::Implies(_, _) | ProofExpr::Identity(_, _) => {
            // Convert the ProofExpr to a kernel type
            let target_type = proof_expr_to_type(conclusion)?;

            // Search declarations for one with matching type
            for (name, decl_type) in ctx.kernel_ctx.iter_declarations() {
                if types_structurally_match(&target_type, decl_type) {
                    return Ok(Term::Global(name.to_string()));
                }
            }

            Err(KernelError::CertificationError(format!(
                "Cannot find hypothesis with type matching: {:?}",
                conclusion
            )))
        }
        _ => Err(KernelError::CertificationError(format!(
            "Cannot certify hypothesis: {:?}",
            conclusion
        ))),
    }
}

/// Check if two kernel terms are alpha-equivalent.
/// Two terms are alpha-equivalent if they are the same up to renaming of bound variables.
fn types_structurally_match(a: &Term, b: &Term) -> bool {
    // Use a helper with a mapping from bound vars in a to bound vars in b
    types_alpha_equiv(a, b, &mut Vec::new())
}

/// Check alpha-equivalence with a mapping of bound variable pairs.
/// `bindings` tracks pairs of corresponding bound variable names.
fn types_alpha_equiv(a: &Term, b: &Term, bindings: &mut Vec<(String, String)>) -> bool {
    match (a, b) {
        (Term::Sort(u1), Term::Sort(u2)) => u1 == u2,
        (Term::Var(v1), Term::Var(v2)) => {
            // Check if these are corresponding bound variables
            for (bound_a, bound_b) in bindings.iter().rev() {
                if v1 == bound_a {
                    return v2 == bound_b;
                }
                if v2 == bound_b {
                    return false; // v2 is bound but v1 doesn't match
                }
            }
            // Both are free variables - must have same name
            v1 == v2
        }
        (Term::Global(g1), Term::Global(g2)) => g1 == g2,
        (Term::App(f1, a1), Term::App(f2, a2)) => {
            types_alpha_equiv(f1, f2, bindings) && types_alpha_equiv(a1, a2, bindings)
        }
        (
            Term::Pi {
                param: p1,
                param_type: pt1,
                body_type: bt1,
            },
            Term::Pi {
                param: p2,
                param_type: pt2,
                body_type: bt2,
            },
        ) => {
            // Parameter types must match in current scope
            if !types_alpha_equiv(pt1, pt2, bindings) {
                return false;
            }
            // Body types must match with the new binding
            bindings.push((p1.clone(), p2.clone()));
            let result = types_alpha_equiv(bt1, bt2, bindings);
            bindings.pop();
            result
        }
        (
            Term::Lambda {
                param: p1,
                param_type: pt1,
                body: b1,
            },
            Term::Lambda {
                param: p2,
                param_type: pt2,
                body: b2,
            },
        ) => {
            // Parameter types must match in current scope
            if !types_alpha_equiv(pt1, pt2, bindings) {
                return false;
            }
            // Bodies must match with the new binding
            bindings.push((p1.clone(), p2.clone()));
            let result = types_alpha_equiv(b1, b2, bindings);
            bindings.pop();
            result
        }
        _ => false,
    }
}

/// Extract P and Q types from an And(P, Q) conclusion.
fn extract_and_types(conclusion: &ProofExpr) -> KernelResult<(Term, Term)> {
    match conclusion {
        ProofExpr::And(p, q) => {
            let p_term = proof_expr_to_type(p)?;
            let q_term = proof_expr_to_type(q)?;
            Ok((p_term, q_term))
        }
        _ => Err(KernelError::CertificationError(format!(
            "Expected And, got {:?}",
            conclusion
        ))),
    }
}

/// Convert a ProofExpr (proposition) to a kernel Term (type).
fn proof_expr_to_type(expr: &ProofExpr) -> KernelResult<Term> {
    match expr {
        ProofExpr::Atom(name) => Ok(Term::Global(name.clone())),
        ProofExpr::And(p, q) => Ok(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("And".to_string())),
                Box::new(proof_expr_to_type(p)?),
            )),
            Box::new(proof_expr_to_type(q)?),
        )),
        ProofExpr::Or(p, q) => Ok(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("Or".to_string())),
                Box::new(proof_expr_to_type(p)?),
            )),
            Box::new(proof_expr_to_type(q)?),
        )),
        ProofExpr::Implies(p, q) => Ok(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(proof_expr_to_type(p)?),
            body_type: Box::new(proof_expr_to_type(q)?),
        }),
        // ForAll ∀x.P(x) becomes Π(x:Entity). P(x)
        ProofExpr::ForAll { variable, body } => {
            let body_type = proof_expr_to_type(body)?;
            Ok(Term::Pi {
                param: variable.clone(),
                param_type: Box::new(Term::Global("Entity".to_string())),
                body_type: Box::new(body_type),
            })
        }
        // Predicate P(x, y, ...) becomes (P x y ...)
        ProofExpr::Predicate { name, args, .. } => {
            let mut result = Term::Global(name.clone());
            for arg in args {
                let arg_term = proof_term_to_kernel_term(arg)?;
                result = Term::App(Box::new(result), Box::new(arg_term));
            }
            Ok(result)
        }
        // Identity t1 = t2 becomes (Eq Entity t1 t2)
        ProofExpr::Identity(l, r) => {
            let l_term = proof_term_to_kernel_term(l)?;
            let r_term = proof_term_to_kernel_term(r)?;
            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("Eq".to_string())),
                        Box::new(Term::Global("Entity".to_string())),
                    )),
                    Box::new(l_term),
                )),
                Box::new(r_term),
            ))
        }
        // Exists ∃x.P(x) becomes Ex ? (λx.P(x))
        // Note: Variable type defaults to Nat - use ExistentialIntro handler for explicit types
        ProofExpr::Exists { variable, body } => {
            let var_type = Term::Global("Nat".to_string()); // Default type
            let body_type = proof_expr_to_type(body)?;

            // Build: Ex VarType (λvar. body_type)
            let predicate = Term::Lambda {
                param: variable.clone(),
                param_type: Box::new(var_type.clone()),
                body: Box::new(body_type),
            };

            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Ex".to_string())),
                    Box::new(var_type),
                )),
                Box::new(predicate),
            ))
        }
        _ => Err(KernelError::CertificationError(format!(
            "Cannot convert {:?} to kernel type",
            expr
        ))),
    }
}

/// Convert a ProofTerm to a kernel Term.
///
/// This bridges the proof engine's term representation to the kernel's.
/// Used for converting witness terms in quantifier instantiation.
fn proof_term_to_kernel_term(term: &ProofTerm) -> KernelResult<Term> {
    match term {
        ProofTerm::Constant(name) => Ok(Term::Global(name.clone())),
        ProofTerm::Variable(name) => Ok(Term::Var(name.clone())),
        ProofTerm::BoundVarRef(name) => Ok(Term::Var(name.clone())),
        ProofTerm::Function(name, args) => {
            // Build nested applications: f(a, b) -> ((f a) b)
            let mut result = Term::Global(name.clone());
            for arg in args {
                let arg_term = proof_term_to_kernel_term(arg)?;
                result = Term::App(Box::new(result), Box::new(arg_term));
            }
            Ok(result)
        }
        ProofTerm::Group(_) => Err(KernelError::CertificationError(
            "Cannot convert Group to kernel term".to_string(),
        )),
    }
}

// =============================================================================
// INDUCTION HELPER FUNCTIONS
// =============================================================================

/// Extract motive body from ForAll conclusion.
/// The motive is the predicate body of the ForAll that we're proving by induction.
fn extract_motive_body(conclusion: &ProofExpr, var_name: &str) -> KernelResult<Term> {
    match conclusion {
        ProofExpr::ForAll { variable, body } if variable == var_name => {
            // Convert body to kernel type (the motive body)
            proof_expr_to_type(body)
        }
        _ => Err(KernelError::CertificationError(format!(
            "StructuralInduction conclusion must be ForAll over {}, got {:?}",
            var_name, conclusion
        ))),
    }
}

/// Compute what the IH conclusion looks like: P(k) when proving P(n).
/// Substitutes the induction variable with the step variable in the ForAll body.
fn compute_ih_conclusion(
    conclusion: &ProofExpr,
    orig_var: &str,
    step_var: &str,
) -> KernelResult<ProofExpr> {
    match conclusion {
        ProofExpr::ForAll { body, .. } => Ok(substitute_var_in_expr(body, orig_var, step_var)),
        _ => Err(KernelError::CertificationError(
            "Expected ForAll for IH computation".to_string(),
        )),
    }
}

/// Build the match motive: λn:IndType. ResultType
/// The motive_param should be the same variable name used in the body
/// so that the body's references are properly captured.
fn build_motive(ind_type: &str, result_type: &Term, motive_param: &str) -> Term {
    Term::Lambda {
        param: motive_param.to_string(),
        param_type: Box::new(Term::Global(ind_type.to_string())),
        body: Box::new(result_type.clone()),
    }
}

/// Substitute variable name in ProofExpr (recursive).
fn substitute_var_in_expr(expr: &ProofExpr, from: &str, to: &str) -> ProofExpr {
    match expr {
        ProofExpr::Atom(s) if s == from => ProofExpr::Atom(to.to_string()),
        ProofExpr::Atom(s) => ProofExpr::Atom(s.clone()),

        ProofExpr::TypedVar { name, typename } => ProofExpr::TypedVar {
            name: if name == from {
                to.to_string()
            } else {
                name.clone()
            },
            typename: typename.clone(),
        },

        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_var_in_term(a, from, to))
                .collect(),
            world: world.clone(),
        },

        ProofExpr::Identity(l, r) => ProofExpr::Identity(
            substitute_var_in_term(l, from, to),
            substitute_var_in_term(r, from, to),
        ),

        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(substitute_var_in_expr(l, from, to)),
            Box::new(substitute_var_in_expr(r, from, to)),
        ),

        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(substitute_var_in_expr(l, from, to)),
            Box::new(substitute_var_in_expr(r, from, to)),
        ),

        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(substitute_var_in_expr(l, from, to)),
            Box::new(substitute_var_in_expr(r, from, to)),
        ),

        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(substitute_var_in_expr(l, from, to)),
            Box::new(substitute_var_in_expr(r, from, to)),
        ),

        ProofExpr::Not(inner) => {
            ProofExpr::Not(Box::new(substitute_var_in_expr(inner, from, to)))
        }

        // Don't substitute inside binding that shadows
        ProofExpr::ForAll { variable, body } if variable != from => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(substitute_var_in_expr(body, from, to)),
        },

        ProofExpr::Exists { variable, body } if variable != from => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(substitute_var_in_expr(body, from, to)),
        },

        ProofExpr::Lambda { variable, body } if variable != from => ProofExpr::Lambda {
            variable: variable.clone(),
            body: Box::new(substitute_var_in_expr(body, from, to)),
        },

        ProofExpr::App(f, a) => ProofExpr::App(
            Box::new(substitute_var_in_expr(f, from, to)),
            Box::new(substitute_var_in_expr(a, from, to)),
        ),

        ProofExpr::Term(t) => ProofExpr::Term(substitute_var_in_term(t, from, to)),

        ProofExpr::Ctor { name, args } => ProofExpr::Ctor {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_var_in_expr(a, from, to))
                .collect(),
        },

        // For anything else (shadowed bindings, modals, temporals, etc.), clone as-is
        other => other.clone(),
    }
}

/// Substitute variable name in ProofTerm.
fn substitute_var_in_term(term: &ProofTerm, from: &str, to: &str) -> ProofTerm {
    match term {
        ProofTerm::Variable(s) if s == from => ProofTerm::Variable(to.to_string()),
        ProofTerm::Variable(s) => ProofTerm::Variable(s.clone()),
        ProofTerm::Constant(s) => ProofTerm::Constant(s.clone()),
        ProofTerm::BoundVarRef(s) if s == from => ProofTerm::BoundVarRef(to.to_string()),
        ProofTerm::BoundVarRef(s) => ProofTerm::BoundVarRef(s.clone()),
        ProofTerm::Function(name, args) => ProofTerm::Function(
            name.clone(),
            args.iter()
                .map(|a| substitute_var_in_term(a, from, to))
                .collect(),
        ),
        ProofTerm::Group(terms) => ProofTerm::Group(
            terms
                .iter()
                .map(|t| substitute_var_in_term(t, from, to))
                .collect(),
        ),
    }
}
