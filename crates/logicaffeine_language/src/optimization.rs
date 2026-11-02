//! The registry of compiler optimization toggles.
//!
//! Every optimization the compiler can perform has exactly one row in
//! [`REGISTRY`], naming it, giving it a human label and a `## No <Keyword>`
//! decorator keyword, recording which execution paths it touches, whether it
//! emits `unsafe` Rust, how it trades memory for speed, and its dependencies
//! and conflicts with other optimizations.
//!
//! A live choice of which optimizations are on is an [`OptimizationConfig`] —
//! an [`OptBits`] set, one bit per [`Opt`]. The default is "everything on"
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
pub const OPT_COUNT: usize = Opt::HintVersion as usize + 1;

/// The 64-bit words an [`OptBits`] set holds: one bit per registered optimization.
pub const OPT_WORDS: usize = OPT_COUNT.div_ceil(64);

/// A set of optimizations, one bit per [`Opt`] (its discriminant), in as many 64-bit words as the registry
/// needs. [`OptimizationConfig`] keeps the enabled set in one and [`FiredOptimizations`] the fired set, in the
/// same layout, so the two relate bit for bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct OptBits([u64; OPT_WORDS]);

impl OptBits {
    /// The empty set.
    pub const EMPTY: OptBits = OptBits([0; OPT_WORDS]);

    const fn place(opt: Opt) -> (usize, u64) {
        (opt as usize / 64, 1u64 << (opt as usize % 64))
    }

    /// Whether `opt` is in the set.
    #[inline]
    pub const fn contains(&self, opt: Opt) -> bool {
        let (word, bit) = Self::place(opt);
        self.0[word] & bit != 0
    }

    /// The set with `opt` added.
    #[inline]
    pub const fn with(mut self, opt: Opt) -> Self {
        let (word, bit) = Self::place(opt);
        self.0[word] |= bit;
        self
    }

    /// The set with `opt` removed.
    #[inline]
    pub const fn without(mut self, opt: Opt) -> Self {
        let (word, bit) = Self::place(opt);
        self.0[word] &= !bit;
        self
    }

    /// The optimizations in either set.
    pub const fn union(mut self, other: OptBits) -> Self {
        let mut i = 0;
        while i < OPT_WORDS {
            self.0[i] |= other.0[i];
            i += 1;
        }
        self
    }

    /// The optimizations in both sets.
    pub const fn intersection(mut self, other: OptBits) -> Self {
        let mut i = 0;
        while i < OPT_WORDS {
            self.0[i] &= other.0[i];
            i += 1;
        }
        self
    }

    /// The optimizations in this set and not in `other`.
    pub const fn difference(mut self, other: OptBits) -> Self {
        let mut i = 0;
        while i < OPT_WORDS {
            self.0[i] &= !other.0[i];
            i += 1;
        }
        self
    }

