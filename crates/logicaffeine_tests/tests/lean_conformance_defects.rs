//! The Lean-4 conformance corpus — the CONFIRMED KERNEL DEFECTS, reproduced.
//!
//! Eight defects, each verified against today's source before the test was written. These are
//! RED by construction: nothing here is aspirational Lean parity, every one is a place where
//! the kernel contradicts ITSELF — one component's answer is refuted by another component of
//! the same kernel. That is why they are worth more than a feature gap: a self-contradiction
//! is a fact, not a judgement call.
//!
//! The defects, and the contradiction each one names:
//!
//! 1. `Universe::succ` says `succ(Prop) = Type 1`; the ℕ level model it is decided against says
//!    `Prop = 0`, `Type n = n+1`, so `succ(Prop)` must be `Type 0`. The OPERATION and the
//!    ORDERING PROCEDURE are two different algebras. (Lean: `Prop : Type 0`.)
//! 2. `is_subsingleton_prop` skips `params + INDICES` binders off the CONSTRUCTOR's telescope.
//!    The real rule is Lean's: a constructor field is admissible iff it is propositional OR it
//!    appears as an index of the constructor's result type. Both kernels carry the identical
//!    miscount, so double-checking gives zero protection here.
//! 3. `get_global` resolves inductive → constructor → definition → declaration; `reduce_step`
//!    and `recheck::whnf` call `get_definition_body` DIRECTLY. Keys are flat `String` with no
//!    duplicate-name guard, so one name can mean one thing to typing and another to reduction.
//! 4. `add_definition` is a two-line `insert`. Nothing checks the body against the declared
//!    type, and there is no checked sibling — so `evil : False := Zero` simply lands.
//! 5. `reduce_step`'s `Term::Const` arm returns the term UNREDUCED on a level-arity mismatch
//!    while `infer_type` returns an error: the reducer calls a term a normal form that the
//!    type checker calls ill-formed.
//! 8. `prelude.rs` registers `BVec : Nat -> Type 0` through the unchecked `add_inductive`, so
//!    its whole arity is treated as parameters — yet `BVNil : BVec Zero` and
//!    `BVCons : … BVec n -> BVec (Succ n)` are genuinely INDEXED.
//! 9. `normalize` runs on `let mut fuel = 10000` and returns the partially-reduced term on
//!    exhaustion. The caller gets back a `Term` and cannot tell "this is the normal form" from
//!    "this is as far as I got".
//! 10. (Found here, not in the corpus.) `literate_parser::build_induction_derivation` ignores
//!    its `_var_name` and builds the motive with NO `SLam` binder, while `try_delim_conclude`
//!    requires `extract_slam_body`. Literate `induction` cannot succeed on any goal.
//!
//! Observation notes — the kernel's `context`/`term`/`type_checker`/`reduction` modules are
//! private, so several verdicts are read off indirectly:
//!   * the level model, only through `Universe::equiv`/`is_subtype_of`;
//!   * the subsingleton verdict, through `derive_recursor`'s motive target sort
//!     (`Sort(Prop)` = large elimination REFUSED, `Sort(Type 0)` = GRANTED);
//!   * reduction's opinion of a name, through `normalize` and `Context::get_definition_body`.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    derive_recursor, double_check, infer_type, normalize, Context, DoubleCheck, Term, Universe,
};

// ==== PER-FILE HELPERS (house style: every kernel test declares its own) ====

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
    xs.iter().fold(f, |a, x| app(a, x.clone()))
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
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn type0() -> Term {
    sort(Universe::Type(0))
}
fn prop() -> Term {
    sort(Universe::Prop)
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// Peel a `Π`-telescope into its binders and the residual body. The real one lives inside
/// `recursor.rs` and is not exported.
fn peel_pis(t: &Term) -> (Vec<(String, Term)>, Term) {
    let mut binders = Vec::new();
    let mut cur = t.clone();
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
        binders.push((param, *param_type));
        cur = *body_type;
    }
    (binders, cur)
}

// =============================================================================
// DEFECT 1 — `Universe::succ` and the ℕ level model are two different algebras.
// =============================================================================

