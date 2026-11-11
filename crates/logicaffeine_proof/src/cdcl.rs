//! A conflict-driven clause-learning (CDCL) SAT core — the modern competition-grade
//! engine, built to the lineage that made SAT practical: two-watched-literal unit
//! propagation (Moskewicz et al., *Chaff*, 2001), first-UIP conflict analysis with
//! learned clauses and non-chronological backtracking (Marques-Silva & Sakallah,
//! *GRASP*, 1996), VSIDS-style activity decisions, and Luby restarts (Luby et al.,
//! 1993). The clean reference implementation it follows is Eén & Sörensson's *MiniSat*
//! (2003).
//!
//! This module is the propositional substrate of a **DPLL(T)** engine (Nieuwenhuis,
//! Oliveras & Tinelli, JACM 2006): theory propagators (an AllDifferent GAC filter for
//! grid categories, EUF, LIA, …) plug in through [`Theory`], and every learned clause is
//! a resolvent the solver can log as a **DRAT/LRAT** proof step (Wetzler/Heule/Hunt,
//! 2014; Cruz-Filipe et al., 2017) for a downstream linear checker.
//!
//! It is deliberately self-contained and value-typed (`Var = u32`, `Lit` a packed
//! sign+index) so it can be exercised against a brute-force oracle in isolation before
//! it is wired to the grid encoder — a SAT core is only as good as its cross-checks.

use std::collections::{HashSet, VecDeque};

/// A propositional variable: an index `0..num_vars`.
pub type Var = u32;

/// A literal: a variable plus a sign, packed as `var << 1 | negated`. Packing keeps the
/// watch lists and the trail cache-dense, the way every fast solver stores them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Lit(u32);

impl Lit {
    /// The positive literal `+v`.
    #[inline]
    pub fn pos(v: Var) -> Lit {
        Lit(v << 1)
    }
    /// The negative literal `¬v`.
    #[inline]
    pub fn neg(v: Var) -> Lit {
        Lit((v << 1) | 1)
    }
    /// Build from a variable and a sign (`true` ⇒ positive).
    #[inline]
    pub fn new(v: Var, positive: bool) -> Lit {
        if positive {
            Lit::pos(v)
        } else {
            Lit::neg(v)
        }
    }
    /// The underlying variable.
    #[inline]
    pub fn var(self) -> Var {
        self.0 >> 1
    }
    /// Whether this is a positive literal.
    #[inline]
    pub fn is_positive(self) -> bool {
        self.0 & 1 == 0
    }
    /// The complementary literal.
    #[inline]
    pub fn negated(self) -> Lit {
        Lit(self.0 ^ 1)
    }
    /// The dense index for watch/seen arrays (`2*var + sign`).
    #[inline]
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// A three-valued assignment cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Val {
    True,
    False,
    Unset,
}

/// Reason a variable was assigned, for the implication graph that conflict analysis
/// walks. `Decision` is a branching guess; `Clause(ci)` means clause `ci` became unit
/// and forced this literal (its other literals are all false, earlier on the trail);
/// `Theory(k)` means a theory forced it, its explanation the `k`th of [`Explanations`].
#[derive(Clone, Copy, Debug)]
enum Reason {
    Decision,
    Clause(usize),
    Theory(u32),
}

/// The explanations of the literals the theories forced, in trail order. One lives while its literal is assigned and is
/// dropped by the backtrack that unassigns it, so a theory's propagations cost the clause database nothing: no watch,
/// no log, no reduction. An explanation is a clause the theory gave at once, or a token whose clause conflict analysis
/// asks the theory for when it resolves on the literal, held until that analysis ends.
#[derive(Clone, Default)]
struct Explanations {
    /// The clauses given at once.
    lits: Vec<Lit>,
    entries: Vec<Explanation>,
    /// The clauses asked of the theories by the analysis in progress, and the explanations they belong to.
    asked: Vec<Lit>,
    asked_by: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Explanation {
    /// The trail position of the literal it explains.
    at: u32,
    /// Where its literals are, in `lits` for a clause given at once and in `asked` for a token asked for, and how
    /// many; `NONE` while a token is unasked.
    start: u32,
    len: u32,
    /// The theory that handed the token out, and the token; `NONE` for a clause given at once.
    theory: u32,
    token: u32,
}

impl Explanations {
    /// Record `lits` as the explanation of the literal about to take trail position `at`; its index.
    fn push(&mut self, at: usize, lits: &[Lit]) -> u32 {
        let k = self.entries.len() as u32;
        self.entries.push(Explanation { at: at as u32, start: self.lits.len() as u32, len: lits.len() as u32, theory: NONE, token: 0 });
        self.lits.extend_from_slice(lits);
        k
    }

    /// Record theory `theory`'s `token` as the explanation of the literal about to take trail position `at`; its index.
    fn push_token(&mut self, at: usize, theory: usize, token: u32) -> u32 {
        let k = self.entries.len() as u32;
        self.entries.push(Explanation { at: at as u32, start: 0, len: NONE, theory: theory as u32, token });
        k
    }

    /// Whether explanation `k`'s literals are at hand.
    fn ready(&self, k: u32) -> bool {
        self.entries[k as usize].len != NONE
    }

    fn get(&self, k: u32) -> &[Lit] {
        let e = self.entries[k as usize];
        debug_assert!(e.len != NONE, "an explanation is read once it is at hand");
        let held = if e.theory == NONE { &self.lits } else { &self.asked };
        &held[e.start as usize..(e.start + e.len) as usize]
    }

    /// Ask its theory for explanation `k`'s clause, unless it is at hand.
    fn ask(&mut self, k: u32, theories: &mut [Box<dyn Theory>]) {
        let Explanations { entries, asked, asked_by, .. } = self;
        let e = &mut entries[k as usize];
        if e.len != NONE {
            return;
        }
        let start = asked.len();
        theories[e.theory as usize].explain(e.token, asked);
        e.start = start as u32;
        e.len = (asked.len() - start) as u32;
        asked_by.push(k);
    }

    /// Let go of the clauses asked for: the analysis that asked has ended.
    fn release(&mut self) {
        for &k in &self.asked_by {
            self.entries[k as usize].len = NONE;
        }
        self.asked_by.clear();
        self.asked.clear();
    }

    /// Drop the explanations of the literals at trail position `len` and past.
    fn truncate(&mut self, len: usize) {
        while let Some(e) = self.entries.last() {
            if (e.at as usize) < len {
                break;
            }
            if e.theory == NONE {
                self.lits.truncate(e.start as usize);
            }
            self.entries.pop();
        }
    }
}

/// A watch-list entry: the clause being watched plus a *blocking literal* — one of the clause's
/// other literals, cached inline. If the blocker is already true the clause is satisfied, so
/// propagation skips it without dereferencing the clause at all (the cache-miss saver from
/// Chaff/MiniSat). Purely an optimization: the verdict is unaffected.
///
/// Eight bytes, as Kissat's watches are: the clause index and the binary flag share one word, so a cache line holds
/// eight watchers and the watch-list scan that dominates propagation moves half the memory a wider entry would.
#[derive(Clone, Copy, Debug)]
struct Watcher {
    blocker: Lit,
    /// The target shifted left once, with the low bit set when the clause is binary. A binary clause's target is its
    /// index: the blocker *is* its only other literal, so when it is not already true the clause is immediately
    /// unit-or-conflict — decided with no clause dereference at all (the implicit-binary trick from
    /// Kissat/CaDiCaL). A long clause's target is the [`ClauseArena`] offset of its first literal, which reaches its
    /// literals, length and index in one access. Kept in sync by watch (re)creation: strengthening always
    /// `unwatch`+`rewatch`es, and compaction rebuilds all watches, so a target is never stale against the arena.
    word: u32,
}

/// The most clauses a database holds: a binary clause's watcher packs its index into 31 bits.
const MAX_CLAUSES: usize = 1 << 31;

impl Watcher {
    #[inline]
    fn binary(clause: usize, other: Lit) -> Watcher {
        Watcher { blocker: other, word: ((clause as u32) << 1) | 1 }
    }

    #[inline]
    fn long(offset: usize, blocker: Lit) -> Watcher {
        Watcher { blocker, word: (offset as u32) << 1 }
    }

    #[inline]
    fn is_binary(self) -> bool {
        self.word & 1 == 1
    }

    /// The clause index of a binary watcher, the arena offset of a long one.
    #[inline]
    fn target(self) -> usize {
        (self.word >> 1) as usize
    }
}

/// The clause database's literals, every clause one block of a single buffer: its length, its index, then its
/// literals. The two header words are stored as raw `Lit` words and read only through [`ClauseArena::len_at`] and
/// [`ClauseArena::index_at`]. A clause's block never moves until [`Solver::compact`] repacks the buffer, and a
/// clause is only ever rewritten shorter, in place, its spare tail words left to the next repack.
#[derive(Clone, Default)]
struct ClauseArena {
    words: Vec<Lit>,
    offset: Vec<u32>,
}

/// The most words the arena holds: a long clause's watcher packs an offset into 31 bits.
const MAX_ARENA: usize = 1 << 31;

impl ClauseArena {
    /// How many clauses the arena holds.
    #[inline]
    fn count(&self) -> usize {
        self.offset.len()
    }

    /// Append a clause; its index.
    fn push(&mut self, lits: &[Lit]) -> usize {
        let ci = self.offset.len();
        let off = self.words.len() + 2;
        assert!(off + lits.len() <= MAX_ARENA, "the clause arena is full: {MAX_ARENA} words");
        self.words.push(Lit(lits.len() as u32));
        self.words.push(Lit(ci as u32));
        self.words.extend_from_slice(lits);
        self.offset.push(off as u32);
        ci
    }

    /// The offset of clause `ci`'s first literal.
    #[inline]
    fn offset(&self, ci: usize) -> usize {
        self.offset[ci] as usize
    }

    /// The length of the clause whose first literal is at `off`.
    #[inline]
    fn len_at(&self, off: usize) -> usize {
        self.words[off - 2].0 as usize
    }

    /// The index of the clause whose first literal is at `off`.
    #[inline]
    fn index_at(&self, off: usize) -> usize {
        self.words[off - 1].0 as usize
    }

    /// Clause `ci`'s literals.
    #[inline]
    fn clause(&self, ci: usize) -> &[Lit] {
        let off = self.offset(ci);
        &self.words[off..off + self.len_at(off)]
    }

    /// Replace clause `ci`'s literals by `lits`, no longer than they were.
    fn rewrite(&mut self, ci: usize, lits: &[Lit]) {
        let off = self.offset(ci);
        assert!(lits.len() <= self.len_at(off), "a clause is only ever rewritten shorter");
        self.words[off..off + lits.len()].copy_from_slice(lits);
        self.words[off - 2] = Lit(lits.len() as u32);
    }

    /// The arena of `survivors`' clauses, in that order and numbered from 0, packed with no spare words.
    fn retained(&self, survivors: &[usize]) -> ClauseArena {
        let words = survivors.iter().map(|&ci| self.clause(ci).len() + 2).sum();
        let mut packed = ClauseArena { words: Vec::with_capacity(words), offset: Vec::with_capacity(survivors.len()) };
        for &ci in survivors {
            packed.push(self.clause(ci));
        }
        packed
    }
}

/// The verdict of [`Solver::solve`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SolveResult {
    /// Satisfiable, with a full model over `0..num_vars` (`true`/`false` per variable).
    Sat(Vec<bool>),
    /// Unsatisfiable.
    Unsat,
}

/// The outcome of a conflict-budgeted solve: a verdict, or budget exhaustion (with the learned
/// clauses left available for symmetric amplification).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BudgetedResult {
    /// Satisfiable, with a full model.
    Sat(Vec<bool>),
    /// Unsatisfiable (proven within budget).
    Unsat,
    /// The conflict budget was exhausted before a verdict; [`Solver::learned`] holds the clauses
    /// derived so far.
    Budget,
}

/// A learned clause logged for proof reconstruction. Each is a resolvent derivable from
/// the formula by reverse unit propagation — the unit of a DRAT/LRAT proof. With proof tracing
/// on ([`Solver::set_lrat`]) it carries its LRAT step: its id and the ids of the clauses that,
/// in order, each become unit under its negation until the last conflicts.
#[derive(Clone, Debug)]
pub struct LearnedClause {
    pub lits: Vec<Lit>,
    pub id: u64,
    pub hints: Vec<u64>,
}

/// A theory propagator (the DPLL(T) seam). The SAT core calls [`Theory::propagate`] at
/// each fixpoint of Boolean propagation; a theory returns a fresh clause to add (an
/// explanation/conflict) or `None` if it has nothing to say. AllDifferent GAC, EUF, and
/// LIA all implement this without the core knowing the theory.
pub trait Theory {
    /// Given the solver's current `trail` (assigned literals in assignment order), return a clause
    /// that is theory-entailed and currently unit or falsified (so the core will propagate or
    /// conflict on it), or `None` at a theory fixpoint. The trail is passed in order — and shrinks
    /// on backtrack — so an incremental theory can sync forward/undo against it (LIFO). The returned
    /// clauses MUST each be a sound consequence of the theory. Returning the whole batch of forced
    /// clauses at once (rather than one per call) lets an incremental theory amortise its work over
    /// one pass instead of rescanning per implication. An empty vec means "theory fixpoint".
    fn propagate(&mut self, trail: &[Lit]) -> Vec<Vec<Lit>>;

    /// [`Theory::propagate`], told that the first `kept` literals of `trail` are the ones it was given last time: the
    /// solver has backtracked no further since. An incremental theory undoes only what lies past them, without
    /// comparing the trail with its own copy of it.
    fn propagate_kept(&mut self, trail: &[Lit], kept: usize) -> Vec<Vec<Lit>> {
        let _ = kept;
        self.propagate(trail)
    }

    /// [`Theory::propagate_kept`], the literals it forces explained on demand: each goes to `forced` with a token,
    /// and [`Theory::explain`] gives its clause when conflict analysis asks, while the literal stays assigned. The
    /// clauses returned are those explained at once, a refutation always among them. By default every one is.
    fn propagate_lazily(&mut self, trail: &[Lit], kept: usize, forced: &mut Vec<(Lit, u32)>) -> Vec<Vec<Lit>> {
        let _ = forced;
        self.propagate_kept(trail, kept)
    }

    /// Append to `clause` the clause of the literal [`Theory::propagate_lazily`] forced with `token`: that literal,
    /// then the literals that force it, each false.
    fn explain(&mut self, token: u32, clause: &mut Vec<Lit>) {
        let _ = (token, clause);
        unreachable!("a theory is asked only for the tokens it handed out")
    }
}

/// Glucose restart-policy constants (Audemard & Simon, 2012). `LBD_WINDOW` recent conflicts feed
/// the fast average; `TRAIL_WINDOW` recent trail lengths feed the blocking average. A restart fires
/// when `fast_lbd_avg * RESTART_K > global_lbd_avg`; it is *blocked* when the trail at conflict
/// exceeds `BLOCK_R ×` the recent average (and at least `BLOCK_MIN_CONFLICTS` have passed).
const LBD_WINDOW: usize = 50;
const TRAIL_WINDOW: usize = 5000;
const RESTART_K: f64 = 0.8;
const BLOCK_R: f64 = 1.4;
const BLOCK_MIN_CONFLICTS: u64 = 10_000;

/// Adaptive restart phase length: the first phase spans `ADAPT_PHASE_BASE` conflicts, and each
/// switch multiplies it by `PHASE_GROWTH`. The base is large enough that a small instance solves
/// entirely inside the first (aggressive Glucose) phase — keeping Glucose's wins there — while a
/// long search alternates into calm Luby phases that help SAT and Luby-favouring instances.
const ADAPT_PHASE_BASE: u64 = 5000;
const PHASE_GROWTH: f64 = 2.0;

/// The most reason-side literals bumped per learned literal, Kissat's `bumpreasonslimit`.
const REASON_SIDE_LIMIT: usize = 10;

/// Conflicts per Luby unit: `LUBY_UNIT` for pure Luby restarts, `STABLE_LUBY_UNIT` in Adaptive's stable phase —
/// Kissat's reluctant doubling, whose rare restarts let target phases run long.
const LUBY_UNIT: u64 = 100;
const STABLE_LUBY_UNIT: u64 = 1024;

/// Stable-mode rephasing, Kissat's schedule: the first rephase after `REPHASE_INIT` conflicts, the `k`-th interval
/// `REPHASE_INT · k · log10(k + 9)³` conflicts, the phases reset in the cycle best, walk, inverted, best, walk,
/// original.
const REPHASE_INIT: u64 = 1000;
const REPHASE_INT: f64 = 1000.0;

/// Learned-clause reduction, Kissat's policy (`reduce.c`, `tiers.c`). The `k`-th reduction runs `REDUCE_INT · √k`
/// conflicts after the one before. It keeps a learned clause of glue at most the tier-one limit that analysis used
/// since the last reductions, and one of glue at most the tier-two limit used since the last one; of the rest, ranked
/// worst first by glue and then length, it deletes a share sliding from `REDUCE_LOW`% toward `REDUCE_HIGH`% as
/// reductions accumulate. The tier limits are the glues at which `TIER1_RELATIVE` and `TIER2_RELATIVE` of all uses
/// so far accumulate, per search mode, with `TIER1_DEFAULT` and `TIER2_DEFAULT` before any use.
const REDUCE_INT: u64 = 1000;
const REDUCE_LOW: f64 = 50.0;
const REDUCE_HIGH: f64 = 90.0;
const MAX_USED: u8 = 31;
const MAX_GLUE_USED: usize = 127;
const TIER1_RELATIVE: f64 = 0.5;
const TIER2_RELATIVE: f64 = 0.9;
const TIER1_DEFAULT: u32 = 2;
const TIER2_DEFAULT: u32 = 6;

/// A walk spends `WALK_EFFORT_PERMILLE`‰ of the search ticks since the last one began, reckoned against at least
/// `WALK_MIN_REFERENCE` ticks (Kissat's `walkeffort` and `mineffort`).
const WALK_EFFORT_PERMILLE: u64 = 50;
const WALK_MIN_REFERENCE: u64 = 10_000_000;

/// The restart heuristic. Glucose's dynamic LBD policy restarts when recent learned-clause quality
/// (literal-block distance) degrades relative to the global average, and *blocks* a restart when
/// the trail is unusually long — a sign the search is closing in on a model. Luby is the classic
/// universal sequence (Luby et al., 1993). Adaptive alternates the two in geometrically growing
/// phases (the CaDiCaL/Kissat "stabilizing / non-stabilizing" idea): aggressive Glucose restarts to
/// EXPLORE, then calm Luby restarts to let phase-saving EXPLOIT and dig toward a model — capturing
/// the strength of each and avoiding the per-instance losses of either alone. The choice is a pure
/// search-ORDER heuristic: it never changes a verdict.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RestartMode {
    /// Alternate Glucose (non-stabilizing) and Luby (stabilizing) phases — the default.
    Adaptive,
    /// Dynamic LBD restarts with blocking — the Glucose lever.
    Glucose,
    /// The Luby universal restart sequence.
    Luby,
}

