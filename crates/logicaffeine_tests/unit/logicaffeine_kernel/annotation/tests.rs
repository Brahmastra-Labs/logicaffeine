use super::*;
use crate::term::{Literal, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn at(file: &str, line: u32, column: u32, t: Term) -> Term {
    Term::annotated(
        Annotation::Source(Provenance::Position(SourcePosition::new(
            file, line, column,
        ))),
        t,
    )
}
fn kind(k: ParamKind, t: Term) -> Term {
    Term::annotated(Annotation::Binder(k), t)
}

/// One term that exercises every variant, so a walk that forgets one is caught by a count.
fn every_variant() -> Term {
    let ty = Term::Sort(Universe::Type(0));
    let body = Term::Match {
        discriminant: Box::new(v("n")),
        motive: Box::new(lam("_", g("Nat"), g("Nat"))),
        cases: vec![
            Term::Lit(Literal::Int(1)),
            Term::Fix {
                name: "f".into(),
                body: Box::new(lam("m", g("Nat"), app(v("f"), v("m")))),
            },
        ],
    };
    let mutual = Term::MutualFix {
        defs: vec![
            ("a".into(), lam("x", g("Nat"), v("x"))),
            ("b".into(), Term::Hole),
        ],
        index: 0,
    };
    let let_in = Term::Let {
        name: "k".into(),
        ty: Box::new(g("Nat")),
        value: Box::new(Term::Const {
            name: "c".into(),
            levels: vec![Universe::Prop],
        }),
        body: Box::new(app(mutual, v("k"))),
    };
    pi("A", ty, lam("n", g("Nat"), app(body, let_in)))
}

/// Wrap EVERY node of `t` in a position, and every binder additionally in an implicit kind.
fn annotate_everywhere(t: &Term) -> Term {
    let rebuilt = match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => t.clone(),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => kind(
            ParamKind::Implicit,
            pi(
                param,
                annotate_everywhere(param_type),
                annotate_everywhere(body_type),
            ),
        ),
        Term::Lambda {
            param,
            param_type,
            body,
        } => kind(
            ParamKind::Instance,
            lam(
                param,
                annotate_everywhere(param_type),
                annotate_everywhere(body),
            ),
        ),
        Term::App(f, a) => app(annotate_everywhere(f), annotate_everywhere(a)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(annotate_everywhere(discriminant)),
            motive: Box::new(annotate_everywhere(motive)),
            cases: cases.iter().map(annotate_everywhere).collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(annotate_everywhere(body)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), annotate_everywhere(b)))
                .collect(),
            index: *index,
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(annotate_everywhere(ty)),
            value: Box::new(annotate_everywhere(value)),
            body: Box::new(annotate_everywhere(body)),
        },
        Term::Annotated { annotation, term } => {
            Term::annotated(annotation.clone(), annotate_everywhere(term))
        }
    };
    at("every.lean", 1, 0, rebuilt)
}

fn annotation_count(t: &Term) -> usize {
    let mut work = vec![t];
    let mut n = 0;
    while let Some(t) = work.pop() {
        if matches!(t, Term::Annotated { .. }) {
            n += 1;
        }
        push_children(t, &mut work);
    }
    n
}

fn node_count(t: &Term, counted: fn(&Term) -> bool) -> usize {
    let mut work = vec![t];
    let mut n = 0;
    while let Some(t) = work.pop() {
        n += usize::from(counted(t));
        push_children(t, &mut work);
    }
    n
}