/// `succ` must BE the level model's successor, for every shape of level — not only for the
/// one shape someone remembered to special-case. The model maps `Prop ↦ 0`, `Type n ↦ n+1`,
/// and the symbolic `Succ(ℓ)` to the model's successor of `ℓ`; so the law that ties the
/// operation to the model is `ℓ.succ() ≡ Succ(ℓ)`, decided by the public `equiv`. Today
/// `Prop.succ() = Type 1` (model value 2) while `Succ(Prop)` is model value 1 — the operation
/// overshoots its own ordering procedure by one, and no fix that hard-codes a single case can
/// satisfy a law quantified over every shape.
#[test]
fn succ_and_the_level_model_are_one_algebra() {
    let u = || Universe::Var("u".to_string());
    let shapes: Vec<(&str, Universe)> = vec![
        ("SProp", Universe::SProp),
        ("Prop", Universe::Prop),
        ("Type 0", Universe::Type(0)),
        ("Type 1", Universe::Type(1)),
        ("Type 7", Universe::Type(7)),
        ("u", u()),
        ("succ u", Universe::Succ(Box::new(u()))),
        (
            "max(Type 0, u)",
            Universe::Max(Box::new(Universe::Type(0)), Box::new(u())),
        ),
        (
            "imax(Type 3, u)",
            Universe::IMax(Box::new(Universe::Type(3)), Box::new(u())),
        ),
    ];

    let mut disagreements: Vec<String> = Vec::new();
    for (label, level) in &shapes {
        let by_operation = level.succ();
        let by_model = Universe::Succ(Box::new(level.clone()));
        if !by_operation.equiv(&by_model) {
            disagreements.push(format!(
                "{label}: succ() = {by_operation:?} but the level model puts it at {by_model:?}"
            ));
        }
    }

    assert!(
        disagreements.is_empty(),
        "`Universe::succ` must agree with the ℕ level model `level_leq` decides over, for EVERY \
         level shape — one self-consistent algebra, not an operation plus a separate ordering \
         that contradicts it. Disagreements: {disagreements:#?}"
    );
}

/// The Lean fact, stated directly: `Prop : Type 0`. `infer_type(Sort u)` is `Sort(u.succ())`,
/// so this is defect 1 observed at the surface every proof actually goes through.
#[test]
fn prop_lives_in_type_zero_exactly_as_lean_says() {
    let ctx = std_ctx();
    let inferred = infer_type(&ctx, &prop()).expect("Prop has a type");
    assert_eq!(
        inferred,
        type0(),
        "`Prop : Type 0` (Lean's `Sort 0 : Sort 1`); the kernel put Prop one level too high: {inferred}"
    );
}

// =============================================================================
// DEFECT 2 — the large-elimination gate counts the WRONG telescope.
//
// Lean's rule: a one-constructor `Prop` is a subsingleton (⇒ may eliminate into `Type`) when
// every constructor FIELD — every argument past the inductive's PARAMETERS — is either
// propositional, or appears as an INDEX of the constructor's result type. The kernel instead
// skips `pi_param_count(ind_type)` = params + indices binders off the CONSTRUCTOR's telescope,
// which is neither. Both kernels (`type_checker::is_subsingleton_prop` and
// `recheck::is_subsingleton`) carry the identical miscount, so the second kernel cannot catch it.
//
// The verdict is read through `derive_recursor`: a `Prop` inductive whose large elimination is
// REFUSED gets a `Sort(Prop)` motive target; one whose large elimination is GRANTED gets
// `Sort(Type 0)`.
// =============================================================================

/// `Tag : Bool -> Prop` with `mk : Π(n:Nat). Tag (decide (Eq Nat n Zero) (decEqNat n Zero))`.
/// Zero parameters. The field `n : Nat` is NOT propositional, and it is not an index of the
/// result either — the index is a `Bool` COMPUTED from `n`, not `n` itself. Lean refuses.
fn register_tag(ctx: &mut Context) {
    ctx.add_indexed_inductive("Tag", arrow(g("Bool"), prop()), 0);
    let n_is_zero = apps(g("Eq"), &[g("Nat"), var("n"), g("Zero")]);
    let decided = apps(
        g("decide"),
        &[n_is_zero, apps(g("decEqNat"), &[var("n"), g("Zero")])],
    );
    ctx.add_constructor("Tag_mk", "Tag", pi("n", g("Nat"), app(g("Tag"), decided)));
}

/// `Weird : Nat -> Prop` with `mk : True -> Π(n:Nat). Weird n`. Zero parameters. `h : True` is
/// propositional and `n : Nat` IS the result's index, so both fields are admissible and Lean
/// grants large elimination. The constructor deliberately puts the propositional field FIRST,
/// so a check that skips a fixed prefix skips the wrong one.
fn register_weird(ctx: &mut Context) {
    ctx.add_indexed_inductive("Weird", arrow(g("Nat"), prop()), 0);
    ctx.add_constructor(
        "Weird_mk",
        "Weird",
        arrow(g("True"), pi("n", g("Nat"), app(g("Weird"), var("n")))),
    );
}

