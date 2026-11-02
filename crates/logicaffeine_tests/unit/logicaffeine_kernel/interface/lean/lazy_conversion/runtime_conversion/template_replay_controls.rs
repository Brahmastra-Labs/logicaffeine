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
fn template_step(search: &Search<'_, '_, '_>) -> usize {
    search
        .proofs
        .iter()
        .position(|p| matches!(p.rule, Rule::Template { .. }))
        .unwrap()
}
#[test]
fn replay_reconstructs_template_ground_slots() {
    with_search(|search, goal, proof, budget| {
        let other = search.pool.atom(2, budget).unwrap();
        let slot = search
            .pool
            .templates
            .iter()
            .position(|r| !r.grounds.is_empty())
            .unwrap();
        search.pool.templates[slot].grounds[0] = other;
        assert!(
            search.replay(goal, proof, budget).is_err(),
            "cached ground IDs cannot replace original template operands"
        );
    });
}
#[test]
fn a_checked_same_typed_template_does_not_authorize_another_body() {
    with_search(|search, goal, proof, budget| {
        let replacement =
            runtime_template::compile(&search.pool.source.importer.ctx, &wrapper("B"), budget)
                .unwrap()
                .unwrap();
        search.pool.templates[0].checked = replacement;
        assert!(
            search.replay(goal, proof, budget).is_err(),
            "valid code for f(B) cannot certify actual f(A)"
        );
    });
}
#[test]
fn template_instruction_premises_must_precede_their_use() {
    with_search(|search, goal, proof, budget| {
        let at = template_step(search);
        let Rule::Template { instructions, .. } = &mut search.proofs[at].rule else {
            unreachable!()
        };
        assert!(!instructions.is_empty());
        instructions[0] = at;
        assert!(
            search.replay(goal, proof, budget).is_err(),
            "self-referential instruction proof must reject"
        );
    });
}
#[test]
fn template_inventory_indices_cannot_panic_replay() {
    with_search(|search, goal, proof, budget| {
        let at = template_step(search);
        let Rule::Template { template, .. } = &mut search.proofs[at].rule else {
            unreachable!()
        };
        *template = usize::MAX;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            search.replay(goal, proof, budget)
        }));
        assert!(outcome.is_ok(), "untrusted template index must not panic");
        assert!(
            outcome.unwrap().is_err(),
            "foreign template index must not be accepted"
        );
    });
}
