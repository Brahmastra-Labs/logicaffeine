use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, Context, Literal, Term};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn value(ty: &Term) -> Term {
    match ty {
        Term::Global(n) if n == "Syntax" => app(g("SName"), Term::Lit(Literal::Text("x".into()))),
        Term::Global(n) if n == "Derivation" => app(g("DAxiom"), value(&g("Syntax"))),
        Term::Global(n) if n == "Int" => Term::Lit(Literal::Int(0)),
        Term::Global(n) if n == "Nat" => g("Zero"),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => Term::Lambda {
            param: "ignored".into(),
            param_type: param_type.clone(),
            body: Box::new(value(body_type)),
        },
        Term::App(head, elem) if **head == g("TList") => app(g("TNil"), (**elem).clone()),
        _ => panic!("no test value for {ty}"),
    }
}
#[test]
fn reflected_computation_preserves_a_redeclared_result_type() {
    let mut base = Context::new();
    StandardLibrary::register(&mut base);
    for name in logicaffeine_kernel::recheck::primitive_rule_names() {
        let mut ctx = base.clone();
        let mut params = Vec::new();
        let mut result = ctx.get_global(name).unwrap();
        while let Term::Pi {
            param_type,
            body_type,
            ..
        } = result
        {
            params.push((**param_type).clone());
            result = body_type;
        }
        let replacement = if *result == g("Nat") {
            g("Int")
        } else {
            g("Nat")
        };
        let new_type = params
            .iter()
            .rev()
            .fold(replacement.clone(), |tail, param| {
                arrow(param.clone(), tail)
            });
        ctx.add_declaration(name, new_type);
        let term = params.iter().fold(g(name), |f, ty| app(f, value(ty)));
        assert_eq!(infer_type(&ctx, &term).unwrap(), replacement, "{name}");
        let reduced = normalize(&ctx, &term);
        assert_eq!(
            infer_type(&ctx, &reduced).unwrap(),
            replacement,
            "{name}: {term} reduced to {reduced}"
        );
    }
}
