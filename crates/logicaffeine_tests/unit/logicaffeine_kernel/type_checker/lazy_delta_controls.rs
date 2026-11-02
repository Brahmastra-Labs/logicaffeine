//! Lazy δ in the primary kernel's conversion: a wrapper chain is converted by unfolding the higher
//! side one step at a time, never by normalizing both sides; an unequal pair is still refused; and
//! the minimal originals of the W06 timeout groups import without one full normalization.
//!
//! The argument is a balanced tree over a free variable whose 2¹⁶ leaves are all different
//! (`pair x 0`, `pair x 1`, …), so no subterm is shared: it is larger than every bounded probe
//! that runs before lazy δ (256 steps for the head probes, 2¹⁶ distinct nodes for the store-backed
//! β/ζ and β/δ probes). Each control first asserts that every one of those probes leaves the pair
//! undecided, so the count it then locks is the lazy loop's alone.
use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn pair(a: Term, b: Term) -> Term {
    app(app(g("pair"), a), b)
}

/// A complete binary `pair` tree of `levels` levels over `variable`, whose leaves are
/// `pair variable 0`, `pair variable 1`, …: no two subtrees are equal.
fn tree(variable: &str, levels: u32) -> Term {
    fn grow(variable: &str, levels: u32, next: &mut u64) -> Term {
        if levels == 0 {
            *next += 1;
            return pair(
                v(variable),
                Term::Lit(Literal::Nat(crate::BigInt::from_u64(*next - 1))),
            );
        }
        let left = grow(variable, levels - 1, next);
        pair(left, grow(variable, levels - 1, next))
    }
    grow(variable, levels, &mut 0)
}

const LEVELS: u32 = 16;

/// `base n := pair n n` (height 1), `mid n := base n` (height 2), `top n := mid n` (height 3).
fn globals() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_declaration("pair", arrow(g("Nat"), arrow(g("Nat"), g("Nat"))));
    let unary = arrow(g("Nat"), g("Nat"));
    c.declare(
        "base",
        unary.clone(),
        lam("n", g("Nat"), pair(v("n"), v("n"))),
    )
    .unwrap();
    c.declare(
        "mid",
        unary.clone(),
        lam("n", g("Nat"), app(g("base"), v("n"))),
    )
    .unwrap();
    c.declare("top", unary, lam("n", g("Nat"), app(g("mid"), v("n"))))
        .unwrap();
    c
}

fn environment() -> Context {
    globals().extend("x", g("Nat")).extend("y", g("Nat"))
}

fn full_normalizations(c: &Context, a: &Term, b: &Term) -> (bool, usize) {
    CONVERSION_FULL_NORMALIZATIONS.with(|count| count.set(0));
    let equal = def_eq(c, a, b);
    (
        equal,
        CONVERSION_FULL_NORMALIZATIONS.with(|count| count.get()),
    )
}

/// Every rule `def_eq_uncached` tries before lazy δ leaves `a ≡ b` undecided.
fn out_of_reach_of_the_earlier_probes(c: &Context, a: &Term, b: &Term) {
    assert!(!alpha_eq(a, b));
    assert_eq!(rigid_conversion(c, a, b), None, "rigid heads");
    assert_eq!(
        crate::reduction::natural_literal_conversion(c, a, b, 1 << 18),
        None,
        "literal bridge"
    );
    assert!(
        !crate::reduction::constructor_equivalent(c, a, b, 1 << 26),
        "certified constructor evaluation"
    );
    assert!(
        !crate::term_store::beta_zeta_equivalent(c, a, b, 1 << 16),
        "store-backed β/ζ probe"
    );
    assert!(
        !crate::term_store::beta_delta_equivalent(c, a, b, 1 << 16),
        "store-backed β/δ probe"
    );
    assert!(!conversion_by_congruence(c, a, b), "congruence");
    assert_ne!(
        bounded_head_conversion(c, a, b),
        Some(true),
        "bounded head probe"
    );
    assert!(!proof_irrelevant(c, a, b), "proof irrelevance");
    assert!(!conversion_by_exposed_heads(c, a, b), "exposed heads");
    assert!(
        !crate::reduction::constructors_disagree(c, a, b, 1 << 26),
        "constructor disagreement"
    );
}

/// Run on a stack sized for the big trees: the kernel sizes its own stack from the input's depth.
fn on_a_large_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(1 << 28)
            .spawn_scoped(scope, f)
            .unwrap()
            .join()
            .unwrap()
    })
}

#[test]
fn the_heights_are_computed_by_the_kernel_from_the_bodies() {
    let c = environment();
    assert_eq!(
        (
            c.definition_height("base"),
            c.definition_height("mid"),
            c.definition_height("top")
        ),
        (1, 2, 3)
    );
    assert_eq!(c.definition_height("pair"), 0, "an axiom has no height");
    assert_eq!(
        c.definition_height("Succ"),
        0,
        "a constructor has no height"
    );
}

#[test]
fn a_wrapper_chain_converts_by_unfolding_the_higher_side_without_full_normalization() {
    on_a_large_stack(|| {
        let c = environment();
        let t = tree("x", LEVELS);
        assert!(crate::term_store::term_depth(&t) < 64);
        for (wrapper, unfoldings) in [("top", 3), ("mid", 2), ("base", 1)] {
            let (a, b) = (app(g(wrapper), t.clone()), pair(t.clone(), t.clone()));
            out_of_reach_of_the_earlier_probes(&c, &a, &b);
            assert_eq!(
                lazy_delta::lazy_delta_conversion(&c, &a, &b),
                Some(true),
                "{wrapper} t ≡ pair t t"
            );
            let (equal, normalizations) = full_normalizations(&c, &a, &b);
            assert!(equal, "{wrapper} t ≡ pair t t");
            assert_eq!(normalizations, 0, "lazy δ unfolds `{wrapper}` {unfoldings} time(s) and meets `pair t t` syntactically");
        }
    });
}

/// `n` applications of `Succ` around `inner`.
fn successors(n: usize, inner: Term) -> Term {
    (0..n).fold(inner, |t, _| app(g("Succ"), t))
}

const DEEP: usize = 300;

