//! Lean's binder names, carried through the importer (`ImportRequest::lean_binder_names`), and the
//! one-import retention of every checked proof (`RetainSource::EveryProof`).
//!
//! The binder names are for readers — emitted LOGOS reads with Lean's own `p`, `h`, `motive` — and
//! must be invisible to checking. The lock is a differential over every pinned Lean 4.14.0 export:
//! importing with and without the names gives the same verdict, the same report, the same retained
//! declarations, and α-EQUIVALENT terms. (Not syntactically identical: the kernel's capture-avoiding
//! substitution primes a binder — `arena.local.3'` — exactly when two names collide, and distinct Lean
//! names collide less often. A prime is a renaming, and α-equivalence is what checking sees.)

use logicaffeine_kernel::interface::lean::{self as lean, ImportRequest, RetainSource};
use logicaffeine_kernel::Term;
use std::path::{Path, PathBuf};

fn corpus() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(root.join("tests/fixtures/lean_conformance/cases_lean414"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "ndjson"))
            .collect();
    let cli = root.join("tests/fixtures/lean/exports");
    files.extend(
        std::fs::read_dir(cli)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "ndjson")),
    );
    files.sort();
    files
        .into_iter()
        .filter(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            text.lines().next().unwrap().contains("\"4.14.0\"") && text.len() < 1 << 20
        })
        .collect()
}

/// α-equivalence: the same term up to the names of bound variables.
fn alpha_eq(a: &Term, b: &Term) -> bool {
    fn go<'t>(a: &'t Term, b: &'t Term, left: &mut Vec<&'t str>, right: &mut Vec<&'t str>) -> bool {
        let bind = |left: &mut Vec<&'t str>, right: &mut Vec<&'t str>, x: &'t str, y: &'t str| {
            left.push(x);
            right.push(y);
        };
        match (a, b) {
            (Term::Var(x), Term::Var(y)) => {
                let i = left.iter().rposition(|n| *n == x);
                let j = right.iter().rposition(|n| *n == y);
                match (i, j) {
                    (Some(i), Some(j)) => left.len() - i == right.len() - j,
                    (None, None) => x == y,
                    _ => false,
                }
            }
            (
                Term::Pi {
                    param: x,
                    param_type: s,
                    body_type: t,
                },
                Term::Pi {
                    param: y,
                    param_type: u,
                    body_type: v,
                },
            )
            | (
                Term::Lambda {
                    param: x,
                    param_type: s,
                    body: t,
                },
                Term::Lambda {
                    param: y,
                    param_type: u,
                    body: v,
                },
            ) => {
                if !go(s, u, left, right) {
                    return false;
                }
                bind(left, right, x, y);
                let same = go(t, v, left, right);
                left.pop();
                right.pop();
                same
            }
            (
                Term::Let {
                    name: x,
                    ty: s,
                    value: t,
                    body: w,
                },
                Term::Let {
                    name: y,
                    ty: u,
                    value: v,
                    body: z,
                },
            ) => {
                if !go(s, u, left, right) || !go(t, v, left, right) {
                    return false;
                }
                bind(left, right, x, y);
                let same = go(w, z, left, right);
                left.pop();
                right.pop();
                same
            }
            (Term::App(f, x), Term::App(g, y)) => go(f, g, left, right) && go(x, y, left, right),
            (
                Term::Match {
                    discriminant: d,
                    motive: m,
                    cases: c,
                },
                Term::Match {
                    discriminant: e,
                    motive: n,
                    cases: k,
                },
            ) => {
                c.len() == k.len()
                    && go(d, e, left, right)
                    && go(m, n, left, right)
                    && c.iter().zip(k).all(|(x, y)| go(x, y, left, right))
            }
            (Term::Fix { name: x, body: s }, Term::Fix { name: y, body: t }) => {
                bind(left, right, x, y);
                let same = go(s, t, left, right);
                left.pop();
                right.pop();
                same
            }
            (Term::MutualFix { defs: d, index: i }, Term::MutualFix { defs: e, index: j }) => {
                if i != j || d.len() != e.len() {
                    return false;
                }
                for ((x, _), (y, _)) in d.iter().zip(e) {
                    bind(left, right, x, y);
                }
                let same = d
                    .iter()
                    .zip(e)
                    .all(|((_, s), (_, t))| go(s, t, left, right));
                for _ in d {
                    left.pop();
                    right.pop();
                }
                same
            }
            (x, y) => x == y,
        }
    }
    go(a, b, &mut Vec::new(), &mut Vec::new())
}

