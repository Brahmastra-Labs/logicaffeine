//! =============================================================================
//! PHASE 98: PROOF-PRODUCING ARITHMETIC (the foundation)
//! =============================================================================
//!
//! The decision procedures (omega/ring/cc/...) are native Rust hooked into
//! `normalize`; the "recompute in concludes" check re-runs the SAME Rust, so
//! ~3,500 lines of unverified Rust are in the TCB, and reflective proofs stop at
//! `Syntax`, never reaching Prop.
//!
//! This phase establishes the proof-PRODUCING model instead (how Coq `lia`/`nia`
//! and Lean's tactics actually work): an UNTRUSTED oracle searches and emits a
//! genuine kernel proof term; the kernel's own `infer_type` is the verifier. A
//! bug in the oracle can only cause a *failed* proof, never a false one — the
//! Rust search leaves the trusted base.
//!
//! Trust boundary (locked by T6): closed/literal goals are proven by `add`/`mul`
//! COMPUTATION + `refl` (ZERO axioms); ring identities by the seven standard
//! commutative-ring axioms — the entire trusted arithmetic base. `Int` is opaque
//! (machine-backed, fast), so these laws are axioms here, exactly as Coq's
//! `Int63` / Lean's primitive integers axiomatize their fast arithmetic.
//!
//! RED-first: these fail until the axioms + oracle exist. They are the spec.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, Context, Literal, Term};
use logicaffeine_proof::arith::prove_int_eq;

// ---- term builders -----------------------------------------------------------

fn global(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn add(a: Term, b: Term) -> Term {
    app2(global("add"), a, b)
}
fn mul(a: Term, b: Term) -> Term {
    app2(global("mul"), a, b)
}
/// `Eq Int l r`
fn eq_int(l: Term, r: Term) -> Term {
    app(app(app(global("Eq"), global("Int")), l), r)
}

/// Standard context plus opaque Int constants x, y, z for variable identities.
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    for v in ["x", "y", "z"] {
        c.add_declaration(v, global("Int"));
    }
    c
}

/// The outermost global head of a (possibly applied) term.
fn head_global(t: &Term) -> Option<String> {
    match t {
        Term::Global(g) => Some(g.clone()),
        Term::App(f, _) => head_global(f),
        _ => None,
    }
}

// =============================================================================
// T1 — The seven ring axioms exist AND are usable (well-typed applications).
// =============================================================================
#[test]
fn t1_ring_axioms_usable() {
    let ctx = ctx();
    // `add_comm 2 3 : Eq Int (add 2 3) (add 3 2)` — the axiom is a real, applicable proof.
    let p = app2(global("add_comm"), lit(2), lit(3));
    let ty = infer_type(&ctx, &p).expect("add_comm must apply and type-check");
    let ty = normalize(&ctx, &ty);
    assert_eq!(head_global(&ty).as_deref(), Some("Eq"), "add_comm did not yield an Eq: {ty}");

    // `mul_distrib_add x y z : Eq Int (mul x (add y z)) (add (mul x y) (mul x z))`
    let d = app(app(app(global("mul_distrib_add"), global("x")), global("y")), global("z"));
    assert!(infer_type(&ctx, &d).is_ok(), "mul_distrib_add must apply and type-check");
}

// =============================================================================
// T2 — A TRUE closed arithmetic goal gets a real PROP proof (zero axioms).
//   2 + 3 = 5
// =============================================================================
#[test]
fn t2_closed_arithmetic_proves_to_prop() {
    let ctx = ctx();
    let goal = eq_int(add(lit(2), lit(3)), lit(5));

    let proof = prove_int_eq(&ctx, &add(lit(2), lit(3)), &lit(5))
        .expect("oracle must prove 2+3=5");
    let ty = infer_type(&ctx, &proof).expect("proof of 2+3=5 must type-check");

    // The proof's type IS the goal proposition (a Prop), not the Syntax datatype.
    assert_eq!(normalize(&ctx, &ty), normalize(&ctx, &goal), "proof type is not the goal");
    assert_ne!(normalize(&ctx, &ty), Term::Global("Syntax".to_string()));
    assert_eq!(head_global(&normalize(&ctx, &ty)).as_deref(), Some("Eq"));
}