/// Two definitions of the same height, `deep_left n := Succ³⁰⁰ (base n)` and
/// `deep_right n := Succ³⁰⁰ ((λm. base m) n)`: convertible, not syntactically equal, and deeper
/// than the bounded head probe's 256 steps, so unfolding either yields a reduct far deeper than
/// the stack a shallow conversion starts on supports, and only the loop can meet them.
fn deep_environment() -> Context {
    let mut c = globals();
    let unary = arrow(g("Nat"), g("Nat"));
    let left = lam("n", g("Nat"), successors(DEEP, app(g("base"), v("n"))));
    let right = lam(
        "n",
        g("Nat"),
        successors(
            DEEP,
            app(lam("m", g("Nat"), app(g("base"), v("m"))), v("n")),
        ),
    );
    c.declare("deep_left", unary.clone(), left).unwrap();
    c.declare("deep_right", unary, right).unwrap();
    c.extend("x", g("Nat")).extend("y", g("Nat"))
}

/// The deep pair's premise, locked on its own: every probe before the loop leaves it undecided, so
/// the test below cannot become vacuous through a change of its input.
#[test]
fn the_deep_pair_is_out_of_reach_of_every_probe_before_the_loop() {
    on_a_large_stack(|| {
        let c = deep_environment();
        let t = tree("x", LEVELS);
        out_of_reach_of_the_earlier_probes(
            &c,
            &app(g("deep_left"), t.clone()),
            &app(g("deep_right"), t),
        );
    });
}

#[test]
fn a_reduct_deeper_than_the_stack_continues_on_a_larger_stack_instead_of_giving_up() {
    on_a_large_stack(|| {
        let c = deep_environment();
        assert_eq!(
            c.definition_height("deep_left"),
            c.definition_height("deep_right")
        );
        let t = tree("x", LEVELS);
        let (a, b) = (
            app(g("deep_left"), t.clone()),
            app(g("deep_right"), t.clone()),
        );
        assert!(
            DEEP > crate::term_store::term_depth(&a),
            "the reduct is far deeper than either input"
        );
        out_of_reach_of_the_earlier_probes(&c, &a, &b);
        assert_eq!(lazy_delta::lazy_delta_conversion(&c, &a, &b), Some(true));
        let (equal, normalizations) = full_normalizations(&c, &a, &b);
        assert!(equal, "deep_left t ≡ deep_right t");
        assert_eq!(
            normalizations, 0,
            "both sides unfold to the same deep reduct on a grown stack"
        );
        let (equal, _) = full_normalizations(&c, &a, &app(g("deep_right"), tree("y", LEVELS)));
        assert!(!equal, "a different argument is still refused");
    });
}

/// Links in the nesting control's chain: each nests one comparison, far past the 32 rungs of the
/// unsized 2 MiB thread the control runs on, with a term depth inside the kernel's door.
const LINKS: u64 = 400;

/// `link (… (link (tip leaf) 0) …) (depth − 1)` over the inductive `Chain` (`tip : Nat → Chain`,
/// `link : Chain → Nat → Chain`): a constructor spine `depth` applications deep.
fn chain(leaf: Term, depth: u64) -> Term {
    (0..depth).fold(app(g("tip"), leaf), |t, i| app(app(g("link"), t), lit(i)))
}

/// The comparisons the loop nests inside one another (argument by argument) take stack of their
/// own. On a thread whose stack the kernel never sized (this test's), a comparison nested far
/// deeper than that stack holds continues on a larger stack instead of overflowing it.
#[test]
fn a_comparison_nested_deeper_than_its_stack_continues_on_a_larger_stack() {
    let mut c = globals();
    c.add_inductive("Chain", Term::Sort(Universe::Type(0)));
    c.add_constructor("tip", "Chain", arrow(g("Nat"), g("Chain")));
    c.add_constructor(
        "link",
        "Chain",
        arrow(g("Chain"), arrow(g("Nat"), g("Chain"))),
    );
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    let c = c.extend("x", g("Nat")).extend("y", g("Nat"));
    let depth = LINKS;
    let a = chain(app(g("mid"), v("x")), depth);
    assert!(
        crate::term_store::term_depth(&a) <= crate::type_checker::MAX_TERM_DEPTH,
        "the input is one the kernel's term-depth door admits"
    );
    assert_eq!(
        lazy_delta::lazy_delta_conversion(&c, &a, &chain(pair(v("x"), v("x")), depth)),
        Some(true),
        "the leaves are convertible (`mid x ≡ pair x x`)"
    );
    assert_eq!(
        lazy_delta::lazy_delta_conversion(&c, &a, &chain(pair(v("x"), v("y")), depth)),
        Some(false),
        "the leaves differ"
    );
}

/// `tower := fix f. λn:Nat. match n with | Zero => Zero | Succ k => Succ (f k)`: `tower n` is the
/// numeral `n`, built one `Succ` per unrolling.
fn tower(argument: Term) -> Term {
    let body = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(lam("_", g("Nat"), g("Nat"))),
            cases: vec![
                g("Zero"),
                lam("k", g("Nat"), app(g("Succ"), app(v("f"), v("k")))),
            ],
        },
    );
    app(
        Term::Fix {
            name: "f".into(),
            body: Box::new(body),
        },
        argument,
    )
}

/// A conversion whose nesting is bounded by neither input's depth — each `Succ` a reduction exposes
/// is one more nested comparison — stops at the kernel's term-depth door as a recorded resource
/// stop ("not shown equal", which `exact` reports as a `ResourceLimit` decline), never by
/// overflowing a stack.
#[test]
fn a_conversion_nested_past_the_term_depth_door_is_a_resource_stop_never_a_crash() {
    let c = environment();
    let beyond = (crate::type_checker::MAX_TERM_DEPTH * 3) as u64;
    let (equal, stop) = crate::reduction::observe_resource_stops(|| {
        def_eq(&c, &tower(lit(beyond)), &tower(lit(beyond + 1)))
    });
    assert!(!equal);
    assert_eq!(
        stop,
        Some(crate::Unreduced::TooDeep),
        "the refusal is a resource stop, not a decided inequality"
    );
    let (equal, stop) = crate::reduction::observe_resource_stops(|| {
        def_eq(
            &c,
            &tower(lit(beyond)),
            &app(g("Succ"), tower(lit(beyond - 1))),
        )
    });
    assert!(
        equal,
        "a shallow equality between the same towers is still found"
    );
    assert_eq!(stop, None);
}

