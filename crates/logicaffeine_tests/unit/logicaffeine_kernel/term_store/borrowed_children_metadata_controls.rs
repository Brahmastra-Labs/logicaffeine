use super::*;
use std::collections::BTreeSet;

#[test]
fn wide_mutual_binders_preserve_annotation_and_value_scopes_and_all_metadata() {
    fn var(n: &str) -> Term {
        Term::Var(n.into())
    }
    fn app(f: Term, x: Term) -> Term {
        Term::App(Box::new(f), Box::new(x))
    }
    let mut definitions = Vec::new();
    let mut expected_free = BTreeSet::new();
    let mut expected_binders = BTreeSet::new();
    let mut expected_constants = BTreeSet::new();
    for i in 0..64 {
        let function = format!("f{i}");
        let next = format!("f{}", (i + 1) % 64);
        let local = format!("x{i}");
        let index = format!("p{i}");
        let external = format!("external{i}");
        let constant = format!("TypeMarker{i}");
        expected_free.extend([local.clone(), index.clone(), external.clone()]);
        expected_binders.extend([function.clone(), local.clone(), index.clone()]);
        expected_constants.insert(constant.clone());
        // Each binder's own annotation remains outside its scope; the outer lambda
        // binds the let value, and all sibling function names are mutually bound.
        let body = Term::Lambda {
            param: local.clone(),
            param_type: Box::new(var(&local)),
            body: Box::new(Term::Let {
                name: local.clone(),
                ty: Box::new(Term::Global(constant)),
                value: Box::new(var(&local)),
                body: Box::new(Term::Pi {
                    param: index.clone(),
                    param_type: Box::new(var(&index)),
                    body_type: Box::new(app(
                        var(&next),
                        app(var(&local), app(var(&index), var(&external))),
                    )),
                }),
            }),
        };
        definitions.push((function, body));
    }
    let term = Term::MutualFix {
        defs: definitions,
        index: 63,
    };
    let mut store = TermStore::new();
    let root = store.intern(&term);
    let names = |values: Vec<&str>| {
        values
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(names(store.free_vars(root)), expected_free);
    assert_eq!(names(store.binder_names(root)), expected_binders);
    assert_eq!(names(store.constants(root)), expected_constants);
    let expected_shapes = [
        NodeShape::MutualFix,
        NodeShape::Lambda,
        NodeShape::Let,
        NodeShape::Pi,
        NodeShape::Global,
        NodeShape::Var,
        NodeShape::App,
    ]
    .into_iter()
    .fold(ShapeMask::NONE, |mask, shape| mask.with(shape));
    assert_eq!(store.shapes(root), expected_shapes);
    assert_eq!(store.depth(root) as usize, term_depth(&term) + 1);
    // Pre-intern unrelated symbols in the reverse order: metadata identity must
    // depend on the names and their scopes, not incidental dense symbol indices.
    let mut other = TermStore::new();
    for i in (0..64).rev() {
        other.mk_var(&format!("external{i}"));
    }
    let same = other.intern(&term);
    assert_eq!(other.structural_hash(same), store.structural_hash(root));
    assert_eq!(names(other.free_vars(same)), expected_free);
}