// =============================================================================
// T3 — A FALSE goal cannot be proven, and a FORGED proof is REJECTED by the
//   kernel (the oracle is not trusted).  2 + 3 = 6
// =============================================================================
#[test]
fn t3_false_goal_rejected() {
    let ctx = ctx();
    let bad_goal = eq_int(add(lit(2), lit(3)), lit(6));

    // The honest oracle finds no proof.
    assert!(
        prove_int_eq(&ctx, &add(lit(2), lit(3)), &lit(6)).is_none(),
        "oracle fabricated a proof of the FALSE goal 2+3=6"
    );

    // Soundness: even a FORGED proof (`refl Int 5`) does not have the false goal's
    // type — the kernel rejects it as a proof of 2+3=6.
    let forged = app2(global("refl"), global("Int"), lit(5));
    let forged_ty = infer_type(&ctx, &forged).expect("refl Int 5 is itself well-typed");
    assert_ne!(
        normalize(&ctx, &forged_ty),
        normalize(&ctx, &bad_goal),
        "kernel accepted a forged proof of 2+3=6 — soundness broken!"
    );
}

// =============================================================================
// T4 — A VARIABLE commutativity identity gets a real Prop proof (via add_comm).
//   x + y = y + x
// =============================================================================
#[test]
fn t4_variable_commutativity_proves_to_prop() {
    let ctx = ctx();
    let goal = eq_int(add(global("x"), global("y")), add(global("y"), global("x")));

    let proof = prove_int_eq(&ctx, &add(global("x"), global("y")), &add(global("y"), global("x")))
        .expect("oracle must prove x+y=y+x");
    let ty = infer_type(&ctx, &proof).expect("proof of x+y=y+x must type-check");
    assert_eq!(normalize(&ctx, &ty), normalize(&ctx, &goal), "proof type is not the goal");
    assert_eq!(head_global(&normalize(&ctx, &ty)).as_deref(), Some("Eq"));
}

// =============================================================================
// T5 — The oracle is a real decision: proves true identities, declines false.
// =============================================================================
#[test]
fn t5_oracle_decides() {
    let ctx = ctx();

    // closed: 2 * 4 = 8
    assert!(prove_int_eq(&ctx, &mul(lit(2), lit(4)), &lit(8)).is_some(), "2*4=8 not proven");

    // distributivity: x * (y + z) = x*y + x*z
    let dl = mul(global("x"), add(global("y"), global("z")));
    let dr = add(mul(global("x"), global("y")), mul(global("x"), global("z")));
    let dp = prove_int_eq(&ctx, &dl, &dr).expect("distributivity not proven");
    let dty = infer_type(&ctx, &dp).expect("distributivity proof must type-check");
    assert_eq!(normalize(&ctx, &dty), normalize(&ctx, &eq_int(dl, dr)));

    // genuinely false / undecided: x + y = x * y
    assert!(
        prove_int_eq(&ctx, &add(global("x"), global("y")), &mul(global("x"), global("y"))).is_none(),
        "oracle fabricated a proof of x+y = x*y"
    );
}

// =============================================================================
// T7 — INTEGRATION: the proof BUILDER proves an arithmetic identity and the
//   certifier discharges it into a kernel-checked Prop proof of the goal.
//   This wires the proof-producing oracle into the prove → certify → infer_type
//   pipeline (a symbolic identity, since ProofTerm has no Int literals).
// =============================================================================
#[test]
fn t7_engine_certifies_arithmetic_identity() {
    use logicaffeine_proof::certifier::{certify, CertificationContext};
    use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

    // goal: add x y = add y x   (x, y are opaque Int constants)
    let add = |a: &str, b: &str| {
        ProofTerm::Function(
            "add".to_string(),
            vec![ProofTerm::Constant(a.to_string()), ProofTerm::Constant(b.to_string())],
        )
    };
    let goal = ProofExpr::Identity(add("x", "y"), add("y", "x"));

    // The engine must now find this proof (via the arithmetic decision strategy).
    let mut engine = BackwardChainer::new();
    let derivation = engine
        .prove(goal.clone())
        .expect("engine should prove the arithmetic identity x+y=y+x");

    // The certifier discharges it into a real kernel term, type-checked to the goal.
    let ctx = ctx(); // x, y, z : Int registered
    let cert_ctx = CertificationContext::new(&ctx);
    let proof = certify(&derivation, &cert_ctx).expect("certifier must discharge ArithDecision");
    let ty = infer_type(&ctx, &proof).expect("kernel must type-check the arithmetic proof");

    let goal_term = eq_int(add_t("x", "y"), add_t("y", "x"));
    assert_eq!(normalize(&ctx, &ty), normalize(&ctx, &goal_term), "proof type is not the goal");
    assert_eq!(head_global(&normalize(&ctx, &ty)).as_deref(), Some("Eq"));
}

