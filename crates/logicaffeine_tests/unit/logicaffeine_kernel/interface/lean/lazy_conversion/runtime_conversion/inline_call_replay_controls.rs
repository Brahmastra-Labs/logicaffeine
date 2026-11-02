//! Exact inline callback keys remain proposals: replay checks original source
//! operands and the actual beta body/capture, including foreign-handle refusal.
use super::*;
use serde_json::json;
fn check_mutation(f: impl FnOnce(&mut Search<'_, '_, '_>, usize, &mut usize)) {
    let mut i = Importer::new(false);
    i.levels.insert(0, Universe::Type(0));
    i.level_vars.insert(0, HashSet::new());
    for (id, name) in [(1, "A"), (2, "B")] {
        i.ctx.add_declaration(name, Term::Sort(Universe::Type(0)));
        i.names.insert(id, name.into());
        i.declarations.insert(name.into(), 0);
        i.exprs
            .insert(id, json!({"ie":id,"const":{"name":id,"us":[]}}));
    }
    i.exprs.insert(0, json!({"ie":0,"sort":0}));
    i.exprs.insert(3, json!({"ie":3,"bvar":0}));
    i.exprs.insert(
        4,
        json!({"ie":4,"lam":{"name":0,"type":0,"body":3,"binderInfo":"default"}}),
    );
    i.exprs.insert(
        5,
        json!({"ie":5,"lam":{"name":0,"type":0,"body":1,"binderInfo":"default"}}),
    );
    i.exprs.insert(6, json!({"ie":6,"app":{"fn":4,"arg":1}}));
    let mut budget = 100_000;
    let typed = TypedSource::check(&i, 6, &HashSet::new(), &mut budget).unwrap();
    let mut s = Search::new(&typed, &mut budget).unwrap();
    let goal = Goal::Eval(s.pool.captured(Syntax::Source(6), 0));
    let root = s.eval(goal, &mut budget).unwrap();
    s.replay(goal, root, &mut budget).unwrap();
    let at = s
        .proofs
        .iter()
        .position(|p| matches!(p.goal, Goal::UnaryCall { .. }))
        .expect("actual inline source beta");
    f(&mut s, at, &mut budget);
    assert!(
        s.replay(goal, root, &mut budget).is_err(),
        "changed callback key is not proof of the original source"
    );
}
#[test]
fn inline_source_beta_rejects_changed_argument() {
    check_mutation(|s, at, budget| {
        let b = s.pool.atom(2, budget).unwrap();
        let b = s.value(RuntimeValue::Ground(b), budget).unwrap();
        let Goal::UnaryCall { argument, .. } = &mut s.proofs[at].goal else {
            unreachable!()
        };
        *argument = b;
    });
}
#[test]
fn inline_source_beta_rejects_another_same_result_function() {
    check_mutation(|s, at, budget| {
        let c = s.pool.captured(Syntax::Source(5), 0);
        let f = s.value(RuntimeValue::Lambda(c), budget).unwrap();
        let Goal::UnaryCall { function, .. } = &mut s.proofs[at].goal else {
            unreachable!()
        };
        *function = f;
    });
}
#[test]
fn inline_source_beta_rejects_foreign_argument_without_indexing_it() {
    check_mutation(|s, at, _| {
        let Goal::UnaryCall { argument, .. } = &mut s.proofs[at].goal else {
            unreachable!()
        };
        *argument = usize::MAX;
    });
}
#[test]
fn inline_source_callback_still_fuses_its_checked_template_body() {
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
    let mut i = Importer::new(false);
    i.levels.insert(0, Universe::Type(0));
    i.level_vars.insert(0, HashSet::new());
    i.ctx.add_declaration("A", ty());
    i.ctx
        .declare(
            "apply_actual",
            pi("f", pi("x", ty(), ty()), pi("x", ty(), ty())),
            lam(
                "f",
                pi("x", ty(), ty()),
                lam(
                    "x",
                    ty(),
                    Term::App(
                        Box::new(Term::Var("f".into())),
                        Box::new(Term::Var("x".into())),
                    ),
                ),
            ),
        )
        .unwrap();
    for (id, name) in [(1, "A"), (2, "apply_actual")] {
        i.names.insert(id, name.into());
        i.declarations.insert(name.into(), 0);
        i.exprs
            .insert(id, json!({"ie":id,"const":{"name":id,"us":[]}}));
    }
    i.exprs.insert(0, json!({"ie":0,"sort":0}));
    i.exprs.insert(3, json!({"ie":3,"bvar":1}));
    i.exprs.insert(
        4,
        json!({"ie":4,"lam":{"name":0,"type":0,"body":3,"binderInfo":"default"}}),
    );
    i.exprs.insert(5, json!({"ie":5,"app":{"fn":2,"arg":4}}));
    i.exprs.insert(6, json!({"ie":6,"app":{"fn":5,"arg":1}}));
    i.exprs.insert(
        7,
        json!({"ie":7,"lam":{"name":0,"type":0,"body":6,"binderInfo":"default"}}),
    );
    i.exprs.insert(8, json!({"ie":8,"app":{"fn":7,"arg":1}}));
    let mut budget = 100_000;
    let typed = TypedSource::check(&i, 8, &HashSet::new(), &mut budget).unwrap();
    let mut s = Search::new(&typed, &mut budget).unwrap();
    let goal = Goal::Eval(s.pool.captured(Syntax::Source(8), 0));
    let root = s.eval(goal, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = s.replay(goal, root, &mut budget).unwrap() else {
        panic!("closed result");
    };
    assert_eq!(s.finish(id, &mut budget).unwrap(), Term::Global("A".into()));
    let outer = |p: &Proof| match p.goal {
        Goal::UnaryCall { function, .. } => matches!(
            s.values[function],
            RuntimeValue::Lambda(Captured {
                node: Syntax::Source(7),
                ..
            })
        ),
        _ => false,
    };
    assert_eq!(
        s.proofs
            .iter()
            .filter(|p| outer(p)
                && matches!(
                    p.rule,
                    Rule::Template {
                        origin: TemplateOrigin::SourceBeta { .. },
                        ..
                    }
                ))
            .count(),
        1,
        "the actual inline outer callback must have a compositional template proof"
    );
    assert_eq!(
        s.proofs
            .iter()
            .filter(|p| outer(p) && matches!(p.rule, Rule::Beta { .. }))
            .count(),
        0,
        "inline call representation must retain the earlier beta/body-template fusion"
    );
}