/// `match d with | Zero => x | Succ p => p` at `Nat`, stuck on the axiom application `d`.
fn stuck_match(discriminant: Term) -> Term {
    Term::Match {
        discriminant: Box::new(discriminant),
        motive: Box::new(lam("_", g("Nat"), g("Nat"))),
        cases: vec![v("x"), lam("p", g("Nat"), v("p"))],
    }
}

#[test]
fn two_stuck_matches_are_compared_by_congruence() {
    on_a_large_stack(|| {
        let c = environment();
        let t = tree("x", LEVELS);
        let id = |n: Term| app(g("base"), n);
        let (a, b) = (
            stuck_match(pair(t.clone(), id(t.clone()))),
            stuck_match(pair(t.clone(), pair(t.clone(), t.clone()))),
        );
        out_of_reach_of_the_earlier_probes(&c, &a, &b);
        assert_eq!(lazy_delta::lazy_delta_conversion(&c, &a, &b), Some(true));
        let (equal, normalizations) = full_normalizations(&c, &a, &b);
        assert!(
            equal,
            "the discriminants are convertible (`base t ≡ pair t t`)"
        );
        assert_eq!(
            normalizations, 0,
            "stuck `match`es are compared by congruence, not by normalizing both sides"
        );
        let different = stuck_match(pair(t.clone(), pair(t.clone(), g("Zero"))));
        let (equal, _) = full_normalizations(&c, &a, &different);
        assert!(!equal, "a different discriminant is still refused");
    });
}

#[test]
fn an_unequal_pair_is_still_refused() {
    on_a_large_stack(|| {
        let c = environment();
        let (x, y) = (tree("x", LEVELS), tree("y", LEVELS));
        let top = app(g("top"), x.clone());
        assert!(!def_eq(&c, &top, &pair(y.clone(), y)), "different leaves");
        assert!(
            !def_eq(&c, &top, &pair(x.clone(), g("Zero"))),
            "a different second component"
        );
        assert!(
            !def_eq(&c, &top, &g("Zero")),
            "an application of an axiom is not zero"
        );
        assert!(!def_eq(&c, &top, &x), "pair t t is not t");
    });
}

// ------------------------------------------------------------------------------------------------
// Phase 2 (approved 2026-09-24): Lean's closing `return false` — R1 rigid heads that differ, R2 one
// local variable over different arguments, R3 binders whose domains or bodies differ — asked only
// after every positive rule, and after unit η. Each refusal is decided without a full
// normalization; each Lean-valid lookalike (unit η, structure η, the literal bridges) stays equal.
// ------------------------------------------------------------------------------------------------

fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}

/// `Unit1` a fieldless singleton (`star1`); `Wrap` a structure whose one field is a `Unit1`;
/// locals `x y : Nat`, `f : Nat → Nat`, `p q : Unit1`, `w v : Wrap`.
fn closing_environment() -> Context {
    let mut c = globals();
    c.add_inductive("Unit1", Term::Sort(Universe::Type(0)));
    c.add_constructor("star1", "Unit1", g("Unit1"));
    c.add_structure("Wrap", &[], &[("u", g("Unit1"))]);
    c.declare(
        "ident",
        arrow(g("Nat"), g("Nat")),
        lam("n", g("Nat"), v("n")),
    )
    .unwrap();
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    c.extend("x", g("Nat"))
        .extend("y", g("Nat"))
        .extend("f", arrow(g("Nat"), g("Nat")))
        .extend("p", g("Unit1"))
        .extend("q", g("Unit1"))
        .extend("w", g("Wrap"))
        .extend("v", g("Wrap"))
}

/// The lazy loop's own answer, the conversion's verdict, and how many full normalizations it took.
fn closing(c: &Context, a: &Term, b: &Term) -> (Option<bool>, bool, usize) {
    let lazy = lazy_delta::lazy_delta_conversion(c, a, b);
    let (equal, normalizations) = full_normalizations(c, a, b);
    (lazy, equal, normalizations)
}

fn refused_lazily(c: &Context, a: &Term, b: &Term, why: &str) {
    assert_eq!(
        closing(c, a, b),
        (Some(false), false, 0),
        "{why}: {a} ≢ {b}"
    );
    assert_eq!(
        closing(c, b, a),
        (Some(false), false, 0),
        "{why}, the other way round: {b} ≢ {a}"
    );
}

fn equal_lazily(c: &Context, a: &Term, b: &Term, why: &str) {
    assert_eq!(closing(c, a, b), (Some(true), true, 0), "{why}: {a} ≡ {b}");
    assert_eq!(
        closing(c, b, a),
        (Some(true), true, 0),
        "{why}, the other way round: {b} ≡ {a}"
    );
}

#[test]
fn rigid_heads_that_can_never_meet_are_refused_without_full_normalization() {
    let c = closing_environment();
    refused_lazily(&c, &v("x"), &v("y"), "two different local variables");
    refused_lazily(&c, &v("x"), &lit(3), "a local variable and a literal");
    refused_lazily(
        &c,
        &g("Zero"),
        &app(g("Succ"), v("x")),
        "two different constructors",
    );
    refused_lazily(
        &c,
        &app(g("Succ"), v("x")),
        &v("x"),
        "a constructor and a local variable",
    );
    refused_lazily(&c, &g("Nat"), &g("Unit1"), "two different inductive types");
    refused_lazily(
        &c,
        &g("Nat"),
        &Term::Sort(Universe::Type(0)),
        "an inductive type and a sort",
    );
    refused_lazily(
        &c,
        &Term::Sort(Universe::Prop),
        &Term::Sort(Universe::Type(0)),
        "two sorts of different levels",
    );
    refused_lazily(
        &c,
        &g("Nat"),
        &arrow(g("Nat"), g("Nat")),
        "an inductive type and a Π",
    );
}

