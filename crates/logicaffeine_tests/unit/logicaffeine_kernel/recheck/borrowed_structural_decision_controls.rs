use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn app(a: Db, b: Db) -> Db {
    Db::App(DbBox::new(a), DbBox::new(b))
}
fn oracle(ctx: &Context, term: &Db) -> Option<usize> {
    let head @ Db::Fix(_) = spine_head(term) else {
        return None;
    };
    let Term::Fix { body, .. } = from_db(head, 0) else {
        unreachable!()
    };
    let column = crate::termination::structural_position(ctx, &body)?;
    spine_ref(term).1.get(column)?;
    Some(column)
}
fn fixed(domains: &[Db], core: Db, supplied: usize) -> Db {
    let body = domains
        .iter()
        .rev()
        .fold(core, |b, d| Db::Lam(DbBox::new(d.clone()), DbBox::new(b)));
    let mut term = Db::Fix(FixBody::owned(Box::new(body)));
    for _ in 0..supplied {
        term = app(term, Db::Var(0));
    }
    term
}
#[test]
fn choosing_a_column_never_materializes_the_recursive_program() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    let mut branch = Db::Var(0);
    for _ in 0..80 {
        branch = app(g("Succ"), branch);
    }
    let core = Db::Match {
        disc: DbBox::new(Db::Var(0)),
        motive: DbBox::new(g("Nat")),
        cases: vec![g("Zero"), branch],
    };
    let term = fixed(&[g("Nat"), g("Nat")], core, 2);
    assert_eq!(oracle(&ctx, &term), Some(1));
    RAISED_DB_NODES.with(|c| c.set(Some(0)));
    let result = fix_structural_column(&ctx, &term);
    let raised = RAISED_DB_NODES.with(|c| c.replace(None).unwrap());
    assert_eq!(result, Some(1));
    assert_eq!(raised, 0);
}
#[test]
fn borrowed_selection_matches_the_shared_named_rule_for_all_scope_positions() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    let domains = [
        g("Nat"),
        g("unknown"),
        Db::Var(0),
        Db::Sort(Universe::Prop),
        app(g("Nat"), Db::Var(0)),
        Db::Const {
            name: "Nat".into(),
            levels: vec![Universe::Var("u".into())],
        },
        Db::Let(
            DbBox::new(g("Nat")),
            DbBox::new(g("Nat")),
            DbBox::new(Db::Var(0)),
        ),
    ];
    for a in &domains {
        for b in &domains {
            for k in 0..5 {
                let cores = [
                    Db::Match {
                        disc: DbBox::new(Db::Var(k)),
                        motive: DbBox::new(g("Nat")),
                        cases: vec![],
                    },
                    Db::Match {
                        disc: DbBox::new(app(g("f"), Db::Var(k))),
                        motive: DbBox::new(g("Nat")),
                        cases: vec![],
                    },
                    Db::Var(k),
                ];
                for core in cores {
                    for supplied in 0..4 {
                        let term = fixed(&[a.clone(), b.clone()], core.clone(), supplied);
                        assert_eq!(
                            fix_structural_column(&ctx, &term),
                            oracle(&ctx, &term),
                            "domains={a:?},{b:?}, core={core:?}, supplied={supplied}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn selection_reads_current_context_and_does_not_guess_through_aliases() {
    let empty = Context::new();
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    ctx.add_definition(
        "NatAlias".into(),
        Term::Sort(Universe::Type(0)),
        Term::Global("Nat".into()),
    );
    for name in ["Nat", "NatAlias", "unknown"] {
        let term = fixed(&[g(name)], Db::Var(0), 1);
        assert_eq!(fix_structural_column(&empty, &term), None);
        assert_eq!(fix_structural_column(&ctx, &term), oracle(&ctx, &term));
        assert_eq!(
            fix_structural_column(&ctx, &term),
            (name == "Nat").then_some(0)
        );
    }
}
