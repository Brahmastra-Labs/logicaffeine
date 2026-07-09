mod defunctionalize;
mod fold;
mod splice_fuse;
mod inline_tiny;
mod inline_leaf;
mod inline_recursive;
mod popcount_leaf;
mod symmetry;
mod dce;
mod propagate;
mod gvn;
mod loop_carried_cse;
mod float_induction_sr;
mod licm;
mod bound_version;
mod closed_form;
mod ctfe;
mod deforest;
// Loop unrolling reuses the codegen peephole's counted-loop recognizer, so it
// rides the same feature gate; the AOT (Rust-emitting) pipeline is its consumer.
#[cfg(feature = "codegen")]
mod unroll;
// Fixed-size float/int Seq scalarization (the interpreter's SROA). Reuses the
// codegen scalarization detection, so it rides the same feature gate; paired
// AFTER the run-path unroller, which makes every surviving index constant.
#[cfg(feature = "codegen")]
mod scalarize;
// Affine read-only `Seq` scalarization (the interpreter's analog of the AOT's
// `codegen::affine_array`): delete a CSR-style offset array built by one affine
// `Push f(i) to arr` loop and substitute every `item k of arr` read with the
// closed form `f(k-1)`. Default OFF behind `LOGOS_RUN_AFFINE`; AOT-only feature
// gate (it rewrites the run-path AST). Runs before `scalarize`, on the array
// form GVN already proved reload-correct.
#[cfg(feature = "codegen")]
mod affine_scalarize;
// Guard-based loop index-set splitting reuses the same recognizer (its emitted
// sub-loops must satisfy it to vectorize), so it rides the same gate; AOT-only.
#[cfg(feature = "codegen")]
mod loop_split;
mod affine;
mod abstract_interp;
pub use abstract_interp::{
    lin_to_rust, oracle_analyze, oracle_analyze_with, oracle_analyze_with_entry_guards, OracleFacts,
    ScalarKind, VarProvenFacts,
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
use crate::optimization::{
    admits_pinned, FiredOptimizations, HotswapConfig, Opt, OptimizationConfig, Pin, Tier,
};

thread_local! {
    /// The optimization config for the CURRENT compile, consulted by codegen and
    /// the VM compiler for their per-emit toggle decisions (e.g. emit
    /// `get_unchecked` vs a checked index). It is set ONCE at each compile entry
    /// (`codegen_program`, `optimize_for_run`) from the explicitly-threaded
    /// `OptimizationConfig`; any path that does not set it sees the all-on
    /// default (the prior, fully-optimized behaviour). This is the single
    /// replacement for the ~16 scattered `std::env::var("LOGOS_*")` reads codegen
    /// and the VM used to perform — one controlled per-compile source, not
    /// ambient process state.
    static ACTIVE_CONFIG: std::cell::Cell<OptimizationConfig> =
        std::cell::Cell::new(OptimizationConfig::all_on());
}

/// Record the optimization config for the current compile. Call at every compile
/// entry (codegen / VM) so the deep codegen + VM toggle reads see it.
pub fn set_active_config(cfg: OptimizationConfig) {
    ACTIVE_CONFIG.with(|c| c.set(cfg));
}

thread_local! {
    /// Expr node addresses whose Int op is PROVEN in-range by the pass that
    /// CONSTRUCTED them — the proof lives where the knowledge lives (e.g.
    /// `try_defer_modulus` sizes its version guard so no op in the guarded
    /// chunk can overflow, a relation the interval oracle's widening cannot
    /// re-derive). Codegen consults this BEFORE the interval oracle and
    /// lowers a registered op as raw i64. Cleared at every optimizer entry
    /// so a freed arena's reused addresses can never leak a stale proof
    /// into a later program on the same thread.
    static PROVEN_RAW_INT_OPS: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Forget all constructed-proof registrations (call at optimizer entry).
pub(crate) fn clear_proven_raw_int_ops() {
    PROVEN_RAW_INT_OPS.with(|s| s.borrow_mut().clear());
}

/// Register an Int op the CONSTRUCTING pass proved can never leave i64.
pub(crate) fn mark_proven_raw_int_op(e: &crate::ast::stmt::Expr) {
    PROVEN_RAW_INT_OPS.with(|s| {
        s.borrow_mut().insert(e as *const crate::ast::stmt::Expr as usize);
    });
}

/// Was this exact op node proven in-range by its constructing pass?
pub fn expr_proven_raw_int_op(e: &crate::ast::stmt::Expr) -> bool {
    PROVEN_RAW_INT_OPS.with(|s| s.borrow().contains(&(e as *const crate::ast::stmt::Expr as usize)))
}

/// The current compile's optimization config (all-on if unset).
pub fn active_config() -> OptimizationConfig {
    ACTIVE_CONFIG.with(|c| c.get())
}

thread_local! {
    /// When `Some`, an optimization-firing trace is active for the current
    /// compile: each optimization records (via [`mark_fired`]) the moment it
    /// actually fires. `None` (the default) means no trace — [`mark_fired`] is
    /// a no-op and the change-detection in [`run_traced`] is skipped, so the
    /// normal compile path pays nothing. Opt-in, set once per traced compile by
    /// [`begin_fired_trace`]; lives on the same thread that runs both the
    /// optimizer and codegen, so codegen-time marks are recorded too.
    static FIRED: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };

    /// The PRECEDENCE decisions recorded during the current trace: each
    /// `(winner, loser)` pair is a spot where the code chose `winner` and skipped
    /// `loser` for an instance (the conflict the optimization SYSTEM resolves).
    /// `None` when no trace is active, so [`mark_preempted`] is then a no-op and
    /// the normal compile path pays nothing. Shares the trace lifetime with
    /// [`FIRED`] — both reset by [`begin_fired_trace`].
    static PREEMPTED: std::cell::RefCell<Option<Vec<(Opt, Opt)>>> =
        const { std::cell::RefCell::new(None) };
}

/// Begin recording which optimizations fire AND which they preempt for the
/// current compile. Resets both accumulators; pair with [`end_fired_trace`] /
/// [`end_preempted_trace`].
pub fn begin_fired_trace() {
    FIRED.with(|f| f.set(Some(0)));
    PREEMPTED.with(|p| *p.borrow_mut() = Some(Vec::new()));
}

/// Stop recording and take the fired set (`None` if no trace was active).
pub fn end_fired_trace() -> Option<FiredOptimizations> {
    FIRED.with(|f| f.take().map(FiredOptimizations::from_bits))
}

/// Whether a firing trace is currently active. Gates the cost of the
/// before/after fingerprint in [`run_traced`].
#[inline]
pub fn fired_trace_active() -> bool {
    FIRED.with(|f| f.get().is_some())
}

/// Record that `opt` fired in the current compile. No-op when no trace is active.
#[inline]
pub fn mark_fired(opt: Opt) {
    FIRED.with(|f| {
        if let Some(bits) = f.get() {
            f.set(Some(bits | opt.bit()));
        }
    });
}

/// Record that `winner` took precedence over `loser` here — the exact spot the
/// code skips one optimization for another (the conflict, traced at its source,
/// the same way [`mark_fired`] traces firing). No-op when no trace is active, so
/// the normal compile path and the benchmark scripts pay nothing. Self-gated on
/// BOTH being enabled in the current config: a preemption is only meaningful when
/// the two genuinely contest an instance — so toggling either off cleanly stops
/// the edge from being reported.
#[inline]
pub fn mark_preempted(winner: Opt, loser: Opt) {
    if !fired_trace_active() {
        return;
    }
    let cfg = active_config();
    if !cfg.is_on(winner) || !cfg.is_on(loser) {
        return;
    }
    PREEMPTED.with(|p| {
        if let Some(v) = p.borrow_mut().as_mut() {
            v.push((winner, loser));
        }
    });
}

/// Stop recording and take the deduplicated set of `(winner, loser)` precedence
/// decisions made during the trace (empty if no trace was active).
pub fn end_preempted_trace() -> Vec<(Opt, Opt)> {
    let mut edges = PREEMPTED.with(|p| p.borrow_mut().take().unwrap_or_default());
    edges.sort_by_key(|&(w, l)| (w as u8, l as u8));
    edges.dedup();
    edges
}

/// Run an AST pass, recording that `opt` fired iff the pass changed the program
/// — but only while a trace is active. The AST derives `Debug` (not `PartialEq`),
/// so a `{:?}` fingerprint is the change signal; it is taken only under a live
/// trace, so the normal compile path runs the pass exactly as before with no
/// extra cost. The pass runs exactly once either way (no behavior change).
#[inline]
pub(crate) fn run_traced<'a>(
    opt: Opt,
    input: Vec<Stmt<'a>>,
    pass: impl FnOnce(Vec<Stmt<'a>>) -> Vec<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    if !fired_trace_active() {
        return pass(input);
    }
    let before = format!("{input:?}");
    let out = pass(input);
    if format!("{out:?}") != before {
        mark_fired(opt);
    }
    out
}

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
    cfg: &OptimizationConfig,
) -> Vec<Stmt<'a>> {
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    if cfg.is_on(Opt::Specialize) {
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
    if cfg.is_on(Opt::Comptime) {
        current = ctfe::ctfe_stmts(current, expr_arena, stmt_arena, interner);
    }
    fold::fold_stmts(current, expr_arena, stmt_arena, interner)
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
    cfg: &OptimizationConfig,
) -> Vec<Stmt<'a>> {
    // The un-tiered entry point is `Tier::T3`: every admitted opt runs, so with an
    // all-on config this is today's whole-program pipeline, bit-for-bit (HOTSWAP §2).
    optimize_for_run_tiered(
        stmts,
        expr_arena,
        stmt_arena,
        interner,
        cfg,
        &HotswapConfig::default(),
        Tier::T3,
    )
}

