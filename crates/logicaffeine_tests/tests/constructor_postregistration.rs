//! Legal hidden telescopes must remain usable after checked publication.
use logicaffeine_kernel::{infer_type, recheck, Context, MutualInductive, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.set_auto_auxiliaries(false);
    ctx.add_inductive("Token", sort(0));
    ctx.add_constructor("token", "Token", g("Token"));
    ctx
}
fn wrap(ctx: &mut Context, term: Term, alias: bool) -> Term {
    let ty = infer_type(ctx, &term).unwrap();
    if alias {
        ctx.declare("TelescopeAlias", ty, term).unwrap();
        assert!(ctx.get_definition_body("TelescopeAlias").is_some());
        g("TelescopeAlias")
    } else {
        Term::Let {
            name: "T".into(),
            ty: Box::new(ty),
            value: Box::new(term),
            body: Box::new(v("T")),
        }
    }
}
fn header() -> Term {
    pi("A", sort(0), sort(0))
}
fn ctor() -> Term {
    pi("A", sort(0), pi("x", v("A"), app(g("Box"), v("A"))))
}
fn use_registered(ctx: &Context) {
    assert_eq!(
        ctx.inductive_arity("Box"),
        1,
        "exposed parameter telescope determines arity"
    );
    assert_eq!(
        ctx.inductive_num_params("Box"),
        1,
        "the uniform parameter is not a stored field"
    );
    let term = app(app(g("box"), g("Token")), g("token"));
    let expected = app(g("Box"), g("Token"));
    assert_eq!(infer_type(ctx, &term).unwrap(), expected);
    assert_eq!(recheck(ctx, &term).unwrap(), expected);
}
fn parent_header(alias: bool) {
    let mut ctx = context();
    let sort = wrap(&mut ctx, header(), alias);
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "Box".into(),
        sort,
        num_params: 1,
        constructors: vec![("box".into(), ctor())],
    }])
    .expect("legal hidden parent telescope");
    use_registered(&ctx);
}
fn constructor_header(alias: bool) {
    let mut ctx = context();
    ctx.add_indexed_inductive("Box", header(), 1);
    let ty = wrap(&mut ctx, ctor(), alias);
    ctx.add_constructor_checked("box", "Box", ty)
        .expect("legal hidden constructor telescope");
    use_registered(&ctx);
}
#[test]
fn checked_alias_parent_telescope_preserves_registered_arity_and_application() {
    parent_header(true)
}
#[test]
fn let_parent_telescope_preserves_registered_arity_and_application() {
    parent_header(false)
}
#[test]
fn checked_alias_constructor_preserves_parameters_and_application() {
    constructor_header(true)
}
#[test]
fn let_constructor_preserves_parameters_and_application() {
    constructor_header(false)
}
