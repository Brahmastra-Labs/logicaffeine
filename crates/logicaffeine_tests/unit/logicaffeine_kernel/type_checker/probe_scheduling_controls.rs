//! Scheduling controls: a bounded conversion success needs no speculative typing.
use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn alias() -> Context {
    let mut c = Context::new();
    c.declare_universe_poly(
        "level_alias",
        vec!["u".into()],
        Term::Sort(Universe::Succ(Box::new(Universe::Var("u".into())))),
        Term::Sort(Universe::Var("u".into())),
    )
    .unwrap();
    c
}
fn at(n: u32) -> Term {
    Term::Const {
        name: "level_alias".into(),
        levels: vec![Universe::Type(n)],
    }
}
fn measured(c: &Context, a: &Term, b: &Term) -> (bool, usize) {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONVERSION_PROOF_TYPE_PROBES.with(|n| n.set(self.0));
        }
    }
    let _restore = Restore(CONVERSION_PROOF_TYPE_PROBES.with(|n| n.replace(Some(0))));
    // Exercise this scheduling decision, independently of a surrounding positive cache.
    let equal = def_eq_uncached(c, a, b);
    let count = CONVERSION_PROOF_TYPE_PROBES.with(|n| n.get().unwrap());
    (equal, count)
}
fn witness(c: &mut Context, carrier: Term, a: Term, b: Term) {
    c.add_declaration("ProbeFamily", pi("x", carrier, sort(0)));
    c.add_declaration("probe_witness", app(g("ProbeFamily"), a));
    let checked = Term::Let {
        name: "result".into(),
        ty: Box::new(app(g("ProbeFamily"), b)),
        value: Box::new(g("probe_witness")),
        body: Box::new(v("result")),
    };
    infer_type(c, &checked).unwrap();
    crate::recheck(c, &checked).unwrap();
}

#[test]
fn checked_explicit_delta_avoids_speculative_proof_typing_in_both_orders() {
    let c = alias();
    let a = at(0);
    let b = sort(0);
    assert!(alpha_eq(&infer_type(&c, &a).unwrap(), &sort(1)));
    assert!(alpha_eq(&crate::recheck(&c, &a).unwrap(), &sort(1)));
    assert_eq!(infer_type(&c, &b).unwrap(), sort(1));
    assert_eq!(crate::recheck(&c, &b).unwrap(), sort(1));
    for (a, b) in [(&a, &b), (&b, &a)] {
        assert!(bounded_conversion(&c, a, b, &mut 256));
        let (equal, probes) = measured(&c, a, b);
        assert!(equal);
        assert_eq!(probes, 0,
            "a checked explicit delta equality must resolve before typing endpoints for proof irrelevance");
    }
}

#[test]
fn explicit_delta_equality_is_valid_in_an_invariant_family_in_both_kernels() {
    for reverse in [false, true] {
        let mut c = alias();
        let (a, b) = if reverse {
            (sort(0), at(0))
        } else {
            (at(0), sort(0))
        };
        witness(&mut c, sort(1), a, b);
    }
}

#[test]
fn propositional_and_strict_propositional_fallbacks_still_type_both_endpoints() {
    for level in [Universe::Prop, Universe::SProp] {
        let mut c = Context::new();
        c.add_inductive("Claim", Term::Sort(level));
        c.add_declaration("first", g("Claim"));
        c.add_declaration("second", g("Claim"));
        for (a, b) in [(g("first"), g("second")), (g("second"), g("first"))] {
            assert!(!bounded_conversion(&c, &a, &b, &mut 256));
            let (equal, probes) = measured(&c, &a, &b);
            assert!(equal);
            assert!(
                probes > 0,
                "inconclusive bounded conversion must retain proof typing"
            );
        }
        witness(&mut c, g("Claim"), g("first"), g("second"));
    }
}

#[test]
fn neutral_unit_inhabitants_retain_unit_eta_fallback() {
    let mut c = Context::new();
    c.add_inductive("ProbeUnit", sort(0));
    c.add_constructor("unit_value", "ProbeUnit", g("ProbeUnit"));
    c.add_declaration("first", g("ProbeUnit"));
    c.add_declaration("second", g("ProbeUnit"));
    assert!(!bounded_conversion(&c, &g("first"), &g("second"), &mut 256));
    assert!(def_eq(&c, &g("first"), &g("second")));
    assert!(def_eq(&c, &g("second"), &g("first")));
    witness(&mut c, g("ProbeUnit"), g("first"), g("second"));
}

