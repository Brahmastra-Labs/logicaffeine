//! Tier 11 — adversarial and soundness traps. The corpus's thesis, as a regression suite.
//!
//! One sentence carries this whole tier: **the LOGOS kernel accepting a term is not evidence
//! that the term is the one Lean meant.** LOGOS is cumulative where Lean is not, `Term::Hole`
//! matches anything, `add_inductive`/`add_constructor` are unchecked, and a `sorry` smuggled in
//! as an `add_declaration` is indistinguishable from a proof. Every case below is a different
//! route to that conclusion, written so the route stays open to inspection rather than rotting
//! shut.
//!
//! Nine of the tier's twenty-two cases are the confirmed LOGOS defects, reproduced in
//! `lean_conformance_defects.rs` / `lean_conformance_defects_proof.rs`; their verdicts in
//! `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/t11_adversarial.json` name those tests. This file covers the other
//! thirteen — T11-001, T11-004, T11-007, T11-008, T11-009, T11-011, T11-012, T11-014, T11-015,
//! T11-016, T11-017, T11-018, T11-019 — plus T11-022, which neither defect file reproduces:
//! its corpus evidence points at `logicaffeine_language::compile_theorem`, out of this crate's
//! reach, and the identical defect turns out to live one layer down at the kernel's own
//! literate surface, which is in reach.
//!
//! Three are `DIVERGE`/`Native` wins (cumulativity, the Peano bridge's fence, `reduceBool`'s
//! two-kernel evaluator hook) and assert OUR behaviour. The rest are `MUST`/`Gap` and are RED
//! on purpose: each names the work and fails until it is done.

use std::collections::BTreeSet;

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, defeq_for_test, derive_recursor, double_check, infer_type, is_subtype,
    normalize, recheck, termination, Context, DoubleCheck, KernelError, Literal, MutualInductive,
    NestedDecl, Term, Universe,
};

