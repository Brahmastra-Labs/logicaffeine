//! Extra locks for the two modules that decide whether an inductive declaration is SOUND:
//! `positivity.rs` (the paradox fence) and `inductive_compile.rs` (the nested-inductive
//! front-end). Everything here is a neighbour the conformance corpus does not name — the
//! refusals that must survive each new acceptance, and the acceptances that must survive
//! each new refusal.
//!
//! Two themes:
//!
//! 1. **`Term::Const` is a REFERENCE, not an inert leaf.** A universe-polymorphic reference
//!    `Bad.{u}` names a declaration exactly as `Term::Global("Bad")` does. Positivity used to
//!    skip `Const` on the assumption that such a reference could never mention the inductive
//!    being declared — true only while `Const` cannot name an inductive, which is precisely
//!    what universe-polymorphic inductives undo. The lock below is DIFFERENTIAL: every
//!    constructor shape is checked in both spellings and the two verdicts must agree, so the
//!    fence cannot regrow a blind spot on one side.
//!
//! 2. **The specialisable container fragment.** Nesting compiles a container away into a
//!    mutual sibling; the fragment is now any fully-applied, non-indexed container of any
//!    arity whose fields are parameters, the spine, or values that do not touch a nested
//!    parameter. Each acceptance here is paired with the neighbouring refusal that a weaker
//!    implementation would also accept.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    check_positivity_for_test, derive_recursor, double_check, infer_type, is_subtype, normalize,
    Context, DoubleCheck, KernelError, MutualInductive, NestedDecl, Term, Universe,
};

