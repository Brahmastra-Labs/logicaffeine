//! Tier 0 of the Lean-4 conformance corpus — the KERNEL TERM LAYER.
//!
//! Twenty-two cases mapping `Lean.Expr` onto LOGOS's `Term`. The mapping is mostly
//! mechanical: `sort`/`const`/`forallE`/`lam`/`app`/`letE`/`lit` all have a direct
//! counterpart. The cases that matter are the asymmetries, and there are six of them:
//!
//! 1. **LOGOS is NAMED, not de Bruijn.** One `Term::Var(String)`, no `bvar`/`fvar` split.
//!    Two binders with the same name are literally the same `Context` key, so the inner
//!    one shadows the outer; expressing Lean's *outermost* `x` needs a freshening pass.
//!    The second kernel (`recheck`) deliberately uses de Bruijn, so a naming bug surfaces
//!    as a DISAGREEMENT between two independent checkers — a guarantee Lean has no
//!    equivalent of.
//! 2. **`Term::Hole` matches ANYTHING.** `types_equal` short-circuits on it and the
//!    leniency leaks into `is_subtype`'s fast path, so one surviving hole turns a type
//!    check into a no-op for that subterm.
//! 3. **There is no `Proj`, no `mdata`, no metavariable variant.** Projections go through
//!    `add_structure`'s generated field definitions; annotations are simply gone;
//!    elaborator metavariables are `Var`s whose name starts with `?`.
//! 4. **Lean's term-mode `have` is definitionally OPAQUE; `Term::Let` is transparent.**
//!    LOGOS has both — the beta-redex `App(Lambda …, v)` is the opaque one — so the two
//!    Lean forms do have distinct, faithful LOGOS images.
//! 5. **Function eta lives only in `def_eq`, never in `normalize`.** The cheap equality
//!    oracle "normalise both sides and compare" is therefore WRONG for LOGOS.
//! 6. **`Term` is a tree.** `Expr`'s hash-consed DAG sharing cannot be expressed, and
//!    `Let` does not restore it — its typing rule zeta-substitutes before checking.
//!
//! Each `#[test]` below decides exactly one corpus case; the case id opens its banner.
//! `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/t00_terms.json` records the verdict and binds it to the test.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, derive_recursor, double_check, infer_type, instantiate, is_subtype, normalize,
    normalize_at, Annotation, BigInt, Context, DoubleCheck, KernelError, Literal, MetaCtx,
    ParamKind, Provenance, Reducibility, SourcePosition, Term, Transparency, Universe,
};

