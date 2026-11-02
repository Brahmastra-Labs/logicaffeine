use super::*;
use crate::ast::stmt::TypeExpr;
use logicaffeine_base::Arena;

struct Fixture<'a> {
    exprs: &'a Arena<Expr<'a>>,
    stmts: &'a Arena<Stmt<'a>>,
    types: &'a Arena<TypeExpr<'a>>,
}

impl<'a> Fixture<'a> {
    fn arenas(&self) -> Arenas<'a> {
        Arenas {
            exprs: self.exprs,
            stmts: self.stmts,
            types: self.types,
        }
    }
}

fn guards(stmts: &[Stmt]) -> Vec<String> {
    let mut out = Vec::new();
    logicaffeine_language::ast_depth::for_each_node(stmts, |node| {
        if let logicaffeine_language::ast_depth::Visited::Stmt(Stmt::Fail { reason }) = node {
            if let Expr::Literal(Literal::Text(_)) = reason {
                out.push(format!("{reason:?}"));
            }
        }
    });
    out
}

#[test]
fn a_program_that_never_pops_is_left_as_it_is() {
    let (e, s, t) = (Arena::new(), Arena::new(), Arena::new());
    let fx = Fixture { exprs: &e, stmts: &s, types: &t };
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let one = fx.exprs.alloc(Expr::Literal(Literal::Number(1)));
    let program = vec![Stmt::Let { var: x, ty: None, value: one, mutable: false }];
    assert!(resolve_pops(&program, fx.arenas(), &mut interner).is_none());
}

#[test]
fn every_pop_is_preceded_by_the_check_that_names_its_list() {
    let (e, s, t) = (Arena::new(), Arena::new(), Arena::new());
    let fx = Fixture { exprs: &e, stmts: &s, types: &t };
    let mut interner = Interner::new();
    let (xs, b, items, v) = (
        interner.intern("xs"),
        interner.intern("b"),
        interner.intern("items"),
        interner.intern("v"),
    );
    let list = fx.exprs.alloc(Expr::Identifier(xs));
    let field = fx.exprs.alloc(Expr::FieldAccess {
        object: fx.exprs.alloc(Expr::Identifier(b)),
        field: items,
    });
    let truth = fx.exprs.alloc(Expr::Literal(Literal::Boolean(true)));
    let nested = fx.stmts.alloc_slice(vec![Stmt::Pop { collection: field, into: None }]);
    let program = vec![
        Stmt::Pop { collection: list, into: Some(v) },
        Stmt::If { cond: truth, then_block: nested, else_block: None },
    ];
    let resolved = resolve_pops(&program, fx.arenas(), &mut interner).expect("the program pops");
    assert_eq!(resolved.len(), 3, "a check before the top-level pop, then the pop, then the If");
    assert!(matches!(resolved[0], Stmt::If { .. }));
    assert!(matches!(resolved[1], Stmt::Pop { .. }));
    let Stmt::If { then_block, .. } = &resolved[2] else { panic!("the If stays") };
    assert_eq!(then_block.len(), 2, "the nested pop is checked too");
    let sentences: Vec<String> = [&resolved[0], &then_block[0]]
        .iter()
        .map(|s| match s {
            Stmt::If { then_block, .. } => match &then_block[0] {
                Stmt::Fail { reason: Expr::Literal(Literal::Text(t)) } => interner.resolve(*t).to_string(),
                other => panic!("not a failure: {other:?}"),
            },
            other => panic!("not a check: {other:?}"),
        })
        .collect();
    assert_eq!(
        sentences,
        vec![
            "Cannot pop from xs: it is empty".to_string(),
            "Cannot pop from b's items: it is empty".to_string()
        ]
    );
    assert_eq!(guards(resolved).len(), 2);
}
