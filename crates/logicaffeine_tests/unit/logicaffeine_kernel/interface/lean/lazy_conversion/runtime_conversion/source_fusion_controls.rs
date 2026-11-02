//! Runtime integration must reuse checked native code across distinct source closures.
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
    i.ctx.add_declaration("A", ty());
    i.ctx.add_declaration("B", ty());
    i.ctx
        .declare(
            "apply_actual",
            pi("f", pi("x", ty(), ty()), pi("y", ty(), ty())),
            lam(
                "f",
                pi("x", ty(), ty()),
                lam("y", ty(), app(v("f"), v("y"))),
            ),
        )
        .unwrap();
    for (id, n) in [(1, "A"), (2, "B"), (3, "apply_actual")] {
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
    ] {
        i.exprs.insert(id, row);
    }
    i
}
fn row(i: &mut Importer, id: usize, data: Value) {
    i.exprs.insert(id, data);
}
#[test]
fn source_template_calls_fuse_immediate_lambda_evidence() {
    let mut i = fixture();
    let mut root = 1;
    // Each closed let value calls the same native wrapper with a different
    // source lambda. The original checker must type every value, even erased ones.
    for step in 0..32 {
        let n = 10 + 4 * step;
        row(
            &mut i,
            n,
            json!({"ie":n,"lam":{"name":0,"type":0,"body":4,"binderInfo":"default"}}),
        );
        row(&mut i, n + 1, json!({"ie":n+1,"app":{"fn":3,"arg":n}}));
        row(&mut i, n + 2, json!({"ie":n+2,"app":{"fn":n+1,"arg":1}}));
        row(
            &mut i,
            n + 3,
            json!({"ie":n+3,"letE":{"name":0,"type":0,"value":n+2,"body":root,"nonDep":true}}),
        );
        root = n + 3;
    }
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&i, root, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let goal = Goal::Eval(search.pool.captured(Syntax::Source(root), 0));
    let proof = search.eval(goal, &mut budget).unwrap();
    let answer = search.replay(goal, proof, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = answer else {
        panic!("closed result")
    };
    assert_eq!(search.finish(id, &mut budget).unwrap(), g("A"));
    assert_eq!(search.proofs.iter().filter(|proof|matches!(proof.rule,Rule::Lambda)).count(),0,
  "immediate source lambda operands need exact inline evidence, not standalone evaluation records");
    assert_eq!(
        search
            .proofs
            .iter()
            .filter(|proof| matches!(proof.rule, Rule::Template { .. })
                && matches!(proof.goal, Goal::Call(_)))
            .count(),
        0,
        "the source application and its template call must share a checked composite certificate"
    );
    assert!(budget > 100_000);
}
