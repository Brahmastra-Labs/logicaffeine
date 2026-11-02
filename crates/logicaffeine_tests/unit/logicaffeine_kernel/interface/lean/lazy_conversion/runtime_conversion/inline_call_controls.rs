//! Source callback calls retain their exact function/argument key inline instead
//! of duplicating that fixed-size key in an owned argument vector and Call table.
use super::*;
use serde_json::json;
fn fixture() -> Importer<'static> {
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
    i.exprs.insert(4, json!({"ie":4,"bvar":1}));
    for (id, body) in [(5, 3), (6, 1), (7, 4)] {
        i.exprs.insert(
            id,
            json!({"ie":id,"lam":{"name":0,"type":0,"body":body,"binderInfo":"default"}}),
        );
    }
    i.exprs.insert(8, json!({"ie":8,"app":{"fn":5,"arg":1}}));
    i
}
fn with_search(
    i: &Importer,
    root: usize,
    expected: &str,
    f: impl FnOnce(&mut Search<'_, '_, '_>, Goal, ProofId, &mut usize),
) {
    let mut budget = 100_000;
    let typed = TypedSource::check(i, root, &HashSet::new(), &mut budget).unwrap();
    let mut s = Search::new(&typed, &mut budget).unwrap();
    let goal = Goal::Eval(s.pool.captured(Syntax::Source(root), 0));
    let proof = s.eval(goal, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = s.replay(goal, proof, &mut budget).unwrap() else {
        panic!("closed result");
    };
    assert_eq!(
        s.finish(id, &mut budget).unwrap(),
        Term::Global(expected.into())
    );
    f(&mut s, goal, proof, &mut budget);
}
#[test]
fn source_unary_call_does_not_retain_an_argument_vector_and_call_record() {
    with_search(&fixture(), 8, "A", |s, _, _, _| {
        assert!(
            s.values.iter().any(|v| matches!(
                v,
                RuntimeValue::Lambda(Captured {
                    node: Syntax::Source(5),
                    ..
                })
            )),
            "the source lambda must actually participate in evaluation"
        );
        assert_eq!(
            s.calls
                .iter()
                .filter(|call| call.arguments.len() == 1
                    && matches!(
                        s.values[call.function],
                        RuntimeValue::Lambda(Captured {
                            node: Syntax::Source(_),
                            ..
                        })
                    ))
                .count(),
            0,
            "a unary source callback should use the existing fixed-size goal key directly"
        );
    });
}
#[test]
fn repeated_exact_source_callback_reuses_its_existing_proof() {
    with_search(&fixture(), 8, "A", |s, _, _, budget| {
        let f = s
            .values
            .iter()
            .position(|v| {
                matches!(
                    v,
                    RuntimeValue::Lambda(Captured {
                        node: Syntax::Source(5),
                        ..
                    })
                )
            })
            .unwrap();
        let a = s.pool.atom(1, budget).unwrap();
        let a = s.value(RuntimeValue::Ground(a), budget).unwrap();
        let before = s.proofs.len();
        let goal = s.call(f, vec![a], budget).unwrap();
        let first = s.eval(goal, budget).unwrap();
        let second = s.eval(goal, budget).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            s.proofs.len(),
            before,
            "the original source application's validated call projection should serve both lookups"
        );
    });
}
#[test]
fn source_unary_callback_preserves_an_outer_capture() {
    let mut i = fixture();
    i.exprs.insert(9, json!({"ie":9,"app":{"fn":7,"arg":2}}));
    i.exprs.insert(
        10,
        json!({"ie":10,"lam":{"name":0,"type":0,"body":9,"binderInfo":"default"}}),
    );
    i.exprs.insert(11, json!({"ie":11,"app":{"fn":10,"arg":1}}));
    with_search(&i, 11, "A", |s, _, _, _| {
        assert!(s.values.iter().any(
            |v| matches!(v,RuntimeValue::Lambda(Captured{node:Syntax::Source(7),env}) if *env!=0)
        ));
        assert_eq!(
            s.calls
                .iter()
                .filter(|call| call.arguments.len() == 1
                    && matches!(
                        s.values[call.function],
                        RuntimeValue::Lambda(Captured {
                            node: Syntax::Source(_),
                            ..
                        })
                    ))
                .count(),
            0
        );
    });
}
