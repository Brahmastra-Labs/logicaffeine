//! LRAT refutations at scale: hint chains found by watched-literal propagation, trimmed to the clauses a refutation
//! uses, and replayed by a checker that trusts nothing the search did.
//!
//! A CDCL solve leaves its learned clauses in order, each derivable by reverse unit propagation (RUP) from the
//! formula and the clauses before it ([`crate::cdcl::Solver::learned`]). [`crate::rup::check_refutation`] replays
//! such a trace by propagating the whole database to a fixpoint for every clause, a cost a circuit miter of tens of
//! thousands of clauses cannot pay a learned clause. [`refutation`] instead finds each clause's LRAT hint chain
//! (Cruz-Filipe et al., 2017) with two watched literals (Moskewicz et al., *Chaff*, 2001), walking the trace
//! backward from the empty clause, the `drat-trim` order (Wetzler, Heule & Hunt, 2014): a learned clause no later
//! step uses is never checked and never shipped. [`check`] replays a proof in time linear in its hints, and
//! [`Lrat::to_text`] prints the standard LRAT text that [`crate::proof_emit::check_lrat`] and `cake_lpr` read.
//! [`derivation`] trims the same way back from any target clauses of a log, not only the empty one — what one
//! incremental solver derives across many queries — and [`check_derivation`] replays it and finds each target.
//!
//! Ids follow the LRAT convention: the formula's clauses are `1..=n` in order, the `j`-th learned clause is
//! `n + 1 + j`, and the empty clause takes the id after the last learned one. A trimmed proof skips the ids of the
//! clauses it drops.

use std::collections::{HashMap, HashSet};

use crate::cdcl::Lit;
use crate::rup::{clause_state, set_true, ClauseState};

/// One addition of an LRAT refutation: under the clause's negation each hinted clause, in order, is unit, until the
/// last conflicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LratStep {
    pub id: u64,
    pub clause: Vec<Lit>,
    pub hints: Vec<u64>,
}

/// An LRAT refutation of a formula whose clauses carry the ids `1..=n`: additions in increasing id order, ending
/// in the empty clause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lrat {
    pub steps: Vec<LratStep>,
}

impl Lrat {
    /// The proof as LRAT text: `<id> <clause> 0 <hints> 0` a line, literals in DIMACS numbering.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for step in &self.steps {
            out.push_str(&step.id.to_string());
            for &l in &step.clause {
                out.push(' ');
                out.push_str(&dimacs(l).to_string());
            }
            out.push_str(" 0");
            for &h in &step.hints {
                out.push(' ');
                out.push_str(&h.to_string());
            }
            out.push_str(" 0\n");
        }
        out
    }
}

fn dimacs(l: Lit) -> i64 {
    let v = l.var() as i64 + 1;
    if l.is_positive() {
        v
    } else {
        -v
    }
}

/// The trimmed LRAT refutation of `original` that `learned` (each clause RUP from the formula and the clauses before
/// it) carries to the empty clause, or `None` when the empty clause is not RUP from them all or a learned clause the
/// refutation needs is not RUP in its place: the [`derivation`] of the empty clause, after the learned clauses.
pub fn refutation(num_vars: usize, original: &[Vec<Lit>], learned: &[Vec<Lit>]) -> Option<Lrat> {
    let mut log = learned.to_vec();
    log.push(Vec::new());
    derivation(num_vars, original, &log, &[learned.len()])
}

/// The trimmed LRAT derivation of the clauses of `log` that `targets` names (by index) from `original`, every clause of
/// `log` RUP from the formula and the clauses before it: a step for each target and for each log clause a step it
/// leans on needs, in log order, the `j`-th log clause numbered after the formula's. `None` when a target, or a clause
/// one needs, is not RUP in its place, or a target names no log clause.
pub fn derivation(num_vars: usize, original: &[Vec<Lit>], log: &[Vec<Lit>], targets: &[usize]) -> Option<Lrat> {
    let n = original.len();
    if original.iter().chain(log).flatten().any(|l| l.var() as usize >= num_vars) || targets.iter().any(|&t| t >= log.len()) {
        return None;
    }
    let mut db = Propagator::new(num_vars, original.iter().chain(log));
    let mut needed = vec![false; n + log.len()];
    for &t in targets {
        needed[n + t] = true;
    }
    let mut chains: Vec<Option<Vec<u64>>> = vec![None; log.len()];
    for j in (0..log.len()).rev() {
        db.deactivate(n + j);
        if !needed[n + j] {
            continue;
        }
        let chain = db.chain(&log[j])?;
        for &h in &chain {
            needed[(h - 1) as usize] = true;
        }
        chains[j] = Some(chain);
    }
    let steps = chains
        .into_iter()
        .enumerate()
        .filter_map(|(j, chain)| chain.map(|hints| LratStep { id: (n + 1 + j) as u64, clause: log[j].clone(), hints }))
        .collect();
    Some(Lrat { steps })
}

