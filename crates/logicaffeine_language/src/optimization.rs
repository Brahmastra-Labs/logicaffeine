//! The single source of truth for compiler optimization toggles.
//!
//! Every optimization the compiler can perform has exactly one row in
//! [`REGISTRY`], naming it, giving it a human label and a `## No <Keyword>`
//! decorator keyword, recording which execution paths it touches, whether it
//! emits `unsafe` Rust, how it trades memory for speed, and its dependencies
//! and conflicts with other optimizations.
//!
//! A live choice of which optimizations are on is an [`OptimizationConfig`] —
//! a single `u64` bitset, one bit per [`Opt`]. The default is "everything on"
//! (tuned for speed); disabling everything yields plain, boring Rust.
//!
//! This lives in `logicaffeine_language` (not the compiler) because the parser
//! maps `## No <Keyword>` decorators to [`Opt`]s, so the type must be visible
//! here; the compiler and JIT see it transitively.

use std::collections::BTreeMap;

/// Execution-path tags (bitmask) recording where an optimization applies.
pub mod path {
    /// Ahead-of-time Rust codegen pipeline (`optimize_program`).
    pub const AOT: u8 = 1 << 0;
    /// Live run-path optimizer (`optimize_for_run`).
    pub const RUN: u8 = 1 << 1;
    /// Bytecode VM.
    pub const VM: u8 = 1 << 2;
    /// Forge JIT tier.
    pub const JIT: u8 = 1 << 3;
    /// Rust source emission (`codegen_program`).
    pub const CODEGEN: u8 = 1 << 4;
}

/// How an optimization trades memory against speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemClass {
    /// Neither clearly grows nor shrinks memory.
    Neutral,
    /// Spends memory (extra allocation, code growth, caches) to gain speed.
    TradesMemForSpeed,
    /// Shrinks memory (tighter representations, fewer allocations).
    SavesMem,
}

/// How expensive an optimization pass is to RUN on the live path — the lever the
/// tiered optimizer uses to decide WHEN (at which hotness tier) to pay for it.
/// Orthogonal to [`MemClass`] (which is about the OUTPUT's memory cost). The derived
/// `Ord` makes a tier gate a single comparison: `Cheap < Medium < Heavy` is exactly
/// the tier-inclusion order T1 ⊂ T2 ⊂ T3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptCost {
    /// A single structural sweep, no fixpoint (inline, DCE, TCO).
    Cheap = 0,
    /// A whole-function analysis or one bounded fixpoint (GVN, LICM, oracle,
    /// fusion, the capped PE-light fixpoint).
    Medium = 1,
    /// A cloning / region-growing pass (the full PE fixpoint, unroll, scalarize,
    /// recursion unfold, equality saturation).
    Heavy = 2,
}

/// A hotness tier — how much optimization budget a unit has earned (HOTSWAP §4.1).
/// Each tier pays for strictly more than the one below; `T3` with an all-on config
/// reproduces today's whole-program `optimize_for_run`, bit-for-bit (the
/// compatibility + soundness anchor). The mechanism that escalates a unit through
/// the tiers (call/back-edge counters, thresholds) lives in the VM; this is just
/// the policy the optimizer reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Baseline: raw parse → bytecode, no optimization.
    T0 = 0,
    /// Warm: Cheap opts.
    T1 = 1,
    /// Hot: Cheap + Medium opts (PE-light).
    T2 = 2,
    /// Very-hot: every opt (PE-full, unroll, recursion unfold).
    T3 = 3,
}

impl Tier {
    /// The most expensive pass this tier will pay for; `None` ⇒ optimize nothing.
    /// The inclusion order T1 ⊂ T2 ⊂ T3 falls straight out of [`OptCost`]'s `Ord`.
    #[inline]
    pub fn budget(self) -> Option<OptCost> {
        match self {
            Tier::T0 => None,
            Tier::T1 => Some(OptCost::Cheap),
            Tier::T2 => Some(OptCost::Medium),
            Tier::T3 => Some(OptCost::Heavy),
        }
    }
}

/// Whether `opt` runs at `tier` under `cfg`: it must be enabled in the config AND
/// cost no more than the tier's budget. The tier→opt-set mapping is therefore
/// DERIVED from the registry, never hardcoded — adding an opt with a cost slots it
/// into the right tier automatically (HOTSWAP §4.2).
///
/// This is pure policy. Run-path callers additionally AND in any
/// `#[cfg(feature = "codegen")]` availability for the codegen-only passes
/// (Affine/Unroll/Scalarize): those passes don't exist on targets without the
/// feature, so tier-invariance is asserted per-platform, not across platforms.
#[inline]
pub fn admits(cfg: &OptimizationConfig, tier: Tier, opt: Opt) -> bool {
    matches!(tier.budget(), Some(b) if opt.cost() <= b) && cfg.is_on(opt)
}

/// The number of [`Opt`] variants — the width of a [`PinSet`]. Derived from the
/// last variant so it tracks the enum automatically (guarded by a test against
/// `REGISTRY.len()`).
pub const OPT_COUNT: usize = Opt::Supercompile as usize + 1;

/// WHEN each allowed optimization runs, as opposed to WHICH are allowed
/// ([`OptimizationConfig`]). The sibling policy of HOTSWAP §8: the bitset says what
/// is permitted; this says at what hotness tier each permitted opt is paid for.
/// Kept separate from `OptimizationConfig` (which is serialized / merged per-function
/// / normalized) so tier state never pollutes the "which opts" bitset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierMode {
    /// Units climb T0→T3 by hotness — the default.
    Tiered,
    /// Everything optimized upfront at T3 — today's behavior, the A/B baseline.
    Eager,
    /// Never optimize — every unit pinned at T0.
    Baseline,
}

/// Call/back-edge counts at which a unit enters each tier (HOTSWAP §5). Defaults:
/// don't optimize code that runs once (T1 at 8); amortize whole-function analyses
/// only once recurring (T2 at 32); pay the cloning passes only on genuinely hot
/// kernels (T3 at 100, aligned with the existing native-tier threshold).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierThresholds {
    pub t1: u32,
    pub t2: u32,
    pub t3: u32,
}

impl Default for TierThresholds {
    fn default() -> Self {
        Self { t1: 8, t2: 32, t3: 100 }
    }
}

impl TierThresholds {
    /// The tier a unit with `count` accumulated calls/back-edges has earned.
    #[inline]
    pub fn tier_for(self, count: u32) -> Tier {
        if count >= self.t3 {
            Tier::T3
        } else if count >= self.t2 {
            Tier::T2
        } else if count >= self.t1 {
            Tier::T1
        } else {
            Tier::T0
        }
    }

    /// `LOGOS_TIER_THRESHOLDS="8,32,100"`, overridable per-rung by
    /// `LOGOS_TIER_T1`/`T2`/`T3`.
    pub fn from_env() -> Self {
        Self::from_spec(
            std::env::var("LOGOS_TIER_THRESHOLDS").ok().as_deref(),
            std::env::var("LOGOS_TIER_T1").ok().as_deref(),
            std::env::var("LOGOS_TIER_T2").ok().as_deref(),
            std::env::var("LOGOS_TIER_T3").ok().as_deref(),
        )
    }

    /// The pure, testable core of `from_env`.
    pub fn from_spec(
        combined: Option<&str>,
        t1: Option<&str>,
        t2: Option<&str>,
        t3: Option<&str>,
    ) -> Self {
        let mut th = Self::default();
        if let Some(c) = combined {
            let parts: Vec<u32> = c.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if parts.len() == 3 {
                th = Self { t1: parts[0], t2: parts[1], t3: parts[2] };
            }
        }
        if let Some(v) = t1.and_then(|s| s.trim().parse().ok()) {
            th.t1 = v;
        }
        if let Some(v) = t2.and_then(|s| s.trim().parse().ok()) {
            th.t2 = v;
        }
        if let Some(v) = t3.and_then(|s| s.trim().parse().ok()) {
            th.t3 = v;
        }
        th
    }
}

/// A per-opt tier override (HOTSWAP §8 `## Tier <kw> <eager|t1|t2|t3|never>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    /// Allowed by config but pinned off the tiering ladder — never run by tiering
    /// (distinct from `## No <opt>`, which forbids it and cascades through normalize).
    Never,
    /// Run as soon as the unit is optimized at all (any tier ≥ T1), ignoring cost.
    Eager,
    /// Run once this tier is reached.
    At(Tier),
}

/// Per-opt tier pins, indexed by [`Opt`] discriminant. `None` ⇒ use the cost-derived
/// tier. `Copy` so [`HotswapConfig`] stays cheap to thread through the run path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinSet {
    pins: [Option<Pin>; OPT_COUNT],
}

impl Default for PinSet {
    fn default() -> Self {
        Self::none()
    }
}

impl PinSet {
    /// No pins — every opt uses its cost-derived tier.
    pub const fn none() -> Self {
        Self { pins: [None; OPT_COUNT] }
    }

    #[inline]
    pub fn get(&self, opt: Opt) -> Option<Pin> {
        self.pins[opt as usize]
    }

    #[inline]
    pub fn set(&mut self, opt: Opt, pin: Pin) {
        self.pins[opt as usize] = Some(pin);
    }