// ==== HELPERS (per-file, house style) ========================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(f, app)
}
fn lam(p: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn pi(p: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn nat(n: i64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_i64(n)))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// A deterministic size proxy: the printed term. Monotone in node count, and stable across
/// runs — unlike wall-clock, which the box's concurrent sessions make unusable as an assertion.
fn printed_size(t: &Term) -> usize {
    format!("{t}").len()
}

/// Every `Global` occurring in `t`, in no particular order.
fn globals_of(t: &Term, out: &mut BTreeSet<String>) {
    match t {
        Term::Global(n) => {
            out.insert(n.clone());
        }
        Term::Const { name, .. } => {
            out.insert(name.clone());
        }
        Term::App(f, x) => {
            globals_of(f, out);
            globals_of(x, out);
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            globals_of(param_type, out);
            globals_of(body_type, out);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            globals_of(param_type, out);
            globals_of(body, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            globals_of(discriminant, out);
            globals_of(motive, out);
            for c in cases {
                globals_of(c, out);
            }
        }
        Term::Fix { body, .. } => globals_of(body, out),
        Term::MutualFix { defs, .. } => {
            for (_, b) in defs {
                globals_of(b, out);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            globals_of(ty, out);
            globals_of(value, out);
            globals_of(body, out);
        }
        Term::Annotated { term, .. } => globals_of(term, out),
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// The TRANSITIVE axiom closure of `t` — Lean's `#print axioms`, hand-rolled from the public
/// `Context` API because the kernel exposes no such computation. Follows every definition's
/// body and type, so an axiom hiding one δ-step inside a lemma is still reported.
fn transitive_axioms(ctx: &Context, t: &Term) -> BTreeSet<String> {
    let declared: BTreeSet<String> = ctx
        .iter_declarations()
        .map(|(n, _)| n.to_string())
        .collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut axioms: BTreeSet<String> = BTreeSet::new();
    let mut frontier: Vec<Term> = vec![t.clone()];
    while let Some(term) = frontier.pop() {
        let mut names = BTreeSet::new();
        globals_of(&term, &mut names);
        for n in names {
            if !seen.insert(n.clone()) {
                continue;
            }
            if declared.contains(&n) {
                axioms.insert(n);
                continue;
            }
            if let Some(body) = ctx.get_definition_body(&n) {
                frontier.push(body.clone());
            }
            if let Some(ty) = ctx.get_definition_type(&n) {
                frontier.push(ty.clone());
            }
        }
    }
    axioms
}

/// An order-independent content digest of a context's axiom set — what `PRELUDE_VERSION`
/// would be if it were derived from `StandardLibrary::register` instead of hand-maintained.
fn axiom_set_digest(ctx: &Context) -> u64 {
    let mut rows: Vec<String> = ctx
        .iter_declarations()
        .map(|(n, ty)| format!("{n}:{ty}"))
        .collect();
    rows.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in rows.join("\u{1}").bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Peel `n` leading `Π`s off `t`, returning the `n`-th binder's type.
fn nth_binder_type(t: &Term, n: usize) -> Option<Term> {
    let mut cur = t;
    for _ in 0..n {
        match cur {
            Term::Pi { body_type, .. } => cur = body_type,
            _ => return None,
        }
    }
    match cur {
        Term::Pi { param_type, .. } => Some((**param_type).clone()),
        _ => None,
    }
}

/// The sort a Π-telescope finally lands in (`Π(a:A). Π(b:B). Sort u` → `u`).
fn telescope_target_sort(t: &Term) -> Option<Universe> {
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        cur = body_type;
    }
    match cur {
        Term::Sort(u) => Some(u.clone()),
        _ => None,
    }
}

// ==== T11-001 · CUMULATIVITY — DIVERGE, and the win is that the boundary holds ===============

/// **DIVERGE.** Lean 4 has no cumulativity at all: `Type 0` and `Type 1` are unrelated and
/// `Prop` is not `Type 0`. LOGOS admits `Prop ≤ Type i ≤ Type j` plus Π variance, which is
/// Coq-style and strictly more permissive — a deliberate, defensible choice, and the reason
/// the corpus's thesis holds: our acceptance set is strictly larger than Lean's, so our
/// kernel's "yes" is not a faithfulness oracle for a Lean translation.
///
/// What makes it defensible rather than sloppy is the BOUNDARY, which this test pins: the
/// leniency lives only at the top of a checking position and inside a `Π`. Every other shape
/// delegates to `def_eq`, so a type ARGUMENT is invariant and `TList Prop` is not a subtype of
/// `TList Nat`. Two opposite leniencies in one kernel would be the real trap; there is one.
#[test]
fn cumulativity_admits_the_sort_judgements_lean_refuses() {
    let ctx = std_ctx();

    // The three judgements Lean 4.14 refuses by name, and the one direction that must not hold.
    assert!(
        is_subtype(&ctx, &sort(Universe::Type(0)), &sort(Universe::Type(1))),
        "LOGOS is cumulative: `Type 0 ≤ Type 1` must hold (Lean refuses `def liftNat : Type 1 := Nat`)"
    );
    assert!(
        is_subtype(&ctx, &sort(Universe::Prop), &sort(Universe::Type(0))),
        "LOGOS is cumulative: `Prop ≤ Type 0` must hold (Lean refuses `def propAsType : Type 0 := True`)"
    );
    assert!(
        !is_subtype(&ctx, &sort(Universe::Type(1)), &sort(Universe::Type(0))),
        "cumulativity is an ORDER, not a collapse: `Type 1 ≤ Type 0` must stay false"
    );

    // Π variance — covariant codomain, contravariant domain. Lean refuses both.
    assert!(
        is_subtype(
            &ctx,
            &arrow(g("Nat"), sort(Universe::Type(0))),
            &arrow(g("Nat"), sort(Universe::Type(1)))
        ),
        "a Π is covariant in its codomain: `Nat → Type 0 ≤ Nat → Type 1`"
    );
    assert!(
        is_subtype(
            &ctx,
            &arrow(sort(Universe::Type(1)), g("Nat")),
            &arrow(sort(Universe::Type(0)), g("Nat"))
        ),
        "a Π is contravariant in its domain: `Type 1 → Nat ≤ Type 0 → Nat`"
    );

    // The ARGUMENT position is cumulative too, because `infer_type`'s `App` arm checks the
    // argument and `check_type` ends in `is_subtype`. Lean reports an application type
    // mismatch for `idT True`; we take it.
    let mut c = std_ctx();
    c.add_definition(
        "idT".to_string(),
        arrow(sort(Universe::Type(0)), sort(Universe::Type(0))),
        lam("a", sort(Universe::Type(0)), var("a")),
    );
    let applied = infer_type(&c, &app(g("idT"), g("True")));
    assert!(
        applied.is_ok(),
        "applying a `Type 0 → Type 0` function to a `Prop` is accepted by cumulativity: {applied:?}"
    );

    // THE BOUNDARY. Inside a type application every position is INVARIANT — the catch-all arm
    // delegates to definitional equality. A translator that assumes cumulativity is uniform
    // gets rejected here, and the tempting repair is a `Term::Hole` (see T11-004).
    assert!(
        !is_subtype(&c, &app(g("TList"), g("True")), &app(g("TList"), g("Nat"))),
        "cumulativity must NOT leak into an invariant position: `TList Prop` is not `TList (Type 0)`"
    );
    assert!(
        is_subtype(&c, &app(g("TList"), g("Nat")), &app(g("TList"), g("Nat"))),
        "the invariant position still admits genuine equality"
    );
}

// ==== T11-004 · `Term::Hole` MATCHES ANYTHING ===============================================

/// **MUST / Gap.** `types_equal` returns `true` before looking at either side when one is a
/// `Hole`, and `is_subtype` calls it FIRST — so one surviving hole turns the kernel's verdict
/// into an unconditional "yes" at that position. The one real fence is the second kernel,
/// which refuses a `Hole` outright; this test pins that fence and then names the work.
#[test]
fn a_hole_turns_the_type_check_into_a_no_op_at_its_position() {
    let ctx = std_ctx();

    // The fence that DOES exist: the independent de Bruijn re-checker will not certify a term
    // containing a `Hole`, so `double_check` never reports `Agreed` for one.
    let heterogeneous = apps(g("Eq"), &[Term::Hole, nat(3), g("true")]);
    let verdict = double_check(&ctx, &heterogeneous);
    assert!(
        !matches!(verdict, DoubleCheck::Agreed),
        "the second kernel must never corroborate a term carrying a `Hole`: {verdict:?}"
    );

    // A `Hole` also survives every reduction — it is never substituted into and `normalize`
    // returns it unchanged, so a hole emitted once stays in the term for the rest of its life.
    assert_eq!(
        normalize(&ctx, &Term::Hole),
        Term::Hole,
        "a `Hole` is its own normal form, so every later `def_eq` against it is vacuous"
    );

    // THE WORK. `is_subtype(Hole, T)` and `is_subtype(T, Hole)` hold for EVERY `T` — including
    // `False`, including two types from different inductive families.
    assert!(
        !is_subtype(&ctx, &Term::Hole, &g("False")),
        "a `Hole` must not be a subtype of `False`: the kernel's verdict at that position is \
         an unconditional yes, so accepting the term proves nothing"
    );
    assert!(
        !is_subtype(&ctx, &g("Nat"), &Term::Hole),
        "a `Hole` must not absorb `Nat` either — the wildcard is symmetric and leaks into \
         `is_subtype`'s fast path ahead of normalisation"
    );

    // And the consequence a translator actually hits: `Eq Hole a b` types for `a : Nat` and
    // `b : Bool`, which is heterogeneous equality in a kernel that deliberately has no `HEq`.
    let typed = infer_type(&ctx, &heterogeneous);
    assert!(
        typed.is_err(),
        "`Eq _ (3 : Nat) (true : Bool)` must be refused — a `Hole` in the TYPE argument turns \
         `Eq` into heterogeneous equality by accident: {typed:?}"
    );
}

// ==== T11-007 · SUBSTITUTION COPIES THE ARGUMENT PER BINDER =================================

/// **MUST / Gap.** Lean's `Expr` is a hash-consed DAG over de Bruijn indices, so instantiating
/// a telescope binder is cheap and sharing survives. LOGOS's `Term` is a named tree with no
/// sharing, so `infer_type`'s `App` arm substitutes a full CLONE of the argument at every
/// occurrence. The measurement below is deterministic (printed size, not wall-clock, because a
/// concurrent session on this box makes timing unusable as an assertion); the wall-clock
/// observed while writing it was 0.19 ms at n=8, 0.32 ms at n=32, 1.47 ms at n=64 and 2.45 ms
/// at n=128, i.e. superlinear in the telescope depth at a fixed argument size.
#[test]
fn substituting_one_argument_into_a_telescope_copies_it_per_binder() {
    // `f : Π(x:Nat). Π(y0 : Eq Nat x x). … Π(y_{n-1} : Eq Nat x x). Nat` — a DEPENDENT
    // telescope, so `substitute`'s non-occurrence fast path is defeated at every level, exactly
    // as a `variable`-heavy section or a bundled algebraic structure produces.
    let telescope = |n: usize| {
        let mut ty = g("Nat");
        for i in 0..n {
            ty = pi(
                &format!("y{i}"),
                apps(g("Eq"), &[g("Nat"), var("x"), var("x")]),
                ty,
            );
        }
        pi("x", g("Nat"), ty)
    };
    // The argument: a 40-deep `Succ` spine, so each copy is visible in the size.
    let mut argument = g("Zero");
    for _ in 0..40 {
        argument = app(g("Succ"), argument);
    }

    let mut measured = Vec::new();
    for n in [8usize, 32, 128] {
        let ty = telescope(n);
        let mut ctx = std_ctx();
        ctx.add_declaration("f", ty.clone());
        let started = std::time::Instant::now();
        let inferred = infer_type(&ctx, &app(g("f"), argument.clone()))
            .expect("applying `f` to a `Nat` must type-check");
        let elapsed = started.elapsed();
        let (ty_size, arg_size, out_size) = (
            printed_size(&ty),
            printed_size(&argument),
            printed_size(&inferred),
        );
        // Each `Eq Nat x x` mentions `x` twice, so a copying substitution leaves 2n clones.
        let copies = (out_size.saturating_sub(ty_size)) as f64 / arg_size as f64;
        measured.push(format!(
            "n={n}: telescope {ty_size}, argument {arg_size}, inferred {out_size} \
             (≈{copies:.1} copies of the argument, {elapsed:?})"
        ));
        if n == 128 {
            assert!(
                out_size <= 4 * (ty_size + arg_size),
                "substituting ONE argument into a {n}-binder dependent telescope must not \
                 materialise a copy of it per occurrence — Lean's DAG keeps one node and \
                 instantiates by de Bruijn index. Measured: {}",
                measured.join("; ")
            );
        }
    }
}

// ==== T11-008 · THE `Nat` LITERAL / PEANO BRIDGE FENCE =======================================

/// **MUST / Native.** `Literal::Nat(BigInt)` is definitionally `Succ^n Zero`, bridged in
/// `extract_constructor` and in `def_eq` in BOTH kernels. A past audit found a negative
/// `Literal::Nat` walked `normalize`/`def_eq` toward −∞; the fix rejects a negative count at
/// typing and clamps the peel at `Zero`. This test is that fence — the kind that rots silently
/// because nothing else in the suite manufactures a negative `Nat`.
#[test]
fn the_nat_literal_peano_bridge_holds_and_refuses_a_negative_count() {
    let ctx = std_ctx();

    // The bridge itself, both directions and both ends.
    let succ4 = app(
        g("Succ"),
        app(g("Succ"), app(g("Succ"), app(g("Succ"), g("Zero")))),
    );
    assert!(
        defeq_for_test(&ctx, &nat(4), &succ4),
        "`Nat(4)` is definitionally `Succ⁴ Zero`"
    );
    assert!(
        defeq_for_test(&ctx, &succ4, &nat(4)),
        "and the bridge is symmetric on Peano terms"
    );
    assert!(
        defeq_for_test(&ctx, &nat(0), &g("Zero")),
        "`Nat(0)` is `Zero`"
    );
    assert!(
        defeq_for_test(&ctx, &nat(300), &app(g("Succ"), nat(299))),
        "a literal against a `Succ`-headed term peels one `Succ` per step"
    );
    assert!(
        !defeq_for_test(&ctx, &nat(300), &app(g("Succ"), nat(300))),
        "and the peel must still DECIDE — `300` is not `Succ 300`"
    );

    // THE FENCE, kernel one: a negative `Nat` literal has no Peano reading and is refused.
    let typed = infer_type(&ctx, &nat(-1));
    assert!(
        matches!(typed, Err(KernelError::CertificationError(_))),
        "a negative `Nat` literal must be refused at typing, not given a `Nat`: {typed:?}"
    );
    // THE FENCE, kernel two: the independent re-checker must agree, or the double-kernel
    // guarantee is worth nothing here — the bridge is implemented in both.
    assert!(
        recheck(&ctx, &nat(-1)).is_err(),
        "the second kernel must refuse a negative `Nat` literal too"
    );
    // THE FENCE, reduction: matching on one must TERMINATE at `Zero` rather than descend.
    let matched = Term::Match {
        discriminant: Box::new(nat(-5)),
        motive: Box::new(lam("_", g("Nat"), g("Nat"))),
        cases: vec![nat(0), lam("k", g("Nat"), nat(1))],
    };
    assert_eq!(
        normalize(&ctx, &matched),
        nat(0),
        "peeling a negative literal must collapse to `Zero`, not walk toward −∞"
    );
}

// ==== T11-009 · PROOF IRRELEVANCE IS A ONE-WAY LAST-RESORT RULE ==============================

/// **MUST / Gap.** Lean's kernel decides proof irrelevance as a primitive conversion rule.
/// LOGOS's is the FALLBACK of `def_eq_normalized`, reached only after congruence fails, and it
/// is invoked in ONE direction only: `proof_irrelevant(ctx, a, b)` infers `a`'s type, requires
/// that to be a `Prop`, and only then looks at `b`. So `def_eq` — whose own doc comment says
/// "(symmetric)" — is NOT symmetric whenever the two sides differ in whether their types infer.
/// A `Term::Hole`-typed hypothesis is the shortest witness, and it is a shape a translator
/// reaches for precisely when a type would not line up.
#[test]
fn proof_irrelevance_is_a_one_way_last_resort_rule() {
    let mut ctx = std_ctx();
    ctx.add_declaration("Q", sort(Universe::Prop));
    ctx.add_declaration("q1", g("Q"));
    ctx.add_declaration("q2", g("Q"));

    // The rule does fire on the common path: two distinct proofs of one proposition…
    assert!(
        defeq_for_test(&ctx, &g("q1"), &g("q2")),
        "two proofs of one `Prop` must be definitionally equal"
    );
    // …including when they are buried inside a data constructor's proof-typed field.
    ctx.add_inductive("Bounded", sort(Universe::Type(0)));
    ctx.add_constructor(
        "bounded",
        "Bounded",
        pi("v", g("Nat"), pi("pf", g("Q"), g("Bounded"))),
    );
    assert!(
        defeq_for_test(
            &ctx,
            &apps(g("bounded"), &[nat(3), g("q1")]),
            &apps(g("bounded"), &[nat(3), g("q2")])
        ),
        "two structures differing only in a proof field must be definitionally equal"
    );

    // THE WORK. `qbad`'s declared type is a `Hole` — the shape a translator emits for a
    // position it could not resolve. Checking gets MORE permissive (a `Hole` matches anything)
    // and conversion gets MORE restrictive (`infer_type` cannot infer a bare `Hole`, so the
    // irrelevance fallback bails) — at the same time, on the same term, in opposite directions.
    ctx.add_declaration("qbad", Term::Hole);
    let forward = defeq_for_test(&ctx, &g("q1"), &g("qbad"));
    let backward = defeq_for_test(&ctx, &g("qbad"), &g("q1"));
    assert_eq!(
        forward, backward,
        "definitional equality must be SYMMETRIC — `def_eq(q1, qbad)` is {forward} and \
         `def_eq(qbad, q1)` is {backward}, because proof irrelevance is tried in one direction \
         only and bails on the side whose type will not infer"
    );
}

// ==== T11-011 · A MUTUAL BLOCK WITH UNEQUAL PARAMETER COUNTS ================================

/// **MUST / Gap.** The parameter-count constraint lives in `derive_mutual_recursor`, which is
/// a SEPARATE, OPTIONAL step the caller initiates. `add_mutual_inductives` checks positivity
/// over the whole block and the CIC universe constraint — and never reconciles the members'
/// parameter counts. So the block registers, its terms type-check, `Match` (which is primitive
/// in LOGOS) works, and the induction principle simply does not exist — discovered one proof at
/// a time, far from the declaration.
#[test]
fn a_mutual_block_with_unequal_parameter_counts_registers_anyway() {
    let mut ctx = std_ctx();
    let nat_to_prop = pi("n", g("Nat"), sort(Universe::Prop));
    // `EvenT`/`OddT` over `Nat`, recorded with a 1/0 parameter split — what a default
    // `add_inductive` lowering produces when one member's arity is read as parameters and the
    // other's as indices. Both are `Prop`, so the impredicative exemption waves the universe
    // check through and nothing else looks at the split.
    let block = vec![
        MutualInductive {
            name: "EvenT".to_string(),
            sort: nat_to_prop.clone(),
            num_params: 1,
            constructors: vec![
                ("even_zero".to_string(), app(g("EvenT"), g("Zero"))),
                (
                    "even_succ".to_string(),
                    pi(
                        "n",
                        g("Nat"),
                        arrow(
                            app(g("OddT"), var("n")),
                            app(g("EvenT"), app(g("Succ"), var("n"))),
                        ),
                    ),
                ),
            ],
        },
        MutualInductive {
            name: "OddT".to_string(),
            sort: nat_to_prop,
            num_params: 0,
            constructors: vec![(
                "odd_succ".to_string(),
                pi(
                    "n",
                    g("Nat"),
                    arrow(
                        app(g("EvenT"), var("n")),
                        app(g("OddT"), app(g("Succ"), var("n"))),
                    ),
                ),
            )],
        },
    ];

    let registered = ctx.add_mutual_inductives(&block);
    if registered.is_ok() {
        // Today's damage, recorded: the block is fully in the context, its terms type-check,
        // and the refusal arrives only when someone asks for the eliminator.
        assert!(
            ctx.is_inductive("EvenT") && ctx.is_inductive("OddT"),
            "an accepted block is a registered block"
        );
        let two_is_even = apps(
            g("even_succ"),
            &[nat(1), apps(g("odd_succ"), &[nat(0), g("even_zero")])],
        );
        assert!(
            infer_type(&ctx, &two_is_even).is_ok(),
            "and its terms type-check, so nothing signals the block is unusable"
        );
        let derived = derive_recursor(&ctx, "EvenT");
        assert!(
            matches!(derived, Err(KernelError::CertificationError(ref m))
                     if m.contains("disagree on parameter count")),
            "the refusal arrives here, at the eliminator, with the block already in context: \
             {derived:?}"
        );
    }
    assert!(
        registered.is_err(),
        "a mutual block whose members disagree on their parameter count has NO induction \
         principle, so it must be refused at declaration time — accepting it and failing later \
         in `derive_mutual_recursor` leaves a type in the context that can be matched on but \
         never inducted over: {registered:?}"
    );
}

// ==== T11-012 · NESTED INDUCTIVE OUTSIDE THE UNARY-CONTAINER FRAGMENT =======================

/// **MUST / Gap.** Lean compiles a nested inductive through ANY previously-declared container.
/// `compile_nested` handles only PURE UNARY containers — every constructor argument must be the
/// bare element or a recursive occurrence — and only at `Type 0`. `Tree` inside `List` works;
/// inside `Prod`, an indexed container, or a container with one extra field it does not. The
/// refusal is loud and correct given what the compiler can build; the gap is the fragment.
#[test]
fn nesting_through_a_container_with_an_extra_field_is_refused() {
    // The control: nesting through the prelude's pure unary `TList` compiles, emitting the
    // specialised sibling and its conversion isos.
    let mut ok_ctx = std_ctx();
    let rose = NestedDecl {
        name: "RoseList".to_string(),
        sort: sort(Universe::Type(0)),
        constructors: vec![(
            "rose_node".to_string(),
            arrow(app(g("TList"), g("RoseList")), g("RoseList")),
        )],
    };
    let compiled = ok_ctx
        .add_nested_inductive(&rose)
        .expect("nesting through `TList` is inside the fragment");
    assert_eq!(
        compiled.siblings,
        vec!["RoseList$TList".to_string()],
        "the container is compiled away into a specialised mutual sibling"
    );
    assert!(
        !compiled.isos.is_empty(),
        "and the conversion isos that relate the sibling to the generic container are emitted"
    );

    // A LABELLED list: `lcons : Π(A). Text → A → Labelled A → Labelled A`. The `Text` field is
    // neither the element nor a recursive occurrence, so the container is outside the fragment.
    let mut ctx = std_ctx();
    ctx.add_inductive(
        "Labelled",
        pi("A", sort(Universe::Type(0)), sort(Universe::Type(0))),
    );
    ctx.add_constructor(
        "lnil",
        "Labelled",
        pi("A", sort(Universe::Type(0)), app(g("Labelled"), var("A"))),
    );
    ctx.add_constructor(
        "lcons",
        "Labelled",
        pi(
            "A",
            sort(Universe::Type(0)),
            arrow(
                g("Text"),
                arrow(
                    var("A"),
                    arrow(app(g("Labelled"), var("A")), app(g("Labelled"), var("A"))),
                ),
            ),
        ),
    );
    let labelled_tree = NestedDecl {
        name: "LTree".to_string(),
        sort: sort(Universe::Type(0)),
        constructors: vec![(
            "ltree_node".to_string(),
            arrow(app(g("Labelled"), g("LTree")), g("LTree")),
        )],
    };
    let out = ctx.add_nested_inductive(&labelled_tree);
    assert!(
        out.is_ok(),
        "Lean nests an inductive through ANY declared container; LOGOS's fragment is pure \
         unary containers only, so a labelled list is refused and the tempting workaround — \
         hand-rolling the mutual block with the UNCHECKED `add_inductive`/`add_constructor` — \
         produces a different type with no isos and no relationship to the container: {out:?}"
    );
}

// ==== T11-014 · `sorry` SMUGGLED IN AS AN AXIOM ==============================================

/// **MUST.** `sorryAx : Π(α : Sort u). α` is one `add_declaration` away, and a typing verdict
/// cannot see it: both kernels certify the type of a `sorry`-backed proof, in Lean as here. What
/// separates a `sorry` from a proof is the AUDIT — Lean's `#print axioms`, a separate walk of the
/// dependency graph printing `depends on axioms: [sorryAx]`. The taint must be TRANSITIVE — the
/// realistic shape is a complete proof citing a lemma that is not — and LOGOS's walk,
/// `axiom_dependencies` (the `Print Axioms` door), must name it through the lemma, and name it
/// exactly: a clean proof beside it reports no `sorryAx`.
#[test]
fn a_sorry_axiom_is_certified_by_both_kernels_with_no_audit_trail() {
    let mut ctx = std_ctx();
    ctx.add_declaration("sorryAx", pi("a", sort(Universe::Type(0)), var("a")));

    // `unproved : 0 = 1 := sorryAx (0 = 1)` — Lean would paint this yellow and list `sorryAx`.
    let false_claim = apps(g("Eq"), &[g("Nat"), nat(0), nat(1)]);
    ctx.add_definition(
        "unproved".to_string(),
        false_claim.clone(),
        app(g("sorryAx"), false_claim.clone()),
    );
    // The realistic shape: a declaration whose OWN proof is complete and which depends on one
    // that is not. Nothing in its own term mentions the axiom.
    let dependent = apps(g("Eq_sym"), &[g("Nat"), nat(0), nat(1), g("unproved")]);

    let inferred = infer_type(&ctx, &dependent);
    assert!(
        inferred.is_ok(),
        "the smuggled axiom makes a FALSE statement type-check — that is the damage: {inferred:?}"
    );
    let corroborated = double_check(&ctx, &dependent);
    assert!(
        matches!(corroborated, DoubleCheck::Agreed),
        "and the second, independent kernel corroborates it — the de Bruijn double-check is a \
         guarantee about TYPES, and gives zero protection against an enlarged axiom set: \
         {corroborated:?}"
    );

    // The answer LOGOS owes, computed here from the public `Context` API to show it is not a
    // research problem: the closure follows `unproved`'s body one δ-step and finds the axiom.
    let owed = transitive_axioms(&ctx, &dependent);
    assert!(
        owed.contains("sorryAx"),
        "the transitive axiom closure of a `sorry`-backed proof must name `sorryAx`; a one-hop \
         scan of the proof term sees only the lemma: {owed:?}"
    );
    let trusted: BTreeSet<String> = std_ctx()
        .iter_declarations()
        .map(|(n, _)| n.to_string())
        .collect();
    let smuggled: Vec<&String> = owed.iter().filter(|a| !trusted.contains(*a)).collect();

    // THE AUDIT. Certifying `1 = 0` from a smuggled `sorryAx` must REPORT the axioms it rests
    // on, transitively, through the lemma its own term never mentions.
    let reported = axiom_dependencies(&ctx, &dependent);
    assert!(
        !smuggled.is_empty() && smuggled.iter().all(|a| reported.axioms.contains(*a)),
        "the audit of a `sorry`-backed proof must name every smuggled axiom {smuggled:?}; \
         without a transitive `#print axioms` a `sorry` is indistinguishable from a proof: \
         {reported:?}"
    );
    assert!(
        reported.unresolved.is_empty(),
        "nothing in the proof is left unresolved: {reported:?}"
    );
    // …and the taint is exact, not context-wide: a clean proof beside it rests on no `sorryAx`.
    let clean = apps(g("refl"), &[g("Nat"), nat(0)]);
    assert!(
        !axiom_dependencies(&ctx, &clean).axioms.contains("sorryAx"),
        "a clean proof in the same context must not be reported as resting on `sorryAx`"
    );
}

// ==== T11-015 · TRUSTED-BASE DRIFT ==========================================================

/// **MUST / Gap.** `certificate::recheck` rebuilds the context from `StandardLibrary::register`
/// — the right design — but a proof minted against a PATCHED prelude fails re-check with a bare
/// `UnboundVariable`. That error reads like a name-resolution bug and invites exactly the wrong
/// repair: add the axiom to the prelude. Once patched, the hand-maintained `PRELUDE_VERSION`
/// string still matches, so every certificate thereafter describes an axiom set that is not the
/// one it was checked against. The content digest below shows the drift IS detectable — the
/// kernel simply does not compute one.
#[test]
fn trusted_base_drift_surfaces_only_as_an_unbound_variable() {
    // The patch re-signs Lean's `ofReduceBool` axiom. (An axiom something in the prelude is
    // checked against, like `Classical_choice`, cannot be re-signed at all: a registered name
    // never changes meaning under its dependents.)
    let trusted = std_ctx();
    let mut patched = std_ctx();
    patched.add_declaration(
        "ofReduceBool",
        pi("A", sort(Universe::Type(0)), arrow(var("A"), var("A"))),
    );
    assert!(
        patched.refused_declarations().is_empty(),
        "{:?}",
        patched.refused_declarations()
    );
    let mut relied_on = std_ctx();
    relied_on.add_declaration(
        "Classical_choice",
        pi("A", sort(Universe::Type(0)), arrow(var("A"), var("A"))),
    );
    assert_eq!(
        axiom_set_digest(&relied_on),
        axiom_set_digest(&trusted),
        "re-signing an axiom the prelude depends on is refused and changes nothing"
    );

    // The two axiom sets are genuinely different, and a content hash sees it immediately.
    assert_ne!(
        axiom_set_digest(&trusted),
        axiom_set_digest(&patched),
        "a content digest of `StandardLibrary::register`'s output distinguishes the two axiom \
         sets — the information a certificate would need to pin its base is available"
    );

    let cited = apps(g("ofReduceBool"), &[g("Nat"), nat(1)]);
    assert!(
        infer_type(&patched, &cited).is_ok(),
        "the proof type-checks against the enlarged base it was minted against"
    );

    // THE WORK. Re-checked against the trusted base, the diagnosis must be about the TRUSTED
    // BASE, not about a name.
    let refused = infer_type(&trusted, &cited)
        .expect_err("the trusted base's `ofReduceBool` is not the patched one");
    assert!(
        !matches!(refused, KernelError::UnboundVariable(_)),
        "re-checking a proof minted against a patched prelude must say the trusted base was \
         enlarged and NAME the axiom. An `UnboundVariable` reads like a name-resolution bug and \
         invites patching `StandardLibrary::register` — after which the version guard passes \
         while describing an axiom set that is not the one the proof used: {refused:?}"
    );
}

// ==== T11-016 · `Int` AXIOMATISED VERSUS DERIVED ============================================

/// **MUST / Gap.** Lean defines `Int` as `ofNat | negSucc` and PROVES its ring and order laws;
/// its whole kernel rests on three axioms. LOGOS's `Int` is an inductive with ZERO constructors
/// — an opaque carrier that cannot be matched on or inducted over — and its seventeen ring and
/// order facts are `add_declaration`s: trusted assumptions with no bodies. That is our largest
/// trusted-base liability, and a judge will find it.
#[test]
fn int_is_an_opaque_carrier_whose_ring_laws_are_assumptions() {
    let ctx = std_ctx();

    // The measurement, reported whichever way the assertions go.
    let int_facts = [
        "add_comm",
        "add_assoc",
        "add_zero",
        "mul_comm",
        "mul_assoc",
        "mul_one",
        "mul_zero",
        "mul_distrib_add",
        "sub_elim",
        "le_refl",
        "le_trans",
        "le_add_mono",
        "le_mul_nonneg",
        "le_sub",
        "lt_succ_le",
        "lt_add1_le",
        "le_total",
    ];
    let assumed: Vec<&str> = int_facts
        .iter()
        .copied()
        .filter(|n| !ctx.is_definition(n) && ctx.get_global(n).is_some())
        .collect();
    let total_axioms = ctx.iter_declarations().count();

    // Lean's `Int` has constructors, so `Int.rec`, `Int.induction_on` and case analysis on
    // `ofNat`/`negSucc` all exist. Ours has none, so every Lean proof by induction on an `Int`
    // has no target at all.
    let ctors = ctx.get_constructors("Int").len();
    assert!(
        ctors >= 2,
        "Lean's `Int` is `ofNat : Nat → Int | negSucc : Nat → Int`; LOGOS's has {ctors} \
         constructors, so `Match` over an `Int` is inexpressible and `derive_recursor` has \
         nothing to derive — every proof by induction or case analysis on an `Int` is a \
         rejection, not a degradation"
    );
    assert!(
        ctx.is_definition("add_comm"),
        "Lean PROVES `Int.add_comm` from the constructor structure; LOGOS assumes it. \
         {} of the {} `Int` ring/order facts are `add_declaration`s with no body ({assumed:?}), \
         inside a trusted base of {total_axioms} declarations — against Lean's three kernel \
         axioms",
        assumed.len(),
        int_facts.len()
    );
}

// ==== T11-017 · `reduceBool` WIRES THE EVALUATOR INTO KERNEL REDUCTION ======================

/// **DIVERGE.** `reduceBool`/`ofReduceBool` are exactly Lean's `Lean.reduceBool` /
/// `Lean.ofReduceBool` pair, and the shape is faithful: a `Bool`-valued decision is run by a
/// trusted evaluator and `ofReduceBool` converts the answer into a proof. The CONTENT differs,
/// and ours is the smaller commitment: Lean's `native_decide` trusts the Lean compiler and its
/// runtime (`Lean.trustCompiler`), while ours trusts `crate::eval`, an in-repo CBV interpreter
/// that both kernels hook identically — so the de Bruijn double-check still covers every term
/// whose typing depends on it, which is not true of a compiled artefact. The honest hazard,
/// pinned below, is that the rule fires on ANY evaluable `Bool`, not only on a `decide` of the
/// proposition being proved — the same unconditional shape `Lean.ofReduceBool` has.
#[test]
fn reduce_bool_admits_the_evaluator_into_both_kernels_conversion() {
    let mut ctx = std_ctx();
    assert!(
        ctx.get_global("reduceBool").is_some() && ctx.get_global("ofReduceBool").is_some(),
        "the `native_decide` pair must be registered"
    );

    // The evaluator genuinely runs inside kernel reduction.
    assert_eq!(
        normalize(&ctx, &app(g("reduceBool"), g("true"))),
        g("true"),
        "`reduceBool t` is reduced by the CBV evaluator, not by δ/ι"
    );

    // The hazard, stated rather than hidden: the hook fires on any argument `eval_bool` can
    // evaluate — here an ordinary definition, with no `decide` anywhere in sight.
    ctx.add_definition("verdict".to_string(), g("Bool"), g("false"));
    assert_eq!(
        normalize(&ctx, &app(g("reduceBool"), g("verdict"))),
        g("false"),
        "nothing checks that `reduceBool`'s argument is the `decide` of the goal"
    );

    // And the discharge: `ofReduceBool verdict false (refl Bool false) : Eq Bool verdict false`,
    // a proof whose validity is the evaluator's answer.
    let by_evaluator = apps(
        g("ofReduceBool"),
        &[
            g("verdict"),
            g("false"),
            apps(g("refl"), &[g("Bool"), g("false")]),
        ],
    );
    let concluded = infer_type(&ctx, &by_evaluator).expect("the native discharge must type-check");
    assert!(
        is_subtype(
            &ctx,
            &concluded,
            &apps(g("Eq"), &[g("Bool"), g("verdict"), g("false")])
        ),
        "`ofReduceBool` turns the evaluator's `Bool` into a proof of the original proposition: \
         {concluded}"
    );

    // The part that makes this DIVERGE rather than a liability we hide: the hook is in BOTH
    // kernels, so the second kernel corroborates a term whose typing needed it.
    let needs_the_hook = apps(g("refl"), &[g("Bool"), g("false")]);
    let corroborated = double_check(&ctx, &needs_the_hook);
    assert!(
        matches!(corroborated, DoubleCheck::Agreed),
        "the re-checker's whnf hooks `reduceBool` too, so the two-kernel guarantee still \
         covers evaluator-discharged proofs: {corroborated:?}"
    );
}

// ==== T11-018 · THE GIMÉNEZ GUARD IS STRICTER THAN LEAN'S STRUCTURAL CHECKER ================

/// **MUST / Gap.** Ours is a syntactic Giménez guard: the recursive call's structural argument
/// must be HEADED by a match-bound smaller variable. Lean's structural elaborator zeta-reduces
/// a `let` and accepts `let rest := xs; f rest`; our `Let` arm deliberately does not mark the
/// bound value as smaller, so the same function is refused — and the refusal is the dangerous
/// part, because the workarounds (an accumulator, a fuel parameter) pass the guard and are
/// DIFFERENT functions with the same type.
///
/// The two fence assertions come first: a past audit found the guard skipped the match MOTIVE
/// and the λ/Π DOMAIN annotations in both kernels — a shared blind spot, since fixed. Those
/// rot silently, so they are pinned here.
#[test]
fn the_gimenez_guard_refuses_a_let_aliased_smaller_variable() {
    let ctx = std_ctx();
    let list = || app(g("TList"), g("Nat"));
    let unfix = |t: &Term| match t {
        Term::Fix { body, .. } => (**body).clone(),
        _ => unreachable!("built as a Fix"),
    };

    // THE FENCE, part one: a recursive occurrence hidden in the match MOTIVE must not evade the
    // guard. `f l` there is a call on the un-peeled structural parameter.
    let in_motive = Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam(
            "l",
            list(),
            Term::Match {
                discriminant: Box::new(var("l")),
                motive: Box::new(lam("_", list(), app(var("f"), var("l")))),
                cases: vec![g("Zero"), lam("h", g("Nat"), lam("t", list(), g("Zero")))],
            },
        )),
    };
    assert!(
        termination::check_termination(&ctx, "f", &unfix(&in_motive)).is_err(),
        "the match MOTIVE is an ordinary subterm and must be guarded — a recursive call hidden \
         there would otherwise evade the check"
    );

    // THE FENCE, part two: the same for a binder's DOMAIN annotation.
    let in_domain = Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam(
            "l",
            list(),
            Term::Match {
                discriminant: Box::new(var("l")),
                motive: Box::new(lam("_", list(), g("Nat"))),
                cases: vec![
                    g("Zero"),
                    lam(
                        "h",
                        g("Nat"),
                        lam("t", list(), lam("z", app(var("f"), var("l")), g("Zero"))),
                    ),
                ],
            },
        )),
    };
    assert!(
        termination::check_termination(&ctx, "f", &unfix(&in_domain)).is_err(),
        "a λ's DOMAIN annotation is in the current scope and must be guarded too"
    );

    // The control: recursing directly on the match-bound tail is accepted, so the refusal
    // below is specific rather than a blanket refusal of recursion.
    let direct = Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam(
            "l",
            list(),
            Term::Match {
                discriminant: Box::new(var("l")),
                motive: Box::new(lam("_", list(), g("Nat"))),
                cases: vec![
                    g("Zero"),
                    lam("h", g("Nat"), lam("t", list(), app(var("f"), var("t")))),
                ],
            },
        )),
    };
    assert!(
        termination::check_termination(&ctx, "f", &unfix(&direct)).is_ok(),
        "recursing on the match-bound tail is the guard's own admissible shape"
    );

    // THE WORK. Lean accepts `| x :: xs => let rest := xs; x + f rest` structurally. The alias
    // is the same value under a `Let` binder, and the guard's `smaller_than` set is populated
    // only by MATCH binders.
    let via_let = Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam(
            "l",
            list(),
            Term::Match {
                discriminant: Box::new(var("l")),
                motive: Box::new(lam("_", list(), g("Nat"))),
                cases: vec![
                    g("Zero"),
                    lam(
                        "h",
                        g("Nat"),
                        lam(
                            "t",
                            list(),
                            Term::Let {
                                name: "rest".to_string(),
                                ty: Box::new(list()),
                                value: Box::new(var("t")),
                                body: Box::new(app(var("f"), var("rest"))),
                            },
                        ),
                    ),
                ],
            },
        )),
    };
    let verdict = termination::check_termination(&ctx, "f", &unfix(&via_let));
    assert!(
        verdict.is_ok(),
        "a `let`-alias of a match-bound smaller variable is the same value and Lean accepts the \
         recursion structurally. Refusing it pushes a translator toward an accumulator or a \
         fuel parameter — both of which PASS the guard and are different functions with the \
         same type, which nothing in LOGOS records: {verdict:?}"
    );
}

