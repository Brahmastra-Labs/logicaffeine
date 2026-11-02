//! LAZY δ IN THE INDEPENDENT KERNEL — Lean's `lazy_delta_reduction`, written for this kernel's own
//! de Bruijn terms and its own reduction steps. It shares no code with the primary kernel's
//! `lazy_delta` module, so the two conversions stay two independent witnesses.
//!
//! Lean's kernel never weak-head-normalizes both sides fully before comparing them. It reduces each
//! side WITHOUT unfolding its head constant (`whnf_core`), compares, and otherwise unfolds ONE side
//! by one δ step — the side whose head was defined later (the greater definitional height), both on
//! a tie, after first trying argument congruence when both heads are the same constant. Proofs are
//! compared by irrelevance before a theorem is unfolded, and theorems unfold last (the 2026-09-23
//! ruling).
//!
//! The head reduction is this kernel's own weak-head machine with δ withheld at the head
//! ([`whnf_core_in`]), so every rule the machine applies before δ — the stuck-head rules, the
//! certified `Nat` order and arithmetic, the equality recurrence — its depth door and its size
//! check hold here exactly as in `whnf`. The loop runs on a fuel scope of its own.
//!
//! What it may answer — phase 1: it can only fail to prove, never invent an equality, and it never
//! refuses a pair the rest of this kernel's conversion would accept:
//! * `Decided(true)` — identity, two equal `Nat` literals, argument congruence under a common head,
//!   and proof irrelevance (asked for a common constructor of a proposition, and before a theorem
//!   is unfolded). Every premise is itself a `def_eq`, and every rewrite of a side is a reduction
//!   step of this kernel's own machine.
//! * `Decided(false)` — ONLY two distinct `Nat` literals, and two applications of the SAME
//!   constructor or inductive whose arguments differ — the complete rule for rigid heads — and never
//!   when any reduction under the loop was cut short by its fuel.
//! * `Open` — undecided: the caller's reduction-based conversion decides the ORIGINAL pair, on the
//!   fuel it would have had, so its verdict is exactly the one it gave before this loop existed.
use super::*;

/// δ steps all the lazy comparisons under one outermost comparison may take.
const STEPS: usize = 1 << 12;
/// Declarations one height computation may visit before it reports "unknown".
const HEIGHT_VISITS: usize = 4096;

fn apply_all(head: Db, args: Vec<Db>) -> Db {
    args.into_iter()
        .fold(head, |f, a| Db::App(DbBox::new(f), DbBox::new(a)))
}

/// An application spine taken apart by ownership: its head and its arguments in source order.
fn unspine(mut t: Db) -> (Db, Vec<Db>) {
    let mut args = Vec::new();
    while let Db::App(f, a) = t {
        args.push(a.into_db());
        t = f.into_db();
    }
    args.reverse();
    (t, args)
}

/// The head constant δ may unfold, and whether it is a checked theorem (unfolded last).
fn delta_head(genv: &Context, t: &Db) -> Option<(String, bool)> {
    let name = match spine_head(t) {
        Db::Global(name) if genv.get_definition_body(name).is_some() => name,
        Db::Const { name, .. }
            if genv.get_universe_poly(name).is_some()
                || genv.get_definition_body(name).is_some() =>
        {
            name
        }
        _ => return None,
    };
    Some((
        name.clone(),
        crate::type_checker::is_transparent_theorem(genv, name),
    ))
}

/// One δ step at the head, then head reduction without δ. `None` when the head has no body this
/// kernel can lower.
fn unfold(genv: &Context, lctx: &dyn LocalTypes, t: Db) -> Option<Db> {
    let body = match spine_head(&t) {
        Db::Global(name) => definition_body(genv, name),
        Db::Const { name, levels } => const_body(genv, name, levels),
        _ => None,
    }?;
    let (_, args) = unspine(t);
    Some(whnf_core_in(genv, lctx, apply_all(body, args)))
}