/// Whether `proof` refutes `original`: every step's id exceeds the one before it (and the formula's), every hint
/// names a clause already present, every step's hints replay (each unit under the step's negation, the last a
/// conflict), and a step derives the empty clause. Every literal must lie below `num_vars`.
pub fn check(num_vars: usize, original: &[Vec<Lit>], proof: &Lrat) -> bool {
    let in_range = |c: &[Lit]| c.iter().all(|l| (l.var() as usize) < num_vars);
    if !original.iter().all(|c| in_range(c)) {
        return false;
    }
    let mut db: HashMap<u64, &[Lit]> =
        original.iter().enumerate().map(|(i, c)| (i as u64 + 1, c.as_slice())).collect();
    let mut assign: Vec<Option<bool>> = vec![None; num_vars];
    let mut last = original.len() as u64;
    for step in &proof.steps {
        if step.id <= last || !in_range(&step.clause) {
            return false;
        }
        if !replay(&db, &step.clause, &step.hints, &mut assign) {
            return false;
        }
        if step.clause.is_empty() {
            return true;
        }
        db.insert(step.id, &step.clause);
        last = step.id;
    }
    false
}

/// Whether `proof` derives every clause of `claims` from `original`: every step's id exceeds the one before it (and the
/// formula's), every hint names a clause already present, every step's hints replay, and each claim, its literals in
/// any order, is the clause of a step or of the formula. Every literal must lie below `num_vars`.
pub fn check_derivation(num_vars: usize, original: &[Vec<Lit>], proof: &Lrat, claims: &[Vec<Lit>]) -> bool {
    let in_range = |c: &[Lit]| c.iter().all(|l| (l.var() as usize) < num_vars);
    if !original.iter().all(|c| in_range(c)) {
        return false;
    }
    let mut db: HashMap<u64, &[Lit]> =
        original.iter().enumerate().map(|(i, c)| (i as u64 + 1, c.as_slice())).collect();
    let mut assign: Vec<Option<bool>> = vec![None; num_vars];
    let mut last = original.len() as u64;
    for step in &proof.steps {
        if step.id <= last || !in_range(&step.clause) || !replay(&db, &step.clause, &step.hints, &mut assign) {
            return false;
        }
        db.insert(step.id, &step.clause);
        last = step.id;
    }
    let key = |c: &[Lit]| -> Vec<(u32, bool)> {
        let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive())).collect();
        k.sort_unstable();
        k.dedup();
        k
    };
    let have: HashSet<Vec<(u32, bool)>> = db.values().map(|c| key(c)).collect();
    claims.iter().all(|c| have.contains(&key(c)))
}

/// Replay one step's hints under the negation of `clause`, leaving `assign` cleared.
fn replay(db: &HashMap<u64, &[Lit]>, clause: &[Lit], hints: &[u64], assign: &mut [Option<bool>]) -> bool {
    let mut set: Vec<Lit> = Vec::new();
    let mut verdict = false;
    'replay: {
        for &l in clause {
            let nl = l.negated();
            if !set_true(assign, nl) {
                verdict = true;
                break 'replay;
            }
            set.push(nl);
        }
        for (i, h) in hints.iter().enumerate() {
            let Some(c) = db.get(h) else { break 'replay };
            match clause_state(c, assign) {
                ClauseState::Conflict => {
                    verdict = i + 1 == hints.len();
                    break 'replay;
                }
                ClauseState::Unit(u) => {
                    set_true(assign, u);
                    set.push(u);
                }
                ClauseState::Quiet => break 'replay,
            }
        }
    }
    for l in set {
        assign[l.var() as usize] = None;
    }
    verdict
}

/// What a clause is to the propagator once its repeated literals are merged.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Empty,
    Unit,
    Watched,
    Tautology,
}

/// A clause database for repeated RUP queries: two watched literals a clause, clauses switched off one by one as the
/// backward walk passes them, every query starting from an empty assignment.
struct Propagator {
    /// Each clause with its repeated literals merged; a watched clause keeps its two watches in front.
    clauses: Vec<Vec<Lit>>,
    shape: Vec<Shape>,
    active: Vec<bool>,
    /// Per literal, the clauses watching it: visited when the literal becomes false.
    watches: Vec<Vec<usize>>,
    units: Vec<usize>,
    empties: Vec<usize>,
    value: Vec<i8>,
    /// Per variable, the clause that forced it plus one (0 when assumed or unset).
    reason: Vec<usize>,
    /// Per variable, its place on the trail.
    pos: Vec<usize>,
    trail: Vec<Lit>,
    seen: Vec<bool>,
}

fn widx(l: Lit) -> usize {
    ((l.var() as usize) << 1) | (!l.is_positive()) as usize
}