// =============================================================================
// PER-FILE HELPERS (house style: every kernel test declares its own)
// =============================================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
/// A universe-polymorphic reference — the spelling positivity used to ignore.
fn k(n: &str) -> Term {
    Term::Const {
        name: n.to_string(),
        levels: Vec::new(),
    }
}
fn k_at(n: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: n.to_string(),
        levels,
    }
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn ty1() -> Term {
    Term::Sort(Universe::Type(1))
}
fn nat() -> Term {
    g("Nat")
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn same_type(ctx: &Context, a: &Term, b: &Term) -> bool {
    is_subtype(ctx, a, b) && is_subtype(ctx, b, a)
}

// =============================================================================
// 1 · `Term::Const` IS A REFERENCE — the differential positivity oracle
// =============================================================================

/// Every constructor shape, spelled once with `Term::Global("Bad")` and once with
/// `Term::Const { name: "Bad", … }`. `label`, the builder, and whether strict positivity
/// must ACCEPT it.
fn positivity_shapes(spell: &dyn Fn() -> Term) -> Vec<(&'static str, Term, bool)> {
    let bad = spell;
    vec![
        ("Bad", bad(), true),
        ("Nat → Bad", arrow(nat(), bad()), true),
        ("Bad → Bad", arrow(bad(), bad()), true),
        ("(Nat → Bad) → Bad", arrow(arrow(nat(), bad()), bad()), true),
        (
            "((Nat → Nat) → Bad) → Bad",
            arrow(arrow(arrow(nat(), nat()), bad()), bad()),
            true,
        ),
        ("Π(x:Bad). Bad", pi("x", bad(), bad()), true),
        ("Nat → Bad → Bad", arrow(nat(), arrow(bad(), bad())), true),
        // …and the paradoxes one arrow away.
        (
            "(Bad → False) → Bad",
            arrow(arrow(bad(), g("False")), bad()),
            false,
        ),
        (
            "(Bad → Nat) → Bad",
            arrow(arrow(bad(), nat()), bad()),
            false,
        ),
        (
            "((Bad → Nat) → Nat) → Bad",
            arrow(arrow(arrow(bad(), nat()), nat()), bad()),
            false,
        ),
        // The λ-domain arm of the checker reaches `occurs_in` too.
        (
            "λ(_:(Bad → Nat)). Bad",
            lam("_", arrow(bad(), nat()), bad()),
            false,
        ),
        // The let arm ζ-exposes the value first, as Lean's `whnf`-based positivity does, so
        // polarity is judged on the unfolded shape: a let that merely NAMES the inductive is
        // strictly positive, and a let hiding a negative domain is refused.
        (
            "let t : Type 0 := Bad in Bad",
            Term::Let {
                name: "t".to_string(),
                ty: Box::new(ty0()),
                value: Box::new(bad()),
                body: Box::new(bad()),
            },
            true,
        ),
        (
            "let t : Type 0 := Bad → Nat in t → Bad",
            Term::Let {
                name: "t".to_string(),
                ty: Box::new(ty0()),
                value: Box::new(arrow(bad(), nat())),
                body: Box::new(arrow(Term::Var("t".to_string()), bad())),
            },
            false,
        ),
    ]
}

#[test]
fn positivity_reads_a_const_reference_exactly_as_it_reads_a_global() {
    // A universe-polymorphic reference NAMES a declaration. If the paradox fence skips that
    // spelling, `Bad.{u} := mk : (Bad.{u} → False) → Bad.{u}` walks straight through it and
    // `False` becomes provable; if it skips it in the other direction — recognising the
    // occurrence but not that it is RECURSIVE — the strictly-positive `(Nat → Bad.{u}) → Bad.{u}`
    // is refused for no reason. Only checking both spellings against each other catches both.
    let mono = positivity_shapes(&|| g("Bad"));
    let poly = positivity_shapes(&|| k("Bad"));
    let poly_at_u = positivity_shapes(&|| k_at("Bad", vec![Universe::Var("u".to_string())]));

    for (((label, mono_ty, accept), (_, poly_ty, _)), (_, poly_u_ty, _)) in
        mono.iter().zip(poly.iter()).zip(poly_at_u.iter())
    {
        let mono_ok = check_positivity_for_test("Bad", "mk", mono_ty).is_ok();
        let poly_ok = check_positivity_for_test("Bad", "mk", poly_ty).is_ok();
        let poly_u_ok = check_positivity_for_test("Bad", "mk", poly_u_ty).is_ok();
        assert_eq!(
            mono_ok,
            *accept,
            "the monomorphic spelling of `{label}` must be {}",
            if *accept { "accepted" } else { "refused" }
        );
        assert_eq!(
            poly_ok, mono_ok,
            "`{label}` written with `Term::Const` must get the SAME verdict as the \
             `Term::Global` spelling — a `Const` reference names the inductive too"
        );
        assert_eq!(
            poly_u_ok, mono_ok,
            "`{label}` at explicit levels `.{{u}}` must get the same verdict — universe \
             arguments change nothing about WHICH declaration is named"
        );
    }
}

#[test]
fn a_const_written_paradox_is_refused_and_leaves_no_trace() {
    // The end-to-end path, not just the standalone predicate: `add_constructor_checked`
    // must refuse, and refuse transactionally.
    for (label, ty) in [
        (
            "Const in the domain",
            arrow(arrow(k("Bad"), g("False")), g("Bad")),
        ),
        (
            "Const in the result",
            arrow(arrow(g("Bad"), g("False")), k("Bad")),
        ),
        (
            "Const on both sides",
            arrow(arrow(k("Bad"), g("False")), k("Bad")),
        ),
        (
            "Const at explicit levels",
            arrow(
                arrow(
                    k_at("Bad", vec![Universe::Var("u".to_string())]),
                    g("False"),
                ),
                k_at("Bad", vec![Universe::Var("u".to_string())]),
            ),
        ),
    ] {
        let mut ctx = std_ctx();
        ctx.add_inductive("Bad", ty0());
        assert!(
            matches!(
                ctx.add_constructor_checked("bad_mk", "Bad", ty.clone()),
                Err(KernelError::PositivityViolation { .. })
            ),
            "{label}: a negative occurrence is a PositivityViolation however it is spelled"
        );
        assert!(
            ctx.get_global("bad_mk").is_none() && ctx.get_constructors("Bad").is_empty(),
            "{label}: a refused constructor must leave no trace"
        );
    }
}

#[test]
fn a_const_sibling_in_a_domain_is_a_cross_block_paradox() {
    // The mutual fence reads the whole block, so a SIBLING spelled as a `Const` must be as
    // visible as one spelled as a `Global`.
    let mut ctx = std_ctx();
    let paradox = vec![
        MutualInductive {
            name: "Ping".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![(
                "pmk".to_string(),
                arrow(arrow(k("Pong"), g("False")), g("Ping")),
            )],
        },
        MutualInductive {
            name: "Pong".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![("qmk".to_string(), arrow(g("Ping"), g("Pong")))],
        },
    ];
    assert!(
        matches!(
            ctx.add_mutual_inductives(&paradox),
            Err(KernelError::PositivityViolation { .. })
        ),
        "`Pong.{{}}` in a domain is the same cross-block paradox as `Pong`"
    );
    assert!(
        !ctx.is_inductive("Ping")
            && !ctx.is_inductive("Pong")
            && ctx.get_global("pmk").is_none()
            && ctx.get_global("qmk").is_none(),
        "and the refusal is transactional"
    );

    // Flipping the occurrence out of the domain makes the same block legal. The fence must
    // not become over-eager in the process — a `Const` sibling to the RIGHT of every arrow
    // is a strictly-positive RECURSIVE occurrence, which only holds if the recursive-argument
    // test reads a `Const` head too. Half a fix (seeing the occurrence but not that it is
    // recursive) refuses every one of these.
    let block = ["PingOk", "PongOk"];
    for (ctor, ty) in [
        ("pok", arrow(k("PongOk"), g("PingOk"))),
        ("pstop", g("PingOk")),
        ("qok", arrow(arrow(nat(), k("PingOk")), g("PongOk"))),
        ("qstop", g("PongOk")),
        (
            "q_telescope",
            arrow(pi("y", nat(), k("PingOk")), g("PongOk")),
        ),
    ] {
        assert!(
            logicaffeine_kernel::positivity::check_positivity_mutual(&block, ctor, &ty).is_ok(),
            "`{ctor} : {ty}` places the sibling only in codomain positions — strictly positive"
        );
    }

    // End to end, the same block must not be turned away BY THE FENCE. It is still turned
    // away today, one layer further on: nothing yet resolves a `Term::Const` to an inductive,
    // so the universe check cannot type the sibling reference. That is the neighbouring gap
    // this lock deliberately does not claim — what it does claim is that the refusal is never
    // a PositivityViolation.
    let mut ok = std_ctx();
    let outcome = ok.add_mutual_inductives(&[
        MutualInductive {
            name: "PingOk".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![
                ("pok".to_string(), arrow(k("PongOk"), g("PingOk"))),
                ("pstop".to_string(), g("PingOk")),
            ],
        },
        MutualInductive {
            name: "PongOk".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![
                (
                    "qok".to_string(),
                    arrow(arrow(nat(), k("PingOk")), g("PongOk")),
                ),
                ("qstop".to_string(), g("PongOk")),
            ],
        },
    ]);
    assert!(
        !matches!(outcome, Err(KernelError::PositivityViolation { .. })),
        "a strictly-positive `Const` sibling must never be a positivity violation, got \
         {outcome:?}"
    );
}

// =============================================================================
// 2 · THE SPECIALISABLE CONTAINER FRAGMENT
// =============================================================================

/// `TPair : Type → Type → Type` with the single constructor `tpair : Π(A).Π(B). A → B → TPair A B`.
fn register_tpair(ctx: &mut Context) {
    ctx.add_inductive("TPair", pi("A", ty0(), pi("B", ty0(), ty0())));
    ctx.add_constructor(
        "tpair",
        "TPair",
        pi(
            "A",
            ty0(),
            pi(
                "B",
                ty0(),
                arrow(v("A"), arrow(v("B"), apps(g("TPair"), &[v("A"), v("B")]))),
            ),
        ),
    );
}

/// A BINARY container with a base case, so its specialisation has closed inhabitants:
/// `bnil : Π(A).Π(B). BList A B`, `bcons : Π(A).Π(B). A → B → BList A B → BList A B`.
fn register_blist(ctx: &mut Context) {
    ctx.add_inductive("BList", pi("A", ty0(), pi("B", ty0(), ty0())));
    let blist_ab = apps(g("BList"), &[v("A"), v("B")]);
    ctx.add_constructor(
        "bnil",
        "BList",
        pi("A", ty0(), pi("B", ty0(), blist_ab.clone())),
    );
    ctx.add_constructor(
        "bcons",
        "BList",
        pi(
            "A",
            ty0(),
            pi(
                "B",
                ty0(),
                arrow(
                    v("A"),
                    arrow(v("B"), arrow(blist_ab.clone(), blist_ab.clone())),
                ),
            ),
        ),
    );
}

/// A unary container carrying an EXTRA field that is neither the element nor the spine:
/// `lcons : Π(A). Text → A → Labelled A → Labelled A`.
fn register_labelled(ctx: &mut Context) {
    ctx.add_inductive("Labelled", pi("A", ty0(), ty0()));
    ctx.add_constructor(
        "lnil",
        "Labelled",
        pi("A", ty0(), app(g("Labelled"), v("A"))),
    );
    ctx.add_constructor(
        "lcons",
        "Labelled",
        pi(
            "A",
            ty0(),
            arrow(
                g("Text"),
                arrow(
                    v("A"),
                    arrow(app(g("Labelled"), v("A")), app(g("Labelled"), v("A"))),
                ),
            ),
        ),
    );
}

/// The sibling constructor types of `sibling`, by name.
fn sibling_ctors(ctx: &Context, sibling: &str) -> Vec<(String, Term)> {
    ctx.get_constructors(sibling)
        .iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect()
}

#[test]
fn a_binary_container_specialises_the_parameter_the_element_actually_occupies() {
    // THE POSITIONAL TEST. A specialiser that assumed "the element is parameter 0" — which
    // is exactly what the unary-only version assumed — produces a type that still REGISTERS
    // and still type-checks; only the field ORDER of the mirrored constructor catches it.
    let mut second = std_ctx();
    register_tpair(&mut second);
    let info = second
        .add_nested_inductive(&NestedDecl {
            name: "PTree".to_string(),
            sort: ty0(),
            constructors: vec![(
                "pnode".to_string(),
                arrow(apps(g("TPair"), &[nat(), g("PTree")]), g("PTree")),
            )],
        })
        .expect("`TPair Nat PTree` is inside the fragment");
    assert_eq!(info.siblings.len(), 1, "one container level, one sibling");
    let sib = info.siblings[0].clone();
    let ctors = sibling_ctors(&second, &sib);
    assert_eq!(
        ctors.len(),
        1,
        "`TPair` has one constructor, so the sibling mirrors one"
    );
    assert!(
        same_type(
            &second,
            &ctors[0].1,
            &arrow(nat(), arrow(g("PTree"), g(&sib)))
        ),
        "the element sits in TPair's SECOND parameter, so the mirrored fields are \
         `Nat → PTree → {sib}`, got {}",
        ctors[0].1
    );

    // …and the mirror image: the element in the FIRST parameter.
    let mut first = std_ctx();
    register_tpair(&mut first);
    let info = first
        .add_nested_inductive(&NestedDecl {
            name: "QTree".to_string(),
            sort: ty0(),
            constructors: vec![(
                "qnode".to_string(),
                arrow(apps(g("TPair"), &[g("QTree"), nat()]), g("QTree")),
            )],
        })
        .expect("`TPair QTree Nat` is inside the fragment");
    let sib = info.siblings[0].clone();
    let ctors = sibling_ctors(&first, &sib);
    assert!(
        same_type(
            &first,
            &ctors[0].1,
            &arrow(g("QTree"), arrow(nat(), g(&sib)))
        ),
        "the element sits in TPair's FIRST parameter, so the mirrored fields are \
         `QTree → Nat → {sib}`, got {}",
        ctors[0].1
    );

    // The isos relate the sibling to the GENERIC container at the right arguments, and both
    // kernels certify them — the compiler is untrusted, so this is where soundness lives.
    let iso = &info.isos[0];
    let to = infer_type(&first, &g(&iso.to_generic)).expect("the to-iso is registered");
    let from = infer_type(&first, &g(&iso.from_generic)).expect("the from-iso is registered");
    let generic = apps(g("TPair"), &[g("QTree"), nat()]);
    assert!(
        same_type(&first, &to, &arrow(g(&sib), generic.clone())),
        "to_generic : {sib} → TPair QTree Nat, got {to}"
    );
    assert!(
        same_type(&first, &from, &arrow(generic, g(&sib))),
        "from_generic : TPair QTree Nat → {sib}, got {from}"
    );
    for name in [&iso.to_generic, &iso.from_generic] {
        match double_check(&first, &g(name)) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify iso {name}, got {other:?}"),
        }
    }
}