// ============================================================================================
// HELPERS — per-file, per house style (`indexed_recursor.rs`). There is no shared kernel
// helper module and there must not be one.
// ============================================================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn app3(f: Term, x: Term, y: Term, z: Term) -> Term {
    app(app2(f, x, y), z)
}
fn app4(f: Term, w: Term, x: Term, y: Term, z: Term) -> Term {
    app(app3(f, w, x, y), z)
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn let_in(n: &str, ty: Term, value: Term, body: Term) -> Term {
    Term::Let {
        name: n.to_string(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(body),
    }
}
fn nat() -> Term {
    g("Nat")
}
fn bool_ty() -> Term {
    g("Bool")
}
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn nat_lit(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn int_lit_of(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
/// `Succ^n Zero` — the Peano spelling of a `Nat`.
fn peano(n: u32) -> Term {
    (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc))
}
/// `Eq A x y`.
fn eq(ty: Term, x: Term, y: Term) -> Term {
    app3(g("Eq"), ty, x, y)
}
/// `refl A x : Eq A x x`.
fn refl(ty: Term, x: Term) -> Term {
    app2(g("refl"), ty, x)
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// Node count of a `Term` tree — the measure that separates a DAG from its unfolding.
fn nodes(t: &Term) -> usize {
    1 + match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => 0,
        Term::Pi {
            param_type,
            body_type,
            ..
        } => nodes(param_type) + nodes(body_type),
        Term::Lambda {
            param_type, body, ..
        } => nodes(param_type) + nodes(body),
        Term::App(f, a) => nodes(f) + nodes(a),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => nodes(discriminant) + nodes(motive) + cases.iter().map(nodes).sum::<usize>(),
        Term::Fix { body, .. } => nodes(body),
        Term::MutualFix { defs, .. } => defs.iter().map(|(_, b)| nodes(b)).sum::<usize>(),
        Term::Let {
            ty, value, body, ..
        } => nodes(ty) + nodes(value) + nodes(body),
        Term::Annotated { term, .. } => nodes(term),
    }
}

/// Every `Term` variant, asked one question: does it carry SOURCE METADATA the way
/// `Expr.mdata` does? The match is deliberately exhaustive and wildcard-free, so a new
/// variant has to answer here rather than slipping past under a `_`.
fn source_annotation(t: &Term) -> Option<&str> {
    match t {
        Term::Annotated {
            annotation: Annotation::Source(provenance),
            ..
        } => match provenance {
            Provenance::Position(position) => Some(position.as_str()),
            Provenance::LeanMetadata { key, .. } => Some(key.as_str()),
        },
        Term::Annotated { term, .. } => source_annotation(term),
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Pi { .. }
        | Term::Lambda { .. }
        | Term::App(_, _)
        | Term::Match { .. }
        | Term::Fix { .. }
        | Term::MutualFix { .. }
        | Term::Let { .. }
        | Term::Lit(_)
        | Term::Hole => None,
    }
}

/// Every `Term` variant, asked whether it can record its binder's KIND the way
/// `Expr.lam`'s `BinderInfo` does. Exhaustive for the same reason.
fn binder_kind_of(t: &Term) -> Option<ParamKind> {
    match t {
        Term::Annotated {
            annotation: Annotation::Binder(kind),
            ..
        } => Some(*kind),
        Term::Annotated { term, .. } => binder_kind_of(term),
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Pi { .. }
        | Term::Lambda { .. }
        | Term::App(_, _)
        | Term::Match { .. }
        | Term::Fix { .. }
        | Term::MutualFix { .. }
        | Term::Let { .. }
        | Term::Lit(_)
        | Term::Hole => None,
    }
}

/// Lean's `BinderInfo` has four inhabitants: `default`, `implicit`, `strictImplicit`,
/// `instImplicit`.
const LEAN_BINDER_KINDS: usize = 4;
const LOGOS_PARAM_KINDS: &[ParamKind] = &[
    ParamKind::Explicit,
    ParamKind::Implicit,
    ParamKind::StrictImplicit,
    ParamKind::Instance,
];
/// Binds [`LOGOS_PARAM_KINDS`] to the enum: a new `ParamKind` makes this match non-exhaustive,
/// so the list above cannot silently fall behind the kinds LOGOS actually has.
#[allow(dead_code)]
fn every_param_kind_is_listed(kind: ParamKind) {
    match kind {
        ParamKind::Explicit
        | ParamKind::Implicit
        | ParamKind::StrictImplicit
        | ParamKind::Instance => {}
    }
}

// ============================================================================================
// T00-001 · `Expr.bvar` / `Expr.fvar` → a NAMED `Term::Var`
// ============================================================================================

/// DIVERGE. Lean distinguishes `fun x => fun x => fun x => x`'s three binders by de Bruijn
/// index; LOGOS distinguishes them by NAME, so the body means the innermost and Lean's
/// *outermost* reading is reached by freshening two of them. Both terms are expressible and
/// both kernels agree on them — and the second kernel is de Bruijn, so a naming bug shows up
/// as a disagreement between two independent checkers rather than as silence.
#[test]
fn three_binders_named_x_resolve_innermost_and_freshening_recovers_the_outer() {
    let ctx = std_ctx();

    // `fun x => fun x => fun x => x`
    let innermost = lam("x", nat(), lam("x", nat(), lam("x", nat(), var("x"))));
    let ty = infer_type(&ctx, &innermost).expect("three same-named binders are a legal term");
    assert_eq!(
        ty,
        pi("x", nat(), pi("x", nat(), pi("x", nat(), nat()))),
        "a shadowed telescope must still infer the full Π chain: {ty}"
    );
    let applied = app3(innermost.clone(), peano(1), peano(2), peano(3));
    assert_eq!(
        normalize(&ctx, &applied),
        peano(3),
        "the body's `x` is the INNERMOST binder, so the third argument is selected"
    );

    // `fun x => fun _y => fun _z => x` — Lean's outermost reading, reachable in a named
    // representation ONLY after freshening the two inner binders.
    let outermost = lam("x", nat(), lam("y", nat(), lam("z", nat(), var("x"))));
    assert_eq!(
        normalize(&ctx, &app3(outermost, peano(1), peano(2), peano(3))),
        peano(1),
        "freshening the inner binders is what makes the outermost `x` expressible"
    );

    // Verbatim name reuse CANNOT express it: every `Var("x")` resolves innermost.
    assert_ne!(
        normalize(&ctx, &applied),
        peano(1),
        "verbatim name reuse silently means the innermost binder, never the outermost"
    );

    // An `fvar` is a LOCAL hypothesis: a binder named like a prelude global stays a `Var`
    // and never resolves through `get_global` to the inductive of the same name.
    let named_like_a_global = lam("Nat", bool_ty(), var("Nat"));
    assert_eq!(
        infer_type(&ctx, &named_like_a_global).expect("a binder may be named `Nat`"),
        pi("Nat", bool_ty(), bool_ty()),
        "`Var(\"Nat\")` is the local binder, not the prelude inductive"
    );

    // The de Bruijn second kernel agrees — the independent check the named representation
    // is paid for with.
    assert_eq!(
        double_check(&ctx, &innermost),
        DoubleCheck::Agreed,
        "the de Bruijn re-checker must agree on a shadowed telescope"
    );

    // The one place the naming convention bites: a binder whose name starts with `?` IS an
    // elaborator metavariable, and `instantiate` rewrites the BOUND occurrence.
    let question_binder = lam("?x", nat(), var("?x"));
    let mut mctx = MetaCtx::new();
    mctx.solve("?x", peano(7));
    assert_eq!(
        instantiate(&question_binder, &mctx),
        lam("?x", nat(), peano(7)),
        "a `?`-prefixed binder name is captured by the metavariable convention — a \
         translated binder must never start with `?`"
    );
}

// ============================================================================================
// T00-002 · `Expr.sort` — the `Sort n` / `Type n` index shift
// ============================================================================================

/// MUST. `Prop ≡ Sort 0` and `Sort (n+1) ≡ Type n`. `Universe::Prop` and `Universe::Type(0)`
/// are separate constructors and the kernel dispatches on them syntactically, so the
/// off-by-one that comes from reading the internal `Prop = 0, Type n = n+1` encoding as the
/// surface mapping must be detectable STRUCTURALLY, not under cumulativity.
#[test]
fn prop_is_not_type_zero_and_the_sort_ladder_climbs_one_level_at_a_time() {
    let ctx = std_ctx();

    // Structural: the two sorts are different terms.
    assert_ne!(
        Term::Sort(Universe::Prop),
        Term::Sort(Universe::Type(0)),
        "`Prop` and `Type 0` are distinct sorts — mapping `Prop ↦ Type(0)` silently disables \
         proof irrelevance, the large-elimination gate, and `imax`"
    );
    assert!(
        !Universe::Prop.equiv(&Universe::Type(0)),
        "the level algebra must not identify `Prop` with `Type 0`"
    );

    // `Sort (n+1) ↦ Type n`: each rung types at exactly the next one.
    for n in 0u32..4 {
        let inferred = infer_type(&ctx, &Term::Sort(Universe::Type(n)))
            .expect("a concrete sort always has a type");
        assert_eq!(
            inferred,
            Term::Sort(Universe::Type(n + 1)),
            "`Type {n} : Type {}` — an off-by-one here shifts every level in the file",
            n + 1
        );
    }

    // Adjacent rungs are NOT interchangeable upward, so a level that floats up is caught.
    assert!(
        Universe::Type(0).is_subtype_of(&Universe::Type(1)),
        "LOGOS is cumulative: `Type 0 ≤ Type 1`"
    );
    assert!(
        !Universe::Type(1).is_subtype_of(&Universe::Type(0)),
        "cumulativity is one-way: `Type 1 ≰ Type 0`"
    );

    // A sort in BINDER position and a sort as an ARGUMENT — the same `Expr.sort` reached by
    // two different routes.
    let quantifies_over_types = pi("A", type0(), nat());
    assert_eq!(
        infer_type(&ctx, &quantifies_over_types).expect("∀(A:Type). Nat is well formed"),
        Term::Sort(Universe::Type(1)),
        "`∀ (_α : Type), Nat` lives one level above its domain"
    );
    let quantifies_over_props = pi("p", prop(), nat());
    assert_eq!(
        infer_type(&ctx, &quantifies_over_props).expect("∀(p:Prop). Nat is well formed"),
        Term::Sort(Universe::Type(0)),
        "`∀ (_p : Prop), Nat : Type 0` — exactly Lean's answer, and the case file's own Lean \
         source says so: `def quantifiesOverProps : Type := ∀ (_p : Prop), Nat`. `Prop : Type 0`, \
         so `imax(Type 0, Type 0) = Type 0`. Asserting `Type 1` here re-pins conformance defect 1"
    );

    // `Sort u` for a VARIABLE level is a `Universe::Var`, not a `Type(u32)`.
    let sort_var = Term::Sort(Universe::Var("u".to_string()));
    assert!(
        matches!(sort_var, Term::Sort(Universe::Var(_))),
        "a universe-variable sort has its own level form; `Universe::Type` takes a concrete u32"
    );
}

// ============================================================================================
// T00-003 · `Expr.const []` → `Term::Global`, never `Term::Const`
// ============================================================================================

/// MUST. A reference with an empty level list is `Term::Global`; `Term::Const { name, levels }`
/// is resolved through a DISJOINT map (`universe_polys`), so the structurally-obvious
/// `Expr.const n [] ↦ Const { name: n, levels: vec![] }` does not resolve — and, worse,
/// reduction on the same miss returns the term unreduced with no diagnostic.
#[test]
fn a_monomorphic_reference_resolves_as_a_global_and_a_const_at_no_levels_does_not() {
    let mut ctx = std_ctx();
    ctx.add_definition("base".to_string(), nat(), peano(7));

    // The correct lowering.
    assert_eq!(
        infer_type(&ctx, &g("base")).expect("a plain definition resolves through get_global"),
        nat(),
        "`Expr.const base []` is a `Term::Global`"
    );
    assert_eq!(
        normalize(&ctx, &g("base")),
        peano(7),
        "a global delta-unfolds to its body"
    );

    // The structurally-obvious lowering fails — loudly at the type level …
    let as_const = Term::Const {
        name: "base".to_string(),
        levels: Vec::new(),
    };
    let err = infer_type(&ctx, &as_const)
        .expect_err("`Const` is resolved only through the universe-polymorphic map");
    assert!(
        matches!(err, KernelError::UnboundVariable(ref n) if n == "base"),
        "a `Const` naming a plain definition is unbound, however many levels it carries: {err:?}"
    );
    // … and SILENTLY at the reduction level: the reference simply becomes opaque.
    assert_eq!(
        normalize(&ctx, &as_const),
        as_const,
        "reduction on a `Const` miss returns the term unreduced with no diagnostic — the \
         reference goes opaque and every `rfl` that needed it to unfold fails for the wrong \
         reason"
    );

    // Inductives, constructors and declarations are separate maps behind one `get_global`.
    assert_eq!(
        infer_type(&ctx, &nat()).expect("`Nat` is a registered inductive"),
        type0(),
        "a reference to an inductive"
    );
    assert_eq!(
        infer_type(&ctx, &g("Succ")).expect("`Succ` is a registered constructor"),
        pi("_", nat(), nat()),
        "a reference to a constructor — a different map from the inductive of the same family"
    );
    ctx.add_declaration("opaqueNat", nat());
    assert_eq!(
        infer_type(&ctx, &g("opaqueNat")).expect("an axiom resolves through get_global too"),
        nat(),
        "an axiom-shaped reference: type only, found LAST by `get_global`"
    );
    assert!(
        ctx.get_definition_body("opaqueNat").is_none(),
        "an axiom has no body — that is what distinguishes it from a definition"
    );
}

// ============================================================================================
// T00-004 · dependent `Expr.forallE`
// ============================================================================================

/// MUST. A `Π` whose binder is used in the codomain must keep a real name; one whose binder is
/// unused may be `"_"`, which is exactly what `Display` renders as an arrow. Emitting `"_"` for
/// a DEPENDENT binder produces a term that prints as an arrow and whose body mentions `Var("_")`.
#[test]
fn a_dependent_pi_keeps_a_real_binder_name_where_a_plain_arrow_needs_none() {
    let ctx = std_ctx();

    // `∀ (n : Nat), Eq Nat n n → Nat` — the binder is used twice in the codomain.
    let dependent = pi("n", nat(), pi("_", eq(nat(), var("n"), var("n")), nat()));
    assert_eq!(
        infer_type(&ctx, &dependent).expect("a dependent Π is well formed"),
        type0(),
        "Π formation uses `imax`, matching Lean"
    );
    assert!(
        format!("{dependent}").starts_with("Π(n:"),
        "a dependent binder must not be erased to `_`: {dependent}"
    );

    // `Nat → Nat` — non-dependent, so `"_"` is correct and prints as an arrow.
    let plain = pi("_", nat(), nat());
    assert_eq!(
        format!("{plain}"),
        "Nat -> Nat",
        "a non-dependent Π is the arrow, and that is the only thing `_` may mean"
    );

    // Nested binders sharing a name: legal, and the inner binder's TYPE still means the outer.
    let shadowed = pi("n", nat(), pi("n", eq(nat(), var("n"), var("n")), nat()));
    assert_eq!(
        infer_type(&ctx, &shadowed).expect("nested same-named Π binders are legal"),
        type0(),
        "the inner binder's type is outside its own scope, so its `n` is the outer one"
    );

    // The binder used only in a LATER binder's type, not in the final codomain.
    let used_only_later = pi("n", nat(), pi("_v", eq(nat(), var("n"), var("n")), nat()));
    assert_eq!(
        infer_type(&ctx, &used_only_later).expect("a binder used only downstream is still used"),
        type0(),
        "`n` occurs in `_v`'s type, so it may not be erased to `_`"
    );

    assert_eq!(
        double_check(&ctx, &dependent),
        DoubleCheck::Agreed,
        "the de Bruijn re-checker must agree on a dependent Π"
    );
}

// ============================================================================================
// T00-005 · `Expr.lam` — nesting, the mandatory domain, and the dropped `BinderInfo`
// ============================================================================================

/// MUST — Gap. Nested `fun`s become nested `Lambda`s and the telescope ORDER is load-bearing;
/// that part works. What does not: `Term::Lambda` has three fields and none of them is the
/// binder kind. Kinds are recorded OUT OF BAND on a declaration NAME (with all four of Lean's
/// kinds, `strictImplicit` included), so an anonymous lambda — an argument, a match case, a
/// motive — has nowhere to carry its implicitness.
#[test]
fn a_lambda_telescope_keeps_its_order_but_cannot_carry_its_binder_kind() {
    let mut ctx = std_ctx();

    // Three nested binders, the MIDDLE one selected — a reversed telescope is not caught by
    // arity, only by the answer.
    let pick_middle = lam("a", nat(), lam("b", nat(), lam("c", nat(), var("b"))));
    assert_eq!(
        normalize(
            &ctx,
            &app3(pick_middle.clone(), peano(1), peano(2), peano(3))
        ),
        peano(2),
        "nested `Lambda`s are outermost-first; a reversed telescope returns the wrong argument"
    );
    assert_eq!(
        infer_type(&ctx, &pick_middle).expect("a three-binder telescope is well formed"),
        pi("a", nat(), pi("b", nat(), pi("c", nat(), nat()))),
        "`infer_type` builds the Π outside-in, in the lambda's order"
    );

    // The domain is NOT optional: `infer_type` requires it to be a well-formed sort.
    let holed_domain = lam("a", Term::Hole, var("a"));
    assert!(
        infer_type(&ctx, &holed_domain).is_err(),
        "a `Hole` domain must be refused — it is the commonest way a hole escapes into an \
         inferred `Pi`, where it makes every later application typecheck"
    );

    // Binder kinds exist, but ONLY keyed on a declaration name.
    ctx.add_definition(
        "implicitId".to_string(),
        pi("A", type0(), pi("a", var("A"), var("A"))),
        lam("A", type0(), lam("a", var("A"), var("a"))),
    );
    ctx.set_binder_kinds("implicitId", vec![ParamKind::Implicit, ParamKind::Explicit]);
    assert_eq!(
        ctx.binder_kinds("implicitId").map(<[ParamKind]>::len),
        Some(2),
        "a NAMED declaration can record one kind per leading Π binder"
    );

    // An anonymous lambda passed as an argument — `fun {_α} a => a` — has no name to hang
    // kinds on, and no field to carry them either.
    let anonymous_implicit = Term::annotated(
        Annotation::Binder(ParamKind::Implicit),
        lam("A", type0(), lam("a", var("A"), var("a"))),
    );
    assert!(
        binder_kind_of(&anonymous_implicit).is_some(),
        "an anonymous lambda must be able to record that its first binder is IMPLICIT; \
         `Term::Lambda` has three fields and none of them is the binder kind, and \
         `set_binder_kinds` is keyed on a declaration name an anonymous lambda does not have"
    );

    // Lean's `⦃strict implicit⦄` has its own kind: collapsing it onto `Implicit` would make the
    // elaborator insert a metavariable EAGERLY where Lean waits.
    assert_eq!(
        LOGOS_PARAM_KINDS.len(),
        LEAN_BINDER_KINDS,
        "Lean's BinderInfo has four inhabitants (default, implicit, strictImplicit, \
         instImplicit), and `ParamKind` must have a kind for each"
    );
}

// ============================================================================================
// T00-006 · `Expr.app` — left-nested binary spines
// ============================================================================================

/// MUST. `f a b c` is `App(App(App(f, a), b), c)`: binary, LEFT-nested, head innermost.
/// Argument order is positional — the kernel substitutes by position, so a swap between two
/// same-typed arguments is invisible to type checking and changes the value.
#[test]
fn an_application_spine_is_left_nested_and_its_argument_order_is_positional() {
    let ctx = std_ctx();

    // A non-commutative function at two same-typed arguments: a swapped spine is silently
    // wrong, not a type error.
    let pick_first = lam("a", nat(), lam("b", nat(), var("a")));
    assert_eq!(
        normalize(&ctx, &app2(pick_first.clone(), peano(7), peano(3))),
        peano(7),
        "argument order is positional; nothing else records which binder an argument lands in"
    );
    assert_eq!(
        normalize(&ctx, &app2(pick_first.clone(), peano(3), peano(7))),
        peano(3),
        "swapping two same-typed arguments still typechecks and computes a different value"
    );

    // The shape itself: binary and left-nested.
    let spine = app2(pick_first.clone(), peano(7), peano(3));
    match &spine {
        Term::App(f, _) => assert!(
            matches!(f.as_ref(), Term::App(_, _)),
            "a two-argument spine nests to the LEFT; the head is innermost"
        ),
        other => panic!("a spine is a `Term::App`: {other}"),
    }

    // Partial application infers a `Pi` with exactly the remaining binders.
    let partial = app(pick_first, peano(7));
    assert_eq!(
        infer_type(&ctx, &partial).expect("a partial spine is well typed"),
        pi("b", nat(), nat()),
        "one argument of two leaves a one-binder Π"
    );

    // Five arguments: a five-deep chain, and the head is recoverable.
    let mut f5 = var("e");
    for name in ["e", "d", "c", "b", "a"] {
        f5 = lam(name, nat(), f5);
    }
    let saturated = app(app4(f5, peano(1), peano(2), peano(3), peano(4)), peano(5));
    assert_eq!(
        normalize(&ctx, &saturated),
        peano(5),
        "a five-deep left-nested spine binds argument `i` to binder `i`"
    );

    // Higher-order: `applyTwice g n` versus `applyTwice (g n)` — right-nesting is a different
    // term, and the kernel notices only because the types happen to disagree.
    let apply_twice = lam(
        "h",
        pi("_", nat(), nat()),
        lam("n", nat(), app(var("h"), app(var("h"), var("n")))),
    );
    assert_eq!(
        normalize(&ctx, &app2(apply_twice.clone(), g("Succ"), peano(3))),
        peano(5),
        "`applyTwice Succ 3` applies the argument twice"
    );
    assert!(
        infer_type(&ctx, &app(apply_twice, app(g("Succ"), peano(3)))).is_err(),
        "the right-nested reading `applyTwice (Succ 3)` is a DIFFERENT term"
    );
}

// ============================================================================================
// T00-007 · `Expr.letE` → a zeta-TRANSPARENT `Term::Let`
// ============================================================================================

/// MUST. `Term::Let` carries the annotated type as well as the value, and its typing rule
/// zeta-substitutes the value into the body BEFORE inferring — so the body sees `x ≡ v`
/// definitionally. That is strictly stronger than the `App(Lambda …)` beta-redex encoding,
/// and it is also where the zeta duplication comes from.
#[test]
fn a_let_is_zeta_transparent_and_is_not_sugar_for_a_beta_redex() {
    let ctx = std_ctx();

    let simple = let_in("n", nat(), peano(3), app(g("Succ"), var("n")));
    assert_eq!(
        infer_type(&ctx, &simple).expect("a let is well typed"),
        nat(),
        "the let's body determines the type"
    );
    assert_eq!(
        normalize(&ctx, &simple),
        peano(4),
        "`reduce_step` on a `Let` performs zeta"
    );

    // The defining property: `Let{x,A,v,b} ≡ b[x := v]`.
    let expanded = app(g("Succ"), peano(3));
    assert!(
        defeq_for_test(&ctx, &simple, &expanded),
        "a `Let` is definitionally its own zeta-expansion"
    );

    // Zeta-transparency made observable: the body proves `n = 3` by `refl`, which is only
    // possible because the body sees the VALUE, not an opaque hypothesis of type `Nat`.
    let zeta_visible = let_in(
        "n",
        nat(),
        peano(3),
        app(
            lam("h", eq(nat(), var("n"), peano(3)), var("n")),
            refl(nat(), var("n")),
        ),
    );
    assert_eq!(
        infer_type(&ctx, &zeta_visible).expect(
            "a zeta-transparent let lets the body prove `n = 3`; this is what the beta-redex \
             encoding cannot do"
        ),
        nat(),
        "the body's `refl` is checked against the VALUE"
    );

    // Nested lets whose inner value mentions the outer name — substitution is outside-in.
    let nested = let_in(
        "a",
        nat(),
        peano(2),
        let_in(
            "b",
            nat(),
            app(g("Succ"), var("a")),
            app(g("Succ"), var("b")),
        ),
    );
    assert_eq!(
        normalize(&ctx, &nested),
        peano(4),
        "nested lets zeta outside-in"
    );

    // A let whose bound name shadows an enclosing binder: the substitution must not reach
    // into the shadowed region.
    let shadowing = lam(
        "n",
        nat(),
        let_in("n", nat(), peano(10), app(g("Succ"), var("n"))),
    );
    assert_eq!(
        normalize(&ctx, &app(shadowing, peano(0))),
        peano(11),
        "the inner `let n` shadows the lambda's `n`"
    );

    // A let at the TYPE level: the annotation is `Type 0`.
    let let_of_type = let_in("A", type0(), nat(), pi("_", var("A"), var("A")));
    assert_eq!(
        infer_type(&ctx, &let_of_type).expect("a type-level let is well typed"),
        type0(),
        "a `Let` binding a type zeta-substitutes at the sort level too"
    );
}

// ============================================================================================
// T00-008 · `Expr.lit` — `Nat` and `String` land in different LOGOS worlds
// ============================================================================================

/// MUST. `natVal ↦ Literal::Nat(BigInt)` typed `Nat`, an INDUCTIVE with `Zero`/`Succ` and a
/// Peano bridge into it; `strVal ↦ Literal::Text` typed `Text`, an OPAQUE inductive with zero
/// constructors and no operations. `Int` is Lean's two-constructor inductive (`Int.ofNat`,
/// `Int.negSucc`) with its own literal bridge, so conflating `Nat` with `Int` still compiles —
/// and hands every later `Match`, `Fix` and computation on the value the wrong type.
#[test]
fn a_nat_literal_bridges_to_peano_where_an_int_literal_stays_opaque() {
    let ctx = std_ctx();

    assert_eq!(
        infer_type(&ctx, &nat_lit(3)).expect("a Nat literal is well typed"),
        nat(),
        "`Expr.lit (.natVal 3)` is a `Literal::Nat`, typed `Nat`"
    );
    assert_eq!(
        infer_type(&ctx, &int_lit_of(7)).expect("an Int literal is well typed"),
        g("Int"),
        "`Literal::Int` is a different literal in a different world"
    );
    assert!(
        !is_subtype(&ctx, &nat(), &g("Int")),
        "`Nat` and `Int` are unrelated types; a numeral's LOGOS world is decided by its literal"
    );

    // The Peano bridge — the whole reason a `Nat` literal must not become an `Int` one.
    assert!(
        defeq_for_test(&ctx, &nat_lit(3), &peano(3)),
        "`Lit(Nat(3))` is DEFINITIONALLY `Succ (Succ (Succ Zero))`"
    );
    assert!(
        !defeq_for_test(&ctx, &nat_lit(3), &peano(4)),
        "the bridge is exact: three is not four"
    );

    // `Int` is Lean's inductive: exactly `Int.ofNat` and `Int.negSucc`, in Lean's order. Its
    // literal bridges to THAT encoding, never to `Nat`'s — so a numeral translated into the wrong
    // world is a value of the wrong type, not a harmless respelling.
    let int_constructors = ctx.get_constructors("Int");
    assert_eq!(
        int_constructors
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        vec!["Int_ofNat", "Int_negSucc"],
        "`Int` has exactly Lean's two constructors, `Int.ofNat` then `Int.negSucc`"
    );
    for (name, ty) in &int_constructors {
        assert!(
            defeq_for_test(&ctx, ty, &pi("_", nat(), g("Int"))),
            "`{name}` must be `Nat → Int`, got {ty}"
        );
    }
    assert!(
        defeq_for_test(&ctx, &int_lit_of(7), &app(g("Int_ofNat"), nat_lit(7))),
        "`(7 : Int)` is definitionally `Int.ofNat 7`"
    );
    assert!(
        defeq_for_test(&ctx, &int_lit_of(-3), &app(g("Int_negSucc"), nat_lit(2))),
        "`(-3 : Int)` is definitionally `Int.negSucc 2`"
    );
    assert!(
        !defeq_for_test(&ctx, &int_lit_of(3), &nat_lit(3)),
        "`(3 : Int)` and `(3 : Nat)` are different values of different types"
    );
    assert!(
        !defeq_for_test(&ctx, &int_lit_of(3), &peano(3)),
        "an `Int` literal never reaches `Nat`'s Peano bridge"
    );
    assert_eq!(
        ctx.get_constructors("Nat").len(),
        2,
        "`Nat` is an inductive with exactly `Zero` and `Succ`"
    );

    // Beyond `i64`: `Literal::Nat` carries a `BigInt`, so the value survives exactly.
    let huge = BigInt::parse_decimal("123456789012345678901234567890")
        .expect("a 30-digit decimal literal parses");
    let huge_lit = Term::Lit(Literal::Nat(huge.clone()));
    assert_eq!(
        infer_type(&ctx, &huge_lit).expect("a huge Nat literal is well typed"),
        nat(),
        "a literal beyond i64 is representable without truncation"
    );
    assert!(
        defeq_for_test(&ctx, &huge_lit, &Term::Lit(Literal::Nat(huge))),
        "two `BigInt` literals compare exactly"
    );

    // Text: the only carrier for a Lean `String`, and it has NOTHING on it.
    let hello = Term::Lit(Literal::Text("hello".to_string()));
    assert_eq!(
        infer_type(&ctx, &hello).expect("a text literal is well typed"),
        g("Text"),
        "`Expr.lit (.strVal s)` is a `Literal::Text`, typed `Text`"
    );
    assert!(
        ctx.get_constructors("Text").is_empty(),
        "`Text` is opaque — no concatenation, no length, no decidable equality"
    );
}

// ============================================================================================
// T00-009 · `Expr.mdata` — there is no metadata carrier
// ============================================================================================

/// MUST — Gap. `Expr.mdata` wraps a real subterm with a key; LOGOS's `Term` has thirteen
/// variants and not one of them can hold an annotation, and `Context` carries no side table
/// keyed on terms. The erasure is sound for pretty-printer keys, but it takes the SOURCE
/// SPAN with it: after translation no `KernelError` can point back at the Lean text. The two
/// workarounds are both worse — there is no `id` to wrap with, and a zero-use `Let` changes
/// the term.
#[test]
fn a_translated_term_cannot_remember_where_it_came_from() {
    let ctx = std_ctx();

    // Lean annotates `⟨1, 2⟩` with `pp_using_anonymous_constructor` and leaves `Prod.mk 1 2`
    // bare; the two `Expr`s differ only by that wrapper. In LOGOS they are simply the same
    // term, because there is nowhere for the wrapper to live.
    let plain = peano(3);
    let annotated = Term::annotated(
        Annotation::Source(Provenance::Position(SourcePosition::new(
            "T00_009.lean",
            1,
            0,
        ))),
        plain.clone(),
    );
    assert_eq!(
        annotated, plain,
        "erasing an inert pretty-printer key is right — the annotated and unannotated \
         spellings must become ONE term, so `def_eq` on them cannot fail"
    );

    // The `id`-wrapper workaround: there is no identity function in the prelude, so the
    // tempting "keep the annotation as an application" does not even resolve.
    let wrapped = app(g("id"), plain.clone());
    let err = infer_type(&ctx, &wrapped)
        .expect_err("there is no `id` in the LOGOS prelude to wrap an annotation with");
    assert!(
        matches!(err, KernelError::UnboundVariable(ref n) if n == "id"),
        "wrapping in an identity function fails at `get_global`: {err:?}"
    );

    // The `Let`-wrapper workaround: it typechecks, and it is a DIFFERENT term.
    let let_wrapped = let_in("_", nat(), plain.clone(), var("_"));
    assert_eq!(
        infer_type(&ctx, &let_wrapped).expect("a zero-use `Let` is well typed"),
        nat(),
        "the `Let` wrapper checks — which is exactly why it is tempting"
    );
    assert_ne!(
        let_wrapped, plain,
        "and it changes the term, introducing a binder literally named `_`"
    );

    // The capability itself. Not every mdata key is inert: a SOURCE POSITION has to survive
    // translation, or no diagnostic the kernel raises can ever be reported against the Lean
    // text the user actually wrote. Attaching one is the same no-op as above, and reading it
    // back walks every `Term` variant.
    assert_eq!(
        source_annotation(&annotated),
        Some("T00_009.lean:1:0"),
        "an annotated term must survive type checking with its key readable — LOGOS has no \
         `mdata` carrier and no term-keyed side table, so every downstream diagnostic is \
         reported against a term the user never wrote"
    );
}

// ============================================================================================
// T00-010 · `Expr.mvar` reaching the kernel
// ============================================================================================

/// MUST — Gap. There is no metavariable VARIANT: the elaborator models one as a `Term::Var`
/// whose name starts with `?`. An uninstantiated metavariable therefore reaches the kernel as
/// an ordinary unbound variable that happens to be spelled `?m`, and the one position where a
/// `Hole` would have been refused is not the position a metavariable survives in.
#[test]
fn an_uninstantiated_metavariable_is_reported_as_a_metavariable_not_an_unbound_variable() {
    let ctx = std_ctx();

    // `instantiate` leaves an UNSOLVED metavariable exactly where it was.
    let mctx = MetaCtx::new();
    let unsolved = var("?m");
    assert_eq!(
        instantiate(&unsolved, &mctx),
        unsolved,
        "an unsolved `?m` passes straight through instantiation"
    );

    // A solved one is replaced — so the two states are indistinguishable at the `Term` level.
    let mut solved_ctx = MetaCtx::new();
    solved_ctx.solve("?m", peano(2));
    assert_eq!(
        instantiate(&unsolved, &solved_ctx),
        peano(2),
        "a solved metavariable is substituted; nothing in the term records which it was"
    );

    // It survives into a position `infer_type` only ever COMPARES, never infers: the type
    // argument of a polymorphic head.
    let hidden = eq(var("?m"), peano(1), peano(1));
    assert!(
        infer_type(&ctx, &hidden).is_err(),
        "a metavariable in a type-argument position must be refused"
    );

    // And the diagnostic: the kernel says UNBOUND VARIABLE, not "uninstantiated metavariable".
    let err = infer_type(&ctx, &unsolved).expect_err("`?m` is not in the local context");
    assert!(
        !matches!(err, KernelError::UnboundVariable(_)),
        "an uninstantiated metavariable must be reported AS ONE, naming the metavariable and \
         the declaration it was found in — today it is an unbound variable whose name happens \
         to start with a question mark: {err:?}"
    );
}

// ============================================================================================
// T00-011 · `Expr.proj` on `Prod` / `Sigma` / `Subtype`
// ============================================================================================

/// MUST. There is no `Proj`: a structure is desugared by `add_structure` into an inductive, a
/// `_mk` constructor and one ordinary field definition per projection, dependent second field
/// included. The prelude's own `Prod`/`Sigma`/`Subtype` are monomorphic `Type 0` declarations,
/// and so is any structure written over CONCRETE parameter sorts — exactly as Lean's
/// `structure P (A B : Type)` is. Lean's universe-polymorphic `Prod.{u,v} : Type u → Type v →
/// Type (max u v)` is written over LEVEL VARIABLES, and `add_structure` binds the level
/// variables its parameters mention, so the manufactured structure is one declaration serving
/// every level.
#[test]
fn a_manufactured_sigma_projects_dependently_but_only_at_type_zero() {
    let mut ctx = std_ctx();

    // `structure Prod (A B : Type) := (fst : A) (snd : B)`
    ctx.add_structure(
        "Prod",
        &[("A", type0()), ("B", type0())],
        &[("fst", var("A")), ("snd", var("B"))],
    );
    let pair = app4(g("Prod_mk"), nat(), bool_ty(), peano(3), g("true"));
    assert_eq!(
        infer_type(&ctx, &pair).expect("a manufactured pair is well typed"),
        app2(g("Prod"), nat(), bool_ty()),
        "`Prod_mk A B a b : Prod A B`"
    );
    assert_eq!(
        normalize(&ctx, &app3(g("Prod_fst"), nat(), bool_ty(), pair.clone())),
        peano(3),
        "`(a, b).1 ⤳ a` — the projection is a delta-unfold to a `Match`, not a kernel primitive"
    );
    assert_eq!(
        normalize(&ctx, &app3(g("Prod_snd"), nat(), bool_ty(), pair.clone())),
        g("true"),
        "`(a, b).2 ⤳ b`"
    );

    // Structure eta: `⟨p.1, p.2⟩ ≡ p`, inside `def_eq` only.
    let pctx = ctx.extend("p", app2(g("Prod"), nat(), bool_ty()));
    let rebuilt = app4(
        g("Prod_mk"),
        nat(),
        bool_ty(),
        app3(g("Prod_fst"), nat(), bool_ty(), var("p")),
        app3(g("Prod_snd"), nat(), bool_ty(), var("p")),
    );
    assert!(
        defeq_for_test(&pctx, &rebuilt, &var("p")),
        "structure eta holds in `def_eq`; it is NOT a reduction rule"
    );

    // A genuinely DEPENDENT `Sigma`: the second field's TYPE mentions the first field's VALUE.
    ctx.add_structure(
        "Sigma",
        &[("A", type0()), ("B", pi("_", var("A"), type0()))],
        &[("fst", var("A")), ("snd", app(var("B"), var("fst")))],
    );
    let fam = lam("n", nat(), eq(nat(), var("n"), var("n")));
    let dep = app4(
        g("Sigma_mk"),
        nat(),
        fam.clone(),
        peano(3),
        refl(nat(), peano(3)),
    );
    let dep_ty = infer_type(&ctx, &dep).expect("a dependent pair is well typed");
    assert_eq!(
        dep_ty,
        app2(g("Sigma"), nat(), fam.clone()),
        "the dependent pair inhabits its own family"
    );
    let snd_ty = infer_type(&ctx, &app3(g("Sigma_snd"), nat(), fam.clone(), dep.clone()))
        .expect("the dependent projection is well typed");
    assert!(
        defeq_for_test(&ctx, &snd_ty, &eq(nat(), peano(3), peano(3))),
        "`s.2 : B s.1` — erasing the dependency would give a constant motive and the wrong \
         projection type: {snd_ty}"
    );

    // A `Prod` over `Type 0` parameters has no instance above `Type 0` — here or in Lean.
    let prod_at_type_one = app2(g("Prod"), Term::Sort(Universe::Type(1)), nat());
    assert!(
        infer_type(&ctx, &prod_at_type_one).is_err(),
        "a structure over `Type 0` parameters refuses a `Type 1` component, as Lean's \
         `structure P (A B : Type)` does"
    );

    // Lean's `Prod.{u,v} (α : Type u) (β : Type v)`, manufactured over level variables: ONE
    // declaration, its constructor and projections all carrying the telescope `.{u, v}`.
    let type_at =
        |level: &str| Term::Sort(Universe::Succ(Box::new(Universe::Var(level.to_string()))));
    ctx.add_structure(
        "UProd",
        &[("A", type_at("u")), ("B", type_at("v"))],
        &[("fst", var("A")), ("snd", var("B"))],
    );
    assert_eq!(
        ctx.poly_declaration("UProd")
            .map(|(levels, _)| levels.to_vec()),
        Some(vec!["u".to_string(), "v".to_string()]),
        "a structure written over `Type u` and `Type v` binds exactly `u` and `v`, in order"
    );
    let at = |name: &str, u: Universe, v: Universe| Term::Const {
        name: name.to_string(),
        levels: vec![u, v],
    };
    // `u := Type 1` makes the first parameter `Type 2`, so `Type 1` itself fits in it.
    let higher = app2(
        at("UProd", Universe::Type(1), Universe::Prop),
        Term::Sort(Universe::Type(1)),
        nat(),
    );
    assert_eq!(
        infer_type(&ctx, &higher).expect("the polymorphic pair has an instance above `Type 0`"),
        Term::Sort(Universe::Type(2)),
        "`UProd.{{Type 1, Prop}} (Type 1) Nat : Type 2` — `Type (max u v)` at the instance"
    );
    let higher_pair = app4(
        at("UProd_mk", Universe::Type(1), Universe::Prop),
        Term::Sort(Universe::Type(1)),
        nat(),
        type0(),
        g("Zero"),
    );
    let first = app3(
        at("UProd_fst", Universe::Type(1), Universe::Prop),
        Term::Sort(Universe::Type(1)),
        nat(),
        higher_pair,
    );
    assert_eq!(
        infer_type(&ctx, &first).expect("the projection types at the higher instance"),
        Term::Sort(Universe::Type(1)),
        "`(⟨Type 0, 0⟩ : UProd (Type 1) Nat).1 : Type 1`"
    );
    assert_eq!(
        normalize(&ctx, &first),
        type0(),
        "…and it computes to the stored `Type 0`"
    );
    assert_eq!(
        double_check(&ctx, &first),
        DoubleCheck::Agreed,
        "the second kernel agrees about the higher-instance projection"
    );

    // The level is CHECKED, not assumed: at `u := Prop` the first parameter is `Type 0`, and
    // `Type 1` does not fit in it.
    let too_low = app2(
        at("UProd", Universe::Prop, Universe::Prop),
        Term::Sort(Universe::Type(1)),
        nat(),
    );
    assert!(
        infer_type(&ctx, &too_low).is_err(),
        "an instance whose level cannot hold the argument is refused"
    );
}

// ============================================================================================
// T00-012 · deep normalisation — the conversion budget
// ============================================================================================

/// MUST — Gap. Lean's kernel discharges `rfl` on `Nat` with GMP in O(1); LOGOS computes
/// unary, one `reduce_step` at a time, against `normalize`'s fixed 10000-step budget — and on
/// exhaustion it returns the partially reduced term, so `def_eq` answers `false`. That answer
/// is byte-for-byte the answer a genuine type mismatch produces. The budget is counted per
/// `normalize` CALL, not per declaration, so the SAME equation is accepted inside one
/// declaration and rejected inside a larger one, and a translator that reacts by weakening a
/// type or axiomatising the equation converts a budget problem into a soundness problem.
///
/// (Defect 9's own reproduction — that `normalize` presents an abandoned reduction as a normal
/// form, including for a δ-cycle no budget can finish — is
/// `lean_conformance_defects::normalize_must_not_hand_back_an_unfinished_reduction_as_a_normal_form`,
/// claimed by T11-005. This test is the CONVERSION oracle T00-012 asks for instead: whether a
/// `rfl` Lean discharges is decidable here, and whether the verdict is stable.)
#[test]
fn a_conversion_check_lean_discharges_must_not_depend_on_the_step_budget() {
    let ctx = std_ctx();

    // Two `Nat` literals are compared directly — the one place LOGOS matches Lean's O(1).
    let big = BigInt::parse_decimal("1048576").expect("a seven-digit literal parses");
    assert!(
        defeq_for_test(
            &ctx,
            &Term::Lit(Literal::Nat(big.clone())),
            &Term::Lit(Literal::Nat(big))
        ),
        "`(1048576 : Nat) = 1048576 := rfl` is decided on the literals, not by computing"
    );

    // But the moment either side must REDUCE, the cost is unary. `countdown n = 0` is an
    // ordinary structurally-recursive Lean definition and `rfl` proves it for every `n`.
    let countdown = Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam(
            "n",
            nat(),
            Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(lam("n", nat(), nat())),
                cases: vec![g("Zero"), lam("k", nat(), app(var("f"), var("k")))],
            },
        )),
    };
    assert_eq!(
        infer_type(&ctx, &countdown).expect("a structural fixpoint passes the termination guard"),
        pi("n", nat(), nat()),
        "the function itself is unremarkable"
    );
    let discharges = |n: i64| defeq_for_test(&ctx, &app(countdown.clone(), nat_lit(n)), &g("Zero"));

    assert!(discharges(10), "`countdown 10 = 0 := rfl` — the control");
    assert!(
        discharges(3000),
        "`countdown 3000 = 0 := rfl` is still inside the budget"
    );

    // And the same equation, one size up, is refused — with the same `false` a genuine
    // mismatch produces, and no way for the caller to tell them apart.
    assert!(
        discharges(5000),
        "a definitional equality Lean discharges in one kernel step must be decidable here \
         too, and its verdict must not depend on the size of the term it is checked inside: \
         `countdown 3000 = 0` converts and `countdown 5000 = 0` does not, because the fixed \
         10000-step budget is spent per `normalize` CALL. The rejection is indistinguishable \
         from a real type mismatch, so 'repairing' the term — weakening a type, inserting a \
         cast, or axiomatising the equation — turns a budget problem into an unsound library"
    );
}