/// The motive's target sort of `ind`'s derived recursor — `Sort(Prop)` when large elimination
/// was refused, `Sort(Type 0)` when granted. The motive is the binder just past the inductive's
/// parameters.
fn large_elimination_verdict(ctx: &Context, ind: &str) -> Result<Term, String> {
    let (rec_ty, _term) =
        derive_recursor(ctx, ind).map_err(|e| format!("{ind}'s recursor must derive: {e:?}"))?;
    let (binders, _res) = peel_pis(&rec_ty);
    let p = ctx.inductive_num_params(ind);
    let motive_ty = binders
        .get(p)
        .ok_or_else(|| format!("{ind}_rec has no motive binder at position {p}: {rec_ty}"))?
        .1
        .clone();
    let (_idx_and_major, target) = peel_pis(&motive_ty);
    Ok(target)
}

/// All three cases from ONE table, so a shared blind spot cannot return: the two counterexamples
/// pin the rule in both directions, and `Acc` — whose `x : A` field is admissible ONLY by the
/// index clause — pins that a fix does not amputate well-founded recursion on the way past.
#[test]
fn large_elimination_admits_exactly_the_propositional_and_indexed_constructor_fields() {
    // (inductive, grant large elimination?, why)
    let table: Vec<(&str, fn(&mut Context), bool, &str)> = vec![
        (
            "Tag",
            register_tag as fn(&mut Context),
            false,
            "the field `n : Nat` is data, and the result index is a Bool COMPUTED from it, not `n` \
             itself — a proof of `Tag b` must not be able to hand back the `n` it was built from",
        ),
        (
            "Weird",
            register_weird as fn(&mut Context),
            true,
            "`h : True` is propositional and `n : Nat` IS the result's index — both fields are \
             admissible, so this subsingleton may eliminate into Type",
        ),
        (
            "Acc",
            (|_: &mut Context| {}) as fn(&mut Context),
            true,
            "Acc's `x : A` field is data admitted ONLY by the index clause (`Acc A R x`); the \
             CONTROL — if a fix loses the index clause, Acc_rec dies and with it every \
             definition by well-founded recursion",
        ),
    ];

    let mut wrong: Vec<String> = Vec::new();
    for (ind, register, grant, why) in &table {
        let mut ctx = std_ctx();
        register(&mut ctx);
        let want = if *grant { type0() } else { prop() };
        match large_elimination_verdict(&ctx, ind) {
            Ok(got) if got == want => {}
            Ok(got) => wrong.push(format!(
                "{ind}: large elimination should be {} ({why}) — its recursor's motive targets \
                 {got} where it must target {want}",
                if *grant { "GRANTED" } else { "REFUSED" },
            )),
            Err(e) => wrong.push(format!("{ind}: {e}")),
        }
    }

    assert!(
        wrong.is_empty(),
        "the large-elimination gate must admit exactly Lean's constructor fields — propositional, \
         or appearing as an index of the constructor's result type — counted off the inductive's \
         PARAMETERS, not off its params+indices: {wrong:#?}"
    );
}

// =============================================================================
// DEFECT 3 — flat `String` keys, four namespaces, no duplicate guard.
// =============================================================================

/// `get_global` prefers the inductive; `reduce_step`/`recheck::whnf` go straight to
/// `get_definition_body`. Register one name as both and the kernel holds two incompatible
/// meanings for it: typing says `Ghost : Type 0`, reduction says `Ghost` unfolds to `Zero`.
/// The registration itself must be refused — a name means one thing.
#[test]
fn a_name_cannot_be_both_an_inductive_and_a_definition() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Ghost", type0());
    ctx.add_definition("Ghost".to_string(), g("Nat"), g("Zero"));

    assert!(
        !(ctx.is_inductive("Ghost") && ctx.is_definition("Ghost")),
        "registering `Ghost` as both an inductive and a definition must be REFUSED: \
         `get_global` resolves it to the inductive ({:?}) while `reduce_step` unfolds it \
         through `get_definition_body` to {:?} — one name, two meanings",
        ctx.get_global("Ghost"),
        ctx.get_definition_body("Ghost"),
    );

    // …and the two views must in fact coincide for whatever single meaning survives.
    let typed = ctx.get_global("Ghost").cloned();
    let reduced = normalize(&ctx, &g("Ghost"));
    assert!(
        !(typed == Some(type0()) && reduced == g("Zero")),
        "typing and reduction must not disagree about `Ghost`: typing gives {typed:?}, \
         reduction gives {reduced}"
    );
}