    /// Whether the set is empty.
    pub const fn is_empty(&self) -> bool {
        let mut i = 0;
        while i < OPT_WORDS {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }
}

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
        Self {
            t1: 8,
            t2: 32,
            t3: 100,
        }
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
                th = Self {
                    t1: parts[0],
                    t2: parts[1],
                    t3: parts[2],
                };
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
        Self {
            pins: [None; OPT_COUNT],
        }
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
        HotswapConfig {
            mode,
            thresholds,
            force_tier,
            pins,
        }
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

/// Parse one pin value: `eager`, `never`, or a tier token (see `parse_tier`). The
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[repr(u8)]
pub enum Opt {
    /// Automatic memoization of pure functions.
    Memo = 0,
    /// Tail-call optimization (self-tail-recursion → loop).
    Tco = 1,
    /// Peephole rewrites (swap, vec-fill, for-range, …).
    Peephole = 2,
    /// Read-only / mutable borrow inference: `&[T]` / `&mut [T]` / `&str` / `&Enum` params, and a loop that reads a
    /// sequence it never writes through a shared borrow instead of a copy.
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
    /// Sparse conditional constant propagation (Wegman–Zadeh): folds constants
    /// AND prunes unreachable branches in one lattice sweep, catching constants
    /// the iterated fold→propagate→deadcode pipeline structurally misses.
    Sccp = 40,
    /// Integer induction-variable strength reduction: a counted loop's affine
    /// `coeff*j + base` index (base loop-invariant) becomes a seeded accumulator
    /// advanced by `coeff*step`, removing the per-iteration multiply. Exact
    /// (no runtime guard).
    IntStrength = 41,
    /// Aggregate-element register promotion: an invariant-indexed read-modify-
    /// write array element is loaded into a scalar once, updated in a register
    /// across the loop, and written back once — store-to-load forwarding LICM
    /// cannot do (the element is written).
    RegPromote = 42,
    /// Loop interchange: reorder two adjacent nested counted loops to put the
    /// unit-stride access innermost (the `ijk`→`ikj` matmul recipe). Fires only
    /// on integer commutative-reduction nests, where any permutation is
    /// bit-identical, and only toward better locality.
    Interchange = 43,
    /// Bounded-accumulator overflow-guard elision: version a hot integer loop that
    /// provably stays `i64`-safe under a compile-time input bound (interval
    /// inductive-invariant check), so its fast branch lowers to bare `i64` instead of
    /// a per-iteration overflow-promotion guard; a huge input takes the unchanged
    /// promoting branch (never an abort).
    AccumBound = 44,
    /// Div-mod fusion: rustc/LLVM does not CSE `a/b` against `a%b` (distinct `sdiv`/`srem`,
    /// each its own magic-multiply for a constant divisor), so a loop computing both pays
    /// two multiplies where gcc keeps the quotient and derives `r = a - q*b`. When one
    /// statement holds both with the same side-effect-free operands, bind `q = a/b` once
    /// and rewrite `a%b -> a - q*b` — the single-division shape, propagating proven-raw.
    DivModFuse = 45,
    /// Range-aware helper specialization: a counted-accumulator loop calls pure integer
    /// helpers (`divModQ`/`divModM`/…) whose bodies branch on argument SIGN, but the loop
    /// counter is provably `>= 0` and the divisor a positive constant, so those branches
    /// are dead. This symbolically inlines the helper chain under the counter's range and
    /// folds the dead branches, leaving bare arithmetic the div-mod fuser + accumulator
    /// versioner then lower to a hand-C-quality loop. A call that isn't fully provable is
    /// left untouched.
    SpecializeLoop = 46,
    /// Bignum by-reference arithmetic (clone elision): a reused promoted (`LogosInt`) operand in
    /// a `+`/`-`/`*` is passed by VALUE to `logos_*_exact`, so a variable read twice (or read then
    /// reassigned) `.clone()`s its heap magnitude every iteration. `LogosInt::{add,sub,mul}` are
    /// already `(&self, &LogosInt)`, and `logos_mul_exact(a,b)` IS `a.into().mul(&b.into())`, so
    /// the by-ref method is byte-identical minus the clone. Reused bignum vars are borrowed.
    BignumByRef = 47,
    /// In-place bignum scalar multiply: `Set acc to acc * <i64>` on a promoted accumulator (dead
    /// last-use, native-int scalar) mutates `acc`'s heap `Vec` in place (`acc.mul_assign_i64(k)`)
    /// instead of allocating a fresh product each iteration (the factorial hot loop).
    BignumInPlace = 48,
    /// Native-width bit intrinsics: the stdlib `rotateLeft`/`rotateRight`/`reverseBits`/
    /// `reverseBytes` with a compile-time native word width (8/16/32) lower to a single Rust
    /// intrinsic (`(x as uW).rotate_left(n)` / `.reverse_bits()` / `.swap_bytes()`) — one CPU
    /// instruction (ROL/ROR, bit-reverse, BSWAP) — instead of a masked loop LLVM cannot recognize.
    /// Fires at the call site AND on the const-specialized masked-rotate form the optimizer folds to.
    BitIntrinsics = 49,
    /// Text-bytes zero-copy view: a `Let sb be text_bytes(s)` binding used ONLY for reads
    /// (`item i of sb`, `length of sb`, `for b in sb`) lowers to a borrowed `s.as_bytes()`
    /// (`&[u8]`) instead of materializing a `Vec<i64>` byte-list (8x blow-up + `Rc<RefCell>`
    /// box + a `.borrow()` and bounds check per byte). Reads cast the `u8` back to `i64`.
    /// The whole text stdlib funnels every operation through `text_bytes`, so this removes
    /// the dominant text-cluster read cost. Read-side half of the byte-list elimination.
    TextBytesView = 50,
    /// String accumulator reuse: `Set X to X combined with Y` (X a mutable owned `String`,
    /// Y not mentioning X) mutates X's buffer in place via `X.push_str(&Y)` — amortized O(1)
    /// growth — instead of `X = format!("{}{}", X, Y)`, which reallocates and copies the whole
    /// accumulator every iteration (turning `repeatText` and any concat-accumulator loop into
    /// O(N^2)). Fires on the SHAPE (self-append concat, target leftmost) so a user's own
    /// accumulator loop benefits identically. General FBIP write-side string reuse.
    StringAccumReuse = 51,
    /// SIMD reduction: an integer accumulator loop summing `trunc(<liftable float in the counter>)`
    /// over a counted range is lowered to an AVX2 four-lane `f64` reduction — the transcendental is
    /// translated to `_mm256` intrinsics (branches → `blendv`), byte-identical to the scalar path
    /// per lane, and the `i64` sum is associative so the reduce equals the sequential sum. Beats
    /// scalar libm across the loop where LLVM refuses to auto-vectorize the `f64`→`i64` cast.
    SimdReduce = 52,
    /// Text write-side byte builder: a `Let out be a new Seq of Int` used ONLY as a byte builder
    /// (`Push <i64> to out`) and consumed by `text_from_bytes(out)` lowers to a bare `Vec<u8>`
    /// (`out.push((e) as u8)`) plus a direct `String::from_utf8(out)` — eliminating the
    /// `Rc<RefCell<Vec<i64>>>` output (8x memory + per-push `borrow_mut` + no presize) AND the
    /// separate `i64`->`u8` rebuild pass that `text_from_bytes` performed. Write-side half of the
    /// byte-list elimination (twin of `TextBytesView`); fires on the SHAPE so any user byte-builder
    /// benefits identically.
    TextFromBytesBuilder = 53,
    /// Text zero-copy sub-slice return: a fn that returns a CONTIGUOUS sub-slice of a borrowed Text
    /// param (compute lo/hi by scans, then copy `item lo..hi of text_bytes(s)` into a builder and
    /// `text_from_bytes(out)`) lowers to a borrowed `&'a str` return (`fn f<'a>(s: &'a str) -> &'a str
    /// { &s[lo-1..hi] }`) when EVERY caller uses the result read-only (never binds/returns/pushes it).
    /// Matches Rust `str::trim`/`strip_prefix` — an O(1) fat-pointer slice instead of an O(n) copy.
    /// Requires the Text param already borrowed (Opt::Borrow); PREEMPTS the write-side builder on the
    /// terminal Return (the slice supersedes building a Vec<u8>). The only ASYMPTOTIC text win.
    TextSliceReturn = 54,
    /// String concat reuse (Let-form): `Let X be A + B...` where A is a fresh OWNED `String` (a call
    /// returning `String`) reuses A's buffer — `let mut X = A; X.push_str(&B)...` — instead of
    /// `format!("{}{}", A, B)`, which allocates a NEW String and copies every operand. The `Let`-form
    /// twin of `StringAccumReuse` (which handles `Set X to X + Y`). Construction-shared across the whole
    /// text benchmark cluster (`repeatText(..) + base`), so it lifts trim/upper/lower/… toward Rust's
    /// buffer-reuse (`"a".repeat(k) + &base`). Fires on the SHAPE; a user's `A + B` benefits identically.
    StringConcatReuse = 55,
    /// String repeat-idiom: `Let out be ""` + a counted loop whose sole body is `Set out to out + chunk`
    /// (chunk loop-invariant) lowers to `let out = chunk.repeat(count)` — one presized alloc + memset,
    /// matching Rust's `chunk.repeat(n)`, instead of `count` un-presized `push_str`es. Fires on the
    /// SHAPE (a counted append of a fixed chunk), so `repeatText` and any user repeat-loop benefit.
    StringRepeatLoop = 56,
    /// ASCII-class bitmask: a pure `Int -> Bool` predicate whose true-set lies entirely in the byte
    /// domain `[0,256)` (the `charIs*` family — range/equality checks over `c`) lowers to a branchless
    /// 256-bit mask test `(0<=c<256) && (MASK[(c>>6)&3] >> (c&63)) & 1`, built by CTFE-evaluating the
    /// predicate over `0..256`. Beats C's `ctype` table (the mask is an immediate/static — no memory
    /// load). Fires on ANY such predicate, so a user's classifier collapses the same way.
    AsciiClassMask = 57,
    /// Power-of-two modulo/division strength reduction: `x % 2^k` → `x & (2^k-1)` and `x / 2^k` →
    /// `x >> k` when `x` is proven non-negative (a loop counter from a `>= 0` start with a positive
    /// step). rustc keeps sign-corrected `%`/`/` for a runtime i64 (several instructions that also
    /// block auto-vectorization); the mask/shift is one op. Sound (for `x >= 0` they agree), general
    /// (every ring-buffer/hash/index loop), and the measured lever that lets loops beat C.
    StrengthReducePow2 = 58,
    /// Branchless accumulate (predicate-guarded increment if-conversion): `if <cond> { X = X + k }`
    /// (no else, X a plain integer accumulator, k a literal) → `X = X + (<cond> as i64) * k`. Removes
    /// the data-dependent branch that blocks the loop vectorizer. Byte-identical (`cond ∈ {0,1}`, and
    /// `cond` is evaluated exactly once either way). Pairs with a wide accumulator to beat C's count.
    BranchlessAccumulate = 59,
    /// Wide (i128) accumulator: a `Let X be 0` counting accumulator incremented ONLY by `+1` (guarded
    /// or not) in a SINGLE counted loop with an i64 bound, read ONLY by `Show X`, widens to `i128` for
    /// the loop — the plain i128 add has no overflow branch (unlike `logos_add_i64`), so the count
    /// vectorizes. SOUND: X ≤ trip-count ≤ i64::MAX, so it fits i64 (narrow `as i64` at the sink is
    /// exact) and i128 can't overflow. The lever that beats C's ctype-count loop (43ms vs 49ms).
    WideAccumulator = 60,
    /// Radix digit buffer (itoa): the base-N integer→text idiom (generate digits LSB-first into a
    /// boxed `Seq`, reverse into a second boxed `Seq`, `text_from_bytes`) lowers to ONE fixed-size
    /// stack `[u8; 64]` written BACK-TO-FRONT (the back-to-front write is the reversal), then one
    /// `String` from the buffer tail — no heap Seq, no reversal copy. SOUND only when the loop
    /// counter is a native i64 (never bignum-promotable) so the digit count is bounded ≤64. Beats
    /// Go's strconv (measured 1.30x). Preempts `TextFromBytesBuilder` (it eliminates the `out`
    /// byte-builder that pass would otherwise lower).
    RadixDigitBuffer = 61,
    /// Byte-slice return (the bytes-trim family): a fn taking a `Seq of Int` param that returns a
    /// CONTIGUOUS `lo..=hi` sub-slice (scan, then copy `item lo..hi of xs` into `out`, `Return out`)
    /// lowers to a borrowed `&'a [i64]` (`fn f<'a>(xs: &'a [i64], …) -> &'a [i64] { … &xs[lo-1..hi] }`)
    /// — Go's O(1) `bytes.Trim` reslice instead of the O(n) build-copy. The bytes twin of
    /// TextSliceReturn (a readonly Seq param is already `&[i64]`, so the sub-slice shares its
    /// lifetime). Caller-gate drops any fn whose result escapes. Preempts OracleHints + Unchecked
    /// (eliminates the indexed-copy loop those bounds-check opts would optimize). Requires Borrow.
    ByteSliceReturn = 62,
    /// Loop tiling: a band of counted loops runs in square tiles for cache locality, only where the dependence
    /// test proves no run can tell the order apart (`optimize::dependence`), the nest versioned on the
    /// preconditions that make it so (`optimize::version`). The order of every item's own updates is kept, so a
    /// Real accumulation stays bit for bit.
    Tile = 63,
    /// Masked-chain narrowing: an integer chain of `+`, `-`, `*`, `&`, `|`, `^` and constant `<<` under a low-bit
    /// mask `& (2^k - 1)`, `k <= 32`, whose every node the range oracle proves stays an i64, computes in 32-bit
    /// wrapping arithmetic. That is exact: those operations' low `k` bits depend only on their operands' low `k`
    /// bits. Narrowed, a masked LCG's unrolled steps become independent multiplies (by a, a², a³, a⁴) instead of one
    /// serial 64-bit multiply chain.
    MaskNarrow = 64,
    /// A remainder by a constant `m > 0` of a value the range oracle bounds to `[0, 2m)`: `x - m` when `x >= m`, and
    /// `x` otherwise, one compare and one subtract in place of a multiply-shift remainder (a value bounded below `m`
    /// is its own remainder). A loop that carries a residue (`b = (a + b) % M`) has its remainder on the critical
    /// path, so the subtraction bounds the loop instead of the division. On the bytecode path the VM compiler decides
    /// it from the same oracle: a value below `m` compiles as itself, and one below `2m` keeps its magic remainder
    /// marked `wrap`, which the VM and the C walker compute as the compare and subtract.
    ModWrap = 65,
    /// Branchless select: `If c: Set x to a. Otherwise: Set x to b.`, where both arms are integer arithmetic that
    /// cannot trap and at least one keeps its exactness behind a guarded dual, computes both arms and lets `c` choose
    /// (a conditional move, not a branch on `c`). The arms' guards are tested on their own before either arm; when one
    /// fails, the original branch runs with its exact arithmetic. A data-dependent `c` (collatz's parity) no longer
    /// mispredicts.
    BranchlessSelect = 66,
    /// Variant boxing: a variant of a recursive enum that holds the enum itself keeps all its fields in one box (the
    /// variant's payload, a struct of them), where each recursive field had a box of its own; a variant with no
    /// fields is a value with no allocation. A tree of `n` leaves allocates once per node rather than once per child,
    /// a leaf included: OCaml's representation of a datatype.
    VariantBox = 67,
    /// Bit packing: a local `Seq of Bool` used only through item reads, item stores, pushes and its length is held one
    /// bit per item (`LogosBits`), an eighth of the bytes of a `Vec<bool>`; a sieve's table of 10^8 flags fits the
    /// cache its passes streamed through.
    BitPack = 68,
    /// Fold/build fusion: a function that only takes a datatype apart, applied to what a function builds of it, is
    /// one function that never builds it (the builder's recursion, each construction replaced by the consumer's arm for
    /// that variant). `check(bottomUpTree(d))` builds no tree, and the fused recursion is a shape the compiler
    /// collapses.
    FoldFusion = 69,
    /// Sorting-nest collapse: a nest of passes of adjacent compare-exchanges over a sequence of Ints (bubble sort)
    /// that has enough passes to sort its window is one in-place heap sort of the window, behind a hoisted check
    /// that the passes suffice and the window lies inside the sequence; `n(n-1)/2` comparisons become `O(n log n)`.
    SortNest = 70,
    /// Divisor sieve: a search, for each number of a counted range, for a divisor no greater than its square root
    /// (trial division) reads one sieve of the range, marked before the loop, behind a guard on the sieve's size;
    /// `O(n sqrt n)` divisions become `O(n log log n)` marks.
    DivisorSieve = 71,
    /// Trace memoization: a pure iteration run from every start of a counted range, whose only effect adds a constant
    /// to an accumulator, keeps what each finished start added in a table, and a trace that reaches a finished start
    /// adds that start's sum and stops.
    TraceMemo = 72,
    /// Probe fusion: `If m contains k:` over a branch that reads `item k of m` reads the value once at the top of
    /// the branch, and the test and the read are one lookup.
    ProbeFuse = 73,
    /// Strided fill: a loop that stores one value into a bit-packed `Seq of Bool` at a fixed stride, every store
    /// proven to name an item and the stride at least 1, is one fill that writes each word once (a sieve crossing
    /// off a small prime's multiples sets dozens of bits per word).
    StrideFill = 74,
    /// Hint versioning: the innermost loop whose bounds hints the Oracle could not discharge runs as two copies, one
    /// that assumes every hint, chosen where every hint holds, and one that assumes none.
    HintVersion = 75,
}

impl Opt {
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
    /// Whether it changes the program's algorithm — see [`OptWork`].
    pub work: OptWork,
}

/// What an optimization does to the WORK a program performs, as opposed to how fast it performs it.
///
/// Benchmarks compare languages running the same algorithm. An optimization that replaces the algorithm makes the
/// comparison a different one — a fair demonstration of the compiler, and one a reader must be told about: every
/// benchmark a [`OptWork::Less`] optimization fires on carries a note saying what the compiler did
/// (`benchmarks/lib/benchmarks.tsv`, locked by `benchmark_collapse_lock`), and is left out of the same-algorithm
/// geometric mean. Every registry row must say which it is, so a new pass cannot change a result unannounced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptWork {
    /// The same algorithm in cheaper machine code: the program's loops, recursion and data structures all still run.
    Same,
    /// A different, cheaper algorithm when it fires: memoized recursion, a loop or recursion replaced by its closed
    /// form, a symmetric search cut in half, a map replaced by an array its keys index, a loop replaced by one
    /// instruction, work moved to compile time.
    Less,
}

