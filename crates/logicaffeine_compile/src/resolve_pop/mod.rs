//! `Pop` from a list with no items stops the program with one sentence naming the list
//! (`logicaffeine_data::failure::pop_from_empty`), on every tier. The rule is written once, here: a `Pop`
//! is preceded by the check `If length of <list> equals 0: Fail with "<sentence>".`, so every engine and
//! every backend runs the same statements, and a `Pop` itself only ever meets a list that has an item to
//! give. A pop proven to find an item ([`nonempty`]) needs no check, and gets none.

use logicaffeine_base::Interner;

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::ast_map::{walk_stmt, Arenas, AstMap};

mod nonempty;

/// The program with a check before every pop that may meet an empty list; `None` when no pop needs one.
pub(crate) fn resolve_pops<'a>(
    stmts: &[Stmt<'a>],
    arenas: Arenas<'a>,
    interner: &mut Interner,
) -> Option<&'a [Stmt<'a>]> {
    if pops(stmts) == 0 {
        return None;
    }
    let block = arenas.stmts.alloc_slice(stmts.to_vec());
    let proven = nonempty::pops_that_find_an_item(block);
    if proven.len() == pops(block) {
        return None;
    }
    let mut guard = Guard {
        arenas,
        interner,
        proven,
    };
    Some(guard.map_block(block))
}

fn pops(stmts: &[Stmt]) -> usize {
    let mut count = 0;
    logicaffeine_language::ast_depth::for_each_node(stmts, |node| {
        if let logicaffeine_language::ast_depth::Visited::Stmt(Stmt::Pop { .. }) = node {
            count += 1;
        }
    });
    count
}

struct Guard<'a, 'i> {
    arenas: Arenas<'a>,
    interner: &'i mut Interner,
    /// The pops (by address) proven to find an item.
    proven: std::collections::HashSet<usize>,
}

impl<'a> Guard<'a, '_> {
    /// The list as the program writes it.
    fn phrase(&self, list: &Expr) -> String {
        match list {
            Expr::Identifier(s) => self.interner.resolve(*s).to_string(),
            Expr::FieldAccess { object, field } => format!(
                "{}'s {}",
                self.phrase(object),
                self.interner.resolve(*field)
            ),
            Expr::Index { collection, index } => format!(
                "item {} of {}",
                logicaffeine_language::printer::render_expr(index, self.interner),
                self.phrase(collection)
            ),
            other => logicaffeine_language::printer::render_expr(other, self.interner),
        }
    }

    /// `If length of <list> equals 0: Fail with "<sentence>".`
    fn check(&mut self, list: &'a Expr<'a>) -> Stmt<'a> {
        let sentence = logicaffeine_data::failure::pop_from_empty(&self.phrase(list));
        let exprs = self.arenas.exprs;
        let length = exprs.alloc(Expr::Length { collection: list });
        let zero = exprs.alloc(Expr::Literal(Literal::Number(0)));
        let empty = exprs.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Eq,
            left: length,
            right: zero,
        });
        let reason = exprs.alloc(Expr::Literal(Literal::Text(self.interner.intern(&sentence))));
        Stmt::If {
            cond: empty,
            then_block: self.arenas.stmts.alloc_slice(vec![Stmt::Fail { reason }]),
            else_block: None,
        }
    }
}

impl<'a> AstMap<'a> for Guard<'a, '_> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_block(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        let mut out = Vec::with_capacity(block.len());
        for s in block {
            let mapped = walk_stmt(self, s);
            if let Stmt::Pop { collection, .. } = &mapped {
                if !self.proven.contains(&(s as *const Stmt as usize)) {
                    out.push(self.check(collection));
                }
            }
            out.push(mapped);
        }
        self.arenas.stmts.alloc_slice(out)
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/resolve_pop/tests.rs"]
mod tests;