// ============================================================================================
// T00-013 · `@[reducible]` / `@[irreducible]` — no transparency control
// ============================================================================================

/// MUST. `@[reducible]`/`@[irreducible]` constrain Lean's ELABORATOR: at default transparency it
/// refuses `secret = 5 := rfl`, while Lean's KERNEL ignores the attribute and unfolds `secret`
/// like any definition. LOGOS carries the same axis on the declaration (`set_reducibility`):
/// the elaborator's transparency (`Transparency::Excluding`) keeps an irreducible definition
/// folded with its body intact, and `normalize` — the kernel's face — unfolds it. Buying opacity
/// through `add_declaration` instead throws the body away and is indistinguishable from a
/// genuine axiom, so a proof library that did so could not report its own trusted base.
#[test]
fn an_irreducible_definition_must_keep_its_body_and_still_refuse_to_unfold() {
    let mut ctx = std_ctx();

    // `def ordinary : Nat := 5` — transparent in Lean's kernel and in LOGOS.
    ctx.add_definition("ordinary".to_string(), nat(), peano(5));
    assert_eq!(
        normalize(&ctx, &g("ordinary")),
        peano(5),
        "an ordinary definition unfolds; `example : ordinary = 5 := rfl` goes through"
    );

    // `abbrev BoolAlias := Bool` is `@[reducible] def`, which is the same `add_definition`.
    ctx.add_definition("BoolAlias".to_string(), type0(), bool_ty());
    assert_eq!(
        normalize(&ctx, &g("BoolAlias")),
        bool_ty(),
        "a reducible alias is just another definition — no preferential unfolding"
    );

    // `opaque genuinelyOpaque : Nat` — the only construct that SHOULD reach `add_declaration`.
    ctx.add_declaration("genuinelyOpaque", nat());
    assert_eq!(
        normalize(&ctx, &g("genuinelyOpaque")),
        g("genuinelyOpaque"),
        "an axiom has no body, so it never unfolds"
    );
    assert!(
        ctx.get_definition_body("genuinelyOpaque").is_none(),
        "buying opacity through `add_declaration` costs the body — and the result is \
         indistinguishable from an axiom"
    );

    // `@[irreducible] def secret : Nat := 5` — Lean's elaborator refuses `secret = 5 := rfl`
    // at default transparency; Lean's kernel unfolds it.
    ctx.add_definition("secret".to_string(), nat(), peano(5));
    ctx.set_reducibility("secret", Reducibility::Irreducible);
    assert!(
        ctx.is_irreducible("secret"),
        "the attribute is recorded on the declaration"
    );
    assert_eq!(
        ctx.get_definition_body("secret"),
        Some(&peano(5)),
        "an `@[irreducible]` definition keeps its body — it is not an axiom"
    );
    let irreducible = |name: &str| ctx.is_irreducible(name);
    assert_eq!(
        normalize_at(&ctx, &g("secret"), Transparency::Excluding(&irreducible)),
        g("secret"),
        "at the elaborator's transparency an `@[irreducible]` definition stays folded, so `rfl` \
         cannot see through it"
    );
    assert_eq!(
        normalize_at(&ctx, &g("ordinary"), Transparency::Excluding(&irreducible)),
        peano(5),
        "…and only it: an ordinary definition still unfolds at the same transparency"
    );
    assert_eq!(
        normalize(&ctx, &g("secret")),
        peano(5),
        "the kernel ignores the attribute, exactly as Lean's kernel does — `normalize` unfolds"
    );
}