#[test]
fn one_local_function_over_different_arguments_is_refused_without_full_normalization() {
    let c = closing_environment();
    refused_lazily(
        &c,
        &app(v("f"), v("x")),
        &app(v("f"), v("y")),
        "f x against f y",
    );
    refused_lazily(
        &c,
        &app(v("f"), lit(0)),
        &app(v("f"), lit(1)),
        "f 0 against f 1",
    );
    equal_lazily(
        &c,
        &app(v("f"), app(app(g("pair"), v("x")), v("x"))),
        &app(v("f"), app(g("base"), v("x"))),
        "f applied to convertible arguments",
    );
}

#[test]
fn binders_whose_bodies_differ_are_refused_without_full_normalization() {
    let c = closing_environment();
    refused_lazily(
        &c,
        &lam("n", g("Nat"), app(g("Succ"), v("n"))),
        &lam("n", g("Nat"), v("n")),
        "λn. Succ n against λn. n",
    );
    refused_lazily(
        &c,
        &arrow(g("Nat"), g("Nat")),
        &arrow(g("Nat"), g("Unit1")),
        "Nat → Nat against Nat → Unit1",
    );
    refused_lazily(
        &c,
        &arrow(g("Unit1"), g("Nat")),
        &arrow(g("Nat"), g("Nat")),
        "different domains",
    );
    equal_lazily(
        &c,
        &lam("n", g("Nat"), app(g("base"), v("n"))),
        &lam("m", g("Nat"), app(app(g("pair"), v("m")), v("m"))),
        "binders with convertible bodies",
    );
}

#[test]
fn two_inhabitants_of_a_unit_type_are_equal_and_never_refused() {
    let c = closing_environment();
    assert!(c.unit_constructor("Unit1").is_some());
    equal_lazily(
        &c,
        &v("p"),
        &v("q"),
        "two local variables of a fieldless singleton (Lean: `x y : PUnit`)",
    );
    equal_lazily(
        &c,
        &v("p"),
        &g("star1"),
        "a local variable and the singleton's constructor",
    );
}

#[test]
fn a_one_field_structure_over_a_unit_type_is_decided_as_lean_decides_it() {
    // Lean 4.14 (checked): `x = y` for `x y : Wrap` is NOT `rfl` (no unit η on a structure with a
    // field, no structure η between two variables); `x.u = y.u` and `x = ⟨p⟩` are.
    let c = closing_environment();
    assert!(
        c.unit_constructor("Wrap").is_none(),
        "one field: not a unit type"
    );
    let info = c
        .struct_info("Wrap")
        .expect("Wrap is a registered structure")
        .clone();
    let field = |t: Term| app(g(&info.projections[0]), t);
    refused_lazily(
        &c,
        &v("w"),
        &v("v"),
        "two different variables of a one-field structure",
    );
    assert!(
        def_eq(&c, &field(v("w")), &field(v("v"))),
        "their fields are equal by unit η"
    );
    assert!(
        def_eq(&c, &v("w"), &app(g(&info.mk), v("p"))),
        "structure η, then unit η on the field"
    );
    assert!(
        def_eq(&c, &app(g(&info.mk), v("p")), &v("w")),
        "and the other way round"
    );
}

#[test]
fn literal_bridges_are_decided_by_the_bridge_and_never_refused_by_their_heads() {
    let c = closing_environment();
    let peano = |n: usize| successors(n, g("Zero"));
    for (literal, spelling) in [(lit(0), peano(0)), (lit(3), peano(3))] {
        assert!(def_eq(&c, &literal, &spelling), "{literal} ≡ {spelling}");
        assert!(def_eq(&c, &spelling, &literal), "{spelling} ≡ {literal}");
    }
    assert!(def_eq(&c, &lit(3), &app(g("Succ"), lit(2))), "3 ≡ Succ 2");
    assert!(!def_eq(&c, &lit(3), &app(g("Succ"), v("x"))), "3 ≢ Succ x");
    // The certified literal rule's `false` is definite: a closed natural computation that finished
    // at another value is refused without normalizing either side.
    refused_lazily(&c, &lit(4), &lit(5), "two different natural literals");
    refused_lazily(
        &c,
        &lit(3),
        &successors(4, g("Zero")),
        "a literal and a different finished numeral",
    );
    let sum = app(app(g("Nat_add"), lit(2)), lit(2));
    refused_lazily(&c, &sum, &lit(5), "2 + 2 against 5");
    equal_lazily(&c, &sum, &lit(4), "2 + 2 against 4");
    let int = |n: i64| Term::Lit(Literal::Int(n));
    assert!(
        def_eq(&c, &int(3), &app(g("Int_ofNat"), lit(3))),
        "an Int literal and its constructor spelling"
    );
    assert!(
        def_eq(&c, &int(-2), &app(g("Int_negSucc"), lit(1))),
        "a negative Int literal and its constructor spelling"
    );
}

// ------------------------------------------------------------------------------------------------
// R4 (approved 2026-09-24, its own step): two stuck `match`/fixpoint terms that can never move —
// each stuck, after weak-head reduction with δ, on a local variable — and that congruence cannot
// equate are refused, as Lean's closing `return false` refuses two stuck recursor applications.
// Unit η and proof irrelevance still equate what they equate; a match that computes is never
// treated as stuck.
// ------------------------------------------------------------------------------------------------

/// `(fix f. λn:Nat. match n with | Zero => Zero | Succ p => f p) arg`: stuck exactly when `arg`
/// is.
fn countdown(argument: Term) -> Term {
    let body = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(lam("_", g("Nat"), g("Nat"))),
            cases: vec![g("Zero"), lam("p", g("Nat"), app(v("f"), v("p")))],
        },
    );
    app(
        Term::Fix {
            name: "f".into(),
            body: Box::new(body),
        },
        argument,
    )
}

/// `match d with | Zero => p | Succ _ => q` at the unit type `Unit1`.
fn unit_match(discriminant: Term) -> Term {
    Term::Match {
        discriminant: Box::new(discriminant),
        motive: Box::new(lam("_", g("Nat"), g("Unit1"))),
        cases: vec![v("p"), lam("k", g("Nat"), v("q"))],
    }
}