    /// Copy every pin set in `other` over this set — `other` wins per-opt. Used to
    /// overlay in-source `## Tier` decorators onto the ambient env pins (the
    /// decorator, being explicit, takes precedence).
    pub fn overlay(&mut self, other: &PinSet) {
        for (slot, pin) in self.pins.iter_mut().zip(other.pins.iter()) {
            if let Some(p) = pin {
                *slot = Some(*p);
            }
        }
    }
}

/// "*When* does each allowed opt run" — the sibling to [`OptimizationConfig`]
/// (HOTSWAP §8). `mode` picks the escalation policy, `thresholds` the rungs,
/// `force_tier` pins a fixed tier for deterministic tests, and `pins` overrides
/// individual opts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotswapConfig {
    pub mode: TierMode,
    pub thresholds: TierThresholds,
    /// Forces a fixed tier regardless of hotness — the determinism lever for
    /// differential tests (`LOGOS_FORCE_TIER`). `None` ⇒ derive from mode + count.
    pub force_tier: Option<Tier>,
    pub pins: PinSet,
}

impl Default for HotswapConfig {
    fn default() -> Self {
        // Eager by default — the §12.2 ratchet (see `from_spec`). Tiered is the locked
        // destination, reached once it is proven not to regress.
        Self {
            mode: TierMode::Eager,
            thresholds: TierThresholds::default(),
            force_tier: None,
            pins: PinSet::none(),
        }
    }
}

impl HotswapConfig {
    /// The tier a unit with `count` accumulated calls/back-edges should run at,
    /// under this config's mode (and any `force_tier` override).
    #[inline]
    pub fn effective_tier(&self, count: u32) -> Tier {
        if let Some(t) = self.force_tier {
            return t;
        }
        match self.mode {
            TierMode::Baseline => Tier::T0,
            TierMode::Eager => Tier::T3,
            TierMode::Tiered => self.thresholds.tier_for(count),
        }
    }

    /// This config's pin for `opt`, if any.
    #[inline]
    pub fn pin(&self, opt: Opt) -> Option<Pin> {
        self.pins.get(opt)
    }

    /// The tier at which the run path optimizes UPFRONT, before the program executes.
    /// `Eager` pays the full optimizer (T3, today's behavior); `Tiered`/`Baseline`
    /// start at the baseline (T0) and rely on the native tier — and, later, per-function
    /// re-optimization — to escalate hot code during the run, reclaiming the upfront
    /// optimizer cost (HOTSWAP §12.1). `force_tier` overrides everything (test
    /// determinism). Distinct from `effective_tier`, which maps a per-unit hotness
    /// COUNT to a tier; this maps the program-level MODE to the upfront tier.
    #[inline]
    pub fn run_tier(&self) -> Tier {
        if let Some(t) = self.force_tier {
            return t;
        }
        match self.mode {
            TierMode::Eager => Tier::T3,
            TierMode::Tiered | TierMode::Baseline => Tier::T0,
        }
    }

    /// Build from the environment — the single external entry point (HOTSWAP §8).
    /// `LOGOS_HOTSWAP=off` forces Eager; `LOGOS_TIER_PROFILE` picks the preset;
    /// `LOGOS_FORCE_TIER` pins a fixed tier; `LOGOS_TIER_PIN` sets per-opt pins.
    pub fn from_env() -> Self {
        Self::from_spec(
            std::env::var("LOGOS_HOTSWAP").ok().as_deref(),
            std::env::var("LOGOS_TIER_PROFILE").ok().as_deref(),
            std::env::var("LOGOS_FORCE_TIER").ok().as_deref(),
            TierThresholds::from_env(),
            std::env::var("LOGOS_TIER_PIN").ok().as_deref(),
        )
    }

    /// The pure, testable core of `from_env`.
    pub fn from_spec(
        hotswap: Option<&str>,
        profile: Option<&str>,
        force_tier: Option<&str>,
        thresholds: TierThresholds,
        pin_spec: Option<&str>,
    ) -> Self {
        // The headline escape hatch: kill tiering, optimize everything upfront.
        // DEFAULT = Eager (today's behavior). The locked product default is Tiered,
        // but the §12.2 RATCHET keeps Eager until the tiered path (P9/P10 per-function
        // Axis-1) is proven >= Eager on the benchmark A/B — so the served number can
        // only improve. Tiered/Baseline are opt-in via LOGOS_TIER_PROFILE until then.
        let mode = if hotswap == Some("off") {
            TierMode::Eager
        } else {
            match profile {
                Some("tiered") => TierMode::Tiered,
                Some("baseline") => TierMode::Baseline,
                _ => TierMode::Eager,
            }
        };
        let force_tier = force_tier.and_then(parse_tier);
        let mut pins = PinSet::none();
        if let Some(spec) = pin_spec {
            apply_pin_spec(&mut pins, spec);
        }
        HotswapConfig { mode, thresholds, force_tier, pins }
    }
}

/// Parse a tier token: `0|1|2|3`, `t0..t3`, or `baseline|warm|hot|veryhot`
/// (case-insensitive). `None` for anything else.
fn parse_tier(s: &str) -> Option<Tier> {
    match s.trim().to_ascii_lowercase().as_str() {
        "0" | "t0" | "baseline" => Some(Tier::T0),
        "1" | "t1" | "warm" => Some(Tier::T1),
        "2" | "t2" | "hot" => Some(Tier::T2),
        "3" | "t3" | "veryhot" | "very_hot" => Some(Tier::T3),
        _ => None,
    }
}

/// Parse one pin value: `eager`, `never`, or a tier token (see [`parse_tier`]). The
/// surface the `## Tier <opt> <value>` parser and `LOGOS_TIER_PIN` both resolve.
pub fn pin_from_str(s: &str) -> Option<Pin> {
    match s.trim().to_ascii_lowercase().as_str() {
        "eager" => Some(Pin::Eager),
        "never" => Some(Pin::Never),
        other => parse_tier(other).map(Pin::At),
    }
}

/// Apply a pin spec — `"specialize:eager,fuse:t1,unfold:never"` — to `pins`. The same
/// keyword list as `LOGOS_OPT_OFF`; unknown keywords / values are skipped.
fn apply_pin_spec(pins: &mut PinSet, spec: &str) {
    for tok in spec.split([',', ';']) {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some((kw, val)) = tok.split_once(':') {
            if let (Some(opt), Some(pin)) = (by_keyword(kw.trim()), pin_from_str(val)) {
                pins.set(opt, pin);
            }
        }
    }
}

/// [`admits`] with per-opt pins applied: a pin overrides the cost-derived tier
/// (HOTSWAP §8). A disabled opt (`!cfg.is_on`) never runs regardless of pin, so
/// `## No <opt>` always wins over `## Tier <opt> eager`.
#[inline]
pub fn admits_pinned(cfg: &OptimizationConfig, hs: &HotswapConfig, tier: Tier, opt: Opt) -> bool {
    if !cfg.is_on(opt) {
        return false;
    }
    match hs.pin(opt) {
        Some(Pin::Never) => false,
        Some(Pin::Eager) => true,
        Some(Pin::At(t)) => tier >= t,
        None => matches!(tier.budget(), Some(b) if opt.cost() <= b),
    }
}

/// Where a `## No <X>` decorator for this optimization may legitimately appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only meaningful program-wide (whole-program analyses).
    ProgramOnly,
    /// Meaningful both program-wide and on an individual function.
    Both,
}

