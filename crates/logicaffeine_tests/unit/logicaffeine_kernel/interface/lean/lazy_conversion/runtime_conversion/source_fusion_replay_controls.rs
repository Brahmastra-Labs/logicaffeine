//! Untrusted template records and instruction premises remain replay checked.
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
fn wrapper(operand: &str) -> Term {
    lam("f", pi("x", ty(), ty()), app(v("f"), g(operand)))
}
fn with_search(check: impl FnOnce(&mut Search<'_, '_, '_>, Goal, ProofId, &mut usize)) {
    let mut i = Importer::new(false);
    i.ctx.add_declaration("A", ty());
    i.ctx.add_declaration("B", ty());
    i.ctx
        .declare("use_A", pi("f", pi("x", ty(), ty()), ty()), wrapper("A"))
        .unwrap();
    for (id, n) in [(1, "A"), (2, "B"), (3, "use_A")] {
        i.names.insert(id, n.into());
        i.declarations.insert(n.into(), 0);
    }
    i.levels.insert(0, Universe::Type(0));
    i.level_vars.insert(0, HashSet::new());
    for (id, row) in [
        (0, json!({"ie":0,"sort":0})),
        (1, json!({"ie":1,"const":{"name":1,"us":[]}})),
        (2, json!({"ie":2,"const":{"name":2,"us":[]}})),
        (3, json!({"ie":3,"const":{"name":3,"us":[]}})),
        (4, json!({"ie":4,"bvar":0})),
        (
            5,
            json!({"ie":5,"lam":{"name":0,"type":0,"body":4,"binderInfo":"default"}}),
        ),
        (6, json!({"ie":6,"app":{"fn":3,"arg":5}})),
    ] {
        i.exprs.insert(id, row);
    }
    i.ctx
        .declare("use_B", pi("f", pi("x", ty(), ty()), ty()), wrapper("B"))
        .unwrap();
    i.names.insert(4, "use_B".into());
    i.declarations.insert("use_B".into(), 0);
    i.exprs.insert(
        7,
        json!({"ie":7,"lam":{"name":0,"type":0,"body":2,"binderInfo":"default"}}),
    );
    i.exprs
        .insert(8, json!({"ie":8,"const":{"name":4,"us":[]}}));
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&i, 6, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let goal = Goal::Eval(search.pool.captured(Syntax::Source(6), 0));
    let proof = search.eval(goal, &mut budget).unwrap();
    let answer = search.replay(goal, proof, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = answer else {
        panic!("closed template result")
    };
    assert_eq!(search.finish(id, &mut budget).unwrap(), g("A"));
    assert!(
        !search.pool.templates.is_empty(),
        "the native call must actually use a template"
    );
    check(&mut search, goal, proof, &mut budget);
    assert!(
        budget > 100_000,
        "a tiny provenance test must leave a generous allowance"
    );
}
fn source_step(search: &Search<'_, '_, '_>) -> usize {
    search
        .proofs
        .iter()
        .position(|proof| {
            matches!(
                proof.rule,
                Rule::Template {
                    origin: TemplateOrigin::Source { .. },
                    ..
                }
            )
        })
        .expect("the actual source call must use fused evidence")
}
#[test]
fn inline_lambda_evidence_cannot_change_the_original_source_operand() {
    with_search(|search, goal, proof, budget| {
        let other = search
            .value(
                RuntimeValue::Lambda(Captured {
                    node: Syntax::Source(7),
                    env: 0,
                }),
                budget,
            )
            .unwrap();
        let at = source_step(search);
        let Rule::Template {
            origin: TemplateOrigin::Source {
                call, arguments, ..
            },
            ..
        } = &mut search.proofs[at].rule
        else {
            unreachable!()
        };
        arguments[0] = OperandEvidence::Lambda(other);
        search.calls[*call].arguments[0] = other;
        assert!(
            search.replay(goal, proof, budget).is_err(),
            "a coherent replacement call still must match the original lambda child"
        );
    });
}
#[test]
fn inline_head_evidence_cannot_change_a_same_typed_checked_definition() {
    with_search(|search, goal, proof, budget| {
        let term = search.pool.atom(8, budget).unwrap();
        let other = search.value(RuntimeValue::Ground(term), budget).unwrap();
        let at = source_step(search);
        let Rule::Template {
            origin: TemplateOrigin::Source { call, head, .. },
            ..
        } = &mut search.proofs[at].rule
        else {
            unreachable!()
        };
        *head = OperandEvidence::Atom(other);
        search.calls[*call].function = other;
        assert!(
            search.replay(goal, proof, budget).is_err(),
            "actual source use_A cannot be replaced with checked same-typed use_B"
        );
    });
}
#[test]
fn a_fused_source_certificate_cannot_project_an_unrelated_call() {
    with_search(|search, goal, proof, budget| {
        let at = source_step(search);
        let old = match &search.proofs[at].rule {
            Rule::Template {
                origin: TemplateOrigin::Source { call, .. },
                ..
            } => *call,
            _ => unreachable!(),
        };
        // Fusion leaves the source's own call as the only entry of the table, so the forgery interns the
        // unrelated call itself: the checked, same-typed use_B on the same ordered operands.
        let term = search.pool.atom(8, budget).unwrap();
        let function = search.value(RuntimeValue::Ground(term), budget).unwrap();
        let arguments = search.calls[old].arguments.clone();
        let Goal::Call(different) = search.call(function, arguments, budget).unwrap() else {
            unreachable!()
        };
        assert_ne!(
            different, old,
            "the forgery projects a call the table holds beside the source's own"
        );
        let Rule::Template {
            origin: TemplateOrigin::Source { call, .. },
            ..
        } = &mut search.proofs[at].rule
        else {
            unreachable!()
        };
        *call = different;
        let Err(Verdict::Reject(reason)) = search.replay(goal, proof, budget) else {
            panic!("replay accepted a fused source certificate projecting an unrelated call");
        };
        assert!(
            reason.contains("fused source application head changed"),
            "projection must have the actual source function and ordered operands: {}",
            reason.message()
        );
    });
}