/// How a conflict under chronological backtracking is to be handled.
enum Chrono {
    /// Every literal of the conflict clause is false at level 0.
    Refuted,
    /// The clause was a missed implication, now assigned.
    Implied,
    /// The search is at the conflict's level: analyze it.
    Analyze,
}

/// What a lucky-phase attempt found.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Lucky {
    /// A model, with every variable assigned.
    Sat(Vec<bool>),
    /// A failed literal's unit conflicted at level 0: the formula is unsatisfiable.
    Unsat,
    /// No attempt assigned every variable.
    Unknown,
}

/// The learned-clause reduction policy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReducePolicy {
    /// Once the live learned clauses reach a limit (`HALVING_LIMIT`, then `HALVING_GROWTH` more after each
    /// reduction), delete the worse half by LBD of those above glue 2 and longer than two literals — the default.
    Halving,
    /// Kissat's tiered policy, at the `REDUCE_INT` constants: usage counters, glue promotion, adaptive tier limits
    /// and a `√k` conflict schedule.
    Tiered,
}

/// The [`ReducePolicy::Halving`] trigger: the live learned clauses that start the first reduction, and the growth of
/// that limit after each.
const HALVING_LIMIT: usize = 2000;
const HALVING_GROWTH: usize = 500;

/// The end of a [`Solver`] variable queue link.
const NONE: u32 = u32::MAX;

/// The CDCL solver.
#[derive(Clone)]
pub struct Solver {
    num_vars: usize,
    /// All clauses (original + learned); a clause is a `Vec<Lit>`.
    arena: ClauseArena,
    /// Watch lists: for each literal, the [`Watcher`]s on it. Two-watched-literal scheme — a
    /// clause is visited only when one of its two watched literals is falsified, and even then is
    /// skipped without a dereference when its cached blocking literal is already true.
    watches: Vec<Vec<Watcher>>,
    /// Per-literal value, indexed by [`Lit::index`]: a literal and its negation are set and cleared together, so
    /// testing a literal is one load with no sign branch.
    vals: Vec<Val>,
    /// Per-variable decision level.
    level: Vec<u32>,
    /// Per-variable reason.
    reason: Vec<Reason>,
    /// The assignment trail (literals in assignment order).
    trail: Vec<Lit>,
    /// Index into `trail` where each decision level begins.
    trail_lim: Vec<usize>,
    /// Head of the propagation queue (index into `trail`).
    qhead: usize,
    /// VSIDS activities and the bump/decay.
    activity: Vec<f64>,
    var_inc: f64,
    /// Phase saving: the last value each variable held before being unset by a backjump. A
    /// decision reuses it, so the search sticks to assignments that previously propagated far —
    /// the standard ~2-3× win on structured SAT. Purely a heuristic: it changes search ORDER,
    /// never completeness, so verdicts are unaffected.
    saved_phase: Vec<bool>,
    /// The phases a solve starts from and the original/inverted rephases restore: all false unless a caller seeded
    /// them ([`Solver::set_initial_phase`]).
    initial_phase: Vec<bool>,
    /// Target phases (Biere & Fleury, *Chasing target phases*, POS 2020): the values on the longest conflict-free
    /// trail since the last rephase, which stable-mode decisions follow before the saved phase. `0` unset, else the
    /// sign of the value.
    target_phase: Vec<i8>,
    target_assigned: usize,
    /// Best phases: the values on the longest conflict-free trail since the last best rephase, which that rephase
    /// copies into the saved phases.
    best_phase: Vec<i8>,
    best_assigned: usize,
    /// Rephases this solve and the conflict count past which stable mode rephases next.
    rephased: u64,
    rephase_limit: u64,
    /// Local-search walks this solve (the walking rephase), and the search ticks when the last one began.
    walks: u64,
    walk_ticks: u64,
    /// Search effort in Kissat's unit: per propagated literal one tick and one per cache line of its watch list, and
    /// one per long clause dereferenced. It prices a walk's budget; no decision reads it.
    ticks: u64,
    /// Whether stable-mode decisions follow target phases, and whether the walking rephase runs (both default on).
    /// For A/B benchmarking; verdict-invariant either way.
    target_enabled: bool,
    walk_enabled: bool,
    /// Whether a solve first tries lucky phases, and how many solves one answered.
    lucky_enabled: bool,
    lucky_hits: u64,
    /// Chronological backtracking (Nadel & Ryvchin, *Chronological backtracking*, SAT 2018): a backjump over more
    /// than this many levels backtracks one level instead, the asserting literal assigned at its own lower level.
    /// `None` is off (the default). `chrono_now` is whether the solve in progress uses it: not under theories, whose
    /// trail sync assumes a trail in level order, and not under assumptions.
    chrono_levels: Option<u32>,
    chrono_now: bool,
    chrono_backtracks: u64,
    /// Per theory of the solve in progress, how much of the trail it was last given is still in place: the trail's
    /// length then, lowered by every backtrack since.
    theory_kept: Vec<usize>,
    /// Whether conflict analysis bumps the reason side of the learned clause, its marks (indexed by var), and the
    /// literals it bumped. The count is pure observability.
    bump_reasons: bool,
    side_mark: Vec<bool>,
    reason_side_bumps: u64,
    /// The search's inprocessing schedule: the conflict count at the last round and the gap before the next.
    last_inprocess: u64,
    inprocess_gap: u64,
    /// Set from outside to pause [`Solver::resume`] at its next conflict.
    stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// The explanations of the literals the theories forced, and the buffer a theory hands its tokens over in.
    explanations: Explanations,
    forced: Vec<(Lit, u32)>,
    /// Learned clauses, logged for proof output.
    learned_log: Vec<LearnedClause>,
    /// Scratch `seen` markers for conflict analysis (indexed by var).
    seen: Vec<bool>,
    /// Reusable redundancy memo for learned-clause minimization (indexed by var): `0` unknown, `1`
    /// redundant, `2` not — solver-resident so `minimize` never allocates a per-conflict `HashMap`.
    min_cache: Vec<u8>,
    /// The vars whose `min_cache` entry was set this minimization, so only those are reset (not the
    /// whole array) — the sparse-clear list that keeps minimization allocation-free.
    min_touched: Vec<Var>,
    /// Per-decision-level stamps for computing a learned clause's LBD without a per-conflict `Vec`
    /// allocation or sort: a level is counted once when its stamp differs from the current generation.
    lbd_stamp: Vec<u32>,
    /// The current LBD-stamp generation, bumped once per conflict (wraps only after 2³² conflicts).
    lbd_gen: u32,
    /// Set once an empty clause is added — the formula is then trivially unsatisfiable.
    empty_clause: bool,
    /// Number of clauses present when the solve began (the original formula); clauses at
    /// indices `>= n_original` are learned. Lets a downstream checker read the original
    /// clauses without a separate copy.
    n_original: usize,
    /// Total conflicts encountered during the solve — the standard measure of search work,
    /// the lever symmetry breaking is meant to collapse. Pure observability; never read by the
    /// search itself, so it cannot change a verdict.
    conflicts: u64,
    /// Total decisions (branchings) — search work. Pure observability; never read by the search.
    decisions: u64,
    /// Total literals propagated by BCP (one per trail literal processed) — throughput work.
    propagations: u64,
    /// Total restarts performed — observability for the restart policy.
    restarts: u64,
    /// Per clause, whether the solver learned it (else it was added): what an incremental solve, whose added clauses
    /// come between its learned ones, deletes from.
    learned_flag: Vec<bool>,
    /// How many clauses of the database are learned ones.
    learned_live: usize,
    /// Each clause's id: the caller's for a clause added with one, else the next free one.
    clause_ids: Vec<u64>,
    next_id: u64,
    /// Whether each learned clause, and each refutation, records its LRAT hints.
    lrat: bool,
    /// The hints [`Solver::analyze`] found for the clause about to be learned.
    pending_hints: Vec<u64>,
    /// The hints of the last unsatisfiable solve's conclusion: the negation of its assumptions, or the empty clause.
    unsat_hints: Vec<u64>,
    /// After an unsatisfiable solve under assumptions, the assumptions its refutation rests on.
    core: Vec<Lit>,
    /// The hints that derive the empty clause, once the formula has been found unsatisfiable at level 0.
    empty_hints: Vec<u64>,
    /// Per variable, its place on the trail while assigned.
    trail_pos: Vec<u32>,
    /// Per-clause LBD (literal-block distance, Audemard & Simon 2009): the number of distinct
    /// decision levels among a learned clause's literals. Low LBD ("glue") clauses are the most
    /// reusable; high-LBD clauses are the deletion candidates. Originals carry `u32::MAX`.
    lbd: Vec<u32>,
    /// Whether to periodically delete learned clauses, keeping the database compact. Verdict-invariant —
    /// deletion only removes redundant learned clauses; the original formula and every reason are kept.
    /// Toggleable for A/B benchmarking.
    reduce_enabled: bool,
    /// Per clause, how recently conflict analysis used it: `MAX_USED` when a learned clause is learned or
    /// resolved on, one less at each reduction after.
    used: Vec<u8>,
    /// Per search mode (focused, stable), how often analysis used a learned clause of each glue: what the
    /// tier limits are read from.
    glue_used: [Vec<u64>; 2],
    /// Per search mode, the tier-one and tier-two glue limits.
    tier1: [u32; 2],
    tier2: [u32; 2],
    /// The reduction policy in force, and the reductions this solver has run.
    reduce_policy: ReducePolicy,
    reductions: u64,
    /// The [`ReducePolicy::Halving`] trigger: the live learned clauses that start the next reduction.
    reduce_limit: usize,
    /// The [`ReducePolicy::Tiered`] schedule: the base interval between reductions and the conflict count of the next.
    reduce_interval: u64,
    reduce_next: u64,
    /// If `Some`, decisions are restricted to variables flagged `true` (the rest are left to theory
    /// propagation). DPLL(XOR) sets this to the non-pivot variables so search ranges only over the
    /// GF(2) kernel; `None` means branch on any variable (ordinary CDCL).
    decision_mask: Option<Vec<bool>>,
    /// VSIDS order heap (MiniSat-style indexed binary max-heap): the next decision is the highest-
    /// activity unassigned variable in O(log n), replacing the old O(n) activity scan — the dominant
    /// throughput lever at scale. `heap_pos[v]` is v's index in `heap`, or -1 when absent.
    heap: Vec<Var>,
    heap_pos: Vec<i32>,
    /// Variable move-to-front (Ryan 2004; the focused-mode order of CaDiCaL and Kissat): a doubly linked queue of
    /// the variables, the most recently bumped last, each stamped with the time of its last move. `vmtf_search` is
    /// a variable at or past the last unassigned one, from which a decision walks toward the front. Decisions follow
    /// it in focused mode when [`Solver::set_focused_queue`] turns it on; the VSIDS heap is kept either way.
    vmtf_prev: Vec<u32>,
    vmtf_next: Vec<u32>,
    vmtf_stamp: Vec<u64>,
    vmtf_first: u32,
    vmtf_last: u32,
    vmtf_search: u32,
    vmtf_clock: u64,
    vmtf_enabled: bool,
    /// Decide by justification ([`Solver::set_justifying`]): each variable's gate, its two fan-ins, when it is one; the
    /// trail before `justified` holds no gate assigned false with both fan-ins open; and per decision level, the first
    /// place on the trail of a gate assigned false whose fan-ins false are of that level and above, which a backtrack
    /// below the level opens again.
    justifying: bool,
    gates: Vec<Option<[Lit; 2]>>,
    /// Per variable, its cover gate's place in `cover_gates` plus one (0 for none): the first of its cubes in
    /// `cube_starts`, then how many are its true value's and how many its false value's; each cube's literals in
    /// `cube_lits` from its start to the next cube's.
    covers: Vec<u32>,
    cover_gates: Vec<(u32, u16, u16)>,
    cube_starts: Vec<u32>,
    cube_lits: Vec<Lit>,
    justified: usize,
    rests: Vec<usize>,
    /// The clauses a justifying search makes true itself ([`Solver::add_side_clause`]), as they were given, and how many
    /// of them, from the first, were true when last looked at since the last backtrack.
    sides: Vec<Vec<Lit>>,
    sided: usize,
    /// The variables the current conflict analysis bumped, moved to the back of the queue together at its end.
    vmtf_bumped: Vec<Var>,
    /// The restart heuristic in force (default [`RestartMode::Glucose`]).
    restart_mode: RestartMode,
    /// Glucose fast window: the LBDs of the most recent `LBD_WINDOW` conflicts, with their running
    /// sum, for the dynamic restart trigger.
    lbd_fast: VecDeque<u32>,
    lbd_fast_sum: u64,
    /// Sum of every conflict's LBD over the whole solve — the global average's numerator (its
    /// denominator is `conflicts`).
    lbd_global_sum: u64,
    /// Glucose blocking window: the trail lengths at the most recent `TRAIL_WINDOW` conflicts, with
    /// their running sum, for the blocking-restart "we look close to a model" test.
    trail_fast: VecDeque<usize>,
    trail_fast_sum: u64,
    /// Restarts suppressed by the blocking heuristic. Pure observability.
    blocked_restarts: u64,
    /// Learned clauses strengthened by vivification. Pure observability.
    vivifications: u64,
    /// Units derived by failed-literal probing. Pure observability.
    probes: u64,
    /// Learned clauses deleted or strengthened by subsumption / self-subsuming resolution. Pure
    /// observability.
    subsumptions: u64,
    /// Whether to run the level-0 inprocessing schedule (probe + vivify + rephase) between
    /// restarts during a long search. Default on; toggleable for A/B benchmarking. Verdict-invariant
    /// either way.
    inprocess_enabled: bool,
    /// Conflicts between inprocessing rounds (default [`INPROCESS_INTERVAL`]). Tunable for
    /// benchmarking and tests.
    inprocess_interval: u64,
    /// Whether probing is still worth running this solve. Set false after a probing round derives no
    /// unit — on most instances probing finds nothing, so it would otherwise be pure overhead. Reset
    /// at each top-level solve.
    probe_active: bool,
    /// Restart bookkeeping, solver-resident so the adaptive phase machinery can reset it cleanly:
    /// conflicts since the last restart, the current Luby limit, and the Luby step index.
    csr: u64,
    restart_limit: u64,
    restart_no: u64,
    /// Adaptive restart phase: `stabilizing` = the calm (Luby) phase vs the aggressive (Glucose)
    /// phase; `phase_start` is the conflict count when the current phase began; `phase_len` is its
    /// length, grown by [`PHASE_GROWTH`] each switch.
    stabilizing: bool,
    phase_start: u64,
    phase_len: u64,
}

/// Conflicts before the FIRST inprocessing round — high enough that short searches never inprocess
/// (their churn can cost more than it saves), so the schedule engages only on the genuinely long
/// solves where it pays off. Measured: at 2000 a couple of short random instances regress; at 6000
/// every tested instance is regression-free while the long-search wins (PHP, gt18, hard random)
/// are kept. Subsequent rounds back off by [`INPROCESS_GROWTH`].
const INPROCESS_INTERVAL: u64 = 6000;
/// Per-round caps so an inprocessing pass stays cheap relative to search.
const PROBE_BUDGET: usize = 256;
const VIVIFY_BUDGET: usize = 400;
const SUBSUME_BUDGET: usize = 600;
/// Each inprocessing round multiplies the conflicts-until-next-round by this factor, so a long
/// search inprocesses often early then rarely — bounding total inprocessing cost (and the churn it
/// can cost on instances where it does not pay off).
const INPROCESS_GROWTH: f64 = 1.5;

impl Solver {
    /// A fresh solver over `num_vars` variables and no clauses.
    pub fn new(num_vars: usize) -> Self {
        Solver {
            num_vars,
            arena: ClauseArena::default(),
            watches: vec![Vec::new(); num_vars * 2],
            vals: vec![Val::Unset; num_vars * 2],
            level: vec![0; num_vars],
            reason: vec![Reason::Decision; num_vars],
            trail: Vec::new(),
            trail_lim: Vec::new(),
            qhead: 0,
            activity: vec![0.0; num_vars],
            var_inc: 1.0,
            saved_phase: vec![false; num_vars],
            initial_phase: vec![false; num_vars],
            target_phase: vec![0; num_vars],
            target_assigned: 0,
            best_phase: vec![0; num_vars],
            best_assigned: 0,
            rephased: 0,
            rephase_limit: REPHASE_INIT,
            walks: 0,
            walk_ticks: 0,
            ticks: 0,
            target_enabled: true,
            walk_enabled: true,
            lucky_enabled: true,
            lucky_hits: 0,
            chrono_levels: None,
            chrono_now: false,
            chrono_backtracks: 0,
            theory_kept: Vec::new(),
            bump_reasons: false,
            side_mark: vec![false; num_vars],
            reason_side_bumps: 0,
            last_inprocess: 0,
            inprocess_gap: INPROCESS_INTERVAL,
            stop: None,
            explanations: Explanations::default(),
            forced: Vec::new(),
            learned_log: Vec::new(),
            seen: vec![false; num_vars],
            min_cache: vec![0u8; num_vars],
            min_touched: Vec::new(),
            lbd_stamp: vec![0u32; num_vars + 1],
            lbd_gen: 0,
            empty_clause: false,
            n_original: 0,
            conflicts: 0,
            decisions: 0,
            propagations: 0,
            restarts: 0,
            lbd: Vec::new(),
            learned_flag: Vec::new(),
            learned_live: 0,
            clause_ids: Vec::new(),
            next_id: 1,
            lrat: false,
            pending_hints: Vec::new(),
            unsat_hints: Vec::new(),
            core: Vec::new(),
            empty_hints: Vec::new(),
            trail_pos: vec![0; num_vars],
            reduce_enabled: true,
            used: Vec::new(),
            glue_used: [vec![0; MAX_GLUE_USED + 1], vec![0; MAX_GLUE_USED + 1]],
            tier1: [TIER1_DEFAULT; 2],
            tier2: [TIER2_DEFAULT; 2],
            reduce_policy: ReducePolicy::Halving,
            reductions: 0,
            reduce_limit: HALVING_LIMIT,
            reduce_interval: REDUCE_INT,
            reduce_next: REDUCE_INT,
            decision_mask: None,
            heap: (0..num_vars as Var).collect(),
            heap_pos: (0..num_vars as i32).collect(),
            vmtf_prev: (0..num_vars as u32).map(|v| if v == 0 { NONE } else { v - 1 }).collect(),
            vmtf_next: (0..num_vars as u32).map(|v| if v + 1 == num_vars as u32 { NONE } else { v + 1 }).collect(),
            vmtf_stamp: (1..=num_vars as u64).collect(),
            vmtf_first: if num_vars == 0 { NONE } else { 0 },
            vmtf_last: if num_vars == 0 { NONE } else { num_vars as u32 - 1 },
            vmtf_search: if num_vars == 0 { NONE } else { num_vars as u32 - 1 },
            vmtf_clock: num_vars as u64,
            vmtf_enabled: false,
            justifying: false,
            gates: Vec::new(),
            covers: Vec::new(),
            cover_gates: Vec::new(),
            cube_starts: vec![0],
            cube_lits: Vec::new(),
            justified: 0,
            rests: Vec::new(),
            sides: Vec::new(),
            sided: 0,
            vmtf_bumped: Vec::new(),
            restart_mode: RestartMode::Adaptive,
            lbd_fast: VecDeque::with_capacity(LBD_WINDOW + 1),
            lbd_fast_sum: 0,
            lbd_global_sum: 0,
            trail_fast: VecDeque::with_capacity(TRAIL_WINDOW + 1),
            trail_fast_sum: 0,
            blocked_restarts: 0,
            vivifications: 0,
            probes: 0,
            subsumptions: 0,
            inprocess_enabled: true,
            inprocess_interval: INPROCESS_INTERVAL,
            probe_active: true,
            csr: 0,
            restart_limit: luby(1) * LUBY_UNIT,
            restart_no: 1,
            stabilizing: false,
            phase_start: 0,
            phase_len: ADAPT_PHASE_BASE,
        }
    }