#[test]
fn a_container_field_that_is_neither_element_nor_spine_is_carried_through() {
    let mut ctx = std_ctx();
    register_labelled(&mut ctx);
    let info = ctx
        .add_nested_inductive(&NestedDecl {
            name: "LTree".to_string(),
            sort: ty0(),
            constructors: vec![(
                "ltree_node".to_string(),
                arrow(app(g("Labelled"), g("LTree")), g("LTree")),
            )],
        })
        .expect("a container with an extra carried field is inside the fragment");
    let sib = info.siblings[0].clone();
    let ctors = sibling_ctors(&ctx, &sib);
    assert_eq!(
        ctors.len(),
        2,
        "both of `Labelled`'s constructors are mirrored"
    );
    assert!(
        same_type(&ctx, &ctors[0].1, &g(&sib)),
        "the nil case mirrors to the bare sibling, got {}",
        ctors[0].1
    );
    assert!(
        same_type(
            &ctx,
            &ctors[1].1,
            &arrow(g("Text"), arrow(g("LTree"), arrow(g(&sib), g(&sib))))
        ),
        "the `Text` label survives UNSPECIALISED, the element becomes `LTree` and the spine \
         becomes the sibling: `Text → LTree → {sib} → {sib}`, got {}",
        ctors[1].1
    );
    let iso = &info.isos[0];
    for name in [&iso.to_generic, &iso.from_generic] {
        match double_check(&ctx, &g(name)) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify iso {name}, got {other:?}"),
        }
    }
    // The payoff: the nested type recurses THROUGH the container.
    let (_, rec_term) = derive_recursor(&ctx, "LTree").expect("LTree's recursor derives");
    match double_check(&ctx, &rec_term) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify LTree_rec, got {other:?}"),
    }
}