#[test]
fn equality_is_kernel_meaning_and_identical_sees_the_annotations() {
    let plain = every_variant();
    let annotated = annotate_everywhere(&plain);
    assert_eq!(
        annotated, plain,
        "`==` compares kernel meaning: annotations are invisible to it"
    );
    assert_eq!(plain, annotated, "and symmetrically");
    assert!(
        !annotated.identical(&plain),
        "`identical` sees an annotation `==` does not"
    );
    assert!(
        annotated.identical(&annotated.clone()),
        "`identical` is reflexive"
    );
    assert!(plain.identical(&plain.clone()));

    let implicit = kind(ParamKind::Implicit, lam("A", g("Type"), v("A")));
    let strict = kind(ParamKind::StrictImplicit, lam("A", g("Type"), v("A")));
    assert_eq!(
        implicit, strict,
        "two binder kinds on one binder are one kernel term"
    );
    assert!(!implicit.identical(&strict), "but not one spelling");

    let here = at("a.lean", 1, 0, g("Nat"));
    let there = at("a.lean", 2, 0, g("Nat"));
    assert!(
        !here.identical(&there),
        "a moved position is a different annotation"
    );
    assert_ne!(
        here,
        g("Int"),
        "an annotation never makes two different terms equal"
    );
    assert_ne!(
        lam("x", g("Nat"), at("f", 1, 0, v("x"))),
        lam("y", g("Nat"), at("f", 1, 0, v("y"))),
        "`==` stays syntactic in the binder name, as the derive was"
    );
}

#[test]
fn stripping_erases_every_annotation_is_idempotent_and_keeps_the_meaning() {
    let plain = every_variant();
    let annotated = annotate_everywhere(&plain);
    assert!(annotated.has_annotations());
    assert!(!plain.has_annotations());
    let nodes = node_count(&plain, |_| true);
    let binders = node_count(&plain, |t| {
        matches!(t, Term::Lambda { .. } | Term::Pi { .. })
    });
    assert_eq!(
        binders, 5,
        "the fixture's Π, its three `fun`s and the mutual body's `fun`"
    );
    assert_eq!(
        annotation_count(&annotated),
        nodes + binders,
        "the fixture annotates every node with a position and every binder with a kind as well"
    );

    let stripped = annotated.strip_annotations();
    assert!(
        !stripped.has_annotations(),
        "no annotation survives erasure"
    );
    assert!(
        stripped.identical(&plain),
        "erasure recovers the plain term exactly"
    );
    assert!(
        stripped.strip_annotations().identical(&stripped),
        "erasure is idempotent"
    );
    assert!(
        annotated.clone().into_stripped().identical(&plain),
        "the owned face agrees"
    );
    assert!(
        annotated.has_annotations(),
        "`strip_annotations` borrows: the annotated term is untouched"
    );
}

#[test]
fn a_term_with_no_annotation_is_handed_back_without_a_copy() {
    let plain = every_variant();
    let ptr = match &plain {
        Term::Pi { body_type, .. } => body_type.as_ref() as *const Term,
        _ => unreachable!(),
    };
    let owned = plain.into_stripped();
    let after = match &owned {
        Term::Pi { body_type, .. } => body_type.as_ref() as *const Term,
        _ => unreachable!(),
    };
    assert_eq!(
        ptr, after,
        "`into_stripped` moves an annotation-free term through untouched"
    );
}

#[test]
fn every_walk_is_iterative_on_a_term_deeper_than_the_native_stack() {
    const DEPTH: usize = 200_000;
    let mut deep = g("Zero");
    for i in 0..DEPTH {
        deep = if i % 2 == 0 {
            at("deep.lean", 1, 0, deep)
        } else {
            app(g("Succ"), deep)
        };
    }
    assert!(deep.has_annotations());
    let mut stripped = std::mem::replace(&mut deep, Term::Hole).into_stripped();
    assert!(!stripped.has_annotations());
    let mut rungs = 0;
    let mut cur = &stripped;
    while let Term::App(_, a) = cur {
        rungs += 1;
        cur = a;
    }
    assert_eq!(rungs, DEPTH / 2, "every application survives erasure");
    assert!(
        stripped.identical(&stripped),
        "`identical` walks the deep spine on the heap"
    );
    // Dismantle iteratively: the derived `Drop` recurses.
    let mut spine = Vec::new();
    loop {
        match std::mem::replace(&mut stripped, Term::Hole) {
            Term::App(f, a) => {
                spine.push(f);
                stripped = *a;
            }
            _ => break,
        }
    }
}