use path::{AOT, CODEGEN, JIT, RUN, VM};
use MemClass::{Neutral, SavesMem, TradesMemForSpeed};
use OptCost::{Cheap, Heavy, Medium};
use Scope::{Both, ProgramOnly};

/// The complete optimization registry: exactly one row per [`Opt`], in
/// discriminant order. Earlier rows win conflicts against later rows.
pub static REGISTRY: &[OptMeta] = &[
    OptMeta { opt: Opt::Memo, keyword: "memo", label: "Memoization", group: "Inlining & calls", default_on: true, paths: AOT | RUN | CODEGEN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Less },
    OptMeta { opt: Opt::Tco, keyword: "tco", label: "Tail-call optimization", group: "Inlining & calls", default_on: true, paths: AOT | RUN | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::Memo], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Peephole, keyword: "peephole", label: "Peephole rewrites", group: "Peephole", default_on: true, paths: AOT | CODEGEN, emits_unsafe: true, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    // Borrow preempts Tco: a `&mut [T]` in-place recursion (quicksort's
    // consume-alias shape) keeps its plain recursive calls — the pair-TCE
    // rewrite would reassign the borrowed param.
    OptMeta { opt: Opt::Borrow, keyword: "borrow", label: "Borrow inference", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::Tco], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Specialize, keyword: "specialize", label: "Partial evaluation", group: "Inlining & calls", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Comptime, keyword: "comptime", label: "Compile-time evaluation (CTFE)", group: "Inlining & calls", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[Opt::Supercompile], scope: Both, work: OptWork::Less },
    OptMeta { opt: Opt::Inline, keyword: "inline", label: "Function inlining", group: "Inlining & calls", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Unfold, keyword: "unfold", label: "Recursion unrolling", group: "Inlining & calls", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Heavy, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Defunctionalize, keyword: "defunctionalize", label: "Defunctionalization", group: "Inlining & calls", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::LoopHoist, keyword: "loophoist", label: "Loop-invariant code motion", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Unroll, keyword: "unroll", label: "Loop unrolling", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Heavy, requires: &[], conflicts: &[], preempts: &[Opt::Interleave], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::LoopSplit, keyword: "loopsplit", label: "Loop index-set splitting", group: "Loops", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    // NOTE: ClosedForm naturally preempts Memo (a recursion it solves has no
    // self-calls left when the run-path memoizer runs right after it, so Memo
    // declines), but NO witness program exhibits both on the same function, so
    // the edge is left UNDECLARED — the corpus-observed-edges lock forbids
    // declaring a precedence nothing exercises (it would be a phantom).
    OptMeta { opt: Opt::ClosedForm, keyword: "closedform", label: "Closed-form loop recognition", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[Opt::Peephole], scope: Both, work: OptWork::Less },
    OptMeta { opt: Opt::Fuse, keyword: "fuse", label: "Deforestation (stream fusion)", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::LoopCse, keyword: "loopcse", label: "Loop-carried CSE", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Cse, keyword: "cse", label: "Common-subexpression elimination (GVN)", group: "Redundancy", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::DeadCode, keyword: "deadcode", label: "Dead-code elimination", group: "Redundancy", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Scalarize, keyword: "scalarize", label: "Array scalarization (SROA)", group: "Arrays & memory", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Heavy, requires: &[Opt::Cse], conflicts: &[], preempts: &[Opt::HoistBorrows, Opt::Interleave, Opt::Unbox], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Affine, keyword: "affine", label: "Affine array → closed form", group: "Arrays & memory", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Medium, requires: &[Opt::Unbox], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    // AoS interleaving is mutually exclusive with Unroll/Scalarize only at RUNTIME
    // (the codegen regime gate skips it when a co-indexed array has any constant
    // index access — i.e. once it has been unrolled/scalarized), NOT at config
    // level. All three stay enabled by default; the gate arbitrates per program.
    OptMeta { opt: Opt::Interleave, keyword: "interleave", label: "Array-of-structs interleaving", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Heavy, requires: &[Opt::Scalarize], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::Unbox, keyword: "unbox", label: "De-Rc (Vec instead of Rc<RefCell>)", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::HoistBorrows, keyword: "hoistborrows", label: "Borrow hoisting", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::HintVersion], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::Narrow, keyword: "narrow", label: "i32 sequence narrowing (codegen)", group: "Number representation", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[Opt::Unbox], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::NarrowVm, keyword: "narrowvm", label: "i32 sequence narrowing (VM)", group: "Number representation", default_on: true, paths: VM | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::NarrowMap, keyword: "narrowmap", label: "i32 map-key narrowing", group: "Number representation", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::DenseMap, keyword: "densemap", label: "Dense direct-addressed map", group: "Number representation", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::NarrowMap], scope: ProgramOnly, work: OptWork::Less },
    OptMeta { opt: Opt::ElemType, keyword: "elemtype", label: "Element-type narrowing", group: "Number representation", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: SavesMem, cost: Medium, requires: &[Opt::Oracle], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::FastDiv, keyword: "fastdiv", label: "Magic-reciprocal division", group: "Number representation", default_on: true, paths: AOT | VM | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::FloatStrength, keyword: "floatstrength", label: "Float induction strength reduction", group: "Number representation", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Oracle, keyword: "oracle", label: "Abstract interpretation (oracle facts)", group: "Bounds & checks", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::OracleHints, keyword: "oraclehints", label: "Oracle bounds-check guards", group: "Bounds & checks", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[Opt::Oracle], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Unchecked, keyword: "unchecked", label: "Oracle-proven unchecked indexing", group: "Bounds & checks", default_on: true, paths: AOT | JIT | CODEGEN, emits_unsafe: true, mem_class: Neutral, cost: Medium, requires: &[Opt::Oracle], conflicts: &[], preempts: &[Opt::HintVersion], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Simd, keyword: "simd", label: "SIMD search kernels", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: true, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::BranchlessAccumulate], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::Cascade, keyword: "cascade", label: "Cascade folding", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::IndexString, keyword: "indexstring", label: "Indexed string search", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::StringAccumReuse], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::CapScale, keyword: "capscale", label: "Capacity-scaling buffer fill", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::Symmetry, keyword: "symmetry", label: "Symmetry breaking", group: "Search-space", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Heavy, requires: &[Opt::Specialize], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Less },
    OptMeta { opt: Opt::Popcount, keyword: "popcount", label: "Popcount leaf collapse", group: "Search-space", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Less },
    OptMeta { opt: Opt::Saturate, keyword: "saturate", label: "Equality saturation (e-graph)", group: "Search-space", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Heavy, requires: &[], conflicts: &[], preempts: &[Opt::BignumInPlace], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Supercompile, keyword: "supercompile", label: "Supercompilation", group: "Search-space", default_on: true, paths: AOT, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Heavy, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Sccp, keyword: "sccp", label: "Sparse conditional constant propagation", group: "Redundancy", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[Opt::Supercompile], scope: Both, work: OptWork::Same },
    // IntStrength and LoopSplit COMPOSE, not preempt: on the AOT path IntStrength
    // DEFERS the square matmul nest to the codegen tiler (LoopSplit family), whose
    // L1 cache tiling beats removing the index multiply and needs the nest's raw
    // shape (`int_induction_sr::is_square_triple_nest`). No precedence edge — both
    // fire, on the loops each is best for.
    OptMeta { opt: Opt::IntStrength, keyword: "intstrength", label: "Integer induction strength reduction", group: "Number representation", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::RegPromote, keyword: "regpromote", label: "Aggregate-element register promotion", group: "Arrays & memory", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::Interchange, keyword: "interchange", label: "Loop interchange", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Heavy, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::AccumBound, keyword: "accumbound", label: "Bounded-accumulator overflow-guard elision", group: "Loops", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::DivModFuse, keyword: "divmodfuse", label: "Div-mod fusion (shared quotient for a/b + a%b)", group: "Loops", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::SpecializeLoop, keyword: "specializeloop", label: "Range-aware helper specialization in counted loops", group: "Loops", default_on: true, paths: AOT | RUN, emits_unsafe: false, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::BignumByRef, keyword: "bignumbyref", label: "Bignum by-ref arithmetic (clone elision)", group: "Bignum", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::BignumInPlace, keyword: "bignuminplace", label: "In-place bignum scalar multiply (mul_assign_i64)", group: "Bignum", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::BignumByRef], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::BitIntrinsics, keyword: "bitintrinsics", label: "Native-width bit intrinsics (rotate/reverse/byteswap)", group: "Bits", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::TextBytesView, keyword: "textbytesview", label: "Text-bytes zero-copy &[u8] view (no Vec<i64> byte-list)", group: "Text", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::StringAccumReuse, keyword: "stringaccumreuse", label: "String accumulator in-place push_str (no per-concat realloc)", group: "Text", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::SimdReduce, keyword: "simdreduce", label: "AVX2 SIMD reduction of a transcendental accumulator loop", group: "Math", default_on: true, paths: AOT | CODEGEN, emits_unsafe: true, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::TextFromBytesBuilder, keyword: "textfrombytesbuilder", label: "Text write-side byte builder (Vec<u8>, no Vec<i64> + no rebuild)", group: "Text", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::TextSliceReturn, keyword: "textslicereturn", label: "Text zero-copy sub-slice return (&str, no build-copy)", group: "Text", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[Opt::Borrow], conflicts: &[], preempts: &[Opt::TextFromBytesBuilder], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::StringConcatReuse, keyword: "stringconcatreuse", label: "String concat buffer reuse (Let X be A+B -> move+push_str, no format! realloc)", group: "Text", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::StringRepeatLoop, keyword: "stringrepeatloop", label: "String repeat idiom (counted push_str loop -> chunk.repeat(n), presized)", group: "Text", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::StringAccumReuse], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::AsciiClassMask, keyword: "asciiclassmask", label: "ASCII-class predicate to branchless 256-bit mask (beats ctype LUT)", group: "Strings & SIMD", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::StrengthReducePow2, keyword: "strengthreducepow2", label: "Power-of-two %/÷ to mask/shift on non-negative operands (unblocks vectorization)", group: "Arithmetic", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::ModWrap], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::BranchlessAccumulate, keyword: "branchlessaccumulate", label: "Predicate-guarded increment if-conversion (branchless, vectorizable count)", group: "Arithmetic", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::WideAccumulator, keyword: "wideaccumulator", label: "i128 widening of a bounded counting accumulator (plain add vectorizes, beats C)", group: "Arithmetic", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::RadixDigitBuffer, keyword: "radixdigitbuffer", label: "itoa stack digit buffer (one [u8;64] back-to-front, no boxed Seq + no reversal, beats Go strconv)", group: "Text", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::TextFromBytesBuilder, Opt::HoistBorrows], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::ByteSliceReturn, keyword: "byteslicereturn", label: "Bytes zero-copy sub-slice return (&[i64], no build-copy)", group: "Bytes", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[Opt::Borrow], conflicts: &[], preempts: &[Opt::OracleHints, Opt::Unchecked], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::Tile, keyword: "tile", label: "Loop tiling", group: "Loops", default_on: true, paths: AOT | CODEGEN, emits_unsafe: true, mem_class: Neutral, cost: Medium, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::MaskNarrow, keyword: "masknarrow", label: "Masked integer chain at the mask's width (32-bit wrapping under & (2^k-1), k <= 32)", group: "Arithmetic", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::StrengthReducePow2], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::ModWrap, keyword: "modwrap", label: "Remainder of a value below twice its constant divisor as one conditional subtraction", group: "Arithmetic", default_on: true, paths: AOT | VM | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::BranchlessSelect, keyword: "branchlessselect", label: "Two-armed integer assignment as a select of both guarded arms (no branch on the condition)", group: "Arithmetic", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::VariantBox, keyword: "variantbox", label: "Recursive enum variant boxed whole (one box per node; a fieldless variant allocates nothing)", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::BitPack, keyword: "bitpack", label: "Local Seq of Bool held one bit per item (an eighth of the memory; a sieve fits the cache)", group: "Arrays & memory", default_on: true, paths: AOT | CODEGEN, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[Opt::Unbox], conflicts: &[], preempts: &[Opt::HoistBorrows], scope: ProgramOnly, work: OptWork::Same },
    OptMeta { opt: Opt::FoldFusion, keyword: "foldfuse", label: "A consumer of a datatype applied to what a builder builds of it, as one function that never builds it", group: "Inlining & calls", default_on: true, paths: AOT, emits_unsafe: false, mem_class: SavesMem, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::SortNest, keyword: "sortnest", label: "A nest of adjacent compare-exchanges that sorts a window, as one in-place heap sort", group: "Loops", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Less },
    OptMeta { opt: Opt::DivisorSieve, keyword: "divisorsieve", label: "A per-number search for a divisor up to its square root, as one sieve of the range", group: "Loops", default_on: true, paths: AOT, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Less },
    // TraceMemo preempts Peephole: the counted loop over starts it rewrites into a guarded walk of the table is
    // no longer the `While i <= n` shape the peephole lowers to a range.
    OptMeta { opt: Opt::TraceMemo, keyword: "tracememo", label: "A pure iteration run from every start of a range, stopped where it reaches a start already finished", group: "Loops", default_on: true, paths: AOT, emits_unsafe: false, mem_class: TradesMemForSpeed, cost: Cheap, requires: &[], conflicts: &[], preempts: &[Opt::Peephole], scope: Both, work: OptWork::Less },
    OptMeta { opt: Opt::ProbeFuse, keyword: "probefuse", label: "A map's membership test and the reads of the same key under it, as one lookup", group: "Redundancy", default_on: true, paths: AOT, emits_unsafe: false, mem_class: Neutral, cost: Cheap, requires: &[], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::StrideFill, keyword: "stridefill", label: "A loop storing one value into a bit-packed Seq of Bool at a fixed stride, as a fill that writes each word once", group: "Arrays & memory", default_on: true, paths: AOT, emits_unsafe: true, mem_class: Neutral, cost: Medium, requires: &[Opt::BitPack, Opt::Unchecked], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
    OptMeta { opt: Opt::HintVersion, keyword: "hintversion", label: "The innermost loop with bounds hints as a copy that assumes them where they all hold and a copy that assumes none", group: "Bounds & checks", default_on: true, paths: AOT, emits_unsafe: true, mem_class: Neutral, cost: Medium, requires: &[Opt::OracleHints], conflicts: &[], preempts: &[], scope: Both, work: OptWork::Same },
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
    enabled: OptBits,
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self::all_on()
    }
}

