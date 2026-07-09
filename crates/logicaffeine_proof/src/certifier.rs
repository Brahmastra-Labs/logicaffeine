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
/// When certifying a recursive case, IH references become recursive calls.
/// A single case may bind several recursive arguments (e.g. `Node(l, r)`), so it
/// carries one (recursive-argument variable, IH conclusion) entry per hypothesis.
struct InductionState {
    /// Name for self-reference in the fixpoint (e.g., "rec_n")
    fix_name: String,
    /// One entry per recursive argument: the bound variable and the IH it proves.
    /// A `PremiseMatch` whose conclusion matches an entry's IH resolves to
    /// `rec_name <var>` — the recursive call on that argument.
    ihs: Vec<(String, ProofExpr)>,
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
    /// Local *hypotheses* in scope, keyed by the proposition they prove and
    /// paired with the kernel term that witnesses it. Introduced by reductio
    /// (the assumed proposition), case analysis (the case formula / its negation
    /// in each branch), and existential elimination (the witness body and its
    /// projected conjuncts). A `PremiseMatch` whose conclusion matches one of
    /// these resolves to its witness term rather than to a global axiom. The
    /// witness is usually a bound variable (`Term::Var`), but for a projected
    /// conjunct of an `And` hypothesis it is a `Match` that eliminates the
    /// conjunction.
    /// Each entry is shared behind `Arc` so extending the context at a binder copies
    /// pointers, not whole (proposition, witness) pairs — building a large certified
    /// proof extends this thousands of times.
    local_hyps: Vec<std::sync::Arc<(ProofExpr, Term)>>,
    /// Induction state for IH resolution (only set during step case)
    induction_state: Option<InductionState>,
}

impl<'a> CertificationContext<'a> {
    pub fn new(kernel_ctx: &'a Context) -> Self {
        Self {
            kernel_ctx,
            locals: Vec::new(),
            local_hyps: Vec::new(),
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
            local_hyps: self.local_hyps.clone(),
            induction_state: self.induction_state.clone(),
        }
    }

    /// Create a new context with an additional local hypothesis: a proof of
    /// `prop` is available under the name `var`.
    fn with_local_hyp(&self, prop: &ProofExpr, var: &str) -> Self {
        self.with_local_hyp_term(prop, Term::Var(var.to_string()))
    }

    /// Create a new context with an additional local hypothesis: a proof of
    /// `prop` is given by the kernel term `witness`. Unlike [`with_local_hyp`],
    /// the witness need not be a bare variable — e.g. a projection that
    /// eliminates an `And` to recover one conjunct.
    fn with_local_hyp_term(&self, prop: &ProofExpr, witness: Term) -> Self {
        let mut new_hyps = self.local_hyps.clone();
        new_hyps.push(std::sync::Arc::new((prop.clone(), witness)));
        Self {
            kernel_ctx: self.kernel_ctx,
            locals: self.locals.clone(),
            local_hyps: new_hyps,
            induction_state: self.induction_state.clone(),
        }
    }

    /// A fresh hypothesis variable name, unique along the current branch.
    fn fresh_hyp_name(&self) -> String {
        format!("_hyp{}", self.local_hyps.len())
    }

    /// If `conclusion` is proved by a local hypothesis, return its witness term.
    fn get_local_hyp(&self, conclusion: &ProofExpr) -> Option<Term> {
        self.local_hyps
            .iter()
            .rev()
            .find(|h| h.0 == *conclusion)
            .map(|h| h.1.clone())
    }

    /// Create a new context with single-IH induction state (the Nat/List step case).
    fn with_induction(&self, fix_name: &str, step_var: &str, ih: ProofExpr) -> Self {
        self.with_induction_multi(fix_name, vec![(step_var.to_string(), ih)])
    }

    /// Create a new context whose induction hypotheses are `ihs` — one entry per
    /// recursive argument of the constructor case being certified.
    fn with_induction_multi(&self, fix_name: &str, ihs: Vec<(String, ProofExpr)>) -> Self {
        Self {
            kernel_ctx: self.kernel_ctx,
            locals: self.locals.clone(),
            local_hyps: self.local_hyps.clone(),
            induction_state: Some(InductionState {
                fix_name: fix_name.to_string(),
                ihs,
            }),
        }
    }

    /// Check if a name is a local variable.
    fn is_local(&self, name: &str) -> bool {
        self.locals.iter().any(|n| n == name)
    }

