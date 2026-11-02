//! A source application's result is the result of ITS call: replay projects the call the
//! premise proves and compares both its function and its ordered arguments with the original.
use super::*;
use serde_json::json;

/// `(λp:Type. (λx:Type. x) A) ((λy:Type. y) A)`. Evaluation proves `(λy. y) A` before the body
/// of `λp` applies `λx`, so an earlier proof of a DIFFERENT call with the same argument and the
/// same value exists when the inner application is concluded.
fn fixture() -> Importer<'static> {
    let mut importer = Importer::new(false);
    importer
        .ctx
        .add_declaration("A", Term::Sort(Universe::Type(0)));
    importer.names.insert(1, "A".into());
    importer.declarations.insert("A".into(), 0);
    importer.levels.insert(0, Universe::Type(0));
    importer.level_vars.insert(0, HashSet::new());
    for (id, row) in [
        (0, json!({"ie":0,"sort":0})),
        (1, json!({"ie":1,"const":{"name":1,"us":[]}})),
        (3, json!({"ie":3,"bvar":0})),
        (
            4,
            json!({"ie":4,"lam":{"name":0,"type":0,"body":3,"binderInfo":"default"}}),
        ),
        (5, json!({"ie":5,"app":{"fn":4,"arg":1}})),
        (
            6,
            json!({"ie":6,"lam":{"name":0,"type":0,"body":5,"binderInfo":"default"}}),
        ),
        (
            7,
            json!({"ie":7,"lam":{"name":0,"type":0,"body":3,"binderInfo":"default"}}),
        ),
        (8, json!({"ie":8,"app":{"fn":7,"arg":1}})),
        (9, json!({"ie":9,"app":{"fn":6,"arg":8}})),
    ] {
        importer.exprs.insert(id, row);
    }
    importer
}

#[test]
fn a_source_application_cannot_borrow_the_result_of_an_unrelated_call() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 9, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let root = Goal::Eval(search.pool.captured(Syntax::Source(9), 0));
    let proof = search.eval(root, &mut budget).unwrap();
    let answer = search.replay(root, proof, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = answer else {
        panic!("the application reduces to the atom A")
    };
    assert_eq!(
        search.finish(id, &mut budget).unwrap(),
        Term::Global("A".into())
    );

    let lambda_of = |search: &Search<'_, '_, '_>, goal: Goal| match goal {
        Goal::UnaryCall { function, .. } => match search.values[function] {
            RuntimeValue::Lambda(Captured {
                node: Syntax::Source(node),
                ..
            }) => Some(node),
            _ => None,
        },
        _ => None,
    };
    // `(λx. x) A` under `λp`: a SourceApply concluding the call of `λp`, whose premise proves the
    // call of `λx`.
    let at = search
        .proofs
        .iter()
        .position(|p| {
            matches!(p.rule, Rule::SourceApply { .. }) && lambda_of(&search, p.goal) == Some(6)
        })
        .expect("the body of λp concludes through its source application");
    let Rule::SourceApply { premise, .. } = search.proofs[at].rule else {
        unreachable!()
    };
    assert_eq!(
        lambda_of(&search, search.proofs[premise].goal),
        Some(4),
        "its premise is the call of λx"
    );
    // The unrelated call `(λy. y) A`: the same argument, the same value, another function.
    let unrelated = search.proofs[..at]
        .iter()
        .position(|p| lambda_of(&search, p.goal) == Some(7) && matches!(p.rule, Rule::Beta { .. }))
        .expect("(λy. y) A was proved first");
    assert_eq!(
        search.proofs[unrelated].value, search.proofs[premise].value,
        "the forgery keeps the value"
    );
    let Rule::SourceApply { premise, .. } = &mut search.proofs[at].rule else {
        unreachable!()
    };
    *premise = unrelated;
    let Err(Verdict::Reject(reason)) = search.replay(root, proof, &mut budget) else {
        panic!("replay accepted a source application proved by an unrelated call");
    };
    assert!(
        reason
            .message()
            .contains("runtime call value/argument mismatch"),
        "the premise's call must have the application's own function: {}",
        reason.message()
    );
}