#[test]
fn the_isos_of_a_generalised_container_round_trip_by_computation() {
    // Registering is not computing. Build closed values on both sides of a BINARY container
    // whose fields exercise all three kinds at once — an inert parameter (`Nat`), a nesting
    // parameter (`BTree`) and the spine — and reduce the round trips.
    let mut ctx = std_ctx();
    register_blist(&mut ctx);
    let info = ctx
        .add_nested_inductive(&NestedDecl {
            name: "BTree".to_string(),
            sort: ty0(),
            constructors: vec![(
                "bnode".to_string(),
                arrow(apps(g("BList"), &[nat(), g("BTree")]), g("BTree")),
            )],
        })
        .expect("`BList Nat BTree` is inside the fragment");
    let sib = info.siblings[0].clone();
    let iso = &info.isos[0];
    let to = g(&iso.to_generic);
    let from = g(&iso.from_generic);

    let ctors = sibling_ctors(&ctx, &sib);
    let sib_nil = g(&ctors[0].0);
    let sib_cons = g(&ctors[1].0);

    // leaf : BTree = bnode (empty specialised list)
    let leaf = app(g("bnode"), sib_nil.clone());
    // spec : sib = bcons' Zero leaf nil'
    let spec = apps(sib_cons, &[g("Zero"), leaf.clone(), sib_nil.clone()]);
    assert_eq!(
        normalize(&ctx, &app(from.clone(), app(to.clone(), spec.clone()))),
        normalize(&ctx, &spec),
        "from_generic (to_generic spec) = spec"
    );

    // gen : BList Nat BTree = bcons Nat BTree Zero leaf (bnil Nat BTree)
    let gen = apps(
        g("bcons"),
        &[
            nat(),
            g("BTree"),
            g("Zero"),
            leaf,
            apps(g("bnil"), &[nat(), g("BTree")]),
        ],
    );
    assert_eq!(
        normalize(&ctx, &app(to, app(from, gen.clone()))),
        normalize(&ctx, &gen),
        "to_generic (from_generic gen) = gen"
    );
}