/// Every optimization the compiler can toggle. The discriminant is the bit
/// index used by [`OptimizationConfig`], so the order here is stable and must
/// not be reordered casually (it also defines conflict-resolution precedence:
/// an earlier variant wins a conflict against a later one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum Opt {
    /// Automatic memoization of pure functions.
    Memo = 0,
    /// Tail-call optimization (self-tail-recursion → loop).
    Tco = 1,
    /// Peephole rewrites (swap, vec-fill, for-range, …).
    Peephole = 2,
    /// Read-only / mutable borrow inference (`&[T]` / `&mut [T]` params).
    Borrow = 3,
    /// Partial evaluation / polyvariant specialization.
    Specialize = 4,
    /// Compile-time function evaluation (CTFE).
    Comptime = 5,
    /// Inlining of tiny and leaf helper functions.
    Inline = 6,
    /// Bounded unrolling of recursive functions.
    Unfold = 7,
    /// Defunctionalization (closures → first-order functions).
    Defunctionalize = 8,
    /// Loop-invariant code motion (LICM).
    LoopHoist = 9,
    /// Loop unrolling of constant-trip loops.
    Unroll = 10,
    /// Guard-based loop index-set splitting (for vectorization).
    LoopSplit = 11,
    /// Accumulator-loop → closed-form recognition.
    ClosedForm = 12,
    /// Deforestation (producer/consumer stream fusion).
    Fuse = 13,
    /// Loop-carried common-subexpression hoisting.
    LoopCse = 14,
    /// Global value numbering / common-subexpression elimination.
    Cse = 15,
    /// Dead-code elimination.
    DeadCode = 16,
    /// Fixed-size sequence scalarization (SROA).
    Scalarize = 17,
    /// Affine offset-array deletion → closed-form indexing.
    Affine = 18,
    /// Array-of-structs interleaving of co-indexed sequences.
    Interleave = 19,
    /// De-`Rc`: unique-owned sequences become plain `Vec`.
    Unbox = 20,
    /// Hoisting borrow live-ranges out of loop nests.
    HoistBorrows = 21,
    /// `i64`→`i32` sequence element narrowing (codegen).
    Narrow = 22,
    /// `i64`→`i32` sequence element narrowing (VM).
    NarrowVm = 23,
    /// `i64`→`i32` map-key narrowing.
    NarrowMap = 24,
    /// Sparse map → dense direct-addressed array.
    DenseMap = 25,
    /// Element-type narrowing via abstract interpretation.
    ElemType = 26,
    /// Constant-divisor magic-reciprocal division.
    FastDiv = 27,
    /// Float induction-variable strength reduction.
    FloatStrength = 28,
    /// Abstract interpretation / oracle-fact production (interval analysis).
    Oracle = 29,
    /// Oracle-derived precise bounds-check guards.
    OracleHints = 30,
    /// Oracle-proven unchecked indexing (`get_unchecked`).
    Unchecked = 31,
    /// SIMD-vectorized search kernels.
    Simd = 32,
    /// Cascade folding of sequential comparisons.
    Cascade = 33,
    /// Indexed string-search codegen.
    IndexString = 34,
    /// Capacity-scaling buffer-fill simplification.
    CapScale = 35,
    /// Reflection symmetry breaking (bitmask search).
    Symmetry = 36,
    /// Popcount base-case collapse for bitmask search.
    Popcount = 37,
    /// Equality-saturation rewriting (e-graph).
    Saturate = 38,
    /// Supercompilation (heavy residual specialization, AOT only).
    Supercompile = 39,
}

impl Opt {
    /// The bit index this optimization occupies in an [`OptimizationConfig`].
    #[inline]
    pub const fn bit(self) -> u64 {
        1u64 << (self as u8)
    }

    /// This optimization's registry row.
    pub fn meta(self) -> &'static OptMeta {
        // Discriminants are 0..REGISTRY.len() in order, so index directly.
        &REGISTRY[self as usize]
    }

    /// How expensive this optimization is to run — the lever the tiered optimizer
    /// uses to decide which hotness tier pays for it.
    #[inline]
    pub fn cost(self) -> OptCost {
        self.meta().cost
    }
}

/// One row of the optimization registry — the complete static description of a
/// single optimization. Adding an optimization means adding one [`Opt`] variant
/// and one row here; nothing else hardcodes the list.
#[derive(Debug, Clone, Copy)]
pub struct OptMeta {
    /// The optimization this row describes.
    pub opt: Opt,
    /// The `## No <Keyword>` decorator word (lowercase, single token).
    pub keyword: &'static str,
    /// Human-readable label for UIs.
    pub label: &'static str,
    /// Which group the UI files this under.
    pub group: &'static str,
    /// Whether this optimization is on in the default (all-on, speed) config.
    pub default_on: bool,
    /// Execution paths it affects (bitmask of [`path`] constants).
    pub paths: u8,
    /// Whether enabling it can emit `unsafe` Rust (disabled by the Safety profile).
    pub emits_unsafe: bool,
    /// Its memory/speed trade-off classification.
    pub mem_class: MemClass,
    /// How expensive the pass is to run — the hotness tier at which the tiered
    /// optimizer starts paying for it (HOTSWAP §3). For every row,
    /// `cost(requires) ≤ cost`, so a tier that admits this opt admits its deps.
    pub cost: OptCost,
    /// Optimizations that must be on for this one to apply; if any is off,
    /// `normalize` turns this one off too.
    pub requires: &'static [Opt],
    /// Optimizations mutually exclusive with this one — GLOBAL exclusion (both on
    /// → `normalize` disables the later-declared). Distinct from `preempts`.
    pub conflicts: &'static [Opt],
    /// Optimizations this one takes PRECEDENCE over, per instance: when both are
    /// enabled, this one is tried/applied first for a given function/loop/array/
    /// map, and the listed ones act as the fallback for the instances it did not
    /// claim. Both stay enabled (unlike `conflicts`); the listed optimization
    /// fires only where this one declined. Disabling THIS one is what can let a
    /// preempted optimization surface — the edge the menu-tree walks.
    pub preempts: &'static [Opt],
    /// Where its decorator may appear.
    pub scope: Scope,
}

use path::{AOT, CODEGEN, JIT, RUN, VM};
use MemClass::{Neutral, SavesMem, TradesMemForSpeed};
use OptCost::{Cheap, Heavy, Medium};
use Scope::{Both, ProgramOnly};

/// The complete optimization registry: exactly one row per [`Opt`], in
/// discriminant order. Earlier rows win conflicts against later rows.
pub static REGISTRY: &[OptMeta] = &[
    OptMeta { opt: Opt::Memo, keyword: "memo", label: "Memoization", group: "Inlining & calls", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Tco, keyword: "tco", label: "Tail-call optimization", group: "Inlining & calls", default_on: true, paths: AOT | RUN | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::Memo], scope: Both },
    OptMeta { opt: Opt::Peephole, keyword: "peephole", label: "Peephole rewrites", group: "Peephole", default_on: true, paths: AOT | CODEGEN, emits_unsafe: true, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    // Borrow preempts Tco: a `&mut [T]` in-place recursion (quicksort's
    // consume-alias shape) keeps its plain recursive calls — the pair-TCE
    // rewrite would reassign the borrowed param.
    OptMeta { opt: Opt::Borrow, keyword: "borrow", label: "Borrow inference", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::Tco], scope: Both },
    OptMeta { opt: Opt::Specialize, keyword: "specialize", label: "Partial evaluation", group: "Inlining & calls", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Comptime, keyword: "comptime", label: "Compile-time evaluation (CTFE)", group: "Inlining & calls", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[Opt::Supercompile], scope: Both },
    OptMeta { opt: Opt::Inline, keyword: "inline", label: "Function inlining", group: "Inlining & calls", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Unfold, keyword: "unfold", label: "Recursion unrolling", group: "Inlining & calls", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Heavy, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Defunctionalize, keyword: "defunctionalize", label: "Defunctionalization", group: "Inlining & calls", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::LoopHoist, keyword: "loophoist", label: "Loop-invariant code motion", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Unroll, keyword: "unroll", label: "Loop unrolling", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Heavy, requires: &[], conflicts: &[], preempts: &[Opt::Interleave], scope: Both },
    OptMeta { opt: Opt::LoopSplit, keyword: "loopsplit", label: "Loop index-set splitting", group: "Loops", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::ClosedForm, keyword: "closedform", label: "Closed-form loop recognition", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[Opt::Peephole], scope: Both },
    OptMeta { opt: Opt::Fuse, keyword: "fuse", label: "Deforestation (stream fusion)", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::LoopCse, keyword: "loopcse", label: "Loop-carried CSE", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Cse, keyword: "cse", label: "Common-subexpression elimination (GVN)", group: "Redundancy", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::DeadCode, keyword: "deadcode", label: "Dead-code elimination", group: "Redundancy", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Scalarize, keyword: "scalarize", label: "Array scalarization (SROA)", group: "Arrays & memory", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Heavy, requires: &[Opt::Cse], conflicts: &[], preempts: &[Opt::HoistBorrows, Opt::Interleave, Opt::Unbox], scope: Both },
    OptMeta { opt: Opt::Affine, keyword: "affine", label: "Affine array → closed form", group: "Arrays & memory", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Medium, requires: &[Opt::Unbox], conflicts: &[], preempts: &[], scope: Both },
    // AoS interleaving is mutually exclusive with Unroll/Scalarize only at RUNTIME
    // (the codegen regime gate skips it when a co-indexed array has any constant
    // index access — i.e. once it has been unrolled/scalarized), NOT at config
    // level. All three stay enabled by default; the gate arbitrates per program.
    OptMeta { opt: Opt::Interleave, keyword: "interleave", label: "Array-of-structs interleaving", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Heavy, requires: &[Opt::Scalarize], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::Unbox, keyword: "unbox", label: "De-Rc (Vec instead of Rc<RefCell>)", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::HoistBorrows, keyword: "hoistborrows", label: "Borrow hoisting", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::Narrow, keyword: "narrow", label: "i32 sequence narrowing (codegen)", group: "Number representation", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[Opt::Unbox], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::NarrowVm, keyword: "narrowvm", label: "i32 sequence narrowing (VM)", group: "Number representation", default_on: true, paths: VM | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::NarrowMap, keyword: "narrowmap", label: "i32 map-key narrowing", group: "Number representation", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::DenseMap, keyword: "densemap", label: "Dense direct-addressed map", group: "Number representation", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::NarrowMap], scope: ProgramOnly },
    OptMeta { opt: Opt::ElemType, keyword: "elemtype", label: "Element-type narrowing", group: "Number representation", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Medium, requires: &[Opt::Oracle], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::FastDiv, keyword: "fastdiv", label: "Magic-reciprocal division", group: "Number representation", default_on: true, paths: AOT | VM | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::FloatStrength, keyword: "floatstrength", label: "Float induction strength reduction", group: "Number representation", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Oracle, keyword: "oracle", label: "Abstract interpretation (oracle facts)", group: "Bounds & checks", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::OracleHints, keyword: "oraclehints", label: "Oracle bounds-check guards", group: "Bounds & checks", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[Opt::Oracle], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Unchecked, keyword: "unchecked", label: "Oracle-proven unchecked indexing", group: "Bounds & checks", default_on: true, paths: AOT | JIT | CODEGEN, emits_unsafe: true, mem_class: Neutral, cost: Medium, requires: &[Opt::Oracle], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Simd, keyword: "simd", label: "SIMD search kernels", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: true, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::Cascade, keyword: "cascade", label: "Cascade folding", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::IndexString, keyword: "indexstring", label: "Indexed string search", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::CapScale, keyword: "capscale", label: "Capacity-scaling buffer fill", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly },
    OptMeta { opt: Opt::Symmetry, keyword: "symmetry", label: "Symmetry breaking", group: "Search-space", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Heavy, requires: &[Opt::Specialize], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Popcount, keyword: "popcount", label: "Popcount leaf collapse", group: "Search-space", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Saturate, keyword: "saturate", label: "Equality saturation (e-graph)", group: "Search-space", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Heavy, requires: &[], conflicts: &[], preempts: &[], scope: Both },
    OptMeta { opt: Opt::Supercompile, keyword: "supercompile", label: "Supercompilation", group: "Search-space", default_on: true, paths: AOT, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Heavy, requires: &[], conflicts: &[], preempts: &[], scope: Both },
];

