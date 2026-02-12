mod fold;
mod dce;

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::Interner;

pub fn optimize_program<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let folded = fold::fold_stmts(stmts, expr_arena, stmt_arena, interner);
    dce::eliminate_dead_code(folded, stmt_arena)
}
