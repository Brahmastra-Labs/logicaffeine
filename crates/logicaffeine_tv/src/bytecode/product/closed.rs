//! The proof for a function whose compiled form computes the source's loops in closed form: the target runs no loop,
//! so its whole run is a handful of paths, each a condition on the inputs and the outcome the target gives under it.
//! Each path is proved against the source on its own. The source is cut at its loop heads as in the lockstep proof,
//! but the invariant at a head relates the source's state to the run's inputs, and every obligation assumes the target
//! path's condition, a fact about the inputs that holds for the whole run.
//!
//! The candidates are read off concrete runs of the source (`sample`), filed by the target path each run's inputs
//! take. Houdini keeps the inductive ones under the path's condition, so `i − n₀ ≤ 1` is kept where the target has
//! taken `n ≥ 1` and dropped where it has not; a candidate a sampled run of that path breaks is dropped before Z3 is
//! asked. The invariant is reported without the conjuncts the others imply (the proof keeps them: an explicit bound is
//! one the solver's nonlinear reasoning uses directly). Then every source path that leaves the loops ends as the target
//! path does, having shown what it shows; where the target stops with an integer overflow, the source has shown at
//! least what it showed. A target path that stops before showing anything constrains the source in nothing, so it is
//! not proved, only recorded: the verdict says the target may stop where the source goes on. A goal is asked with a
//! lemma for each division in it that its polynomial always meets exactly (`divides`), and again without them, each
//! first of the incremental engine and then of a fresh solver: a lemma Z3 does not need can slow its search, and one it
//! needs (`n·(n+1)` is even) it does not find. A wrap the path's guard rules out is replaced by the exact value first.
//!
//! The target always ends, so the source must too: at each head some quantity the invariant bounds from above
//! (`i − n₀ ≤ 1`) rises by at least one on every iteration, so the loop is left on every run. Loops that reach each
//! other would need an argument across both and are refused.
//!
//! A value shown before a head is matched against the target's output where it falls. A head reached having shown
//! different numbers of values, as a loop that shows a value is, cannot be matched against an output the target
//! computes all at once, and is refused.

use z3::ast::{Ast, Bool, Dynamic};
use z3::{SatResult, Solver};

use super::divides::lemmas;
use super::sample::{prune, render, Fact, Samples};
use super::{budget, decide, decide_fresh, decide_within, describe_end, discharge_wraps, Decided, Finding, Profile, Prover, Side, HOUDINI_RLIMIT, INCREMENTAL_RLIMIT, QUERY_RLIMIT};
use crate::bytecode::path::{End, Path, State};
use crate::bytecode::value::{same, SymVal};
use crate::symexec::Unsupported;