#[test]
fn two_nestings_of_one_container_get_distinct_siblings() {
    // `TPair Nat T` and `TPair Text T` share a container and an element; only the inert
    // argument separates them. A specialiser that keyed its sibling on the container (or on
    // the element alone) would silently give the second nesting the FIRST one's type.
    let mut ctx = std_ctx();
    register_tpair(&mut ctx);
    let info = ctx
        .add_nested_inductive(&NestedDecl {
            name: "MTree".to_string(),
            sort: ty0(),
            constructors: vec![
                (
                    "m_nat".to_string(),
                    arrow(apps(g("TPair"), &[nat(), g("MTree")]), g("MTree")),
                ),
                (
                    "m_text".to_string(),
                    arrow(apps(g("TPair"), &[g("Text"), g("MTree")]), g("MTree")),
                ),
            ],
        })
        .expect("two different nestings of one container compile");
    assert_eq!(
        info.siblings.len(),
        2,
        "two distinct nestings, two distinct siblings"
    );
    assert_ne!(
        info.siblings[0], info.siblings[1],
        "the sibling names must differ"
    );

    let nat_sib = &info.siblings[0];
    let text_sib = &info.siblings[1];
    assert!(
        same_type(
            &ctx,
            &sibling_ctors(&ctx, nat_sib)[0].1,
            &arrow(nat(), arrow(g("MTree"), g(nat_sib)))
        ),
        "the first sibling carries a `Nat`"
    );
    assert!(
        same_type(
            &ctx,
            &sibling_ctors(&ctx, text_sib)[0].1,
            &arrow(g("Text"), arrow(g("MTree"), g(text_sib)))
        ),
        "the second carries a `Text` — not a second copy of the first"
    );
    assert!(
        same_type(
            &ctx,
            &infer_type(&ctx, &g("m_nat")).expect("m_nat registers"),
            &arrow(g(nat_sib), g("MTree"))
        ),
        "and each constructor takes ITS OWN sibling"
    );
    assert!(
        same_type(
            &ctx,
            &infer_type(&ctx, &g("m_text")).expect("m_text registers"),
            &arrow(g(text_sib), g("MTree"))
        ),
        "and each constructor takes ITS OWN sibling"
    );
}

