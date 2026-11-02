//! Constructor pruning demands index heads even when the family head is already exposed.
//! These are fresh controls; prior RED assertions remain unchanged.
use logicaffeine_kernel::{infer_type, recheck, Context, MutualInductive, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
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
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn family(index: Term) -> Term {
    app(g("TwoIndices"), index)
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", sort());
    c.add_constructor_checked("Zero", "Nat", g("Nat")).unwrap();
    c.add_constructor_checked("Succ", "Nat", pi("n", g("Nat"), g("Nat")))
        .unwrap();
    c.add_mutual_inductives(&[MutualInductive {
        name: "TwoIndices".into(),
        sort: pi("n", g("Nat"), sort()),
        num_params: 0,
        constructors: vec![
            ("TwoIndices.zero".into(), family(g("Zero"))),
            ("TwoIndices.one".into(), family(app(g("Succ"), g("Zero")))),
        ],
    }])
    .unwrap();
    c.declare("zeroAlias", g("Nat"), g("Zero")).unwrap();
    c.declare(
        "FamilyAlias",
        pi("n", g("Nat"), sort()),
        lam("n", g("Nat"), family(v("n"))),
    )
    .unwrap();
    c
}
fn index(mode: usize) -> Term {
    match mode {
        0 => g("Zero"),
        1 => app(lam("n", g("Nat"), v("n")), g("Zero")),
        2 => Term::Let {
            name: "n".into(),
            ty: Box::new(g("Nat")),
            value: Box::new(g("Zero")),
            body: Box::new(v("n")),
        },
        3 => g("zeroAlias"),
        _ => unreachable!(),
    }
}
fn matched(domain: Term, full: bool, wrong_motive: bool) -> Term {
    let generic = if wrong_motive { g("Zero") } else { v("index") };
    lam(
        "major",
        domain,
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam(
                "index",
                g("Nat"),
                lam("witness", family(generic), g("Nat")),
            )),
            cases: if full {
                vec![g("Zero"), g("Zero")]
            } else {
                vec![g("Zero")]
            },
        },
    )
}
fn fixture(mode: usize, alias: bool, full: bool) -> (Context, Term) {
    let c = context();
    let domain = app(
        g(if alias { "FamilyAlias" } else { "TwoIndices" }),
        index(mode),
    );
    (c, matched(domain, full, false))
}
fn main_accept(mode: usize, alias: bool) {
    let (c, t) = fixture(mode, alias, false);
    let result = infer_type(&c, &t);
    assert!(
        result.is_ok(),
        "valid pruned indexed match, mode {mode}, family alias {alias}: {result:?}"
    );
}
fn independent_accept(mode: usize, alias: bool) {
    let (c, t) = fixture(mode, alias, false);
    let result = recheck(&c, &t);
    assert!(
        result.is_ok(),
        "independent valid pruned match, mode {mode}, family alias {alias}: {result:?}"
    );
}

#[test]
fn direct_constructor_index_prunes_in_main() {
    main_accept(0, false)
}
#[test]
fn direct_constructor_index_prunes_independently() {
    independent_accept(0, false)
}
#[test]
fn beta_index_prunes_in_main() {
    main_accept(1, false)
}
#[test]
fn beta_index_prunes_independently() {
    independent_accept(1, false)
}
#[test]
fn let_index_prunes_in_main() {
    main_accept(2, false)
}
#[test]
fn let_index_prunes_independently() {
    independent_accept(2, false)
}
#[test]
fn delta_index_prunes_in_main() {
    main_accept(3, false)
}
#[test]
fn delta_index_prunes_independently() {
    independent_accept(3, false)
}
#[test]
fn transparent_family_and_index_synonyms_prune_in_main() {
    main_accept(3, true)
}
#[test]
fn transparent_family_and_index_synonyms_prune_independently() {
    independent_accept(3, true)
}

#[test]
fn full_coverage_remains_valid_for_all_reducible_suffixes() {
    for mode in 0..4 {
        let (c, t) = fixture(mode, true, true);
        let main = infer_type(&c, &t);
        let independent = recheck(&c, &t);
        assert!(main.is_ok(), "mode {mode}: {main:?}");
        assert!(independent.is_ok(), "mode {mode}: {independent:?}");
    }
}

#[test]
fn genuinely_reachable_constructor_cannot_be_omitted() {
    let c = context();
    let t = lam(
        "unknown",
        g("Nat"),
        matched(family(v("unknown")), false, false),
    );
    assert!(
        infer_type(&c, &t).is_err(),
        "an unknown index still requires both branches"
    );
    assert!(
        recheck(&c, &t).is_err(),
        "independent coverage must retain the reachable branch"
    );
}