#[test]
fn alpha_equivalence_is_up_to_bound_names_only() {
    let lam = |p: &str, body: Term| Term::Lambda {
        param: p.into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(body),
    };
    let v = |n: &str| Term::Var(n.into());
    assert!(alpha_eq(
        &lam("a", lam("b", v("a"))),
        &lam("x", lam("y", v("x")))
    ));
    assert!(!alpha_eq(
        &lam("a", lam("b", v("a"))),
        &lam("x", lam("y", v("y")))
    ));
    assert!(!alpha_eq(&lam("a", v("free")), &lam("a", v("other"))));
    assert!(
        alpha_eq(&lam("a", lam("a", v("a"))), &lam("x", lam("y", v("y")))),
        "shadowing resolves innermost"
    );
}

/// `term` with every carried Lean binder name dropped: `arena.local.3.h` → `arena.local.3`.
fn erase(term: &Term) -> Term {
    let name = |n: &String| match lean::lean_binder_name(n) {
        Some(lean) => n[..n.len() - lean.len() - 1].to_string(),
        None => n.clone(),
    };
    match term {
        Term::Var(n) => Term::Var(name(n)),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: name(param),
            param_type: Box::new(erase(param_type)),
            body_type: Box::new(erase(body_type)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: name(param),
            param_type: Box::new(erase(param_type)),
            body: Box::new(erase(body)),
        },
        Term::Let {
            name: n,
            ty,
            value,
            body,
        } => Term::Let {
            name: name(n),
            ty: Box::new(erase(ty)),
            value: Box::new(erase(value)),
            body: Box::new(erase(body)),
        },
        Term::App(f, a) => Term::App(Box::new(erase(f)), Box::new(erase(a))),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(erase(discriminant)),
            motive: Box::new(erase(motive)),
            cases: cases.iter().map(erase).collect(),
        },
        Term::Fix { name: n, body } => Term::Fix {
            name: n.clone(),
            body: Box::new(erase(body)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs.iter().map(|(n, t)| (n.clone(), erase(t))).collect(),
            index: *index,
        },
        other => other.clone(),
    }
}

fn binders(term: &Term, out: &mut Vec<String>) {
    match term {
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            out.push(param.clone());
            binders(param_type, out);
            binders(body_type, out);
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            out.push(param.clone());
            binders(param_type, out);
            binders(body, out);
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            out.push(name.clone());
            binders(ty, out);
            binders(value, out);
            binders(body, out);
        }
        Term::App(f, a) => {
            binders(f, out);
            binders(a, out);
        }
        _ => {}
    }
}

fn on_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn request(audit: bool, names: bool) -> ImportRequest {
    ImportRequest {
        audit,
        retain: RetainSource::EveryProof,
        lean_binder_names: names,
    }
}

/// ★ The names change nothing the kernel decides.
#[test]
fn binder_names_are_invisible_to_checking() {
    on_big_stack(|| {
        let files = corpus();
        assert!(
            files.len() >= 15,
            "the pinned corpus is exercised: {}",
            files.len()
        );
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            for audit in [false, true] {
                let plain = lean::import_with(&text, request(audit, false));
                let named = lean::import_with(&text, request(audit, true));
                match (plain, named) {
                    (Ok((plain_ctx, plain_report)), Ok((named_ctx, named_report))) => {
                        assert_eq!(
                            plain_report,
                            named_report,
                            "{} audit={audit}: the report",
                            path.display()
                        );
                        let plain: Vec<_> = plain_ctx.retained_declarations().collect();
                        let named: Vec<_> = named_ctx.retained_declarations().collect();
                        assert_eq!(plain.len(), named.len(), "{}", path.display());
                        for ((pn, pt, pb), (nn, nt, nb)) in plain.iter().zip(&named) {
                            assert_eq!(pn, nn);
                            assert!(alpha_eq(pt, nt), "{}: {pn} type", path.display());
                            assert!(alpha_eq(pb, nb), "{}: {pn} body", path.display());
                            assert_eq!(pt, &&erase(pt), "the historical spelling carries no names");
                        }
                    }
                    (Err(plain), Err(named)) => {
                        assert_eq!(
                            plain.code(),
                            named.code(),
                            "{}: the verdict kind",
                            path.display()
                        );
                        assert_eq!(
                            plain.reason().code(),
                            named.reason().code(),
                            "{}: the code",
                            path.display()
                        );
                        assert_eq!(
                            plain.reason().line(),
                            named.reason().line(),
                            "{}: the line",
                            path.display()
                        );
                    }
                    (plain, named) => panic!(
                        "{} audit={audit}: names changed the verdict: {:?} vs {:?}",
                        path.display(),
                        plain.err().map(|v| v.reason().message().to_string()),
                        named.err().map(|v| v.reason().message().to_string())
                    ),
                }
            }
        }
    });
}