/// How many partial-evaluation fixpoint iterations to pay for. `Tier::T3` runs the
/// full 16-iteration fixpoint (PE-full); `Tier::T2` runs a capped 4 (PE-light) to feed
/// the Medium passes folded/specialized code without the full tax. Below T2 the
/// Specialize pass is not admitted at all (it is `Medium`), so the cap is unused. A
/// `## Tier specialize <eager|t..>` pin brings PE forward AND gives it the full
/// fixpoint ("ignore its cost", HOTSWAP §8).
fn pe_iters(hs: &HotswapConfig, tier: Tier) -> usize {
    if matches!(hs.pin(Opt::Specialize), Some(Pin::Eager) | Some(Pin::At(_))) {
        return 16;
    }
    match tier {
        Tier::T0 | Tier::T1 => 0,
        Tier::T2 => 4,
        Tier::T3 => 16,
    }
}

/// The run-path optimizer, gated by the unit's hotness `tier` (HOTSWAP §4). Every
/// pass runs in the SAME order as the un-tiered pipeline; `tier` decides WHICH passes
/// run (via [`admits_pinned`], a single cost comparison, with `hs`'s per-opt pins
/// applied) and how many PE-fixpoint iterations to pay for. `Tier::T3` with an all-on
/// config and no pins reproduces [`optimize_for_run`] exactly — the compatibility +
/// soundness anchor.
pub fn optimize_for_run_tiered<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    cfg: &OptimizationConfig,
    hs: &HotswapConfig,
    tier: Tier,
) -> Vec<Stmt<'a>> {
    set_active_config(*cfg);
    clear_proven_raw_int_ops();
    if cfg.is_all_off() || tier == Tier::T0 {
        return stmts;
    }
    const MAX_RUN_OPT_STMTS: usize = 5_000;
    if stmts.len() > MAX_RUN_OPT_STMTS {
        return stmts;
    }
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    if admits_pinned(cfg, hs, tier, Opt::Inline) {
        // Tiny pure helpers inline first, so PE/fold/GVN/LICM see straight
        // arithmetic and the loop regions compile call-free.
        current = run_traced(Opt::Inline, current, |c| {
            inline_tiny::inline_tiny_fns(c, expr_arena, stmt_arena, interner)
        });
        // Statement-body leaf helpers (iterative gcd shape) fold in next, so
        // their calling loops compile as one call-free region.
        current = run_traced(Opt::Inline, current, |c| {
            inline_leaf::inline_leaf_fns(c, expr_arena, stmt_arena, interner)
        });
    }
    if admits_pinned(cfg, hs, tier, Opt::Specialize) {
        let mut variant_count: HashMap<Symbol, usize> = HashMap::new();
        let mut specialized_any = false;
        let cap = pe_iters(hs, tier);
        let mut capped_early = false;
        for i in 0..cap {
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
            specialized_any = true;
            // The fixpoint had not converged by the tier's iteration budget.
            capped_early = i + 1 == cap;
        }
        if specialized_any {
            mark_fired(Opt::Specialize);
        }
        // PE-light (T2) stops the fixpoint early, which can leave the residual not
        // fold-stable; the Medium passes below (loop-carried CSE, GVN, closed-form)
        // assume canonical sub-expressions. Re-normalize after a capped-early exit so
        // they fire on the same shape they would at T3. T3 runs to convergence (its
        // last iteration specialized 0 changes on folded+propagated input), so it is
        // already canonical and stays byte-for-byte today's pipeline.
        if capped_early && tier < Tier::T3 {
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    if admits_pinned(cfg, hs, tier, Opt::Comptime) {
        current = run_traced(Opt::Comptime, current, |c| {
            ctfe::ctfe_stmts(c, expr_arena, stmt_arena, interner)
        });
        current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
    }
    // Loop-carried CSE: hoist a squared term that a `While`'s escape guard
    // computes over the freshly assigned loop iterate and the next iteration's
    // body recomputes over the same value (mandelbrot's `zr*zr` / `zi*zi`) into a
    // fresh per-iteration temp — computed ONCE just after the operand's `Set`,
    // substituted into both the guard and the body recomputation. Runs after the
    // mask&1 PE fixpoint (so identical sub-expressions are already canonical) and
    // before GVN; fold+propagate refold the substituted forms. PROMOTED 2026-06-21:
    // default ON (kill-switch LOGOS_RUN_LCSE=0) — mandelbrot -14% on the faithful
    // interleaved A/B (2.40x->2.07x Node), 30/30 benchmarks bit-identical, no regress.
    if admits_pinned(cfg, hs, tier, Opt::LoopCse) {
        let (c, fired) =
            loop_carried_cse::loop_carried_cse_stmts(current, expr_arena, stmt_arena, interner);
        current = c;
        if fired {
            mark_fired(Opt::LoopCse);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    // U6: float induction-variable strength reduction — replace a per-iteration
    // `c1*k + c2` (a multiply + int->float cvtsi2sd) with a guarded incremental
    // float accumulator. PROMOTED 2026-06-21: default ON (kill-switch
    // LOGOS_RUN_FLOATSR=0) — pi_leibniz -31.7% on the faithful A/B (geomean
    // 0.891->0.879), bit-identical, no regression (the runtime trip-count guard
    // keeps it bit-identical for ALL inputs). Found via the stencil investigation.
    if admits_pinned(cfg, hs, tier, Opt::FloatStrength) {
        let (c, fired) = float_induction_sr::float_induction_sr_stmts(
            current, expr_arena, stmt_arena, interner,
        );
        current = c;
        if fired {
            mark_fired(Opt::FloatStrength);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    if admits_pinned(cfg, hs, tier, Opt::Cse) {
        current = run_traced(Opt::Cse, current, |c| {
            gvn::cse_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    // Run-path array scalarization (the interpreter's SROA): fully unroll the
    // constant-trip loops over a fixed-size float/int Seq, then replace the Seq
    // with N scalar locals so every `item k of arr` is a register-resident
    // variable read instead of a bounds-checked heap load. Unroll alone was a
    // measured loss (a bigger region over the same loads); the scalarize half is
    // what removes them. PROMOTED 2026-06-21: default ON (kill-switch
    // LOGOS_RUN_SCALARIZE=0) — nbody -17% on the faithful interleaved A/B,
    // 30/30 benchmarks bit-identical, no benchmark regressed.
    //
    // It runs AFTER GVN deliberately: GVN's value numbering is reload-correct on
    // the array form (it never CSEs an `item k of arr` read across a write), but
    // it would unsoundly CSE the loop-carried SCALAR reads this pass introduces
    // across the step loop's mutations. Sequencing scalarization after GVN keeps
    // GVN on the sound array form; the structural passes below (LICM, closed-form,
    // deforestation, interval analysis, DCE) are reload-correct on the scalar form.
    // Affine read-only array scalarization: delete a CSR-style offset array
    // (graph_bfs's `adjStarts`, built by one affine `Push i*5 to adjStarts`
    // loop) and substitute every `item k of arr` random heap load with the
    // closed form `5*(k-1)` — C's `v*5` shift. Runs on the ARRAY form, BEFORE
    // unroll/scalarize, so GVN (which already ran) stayed reload-correct over
    // the original reads. fold+propagate after it collapse the substituted
    // closed forms. PROMOTED 2026-06-21: default ON (kill-switch
    // LOGOS_RUN_AFFINE=0) — graph_bfs -20% on the faithful interleaved A/B
    // (2.86x->2.29x Node), 30/30 benchmarks bit-identical, no benchmark regressed.
    #[cfg(feature = "codegen")]
    if admits_pinned(cfg, hs, tier, Opt::Affine) {
        let (a, ca) =
            affine_scalarize::affine_scalarize_seqs(current, expr_arena, stmt_arena, interner);
        current = a;
        if ca {
            mark_fired(Opt::Affine);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    #[cfg(feature = "codegen")]
    {
        let mut changed = false;
        if admits_pinned(cfg, hs, tier, Opt::Unroll) {
            let (u, c1) = unroll::unroll_stmts_run(current, expr_arena, stmt_arena, interner);
            current = u;
            if c1 {
                mark_fired(Opt::Unroll);
            }
            changed |= c1;
        }
        if admits_pinned(cfg, hs, tier, Opt::Scalarize) {
            let (s, c2) = scalarize::scalarize_seqs(current, expr_arena, stmt_arena, interner);
            current = s;
            if c2 {
                mark_fired(Opt::Scalarize);
            }
            changed |= c2;
        }
        if changed {
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    if admits_pinned(cfg, hs, tier, Opt::LoopHoist) {
        current = run_traced(Opt::LoopHoist, current, |c| {
            licm::licm_stmts_run(c, expr_arena, stmt_arena, interner)
        });
    }
    if admits_pinned(cfg, hs, tier, Opt::ClosedForm) {
        current = run_traced(Opt::ClosedForm, current, |c| {
            closed_form::closed_form_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    if admits_pinned(cfg, hs, tier, Opt::Fuse) {
        current = run_traced(Opt::Fuse, current, |c| {
            deforest::deforest_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    if admits_pinned(cfg, hs, tier, Opt::Oracle) {
        current = run_traced(Opt::Oracle, current, |c| {
            abstract_interp::abstract_interp_stmts(c, expr_arena, stmt_arena)
        });
    }
    if admits_pinned(cfg, hs, tier, Opt::DeadCode) {
        current = run_traced(Opt::DeadCode, current, |c| {
            dce::eliminate_dead_code(c, stmt_arena, expr_arena)
        });
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
    if admits_pinned(cfg, hs, tier, Opt::Unfold) && current.len() <= MAX_RUN_INLINE_STMTS {
        let unrolled = run_traced(Opt::Unfold, current, |c| {
            inline_recursive::inline_recursive_fns_run(c, expr_arena, stmt_arena, interner)
        });
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
    cfg: &OptimizationConfig,
) -> Vec<Stmt<'a>> {
    set_active_config(*cfg);
    clear_proven_raw_int_ops();
    // De-desugar FIRST: under reference semantics the parser's place-write
    // Splices fuse back to the direct nested writes every downstream pass
    // (BTA, borrow-hoist, bounds elision) pattern-matches.
    let stmts = splice_fuse::fuse_place_splices(stmts, expr_arena, stmt_arena, interner);
    // NOTE: no early tiny-fn inliner here — that is a RUN-PATH device
    // (optimize_for_run skips supercompile for budget). This AOT pipeline
    // inlines through supercompile at the end exactly like v1, so the PE
    // machinery keeps its subjects.
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    if cfg.is_on(Opt::Specialize) {
        let mut variant_count: HashMap<Symbol, usize> = HashMap::new();
        let mut specialized_any = false;
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
            specialized_any = true;
        }
        if specialized_any {
            mark_fired(Opt::Specialize);
        }
    }
    if cfg.is_on(Opt::Comptime) {
        current = run_traced(Opt::Comptime, current, |c| {
            ctfe::ctfe_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
    // Reflection symmetry-breaking: halve a bitmask counting search's first-row
    // enumeration (×2 + odd-n middle) when the kernel proves the reflection
    // invariance for all n. Runs on the clean specialized entry, before the
    // structural passes; the sub-counts it calls still get popcount-leaf +
    // recursion unrolling later. Fail-closed (kernel certificate gates it).
    if cfg.is_on(Opt::Symmetry) {
        current = run_traced(Opt::Symmetry, current, |c| {
            symmetry::break_symmetry_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
    // Fully unroll small constant-trip loops nested inside hot loops (nbody's
    // per-step force loops), then refold/repropagate so the substituted indices
    // collapse to constants (`item (3-1) of bx` → `bx[2]`) and the dead counter
    // scaffolding is cleaned up. LLVM then SROAs the `[f64; N]` arrays into
    // registers and vectorizes the straight-line body — the codegen it already
    // produces for top-level constant loops but refuses inside a runtime loop.
    #[cfg(feature = "codegen")]
    if cfg.is_on(Opt::Unroll) {
        let (unrolled, changed) = unroll::unroll_stmts(current, expr_arena, stmt_arena, interner);
        current = unrolled;
        if changed {
            mark_fired(Opt::Unroll);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    // Group 4: direct closures lift to first-order functions BEFORE the
    // e-graph runs, so the Architect (and every later pass) sees plain
    // calls instead of heap closures.
    if cfg.is_on(Opt::Defunctionalize) {
        current = run_traced(Opt::Defunctionalize, current, |c| {
            defunctionalize::defunctionalize_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    // The Architect saturates every expression under kernel-certified
    // rewrites with Oracle-gated conditionals. GVN/LICM/closed-form STAY
    // until the e-graph's cross-statement runs (M13's SSA versioning)
    // subsume their capabilities — the structural suite pins them, and a
    // capability superset is the honest sprint-22 wiring.
    if cfg.is_on(Opt::Saturate) {
        current = run_traced(Opt::Saturate, current, |c| {
            egraph::convert::egraph_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    if cfg.is_on(Opt::Cse) {
        current = run_traced(Opt::Cse, current, |c| {
            gvn::cse_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    if cfg.is_on(Opt::LoopHoist) {
        current = run_traced(Opt::LoopHoist, current, |c| {
            licm::licm_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    if cfg.is_on(Opt::ClosedForm) {
        current = run_traced(Opt::ClosedForm, current, |c| {
            closed_form::closed_form_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    if cfg.is_on(Opt::Fuse) {
        current = run_traced(Opt::Fuse, current, |c| {
            deforest::deforest_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
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
        current = run_traced(Opt::LoopSplit, current, |c| {
            loop_split::loop_split_stmts(c, expr_arena, stmt_arena, interner, cfg)
        });
        current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
        current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
    }
    if cfg.is_on(Opt::Oracle) {
        current = run_traced(Opt::Oracle, current, |c| {
            abstract_interp::abstract_interp_stmts(c, expr_arena, stmt_arena)
        });
    }
    let dced = if cfg.is_on(Opt::DeadCode) {
        run_traced(Opt::DeadCode, current, |c| {
            dce::eliminate_dead_code(c, stmt_arena, expr_arena)
        })
    } else {
        current
    };
    let current = if cfg.is_on(Opt::Supercompile) {
        run_traced(Opt::Supercompile, dced, |c| {
            supercompile::supercompile_stmts(c, expr_arena, stmt_arena, interner)
        })
    } else {
        dced
    };
    // Popcount-leaf: collapse the second-to-last level of a bitmask counting
    // search (`row == n-1`, every remaining bit is one solution) into a single
    // `count_ones`. Runs BEFORE inline_recursive so the fast path is carried
    // into every unrolled clone. Pure structural rewrite (no kernel).
    let current = run_traced(Opt::Popcount, current, |c| {
        popcount_leaf::popcount_leaf_stmts(c, expr_arena, stmt_arena, interner, cfg)
    });
    // Bounded recursive inlining (recursion unrolling) runs LAST: flatten each
    // self-recursive function's own body k levels deep — the inline LLVM
    // refuses but gcc -O3 performs (the lever that lets compiled-LOGOS
    // n-queens match/beat C). Placed after the heavy analyses (e-graph,
    // supercompile) so they optimize the ORIGINAL body once and never pay to
    // re-analyze the k enlarged copies — each clone is already optimized.
    // fold+propagate+DCE then collapse the argument-binding temps the splice
    // introduces (`__rN_n = n`).
    let unrolled = if cfg.is_on(Opt::Unfold) {
        run_traced(Opt::Unfold, current, |c| {
            inline_recursive::inline_recursive_fns(c, expr_arena, stmt_arena, interner)
        })
    } else {
        current
    };
    let folded = fold::fold_stmts(unrolled, expr_arena, stmt_arena, interner);
    let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
    // O9 — bound-versioned loop nests (spectral_norm's vectorization): runs
    // LAST among the rewriters so nothing rebuilds its proven-raw-marked
    // clone nodes before codegen. Versioning is guard-based loop cloning, the
    // LoopSplit family; AOT-only like its sibling (the payoff is SIMD).
    #[cfg(feature = "codegen")]
    let propagated = if cfg.is_on(Opt::LoopSplit) {
        run_traced(Opt::LoopSplit, propagated, |c| {
            bound_version::bound_version_stmts(c, expr_arena, stmt_arena)
        })
    } else {
        propagated
    };
    if cfg.is_on(Opt::DeadCode) {
        run_traced(Opt::DeadCode, propagated, |c| {
            dce::eliminate_dead_code(c, stmt_arena, expr_arena)
        })
    } else {
        propagated
    }
}
