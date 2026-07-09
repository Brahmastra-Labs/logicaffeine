//! `Universe::IMax` and Prop-ranging universe variables — the full Lean level
//! semantics. Two things ride on this: closing the soundness hole where a
//! variable-level definition could be instantiated at `Prop` (`promote.{Prop}`
//! putting `Nat : Prop`), and gaining `id.{Prop}` (a polymorphic definition
//! usable at `Prop`), which the old `Type 0`-floored variable model forbade.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Term, Universe};

fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn var(v: &str) -> Universe {
    Universe::Var(v.to_string())
}
fn ty(n: u32) -> Universe {
    Universe::Type(n)
}

// --- the soundness fix: a variable ranges over ALL levels, including Prop ----

#[test]
fn type0_is_not_below_a_bare_variable() {
    // THE HOLE: `Type 0 ≤ u` must be FALSE, because `u` may be instantiated at
    // `Prop`, and `Type 0 ≤ Prop` is false. (Old model: variables floored at
    // `Type 0`, so this wrongly held — letting `promote.{u}:Sort u := Nat`
    // check and then `promote.{Prop} : Prop` put `Nat : Prop`.)
    assert!(
        !ty(0).is_subtype_of(&var("u")),
        "Type 0 ≤ u must be false (u can be Prop)"
    );
}

#[test]
fn prop_is_below_everything() {
    assert!(Universe::Prop.is_subtype_of(&var("u")), "Prop ≤ u");
    assert!(Universe::Prop.is_subtype_of(&ty(0)), "Prop ≤ Type 0");
    assert!(Universe::Prop.is_subtype_of(&Universe::Prop), "Prop ≤ Prop");
}

#[test]
fn variable_is_reflexively_a_subtype() {
    assert!(var("u").is_subtype_of(&var("u")), "u ≤ u");
    assert!(var("u").is_subtype_of(&var("u").succ()), "u ≤ u+1");
}

#[test]
fn distinct_variables_are_incomparable() {
    assert!(!var("u").is_subtype_of(&var("v")), "u ≤ v must fail");
    assert!(!var("v").is_subtype_of(&var("u")), "v ≤ u must fail");
}

#[test]
fn sort_subtyping_reflects_the_level_fix() {
    let ctx = std_ctx();
    // Sort(Type 0) ≤ Sort(u) is false; Sort(Prop) ≤ Sort(u) is true.
    assert!(!is_subtype(&ctx, &sort(ty(0)), &sort(var("u"))));
    assert!(is_subtype(&ctx, &sort(Universe::Prop), &sort(var("u"))));
}

// --- imax algebra -----------------------------------------------------------

#[test]
fn imax_collapses_concrete_right_argument() {
    // imax(a, Prop) = Prop  (a Π into a proposition is a proposition).
    assert!(Universe::imax(&ty(3), &Universe::Prop).equiv(&Universe::Prop));
    // imax(a, Type n) = max(a, Type n)  (predicative when the codomain is a Type).
    assert!(Universe::imax(&ty(1), &ty(2)).equiv(&ty(1).max(&ty(2))));
    assert!(Universe::imax(&ty(5), &ty(2)).equiv(&ty(5)));
}

#[test]
fn imax_idempotent_and_prop_left() {
    // imax(u, u) = u.
    assert!(Universe::imax(&var("u"), &var("u")).equiv(&var("u")));
    // imax(Prop, u) stays conditional on u — it is u when u≥1, Prop when u=0,
    // which is exactly `u` (Prop = 0). So imax(Prop, u) = u.
    assert!(Universe::imax(&Universe::Prop, &var("u")).equiv(&var("u")));
}

#[test]
fn imax_with_variable_right_is_not_max() {
    // imax(Type 0, u) ≠ max(Type 0, u): they differ at u = Prop, where
    // imax = Prop (0) but max = Type 0 (1). The distinction the old model lost.
    let im = Universe::imax(&ty(0), &var("u"));
    let mx = ty(0).max(&var("u"));
    assert!(!im.equiv(&mx), "imax(Type0,u) must differ from max(Type0,u)");
}

#[test]
fn imax_right_distributes_over_max_semantically() {
    // imax(a, max(b,c)) = max(imax(a,b), imax(a,c)) as a semantic identity
    // (both sides equal, decided by the level algebra).
    let a = var("a");
    let b = var("b");
    let c = var("c");
    let lhs = Universe::imax(&a, &b.max(&c));
    let rhs = Universe::imax(&a, &b).max(&Universe::imax(&a, &c));
    assert!(lhs.equiv(&rhs), "imax right-distributes over max");
}

// --- the Pi rule uses imax --------------------------------------------------

#[test]
fn pi_into_concrete_prop_stays_prop() {
    // Regression: a Π whose codomain is a concrete Prop is a Prop, byte-for-byte
    // (FOL formulas depend on this). Π(x:Nat). (Eq Nat x x) : Prop.
    let ctx = std_ctx();
    let eq_xx = {
        let g = |s: &str| Term::Global(s.to_string());
        let app = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        app(app(app(g("Eq"), g("Nat")), Term::Var("x".to_string())), Term::Var("x".to_string()))
    };
    let pi = Term::Pi {
        param: "x".to_string(),
        param_type: Box::new(Term::Global("Nat".to_string())),
        body_type: Box::new(eq_xx),
    };
    let ty = infer_type(&ctx, &pi).expect("Π into Prop checks");
    assert!(is_subtype(&ctx, &ty, &sort(Universe::Prop)), "Π into Prop is Prop, got {ty}");
}

// --- end-to-end: the attack is blocked, the capability is gained ------------