#[test]
fn stuck_matches_on_different_locals_are_refused_without_full_normalization() {
    let c = closing_environment();
    refused_lazily(
        &c,
        &stuck_match(v("x")),
        &stuck_match(v("y")),
        "matches on two different local variables",
    );
    refused_lazily(
        &c,
        &stuck_match(app(g("ident"), v("x"))),
        &stuck_match(v("y")),
        "a discriminant that unfolds to a local",
    );
    refused_lazily(
        &c,
        &stuck_match(v("x")),
        &v("y"),
        "a stuck match and a local variable",
    );
    refused_lazily(
        &c,
        &stuck_match(v("x")),
        &lit(3),
        "a stuck match and a literal",
    );
    refused_lazily(
        &c,
        &stuck_match(v("x")),
        &app(g("Succ"), v("y")),
        "a stuck match and a constructor",
    );
    equal_lazily(
        &c,
        &stuck_match(app(g("ident"), v("x"))),
        &stuck_match(v("x")),
        "matches on convertible discriminants",
    );
}

#[test]
fn stuck_fixpoints_on_different_locals_are_refused_without_full_normalization() {
    let c = closing_environment();
    refused_lazily(
        &c,
        &countdown(v("x")),
        &countdown(v("y")),
        "a recursion stuck on two different locals",
    );
    refused_lazily(
        &c,
        &countdown(v("x")),
        &g("Zero"),
        "a stuck recursion and a constructor",
    );
    refused_lazily(
        &c,
        &countdown(v("x")),
        &stuck_match(v("x")),
        "a stuck recursion and a stuck match",
    );
    equal_lazily(
        &c,
        &countdown(app(g("ident"), v("x"))),
        &countdown(v("x")),
        "recursions on convertible arguments",
    );
    assert!(
        def_eq(&c, &countdown(lit(3)), &g("Zero")),
        "a recursion on a literal computes"
    );
}

#[test]
fn stuck_terms_of_a_unit_type_or_a_proposition_stay_equal() {
    let c = closing_environment();
    equal_lazily(
        &c,
        &unit_match(v("x")),
        &unit_match(v("y")),
        "two stuck matches at a unit type (unit η)",
    );
    equal_lazily(
        &c,
        &unit_match(v("x")),
        &v("q"),
        "a stuck match and a local at a unit type",
    );
    let c = c.extend("h1", g("True")).extend("h2", g("True"));
    let proof = |d: Term| Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(lam("_", g("Nat"), g("True"))),
        cases: vec![v("h1"), lam("k", g("Nat"), v("h2"))],
    };
    assert!(
        def_eq(&c, &proof(v("x")), &proof(v("y"))),
        "two stuck proofs of one proposition (proof irrelevance)"
    );
    assert!(
        def_eq(&c, &proof(v("x")), &v("h2")),
        "a stuck proof and a local proof"
    );
}

// ------------------------------------------------------------------------------------------------
// Registered rewrite rules (KB's finding, 2026-09-24): one unrelated `@[simp]` rule used to shut
// the loop off, so every equation lemma of a δ-chain was decided by normalizing the whole chain.
// With rules the loop still proves equalities — rules only add equalities — and never refutes.
// ------------------------------------------------------------------------------------------------

/// The standard environment with an assumed rule `p Zero = Zero` registered as `@[simp]`.
fn with_a_rule() -> Context {
    let mut c = globals();
    let unary = arrow(g("Nat"), g("Nat"));
    c.add_declaration("p", unary);
    let eq = |l: Term, r: Term| app(app(app(g("Eq"), g("Nat")), l), r);
    c.add_declaration("p_rule", eq(app(g("p"), g("Zero")), g("Zero")));
    c.add_simp_lemma("p_rule").expect("the rule compiles");
    assert!(!c.simp_rules().is_empty());
    c.extend("x", g("Nat")).extend("y", g("Nat"))
}

#[test]
fn with_a_rewrite_rule_registered_a_wrapper_chain_still_converts_without_full_normalization() {
    on_a_large_stack(|| {
        let c = with_a_rule();
        let t = tree("x", LEVELS);
        let (a, b) = (app(g("top"), t.clone()), pair(t.clone(), t));
        assert_eq!(lazy_delta::lazy_delta_conversion(&c, &a, &b), Some(true));
        let (equal, normalizations) = full_normalizations(&c, &a, &b);
        assert!(equal);
        assert_eq!(
            normalizations, 0,
            "an unrelated rule does not shut the loop off"
        );
    });
}

#[test]
fn with_a_rewrite_rule_registered_the_loop_never_refutes() {
    let c = with_a_rule();
    assert_eq!(
        lazy_delta::lazy_delta_conversion(&c, &v("x"), &v("y")),
        None,
        "R1 is left to the rule-aware fallback"
    );
    assert_eq!(
        lazy_delta::lazy_delta_conversion(&c, &g("Zero"), &app(g("Succ"), v("x"))),
        None
    );
    assert!(
        !def_eq(&c, &v("x"), &v("y")),
        "and the fallback still refuses"
    );
    assert!(
        def_eq(
            &c,
            &app(g("p"), app(lam("n", g("Nat"), v("n")), g("Zero"))),
            &g("Zero")
        ),
        "the rule still fires once its argument is normalized (`p ((λn. n) Zero) ≡ Zero`)"
    );
}

/// Registering a δ-chain checks one equation lemma per link; with a rule registered each check
/// used to normalize the whole chain below it.
#[test]
fn registering_a_delta_chain_under_a_rewrite_rule_needs_no_full_normalization() {
    let mut c = with_a_rule();
    let unary = arrow(g("Nat"), g("Nat"));
    c.declare(
        "link0",
        unary.clone(),
        lam("n", g("Nat"), app(g("Succ"), v("n"))),
    )
    .unwrap();
    CONVERSION_FULL_NORMALIZATIONS.with(|count| count.set(0));
    for i in 1..200 {
        c.declare(
            &format!("link{i}"),
            unary.clone(),
            lam("n", g("Nat"), app(g(&format!("link{}", i - 1)), v("n"))),
        )
        .unwrap();
    }
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    assert_eq!(CONVERSION_FULL_NORMALIZATIONS.with(|count| count.get()), 0);
    let (equal, normalizations) =
        full_normalizations(&c, &app(g("link199"), v("x")), &app(g("Succ"), v("x")));
    assert!(equal);
    assert_eq!(normalizations, 0);
}