impl OptimizationConfig {
    /// Every default-on optimization enabled (the speed baseline).
    pub fn all_on() -> Self {
        let mut enabled = OptBits::EMPTY;
        for m in REGISTRY {
            if m.default_on {
                enabled = enabled.with(m.opt);
            }
        }
        Self { enabled }
    }

    /// No optimizations enabled — the "boring Rust" config.
    pub const fn all_off() -> Self {
        Self {
            enabled: OptBits::EMPTY,
        }
    }

    /// Whether a given optimization is on.
    #[inline]
    pub const fn is_on(&self, opt: Opt) -> bool {
        self.enabled.contains(opt)
    }

    /// Turn an optimization on or off.
    #[inline]
    pub fn set(&mut self, opt: Opt, on: bool) {
        self.enabled = if on {
            self.enabled.with(opt)
        } else {
            self.enabled.without(opt)
        };
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
        let mut visited = OptBits::EMPTY;
        let mut stack = vec![opt];
        while let Some(o) = stack.pop() {
            if visited.contains(o) {
                continue;
            }
            visited = visited.with(o);
            self.set(o, true);
            for &req in o.meta().requires {
                stack.push(req);
            }
        }
    }

    /// Whether every optimization is off.
    pub const fn is_all_off(&self) -> bool {
        self.enabled.is_empty()
    }