#[test]
fn two_containers_in_one_constructor_specialise_in_field_order() {
    // A constructor may nest through more than one container. The siblings come out in the
    // order the FIELDS mention them, and each field keeps its own — swapping them is
    // type-visible only because the two containers differ.
    let mut ctx = std_ctx();
    register_tpair(&mut ctx);
    let info = ctx
        .add_nested_inductive(&NestedDecl {
            name: "ZTree".to_string(),
            sort: ty0(),
            constructors: vec![(
                "znode".to_string(),
                arrow(
                    app(g("TList"), g("ZTree")),
                    arrow(apps(g("TPair"), &[nat(), g("ZTree")]), g("ZTree")),
                ),
            )],
        })
        .expect("two containers in one constructor compile");
    assert_eq!(info.siblings.len(), 2, "one sibling per container level");
    let (list_sib, pair_sib) = (&info.siblings[0], &info.siblings[1]);
    assert!(
        list_sib.ends_with("TList") && pair_sib.ends_with("TPair"),
        "the `TList` field comes first in the source, so its sibling comes first: {:?}",
        info.siblings
    );
    assert!(
        same_type(
            &ctx,
            &infer_type(&ctx, &g("znode")).expect("znode registers"),
            &arrow(g(list_sib), arrow(g(pair_sib), g("ZTree")))
        ),
        "znode : {list_sib} → {pair_sib} → ZTree"
    );
    for iso in &info.isos {
        for name in [&iso.to_generic, &iso.from_generic] {
            match double_check(&ctx, &g(name)) {
                DoubleCheck::Agreed => {}
                other => panic!("both kernels must certify iso {name}, got {other:?}"),
            }
        }
    }
}

// --- the refusals that must survive every one of those acceptances -------------------

/// Register a container and try to nest `name` through it, returning the outcome together
/// with a proof that a refusal left the context untouched.
fn nest_through(ctx: &mut Context, name: &str, field: Term) -> Result<(), String> {
    let decl = NestedDecl {
        name: name.to_string(),
        sort: ty0(),
        constructors: vec![(format!("{name}_mk"), arrow(field, g(name)))],
    };
    match ctx.add_nested_inductive(&decl) {
        Ok(info) => Err(format!("accepted, siblings {:?}", info.siblings)),
        Err(e) => {
            assert!(
                !ctx.is_inductive(name)
                    && ctx.get_global(&format!("{name}_mk")).is_none()
                    && ctx.mutual_block_of(name).is_none(),
                "a refused nesting of '{name}' must register no member, no sibling and no \
                 iso — half an emitted block is worse than none ({e:?})"
            );
            Ok(())
        }
    }
}