thread_local! {
    /// δ steps left to the lazy comparisons under the outermost one; `None` outside any.
    static ALLOWANCE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// How many outermost lazy scopes opened on this thread.
    static SCOPES_OPENED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Take one δ step from the shared allowance.
fn spend() -> bool {
    ALLOWANCE.with(|a| match a.get() {
        Some(left) if left > 0 => {
            a.set(Some(left - 1));
            true
        }
        _ => false,
    })
}

/// Weak-head reduction fuel for all the lazy comparisons under one outermost comparison.
const FUEL: usize = 1 << 16;

/// The outermost lazy comparison's scope. It opens a δ allowance and a fuel budget of its own
/// (the machine's fuel is then shared by every reduction under it, nested conversions included),
/// and on exit restores the caller's fuel and exhaustion record untouched — the loop's speculation
/// is never charged to, and never recorded in, the check it serves. Nested comparisons share it.
///
/// It opens only where no other budget is running: a conversion inside a reduction, inside a
/// speculative probe or inside a local-context allowance keeps that budget's discipline, and the
/// loop does not run there (`None`) — it could otherwise do more work than the enclosing budget
/// allows.
struct Scope(Option<(usize, bool)>);
impl Scope {
    fn enter() -> Option<Self> {
        if ALLOWANCE.with(|a| a.get().is_some()) {
            return Some(Scope(None));
        }
        if recheck_budget_active() {
            return None;
        }
        ALLOWANCE.with(|a| a.set(Some(STEPS)));
        #[cfg(test)]
        SCOPES_OPENED.with(|n| n.set(n.get() + 1));
        let fuel = WHNF_FUEL.with(|f| f.replace(FUEL));
        let exhausted = WHNF_EXHAUSTED.with(|e| e.replace(false));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|d| d.set(d.get() + 1));
        Some(Scope(Some((fuel, exhausted))))
    }
    /// Whether any reduction under the outermost comparison was cut short.
    fn cut_short(&self) -> bool {
        WHNF_EXHAUSTED.with(|e| e.get())
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let Some((fuel, exhausted)) = self.0 else {
            return;
        };
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|d| d.set(d.get() - 1));
        WHNF_EXHAUSTED.with(|e| e.set(exhausted));
        WHNF_FUEL.with(|f| f.set(fuel));
        ALLOWANCE.with(|a| a.set(None));
    }
}

thread_local! {
    /// Definitional heights computed by THIS kernel, for one context within one check scope.
    static HEIGHTS: std::cell::RefCell<(usize, usize, HashMap<String, u32>)> =
        std::cell::RefCell::new((0, 0, HashMap::new()));
}

/// The body a constant unfolds to, as the context holds it.
fn body_of<'c>(genv: &'c Context, name: &str) -> Option<&'c Term> {
    genv.get_universe_poly(name)
        .map(|(_, _, body)| body)
        .or_else(|| genv.get_definition_body(name))
}

/// Every constant a term mentions, walked iteratively.
fn mentions(t: &Term, out: &mut Vec<String>) {
    let mut pending = vec![t];
    while let Some(cur) = pending.pop() {
        match cur {
            Term::Global(n) | Term::Const { name: n, .. } => out.push(n.clone()),
            Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
            Term::App(f, a) => {
                pending.push(f);
                pending.push(a);
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                pending.push(param_type);
                pending.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                pending.push(param_type);
                pending.push(body);
            }
            Term::Let {
                ty, value, body, ..
            } => {
                pending.push(ty);
                pending.push(value);
                pending.push(body);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                pending.push(discriminant);
                pending.push(motive);
                pending.extend(cases.iter());
            }
            Term::Fix { body, .. } => pending.push(body),
            Term::MutualFix { defs, .. } => pending.extend(defs.iter().map(|(_, d)| d)),
            Term::Annotated { term, .. } => pending.push(term),
        }
    }
}

/// The definitional height of `name`: 0 without a body, else one more than the greatest height of
/// the constants its body mentions — this kernel's own computation over the context's bodies.
/// `None` when the walk would exceed its visit allowance; the caller then unfolds both sides.
/// A height only orders the unfolding: it can change how much work a comparison does, never what
/// it answers.
fn height(genv: &Context, name: &str) -> Option<u32> {
    let scope = PREFIX_MEMORY.with(|m| m.get().scope);
    let identity = genv as *const Context as usize;
    HEIGHTS.with(|cell| {
        let mut cell = cell.borrow_mut();
        if scope == 0 || cell.0 != identity || cell.1 != scope {
            *cell = (identity, scope, HashMap::new());
        }
        let memo = &mut cell.2;
        if let Some(h) = memo.get(name) {
            return Some(*h);
        }
        let mut visits = HEIGHT_VISITS;
        let mut pending: Vec<(String, bool)> = vec![(name.to_string(), false)];
        while let Some((current, expanded)) = pending.pop() {
            if memo.contains_key(&current) {
                continue;
            }
            let Some(body) = body_of(genv, &current) else {
                memo.insert(current, 0);
                continue;
            };
            let mut deps = Vec::new();
            mentions(body, &mut deps);
            if expanded {
                let h = 1 + deps
                    .iter()
                    .filter_map(|d| memo.get(d))
                    .copied()
                    .max()
                    .unwrap_or(0);
                memo.insert(current, h);
                continue;
            }
            pending.push((current.clone(), true));
            for dep in deps {
                if dep != current && !memo.contains_key(&dep) {
                    visits = visits.checked_sub(1)?;
                    pending.push((dep, false));
                }
            }
        }
        memo.get(name).copied()
    })
}

