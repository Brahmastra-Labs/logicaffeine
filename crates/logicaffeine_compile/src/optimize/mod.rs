mod fold;
mod dce;
mod propagate;

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::Interner;

pub fn optimize_program<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    // Pass 1: Constant folding (2+3 → 5)
    let folded = fold::fold_stmts(stmts, expr_arena, stmt_arena, interner);
    // Pass 2: Constant propagation (Let x=5; x+3 → 5+3) + fold (→ 8)
    let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
    // Pass 3: Dead code elimination (if false → remove)
    dce::eliminate_dead_code(propagated, stmt_arena)
}
