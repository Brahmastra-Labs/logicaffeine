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

use std::collections::VecDeque;

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
    fn index(self) -> usize {
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
/// and forced this literal (its other literals are all false, earlier on the trail).
#[derive(Clone, Copy, Debug)]
enum Reason {
    Decision,
    Clause(usize),
}

/// A watch-list entry: the clause being watched plus a *blocking literal* — one of the clause's
/// other literals, cached inline. If the blocker is already true the clause is satisfied, so
/// propagation skips it without dereferencing the clause at all (the cache-miss saver from
/// Chaff/MiniSat). Purely an optimization: the verdict is unaffected.
#[derive(Clone, Copy, Debug)]
struct Watcher {
    clause: usize,
    blocker: Lit,
    /// Whether the watched clause is binary. For a binary clause the blocker *is* the only other
    /// literal, so when it is not already true the clause is immediately unit-or-conflict — decided
    /// with no clause dereference at all (the implicit-binary trick from Kissat/CaDiCaL). Kept in sync
    /// by watch (re)creation: strengthening always `unwatch`+`rewatch`es, and `reduce_db` rebuilds all
    /// watches, so a binary flag is never stale against clause contents.
    binary: bool,
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
/// the formula by reverse unit propagation — the unit of a DRAT/LRAT proof.
#[derive(Clone, Debug)]
pub struct LearnedClause {
    pub lits: Vec<Lit>,
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

/// The CDCL solver.
pub struct Solver {
    num_vars: usize,
    /// All clauses (original + learned); a clause is a `Vec<Lit>`.
    clauses: Vec<Vec<Lit>>,
    /// Watch lists: for each literal, the [`Watcher`]s on it. Two-watched-literal scheme — a
    /// clause is visited only when one of its two watched literals is falsified, and even then is
    /// skipped without a dereference when its cached blocking literal is already true.
    watches: Vec<Vec<Watcher>>,
    /// Per-variable value.
    value: Vec<Val>,
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
    /// Per-clause LBD (literal-block distance, Audemard & Simon 2009): the number of distinct
    /// decision levels among a learned clause's literals. Low LBD ("glue") clauses are the most
    /// reusable; high-LBD clauses are the deletion candidates. Originals carry `u32::MAX`.
    lbd: Vec<u32>,
    /// Whether to periodically delete high-LBD learned clauses, keeping the database compact (the
    /// Glucose lever). Verdict-invariant — deletion only removes redundant learned clauses; the
    /// original formula and all locked/glue clauses are kept. Toggleable for A/B benchmarking.
    reduce_enabled: bool,
    /// The live-learned-clause count that triggers a reduction; grows after each reduction.
    reduce_limit: usize,
    /// If `Some`, decisions are restricted to variables flagged `true` (the rest are left to theory
    /// propagation). DPLL(XOR) sets this to the non-pivot variables so search ranges only over the
    /// GF(2) kernel; `None` means branch on any variable (ordinary CDCL).
    decision_mask: Option<Vec<bool>>,
    /// VSIDS order heap (MiniSat-style indexed binary max-heap): the next decision is the highest-
    /// activity unassigned variable in O(log n), replacing the old O(n) activity scan — the dominant
    /// throughput lever at scale. `heap_pos[v]` is v's index in `heap`, or -1 when absent.
    heap: Vec<Var>,
    heap_pos: Vec<i32>,
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
            clauses: Vec::new(),
            watches: vec![Vec::new(); num_vars * 2],
            value: vec![Val::Unset; num_vars],
            level: vec![0; num_vars],
            reason: vec![Reason::Decision; num_vars],
            trail: Vec::new(),
            trail_lim: Vec::new(),
            qhead: 0,
            activity: vec![0.0; num_vars],
            var_inc: 1.0,
            saved_phase: vec![false; num_vars],
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
            reduce_enabled: true,
            reduce_limit: 2000,
            decision_mask: None,
            heap: (0..num_vars as Var).collect(),
            heap_pos: (0..num_vars as i32).collect(),
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
            restart_limit: luby(1) * 100,
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

    /// Set the conflict interval between inprocessing rounds (default [`INPROCESS_INTERVAL`]).
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
        self.csr = 0;
        self.restart_limit = luby(1) * 100;
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
            self.restart_limit = luby(1) * 100;
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

    /// Perform a restart: jump to level 0, advance the Luby counter, and reset the Glucose fast
    /// window so the next trigger measures freshly.
    fn do_restart(&mut self) {
        self.backtrack_to(0);
        self.csr = 0;
        self.restarts += 1;
        self.restart_no += 1;
        self.restart_limit = luby(self.restart_no) * 100;
        self.lbd_fast.clear();
        self.lbd_fast_sum = 0;
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
            if l < n && self.activity[self.heap[l] as usize] > self.activity[self.heap[m] as usize] {
                m = l;
            }
            if r < n && self.activity[self.heap[r] as usize] > self.activity[self.heap[m] as usize] {
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

    /// Set the live-learned-clause count that triggers a reduction (tuning / stress-testing).
    pub fn set_reduce_limit(&mut self, limit: usize) {
        self.reduce_limit = limit.max(1);
    }

    /// Seed the saved-phase polarities (the order decisions try first) from an external assignment.
    /// Purely a search-order hint — it never changes which models exist, so it cannot affect the
    /// verdict — but starting decisions on, e.g., a GF(2)-consistent assignment lets the hybrid XOR
    /// route begin on the linear system's solution manifold and only repair the residual clauses.
    pub fn set_initial_phase(&mut self, phases: &[bool]) {
        for (v, &p) in phases.iter().enumerate().take(self.num_vars) {
            self.saved_phase[v] = p;
        }
    }

    /// The number of learned clauses currently live in the database (originals excluded).
    pub fn live_learned(&self) -> usize {
        self.clauses.len().saturating_sub(self.n_original)
    }

    /// The learned clauses produced during the last solve (the DRAT/LRAT proof skeleton).
    pub fn learned(&self) -> &[LearnedClause] {
        &self.learned_log
    }

    /// The ORIGINAL clauses (those present before solving), borrowed in place — so a RUP
    /// checker can replay over them without copying the clause set.
    pub fn original_clauses(&self) -> &[Vec<Lit>] {
        &self.clauses[..self.n_original]
    }

    /// Add a clause. An empty clause makes the formula trivially unsatisfiable; a unit
    /// clause is enqueued as a top-level fact.
    pub fn add_clause(&mut self, mut lits: Vec<Lit>) {
        // Dedup and drop a clause that is already a tautology (`v ∨ ¬v`).
        lits.sort_by_key(|l| l.0);
        lits.dedup();
        for w in lits.windows(2) {
            if w[0].var() == w[1].var() {
                // contains both polarities of some var → tautology, skip
                return;
            }
        }
        self.add_clause_raw(lits, false);
    }

    /// Internal: register a clause and set up its two watches. `learned` clauses are also
    /// logged. Returns the clause index.
    fn add_clause_raw(&mut self, lits: Vec<Lit>, learned: bool) -> usize {
        let ci = self.clauses.len();
        // Keep the per-clause LBD vector aligned with `clauses` (grows by exactly one here).
        // Originals/units default to `u32::MAX` (never deleted); the caller overwrites a learned
        // clause's slot with its real LBD.
        self.lbd.push(u32::MAX);
        if learned {
            self.learned_log.push(LearnedClause { lits: lits.clone() });
        }
        if lits.is_empty() {
            self.empty_clause = true;
            self.clauses.push(lits);
            return ci;
        }
        if lits.len() == 1 {
            let l = lits[0];
            self.clauses.push(lits);
            // A unit clause forces its literal. If it is already FALSE on the trail, the
            // formula is unsatisfiable (a top-level conflict); if already true, nothing to
            // do; else enqueue it.
            match self.val_of(l) {
                Val::True => {}
                Val::False => self.empty_clause = true,
                Val::Unset => self.enqueue(l, Reason::Clause(ci)),
            }
            return ci;
        }
        // Watch the first two literals, each blocked by the other.
        let (w0, w1) = (lits[0], lits[1]);
        let bin = lits.len() == 2;
        self.watches[w0.index()].push(Watcher { clause: ci, blocker: w1, binary: bin });
        self.watches[w1.index()].push(Watcher { clause: ci, blocker: w0, binary: bin });
        self.clauses.push(lits);
        ci
    }

    #[inline]
    fn val_of(&self, l: Lit) -> Val {
        match self.value[l.var() as usize] {
            Val::Unset => Val::Unset,
            v => {
                if l.is_positive() {
                    v
                } else {
                    match v {
                        Val::True => Val::False,
                        Val::False => Val::True,
                        Val::Unset => Val::Unset,
                    }
                }
            }
        }
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
        let v = l.var() as usize;
        self.value[v] = if l.is_positive() { Val::True } else { Val::False };
        self.level[v] = self.trail_lim.len() as u32;
        self.reason[v] = r;
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
                if watchers[wi].binary {
                    let other = watchers[wi].blocker;
                    let ci = watchers[wi].clause;
                    if self.lit_false(other) {
                        self.watches[false_lit.index()] = watchers;
                        return Some(ci);
                    }
                    self.enqueue(other, Reason::Clause(ci));
                    wi += 1;
                    continue;
                }
                let ci = watchers[wi].clause;
                let clause_len = self.clauses[ci].len();
                // Ensure the watched literal we keep is in slot 0/1; make slot 1 = false_lit.
                if self.clauses[ci][0] == false_lit {
                    self.clauses[ci].swap(0, 1);
                }
                // Slot 0 is the other watched literal; refresh it as this watch's blocker.
                let other = self.clauses[ci][0];
                watchers[wi].blocker = other;
                // If it is already satisfied, the clause is fine — keep watching false_lit.
                if self.lit_true(other) {
                    wi += 1;
                    continue;
                }
                // Look for a new, non-false literal to watch (slots 2..).
                for k in 2..clause_len {
                    let lk = self.clauses[ci][k];
                    if !self.lit_false(lk) {
                        // Move lk into slot 1, watch it (blocked by `other`), drop this watch.
                        self.clauses[ci].swap(1, k);
                        // This path only fires for clauses with a third literal to move to, so the
                        // clause is never binary here (`clause_len >= 3`).
                        self.watches[lk.index()].push(Watcher { clause: ci, blocker: other, binary: clause_len == 2 });
                        watchers.swap_remove(wi);
                        continue 'next_clause;
                    }
                }
                // No new watch: clause is unit on `other` or conflicting.
                if self.lit_false(other) {
                    // Conflict: restore the rest of this watch list and report.
                    self.watches[false_lit.index()] = watchers;
                    return Some(ci);
                }
                // Unit: propagate `other`.
                self.enqueue(other, Reason::Clause(ci));
                wi += 1;
            }
            self.watches[false_lit.index()] = watchers;
        }
        None
    }

    /// First-UIP conflict analysis (GRASP/MiniSat). Walks the implication graph back from
    /// the conflicting clause to the first unique implication point at the current level,
    /// producing a learned clause and the level to backjump to.
    fn analyze(&mut self, conflict: usize) -> (Vec<Lit>, u32, u32) {
        let decision_level = self.trail_lim.len() as u32;
        let mut learned: Vec<Lit> = vec![Lit::pos(0)]; // slot 0 reserved for the UIP
        let mut counter = 0usize; // unresolved literals at the current level
        let mut p: Option<Lit> = None;
        let mut trail_idx = self.trail.len();
        let mut clause = conflict;

        loop {
            // Resolve with `clause` (its literals are all false; for the conflict clause, and for
            // each antecedent reason clause thereafter). Index rather than clone the clause: copying
            // each `Lit` out ends the immutable borrow before we `bump`/mark, so the hot 1UIP loop
            // never allocates — the way a production solver walks reason clauses.
            for k in 0..self.clauses[clause].len() {
                let q = self.clauses[clause][k];
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
                if self.seen[l.var() as usize] {
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
            clause = match self.reason[pv] {
                Reason::Clause(ci) => ci,
                Reason::Decision => unreachable!("UIP reached a decision before counter hit 0"),
            };
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
        self.minimize(&mut learned);
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
        (learned, backjump, lbd)
    }

    /// Recursive learned-clause minimization (Sörensson & Biere, 2009). A non-asserting literal is
    /// dropped if its reason clause's other literals are all already in the learned clause, are
    /// level-0 facts, or are themselves removable — meaning the literal is redundant. The result is
    /// a sub-clause still implied by the formula, so it is verdict-invariant; it just yields
    /// shorter, lower-LBD clauses that propagate and delete better.
    fn minimize(&mut self, learned: &mut Vec<Lit>) {
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
        // Sparse-reset the memo and hand the buffers back to `self` for the next conflict.
        for &v in &touched {
            cache[v as usize] = 0;
        }
        touched.clear();
        self.min_cache = cache;
        self.min_touched = touched;
    }

    fn lit_redundant(&self, lit: Lit, cache: &mut [u8], touched: &mut Vec<Var>, depth: u32) -> bool {
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
            // A decision literal anchors its level; it is never redundant.
            Reason::Decision => false,
            // No clone: the reason clause and the recursive call are both immutable borrows of `self`.
            // `seen[qv]` is the in-learned-clause test (marked by `minimize`).
            Reason::Clause(ci) => self.clauses[ci].iter().all(|&q| {
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

    fn bump(&mut self, v: Var) {
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
        while self.trail.len() > target {
            let l = self.trail.pop().unwrap();
            let v = l.var() as usize;
            // Remember the polarity this variable held, so the next decision on it reuses it.
            self.saved_phase[v] = self.value[v] == Val::True;
            self.value[v] = Val::Unset;
            // Return the now-unassigned variable to the order heap (if it's a decision candidate).
            if self.heap_pos[v] < 0 && self.decision_mask.as_ref().is_none_or(|m| m[v]) {
                self.heap_insert(v as Var);
            }
        }
        self.qhead = target;
        self.trail_lim.truncate(level as usize);
    }

    /// Delete the high-LBD half of the (deletable) learned clauses and compact the database.
    ///
    /// Soundness/verdict-invariance: only learned clauses are ever removed, and never a **locked**
    /// clause (a current reason on the trail), a **glue** clause (LBD ≤ 2), or a unit/binary. A
    /// learned clause is a resolvent already implied by the formula, so dropping it cannot change
    /// satisfiability. Surviving clauses keep their two watched literals (slots 0/1) unchanged, so
    /// the two-watched-literal invariant is preserved — we only renumber and rebuild the watch
    /// lists. The full proof trace (`learned_log`) is untouched, so a downstream RUP/PR check still
    /// replays every learned clause.
    fn reduce_db(&mut self) {
        let n = self.clauses.len();
        // A clause currently justifying a trail literal must not be deleted.
        let mut locked = vec![false; n];
        for &l in &self.trail {
            if let Reason::Clause(ci) = self.reason[l.var() as usize] {
                locked[ci] = true;
            }
        }
        // Deletion candidates: learned, unlocked, non-glue, length > 2.
        let mut cand: Vec<usize> = (self.n_original..n)
            .filter(|&ci| !locked[ci] && self.lbd[ci] > 2 && self.clauses[ci].len() > 2)
            .collect();
        if cand.is_empty() {
            return;
        }
        cand.sort_by(|&a, &b| self.lbd[b].cmp(&self.lbd[a])); // worst (highest LBD) first
        let drop_n = cand.len() / 2;
        let delete: std::collections::HashSet<usize> = cand.into_iter().take(drop_n).collect();
        self.compact(&delete);
    }

    /// Rebuild the clause database keeping every clause NOT in `delete`, renumbering reasons and
    /// rebuilding watch lists from each survivor's (unchanged) first two literals. Originals stay
    /// first — so `n_original` / `original_clauses()` remain valid — provided `delete` holds only
    /// learned indices, and no deleted clause is a current reason (locked). Shared by LBD reduction
    /// and subsumption.
    fn compact(&mut self, delete: &std::collections::HashSet<usize>) {
        if delete.is_empty() {
            return;
        }
        let n = self.clauses.len();
        let survivors: Vec<usize> = (0..n).filter(|ci| !delete.contains(ci)).collect();
        let mut remap = vec![usize::MAX; n];
        for (new, &old) in survivors.iter().enumerate() {
            remap[old] = new;
        }
        let new_clauses: Vec<Vec<Lit>> = survivors.iter().map(|&ci| self.clauses[ci].clone()).collect();
        let new_lbd: Vec<u32> = survivors.iter().map(|&ci| self.lbd[ci]).collect();
        for v in 0..self.num_vars {
            if let Reason::Clause(ci) = self.reason[v] {
                if remap[ci] != usize::MAX {
                    self.reason[v] = Reason::Clause(remap[ci]);
                }
            }
        }
        self.clauses = new_clauses;
        self.lbd = new_lbd;
        for w in self.watches.iter_mut() {
            w.clear();
        }
        for (ci, c) in self.clauses.iter().enumerate() {
            if c.len() >= 2 {
                let (l0, l1) = (c[0], c[1]);
                let bin = c.len() == 2;
                self.watches[l0.index()].push(Watcher { clause: ci, blocker: l1, binary: bin });
                self.watches[l1.index()].push(Watcher { clause: ci, blocker: l0, binary: bin });
            }
        }
    }

    /// Lift the two watchers of clause `ci` from the watch lists (its watched literals are, by
    /// invariant, `clauses[ci][0..2]`). Used to EXCLUDE a clause from propagation while it is being
    /// vivified, so the strengthened clause is implied by `F \ {C}` — the soundness key.
    fn unwatch(&mut self, ci: usize) {
        if self.clauses[ci].len() < 2 {
            return;
        }
        let (a, b) = (self.clauses[ci][0].index(), self.clauses[ci][1].index());
        if let Some(p) = self.watches[a].iter().position(|w| w.clause == ci) {
            self.watches[a].swap_remove(p);
        }
        if let Some(p) = self.watches[b].iter().position(|w| w.clause == ci) {
            self.watches[b].swap_remove(p);
        }
    }

    /// Re-establish watches on `clauses[ci]`'s current first two literals (the inverse of
    /// [`Self::unwatch`], after a possible literal rewrite).
    fn rewatch(&mut self, ci: usize) {
        if self.clauses[ci].len() < 2 {
            return;
        }
        let (l0, l1) = (self.clauses[ci][0], self.clauses[ci][1]);
        let bin = self.clauses[ci].len() == 2;
        self.watches[l0.index()].push(Watcher { clause: ci, blocker: l1, binary: bin });
        self.watches[l1.index()].push(Watcher { clause: ci, blocker: l0, binary: bin });
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
        let c = self.clauses[ci].clone();
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
        let n = self.clauses.len();
        let cap = if budget == 0 { usize::MAX } else { budget };
        let mut done = 0usize;
        for ci in self.n_original..n {
            if done >= cap {
                break;
            }
            if self.clauses[ci].len() < 2 {
                continue; // units/empties: nothing to strengthen
            }
            done += 1;
            self.unwatch(ci);
            match self.vivify_clause(ci) {
                None => self.rewatch(ci),
                Some(kept) => {
                    self.vivifications += 1;
                    self.learned_log.push(LearnedClause { lits: kept.clone() });
                    if kept.is_empty() {
                        self.empty_clause = true;
                        self.clauses[ci] = kept;
                        return false;
                    }
                    if kept.len() == 1 {
                        let u = kept[0];
                        self.clauses[ci] = kept;
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
                    self.clauses[ci] = kept;
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
            if self.value[v as usize] != Val::Unset {
                continue; // already a level-0 fact
            }
            if self.decision_mask.as_ref().is_some_and(|m| !m[v as usize]) {
                continue; // not a decision candidate
            }
            done += 1;
            for probe in [Lit::pos(v), Lit::neg(v)] {
                if self.value[v as usize] != Val::Unset {
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
        let n = self.clauses.len();
        if n <= self.n_original {
            return true;
        }
        let code = |l: Lit| l.var() * 2 + u32::from(!l.is_positive());
        let mut coded: Vec<Vec<u32>> = Vec::with_capacity(n);
        let mut sigs: Vec<u64> = Vec::with_capacity(n);
        for c in &self.clauses {
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
            let new: Vec<Lit> = self.clauses[di]
                .iter()
                .copied()
                .filter(|&l| code(l) != drop_code)
                .collect();
            self.subsumptions += 1;
            self.learned_log.push(LearnedClause { lits: new.clone() });
            if new.is_empty() {
                self.empty_clause = true;
                self.clauses[di] = new;
                return false;
            }
            if new.len() == 1 {
                let u = new[0];
                self.clauses[di] = new;
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
                self.clauses[di] = new;
                self.rewatch(di);
            }
        }
        self.subsumptions += delete.len() as u64;
        self.compact(&delete);
        true
    }

    /// Rotate the saved-phase strategy (rephasing). Decisions reuse `saved_phase`; periodically
    /// overwriting it — invert, all-false, all-true, then leave the search's own saved phases — kicks
    /// the search out of a basin without changing completeness. Pure heuristic; verdict-invariant.
    fn rephase(&mut self, round: u64) {
        match round % 4 {
            0 => {
                for p in self.saved_phase.iter_mut() {
                    *p = !*p;
                }
            }
            1 => self.saved_phase.iter_mut().for_each(|p| *p = false),
            2 => self.saved_phase.iter_mut().for_each(|p| *p = true),
            _ => {}
        }
    }

    /// Run one level-0 inprocessing round: failed-literal probing (only while it keeps paying off),
    /// then subsumption + self-subsuming resolution, then learned-clause vivification, then a
    /// rephase. Returns `false` if it proves the formula UNSAT. Must be called at decision level 0
    /// (the scheduler calls it right after a restart). Verdict-invariant.
    fn inprocess(&mut self, round: u64) -> bool {
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
        if !self.vivify_round(VIVIFY_BUDGET) {
            return false;
        }
        self.rephase(round);
        true
    }

    /// The decision literal for `v`: its saved phase (false-first on the first ever decision).
    fn decision_lit(&self, v: Var) -> Lit {
        if self.saved_phase[v as usize] {
            Lit::pos(v)
        } else {
            Lit::neg(v)
        }
    }

    /// Pick the highest-activity unassigned decision variable in O(log n) via the order heap,
    /// discarding popped variables that are already assigned (lazy deletion). `None` once every
    /// decision candidate is assigned. Backtracking re-inserts unassigned candidates, so the heap is
    /// never missing a candidate when a decision is actually needed.
    fn pick_branch(&mut self) -> Option<Var> {
        loop {
            let v = self.heap_pop()?;
            if self.value[v as usize] == Val::Unset {
                return Some(v);
            }
        }
    }

    /// The Literal-Block-Distance of a clause: the number of distinct decision levels among its
    /// literals (Audemard & Simon, 2009). Low LBD ⇒ "glue" ⇒ kept across reductions.
    fn clause_lbd(&self, lits: &[Lit]) -> u32 {
        let mut levels: Vec<u32> = lits.iter().map(|l| self.level[l.var() as usize]).collect();
        levels.sort_unstable();
        levels.dedup();
        levels.len() as u32
    }

    /// The conflict aftermath shared by Boolean and theory conflicts: derive the 1-UIP asserting
    /// clause from conflicting clause `ci`, backjump, learn + enqueue it, then run the decay /
    /// reduce-DB / restart bookkeeping. Returns `true` when the conflict is at decision level 0 — the
    /// formula is UNSAT.
    fn after_conflict(&mut self, ci: usize) -> bool {
        if self.trail_lim.is_empty() {
            return true; // conflict at level 0
        }
        let trail_at_conflict = self.trail.len();
        let (learned, backjump, lbd) = self.analyze(ci);
        self.note_conflict(lbd, trail_at_conflict);
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
        self.csr += 1;
        if self.reduce_enabled && self.live_learned() >= self.reduce_limit {
            self.reduce_db();
            self.reduce_limit += 500;
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
        // Mark which clauses are original so DB reduction never deletes them (the omission that
        // would otherwise let reduction drop the formula itself and report a bogus SAT).
        self.n_original = self.clauses.len();
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
                if self.trail_lim.is_empty() {
                    return BudgetedResult::Unsat;
                }
                let trail_at_conflict = self.trail.len();
                let (learned, backjump, lbd) = self.analyze(ci);
                self.note_conflict(lbd, trail_at_conflict);
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
                self.csr += 1;
                if max_conflicts != 0 && self.conflicts - start >= max_conflicts {
                    self.backtrack_to(0);
                    return BudgetedResult::Budget;
                }
                if self.reduce_enabled && self.live_learned() >= self.reduce_limit {
                    self.reduce_db();
                    self.reduce_limit += 500;
                }
                self.advance_restart_phase();
                if self.want_restart() {
                    self.do_restart();
                }
                continue;
            }
            match self.pick_branch() {
                None => {
                    let model = (0..self.num_vars).map(|v| self.value[v] == Val::True).collect();
                    return BudgetedResult::Sat(model);
                }
                Some(v) => {
                    self.trail_lim.push(self.trail.len());
                    self.decisions += 1;
                    self.enqueue(self.decision_lit(v), Reason::Decision);
                }
            }
        }
    }

    /// Solve with theory propagators. Each is consulted at every Boolean fixpoint; a
    /// returned clause is added to the formula (and may immediately propagate or conflict).
    pub fn solve_with(&mut self, theories: &mut [Box<dyn Theory>]) -> SolveResult {
        self.n_original = self.clauses.len();
        if self.empty_clause {
            return SolveResult::Unsat;
        }
        // Top-level propagation of any unit clauses already enqueued.
        if self.propagate().is_some() {
            return SolveResult::Unsat;
        }
        self.reset_restart_state();
        let mut last_inprocess = 0u64;
        let mut inprocess_rounds = 0u64;
        let mut inprocess_gap = self.inprocess_interval;
        self.probe_active = true; // re-arm probing for this solve

        loop {
            let conflict = self.propagate();
            if let Some(ci) = conflict {
                let restarts_before = self.restarts;
                if self.after_conflict(ci) {
                    return SolveResult::Unsat;
                }
                // A restart just returned us to level 0 — the safe point to inprocess. Gated by a
                // GROWING gap so only long searches pay, and less and less as the search runs on
                // (bounding churn). probe + subsume + vivify + rephase, all verdict-invariant; a
                // `false` return means inprocessing proved UNSAT.
                if self.inprocess_enabled
                    && theories.is_empty()
                    && self.restarts > restarts_before
                    && self.conflicts - last_inprocess >= inprocess_gap
                {
                    last_inprocess = self.conflicts;
                    if !self.inprocess(inprocess_rounds) {
                        return SolveResult::Unsat;
                    }
                    inprocess_rounds += 1;
                    inprocess_gap = ((inprocess_gap as f64) * INPROCESS_GROWTH) as u64;
                }
                continue;
            }
            // Boolean fixpoint — consult theories before branching. Every clause a theory hands back
            // is a globally-valid no-good (a logical consequence of the formula), so we CARRY it into
            // the learned database: a contradiction found by one strategy holds in every model, and a
            // carried clause prunes that branch for the rest of the search. A theory conflict (all
            // literals false) drives a 1-UIP backjump exactly like a Boolean conflict — sound because
            // we consult at EVERY fixpoint, so the inconsistency is caught at the level of its last
            // contributing assignment and the carried clause has a current-level literal. A theory
            // propagation (one literal still free) is enqueued with the carried clause as its reason,
            // so later conflict analysis resolves through the theory's reasoning.
            let mut theory_acted = false;
            for ti in 0..theories.len() {
                let implied = theories[ti].propagate(&self.trail);
                if implied.is_empty() {
                    continue;
                }
                // A theory conflict takes priority: carry it and run conflict analysis.
                if let Some(conf) = implied.iter().find(|c| c.iter().all(|&l| self.lit_false(l))) {
                    if conf.is_empty() {
                        return SolveResult::Unsat; // an unconditional contradiction (0 = 1)
                    }
                    let lbd = self.clause_lbd(conf);
                    let ci = self.add_clause_raw(conf.clone(), true);
                    self.lbd[ci] = lbd;
                    if self.after_conflict(ci) {
                        return SolveResult::Unsat;
                    }
                    theory_acted = true;
                    break;
                }
                // Otherwise carry every implied unit, enqueueing its one free literal.
                for c in &implied {
                    if c.iter().any(|&l| self.lit_true(l)) {
                        continue; // already satisfied — stale
                    }
                    let free: Vec<usize> = (0..c.len()).filter(|&i| self.val_of(c[i]) == Val::Unset).collect();
                    if free.len() != 1 {
                        continue; // not a clean unit propagation — skip defensively
                    }
                    let mut lits = c.clone();
                    lits.swap(0, free[0]);
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
            // Decide.
            match self.pick_branch() {
                None => {
                    // Full assignment → SAT.
                    let model = (0..self.num_vars)
                        .map(|v| self.value[v] == Val::True)
                        .collect();
                    return SolveResult::Sat(model);
                }
                Some(v) => {
                    self.trail_lim.push(self.trail.len());
                    // Phase saving: reuse the variable's last polarity (false-first initially).
                    self.decisions += 1;
                    self.enqueue(self.decision_lit(v), Reason::Decision);
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
    /// Restarts are disabled in this path: the assumptions occupy the bottom decision levels,
    /// and skipping restarts keeps them pinned without a restart-floor dance. The small,
    /// bounded queries this serves do not need restarts; correctness beats the heuristic.
    /// (Does not touch `n_original`; do not mix with [`Solver::original_clauses`]/RUP on the
    /// same solver.)
    pub fn solve_under_assumptions(&mut self, assumptions: &[Lit]) -> SolveResult {
        // Drop any prior search state, keeping level-0 facts and all learned clauses.
        self.backtrack_to(0);
        if self.empty_clause {
            return SolveResult::Unsat;
        }
        // Level-0 propagation: if the permanent formula is already unsatisfiable, no
        // assumption can rescue it — and it stays unsat for every future query, so latch the
        // permanent-unsat flag (this also guarantees a clean state on the next call, which a
        // no-op `backtrack_to(0)` over an empty `trail_lim` would otherwise inherit dirty).
        if self.propagate().is_some() {
            self.empty_clause = true;
            return SolveResult::Unsat;
        }
        loop {
            if let Some(ci) = self.propagate() {
                if self.trail_lim.is_empty() {
                    self.empty_clause = true; // conflict with no decisions ⇒ unconditionally unsat
                    return SolveResult::Unsat;
                }
                let (learned, backjump, lbd) = self.analyze(ci);
                self.backtrack_to(backjump);
                let asserting = learned[0];
                let unit = learned.len() == 1;
                let new_ci = self.add_clause_raw(learned, true);
                self.lbd[new_ci] = lbd;
                if !unit {
                    self.enqueue(asserting, Reason::Clause(new_ci));
                }
                self.decay();
                continue;
            }
            // Decide: place the first not-yet-satisfied assumption (so the search always
            // explores under the full assumption set, even after a backjump unset some).
            let mut decided = false;
            for &a in assumptions {
                match self.val_of(a) {
                    // The assumption is forced false ⇒ no model under the assumptions.
                    Val::False => return SolveResult::Unsat,
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
            match self.pick_branch() {
                None => {
                    let model = (0..self.num_vars)
                        .map(|v| self.value[v] == Val::True)
                        .collect();
                    return SolveResult::Sat(model);
                }
                Some(v) => {
                    self.trail_lim.push(self.trail.len());
                    self.decisions += 1;
                    self.enqueue(self.decision_lit(v), Reason::Decision);
                }
            }
        }
    }
}

/// The Luby restart sequence `1,1,2,1,1,2,4,1,…` (Luby, Sinclair & Zuckerman, 1993) —
/// the optimal universal restart schedule.
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
mod tests {
    use super::*;

    #[test]
    #[ignore = "bench: random-3SAT solve throughput — before/after engine tuning"]
    fn bench_random_3sat_throughput() {
        use std::time::Instant;
        let mut total_ms = 0.0;
        let mut conflicts = 0u64;
        let mut props = 0u64;
        for seed in 0..24u64 {
            let n = 150;
            let m = (n as f64 * 4.26) as usize;
            let cnf = crate::families::random_3sat(n, m, 0xBEEFu64 ^ seed);
            let mut s = Solver::new(cnf.num_vars);
            for c in &cnf.clauses {
                s.add_clause(c.clone());
            }
            let t = Instant::now();
            let _ = s.solve();
            total_ms += t.elapsed().as_secs_f64() * 1e3;
            conflicts += s.conflicts();
            props += s.propagations();
        }
        eprintln!(
            "[bench] random-3SAT ×24 @ n=150: {total_ms:.1}ms total ({:.3}ms/inst), {conflicts} conflicts, {props} propagations",
            total_ms / 24.0
        );
    }

    #[test]
    fn counters_track_search_work() {
        // All 8 clauses over 3 vars (every assignment blocked) → UNSAT, forcing real search:
        // decisions, propagations, and conflicts must all register, and propagations (one per
        // trail literal processed) must dominate conflicts.
        let mut s = Solver::new(3);
        for mask in 0..8u32 {
            let c: Vec<Lit> = (0..3).map(|v| Lit::new(v, (mask >> v) & 1 == 0)).collect();
            s.add_clause(c);
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
        assert!(s.conflicts() > 0, "expected conflicts, got 0");
        assert!(s.decisions() > 0, "expected decisions, got 0");
        assert!(s.propagations() >= s.conflicts(), "propagations should dominate conflicts");
    }

    #[test]
    fn order_heap_pops_in_descending_activity() {
        let mut s = Solver::new(5);
        for _ in 0..5 {
            s.bump(4); // activity[4] = 5
        }
        for _ in 0..3 {
            s.bump(2); // activity[2] = 3
        }
        s.bump(0); // activity[0] = 1; vars 1,3 stay at 0
        let mut order = Vec::new();
        while let Some(v) = s.heap_pop() {
            order.push(v);
        }
        assert_eq!(&order[..3], &[4, 2, 0], "highest activity first");
        let mut rest = order[3..].to_vec();
        rest.sort();
        assert_eq!(rest, vec![1, 3], "zero-activity vars come last");
    }

    fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
        // Enumerate all 2^n assignments; true iff some assignment satisfies every clause.
        for mask in 0u64..(1u64 << num_vars) {
            let val = |v: Var| (mask >> v) & 1 == 1;
            let ok = clauses.iter().all(|c| {
                c.iter().any(|l| {
                    let b = val(l.var());
                    if l.is_positive() {
                        b
                    } else {
                        !b
                    }
                })
            });
            if ok {
                return true;
            }
        }
        false
    }

    fn check_model(clauses: &[Vec<Lit>], model: &[bool]) -> bool {
        clauses.iter().all(|c| {
            c.iter().any(|l| {
                let b = model[l.var() as usize];
                if l.is_positive() {
                    b
                } else {
                    !b
                }
            })
        })
    }

    #[test]
    fn unit_and_empty() {
        // Empty clause ⇒ Unsat.
        let mut s = Solver::new(1);
        s.add_clause(vec![]);
        assert_eq!(s.solve(), SolveResult::Unsat);

        // x ∧ ¬x ⇒ Unsat.
        let mut s = Solver::new(1);
        s.add_clause(vec![Lit::pos(0)]);
        s.add_clause(vec![Lit::neg(0)]);
        assert_eq!(s.solve(), SolveResult::Unsat);
    }

    #[test]
    fn tiny_sat() {
        // (x ∨ y) ∧ (¬x ∨ y) ∧ (¬y ∨ z): forces y, then z; x free.
        let mut s = Solver::new(3);
        s.add_clause(vec![Lit::pos(0), Lit::pos(1)]);
        s.add_clause(vec![Lit::neg(0), Lit::pos(1)]);
        s.add_clause(vec![Lit::neg(1), Lit::pos(2)]);
        match s.solve() {
            SolveResult::Sat(m) => {
                assert!(m[1] && m[2], "y and z forced true");
            }
            SolveResult::Unsat => panic!("should be SAT"),
        }
    }

    #[test]
    fn pigeonhole_3_into_2_unsat() {
        // 3 pigeons, 2 holes: p_{i,h} = pigeon i in hole h. Each pigeon in some hole; no
        // two pigeons share a hole. Classic UNSAT (PHP) — exercises conflict learning.
        let var = |i: usize, h: usize| (i * 2 + h) as Var;
        let mut s = Solver::new(6);
        for i in 0..3 {
            s.add_clause(vec![Lit::pos(var(i, 0)), Lit::pos(var(i, 1))]);
        }
        for h in 0..2 {
            for i in 0..3 {
                for j in (i + 1)..3 {
                    s.add_clause(vec![Lit::neg(var(i, h)), Lit::neg(var(j, h))]);
                }
            }
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
    }

    #[test]
    fn random_against_brute_force() {
        // Deterministic pseudo-random 3-CNFs over up to 6 vars; cross-check SAT/UNSAT and
        // validate every returned model. The only honest way to trust a SAT core.
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _trial in 0..400 {
            let num_vars = 3 + (next() % 4) as usize; // 3..6
            let num_clauses = 3 + (next() % 12) as usize;
            let mut clauses = Vec::new();
            for _ in 0..num_clauses {
                let mut c = Vec::new();
                for _ in 0..3 {
                    let v = (next() % num_vars as u64) as Var;
                    let positive = next() & 1 == 0;
                    c.push(Lit::new(v, positive));
                }
                clauses.push(c);
            }
            let expected = sat_brute(num_vars, &clauses);
            let mut s = Solver::new(num_vars);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            match s.solve() {
                SolveResult::Sat(m) => {
                    assert!(expected, "solver said SAT but brute force says UNSAT");
                    assert!(check_model(&clauses, &m), "returned model does not satisfy the formula");
                }
                SolveResult::Unsat => {
                    assert!(!expected, "solver said UNSAT but brute force found a model");
                }
            }
        }
    }

    #[test]
    fn reduction_preserves_verdicts_and_models() {
        // LBD clause deletion must be verdict-invariant. With reduction forced after every few
        // learned clauses — so the delete + reason-remap + watch-rebuild path runs constantly —
        // every verdict must still match brute force and every model must satisfy the formula.
        // The strongest guard against a reduceDB bug.
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _trial in 0..500 {
            let num_vars = 3 + (next() % 5) as usize; // 3..7
            let num_clauses = 10 + (next() % 25) as usize; // over-constrained → conflicts → reductions
            let mut clauses = Vec::new();
            for _ in 0..num_clauses {
                let mut c = Vec::new();
                for _ in 0..3 {
                    c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
                }
                clauses.push(c);
            }
            let expected = sat_brute(num_vars, &clauses);
            let mut s = Solver::new(num_vars);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            s.set_reduce_limit(4); // hammer the reduction path
            match s.solve() {
                SolveResult::Sat(m) => {
                    assert!(expected, "reduce-on solver said SAT but brute force says UNSAT");
                    assert!(check_model(&clauses, &m), "model invalid under reduction");
                }
                SolveResult::Unsat => {
                    assert!(!expected, "reduce-on solver said UNSAT but a model exists");
                }
            }
        }
    }

    fn rng(seed: u64) -> impl FnMut() -> u64 {
        let mut state = seed;
        move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        }
    }

    #[test]
    fn restart_modes_are_all_verdict_invariant_default_adaptive() {
        // The adaptive (alternating Glucose/Luby phase) policy is the default, and switching restart
        // heuristics changes only search ORDER — every verdict and model must still agree with brute
        // force under ALL three policies.
        assert_eq!(
            Solver::new(1).restart_mode(),
            RestartMode::Adaptive,
            "adaptive restarts are the default"
        );
        let mut next = rng(0x51ed_2701_a1b2_c3d4);
        for _ in 0..400 {
            let num_vars = 3 + (next() % 5) as usize; // 3..7
            let num_clauses = 8 + (next() % 22) as usize;
            let mut clauses = Vec::new();
            for _ in 0..num_clauses {
                let mut c = Vec::new();
                for _ in 0..3 {
                    c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
                }
                clauses.push(c);
            }
            let expected = sat_brute(num_vars, &clauses);
            for mode in [RestartMode::Adaptive, RestartMode::Glucose, RestartMode::Luby] {
                let mut s = Solver::new(num_vars);
                s.set_restart_mode(mode);
                for c in &clauses {
                    s.add_clause(c.clone());
                }
                match s.solve() {
                    SolveResult::Sat(m) => {
                        assert!(expected, "{mode:?}: SAT but brute force UNSAT");
                        assert!(check_model(&clauses, &m), "{mode:?}: invalid model");
                    }
                    SolveResult::Unsat => assert!(!expected, "{mode:?}: UNSAT but SAT"),
                }
            }
        }
    }

    fn php_solver(n: usize, holes: usize, mode: RestartMode) -> Solver {
        let var = |p: usize, h: usize| (p * holes + h) as Var;
        let mut s = Solver::new(n * holes);
        s.set_restart_mode(mode);
        for p in 0..n {
            s.add_clause((0..holes).map(|h| Lit::pos(var(p, h))).collect());
        }
        for h in 0..holes {
            for i in 0..n {
                for j in (i + 1)..n {
                    s.add_clause(vec![Lit::neg(var(i, h)), Lit::neg(var(j, h))]);
                }
            }
        }
        s
    }

    #[test]
    fn glucose_restarts_fire_and_beat_luby_on_pigeonhole() {
        // PHP(8→7) does real exponential-resolution work, and on it the dynamic LBD policy must
        // (a) actually restart — proving the mechanism is wired, not dormant — and (b) need no
        // more conflicts than the Luby baseline (here it roughly halves them). The blocking
        // counter is read to exercise its accounting. Verdicts are checked in the differential
        // test above; this one is about the restart *heuristic* paying off.
        let mut g = php_solver(8, 7, RestartMode::Glucose);
        assert_eq!(g.solve(), SolveResult::Unsat);
        assert!(g.restarts() > 0, "Glucose must restart on PHP(8); got {}", g.restarts());
        let _ = g.blocked_restarts();

        let mut l = php_solver(8, 7, RestartMode::Luby);
        assert_eq!(l.solve(), SolveResult::Unsat);
        assert!(
            g.conflicts() <= l.conflicts(),
            "Glucose ({} conflicts) should not exceed Luby ({} conflicts) on PHP(8)",
            g.conflicts(),
            l.conflicts(),
        );
    }

    #[test]
    fn vivify_preserves_verdicts() {
        // Interleave a vivification round into solving: learn a few clauses with a tiny budget,
        // strengthen them, then solve to completion — the verdict and any model must still match
        // brute force on the ORIGINAL formula. Vivify replaces a learned clause with an implied
        // sub-clause, so it can never change satisfiability. (That it *fires* is proven on
        // pigeonhole below, where instances are big enough to learn strengthenable clauses.)
        let mut next = rng(0x7654_3210_fedc_ba98);
        for _ in 0..600 {
            let num_vars = 3 + (next() % 5) as usize; // 3..7
            let num_clauses = 10 + (next() % 22) as usize; // over-constrained → conflicts
            let mut clauses = Vec::new();
            for _ in 0..num_clauses {
                let width = 2 + (next() % 2) as usize;
                let mut c = Vec::new();
                for _ in 0..width {
                    c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
                }
                clauses.push(c);
            }
            let expected = sat_brute(num_vars, &clauses);
            let mut s = Solver::new(num_vars);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            // Vivify only from a clean budget-exhausted state (the supported inprocessing point,
            // mirroring the scheduler). A terminal verdict from the budgeted call is just checked.
            match s.solve_budgeted(12) {
                BudgetedResult::Sat(m) => {
                    assert!(expected, "budgeted SAT but brute force UNSAT");
                    assert!(check_model(&clauses, &m), "budgeted model invalid");
                }
                BudgetedResult::Unsat => assert!(!expected, "budgeted UNSAT but a model exists"),
                BudgetedResult::Budget => {
                    if !s.vivify() {
                        assert!(!expected, "vivify reported UNSAT but a model exists");
                        continue;
                    }
                    match s.solve() {
                        SolveResult::Sat(m) => {
                            assert!(expected, "post-vivify SAT but brute force UNSAT");
                            assert!(check_model(&clauses, &m), "post-vivify model invalid");
                        }
                        SolveResult::Unsat => {
                            assert!(!expected, "post-vivify UNSAT but a model exists")
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn vivify_fires_and_preserves_verdict_on_pigeonhole() {
        // PHP is big enough to learn strengthenable clauses, so vivification must actually fire
        // (not be a dormant no-op) AND keep the UNSAT verdict. PHP(6→5) yields ~16 strengthenings.
        let mut s = php_solver(6, 5, RestartMode::Glucose);
        assert_eq!(s.solve_budgeted(30), BudgetedResult::Budget, "budget should exhaust mid-search");
        assert!(s.vivify(), "vivify should not yet prove UNSAT");
        assert!(s.vivifications() > 0, "vivification must fire on PHP — the pass is dormant");
        assert_eq!(s.solve(), SolveResult::Unsat, "UNSAT preserved through vivification");
    }

    #[test]
    fn vivify_on_a_budgeted_prefix_preserves_pigeonhole_unsat() {
        // PHP(5→4) needs real search, so a small conflict budget exhausts cleanly — the supported
        // inprocessing state. Vivifying the learned clauses and finishing the solve must still
        // prove UNSAT (and the budgeted prefix must not regress to a bogus SAT).
        let mut s = php_solver(5, 4, RestartMode::Glucose);
        match s.solve_budgeted(5) {
            BudgetedResult::Unsat => {} // already proven within budget — still correct
            BudgetedResult::Sat(_) => panic!("PHP(5) is UNSAT"),
            BudgetedResult::Budget => {
                assert!(s.vivify(), "vivify should not yet prove UNSAT");
                assert_eq!(s.solve(), SolveResult::Unsat, "UNSAT preserved through vivification");
            }
        }
    }

    #[test]
    fn probe_preserves_verdicts() {
        // Failed-literal probing derives only units that the formula entails, so running a probing
        // round before the solve must never change satisfiability or invalidate a model. Checked
        // against brute force on many random formulas.
        let mut next = rng(0x1c1c_2d2d_3e3e_4f4f);
        for _ in 0..600 {
            let num_vars = 3 + (next() % 5) as usize; // 3..7
            let num_clauses = 6 + (next() % 18) as usize;
            let mut clauses = Vec::new();
            for _ in 0..num_clauses {
                let width = 2 + (next() % 2) as usize;
                let mut c = Vec::new();
                for _ in 0..width {
                    c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
                }
                clauses.push(c);
            }
            let expected = sat_brute(num_vars, &clauses);
            let mut s = Solver::new(num_vars);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            if !s.probe() {
                assert!(!expected, "probe reported UNSAT but a model exists");
                continue;
            }
            match s.solve() {
                SolveResult::Sat(m) => {
                    assert!(expected, "post-probe SAT but brute force UNSAT");
                    assert!(check_model(&clauses, &m), "post-probe model invalid");
                }
                SolveResult::Unsat => assert!(!expected, "post-probe UNSAT but a model exists"),
            }
        }
    }

    #[test]
    fn probe_derives_a_failed_literal_unit() {
        // (¬v ∨ a) ∧ (¬v ∨ ¬a): assuming v=true forces a AND ¬a — a conflict — so probing must
        // derive the unit ¬v. The formula is SAT (v=false), and the post-probe model must reflect
        // the forced ¬v.
        let (v, a) = (0u32, 1u32);
        let clauses = vec![
            vec![Lit::neg(v), Lit::pos(a)],
            vec![Lit::neg(v), Lit::neg(a)],
        ];
        let mut s = Solver::new(2);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        assert!(s.probe(), "probing should not prove this SAT formula UNSAT");
        assert!(s.probes() > 0, "probing must derive the failed-literal unit ¬v");
        match s.solve() {
            SolveResult::Sat(m) => {
                assert!(!m[v as usize], "¬v was forced by probing");
                assert!(check_model(&clauses, &m));
            }
            SolveResult::Unsat => panic!("formula is satisfiable (v = false)"),
        }
    }

    #[test]
    fn inprocessing_engages_inside_solve_and_improves_search() {
        // PHP(9→8) crosses the inprocessing interval many times (at the tuned-down test interval),
        // so the scheduler must (a) actually fire inside solve() — observable via the counters —
        // and (b) cut the conflict count versus the same solve with inprocessing disabled. The
        // verdict is preserved both ways. (On much larger PHP instances the win grows to ~2×.)
        let mut on = php_solver(9, 8, RestartMode::Glucose);
        on.set_inprocess_interval(400);
        assert_eq!(on.solve(), SolveResult::Unsat);
        assert!(
            on.vivifications() > 0,
            "inprocessing must engage on a long solve; vivifications={}",
            on.vivifications(),
        );

        let mut off = php_solver(9, 8, RestartMode::Glucose);
        off.set_inprocess(false);
        assert_eq!(off.solve(), SolveResult::Unsat);
        assert_eq!(off.vivifications() + off.probes(), 0, "toggle must suppress inprocessing");
        assert!(
            on.conflicts() < off.conflicts(),
            "inprocessing should cut conflicts: on={} off={}",
            on.conflicts(),
            off.conflicts(),
        );
    }

    #[test]
    fn self_subsumes_classifies_subsumption_and_ssr() {
        // Direct test of the subsumption classifier on (var,sign) codes: 2v = +v, 2v+1 = ¬v.
        let pos = |v: u32| 2 * v;
        let neg = |v: u32| 2 * v + 1;
        let sorted = |mut x: Vec<u32>| {
            x.sort_unstable();
            x
        };
        let c = sorted(vec![pos(0), pos(1)]); // {a, b}
        // {a,b} ⊆ {a,b,c} → subsumes.
        assert!(matches!(
            self_subsumes(&c, &sorted(vec![pos(0), pos(1), pos(2)])),
            Sub::Subsumes
        ));
        // {a,b} vs {¬a,b,c}: a flips → strengthen, dropping ¬a from D.
        assert!(matches!(
            self_subsumes(&c, &sorted(vec![neg(0), pos(1), pos(2)])),
            Sub::Strengthen(p) if p == neg(0)
        ));
        // {a,d} vs {¬a,b,c}: d absent in either polarity → no relation.
        assert!(matches!(
            self_subsumes(&sorted(vec![pos(0), pos(3)]), &sorted(vec![neg(0), pos(1), pos(2)])),
            Sub::No
        ));
        // {a,b} vs {¬a,¬b,c}: two flips → no relation (resolution would not subsume).
        assert!(matches!(
            self_subsumes(&c, &sorted(vec![neg(0), neg(1), pos(2)])),
            Sub::No
        ));
    }

    #[test]
    fn subsume_preserves_verdicts() {
        // Subsumption deletes only entailed learned clauses and SSR strengthens to sound resolvents,
        // so a subsumption round before finishing the solve must never change the verdict or
        // invalidate a model. Checked against brute force. (Firing at scale is covered by the
        // arena measurement; here soundness is the point.)
        let mut next = rng(0xa5a5_5a5a_c3c3_3c3c);
        for _ in 0..600 {
            let num_vars = 3 + (next() % 5) as usize;
            let num_clauses = 10 + (next() % 22) as usize;
            let mut clauses = Vec::new();
            for _ in 0..num_clauses {
                let width = 2 + (next() % 2) as usize;
                let mut c = Vec::new();
                for _ in 0..width {
                    c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
                }
                clauses.push(c);
            }
            let expected = sat_brute(num_vars, &clauses);
            let mut s = Solver::new(num_vars);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            match s.solve_budgeted(12) {
                BudgetedResult::Sat(m) => {
                    assert!(expected);
                    assert!(check_model(&clauses, &m));
                }
                BudgetedResult::Unsat => assert!(!expected),
                BudgetedResult::Budget => {
                    if !s.subsume() {
                        assert!(!expected, "subsume reported UNSAT but a model exists");
                        continue;
                    }
                    match s.solve() {
                        SolveResult::Sat(m) => {
                            assert!(expected, "post-subsume SAT but brute force UNSAT");
                            assert!(check_model(&clauses, &m), "post-subsume model invalid");
                        }
                        SolveResult::Unsat => {
                            assert!(!expected, "post-subsume UNSAT but a model exists")
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn solve_under_assumptions_matches_brute_force() {
        // Incremental SAT, validated against the oracle: for each random formula, fire MANY
        // assumption queries at the SAME solver (so it accumulates learned clauses), and
        // demand every verdict + model agree with brute force on `clauses ∧ assumptions`.
        // Reusing the solver is the whole point — it proves learned-clause reuse across
        // different assumption sets stays sound.
        let mut state = 0x243f_6a88_85a3_08d3u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _trial in 0..300 {
            let num_vars = 3 + (next() % 4) as usize; // 3..6
            let num_clauses = 3 + (next() % 10) as usize;
            let mut clauses = Vec::new();
            for _ in 0..num_clauses {
                let width = 2 + (next() % 2) as usize; // 2- or 3-literal
                let mut c = Vec::new();
                for _ in 0..width {
                    let v = (next() % num_vars as u64) as Var;
                    let positive = next() & 1 == 0;
                    c.push(Lit::new(v, positive));
                }
                clauses.push(c);
            }
            let mut s = Solver::new(num_vars);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            // Several assumption queries on this one (clause-accumulating) solver.
            for _ in 0..8 {
                let a_count = (next() % 3) as usize; // 0..2 assumptions (may contradict)
                let mut asm = Vec::new();
                for _ in 0..a_count {
                    let v = (next() % num_vars as u64) as Var;
                    let positive = next() & 1 == 0;
                    asm.push(Lit::new(v, positive));
                }
                let mut full = clauses.clone();
                for &a in &asm {
                    full.push(vec![a]);
                }
                let expected = sat_brute(num_vars, &full);
                match s.solve_under_assumptions(&asm) {
                    SolveResult::Sat(m) => {
                        assert!(expected, "under {asm:?}: solver SAT but brute force UNSAT");
                        assert!(
                            check_model(&full, &m),
                            "under {asm:?}: model violates clauses or assumptions"
                        );
                    }
                    SolveResult::Unsat => {
                        assert!(!expected, "under {asm:?}: solver UNSAT but brute force SAT");
                    }
                }
            }
        }
    }
}
