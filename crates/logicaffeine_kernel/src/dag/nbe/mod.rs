//! Normalization by evaluation: the kernel's conversion engine over closures instead of
//! substitution.
//!
//! The classic checker ([`crate::dag::tc`]) decides `t ≡ s` by weak-head reducing *terms*:
//! every β-step substitutes an argument into a body, building a new term, and every δ-step
//! copies a definition's body with its universe parameters replaced. This engine evaluates both
//! sides into [`value::Val`]s instead — β extends an environment, δ evaluates a body under a
//! universe substitution, arguments are thunks forced at most once — and compares values.
//! The comparison is Lean's algorithm (lazy δ by definitional height, proof irrelevance, η,
//! struct-η, unit-like types, K, quotients, literals), so it answers exactly what the classic
//! engine answers; `tests/dag_nbe_*` hold the two to that on every Arena input.
//!
//! [`State`] is one declaration's arena and caches; [`Nbe`] pairs it with the kernel.

pub mod check;
pub mod conv;
pub mod eval;
pub mod quote;
pub mod types;
pub mod value;
pub mod whnf;

use super::error::{KResult, KernelFailure};
use super::expr::ExprId;
use super::level::LevelsId;
use super::name::NameId;
use super::tc::Kernel;
use rustc_hash::{FxHashMap as FxMap, FxHashSet as FxSet};
use value::{Arena, VId};

/// Recursion bound for eval/whnf/conv; the checker thread's stack is sized for it.
const MAX_DEPTH: u32 = 200_000;

/// One declaration's values and caches.
#[derive(Clone, Default)]
pub struct State {
    pub arena: Arena,
    const_val: FxMap<(NameId, LevelsId), VId>,
    const_ty: FxMap<(NameId, LevelsId), VId>,
    const_body: FxMap<(NameId, LevelsId), VId>,
    eval_cache: FxMap<(value::EnvId, ExprId), VId>,
    whnf_cache: FxMap<VId, VId>,
    whnf_core_cache: FxMap<VId, VId>,
    type_cache: FxMap<VId, VId>,
    /// The checker's types: of an expression, by the environment and the context it can see.
    check_cache: FxMap<(value::EnvId, value::CtxId, ExprId), VId>,
    /// Pairs proven equal. A proof answers every later question about its pair, and about any
    /// pair built from proven parts, in either `eagerReduce` mode — as Lean's equivalence manager
    /// answers them.
    proven: FxSet<(VId, VId)>,
    /// How many pairs have been proven: what a remembered refutation is stamped with.
    proven_generation: u64,
    /// Pairs refuted, each with the `eagerReduce` mode it was refuted in and the proof generation
    /// when its comparison began. A refutation answers only a question asked in the same mode —
    /// the other mode can decide the pair differently (Arena `bugs/rec-of-subst-prop` is built on
    /// a pair that is equal without `eagerReduce` and unequal under it) — with nothing proven
    /// since: a new proof can make the pair equal, by congruence or through a sub-comparison.
    refuted: FxMap<(VId, VId, bool), u64>,
    locals_cache: FxMap<VId, bool>,
    failure: FxSet<(VId, VId)>,
    depth: u32,
    /// Conversion steps taken since the state was created (entries into the core of the
    /// comparison): what conversion has cost, independent of the machine.
    conv_steps: u64,
    /// The binder depth: the depth the next local is bound at.
    level: u32,
    /// Lean's `eagerReduce`: while checking an argument marked for eager reduction, literal
    /// arithmetic and the `≡ true` shortcut are attempted even on values with variables. Part of
    /// every conversion question: the same pair can be equal in one mode and not in the other.
    pub eager: bool,
    /// With tracing on: the operation at each recursion level, dumped when the limit is hit.
    trace_stack: Vec<(&'static str, VId)>,
}

/// `LOGOS_NBE_TRACE=1`: report what the engine is doing each time recursion deepens by a further
/// 20 000 levels (diagnosing runaway evaluation). Read once per process.
pub(crate) fn tracing_on() -> bool {
    tracing()
}

fn tracing() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOGOS_NBE_TRACE").is_some())
}