impl<S: Side, T: Side> Prover<'_, S, T> {
    /// The proof when the target runs no loop and the source does: each of the target's paths from its entry against
    /// the source, under an invariant per source head over the head's state and the inputs.
    pub(super) fn prove_closed_form(
        &self,
        params: &[SymVal],
        assumed: Vec<String>,
        entry: &[Path<S::Loc>],
        runs: &[Path<T::Loc>],
        profiles: &[Option<Profile<S::Loc>>],
    ) -> Result<Finding, Unsupported> {
        let ns = profiles.len();
        let mut heads: Vec<Option<State<S::Loc>>> = vec![None; ns];
        let mut from: Vec<Vec<Path<S::Loc>>> = vec![Vec::new(); ns];
        for k in 0..ns {
            if let Some(p) = &profiles[k] {
                let h = p.havoc(|l| format!("s.loop{}.{}", k + 1, self.source.describe(l)))?;
                from[k] = self.source.explore(Some(k), h.clone())?;
                heads[k] = Some(h);
            }
        }
        let shown = self.shown_at_heads(entry, &from)?;
        self.acyclic(&from)?;

        let loc = |l: &S::Loc| self.source.describe(l);
        let regions: Vec<Bool> = runs.iter().map(|q| q.cond.clone()).collect();
        let samples = Samples::gather(self.source, params, profiles, &regions);
        let facts: Vec<Vec<Fact<S::Loc>>> = (0..ns).map(|k| samples.facts(k, &loc)).collect();
        let inputs = samples.input_names(params);

        let solver = Solver::new();
        solver.set_params(&budget(INCREMENTAL_RLIMIT));
        solver.assert(&self.source.assumes(params));
        solver.assert(&self.target.assumes(params));
        let blank = State { env: Default::default(), depth: 0 };
        let mut segments: Vec<(Option<usize>, &[Path<S::Loc>])> = vec![(None, entry)];
        segments.extend((0..ns).filter(|&k| heads[k].is_some()).map(|k| (Some(k), from[k].as_slice())));

        let mut invariants = Vec::new();
        let mut overflow = false;
        for (qi, q) in runs.iter().enumerate() {
            if matches!(q.end, End::Overflow) && q.out.is_empty() {
                overflow = true;
                continue;
            }
            solver.push();
            solver.assert(&q.cond);
            if solver.check() == SatResult::Unsat {
                solver.pop(1);
                continue;
            }
            let mut cands: Vec<Vec<Fact<S::Loc>>> = (0..ns)
                .map(|k| facts[k].iter().filter(|f| !samples.breaks(k, qi, f)).cloned().collect())
                .collect();
            let inv = |at: Option<usize>, cands: &[Vec<Fact<S::Loc>>]| -> Bool {
                match at {
                    None => Bool::from_bool(true),
                    Some(k) => match &heads[k] {
                        Some(h) => Bool::and(&cands[k].iter().map(|f| f.holds(h, params)).collect::<Vec<_>>()),
                        None => Bool::from_bool(false),
                    },
                }
            };
            loop {
                let mut dropped = false;
                for (at, paths) in &segments {
                    let assume = inv(*at, &cands);
                    for p in paths.iter() {
                        let End::Cut(k) = p.end else { continue };
                        solver.push();
                        solver.assert(&assume);
                        solver.assert(&p.cond);
                        if decide(&solver, &[]).answer != SatResult::Unsat {
                            let before = cands[k].len();
                            cands[k].retain(|f| {
                                decide_within(&solver, &[f.holds(&p.state, params).not()], HOUDINI_RLIMIT).answer
                                    == SatResult::Unsat
                            });
                            dropped |= cands[k].len() != before;
                        }
                        solver.pop(1);
                    }
                }
                if !dropped {
                    break;
                }
            }

            if let Some(k) = (0..ns).find(|&k| heads[k].is_some() && !self.leaves(&solver, k, &heads, &from, &cands, params)) {
                solver.pop(1);
                return Ok(Finding::Failed {
                    at: format!(
                        "the head of loop {}: no quantity the invariant bounds grows on every iteration, so the source \
                         may never leave the loop where {} returns",
                        k + 1,
                        self.what
                    ),
                    state: String::new(),
                    values: Vec::new(),
                });
            }
            let case = format!(
                "where {} takes its path {} of {}, {}",
                self.what,
                qi + 1,
                runs.len(),
                render(&Dynamic::from_ast(&q.cond.simplify()), &inputs)
            );
            for (at, paths) in &segments {
                let assume = inv(*at, &cands);
                let (base, start, place) = match at {
                    None => (0, None, "from the entry".to_string()),
                    Some(k) => (
                        shown[*k].unwrap_or(0),
                        heads[*k].clone().map(|h| (h, blank.clone())),
                        format!("from the head of loop {}", k + 1),
                    ),
                };
                let place = format!("{place}, {case}");
                for p in paths.iter() {
                    solver.push();
                    solver.assert(&assume);
                    solver.assert(&p.cond);
                    let verdict = self.closed_obligations(&solver, p, q, base, &place, start.as_ref(), params, &mut overflow);
                    solver.pop(1);
                    if let Some(v) = verdict {
                        solver.pop(1);
                        return Ok(v);
                    }
                }
            }
            for k in 0..ns {
                if let Some(h) = &heads[k] {
                    let mut conj = vec![case.clone()];
                    let shown = prune(&solver, cands[k].clone(), h, params);
                    conj.extend(shown.iter().map(|f| f.describe(&|v| samples.name(v, &loc), &loc)));
                    invariants.push(conj);
                }
            }
            solver.pop(1);
        }
        Ok(Finding::Proven { invariants, assumed, overflow })
    }

    /// The source leaves loop `k` on every run: some quantity the invariant at `k` bounds from above rises by at least
    /// one on every path from the head back to it. (Heads that reach each other are refused first, in
    /// [`Self::acyclic`], so a path back to the head is one iteration.)
    #[allow(clippy::too_many_arguments)]
    fn leaves(
        &self,
        solver: &Solver,
        k: usize,
        heads: &[Option<State<S::Loc>>],
        from: &[Vec<Path<S::Loc>>],
        cands: &[Vec<Fact<S::Loc>>],
        params: &[SymVal],
    ) -> bool {
        let Some(h) = &heads[k] else { return true };
        let back: Vec<&Path<S::Loc>> = from[k].iter().filter(|p| matches!(p.end, End::Cut(j) if j == k)).collect();
        let inv = Bool::and(&cands[k].iter().map(|f| f.holds(h, params)).collect::<Vec<_>>());
        cands[k].iter().any(|f| {
            let Some(before) = f.capped(h, params) else { return false };
            back.iter().all(|p| {
                let Some(after) = f.capped(&p.state, params) else { return false };
                let rises = after.ge(&before + 1i64);
                decide_within(solver, &[inv.clone(), p.cond.clone(), rises.not()], HOUDINI_RLIMIT).answer == SatResult::Unsat
            })
        })
    }

    /// No two heads reach each other: every cycle in the source is a single loop, which [`Self::leaves`] can bound.
    fn acyclic(&self, from: &[Vec<Path<S::Loc>>]) -> Result<(), Unsupported> {
        let next = |k: usize| from[k].iter().filter_map(move |p| match p.end {
            End::Cut(j) if j != k => Some(j),
            _ => None,
        });
        for start in 0..from.len() {
            let mut seen = vec![false; from.len()];
            let mut work: Vec<usize> = next(start).collect();
            while let Some(k) = work.pop() {
                if k == start {
                    return Err(Unsupported(format!(
                        "loop {} reaches itself through another loop, and {} computes them in closed form: their \
                         ending needs an argument across both",
                        start + 1,
                        self.what
                    )));
                }
                if !std::mem::replace(&mut seen[k], true) {
                    work.extend(next(k));
                }
            }
        }
        Ok(())
    }

    /// How many values the source has shown when it reaches each head, the same on every path there.
    fn shown_at_heads(&self, entry: &[Path<S::Loc>], from: &[Vec<Path<S::Loc>>]) -> Result<Vec<Option<usize>>, Unsupported> {
        let mut shown: Vec<Option<usize>> = vec![None; from.len()];
        let settle = |k: usize, n: usize, shown: &mut Vec<Option<usize>>| -> Result<bool, Unsupported> {
            match shown[k] {
                None => {
                    shown[k] = Some(n);
                    Ok(true)
                }
                Some(m) if m == n => Ok(false),
                Some(_) => Err(Unsupported(format!(
                    "the head of loop {} is reached having shown different numbers of values, and {} computes the \
                     loop in closed form, so what it shows cannot be matched value by value",
                    k + 1,
                    self.what
                ))),
            }
        };
        for p in entry {
            if let End::Cut(k) = p.end {
                settle(k, p.out.len(), &mut shown)?;
            }
        }
        loop {
            let mut changed = false;
            for (k, paths) in from.iter().enumerate() {
                let Some(base) = shown[k] else { continue };
                for p in paths {
                    if let End::Cut(k2) = p.end {
                        changed |= settle(k2, base + p.out.len(), &mut shown)?;
                    }
                }
            }
            if !changed {
                return Ok(shown);
            }
        }
    }

    /// One source path against the target's run `q`, under the assertions on the solver: `base` values were shown
    /// before the path began. `None` when every obligation holds.
    #[allow(clippy::too_many_arguments)]
    fn closed_obligations(
        &self,
        solver: &Solver,
        p: &Path<S::Loc>,
        q: &Path<T::Loc>,
        base: usize,
        at: &str,
        start: Option<&(State<S::Loc>, State<T::Loc>)>,
        params: &[SymVal],
        overflow: &mut bool,
    ) -> Option<Finding> {
        let feasible = decide(solver, &[]);
        match feasible.answer {
            SatResult::Unsat => return None,
            SatResult::Unknown => return Some(self.unknown(feasible.by(solver), at)),
            SatResult::Sat => {}
        }
        let refute = |what: String, by: &Solver| {
            let (state, values) = self.witness(by, start, params);
            Finding::Failed { at: format!("{at}: {what}"), state, values }
        };
        let stopped = matches!(q.end, End::Overflow);
        let ends = !matches!(p.end, End::Cut(_));
        let upto = base + p.out.len();
        if ends && !stopped {
            let alike = matches!(
                (&p.end, &q.end),
                (End::Return(_), End::Return(_)) | (End::Raise, End::Raise) | (End::Halt, End::Halt)
            );
            if !alike {
                return Some(refute(
                    format!("the source {} where {} {}", describe_end(&p.end), self.what, describe_end(&q.end)),
                    feasible.by(solver),
                ));
            }
        }
        if (!stopped && upto > q.out.len()) || (ends && !stopped && upto != q.out.len()) || (ends && upto < q.out.len()) {
            return Some(refute(
                format!("the source shows {upto} value(s) by here where {} shows {}", self.what, q.out.len()),
                feasible.by(solver),
            ));
        }
        let mut goals: Vec<(String, Bool)> = p
            .out
            .iter()
            .enumerate()
            .filter(|(i, _)| base + i < q.out.len())
            .map(|(i, v)| (format!("shown value {} differs", base + i + 1), same(v, &q.out[base + i])))
            .collect();
        if let (End::Return(a), End::Return(b)) = (&p.end, &q.end) {
            goals.push(("the returned value differs".into(), same(a, b)));
        }
        let goals: Vec<(String, Bool)> = goals.into_iter().map(|(what, g)| (what, discharge_wraps(solver, &g))).collect();
        let given = lemmas(&goals.iter().map(|(_, g)| Dynamic::from_ast(g)).collect::<Vec<_>>());
        for (what, goal) in goals {
            let with: Vec<Bool> = given.iter().cloned().chain([goal.not()]).collect();
            let without = [goal.not()];
            let mut d = Decided { answer: solver.check_assumptions(&with), fresh: None };
            if d.answer == SatResult::Unknown && !given.is_empty() {
                d = Decided { answer: solver.check_assumptions(&without), fresh: None };
            }
            if d.answer == SatResult::Unknown {
                d = decide_fresh(solver, &with, QUERY_RLIMIT);
            }
            if d.answer == SatResult::Unknown && !given.is_empty() {
                d = decide_fresh(solver, &without, QUERY_RLIMIT);
            }
            match d.answer {
                SatResult::Unsat => {}
                SatResult::Sat => return Some(refute(what, d.by(solver))),
                SatResult::Unknown => return Some(self.unknown(d.by(solver), at)),
            }
        }
        if stopped && ends {
            *overflow = true;
        }
        None
    }
}