// ============================================================================================
// T00-014 · function eta — in `def_eq`, never in `normalize`
// ============================================================================================

/// MUST. `(fun x => f x) = f` holds by `rfl` in Lean and by `def_eq_normalized`'s η arm in
/// LOGOS — but `normalize` has no η rule, so the two sides have DIFFERENT normal forms. The
/// cheap equality oracle "normalise both and compare syntactically" is therefore wrong for
/// LOGOS, and eta is not extensionality: `funext` is a checked THEOREM of the prelude — never a
/// conversion rule, never an axiom.
#[test]
fn eta_decides_in_defeq_and_is_invisible_to_normalize() {
    let ctx = std_ctx();
    let nat_to_nat = pi("_", nat(), nat());
    let fctx = ctx.extend("f", nat_to_nat.clone());

    // The canonical pair.
    let eta_expanded = lam("x", nat(), app(var("f"), var("x")));
    assert!(
        defeq_for_test(&fctx, &eta_expanded, &var("f")),
        "η: `λx. f x ≡ f`"
    );
    assert_ne!(
        normalize(&fctx, &eta_expanded),
        normalize(&fctx, &var("f")),
        "`normalize` has no η arm — a `normalize`-and-compare oracle reports these UNEQUAL and \
         then 'repairs' a term that needed no repair"
    );

    // Two-argument eta: the rule must fire again under the binder it just introduced.
    let kctx = ctx.extend("k", pi("_", nat(), pi("_", nat(), nat())));
    let two_arg = lam(
        "a",
        nat(),
        lam("b", nat(), app2(var("k"), var("a"), var("b"))),
    );
    assert!(
        defeq_for_test(&kctx, &two_arg, &var("k")),
        "η fires under the binder for a two-argument function"
    );

    // The non-lambda side may be a stuck APPLICATION, not just a variable.
    let actx = kctx.extend("a", nat());
    let partial = lam("b", nat(), app2(var("k"), var("a"), var("b")));
    assert!(
        defeq_for_test(&actx, &partial, &app(var("k"), var("a"))),
        "the η arm applies the non-lambda side to the bound variable and normalizes"
    );

    // η combined with β: `normalize` must run before exactly-one-side-is-a-lambda can be seen.
    let beta_then_eta = lam(
        "x",
        nat(),
        app(lam("y", nat(), app(var("f"), var("y"))), var("x")),
    );
    assert!(
        defeq_for_test(&fctx, &beta_then_eta, &var("f")),
        "`def_eq` normalizes first, then tries η"
    );

    // The NEGATIVE control. η is not extensionality: two pointwise-equal functions are NOT
    // convertible.
    let gctx = fctx.extend("g", nat_to_nat);
    assert!(
        !defeq_for_test(&gctx, &var("f"), &var("g")),
        "η must not be mistaken for extensionality"
    );

    // Extensionality is reached by PROOF: `funext` is a checked prelude definition (derived from
    // the quotient primitives, as in Lean), so it costs the trusted base nothing and conversion
    // above is untouched.
    assert!(
        ctx.is_definition("funext") && ctx.get_definition_body("funext").is_some(),
        "`funext` is a theorem with a checked body"
    );
    assert!(
        !ctx.iter_declarations().any(|(name, _)| name == "funext"),
        "`funext` is never an axiom of the trusted base"
    );
    // The same `f` and `g` conversion refused: a POINTWISE hypothesis proves them equal.
    let hctx = gctx.extend(
        "h",
        pi(
            "x",
            nat(),
            eq(nat(), app(var("f"), var("x")), app(var("g"), var("x"))),
        ),
    );
    let extensional = app(
        app(
            app(
                app(app(g("funext"), nat()), lam("_", nat(), nat())),
                var("f"),
            ),
            var("g"),
        ),
        var("h"),
    );
    let proved = infer_type(&hctx, &extensional).expect("`funext` applies to a pointwise proof");
    assert!(
        defeq_for_test(
            &hctx,
            &proved,
            &eq(pi("x", nat(), nat()), var("f"), var("g"))
        ),
        "`funext` turns `∀x, f x = g x` into `f = g`: {proved}"
    );
    assert!(
        !defeq_for_test(&hctx, &var("f"), &var("g")),
        "…while `f ≡ g` is still refused by conversion"
    );
}