    /// The enabled set. Relates an [`OptimizationConfig`] to a [`FiredOptimizations`] set bit for bit.
    pub const fn bits(&self) -> OptBits {
        self.enabled
    }

    /// Combine a program-level config with a function's per-function overrides:
    /// an optimization is on for the function only if it is on in both.
    pub const fn merged(&self, func: &OptimizationConfig) -> OptimizationConfig {
        OptimizationConfig {
            enabled: self.enabled.intersection(func.enabled),
        }
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
                    report
                        .auto_disabled
                        .push((m.opt, Reason::ConflictWith(other)));
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
                        report
                            .auto_disabled
                            .push((m.opt, Reason::DependencyOff(req)));
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

/// The benchmark-visible execution backend whose optimization decisions are
/// being traced. This is deliberately independent of [`path`]: `path` says
/// where an optimization is allowed statically, while this identifies the
/// concrete lowering that was asked to consume the optimized program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationBackend {
    RustAot,
    CAst,
    CBytecode,
    TreeWalker,
    Vm,
    Jit,
}

/// An explicit backend escape hatch taken while lowering. A fallback is data,
/// never a silent control-flow edge: benchmark tooling can ratchet this list to
/// zero without pretending the fallback backend performed the requested work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizationFallback {
    pub from: OptimizationBackend,
    pub to: OptimizationBackend,
    pub reason: String,
}