/// And they are really there: `Demo.andSwap (p q : Prop) (h : p ∧ q) : q ∧ p` keeps `p`, `q`, `h`.
#[test]
fn a_retained_proof_reads_with_leans_binder_names() {
    on_big_stack(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text =
            std::fs::read_to_string(root.join("tests/fixtures/lean/exports/Demo.andSwap.ndjson"))
                .unwrap();
        let (ctx, _) = lean::import_with(&text, request(false, true)).unwrap();
        let (ty, body) = ctx
            .retained(r#"/s:"Demo"/s:"andSwap""#)
            .expect("every proof is retained");
        let mut names = Vec::new();
        binders(ty, &mut names);
        binders(body, &mut names);
        let lean: Vec<&str> = names
            .iter()
            .filter_map(|n| lean::lean_binder_name(n))
            .collect();
        for expected in ["p", "q", "h"] {
            assert!(lean.contains(&expected), "{expected} is carried: {names:?}");
        }
    });
}

#[test]
fn a_binder_name_reads_back_from_its_kernel_spelling() {
    assert_eq!(lean::lean_binder_name("arena.local.3.h"), Some("h"));
    assert_eq!(lean::lean_binder_name("arena.local.12.a.b"), Some("a.b"));
    assert_eq!(lean::lean_binder_name("arena.local.3"), None);
    assert_eq!(lean::lean_binder_name("arena.local..h"), None);
    assert_eq!(lean::lean_binder_name("x"), None);
}

/// One import retains every checked theorem and opaque proof — and only those.
#[test]
fn every_proof_is_retained_in_one_import() {
    on_big_stack(|| {
        for path in corpus() {
            let text = std::fs::read_to_string(&path).unwrap();
            let Ok((ctx, _)) = lean::import_with(&text, request(false, true)) else {
                continue;
            };
            let expected: Vec<String> = {
                let read = lean::intake::read(&text, &lean::intake::STAGE_A).unwrap();
                read.declarations()
                    .iter()
                    .filter(|d| {
                        matches!(
                            d.kind,
                            lean::intake::DeclarationKind::Theorem
                                | lean::intake::DeclarationKind::Opaque
                        )
                    })
                    .map(|d| d.kernel_name.clone())
                    .collect()
            };
            let retained: Vec<String> = ctx
                .retained_declarations()
                .map(|(n, ..)| n.to_string())
                .collect();
            // A runtime (unsafe or partial) opaque is quarantined, never checked, so never retained.
            for name in &retained {
                assert!(
                    expected.contains(name),
                    "{}: {name} is not a theorem or opaque",
                    path.display()
                );
            }
            let quarantined = text.contains("\"isUnsafe\":true");
            if !quarantined {
                assert_eq!(
                    retained,
                    expected,
                    "{}: every checked proof, in export order",
                    path.display()
                );
            }
            assert!(
                ctx.retained_source().is_none(),
                "no single declaration was requested"
            );
        }
    });
}

/// The public non-audit door returns the checked context and keeps the historical APIs as they were.
#[test]
fn import_returns_the_context_and_the_historical_doors_are_unchanged() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text =
        std::fs::read_to_string(root.join(
            "tests/fixtures/lean_conformance/cases_lean414/T00-019.sharing.structural.ndjson",
        ))
        .unwrap();
    let (ctx, report) = lean::import(&text).unwrap();
    assert!(!report.declaration_audit_requested);
    assert!(ctx.get_global(r#"/s:"T00_019"/s:"sharedProof""#).is_some());
    assert_eq!(ctx.retained_declarations().count(), 0);

    let target = r#"/s:"T00_019"/s:"sharedProof""#;
    let (ctx, _) = lean::import_audited_retaining_source(&text, target).unwrap();
    let (_, ty, body) = ctx.retained_source().unwrap();
    let mut names = Vec::new();
    binders(ty, &mut names);
    binders(body, &mut names);
    assert!(
        names.iter().all(|n| lean::lean_binder_name(n).is_none()),
        "import_audited_retaining_source keeps its historical binder spelling: {names:?}"
    );
}