// ==== T11-019 · A `Type 1` INDUCTIVE GETS A RECURSOR PINNED TO `Type 0` =====================

/// **MUST / Gap — and deliberately not `DIVERGE`.** LOGOS's constructor universe check decides
/// `field ≤ target` with the CUMULATIVE order; Lean's inductive constraint is on LEVELS, which
/// is the same relation, so the two agree on the declaration and the divergence the corpus
/// expected is not there (the control assertions below pin that agreement). What survives is a
/// plain capability gap: `derive_recursor` hard-codes a non-`Prop` inductive's motive to
/// `Sort(Type 0)`, so an inductive that passes the universe check AT `Type 1` gets an eliminator
/// that cannot reach its own level. Lean eliminates a `Type 1` inductive into `Type 1`. The
/// declaration is accepted and the type is not usable — which is a `MUST`, not a defensible
/// difference.
#[test]
fn a_type_one_inductive_gets_a_recursor_pinned_to_type_zero() {
    // The control, both ways: the universe check is real and agrees with Lean's judgement.
    let mut too_big = std_ctx();
    too_big.add_inductive("TooBig", sort(Universe::Type(0)));
    assert!(
        too_big
            .add_constructor_checked(
                "too_big_mk",
                "TooBig",
                arrow(sort(Universe::Type(0)), g("TooBig"))
            )
            .is_err(),
        "a `Type 0` inductive storing a `Type 0`-typed field is the Girard/Hurkens shape and \
         must be refused — Lean refuses it too"
    );
    let mut ctx = std_ctx();
    ctx.add_inductive("AtLevelOne", sort(Universe::Type(1)));
    assert!(
        ctx.add_constructor_checked(
            "at_level_one",
            "AtLevelOne",
            arrow(sort(Universe::Type(0)), g("AtLevelOne"))
        )
        .is_ok(),
        "declared one level up it is consistent, and both systems accept it"
    );

    // The eliminator LOGOS derives for it.
    let (recursor_ty, recursor_tm) =
        derive_recursor(&ctx, "AtLevelOne").expect("a well-formed inductive must have a recursor");
    let motive_ty = nth_binder_type(&recursor_ty, 0).expect("the recursor binds a motive first");
    let motive_target = telescope_target_sort(&motive_ty)
        .expect("the motive's type is a Π-telescope ending in a sort");

    // The consequence, made concrete: eliminating `AtLevelOne` into `Type` — recovering the
    // stored type — is exactly what the constructor's field invites, and it does not check.
    ctx.add_definition(
        "AtLevelOne_rec".to_string(),
        recursor_ty.clone(),
        recursor_tm,
    );
    let large_elimination = apps(
        g("AtLevelOne_rec"),
        &[
            lam("_", g("AtLevelOne"), sort(Universe::Type(0))),
            lam("t", sort(Universe::Type(0)), var("t")),
        ],
    );
    let eliminated = infer_type(&ctx, &large_elimination);

    assert_eq!(
        motive_target,
        Universe::Type(1),
        "an inductive declared at `Type 1` must get an eliminator whose motive can reach its \
         OWN level; `derive_recursor` pins the motive to {motive_target} for everything that is \
         not a non-subsingleton `Prop`, so recovering the type this inductive stores fails with \
         {eliminated:?} — the declaration is accepted and the type is not usable"
    );
}