/// A supported optimization that a backend deliberately chose not to apply.
///
/// This is distinct from [`OptimizationFallback`]: a decline stays on the
/// requested backend and records a cost-model, proof, or capability decision;
/// a fallback changes the backend that executes the program. Keeping the two
/// channels separate lets benchmark ratchets reject silent backend changes
/// without also rejecting an explicitly justified profitability decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizationDecline {
    pub backend: OptimizationBackend,
    pub opt: Opt,
    pub reason: String,
}

/// Stage-separated optimization evidence for one backend compile/run.
///
/// `optimizer` contains shared IR/AST transformations. `lowering` contains
/// choices that fired only while the selected backend emitted or compiled its
/// target. Keeping these sets separate is the ratchet: a shared pass reaching C
/// can no longer be mistaken for the C emitter actually preserving/consuming
/// its optimized form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendOptimizationTrace {
    pub backend: OptimizationBackend,
    pub optimizer: FiredOptimizations,
    pub lowering: FiredOptimizations,
    pub declines: Vec<OptimizationDecline>,
    pub fallbacks: Vec<OptimizationFallback>,
}

impl BackendOptimizationTrace {
    /// Compatibility union used by the original single-bitset trace API.
    pub fn all_fired(&self) -> FiredOptimizations {
        let mut all = self.optimizer;
        all.merge(&self.lowering);
        all
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
    fired: OptBits,
}

impl FiredOptimizations {
    /// An empty set — nothing fired.
    pub const fn new() -> Self {
        Self {
            fired: OptBits::EMPTY,
        }
    }

    /// Build from a set of fired optimizations.
    pub const fn from_bits(fired: OptBits) -> Self {
        Self { fired }
    }

    /// Record that `opt` fired.
    #[inline]
    pub fn mark(&mut self, opt: Opt) {
        self.fired = self.fired.with(opt);
    }

    /// Whether `opt` fired.
    #[inline]
    pub const fn fired(&self, opt: Opt) -> bool {
        self.fired.contains(opt)
    }

    /// Whether nothing fired.
    pub const fn is_empty(&self) -> bool {
        self.fired.is_empty()
    }

    /// The fired set.
    pub const fn bits(&self) -> OptBits {
        self.fired
    }

    /// Fold another set's fired bits into this one.
    pub fn merge(&mut self, other: &FiredOptimizations) {
        self.fired = self.fired.union(other.fired);
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/optimization/tests.rs"]
mod tests;
