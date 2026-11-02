//! Same source node, same endpoint, different captured environment: only the
//! exact earlier Eval judgment can discharge an operand conversion premise.
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
fn sa(i: &mut Importer, id: usize, f: usize, a: usize) {
    i.exprs.insert(id, json!({"ie":id,"app":{"fn":f,"arg":a}}));
}
fn sl(i: &mut Importer, id: usize, body: usize) {
    i.exprs.insert(
        id,
        json!({"ie":id,"lam":{"name":0,"type":0,"body":body,"binderInfo":"default"}}),
    );
}
#[test]
fn identical_node_and_endpoint_do_not_erase_operand_capture_identity() {
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
    for (id, name) in [(1, "A"), (2, "B"), (3, "first")] {
        i.names.insert(id, name.into());
        i.declarations.insert(name.into(), 0);
        i.exprs
            .insert(id, json!({"ie":id,"const":{"name":id,"us":[]}}));
    }
    i.exprs.insert(0, json!({"ie":0,"sort":0}));
    i.exprs.insert(5, json!({"ie":5,"bvar":0}));
    // f x = first ((fun y => y) A) x. Node 8 is reused verbatim
    // under f(A)'s and f(B)'s distinct captured environments.
    sl(&mut i, 7, 5);
    sa(&mut i, 8, 7, 1);
    sa(&mut i, 9, 3, 8);
    sa(&mut i, 10, 9, 5);
    sl(&mut i, 11, 10);
    sa(&mut i, 12, 11, 1);
    sa(&mut i, 13, 11, 2);
    sa(&mut i, 14, 3, 12);
    sa(&mut i, 15, 14, 13);
    let mut budget = 100_000;
    let typed = TypedSource::check(&i, 15, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let goal = Goal::Eval(search.pool.captured(Syntax::Source(15), 0));
    let root = search.eval(goal, &mut budget).unwrap();
    let answer = search.replay(goal, root, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = answer else {
        panic!("expected a closed result");
    };
    assert_eq!(search.finish(id, &mut budget).unwrap(), g("A"));

    let mut evidence = Vec::new();
    for (at, p) in search.proofs.iter().enumerate() {
        if let Rule::SourceGround { arguments, .. } = &p.rule {
            for (slot, operand) in arguments.iter().enumerate() {
                if let OperandEvidence::Proof { proof, value } = *operand {
                    if let Goal::Eval(capture) = search.proofs[proof].goal {
                        if capture.node == Syntax::Source(8) {
                            evidence.push((at, slot, proof, value, capture));
                        }
                    }
                }
            }
        }
    }
    let mut selected = None;
    for &(at, slot, original, value, capture) in &evidence {
        for &(_, _, replacement, other_value, other_capture) in &evidence {
            if replacement < at && capture.env != other_capture.env && value == other_value {
                selected = Some((
                    at,
                    slot,
                    original,
                    replacement,
                    value,
                    capture,
                    other_capture,
                ));
                break;
            }
        }
        if selected.is_some() {
            break;
        }
    }
    let (at, slot, original, replacement, value, capture, other_capture) = selected.expect(
        "fixture must produce an earlier same-node, same-valued proof under another environment",
    );
    assert!(
        original < at && replacement < at,
        "both premises must be backward, not cyclic"
    );
    assert_ne!(original, replacement);
    assert_eq!(capture.node, other_capture.node);
    assert_ne!(capture.env, other_capture.env);
    assert_eq!(
        search.proofs[original].value,
        search.proofs[replacement].value
    );
    let Rule::SourceGround { arguments, .. } = &mut search.proofs[at].rule else {
        unreachable!()
    };
    // Change only the referenced proof. The source, environment, claimed value,
    // native link and ordered call endpoints all remain untouched.
    arguments[slot] = OperandEvidence::Proof {
        proof: replacement,
        value,
    };
    assert!(
        search.replay(goal, root, &mut budget).is_err(),
        "same source node and same result cannot substitute another capture's Eval proof"
    );
}
