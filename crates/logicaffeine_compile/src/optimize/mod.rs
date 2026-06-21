mod defunctionalize;
mod fold;
mod inline_tiny;
mod inline_leaf;
mod inline_recursive;
mod popcount_leaf;
mod symmetry;
mod dce;
mod propagate;
mod gvn;
mod licm;
mod closed_form;
mod ctfe;
mod deforest;
// Loop unrolling reuses the codegen peephole's counted-loop recognizer, so it
// rides the same feature gate; the AOT (Rust-emitting) pipeline is its consumer.
#[cfg(feature = "codegen")]
mod unroll;
// Guard-based loop index-set splitting reuses the same recognizer (its emitted
// sub-loops must satisfy it to vectorize), so it rides the same gate; AOT-only.
#[cfg(feature = "codegen")]
mod loop_split;
mod affine;
mod abstract_interp;
pub use abstract_interp::{
    lin_to_rust, oracle_analyze, oracle_analyze_with, oracle_analyze_with_entry_guards, OracleFacts,
    ScalarKind,
};
pub mod egraph;
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

/// The RUN-PATH pipeline (EXODIA D1): the Futamura residual — fold,
/// propagate, polyvariant PE, CTFE, GVN, LICM, closed-form, deforestation,
/// interval analysis, DCE — everything except supercompilation (whose
/// driving cost is unbounded; it stays AOT-only). Budgeted: programs beyond
/// the statement gate run raw, and `LOGOS_RUN_OPT=0` kills the pass
/// entirely. Optimizer time lands inside the measured run, so the budget is
/// part of the contract.
pub fn optimize_for_run<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    if std::env::var_os("LOGOS_RUN_OPT").is_some_and(|v| v == "0") {
        return stmts;
    }
    const MAX_RUN_OPT_STMTS: usize = 5_000;
    if stmts.len() > MAX_RUN_OPT_STMTS {
        return stmts;
    }
    // Pass mask for surgical bisection/kill-switches. (1 = PE fixpoint loop,
    // 2 = CTFE+refold, 4 = GVN, 8 = LICM, 16 = closed-form, 32 = deforestation,
    // 64 = abstract interpretation, 128 = DCE, 256 = bounded recursion inlining.)
    // Bit 256 is ON by default: the run-path recursion unroller
    // (`inline_recursive_fns_run`) is shape-aware — it unrolls return-position
    // tree recursion deep (fib/binary_trees, which the live path has no
    // closed-form/memoization transform for) and caps loop-interleaved recursion
    // shallow (n-queens, whose nested-loop body would otherwise drop to the
    // bytecode tier). The prior net-negative came from the AOT depth-8 unroll on
    // n-queens; with the shape-aware run-path depth an interleaved A/B is a clean
    // win (fib ≈ 0.53, binary_trees ≈ 0.57, n-queens ≈ 0.94 of un-inlined; every
    // non-recursive / out-of-fragment bench unchanged). Clear the bit to disable.
    let mask: u32 = std::env::var("LOGOS_RUN_OPT_MASK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(u32::MAX);
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    if mask & 1 != 0 {
        // Tiny pure helpers inline first, so PE/fold/GVN/LICM see straight
        // arithmetic and the loop regions compile call-free.
        current = inline_tiny::inline_tiny_fns(current, expr_arena, stmt_arena, interner);
        // Statement-body leaf helpers (iterative gcd shape) fold in next, so
        // their calling loops compile as one call-free region.
        current = inline_leaf::inline_leaf_fns(current, expr_arena, stmt_arena, interner);
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
    }
    if mask & 2 != 0 {
        current = ctfe::ctfe_stmts(current, expr_arena, stmt_arena, interner);
        current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
    }
    if mask & 4 != 0 {
        current = gvn::cse_stmts(current, expr_arena, stmt_arena, interner);
    }
    if mask & 8 != 0 {
        current = licm::licm_stmts_run(current, expr_arena, stmt_arena, interner);
    }
    if mask & 16 != 0 {
        current = closed_form::closed_form_stmts(current, expr_arena, stmt_arena, interner);
    }
    if mask & 32 != 0 {
        current = deforest::deforest_stmts(current, expr_arena, stmt_arena, interner);
    }
    if mask & 64 != 0 {
        current = abstract_interp::abstract_interp_stmts(current, expr_arena, stmt_arena);
    }
    if mask & 128 != 0 {
        current = dce::eliminate_dead_code(current, stmt_arena, expr_arena);
    }
    // Bounded recursion inlining (recursion unrolling) — the lone AOT pass we
    // also run on the live run path. It flattens a self-recursive function's
    // own loop-interleaved body k levels deep (the inline LLVM refuses, gcc
    // -O3 performs), cutting the per-call VM overhead that dominates the
    // recursion benchmarks. It MUST run LAST: placed before the fixpoint
    // passes it re-analyses k enlarged clones and the optimizer cost balloons
    // ~30×, so it runs on the already-optimized residual exactly like the AOT
    // pipeline (each clone is pre-optimized). It defers to TCE/accumulator by
    // construction — it fires only on loop-interleaved recursion, never on the
    // return-position shape `tail_call` later rewrites to a constant-stack
    // loop. Optimizer time lands inside the measured run, so it is gated twice:
    // the mask bit (256), and a tight statement budget (`MAX_RUN_INLINE_STMTS`)
    // below the broad run-opt gate — the recursive kernels are tiny source
    // programs, while a large body's inlining cost would outweigh the savings.
    const MAX_RUN_INLINE_STMTS: usize = 1_500;
    if mask & 256 != 0 && current.len() <= MAX_RUN_INLINE_STMTS {
        let unrolled =
            inline_recursive::inline_recursive_fns_run(current, expr_arena, stmt_arena, interner);
        // Collapse the argument-binding temps the splice introduces
        // (`__rN_n = n`), exactly as the AOT pipeline does after this pass.
        let folded = fold::fold_stmts(unrolled, expr_arena, stmt_arena, interner);
        let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
        current = dce::eliminate_dead_code(propagated, stmt_arena, expr_arena);
    }
    current
}