#[test]
fn the_head_annotations_are_read_outermost_first() {
    let binder = at(
        "m.lean",
        3,
        4,
        kind(
            ParamKind::Implicit,
            kind(ParamKind::Instance, lam("A", g("Type"), v("A"))),
        ),
    );
    assert_eq!(
        binder.binder_kind(),
        Some(ParamKind::Implicit),
        "the outermost kind is the binder's"
    );
    assert!(matches!(binder.peel_annotations(), Term::Lambda { .. }));
    assert_eq!(binder.head_annotations().count(), 3);
    assert_eq!(
        binder.provenance(),
        Some(&Provenance::Position(SourcePosition::new("m.lean", 3, 4)))
    );
    assert_eq!(
        kind(ParamKind::Implicit, g("Nat")).binder_kind(),
        None,
        "only a binder has a kind"
    );
    assert_eq!(
        lam("A", g("Type"), v("A")).binder_kind(),
        None,
        "an unannotated binder records none"
    );
    assert_eq!(g("Nat").provenance(), None);
}

#[test]
fn a_position_reads_back_from_its_display() {
    let position = SourcePosition::new("T00_009.lean", 1, 0);
    assert_eq!(
        position.as_str(),
        "T00_009.lean:1:0",
        "Lean's `file:line:column`"
    );
    assert_eq!(position.to_string(), position.as_str());
    assert_eq!(
        (position.file(), position.line(), position.column()),
        ("T00_009.lean", 1, 0)
    );
    assert_eq!(SourcePosition::parse("T00_009.lean:1:0"), Some(position));
    let windows =
        SourcePosition::parse("C:\\src\\A.lean:12:7").expect("a file name may contain `:`");
    assert_eq!(windows, SourcePosition::new("C:\\src\\A.lean", 12, 7));
    assert_eq!(windows.file(), "C:\\src\\A.lean");
    for refused in [
        "",
        "a.lean",
        "a.lean:1",
        ":1:0",
        "a.lean:0:0",
        "a.lean:x:0",
        "a.lean:1:-1",
        "a.lean:01:0",
        "a.lean:1:00",
        "a.lean:+1:0",
        "a.lean:1: 0",
        "a.lean:4294967296:0",
    ] {
        assert_eq!(
            SourcePosition::parse(refused),
            None,
            "{refused:?} is not a canonical position"
        );
    }
    for text in ["x:1:0", "a b.lean:4294967295:4294967295", "dir/a.lean:7:12"] {
        let read = SourcePosition::parse(text).expect("a canonical position reads");
        assert_eq!(read.as_str(), text, "reading and printing are inverse");
    }
}

#[test]
fn the_binder_kinds_are_leans_four_binder_infos_one_each() {
    let kinds = [
        ParamKind::Explicit,
        ParamKind::Implicit,
        ParamKind::StrictImplicit,
        ParamKind::Instance,
    ];
    let names: Vec<&str> = kinds.iter().map(|k| k.lean_binder_info()).collect();
    assert_eq!(
        names,
        ["default", "implicit", "strictImplicit", "instImplicit"]
    );
    for k in kinds {
        assert_eq!(
            ParamKind::from_lean_binder_info(k.lean_binder_info()),
            Some(k)
        );
    }
    assert_eq!(ParamKind::from_lean_binder_info("auxDecl"), None);
}

#[test]
fn an_annotation_displays_in_the_surface_spelling() {
    assert_eq!(
        Annotation::Binder(ParamKind::StrictImplicit).to_string(),
        "binder strictImplicit"
    );
    assert_eq!(
        Annotation::Source(Provenance::Position(SourcePosition::new("A.lean", 2, 5))).to_string(),
        "source \"A.lean:2:5\""
    );
    assert_eq!(
        Annotation::Source(Provenance::LeanMetadata {
            key: "pp.nodot".into(),
            value: "Lean.DataValue.ofBool true".into(),
        })
        .to_string(),
        "mdata \"pp.nodot\" \"Lean.DataValue.ofBool true\""
    );
    assert_eq!(
        at("A.lean", 2, 5, g("Nat")).to_string(),
        "@[source \"A.lean:2:5\"] Nat",
        "the kernel's diagnostic display shows the annotation it carries"
    );
}