/// Which sides to unfold: theorems last, then the greater height, both on a tie or when unknown.
fn unfold_order(genv: &Context, left: &(String, bool), right: &(String, bool)) -> (bool, bool) {
    match (left.1, right.1) {
        (false, true) => (true, false),
        (true, false) => (false, true),
        _ => match (height(genv, &left.0), height(genv, &right.0)) {
            (Some(l), Some(r)) if l > r => (true, false),
            (Some(l), Some(r)) if l < r => (false, true),
            _ => (true, true),
        },
    }
}

/// `h a₁ … aₙ ≡ h b₁ … bₙ` for one common head when every `aᵢ ≡ bᵢ` — positive only. `None`: not
/// two applications of one head, so no argument was compared.
fn same_head_arguments(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> Option<bool> {
    let (ah, aa) = spine_ref(a);
    let (bh, ba) = spine_ref(b);
    if aa.is_empty() || aa.len() != ba.len() || ah != bh {
        return None;
    }
    Some(aa.iter().zip(&ba).all(|(x, y)| def_eq(genv, lctx, x, y)))
}

/// Two sides neither of which δ can unfold, with one common constructor or inductive head: the
/// complete rule for rigid heads. `None` for any other pair — the caller's to decide.
fn rigid_pair(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> Option<bool> {
    let (ah, aa) = spine_ref(a);
    let (bh, ba) = spine_ref(b);
    if ah != bh || aa.len() != ba.len() || aa.is_empty() {
        return None;
    }
    let (Db::Global(name) | Db::Const { name, .. }) = ah else {
        return None;
    };
    let constructor = genv.is_constructor(name);
    if !constructor && !genv.is_inductive(name) {
        return None;
    }
    if constructor && proof_irrel(genv, lctx, a, b) {
        return Some(true);
    }
    Some(aa.iter().zip(&ba).all(|(x, y)| def_eq(genv, lctx, x, y)))
}

/// What the lazy loop concluded about a pair.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Lazy {
    Decided(bool),
    /// Undecided: the caller's conversion decides the original pair.
    Open,
}

/// Lean's lazy δ conversion in this kernel. See the module documentation for what each answer means.
pub(super) fn conversion(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> Lazy {
    if !genv.simp_rules().is_empty() || WHNF_EXHAUSTED.with(|e| e.get()) || oracle_running() {
        return Lazy::Open;
    }
    let Some(scope) = Scope::enter() else {
        return Lazy::Open;
    };
    // The loop reduces its own copies of the two sides; they are WHNF input copies like any other.
    let answer = lazy(genv, lctx, clone_whnf_input(a), clone_whnf_input(b));
    // A refutation reached while a reduction under the loop was cut short compared a partial term:
    // it decided nothing. The caller decides the pair instead, on its own fuel.
    match answer {
        Lazy::Decided(false) if scope.cut_short() => Lazy::Open,
        answer => answer,
    }
}

fn lazy(genv: &Context, lctx: &dyn LocalTypes, a: Db, b: Db) -> Lazy {
    let mut a = whnf_core_in(genv, lctx, a);
    let mut b = whnf_core_in(genv, lctx, b);
    let mut irrelevance_tried = false;
    loop {
        if WHNF_EXHAUSTED.with(|e| e.get()) {
            return Lazy::Open;
        }
        if a == b {
            return Lazy::Decided(true);
        }
        if let (Db::Lit(Literal::Nat(x)), Db::Lit(Literal::Nat(y))) = (&a, &b) {
            return Lazy::Decided(x == y);
        }
        let (unfold_a, unfold_b) = match (delta_head(genv, &a), delta_head(genv, &b)) {
            (None, None) => {
                return match rigid_pair(genv, lctx, &a, &b) {
                    Some(answer) => Lazy::Decided(answer),
                    None => Lazy::Open,
                }
            }
            (Some(_), None) => (true, false),
            (None, Some(_)) => (false, true),
            (Some(left), Some(right)) => {
                // Lean's `failed_before`: arguments refuted once in this check are not compared
                // again, or every level of a nested comparison would descend twice.
                if left.0 == right.0
                    && !super::conversion_memo::arguments_refuted(genv, lctx, &a, &b)
                {
                    match same_head_arguments(genv, lctx, &a, &b) {
                        Some(true) => return Lazy::Decided(true),
                        Some(false) if !WHNF_EXHAUSTED.with(|e| e.get()) => {
                            super::conversion_memo::record_refuted_arguments(genv, lctx, &a, &b);
                        }
                        _ => {}
                    }
                }
                unfold_order(genv, &left, &right)
            }
        };
        // Lean compares proofs by irrelevance before any δ; here that is asked the first time a
        // THEOREM is about to be unfolded — the one unfolding it exists to avoid.
        let theorem_next = (unfold_a && delta_head(genv, &a).is_some_and(|(_, theorem)| theorem))
            || (unfold_b && delta_head(genv, &b).is_some_and(|(_, theorem)| theorem));
        if theorem_next && !irrelevance_tried {
            irrelevance_tried = true;
            if proof_irrel(genv, lctx, &a, &b) {
                return Lazy::Decided(true);
            }
        }
        if !spend() {
            return Lazy::Open;
        }
        if unfold_a {
            let Some(next) = unfold(genv, lctx, a) else {
                return Lazy::Open;
            };
            a = next;
        }
        if unfold_b {
            let Some(next) = unfold(genv, lctx, b) else {
                return Lazy::Open;
            };
            b = next;
        }
    }
}

thread_local! {
    /// The differential's oracle is running: every conversion under it is the reduction-based one.
    static ORACLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn oracle_running() -> bool {
    ORACLE.with(|o| o.get())
}

/// With `LOGOS_RECHECK_LAZY_DELTA_DIFFERENTIAL` set — or a test's own switch — a debug build
/// re-decides every pair the lazy loop answered with the reduction-based conversion, and aborts on
/// any pair that conversion DECIDES differently (one cut short by its budget decided nothing and is
/// not compared). The oracle is exactly the conversion this kernel had before the loop: no lazy
/// loop runs anywhere under it, so it neither recurses into further differentials nor measures the
/// loop against itself. It runs on its own fuel and exhaustion record, and is invisible to the
/// loop's own counters, so it cannot change the verdict or the measurements of the check it
/// observes.
#[cfg(debug_assertions)]
pub(super) fn differential(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db, answer: bool) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let enabled = *ENABLED
        .get_or_init(|| std::env::var_os("LOGOS_RECHECK_LAZY_DELTA_DIFFERENTIAL").is_some());
    if !enabled && !DIFFERENTIAL.with(|d| d.get()) {
        return;
    }
    struct Restore {
        fuel: usize,
        exhausted: bool,
        oracle: bool,
        #[cfg(test)]
        conversions: usize,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            WHNF_FUEL.with(|f| f.set(self.fuel));
            WHNF_EXHAUSTED.with(|e| e.set(self.exhausted));
            ORACLE.with(|o| o.set(self.oracle));
            #[cfg(test)]
            WHNF_CONVERSIONS.with(|n| n.set(self.conversions));
        }
    }
    let restore = Restore {
        fuel: WHNF_FUEL.with(|f| f.get()),
        exhausted: WHNF_EXHAUSTED.with(|e| e.replace(false)),
        oracle: ORACLE.with(|o| o.replace(true)),
        #[cfg(test)]
        conversions: WHNF_CONVERSIONS.with(|n| n.get()),
    };
    let fallback = def_eq_by_whnf(genv, lctx, a, b);
    let decided = !WHNF_EXHAUSTED.with(|e| e.get());
    drop(restore);
    if decided {
        DIFFERENTIAL_COMPARISONS.with(|n| n.set(n.get() + 1));
        assert_eq!(answer, fallback,
            "RECHECK-LAZY-DELTA-DISAGREEMENT: lazy δ answered {answer}, the reduction-based conversion {fallback}");
    }
}

#[cfg(debug_assertions)]
thread_local! {
    /// A test's own switch for [`differential`], independent of the environment.
    pub(super) static DIFFERENTIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// How many pairs the differential compared on this thread.
    pub(super) static DIFFERENTIAL_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/lazy_delta/controls.rs"]
mod controls;
