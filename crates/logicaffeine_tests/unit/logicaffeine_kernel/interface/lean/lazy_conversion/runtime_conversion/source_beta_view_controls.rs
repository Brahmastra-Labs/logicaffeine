//! A fused source-beta template already retains exact operand evidence. Its
//! native argument slots can borrow that evidence without a second Call record.
use super::*;
use serde_json::json;
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
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn fixture() -> Importer<'static> {
    let mut i = Importer::new(false);
    i.levels.insert(0, Universe::Type(0));
    i.level_vars.insert(0, HashSet::new());
    i.ctx.add_declaration("A", ty());
    i.ctx.add_declaration("B", ty());
    let ft = pi("x", ty(), ty());
    i.ctx
        .declare(
            "choose_actual",
            pi("f", ft.clone(), pi("g", ft.clone(), ty())),
            lam(
                "f",
                ft.clone(),
                lam(
                    "g",
                    ft.clone(),
                    Term::App(Box::new(v("f")), Box::new(g("A"))),
                ),
            ),
        )
        .unwrap();
    i.ctx
        .declare(
            "pass_actual",
            pi("f", ft.clone(), ft.clone()),
            lam("f", ft, v("f")),
        )
        .unwrap();
    for (id, name) in [(1, "A"), (2, "B"), (3, "choose_actual"), (4, "pass_actual")] {
        i.names.insert(id, name.into());
        i.declarations.insert(name.into(), 0);
        i.exprs
            .insert(id, json!({"ie":id,"const":{"name":id,"us":[]}}));
    }
    i.exprs.insert(0, json!({"ie":0,"sort":0}));
    i.exprs.insert(5, json!({"ie":5,"bvar":0}));
    i.exprs.insert(6, json!({"ie":6,"bvar":1}));
    for (id, body) in [(7, 6), (8, 2), (11, 10), (22, 21)] {
        i.exprs.insert(
            id,
            json!({"ie":id,"lam":{"name":0,"type":0,"body":body,"binderInfo":"default"}}),
        );
    }
    for (id, f, a) in [
        (9, 3, 7),
        (10, 9, 8),
        (12, 11, 1),
        (20, 4, 7),
        (21, 20, 2),
        (23, 22, 1),
    ] {
        i.exprs.insert(id, json!({"ie":id,"app":{"fn":f,"arg":a}}));
    }
    i
}
fn check(root: usize, f: impl FnOnce(&mut Search<'_, '_, '_>, Goal, ProofId, &mut usize)) {
    let i = fixture();
    let mut budget = 100_000;
    let typed = TypedSource::check(&i, root, &HashSet::new(), &mut budget).unwrap();
    let mut s = Search::new(&typed, &mut budget).unwrap();
    let goal = Goal::Eval(s.pool.captured(Syntax::Source(root), 0));
    let proof = s.eval(goal, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = s.replay(goal, proof, &mut budget).unwrap() else {
        panic!("closed output");
    };
    assert_eq!(s.finish(id, &mut budget).unwrap(), g("A"));
    f(&mut s, goal, proof, &mut budget);
}
#[test]
fn source_beta_template_borrows_ordered_operands_without_an_inner_call_record() {
    check(12, |s, _, _, budget| {
        assert!(
            s.proofs.iter().any(|p| matches!(
                p.rule,
                Rule::Template {
                    origin: TemplateOrigin::SourceBeta { .. },
                    ..
                }
            )),
            "the exact callback must really use source-beta template fusion"
        );
        let head = s.pool.atom(3, budget).unwrap();
        assert_eq!(s.calls.iter().filter(|c|s.values[c.function]==RuntimeValue::Ground(head)).count(),0,
            "native template slots should borrow existing source operands instead of retaining an inner Call");
    });
}
#[test]
fn evidence_backed_template_preserves_oversaturation_and_captured_result() {
    check(23, |s, _, _, budget| {
        assert!(s.proofs.iter().any(|p|matches!(p.rule,Rule::Template{origin:TemplateOrigin::SourceBeta{..},tail:Some(_),..})),
            "the generic identity template must pass its captured function to an actual trailing callback");
        let head = s.pool.atom(4, budget).unwrap();
        assert_eq!(
            s.calls
                .iter()
                .filter(|c| s.values[c.function] == RuntimeValue::Ground(head))
                .count(),
            0
        );
    });
}
#[test]
fn reordered_borrowed_source_beta_evidence_is_not_conversion_authority() {
    check(12, |s, goal, root, budget| {
        let at = s
            .proofs
            .iter()
            .position(|p| {
                matches!(
                    p.rule,
                    Rule::Template {
                        origin: TemplateOrigin::SourceBeta { .. },
                        ..
                    }
                )
            })
            .unwrap();
        let Rule::Template {
            origin: TemplateOrigin::SourceBeta { arguments, .. },
            ..
        } = &mut s.proofs[at].rule
        else {
            unreachable!()
        };
        assert_eq!(arguments.len(), 2);
        assert_ne!(arguments[0].value(), arguments[1].value());
        arguments.swap(0, 1);
        assert!(s.replay(goal,root,budget).is_err(),
            "self-consistent evidence-backed slots must still match the exact original source/capture order");
    });
}