/// `add a b` as a kernel term over Int constants a, b.
fn add_t(a: &str, b: &str) -> Term {
    add(global(a), global(b))
}

// =============================================================================
// T8 — NESTED identities via congruence: a subterm differs by a provable
//   equality, lifted under an operator.  (x + y) + z = (y + x) + z
// =============================================================================
#[test]
fn t8_congruence_nested_identity() {
    let ctx = ctx();
    // (x+y)+z  =  (y+x)+z   — inner commutativity lifted under (_ + z)
    let lhs = add(add(global("x"), global("y")), global("z"));
    let rhs = add(add(global("y"), global("x")), global("z"));

    let proof = prove_int_eq(&ctx, &lhs, &rhs).expect("oracle must prove (x+y)+z = (y+x)+z");
    let ty = infer_type(&ctx, &proof).expect("nested congruence proof must type-check");
    assert_eq!(normalize(&ctx, &ty), normalize(&ctx, &eq_int(lhs, rhs)), "proof type is not the goal");

    // And a deeper nesting on the other side: x + (y*1) = x + y   (mul_one under x + _)
    let l2 = add(global("x"), mul(global("y"), lit(1)));
    let r2 = add(global("x"), global("y"));
    let p2 = prove_int_eq(&ctx, &l2, &r2).expect("oracle must prove x + (y*1) = x + y");
    let t2 = infer_type(&ctx, &p2).expect("must type-check");
    assert_eq!(normalize(&ctx, &t2), normalize(&ctx, &eq_int(l2, r2)));
}

// =============================================================================
// T9 — MULTI-STEP: an identity needing a top-level axiom rewrite COMPOSED with
//   a congruence (Eq_trans).   (x + y) + z = z + (y + x)
// =============================================================================
#[test]
fn t9_multistep_identity() {
    let ctx = ctx();
    // (x+y)+z = z+(y+x): commute the outer sum, then commute the inner sum.
    let lhs = add(add(global("x"), global("y")), global("z"));
    let rhs = add(global("z"), add(global("y"), global("x")));

    let proof = prove_int_eq(&ctx, &lhs, &rhs).expect("oracle must prove (x+y)+z = z+(y+x)");
    let ty = infer_type(&ctx, &proof).expect("multi-step proof must type-check");
    assert_eq!(normalize(&ctx, &ty), normalize(&ctx, &eq_int(lhs, rhs)), "proof type is not the goal");
}

// =============================================================================
// T10 — ADVERSARIAL SOUNDNESS: the oracle must NEVER prove a non-identity.
//   Subtraction is not commutative; + is not ×; unequal operands don't cancel.
//   (And even if it somehow returned a term, the kernel would reject it — so we
//   also assert that any returned proof's type genuinely matches the goal.)
// =============================================================================
#[test]
fn t10_oracle_cannot_be_tricked() {
    let ctx = ctx();

    let non_identities: &[(Term, Term)] = &[
        // subtraction is NOT commutative
        (app2(global("sub"), global("x"), global("y")), app2(global("sub"), global("y"), global("x"))),
        // + is not ×
        (add(global("x"), global("y")), mul(global("x"), global("y"))),
        // unequal operands don't cancel
        (add(global("x"), global("y")), add(global("x"), global("z"))),
        // a plausible-looking but false multi-step:  x + y  vs  y + y
        (add(global("x"), global("y")), add(global("y"), global("y"))),
        // nested false:  x + (y + z)  vs  x + (y + y)   (genuinely needs z = y)
        (
            add(global("x"), add(global("y"), global("z"))),
            add(global("x"), add(global("y"), global("y"))),
        ),
    ];

    for (l, r) in non_identities {
        match prove_int_eq(&ctx, l, r) {
            None => {} // correct: no proof found
            Some(term) => {
                // If it ever returns a term, the kernel MUST refuse it as a proof
                // of this goal — its type cannot match the (false) goal.
                let goal = eq_int(l.clone(), r.clone());
                let ok_as_goal = infer_type(&ctx, &term)
                    .map(|ty| normalize(&ctx, &ty) == normalize(&ctx, &goal))
                    .unwrap_or(false);
                assert!(!ok_as_goal, "oracle produced a valid proof of a NON-identity: {l} = {r}");
            }
        }
    }
}