/// Look up an optimization by its decorator keyword (case-insensitive).
pub fn by_keyword(word: &str) -> Option<Opt> {
    let w = word.to_ascii_lowercase();
    REGISTRY.iter().find(|m| m.keyword == w).map(|m| m.opt)
}

/// Produce a copy of `src` with a file-level `## No <keyword>` decorator inserted
/// (just before `## Main`, where the parser folds it into the program-wide config)
/// for each disabled-optimization keyword. This is what the benchmarks UI shows on
/// the Logos source when a toggle is flipped off; compiling the result applies the
/// same config the toggle represents.
pub fn decorate_source(src: &str, disabled_keywords: &[&str]) -> String {
    if disabled_keywords.is_empty() {
        return src.to_string();
    }
    let decorators: String = disabled_keywords
        .iter()
        .map(|kw| format!("## No {kw}\n"))
        .collect();
    match src.find("## Main") {
        Some(idx) => format!("{}{}{}", &src[..idx], decorators, &src[idx..]),
        None => format!("{src}{decorators}"),
    }
}

/// Produce a copy of `src` with a file-level `## Tier <keyword> <value>` decorator
/// inserted (just before `## Main`, where the parser collects program-level tier
/// pins) for each `(keyword, value)` pair. The tiering analog of [`decorate_source`]:
/// the benchmarks UI injects these to pin an optimization to a tier exactly as it
/// injects `## No <X>` to disable one. `value` is one of `eager|t1|t2|t3|never`.
pub fn decorate_tiers(src: &str, pins: &[(&str, &str)]) -> String {
    if pins.is_empty() {
        return src.to_string();
    }
    let decorators: String = pins
        .iter()
        .map(|(kw, val)| format!("## Tier {kw} {val}\n"))
        .collect();
    match src.find("## Main") {
        Some(idx) => format!("{}{}{}", &src[..idx], decorators, &src[idx..]),
        None => format!("{src}{decorators}"),
    }
}

/// Why `normalize` flipped an optimization off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// A required optimization was off.
    DependencyOff(Opt),
    /// A conflicting optimization (declared earlier) was on.
    ConflictWith(Opt),
}

/// What `normalize` changed, so a UI can explain auto-disabled toggles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizeReport {
    /// Optimizations turned off by normalization, with the reason.
    pub auto_disabled: Vec<(Opt, Reason)>,
}

impl NormalizeReport {
    /// Whether normalization changed anything.
    pub fn is_empty(&self) -> bool {
        self.auto_disabled.is_empty()
    }
}

/// A live choice of which optimizations are enabled — one bit per [`Opt`].
///
/// The default is every optimization on (the speed-tuned baseline). Clearing
/// all bits yields plain, boring Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OptimizationConfig {
    enabled: u64,
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self::all_on()
    }
}

impl OptimizationConfig {
    /// Every default-on optimization enabled (the speed baseline).
    pub fn all_on() -> Self {
        let mut enabled = 0u64;
        for m in REGISTRY {
            if m.default_on {
                enabled |= m.opt.bit();
            }
        }
        Self { enabled }
    }

    /// No optimizations enabled — the "boring Rust" config.
    pub const fn all_off() -> Self {
        Self { enabled: 0 }
    }

    /// Whether a given optimization is on.
    #[inline]
    pub const fn is_on(&self, opt: Opt) -> bool {
        self.enabled & opt.bit() != 0
    }

    /// Turn an optimization on or off.
    #[inline]
    pub fn set(&mut self, opt: Opt, on: bool) {
        if on {
            self.enabled |= opt.bit();
        } else {
            self.enabled &= !opt.bit();
        }
    }

    /// Turn an optimization off (builder style).
    pub fn disable(mut self, opt: Opt) -> Self {
        self.set(opt, false);
        self
    }

    /// Turn an optimization on (builder style).
    pub fn enable(mut self, opt: Opt) -> Self {
        self.set(opt, true);
        self
    }

    /// Turn `opt` on together with the transitive closure of what it `requires`.
    ///
    /// The complement of [`normalize`](Self::normalize)'s dependency-closure
    /// (which turns *dependents* off when a requirement goes off): enabling a
    /// leaf must pull its whole requires-chain on, or `normalize` would just turn
    /// the leaf back off. This is the on-direction of the UI's toggle-linking.
    pub fn enable_with_requires(&mut self, opt: Opt) {
        let mut visited = 0u64;
        let mut stack = vec![opt];
        while let Some(o) = stack.pop() {
            if visited & o.bit() != 0 {
                continue;
            }
            visited |= o.bit();
            self.set(o, true);
            for &req in o.meta().requires {
                stack.push(req);
            }
        }
    }

    /// Whether every optimization is off.
    pub const fn is_all_off(&self) -> bool {
        self.enabled == 0
    }

    /// The raw enabled bitmask (bit layout = [`Opt::bit`]). Lets callers relate
    /// an [`OptimizationConfig`] to a [`FiredOptimizations`] set bit-for-bit.
    pub const fn bits(&self) -> u64 {
        self.enabled
    }

    /// Combine a program-level config with a function's per-function overrides:
    /// an optimization is on for the function only if it is on in both.
    pub const fn merged(&self, func: &OptimizationConfig) -> OptimizationConfig {
        OptimizationConfig { enabled: self.enabled & func.enabled }
    }

    /// The keywords of every optimization currently off (registry order).
    pub fn disabled_keywords(&self) -> impl Iterator<Item = &'static str> + '_ {
        REGISTRY
            .iter()
            .filter(move |m| !self.is_on(m.opt))
            .map(|m| m.keyword)
    }