// ------------------------------------------------------------------------------------------------
// The W06 timeout groups' minimal originals: Lean's own `Init` theorems, each exported with its
// dependency closure and nothing else (`tests/fixtures/lean_scale/LG_{Omega,Linear,ModCore}.lean`,
// pinned Lean 4.14.0 and lean4export). Before lazy δ every conformance-corpus root that used them stalled
// for more than 300 s in the conversion fallback. The count is the whole import's, both kernels
// and every declaration of the closure included, so it runs alone in a child process.
// ------------------------------------------------------------------------------------------------

const ORIGINAL_ENV: &str = "LOGOS_LG_LAZY_DELTA_ORIGINAL";
const ORIGINAL_CHILD: &str = "type_checker::lazy_delta_controls::one_original_imported_alone";

/// The pinned export of a minimal original and its roots (Lean spellings), verified against its
/// provenance.
fn original(stem: &str) -> (String, Vec<String>) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_tests/tests/fixtures/lean_scale");
    let export =
        std::fs::read_to_string(dir.join(format!("{stem}.ndjson"))).expect("pinned export");
    let provenance: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("{stem}.provenance.json"))).expect("provenance"),
    )
    .expect("provenance parses");
    assert_eq!(
        crate::interface::lean::package::sha256_hex(export.as_bytes()),
        provenance["export_sha256"],
        "{stem}: export bytes"
    );
    assert_eq!(
        provenance["declaration_selection"],
        "explicit_witness_roots"
    );
    assert_eq!(provenance["toolchain"], "leanprover/lean4:v4.14.0");
    let roots = provenance["declarations"]
        .as_array()
        .expect("roots")
        .iter()
        .map(|root| root.as_str().expect("a root name").to_owned())
        .collect();
    (export, roots)
}

/// The child half of [`full_normalizations_importing`], a no-op anywhere else: import one original
/// alone, confirm every root was checked, and report this process's full-normalization fallbacks.
#[test]
fn one_original_imported_alone() {
    let Ok(stem) = std::env::var(ORIGINAL_ENV) else {
        return;
    };
    let (export, roots) = original(&stem);
    let verdict = match crate::interface::lean::import(&export) {
        Ok((ctx, _)) => {
            let missing: Vec<&String> = roots
                .iter()
                .filter(|root| {
                    let components: Vec<&str> = root.split('.').collect();
                    ctx.get_global(
                        &crate::interface::lean::reference::kernel_name_of_components(&components),
                    )
                    .is_none()
                })
                .collect();
            if missing.is_empty() {
                "ACCEPT".to_owned()
            } else {
                format!("MISSING {missing:?}")
            }
        }
        Err(declined) => format!("{declined:?}").replace('\n', " "),
    };
    let fallbacks = PROCESS_FULL_NORMALIZATIONS.load(std::sync::atomic::Ordering::Relaxed);
    println!("LG_ORIGINAL {stem} {fallbacks} {verdict}");
    std::process::exit(0);
}

/// Import `stem` in a child process: its verdict and how many conversions fell back to full
/// normalization.
fn full_normalizations_importing(stem: &str) -> (String, usize) {
    let out = std::process::Command::new(
        std::env::current_exe().expect("the test binary knows its path"),
    )
    .args(["--exact", ORIGINAL_CHILD, "--test-threads=1", "--nocapture"])
    .env(ORIGINAL_ENV, stem)
    .output()
    .expect("the child starts");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // libtest prints `test <name> ... ` without a newline before the test's own output.
    let report = stdout
        .lines()
        .find_map(|line| line.split_once("LG_ORIGINAL ").map(|(_, report)| report))
        .unwrap_or_else(|| {
            panic!(
                "{stem}: the child reported nothing ({:?})\n{stdout}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            )
        });
    let mut fields = report.splitn(3, ' ');
    assert_eq!(fields.next(), Some(stem));
    let fallbacks = fields.next().and_then(|n| n.parse().ok()).expect("a count");
    (fields.next().unwrap_or_default().to_owned(), fallbacks)
}

#[test]
fn the_omega_originals_import_without_a_full_normalization() {
    let (verdict, fallbacks) = full_normalizations_importing("LG_Omega");
    assert_eq!(
        verdict, "ACCEPT",
        "dot_mod_gcd_left, dot_set_left, gcd_cons_div_left, normalize_sat, natAbs_dvd_natAbs"
    );
    assert_eq!(
        fallbacks, 0,
        "every conversion of the Omega closure is decided before the normalization fallback"
    );
}

#[test]
fn the_nat_linear_originals_import_without_a_full_normalization() {
    let (verdict, fallbacks) = full_normalizations_importing("LG_Linear");
    assert_eq!(
        verdict, "ACCEPT",
        "of_denote_eq_cancelAux, denote_sort_go, denote_toPoly"
    );
    assert_eq!(
        fallbacks, 0,
        "every conversion of the Nat.Linear closure is decided before the normalization fallback"
    );
}

#[test]
fn the_mod_core_originals_import_without_a_full_normalization() {
    let (verdict, fallbacks) = full_normalizations_importing("LG_ModCore");
    assert_eq!(verdict, "ACCEPT", "Nat.modCore_eq_mod, Nat.gcd_zero_right");
    assert_eq!(
        fallbacks, 0,
        "every conversion of the modCore closure is decided before the normalization fallback"
    );
}

// ------------------------------------------------------------------------------------------------
// The literal bridge inside the loop: a literal against a constructor application is compared as
// the constructor spelling the kernel's literal rule gives it (`0` is `Zero`, `n + 1` is
// `Succ n`, an integer literal its `Int` constructor), as Lean's `is_def_eq` expands a literal
// against a constructor. The W06 Omega closure compares `1` with `x + 1` and `0` with
// `(x + 1) * (y + 1)` for local naturals `x y`; each was left to the normalization fallback.
// ------------------------------------------------------------------------------------------------

#[test]
fn a_literal_against_a_constructor_is_compared_through_its_spelling_without_full_normalization() {
    let c = closing_environment();
    refused_lazily(
        &c,
        &lit(1),
        &app(g("Succ"), v("x")),
        "1 against Succ x: 0 is not the local x",
    );
    refused_lazily(
        &c,
        &lit(0),
        &app(g("Succ"), v("x")),
        "0 is Zero, never a successor",
    );
    refused_lazily(
        &c,
        &lit(2),
        &app(g("Succ"), app(g("Succ"), app(g("Succ"), v("x")))),
        "2 is two successors of Zero",
    );
    refused_lazily(
        &c,
        &lit(3),
        &app(g("Succ"), app(g("Succ"), v("x"))),
        "3 against x + 2: 1 is not the local x",
    );
    let zero_of = |n: Term| app(lam("m", g("Nat"), g("Zero")), n);
    equal_lazily(
        &c,
        &lit(1),
        &app(g("Succ"), zero_of(v("x"))),
        "1 against Succ ((λm. Zero) x)",
    );
    equal_lazily(
        &c,
        &lit(2),
        &app(g("Succ"), app(g("Succ"), zero_of(v("y")))),
        "2 against Succ (Succ ((λm. Zero) y))",
    );
    let int = |n: i64| Term::Lit(Literal::Int(n));
    refused_lazily(
        &c,
        &int(3),
        &app(g("Int_ofNat"), v("x")),
        "the Int literal 3 against Int_ofNat x",
    );
    refused_lazily(
        &c,
        &int(-2),
        &app(g("Int_ofNat"), v("x")),
        "a negative Int literal is never Int_ofNat",
    );
    equal_lazily(
        &c,
        &int(-2),
        &app(g("Int_negSucc"), app(g("Succ"), zero_of(v("x")))),
        "-2 against Int_negSucc (Succ ((λm. Zero) x))",
    );
}

/// `fix f. λn:Nat. match n return (λm. Nat) n with | Zero => Zero | Succ p => step (f p)` at `arg`,
/// with a motive that is itself a redex, so no syntactic reading gives the match's family.
fn recursion(step: impl Fn(Term) -> Term, argument: Term) -> Term {
    let motive = lam("k", g("Nat"), app(lam("m", g("Nat"), g("Nat")), v("k")));
    let body = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(motive),
            cases: vec![g("Zero"), lam("p", g("Nat"), step(app(v("f"), v("p"))))],
        },
    );
    app(
        Term::Fix {
            name: "f".into(),
            body: Box::new(body),
        },
        argument,
    )
}