#[test]
fn a_container_field_using_the_element_compoundly_nests_through_both_containers() {
    // `Boxed A := bmk : TList A → Boxed A`. Nesting `BoxTree` through `Boxed` puts it inside
    // `TList` as well, and — as Lean 4.14 does for `inductive BoxTree | mk : Boxed BoxTree →
    // BoxTree`, whose `BoxTree.rec` has one motive per auxiliary (run on this box) — both
    // containers are specialised into siblings, with isos both kernels certify. The refusals that
    // must survive this acceptance (a container negative in its parameter, a sort-valued field,
    // a permuted result) are the neighbouring tests, unchanged.
    let mut ctx = std_ctx();
    ctx.add_inductive("Boxed", pi("A", ty0(), ty0()));
    ctx.add_constructor(
        "bmk",
        "Boxed",
        pi(
            "A",
            ty0(),
            arrow(app(g("TList"), v("A")), app(g("Boxed"), v("A"))),
        ),
    );
    let decl = NestedDecl {
        name: "BoxTree".to_string(),
        sort: ty0(),
        constructors: vec![(
            "BoxTree_mk".to_string(),
            arrow(app(g("Boxed"), g("BoxTree")), g("BoxTree")),
        )],
    };
    let info = ctx
        .add_nested_inductive(&decl)
        .expect("a compound use of the nested parameter nests through both containers, as in Lean");
    assert_eq!(
        info.siblings,
        vec!["BoxTree$TList".to_string(), "BoxTree$Boxed".to_string()],
        "one sibling per container level, the inner one first"
    );
    for iso in &info.isos {
        for name in [&iso.to_generic, &iso.from_generic] {
            match double_check(&ctx, &g(name)) {
                DoubleCheck::Agreed => {}
                other => panic!("both kernels must certify iso {name}, got {other:?}"),
            }
        }
    }
    assert!(
        ctx.is_inductive("BoxTree"),
        "the accepted declaration is registered"
    );
}

#[test]
fn a_container_negative_in_its_parameter_is_refused() {
    // THE SOUNDNESS NEIGHBOUR. `Weird A := wmk : (A → Nat) → Weird A` places its parameter in
    // a DOMAIN. Specialising it would put `WTree` in a domain of its own constructor — the
    // paradox the whole fence exists to stop.
    let mut ctx = std_ctx();
    ctx.add_inductive("Weird", pi("A", ty0(), ty0()));
    ctx.add_constructor(
        "wmk",
        "Weird",
        pi(
            "A",
            ty0(),
            arrow(arrow(v("A"), nat()), app(g("Weird"), v("A"))),
        ),
    );
    nest_through(&mut ctx, "WTree", app(g("Weird"), g("WTree")))
        .expect("nesting through a container that is negative in its parameter is refused");
}

#[test]
fn an_indexed_container_is_refused() {
    // `VecLike : Π(A:Type 0). Π(n:Nat). Type 0` with ONE parameter and one index. Fixing the
    // index as if it were a uniform parameter would mirror a constructor that does not exist
    // at that index.
    let mut ctx = std_ctx();
    ctx.add_indexed_inductive("VecLike", pi("A", ty0(), pi("n", nat(), ty0())), 1);
    ctx.add_constructor(
        "vnil",
        "VecLike",
        pi("A", ty0(), apps(g("VecLike"), &[v("A"), g("Zero")])),
    );
    nest_through(
        &mut ctx,
        "VTree",
        apps(g("VecLike"), &[g("VTree"), g("Zero")]),
    )
    .expect("an indexed family is not a specialisable container");
}

#[test]
fn a_container_whose_constructor_permutes_its_parameters_is_refused() {
    // `smk : Π(A).Π(B). A → B → Swap B A` — the field positions do NOT correspond to the
    // nesting's arguments, so mirroring them positionally would build a different type.
    let mut ctx = std_ctx();
    ctx.add_inductive("Swap", pi("A", ty0(), pi("B", ty0(), ty0())));
    ctx.add_constructor(
        "smk",
        "Swap",
        pi(
            "A",
            ty0(),
            pi(
                "B",
                ty0(),
                arrow(v("A"), arrow(v("B"), apps(g("Swap"), &[v("B"), v("A")]))),
            ),
        ),
    );
    nest_through(&mut ctx, "STree", apps(g("Swap"), &[nat(), g("STree")]))
        .expect("a permuted result type is outside the fragment");
}

#[test]
fn a_dependent_container_field_is_refused() {
    // `dmk : Π(A). Π(x:Nat). Eq Nat x x → A → Dep A` — the third field's type mentions the
    // second field's BINDER, which the non-dependent sibling constructor cannot express.
    let mut ctx = std_ctx();
    ctx.add_inductive("Dep", pi("A", ty0(), ty0()));
    ctx.add_constructor(
        "dmk",
        "Dep",
        pi(
            "A",
            ty0(),
            pi(
                "x",
                nat(),
                arrow(
                    apps(g("Eq"), &[nat(), v("x"), v("x")]),
                    arrow(v("A"), app(g("Dep"), v("A"))),
                ),
            ),
        ),
    );
    nest_through(&mut ctx, "DTree2", app(g("Dep"), g("DTree2")))
        .expect("a dependent container field is outside the fragment");
}