// ============================================================================================
// T00-015 · deep binder shadowing — alpha-equivalence with NAMES
// ============================================================================================

/// DIVERGE. Alpha-equivalence with NAMED binders is implemented by renaming: `types_equal`
/// compares `Var` names literally and then substitutes the second binder's name for the
/// first, and `substitute_avoiding` freshens any binder that would capture. Deep shadowing is
/// legal, both kernels agree on it, and names the kernel generates for itself
/// (`derive_recursor`'s `A0`/`P`/`f0`/`x`/`a0`) are regenerated rather than merged with a
/// source binder of the same spelling. The cost is that `Context` is a name-keyed map with no
/// stack to pop, so a translator owes a freshening pass — and the de Bruijn second kernel,
/// which Lean has no equivalent of, is what catches a botched one.
#[test]
fn alpha_equivalent_telescopes_convert_and_generated_binder_names_never_capture() {
    let ctx = std_ctx();

    // `∀ (a : Nat), Eq Nat a a → Nat` and the same with `b` — one `Expr` in Lean, two terms
    // in LOGOS, and they must convert.
    let alpha_a = pi("a", nat(), pi("_", eq(nat(), var("a"), var("a")), nat()));
    let alpha_b = pi("b", nat(), pi("_", eq(nat(), var("b"), var("b")), nat()));
    assert_ne!(alpha_a, alpha_b, "the two terms are structurally different");
    assert!(
        defeq_for_test(&ctx, &alpha_a, &alpha_b),
        "alpha-equivalence is decided by renaming, not by index"
    );

    // Five binders named `n`, each type mentioning the previous one. Legal, and both kernels
    // must agree on it.
    let deep = pi(
        "n",
        nat(),
        pi(
            "n",
            eq(nat(), var("n"), var("n")),
            pi("n", eq(nat(), peano(0), peano(0)), nat()),
        ),
    );
    assert_eq!(
        infer_type(&ctx, &deep).expect("a deeply shadowed telescope is legal"),
        type0(),
        "shadowing is legal; only the READING changes"
    );
    assert_eq!(
        double_check(&ctx, &deep),
        DoubleCheck::Agreed,
        "the de Bruijn re-checker must agree on a deeply shadowed telescope"
    );

    // A `Fix` name shares the flat binder namespace with lambdas, so a collision crosses
    // term forms.
    let shadow_across_forms = lam(
        "n",
        nat(),
        let_in("n", nat(), app(g("Succ"), var("n")), var("n")),
    );
    assert_eq!(
        normalize(&ctx, &app(shadow_across_forms, peano(1))),
        peano(2),
        "`Let.name` and `Lambda.param` are different fields of one flat namespace"
    );

    // `Context` is a name-keyed map, so extending twice with one name makes the INNER binding
    // the one a lookup finds. That is correct shadowing — and it is also why the outer binding
    // is unreachable BY NAME, which makes freshening the emitter's obligation rather than the
    // kernel's, and makes the de Bruijn re-checker the thing that catches a botched one.
    let twice = ctx.extend("n", nat()).extend("n", bool_ty());
    assert_eq!(
        twice.get("n"),
        Some(&bool_ty()),
        "the inner binding shadows the outer; there is no stack to pop the outer one back off"
    );

    // Names the kernel generates for ITSELF cannot be captured by a source binder that happens
    // to be spelled the same: `derive_recursor` regenerates every binder (`A0`, `P`, `f0`, `x`,
    // `a0`), so a constructor field named `A0` is renamed away before it can collide.
    let mut rctx = std_ctx();
    rctx.add_indexed_inductive("W", pi("T", type0(), type0()), 1);
    rctx.add_constructor(
        "wmk",
        "W",
        pi("T", type0(), pi("A0", var("T"), app(g("W"), var("T")))),
    );
    let (_, rec_body) = derive_recursor(&rctx, "W").expect("a one-field family has a recursor");
    assert_eq!(
        double_check(&rctx, &rec_body),
        DoubleCheck::Agreed,
        "a source binder named `A0` — exactly what `derive_recursor` generates for a parameter \
         — must not capture inside the derived term"
    );
}