    /// Resolve conflicts and dependencies, returning what was auto-disabled.
    ///
    /// Conflicts: when two mutually exclusive optimizations are both on, the
    /// one declared earlier in [`REGISTRY`] wins and the later is disabled.
    /// Dependencies: any optimization whose `requires` set is not fully on is
    /// disabled (transitively, to a fixed point).
    pub fn normalize(&mut self) -> NormalizeReport {
        let mut report = NormalizeReport::default();
        if self.is_all_off() {
            return report;
        }

        // Conflict resolution: earlier-declared optimization wins.
        for m in REGISTRY {
            if !self.is_on(m.opt) {
                continue;
            }
            for &other in m.conflicts {
                if (other as u8) < (m.opt as u8) && self.is_on(other) {
                    self.set(m.opt, false);
                    report.auto_disabled.push((m.opt, Reason::ConflictWith(other)));
                    break;
                }
            }
        }

        // Dependency closure: disable anything whose requirement is off.
        loop {
            let mut changed = false;
            for m in REGISTRY {
                if !self.is_on(m.opt) {
                    continue;
                }
                for &req in m.requires {
                    if !self.is_on(req) {
                        self.set(m.opt, false);
                        report.auto_disabled.push((m.opt, Reason::DependencyOff(req)));
                        changed = true;
                        break;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        report
    }

    /// Build a config from the environment, the single external entry point.
    ///
    /// - `LOGOS_OPT=off` (or `LOGOS_NO_OPTIMIZE=1`) disables everything.
    /// - `LOGOS_OPT_PROFILE=speed|memory|safety` selects a base profile.
    /// - `LOGOS_OPT_OFF="scalarize,unroll,…"` disables the listed keywords.
    ///
    /// The result is normalized.
    pub fn from_env() -> Self {
        let opt = std::env::var("LOGOS_OPT").ok();
        let no_opt = std::env::var("LOGOS_NO_OPTIMIZE").ok();
        let profile = std::env::var("LOGOS_OPT_PROFILE").ok();
        let off = std::env::var("LOGOS_OPT_OFF").ok();
        let master_off = opt.as_deref() == Some("off") || no_opt.as_deref() == Some("1");
        Self::from_spec(master_off, profile.as_deref(), off.as_deref())
    }

    /// The pure, testable core of `from_env`. `master_off` disables everything;
    /// `profile` picks the base preset (speed by default); `off_list` is a
    /// comma/space/`;`-separated list of keywords to disable. The result is
    /// normalized.
    pub fn from_spec(master_off: bool, profile: Option<&str>, off_list: Option<&str>) -> Self {
        if master_off {
            return Self::all_off();
        }

        let mut cfg = match profile {
            Some("memory") => Profile::Memory.config(),
            Some("safety") => Profile::Safety.config(),
            _ => Profile::Speed.config(),
        };

        if let Some(list) = off_list {
            for tok in list.split([',', ' ', ';']) {
                let tok = tok.trim();
                if tok.is_empty() {
                    continue;
                }
                if let Some(opt) = by_keyword(tok) {
                    cfg.set(opt, false);
                }
            }
        }

        cfg.normalize();
        cfg
    }

    /// Build a config from a keyword→enabled map (e.g. UI toggles), returning
    /// the normalized config and the optimizations normalization forced off.
    pub fn from_toggles(toggles: &BTreeMap<String, bool>) -> (Self, Vec<Opt>) {
        let mut cfg = Self::all_on();
        for m in REGISTRY {
            if let Some(&on) = toggles.get(m.keyword) {
                cfg.set(m.opt, on);
            }
        }
        let before = cfg.enabled;
        let report = cfg.normalize();
        let _ = before;
        let forced = report.auto_disabled.iter().map(|(opt, _)| *opt).collect();
        (cfg, forced)
    }
}

/// Which optimizations actually FIRED during a single compile — one bit per
/// [`Opt`], the same bit layout as [`OptimizationConfig`]. An optimization is
/// "fired" when it actually changed the program or emitted its optimized form
/// for this compile, as distinct from merely being *enabled*. Populated only
/// while a firing trace is active (see `logicaffeine_compile`'s traced compile
/// entry points); the normal compile path never records into it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FiredOptimizations {
    fired: u64,
}

impl FiredOptimizations {
    /// An empty set — nothing fired.
    pub const fn new() -> Self {
        Self { fired: 0 }
    }

    /// Build from a raw bitmask (bit layout = [`Opt::bit`]).
    pub const fn from_bits(fired: u64) -> Self {
        Self { fired }
    }

    /// Record that `opt` fired.
    #[inline]
    pub fn mark(&mut self, opt: Opt) {
        self.fired |= opt.bit();
    }

    /// Whether `opt` fired.
    #[inline]
    pub const fn fired(&self, opt: Opt) -> bool {
        self.fired & opt.bit() != 0
    }

    /// Whether nothing fired.
    pub const fn is_empty(&self) -> bool {
        self.fired == 0
    }

    /// The raw fired bitmask.
    pub const fn bits(&self) -> u64 {
        self.fired
    }

    /// Fold another set's fired bits into this one.
    pub fn merge(&mut self, other: &FiredOptimizations) {
        self.fired |= other.fired;
    }

    /// The optimizations that fired, in registry order.
    pub fn opts(&self) -> Vec<Opt> {
        REGISTRY
            .iter()
            .filter(|m| self.fired(m.opt))
            .map(|m| m.opt)
            .collect()
    }

    /// The decorator keywords of the optimizations that fired, in registry order.
    pub fn keywords(&self) -> Vec<&'static str> {
        REGISTRY
            .iter()
            .filter(|m| self.fired(m.opt))
            .map(|m| m.keyword)
            .collect()
    }
}

/// Why an optimization appears in a program's [`relationship_tree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptRole {
    /// It fired for this program.
    Fired,
    /// A `requires`-parent pulled in so a fired/preempted descendant has its
    /// dependency shown — it did not itself fire (e.g. `oracle` for a program
    /// that fires `unchecked` but produces no oracle-folded constant).
    Enabler,
    /// It had a candidate site but a higher-precedence optimization claimed it
    /// (a `preempts` loser) — it is enabled, and would fire if its winner were
    /// turned off. The "skipped because they don't play nice together" node.
    Preempted,
}

/// One node of a program's optimization relationship tree: an optimization that
/// is *in play* for that program, with its nesting depth and its edges.
#[derive(Debug, Clone)]
pub struct OptNode {
    /// The optimization.
    pub opt: Opt,
    /// Nesting depth (0 = root) for tree indentation — under its `requires`-parent
    /// or its per-program dependency parent.
    pub depth: usize,
    /// Why it is in play.
    pub role: OptRole,
    /// Whether any in-play optimization depends on this one (drives the chevron).
    pub has_children: bool,
    /// What it requires (static, from the registry).
    pub requires: Vec<Opt>,
    /// What it takes precedence over (static, from the registry).
    pub preempts: Vec<Opt>,
    /// The winners that beat this optimization on THIS program (from the trace).
    pub preempted_by: Vec<Opt>,
    /// The optimizations this one only fired because of, on THIS program (the
    /// emergent dependencies the all-on evaluation revealed — distinct from the
    /// universal `requires`).
    pub depends_on: Vec<Opt>,
}

/// The complete optimization chain for one program, derived deterministically
/// from a single all-optimizations-on evaluation plus the static registry graph —
/// no differential toggling in the hot path. `fired` is the set that fired;
/// `preempted` is the `(winner, loser)` BLOCKER pairs that occurred; `dependencies`
/// is the `(dependent, dep)` per-program DEPENDENCY pairs (one optimization only
/// fired because another was on) — all three come from the baked per-program graph
/// (`compile::optimization_graph`).
///
/// The in-play set is `fired ∪ {losers} ∪ {dependency endpoints} ∪ closure` over
/// the parent relation, where a node's parents are its static `requires` PLUS its
/// per-program dependencies: every opt that fired, every opt skipped because a
/// higher-precedence one claimed it, and the parents they hang under (so the tree
/// never orphans a child). Each node is placed by a depth-first walk along that
/// combined parent relation, parents before children. Closures over the fixed
/// 40-row registry — O(n²), and the same for identical input.
pub fn relationship_tree(
    fired: &[Opt],
    preempted: &[(Opt, Opt)],
    dependencies: &[(Opt, Opt)],
) -> Vec<OptNode> {
    let n = REGISTRY.len();
    let idx = |o: Opt| o as usize;

    // A node's parents for nesting: its static `requires` PLUS the per-program
    // dependencies it only fired because of.
    let parents_of = |y: usize| -> Vec<usize> {
        let yopt = REGISTRY[y].opt;
        let mut ps: Vec<usize> = REGISTRY[y].requires.iter().map(|&r| idx(r)).collect();
        for &(dep, on) in dependencies {
            if dep == yopt {
                ps.push(idx(on));
            }
        }
        ps.sort_unstable();
        ps.dedup();
        ps
    };

    // 1. The in-play set: seed with fired + blockers' losers + dependency
    //    endpoints, then pull in the transitive parent-closure.
    let mut want = vec![false; n];
    for &o in fired {
        want[idx(o)] = true;
    }
    for &(_, loser) in preempted {
        want[idx(loser)] = true;
    }
    for &(dep, on) in dependencies {
        want[idx(dep)] = true;
        want[idx(on)] = true;
    }
    loop {
        let mut changed = false;
        for i in 0..n {
            if want[i] {
                for p in parents_of(i) {
                    if !want[p] {
                        want[p] = true;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // 2. Role for each in-play opt: Fired wins over Preempted wins over Enabler
    //    (an opt that fired somewhere but was preempted on one instance is still
    //    Fired — the per-instance loss is shown as a live annotation, not a role).
    let fired_set: Vec<bool> = {
        let mut v = vec![false; n];
        for &o in fired {
            v[idx(o)] = true;
        }
        v
    };
    let loser_set: Vec<bool> = {
        let mut v = vec![false; n];
        for &(_, l) in preempted {
            v[idx(l)] = true;
        }
        v
    };
    let role_of = |i: usize| {
        if fired_set[i] {
            OptRole::Fired
        } else if loser_set[i] {
            OptRole::Preempted
        } else {
            OptRole::Enabler
        }
    };

    // 3. Depth-first order along the combined parent relation: roots are in-play
    //    opts with no in-play parent; children are in-play opts that hang under the
    //    current one (require it, or depend on it for this program).
    let parent_in_want = |i: usize| parents_of(i).iter().any(|&p| want[p]);
    let mut order: Vec<(usize, usize)> = Vec::new();
    let mut visited = vec![false; n];
    let roots: Vec<usize> = (0..n).filter(|&i| want[i] && !parent_in_want(i)).collect();
    let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|&i| (i, 0usize)).collect();
    while let Some((i, depth)) = stack.pop() {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        order.push((i, depth));
        let kids: Vec<usize> = (0..n)
            .filter(|&j| want[j] && !visited[j] && parents_of(j).contains(&i))
            .collect();
        for &k in kids.iter().rev() {
            stack.push((k, depth + 1));
        }
    }
    // Any in-play opt not reached by the walk (defensive; multi-parent diamonds
    // are already handled by `visited`) is appended at depth 0.
    for i in 0..n {
        if want[i] && !visited[i] {
            order.push((i, 0));
        }
    }

    // 4. Materialize the nodes.
    order
        .into_iter()
        .map(|(i, depth)| {
            let opt = REGISTRY[i].opt;
            let has_children = (0..n).any(|j| want[j] && parents_of(j).contains(&i));
            let preempted_by: Vec<Opt> = preempted
                .iter()
                .filter(|&&(_, l)| l == opt)
                .map(|&(w, _)| w)
                .collect();
            let depends_on: Vec<Opt> = dependencies
                .iter()
                .filter(|&&(d, _)| d == opt)
                .map(|&(_, x)| x)
                .collect();
            OptNode {
                opt,
                depth,
                role: role_of(i),
                has_children,
                requires: REGISTRY[i].requires.to_vec(),
                preempts: REGISTRY[i].preempts.to_vec(),
                preempted_by,
                depends_on,
            }
        })
        .collect()
}

/// A named optimization preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Everything on (the default).
    Speed,
    /// Drop optimizations that spend memory for speed.
    Memory,
    /// Drop optimizations that can emit `unsafe` Rust.
    Safety,
}

impl Profile {
    /// The (normalized) config this profile produces.
    pub fn config(self) -> OptimizationConfig {
        let mut cfg = OptimizationConfig::all_on();
        match self {
            Profile::Speed => {}
            Profile::Memory => {
                for m in REGISTRY {
                    if m.mem_class == MemClass::TradesMemForSpeed {
                        cfg.set(m.opt, false);
                    }
                }
            }
            Profile::Safety => {
                for m in REGISTRY {
                    if m.emits_unsafe {
                        cfg.set(m.opt, false);
                    }
                }
            }
        }
        cfg.normalize();
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_has_one_row_per_opt_in_order() {
        // Every row's opt must equal its index (discriminant == bit == index).
        for (i, m) in REGISTRY.iter().enumerate() {
            assert_eq!(m.opt as usize, i, "registry row {i} out of order: {:?}", m.opt);
        }
    }

    #[test]
    fn keywords_are_unique_lowercase_single_tokens() {
        let mut seen = HashSet::new();
        for m in REGISTRY {
            assert!(seen.insert(m.keyword), "duplicate keyword {}", m.keyword);
            assert_eq!(m.keyword, m.keyword.to_ascii_lowercase(), "keyword not lowercase: {}", m.keyword);
            assert!(!m.keyword.is_empty(), "empty keyword");
            assert!(!m.keyword.contains(char::is_whitespace), "keyword has whitespace: {}", m.keyword);
        }
    }

    #[test]
    fn requires_and_conflicts_reference_real_variants_no_cycle() {
        // All references resolve (they are typed `Opt`, so this is structural):
        // assert no opt requires itself and there is no trivial requires-cycle.
        for m in REGISTRY {
            assert!(!m.requires.contains(&m.opt), "{:?} requires itself", m.opt);
            assert!(!m.conflicts.contains(&m.opt), "{:?} conflicts with itself", m.opt);
            for &r in m.requires {
                // No 2-cycle: the required opt must not require this one back.
                assert!(!r.meta().requires.contains(&m.opt), "requires cycle: {:?} <-> {:?}", m.opt, r);
            }
        }
    }

    #[test]
    fn run_path_opt_costs_match_fixture() {
        // The RUN-path opts `optimize_for_run` actually gates, with the cost that
        // decides which hotness tier pays for them (HOTSWAP §3). Pinned as a fixture
        // so a misclassification is a hard failure, never silently absorbed.
        use OptCost::{Cheap, Heavy, Medium};
        let expect = |opt: Opt, c: OptCost| assert_eq!(opt.cost(), c, "{opt:?} cost");
        for (opt, c) in [
            (Opt::Inline, Cheap),
            (Opt::Tco, Cheap),
            (Opt::DeadCode, Cheap),
            (Opt::Specialize, Medium), // PE-light at T2 / PE-full at T3 via the iteration cap
            (Opt::Comptime, Medium),
            (Opt::LoopCse, Medium),
            (Opt::FloatStrength, Medium),
            (Opt::Cse, Medium),
            (Opt::Affine, Medium),
            (Opt::LoopHoist, Medium),
            (Opt::ClosedForm, Medium),
            (Opt::Fuse, Medium),
            (Opt::Oracle, Medium),
            (Opt::ElemType, Medium),
            (Opt::Unroll, Heavy),
            (Opt::Scalarize, Heavy),
            (Opt::Unfold, Heavy),
        ] {
            expect(opt, c);
        }
    }

    #[test]
    fn requires_cost_monotone() {
        // Well-formedness for the tiered optimizer: every opt's `requires` must cost
        // no more than the opt itself, so a tier that admits the dependent always
        // admits its dependencies (HOTSWAP §4.2). A failure is a HARD STOP — never
        // resolve it by lowering a dependency's cost.
        for m in REGISTRY {
            for &r in m.requires {
                assert!(
                    r.cost() <= m.opt.cost(),
                    "{:?} (cost {:?}) requires {:?} (cost {:?}) — violates cost monotonicity",
                    m.opt,
                    m.opt.cost(),
                    r,
                    r.cost()
                );
            }
        }
    }

    #[test]
    fn tier_budget_and_admits_truth_table() {
        use OptCost::{Cheap, Heavy, Medium};
        assert_eq!(Tier::T0.budget(), None);
        assert_eq!(Tier::T1.budget(), Some(Cheap));
        assert_eq!(Tier::T2.budget(), Some(Medium));
        assert_eq!(Tier::T3.budget(), Some(Heavy));

        let on = OptimizationConfig::all_on();
        // T0 optimizes nothing.
        assert!(!admits(&on, Tier::T0, Opt::Inline));
        // T1: Cheap only.
        assert!(admits(&on, Tier::T1, Opt::Inline)); // Cheap
        assert!(!admits(&on, Tier::T1, Opt::Cse)); // Medium
        assert!(!admits(&on, Tier::T1, Opt::Unroll)); // Heavy
        // T2: Cheap + Medium.
        assert!(admits(&on, Tier::T2, Opt::Inline));
        assert!(admits(&on, Tier::T2, Opt::Cse));
        assert!(!admits(&on, Tier::T2, Opt::Unroll));
        // T3: everything.
        assert!(admits(&on, Tier::T3, Opt::Inline));
        assert!(admits(&on, Tier::T3, Opt::Cse));
        assert!(admits(&on, Tier::T3, Opt::Unroll));
        // admits still respects cfg.is_on — a disabled opt never runs at any tier.
        let off = OptimizationConfig::all_on().disable(Opt::Inline);
        assert!(!admits(&off, Tier::T3, Opt::Inline));
        // Tiers order like their budgets.
        assert!(Tier::T0 < Tier::T1 && Tier::T1 < Tier::T2 && Tier::T2 < Tier::T3);
    }

    #[test]
    fn opt_count_matches_registry() {
        assert_eq!(OPT_COUNT, REGISTRY.len());
    }

    #[test]
    fn tier_thresholds_default_and_tier_for() {
        let t = TierThresholds::default();
        assert_eq!((t.t1, t.t2, t.t3), (8, 32, 100));
        assert_eq!(t.tier_for(0), Tier::T0);
        assert_eq!(t.tier_for(7), Tier::T0);
        assert_eq!(t.tier_for(8), Tier::T1);
        assert_eq!(t.tier_for(31), Tier::T1);
        assert_eq!(t.tier_for(32), Tier::T2);
        assert_eq!(t.tier_for(99), Tier::T2);
        assert_eq!(t.tier_for(100), Tier::T3);
        assert_eq!(t.tier_for(10_000), Tier::T3);
    }

    #[test]
    fn hotswap_mode_effective_tier() {
        let th = TierThresholds::default();
        let mk = |mode, force_tier| HotswapConfig {
            mode,
            thresholds: th,
            force_tier,
            pins: PinSet::none(),
        };
        // Eager: always T3 regardless of count.
        assert_eq!(mk(TierMode::Eager, None).effective_tier(0), Tier::T3);
        // Baseline: always T0.
        assert_eq!(mk(TierMode::Baseline, None).effective_tier(10_000), Tier::T0);
        // Tiered: climbs by hotness.
        let tiered = mk(TierMode::Tiered, None);
        assert_eq!(tiered.effective_tier(0), Tier::T0);
        assert_eq!(tiered.effective_tier(8), Tier::T1);
        assert_eq!(tiered.effective_tier(32), Tier::T2);
        assert_eq!(tiered.effective_tier(100), Tier::T3);
        // force_tier overrides everything (the determinism lever for tests).
        let forced = mk(TierMode::Tiered, Some(Tier::T2));
        assert_eq!(forced.effective_tier(0), Tier::T2);
        assert_eq!(forced.effective_tier(10_000), Tier::T2);
    }

    #[test]
    fn hotswap_from_spec_modes_and_force() {
        let th = TierThresholds::default();
        // LOGOS_HOTSWAP=off forces Eager.
        assert_eq!(
            HotswapConfig::from_spec(Some("off"), Some("baseline"), None, th, None).mode,
            TierMode::Eager
        );
        // Profiles.
        assert_eq!(HotswapConfig::from_spec(None, Some("eager"), None, th, None).mode, TierMode::Eager);
        assert_eq!(HotswapConfig::from_spec(None, Some("baseline"), None, th, None).mode, TierMode::Baseline);
        assert_eq!(HotswapConfig::from_spec(None, Some("tiered"), None, th, None).mode, TierMode::Tiered);
        // Default mode is Eager (the §12.2 ratchet — Tiered is opt-in until proven).
        assert_eq!(HotswapConfig::from_spec(None, None, None, th, None).mode, TierMode::Eager);
        assert_eq!(HotswapConfig::default().mode, TierMode::Eager);
        // force_tier parses number and word forms.
        assert_eq!(HotswapConfig::from_spec(None, None, Some("2"), th, None).force_tier, Some(Tier::T2));
        assert_eq!(HotswapConfig::from_spec(None, None, Some("veryhot"), th, None).force_tier, Some(Tier::T3));
        assert_eq!(HotswapConfig::from_spec(None, None, Some("nonsense"), th, None).force_tier, None);
    }

    #[test]
    fn run_tier_maps_mode_to_upfront_tier() {
        let th = TierThresholds::default();
        let mk = |mode, force_tier| HotswapConfig { mode, thresholds: th, force_tier, pins: PinSet::none() };
        // Eager pays the full optimizer upfront; Tiered/Baseline start at the baseline.
        assert_eq!(mk(TierMode::Eager, None).run_tier(), Tier::T3);
        assert_eq!(mk(TierMode::Tiered, None).run_tier(), Tier::T0);
        assert_eq!(mk(TierMode::Baseline, None).run_tier(), Tier::T0);
        // force_tier overrides the mode (test determinism).
        assert_eq!(mk(TierMode::Tiered, Some(Tier::T2)).run_tier(), Tier::T2);
        assert_eq!(mk(TierMode::Eager, Some(Tier::T1)).run_tier(), Tier::T1);
    }

    #[test]
    fn admits_pinned_overrides_cost_tier() {
        let on = OptimizationConfig::all_on();
        let mut pins = PinSet::none();
        pins.set(Opt::Unfold, Pin::Never);
        pins.set(Opt::Fuse, Pin::Eager);
        pins.set(Opt::Specialize, Pin::At(Tier::T2));
        let hs = HotswapConfig {
            mode: TierMode::Tiered,
            thresholds: TierThresholds::default(),
            force_tier: None,
            pins,
        };
        // Never: off at every tier.
        assert!(!admits_pinned(&on, &hs, Tier::T3, Opt::Unfold));
        // Eager: on as soon as warm (T1), despite Medium cost.
        assert!(admits_pinned(&on, &hs, Tier::T1, Opt::Fuse));
        // At(T2): off at T1, on at T2.
        assert!(!admits_pinned(&on, &hs, Tier::T1, Opt::Specialize));
        assert!(admits_pinned(&on, &hs, Tier::T2, Opt::Specialize));
        // Unpinned: cost-derived (matches `admits`).
        assert!(admits_pinned(&on, &hs, Tier::T1, Opt::Inline));
        assert!(!admits_pinned(&on, &hs, Tier::T1, Opt::Cse));
        // A disabled opt never runs regardless of pin.
        let off = OptimizationConfig::all_on().disable(Opt::Fuse);
        assert!(!admits_pinned(&off, &hs, Tier::T3, Opt::Fuse));
    }

    #[test]
    fn pin_spec_parses_keyword_value_pairs() {
        let mut pins = PinSet::none();
        apply_pin_spec(&mut pins, "specialize:eager, fuse:t1 ; unfold:never, bogus:t2, cse:nonsense");
        assert_eq!(pins.get(Opt::Specialize), Some(Pin::Eager));
        assert_eq!(pins.get(Opt::Fuse), Some(Pin::At(Tier::T1)));
        assert_eq!(pins.get(Opt::Unfold), Some(Pin::Never));
        assert_eq!(pins.get(Opt::Cse), None, "unparseable value is ignored");
    }

    #[test]
    fn decorate_source_inserts_file_level_decorators() {
        let src = "## To f (x: Int) -> Int:\n    Return x.\n\n## Main\nShow 1.\n";
        let d = decorate_source(src, &["scalarize", "unroll"]);
        assert!(d.contains("## No scalarize\n## No unroll\n## Main"), "got:\n{d}");
        assert!(d.starts_with("## To f"), "functions stay before the decorators");
        // Empty list is a no-op.
        assert_eq!(decorate_source(src, &[]), src);
        // A library with no `## Main` appends the decorators (EOF = file-level).
        let lib = "## To f (x: Int) -> Int:\n    Return x.\n";
        assert!(decorate_source(lib, &["cse"]).ends_with("## No cse\n"));
    }

    #[test]
    fn decorate_tiers_inserts_file_level_pins() {
        let src = "## To f (x: Int) -> Int:\n    Return x.\n\n## Main\nShow 1.\n";
        let d = decorate_tiers(src, &[("specialize", "eager"), ("unfold", "never")]);
        assert!(
            d.contains("## Tier specialize eager\n## Tier unfold never\n## Main"),
            "got:\n{d}"
        );
        assert!(d.starts_with("## To f"), "functions stay before the decorators");
        assert_eq!(decorate_tiers(src, &[]), src, "empty list is a no-op");
        let lib = "## To f (x: Int) -> Int:\n    Return x.\n";
        assert!(decorate_tiers(lib, &[("fuse", "t1")]).ends_with("## Tier fuse t1\n"));
    }

    #[test]
    fn by_keyword_round_trips() {
        for m in REGISTRY {
            assert_eq!(by_keyword(m.keyword), Some(m.opt));
            assert_eq!(by_keyword(&m.keyword.to_uppercase()), Some(m.opt));
        }
        assert_eq!(by_keyword("nonsense"), None);
    }

    #[test]
    fn all_on_enables_everything_all_off_nothing() {
        let on = OptimizationConfig::all_on();
        let off = OptimizationConfig::all_off();
        for m in REGISTRY {
            assert!(on.is_on(m.opt), "{:?} should be on", m.opt);
            assert!(!off.is_on(m.opt), "{:?} should be off", m.opt);
        }
        assert!(off.is_all_off());
        assert_eq!(OptimizationConfig::default(), on);
    }

    #[test]
    fn serde_round_trips() {
        let cfg = OptimizationConfig::all_on().disable(Opt::Scalarize).disable(Opt::Unroll);
        let json = serde_json::to_string(&cfg).unwrap();
        let back: OptimizationConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn normalize_disables_dependents_and_reports() {
        // Scalarize requires Cse; turning Cse off must disable Scalarize.
        let mut cfg = OptimizationConfig::all_on().disable(Opt::Cse);
        let report = cfg.normalize();
        assert!(!cfg.is_on(Opt::Scalarize), "Scalarize must follow Cse off");
        assert!(report
            .auto_disabled
            .contains(&(Opt::Scalarize, Reason::DependencyOff(Opt::Cse))));
    }

    #[test]
    fn normalize_oracle_closure() {
        // Oracle off must disable ElemType, OracleHints, Unchecked.
        let mut cfg = OptimizationConfig::all_on().disable(Opt::Oracle);
        cfg.normalize();
        assert!(!cfg.is_on(Opt::ElemType));
        assert!(!cfg.is_on(Opt::OracleHints));
        assert!(!cfg.is_on(Opt::Unchecked));
    }

    #[test]
    fn default_config_is_stable_under_normalize() {
        // The default (all-on) config has NO config-level conflicts: AoS/Interleave
        // is mutually exclusive with Unroll/Scalarize only at RUNTIME (the codegen
        // regime gate), never in the config, so all three stay enabled by default
        // and normalization is a no-op. Guards the regression where a config-level
        // Interleave⇄Unroll/Scalarize conflict silently disabled AoS by default.
        let mut cfg = OptimizationConfig::all_on();
        let report = cfg.normalize();
        assert!(report.is_empty(), "default config must normalize to a no-op, got {:?}", report);
        assert!(cfg.is_on(Opt::Interleave));
        assert!(cfg.is_on(Opt::Unroll));
        assert!(cfg.is_on(Opt::Scalarize));
        assert_eq!(cfg, OptimizationConfig::all_on());
    }

    #[test]
    fn safety_profile_drops_all_unsafe() {
        let cfg = Profile::Safety.config();
        for m in REGISTRY {
            if m.emits_unsafe {
                assert!(!cfg.is_on(m.opt), "Safety must disable unsafe-emitting {:?}", m.opt);
            }
        }
    }

    #[test]
    fn memory_profile_drops_mem_hogs_keeps_savers() {
        let cfg = Profile::Memory.config();
        assert!(!cfg.is_on(Opt::Unroll));
        assert!(!cfg.is_on(Opt::Scalarize));
        assert!(cfg.is_on(Opt::Narrow) || !OptimizationConfig::all_on().is_on(Opt::Unbox));
        assert!(cfg.is_on(Opt::Unbox), "Memory keeps memory-saving Unbox");
    }

    #[test]
    fn from_toggles_reports_forced_off() {
        let mut t = BTreeMap::new();
        t.insert("cse".to_string(), false);
        let (cfg, forced) = OptimizationConfig::from_toggles(&t);
        assert!(!cfg.is_on(Opt::Cse));
        assert!(!cfg.is_on(Opt::Scalarize));
        assert!(forced.contains(&Opt::Scalarize));
    }

    #[test]
    fn from_spec_master_off_is_all_off() {
        assert!(OptimizationConfig::from_spec(true, None, None).is_all_off());
        assert!(OptimizationConfig::from_spec(true, Some("safety"), Some("cse")).is_all_off());
    }

    #[test]
    fn from_spec_default_is_all_on() {
        assert_eq!(OptimizationConfig::from_spec(false, None, None), OptimizationConfig::all_on());
    }

    #[test]
    fn from_spec_off_list_disables_listed_keywords() {
        let cfg = OptimizationConfig::from_spec(false, None, Some("scalarize, unroll;nonsense"));
        assert!(!cfg.is_on(Opt::Scalarize));
        assert!(!cfg.is_on(Opt::Unroll));
        assert!(cfg.is_on(Opt::Memo), "unlisted opts stay on; unknown tokens ignored");
    }

    #[test]
    fn from_spec_off_list_normalizes_dependents() {
        // Disabling Oracle cascades to its dependents.
        let cfg = OptimizationConfig::from_spec(false, None, Some("oracle"));
        assert!(!cfg.is_on(Opt::Oracle));
        assert!(!cfg.is_on(Opt::Unchecked));
        assert!(!cfg.is_on(Opt::OracleHints));
        assert!(!cfg.is_on(Opt::ElemType));
    }

    #[test]
    fn from_spec_profiles_drop_the_right_opts() {
        let safety = OptimizationConfig::from_spec(false, Some("safety"), None);
        for m in REGISTRY {
            if m.emits_unsafe {
                assert!(!safety.is_on(m.opt), "Safety must drop unsafe-emitting {:?}", m.opt);
            }
        }
        let memory = OptimizationConfig::from_spec(false, Some("memory"), None);
        assert!(!memory.is_on(Opt::Unroll));
        assert!(memory.is_on(Opt::Unbox), "Memory keeps memory-saving opts");
    }

    // --- Toggle-linking: the requires graph drives both cascade directions ---

    #[test]
    fn disabling_any_requirement_cascades_to_dependents() {
        // The toggle-link contract, generic over the registry: for EVERY `requires`
        // edge, turning the requirement off (from all-on) must, after normalize,
        // also turn the dependent off. This is what makes "turn a parent off and
        // its children turn off" hold for every edge, not just the hand-picked ones.
        for m in REGISTRY {
            for &req in m.requires {
                let mut cfg = OptimizationConfig::all_on();
                cfg.set(req, false);
                cfg.normalize();
                assert!(
                    !cfg.is_on(m.opt),
                    "{:?} requires {:?}; disabling {:?} must cascade {:?} off",
                    m.opt, req, req, m.opt
                );
            }
        }
    }

    #[test]
    fn enable_with_requires_pulls_ancestors() {
        // Enabling a leaf from all-off must pull its whole requires-chain on, so the
        // act of turning a child on is not instantly undone by normalize.
        let mut cfg = OptimizationConfig::all_off();
        cfg.enable_with_requires(Opt::Interleave);
        assert!(cfg.is_on(Opt::Interleave));
        assert!(cfg.is_on(Opt::Scalarize), "Interleave needs Scalarize");
        assert!(cfg.is_on(Opt::Cse), "…which needs Cse");
        assert!(!cfg.is_on(Opt::Unroll), "unrelated opts stay off");
        assert!(!cfg.is_on(Opt::Oracle), "unrelated opts stay off");
        let report = cfg.normalize();
        assert!(report.is_empty(), "an enabled chain must be normalize-stable: {report:?}");
    }

    // --- relationship_tree: the deterministic per-program chain from one trace ---

    fn role_of(tree: &[OptNode], opt: Opt) -> Option<OptRole> {
        tree.iter().find(|n| n.opt == opt).map(|n| n.role)
    }
    fn depth_of(tree: &[OptNode], opt: Opt) -> usize {
        tree.iter().find(|n| n.opt == opt).unwrap().depth
    }
    fn pos_of(tree: &[OptNode], opt: Opt) -> usize {
        tree.iter().position(|n| n.opt == opt).unwrap()
    }

    #[test]
    fn relationship_tree_pulls_enabler_parents_for_orphan_children() {
        // coins-like: unchecked/oraclehints/elemtype fire, but their `requires`
        // parent oracle does not — it must still appear, as an Enabler, parent of
        // the children, so the tree is never orphaned.
        let fired = [Opt::Unchecked, Opt::OracleHints, Opt::ElemType];
        let tree = relationship_tree(&fired, &[], &[]);
        assert_eq!(role_of(&tree, Opt::Unchecked), Some(OptRole::Fired));
        assert_eq!(role_of(&tree, Opt::Oracle), Some(OptRole::Enabler));
        assert!(depth_of(&tree, Opt::Oracle) < depth_of(&tree, Opt::Unchecked));
        assert!(tree.iter().find(|n| n.opt == Opt::Oracle).unwrap().has_children);
    }

    #[test]
    fn relationship_tree_surfaces_preempted_losers() {
        // densemap fired and beat narrowmap: narrowmap must appear as a Preempted
        // node (the "skipped because they don't play nice" opt), annotated with the
        // winner that beat it, even though it never fired.
        let fired = [Opt::DenseMap];
        let preempted = [(Opt::DenseMap, Opt::NarrowMap)];
        let tree = relationship_tree(&fired, &preempted, &[]);
        let node = tree.iter().find(|n| n.opt == Opt::NarrowMap).expect("narrowmap present");
        assert_eq!(node.role, OptRole::Preempted);
        assert!(node.preempted_by.contains(&Opt::DenseMap));
        assert_eq!(role_of(&tree, Opt::DenseMap), Some(OptRole::Fired));
    }

    #[test]
    fn relationship_tree_nests_requires_chain_by_depth() {
        // cse → scalarize → interleave all fired: depths 0,1,2, parents first.
        let fired = [Opt::Cse, Opt::Scalarize, Opt::Interleave];
        let tree = relationship_tree(&fired, &[], &[]);
        assert_eq!(depth_of(&tree, Opt::Cse), 0);
        assert_eq!(depth_of(&tree, Opt::Scalarize), 1);
        assert_eq!(depth_of(&tree, Opt::Interleave), 2);
        assert!(pos_of(&tree, Opt::Cse) < pos_of(&tree, Opt::Scalarize));
        assert!(pos_of(&tree, Opt::Scalarize) < pos_of(&tree, Opt::Interleave));
    }

    #[test]
    fn relationship_tree_is_deterministic_and_only_in_play() {
        // The in-play set is exactly fired ∪ losers ∪ requires-closure — nothing
        // else — and the derivation is deterministic (same input → same output).
        let fired = [Opt::DenseMap, Opt::Unchecked];
        let preempted = [(Opt::DenseMap, Opt::NarrowMap)];
        let a = relationship_tree(&fired, &preempted, &[]);
        let b = relationship_tree(&fired, &preempted, &[]);
        let keys = |t: &[OptNode]| {
            t.iter().map(|n| (n.opt, n.depth, n.role)).collect::<Vec<_>>()
        };
        assert_eq!(keys(&a), keys(&b), "derivation must be deterministic");
        let in_play: std::collections::BTreeSet<Opt> = a.iter().map(|n| n.opt).collect();
        let expected: std::collections::BTreeSet<Opt> =
            [Opt::DenseMap, Opt::NarrowMap, Opt::Unchecked, Opt::Oracle].into_iter().collect();
        assert_eq!(in_play, expected, "only in-play opts appear");
    }

    #[test]
    fn relationship_tree_nests_by_per_program_dependencies() {
        // A per-program dependency (dependent, dep) nests the dependent under the
        // dep even with no static `requires` edge — e.g. dead-code elimination only
        // fired because scalarization produced the dead code. Both fired.
        let fired = [Opt::Scalarize, Opt::DeadCode, Opt::Cse];
        let deps = [(Opt::DeadCode, Opt::Scalarize)];
        let tree = relationship_tree(&fired, &[], &deps);
        // deadcode hangs under scalarize (deeper), and records the dependency.
        assert!(depth_of(&tree, Opt::DeadCode) > depth_of(&tree, Opt::Scalarize));
        assert!(pos_of(&tree, Opt::Scalarize) < pos_of(&tree, Opt::DeadCode));
        let dc = tree.iter().find(|n| n.opt == Opt::DeadCode).unwrap();
        assert!(dc.depends_on.contains(&Opt::Scalarize));
        assert_eq!(dc.role, OptRole::Fired);
        // scalarize has children now (deadcode depends on it).
        assert!(tree.iter().find(|n| n.opt == Opt::Scalarize).unwrap().has_children);
        // static requires still compose: scalarize under cse.
        assert!(depth_of(&tree, Opt::Scalarize) > depth_of(&tree, Opt::Cse));
    }
}
