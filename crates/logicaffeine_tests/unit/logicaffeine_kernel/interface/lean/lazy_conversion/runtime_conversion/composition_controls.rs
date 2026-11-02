//! Composition controls for source applications with computed operands and
//! source-lambda calls whose bodies enter an independently checked native template.
use super::*;
use serde_json::json;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
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
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn fixture() -> Importer<'static> {
    let mut i = Importer::new(false);
    i.levels.insert(0, Universe::Type(0));
    i.level_vars.insert(0, HashSet::new());
    i.ctx.add_declaration("A", ty());
    i.ctx.add_declaration("B", ty());
    i.ctx
        .declare(
            "first",
            pi("a", ty(), pi("b", ty(), ty())),
            lam("a", ty(), lam("b", ty(), v("a"))),
        )
        .unwrap();
    i.ctx
        .declare(
            "apply_actual",
            pi("f", pi("x", ty(), ty()), pi("x", ty(), ty())),
            lam(
                "f",
                pi("x", ty(), ty()),
                lam("x", ty(), app(v("f"), v("x"))),
            ),
        )
        .unwrap();
    for (id, n) in [(1, "A"), (2, "B"), (3, "first"), (4, "apply_actual")] {
        i.names.insert(id, n.into());
        i.declarations.insert(n.into(), 0);
        i.exprs
            .insert(id, json!({"ie":id,"const":{"name":id,"us":[]}}));
    }
    i.exprs.insert(0, json!({"ie":0,"sort":0}));
    i.exprs.insert(5, json!({"ie":5,"bvar":0}));
    i.exprs.insert(6, json!({"ie":6,"bvar":1}));
    i
}
fn sl(i: &mut Importer, id: usize, body: usize) {
    i.exprs.insert(
        id,
        json!({"ie":id,"lam":{"name":0,"type":0,"body":body,"binderInfo":"default"}}),
    );
}
fn sa(i: &mut Importer, id: usize, f: usize, a: usize) {
    i.exprs.insert(id, json!({"ie":id,"app":{"fn":f,"arg":a}}));
}
fn with_search(
    i: &Importer,
    root: usize,
    expected: &Term,
    f: impl FnOnce(&mut Search<'_, '_, '_>, Goal, ProofId, &mut usize),
) {
    let mut budget = 100_000;
    let typed = TypedSource::check(i, root, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let goal = Goal::Eval(search.pool.captured(Syntax::Source(root), 0));
    let proof = search.eval(goal, &mut budget).unwrap();
    let value = search.replay(goal, proof, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = value else {
        panic!("expected closed output");
    };
    assert_eq!(search.finish(id, &mut budget).unwrap(), *expected);
    f(&mut search, goal, proof, &mut budget);
}
fn compound_operands() -> Importer<'static> {
    let mut i = fixture();
    sl(&mut i, 7, 5);
    sa(&mut i, 8, 7, 1);
    sa(&mut i, 9, 7, 2);
    sa(&mut i, 10, 3, 8);
    sa(&mut i, 11, 10, 9);
    i
}
#[test]
fn computed_source_operands_do_not_require_a_second_spine_certificate() {
    let i = compound_operands();
    with_search(&i, 11, &g("A"), |search, _, _, _| {
        assert_eq!(
            search
                .proofs
                .iter()
                .filter(|p| matches!(p.rule, Rule::Spine { .. }))
                .count(),
            0,
            "source applications should cite exact earlier operand proofs directly"
        );
    });
}
#[test]
fn source_lambda_call_and_native_template_body_share_one_certificate() {
    let mut i = fixture();
    sl(&mut i, 7, 6);
    sa(&mut i, 8, 4, 7);
    sa(&mut i, 9, 8, 2);
    sl(&mut i, 10, 9);
    sa(&mut i, 11, 10, 1);
    with_search(&i, 11, &g("A"), |search, _, _, _| {
        let redundant = search
            .proofs
            .iter()
            .filter(|p| {
                if !matches!(p.rule, Rule::Beta { .. }) {
                    return false;
                }
                let Goal::Call(call) = p.goal else {
                    return false;
                };
                matches!(
                    search.values[search.calls[call].function],
                    RuntimeValue::Lambda(Captured {
                        node: Syntax::Source(10),
                        ..
                    })
                )
            })
            .count();
        assert_eq!(
            redundant, 0,
            "the exact beta body/template link should be one source-call certificate"
        );
    });
}
#[test]
fn composed_calls_distinguish_the_same_lambda_under_different_captures() {
    let mut i = fixture();
    sl(&mut i, 7, 6);
    sa(&mut i, 8, 4, 7);
    sa(&mut i, 9, 8, 2);
    sl(&mut i, 10, 9);
    sa(&mut i, 11, 10, 1);
    sa(&mut i, 12, 10, 2);
    sa(&mut i, 13, 3, 11);
    sa(&mut i, 14, 13, 12);
    with_search(&i, 11, &g("A"), |_, _, _, _| {});
    with_search(&i, 12, &g("B"), |_, _, _, _| {});
    with_search(&i, 14, &g("A"), |_, _, _, _| {});
}
#[test]
fn coherent_native_reordering_still_cannot_change_original_source_arguments() {
    let i = compound_operands();
    with_search(&i, 11, &g("A"), |search, goal, root, budget| {
        let first = search.pool.atom(3, budget).unwrap();
        let call = search
            .calls
            .iter()
            .position(|c| {
                search.values[c.function] == RuntimeValue::Ground(first) && c.arguments.len() == 2
            })
            .unwrap();
        let original = search.calls[call].arguments.clone();
        let RuntimeValue::Ground(a) = search.values[original[0]] else {
            panic!("first argument ground");
        };
        let RuntimeValue::Ground(b) = search.values[original[1]] else {
            panic!("second argument ground");
        };
        let old_fun = search.pool.app(first, a, budget).unwrap();
        let old = search.pool.app(old_fun, b, budget).unwrap();
        let new_fun = search.pool.app(first, b, budget).unwrap();
        let new = search.pool.app(new_fun, a, budget).unwrap();
        let link = search.pool.ground(new, true, budget).unwrap();
        let result = search.pool.links[link].to;
        assert_eq!(search.pool.copy(result, budget).unwrap(), g("B"));
        search.calls[call].arguments.swap(0, 1);
        for link in &mut search.pool.links {
            if link.from == old {
                link.from = new;
                link.to = result;
            }
        }
        for p in &mut search.proofs {
            if p.goal == Goal::Call(call) || p.goal == goal {
                p.value = original[1];
            }
        }
        assert!(
            search.replay(goal, root, budget).is_err(),
            "a valid native first(B,A)=B link does not certify original first(id(A),id(B))"
        );
    });
}