impl Propagator {
    fn new<'a>(num_vars: usize, clauses: impl Iterator<Item = &'a Vec<Lit>>) -> Propagator {
        let mut p = Propagator {
            clauses: Vec::new(),
            shape: Vec::new(),
            active: Vec::new(),
            watches: vec![Vec::new(); num_vars * 2],
            units: Vec::new(),
            empties: Vec::new(),
            value: vec![0; num_vars],
            reason: vec![0; num_vars],
            pos: vec![0; num_vars],
            trail: Vec::new(),
            seen: vec![false; num_vars],
        };
        for c in clauses {
            let ci = p.clauses.len();
            let mut lits: Vec<Lit> = Vec::with_capacity(c.len());
            for &l in c {
                if !lits.contains(&l) {
                    lits.push(l);
                }
            }
            let shape = if lits.iter().any(|l| lits.contains(&l.negated())) {
                Shape::Tautology
            } else {
                match lits.len() {
                    0 => Shape::Empty,
                    1 => Shape::Unit,
                    _ => Shape::Watched,
                }
            };
            match shape {
                Shape::Empty => p.empties.push(ci),
                Shape::Unit => p.units.push(ci),
                Shape::Watched => {
                    p.watches[widx(lits[0])].push(ci);
                    p.watches[widx(lits[1])].push(ci);
                }
                Shape::Tautology => {}
            }
            p.clauses.push(lits);
            p.shape.push(shape);
            p.active.push(true);
        }
        p
    }

    fn deactivate(&mut self, ci: usize) {
        self.active[ci] = false;
    }

    fn val(&self, l: Lit) -> i8 {
        let v = self.value[l.var() as usize];
        if l.is_positive() {
            v
        } else {
            -v
        }
    }

    fn assign(&mut self, l: Lit, reason: usize) {
        let v = l.var() as usize;
        self.value[v] = if l.is_positive() { 1 } else { -1 };
        self.reason[v] = reason;
        self.pos[v] = self.trail.len();
        self.trail.push(l);
    }

    /// The hint chain that refutes `¬c` over the active clauses, or `None` when propagation stops short of a
    /// conflict.
    fn chain(&mut self, c: &[Lit]) -> Option<Vec<u64>> {
        let result = self.search(c);
        for i in 0..self.trail.len() {
            let v = self.trail[i].var() as usize;
            self.value[v] = 0;
            self.reason[v] = 0;
        }
        self.trail.clear();
        result
    }

    fn search(&mut self, c: &[Lit]) -> Option<Vec<u64>> {
        if let Some(&e) = self.empties.iter().find(|&&e| self.active[e]) {
            return Some(vec![e as u64 + 1]);
        }
        for &l in c {
            let nl = l.negated();
            match self.val(nl) {
                1 => {}
                -1 => return Some(Vec::new()),
                _ => self.assign(nl, 0),
            }
        }
        for k in 0..self.units.len() {
            let ci = self.units[k];
            if !self.active[ci] {
                continue;
            }
            let u = self.clauses[ci][0];
            match self.val(u) {
                1 => {}
                -1 => return Some(self.explain(ci)),
                _ => self.assign(u, ci + 1),
            }
        }
        let mut head = 0;
        while head < self.trail.len() {
            let falsified = self.trail[head].negated();
            head += 1;
            let ws = std::mem::take(&mut self.watches[widx(falsified)]);
            let mut kept: Vec<usize> = Vec::with_capacity(ws.len());
            let mut conflict = None;
            let mut i = 0;
            while i < ws.len() {
                let ci = ws[i];
                i += 1;
                if !self.active[ci] {
                    kept.push(ci);
                    continue;
                }
                if self.clauses[ci][0] == falsified {
                    self.clauses[ci].swap(0, 1);
                }
                let first = self.clauses[ci][0];
                if self.val(first) == 1 {
                    kept.push(ci);
                    continue;
                }
                let replacement = (2..self.clauses[ci].len()).find(|&k| self.val(self.clauses[ci][k]) != -1);
                if let Some(k) = replacement {
                    self.clauses[ci].swap(1, k);
                    let w = widx(self.clauses[ci][1]);
                    self.watches[w].push(ci);
                    continue;
                }
                kept.push(ci);
                if self.val(first) == -1 {
                    conflict = Some(ci);
                    break;
                }
                self.assign(first, ci + 1);
            }
            kept.extend_from_slice(&ws[i..]);
            self.watches[widx(falsified)] = kept;
            if let Some(ci) = conflict {
                return Some(self.explain(ci));
            }
        }
        None
    }

    /// The reasons behind every literal of the conflict clause `ci`, in trail order, then `ci` itself.
    fn explain(&mut self, ci: usize) -> Vec<u64> {
        let mut needed: Vec<(usize, usize)> = Vec::new();
        let mut stack: Vec<usize> = self.clauses[ci].iter().map(|l| l.var() as usize).collect();
        let mut marked: Vec<usize> = Vec::new();
        while let Some(v) = stack.pop() {
            if self.seen[v] {
                continue;
            }
            self.seen[v] = true;
            marked.push(v);
            let r = self.reason[v];
            if r != 0 {
                needed.push((self.pos[v], r - 1));
                stack.extend(self.clauses[r - 1].iter().map(|l| l.var() as usize));
            }
        }
        for v in marked {
            self.seen[v] = false;
        }
        needed.sort_unstable();
        let mut chain: Vec<u64> = needed.into_iter().map(|(_, r)| r as u64 + 1).collect();
        chain.push(ci as u64 + 1);
        chain
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/lrat/tests.rs"]
mod tests;