impl State {
    pub fn new() -> State {
        State::default()
    }

    /// The conversion steps this state has taken (see `conv_steps`).
    pub fn conversion_steps(&self) -> u64 {
        self.conv_steps
    }

    /// Drops every value; called when a declaration is done.
    pub fn reset(&mut self) {
        self.arena.reset();
        self.const_val.clear();
        self.const_ty.clear();
        self.const_body.clear();
        self.eval_cache.clear();
        self.whnf_cache.clear();
        self.whnf_core_cache.clear();
        self.type_cache.clear();
        self.check_cache.clear();
        self.proven.clear();
        self.refuted.clear();
        self.locals_cache.clear();
        self.failure.clear();
        self.depth = 0;
        self.level = 0;
        self.eager = false;
        self.trace_stack.clear();
    }
}

/// The engine: the kernel (environment, stores, local context) and one declaration's state.
pub struct Nbe<'a> {
    pub k: &'a mut Kernel,
    pub st: &'a mut State,
}

impl<'a> Nbe<'a> {
    pub fn new(k: &'a mut Kernel, st: &'a mut State) -> Nbe<'a> {
        Nbe { k, st }
    }

    #[inline]
    pub(crate) fn enter(&mut self) -> KResult<()> {
        self.st.depth += 1;
        if self.st.depth > MAX_DEPTH {
            if tracing() {
                let outer: Vec<String> = self
                    .st
                    .trace_stack
                    .iter()
                    .take(40)
                    .map(|(w, v)| format!("{w}: {}", self.describe(*v)))
                    .collect();
                eprintln!(
                    "[nbe] recursion limit; outermost frames:\n  {}",
                    outer.join("\n  ")
                );
            }
            return Err(KernelFailure::resource("recursion depth limit reached"));
        }
        Ok(())
    }

    /// With tracing on, reports `what` at every 20 000th level of recursion.
    #[inline]
    pub(crate) fn trace(&mut self, what: &'static str, v: VId) {
        if !tracing() {
            return;
        }
        self.st
            .trace_stack
            .truncate(self.st.depth.saturating_sub(1) as usize);
        self.st.trace_stack.push((what, v));
        if self.st.depth % 20_000 == 0 {
            eprintln!("[nbe depth {}] {what}: {}", self.st.depth, self.describe(v));
        }
    }

    /// A one-line description of a value's head, for traces.
    pub(crate) fn describe(&self, v: VId) -> String {
        use value::{Head, Val};
        match self.st.arena.get(v) {
            Val::Neutral {
                head: Head::Const(_, n, _),
                spine,
            }
            | Val::Glued { name: n, spine, .. } => {
                format!(
                    "{} ({} elims)",
                    self.k.s.names.display(n),
                    self.st.arena.spine_len(spine)
                )
            }
            Val::Neutral { head, spine } => {
                format!("{head:?} ({} elims)", self.st.arena.spine_len(spine))
            }
            Val::Thunk { expr, .. } => format!("thunk {}", self.k.show(expr)),
            other => format!("{other:?}"),
        }
    }

    #[inline]
    pub(crate) fn leave(&mut self) {
        self.st.depth -= 1;
        if tracing() {
            self.st.trace_stack.truncate(self.st.depth as usize);
        }
    }

    /// Definitional equality of two classic terms (which may mention the classic local context).
    pub fn is_def_eq(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        if t == s {
            return Ok(true);
        }
        let env = value::EnvId::EMPTY;
        let a = self.eval(env, t)?;
        let b = self.eval(env, s)?;
        self.conv(a, b)
    }
}

/// Definitional equality through the NbE engine, with a fresh state (for tests and the
/// differential harness; the checker keeps one [`State`] per declaration).
pub fn is_def_eq(k: &mut Kernel, st: &mut State, t: ExprId, s: ExprId) -> KResult<bool> {
    Nbe::new(k, st).is_def_eq(t, s)
}
