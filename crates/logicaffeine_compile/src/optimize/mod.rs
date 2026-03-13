mod fold;
mod dce;
mod propagate;
mod gvn;
mod licm;
mod closed_form;
mod ctfe;
mod deforest;
mod abstract_interp;
pub mod supercompile;
pub mod effects;
pub mod bta;
pub mod partial_eval;

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};

/// Lighter optimization for Futamura P1: fold + propagate + PE + CTFE.
/// Skips DCE, abstract interpretation, supercompilation, and structural
/// transforms (LICM, closed-form, deforestation) that eliminate branches
/// or restructure control flow. The residual preserves the program's
/// original control-flow structure while still folding constants,
/// propagating values, specializing functions, and evaluating CTFE.
pub fn optimize_for_projection<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    let mut variant_count: HashMap<Symbol, usize> = HashMap::new();
    for _ in 0..16 {
        let folded = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
        let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
        let (specialized, changes) = partial_eval::specialize_stmts_with_state(
            propagated, expr_arena, stmt_arena, interner, &mut variant_count,
            Some(&bta_cache),
        );
        current = specialized;
        if changes == 0 {
            break;
        }
    }
    let ctfe_d = ctfe::ctfe_stmts(current, expr_arena, stmt_arena, interner);
    fold::fold_stmts(ctfe_d, expr_arena, stmt_arena, interner)
}

pub fn optimize_program<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    // Compute BTA once — structural, stable across PE iterations
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    // Fixpoint loop: fold → propagate → PE, iterated until stable (max 8 cycles)
    let mut current = stmts;
    let mut variant_count: HashMap<Symbol, usize> = HashMap::new();
    for _ in 0..16 {
        let folded = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
        let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
        let (specialized, changes) = partial_eval::specialize_stmts_with_state(
            propagated, expr_arena, stmt_arena, interner, &mut variant_count,
            Some(&bta_cache),
        );
        current = specialized;
        if changes == 0 {
            break;
        }
    }
    // CTFE: compile-time function evaluation (factorial(10) → 3628800)
    let ctfe_d = ctfe::ctfe_stmts(current, expr_arena, stmt_arena, interner);
    // Fold again after CTFE to simplify any new constants
    let refolded = fold::fold_stmts(ctfe_d, expr_arena, stmt_arena, interner);
    // Common subexpression elimination (a+b computed twice → reuse)
    let cse_d = gvn::cse_stmts(refolded, expr_arena, stmt_arena, interner);
    // Loop-invariant code motion (hoist invariant Lets above loops)
    let licm_d = licm::licm_stmts(cse_d, stmt_arena, interner);
    // Closed-form loop recognition (sum += i → Gauss formula)
    let closed = closed_form::closed_form_stmts(licm_d, expr_arena, stmt_arena, interner);
    // Deforestation (fuse producer-consumer loop chains)
    let deforested = deforest::deforest_stmts(closed, expr_arena, stmt_arena, interner);
    // Abstract interpretation (value range analysis + dead branch elimination)
    let range_analyzed = abstract_interp::abstract_interp_stmts(deforested, expr_arena, stmt_arena);
    // Dead store + dead code elimination
    let dce_d = dce::eliminate_dead_code(range_analyzed, stmt_arena, expr_arena);
    // Supercompilation (unified inline + propagate + fold for remaining opportunities)
    supercompile::supercompile_stmts(dce_d, expr_arena, stmt_arena, interner)
}