/// The standing invariant the guard has to protect: in the standard prelude, no name is claimed
/// by two namespaces, so `get_global`'s resolution order and `get_definition_body`'s direct
/// lookup can never pick different entities. This is the control for the test above — it must
/// stay green while that one goes from red to green.
#[test]
fn every_prelude_name_means_the_same_thing_to_typing_and_to_reduction() {
    let ctx = std_ctx();

    let mut names: Vec<String> = Vec::new();
    for (n, _) in ctx.iter_inductives() {
        names.push(n.to_string());
        for (c, _) in ctx.get_constructors(n) {
            names.push(c.to_string());
        }
    }
    for (n, _, _) in ctx.iter_definitions() {
        names.push(n.to_string());
    }
    for (n, _) in ctx.iter_declarations() {
        names.push(n.to_string());
    }
    names.sort();
    names.dedup();
    assert!(
        names.len() > 100,
        "the standard prelude should be substantial, got {}",
        names.len()
    );

    let mut collisions: Vec<String> = Vec::new();
    for n in &names {
        // `reduce_step` asks ONLY `get_definition_body`; `infer_type` asks `get_global`, which
        // prefers inductives and constructors. A name in both is a name with two meanings.
        if ctx.get_definition_body(n).is_some() && (ctx.is_inductive(n) || ctx.is_constructor(n)) {
            collisions.push(format!(
                "{n}: is_inductive={} is_constructor={} and also has a definition body",
                ctx.is_inductive(n),
                ctx.is_constructor(n),
            ));
        }
        if ctx.is_inductive(n) && ctx.is_constructor(n) {
            collisions.push(format!(
                "{n}: registered as both an inductive and a constructor"
            ));
        }
    }

    assert!(
        collisions.is_empty(),
        "every prelude name must resolve to exactly one entity, so typing (`get_global`) and \
         reduction (`get_definition_body`) can never disagree: {collisions:#?}"
    );
}

// =============================================================================
// DEFECT 4 — `add_definition` never checks the body against the declared type.
// =============================================================================

/// `evil : False := Zero` must not land. Today `add_definition` is an `insert`, so it does —
/// and `Global("evil")` is then a proof of `False`, from which everything follows.
#[test]
fn add_definition_rejects_a_mistyped_body() {
    let mut ctx = std_ctx();
    ctx.add_definition("evil".to_string(), g("False"), g("Zero"));

    assert!(
        ctx.get_global("evil").is_none(),
        "`evil : False := Zero` must be REFUSED — `Zero : Nat`, not `False`; instead the kernel \
         stored it and now hands out {:?} as its type",
        ctx.get_global("evil"),
    );

    let proves_false = matches!(infer_type(&ctx, &g("evil")), Ok(t) if t == g("False"));
    assert!(
        !proves_false,
        "the kernel must not certify `evil : False` from a body that is a natural number — \
         from a proof of False every proposition follows"
    );
}

/// The paired control: a CORRECTLY typed definition must still be registered and must still
/// unfold. A "fix" that makes `add_definition` drop everything would be caught here.
#[test]
fn add_definition_still_accepts_a_correctly_typed_body() {
    let mut ctx = std_ctx();
    ctx.add_definition("one".to_string(), g("Nat"), app(g("Succ"), g("Zero")));
    assert_eq!(
        ctx.get_global("one").cloned(),
        Some(g("Nat")),
        "a well-typed definition must be registered with its declared type"
    );
    assert_eq!(
        normalize(&ctx, &g("one")),
        app(g("Succ"), g("Zero")),
        "a well-typed definition must still δ-unfold"
    );
}

// =============================================================================
// DEFECT 4b — the declared type itself was never checked.
// =============================================================================

fn on_a_large_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .expect("spawns")
        .join()
        .expect("joins")
}

/// A definition's declared type must be a type. `(λ(x : Nat). λ(y : Bool). Nat) Zero Zero` is
/// ill-typed, `Zero` being no `Bool`, and still β-reduces to `Nat`; a body compared with it only by
/// conversion lands at a type no judgement derives.
#[test]
fn declare_refuses_a_declared_type_that_is_not_a_type() {
    let mut ctx = std_ctx();
    let ill = apps(
        lam("x", g("Nat"), lam("y", g("Bool"), g("Nat"))),
        &[g("Zero"), g("Zero")],
    );
    let verdict = ctx.declare("weird", ill, g("Zero"));
    assert!(
        verdict.is_err(),
        "a declared type that is not a type must be refused, not {verdict:?}"
    );
    assert!(
        ctx.get_global("weird").is_none(),
        "`weird` must never reach the environment"
    );
    let fine = app(lam("x", g("Nat"), g("Nat")), g("Zero"));
    let control = ctx.declare("fine", fine, g("Zero"));
    assert!(
        control.is_ok(),
        "a well-typed declared type that reduces to `Nat` stands: {control:?}"
    );
}