#[test]
fn reducible_suffix_does_not_hide_wrong_generic_motive() {
    let c = context();
    let t = matched(family(g("zeroAlias")), true, true);
    assert!(
        infer_type(&c, &t).is_err(),
        "a specialized major is not a generic indexed motive"
    );
    assert!(
        recheck(&c, &t).is_err(),
        "independent generic motive validation still applies"
    );
}

fn nested_fixture() -> (Context, Term) {
    let mut c = context();
    let deep = |n| app(g("DeepIndices"), n);
    c.add_mutual_inductives(&[MutualInductive {
        name: "DeepIndices".into(),
        sort: pi("n", g("Nat"), sort()),
        num_params: 0,
        constructors: vec![
            ("DeepIndices.one".into(), deep(app(g("Succ"), g("Zero")))),
            (
                "DeepIndices.two".into(),
                deep(app(g("Succ"), app(g("Succ"), g("Zero")))),
            ),
        ],
    }])
    .unwrap();
    let t = lam(
        "major",
        deep(app(g("Succ"), g("zeroAlias"))),
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam("n", g("Nat"), lam("witness", deep(v("n")), g("Nat")))),
            cases: vec![g("Zero")],
        },
    );
    (c, t)
}

#[test]
fn nested_constructor_suffix_demands_inner_alias_in_main() {
    let (c, t) = nested_fixture();
    let result = infer_type(&c, &t);
    assert!(
        result.is_ok(),
        "nested alias must expose the inner disjoint constructor: {result:?}"
    );
}
#[test]
fn nested_constructor_suffix_demands_inner_alias_independently() {
    let (c, t) = nested_fixture();
    let result = recheck(&c, &t);
    assert!(
        result.is_ok(),
        "independent nested alias must expose the inner disjoint constructor: {result:?}"
    );
}

fn long_alias_fixture() -> (Context, Term) {
    let mut c = context();
    let mut previous = "Zero".to_owned();
    for i in 0..300 {
        let name = format!("indexAlias{i}");
        c.declare(&name, g("Nat"), g(&previous)).unwrap();
        previous = name;
    }
    (c, matched(family(g(&previous)), false, false))
}
#[test]
fn index_alias_chain_beyond_fast_probe_in_main() {
    let (c, t) = long_alias_fixture();
    let result = infer_type(&c, &t);
    assert!(
        result.is_ok(),
        "general demanded-index fallback must retain long alias support: {result:?}"
    );
}
#[test]
fn index_alias_chain_beyond_fast_probe_independently() {
    let (c, t) = long_alias_fixture();
    let result = recheck(&c, &t);
    assert!(
        result.is_ok(),
        "independent long index aliases remain supported: {result:?}"
    );
}
fn wide_index_fixture() -> (Context, Term) {
    fn wide(depth: usize) -> Term {
        if depth == 0 {
            g("Zero")
        } else {
            app(app(v("join"), wide(depth - 1)), wide(depth - 1))
        }
    }
    let c = context();
    let large_index = app(lam("ignored", g("Nat"), g("Zero")), wide(11));
    let t = lam(
        "join",
        pi("a", g("Nat"), pi("b", g("Nat"), g("Nat"))),
        matched(family(large_index), false, false),
    );
    (c, t)
}
#[test]
fn index_tree_beyond_fast_probe_in_main() {
    let (c, t) = wide_index_fixture();
    let result = infer_type(&c, &t);
    assert!(
        result.is_ok(),
        "a shallow wide index must retain checked fallback support: {result:?}"
    );
}
#[test]
fn index_tree_beyond_fast_probe_independently() {
    let (c, t) = wide_index_fixture();
    let result = recheck(&c, &t);
    assert!(
        result.is_ok(),
        "independent shallow wide index remains supported: {result:?}"
    );
}
#[test]
fn opaque_global_index_keeps_all_reachable_branches() {
    let mut c = context();
    c.add_declaration("opaqueIndex", g("Nat"));
    let incomplete = matched(family(g("opaqueIndex")), false, false);
    assert!(
        infer_type(&c, &incomplete).is_err(),
        "opaque index is not evidence of a clash"
    );
    assert!(
        recheck(&c, &incomplete).is_err(),
        "independent opaque index keeps both cases"
    );
    let complete = matched(family(g("opaqueIndex")), true, false);
    assert!(
        infer_type(&c, &complete).is_ok(),
        "full coverage works for an opaque index"
    );
    assert!(
        recheck(&c, &complete).is_ok(),
        "independent full coverage works for an opaque index"
    );
}
