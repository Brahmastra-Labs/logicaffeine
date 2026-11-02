//! Frozen contracts for generic native call templates. No original-case names
//! or source indices grant compilation or conversion authority.
use super::*;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn sort(level: u32) -> Term {
    Term::Sort(Universe::Type(level))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(name: &str, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}
fn lam(name: &str, domain: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(domain),
        body: Box::new(body),
    }
}
fn function_type(level: u32) -> Term {
    pi("x", sort(level), sort(level))
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.add_declaration("A", sort(0));
    ctx.add_declaration("B", sort(0));
    ctx.add_declaration("combine", pi("a", sort(0), pi("b", sort(0), sort(0))));
    ctx
}
fn original() -> Term {
    lam("f", function_type(0), app(v("f"), g("A")))
}
fn program(argument: Term) -> Program {
    Program {
        parameters: vec![("f".into(), function_type(0))],
        grounds: vec![argument],
        calls: vec![CallInstruction {
            function: TemplateRef::Argument(0),
            arguments: vec![TemplateRef::Ground(0)],
        }],
        result: TemplateRef::Call(0),
    }
}
fn required(ctx: &Context, original: &Term, budget: &mut usize) -> CheckedTemplate {
    match compile(ctx, original, budget).unwrap_or_else(|e| panic!("template compiler: {e:?}")) {
        Some(checked) => checked,
        None => panic!("small supported native lambda needs a checked template"),
    }
}
fn goal(ctx: &Context, body: Term, expected: &Term) {
    let annotated = Term::Let {
        name: "template.goal".into(),
        ty: Box::new(expected.clone()),
        value: Box::new(body),
        body: Box::new(v("template.goal")),
    };
    lean_type(ctx, &annotated).unwrap();
    crate::recheck(ctx, &annotated).unwrap();
}

#[test]
fn a_small_parameterized_call_compiles_and_reifies_with_its_actual_type() {
    let ctx = context();
    let original = original();
    let mut allowance = 100_000;
    let checked = required(&ctx, &original, &mut allowance);
    assert_eq!(checked.program().parameters.len(), 1);
    assert_eq!(
        checked.program().calls.len(),
        1,
        "one parameter call is one static instruction"
    );
    let rebuilt = reify(checked.program(), &mut allowance).unwrap();
    goal(&ctx, rebuilt, checked.actual_type());
    assert!(definitionally_equal(
        &ctx,
        checked.actual_type(),
        &lean_type(&ctx, &original).unwrap()
    ));
    assert!(allowance > 0 && allowance < 100_000);
}

#[test]
fn different_formal_arguments_keep_their_order_in_the_call_dag() {
    let ctx = context();
    let original = lam(
        "left",
        function_type(0),
        lam(
            "right",
            function_type(0),
            app(
                app(g("combine"), app(v("left"), g("A"))),
                app(v("right"), g("B")),
            ),
        ),
    );
    let checked = required(&ctx, &original, &mut 100_000);
    let code = checked.program();
    assert_eq!(code.parameters.len(), 2);
    let formal_calls: Vec<_> = code
        .calls
        .iter()
        .filter_map(|call| match call.function {
            TemplateRef::Argument(slot) => Some(slot),
            _ => None,
        })
        .collect();
    assert!(formal_calls.contains(&0) && formal_calls.contains(&1));
    let rebuilt = reify(code, &mut 100_000).unwrap();
    let left = lam("x", sort(0), g("A"));
    let right = lam("x", sort(0), g("B"));
    let actual = app(app(rebuilt, left), right);
    let expected = app(app(g("combine"), g("A")), g("B"));
    native_conversion::NativeCertificates::new(&ctx, &mut 100_000)
        .unwrap()
        .conversion(&actual, &expected, &sort(0), &mut 100_000)
        .unwrap();
}

#[test]
fn a_let_shadowing_a_formal_does_not_refer_to_the_hidden_argument() {
    let ctx = context();
    let identity = lam("x", sort(0), v("x"));
    let body = Term::Let {
        name: "f".into(),
        ty: Box::new(function_type(0)),
        value: Box::new(identity),
        body: Box::new(app(v("f"), g("A"))),
    };
    let original = lam("f", function_type(0), body);
    let checked = required(&ctx, &original, &mut 100_000);
    let rebuilt = reify(checked.program(), &mut 100_000).unwrap();
    let actual = app(rebuilt, lam("x", sort(0), g("B")));
    native_conversion::NativeCertificates::new(&ctx, &mut 100_000)
        .unwrap()
        .conversion(&actual, &g("A"), &sort(0), &mut 100_000)
        .unwrap();
}