/// Two fixpoints stuck on one local natural whose bodies differ — Lean's two stuck recursor
/// applications with different minor premises. Their bodies are compared with the recursive name
/// opened as a fresh variable that has no type, so neither body's `match` has an inferable type:
/// unit η does not apply to it, as the fallback's own unit-η step decides, and R4 refutes.
#[test]
fn two_fixpoints_whose_bodies_differ_are_refused_without_full_normalization() {
    let c = closing_environment();
    let count = |t: Term| t;
    let tower = |t: Term| app(g("Succ"), t);
    refused_lazily(
        &c,
        &recursion(count, v("x")),
        &recursion(tower, v("x")),
        "countdown x against tower x",
    );
    equal_lazily(
        &c,
        &recursion(count, v("x")),
        &recursion(count, v("x")),
        "one recursion against itself",
    );
    equal_lazily(
        &c,
        &recursion(tower, app(g("ident"), v("x"))),
        &recursion(tower, v("x")),
        "one recursion on convertible arguments",
    );
}

// ------------------------------------------------------------------------------------------------
// Lean's `reduce_nat` inside the loop: a program the evaluator certifies, applied to operands whose
// weak-head normal forms are numerals, is its value — Lean's kernel computes `Nat.beq`, `Nat.ble`,
// `+`, `/`, `%` on literals before any δ step. The W06 `Nat.Linear` closure decides
// `Var.denote ctx fixedVar` with `fixedVar = 100000000`: unrolling `Nat.beq` there is 10⁸ steps.
// ------------------------------------------------------------------------------------------------

/// `beq m n` by Lean's `Nat.beq` recursion, and `pick b x y := match b with true => x | false => y`.
fn with_a_certified_equality() -> Context {
    let mut c = globals();
    let bool_of = |d: Term, zero: Term, successor: Term| Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(lam("_", g("Nat"), g("Bool"))),
        cases: vec![zero, successor],
    };
    let body = lam(
        "m",
        g("Nat"),
        lam(
            "n",
            g("Nat"),
            bool_of(
                v("m"),
                bool_of(v("n"), g("true"), lam("pn", g("Nat"), g("false"))),
                lam(
                    "pm",
                    g("Nat"),
                    bool_of(
                        v("n"),
                        g("false"),
                        lam("pn", g("Nat"), app(app(v("beq"), v("pm")), v("pn"))),
                    ),
                ),
            ),
        ),
    );
    c.declare(
        "beq",
        arrow(g("Nat"), arrow(g("Nat"), g("Bool"))),
        Term::Fix {
            name: "beq".into(),
            body: Box::new(body),
        },
    )
    .unwrap();
    let pick = lam(
        "b",
        g("Bool"),
        lam(
            "x",
            g("Nat"),
            lam(
                "y",
                g("Nat"),
                Term::Match {
                    discriminant: Box::new(v("b")),
                    motive: Box::new(lam("_", g("Bool"), g("Nat"))),
                    cases: vec![v("x"), v("y")],
                },
            ),
        ),
    );
    c.declare(
        "pick",
        arrow(g("Bool"), arrow(g("Nat"), arrow(g("Nat"), g("Nat")))),
        pick,
    )
    .unwrap();
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    c.extend("x", g("Nat")).extend("y", g("Nat"))
}