/// A declared type may name a constant declared later, as a body may: the definition waits for it
/// as a pending obligation, invisible meanwhile, and is checked, type and body, when it arrives.
#[test]
fn a_definition_whose_declared_type_names_a_later_constant_waits_for_it() {
    let mut ctx = std_ctx();
    let deferred = ctx.declare("early", g("LaterNat"), g("Zero"));
    assert!(
        deferred.is_ok(),
        "a forward reference in the declared type is owed, not refused: {deferred:?}"
    );
    assert!(
        ctx.get_global("early").is_none(),
        "an unchecked definition is not visible"
    );
    assert!(
        ctx.pending_obligations().contains(&("early", "LaterNat")),
        "the obligation names what it waits for: {:?}",
        ctx.pending_obligations()
    );
    ctx.declare("LaterNat", type0(), g("Nat"))
        .expect("`LaterNat := Nat` is a type");
    assert_eq!(
        ctx.get_global("early").cloned(),
        Some(g("LaterNat")),
        "checked once `LaterNat` arrives"
    );
    assert!(
        ctx.pending_obligations().is_empty(),
        "nothing is owed: {:?}",
        ctx.pending_obligations()
    );
}

/// An assumption's type may name a constant declared later too: the assumption waits for it,
/// invisible meanwhile, and is checked when it arrives.
#[test]
fn an_assumption_whose_type_names_a_later_constant_waits_for_it() {
    let mut ctx = std_ctx();
    ctx.add_declaration("early_ax", g("LaterType"));
    assert!(
        ctx.get_global("early_ax").is_none(),
        "an unchecked assumption is not visible"
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "it is owed, not refused: {:?}",
        ctx.refused_declarations()
    );
    assert!(
        ctx.pending_obligations()
            .contains(&("early_ax", "LaterType")),
        "the obligation names what it waits for: {:?}",
        ctx.pending_obligations()
    );
    ctx.declare("LaterType", type0(), g("Nat"))
        .expect("`LaterType := Nat` is a type");
    assert_eq!(
        ctx.get_global("early_ax").cloned(),
        Some(g("LaterType")),
        "checked once `LaterType` arrives"
    );
    assert!(
        ctx.pending_obligations().is_empty(),
        "nothing is owed: {:?}",
        ctx.pending_obligations()
    );
}

/// An axiom's type must be a type: `Zero` is a number, and `ax : Zero` declares nothing.
#[test]
fn add_declaration_refuses_an_axiom_whose_type_is_not_a_type() {
    let mut ctx = std_ctx();
    ctx.add_declaration("ax", g("Zero"));
    assert!(
        ctx.get_global("ax").is_none(),
        "an axiom typed by a number must not be registered"
    );
    assert!(
        ctx.refused_declarations()
            .iter()
            .any(|(name, _)| name == "ax"),
        "the refusal is recorded: {:?}",
        ctx.refused_declarations()
    );
    ctx.add_declaration("hyp", apps(g("Eq"), &[g("Nat"), g("Zero"), g("Zero")]));
    assert!(
        ctx.get_global("hyp").is_some(),
        "an axiom at a proposition stands"
    );
}

/// Every declaration of the standard library has a declared type the kernel types. The bit-vector
/// equations (`bv_and_eq_def`, …) state `bv_and` as a fixpoint whose result type no `match` motive
/// states, `λn v1 v2. (match v1 … ) v2`; a fixpoint in a checking position, here an argument of
/// `Eq`, is typed against the type the position expects. The census reads the declarations the
/// prelude holds, so it first holds the prelude whole: a declaration refused or still owed at
/// registration would leave the census nothing of it to read.
#[test]
fn every_prelude_declaration_has_a_type_the_kernel_types() {
    on_a_large_stack(|| {
        let ctx = std_ctx();
        assert!(
            ctx.refused_declarations().is_empty(),
            "the prelude refused: {:?}",
            ctx.refused_declarations()
        );
        assert!(
            ctx.pending_obligations().is_empty(),
            "the prelude still owes: {:?}",
            ctx.pending_obligations()
        );
        assert!(
            ctx.get_global("bv_and_eq_def").is_some(),
            "the census's subject, `bv_and`'s equation, is registered"
        );
        let mut untyped = Vec::new();
        let mut is_a_type =
            |name: &str, ty: &Term| match infer_type(&ctx, ty).map(|s| normalize(&ctx, &s)) {
                Ok(Term::Sort(_)) => {}
                other => untyped.push(format!("{name}: {other:?}")),
            };
        let mut count = 0usize;
        for (name, ty, _) in ctx.iter_definitions() {
            is_a_type(name, ty);
            count += 1;
        }
        for (name, _, ty, _) in ctx.iter_universe_polys() {
            is_a_type(name, ty);
            count += 1;
        }
        for (name, ty) in ctx.iter_declarations() {
            is_a_type(name, ty);
            count += 1;
        }
        assert!(
            count > 500,
            "the prelude is the prelude: {count} declarations"
        );
        assert!(
            untyped.is_empty(),
            "declared types the kernel cannot type:\n{}",
            untyped.join("\n")
        );
    });
}