// ==== T11-022 · ONLY THE FIRST `## Theorem` BLOCK IS PROVED ==================================

/// **MUST / Gap.** The corpus points at `logicaffeine_language::compile::compile_theorem`,
/// whose `find_map` keeps the FIRST `Stmt::Theorem` and silently drops the rest. The kernel's
/// own literate surface has the same shape one layer down: `Repl::execute` parses ONE command
/// out of its input, proves it, and returns `Ok` — having consumed a prefix and discarded
/// everything after it with no diagnostic and no count.
///
/// Worse than the corpus records: the discarded remainder may be FALSE. `5 < 2` by `lia` is
/// correctly refused when it is the only block, and is accepted by silence when it follows a
/// true one. So the soundness gate `literate_proof_soundness.rs` pins is bypassed by
/// concatenation, and a document reports success while containing a theorem the kernel would
/// have rejected. Written here rather than in `lean_conformance_defects.rs` because that file carries
/// no reproduction for this case and `compile_theorem` is out of the kernel crate's reach —
/// this is the same defect at the surface that IS in reach.
#[test]
fn a_document_with_two_theorem_blocks_proves_only_the_first() {
    use logicaffeine_kernel::interface::Repl;

    let first = "## Theorem: First\n    Statement: 48 equals (mul 6 8).\n    Proof: ring.";
    let truthful = "## Theorem: Second\n    Statement: 2 < 5.\n    Proof: lia.";
    let false_second = "## Theorem: Second\n    Statement: 5 < 2.\n    Proof: lia.";

    // The baseline each block must meet on its own — the true ones certify, the false one is
    // refused. This is the gate the concatenation below walks around.
    assert!(
        Repl::new().execute(first).is_ok(),
        "`48 = 6 * 8` by `ring` certifies alone"
    );
    assert!(
        Repl::new().execute(truthful).is_ok(),
        "`2 < 5` by `lia` certifies alone"
    );
    assert!(
        Repl::new().execute(false_second).is_err(),
        "`5 < 2` by `lia` must be refused alone — that refusal is the soundness gate"
    );

    // Two true theorems in one document: the second is parsed away and never registered, so a
    // caller who counted its inputs cannot tell from the result that one of them vanished.
    let mut repl = Repl::new();
    let document = repl.execute(&format!("{first}\n\n{truthful}"));
    if document.is_ok() {
        assert!(
            repl.execute("Check First.").is_ok(),
            "the FIRST block is the one that was proved"
        );
        assert!(
            repl.execute("Check Second.").is_err(),
            "and the second is discarded with no diagnostic and no count — the return value \
             carries no field a caller could check"
        );
    }

    // THE WORK. The discarded remainder may be a theorem the kernel would have REFUSED, and
    // the document still reports success.
    let smuggled = Repl::new().execute(&format!("{first}\n\n{false_second}"));
    assert!(
        smuggled.is_err(),
        "a document whose second `## Theorem` block is FALSE must not report success. \
         `5 < 2` by `lia` is refused when it stands alone and accepted by silence when it \
         follows a true theorem, so concatenation walks straight around the `concludes` gate: \
         {smuggled:?}"
    );
}
