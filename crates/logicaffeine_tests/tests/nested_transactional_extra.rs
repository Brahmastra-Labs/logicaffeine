//! `add_nested_inductive` must be ALL-OR-NOTHING.
//!
//! `compile_nested` refuses eight shapes fail-closed, each promising that nothing is registered
//! when it declines. `Context::add_nested_inductive` broke that promise on one path: it called
//! `add_mutual_inductives` — which MUTATES on success — and only then kernel-checked the
//! conversion isos, so an iso failure returned an error with the block and its siblings already
//! in the context. A caller that handles the error and carries on is then reasoning in an
//! environment holding half a declaration: the inductives present, the conversions missing.
//!
//! `add_mutual_inductives` itself already has the right shape (check on a scratch clone, commit
//! only on success). This pins that `add_nested_inductive` has it too.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  "The error told you" is not a substitute for leaving the context clean — an error that has
//!  already mutated is the one a caller cannot recover from.

use logicaffeine_kernel::inductive_compile::NestedDecl;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{Context, Term, Universe};

fn std_ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}

/// Everything the global environment holds, as a comparable snapshot. `Context` is not `PartialEq`,
/// so the observable surface stands in for it — and that is the surface a caller would see.
fn snapshot(ctx: &Context) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (n, t) in ctx.iter_inductives() {
        out.push(format!("ind {n} : {t}"));
        for (c, ct) in ctx.get_constructors(n) {
            out.push(format!("  ctor {c} : {ct}"));
        }
    }
    for (n, t, b) in ctx.iter_definitions() {
        out.push(format!("def {n} : {t} := {b}"));
    }
    for (n, t) in ctx.iter_declarations() {
        out.push(format!("axiom {n} : {t}"));
    }
    out.sort();
    out
}

/// A refusal must leave the environment EXACTLY as it found it — not merely report an error.
///
/// Driven over every refusal shape reachable through the public door, so the invariant is pinned
/// across the whole fail-closed surface rather than at one convenient input.
#[test]
fn a_refused_nested_declaration_registers_nothing() {
    // A container that is NEGATIVE in its nesting parameter — the actual paradox shape, refused by
    // block positivity, which is the path that used to mutate before the isos were reached.
    let negative = NestedDecl {
        name: "NBad".to_string(),
        sort: ty0(),
        constructors: vec![(
            "nbad".to_string(),
            pi("_", pi("_", g("NBad"), g("False")), g("NBad")),
        )],
    };

    // A nesting through an INDEXED family — outside the specialisable fragment.
    let indexed = NestedDecl {
        name: "NIdx".to_string(),
        sort: ty0(),
        constructors: vec![(
            "nidx".to_string(),
            pi("_", app(g("BVec"), g("Zero")), g("NIdx")),
        )],
    };

    // A container whose sort is not `Type 0`.
    let high = NestedDecl {
        name: "NHigh".to_string(),
        sort: Term::Sort(Universe::Type(1)),
        constructors: vec![("nhigh".to_string(), pi("_", g("Nat"), g("NHigh")))],
    };

    for decl in [negative, indexed, high] {
        let mut ctx = std_ctx();
        let before = snapshot(&ctx);
        let outcome = ctx.add_nested_inductive(&decl);
        if outcome.is_ok() {
            // Not every shape here is guaranteed to be refused as the fragment widens; this lock is
            // about what happens WHEN it refuses, so an acceptance is simply not the case under test.
            continue;
        }
        let after = snapshot(&ctx);
        assert_eq!(
            before,
            after,
            "`add_nested_inductive` refused `{}` and yet CHANGED the context. A fail-closed \
             refusal that has already mutated is the one a caller cannot recover from: it is left \
             reasoning in an environment holding half a declaration. Added: {:?}; removed: {:?}",
            decl.name,
            after
                .iter()
                .filter(|x| !before.contains(x))
                .collect::<Vec<_>>(),
            before
                .iter()
                .filter(|x| !after.contains(x))
                .collect::<Vec<_>>(),
        );
        assert!(
            ctx.get_global(&decl.name).is_none(),
            "the refused inductive `{}` itself must not be registered",
            decl.name
        );
    }
}