    /// Check if this conclusion matches one of the IHs in the current induction
    /// context. Returns the corresponding recursive call `rec <var>` if it matches.
    fn get_ih_term(&self, conclusion: &ProofExpr) -> Option<Term> {
        let state = self.induction_state.as_ref()?;
        for (var, ih) in &state.ihs {
            if conclusions_match(conclusion, ih) {
                // IH becomes a recursive call on that argument: rec <var>
                return Some(Term::App(
                    Box::new(Term::Var(state.fix_name.clone())),
                    Box::new(Term::Var(var.clone())),
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
            ihs: self.ihs.clone(),
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

        // Modus Tollens: P → Q, ¬Q ⊢ ¬P.  Since ¬P unfolds to P → False, the
        // proof is λ(hp:P). neg_q (impl hp) — feed a hypothetical P through the
        // implication to get Q, then through ¬Q (= Q → False) to get False.
        InferenceRule::ModusTollens => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "ModusTollens requires exactly 2 premises (implication, negated consequent)"
                        .to_string(),
                ));
            }
            let p = match &tree.conclusion {
                ProofExpr::Not(p) => p.as_ref().clone(),
                _ => {
                    return Err(KernelError::CertificationError(
                        "ModusTollens conclusion must be a negation".to_string(),
                    ))
                }
            };
            let p_type = proof_expr_to_type(&p)?;
            let impl_proof = certify(&tree.premises[0], ctx)?;
            let neg_q_proof = certify(&tree.premises[1], ctx)?;
            let hp = "__mt_hp".to_string();
            // λ(hp:P). neg_q_proof (impl_proof hp)  :  P → False  =  ¬P
            Ok(Term::Lambda {
                param: hp.clone(),
                param_type: Box::new(p_type),
                body: Box::new(Term::App(
                    Box::new(neg_q_proof),
                    Box::new(Term::App(
                        Box::new(impl_proof),
                        Box::new(Term::Var(hp)),
                    )),
                )),
            })
        }

        // Reflexivity of equality: a = a.  Curry-Howard: `refl T a`, where
        // `refl : Π(A:Type). Π(x:A). Eq A x x`. The two sides are definitionally
        // equal, so `refl T l : Eq T l l` reconciles with `Eq T l r`. The domain `T`
        // is inferred from the operands (so a ground `le 2 5 = true`, where both
        // sides reduce to `true : Bool`, certifies as `refl Bool (le 2 5)`).
        InferenceRule::Reflexivity => {
            let (l, r) = match &tree.conclusion {
                ProofExpr::Identity(l, r) => (l, r),
                _ => {
                    return Err(KernelError::CertificationError(
                        "Reflexivity conclusion must be an Identity".to_string(),
                    ))
                }
            };
            let domain = Term::Global(identity_domain(l, r).to_string());
            let term = proof_term_to_kernel_term(l)?;
            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("refl".to_string())),
                    Box::new(domain),
                )),
                Box::new(term),
            ))
        }

        // Ex falso quodlibet: ⊥ ⊢ G.  False has no constructors, so its eliminator
        // is a `match` with zero cases — `match false_proof return (λ_:False. G) {}`.
        InferenceRule::ExFalso => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "ExFalso requires exactly 1 premise (a proof of False)".to_string(),
                ));
            }
            let false_proof = certify(&tree.premises[0], ctx)?;
            let goal_type = proof_expr_to_type(&tree.conclusion)?;
            Ok(Term::Match {
                discriminant: Box::new(false_proof),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(Term::Global("False".to_string())),
                    body: Box::new(goal_type),
                }),
                cases: vec![],
            })
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

        // Disjunction Introduction: P ⊢ P ∨ Q  (or Q ⊢ P ∨ Q)
        // Curry-Howard: `left P Q p` or `right P Q q`. The rule is side-agnostic;
        // we recover the side by matching the proved premise against the disjuncts.
        InferenceRule::DisjunctionIntro => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "DisjunctionIntro requires exactly 1 premise".to_string(),
                ));
            }
            let (p, q) = match &tree.conclusion {
                ProofExpr::Or(p, q) => (p.as_ref().clone(), q.as_ref().clone()),
                _ => {
                    return Err(KernelError::CertificationError(
                        "DisjunctionIntro conclusion must be Or".to_string(),
                    ))
                }
            };
            let proved = &tree.premises[0].conclusion;
            let ctor = if proved == &p {
                "left"
            } else if proved == &q {
                "right"
            } else {
                return Err(KernelError::CertificationError(
                    "DisjunctionIntro premise proves neither disjunct".to_string(),
                ));
            };
            let p_type = proof_expr_to_type(&p)?;
            let q_type = proof_expr_to_type(&q)?;
            let proof_term = certify(&tree.premises[0], ctx)?;

            // left/right : Π(P:Prop). Π(Q:Prop). (P|Q) → Or P Q
            let applied = Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global(ctor.to_string())),
                        Box::new(p_type),
                    )),
                    Box::new(q_type),
                )),
                Box::new(proof_term),
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

        // Universal Instantiation at an arbitrary witness TERM: the same
        // Curry-Howard application, with the witness converted as a full term
        // (compound witnesses like `add(a, Zero)` included).
        InferenceRule::UniversalInstTerm(witness) => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "UniversalInstTerm requires exactly 1 premise".to_string(),
                ));
            }
            let forall_proof = certify(&tree.premises[0], ctx)?;
            let witness_term = match witness {
                ProofTerm::Constant(name) | ProofTerm::Variable(name)
                    if ctx.is_local(name) =>
                {
                    Term::Var(name.clone())
                }
                _ => proof_term_to_kernel_term(witness)?,
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

        // Generic structural induction over an arbitrary inductive type: one case
        // per constructor (in registration order), each recursive argument carrying
        // its own induction hypothesis. Certifies to `fix rec. λx. match x { … }`,
        // the dependent eliminator — the kernel re-checks coverage, case types, and
        // the termination guard, so an ill-formed scheme is rejected here.
        InferenceRule::InductionScheme {
            variable,
            ind_type,
            cases,
        } => {
            if tree.premises.len() != cases.len() {
                return Err(KernelError::CertificationError(format!(
                    "InductionScheme requires one premise per constructor: {} cases, {} premises",
                    cases.len(),
                    tree.premises.len()
                )));
            }

            let motive_body = extract_motive_body(&tree.conclusion, variable)?;
            let fix_name = format!("rec_{}", variable);

            let mut case_terms = Vec::with_capacity(cases.len());
            for (case, premise) in cases.iter().zip(tree.premises.iter()) {
                // Argument kernel types, peeled from the constructor's signature.
                let arg_types = constructor_arg_types(ctx.kernel_ctx, &case.constructor)?;
                if arg_types.len() != case.args.len() {
                    return Err(KernelError::CertificationError(format!(
                        "constructor {} takes {} arguments, case binds {}",
                        case.constructor,
                        arg_types.len(),
                        case.args.len()
                    )));
                }

                // Each recursive argument contributes an IH: the motive at that
                // argument (the induction variable substituted by the argument name).
                let mut ihs = Vec::new();
                for arg in &case.args {
                    if arg.recursive {
                        let ih = compute_ih_conclusion(&tree.conclusion, variable, &arg.name)?;
                        ihs.push((arg.name.clone(), ih));
                    }
                }

                // Certify the case under a context binding every argument as a local
                // and exposing the recursive IHs (so IH references become `rec <arg>`).
                let mut case_ctx = ctx.with_induction_multi(&fix_name, ihs);
                for arg in &case.args {
                    case_ctx = case_ctx.with_local(&arg.name);
                }
                let case_body = certify(premise, &case_ctx)?;

                // Wrap the body in one lambda per constructor argument:
                // λ(a0:T0). … λ(aN:TN). body  (nullary constructors stay bare).
                let mut term = case_body;
                for (arg, ty) in case.args.iter().zip(arg_types.iter()).rev() {
                    term = Term::Lambda {
                        param: arg.name.clone(),
                        param_type: Box::new(ty.clone()),
                        body: Box::new(term),
                    };
                }
                case_terms.push(term);
            }

            // match x return (λx:Ind. P(x)) with { case₀, …, caseₙ }
            let match_term = Term::Match {
                discriminant: Box::new(Term::Var(variable.clone())),
                motive: Box::new(build_motive(ind_type, &motive_body, variable)),
                cases: case_terms,
            };

            // fix rec_x. λx:Ind. match x { … }
            let lambda_term = Term::Lambda {
                param: variable.clone(),
                param_type: Box::new(Term::Global(ind_type.clone())),
                body: Box::new(match_term),
            };

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

            // Build predicate P = λvar:A. body_type — matching `proof_expr_to_type`'s
            // encoding of `Exists` EXACTLY (no eta-contracted `P`-direct shortcut for
            // the `∃y.P(y)` case), so the certified term's type and the goal type
            // agree structurally without relying on eta-conversion in the kernel.
            let predicate = Term::Lambda {
                param: variable.clone(),
                param_type: Box::new(type_a.clone()),
                body: Box::new(proof_expr_to_type(&body)?),
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

        // Existential Elimination: from ∃x.P(x) and a proof of the goal that uses
        // a fresh witness `c` with hypothesis P(c), eliminate the existential.
        // Curry-Howard: pattern-match the existential —
        //   match ex return (λ_:Ex Entity P. Goal) with witness c h => <Goal proof>
        // The witness `c` is the Match-bound variable, so within the body we turn
        // the placeholder constant `c` into that bound variable. Two shapes flow
        // through here: the prove-⊥ form (`cert_derive_falsum`, conclusion ⊥, body
        // references the whole `P(c)`) and the forward form (`try_existential_
        // elimination`, conclusion = the actual goal, body references the FLATTENED
        // conjuncts of `P(c)`). The motive's body is the conclusion's type — ⊥
        // gives `False`, recovering the prove-⊥ case unchanged — and the body
        // proof sees both the whole `P(c)` and each of its conjuncts (the latter
        // recovered by genuine ∧-elimination of `h`).
        InferenceRule::ExistentialElim { witness } => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "ExistentialElim requires exactly 2 premises (existential, body)".to_string(),
                ));
            }
            let exist_premise = &tree.premises[0];
            let body = &tree.premises[1];

            let (var, p_body) = match &exist_premise.conclusion {
                ProofExpr::Exists { variable, body } => (variable.clone(), body.as_ref().clone()),
                _ => {
                    return Err(KernelError::CertificationError(
                        "ExistentialElim first premise must conclude an Exists".to_string(),
                    ))
                }
            };

            // P(c) as a ProofExpr — the witness substituted for the bound var.
            let mut subst = crate::unify::Substitution::new();
            subst.insert(var.clone(), ProofTerm::Constant(witness.clone()));
            let p_c_expr = crate::unify::apply_subst_to_expr(&p_body, &subst);

            // Discriminant: the proof of the existential (type Ex Entity P).
            let disc = certify(exist_premise, ctx)?;

            // The witness body is proved by the bound hypothesis `h : P(c)`. Bind
            // the whole body (the prove-⊥ form references it directly) AND every
            // flattened conjunct, each recovered by ∧-eliminating `h` (the forward
            // form references the conjuncts separately).
            let h = format!("_exh_{}", witness);
            let h_var = Term::Var(h.clone());
            let mut body_ctx = ctx.with_local_hyp_term(&p_c_expr, h_var.clone());
            for (conjunct, proj) in collect_conjunct_hyps(&p_c_expr, &h_var)? {
                body_ctx = body_ctx.with_local_hyp_term(&conjunct, proj);
            }

            let body_raw = certify(body, &body_ctx)?;
            let c_global = Term::Global(witness.clone());
            let c_var = Term::Var(witness.clone());
            let body_term = substitute_term_in_kernel(&body_raw, &c_global, &c_var);
            let p_c_type =
                substitute_term_in_kernel(&proof_expr_to_type(&p_c_expr)?, &c_global, &c_var);

            // CONSTANT motive: the witness does not escape — the goal is closed (it does
            // not mention `c`) — so pass the goal type raw and let the kernel synthesize
            // `λ_:(Ex Entity P). Goal` from the discriminant's own type. (⊥ ↦ `False`,
            // keeping the prove-⊥ form intact.)
            let motive = proof_expr_to_type(&tree.conclusion)?;

            // Case for `witness`: λ(c:Entity). λ(h:P(c)). <Goal proof>
            let case = Term::Lambda {
                param: witness.clone(),
                param_type: Box::new(Term::Global("Entity".to_string())),
                body: Box::new(Term::Lambda {
                    param: h,
                    param_type: Box::new(p_c_type),
                    body: Box::new(body_term),
                }),
            };

            Ok(Term::Match {
                discriminant: Box::new(disc),
                motive: Box::new(motive),
                cases: vec![case],
            })
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
            // Build: Eq_rec D from P source_proof to eq_proof, where D is the domain
            // of the rewritten terms (Int for arithmetic, else Entity).
            let eq_rec = Term::Global("Eq_rec".to_string());
            let domain = Term::Global(term_domain(from).to_string());

            let applied = Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::App(Box::new(eq_rec), Box::new(domain))),
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
            let (x, y, domain) = match &tree.premises[0].conclusion {
                ProofExpr::Identity(l, r) => (
                    proof_term_to_kernel_term(l)?,
                    proof_term_to_kernel_term(r)?,
                    Term::Global(identity_domain(l, r).to_string()),
                ),
                _ => {
                    return Err(KernelError::CertificationError(
                        "EqualitySymmetry premise must be an Identity".to_string(),
                    ))
                }
            };

            // Eq_sym : Π(A:Type). Π(x:A). Π(y:A). Eq A x y → Eq A y x
            // Build: Eq_sym D x y proof, with D the operands' domain
            // (Int for arithmetic, Bool for comparisons, else Entity).
            let eq_sym = Term::Global("Eq_sym".to_string());

            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(Box::new(eq_sym), Box::new(domain))),
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
            let (x, y, domain) = match &tree.premises[0].conclusion {
                ProofExpr::Identity(l, r) => (
                    proof_term_to_kernel_term(l)?,
                    proof_term_to_kernel_term(r)?,
                    Term::Global(identity_domain(l, r).to_string()),
                ),
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
            // Build: Eq_trans D x y z proof1 proof2, with D the operands' domain
            // (Int for arithmetic, Bool for comparisons, else Entity).
            let eq_trans = Term::Global("Eq_trans".to_string());

            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::App(Box::new(eq_trans), Box::new(domain))),
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

        // `a ≤ b`, `b ≤ c` ⊢ `a ≤ c`:  le_trans a b c p₀ p₁.  Inequalities are the
        // Prop `Eq Bool (le a b) true`; the middle term comes from the first premise.
        InferenceRule::LeTrans => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "LeTrans requires exactly 2 premises".to_string(),
                ));
            }
            let (a, b) = le_terms(&tree.conclusion)?;
            let (_, mid) = le_terms(&tree.premises[0].conclusion)?;
            let p0 = certify(&tree.premises[0], ctx)?;
            let p1 = certify(&tree.premises[1], ctx)?;
            let mut t = Term::Global("le_trans".to_string());
            for arg in [
                proof_term_to_kernel_term(&a)?,
                proof_term_to_kernel_term(&mid)?,
                proof_term_to_kernel_term(&b)?,
                p0,
                p1,
            ] {
                t = Term::App(Box::new(t), Box::new(arg));
            }
            Ok(t)
        }

        // `⊢ a ≤ a`:  le_refl a.
        InferenceRule::LeRefl => {
            let (a, _) = le_terms(&tree.conclusion)?;
            Ok(Term::App(
                Box::new(Term::Global("le_refl".to_string())),
                Box::new(proof_term_to_kernel_term(&a)?),
            ))
        }

        // `a ≤ b`, `c ≤ d` ⊢ `a + c ≤ b + d`:  le_add_mono a b c d p₀ p₁.
        InferenceRule::LeAddMono => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "LeAddMono requires exactly 2 premises".to_string(),
                ));
            }
            let (lhs, rhs) = le_terms(&tree.conclusion)?;
            let (a, c) = add_terms(&lhs)?;
            let (b, d) = add_terms(&rhs)?;
            let p0 = certify(&tree.premises[0], ctx)?;
            let p1 = certify(&tree.premises[1], ctx)?;
            let mut t = Term::Global("le_add_mono".to_string());
            for arg in [
                proof_term_to_kernel_term(&a)?,
                proof_term_to_kernel_term(&b)?,
                proof_term_to_kernel_term(&c)?,
                proof_term_to_kernel_term(&d)?,
                p0,
                p1,
            ] {
                t = Term::App(Box::new(t), Box::new(arg));
            }
            Ok(t)
        }

        // Linear contradiction → ⊥: `premise[0] : le(m,n) = true` with `m > n` is, by
        // computation, a proof of `Eq Bool false true`; the Bool no-confusion
        // discriminator turns it into `False`.
        InferenceRule::LinFalse => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "LinFalse requires exactly 1 premise".to_string(),
                ));
            }
            let le_proof = certify(&tree.premises[0], ctx)?;
            Ok(build_bool_discriminator(le_proof))
        }

        // `0 ≤ k`, `a ≤ b` ⊢ `k·a ≤ k·b`:  le_mul_nonneg k a b p₀ p₁.
        InferenceRule::LeMulNonneg => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "LeMulNonneg requires exactly 2 premises".to_string(),
                ));
            }
            let (lhs, rhs) = le_terms(&tree.conclusion)?;
            let (k, a) = binop_terms("mul", &lhs)?;
            let (_, b) = binop_terms("mul", &rhs)?;
            let p0 = certify(&tree.premises[0], ctx)?;
            let p1 = certify(&tree.premises[1], ctx)?;
            let mut t = Term::Global("le_mul_nonneg".to_string());
            for arg in [
                proof_term_to_kernel_term(&k)?,
                proof_term_to_kernel_term(&a)?,
                proof_term_to_kernel_term(&b)?,
                p0,
                p1,
            ] {
                t = Term::App(Box::new(t), Box::new(arg));
            }
            Ok(t)
        }

        // `a ≤ b` ⊢ `0 ≤ b + (-1)·a`:  le_sub a b p₀.
        InferenceRule::LeSub => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "LeSub requires exactly 1 premise".to_string(),
                ));
            }
            let (_zero, diff) = le_terms(&tree.conclusion)?;
            let (b, neg_a) = binop_terms("add", &diff)?;
            let (_neg_one, a) = binop_terms("mul", &neg_a)?;
            let p0 = certify(&tree.premises[0], ctx)?;
            let mut t = Term::Global("le_sub".to_string());
            for arg in [proof_term_to_kernel_term(&a)?, proof_term_to_kernel_term(&b)?, p0] {
                t = Term::App(Box::new(t), Box::new(arg));
            }
            Ok(t)
        }

        // `a < b` ⊢ `(a + 1) ≤ b` — integer discreteness. The conclusion is
        // `le(add a 1, b) = true`, so `a` is the first summand of the sum and `b`
        // the right operand; `premise[0]` proves `lt(a, b) = true`. Certifies to
        // `lt_succ_le a b p₀` — the kernel axiom application.
        InferenceRule::LtSuccLe => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "LtSuccLe requires exactly 1 premise".to_string(),
                ));
            }
            let (succ_a, b) = le_terms(&tree.conclusion)?;
            let (a, _one) = binop_terms("add", &succ_a)?;
            let p0 = certify(&tree.premises[0], ctx)?;
            let mut t = Term::Global("lt_succ_le".to_string());
            for arg in [proof_term_to_kernel_term(&a)?, proof_term_to_kernel_term(&b)?, p0] {
                t = Term::App(Box::new(t), Box::new(arg));
            }
            Ok(t)
        }

        // `a < b + 1` ⊢ `a ≤ b`. The conclusion is `le(a, b) = true`; `premise[0]`
        // proves `lt(a, add b 1) = true`. Certifies to `lt_add1_le a b p₀`.
        InferenceRule::LtAdd1Le => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "LtAdd1Le requires exactly 1 premise".to_string(),
                ));
            }
            let (a, b) = le_terms(&tree.conclusion)?;
            let p0 = certify(&tree.premises[0], ctx)?;
            let mut t = Term::Global("lt_add1_le".to_string());
            for arg in [proof_term_to_kernel_term(&a)?, proof_term_to_kernel_term(&b)?, p0] {
                t = Term::App(Box::new(t), Box::new(arg));
            }
            Ok(t)
        }

        // Arithmetic decision: discharge an Int equality with the proof-producing
        // oracle. The oracle is untrusted — the term it returns is type-checked by
        // the kernel (here, by the caller's `infer_type`), so a wrong proof is
        // rejected. Proofs are built from computation + the registered ring axioms.
        InferenceRule::ArithDecision => match &tree.conclusion {
            ProofExpr::Identity(l, r) => {
                let kl = proof_term_to_kernel_term(l)?;
                let kr = proof_term_to_kernel_term(r)?;
                crate::arith::prove_int_eq(ctx.kernel_ctx, &kl, &kr).ok_or_else(|| {
                    KernelError::CertificationError(
                        "ArithDecision: arithmetic oracle found no proof".to_string(),
                    )
                })
            }
            _ => Err(KernelError::CertificationError(
                "ArithDecision conclusion must be an Identity".to_string(),
            )),
        },

        // Proof by kernel evaluation: the leaf carries only the claim. Build
        // the kernel proposition and its `Decidable` instance, then let
        // `native_decide` construct the `of_decide_eq_true`-shaped term — the
        // kernel re-checks it (via the `reduceBool` hook), so a false claim
        // certifies to nothing.
        InferenceRule::NativeDecide => {
            if !tree.premises.is_empty() {
                return Err(KernelError::CertificationError(
                    "NativeDecide is a leaf (no premises)".to_string(),
                ));
            }
            let prop = proof_expr_to_type_ctx(&tree.conclusion, ctx.kernel_ctx)?;
            let inst = decidable_instance_for(&prop).ok_or_else(|| {
                KernelError::CertificationError(
                    "NativeDecide: no Decidable instance for this proposition".to_string(),
                )
            })?;
            logicaffeine_kernel::native_decide(ctx.kernel_ctx, &prop, &inst).ok_or_else(|| {
                KernelError::CertificationError(
                    "NativeDecide: evaluation did not decide the goal true".to_string(),
                )
            })
        }

        // Contradiction: P and ¬P jointly yield ⊥.
        // Curry-Howard: ¬P is `P → False`, so applying the proof of ¬P to the
        // proof of P gives `False`. We detect which premise is the negation.
        InferenceRule::Contradiction => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "Contradiction requires exactly 2 premises (P and ¬P)".to_string(),
                ));
            }
            let a = &tree.premises[0];
            let b = &tree.premises[1];
            let (pos, neg) = if is_negation_of(&b.conclusion, &a.conclusion) {
                (a, b)
            } else if is_negation_of(&a.conclusion, &b.conclusion) {
                (b, a)
            } else {
                return Err(KernelError::CertificationError(format!(
                    "Contradiction premises are not a proposition and its negation: {:?} vs {:?}",
                    a.conclusion, b.conclusion
                )));
            };
            let pos_term = certify(pos, ctx)?;
            let neg_term = certify(neg, ctx)?;
            // (¬P) P : False
            Ok(Term::App(Box::new(neg_term), Box::new(pos_term)))
        }

        // Reductio ad absurdum: assume P, derive ⊥, conclude ¬P.
        // Curry-Howard: λ(h:P). <⊥-proof using h> : P → False = Not P.
        // The assumption becomes a local hypothesis available to the ⊥-derivation.
        InferenceRule::ReductioAdAbsurdum => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "ReductioAdAbsurdum requires exactly 2 premises (assumption, ⊥-derivation)"
                        .to_string(),
                ));
            }
            let assumed = &tree.premises[0].conclusion;
            let contradiction = &tree.premises[1];
            let hyp_name = ctx.fresh_hyp_name();
            let branch_ctx = ctx.with_local_hyp(assumed, &hyp_name);
            let body = certify(contradiction, &branch_ctx)?;
            Ok(Term::Lambda {
                param: hyp_name,
                param_type: Box::new(proof_expr_to_type(assumed)?),
                body: Box::new(body),
            })
        }

        // Case analysis where both cases reach ⊥. Intuitionistic form, needing
        // NO excluded middle: from the C-branch build `¬C : C → False`, from the
        // ¬C-branch build `¬¬C : (Not C) → False`, then `(¬¬C) (¬C) : False`.
        InferenceRule::CaseAnalysis { case_formula } => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "CaseAnalysis requires exactly 2 premises (C-branch, ¬C-branch)".to_string(),
                ));
            }
            let c = case_formula.as_ref();
            let not_c = ProofExpr::Not(Box::new(c.clone()));

            let branch_c = unwrap_case_branch(&tree.premises[0]);
            let branch_nc = unwrap_case_branch(&tree.premises[1]);

            // ¬C : C → False
            let h1 = ctx.fresh_hyp_name();
            let ctx_c = ctx.with_local_hyp(c, &h1);
            let neg_c = Term::Lambda {
                param: h1,
                param_type: Box::new(proof_expr_to_type(c)?),
                body: Box::new(certify(branch_c, &ctx_c)?),
            };

            // ¬¬C : (Not C) → False  (fresh name distinct from h1)
            let h2 = ctx_c.fresh_hyp_name();
            let ctx_nc = ctx.with_local_hyp(&not_c, &h2);
            let neg_neg_c = Term::Lambda {
                param: h2,
                param_type: Box::new(proof_expr_to_type(&not_c)?),
                body: Box::new(certify(branch_nc, &ctx_nc)?),
            };

            // (¬¬C) (¬C) : False
            Ok(Term::App(Box::new(neg_neg_c), Box::new(neg_c)))
        }

        // Disjunctive syllogism: from `A ∨ B` and `¬A`, conclude `B` (and the
        // mirror `¬B` ⊢ `A`). Curry-Howard, intuitionistically (no excluded
        // middle): match the `Or A B` proof. In the branch whose disjunct is
        // refuted, apply the negation `Not X = X → False` to the bound proof of
        // `X`, obtaining `False`, then eliminate `False` into the goal (a match
        // with no cases — `False` has no constructors). The surviving branch
        // returns its bound proof, which IS the goal.
        InferenceRule::DisjunctionElim => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "DisjunctionElim requires exactly 2 premises (disjunction, negation)"
                        .to_string(),
                ));
            }
            let disj_premise = &tree.premises[0];
            let neg_premise = &tree.premises[1];

            let (left, right) = match &disj_premise.conclusion {
                ProofExpr::Or(l, r) => (l.as_ref().clone(), r.as_ref().clone()),
                other => {
                    return Err(KernelError::CertificationError(format!(
                        "DisjunctionElim first premise must conclude a disjunction, got {:?}",
                        other
                    )))
                }
            };
            let refuted = match &neg_premise.conclusion {
                ProofExpr::Not(inner) => inner.as_ref().clone(),
                other => {
                    return Err(KernelError::CertificationError(format!(
                        "DisjunctionElim second premise must conclude a negation, got {:?}",
                        other
                    )))
                }
            };

            let left_is_refuted = conclusions_match(&refuted, &left);
            let right_is_refuted = conclusions_match(&refuted, &right);
            if left_is_refuted == right_is_refuted {
                return Err(KernelError::CertificationError(format!(
                    "DisjunctionElim negation {:?} matches neither (or both) disjuncts of {:?}",
                    refuted, disj_premise.conclusion
                )));
            }

            let goal_type = proof_expr_to_type(&tree.conclusion)?;
            let disc = certify(disj_premise, ctx)?;
            let neg_term = certify(neg_premise, ctx)?;

            // CONSTANT motive: pass the goal type raw (see DisjunctionCases) — the kernel
            // rebuilds `λ_:disc_type. goal` from the discriminant's `Or` type.
            let motive = goal_type.clone();

            // A branch that returns its bound proof directly (the surviving
            // disjunct equals the goal): λ(__d:D). __d.
            let return_case = |disjunct: &ProofExpr, binder: &str| -> KernelResult<Term> {
                Ok(Term::Lambda {
                    param: binder.to_string(),
                    param_type: Box::new(proof_expr_to_type(disjunct)?),
                    body: Box::new(Term::Var(binder.to_string())),
                })
            };
            // A branch whose disjunct is refuted: λ(__d:D). False_elim G ((¬D) __d).
            let absurd_case = |disjunct: &ProofExpr, binder: &str| -> KernelResult<Term> {
                let falsum = Term::App(
                    Box::new(neg_term.clone()),
                    Box::new(Term::Var(binder.to_string())),
                );
                let ex_falso = Term::Match {
                    discriminant: Box::new(falsum),
                    motive: Box::new(Term::Lambda {
                        param: "_".to_string(),
                        param_type: Box::new(Term::Global("False".to_string())),
                        body: Box::new(goal_type.clone()),
                    }),
                    cases: vec![],
                };
                Ok(Term::Lambda {
                    param: binder.to_string(),
                    param_type: Box::new(proof_expr_to_type(disjunct)?),
                    body: Box::new(ex_falso),
                })
            };

            let left_case = if left_is_refuted {
                absurd_case(&left, "__dl")?
            } else {
                return_case(&left, "__dl")?
            };
            let right_case = if right_is_refuted {
                absurd_case(&right, "__dr")?
            } else {
                return_case(&right, "__dr")?
            };

            Ok(Term::Match {
                discriminant: Box::new(disc),
                motive: Box::new(motive),
                cases: vec![left_case, right_case],
            })
        }

        // Disjunction elimination to a COMMON conclusion (here always ⊥): from
        // `A ∨ B`, a proof of the goal assuming `A`, and a proof assuming `B`,
        // Curry-Howard matches the `Or` and runs the matching branch. Each branch
        // binds its disjunct — and, when the disjunct is a conjunction, each
        // conjunct (recovered by ∧-elimination, as in `ExistentialElim`) — as a
        // local hypothesis, so the branch proof may cite them directly. This is the
        // case analysis a grid's of-pair / either-or / closure clause needs, and it
        // is intuitionistic (no excluded middle).
        InferenceRule::DisjunctionCases => {
            if tree.premises.len() != 3 {
                return Err(KernelError::CertificationError(
                    "DisjunctionCases requires exactly 3 premises (disjunction, A-branch, B-branch)"
                        .to_string(),
                ));
            }
            let disj_premise = &tree.premises[0];
            let (left, right) = match &disj_premise.conclusion {
                ProofExpr::Or(l, r) => (l.as_ref().clone(), r.as_ref().clone()),
                other => {
                    return Err(KernelError::CertificationError(format!(
                        "DisjunctionCases first premise must conclude a disjunction, got {:?}",
                        other
                    )))
                }
            };
            let disc = certify(disj_premise, ctx)?;
            // CONSTANT motive (the goal is independent of which disjunct held): pass the
            // goal TYPE raw and let the kernel synthesize `λ_:disc_type. goal` from the
            // discriminant's `Or` type. This keeps the large, left-nested `Or` OUT of the
            // emitted term at EVERY nested case split — the O(n²) certification blowup.
            let motive = proof_expr_to_type(&tree.conclusion)?;

            let build_arm = |disjunct: &ProofExpr,
                             branch: &DerivationTree,
                             binder: &str|
             -> KernelResult<Term> {
                let h_var = Term::Var(binder.to_string());
                let mut bctx = ctx.with_local_hyp_term(disjunct, h_var.clone());
                for (conjunct, proj) in collect_conjunct_hyps(disjunct, &h_var)? {
                    bctx = bctx.with_local_hyp_term(&conjunct, proj);
                }
                let body = certify(branch, &bctx)?;
                Ok(Term::Lambda {
                    param: binder.to_string(),
                    param_type: Box::new(proof_expr_to_type(disjunct)?),
                    body: Box::new(body),
                })
            };

            // FRESH binder names (`_hyp{depth}`), not fixed `__dcl`/`__dcr`: a branch
            // may itself contain a nested case-analysis, and a reused name would let the
            // inner binder CAPTURE an outer disjunct hypothesis in the kernel term (a
            // leaf citing the outer disjunct resolves to the inner λ of the wrong type).
            let lname = ctx.fresh_hyp_name();
            let rname = ctx.fresh_hyp_name();
            let left_case = build_arm(&left, &tree.premises[1], &lname)?;
            let right_case = build_arm(&right, &tree.premises[2], &rname)?;

            Ok(Term::Match {
                discriminant: Box::new(disc),
                motive: Box::new(motive),
                cases: vec![left_case, right_case],
            })
        }

        // Biconditional introduction (↔I): `conj (P→Q) (Q→P) <pq> <qp>` — the pair of
        // direction proofs combined, matching the `Iff ≡ And (P→Q) (Q→P)` encoding.
        InferenceRule::BicondIntro => {
            if tree.premises.len() != 2 {
                return Err(KernelError::CertificationError(
                    "BicondIntro requires exactly 2 premises (P→Q, Q→P)".to_string(),
                ));
            }
            let (p, q) = match &tree.conclusion {
                ProofExpr::Iff(p, q) => (p.as_ref().clone(), q.as_ref().clone()),
                other => {
                    return Err(KernelError::CertificationError(format!(
                        "BicondIntro conclusion must be a biconditional, got {:?}",
                        other
                    )))
                }
            };
            let pq_type =
                proof_expr_to_type(&ProofExpr::Implies(Box::new(p.clone()), Box::new(q.clone())))?;
            let qp_type =
                proof_expr_to_type(&ProofExpr::Implies(Box::new(q), Box::new(p)))?;
            let pq_proof = certify(&tree.premises[0], ctx)?;
            let qp_proof = certify(&tree.premises[1], ctx)?;
            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("conj".to_string())),
                            Box::new(pq_type),
                        )),
                        Box::new(qp_type),
                    )),
                    Box::new(pq_proof),
                )),
                Box::new(qp_proof),
            ))
        }

        // Double-negation introduction (constructive): P ⊢ ¬¬P. With `¬X ≡ X→False`,
        // `¬¬P = (P→False)→False`, proved by `λ(hnp:¬P). hnp p`.
        InferenceRule::DoubleNegation => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "DoubleNegation requires exactly 1 premise (a proof of P)".to_string(),
                ));
            }
            let p = match &tree.conclusion {
                ProofExpr::Not(inner) => match inner.as_ref() {
                    ProofExpr::Not(core) => core.as_ref().clone(),
                    other => {
                        return Err(KernelError::CertificationError(format!(
                            "DoubleNegation conclusion must be ¬¬P, got ¬{:?}",
                            other
                        )))
                    }
                },
                other => {
                    return Err(KernelError::CertificationError(format!(
                        "DoubleNegation conclusion must be ¬¬P, got {:?}",
                        other
                    )))
                }
            };
            let p_type = proof_expr_to_type(&p)?;
            let p_proof = certify(&tree.premises[0], ctx)?;
            let not_p_type = Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(p_type),
                body_type: Box::new(Term::Global("False".to_string())),
            };
            let hnp = "__dn_hnp".to_string();
            Ok(Term::Lambda {
                param: hnp.clone(),
                param_type: Box::new(not_p_type),
                body: Box::new(Term::App(Box::new(Term::Var(hnp)), Box::new(p_proof))),
            })
        }

        // Classical reductio: assume ¬G, derive ⊥, conclude G via the `dne` axiom.
        // Build `dne G (λ(hng:¬G). <⊥-proof>)`, where `λ(hng:¬G). ⊥ : ¬¬G`.
        InferenceRule::ClassicalReductio => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "ClassicalReductio requires exactly 1 premise (a proof of False)".to_string(),
                ));
            }
            let g = tree.conclusion.clone();
            let g_type = proof_expr_to_type(&g)?;
            let neg_g = ProofExpr::Not(Box::new(g));
            let neg_g_type = proof_expr_to_type(&neg_g)?; // G → False
            let binder = ctx.fresh_hyp_name();
            let bctx = ctx.with_local_hyp_term(&neg_g, Term::Var(binder.clone()));
            let false_proof = certify(&tree.premises[0], &bctx)?;
            let nn_g = Term::Lambda {
                param: binder,
                param_type: Box::new(neg_g_type),
                body: Box::new(false_proof),
            };
            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("dne".to_string())),
                    Box::new(g_type),
                )),
                Box::new(nn_g),
            ))
        }

        // Implication introduction (→I): `λ(hp:P). <Q-proof>`, binding the antecedent
        // P as a local hypothesis the consequent proof may cite — and, when P is a
        // conjunction, each conjunct by ∧-elimination (same as DisjunctionCases).
        InferenceRule::ImpliesIntro => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "ImpliesIntro requires exactly 1 premise (the consequent proof)".to_string(),
                ));
            }
            let ant = match &tree.conclusion {
                ProofExpr::Implies(a, _) => a.as_ref().clone(),
                other => {
                    return Err(KernelError::CertificationError(format!(
                        "ImpliesIntro conclusion must be an implication, got {:?}",
                        other
                    )))
                }
            };
            let binder = ctx.fresh_hyp_name();
            let h_var = Term::Var(binder.clone());
            let mut bctx = ctx.with_local_hyp_term(&ant, h_var.clone());
            for (conjunct, proj) in collect_conjunct_hyps(&ant, &h_var)? {
                bctx = bctx.with_local_hyp_term(&conjunct, proj);
            }
            let body = certify(&tree.premises[0], &bctx)?;
            Ok(Term::Lambda {
                param: binder,
                param_type: Box::new(proof_expr_to_type(&ant)?),
                body: Box::new(body),
            })
        }

        // Conjunction elimination: from a proof of `A ∧ B`, recover the conjunct
        // (`A` or `B`, whichever the conclusion names) by `∧`-elimination.
        InferenceRule::ConjunctionElim => {
            if tree.premises.len() != 1 {
                return Err(KernelError::CertificationError(
                    "ConjunctionElim requires exactly 1 premise (the conjunction)".to_string(),
                ));
            }
            let conj_premise = &tree.premises[0];
            let (a, b) = match &conj_premise.conclusion {
                ProofExpr::And(l, r) => (l.as_ref().clone(), r.as_ref().clone()),
                // Iff P Q ≡ And (P → Q) (Q → P) in the kernel encoding, so an
                // Iff premise projects to either implication.
                ProofExpr::Iff(l, r) => (
                    ProofExpr::Implies(l.clone(), r.clone()),
                    ProofExpr::Implies(r.clone(), l.clone()),
                ),
                other => {
                    return Err(KernelError::CertificationError(format!(
                        "ConjunctionElim premise must conclude a conjunction, got {:?}",
                        other
                    )))
                }
            };
            let take_left = conclusions_match(&tree.conclusion, &a);
            if !take_left && !conclusions_match(&tree.conclusion, &b) {
                return Err(KernelError::CertificationError(format!(
                    "ConjunctionElim conclusion {:?} matches neither conjunct of {:?}",
                    tree.conclusion, conj_premise.conclusion
                )));
            }
            let disc = certify(conj_premise, ctx)?;
            let left_type = proof_expr_to_type(&a)?;
            let right_type = proof_expr_to_type(&b)?;
            Ok(project_conjunct(&disc, &left_type, &right_type, take_left))
        }

        // Z3 oracle results are deliberately NOT certifiable. The oracle attests
        // *satisfiability* but hands back no checkable proof term — so it can
        // never be turned into a kernel certificate, and a goal discharged only
        // by the oracle is reported as unverified by the trusted door. (For
        // arithmetic, the proof-producing `ArithDecision` path yields a real
        // certificate instead.) Making this explicit keeps Z3 firmly outside the
        // trusted base.
        InferenceRule::OracleVerification(detail) => Err(KernelError::CertificationError(format!(
            "oracle (Z3) results are not kernel-certifiable — they attest \
             satisfiability but produce no checkable proof term ({})",
            detail
        ))),

        // Fallback for unimplemented rules
        rule => Err(KernelError::CertificationError(format!(
            "Certification not implemented for {:?}",
            rule
        ))),
    }
}