// =============================================================================
// DEFECT 5 — reduction and typing disagree about a level-arity mismatch.
//
// `tests/universe_poly.rs` already builds exactly this `bad` term and asserts the `infer_type`
// half. The missing half is what the REDUCER does with it: `reduce_step`'s `Term::Const` arm
// falls through to `term.clone()`, so `normalize` reports it as a normal form.
// =============================================================================

#[test]
fn reduction_and_typing_agree_about_a_level_arity_mismatch() {
    let mut ctx = std_ctx();
    let u = Universe::Var("u".to_string());
    // id.{u} : Π(A:Sort u). A → A := λA:Sort u. λa:A. a   — ONE universe parameter.
    ctx.add_universe_poly(
        "id",
        vec!["u".to_string()],
        pi("A", sort(u.clone()), arrow(var("A"), var("A"))),
        lam("A", sort(u), lam("a", var("A"), var("a"))),
    );

    let good = Term::Const {
        name: "id".to_string(),
        levels: vec![Universe::Type(0)],
    };
    let bad = Term::Const {
        name: "id".to_string(),
        levels: vec![Universe::Type(0), Universe::Type(1)],
    };

    // Control: at the right arity the two paths agree — it type-checks AND it δ-unfolds.
    assert!(infer_type(&ctx, &good).is_ok(), "id.{{0}} type-checks");
    assert_ne!(
        normalize(&ctx, &good),
        good,
        "id.{{0}} must δ-unfold to its instantiated body"
    );

    let typing_refuses = infer_type(&ctx, &bad).is_err();
    let reducer_calls_it_a_normal_form = normalize(&ctx, &bad) == bad;
    assert!(
        !(typing_refuses && reducer_calls_it_a_normal_form),
        "reduction and typing must agree about `id.{{0,1}}`: `infer_type` refuses it \
         (err = {typing_refuses}) while `normalize` hands it straight back as a settled normal \
         form (= {reducer_calls_it_a_normal_form}). One kernel, one verdict."
    );

    // …and the agreement must be reached HONESTLY. Whatever the reducer does with a term the
    // type checker refuses, it must never launder it into a well-typed one — e.g. by zipping
    // the level arguments and silently dropping the extra.
    let nf = normalize(&ctx, &bad);
    assert!(
        infer_type(&ctx, &nf).is_err(),
        "normalize must not turn an ill-typed `Const` into a well-typed term: {nf}"
    );
}

// =============================================================================
// DEFECT 8 — `BVec`'s `Nat` is an INDEX registered as a parameter.
// =============================================================================

#[test]
fn bvec_s_nat_is_an_index_not_a_parameter() {
    let ctx = std_ctx();
    assert_eq!(
        ctx.inductive_num_params("BVec"),
        0,
        "`BVec : Nat -> Type 0` has NO uniform parameter — `BVNil : BVec Zero` and \
         `BVCons : Bit -> Π(n:Nat). BVec n -> BVec (Succ n)` target different lengths, so the \
         `Nat` is an INDEX. The unchecked `add_inductive` treated the whole arity as parameters."
    );
    assert_eq!(
        ctx.inductive_num_indices("BVec"),
        1,
        "`BVec`'s single arity slot is its one index"
    );
}

/// The consequence: because the length is recorded as a parameter, a `match` whose motive
/// abstracts the LENGTH — the only kind of match a length-indexed bitvector is for — is not
/// typeable. This is the `Vector`-shaped match that `tests/indexed_recursor.rs` shows working
/// for a properly registered indexed family.
#[test]
fn an_indexed_match_on_bvec_typechecks_and_computes() {
    let ctx = std_ctx();
    // match (BVNil : BVec Zero) return λ(n:Nat). λ(v:BVec n). Nat with
    //   | BVNil  => Zero
    //   | BVCons => λb. λn. λt. Succ Zero
    let motive = lam("n", g("Nat"), lam("v", app(g("BVec"), var("n")), g("Nat")));
    let m = Term::Match {
        discriminant: Box::new(g("BVNil")),
        motive: Box::new(motive),
        cases: vec![
            g("Zero"),
            lam(
                "b",
                g("Bit"),
                lam(
                    "n",
                    g("Nat"),
                    lam("t", app(g("BVec"), var("n")), app(g("Succ"), g("Zero"))),
                ),
            ),
        ],
    };

    let inferred = infer_type(&ctx, &m);
    assert!(
        matches!(&inferred, Ok(t) if *t == g("Nat")),
        "a length-abstracting match on `BVec` must type-check at `Nat`: {inferred:?}"
    );
    assert_eq!(
        double_check(&ctx, &m),
        DoubleCheck::Agreed,
        "the second kernel must agree about the indexed `BVec` match, got {:?}",
        double_check(&ctx, &m)
    );
    assert_eq!(
        normalize(&ctx, &m),
        g("Zero"),
        "the match must ι-compute: the discriminant is `BVNil`, so it selects the first case"
    );
}