    /// Run one subsumption + self-subsuming-resolution round over the learned clauses (Phase 4
    /// inprocessing). Returns `false` if it proves the formula UNSAT. Verdict-invariant.
    pub fn subsume(&mut self) -> bool {
        self.subsume_round(0)
    }

    /// Learned clauses deleted or strengthened by subsumption during this solver's life.
    pub fn subsumptions(&self) -> u64 {
        self.subsumptions
    }

    /// Enable or disable the between-restart inprocessing schedule (default on). For A/B
    /// benchmarking; verdict-invariant either way.
    pub fn set_inprocess(&mut self, on: bool) {
        self.inprocess_enabled = on;
    }

    /// Set the conflict interval between inprocessing rounds (default `INPROCESS_INTERVAL`).
    /// Tuning / test knob; verdict-invariant.
    pub fn set_inprocess_interval(&mut self, conflicts: u64) {
        self.inprocess_interval = conflicts.max(1);
    }

    /// Run one vivification round over the learned clauses, strengthening each shortenable clause
    /// in place (Phase 2 inprocessing). Returns `false` if vivification proves the formula UNSAT.
    /// Verdict-invariant; intended to be scheduled at decision level 0 between restarts.
    pub fn vivify(&mut self) -> bool {
        self.vivify_round(0)
    }

    /// Learned clauses strengthened by vivification during this solver's life. Pure observability.
    pub fn vivifications(&self) -> u64 {
        self.vivifications
    }

    /// Run one failed-literal probing round (Phase 3 inprocessing): try each phase of each free
    /// variable as a level-0 assumption; if propagation conflicts, the opposite literal is a forced
    /// unit, learned permanently. Returns `false` if probing proves the formula UNSAT.
    /// Verdict-invariant; scheduled at decision level 0.
    pub fn probe(&mut self) -> bool {
        self.probe_round(0)
    }

    /// Units derived by failed-literal probing during this solver's life. Pure observability.
    pub fn probes(&self) -> u64 {
        self.probes
    }

    /// Select the restart heuristic (default [`RestartMode::Glucose`]). Search-order only — a
    /// verdict is unaffected, so this is for A/B tuning.
    pub fn set_restart_mode(&mut self, mode: RestartMode) {
        self.restart_mode = mode;
    }

    /// The restart heuristic in force.
    pub fn restart_mode(&self) -> RestartMode {
        self.restart_mode
    }

    /// Restarts suppressed by Glucose blocking during the last solve.
    pub fn blocked_restarts(&self) -> u64 {
        self.blocked_restarts
    }

    /// Record a conflict's LBD and the trail length at the moment of conflict, updating the Glucose
    /// fast and blocking windows. If the trail was much longer than the recent average, empty the
    /// fast LBD window — that defers the next dynamic restart (the blocking heuristic: a long trail
    /// means a promising branch). Pure heuristic state; never affects a verdict. Call once per
    /// conflict, before `conflicts` is incremented.
    fn note_conflict(&mut self, lbd: u32, trail_at_conflict: usize) {
        self.lbd_global_sum += u64::from(lbd);
        self.lbd_fast.push_back(lbd);
        self.lbd_fast_sum += u64::from(lbd);
        if self.lbd_fast.len() > LBD_WINDOW {
            self.lbd_fast_sum -= u64::from(self.lbd_fast.pop_front().unwrap());
        }
        self.trail_fast.push_back(trail_at_conflict);
        self.trail_fast_sum += trail_at_conflict as u64;
        if self.trail_fast.len() > TRAIL_WINDOW {
            self.trail_fast_sum -= self.trail_fast.pop_front().unwrap() as u64;
        }
        if self.conflicts >= BLOCK_MIN_CONFLICTS
            && self.lbd_fast.len() >= LBD_WINDOW
            && (trail_at_conflict as f64)
                > BLOCK_R * (self.trail_fast_sum as f64 / self.trail_fast.len() as f64)
        {
            self.lbd_fast.clear();
            self.lbd_fast_sum = 0;
            self.blocked_restarts += 1;
        }
    }

    /// The Glucose dynamic-restart trigger: the fast-window LBD average has degraded past
    /// `RESTART_K ×` the global average. Fires only once the fast window is full (and is emptied by
    /// blocking, which is how a block suppresses the next restart).
    fn glucose_should_restart(&self) -> bool {
        if self.lbd_fast.len() < LBD_WINDOW || self.conflicts == 0 {
            return false;
        }
        let fast = self.lbd_fast_sum as f64 / self.lbd_fast.len() as f64;
        let global = self.lbd_global_sum as f64 / self.conflicts as f64;
        fast * RESTART_K > global
    }

    /// Reset all restart bookkeeping for a fresh top-level solve (Luby counter, Glucose windows, and
    /// the adaptive phase, which starts in the aggressive Glucose phase).
    fn reset_restart_state(&mut self) {
        self.rephased = 0;
        self.rephase_limit = self.conflicts + REPHASE_INIT;
        self.target_assigned = 0;
        self.best_assigned = 0;
        self.csr = 0;
        self.restart_limit = luby(1) * self.luby_unit();
        self.restart_no = 1;
        self.stabilizing = false;
        self.phase_start = self.conflicts;
        self.phase_len = ADAPT_PHASE_BASE;
        self.lbd_fast.clear();
        self.lbd_fast_sum = 0;
    }

    /// In Adaptive mode, switch between the aggressive (Glucose) and calm (Luby) phases once the
    /// current phase's conflict budget is spent, growing the next phase and resetting both policies'
    /// trigger state so neither carries a stale signal across the boundary. No-op in fixed modes.
    fn advance_restart_phase(&mut self) {
        if self.restart_mode != RestartMode::Adaptive {
            return;
        }
        if self.conflicts - self.phase_start >= self.phase_len {
            self.stabilizing = !self.stabilizing;
            self.phase_start = self.conflicts;
            self.phase_len = ((self.phase_len as f64) * PHASE_GROWTH) as u64;
            self.csr = 0;
            self.restart_no = 1;
            self.restart_limit = luby(1) * self.luby_unit();
            self.lbd_fast.clear();
            self.lbd_fast_sum = 0;
        }
    }

    /// Whether to restart now, per the active policy. Adaptive consults Glucose in its aggressive
    /// phase and Luby in its calm (stabilizing) phase.
    fn want_restart(&self) -> bool {
        match self.restart_mode {
            RestartMode::Glucose => self.glucose_should_restart(),
            RestartMode::Luby => self.csr >= self.restart_limit,
            RestartMode::Adaptive => {
                if self.stabilizing {
                    self.csr >= self.restart_limit
                } else {
                    self.glucose_should_restart()
                }
            }
        }
    }

    /// The conflicts per Luby unit in force.
    fn luby_unit(&self) -> u64 {
        if self.restart_mode == RestartMode::Adaptive {
            STABLE_LUBY_UNIT
        } else {
            LUBY_UNIT
        }
    }

    /// Whether the search is in stable mode: the calm Luby phase of Adaptive, all of Luby, none of Glucose. Stable
    /// mode follows target phases and rephases.
    fn stable(&self) -> bool {
        match self.restart_mode {
            RestartMode::Adaptive => self.stabilizing,
            RestartMode::Luby => true,
            RestartMode::Glucose => false,
        }
    }

    /// After a conflict, on the trail one level below it (consistent again): a trail longer than the target's or the
    /// best's becomes that phase vector. Only assigned variables are written; the rest keep what they held.
    fn update_target_and_best(&mut self) {
        let assigned = self.trail.len();
        if assigned > self.target_assigned {
            self.target_assigned = assigned;
            for &l in &self.trail {
                self.target_phase[l.var() as usize] = if l.is_positive() { 1 } else { -1 };
            }
        }
        if assigned > self.best_assigned {
            self.best_assigned = assigned;
            for &l in &self.trail {
                self.best_phase[l.var() as usize] = if l.is_positive() { 1 } else { -1 };
            }
        }
    }

    /// Rephase when stable mode has passed its conflict limit: drop to level 0, reset the saved phases by the next
    /// step of the schedule, make them the targets, and set the next limit. A walk that finds a model of every
    /// clause the solver was given leaves that model as the phases, which the search then decides without a
    /// conflict. Returns `false` when level-0 propagation shows the formula unsatisfiable.
    fn maybe_rephase(&mut self) -> bool {
        if !self.stable() || self.conflicts <= self.rephase_limit {
            return true;
        }
        self.backtrack_to(0);
        if let Some(ci) = self.propagate() {
            self.found_empty(ci);
            return false;
        }
        self.rephased += 1;
        let step = (self.rephased - 1) % 6;
        match step {
            0 | 3 => {
                for v in 0..self.num_vars {
                    if self.best_phase[v] != 0 {
                        self.saved_phase[v] = self.best_phase[v] > 0;
                    }
                }
                self.best_assigned = 0;
            }
            1 | 4 if self.walk_enabled => self.walk_phases(),
            1 | 4 => {}
            2 => {
                for v in 0..self.num_vars {
                    self.saved_phase[v] = !self.initial_phase[v];
                }
            }
            _ => self.saved_phase.clone_from(&self.initial_phase),
        }
        for v in 0..self.num_vars {
            self.target_phase[v] = if self.saved_phase[v] { 1 } else { -1 };
        }
        self.target_assigned = 0;
        let k = self.rephased as f64;
        let scale = k * (k + 9.0).log10().powi(3);
        self.rephase_limit = self.conflicts + (REPHASE_INT * scale) as u64;
        true
    }

    /// The walking rephase: ProbSAT over the clauses the solver was given (learned ones are implied by them), from
    /// the decision phases, the level-0 facts held. An improvement on the start becomes the saved phases.
    fn walk_phases(&mut self) {
        let reference = (self.ticks - self.walk_ticks).max(WALK_MIN_REFERENCE);
        self.walk_ticks = self.ticks;
        self.walks += 1;
        let start: Vec<bool> = (0..self.num_vars as Var).map(|v| self.decision_lit(v).is_positive()).collect();
        let (mut width, mut count) = (0usize, 0usize);
        for ci in 0..self.arena.count() {
            if !self.learned_flag[ci] {
                width += self.arena.clause(ci).len();
                count += 1;
            }
        }
        let options = crate::walk::WalkOptions {
            cb: if self.walks % 2 == 1 { crate::walk::fitted_cb(width as f64 / count.max(1) as f64) } else { 2.0 },
            seed: self.walks,
            step_limit: reference * WALK_EFFORT_PERMILLE / 1000,
        };
        let vals = &self.vals;
        let fixed = |v: Var| match vals[Lit::pos(v).index()] {
            Val::True => Some(true),
            Val::False => Some(false),
            Val::Unset => None,
        };
        let originals = (0..self.arena.count()).filter(|&ci| !self.learned_flag[ci]).map(|ci| self.arena.clause(ci));
        let walk = crate::walk::walk(self.num_vars, originals, fixed, &start, options);
        if walk.unsatisfied < walk.started_unsatisfied {
            for v in 0..self.num_vars {
                if self.var_val(v) == Val::Unset {
                    self.saved_phase[v] = walk.phases[v];
                }
            }
        }
    }

    /// Let stable-mode decisions follow target phases or not (default on). For A/B benchmarking; verdict-invariant.
    pub fn set_target_phases(&mut self, on: bool) {
        self.target_enabled = on;
    }

    /// Run the walking rephase or skip it (default on). For A/B benchmarking; verdict-invariant.
    pub fn set_walk(&mut self, on: bool) {
        self.walk_enabled = on;
    }

    /// Lucky phases (Kissat's `lucky.c`, after CaDiCaL): before search, at level 0, try complete assignments made by
    /// deciding every variable in turn and propagating — the decision phases in variable order, then all false and
    /// all true, forward and backward. A decision that conflicts is flipped; a flip that conflicts too ends the
    /// attempt. A first decision that conflicts on its own is a failed literal: its negation is learned as a unit (a
    /// RUP step) and the attempt goes on, and a unit that conflicts proves the formula unsatisfiable. An attempt that
    /// assigns every variable is a model. The trail is left at level 0 either way. Off with a decision set (its
    /// excluded variables are a theory's to assign), with LRAT tracing (its units carry no hints), or by
    /// [`Solver::set_lucky`].
    fn lucky(&mut self) -> Lucky {
        if !self.lucky_enabled || self.lrat || self.decision_mask.is_some() || !self.trail_lim.is_empty() {
            return Lucky::Unknown;
        }
        if self.empty_clause {
            return Lucky::Unsat;
        }
        if let Some(ci) = self.propagate() {
            self.found_empty(ci);
            return Lucky::Unsat;
        }
        // The attempts run on a copy: their propagation moves watches and their backtracking saves phases, and the
        // search after a fruitless pass should meet the formula as it would have without one. What they prove comes
        // back — a model, or the units their failed first decisions refute, learned here — and nothing else does.
        let mut probe = self.clone();
        let mut units = Vec::new();
        match probe.lucky_attempts(&mut units) {
            Lucky::Sat(model) => {
                self.lucky_hits += 1;
                Lucky::Sat(model)
            }
            found => {
                for u in units {
                    if self.val_of(u) == Val::Unset {
                        self.add_clause_raw(vec![u], true);
                    }
                }
                if self.empty_clause {
                    return Lucky::Unsat;
                }
                if let Some(ci) = self.propagate() {
                    self.found_empty(ci);
                    return Lucky::Unsat;
                }
                debug_assert!(!matches!(found, Lucky::Unsat), "the units the probe refuted with refute here too");
                Lucky::Unknown
            }
        }
    }

    /// The attempts themselves, on the probe copy [`Solver::lucky`] makes; each unit a failed first decision
    /// refutes is learned and pushed to `units`.
    fn lucky_attempts(&mut self, units: &mut Vec<Lit>) -> Lucky {
        let n = self.num_vars as Var;
        let attempts: [(bool, Option<bool>); 5] = [(true, None), (true, Some(false)), (true, Some(true)), (false, Some(false)), (false, Some(true))];
        for (forward, phase) in attempts {
            let mut failed = false;
            for k in 0..n {
                let v = if forward { k } else { n - 1 - k };
                if self.var_val(v as usize) != Val::Unset {
                    continue;
                }
                let lit = match phase {
                    Some(p) => Lit::new(v, p),
                    None => self.decision_lit(v),
                };
                self.trail_lim.push(self.trail.len());
                self.enqueue(lit, Reason::Decision);
                if self.propagate().is_none() {
                    continue;
                }
                if self.trail_lim.len() > 1 {
                    self.backtrack_to(self.trail_lim.len() as u32 - 1);
                    self.trail_lim.push(self.trail.len());
                    self.enqueue(lit.negated(), Reason::Decision);
                    if self.propagate().is_some() {
                        failed = true;
                        break;
                    }
                } else {
                    self.backtrack_to(0);
                    self.add_clause_raw(vec![lit.negated()], true);
                    units.push(lit.negated());
                    if self.empty_clause {
                        return Lucky::Unsat;
                    }
                    if let Some(ci) = self.propagate() {
                        self.found_empty(ci);
                        return Lucky::Unsat;
                    }
                }
            }
            if !failed {
                return Lucky::Sat((0..self.num_vars).map(|v| self.var_val(v) == Val::True).collect());
            }
            self.backtrack_to(0);
        }
        Lucky::Unknown
    }

    /// Try lucky phases before search or not (default on). For A/B benchmarking; verdict-invariant either way.
    pub fn set_lucky(&mut self, on: bool) {
        self.lucky_enabled = on;
    }

    /// Solves this solver answered by a lucky phase.
    pub fn lucky_hits(&self) -> u64 {
        self.lucky_hits
    }

    /// Rephases this solve.
    pub fn rephases(&self) -> u64 {
        self.rephased
    }

    /// Local-search walks this solve.
    pub fn walks(&self) -> u64 {
        self.walks
    }

    /// Perform a restart: jump to level 0, advance the Luby counter, and reset the Glucose fast
    /// window so the next trigger measures freshly.
    fn do_restart(&mut self) {
        self.backtrack_to(0);
        self.csr = 0;
        self.restarts += 1;
        self.restart_no += 1;
        self.restart_limit = luby(self.restart_no) * self.luby_unit();
        self.lbd_fast.clear();
        self.lbd_fast_sum = 0;
    }

    /// Grow the solver by `k` fresh variables, numbered after those it has, and return the first one's number: each
    /// unassigned and unconstrained, a decision variable unless a decision set restricts them (one then joins it by
    /// [`Solver::add_decision_vars`]). A user that encodes a formula a part at a time numbers its variables densely
    /// as it goes, so a solver is as large as what it was asked.
    pub fn add_vars(&mut self, k: usize) -> usize {
        let first = self.num_vars;
        let n = first + k;
        self.num_vars = n;
        self.watches.resize(n * 2, Vec::new());
        self.vals.resize(n * 2, Val::Unset);
        self.level.resize(n, 0);
        self.reason.resize(n, Reason::Decision);
        self.activity.resize(n, 0.0);
        self.saved_phase.resize(n, false);
        self.initial_phase.resize(n, false);
        self.target_phase.resize(n, 0);
        self.best_phase.resize(n, 0);
        self.seen.resize(n, false);
        self.side_mark.resize(n, false);
        self.min_cache.resize(n, 0);
        self.lbd_stamp.resize(n + 1, 0);
        self.trail_pos.resize(n, 0);
        self.heap_pos.resize(n, -1);
        self.vmtf_prev.resize(n, NONE);
        self.vmtf_next.resize(n, NONE);
        self.vmtf_stamp.resize(n, 0);
        for v in first..n {
            self.vmtf_append(v as Var);
            self.vmtf_search = v as u32;
        }
        match self.decision_mask.as_mut() {
            Some(mask) => mask.resize(n, false),
            None => {
                for v in first..n {
                    self.heap_insert(v as Var);
                }
            }
        }
        first
    }