#[test]
fn explicit_universe_instances_survive_ground_slots() {
    let mut ctx = context();
    ctx.add_universe_poly_declaration("U", vec!["u".into()], Term::Sort(Universe::Var("u".into())))
        .unwrap();
    for level in [0, 1] {
        let constant = Term::Const {
            name: "U".into(),
            levels: vec![Universe::Type(level)],
        };
        let original = lam("f", function_type(level), app(v("f"), constant.clone()));
        let checked = required(&ctx, &original, &mut 100_000);
        assert!(checked
            .program()
            .grounds
            .iter()
            .any(|ground| *ground == constant));
        let rebuilt = reify(checked.program(), &mut 100_000).unwrap();
        goal(&ctx, rebuilt, checked.actual_type());
    }
}

#[test]
fn validator_rejects_a_different_same_typed_ground_operand() {
    let ctx = context();
    assert!(matches!(
        validate(&ctx, &original(), program(g("B")), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn validator_rejects_forward_call_edges_and_out_of_scope_arguments() {
    let ctx = context();
    let mut forward = program(g("A"));
    forward.calls[0].function = TemplateRef::Call(0);
    assert!(matches!(
        validate(&ctx, &original(), forward, &mut 100_000),
        Err(Verdict::Reject(_))
    ));
    let mut missing = program(g("A"));
    missing.calls[0].function = TemplateRef::Argument(1);
    assert!(matches!(
        validate(&ctx, &original(), missing, &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn erasing_a_bad_original_argument_cannot_make_a_template_valid() {
    let ctx = context();
    let bad = lam(
        "f",
        function_type(0),
        app(lam("ignored", sort(0), g("A")), sort(0)),
    );
    assert!(matches!(
        compile(&ctx, &bad, &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn cached_structure_does_not_authorize_unbudgeted_payload_readback() {
    let mut ctx = context();
    let name = "long_type_".repeat(128);
    ctx.add_declaration(&name, sort(0));
    let original = lam("f", function_type(0), app(v("f"), g(&name)));
    let checked = required(&ctx, &original, &mut 100_000);
    let mut allowance = 32;
    assert!(matches!(
        reify(checked.program(), &mut allowance),
        Err(Verdict::Decline(_))
    ));
    assert!(allowance <= 32);
    reify(checked.program(), &mut 100_000).unwrap();
}

#[test]
fn zero_allowance_cannot_compile_reify_or_validate_even_small_programs() {
    let ctx = context();
    let mut empty = 0;
    assert!(matches!(
        compile(&ctx, &original(), &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert!(matches!(
        reify(&program(g("A")), &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert!(matches!(
        validate(&ctx, &original(), program(g("A")), &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert_eq!(empty, 0);
}

#[test]
fn an_explicit_correct_program_can_be_independently_validated() {
    let ctx = context();
    let checked = validate(&ctx, &original(), program(g("A")), &mut 100_000)
        .unwrap_or_else(|e| panic!("exact call program: {e:?}"));
    let rebuilt = reify(checked.program(), &mut 100_000).unwrap();
    goal(&ctx, rebuilt, checked.actual_type());
}

#[test]
fn small_shared_code_cannot_hide_an_oversized_expanded_readback() {
    let mut code = Program {
        parameters: vec![],
        grounds: vec![g("A"), g("combine")],
        calls: vec![],
        result: TemplateRef::Ground(0),
    };
    for _ in 0..8 {
        let prior = code.result;
        let index = code.calls.len();
        code.calls.push(CallInstruction {
            function: TemplateRef::Ground(1),
            arguments: vec![prior, prior],
        });
        code.result = TemplateRef::Call(index);
    }
    // Eight DAG instructions expand to over one thousand Term nodes.
    // Reification must refuse before cloning that repeated tree.
    assert!(matches!(
        reify(&code, &mut 100_000),
        Err(Verdict::Decline(_))
    ));
}