// =============================================================================
// DEFECT 9 — `normalize`'s fuel is spent in silence.
//
// The caller receives a `Term` and nothing else. So the ONLY way the caller can distinguish
// "this is the normal form" from "this is as far as I got" is if the returned term is never
// itself a redex. Two shapes make the point from both sides: a δ-chain that is merely LONGER
// than any fixed fuel (a fuel bump would hide it) and a δ-CYCLE that no fuel budget can ever
// finish (a fuel bump cannot). The cycle is answered the way Lean answers it: the environment
// cannot be made cyclic — a constant's value may only mention constants that do not depend on
// it — so the closing half is refused at the gateway and there is no cycle to abandon. (The one
// door that can still build a reduction with no normal form, a `@[simp]` rewrite loop, is held
// to the honest channel by `R_reduction_extra`.)
// =============================================================================

/// `Global(n)` where `n` is a definition is a δ-redex by construction — `reduce_step` unfolds
/// it. So `normalize` returning one is `normalize` saying "done" about a term it has not
/// finished.
fn is_still_a_redex(ctx: &Context, t: &Term) -> bool {
    matches!(t, Term::Global(n) if ctx.is_definition(n))
}

#[test]
fn normalize_must_not_hand_back_an_unfinished_reduction_as_a_normal_form() {
    let mut ctx = std_ctx();

    // (a) A δ-chain `chain0 → chain1 → … → chain19999 → Zero`, far longer than the fuel but
    //     perfectly terminating: its normal form is `Zero`.
    const CHAIN: usize = 20_000;
    for i in 0..CHAIN {
        let next = if i + 1 == CHAIN {
            g("Zero")
        } else {
            g(&format!("chain{}", i + 1))
        };
        ctx.add_definition(format!("chain{i}"), g("Nat"), next);
    }
    // (b) A δ-CYCLE `loop_a → loop_b → loop_a → …` would have no normal form at all — so the
    //     second half, which closes it, is refused before anything is inserted.
    ctx.add_definition("loop_a".to_string(), g("Nat"), g("loop_b"));
    ctx.add_definition("loop_b".to_string(), g("Nat"), g("loop_a"));
    assert!(
        ctx.refused_declarations()
            .iter()
            .any(|(name, why)| name == "loop_b"
                && why.contains("cannot be defined in terms of `loop_a`")),
        "the definition that would close the δ-cycle is refused, and the refusal says why: {:?}",
        ctx.refused_declarations()
    );
    assert!(
        ctx.get_definition_body("loop_b").is_none(),
        "…and it registers nothing"
    );
    assert!(
        ctx.pending_obligations()
            .iter()
            .any(|(name, owed)| *name == "loop_a" && *owed == "loop_b"),
        "`loop_a` is left owing its forward reference, tracked as an unpaid obligation"
    );
    assert!(
        infer_type(&ctx, &g("loop_a")).is_err(),
        "…so nothing can type it, and no proof can be written about it"
    );

    // The reference point: what a genuine normal form looks like coming back.
    let settled = normalize(&ctx, &g("Zero"));
    assert_eq!(settled, g("Zero"), "`Zero` is already a normal form");
    assert!(!is_still_a_redex(&ctx, &settled), "…and it is not a redex");

    let chained = normalize(&ctx, &g("chain0"));
    let cycled = normalize(&ctx, &g("loop_a"));

    let mut unfinished: Vec<String> = Vec::new();
    if chained != g("Zero") {
        unfinished.push(format!(
            "a 20000-long δ-chain: normalize returned {chained}, not its normal form `Zero` \
             (still a redex = {})",
            is_still_a_redex(&ctx, &chained)
        ));
    }
    if cycled != g("loop_a") {
        unfinished.push(format!(
            "`loop_a`'s body is an UNCHECKED obligation, so δ must never unfold it — normalize \
             returned {cycled}"
        ));
    }

    assert!(
        unfinished.is_empty(),
        "`normalize` must never present a partially-reduced term as a normal form: the caller \
         gets only a `Term` back, so an abandoned reduction has to be distinguishable from a \
         finished one — and no fuel constant is what stands between it and a cycle: the \
         environment refuses to contain one. {unfinished:#?}"
    );
}