    /// Add `vars` to the variables decisions fall on, beside those already there: an incremental user grows the set as
    /// it adds the clauses that constrain them. Without a set yet, every variable is one already. Out-of-range and
    /// repeated variables are ignored.
    pub fn add_decision_vars(&mut self, vars: &[usize]) {
        let Some(mask) = self.decision_mask.as_mut() else { return };
        let mut joined: Vec<usize> = Vec::new();
        for &v in vars {
            if v < self.num_vars && !mask[v] {
                mask[v] = true;
                joined.push(v);
            }
        }
        for v in joined {
            if self.var_val(v) == Val::Unset {
                self.heap_insert(v as Var);
            }
        }
    }

    /// Restrict branching to `vars` (DPLL(XOR): the engine's non-pivot variables; the theory forces
    /// the rest). A model is still complete because theory propagation assigns every excluded
    /// variable before all decision variables are exhausted.
    pub fn set_decision_vars(&mut self, vars: &[usize]) {
        let mut mask = vec![false; self.num_vars];
        for &v in vars {
            if v < self.num_vars {
                mask[v] = true;
            }
        }
        self.decision_mask = Some(mask);
        // Rebuild the order heap to hold only the decision candidates.
        self.heap.clear();
        self.heap_pos = vec![-1; self.num_vars];
        for &v in vars {
            if v < self.num_vars && self.heap_pos[v] < 0 {
                self.heap_pos[v] = self.heap.len() as i32;
                self.heap.push(v as Var);
            }
        }
        for i in (0..self.heap.len() / 2).rev() {
            self.sift_down(i);
        }
    }

    /// Variable `g` is the conjunction of `a` and `b`, by clauses the solver holds: [`Solver::set_justifying`]
    /// justifies its value by theirs.
    pub fn set_gate(&mut self, g: usize, a: Lit, b: Lit) {
        if self.gates.len() <= g {
            self.gates.resize(g + 1, None);
        }
        self.gates[g] = Some([a, b]);
    }

    /// Variable `g` is true exactly when one of the products `on` holds and false exactly when one of `off` does, by
    /// clauses the solver holds (each product's literals all true imply `g`'s value): a gate of several leaves, which
    /// [`Solver::set_justifying`] justifies by a product of its value whose literals all hold.
    pub fn set_cover_gate(&mut self, g: usize, on: &[Vec<Lit>], off: &[Vec<Lit>]) {
        if self.covers.len() <= g {
            self.covers.resize(g + 1, 0);
        }
        let first = self.cube_starts.len() as u32 - 1;
        for cube in on.iter().chain(off) {
            self.cube_lits.extend_from_slice(cube);
            self.cube_starts.push(self.cube_lits.len() as u32);
        }
        self.cover_gates.push((first, on.len() as u16, off.len() as u16));
        self.covers[g] = self.cover_gates.len() as u32;
    }

    /// Decide by justification (off by default): every decision sets false a fan-in of a gate ([`Solver::set_gate`])
    /// assigned false with both fan-ins open, and once no such gate is left, a literal of a side clause
    /// ([`Solver::add_side_clause`]) none of whose literals is true; a search left with neither is satisfiable, answered
    /// with what it has assigned (a variable left open reading false). For a circuit: every variable an input or a gate,
    /// and every clause the solver holds either a side clause or true in every evaluation of the gates over their
    /// inputs (the gates' definitions, and whatever follows from them and the side clauses). An assignment without
    /// conflict in which each gate's value is its fan-ins' and each side clause is true is then, whatever value the
    /// inputs it leaves open take, the evaluation of its inputs and a model, so a query's search and its model are the
    /// part of the circuit its assumptions and side clauses need, however much of it the solver holds.
    pub fn set_justifying(&mut self, on: bool) {
        self.justifying = on;
    }

    /// The lookahead a cuber makes on the search as it stands: a decision level of its own holding `l` (unassigned),
    /// propagated to a fixpoint. The literals the level assigned, `l` among them, or `None` when it conflicts; either
    /// way the level stays until [`Solver::retract`] takes it back.
    pub(crate) fn assume(&mut self, l: Lit) -> Option<usize> {
        debug_assert_eq!(self.val_of(l), Val::Unset, "a lookahead assumes an open literal");
        self.trail_lim.push(self.trail.len());
        let before = self.trail.len();
        self.enqueue(l, Reason::Decision);
        match self.propagate() {
            Some(_) => None,
            None => Some(self.trail.len() - before),
        }
    }

    /// Take back the last level [`Solver::assume`] made.
    pub(crate) fn retract(&mut self) {
        let level = self.trail_lim.len().checked_sub(1).expect("a level to take back");
        self.backtrack_to(level as u32);
    }

    /// Propagate the level-0 facts from scratch: false when they refute the formula.
    pub(crate) fn settle(&mut self) -> bool {
        self.backtrack_to(0);
        if self.empty_clause {
            return false;
        }
        match self.propagate() {
            Some(ci) => {
                self.found_empty(ci);
                false
            }
            None => true,
        }
    }

    /// Add a clause the solver holds that a justifying search ([`Solver::set_justifying`]) makes true itself, by deciding
    /// one of its literals, beside the gates it justifies: a constraint on the inputs, or a clause no evaluation of the
    /// gates need satisfy. A search that does not justify reads it as any other clause.
    pub fn add_side_clause(&mut self, lits: Vec<Lit>) {
        self.sides.push(lits.clone());
        self.add_clause(lits);
    }

    /// The value the last search left variable `v` at, a model's after a satisfiable answer, or `None` for one it left
    /// open (a justifying search leaves open what its assumptions do not need) or the solver does not have.
    pub fn value(&self, v: usize) -> Option<bool> {
        match (v < self.num_vars).then(|| self.var_val(v))? {
            Val::True => Some(true),
            Val::False => Some(false),
            Val::Unset => None,
        }
    }

    // --- VSIDS order heap (indexed binary max-heap on `activity`) ---

    fn heap_swap(&mut self, i: usize, j: usize) {
        self.heap.swap(i, j);
        self.heap_pos[self.heap[i] as usize] = i as i32;
        self.heap_pos[self.heap[j] as usize] = j as i32;
    }

    fn sift_up(&mut self, mut i: usize) {
        while i > 0 {
            let p = (i - 1) / 2;
            if self.activity[self.heap[i] as usize] > self.activity[self.heap[p] as usize] {
                self.heap_swap(i, p);
                i = p;
            } else {
                break;
            }
        }
    }

    fn sift_down(&mut self, mut i: usize) {
        let n = self.heap.len();
        loop {
            let (l, r) = (2 * i + 1, 2 * i + 2);
            let mut m = i;
            if l < n && self.activity[self.heap[l] as usize] > self.activity[self.heap[m] as usize]
            {
                m = l;
            }
            if r < n && self.activity[self.heap[r] as usize] > self.activity[self.heap[m] as usize]
            {
                m = r;
            }
            if m == i {
                break;
            }
            self.heap_swap(i, m);
            i = m;
        }
    }

    /// Insert `v` (idempotent: a no-op if already present).
    fn heap_insert(&mut self, v: Var) {
        if self.heap_pos[v as usize] >= 0 {
            return;
        }
        let i = self.heap.len();
        self.heap.push(v);
        self.heap_pos[v as usize] = i as i32;
        self.sift_up(i);
    }

    /// Remove and return the highest-activity variable, or `None` if empty.
    fn heap_pop(&mut self) -> Option<Var> {
        if self.heap.is_empty() {
            return None;
        }
        let top = self.heap[0];
        self.heap_pos[top as usize] = -1;
        let last = self.heap.pop().unwrap();
        if !self.heap.is_empty() {
            self.heap[0] = last;
            self.heap_pos[last as usize] = 0;
            self.sift_down(0);
        }
        Some(top)
    }

    /// Restore the heap after `v`'s activity rose (it can only move toward the root).
    fn heap_increase(&mut self, v: Var) {
        let i = self.heap_pos[v as usize];
        if i >= 0 {
            self.sift_up(i as usize);
        }
    }

    // --- variable move-to-front queue ---

    /// Link `v` at the back of the queue with a fresh stamp (it must not be linked).
    fn vmtf_append(&mut self, v: Var) {
        let vi = v as usize;
        self.vmtf_clock += 1;
        self.vmtf_stamp[vi] = self.vmtf_clock;
        self.vmtf_prev[vi] = self.vmtf_last;
        self.vmtf_next[vi] = NONE;
        if self.vmtf_last == NONE {
            self.vmtf_first = v;
        } else {
            self.vmtf_next[self.vmtf_last as usize] = v;
        }
        self.vmtf_last = v;
    }

    /// Move `v` to the back of the queue; an unassigned `v` becomes the search start.
    fn vmtf_move_to_back(&mut self, v: Var) {
        let vi = v as usize;
        if self.vmtf_last == v {
            self.vmtf_clock += 1;
            self.vmtf_stamp[vi] = self.vmtf_clock;
        } else {
            let (prev, next) = (self.vmtf_prev[vi], self.vmtf_next[vi]);
            if prev == NONE {
                self.vmtf_first = next;
            } else {
                self.vmtf_next[prev as usize] = next;
            }
            self.vmtf_prev[next as usize] = prev;
            self.vmtf_append(v);
        }
        if self.var_val(vi) == Val::Unset {
            self.vmtf_search = v;
        }
    }

    /// Move the variables this conflict's analysis bumped to the back of the queue, oldest stamp first, so their
    /// order among themselves is kept.
    fn vmtf_bump_analyzed(&mut self) {
        let mut bumped = std::mem::take(&mut self.vmtf_bumped);
        bumped.sort_unstable_by_key(|&v| self.vmtf_stamp[v as usize]);
        for &v in &bumped {
            self.vmtf_move_to_back(v);
        }
        bumped.clear();
        self.vmtf_bumped = bumped;
    }

    /// The next decision variable by the queue: walking toward the front from the search start, the first that is
    /// unassigned and a decision candidate.
    fn vmtf_pick(&mut self) -> Option<Var> {
        let mut v = self.vmtf_search;
        while v != NONE {
            let vi = v as usize;
            if self.var_val(vi) == Val::Unset && self.decision_mask.as_ref().is_none_or(|m| m[vi]) {
                self.vmtf_search = v;
                return Some(v);
            }
            v = self.vmtf_prev[vi];
        }
        None
    }

    /// Bump the reason side of each learned clause too (default off). Verdict-invariant either way.
    pub fn set_bump_reasons(&mut self, on: bool) {
        self.bump_reasons = on;
    }

    /// Take in `clauses`, each implied by the formula, as learned clauses: the search drops to level 0, as at a restart,
    /// and each clause joins as the level-0 facts leave it — none when one of its literals holds, a fact when one is
    /// left, a refutation when none is. Nothing changes when `clauses` is empty. `false` when the formula is refuted.
    pub fn import(&mut self, clauses: &[Vec<Lit>]) -> bool {
        if clauses.is_empty() {
            return !self.empty_clause;
        }
        self.backtrack_to(0);
        for clause in clauses {
            if clause.iter().any(|&l| self.lit_true(l)) {
                continue;
            }
            let mut open: Vec<Lit> = clause.iter().copied().filter(|&l| !self.lit_false(l)).collect();
            open.sort_unstable_by_key(|l| l.index());
            open.dedup();
            if open.windows(2).any(|w| w[0].var() == w[1].var()) {
                continue;
            }
            let len = open.len();
            let ci = self.add_clause_raw(open, true);
            self.lbd[ci] = len.min(2) as u32;
            if self.empty_clause {
                return false;
            }
        }
        !self.empty_clause
    }

    /// A flag that, once set, pauses [`Solver::resume`] at its next conflict; `None` for none.
    pub fn set_stop(&mut self, stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>) {
        self.stop = stop;
    }

    /// Reason-side literals bumped by the solves so far.
    pub fn reason_side_bumps(&self) -> u64 {
        self.reason_side_bumps
    }

    /// Let focused-mode decisions follow the move-to-front queue (default off: VSIDS in every mode). For A/B
    /// benchmarking; verdict-invariant either way.
    pub fn set_focused_queue(&mut self, on: bool) {
        self.vmtf_enabled = on;
    }

    /// Total conflicts during the last solve — the search-work metric.
    pub fn conflicts(&self) -> u64 {
        self.conflicts
    }

    /// Total decisions during the last solve.
    pub fn decisions(&self) -> u64 {
        self.decisions
    }

    /// Total BCP propagations during the last solve — the throughput metric.
    pub fn propagations(&self) -> u64 {
        self.propagations
    }

    /// Total restarts during the last solve.
    pub fn restarts(&self) -> u64 {
        self.restarts
    }

    /// Enable or disable LBD-based learned-clause deletion (default on). For A/B benchmarking.
    pub fn set_reduce(&mut self, on: bool) {
        self.reduce_enabled = on;
    }

    /// Set the reduction trigger (tuning / stress-testing): under [`ReducePolicy::Halving`] the live learned clauses
    /// that start the next reduction (default `HALVING_LIMIT`), under [`ReducePolicy::Tiered`] the base number of
    /// conflicts between reductions (default `REDUCE_INT`; the `k`-th interval is the base times `√k`).
    pub fn set_reduce_limit(&mut self, limit: usize) {
        self.reduce_limit = limit.max(1);
        self.reduce_interval = limit.max(1) as u64;
        self.reduce_next = self.conflicts + self.reduce_interval;
    }

    /// Select the learned-clause reduction policy (default [`ReducePolicy::Halving`]). Verdict-invariant either way.
    pub fn set_reduce_policy(&mut self, policy: ReducePolicy) {
        self.reduce_policy = policy;
    }

    /// Learned-clause reductions this solver has run.
    pub fn reductions(&self) -> u64 {
        self.reductions
    }

    /// Seed the saved-phase polarities (the order decisions try first) from an external assignment.
    /// Purely a search-order hint — it never changes which models exist, so it cannot affect the
    /// verdict — but starting decisions on, e.g., a GF(2)-consistent assignment lets the hybrid XOR
    /// route begin on the linear system's solution manifold and only repair the residual clauses.
    pub fn set_initial_phase(&mut self, phases: &[bool]) {
        for (v, &p) in phases.iter().enumerate().take(self.num_vars) {
            self.saved_phase[v] = p;
            self.initial_phase[v] = p;
        }
    }

    /// The number of learned clauses currently live in the database (originals excluded).
    pub fn live_learned(&self) -> usize {
        self.arena.count().saturating_sub(self.n_original)
    }

    /// The learned clauses produced during the last solve (the DRAT/LRAT proof skeleton).
    pub fn learned(&self) -> &[LearnedClause] {
        &self.learned_log
    }

    /// The learned clauses since the last take, moved out: a caller that keeps its own proof log takes each once, so
    /// the solver does not hold every one for its whole life.
    pub fn take_learned(&mut self) -> Vec<LearnedClause> {
        std::mem::take(&mut self.learned_log)
    }

    /// The ids of the clauses the database holds. A hint a later solve gives names one of these or a clause it learned
    /// since, so a proof log may forget whatever none of them rests on.
    pub fn live_ids(&self) -> &[u64] {
        &self.clause_ids
    }

    /// The ORIGINAL clauses (those present before solving), copied out of the arena, for a RUP
    /// checker to replay over.
    pub fn original_clauses(&self) -> Vec<Vec<Lit>> {
        (0..self.n_original).map(|ci| self.arena.clause(ci).to_vec()).collect()
    }

    /// Add a clause. An empty clause makes the formula trivially unsatisfiable; a unit
    /// clause is enqueued as a top-level fact. Between incremental solves the search is
    /// dropped first and the clause read against the level-0 facts: its literals not yet
    /// false lead, so the two it watches are ones that can still become false; one left is a
    /// fact enqueued now, none left makes the formula unsatisfiable.
    pub fn add_clause(&mut self, lits: Vec<Lit>) {
        let id = self.next_id;
        self.add_clause_with_id(lits, id);
    }

    /// [`Solver::add_clause`] with the id the clause's hints name it by chosen by the caller, above every id before it.
    /// A tautology is dropped and its id left unused.
    pub fn add_clause_with_id(&mut self, mut lits: Vec<Lit>, id: u64) {
        self.next_id = self.next_id.max(id + 1);
        // Dedup and drop a clause that is already a tautology (`v ∨ ¬v`).
        lits.sort_by_key(|l| l.0);
        lits.dedup();
        for w in lits.windows(2) {
            if w[0].var() == w[1].var() {
                // contains both polarities of some var → tautology, skip
                return;
            }
        }
        self.backtrack_to(0);
        if lits.len() >= 2 {
            lits.sort_by_key(|&l| self.lit_false(l));
            let open = lits.iter().filter(|&&l| !self.lit_false(l)).count();
            if open <= 1 {
                let ci = self.store(lits.clone(), false, id, Vec::new());
                match lits.first() {
                    Some(&l) if open == 1 && self.val_of(l) == Val::Unset => self.enqueue(l, Reason::Clause(ci)),
                    Some(&l) if open == 1 && self.lit_true(l) => {}
                    _ => self.found_empty(ci),
                }
                return;
            }
        }
        self.store(lits, false, id, Vec::new());
    }

    /// Number the clauses the solver makes from `id` on (when that is above every id it has used): a caller that gives
    /// its own clauses ids from another range keeps the two apart.
    pub fn set_next_clause_id(&mut self, id: u64) {
        self.next_id = self.next_id.max(id);
    }