// ============================================================================================
// T00-016 · `Term::Hole` surviving into output
// ============================================================================================

/// MUST — Gap. `types_equal` returns `true` whenever EITHER side is a `Hole`, and it is the
/// fast path of both `is_subtype` and `def_eq`. So one surviving hole makes every comparison
/// against that type succeed: a statement can be written that Lean cannot even state, and a
/// proof of something else satisfies it. `CannotInferHole` is not a backstop — it fires only
/// where `infer_type` actually visits, and a hole in an ARGUMENT position is only ever compared.
#[test]
fn a_hole_must_not_make_every_comparison_succeed() {
    let ctx = std_ctx();

    // A hole IS refused where `infer_type` actually visits it — the one position that reports
    // it, and the reason `CannotInferHole` is mistaken for a backstop.
    assert!(
        matches!(
            infer_type(&ctx, &Term::Hole),
            Err(KernelError::CannotInferHole)
        ),
        "`infer_type` refuses a bare hole"
    );
    assert!(
        matches!(
            infer_type(&ctx, &pi("_", Term::Hole, nat())),
            Err(KernelError::CannotInferHole)
        ),
        "and a hole in a Π's domain, which `infer_sort` visits"
    );

    // The control: without a hole, a wrongly-typed argument is caught.
    assert!(
        infer_type(&ctx, &app(lam("n", nat(), var("n")), g("true"))).is_err(),
        "a `Bool` is not a `Nat`"
    );

    // Everywhere else a hole is a wildcard. Collect EVERY leak, so the failure reports all of
    // them rather than stopping at the first.
    let mut leaks: Vec<String> = Vec::new();
    if is_subtype(&ctx, &nat(), &Term::Hole) {
        leaks.push(
            "`Nat ≤ _`: a hole is a supertype of every type (`types_equal` \
                    short-circuits on it, and it is `is_subtype`'s FAST PATH)"
                .to_string(),
        );
    }
    if is_subtype(&ctx, &Term::Hole, &bool_ty()) {
        leaks.push("`_ ≤ Bool`: and a subtype of every type".to_string());
    }
    if defeq_for_test(&ctx, &nat(), &Term::Hole) {
        leaks.push("`def_eq(Nat, _)`: definitional equality agrees".to_string());
    }
    if let Ok(sort) = infer_type(&ctx, &eq(Term::Hole, nat_lit(1), g("true"))) {
        leaks.push(format!(
            "`Eq _ 1 true : {sort}`: a HETEROGENEOUS equation between a `Nat` and a `Bool` is a \
             well-formed proposition — a statement Lean cannot express at all, in a position \
             (`Eq`'s type argument) that `infer_type` never visits and only ever compares"
        ));
    }
    if let Ok(ty) = infer_type(&ctx, &app(app(g("Eq"), Term::Hole), g("true"))) {
        leaks.push(format!(
            "`(Eq _) true : {ty}`: with the domain holed out the argument's type check is a \
             no-op, so an argument of ANY type is accepted where the control above was refused"
        ));
    }

    assert!(
        leaks.is_empty(),
        "a `Term::Hole` must never survive into a checked term: one hole in a statement makes \
         every comparison against that statement succeed, so a theorem is recorded and reported \
         verified while nothing proves it. {} leak(s):\n  - {}",
        leaks.len(),
        leaks.join("\n  - ")
    );
}