#[test]
fn a_container_that_shadows_a_parameter_name_is_refused() {
    // `shmk : Π(A).Π(A). A → Shadow A A`. Field positions are assigned by matching a field
    // against a parameter NAME, so under shadowing the `A` field would be read as the FIRST
    // parameter's slot while it actually carries the second's — a silently wrong type.
    let mut ctx = std_ctx();
    ctx.add_inductive("Shadow", pi("A", ty0(), pi("A", ty0(), ty0())));
    ctx.add_constructor(
        "shmk",
        "Shadow",
        pi(
            "A",
            ty0(),
            pi(
                "A",
                ty0(),
                arrow(v("A"), apps(g("Shadow"), &[v("A"), v("A")])),
            ),
        ),
    );
    nest_through(&mut ctx, "ShTree", apps(g("Shadow"), &[nat(), g("ShTree")]))
        .expect("a shadowed parameter telescope is outside the fragment");
}

#[test]
fn a_sort_valued_container_field_is_refused() {
    // `umk : Π(A). Type 0 → A → Univ A`. The kernel's ι-reduction counts LEADING sort-typed
    // binders as parameters, so a sort-valued value field would be skipped as if it were one
    // and the specialised match would drop it.
    let mut ctx = std_ctx();
    ctx.add_inductive("Univ", pi("A", ty0(), ty0()));
    ctx.add_constructor(
        "umk",
        "Univ",
        pi(
            "A",
            ty0(),
            arrow(ty0(), arrow(v("A"), app(g("Univ"), v("A")))),
        ),
    );
    nest_through(&mut ctx, "UTree", app(g("Univ"), g("UTree")))
        .expect("a sort-valued value field is outside the fragment");
}

#[test]
fn a_higher_universe_carried_field_is_refused_by_the_kernel() {
    // THE PROPERTY THE OLD BLANKET REFUSAL WAS STANDING IN FOR. Carrying an arbitrary field
    // through must not smuggle a `Type 1` value into a `Type 0` sibling — and it does not,
    // because `add_mutual_inductives` checks constructor universes in a scratch environment
    // BEFORE mutating, so the whole registration is refused and nothing is left behind.
    let mut ctx = std_ctx();
    ctx.add_declaration("BigThing", ty1());
    ctx.add_inductive("Big", pi("A", ty0(), ty0()));
    ctx.add_constructor("bignil", "Big", pi("A", ty0(), app(g("Big"), v("A"))));
    ctx.add_constructor(
        "bigmk",
        "Big",
        pi(
            "A",
            ty0(),
            arrow(g("BigThing"), arrow(v("A"), app(g("Big"), v("A")))),
        ),
    );
    nest_through(&mut ctx, "BigTree", app(g("Big"), g("BigTree")))
        .expect("a `Type 1` carried field cannot live in a `Type 0` sibling");
}

#[test]
fn a_const_headed_container_is_refused() {
    // The nesting detector must SEE a universe-polymorphic container rather than treating it
    // as an inert leaf and passing the field through unspecialised. Specialising one would
    // have to carry its levels into the sibling and the isos, so it is refused explicitly.
    let mut ctx = std_ctx();
    nest_through(&mut ctx, "KTree", app(k("TList"), g("KTree")))
        .expect("a `Const`-headed container is refused, not silently ignored");
}

#[test]
fn a_nested_constructor_must_land_on_the_bare_inductive() {
    // A nested inductive is emitted with `num_params: 0`, so a constructor returning anything
    // other than the inductive itself has no sibling to name.
    let mut ctx = std_ctx();
    let decl = NestedDecl {
        name: "PTree2".to_string(),
        sort: ty0(),
        constructors: vec![(
            "p2".to_string(),
            arrow(app(g("TList"), g("PTree2")), app(g("PTree2"), nat())),
        )],
    };
    assert!(
        ctx.add_nested_inductive(&decl).is_err(),
        "a constructor returning `PTree2 Nat` is not a declaration this path can emit"
    );
    assert!(
        !ctx.is_inductive("PTree2") && ctx.get_global("p2").is_none(),
        "and the refusal leaves nothing behind"
    );
}