#[test]
fn different_explicit_levels_do_not_certify_an_invariant_cast() {
    let mut c = alias();
    assert!(!bounded_conversion(&c, &at(1), &sort(0), &mut 256));
    assert!(!def_eq(&c, &at(1), &sort(0)));
    assert!(!def_eq(&c, &sort(0), &at(1)));
    c.add_declaration("LevelFamily", pi("A", sort(2), sort(0)));
    c.add_declaration("level_witness", app(g("LevelFamily"), at(1)));
    let bad = Term::Let {
        name: "bad".into(),
        ty: Box::new(app(g("LevelFamily"), sort(0))),
        value: Box::new(g("level_witness")),
        body: Box::new(v("bad")),
    };
    assert!(infer_type(&c, &bad).is_err());
    assert!(crate::recheck(&c, &bad).is_err());
}

#[test]
fn explicit_level_arity_remains_checked() {
    let c = alias();
    for levels in [vec![], vec![Universe::Type(0), Universe::Type(1)]] {
        let malformed = Term::Const {
            name: "level_alias".into(),
            levels,
        };
        assert!(!bounded_conversion(&c, &malformed, &sort(0), &mut 256));
        assert!(infer_type(&c, &malformed).is_err());
        assert!(crate::recheck(&c, &malformed).is_err());
    }
}

#[test]
fn erased_holes_do_not_enter_the_guarded_head_probe() {
    let c = Context::new();
    let erased = app(lam("unused", sort(1), sort(0)), Term::Hole);
    assert!(!conversion_by_unfolded_head(&c, &erased, &sort(0)));
    assert!(!conversion_by_unfolded_head(&c, &sort(0), &erased));
}

#[test]
fn rewrite_contexts_retain_the_existing_head_probe_gate() {
    let mut c = alias();
    c.add_inductive("Input", sort(0));
    c.add_constructor("input", "Input", g("Input"));
    c.add_declaration(
        "Eq",
        pi(
            "A",
            sort(0),
            pi("x", v("A"), pi("y", v("A"), Term::Sort(Universe::Prop))),
        ),
    );
    c.add_declaration("opaque", pi("x", g("Input"), g("Input")));
    let eq = app(
        app(app(g("Eq"), g("Input")), app(g("opaque"), g("input"))),
        g("input"),
    );
    c.add_declaration("rewrite_input", eq);
    c.add_simp_lemma("rewrite_input").unwrap();
    assert!(!c.simp_rules().is_empty());
    assert!(!conversion_by_unfolded_head(&c, &at(0), &sort(0)));
}

#[test]
fn open_definition_bodies_cannot_capture_local_names() {
    let mut c = Context::new();
    c.add_definition("open".into(), sort(1), v("local"));
    let c = c.extend("local", sort(1));
    assert!(!bounded_conversion(&c, &g("open"), &v("local"), &mut 256));
    assert!(!bounded_conversion(&c, &v("local"), &g("open"), &mut 256));
}

#[test]
fn opaque_names_do_not_gain_a_delta_body() {
    let mut c = Context::new();
    c.add_declaration("opaque_type", sort(1));
    assert!(!bounded_conversion(
        &c,
        &g("opaque_type"),
        &sort(0),
        &mut 256
    ));
    assert!(!def_eq(&c, &g("opaque_type"), &sort(0)));
}

#[test]
fn a_failed_or_cyclic_probe_never_refills_its_allowance() {
    let c = alias();
    for allowance in [0, 1] {
        let mut budget = allowance;
        assert!(!bounded_conversion(&c, &at(0), &sort(0), &mut budget));
        assert_eq!(budget, 0);
    }
    let mut c = Context::new();
    assert!(
        c.declare("cycle", sort(1), g("cycle")).is_err(),
        "the gateway refuses a δ-cycle"
    );
    assert!(c.get_definition_body("cycle").is_none());
    let length = 200;
    c.declare(&format!("chain{length}"), sort(1), sort(0))
        .unwrap();
    for i in (0..length).rev() {
        c.declare(&format!("chain{i}"), sort(1), g(&format!("chain{}", i + 1)))
            .unwrap();
    }
    let mut budget = 32;
    assert!(!bounded_conversion(&c, &g("chain0"), &sort(0), &mut budget));
    assert_eq!(
        budget, 0,
        "an exhausted δ-probe never refills its allowance"
    );
    let mut budget = 1 << 16;
    assert!(
        bounded_conversion(&c, &g("chain0"), &sort(0), &mut budget),
        "the chain does meet `Sort 0`"
    );
}