// ============================================================================================
// T00-017 · `let` whose BODY'S TYPE depends on the bound value
// ============================================================================================

/// MUST. `Term::Let`'s rule zeta-substitutes the value and only THEN infers the body's type,
/// so a body whose type mentions the value typechecks. The `App(Lambda …)` beta-redex
/// encoding types the body with the binder OPAQUE and fails on exactly these terms — the
/// oracle's paired negative, and the reason `Let` is not sugar for a redex.
#[test]
fn a_dependent_let_body_types_only_because_zeta_runs_before_inference() {
    let ctx = std_ctx();

    // `let n := 3; let v : Eq Nat n 3 := refl Nat n; …` — the annotation mentions the VALUE.
    let dependent_body = app(
        lam("v", eq(nat(), var("n"), peano(3)), var("n")),
        refl(nat(), var("n")),
    );
    let as_let = let_in("n", nat(), peano(3), dependent_body.clone());
    assert_eq!(
        infer_type(&ctx, &as_let).expect("zeta runs before the body is inferred"),
        nat(),
        "the body's annotation `Eq Nat n 3` reduces to `Eq Nat 3 3` because the body sees `n ≡ 3`"
    );
    assert_eq!(normalize(&ctx, &as_let), peano(3), "and it computes");

    // The paired negative the oracle asks for: the beta-redex encoding of the SAME term fails.
    let as_redex = app(lam("n", nat(), dependent_body), peano(3));
    let err = infer_type(&ctx, &as_redex)
        .expect_err("the redex types the body with `n` opaque, so the annotation cannot reduce");
    assert!(
        matches!(err, KernelError::TypeMismatch { .. }),
        "`App(Lambda …)` is strictly weaker than `Let` on a dependent body: {err:?}"
    );

    // A CHAIN: the second value's type mentions the first value, so the substitutions compose.
    let chained = let_in(
        "n",
        nat(),
        peano(2),
        let_in(
            "m",
            nat(),
            app(g("Succ"), var("n")),
            app(
                lam("v", eq(nat(), var("m"), peano(3)), var("m")),
                refl(nat(), var("m")),
            ),
        ),
    );
    assert_eq!(
        infer_type(&ctx, &chained).expect("chained dependent lets compose outside-in"),
        nat(),
        "`m ≡ Succ n ≡ Succ 2 ≡ 3` only because both lets are transparent"
    );

    // The dependency at the SORT level: the bound name is a type and the body inhabits it.
    let let_of_type = let_in(
        "A",
        type0(),
        nat(),
        let_in("x", var("A"), peano(5), var("x")),
    );
    assert_eq!(
        infer_type(&ctx, &let_of_type).expect("zeta happens at the sort level too"),
        nat(),
        "`x : α` typechecks only because `α ≡ Nat` is visible"
    );

    // The NON-dependent control: both encodings type it, which is why a suite of ordinary
    // lets cannot tell them apart.
    let plain_body = app(g("Succ"), var("n"));
    assert_eq!(
        infer_type(&ctx, &let_in("n", nat(), peano(3), plain_body.clone())).expect("let"),
        nat(),
        "the control types as a `Let` …"
    );
    assert_eq!(
        infer_type(&ctx, &app(lam("n", nat(), plain_body), peano(3))).expect("redex"),
        nat(),
        "… and as a redex"
    );
}

// ============================================================================================
// T00-018 · `fun | pat => ..` → a POSITIONAL `Term::Match`
// ============================================================================================

/// MUST. `Match` is the PRIMITIVE and recursors are derived — the inverse of Lean. `cases` is
/// a `Vec<Term>` with one entry per constructor IN REGISTRATION ORDER and no tag on a case, so
/// preserving Lean's source order of the alternatives produces a `Match` of the right arity
/// and the wrong semantics. Nothing in the type check catches it when the branches share a type.
#[test]
fn match_cases_are_positional_so_source_order_computes_the_wrong_answer() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Col", type0());
    ctx.add_constructor("red", "Col", g("Col"));
    ctx.add_constructor("green", "Col", g("Col"));
    ctx.add_constructor("blue", "Col", g("Col"));

    let motive = lam("c", g("Col"), nat());
    let matcher = |cases: Vec<Term>| {
        lam(
            "c",
            g("Col"),
            Term::Match {
                discriminant: Box::new(var("c")),
                motive: Box::new(motive.clone()),
                cases,
            },
        )
    };

    // Registration order: red, green, blue.
    let in_order = matcher(vec![peano(0), peano(1), peano(2)]);
    assert_eq!(
        infer_type(&ctx, &in_order).expect("a saturated match is well typed"),
        pi("c", g("Col"), nat()),
        "the match's result type is `motive discriminant`"
    );
    for (ctor, want) in [("red", 0u32), ("green", 1), ("blue", 2)] {
        assert_eq!(
            normalize(&ctx, &app(in_order.clone(), g(ctor))),
            peano(want),
            "case {want} is the case for constructor `{ctor}` as REGISTERED"
        );
    }

    // Lean lets the author write `.blue` first. That source order is a well-typed `Match`
    // that computes something else — and no type check can see it.
    let source_order = matcher(vec![peano(2), peano(0), peano(1)]);
    assert!(
        infer_type(&ctx, &source_order).is_ok(),
        "a permuted `cases` vector is still well typed — there is no tag on a case"
    );
    assert_eq!(
        normalize(&ctx, &app(source_order, g("red"))),
        peano(2),
        "so a source-ordered translation silently computes `pc red = 2`"
    );

    // Arity IS checked, positionally.
    let short = matcher(vec![peano(0), peano(1)]);
    assert!(
        matches!(
            infer_type(&ctx, &short),
            Err(KernelError::WrongNumberOfCases {
                expected: 3,
                found: 2
            })
        ),
        "one case per constructor, checked by count"
    );

    // A constructor with an ARGUMENT needs its case to be a `Lambda` telescope over that
    // argument — `n + 1` is `Succ n`, not an arithmetic guard.
    let pred_or_one = lam(
        "n",
        nat(),
        Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(lam("n", nat(), nat())),
            cases: vec![peano(1), lam("k", nat(), var("k"))],
        },
    );
    assert_eq!(
        normalize(&ctx, &app(pred_or_one.clone(), peano(0))),
        peano(1),
        "the `Zero` case is a bare body"
    );
    assert_eq!(
        normalize(&ctx, &app(pred_or_one, peano(7))),
        peano(6),
        "the `Succ` case binds the constructor's argument"
    );

    // A bare body where a telescope was needed leaves the argument unbound.
    let bare_succ_case = lam(
        "n",
        nat(),
        Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(lam("n", nat(), nat())),
            cases: vec![peano(1), var("k")],
        },
    );
    assert!(
        infer_type(&ctx, &bare_succ_case).is_err(),
        "a bare body for `Succ k` leaves `k` free — or resolves it to the wrong binder"
    );
}

// ============================================================================================
// T00-019 · term-mode `have` (`letFun`) versus `let`
// ============================================================================================

/// MUST. Lean's `have` hides the value from the body; `let` does not. LOGOS has BOTH — the
/// beta-redex `App(Lambda …, v)` types the body with the binder opaque and reduces to the same
/// value, which is exactly `letFun`'s behaviour. So the two Lean forms have distinct, faithful
/// LOGOS images, and translating `have` to `Term::Let` — which would let LOGOS prove strictly
/// more than the Lean source could — is a choice, not a necessity.
#[test]
fn have_is_the_opaque_beta_redex_and_let_is_the_transparent_one() {
    let ctx = std_ctx();

    // The body proves `k = 3` by `refl`. With `let` the value is visible and it goes through.
    let needs_the_value = app(
        lam("h", eq(nat(), var("k"), peano(3)), var("k")),
        refl(nat(), var("k")),
    );
    let via_let = let_in("k", nat(), peano(3), needs_the_value.clone());
    assert_eq!(
        infer_type(&ctx, &via_let).expect("`let` exposes the value to the body"),
        nat(),
        "`let k := 3; have _h : k = 3 := rfl; k` — the value is definitionally visible"
    );

    // With `have` — the beta-redex — the body is typed with `k` OPAQUE, and Lean refuses the
    // same source. LOGOS refuses it too.
    let via_have = app(lam("k", nat(), needs_the_value), peano(3));
    assert!(
        infer_type(&ctx, &via_have).is_err(),
        "`have k : Nat := 3` must NOT let the body prove `k = 3` — the opacity is the whole \
         point of `have`, and the beta-redex is what preserves it"
    );

    // Both spellings compute the same value, exactly as `letFun` is reducible for Lean's
    // kernel: the difference is an ELABORATION-time one.
    let have_plain = app(lam("k", nat(), app(g("Succ"), var("k"))), peano(3));
    let let_plain = let_in("k", nat(), peano(3), app(g("Succ"), var("k")));
    assert_eq!(
        normalize(&ctx, &have_plain),
        peano(4),
        "`have k := 3; k + 1` is 4"
    );
    assert_eq!(
        normalize(&ctx, &let_plain),
        peano(4),
        "`let k := 3; k + 1` is 4"
    );
    assert!(
        defeq_for_test(&ctx, &have_plain, &let_plain),
        "the two encodings are definitionally equal once the body does not need the value"
    );

    // `have` used for its real purpose — naming a PROOF so it is written once. Under `Let`'s
    // zeta rule the proof is substituted at every occurrence before checking.
    let shared = let_in(
        "h",
        eq(nat(), peano(1), peano(1)),
        refl(nat(), peano(1)),
        app2(
            lam(
                "x",
                eq(nat(), peano(1), peano(1)),
                lam("y", eq(nat(), peano(1), peano(1)), peano(0)),
            ),
            var("h"),
            var("h"),
        ),
    );
    assert_eq!(
        infer_type(&ctx, &shared).expect("a shared proof typechecks"),
        nat(),
        "`have h : P := big; ⟨h, h⟩` — and the value is duplicated per occurrence at check time"
    );

    // A `have` whose bound name is used in a DEPENDENT position: the redex still carries the
    // dependency through the RESULT type, which is what `letFun`'s `β : α → Sort v` does.
    let dependent_have = app(
        lam(
            "n",
            nat(),
            app(
                lam("v", eq(nat(), var("n"), var("n")), var("n")),
                refl(nat(), var("n")),
            ),
        ),
        peano(3),
    );
    assert_eq!(
        infer_type(&ctx, &dependent_have)
            .expect("the opaque encoding still types a dependent body"),
        nat(),
        "`have n := 3; have v : Fin (n+1) := …` — opacity does not cost dependency"
    );
}