#[test]
fn checking_nat_against_sort_variable_is_rejected() {
    // THE ATTACK, end to end: a definition `promote.{u} : Sort u := Nat` is only
    // sound if the kernel accepts `Nat : Sort u`. Force that exact check with the
    // application `(λ(x : Sort u). x) Nat` — `Nat : Type 0`, and `Type 0 ≤ Sort u`
    // is now FALSE (u could be Prop), so the kernel MUST reject it. Before the
    // fix this checked, letting `promote.{Prop} : Prop` put `Nat : Prop`.
    let ctx = std_ctx();
    let coerce = Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(sort(var("u"))),
        body: Box::new(Term::Var("x".to_string())),
    };
    let attack = Term::App(Box::new(coerce), Box::new(Term::Global("Nat".to_string())));
    assert!(
        infer_type(&ctx, &attack).is_err(),
        "`Nat : Sort u` must be rejected — u may be Prop"
    );
}

#[test]
fn polymorphic_identity_instantiates_at_prop() {
    // THE CAPABILITY, gained: the polymorphic identity `λ(A:Sort u). λ(a:A). a`
    // instantiated at u := Prop is the identity on PROOFS — `id.{Prop} : Π(A:Prop). A → A`.
    // The old `Type 0`-floored model could not soundly reach Prop; the full imax
    // semantics can, matching Lean.
    let ctx = std_ctx();
    let id_poly = Term::Lambda {
        param: "A".to_string(),
        param_type: Box::new(sort(var("u"))),
        body: Box::new(Term::Lambda {
            param: "a".to_string(),
            param_type: Box::new(Term::Var("A".to_string())),
            body: Box::new(Term::Var("a".to_string())),
        }),
    };
    let subst: std::collections::HashMap<String, Universe> =
        [("u".to_string(), Universe::Prop)].into_iter().collect();
    let id_prop = logicaffeine_kernel::instantiate_universes(&id_poly, &subst);
    assert!(infer_type(&ctx, &id_prop).is_ok(), "id.{{Prop}} type-checks");
    // Apply it to the proposition `True` and its proof `I` — the identity on a proof.
    let app = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
    let applied = app(app(id_prop, Term::Global("True".to_string())), Term::Global("I".to_string()));
    let t = infer_type(&ctx, &applied).expect("id.{{Prop}} True I : True");
    assert!(is_subtype(&ctx, &t, &Term::Global("True".to_string())), "the result is a proof of True, got {t}");
}

// --- exhaustive model check: the algebra agrees with the ground truth -------

/// Ground-truth value of a level under an assignment (Prop = 0, Type n = n+1).
fn eval_level(u: &Universe, env: &std::collections::HashMap<String, u64>) -> u64 {
    match u {
        // The generator below never produces SProp, so this arm is only for exhaustiveness.
        Universe::SProp => 0,
        Universe::Prop => 0,
        Universe::Type(n) => *n as u64 + 1,
        Universe::Var(v) => *env.get(v).unwrap_or(&0),
        Universe::Succ(l) => eval_level(l, env) + 1,
        Universe::Max(a, b) => eval_level(a, env).max(eval_level(b, env)),
        Universe::IMax(a, b) => {
            let bv = eval_level(b, env);
            if bv == 0 {
                0
            } else {
                eval_level(a, env).max(bv)
            }
        }
    }
}

#[test]
fn level_leq_agrees_with_semantics_exhaustively() {
    // Enumerate a battery of level expressions of depth ≤ 3 over two variables,
    // and for EVERY ordered pair assert that `is_subtype_of` (`≤` for all
    // instantiations) agrees with the ground truth: `a ≤ b` holds iff
    // `eval(a) ≤ eval(b)` at every assignment of the variables. The grid
    // {0=Prop, 1=Type 0, 2, 9} per variable covers both `imax` regimes (b = 0 vs
    // b ≥ 1) and the "drive a variable large" corner, so a mismatch anywhere is
    // caught. This is the single test that guards the whole level algebra.
    let atoms: Vec<Universe> = vec![
        Universe::Prop,
        ty(0),
        ty(1),
        var("a"),
        var("b"),
        var("a").succ(),
        var("a").max(&var("b")),
        var("a").max(&ty(1)),
        Universe::imax(&var("a"), &var("b")),
        Universe::imax(&ty(0), &var("a")),
        Universe::imax(&var("a"), &ty(0)),
        var("a").succ().max(&var("b")),
        Universe::imax(&var("b"), &var("a").succ()),
    ];
    // All var assignments over the corner grid.
    let grid = [0u64, 1, 2, 9];
    let mut envs = Vec::new();
    for &av in &grid {
        for &bv in &grid {
            let mut m = std::collections::HashMap::new();
            m.insert("a".to_string(), av);
            m.insert("b".to_string(), bv);
            // fresh split-vars introduced by the decider never appear in these
            // expressions, so they default to 0 in eval — harmless.
            envs.push(m);
        }
    }

    for a in &atoms {
        for b in &atoms {
            let decided = a.is_subtype_of(b);
            let truth = envs.iter().all(|env| eval_level(a, env) <= eval_level(b, env));
            assert_eq!(
                decided, truth,
                "level_leq disagreement: `{a}` ≤ `{b}` decided {decided}, ground truth {truth}"
            );
        }
    }
}

#[test]
fn pi_predicative_into_type() {
    // Π(x:Type 0). Type 0 : Type 1 (predicative — the codomain is a Type).
    let ctx = std_ctx();
    let pi = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(sort(ty(0))),
        body_type: Box::new(sort(ty(0))),
    };
    let inferred = infer_type(&ctx, &pi).expect("Π into Type checks");
    assert!(is_subtype(&ctx, &inferred, &sort(ty(1))), "predicative Π : Type 1, got {inferred}");
}