    /// The next clause id, taken: one a caller gives a clause it derives itself.
    pub fn next_clause_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id - 1
    }

    /// Record LRAT hints: each learned clause's, and each refutation's (see [`Solver::unsat_hints`]). For pure CNF
    /// solving: inprocessing, which strengthens clauses without a chain, is turned off, and a theory's clauses carry
    /// none.
    pub fn set_lrat(&mut self, on: bool) {
        self.lrat = on;
        if on {
            self.inprocess_enabled = false;
        }
    }

    /// After an unsatisfiable solve under assumptions: the assumptions its refutation rests on, in the order they were
    /// given, the formula refuting their conjunction alone; empty when the formula itself is unsatisfiable.
    pub fn unsat_core(&self) -> &[Lit] {
        &self.core
    }

    /// The decisions variable `v`'s value rests on: every one its reasons reach back to, level-0 facts aside.
    fn decisions_behind(&self, v: Var) -> HashSet<Var> {
        let (mut stack, mut seen, mut decisions) = (vec![v], HashSet::new(), HashSet::new());
        while let Some(u) = stack.pop() {
            if !seen.insert(u) || self.var_val(u as usize) == Val::Unset || self.level[u as usize] == 0 {
                continue;
            }
            match self.reason[u as usize] {
                Reason::Decision => {
                    decisions.insert(u);
                }
                reason => stack.extend(self.reason_lits(reason).iter().map(|l| l.var()).filter(|&w| w != u)),
            }
        }
        decisions
    }

    /// After an unsatisfiable solve under assumptions, with tracing on: the hints of the clause of the assumptions'
    /// negations (the empty clause's when the formula itself is unsatisfiable), each id in order a clause that is unit
    /// under that clause's negation, the last conflicting.
    pub fn unsat_hints(&self) -> &[u64] {
        &self.unsat_hints
    }

    /// The trail-ordered reasons (by clause index) of every variable that `vars` lean on: each of them and, through its
    /// reason, every variable that reason's other literals hold, stopping at decisions and at level-0 facts, each of
    /// which its unit clause stands for ([`Solver::unit_of`]).
    fn cone(&mut self, vars: impl IntoIterator<Item = Var>) -> Vec<usize> {
        let mut stack: Vec<Var> = vars.into_iter().collect();
        let mut visited: HashSet<Var> = HashSet::new();
        let mut found: Vec<(u32, usize)> = Vec::new();
        while let Some(v) = stack.pop() {
            if !visited.insert(v) || self.var_val(v as usize) == Val::Unset {
                continue;
            }
            if self.level[v as usize] == 0 {
                if let Some(ci) = self.unit_of(v) {
                    found.push((self.trail_pos[v as usize], ci));
                }
                continue;
            }
            if let Reason::Clause(ci) = self.reason[v as usize] {
                found.push((self.trail_pos[v as usize], ci));
                stack.extend(self.arena.clause(ci).iter().map(|l| l.var()).filter(|&w| w != v));
            }
        }
        found.sort_unstable();
        found.into_iter().map(|(_, ci)| ci).collect()
    }

    /// The database index of the unit clause of the level-0 fact on `v`, derived the first time a chain needs it: the
    /// units of its reason's other literals (each false at level 0), then the reason. It is logged as a learned clause
    /// and kept as a permanent one, `v`'s reason from then on, so a later chain names the fact by one hint and not by
    /// everything level 0 derived it from. `None` for a fact with no reason.
    fn unit_of(&mut self, v: Var) -> Option<usize> {
        let long = |s: &Solver, w: Var| matches!(s.reason[w as usize], Reason::Clause(ci) if s.arena.clause(ci).len() > 1);
        let mut stack: Vec<(Var, bool)> = vec![(v, false)];
        while let Some((u, ready)) = stack.pop() {
            let Reason::Clause(ci) = self.reason[u as usize] else { continue };
            if self.arena.clause(ci).len() == 1 {
                continue;
            }
            if !ready {
                stack.push((u, true));
                for k in 0..self.arena.clause(ci).len() {
                    let w = self.arena.clause(ci)[k].var();
                    if w != u && long(self, w) {
                        stack.push((w, false));
                    }
                }
                continue;
            }
            let mut hints: Vec<u64> = Vec::with_capacity(self.arena.clause(ci).len());
            for l in self.arena.clause(ci) {
                if let (true, Reason::Clause(cj)) = (l.var() != u, self.reason[l.var() as usize]) {
                    hints.push(self.clause_ids[cj]);
                }
            }
            hints.push(self.clause_ids[ci]);
            let fact = Lit::new(u, self.var_val(u as usize) == Val::True);
            let id = self.next_clause_id();
            self.learned_log.push(LearnedClause { lits: vec![fact], id, hints });
            let unit = self.store(vec![fact], false, id, Vec::new());
            self.reason[u as usize] = Reason::Clause(unit);
        }
        match self.reason[v as usize] {
            Reason::Clause(ci) => Some(ci),
            Reason::Decision => None,
            Reason::Theory(_) => unreachable!("a proof-logging solve keeps a theory's clauses"),
        }
    }

    /// [`Solver::cone`]'s clause ids.
    fn cone_hints(&mut self, vars: impl IntoIterator<Item = Var>) -> Vec<u64> {
        self.cone(vars).into_iter().map(|ci| self.clause_ids[ci]).collect()
    }

    /// The hints of `clause` from `reasons` (clause indices in trail order): replayed under its negation, each reason
    /// a unit is kept, one already satisfied is dropped, and the first that conflicts ends the chain. The negation of
    /// a set of assumptions can contradict a level-0 fact before the reason the solver found it by.
    fn replayed(&self, clause: &[Lit], reasons: &[usize]) -> Vec<u64> {
        let mut assign: std::collections::HashMap<Var, bool> = clause.iter().map(|l| (l.var(), !l.is_positive())).collect();
        let truth = |assign: &std::collections::HashMap<Var, bool>, l: Lit| assign.get(&l.var()).map(|&v| v == l.is_positive());
        let mut hints = Vec::new();
        for &ci in reasons {
            let mut open: Option<Lit> = None;
            let mut satisfied = false;
            let mut several = false;
            for &l in self.arena.clause(ci) {
                match truth(&assign, l) {
                    Some(true) => satisfied = true,
                    Some(false) => {}
                    None if open.is_none() || open == Some(l) => open = Some(l),
                    None => several = true,
                }
            }
            if satisfied {
                continue;
            }
            hints.push(self.clause_ids[ci]);
            match open {
                None => return hints,
                Some(_) if several => return reasons.iter().map(|&ci| self.clause_ids[ci]).collect(),
                Some(u) => {
                    assign.insert(u.var(), u.is_positive());
                }
            }
        }
        hints
    }

    /// The formula is unsatisfiable at level 0, clause `ci` false there: the empty clause's hints are the units of its
    /// literals' level-0 facts, then it.
    fn found_empty(&mut self, ci: usize) {
        self.empty_clause = true;
        if self.lrat && self.empty_hints.is_empty() {
            let vars: Vec<Var> = self.arena.clause(ci).iter().map(|l| l.var()).collect();
            let mut hints = self.cone_hints(vars);
            hints.push(self.clause_ids[ci]);
            self.empty_hints = hints;
        }
    }

    /// Internal: register a clause under the next free id and set up its two watches. `learned`
    /// clauses are also logged, with the hints [`Solver::analyze`] left. Returns the clause index.
    fn add_clause_raw(&mut self, lits: Vec<Lit>, learned: bool) -> usize {
        let id = self.next_clause_id();
        let hints = if learned { std::mem::take(&mut self.pending_hints) } else { Vec::new() };
        self.store(lits, learned, id, hints)
    }

    /// Register a clause with its id and hints and set up its two watches.
    fn store(&mut self, lits: Vec<Lit>, learned: bool, id: u64, hints: Vec<u64>) -> usize {
        let ci = self.arena.count();
        assert!(ci < MAX_CLAUSES, "the clause database is full: {MAX_CLAUSES} clauses");
        self.clause_ids.push(id);
        // Keep the per-clause LBD vector aligned with `clauses` (grows by exactly one here).
        // Originals/units default to `u32::MAX` (never deleted); the caller overwrites a learned
        // clause's slot with its real LBD.
        self.lbd.push(u32::MAX);
        self.used.push(if learned { MAX_USED } else { 0 });
        self.learned_flag.push(learned);
        self.learned_live += learned as usize;
        if learned {
            self.learned_log.push(LearnedClause { lits: lits.clone(), id, hints });
        }
        self.arena.push(&lits);
        if lits.is_empty() {
            self.found_empty(ci);
            return ci;
        }
        if lits.len() == 1 {
            let l = lits[0];
            // A unit clause forces its literal. If it is already FALSE on the trail, the
            // formula is unsatisfiable (a top-level conflict); if already true, nothing to
            // do; else enqueue it.
            match self.val_of(l) {
                Val::True => {}
                Val::False => self.found_empty(ci),
                Val::Unset => self.enqueue(l, Reason::Clause(ci)),
            }
            return ci;
        }
        self.watch(ci);
        ci
    }

    /// Clause `ci`'s two watchers, on its first two literals, each blocked by the other.
    fn watch(&mut self, ci: usize) {
        let (w0, w1, binary) = {
            let c = self.arena.clause(ci);
            (c[0], c[1], c.len() == 2)
        };
        let off = self.arena.offset(ci);
        let make = |blocker: Lit| if binary { Watcher::binary(ci, blocker) } else { Watcher::long(off, blocker) };
        self.watches[w0.index()].push(make(w1));
        self.watches[w1.index()].push(make(w0));
    }

    #[inline]
    fn val_of(&self, l: Lit) -> Val {
        self.vals[l.index()]
    }

    /// The value of variable `v`: its positive literal's.
    #[inline]
    fn var_val(&self, v: usize) -> Val {
        self.vals[v << 1]
    }

    #[inline]
    fn lit_true(&self, l: Lit) -> bool {
        self.val_of(l) == Val::True
    }
    #[inline]
    fn lit_false(&self, l: Lit) -> bool {
        self.val_of(l) == Val::False
    }

    /// Assign `l` true with the given reason, push it on the trail. Assumes `l` is currently
    /// unset (callers check).
    fn enqueue(&mut self, l: Lit, r: Reason) {
        self.enqueue_at(l, r, self.trail_lim.len() as u32);
    }

    /// [`Solver::enqueue`] at decision level `level`, which under chronological backtracking may lie below the
    /// current one: the literal then sits on the trail above literals of higher levels.
    fn enqueue_at(&mut self, l: Lit, r: Reason, level: u32) {
        let v = l.var() as usize;
        self.vals[l.index()] = Val::True;
        self.vals[l.negated().index()] = Val::False;
        self.level[v] = level;
        self.reason[v] = r;
        self.trail_pos[v] = self.trail.len() as u32;
        self.trail.push(l);
    }

    /// Two-watched-literal unit propagation. Returns the index of a conflicting clause, or
    /// `None` at a Boolean fixpoint.
    fn propagate(&mut self) -> Option<usize> {
        while self.qhead < self.trail.len() {
            let p = self.trail[self.qhead];
            self.qhead += 1;
            self.propagations += 1;
            // Clauses watching ¬p may have become unit/false.
            let false_lit = p.negated();
            let mut wi = 0;
            // Take the watch list out to satisfy the borrow checker; we rebuild it.
            let mut watchers = std::mem::take(&mut self.watches[false_lit.index()]);
            self.ticks += 1 + (watchers.len() * std::mem::size_of::<Watcher>()).div_ceil(64) as u64;
            'next_clause: while wi < watchers.len() {
                // Blocking literal: if it is already true, the clause is satisfied — skip it
                // without dereferencing the clause (the cache-miss saver).
                if self.lit_true(watchers[wi].blocker) {
                    wi += 1;
                    continue;
                }
                // Binary clause: the blocker is the *only* other literal and it is not true, so the
                // clause is immediately unit (blocker unassigned) or a conflict (blocker false) — with
                // no clause dereference at all. The bulk of a mature learned database is binary, so this
                // is a real BCP win, and it is exact.
                if watchers[wi].is_binary() {
                    let other = watchers[wi].blocker;
                    let ci = watchers[wi].target();
                    if self.lit_false(other) {
                        self.watches[false_lit.index()] = watchers;
                        return Some(ci);
                    }
                    let level = if self.chrono_now { self.level[false_lit.var() as usize] } else { self.trail_lim.len() as u32 };
                    self.enqueue_at(other, Reason::Clause(ci), level);
                    wi += 1;
                    continue;
                }
                let off = watchers[wi].target();
                self.ticks += 1;
                let clause_len = self.arena.len_at(off);
                // Ensure the watched literal we keep is in slot 0/1; make slot 1 = false_lit.
                if self.arena.words[off] == false_lit {
                    self.arena.words.swap(off, off + 1);
                }
                // Slot 0 is the other watched literal; refresh it as this watch's blocker.
                let other = self.arena.words[off];
                watchers[wi].blocker = other;
                // If it is already satisfied, the clause is fine — keep watching false_lit.
                if self.lit_true(other) {
                    wi += 1;
                    continue;
                }
                // Look for a new, non-false literal to watch (slots 2..).
                for k in 2..clause_len {
                    let lk = self.arena.words[off + k];
                    if !self.lit_false(lk) {
                        // Move lk into slot 1, watch it (blocked by `other`), drop this watch. Only a clause
                        // of three or more literals reaches here, so the new watcher is a long one.
                        self.arena.words.swap(off + 1, off + k);
                        self.watches[lk.index()].push(Watcher::long(off, other));
                        watchers.swap_remove(wi);
                        continue 'next_clause;
                    }
                }
                // No new watch: clause is unit on `other` or conflicting.
                let ci = self.arena.index_at(off);
                if self.lit_false(other) {
                    // Conflict: restore the rest of this watch list and report.
                    self.watches[false_lit.index()] = watchers;
                    return Some(ci);
                }
                // Unit: propagate `other`. Under chronological backtracking it takes the highest level among the
                // false literals, and that literal becomes the second watch: a backtrack that unassigns `other` then
                // unassigns it too, so the clause can never sit unit with both watches false.
                if self.chrono_now {
                    let (mut high, mut at) = (self.level[false_lit.var() as usize], 1usize);
                    for k in 2..clause_len {
                        let lk = self.level[self.arena.words[off + k].var() as usize];
                        if lk > high {
                            high = lk;
                            at = k;
                        }
                    }
                    if at != 1 {
                        self.arena.words.swap(off + 1, off + at);
                        let watched = self.arena.words[off + 1];
                        self.watches[watched.index()].push(Watcher::long(off, other));
                        watchers.swap_remove(wi);
                        self.enqueue_at(other, Reason::Clause(ci), high);
                        continue;
                    }
                    self.enqueue_at(other, Reason::Clause(ci), high);
                    wi += 1;
                    continue;
                }
                self.enqueue(other, Reason::Clause(ci));
                wi += 1;
            }
            self.watches[false_lit.index()] = watchers;
        }
        None
    }

    /// The literals of `reason`: its clause's, or its theory explanation's; none for a decision.
    #[inline]
    fn reason_lits(&self, reason: Reason) -> &[Lit] {
        match reason {
            Reason::Clause(ci) => self.arena.clause(ci),
            Reason::Theory(k) => self.explanations.get(k),
            Reason::Decision => &[],
        }
    }

    /// First-UIP conflict analysis (GRASP/MiniSat). Walks the implication graph back from
    /// the conflicting clause to the first unique implication point at the current level,
    /// producing a learned clause and the level to backjump to. A theory's token on the way is
    /// asked for its clause, held until the analysis ends.
    fn analyze(&mut self, conflict: usize, theories: &mut [Box<dyn Theory>]) -> (Vec<Lit>, u32, u32) {
        let decision_level = self.trail_lim.len() as u32;
        let mut learned: Vec<Lit> = vec![Lit::pos(0)]; // slot 0 reserved for the UIP
        let mut counter = 0usize; // unresolved literals at the current level
        let mut p: Option<Lit> = None;
        let mut trail_idx = self.trail.len();
        let mut reason = Reason::Clause(conflict);
        // With tracing: the variables whose reasons the resolution used.
        let mut resolved: Vec<Var> = Vec::new();

        loop {
            match reason {
                Reason::Clause(clause) if self.reduce_policy == ReducePolicy::Tiered && self.learned_flag[clause] => {
                    self.mark_used(clause);
                }
                Reason::Theory(k) => self.explanations.ask(k, theories),
                _ => {}
            }
            // Resolve with `reason` (its literals are all false; for the conflict clause, and for
            // each antecedent reason thereafter). Index rather than clone it: copying each `Lit`
            // out ends the immutable borrow before we `bump`/mark, so the hot 1UIP loop never
            // allocates — the way a production solver walks reason clauses.
            for k in 0..self.reason_lits(reason).len() {
                let q = self.reason_lits(reason)[k];
                if Some(q) == p {
                    continue; // skip the pivot we just resolved on
                }
                let v = q.var() as usize;
                if !self.seen[v] && self.level[v] > 0 {
                    self.bump(q.var());
                    self.seen[v] = true;
                    if self.level[v] == decision_level {
                        counter += 1;
                    } else {
                        learned.push(q);
                    }
                }
            }
            // Pick the next literal to resolve: the most recent seen literal on the trail.
            loop {
                trail_idx -= 1;
                let l = self.trail[trail_idx];
                let lv = l.var() as usize;
                if self.seen[lv] && self.level[lv] == decision_level {
                    p = Some(l);
                    break;
                }
            }
            let pv = p.unwrap().var() as usize;
            self.seen[pv] = false;
            counter -= 1;
            if counter == 0 {
                break;
            }
            reason = match self.reason[pv] {
                Reason::Decision => unreachable!("UIP reached a decision before counter hit 0"),
                antecedent => antecedent,
            };
            if self.lrat {
                resolved.push(pv as Var);
            }
        }
        // The asserting literal is ¬p (p is the UIP, currently true).
        learned[0] = p.unwrap().negated();
        // Clear the `seen` marks left on the non-UIP learned literals.
        for &l in &learned[1..] {
            self.seen[l.var() as usize] = false;
        }
        // Recursive minimization: drop any learned literal whose reason is already covered by the
        // rest of the clause. Strictly strengthens the (still-implied) clause, shrinking its size
        // and LBD — verdict-invariant.
        let redundant = self.minimize(&mut learned);
        if self.bump_reasons {
            self.bump_reason_side(&learned);
        }
        self.explanations.release();
        if self.lrat {
            // The chain: under the clause's negation, the reasons of the resolved and the minimized-away variables and
            // of every level-0 fact a clause among them leans on, in trail order, then the conflict.
            let mut used: Vec<Var> = resolved;
            used.extend(redundant);
            let mut zero: Vec<Var> = self.arena.clause(conflict).iter().map(|l| l.var()).filter(|&v| self.level[v as usize] == 0).collect();
            for &v in &used {
                if let Reason::Clause(ci) = self.reason[v as usize] {
                    zero.extend(self.arena.clause(ci).iter().map(|l| l.var()).filter(|&w| self.level[w as usize] == 0));
                }
            }
            let mut ordered: Vec<(u32, u64)> = used
                .iter()
                .filter_map(|&v| match self.reason[v as usize] {
                    Reason::Clause(ci) => Some((self.trail_pos[v as usize], self.clause_ids[ci])),
                    Reason::Decision => None,
                    Reason::Theory(_) => unreachable!("a proof-logging solve keeps a theory's clauses"),
                })
                .collect();
            let level0 = self.cone_hints(zero);
            ordered.sort_unstable();
            ordered.dedup();
            let mut hints = level0;
            hints.extend(ordered.into_iter().map(|(_, id)| id));
            hints.push(self.clause_ids[conflict]);
            self.pending_hints = hints;
        }
        // Move the highest-decision-level literal into slot 1. The two-watched-literal invariant
        // requires the second watch to be the most recently falsified literal (the backjump
        // level); otherwise the learned clause can be falsified later without the watch firing.
        let mut backjump = 0u32;
        let mut max_idx = 1usize;
        for i in 1..learned.len() {
            let lv = self.level[learned[i].var() as usize];
            if lv > backjump {
                backjump = lv;
                max_idx = i;
            }
        }
        if learned.len() >= 2 {
            learned.swap(1, max_idx);
        }
        // LBD (distinct decision levels among the learned literals) via generation-stamped counting —
        // no per-conflict Vec allocation or sort. Each level is counted once per conflict.
        self.lbd_gen = self.lbd_gen.wrapping_add(1);
        if self.lbd_gen == 0 {
            // Generation wrapped after 2³² conflicts — reset the stamps once and restart at 1.
            for s in self.lbd_stamp.iter_mut() {
                *s = 0;
            }
            self.lbd_gen = 1;
        }
        let mut lbd = 0u32;
        for &l in learned.iter() {
            let lev = self.level[l.var() as usize] as usize;
            if self.lbd_stamp[lev] != self.lbd_gen {
                self.lbd_stamp[lev] = self.lbd_gen;
                lbd += 1;
            }
        }
        if self.vmtf_enabled {
            self.vmtf_bump_analyzed();
        }
        (learned, backjump, lbd)
    }

    /// Recursive learned-clause minimization (Sörensson & Biere, 2009). A non-asserting literal is
    /// dropped if its reason clause's other literals are all already in the learned clause, are
    /// level-0 facts, or are themselves removable — meaning the literal is redundant. The result is
    /// a sub-clause still implied by the formula, so it is verdict-invariant; it just yields
    /// shorter, lower-LBD clauses that propagate and delete better.
    /// Drop the learned literals implied by the rest; the variables found redundant, whose reasons the shortened clause
    /// leans on.
    fn minimize(&mut self, learned: &mut Vec<Lit>) -> Vec<Var> {
        // Mark every learned literal in the shared `seen` array — the in-clause test, no per-conflict
        // HashSet — and borrow the reusable memo/touch buffers out of `self` so `lit_redundant` can
        // read `self` immutably while writing them (no per-conflict HashMap).
        for &l in learned.iter() {
            self.seen[l.var() as usize] = true;
        }
        let mut cache = std::mem::take(&mut self.min_cache);
        let mut touched = std::mem::take(&mut self.min_touched);
        // Pass 1: fill the memo (`cache[var] == 1` ⟺ that literal is redundant), with `seen` intact.
        for i in 1..learned.len() {
            let _ = self.lit_redundant(learned[i], &mut cache, &mut touched, 0);
        }
        // Clear `seen` off the full (not-yet-compacted) learned clause — the next analyze starts clean.
        for &l in learned.iter() {
            self.seen[l.var() as usize] = false;
        }
        // Pass 2: drop the redundant literals.
        let mut j = 1;
        for i in 1..learned.len() {
            let l = learned[i];
            if cache[l.var() as usize] != 1 {
                learned[j] = l;
                j += 1;
            }
        }
        learned.truncate(j);
        let redundant: Vec<Var> = if self.lrat { touched.iter().copied().filter(|&v| cache[v as usize] == 1).collect() } else { Vec::new() };
        // Sparse-reset the memo and hand the buffers back to `self` for the next conflict.
        for &v in &touched {
            cache[v as usize] = 0;
        }
        touched.clear();
        self.min_cache = cache;
        self.min_touched = touched;
        redundant
    }

    fn lit_redundant(
        &self,
        lit: Lit,
        cache: &mut [u8],
        touched: &mut Vec<Var>,
        depth: u32,
    ) -> bool {
        let v = lit.var();
        match cache[v as usize] {
            1 => return true,
            2 => return false,
            _ => {}
        }
        // Bound the recursion: a deep implication chain could otherwise overflow the stack. Not
        // minimizing a literal is always sound, so we conservatively report "not redundant" past the
        // limit — and do NOT cache that cutoff, which is not the true answer.
        if depth >= 48 {
            return false;
        }
        let result = match self.reason[v as usize] {
            // A decision literal anchors its level; it is never redundant. Nor, conservatively, a literal whose
            // explanation the analysis did not ask for.
            Reason::Decision => false,
            Reason::Theory(k) if !self.explanations.ready(k) => false,
            // No clone: the reason and the recursive call are both immutable borrows of `self`.
            // `seen[qv]` is the in-learned-clause test (marked by `minimize`).
            reason => self.reason_lits(reason).iter().all(|&q| {
                let qv = q.var();
                qv == v
                    || self.level[qv as usize] == 0
                    || self.seen[qv as usize]
                    || self.lit_redundant(q, cache, touched, depth + 1)
            }),
        };
        cache[v as usize] = if result { 1 } else { 2 };
        touched.push(v);
        result
    }

    /// Bump the reason side of a learned clause (Kissat's `bumpreasons`; Liang, Ganesh, Poupart & Czarnecki,
    /// *Learning Rate Based Branching Heuristic for SAT Solvers*, SAT 2016): the literals of its literals' reasons,
    /// above level 0 and not bumped already — unless they number more than `REASON_SIDE_LIMIT` a learned literal.
    fn bump_reason_side(&mut self, learned: &[Lit]) {
        let limit = REASON_SIDE_LIMIT * learned.len();
        let mut side: Vec<Var> = Vec::new();
        for &l in learned {
            self.side_mark[l.var() as usize] = true;
        }
        'gather: for &l in learned {
            let reason = self.reason[l.var() as usize];
            if let Reason::Theory(k) = reason {
                if !self.explanations.ready(k) {
                    continue;
                }
            }
            for i in 0..self.reason_lits(reason).len() {
                let q = self.reason_lits(reason)[i].var();
                if !self.side_mark[q as usize] && self.level[q as usize] > 0 {
                    self.side_mark[q as usize] = true;
                    side.push(q);
                    if side.len() > limit {
                        break 'gather;
                    }
                }
            }
        }
        for &l in learned {
            self.side_mark[l.var() as usize] = false;
        }
        for &q in &side {
            self.side_mark[q as usize] = false;
        }
        if side.len() <= limit {
            self.reason_side_bumps += side.len() as u64;
            for q in side {
                self.bump(q);
            }
        }
    }

    fn bump(&mut self, v: Var) {
        if self.vmtf_enabled {
            self.vmtf_bumped.push(v);
        }
        self.activity[v as usize] += self.var_inc;
        if self.activity[v as usize] > 1e100 {
            // Rescale all activities; relative order (hence the heap) is preserved.
            for a in self.activity.iter_mut() {
                *a *= 1e-100;
            }
            self.var_inc *= 1e-100;
        }
        self.heap_increase(v);
    }

    fn decay(&mut self) {
        self.var_inc /= 0.95;
    }

    /// Undo assignments down to (but not including) `level`.
    fn backtrack_to(&mut self, level: u32) {
        if self.trail_lim.len() as u32 <= level {
            return;
        }
        let target = self.trail_lim[level as usize];
        let mut kept: Vec<Lit> = Vec::new();
        while self.trail.len() > target {
            let l = self.trail.pop().unwrap();
            let v = l.var() as usize;
            // A literal of a level at or below the target stays assigned (an out-of-order assignment under
            // chronological backtracking): it goes back on the trail, in order, and is propagated again.
            if self.level[v] <= level {
                debug_assert!(!matches!(self.reason[v], Reason::Theory(_)), "a theory's literal is never kept out of order");
                kept.push(l);
                continue;
            }
            // Remember the polarity this variable held, so the next decision on it reuses it.
            self.saved_phase[v] = l.is_positive();
            self.vals[l.index()] = Val::Unset;
            self.vals[l.negated().index()] = Val::Unset;
            self.reason[v] = Reason::Decision;
            // Return the now-unassigned variable to the order heap (if it's a decision candidate).
            if self.heap_pos[v] < 0 && self.decision_mask.as_ref().is_none_or(|m| m[v]) {
                self.heap_insert(v as Var);
            }
            if self.vmtf_search == NONE || self.vmtf_stamp[v] > self.vmtf_stamp[self.vmtf_search as usize] {
                self.vmtf_search = v as u32;
            }
        }
        for &l in kept.iter().rev() {
            self.trail_pos[l.var() as usize] = self.trail.len() as u32;
            self.trail.push(l);
        }
        self.qhead = target;
        self.justified = self.rests.iter().skip(level as usize + 1).fold(self.justified.min(target), |j, &r| j.min(r));
        self.sided = 0;
        self.rests.truncate(level as usize + 1);
        self.trail_lim.truncate(level as usize);
        for kept in &mut self.theory_kept {
            *kept = (*kept).min(target);
        }
        self.explanations.truncate(target);
    }

    /// Reduce the learned clauses at index `from` and after (the solve's own for [`Solver::solve`], every learned
    /// one for an incremental solve, whose added clauses lie between them), by the policy in force.
    ///
    /// Soundness/verdict-invariance: only learned clauses are ever removed, never a reason of a trail literal nor
    /// a binary clause. A learned clause is implied by the formula, so dropping it cannot change satisfiability.
    /// Surviving clauses keep their two watched literals, so the two-watched-literal invariant is preserved — we
    /// only renumber and rebuild the watch lists. The proof trace (`learned_log`) is untouched, so a downstream
    /// RUP/PR check still replays every learned clause.
    fn reduce(&mut self, from: usize) {
        match self.reduce_policy {
            ReducePolicy::Halving => self.reduce_halving(from),
            ReducePolicy::Tiered => self.reduce_tiered(from),
        }
    }

    /// Whether a reduction is due now.
    fn reduce_due(&self) -> bool {
        self.reduce_enabled
            && match self.reduce_policy {
                ReducePolicy::Halving => self.learned_live >= self.reduce_limit,
                ReducePolicy::Tiered => self.conflicts >= self.reduce_next,
            }
    }

    /// [`ReducePolicy::Halving`]: delete the worse half by LBD of the unlocked learned clauses at index `from` and
    /// after with glue above 2 and more than two literals, then raise the limit.
    fn reduce_halving(&mut self, from: usize) {
        self.reductions += 1;
        self.reduce_limit += HALVING_GROWTH;
        let n = self.arena.count();
        let mut locked = vec![false; n];
        for &l in &self.trail {
            if let Reason::Clause(ci) = self.reason[l.var() as usize] {
                locked[ci] = true;
            }
        }
        let mut candidates: Vec<usize> = (from..n)
            .filter(|&ci| self.learned_flag[ci] && !locked[ci] && self.lbd[ci] > 2 && self.arena.clause(ci).len() > 2)
            .collect();
        if candidates.is_empty() {
            return;
        }
        candidates.sort_by(|&a, &b| self.lbd[b].cmp(&self.lbd[a]));
        let drop = candidates.len() / 2;
        let delete: std::collections::HashSet<usize> = candidates.into_iter().take(drop).collect();
        self.compact(&delete);
    }

    /// [`ReducePolicy::Tiered`]: the policy at `REDUCE_INT`.
    fn reduce_tiered(&mut self, from: usize) {
        self.reductions += 1;
        self.retier();
        let mode = usize::from(self.stable());
        let (tier1, tier2) = (self.tier1[mode], self.tier2[mode].max(self.tier1[mode]));
        let n = self.arena.count();
        let mut locked = vec![false; n];
        for &l in &self.trail {
            if let Reason::Clause(ci) = self.reason[l.var() as usize] {
                locked[ci] = true;
            }
        }
        let mut candidates: Vec<usize> = Vec::new();
        for ci in from..n {
            if !self.learned_flag[ci] || self.arena.clause(ci).len() <= 2 {
                continue;
            }
            let used = self.used[ci];
            self.used[ci] = used.saturating_sub(1);
            if locked[ci] {
                continue;
            }
            let glue = self.lbd[ci];
            if (glue <= tier1 && used > 0) || (glue <= tier2 && used >= MAX_USED - 1) {
                continue;
            }
            candidates.push(ci);
        }
        candidates.sort_unstable_by_key(|&ci| (std::cmp::Reverse(self.lbd[ci]), std::cmp::Reverse(self.arena.clause(ci).len()), ci));
        let percent = REDUCE_HIGH - (REDUCE_HIGH - REDUCE_LOW) / ((self.reductions + 9) as f64).log10();
        let drop = (candidates.len() as f64 * percent / 100.0) as usize;
        let delete: std::collections::HashSet<usize> = candidates.into_iter().take(drop).collect();
        self.compact(&delete);
        self.reduce_next = self.conflicts + (self.reduce_interval as f64 * (self.reductions as f64).sqrt()) as u64;
    }

    /// Recompute the current mode's tier limits from how analysis has used learned clauses of each glue.
    fn retier(&mut self) {
        let mode = usize::from(self.stable());
        let used = &self.glue_used[mode];
        let total: u64 = used.iter().sum();
        let (mut tier1, mut tier2) = (None, None);
        if total > 0 {
            let limit1 = (total as f64 * TIER1_RELATIVE) as u64;
            let limit2 = (total as f64 * TIER2_RELATIVE) as u64;
            let mut accumulated = 0u64;
            let mut glue = 0;
            while glue <= MAX_GLUE_USED {
                accumulated += used[glue];
                if accumulated >= limit1 {
                    tier1 = Some(glue as u32);
                    break;
                }
                glue += 1;
            }
            if accumulated < limit2 {
                for g in glue + 1..=MAX_GLUE_USED {
                    accumulated += used[g];
                    if accumulated >= limit2 {
                        tier2 = Some(g as u32);
                        break;
                    }
                }
            }
        }
        let (t1, t2) = match (tier1, tier2) {
            (None, _) => (TIER1_DEFAULT, TIER2_DEFAULT.max(TIER1_DEFAULT)),
            (Some(t1), None) => (t1, t1),
            (Some(t1), Some(t2)) => (t1, t2),
        };
        self.tier1[mode] = t1;
        self.tier2[mode] = t2;
    }

    /// Conflict analysis resolved on learned clause `ci`: it is used now, its glue is recomputed from the current
    /// levels and kept if lower (the clause is promoted; the count stops once it reaches the old glue), and the use
    /// is counted at its glue for the tier limits.
    fn mark_used(&mut self, ci: usize) {
        self.used[ci] = MAX_USED;
        self.lbd_gen = self.lbd_gen.wrapping_add(1);
        if self.lbd_gen == 0 {
            self.lbd_stamp.iter_mut().for_each(|s| *s = 0);
            self.lbd_gen = 1;
        }
        let old = self.lbd[ci];
        let mut glue = 0u32;
        for &l in self.arena.clause(ci) {
            let level = self.level[l.var() as usize] as usize;
            if level > 0 && self.lbd_stamp[level] != self.lbd_gen {
                self.lbd_stamp[level] = self.lbd_gen;
                glue += 1;
                if glue >= old {
                    break;
                }
            }
        }
        if glue < old {
            self.lbd[ci] = glue;
        }
        let mode = usize::from(self.stable());
        self.glue_used[mode][(self.lbd[ci] as usize).min(MAX_GLUE_USED)] += 1;
    }

    /// Rebuild the clause database keeping every clause NOT in `delete`, renumbering reasons and
    /// rebuilding watch lists from each survivor's (unchanged) first two literals. Originals stay
    /// first — so `n_original` / `original_clauses()` remain valid — provided `delete` holds only
    /// learned indices. A clause that is the reason of a trail literal is kept whatever `delete`
    /// says: a caller's locked set can predate the propagation that made it one. Shared by LBD
    /// reduction and subsumption.
    fn compact(&mut self, delete: &std::collections::HashSet<usize>) {
        if delete.is_empty() {
            return;
        }
        let n = self.arena.count();
        let reasons: std::collections::HashSet<usize> = self
            .trail
            .iter()
            .filter_map(|l| match self.reason[l.var() as usize] {
                Reason::Clause(ci) => Some(ci),
                Reason::Decision | Reason::Theory(_) => None,
            })
            .collect();
        let survivors: Vec<usize> = (0..n).filter(|ci| !delete.contains(ci) || reasons.contains(ci)).collect();
        let mut remap = vec![usize::MAX; n];
        for (new, &old) in survivors.iter().enumerate() {
            remap[old] = new;
        }
        let new_arena = self.arena.retained(&survivors);
        let new_lbd: Vec<u32> = survivors.iter().map(|&ci| self.lbd[ci]).collect();
        let new_used: Vec<u8> = survivors.iter().map(|&ci| self.used[ci]).collect();
        let new_flag: Vec<bool> = survivors.iter().map(|&ci| self.learned_flag[ci]).collect();
        let new_ids: Vec<u64> = survivors.iter().map(|&ci| self.clause_ids[ci]).collect();
        for &l in &self.trail {
            let v = l.var() as usize;
            if let Reason::Clause(ci) = self.reason[v] {
                debug_assert_ne!(remap[ci], usize::MAX, "a deleted clause was the reason of a trail literal");
                self.reason[v] = Reason::Clause(remap[ci]);
            }
        }
        self.arena = new_arena;
        self.lbd = new_lbd;
        self.used = new_used;
        self.learned_live = new_flag.iter().filter(|&&f| f).count();
        self.learned_flag = new_flag;
        self.clause_ids = new_ids;
        for w in self.watches.iter_mut() {
            w.clear();
        }
        for ci in 0..self.arena.count() {
            if self.arena.clause(ci).len() >= 2 {
                self.watch(ci);
            }
        }    }

    /// Lift the two watchers of clause `ci` from the watch lists (its watched literals are, by
    /// invariant, `clauses[ci][0..2]`). Used to EXCLUDE a clause from propagation while it is being
    /// vivified, so the strengthened clause is implied by `F \ {C}` — the soundness key.
    fn unwatch(&mut self, ci: usize) {
        let c = self.arena.clause(ci);
        if c.len() < 2 {
            return;
        }
        let (a, b) = (c[0].index(), c[1].index());
        let word = if c.len() == 2 { Watcher::binary(ci, c[1]).word } else { Watcher::long(self.arena.offset(ci), c[1]).word };
        if let Some(p) = self.watches[a].iter().position(|w| w.word == word) {
            self.watches[a].swap_remove(p);
        }
        if let Some(p) = self.watches[b].iter().position(|w| w.word == word) {
            self.watches[b].swap_remove(p);
        }
    }

    /// Re-establish watches on `clauses[ci]`'s current first two literals (the inverse of
    /// [`Self::unwatch`], after a possible literal rewrite).
    fn rewatch(&mut self, ci: usize) {
        if self.arena.clause(ci).len() >= 2 {
            self.watch(ci);
        }
    }

    /// Vivify the clause at `ci` (asymmetric branching; Piette, Hamadi & Saïs 2008). Must be called
    /// at decision level 0 with the top-level fixpoint reached and `ci` already un-watched. Pushes
    /// trial decisions `¬lᵢ` for the clause's literals in order, propagating against `F \ {C}`:
    ///
    /// - `lᵢ` already FALSE ⇒ `F\{C} ⊨ ¬lᵢ` under the assumed prefix ⇒ `lᵢ` is redundant — drop it.
    /// - `lᵢ` already TRUE  ⇒ `F\{C} ⊨ (kept ∨ lᵢ)` ⇒ that prefix subsumes C — strengthen to it.
    /// - propagating `¬lᵢ` CONFLICTS ⇒ `F\{C} ⊨ kept` ⇒ strengthen to the prefix `kept`.
    ///
    /// Returns the strengthened literal set if C can be shortened (a strict, still-implied subset),
    /// else `None`. Leaves the trail back at level 0; the clause is left un-watched for the caller to
    /// rewatch or replace. Because the result is implied by `F \ {C}` and is `⊆ C`, replacing C with
    /// it preserves every model and the new clause is a valid RUP/DRAT addition.
    fn vivify_clause(&mut self, ci: usize) -> Option<Vec<Lit>> {
        let c = self.arena.clause(ci).to_vec();
        let mut kept: Vec<Lit> = Vec::with_capacity(c.len());
        let mut shortened = false;
        for &l in &c {
            match self.val_of(l) {
                Val::False => shortened = true, // redundant literal — drop it
                Val::True => {
                    kept.push(l);
                    shortened = kept.len() < c.len();
                    break;
                }
                Val::Unset => {
                    kept.push(l);
                    self.trail_lim.push(self.trail.len());
                    self.enqueue(l.negated(), Reason::Decision);
                    if self.propagate().is_some() {
                        shortened = kept.len() < c.len();
                        break;
                    }
                }
            }
        }
        self.backtrack_to(0);
        if shortened && !kept.is_empty() {
            Some(kept)
        } else {
            None
        }
    }

    /// One vivification round over the learned clauses (Phase 2 inprocessing). Strengthens each
    /// shortenable learned clause in place, appending the result to the proof log as a valid RUP
    /// addition. Must be invoked at decision level 0. Returns `false` if a clause vivifies down to a
    /// unit that conflicts (or to empty) — i.e. the formula is proven UNSAT. Verdict-invariant: only
    /// replaces a learned clause with an equally- or more-constraining clause implied by the rest of
    /// the formula. A `budget` of 0 means "every learned clause".
    fn vivify_round(&mut self, budget: usize) -> bool {
        self.backtrack_to(0);
        if self.propagate().is_some() {
            return false; // level-0 conflict ⇒ UNSAT
        }
        let n = self.arena.count();
        let cap = if budget == 0 { usize::MAX } else { budget };
        let mut done = 0usize;
        for ci in self.n_original..n {
            if done >= cap {
                break;
            }
            if self.arena.clause(ci).len() < 2 {
                continue; // units/empties: nothing to strengthen
            }
            done += 1;
            self.unwatch(ci);
            match self.vivify_clause(ci) {
                None => self.rewatch(ci),
                Some(kept) => {
                    self.vivifications += 1;
                    self.learned_log.push(LearnedClause { lits: kept.clone(), id: 0, hints: Vec::new() });
                    if kept.is_empty() {
                        self.empty_clause = true;
                        self.arena.rewrite(ci, &kept);
                        return false;
                    }
                    if kept.len() == 1 {
                        let u = kept[0];
                        self.arena.rewrite(ci, &kept);
                        match self.val_of(u) {
                            Val::False => {
                                self.empty_clause = true;
                                return false;
                            }
                            Val::Unset => self.enqueue(u, Reason::Clause(ci)),
                            Val::True => {}
                        }
                        if self.propagate().is_some() {
                            return false;
                        }
                        continue;
                    }
                    self.lbd[ci] = self.clause_lbd(&kept);
                    self.arena.rewrite(ci, &kept);
                    self.rewatch(ci);
                }
            }
        }
        true
    }

    /// One failed-literal probing round. For each free variable `v` (a decision candidate), assume
    /// each phase as a fresh decision and propagate against the whole formula: if `F ∧ probe`
    /// conflicts at level 0, then `F ⊨ ¬probe`, so `¬probe` is added as a permanent unit (a valid
    /// RUP step) and propagated. Returns `false` if a derived unit conflicts — the formula is UNSAT.
    /// Verdict-invariant. A `budget` of 0 means "every variable".
    fn probe_round(&mut self, budget: usize) -> bool {
        self.backtrack_to(0);
        if self.propagate().is_some() {
            return false; // level-0 conflict ⇒ UNSAT
        }
        let cap = if budget == 0 { usize::MAX } else { budget };
        let mut done = 0usize;
        for v in 0..self.num_vars as Var {
            if done >= cap {
                break;
            }
            if self.var_val(v as usize) != Val::Unset {
                continue; // already a level-0 fact
            }
            if self.decision_mask.as_ref().is_some_and(|m| !m[v as usize]) {
                continue; // not a decision candidate
            }
            done += 1;
            for probe in [Lit::pos(v), Lit::neg(v)] {
                if self.var_val(v as usize) != Val::Unset {
                    break; // fixed by the other phase's probe
                }
                self.trail_lim.push(self.trail.len());
                self.enqueue(probe, Reason::Decision);
                let conflict = self.propagate().is_some();
                self.backtrack_to(0);
                if conflict {
                    // F ∧ probe is UNSAT ⇒ ¬probe is forced; learn it as a unit (RUP) and apply it.
                    self.probes += 1;
                    self.add_clause_raw(vec![probe.negated()], true);
                    if self.empty_clause || self.propagate().is_some() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// One subsumption + self-subsuming-resolution round over the LEARNED clauses. A learned clause
    /// subsumed by any clause is deleted; a learned clause that self-subsumes against another
    /// (exactly one literal resolves away) is strengthened. Only learned, unlocked clauses are
    /// removed/strengthened — originals (and the RUP certificate, and `n_original`) stay valid.
    /// Verdict-invariant: a deleted clause is entailed by its subsumer (and subsumption is
    /// transitive, so soundness survives even if the subsumer is itself later removed), and a
    /// strengthened clause is a sound resolvent (logged as RUP). Returns `false` if a strengthening
    /// derives the empty clause. `budget` of 0 means "every learned clause".
    fn subsume_round(&mut self, budget: usize) -> bool {
        self.backtrack_to(0);
        if self.propagate().is_some() {
            return false;
        }
        let n = self.arena.count();
        if n <= self.n_original {
            return true;
        }
        let code = |l: Lit| l.var() * 2 + u32::from(!l.is_positive());
        let mut coded: Vec<Vec<u32>> = Vec::with_capacity(n);
        let mut sigs: Vec<u64> = Vec::with_capacity(n);
        for ci in 0..n {
            let c = self.arena.clause(ci);
            let mut v: Vec<u32> = c.iter().map(|&l| code(l)).collect();
            v.sort_unstable();
            v.dedup();
            sigs.push(c.iter().fold(0u64, |s, l| s | (1u64 << (l.var() % 64))));
            coded.push(v);
        }
        let mut occ: Vec<Vec<usize>> = vec![Vec::new(); self.num_vars];
        for (ci, cc) in coded.iter().enumerate() {
            for &x in cc {
                occ[(x / 2) as usize].push(ci);
            }
        }
        let mut locked = vec![false; n];
        for &l in &self.trail {
            if let Reason::Clause(ci) = self.reason[l.var() as usize] {
                if ci < n {
                    locked[ci] = true;
                }
            }
        }
        let mut delete: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut strengthen: Vec<(usize, u32)> = Vec::new();
        let cap = if budget == 0 { usize::MAX } else { budget };
        let mut done = 0usize;
        for di in self.n_original..n {
            if done >= cap {
                break;
            }
            if coded[di].len() < 2 || delete.contains(&di) {
                continue;
            }
            done += 1;
            let rare = *coded[di]
                .iter()
                .min_by_key(|&&x| occ[(x / 2) as usize].len())
                .unwrap();
            let mut tried = 0;
            for &ci in &occ[(rare / 2) as usize] {
                if ci == di || delete.contains(&ci) {
                    continue;
                }
                tried += 1;
                if tried > 96 {
                    break;
                }
                if coded[ci].len() > coded[di].len() || (sigs[ci] & !sigs[di]) != 0 {
                    continue;
                }
                match self_subsumes(&coded[ci], &coded[di]) {
                    Sub::Subsumes => {
                        if !locked[di] {
                            delete.insert(di);
                        }
                        break;
                    }
                    Sub::Strengthen(pivot) => {
                        strengthen.push((di, pivot));
                        break;
                    }
                    Sub::No => {}
                }
            }
        }
        for (di, drop_code) in strengthen {
            if delete.contains(&di) {
                continue;
            }
            self.unwatch(di);
            let new: Vec<Lit> = self.arena.clause(di)
                .iter()
                .copied()
                .filter(|&l| code(l) != drop_code)
                .collect();
            self.subsumptions += 1;
            self.learned_log.push(LearnedClause { lits: new.clone(), id: 0, hints: Vec::new() });
            if new.is_empty() {
                self.empty_clause = true;
                self.arena.rewrite(di, &new);
                return false;
            }
            if new.len() == 1 {
                let u = new[0];
                self.arena.rewrite(di, &new);
                match self.val_of(u) {
                    Val::False => {
                        self.empty_clause = true;
                        return false;
                    }
                    Val::Unset => self.enqueue(u, Reason::Clause(di)),
                    Val::True => {}
                }
                if self.propagate().is_some() {
                    return false;
                }
            } else {
                self.lbd[di] = self.clause_lbd(&new);
                self.arena.rewrite(di, &new);
                self.rewatch(di);
            }
        }
        self.subsumptions += delete.len() as u64;
        self.compact(&delete);
        true
    }

    /// Run one level-0 inprocessing round: failed-literal probing (only while it keeps paying off),
    /// then subsumption + self-subsuming resolution, then learned-clause vivification. Returns `false`
    /// if it proves the formula UNSAT. Must be called at decision level 0 (the scheduler calls it right
    /// after a restart). Verdict-invariant.
    fn inprocess(&mut self) -> bool {
        if self.probe_active {
            let before = self.probes;
            if !self.probe_round(PROBE_BUDGET) {
                return false;
            }
            // Most instances yield no failed literals; once a round finds none, stop probing this
            // solve so it is not pure overhead.
            if self.probes == before {
                self.probe_active = false;
            }
        }
        if !self.subsume_round(SUBSUME_BUDGET) {
            return false;
        }
        self.vivify_round(VIVIFY_BUDGET)
    }

    /// The decision literal for `v`: in stable mode its target phase when it has one, else its saved phase
    /// (false-first on the first ever decision, unless seeded).
    fn decision_lit(&self, v: Var) -> Lit {
        let target = self.target_phase[v as usize];
        if self.target_enabled && self.stable() && target != 0 {
            return Lit::new(v, target > 0);
        }
        Lit::new(v, self.saved_phase[v as usize])
    }

    /// The next decision: by justification when the solver justifies, and otherwise the variable
    /// [`Solver::pick_branch`] picks, in its saved (or target) phase. `None` when nothing is left to decide.
    fn decide(&mut self) -> Option<Lit> {
        if self.justifying {
            return self.justify().or_else(|| self.side());
        }
        self.pick_branch().map(|v| self.decision_lit(v))
    }

    /// The decision a justifying search takes once no gate is left to justify: of the first side clause with no literal
    /// true, its open literal of highest activity, set true. A side clause with no literal true holds an open one, or
    /// the propagation before this would have met a conflict.
    fn side(&mut self) -> Option<Lit> {
        while let Some(c) = self.sides.get(self.sided) {
            if c.iter().any(|&l| self.lit_true(l)) {
                self.sided += 1;
                continue;
            }
            let activity = |l: &Lit| self.activity[l.var() as usize];
            let open = c.iter().copied().filter(|&l| self.val_of(l) == Val::Unset);
            let pick = open.max_by(|a, b| activity(a).total_cmp(&activity(b)));
            debug_assert!(pick.is_some(), "a side clause with no literal true holds an open one at a fixpoint");
            return pick;
        }
        None
    }

    /// The decision justification takes next: a fan-in, the one of higher activity, set false, of the first gate on the
    /// trail assigned false with both fan-ins open. At a propagation fixpoint a gate assigned true has its fan-ins
    /// true, and one assigned false with a fan-in true has the other false, so `None`, no such gate left, is every
    /// gate's value given by its fan-ins'.
    fn justify(&mut self) -> Option<Lit> {
        while self.justified < self.trail.len() {
            let l = self.trail[self.justified];
            if let Some([a, b]) = self.gates.get(l.var() as usize).copied().flatten().filter(|_| !l.is_positive()) {
                if self.val_of(a) == Val::Unset && self.val_of(b) == Val::Unset {
                    let pick = if self.activity[b.var() as usize] > self.activity[a.var() as usize] { b } else { a };
                    return Some(pick.negated());
                }
                let level = |x: Lit| if self.lit_false(x) { self.level[x.var() as usize] } else { u32::MAX };
                let rest = level(a).min(level(b)) as usize;
                self.rest_at(rest);
            } else if let Some(&k) = self.covers.get(l.var() as usize).filter(|&&k| k != 0) {
                let (first, on, off) = self.cover_gates[k as usize - 1];
                let (from, count) = if l.is_positive() { (first, on) } else { (first + on as u32, off) };
                let mut rest = u32::MAX;
                let mut pick: Option<Lit> = None;
                for c in from..from + count as u32 {
                    let cube = &self.cube_lits[self.cube_starts[c as usize] as usize..self.cube_starts[c as usize + 1] as usize];
                    let (mut held, mut open, mut top) = (true, None::<Lit>, 0u32);
                    for &x in cube {
                        match self.val_of(x) {
                            Val::False => {
                                held = false;
                                open = None;
                                break;
                            }
                            Val::True => top = top.max(self.level[x.var() as usize]),
                            Val::Unset => {
                                held = false;
                                let act = |y: Lit| self.activity[y.var() as usize];
                                if open.is_none_or(|o| act(x) > act(o)) {
                                    open = Some(x);
                                }
                            }
                        }
                    }
                    if held {
                        rest = rest.min(top);
                    } else if let Some(x) = open {
                        let act = |y: Lit| self.activity[y.var() as usize];
                        if pick.is_none_or(|p| act(x) > act(p)) {
                            pick = Some(x);
                        }
                    }
                }
                if rest == u32::MAX {
                    if pick.is_some() {
                        return pick;
                    }
                    let all = &self.cube_lits[self.cube_starts[first as usize] as usize
                        ..self.cube_starts[(first + on as u32 + off as u32) as usize] as usize];
                    let leaf = all.iter().copied().find(|&x| self.val_of(x) == Val::Unset);
                    debug_assert!(leaf.is_some(), "a gate whose leaves are all set holds a product of its value");
                    if let Some(x) = leaf {
                        return Some(self.decision_lit(x.var()));
                    }
                }
                self.rest_at(rest as usize);
            }
            self.justified += 1;
        }
        None
    }

    /// The gate at the justification cursor rests on literals of level `rest` and below: a backtrack below `rest`
    /// takes the cursor back to it.
    fn rest_at(&mut self, rest: usize) {
        if self.rests.len() <= rest {
            self.rests.resize(rest + 1, usize::MAX);
        }
        self.rests[rest] = self.rests[rest].min(self.justified);
    }

    /// Pick the highest-activity unassigned decision variable in O(log n) via the order heap,
    /// discarding popped variables that are already assigned (lazy deletion). `None` once every
    /// decision candidate is assigned. Backtracking re-inserts unassigned candidates, so the heap is
    /// never missing a candidate when a decision is actually needed.
    fn pick_branch(&mut self) -> Option<Var> {
        if self.vmtf_enabled && !self.stable() {
            return self.vmtf_pick().or_else(|| self.open_outside_decisions());
        }
        loop {
            let Some(v) = self.heap_pop() else { return self.open_outside_decisions() };
            if self.var_val(v as usize) == Val::Unset {
                return Some(v);
            }
        }
    }

    /// With the decisions restricted and every one of them made, an unassigned variable of a clause still without a
    /// true literal. The restriction leaves such a variable to propagation; one that propagation left open is decided,
    /// so a model never answers a clause with a default value. `None` when every clause has a true literal.
    fn open_outside_decisions(&self) -> Option<Var> {
        if self.decision_mask.is_none() || self.trail.len() == self.num_vars {
            return None;
        }
        (0..self.arena.count()).find_map(|ci| {
            let clause = self.arena.clause(ci);
            if clause.iter().any(|&l| self.lit_true(l)) {
                return None;
            }
            clause.iter().find(|&&l| self.val_of(l) == Val::Unset).map(|l| l.var())
        })
    }

    /// The Literal-Block-Distance of a clause: the number of distinct decision levels among its
    /// literals (Audemard & Simon, 2009). Low LBD ⇒ "glue" ⇒ kept across reductions. For a clause
    /// strengthened at level 0 its unassigned literals count at the levels of their last assignment:
    /// the clause's LBD as the search before the restart left it.
    fn clause_lbd(&self, lits: &[Lit]) -> u32 {
        let mut levels: Vec<u32> = lits.iter().map(|l| self.level[l.var() as usize]).collect();
        levels.sort_unstable();
        levels.dedup();
        levels.len() as u32
    }

    /// Under chronological backtracking a conflict clause's literals may all lie below the current level. Its real
    /// level is the highest among them: at level 0 the formula is refuted; with one literal there the clause is a
    /// missed implication — backtrack below that level and assign the literal true, at the highest of the others'
    /// levels; otherwise backtrack to that level and analyze as usual. Either way the clause is rewatched on its two
    /// highest-level literals, so no later backtrack leaves it unit or false behind two false watches.
    fn chrono_conflict(&mut self, ci: usize) -> Chrono {
        let lits: Vec<Lit> = self.arena.clause(ci).to_vec();
        let level_of = |s: &Solver, l: Lit| s.level[l.var() as usize];
        let top = lits.iter().map(|&l| level_of(self, l)).max().unwrap_or(0);
        if top == 0 {
            return Chrono::Refuted;
        }
        let at_top = lits.iter().filter(|&&l| level_of(self, l) == top).count();
        if lits.len() >= 2 {
            self.unwatch(ci);
            let mut order: Vec<Lit> = lits.clone();
            order.sort_by_key(|&l| std::cmp::Reverse(level_of(self, l)));
            self.arena.rewrite(ci, &order);
            self.rewatch(ci);
        }
        if at_top == 1 {
            let forced = self.arena.clause(ci)[0];
            let below = self.arena.clause(ci).get(1).map_or(0, |&l| level_of(self, l));
            self.backtrack_to(top - 1);
            self.enqueue_at(forced, Reason::Clause(ci), below);
            return Chrono::Implied;
        }
        if top < self.trail_lim.len() as u32 {
            self.backtrack_to(top);
        }
        Chrono::Analyze
    }

    /// Turn chronological backtracking on with a jump limit (`Some(levels)`; Kissat's default is 100), or off
    /// (`None`, the default). Verdict-invariant either way.
    pub fn set_chrono(&mut self, levels: Option<u32>) {
        self.chrono_levels = levels;
    }

    /// Backjumps taken chronologically during this solver's life.
    pub fn chrono_backtracks(&self) -> u64 {
        self.chrono_backtracks
    }

    /// The conflict aftermath shared by Boolean and theory conflicts: derive the 1-UIP asserting
    /// clause from conflicting clause `ci`, backjump, learn + enqueue it, then run the decay /
    /// reduce-DB / restart bookkeeping. Returns `true` when the conflict is at decision level 0 — the
    /// formula is UNSAT.
    fn after_conflict(&mut self, ci: usize, theories: &mut [Box<dyn Theory>]) -> bool {
        if self.trail_lim.is_empty() {
            return true; // conflict at level 0
        }
        if self.chrono_now {
            match self.chrono_conflict(ci) {
                Chrono::Refuted => return true,
                Chrono::Implied => return false,
                Chrono::Analyze => {}
            }
        }
        let conflict_level = self.trail_lim.len() as u32;
        let trail_at_conflict = self.trail.len();
        let (learned, backjump, lbd) = self.analyze(ci, theories);
        self.note_conflict(lbd, trail_at_conflict);
        if self.stable() {
            self.backtrack_to(self.trail_lim.len() as u32 - 1);
            self.update_target_and_best();
        }
        let target = match self.chrono_levels {
            Some(limit) if self.chrono_now && backjump > 0 && conflict_level - backjump > limit => {
                self.chrono_backtracks += 1;
                conflict_level - 1
            }
            _ => backjump,
        };
        self.backtrack_to(target);
        let asserting = learned[0];
        let unit = learned.len() == 1;
        let new_ci = self.add_clause_raw(learned, true);
        self.lbd[new_ci] = lbd;
        if !unit {
            self.enqueue_at(asserting, Reason::Clause(new_ci), backjump);
        }
        self.decay();
        self.conflicts += 1;
        self.csr += 1;
        if self.reduce_due() {
            self.reduce(self.n_original);
        }
        self.advance_restart_phase();
        if self.want_restart() {
            self.do_restart();
        }
        false
    }

    /// Solve, optionally under a list of theory propagators (DPLL(T)). Returns a model or
    /// `Unsat`. The learned-clause log is available afterwards via [`Solver::learned`].
    pub fn solve(&mut self) -> SolveResult {
        self.solve_with(&mut [])
    }

    /// Solve but give up after `max_conflicts` conflicts, returning [`BudgetedResult::Budget`] with
    /// the learned clauses ([`Self::learned`]) intact — the hook dynamic symmetry breaking needs to
    /// interleave bounded search with symmetric clause amplification. A `max_conflicts` of 0 means
    /// unlimited (equivalent to [`Self::solve`]).
    pub fn solve_budgeted(&mut self, max_conflicts: u64) -> BudgetedResult {
        self.chrono_now = self.chrono_levels.is_some();
        // Mark which clauses are original so DB reduction never deletes them (the omission that
        // would otherwise let reduction drop the formula itself and report a bogus SAT).
        self.n_original = self.arena.count();
        if self.empty_clause {
            return BudgetedResult::Unsat;
        }
        if self.propagate().is_some() {
            return BudgetedResult::Unsat;
        }
        self.reset_restart_state();
        let start = self.conflicts;
        loop {
            if let Some(ci) = self.propagate() {
                if self.after_conflict(ci, &mut []) {
                    return BudgetedResult::Unsat;
                }
                if max_conflicts != 0 && self.conflicts - start >= max_conflicts {
                    self.backtrack_to(0);
                    return BudgetedResult::Budget;
                }
                continue;
            }
            if !self.maybe_rephase() {
                return BudgetedResult::Unsat;
            }
            if self.trail_lim.is_empty() && self.qhead < self.trail.len() {
                continue;
            }
            match self.decide() {
                None => {
                    let model = (0..self.num_vars)
                        .map(|v| self.var_val(v) == Val::True)
                        .collect();
                    return BudgetedResult::Sat(model);
                }
                Some(l) => {
                    self.trail_lim.push(self.trail.len());
                    self.decisions += 1;
                    self.enqueue(l, Reason::Decision);
                }
            }
        }
    }

    /// Solve with theory propagators. Each is consulted at every Boolean fixpoint; a
    /// returned clause is added to the formula (and may immediately propagate or conflict).
    pub fn solve_with(&mut self, theories: &mut [Box<dyn Theory>]) -> SolveResult {
        match self.begin(theories.len()) {
            Some(verdict) => verdict,
            None => self.resume(theories, 0).expect("a search without a budget runs to its verdict"),
        }
    }

    /// Begin a search under `theories` theory propagators: the clauses added so far become the formula, level 0
    /// propagates, lucky phases try their assignments (with no theory), and the restart, rephase and inprocessing
    /// schedules start afresh. `Some` when that alone decides the formula; otherwise [`Solver::resume`] searches.
    pub fn begin(&mut self, theories: usize) -> Option<SolveResult> {
        self.n_original = self.arena.count();
        if self.empty_clause {
            return Some(SolveResult::Unsat);
        }
        // Top-level propagation of any unit clauses already enqueued.
        if self.propagate().is_some() {
            return Some(SolveResult::Unsat);
        }
        self.chrono_now = self.chrono_levels.is_some() && theories == 0;
        self.theory_kept = vec![0; theories];
        if theories == 0 {
            match self.lucky() {
                Lucky::Sat(model) => return Some(SolveResult::Sat(model)),
                Lucky::Unsat => return Some(SolveResult::Unsat),
                Lucky::Unknown => {}
            }
        }
        self.reset_restart_state();
        self.last_inprocess = 0;
        self.inprocess_gap = self.inprocess_interval;
        self.probe_active = true; // re-arm probing for this solve
        None

    }

    /// Continue the search [`Solver::begin`] began, under the same theories, for up to `max_conflicts` more conflicts
    /// (0: no limit): its verdict, or `None` once the budget is spent or the stop flag ([`Solver::set_stop`]) is set,
    /// the search left where it stood for the next call — every schedule, phase and learned clause in place, as if it
    /// had never stopped.
    pub fn resume(&mut self, theories: &mut [Box<dyn Theory>], max_conflicts: u64) -> Option<SolveResult> {
        let start = self.conflicts;
        loop {
            let conflict = self.propagate();
            if let Some(ci) = conflict {
                let restarts_before = self.restarts;
                if self.after_conflict(ci, theories) {
                    return Some(SolveResult::Unsat);
                }
                // A restart just returned us to level 0 — the safe point to inprocess. Gated by a
                // GROWING gap so only long searches pay, and less and less as the search runs on
                // (bounding churn). probe + subsume + vivify, all verdict-invariant; a `false` return
                // means inprocessing proved UNSAT.
                if self.inprocess_enabled
                    && theories.is_empty()
                    && self.restarts > restarts_before
                    && self.conflicts - self.last_inprocess >= self.inprocess_gap
                {
                    self.last_inprocess = self.conflicts;
                    if !self.inprocess() {
                        return Some(SolveResult::Unsat);
                    }
                    self.inprocess_gap = ((self.inprocess_gap as f64) * INPROCESS_GROWTH) as u64;
                }
                if max_conflicts != 0 && self.conflicts - start >= max_conflicts {
                    return None;
                }
                if self.stop.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed)) {
                    return None;
                }
                continue;
            }
            // Boolean fixpoint — consult theories before branching. Every clause a theory hands back
            // is a globally-valid no-good (a logical consequence of the formula). A theory conflict (all
            // literals false) is carried into the learned database and drives a 1-UIP backjump exactly
            // like a Boolean conflict — sound because we consult at EVERY fixpoint, so the inconsistency
            // is caught at the level of its last contributing assignment and the clause has a
            // current-level literal. A theory propagation (one literal still free) is enqueued with its
            // clause or token as its explanation, so later conflict analysis resolves through the
            // theory's reasoning; a proof-logging solve carries the clause into the database instead.
            let mut theory_acted = false;
            for ti in 0..theories.len() {
                let mut forced = std::mem::take(&mut self.forced);
                forced.clear();
                let implied = if self.lrat {
                    theories[ti].propagate_kept(&self.trail, self.theory_kept[ti])
                } else {
                    theories[ti].propagate_lazily(&self.trail, self.theory_kept[ti], &mut forced)
                };
                self.theory_kept[ti] = self.trail.len();
                if implied.is_empty() && forced.is_empty() {
                    self.forced = forced;
                    continue;
                }
                // A theory conflict takes priority: carry it and run conflict analysis. A literal forced against its
                // value is one too, its clause all false.
                let mut conflict = implied.iter().find(|c| c.iter().all(|&l| self.lit_false(l))).cloned();
                if conflict.is_none() {
                    if let Some(&(_, token)) = forced.iter().find(|(l, _)| self.lit_false(*l)) {
                        let mut clause = Vec::new();
                        theories[ti].explain(token, &mut clause);
                        conflict = Some(clause);
                    }
                }
                if let Some(conf) = conflict {
                    self.forced = forced;
                    if conf.is_empty() {
                        return Some(SolveResult::Unsat); // an unconditional contradiction (0 = 1)
                    }
                    let lbd = self.clause_lbd(&conf);
                    let ci = self.add_clause_raw(conf, true);
                    self.lbd[ci] = lbd;
                    if self.after_conflict(ci, theories) {
                        return Some(SolveResult::Unsat);
                    }
                    theory_acted = true;
                    break;
                }
                // Each literal forced by a token is explained by it, asked for only if analysis resolves on it.
                for &(lit, token) in &forced {
                    if self.val_of(lit) == Val::Unset {
                        let k = self.explanations.push_token(self.trail.len(), ti, token);
                        self.enqueue(lit, Reason::Theory(k));
                        theory_acted = true;
                    }
                }
                self.forced = forced;
                // Otherwise enqueue every implied unit's one free literal: explained by the clause, kept only while
                // the literal is assigned — or, logging a proof, carried into the database.
                for mut lits in implied {
                    if lits.iter().any(|&l| self.lit_true(l)) {
                        continue; // already satisfied — stale
                    }
                    let free: Vec<usize> = (0..lits.len())
                        .filter(|&i| self.val_of(lits[i]) == Val::Unset)
                        .collect();
                    if free.len() != 1 {
                        continue; // not a clean unit propagation — skip defensively
                    }
                    lits.swap(0, free[0]);
                    if !self.lrat {
                        let k = self.explanations.push(self.trail.len(), &lits);
                        self.enqueue(lits[0], Reason::Theory(k));
                        theory_acted = true;
                        continue;
                    }
                    if lits.len() > 1 {
                        // Watch the highest-level false literal alongside the implied one.
                        let mut best = 1;
                        let mut best_lv = 0u32;
                        for i in 1..lits.len() {
                            let lv = self.level[lits[i].var() as usize];
                            if lv >= best_lv {
                                best_lv = lv;
                                best = i;
                            }
                        }
                        lits.swap(1, best);
                    }
                    let implied_lit = lits[0];
                    let multi = lits.len() > 1;
                    let lbd = self.clause_lbd(&lits);
                    let ci = self.add_clause_raw(lits, true);
                    self.lbd[ci] = lbd;
                    if multi {
                        // A unit clause was already enqueued by add_clause_raw's unit path.
                        self.enqueue(implied_lit, Reason::Clause(ci));
                    }
                    theory_acted = true;
                }
                if theory_acted {
                    break;
                }
            }
            if theory_acted {
                continue;
            }
            if !self.maybe_rephase() {
                return Some(SolveResult::Unsat);
            }
            if self.trail_lim.is_empty() && self.qhead < self.trail.len() {
                continue;
            }
            // Decide.
            match self.decide() {
                None => {
                    // Full assignment → SAT.
                    let model = (0..self.num_vars)
                        .map(|v| self.var_val(v) == Val::True)
                        .collect();
                    return Some(SolveResult::Sat(model));
                }
                Some(l) => {
                    self.trail_lim.push(self.trail.len());
                    self.decisions += 1;
                    self.enqueue(l, Reason::Decision);
                }
            }
        }
    }

    /// Solve under temporary `assumptions` (literals forced true for THIS query only),
    /// reusing every clause learned so far. The permanent clause database is untouched, so a
    /// later call with different assumptions may well be satisfiable — successive queries on
    /// the same solver (e.g. BMC at increasing depths) amortise learning. This is the
    /// incremental-SAT (IPASIR) pattern. `Unsat` here means "unsatisfiable UNDER these
    /// assumptions".
    ///
    /// Soundness of reuse: conflict analysis keeps decision-level literals (assumptions are
    /// decisions) and drops level-0 facts, so each learned clause is a consequence of the
    /// PERMANENT clauses alone — valid no matter which assumptions a future query makes.
    ///
    /// Restarts follow the Luby sequence (100 conflicts a unit) and drop the search to level 0:
    /// the next decisions place the assumptions again, so they stay the bottom levels.
    /// (Does not touch `n_original`; do not mix with [`Solver::original_clauses`]/RUP on the
    /// same solver.)
    pub fn solve_under_assumptions(&mut self, assumptions: &[Lit]) -> SolveResult {
        match self.solve_under_assumptions_budgeted(assumptions, 0) {
            BudgetedResult::Sat(model) => SolveResult::Sat(model),
            BudgetedResult::Unsat => SolveResult::Unsat,
            BudgetedResult::Budget => unreachable!("a solve without a budget runs to a verdict"),
        }
    }

    /// [`Solver::solve_under_assumptions`] within `max_conflicts` conflicts (0: no limit), `Budget` when they run out
    /// with the search dropped and every learned clause kept. On `Unsat`, the clause of the assumptions' negations is
    /// RUP from the clauses added and [`Solver::learned`] so far. Learned clauses are deleted, the added ones never,
    /// whatever their order: a long run of queries keeps its database in bounds.
    pub fn solve_under_assumptions_budgeted(&mut self, assumptions: &[Lit], max_conflicts: u64) -> BudgetedResult {
        self.core.clear();
        // Drop any prior search state, keeping level-0 facts and all learned clauses.
        self.backtrack_to(0);
        self.chrono_now = false;
        if self.empty_clause {
            self.unsat_hints = self.empty_hints.clone();
            return BudgetedResult::Unsat;
        }
        // Level-0 propagation: if the permanent formula is already unsatisfiable, no
        // assumption can rescue it — and it stays unsat for every future query, so latch the
        // permanent-unsat flag (this also guarantees a clean state on the next call, which a
        // no-op `backtrack_to(0)` over an empty `trail_lim` would otherwise inherit dirty).
        if let Some(ci) = self.propagate() {
            self.found_empty(ci);
            self.unsat_hints = self.empty_hints.clone();
            return BudgetedResult::Unsat;
        }
        let mut spent = 0u64;
        let (mut since_restart, mut restart_no) = (0u64, 1u64);
        loop {
            if let Some(ci) = self.propagate() {
                if self.trail_lim.is_empty() {
                    // A conflict with no decisions: the formula is unsatisfiable.
                    self.found_empty(ci);
                    self.unsat_hints = self.empty_hints.clone();
                    return BudgetedResult::Unsat;
                }
                let (learned, backjump, lbd) = self.analyze(ci, &mut []);
                self.backtrack_to(backjump);
                let asserting = learned[0];
                let unit = learned.len() == 1;
                let new_ci = self.add_clause_raw(learned, true);
                self.lbd[new_ci] = lbd;
                if !unit {
                    self.enqueue(asserting, Reason::Clause(new_ci));
                }
                self.decay();
                self.conflicts += 1;
                spent += 1;
                if self.reduce_due() {
                    self.reduce(0);
                }
                if max_conflicts > 0 && spent >= max_conflicts {
                    self.backtrack_to(0);
                    return BudgetedResult::Budget;
                }
                since_restart += 1;
                if since_restart >= luby(restart_no) * LUBY_UNIT {
                    self.backtrack_to(0);
                    self.restarts += 1;
                    since_restart = 0;
                    restart_no += 1;
                }
                continue;
            }
            // Decide: place the first not-yet-satisfied assumption (so the search always
            // explores under the full assumption set, even after a backjump unset some).
            let mut decided = false;
            for &a in assumptions {
                match self.val_of(a) {
                    // The assumption is forced false ⇒ no model under the assumptions: the chain is the cone
                    // of its negation back to the assumptions before it.
                    Val::False => {
                        if self.lrat {
                            let negated: Vec<Lit> = assumptions.iter().map(|x| x.negated()).collect();
                            let reasons = self.cone([a.var()]);
                            self.unsat_hints = self.replayed(&negated, &reasons);
                        }
                        let behind = self.decisions_behind(a.var());
                        let rests = |x: &Lit| *x == a || behind.contains(&x.var());
                        self.core = assumptions.iter().copied().filter(rests).collect();
                        return BudgetedResult::Unsat;
                    }
                    Val::True => {}
                    Val::Unset => {
                        self.trail_lim.push(self.trail.len());
                        self.enqueue(a, Reason::Decision);
                        decided = true;
                        break;
                    }
                }
            }
            if decided {
                continue;
            }
            // All assumptions hold — branch on the remaining variables.
            match self.decide() {
                None => {
                    let model = (0..self.num_vars)
                        .map(|v| self.var_val(v) == Val::True)
                        .collect();
                    return BudgetedResult::Sat(model);
                }
                Some(l) => {
                    self.trail_lim.push(self.trail.len());
                    self.decisions += 1;
                    self.enqueue(l, Reason::Decision);
                }
            }
        }
    }
}

/// The outcome of testing clause `C` (as sorted `(var,sign)` codes) against clause `D`.
enum Sub {
    /// `C ⊆ D` — `C` subsumes `D`, so `D` is redundant.
    Subsumes,
    /// `C` self-subsumes `D` on one literal: all of `C` is in `D` except a single literal whose
    /// negation is in `D`. `D` can drop that negated literal (the carried code). Resolution + the
    /// resulting subsumption.
    Strengthen(u32),
    /// No subsumption relationship.
    No,
}

/// Classify `c` against `d` (both sorted, deduped `(var,sign)` code vectors). A code's sign bit is
/// the low bit, so `code ^ 1` is the opposite-polarity literal.
fn self_subsumes(c: &[u32], d: &[u32]) -> Sub {
    let mut pivot: Option<u32> = None;
    for &x in c {
        if d.binary_search(&x).is_ok() {
            continue; // x ∈ D
        }
        if d.binary_search(&(x ^ 1)).is_ok() {
            if pivot.is_some() {
                return Sub::No; // a second flipped literal ⇒ not (self-)subsuming
            }
            pivot = Some(x ^ 1);
        } else {
            return Sub::No; // x neither in D nor its negation ⇒ C ⊄ D
        }
    }
    match pivot {
        None => Sub::Subsumes,
        Some(p) => Sub::Strengthen(p),
    }
}

/// The Luby restart sequence `1,1,2,1,1,2,4,1,…` (Luby, Sinclair & Zuckerman, 1993) —
/// the optimal universal restart schedule.
fn luby(mut i: u64) -> u64 {
    // 1-indexed Luby.
    let mut k = 1u32;
    loop {
        let span = (1u64 << k) - 1;
        if i == span {
            return 1u64 << (k - 1);
        }
        if i < span {
            i -= (1u64 << (k - 1)) - 1;
            k = 1;
            continue;
        }
        k += 1;
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/cdcl/tests.rs"]
mod tests;