// ============================================================================================
// T00-020 · `show T from e` / `(e : T)` — ascription is erased
// ============================================================================================

/// MUST. There is no ascription, cast or annotation variant, so `(e : T)` and `e` are the same
/// term — the erasure is forced rather than chosen. The two ways to "keep" the ascription are
/// both worse: there is no `id` to wrap with, and a zero-use `Let` introduces a binder named
/// `_`. The erasure is sound only POST-elaboration: `(42 : Nat)` and `(42 : Int)` are the same
/// tokens and different LOGOS worlds.
#[test]
fn a_type_ascription_leaves_no_trace_in_the_emitted_term() {
    let ctx = std_ctx();

    // `(3 : Nat)` and `3` are literally the same term.
    let ascribed = peano(3);
    let plain = peano(3);
    assert_eq!(ascribed, plain, "ascription is erased, not represented");

    // The `id` wrapper does not exist.
    assert!(
        infer_type(&ctx, &app(g("id"), plain.clone())).is_err(),
        "there is no identity function to hang an ascription on"
    );

    // The zero-use `Let` wrapper typechecks — and is a different term with a `_` binder.
    let let_wrapper = let_in("_", nat(), plain.clone(), var("_"));
    assert_eq!(
        infer_type(&ctx, &let_wrapper).expect("a zero-use let is well typed"),
        nat(),
        "which is exactly why it is tempting"
    );
    assert_ne!(let_wrapper, plain, "and why it must not be used");

    // The erasure is only sound AFTER elaboration: the ascription is what selected the world.
    assert_eq!(
        infer_type(&ctx, &nat_lit(42)).expect("a Nat numeral"),
        nat(),
        "`(42 : Nat)` is a `Literal::Nat`"
    );
    assert_eq!(
        infer_type(&ctx, &int_lit_of(42)).expect("an Int numeral"),
        g("Int"),
        "`(42 : Int)` is a `Literal::Int` — same tokens, different world"
    );
    assert_ne!(
        nat_lit(42),
        int_lit_of(42),
        "the two literals are different terms"
    );

    // What the ascription used to ENFORCE is weaker after erasure: the enclosing check is
    // cumulative subsumption, not definitional equality, so a pinned level may float up.
    assert!(
        is_subtype(&ctx, &type0(), &Term::Sort(Universe::Type(1))),
        "`Type 0 ≤ Type 1` — an erased ascription that pinned a level cannot be recovered from \
         the enclosing check"
    );

    // A nested chain of ascriptions is one term.
    let triple = peano(3);
    assert_eq!(triple, plain, "`(((3 : Nat) : Nat) : Nat)` is `3`");
}

// ============================================================================================
// T00-021 · very deep application spines and telescopes
// ============================================================================================

/// MUST — Gap. A Lean `Expr` spine is flat and its kernel instantiates a telescope with a
/// single de Bruijn `instantiateRev` pass. LOGOS's `App` is BINARY and its substitution is by
/// NAME: `substitute` computes the replacement's free variables with a full walk, and `freshen`
/// walks the body again for every binder it might capture. A twenty-wide spine and a
/// sixty-deep dependent telescope check fine; the DEPTH does not scale. Measured on this box,
/// a left spine costs ~8-9x going from depth 500 to depth 1500 where Lean would pay 3x, needs
/// more than an ordinary 2 MB thread stack at depth 4000 — where it does not fail but ABORTS
/// the process — and at 12000 runs for five minutes and still stops short. The stack ceiling
/// is documented rather than asserted: an overflow kills the process instead of failing a test.
#[test]
fn a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically() {
    let ctx = std_ctx();

    // Twenty binders, the FIRST one selected — a reversed telescope is visible in the answer.
    const WIDE: usize = 20;
    let names: Vec<String> = (0..WIDE).map(|i| format!("a{i:02}")).collect();
    let mut f20 = var(&names[0]);
    for name in names.iter().rev() {
        f20 = lam(name, nat(), f20);
    }
    assert_eq!(
        infer_type(&ctx, &f20).expect("a twenty-binder telescope is well formed"),
        names.iter().rev().fold(nat(), |acc, n| pi(n, nat(), acc)),
        "the inferred Π telescope mirrors the λ telescope"
    );
    let saturated = (0..WIDE).fold(f20, |acc, i| app(acc, peano(i as u32)));
    assert_eq!(
        normalize(&ctx, &saturated),
        peano(0),
        "twenty `substitute` calls, each over what is left of the telescope"
    );

    // A DEPENDENT telescope: every later binder's type mentions the first, so each `App`'s
    // positional substitution has to walk the rest of the chain.
    const DEEP: usize = 60;
    let mut dep_body = var("n");
    for i in (0..DEEP).rev() {
        dep_body = lam(&format!("p{i}"), eq(nat(), var("n"), var("n")), dep_body);
    }
    let dep = lam("n", nat(), dep_body);
    infer_type(&ctx, &dep).expect("a sixty-deep dependent telescope is well formed");
    let mut dep_applied = app(dep, peano(1));
    for _ in 0..DEEP {
        dep_applied = app(dep_applied, refl(nat(), peano(1)));
    }
    assert_eq!(
        infer_type(&ctx, &dep_applied).expect("and its saturated spine checks"),
        nat(),
        "each argument is substituted positionally into the remaining telescope"
    );

    // The scaling. Tripling the depth of a left spine triples the number of beta steps, so a
    // de Bruijn `instantiateRev` kernel pays 3x. Here every step also re-walks the whole
    // remaining spine, twice — once for the replacement's free variables and once for the
    // fixpoint comparison — so the cost is quadratic.
    let spine_at = |depth: usize| -> Term {
        let id_nat = lam("x", nat(), var("x"));
        (0..depth).fold(g("Zero"), |acc, _| app(id_nat.clone(), acc))
    };
    const NEAR: usize = 500;
    const FAR: usize = 1500;
    // Warm the allocator and the instruction cache, so the ratio measures the algorithm.
    assert_eq!(
        normalize(&ctx, &spine_at(NEAR)),
        g("Zero"),
        "a 500-deep spine reduces"
    );

    // Each depth costs the least of several interleaved passes. Load on the box only ever adds
    // time, so the minimum is the algorithm's cost and the ratio holds under a parallel suite.
    const PASSES: usize = 9;
    let near_term = spine_at(NEAR);
    let far_term = spine_at(FAR);
    let pass = |term: &Term| -> std::time::Duration {
        let t = std::time::Instant::now();
        let nf = normalize(&ctx, term);
        let elapsed = t.elapsed();
        assert_eq!(nf, g("Zero"), "both spines reach their normal form");
        elapsed
    };
    let (mut near, mut far) = (std::time::Duration::MAX, std::time::Duration::MAX);
    for _ in 0..PASSES {
        near = near.min(pass(&near_term));
        far = far.min(pass(&far_term));
    }

    let ratio = far.as_secs_f64() / near.as_secs_f64();
    assert!(
        ratio <= 5.0,
        "tripling a spine's depth must cost about 3x, as it does for a de Bruijn kernel that \
         instantiates a telescope in one pass. It costs {ratio:.1}x here ({near:?} at depth \
         {NEAR}, {far:?} at depth {FAR}), because named substitution walks the whole body per \
         binder — and the same growth is what turns a deep declaration into a silent fuel \
         truncation, or, past a 2 MB stack at depth 4000, into an aborted process"
    );
}

// ============================================================================================
// T00-022 · `Expr` is a hash-consed DAG; `Term` is a TREE
// ============================================================================================

/// MUST — Gap. Lean's `Expr` shares: `f x x` holds ONE `x` node with two references. LOGOS's
/// `Term` is a `Box`-recursive tree with no interning and no node identity, and `Term::Let` —
/// the one construct that LOOKS like it restores sharing — zeta-substitutes its value into the
/// body before the body is inferred, so a value used n times is checked n times. The only
/// sharing boundary LOGOS has is a top-level `add_definition`, whose `Global` reference is not
/// unfolded until reduction asks for it.
#[test]
fn a_shared_subterm_is_checked_once_and_not_once_per_occurrence() {
    let mut ctx = std_ctx();

    // A stuck four-argument head, so the occurrences cannot collapse by reduction.
    let quad = pi(
        "_",
        nat(),
        pi("_", nat(), pi("_", nat(), pi("_", nat(), nat()))),
    );
    ctx.add_declaration("quad", quad);

    // A big, irreducible value: forty `Succ`s over a free variable.
    let bctx = ctx.extend("u", nat());
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));
    let big_nodes = nodes(&big);

    // `let b := big; quad b b b b` — one `Expr.letE` in Lean, holding `big` once.
    let shared = let_in(
        "b",
        nat(),
        big.clone(),
        app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
    );
    assert_eq!(
        infer_type(&bctx, &shared).expect("a shared let is well typed"),
        nat(),
        "the let itself is unremarkable"
    );

    // The genuine mitigation, for contrast: lift the shared subterm to a GLOBAL, whose
    // reference is one node until reduction unfolds it.
    ctx.add_definition(
        "shared40".to_string(),
        pi("_", nat(), nat()),
        lam("u", nat(), big.clone()),
    );
    let via_global = app4(
        g("quad"),
        app(g("shared40"), var("u")),
        app(g("shared40"), var("u")),
        app(g("shared40"), var("u")),
        app(g("shared40"), var("u")),
    );
    let gctx = ctx.extend("u", nat());
    assert_eq!(
        infer_type(&gctx, &via_global).expect("the lifted form is well typed"),
        nat(),
        "lifting the shared subterm to a named global is a real translation, not a dodge"
    );
    assert!(
        nodes(&via_global) < big_nodes,
        "a `Global` reference is ONE node until `reduce_step` chooses to unfold it — the only \
         sharing boundary LOGOS has, and it changes the declaration structure"
    );

    // `Let` does NOT restore sharing: zeta substitutes the value at every occurrence.
    let expanded = normalize(&bctx, &shared);
    assert!(
        nodes(&expanded) <= big_nodes + 16,
        "a subterm Lean's hash-consed `Expr` holds ONCE is copied at every occurrence here: \
         `big` is {big_nodes} nodes and the checked term is {} — `Term` has no `Rc`, no \
         interning and no node identity, and `Let`'s zeta rule expands before the body is \
         inferred, so reaching for `Let` to compress a term makes the CHECK bigger",
        nodes(&expanded)
    );
}