// =============================================================================
// DEFECT 10 — literate `induction` can never succeed (our own find).
//
// An induction proof needs two things from the literate engine: a motive that ABSTRACTS the
// variable it inducts on (`try_delim_conclude` opens it with `extract_slam_body`, so a motive
// without its `SLam` binder concludes the `SName "Error"` sentinel), and a step case that can
// USE its induction hypothesis. Lean's `0 + (k+1)` computes to `succ (0 + k)`, and only the
// hypothesis `0 + k = k` turns that into `succ k`.
// =============================================================================

/// A literate `Proof: induction n.` with bullet cases must certify a goal that is genuinely true
/// by induction. Two guards make this un-patchable: the goals are chosen so that EVERY
/// non-inductive tactic refuses them (so `induction` cannot be quietly aliased to `auto`), and
/// several phrasings are driven from one table (so a failure cannot be blamed on one unlucky
/// source string). The goals are Lean's own on `Nat` (`Nat_add`/`Nat_mul`, which recurse on the
/// second argument exactly as `Nat.add`/`Nat.mul` do), so each one is a left identity or a left
/// annihilator: `n + 0 = n` and `n * 0 = 0` hold by `rfl` in Lean and are no test of induction.
#[test]
fn literate_induction_certifies_a_goal_that_is_true_by_induction() {
    // (theorem name, the goal in literate statement form, the induction variable)
    let goals: Vec<(&str, &str, &str)> = vec![
        ("ZeroAddLeft", "(Nat_add Zero n) equals n", "n"),
        ("ZeroMulLeft", "(Nat_mul Zero n) equals Zero", "n"),
        ("OneAddLeft", "(Nat_add (Succ Zero) n) equals (Succ n)", "n"),
        ("OneMulLeft", "(Nat_mul (Succ Zero) n) equals n", "n"),
    ];

    let mut leaked: Vec<String> = Vec::new();
    let mut refused: Vec<String> = Vec::new();

    for (name, statement, v) in &goals {
        // (1) The goal must genuinely REQUIRE induction: `Zero`/`Succ` are opaque constructors
        // to the polynomial and arithmetic tactics, so none of them may prove it. Without this
        // guard, "fixing" induction by delegating to `auto` would pass the test below.
        for tactic in [
            "ring", "lia", "omega", "cc", "simp", "auto", "refl", "decide",
        ] {
            let src = format!(
                "## Theorem: {name}_direct\n    Statement: {statement}.\n    Proof: {tactic}."
            );
            if Repl::new().execute(&src).is_ok() {
                leaked.push(format!("`{statement}` was proved outright by `{tactic}`"));
            }
        }

        // (2) …and the induction proof must certify it. `Nat` has two constructors, so two bullets.
        let src = format!(
            "## Theorem: {name}\n    Statement: {statement}.\n    Proof: induction {v}.\n    - simp.\n    - simp."
        );
        let mut repl = Repl::new();
        match repl.execute(&src) {
            Err(e) => refused.push(format!("`{statement}` by induction on {v}: {e:?}")),
            Ok(_) => {
                // The REPL's soundness gate already required `concludes body` to reduce to the
                // stated goal, but re-check the stored derivation ourselves rather than trusting
                // the door's `Result`.
                let ctx = repl.context();
                match ctx.get_definition_body(name).cloned() {
                    None => refused.push(format!("`{statement}`: accepted but nothing was stored")),
                    Some(body) => {
                        if let Err(e) = infer_type(ctx, &body) {
                            refused.push(format!(
                                "`{statement}`: the stored derivation does not re-check: {e:?}"
                            ));
                        }
                        if double_check(ctx, &body) != DoubleCheck::Agreed {
                            refused.push(format!(
                                "`{statement}`: the second kernel does not agree about the \
                                 stored derivation"
                            ));
                        }
                    }
                }
            }
        }
    }

    assert!(
        leaked.is_empty(),
        "these goals were chosen because only induction can reach them — if a direct tactic \
         proves one, the test below would pass for the wrong reason: {leaked:#?}"
    );
    assert!(
        refused.is_empty(),
        "a literate `Proof: induction n.` with bullet cases must certify a goal that is true by \
         induction: the motive abstracts `n`, and each step case closes its goal with the \
         induction hypothesis in scope. Refusals: {refused:#?}"
    );
}
