use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, infer_type, recheck, BigInt, Context, Literal, Term, Universe,
};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(f, app)
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn nn(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn power(n: i64) -> Term {
    apps(g("Nat_pow"), &[nn(2), nn(n)])
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn predicate(bound: Term) -> Term {
    lam(
        "fresh_neutral",
        g("Nat"),
        apps(g("natLe"), &[v("fresh_neutral"), bound]),
    )
}
fn check_conversion(c: &mut Context, carrier: Term, a: Term, b: Term) -> Term {
    c.add_declaration("ProbeFamily", pi("value", carrier, ty()));
    c.add_declaration("probe_witness", app(g("ProbeFamily"), a));
    Term::Let {
        name: "checked".into(),
        ty: Box::new(app(g("ProbeFamily"), b)),
        value: Box::new(g("probe_witness")),
        body: Box::new(v("checked")),
    }
}
#[test]
fn neutral_computed_bound_functions_remain_distinct_without_recursively_typing_expanded_arguments()
{
    let mut c = context();
    let a = predicate(power(8));
    let b = predicate(power(4));
    let carrier = pi("n", g("Nat"), g("Bool"));
    assert!(defeq_for_test(&c, &infer_type(&c, &a).unwrap(), &carrier));
    assert!(defeq_for_test(&c, &recheck(&c, &b).unwrap(), &carrier));
    let term = check_conversion(&mut c, carrier, a, b);
    assert!(infer_type(&c, &term).is_err());
    assert!(recheck(&c, &term).is_err());
}
#[test]
fn neutral_equal_computed_bound_functions_remain_convertible() {
    let mut c = context();
    let carrier = pi("n", g("Nat"), g("Bool"));
    let term = check_conversion(&mut c, carrier, predicate(power(8)), predicate(nn(256)));
    infer_type(&c, &term).unwrap();
    recheck(&c, &term).unwrap();
}
fn neutral_fix(result: Term, zero: Term, step: Term) -> Term {
    Term::Fix {
        name: "arbitrary_function_name".into(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            lam(
                "irrelevant_bound",
                g("Nat"),
                Term::Match {
                    discriminant: Box::new(v("n")),
                    motive: Box::new(lam("unrelated_motive_name", g("Nat"), result)),
                    cases: vec![zero, lam("predecessor", g("Nat"), step)],
                },
            ),
        )),
    }
}
#[test]
fn applied_fixpoint_unit_results_keep_unit_eta_in_both_kernels() {
    let mut c = context();
    c.add_inductive("ProbeUnit", ty());
    c.add_constructor_checked("probe_unit", "ProbeUnit", g("ProbeUnit"))
        .unwrap();
    c.add_declaration("neutral_n", g("Nat"));
    let f = neutral_fix(g("ProbeUnit"), g("probe_unit"), g("probe_unit"));
    let a = apps(f.clone(), &[g("neutral_n"), nn(0)]);
    let b = apps(f, &[g("neutral_n"), nn(1)]);
    assert_eq!(infer_type(&c, &a).unwrap(), g("ProbeUnit"));
    assert!(defeq_for_test(&c, &a, &b));
    let term = check_conversion(&mut c, g("ProbeUnit"), a, b);
    infer_type(&c, &term).unwrap();
    recheck(&c, &term).unwrap();
}
#[test]
fn applied_fixpoint_proposition_results_keep_proof_irrelevance_in_both_kernels() {
    let mut c = context();
    c.add_inductive("ProbeProp", Term::Sort(Universe::Prop));
    c.add_constructor_checked("probe_proof", "ProbeProp", g("ProbeProp"))
        .unwrap();
    c.add_declaration("neutral_n", g("Nat"));
    let f = neutral_fix(g("ProbeProp"), g("probe_proof"), g("probe_proof"));
    let a = apps(f.clone(), &[g("neutral_n"), nn(0)]);
    let b = apps(f, &[g("neutral_n"), nn(1)]);
    assert_eq!(infer_type(&c, &a).unwrap(), g("ProbeProp"));
    assert!(defeq_for_test(&c, &a, &b));
    let term = check_conversion(&mut c, g("ProbeProp"), a, b);
    infer_type(&c, &term).unwrap();
    recheck(&c, &term).unwrap();
}
#[test]
fn renamed_fixpoint_data_results_preserve_distinct_neutral_computations() {
    let mut c = context();
    c.add_declaration("neutral_n", g("Nat"));
    let f = neutral_fix(
        g("Nat"),
        v("irrelevant_bound"),
        app(g("Succ"), v("irrelevant_bound")),
    );
    let a = apps(f.clone(), &[g("neutral_n"), nn(0)]);
    let b = apps(f, &[g("neutral_n"), nn(1)]);
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(recheck(&c, &b).unwrap(), g("Nat"));
    assert!(!defeq_for_test(&c, &a, &b));
    let term = check_conversion(&mut c, g("Nat"), a, b);
    assert!(infer_type(&c, &term).is_err());
    assert!(recheck(&c, &term).is_err());
}
