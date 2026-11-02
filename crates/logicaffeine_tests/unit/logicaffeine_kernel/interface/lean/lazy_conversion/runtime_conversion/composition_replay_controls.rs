//! Direct adversarial controls for the new composite proof fields. These use
//! the candidate API and should be wired after the baseline-compatible RED.
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
    let answer = search.replay(goal, proof, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = answer else {
        panic!("closed result");
    };
    assert_eq!(search.finish(id, &mut budget).unwrap(), *expected);
    f(&mut search, goal, proof, &mut budget);
}
fn computed() -> Importer<'static> {
    let mut i = fixture();
    sl(&mut i, 7, 5);
    sa(&mut i, 8, 7, 1);
    sa(&mut i, 9, 7, 1);
    sa(&mut i, 10, 3, 8);
    sa(&mut i, 11, 10, 9);
    i
}
#[test]
fn computed_operand_evidence_cannot_point_forward_to_its_own_conclusion() {
    let i = computed();
    with_search(&i, 11, &g("A"), |search, goal, root, budget| {
        let Rule::SourceGround { arguments, .. } = &mut search.proofs[root].rule else {
            panic!("computed source ground call must be fused");
        };
        let OperandEvidence::Proof { proof, .. } = &mut arguments[0] else {
            panic!("non-immediate argument needs an earlier proof");
        };
        *proof = root;
        assert!(
            search.replay(goal, root, budget).is_err(),
            "same-valued cyclic evidence is not a source conversion"
        );
    });
}
#[test]
fn equal_endpoint_proofs_cannot_replace_a_different_original_source_operand() {
    let i = computed();
    with_search(&i, 11, &g("A"), |search, goal, root, budget| {
        let Rule::SourceGround { arguments, .. } = &mut search.proofs[root].rule else {
            panic!("computed source ground call must be fused");
        };
        let OperandEvidence::Proof {
            proof: a,
            value: av,
        } = arguments[0]
        else {
            panic!("first computed operand");
        };
        let OperandEvidence::Proof {
            proof: b,
            value: bv,
        } = arguments[1]
        else {
            panic!("second computed operand");
        };
        assert_ne!(a, b);
        assert_eq!(av, bv, "this forgery must preserve every dynamic endpoint");
        arguments[0] = OperandEvidence::Proof {
            proof: b,
            value: bv,
        };
        assert!(
            search.replay(goal, root, budget).is_err(),
            "source IDs and environments remain proof obligations even when results agree"
        );
    });
}
#[test]
fn fused_beta_template_cannot_replace_its_capture_with_another_valid_environment() {
    let mut i = fixture();
    sl(&mut i, 7, 6);
    sa(&mut i, 8, 4, 7);
    sa(&mut i, 9, 8, 1);
    sl(&mut i, 10, 9);
    sa(&mut i, 11, 10, 2);
    with_search(&i, 11, &g("B"), |search, goal, root, budget| {
        let at = search
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
            .expect("the source beta/body-template edge must really be fused");
        let native = search.pool.atom(1, budget).unwrap();
        let a = search.value(RuntimeValue::Ground(native), budget).unwrap();
        let different = search.envs.extend(0, a, None, budget).unwrap();
        let Rule::Template {
            origin: TemplateOrigin::SourceBeta { source, .. },
            ..
        } = &mut search.proofs[at].rule
        else {
            unreachable!()
        };
        assert_ne!(source.env, different);
        source.env = different;
        assert!(
            search.replay(goal, root, budget).is_err(),
            "a well-formed environment must still contain the actual beta argument"
        );
    });
}
