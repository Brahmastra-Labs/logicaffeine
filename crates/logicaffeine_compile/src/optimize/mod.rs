mod dce;
mod defunctionalize;
pub(crate) mod dependence;
pub(crate) mod facts;
mod fold;
mod gvn;
mod inline_leaf;
mod inline_recursive;
mod inline_tiny;
pub(crate) mod iteration;
pub(crate) mod item_growth;
pub(crate) mod kinds;
mod popcount_leaf;
mod propagate;
pub(crate) mod purity;
mod sccp;
pub(crate) mod sequence;
mod splice_fuse;
mod symmetry;
pub(crate) mod version;
pub(crate) mod writes;
pub(crate) use writes::WriteSet;
pub(crate) mod mentions;
pub(crate) mod modwrap;
pub(crate) use mentions::{expr_touches, stmt_touches, use_counts, Mentions};
mod swap_classes;
pub(crate) use swap_classes::{swapped_pairs, SwapClasses};
mod accum_bound;
mod bound_version;
pub(crate) mod capture;
mod closed_form;
mod ctfe;
mod divmod_fuse;
mod float_induction_sr;
mod int_induction_sr;
mod licm;
mod loop_carried_cse;
mod loop_interchange;
mod memoize_run;
mod reg_promote;
mod specialize_loop;
pub(crate) use ctfe::ascii_class_masks;
mod deforest;
mod ast_build;
mod divisor_sieve;
mod fold_fusion;
mod sort_nest;
mod trace_memo;
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
mod abstract_interp;
mod affine;
mod karr;
#[cfg(feature = "codegen")]
mod loop_split;
pub use abstract_interp::{
    lin_to_rust, oracle_analyze, oracle_analyze_with, oracle_analyze_with_entry_guards,
    OracleFacts, ScalarKind, VarProvenFacts,
};
pub mod bta;
pub mod core_builtins;
pub mod core_defaults;
pub mod core_formats;
pub mod core_operators;
pub mod memo_keys;
pub mod mix_bta;
pub mod mix_lift_types;
pub mod effects;
pub mod egraph;
pub mod partial_eval;
pub mod supercompile;

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};
use crate::optimization::{
    admits_pinned, BackendOptimizationTrace, FiredOptimizations, HotswapConfig, Opt,
    OptimizationBackend, OptimizationConfig, OptimizationDecline, OptimizationFallback, Pin, Tier,
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

/// Whether `a` and `b` are the same expression, written alike: one literal, one name, one operator over the same
/// operands, one read of the same item. Syntax only: whether the two evaluate alike where they stand is the
/// caller's question, since a write between them can change what a read sees.
pub(crate) fn same_expr(a: &crate::ast::stmt::Expr, b: &crate::ast::stmt::Expr) -> bool {
    use crate::ast::stmt::Expr;
    match (a, b) {
        (Expr::Literal(x), Expr::Literal(y)) => x == y,
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        (
            Expr::BinaryOp {
                op: o1,
                left: l1,
                right: r1,
            },
            Expr::BinaryOp {
                op: o2,
                left: l2,
                right: r2,
            },
        ) => o1 == o2 && same_expr(l1, l2) && same_expr(r1, r2),
        (Expr::Not { operand: x }, Expr::Not { operand: y }) => same_expr(x, y),
        (
            Expr::Index {
                collection: c1,
                index: i1,
            },
            Expr::Index {
                collection: c2,
                index: i2,
            },
        ) => same_expr(c1, c2) && same_expr(i1, i2),
        (Expr::Length { collection: x }, Expr::Length { collection: y }) => same_expr(x, y),
        _ => false,
    }
}

thread_local! {
    /// The symbols of the clock reads ([`ClockRead`](crate::semantics::temporal::ClockRead)) in the program being
    /// optimized. A clock read is no variable: each read reads the clock afresh, so no pass may equate two reads,
    /// forward a binding of one to a later read, or move one. Installed at optimizer entry (symbols are reused
    /// across programs on a thread).
    static CLOCK_READS: std::cell::RefCell<std::collections::HashSet<crate::intern::Symbol>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Record the clock reads' symbols for the program `interner` holds (call at optimizer entry).
pub(crate) fn install_clock_reads(interner: &Interner) {
    let reads = crate::semantics::temporal::ClockRead::ALL
        .into_iter()
        .filter_map(|read| interner.lookup(read.name()))
        .collect();
    CLOCK_READS.with(|s| *s.borrow_mut() = reads);
}

/// Whether `sym` is a clock read of the program being optimized.
pub(crate) fn is_clock_read(sym: crate::intern::Symbol) -> bool {
    CLOCK_READS.with(|s| s.borrow().contains(&sym))
}

thread_local! {
    /// The functions defunctionalization lifted out of closures: function values the program wrote, whose
    /// integers every tier keeps exact as it keeps a closure's. Cleared at optimizer entry (symbols are reused
    /// across programs on a thread).
    static LIFTED_CLOSURES: std::cell::RefCell<std::collections::HashSet<crate::intern::Symbol>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Forget the lifted closures (call at optimizer entry).
pub(crate) fn clear_lifted_closures() {
    LIFTED_CLOSURES.with(|s| s.borrow_mut().clear());
}

/// Record `f` as a function lifted out of a closure.
pub(crate) fn mark_lifted_closure(f: crate::intern::Symbol) {
    LIFTED_CLOSURES.with(|s| {
        s.borrow_mut().insert(f);
    });
}

/// Was `f` lifted out of a closure by defunctionalization?
pub fn is_lifted_closure(f: crate::intern::Symbol) -> bool {
    LIFTED_CLOSURES.with(|s| s.borrow().contains(&f))
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
    static FIRED: std::cell::Cell<Option<crate::optimization::OptBits>> = const { std::cell::Cell::new(None) };

    /// The PRECEDENCE decisions recorded during the current trace: each
    /// `(winner, loser)` pair is a spot where the code chose `winner` and skipped
    /// `loser` for an instance (the conflict the optimization SYSTEM resolves).
    /// `None` when no trace is active, so [`mark_preempted`] is then a no-op and
    /// the normal compile path pays nothing. Shares the trace lifetime with
    /// [`FIRED`] — both reset by [`begin_fired_trace`].
    static PREEMPTED: std::cell::RefCell<Option<Vec<(Opt, Opt)>>> =
        const { std::cell::RefCell::new(None) };

    /// Stage-separated evidence layered over the legacy union above. These are
    /// populated only by `begin_backend_trace`; ordinary compilation and the
    /// original fired-trace API retain their previous single-bitset behavior.
    static TRACE_BACKEND: std::cell::Cell<Option<OptimizationBackend>> =
        const { std::cell::Cell::new(None) };
    static TRACE_STAGE: std::cell::Cell<TraceStage> =
        const { std::cell::Cell::new(TraceStage::Optimizer) };
    static OPTIMIZER_FIRED: std::cell::Cell<Option<crate::optimization::OptBits>> =
        const { std::cell::Cell::new(None) };
    static LOWERING_FIRED: std::cell::Cell<Option<crate::optimization::OptBits>> =
        const { std::cell::Cell::new(None) };
    static TRACE_DECLINES: std::cell::RefCell<Option<Vec<OptimizationDecline>>> =
        const { std::cell::RefCell::new(None) };
    static TRACE_FALLBACKS: std::cell::RefCell<Option<Vec<OptimizationFallback>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Clone, Copy)]
enum TraceStage {
    Optimizer,
    Lowering,
}

/// Begin recording which optimizations fire AND which they preempt for the
/// current compile. Resets both accumulators; pair with [`end_fired_trace`] /
/// [`end_preempted_trace`].
pub fn begin_fired_trace() {
    FIRED.with(|f| f.set(Some(crate::optimization::OptBits::EMPTY)));
    PREEMPTED.with(|p| *p.borrow_mut() = Some(Vec::new()));
}

/// Begin a stage-separated trace for one concrete backend. The legacy `FIRED`
/// union is opened too, preserving `## Require` and existing tooling exactly.
pub fn begin_backend_trace(backend: OptimizationBackend) {
    begin_fired_trace();
    TRACE_BACKEND.with(|b| b.set(Some(backend)));
    TRACE_STAGE.with(|s| s.set(TraceStage::Optimizer));
    OPTIMIZER_FIRED.with(|f| f.set(Some(crate::optimization::OptBits::EMPTY)));
    LOWERING_FIRED.with(|f| f.set(Some(crate::optimization::OptBits::EMPTY)));
    TRACE_DECLINES.with(|f| *f.borrow_mut() = Some(Vec::new()));
    TRACE_FALLBACKS.with(|f| *f.borrow_mut() = Some(Vec::new()));
}

/// Close a stage-separated trace. Always clears every accumulator, including
/// after a failed compile, so a later compile on the same thread starts clean.
pub fn end_backend_trace() -> Option<BackendOptimizationTrace> {
    let backend = TRACE_BACKEND.with(|b| b.take());
    let optimizer = OPTIMIZER_FIRED.with(|f| f.take().map(FiredOptimizations::from_bits));
    let lowering = LOWERING_FIRED.with(|f| f.take().map(FiredOptimizations::from_bits));
    let declines = TRACE_DECLINES.with(|f| f.borrow_mut().take().unwrap_or_default());
    let fallbacks = TRACE_FALLBACKS.with(|f| f.borrow_mut().take().unwrap_or_default());
    TRACE_STAGE.with(|s| s.set(TraceStage::Optimizer));
    let _ = end_fired_trace();
    let _ = end_preempted_trace();
    backend.map(|backend| BackendOptimizationTrace {
        backend,
        optimizer: optimizer.unwrap_or_default(),
        lowering: lowering.unwrap_or_default(),
        declines,
        fallbacks,
    })
}

struct TraceStageGuard(TraceStage);

impl Drop for TraceStageGuard {
    fn drop(&mut self) {
        TRACE_STAGE.with(|s| s.set(self.0));
    }
}

/// Attribute every `mark_fired` in `f` to backend lowering rather than the
/// shared optimizer. Outside a backend trace this is a zero-behavior wrapper.
pub fn with_backend_lowering<T>(f: impl FnOnce() -> T) -> T {
    let previous = TRACE_STAGE.with(|s| {
        let previous = s.get();
        if TRACE_BACKEND.with(|b| b.get().is_some()) {
            s.set(TraceStage::Lowering);
        }
        previous
    });
    let _guard = TraceStageGuard(previous);
    f()
}

/// Record an explicit backend fallback. Outside a backend trace this is a
/// no-op, so production compilation pays neither allocation nor formatting.
pub fn mark_backend_fallback(to: OptimizationBackend, reason: impl Into<String>) {
    let Some(from) = TRACE_BACKEND.with(|b| b.get()) else {
        return;
    };
    TRACE_FALLBACKS.with(|f| {
        if let Some(fallbacks) = f.borrow_mut().as_mut() {
            fallbacks.push(OptimizationFallback {
                from,
                to,
                reason: reason.into(),
            });
        }
    });
}

/// Record that the current backend deliberately declined one optimization.
/// Outside a backend trace this is a no-op. Duplicate decisions are collapsed
/// so a loop containing many equivalent candidates produces one audit reason.
pub fn mark_backend_decline(opt: Opt, reason: impl Into<String>) {
    let Some(backend) = TRACE_BACKEND.with(|b| b.get()) else {
        return;
    };
    let reason = reason.into();
    TRACE_DECLINES.with(|f| {
        if let Some(declines) = f.borrow_mut().as_mut() {
            if !declines
                .iter()
                .any(|d| d.backend == backend && d.opt == opt && d.reason == reason)
            {
                declines.push(OptimizationDecline {
                    backend,
                    opt,
                    reason,
                });
            }
        }
    });
}

/// Stop recording and take the fired set (`None` if no trace was active).
pub fn end_fired_trace() -> Option<FiredOptimizations> {
    FIRED.with(|f| f.take().map(FiredOptimizations::from_bits))
}

/// Whether a firing trace is currently active. Gates the cost of the
/// before/after fingerprint in `run_traced`.
#[inline]
pub fn fired_trace_active() -> bool {
    FIRED.with(|f| f.get().is_some())
}

/// Record that `opt` fired in the current compile. No-op when no trace is active.
#[inline]
pub fn mark_fired(opt: Opt) {
    FIRED.with(|f| {
        if let Some(bits) = f.get() {
            f.set(Some(bits.with(opt)));
        }
    });
    if TRACE_BACKEND.with(|b| b.get().is_some()) {
        TRACE_STAGE.with(|stage| match stage.get() {
            TraceStage::Optimizer => OPTIMIZER_FIRED.with(|f| {
                if let Some(bits) = f.get() {
                    f.set(Some(bits.with(opt)));
                }
            }),
            TraceStage::Lowering => LOWERING_FIRED.with(|f| {
                if let Some(bits) = f.get() {
                    f.set(Some(bits.with(opt)));
                }
            }),
        });
    }
}

/// The file-level optimization-decorator contract (`## No <opt>` / `## Require <opt>`) in ONE
/// place, so no compile/run/emit tier can silently drop it — the drift that let the decorators
/// be ignored on the C, VM, wasm and isa paths. Every tier does the same two steps:
/// `install_decorators(...)` at scope entry, then `gate.verify()?` after compilation.
#[must_use = "a DecoratorGate must be `.verify()`-ed after codegen so `## Require` is enforced"]
pub struct DecoratorGate {
    required: Vec<Opt>,
    owns_trace: bool,
}

/// Merge the file's `## No` flags into `env`, install the result as the thread-local active
/// config (so EVERY backend's `active_config().is_on(..)` gate honors them — AOT codegen, the
/// VM compiler, the JIT), and open a fired-trace when any `## Require` was declared. Returns
/// the merged config to hand to the optimizer/codegen plus the gate to `verify()` after the
/// compile has run. When a trace is already active (a nested compile), the outer gate owns the
/// verification, so this one is a no-op `verify()`.
pub fn install_decorators(
    env: OptimizationConfig,
    flags: &OptimizationConfig,
    required: Vec<Opt>,
) -> (OptimizationConfig, DecoratorGate) {
    let mut cfg = env.merged(flags);
    cfg.normalize();
    set_active_config(cfg);
    let owns_trace = !required.is_empty() && !fired_trace_active();
    if owns_trace {
        begin_fired_trace();
    }
    (
        cfg,
        DecoratorGate {
            required,
            owns_trace,
        },
    )
}

impl DecoratorGate {
    /// Open a fired-trace for `required` (when non-empty and no trace is already active) WITHOUT
    /// touching the active config — for the run/VM/wasm/isa seam, where `## No` was already
    /// installed upstream (`parse_seam`) and only `## Require` remains to enforce. Pair with
    /// [`verify`](Self::verify) (checks) or [`close`](Self::close) (discards on a prior error).
    pub fn for_required(required: Vec<Opt>) -> DecoratorGate {
        let owns_trace = !required.is_empty() && !fired_trace_active();
        if owns_trace {
            begin_fired_trace();
        }
        DecoratorGate {
            required,
            owns_trace,
        }
    }

    /// End the trace this gate owns without checking `## Require` — used when compilation already
    /// failed so its own error propagates instead of a spurious `## Require` message.
    pub fn close(self) {
        if self.owns_trace {
            let _ = end_fired_trace();
        }
    }

    /// Close the trace this gate owns and fail loudly on any `## Require <opt>` whose
    /// optimization did not fire for the program — the honest answer when a backend cannot
    /// provide a demanded optimization (e.g. `## Require memo` under the VM/wasm tier, where
    /// memoization is an AOT-codegen construct). Returns the ready-to-surface error message.
    pub fn verify(self) -> Result<(), String> {
        if !self.owns_trace {
            return Ok(());
        }
        let fired = end_fired_trace().unwrap_or_default();
        for opt in &self.required {
            if !fired.fired(*opt) {
                let kw = opt.meta().keyword;
                return Err(format!(
                    "`## Require {kw}` was declared, but the `{kw}` optimization did not \
                     fire for this program — the optimizer found nowhere to apply it. \
                     Rewrite so `{kw}` has something to optimize, or remove the `## Require {kw}`."
                ));
            }
        }
        Ok(())
    }
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
    crate::analysis::callgraph::install_lending_parameters(&stmts);
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    if cfg.is_on(Opt::Specialize) {
        let mut variant_count: HashMap<Symbol, usize> = HashMap::new();
        for _ in 0..16 {
            let folded = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
            let (specialized, changes) = partial_eval::specialize_stmts_with_state(
                propagated,
                expr_arena,
                stmt_arena,
                interner,
                &mut variant_count,
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
        current = ctfe::tableize_stmts(current, expr_arena, stmt_arena, interner);
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
    clear_lifted_closures();
    install_clock_reads(interner);
    crate::analysis::callgraph::install_lending_parameters(&stmts);
    if cfg.is_all_off() || tier == Tier::T0 {
        return stmts;
    }
    const MAX_RUN_OPT_STMTS: usize = 5_000;
    if stmts.len() > MAX_RUN_OPT_STMTS {
        return stmts;
    }
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    // Range-specialize helper calls before the general inliners erase the
    // counted-loop context that proves their arguments non-negative. This is
    // a backend-neutral AST rewrite: the run/VM/JIT family benefits from the
    // same call-free arithmetic residual as AOT, while the tier/cost gate keeps
    // it out of cold units.
    #[cfg(feature = "codegen")]
    if admits_pinned(cfg, hs, tier, Opt::SpecializeLoop) {
        current = run_traced(Opt::SpecializeLoop, current, |c| {
            specialize_loop::specialize_loop_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
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
    // Sparse conditional constant propagation before the PE fixpoint: prune
    // dead branches so specialization/fold/GVN see the smaller, no-poison
    // residual (the branch a proven-constant guard can never take is gone, and
    // its `Set`s can no longer force a variable to ⊥).
    if admits_pinned(cfg, hs, tier, Opt::Sccp) {
        current = run_traced(Opt::Sccp, current, |c| {
            sccp::sccp_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    // Loop interchange on the RAW affine nest — before specialization/IntStrength
    // linearize the indices — reorders an integer commutative-reduction nest to
    // put the unit-stride access innermost. Bit-identical (integer +/% reduction
    // is order-invariant) and only toward better locality.
    if admits_pinned(cfg, hs, tier, Opt::Interchange) {
        let (c, fired) =
            loop_interchange::interchange_stmts(current, expr_arena, stmt_arena, interner);
        current = c;
        if fired {
            mark_fired(Opt::Interchange);
        }
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
                propagated,
                expr_arena,
                stmt_arena,
                interner,
                &mut variant_count,
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
            let c = ctfe::ctfe_stmts(c, expr_arena, stmt_arena, interner);
            ctfe::tableize_stmts(c, expr_arena, stmt_arena, interner)
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
        let (c, fired) =
            float_induction_sr::float_induction_sr_stmts(current, expr_arena, stmt_arena, interner);
        current = c;
        if fired {
            mark_fired(Opt::FloatStrength);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    // Integer induction strength reduction: a counted loop's affine index
    // `coeff*j + base` (base loop-invariant) becomes a seeded accumulator
    // advanced by `coeff*step`, removing the per-iteration multiply. Runs after
    // float strength reduction (its integer sibling) and BEFORE GVN, so the
    // linearized single-identifier indices are what GVN value-numbers. Exact —
    // no runtime guard (integer arithmetic promotes to BigInt identically).
    if admits_pinned(cfg, hs, tier, Opt::IntStrength) {
        // Run path: no codegen tiler downstream, so reduce every affine index
        // (the interpreter has no cache-tiling to defer to).
        let (c, fired) = int_induction_sr::int_induction_sr_stmts(
            current, expr_arena, stmt_arena, interner, false,
        );
        current = c;
        if fired {
            mark_fired(Opt::IntStrength);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
    // Register-promote an invariant-indexed RMW element (store-to-load
    // forwarding across the loop) BEFORE GVN, so GVN never restructures the
    // `item IDX of C` read/write pair this pass recognizes. Its `loads_equal`
    // alias check benefits from IntStrength having just linearized the index.
    if admits_pinned(cfg, hs, tier, Opt::RegPromote) {
        let (c, fired) = reg_promote::reg_promote_stmts(current, expr_arena, stmt_arena, interner);
        current = c;
        if fired {
            mark_fired(Opt::RegPromote);
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
    // Run-path memoization: cache a pure single-arg self-recursive function in a
    // module-level `Map` global (the interpreter/VM analog of the AOT thread-local
    // cache). AFTER ClosedForm, so a function ClosedForm already solved has no
    // self-calls left and this declines — the natural ClosedForm-preempts-Memo.
    if admits_pinned(cfg, hs, tier, Opt::Memo) {
        let (c, fired) = memoize_run::memoize_run_stmts(current, expr_arena, stmt_arena, interner);
        current = c;
        if fired {
            mark_fired(Opt::Memo);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
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
/// Memoize the pure single-argument self-recursive functions of an already-optimized program for a
/// lowering that has no native memoization construct. Rust emits its own cache during codegen; direct C
/// and bytecode-derived backends instead consume this explicit Map-based residual. Keeping the transform
/// here makes `Memo` one optimization with one eligibility rule and firing trace across those backends.
pub fn memoize_for_explicit_lowering<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    cfg: &OptimizationConfig,
) -> Vec<Stmt<'a>> {
    if !cfg.is_on(Opt::Memo) {
        return stmts;
    }
    let (current, fired) = memoize_run::memoize_run_stmts(stmts, expr_arena, stmt_arena, interner);
    if !fired {
        return current;
    }
    mark_fired(Opt::Memo);
    let current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
    propagate::propagate_stmts(current, expr_arena, stmt_arena, interner)
}

pub fn optimize_program<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    cfg: &OptimizationConfig,
) -> Vec<Stmt<'a>> {
    set_active_config(*cfg);
    clear_lifted_closures();
    install_clock_reads(interner);
    crate::analysis::callgraph::install_lending_parameters(&stmts);
    // De-desugar FIRST: under reference semantics the parser's place-write
    // Splices fuse back to the direct nested writes every downstream pass
    // (BTA, borrow-hoist, bounds elision) pattern-matches.
    let stmts = splice_fuse::fuse_place_splices(stmts, expr_arena, stmt_arena, interner);
    // Fold/build fusion on the program as written: a consumer of a datatype applied to what a builder builds of it
    // compiles to one function that never builds it, before any pass inlines or specializes either function.
    let stmts = if cfg.is_on(Opt::FoldFusion) {
        let (fused, fired) =
            fold_fusion::fold_fusion_stmts(stmts, expr_arena, stmt_arena, interner);
        if fired {
            mark_fired(Opt::FoldFusion);
        }
        fused
    } else {
        stmts
    };
    // A bubble sort's nest, before any loop pass reshapes it, sorts its window by one heap sort behind a guard.
    let stmts = if cfg.is_on(Opt::SortNest) {
        let (sorted, fired) = sort_nest::sort_nest_stmts(stmts, expr_arena, stmt_arena, interner);
        if fired {
            mark_fired(Opt::SortNest);
        }
        sorted
    } else {
        stmts
    };
    // A per-number search for a divisor up to its square root reads one sieve of the range, behind a size guard.
    let stmts = if cfg.is_on(Opt::DivisorSieve) {
        let (sieved, fired) =
            divisor_sieve::divisor_sieve_stmts(stmts, expr_arena, stmt_arena, interner);
        if fired {
            mark_fired(Opt::DivisorSieve);
        }
        sieved
    } else {
        stmts
    };
    // A pure iteration run from every start of a range stops where it reaches a start already finished.
    let stmts = if cfg.is_on(Opt::TraceMemo) {
        let (memo, fired) = trace_memo::trace_memo_stmts(stmts, expr_arena, stmt_arena, interner);
        if fired {
            mark_fired(Opt::TraceMemo);
        }
        memo
    } else {
        stmts
    };
    // NOTE: no early tiny-fn inliner here — that is a RUN-PATH device
    // (optimize_for_run skips supercompile for budget). This AOT pipeline
    // inlines through supercompile at the end exactly like v1, so the PE
    // machinery keeps its subjects.
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    // Loop interchange on the raw affine nest, before the e-graph / IntStrength
    // touch the indices: reorder an integer commutative-reduction nest toward
    // the unit-stride-innermost order. Bit-identical and locality-improving only.
    if cfg.is_on(Opt::Interchange) {
        let (c, fired) =
            loop_interchange::interchange_stmts(current, expr_arena, stmt_arena, interner);
        current = c;
        if fired {
            mark_fired(Opt::Interchange);
        }
    }
    if cfg.is_on(Opt::Specialize) {
        let mut variant_count: HashMap<Symbol, usize> = HashMap::new();
        let mut specialized_any = false;
        for _ in 0..16 {
            let folded = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
            let (specialized, changes) = partial_eval::specialize_stmts_with_state(
                propagated,
                expr_arena,
                stmt_arena,
                interner,
                &mut variant_count,
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
            let c = ctfe::ctfe_stmts(c, expr_arena, stmt_arena, interner);
            ctfe::tableize_stmts(c, expr_arena, stmt_arena, interner)
        });
    }
    current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
    // Sparse conditional constant propagation: fold constants and prune
    // unreachable branches in one lattice sweep — the reachability half the
    // e-graph does not do. Runs on the folded PE residual, BEFORE the Architect,
    // so a dead branch (and any `Set` inside it that would otherwise poison a
    // variable) is gone before saturation / GVN / the recurrence passes see it.
    if cfg.is_on(Opt::Sccp) {
        current = run_traced(Opt::Sccp, current, |c| {
            sccp::sccp_stmts(c, expr_arena, stmt_arena, interner)
        });
        current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
        current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
    }
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
    // Register-promote an invariant-indexed RMW element BEFORE the Architect:
    // the e-graph canonicalizes the `item IDX of C` read/write pair out of the
    // shape this pass recognizes, so it must run on the pre-saturation form.
    if cfg.is_on(Opt::RegPromote) {
        let (c, fired) = reg_promote::reg_promote_stmts(current, expr_arena, stmt_arena, interner);
        current = c;
        if fired {
            mark_fired(Opt::RegPromote);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
    }
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
    // Integer induction strength reduction on the post-LICM index arithmetic:
    // what LICM hoisted as invariant is gone, leaving the `coeff*j + base` form
    // this pass folds into an accumulator. Before ClosedForm so its accumulator
    // recurrences are visible to the recurrence solver.
    if cfg.is_on(Opt::IntStrength) {
        // AOT/Rust-codegen path: defer the square matmul nest to the codegen
        // tiler (`peephole::try_emit_tiled_inner`), whose L1 cache tiling beats
        // removing the index multiply and needs the nest's exact raw shape.
        let (c, fired) = int_induction_sr::int_induction_sr_stmts(
            current, expr_arena, stmt_arena, interner, true,
        );
        current = c;
        if fired {
            mark_fired(Opt::IntStrength);
            current = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
            current = propagate::propagate_stmts(current, expr_arena, stmt_arena, interner);
        }
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
    // Range-aware helper specialization: symbolically inline pure integer helper calls
    // (`divModQ`/`divModM`/…) in a counted-accumulator loop under the counter's proven
    // `>= 0` range and fold the dead sign branches, turning the function-based term into
    // bare arithmetic. Runs FIRST so its inlined output flows through the accumulator
    // versioner (guard elision) and div-mod fusion below.
    #[cfg(feature = "codegen")]
    let propagated = if cfg.is_on(Opt::SpecializeLoop) {
        run_traced(Opt::SpecializeLoop, propagated, |c| {
            specialize_loop::specialize_loop_stmts(c, expr_arena, stmt_arena, interner)
        })
    } else {
        propagated
    };
    // Pass 1 — bounded-accumulator overflow-guard elision: proves a hot integer loop
    // stays i64-safe once its inputs fit a compile-time bound, versions it, and marks
    // the fast branch's arithmetic proven-raw so codegen drops the per-iteration
    // overflow guard. Runs BEFORE bound_version so the tighter accumulator bound
    // (`n <= 2e9`, marking the GROWING accumulator too) wins over bound_version's
    // coarser counter-only split (`n <= 2^62`, which cannot bound a growing sum and so
    // leaves its per-iteration guard); bound_version then skips the loop accum_bound
    // already versioned (its body arith is all proven-raw) and only splits the fallback.
    #[cfg(feature = "codegen")]
    let propagated = if cfg.is_on(Opt::AccumBound) {
        run_traced(Opt::AccumBound, propagated, |c| {
            accum_bound::accum_bound_stmts(c, expr_arena, stmt_arena, interner)
        })
    } else {
        propagated
    };
    #[cfg(feature = "codegen")]
    let propagated = if cfg.is_on(Opt::LoopSplit) {
        run_traced(Opt::LoopSplit, propagated, |c| {
            bound_version::bound_version_stmts(c, expr_arena, stmt_arena)
        })
    } else {
        propagated
    };
    // Div-mod fusion: restore the `a/b`+`a%b` CSE that rustc/LLVM won't do — bind the shared
    // quotient once and derive the remainder. Runs LAST among the rewriters (after the bounded-
    // loop versioners have marked the accumulator fast branch proven-raw) so its rewritten nodes
    // inherit those marks and stay bare-i64; AOT-only.
    #[cfg(feature = "codegen")]
    let propagated = if cfg.is_on(Opt::DivModFuse) {
        run_traced(Opt::DivModFuse, propagated, |c| {
            divmod_fuse::divmod_fuse_stmts(c, expr_arena, stmt_arena, interner)
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