#[test]
fn a_certified_program_at_numerals_is_computed_inside_an_open_comparison() {
    let c = with_a_certified_equality();
    let big = || lit(100_000_000);
    let picked = |b: Term| app(app(app(g("pick"), b), v("x")), v("y"));
    let beq = |m: Term, n: Term| app(app(g("beq"), m), n);
    equal_lazily(
        &c,
        &picked(beq(big(), big())),
        &v("x"),
        "pick (beq 10⁸ 10⁸) x y against x",
    );
    refused_lazily(
        &c,
        &picked(beq(big(), lit(100_000_001))),
        &v("x"),
        "pick (beq 10⁸ (10⁸+1)) x y is y, not x",
    );
    equal_lazily(
        &c,
        &picked(beq(app(g("Succ"), lit(99_999_999)), big())),
        &v("x"),
        "a successor of a literal is the numeral it spells",
    );
    refused_lazily(
        &c,
        &picked(beq(v("x"), big())),
        &v("x"),
        "an open operand is never computed",
    );
}

// ------------------------------------------------------------------------------------------------
// Inert constants: an axiom or opaque constant that no rule computes with is neutral for good, as
// Lean's are — its application is a rigid head (R1, R2) and a recursion stuck on it is stuck for good
// (R4). `LG_DivImpostor.axMod 5 5` unrolls to a recursion stuck on the axiom `opaqueSub 5 5`, which
// the normalization fallback never finishes. A name a rule computes with is never inert.
// ------------------------------------------------------------------------------------------------

fn with_axioms() -> Context {
    let mut c = globals();
    c.add_declaration("opaque_f", arrow(g("Nat"), g("Nat")));
    c.add_declaration("opaque_g", arrow(g("Nat"), g("Nat")));
    c.add_declaration("opaque_c", g("Nat"));
    c.declare(
        "ident",
        arrow(g("Nat"), g("Nat")),
        lam("n", g("Nat"), v("n")),
    )
    .unwrap();
    c.add_inductive("Unit1", Term::Sort(Universe::Type(0)));
    c.add_constructor("star1", "Unit1", g("Unit1"));
    c.add_declaration("opaque_unit", arrow(g("Nat"), g("Unit1")));
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    c.extend("x", g("Nat")).extend("y", g("Nat"))
}

#[test]
fn an_inert_constant_is_a_rigid_head() {
    let c = with_axioms();
    let f = |t: Term| app(g("opaque_f"), t);
    refused_lazily(
        &c,
        &f(v("x")),
        &g("Zero"),
        "an axiom application against a constructor",
    );
    refused_lazily(
        &c,
        &f(v("x")),
        &lit(3),
        "an axiom application against a literal",
    );
    refused_lazily(
        &c,
        &f(v("x")),
        &v("y"),
        "an axiom application against a local",
    );
    refused_lazily(
        &c,
        &f(v("x")),
        &app(g("opaque_g"), v("x")),
        "two different axioms",
    );
    refused_lazily(
        &c,
        &g("opaque_c"),
        &lit(0),
        "a nullary axiom against a literal",
    );
    refused_lazily(
        &c,
        &f(lit(1)),
        &f(lit(2)),
        "one axiom over different arguments",
    );
    equal_lazily(
        &c,
        &f(app(g("ident"), v("x"))),
        &f(v("x")),
        "one axiom over convertible arguments",
    );
    let unit = |t: Term| app(g("opaque_unit"), t);
    equal_lazily(
        &c,
        &unit(v("x")),
        &unit(v("y")),
        "two applications of an axiom into a unit type are equal by unit η",
    );
}

#[test]
fn a_recursion_stuck_on_an_inert_constant_is_refused_without_full_normalization() {
    let c = with_axioms();
    let f = |t: Term| app(g("opaque_f"), t);
    refused_lazily(
        &c,
        &countdown(f(lit(5))),
        &g("Zero"),
        "a recursion stuck on opaque_f 5 against Zero",
    );
    refused_lazily(
        &c,
        &countdown(f(lit(5))),
        &countdown(app(g("opaque_g"), lit(5))),
        "recursions stuck on two different axioms",
    );
    refused_lazily(
        &c,
        &stuck_match(f(v("x"))),
        &lit(3),
        "a match stuck on an axiom application against a literal",
    );
    equal_lazily(
        &c,
        &countdown(f(app(g("ident"), lit(5)))),
        &countdown(f(lit(5))),
        "recursions stuck on convertible applications",
    );
}

/// Three chains of `depth` definitions over proofs — `zero_k, again_k : 0 = 0` and `one_k : 1 = 1`,
/// each `k` defined as `k - 1`, bottoming out at `refl` — so unfolding a side of a comparison takes
/// `depth` δ steps.
fn proof_chains(depth: usize) -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let eq = |n: u64| app(app(app(g("Eq"), g("Nat")), lit(n)), lit(n));
    for (chain, n) in [("zero", 0), ("again", 0), ("one", 1)] {
        c.declare(
            &format!("{chain}_0"),
            eq(n),
            app(app(g("refl"), g("Nat")), lit(n)),
        )
        .unwrap();
        for k in 1..=depth {
            c.declare(
                &format!("{chain}_{k}"),
                eq(n),
                g(&format!("{chain}_{}", k - 1)),
            )
            .unwrap();
        }
    }
    c
}

/// Lean's `is_def_eq_proof_irrel`, both halves: two proofs are compared by their propositions and
/// never unfolded — equal when the propositions convert, refused when they differ — and a proof is
/// never equal to a term of another type. Each proof sits beneath more δ steps than the lazy loop's
/// allowance, so a comparison that unfolded them would end in the normalization fallback.
#[test]
fn two_proofs_are_decided_by_their_propositions_without_unfolding_either() {
    on_a_large_stack(|| {
        let depth = 5000;
        let c = proof_chains(depth);
        let top = |chain: &str| g(&format!("{chain}_{depth}"));
        assert_eq!(
            full_normalizations(&c, &top("zero"), &top("again")),
            (true, 0),
            "two proofs of 0 = 0"
        );
        assert_eq!(
            full_normalizations(&c, &top("zero"), &top("one")),
            (false, 0),
            "proofs of 0 = 0 and of 1 = 1"
        );
        assert_eq!(
            full_normalizations(&c, &top("one"), &top("zero")),
            (false, 0),
            "proofs of 1 = 1 and of 0 = 0"
        );
        assert_eq!(
            full_normalizations(&c, &top("zero"), &lit(0)),
            (false, 0),
            "a proof against a natural"
        );
    });
}