/// The AOT optimizer — ONE pipeline. The Futamura residual (fold, propagate,
/// polyvariant PE, CTFE) then the ARCHITECT: equality saturation over a
/// kernel-certified e-graph, Oracle facts gating the conditional rewrites
/// (value facts suppressed for loop-mutated variables — see `egraph::convert`);
/// then defunctionalization, deforestation, interval analysis, DCE, and
/// supercompilation. GVN/LICM/closed-form STILL run after the e-graph until it
/// subsumes their cross-statement and loop-recurrence reach (the EG waves).
pub fn optimize_program<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    // NOTE: no early tiny-fn inliner here — that is a RUN-PATH device
    // (optimize_for_run skips supercompile for budget). This AOT pipeline
    // inlines through supercompile at the end exactly like v1, so the PE
    // machinery keeps its subjects.
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
    current = ctfe::ctfe_stmts(current, expr_arena, stmt_arena, interner);
    current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
    // Reflection symmetry-breaking: halve a bitmask counting search's first-row
    // enumeration (×2 + odd-n middle) when the kernel proves the reflection
    // invariance for all n. Runs on the clean specialized entry, before the
    // structural passes; the sub-counts it calls still get popcount-leaf +
    // recursion unrolling later. Fail-closed (kernel certificate gates it).
    current = symmetry::break_symmetry_stmts(current, expr_arena, stmt_arena, interner);
    current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
    // Fully unroll small constant-trip loops nested inside hot loops (nbody's
    // per-step force loops), then refold/repropagate so the substituted indices
    // collapse to constants (`item (3-1) of bx` → `bx[2]`) and the dead counter
    // scaffolding is cleaned up. LLVM then SROAs the `[f64; N]` arrays into
    // registers and vectorizes the straight-line body — the codegen it already
    // produces for top-level constant loops but refuses inside a runtime loop.
    #[cfg(feature = "codegen")]
    {
        let (unrolled, changed) = unroll::unroll_stmts(current, expr_arena, stmt_arena, interner);
        current = unrolled;
        if changed {
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    // Group 4: direct closures lift to first-order functions BEFORE the
    // e-graph runs, so the Architect (and every later pass) sees plain
    // calls instead of heap closures.
    current = defunctionalize::defunctionalize_stmts(current, expr_arena, stmt_arena, interner);
    // The Architect saturates every expression under kernel-certified
    // rewrites with Oracle-gated conditionals. GVN/LICM/closed-form STAY
    // until the e-graph's cross-statement runs (M13's SSA versioning)
    // subsume their capabilities — the structural suite pins them, and a
    // capability superset is the honest sprint-22 wiring.
    current = egraph::convert::egraph_stmts(current, expr_arena, stmt_arena, interner);
    current = gvn::cse_stmts(current, expr_arena, stmt_arena, interner);
    current = licm::licm_stmts(current, expr_arena, stmt_arena, interner);
    current = closed_form::closed_form_stmts(current, expr_arena, stmt_arena, interner);
    current = deforest::deforest_stmts(current, expr_arena, stmt_arena, interner);
    // Guard-based loop index-set splitting: split a counted loop carrying a
    // top-level affine-monotone IV guard into a guard-false prefix (then-block
    // dropped → memcpy) and a guard-true suffix (then-block inlined, branch-free
    // → vectorizable unconditional load), behind a version guard that keeps the
    // original loop for an out-of-range threshold. fold+propagate immediately
    // after collapse the version guard / branches where statically resolvable,
    // and the suffix's literal-threshold IV init lets abstract_interp re-prove
    // the now-unconditional access in range. AOT-only (no SIMD payoff for the
    // interpreter), so it is not in optimize_for_run / optimize_for_projection.
    #[cfg(feature = "codegen")]
    {
        current = loop_split::loop_split_stmts(current, expr_arena, stmt_arena, interner);
        current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
        current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
    }
    current = abstract_interp::abstract_interp_stmts(current, expr_arena, stmt_arena);
    let dced = dce::eliminate_dead_code(current, stmt_arena, expr_arena);
    let current = supercompile::supercompile_stmts(dced, expr_arena, stmt_arena, interner);
    // Popcount-leaf: collapse the second-to-last level of a bitmask counting
    // search (`row == n-1`, every remaining bit is one solution) into a single
    // `count_ones`. Runs BEFORE inline_recursive so the fast path is carried
    // into every unrolled clone. Pure structural rewrite (no kernel).
    let current = popcount_leaf::popcount_leaf_stmts(current, expr_arena, stmt_arena, interner);
    // Bounded recursive inlining (recursion unrolling) runs LAST: flatten each
    // self-recursive function's own body k levels deep — the inline LLVM
    // refuses but gcc -O3 performs (the lever that lets compiled-LOGOS
    // n-queens match/beat C). Placed after the heavy analyses (e-graph,
    // supercompile) so they optimize the ORIGINAL body once and never pay to
    // re-analyze the k enlarged copies — each clone is already optimized.
    // fold+propagate+DCE then collapse the argument-binding temps the splice
    // introduces (`__rN_n = n`).
    let unrolled = inline_recursive::inline_recursive_fns(current, expr_arena, stmt_arena, interner);
    let folded = fold::fold_stmts(unrolled, expr_arena, stmt_arena, interner);
    let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
    dce::eliminate_dead_code(propagated, stmt_arena, expr_arena)
}