/// Whether `neg` is syntactically the negation of `pos` (i.e. `neg = ¬pos`).
fn is_negation_of(neg: &ProofExpr, pos: &ProofExpr) -> bool {
    matches!(neg, ProofExpr::Not(inner) if inner.as_ref() == pos)
}

/// Unwrap a case-analysis branch. The engine wraps each branch's ⊥-derivation
/// in a trivial `PremiseMatch(⊥)` node with a single child; the real proof is
/// that child. Any other shape is returned as-is.
fn unwrap_case_branch(tree: &DerivationTree) -> &DerivationTree {
    if matches!(tree.rule, InferenceRule::PremiseMatch) && tree.premises.len() == 1 {
        &tree.premises[0]
    } else {
        tree
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
        // The bound variable replaces `replace_term`, so it has that term's domain
        // (Int when rewriting arithmetic, else Entity).
        param_type: Box::new(Term::Global(term_domain(replace_term).to_string())),
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

/// Eliminate an `And A B` proof `disc` to recover one conjunct.
///
/// Builds `match disc return C with conj => λ(__l:A).λ(__r:B). __x`
/// (a CONSTANT motive — the kernel re-derives `λ_:And A B. C` from `disc`'s type)
/// where `C` is the chosen conjunct's type and `__x` is the corresponding bound
/// proof. This is genuine ∧-elimination — `And` is the inductive with the single
/// constructor `conj : Π P Q. P → Q → And P Q`, so the singleton-eliminator match
/// is total and the recovered term has exactly the conjunct's type.
fn project_conjunct(
    disc: &Term,
    left_type: &Term,
    right_type: &Term,
    take_left: bool,
) -> Term {
    let chosen = if take_left { left_type } else { right_type };
    let result = if take_left { "__l" } else { "__r" };
    let case = Term::Lambda {
        param: "__l".to_string(),
        param_type: Box::new(left_type.clone()),
        body: Box::new(Term::Lambda {
            param: "__r".to_string(),
            param_type: Box::new(right_type.clone()),
            body: Box::new(Term::Var(result.to_string())),
        }),
    };
    Term::Match {
        discriminant: Box::new(disc.clone()),
        // CONSTANT motive: pass the chosen conjunct's type raw; the kernel synthesizes
        // `λ_:disc_type. C` from the discriminant's own `And A B` type (type_checker's
        // Match arm wraps a Sort-typed motive automatically), keeping the conjunction
        // — which carries the of-pair's nested XOR — OUT of the emitted term.
        motive: Box::new(chosen.clone()),
        cases: vec![case],
    }
}

/// Collect each leaf conjunct of `expr` paired with a kernel term that proves it,
/// derived from `witness` (a proof of `proof_expr_to_type(expr)`). A non-`And`
/// proposition is its own single leaf, proved directly by `witness`. An
/// `And(l, r)` yields the leaves of `l` (proved by projecting `l` out of
/// `witness`) followed by the leaves of `r` (projecting `r`). This mirrors the
/// engine's `flatten_conjunction`, so the body proof — which references the
/// flattened conjuncts as separate premises — finds each one as a local
/// hypothesis backed by a sound ∧-elimination.
fn collect_conjunct_hyps(expr: &ProofExpr, witness: &Term) -> KernelResult<Vec<(ProofExpr, Term)>> {
    match expr {
        ProofExpr::And(l, r) => {
            let left_type = proof_expr_to_type(l)?;
            let right_type = proof_expr_to_type(r)?;

            let left_proj = project_conjunct(witness, &left_type, &right_type, true);
            let right_proj = project_conjunct(witness, &left_type, &right_type, false);

            let mut hyps = collect_conjunct_hyps(l, &left_proj)?;
            hyps.extend(collect_conjunct_hyps(r, &right_proj)?);
            Ok(hyps)
        }
        _ => Ok(vec![(expr.clone(), witness.clone())]),
    }
}

/// Certify a hypothesis reference (Axiom or PremiseMatch).
fn certify_hypothesis(conclusion: &ProofExpr, ctx: &CertificationContext) -> KernelResult<Term> {
    // Check if this is an IH reference (MUST check first!)
    if let Some(ih_term) = ctx.get_ih_term(conclusion) {
        return Ok(ih_term);
    }

    // A locally-assumed proposition (from reductio / case analysis) resolves to
    // its bound variable, regardless of the proposition's shape.
    if let Some(hyp) = ctx.get_local_hyp(conclusion) {
        return Ok(hyp);
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
            // Build the target type: P(a, b, ...) as nested application. Entity-position
            // lowering (predicate arguments are entities) so the lookup type matches the
            // hypothesis as registered — a numeric label `2004` is `Global`, not `Int`.
            let mut target_type = Term::Global(name.clone());
            for arg in args {
                let arg_term = proof_term_to_kernel_term_entity(arg)?;
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
        // Any other proposition (ForAll, Implies, Identity, Not, Or, And, …):
        // convert to its kernel type and look it up among the registered
        // hypotheses by structural match.
        _ => {
            // Context-aware so a `∀`-premise registered with a `Nat` binder (induction)
            // is found — its lookup type must match the binder domain it was declared with.
            let target_type = proof_expr_to_type_ctx(conclusion, ctx.kernel_ctx).map_err(|_| {
                KernelError::CertificationError(format!(
                    "Cannot certify hypothesis: {:?}",
                    conclusion
                ))
            })?;

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
        (Term::Lit(l1), Term::Lit(l2)) => l1 == l2,
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

/// The domain (the `Global` type name) of parameter `pos` in the registered
/// signature of `name`: peel its `Π`s and return the `pos`-th parameter type when it
/// is a `Global`. The seam that lets binder typing read a symbol's declared shape
/// instead of hardcoding constructor names.
fn param_domain(ctx: &Context, name: &str, pos: usize) -> Option<String> {
    let mut ty = ctx.get_global(name)?;
    let mut i = 0;
    while let Term::Pi { param_type, body_type, .. } = ty {
        if i == pos {
            return match param_type.as_ref() {
                Term::Global(g) => Some(g.clone()),
                _ => None,
            };
        }
        ty = body_type;
        i += 1;
    }
    None
}

/// Collect every kernel domain `variable` is constrained to within `t`: a DIRECT
/// argument at position `j` of a function/constructor whose signature types that
/// parameter `Global(g)` contributes `g`. Recurses through nested arguments, so a
/// variable under `Succ` (⇒ `Nat`) or in `Cons`'s tail (⇒ `List`) is found.
fn collect_var_domains_term(t: &ProofTerm, variable: &str, ctx: &Context, out: &mut Vec<String>) {
    match t {
        ProofTerm::Function(name, args) => {
            for (j, a) in args.iter().enumerate() {
                if matches!(a, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == variable) {
                    if let Some(g) = param_domain(ctx, name, j) {
                        out.push(g);
                    }
                }
                collect_var_domains_term(a, variable, ctx, out);
            }
        }
        ProofTerm::Group(args) => {
            for a in args {
                collect_var_domains_term(a, variable, ctx, out);
            }
        }
        _ => {}
    }
}

/// Like [`collect_var_domains_term`] but over a proposition: a variable fed DIRECTLY
/// to a predicate at position `j` is constrained to that predicate's `j`-th parameter
/// domain (so `P(t)` with `P : List → Prop` makes `t` a `List`).
fn collect_var_domains_expr(e: &ProofExpr, variable: &str, ctx: &Context, out: &mut Vec<String>) {
    match e {
        ProofExpr::Predicate { name, args, .. } => {
            for (j, a) in args.iter().enumerate() {
                if matches!(a, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == variable) {
                    if let Some(g) = param_domain(ctx, name, j) {
                        out.push(g);
                    }
                }
                collect_var_domains_term(a, variable, ctx, out);
            }
        }
        ProofExpr::Identity(l, r) => {
            collect_var_domains_term(l, variable, ctx, out);
            collect_var_domains_term(r, variable, ctx, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_var_domains_expr(l, variable, ctx, out);
            collect_var_domains_expr(r, variable, ctx, out);
        }
        ProofExpr::Not(p)
        | ProofExpr::ForAll { body: p, .. }
        | ProofExpr::Exists { body: p, .. } => collect_var_domains_expr(p, variable, ctx, out),
        _ => {}
    }
}

/// Infer the kernel domain of a `∀`-bound `variable` from how `body` uses it, by
/// reading the registered signatures of the predicates/constructors it is an argument
/// of: an inductive constraint (`Nat` from `Succ` or a `Nat`-predicate, `List` from a
/// `List`-predicate or `Cons`'s tail) wins over the `Entity` default, while `Cons`'s
/// head position correctly stays `Entity`. The slice of elaboration that lets a
/// context-free proposition acquire the binder type its symbols imply.
fn binder_domain(variable: &str, body: &ProofExpr, ctx: &Context) -> String {
    let mut domains = Vec::new();
    collect_var_domains_expr(body, variable, ctx, &mut domains);
    domains
        .into_iter()
        .find(|d| d != "Entity")
        .unwrap_or_else(|| "Entity".to_string())
}

/// Like [`proof_expr_to_type`] but resolves `∀`-binder domains against `ctx` (so a
/// quantifier over a `Nat`-used variable types as `Π(n:Nat). …`). Used where the
/// binder type is load-bearing — premise registration and the goal-type check —
/// while the context-free version still serves the inner certifier arms.
pub(crate) fn proof_expr_to_type_ctx(expr: &ProofExpr, ctx: &Context) -> KernelResult<Term> {
    if let ProofExpr::ForAll { variable, body } = expr {
        let dom = binder_domain(variable, body, ctx);
        let body_type = proof_expr_to_type_ctx(body, ctx)?;
        return Ok(Term::Pi {
            param: variable.clone(),
            param_type: Box::new(Term::Global(dom)),
            body_type: Box::new(body_type),
        });
    }
    proof_expr_to_type(expr)
}

pub(crate) fn proof_expr_to_type(expr: &ProofExpr) -> KernelResult<Term> {
    match expr {
        // Falsum maps to the kernel's `False : Prop`; other atoms are
        // propositional constants named directly.
        ProofExpr::Atom(name) if name == "⊥" || name == "False" || name == "false" => {
            Ok(Term::Global("False".to_string()))
        }
        ProofExpr::Atom(name) => Ok(Term::Global(name.clone())),
        // ¬P is `Not P`, which the kernel *defines* as `P → False`. We emit the
        // unfolded Pi form directly so the proposition is syntactically a
        // function in every position (application, hypothesis lookup, lambda
        // parameter types) without relying on delta-unfolding mid-typecheck.
        ProofExpr::Not(p) => Ok(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(proof_expr_to_type(p)?),
            body_type: Box::new(Term::Global("False".to_string())),
        }),
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
        // Iff P Q ≡ And (P → Q) (Q → P) — the same encoding `verify.rs:expand_iff`
        // uses for Iff premises, so Iff goals and Iff hypotheses agree.
        ProofExpr::Iff(p, q) => {
            let pt = proof_expr_to_type(p)?;
            let qt = proof_expr_to_type(q)?;
            let pq = Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(pt.clone()),
                body_type: Box::new(qt.clone()),
            };
            let qp = Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(qt),
                body_type: Box::new(pt),
            };
            Ok(Term::App(
                Box::new(Term::App(Box::new(Term::Global("And".to_string())), Box::new(pq))),
                Box::new(qp),
            ))
        }
        // ForAll ∀x.P(x) becomes Π(x:Entity). P(x)
        ProofExpr::ForAll { variable, body } => {
            let body_type = proof_expr_to_type(body)?;
            Ok(Term::Pi {
                param: variable.clone(),
                param_type: Box::new(Term::Global("Entity".to_string())),
                body_type: Box::new(body_type),
            })
        }
        // Predicate P(x, y, ...) becomes (P x y ...). A predicate is an uninterpreted
        // `Entity → … → Prop` relation, so its arguments are entities — a numeric
        // argument is an opaque label (a grid year `2004`), not an arithmetic literal.
        ProofExpr::Predicate { name, args, .. } => {
            let mut result = Term::Global(name.clone());
            for arg in args {
                let arg_term = proof_term_to_kernel_term_entity(arg)?;
                result = Term::App(Box::new(result), Box::new(arg_term));
            }
            Ok(result)
        }
        // Identity t1 = t2 becomes (Eq T t1 t2). The domain T is inferred from the
        // operands: a comparison (`le`/`lt`/…) or a Bool literal makes it `Eq Bool`
        // (the encoding `le a b = true` for `a ≤ b`); an arithmetic operator or
        // integer literal makes it `Eq Int`; otherwise the FOL default `Eq Entity`.
        ProofExpr::Identity(l, r) => {
            let l_term = proof_term_to_kernel_term(l)?;
            let r_term = proof_term_to_kernel_term(r)?;
            let domain = Term::Global(identity_domain(l, r).to_string());
            Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("Eq".to_string())),
                        Box::new(domain),
                    )),
                    Box::new(l_term),
                )),
                Box::new(r_term),
            ))
        }
        // Exists ∃x.P(x) becomes Ex Entity (λx.P(x)). FOL quantifies over
        // entities (matching the ForAll arm); `ExistentialIntro` may still carry
        // an explicit witness type for other domains.
        ProofExpr::Exists { variable, body } => {
            let var_type = Term::Global("Entity".to_string());
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
        // A temporal modality `Op(P)` — "he was seen" is `Past(see(butler))` —
        // is an opaque unary operator on propositions. It lowers to `(Op P)` with
        // `Op : Prop → Prop` (registered by the symbol collector), keeping
        // `Past(P)` a distinct proposition from `P` so a modus-tollens chain over
        // tensed premises certifies without conflating the two.
        ProofExpr::Temporal { operator, body } => Ok(Term::App(
            Box::new(Term::Global(operator.clone())),
            Box::new(proof_expr_to_type(body)?),
        )),
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
/// Translate the shared IR's canonical arithmetic function names to the kernel's
/// internal ring vocabulary. The proof IR is canonicalised to `Add`/`Sub`/`Mul`/
/// `Div` (so the SMT oracle recognises it); the kernel's LIA prover ([`crate::arith`])
/// and its ring axioms are written in lowercase `add`/`sub`/`mul`/`div`. This is the
/// single translation boundary — every other name passes through verbatim.
fn kernel_arith_name(name: &str) -> &str {
    match name {
        "Add" => "add",
        "Sub" => "sub",
        "Mul" => "mul",
        "Div" => "div",
        other => other,
    }
}

/// Is `name` an arithmetic or comparison builtin? Its operands are `Int` (so the
/// kernel's δ-rules compute on them — `add 2 3 ⇝ 5`, `le 2 5 ⇝ true`); every other
/// function is uninterpreted and `Entity`-valued. Mirrors the operand classification in
/// `verify::SymbolCollector` so registration and lowering agree on which numerics are Int.
fn is_arith_builtin(name: &str) -> bool {
    matches!(
        name,
        "le" | "lt" | "ge" | "gt" | "add" | "sub" | "mul" | "div" | "mod" | "Add" | "Sub"
            | "Mul" | "Div"
    )
}

/// Lower a proof term at an ENTITY position — a predicate argument, an uninterpreted
/// function's argument, an entity-domain identity operand. A bare numeric constant here
/// is an opaque `Entity` LABEL (a grid year `2004`, a jersey number) carried by a
/// first-order relation, NOT an arithmetic literal: it lowers to `Global "2004"` (which
/// resolves to the `Entity` constant `verify.rs` registers for it) so a monomorphic
/// relation like `in : Entity → Entity → Prop` — shared by a grid's year and state
/// columns — stays well-typed. A numeric nested under an arithmetic builtin still lowers
/// to an `Int` literal so the δ-rules fire. The position chooses the sort; the same
/// constant is one or the other by where it sits, never both. The entity-position dual of
/// [`proof_term_to_kernel_term`] (which lowers numerics as `Int`).
fn proof_term_to_kernel_term_entity(term: &ProofTerm) -> KernelResult<Term> {
    match term {
        ProofTerm::Constant(name) => Ok(Term::Global(name.clone())),
        ProofTerm::Variable(name) | ProofTerm::BoundVarRef(name) => Ok(Term::Var(name.clone())),
        ProofTerm::Function(name, args) => {
            let arith = is_arith_builtin(name);
            let mut result = Term::Global(kernel_arith_name(name).to_string());
            for arg in args {
                let arg_term = if arith {
                    proof_term_to_kernel_term(arg)?
                } else {
                    proof_term_to_kernel_term_entity(arg)?
                };
                result = Term::App(Box::new(result), Box::new(arg_term));
            }
            Ok(result)
        }
        ProofTerm::Group(_) => Err(KernelError::CertificationError(
            "Cannot convert Group to kernel term".to_string(),
        )),
    }
}

pub(crate) fn proof_term_to_kernel_term(term: &ProofTerm) -> KernelResult<Term> {
    match term {
        // A numeric constant is an integer literal — it must be `Lit(Int)` (not an
        // opaque `Global`) so the kernel's arithmetic/comparison delta rules fire
        // (`add 2 3 ⇝ 5`, `le 2 5 ⇝ true`), deciding ground facts by computation.
        ProofTerm::Constant(name) => match name.parse::<i64>() {
            Ok(n) => Ok(Term::Lit(logicaffeine_kernel::Literal::Int(n))),
            Err(_) => Ok(Term::Global(name.clone())),
        },
        ProofTerm::Variable(name) => Ok(Term::Var(name.clone())),
        ProofTerm::BoundVarRef(name) => Ok(Term::Var(name.clone())),
        ProofTerm::Function(name, args) => {
            // Build nested applications: f(a, b) -> ((f a) b). Canonical arithmetic
            // names are lowered to the kernel's ring vocabulary at this boundary.
            let mut result = Term::Global(kernel_arith_name(name).to_string());
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

/// The `Decidable` instance term for a kernel proposition, when one is
/// registered: `Eq Bool a b ↦ decEqBool a b`, `Eq Nat a b ↦ decEqNat a b`.
/// (Comparisons arrive here already lowered to `Eq Bool (le a b) true`.)
fn decidable_instance_for(prop: &Term) -> Option<Term> {
    let Term::App(f1, b) = prop else { return None };
    let Term::App(f2, a) = f1.as_ref() else { return None };
    let Term::App(eq, dom) = f2.as_ref() else { return None };
    let Term::Global(eq_name) = eq.as_ref() else { return None };
    if eq_name != "Eq" {
        return None;
    }
    let Term::Global(domain) = dom.as_ref() else { return None };
    let inst = match domain.as_str() {
        "Bool" => "decEqBool",
        "Nat" => "decEqNat",
        _ => return None,
    };
    Some(Term::App(
        Box::new(Term::App(
            Box::new(Term::Global(inst.to_string())),
            a.clone(),
        )),
        b.clone(),
    ))
}

/// Infer the `Eq` domain for an `Identity` from its operands (see the `Identity`
/// arm of [`proof_expr_to_type`]). A comparison (`le`/`lt`/…) or a Bool literal →
/// `Bool`; an arithmetic operator or integer literal → `Int`; otherwise the
/// first-order default `Entity`.
fn identity_domain(l: &ProofTerm, r: &ProofTerm) -> &'static str {
    match (term_domain(l), term_domain(r)) {
        ("Bool", _) | (_, "Bool") => "Bool",
        ("Int", _) | (_, "Int") => "Int",
        _ => "Entity",
    }
}

/// The kernel domain a single term belongs to: a comparison / Bool literal → `Bool`;
/// an arithmetic operator or integer literal → `Int`; otherwise `Entity`.
fn term_domain(t: &ProofTerm) -> &'static str {
    match t {
        ProofTerm::Function(n, _) if matches!(n.as_str(), "le" | "lt" | "ge" | "gt") => "Bool",
        ProofTerm::Function(n, _)
            if matches!(
                n.as_str(),
                "add" | "sub" | "mul" | "div" | "mod" | "Add" | "Sub" | "Mul" | "Div"
            ) =>
        {
            "Int"
        }
        ProofTerm::Constant(s) if s == "true" || s == "false" => "Bool",
        ProofTerm::Constant(s) if s.parse::<i64>().is_ok() => "Int",
        _ => "Entity",
    }
}

/// Extract `(a, b)` from an inequality conclusion `le(a, b) = true`, encoded in the
/// proof layer as `Identity(Function("le", [a, b]), Constant("true"))`.
fn le_terms(expr: &ProofExpr) -> KernelResult<(ProofTerm, ProofTerm)> {
    if let ProofExpr::Identity(lhs, _) = expr {
        if let ProofTerm::Function(name, args) = lhs {
            if name == "le" && args.len() == 2 {
                return Ok((args[0].clone(), args[1].clone()));
            }
        }
    }
    Err(KernelError::CertificationError(format!(
        "expected an `le(a, b) = true` inequality, got {:?}",
        expr
    )))
}

/// Extract `(x, y)` from a sum `add(x, y)`.
fn add_terms(t: &ProofTerm) -> KernelResult<(ProofTerm, ProofTerm)> {
    binop_terms("add", t)
}

/// Extract `(x, y)` from a binary application `op(x, y)`.
fn binop_terms(op: &str, t: &ProofTerm) -> KernelResult<(ProofTerm, ProofTerm)> {
    if let ProofTerm::Function(name, args) = t {
        if name == op && args.len() == 2 {
            return Ok((args[0].clone(), args[1].clone()));
        }
    }
    Err(KernelError::CertificationError(format!(
        "expected `{}(x, y)`, got {:?}",
        op, t
    )))
}

/// `Eq_rec Bool false P I true h : False`, where `h : Eq Bool false true` (or, by
/// computation, any `Eq Bool (le m n) true` with `m > n`). The motive
/// `P b = match b with true ⇒ False | false ⇒ True` is the Bool no-confusion
/// discriminator — turning a derived ground-false inequality into `⊥`.
fn build_bool_discriminator(h: Term) -> Term {
    let g = |s: &str| Term::Global(s.to_string());
    // P = λb:Bool. match b return (λ_:Bool. Prop) with | true ⇒ False | false ⇒ True
    let motive = Term::Lambda {
        param: "b".to_string(),
        param_type: Box::new(g("Bool")),
        body: Box::new(Term::Match {
            discriminant: Box::new(Term::Var("b".to_string())),
            motive: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(g("Bool")),
                body: Box::new(Term::Sort(logicaffeine_kernel::Universe::Prop)),
            }),
            cases: vec![g("False"), g("True")],
        }),
    };
    [g("Bool"), g("false"), motive, g("I"), g("true"), h]
        .into_iter()
        .fold(g("Eq_rec"), |acc, arg| Term::App(Box::new(acc), Box::new(arg)))
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

/// Peel a constructor's argument types from its signature. A monomorphic
/// inductive's constructor has type `Π(a₀:T₀). … Π(aₙ:Tₙ). Ind`, so the argument
/// types are exactly the `Pi` parameter types in order (a nullary constructor
/// yields an empty list). Used to type the case lambdas of a generic
/// [`InferenceRule::InductionScheme`] eliminator.
fn constructor_arg_types(ctx: &Context, constructor: &str) -> KernelResult<Vec<Term>> {
    let mut ty = ctx
        .get_global(constructor)
        .ok_or_else(|| {
            KernelError::CertificationError(format!("unknown constructor {}", constructor))
        })?
        .clone();
    let mut args = Vec::new();
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = ty
    {
        args.push(*param_type);
        ty = *body_type;
    }
    Ok(args)
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

#[cfg(test)]
mod canonical_arith_boundary_tests {
    use super::*;

    #[test]
    fn kernel_arith_name_maps_canonical_to_ring() {
        // The shared IR is canonicalised to Add/Sub/Mul/Div; the kernel ring axioms
        // and LIA prover speak lowercase add/sub/mul/div. The boundary translates.
        assert_eq!(kernel_arith_name("Add"), "add");
        assert_eq!(kernel_arith_name("Sub"), "sub");
        assert_eq!(kernel_arith_name("Mul"), "mul");
        assert_eq!(kernel_arith_name("Div"), "div");
        // Everything else passes through verbatim (measure fns, relations, names).
        assert_eq!(kernel_arith_name("Score"), "Score");
        assert_eq!(kernel_arith_name("add"), "add");
    }

    #[test]
    fn canonical_add_lowers_to_kernel_ring_head() {
        // ProofTerm::Function("Add", [x, y]) must lower to ((add x) y) so the
        // kernel arith prover (which matches Global("add")) can discharge it.
        let t = proof_term_to_kernel_term(&ProofTerm::Function(
            "Add".to_string(),
            vec![
                ProofTerm::Variable("x".to_string()),
                ProofTerm::Variable("y".to_string()),
            ],
        ))
        .expect("Add term converts to a kernel term");
        match t {
            Term::App(f, _) => match *f {
                Term::App(g, _) => match *g {
                    Term::Global(name) => assert_eq!(
                        name, "add",
                        "canonical Add must lower to the kernel ring name 'add'; got {name}"
                    ),
                    other => panic!("expected a Global head, got {:?}", other),
                },
                other => panic!("expected a nested App, got {:?}", other),
            },
            other => panic!("expected an App, got {:?}", other),
        }
    }

    // A numeric constant is ambiguous: an arithmetic `Int` literal in an Int position,
    // an opaque `Entity` LABEL (a grid year `2004`) in a relation position. The position
    // decides — this is what keeps a monomorphic relation (`in : Entity → Entity → Prop`,
    // shared by a logic grid's year and state columns) well-typed. These pin both arms;
    // the regression they guard froze the Studio's Simon puzzle by making `in(Beta, 2002)`
    // fail certification (`expected Entity, found Int`), which fell through to an unbounded
    // backward-chainer grind.

    /// The second argument of `in(Beta, 2002)` is the YEAR ENTITY `2002`, so it must lower
    /// to `Global("2002")` (resolving to the `Entity` constant `verify.rs` registers),
    /// never an `Int` literal — otherwise it clashes with `in : Entity → Entity → Prop`.
    #[test]
    fn numeric_predicate_argument_lowers_to_entity_label() {
        let in_beta_2002 = ProofExpr::Predicate {
            name: "in".to_string(),
            args: vec![
                ProofTerm::Constant("Beta".to_string()),
                ProofTerm::Constant("2002".to_string()),
            ],
            world: None,
        };
        let t = proof_expr_to_type(&in_beta_2002).expect("predicate lowers to a kernel type");
        // (in Beta) 2002 — the outer App's argument is the year term.
        match t {
            Term::App(_, year) => assert!(
                matches!(*year, Term::Global(ref n) if n == "2002"),
                "year `2002` in a relation must be an Entity Global, got {:?}",
                year
            ),
            other => panic!("expected `(in Beta) 2002` application, got {:?}", other),
        }
    }

    /// The dual: a numeric under an arithmetic builtin stays an `Int` literal so the
    /// kernel's δ-rules fire (`le 2 5 ⇝ true`). The fix must not touch this path.
    #[test]
    fn numeric_arithmetic_operand_stays_int_literal() {
        use logicaffeine_kernel::Literal;
        let le_2_5 = ProofTerm::Function(
            "le".to_string(),
            vec![
                ProofTerm::Constant("2".to_string()),
                ProofTerm::Constant("5".to_string()),
            ],
        );
        let t = proof_term_to_kernel_term(&le_2_5).expect("le term converts");
        match t {
            Term::App(_, five) => assert!(
                matches!(*five, Term::Lit(Literal::Int(5))),
                "an arithmetic operand must stay an Int literal, got {:?}",
                five
            ),
            other => panic!("expected `(le 2) 5` application, got {:?}", other),
        }
    }

    /// Entity-position lowering keeps arithmetic builtins nested inside it on the `Int`
    /// path (their operands still compute), while a bare numeric becomes an Entity label.
    #[test]
    fn entity_lowering_is_numeric_aware_but_arith_preserving() {
        use logicaffeine_kernel::Literal;
        assert!(
            matches!(
                proof_term_to_kernel_term_entity(&ProofTerm::Constant("2004".to_string())).unwrap(),
                Term::Global(ref n) if n == "2004"
            ),
            "a bare numeric in an entity position is an Entity label"
        );
        // add(2, 3) under an entity position still lowers its operands as Int literals.
        let add = ProofTerm::Function(
            "add".to_string(),
            vec![
                ProofTerm::Constant("2".to_string()),
                ProofTerm::Constant("3".to_string()),
            ],
        );
        match proof_term_to_kernel_term_entity(&add).unwrap() {
            Term::App(_, three) => assert!(
                matches!(*three, Term::Lit(Literal::Int(3))),
                "an arithmetic operand stays Int even inside an entity position, got {:?}",
                three
            ),
            other => panic!("expected `(add 2) 3`, got {:?}", other),
        }
    }
}