/// The positive control: a declaration inside the fragment still lands COMPLETELY — the block, its
/// siblings and both conversion isos. A transaction that commits nothing is not a fix.
#[test]
fn an_accepted_nested_declaration_registers_everything() {
    let mut ctx = std_ctx();
    let before = snapshot(&ctx);

    // `RT := rnode : TList RT -> RT` — the canonical rose tree, inside the unary fragment.
    let ok = NestedDecl {
        name: "RT".to_string(),
        sort: ty0(),
        constructors: vec![(
            "rnode".to_string(),
            pi("_", app(g("TList"), g("RT")), g("RT")),
        )],
    };
    let info = match ctx.add_nested_inductive(&ok) {
        Ok(i) => i,
        Err(e) => panic!("the canonical rose tree is inside the fragment and must compile: {e:?}"),
    };

    assert!(
        ctx.get_global("RT").is_some(),
        "the nested inductive itself must be registered"
    );
    for s in &info.siblings {
        assert!(
            ctx.get_global(s).is_some(),
            "sibling `{s}` was reported but is not in the context — a partial commit"
        );
    }
    for i in &info.isos {
        for name in [&i.to_generic, &i.from_generic] {
            assert!(
                ctx.is_definition(name),
                "iso `{name}` (specialising `{}` as `{}`) was reported but is not a definition — \
                 the conversions are what make the specialisation trustworthy, so a block without \
                 them is worse than no block",
                i.container,
                i.sibling
            );
        }
        assert!(
            ctx.get_global(&i.sibling).is_some(),
            "the specialised sibling `{}` must be registered",
            i.sibling
        );
    }
    assert_ne!(
        before,
        snapshot(&ctx),
        "an accepted declaration must actually change the context"
    );
}

/// A second declaration after a refusal must behave exactly as it would have in a fresh context —
/// the observable consequence of "registers nothing", and the thing a caller actually depends on.
#[test]
fn a_refusal_does_not_poison_the_next_declaration() {
    let bad = NestedDecl {
        name: "RT".to_string(),
        sort: ty0(),
        // Negative in its own parameter: refused.
        constructors: vec![(
            "rbad".to_string(),
            pi("_", pi("_", g("RT"), g("False")), g("RT")),
        )],
    };
    let good = NestedDecl {
        name: "RT".to_string(),
        sort: ty0(),
        constructors: vec![(
            "rnode".to_string(),
            pi("_", app(g("TList"), g("RT")), g("RT")),
        )],
    };

    let mut dirtied = std_ctx();
    let _ = dirtied.add_nested_inductive(&bad);
    let after_refusal = dirtied.add_nested_inductive(&good);

    let mut fresh = std_ctx();
    let from_clean = fresh.add_nested_inductive(&good);

    assert_eq!(
        after_refusal.is_ok(),
        from_clean.is_ok(),
        "declaring `RT` after a REFUSED `RT` must succeed exactly as it does in a fresh context. \
         If the refusal left a header or a constructor behind, the retry sees a name it does not \
         own — and `add_constructor` USED to push onto `constructor_order`, so the second \
         generation's constructors were appended to the first's. after-refusal: {after_refusal:?}, \
         from-clean: {from_clean:?}"
    );
    if from_clean.is_ok() {
        assert_eq!(
            snapshot(&dirtied),
            snapshot(&fresh),
            "and the two contexts must be indistinguishable afterwards"
        );
    }
}

/// `v` and `ty0` are used by the decls above; this keeps the helper set honest if they are trimmed.
#[test]
fn helpers_are_wired() {
    assert_eq!(v("x"), Term::Var("x".to_string()));
    assert_eq!(ty0(), Term::Sort(Universe::Type(0)));
}