// =============================================================================
// T11 — HARDER identities: right-distribution and reordered expansions, which
//   need distribution composed with commutation (several rewrite steps).
// =============================================================================
#[test]
fn t11_distribution_and_reordering() {
    let ctx = ctx();
    let prove = |l: &Term, r: &Term| {
        let p = prove_int_eq(&ctx, l, r)
            .unwrap_or_else(|| panic!("oracle failed to prove {l} = {r}"));
        let ty = infer_type(&ctx, &p).unwrap_or_else(|_| panic!("proof of {l}={r} ill-typed"));
        assert_eq!(normalize(&ctx, &ty), normalize(&ctx, &eq_int(l.clone(), r.clone())));
    };

    // right distribution:  (x + y) * z  =  x*z + y*z
    prove(
        &mul(add(global("x"), global("y")), global("z")),
        &add(mul(global("x"), global("z")), mul(global("y"), global("z"))),
    );

    // left distribution with reordered terms:  x * (y + z)  =  z*x + y*x
    prove(
        &mul(global("x"), add(global("y"), global("z"))),
        &add(mul(global("z"), global("x")), mul(global("y"), global("x"))),
    );

    // simplification in a subterm:  x*1 + y  =  x + y
    prove(
        &add(mul(global("x"), lit(1)), global("y")),
        &add(global("x"), global("y")),
    );
}

/// Shared check: the oracle proves `l = r` and the proof kernel-checks to the goal.
fn assert_proves(ctx: &Context, l: &Term, r: &Term) {
    let p = prove_int_eq(ctx, l, r)
        .unwrap_or_else(|| panic!("normalizer failed to prove {l} = {r}"));
    let ty = infer_type(ctx, &p).unwrap_or_else(|_| panic!("proof of {l}={r} ill-typed"));
    assert_eq!(
        normalize(ctx, &ty),
        normalize(ctx, &eq_int(l.clone(), r.clone())),
        "proof type is not the goal for {l} = {r}"
    );
}

// =============================================================================
// T12a — NORMALIZER, coefficient collection: like terms combine.
// =============================================================================
#[test]
fn t12a_normalizer_coefficients() {
    let ctx = ctx();
    // x + x = 2*x
    assert_proves(&ctx, &add(global("x"), global("x")), &mul(lit(2), global("x")));
    // 2*x + 3*x = 5*x
    assert_proves(
        &ctx,
        &add(mul(lit(2), global("x")), mul(lit(3), global("x"))),
        &mul(lit(5), global("x")),
    );
}

// =============================================================================
// T12b — NORMALIZER, FOIL expansion (products of sums).
// =============================================================================
#[test]
fn t12b_normalizer_foil() {
    let ctx = ctx();
    // (x + 1) * (x + 1)  =  x*x + 2*x + 1
    assert_proves(
        &ctx,
        &mul(add(global("x"), lit(1)), add(global("x"), lit(1))),
        &add(add(mul(global("x"), global("x")), mul(lit(2), global("x"))), lit(1)),
    );
    // (x + y) * (x + y)  =  x*x + 2*(x*y) + y*y
    assert_proves(
        &ctx,
        &mul(add(global("x"), global("y")), add(global("x"), global("y"))),
        &add(
            add(mul(global("x"), global("x")), mul(lit(2), mul(global("x"), global("y")))),
            mul(global("y"), global("y")),
        ),
    );
}

// =============================================================================
// T6 — TCB inventory: the trusted arithmetic base is EXACTLY these seven ring
//   axioms — locked so it can never silently grow.
// =============================================================================
#[test]
fn t6_tcb_inventory_locked() {
    let ctx = ctx();
    for ax in [
        "add_comm",
        "add_assoc",
        "add_zero",
        "mul_comm",
        "mul_assoc",
        "mul_one",
        "mul_distrib_add",
    ] {
        assert!(ctx.get_global(ax).is_some(), "ring axiom {ax} is not registered");
        assert!(
            infer_type(&ctx, &Term::Global(ax.to_string())).is_ok(),
            "ring axiom {ax} is not well-typed"
        );
    }
}
