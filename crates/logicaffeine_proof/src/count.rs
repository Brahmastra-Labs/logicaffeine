//! Exact model counting (#SAT): the number of satisfying assignments of a CNF, by DPLL with unit propagation,
//! decomposition into connected components, a component cache and clause learning — the scheme of Bacchus, Dalmao &
//! Pitassi's *Algorithms and complexity results for #SAT and Bayesian inference* (FOCS 2003), of Sang, Bacchus, Beame,
//! Kautz & Pitassi's Cachet (*Combining component caching and clause learning for effective model counting*, SAT 2004)
//! and of Thurley's sharpSAT (SAT 2006).
//!
//! The formula is first preprocessed by the simplifications that keep its count ([`crate::preprocess`] with
//! [`Options::defined_only`]): level-0 propagation, equivalent-literal substitution, congruence closure, subsumption,
//! and elimination of the variables the others define, a gate's output or a variable a local check shows decided. An
//! elimination is taken only where it makes no clause, every resolvent a tautology: a resolvent would join its
//! variable's neighbours in the primal graph and widen the decomposition the search branches along, where one that
//! makes none only takes edges away. On costs renewed as each elimination goes ([`Options::renew_costs`]) a circuit's
//! gates go from its outputs in, each once its last reader is gone, and a circuit with no constraint leaves its inputs
//! alone, free. The variables preprocessing settles leave the search, their values following from the rest's.
//!
//! The formula stays put: an assignment is a trail, each clause of the formula keeps how many of its literals are true
//! and how many false, and propagation walks the occurrence lists of what it assigns, so a branch costs what it
//! assigns, and taking it back costs the same. A component is a set of open variables and the formula's clauses still
//! open among them, found by a search over those clauses from a variable; a clause still open has every literal
//! outside its component false, so the component's variables and its clauses' numbers name its residual formula
//! exactly, and that pair is its key in the cache. A component missing from the cache is counted by branching on its
//! variable first in a min-degree elimination and then in the most open clauses: each branch propagated, the
//! components it leaves counted and multiplied, a variable no open clause holds doubling the count, the two branches
//! summed. Counts are exact integers of any size.
//!
//! A branch that propagates to a falsified clause teaches a clause, by resolution back to the first unique implication
//! point of its level (Marques-Silva & Sakallah's GRASP). The formula implies every taught clause, so a taught clause
//! propagates, watched by two of its literals, but makes no component and enters no key; once the first branch of a
//! component is taken back, each clause taught since that is unit is asserted for the second, which is no branch at all
//! when they imply it. Propagating what the whole formula implies inside one component is sound only while the rest of
//! the residual formula has a model. When a component a split leaves has none, the counts the cache took in since the
//! split began may rest on that, and the cache withdraws them, as Cachet does, whenever a taught clause acted meanwhile.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use logicaffeine_base::BigInt;

use crate::cdcl::{Lit, Var};
use crate::preprocess::{preprocess, Options};

/// A model count and the search it took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Count {
    /// The number of assignments of every variable that satisfy every clause.
    pub models: BigInt,
    /// Branches taken.
    pub decisions: u64,
    /// Components counted (each counted once, the rest answered by the cache).
    pub components: u64,
    /// Components answered by the cache.
    pub cache_hits: u64,
    /// Conflicts met: each teaches a clause, but for one with no branch on the trail, which leaves the formula no
    /// model.
    pub conflicts: u64,
    /// Variables preprocessing settled, fixed or decided by the others, and left out of the search.
    pub settled: u64,
}

/// The preprocessing before a count: the simplifications that keep it, a defined variable eliminated only where that
/// makes no clause, on renewed costs.
fn preprocessing() -> Options {
    Options { defined_only: true, renew_costs: true, clause_limit: 0, ..Options::default() }
}

/// Count the models of `clauses` over `num_vars` variables (a variable no clause mentions is free): preprocessed first
/// by the simplifications that keep the count ([`preprocess`], a defined variable eliminated only where that makes no
/// clause), the variables it settles left out of the search, whose values the others' decide.
pub fn count_models(num_vars: usize, clauses: &[Vec<Lit>]) -> Count {
    let normal: Vec<Vec<Lit>> = clauses.iter().filter_map(|c| normalized(c)).collect();
    let pre = preprocess(num_vars, &normal, &preprocessing());
    let settled = pre.settled.iter().filter(|&&s| s).count() as u64;
    if pre.unsat {
        return Count { models: BigInt::zero(), decisions: 0, components: 0, cache_hits: 0, conflicts: 0, settled };
    }
    let mut renamed = vec![u32::MAX; num_vars];
    let mut kept = 0;
    for v in 0..num_vars {
        if !pre.settled[v] {
            renamed[v] = kept;
            kept += 1;
        }
    }
    let simplified: Vec<Vec<Lit>> = pre
        .clauses
        .iter()
        .map(|c| c.iter().map(|l| Lit::new(renamed[l.var() as usize], l.is_positive())).collect())
        .collect();
    // The recursion is as deep as the branches on one path; a dedicated thread gives it room on any input.
    let worker = std::thread::Builder::new().stack_size(1 << 30).spawn(move || {
        let mut counter = Counter::new(kept as usize, simplified);
        let models = counter.run();
        Count {
            models,
            decisions: counter.decisions,
            components: counter.components,
            cache_hits: counter.cache_hits,
            conflicts: counter.conflicts,
            settled,
        }
    });
    worker.expect("the counting thread starts").join().expect("the counting thread finishes")
}

/// Sort and deduplicate `c`; `None` when it holds a literal and its negation.
fn normalized(c: &[Lit]) -> Option<Vec<Lit>> {
    let mut c = c.to_vec();
    c.sort_unstable_by_key(|l| l.index());
    c.dedup();
    if c.windows(2).any(|w| w[0].var() == w[1].var()) {
        return None;
    }
    Some(c)
}

/// The fill-in edges an elimination may add before it is given up.
const FILL: usize = 20_000_000;

/// Each variable's place in a min-degree elimination of the formula's primal graph (two variables adjacent when a
/// clause holds both), the first eliminated at 0: the variables eliminated last are the separators of the tree
/// decomposition the elimination makes, so branching on them first splits the formula into the components below them.
/// `None` when the graph and its fill-in pass `fill` edges.
fn elimination(num_vars: usize, clauses: &[Vec<Lit>], fill: usize) -> Option<Vec<u32>> {
    let mut adj: Vec<HashSet<u32>> = vec![HashSet::new(); num_vars];
    let mut edges = 0usize;
    for c in clauses {
        for (i, a) in c.iter().enumerate() {
            for b in &c[i + 1..] {
                let (x, y) = (a.var(), b.var());
                if x != y && adj[x as usize].insert(y) {
                    adj[y as usize].insert(x);
                    edges += 1;
                }
            }
        }
        if edges > fill {
            return None;
        }
    }
    let mut heap: BinaryHeap<Reverse<(usize, u32)>> = (0..num_vars).map(|v| Reverse((adj[v].len(), v as u32))).collect();
    let mut place = vec![u32::MAX; num_vars];
    let mut next = 0;
    while let Some(Reverse((degree, v))) = heap.pop() {
        if place[v as usize] != u32::MAX || degree != adj[v as usize].len() {
            continue;
        }
        place[v as usize] = next;
        next += 1;
        let around: Vec<u32> = adj[v as usize].drain().collect();
        for &a in &around {
            adj[a as usize].remove(&v);
        }
        for (i, &a) in around.iter().enumerate() {
            for &b in &around[i + 1..] {
                if adj[a as usize].insert(b) {
                    adj[b as usize].insert(a);
                    edges += 1;
                }
            }
        }
        if edges > fill {
            return None;
        }
        for &a in &around {
            heap.push(Reverse((adj[a as usize].len(), a)));
        }
    }
    Some(place)
}

/// `2^k`.
fn power_of_two(k: usize) -> BigInt {
    BigInt::from_u64(1).shl_bits(k as u64)
}

/// The reason of a variable assigned by a branch rather than implied by a clause.
const BRANCH: u32 = u32::MAX;

/// The value of `l` under `value`: `1` true, `-1` false, `0` open.
fn truth(value: &[i8], l: Lit) -> i8 {
    let v = value[l.var() as usize];
    if l.is_positive() {
        v
    } else {
        -v
    }
}

struct Counter {
    /// The formula's clauses, the first `original`, then the clauses conflicts teach. Only the formula's make
    /// components and their keys.
    clauses: Vec<Vec<Lit>>,
    original: usize,
    /// Per literal, the formula's clauses holding it.
    occurs: Vec<Vec<u32>>,
    /// Per literal, the taught clauses watching it, each watched by its first two literals.
    watches: Vec<Vec<u32>>,
    /// Per variable its value (`1` true, `-1` false, `0` open); the literals assigned, in order; per clause of the
    /// formula how many of its literals are true and how many false.
    value: Vec<i8>,
    trail: Vec<Lit>,
    trues: Vec<u32>,
    falses: Vec<u32>,
    /// Per variable the clause that implied it ([`BRANCH`] for a branch) and its level: the branches on the trail when
    /// it was assigned. The trail's length before each branch on it.
    reason: Vec<u32>,
    level: Vec<u32>,
    branches: Vec<usize>,
    /// The taught clauses that may be asserted, each with its level: the latest level among its literals but the one
    /// it asserts.
    pending: Vec<(u32, u32)>,
    seen: Vec<bool>,
    /// Per component key its count and its insertion's number; the insertions withdrawn, disjoint ranges in order.
    cache: HashMap<Vec<u32>, (BigInt, u64)>,
    inserted: u64,
    withdrawn: Vec<(u64, u64)>,
    /// How many times a taught clause has implied a literal or been falsified.
    taught_acts: u64,
    /// Per variable its place in the formula's elimination ([`elimination`]), all 0 when there is none.
    place: Vec<u32>,
    /// Per variable and per clause of the formula, the last search that met it, and that search's number; per
    /// variable, its open literals among the clauses of the component the last search that met it found.
    met_var: Vec<u32>,
    met_clause: Vec<u32>,
    epoch: u32,
    weight: Vec<u32>,
    decisions: u64,
    components: u64,
    cache_hits: u64,
    conflicts: u64,
}

impl Counter {
    fn new(num_vars: usize, clauses: Vec<Vec<Lit>>) -> Counter {
        let mut occurs = vec![Vec::new(); 2 * num_vars];
        for (k, c) in clauses.iter().enumerate() {
            for l in c {
                occurs[l.index()].push(k as u32);
            }
        }
        let n = clauses.len();
        let place = elimination(num_vars, &clauses, FILL).unwrap_or_else(|| vec![0; num_vars]);
        Counter {
            place,
            clauses,
            original: n,
            occurs,
            watches: vec![Vec::new(); 2 * num_vars],
            value: vec![0; num_vars],
            trail: Vec::new(),
            trues: vec![0; n],
            falses: vec![0; n],
            reason: vec![BRANCH; num_vars],
            level: vec![0; num_vars],
            branches: Vec::new(),
            pending: Vec::new(),
            seen: vec![false; num_vars],
            cache: HashMap::new(),
            inserted: 0,
            withdrawn: Vec::new(),
            taught_acts: 0,
            met_var: vec![0; num_vars],
            met_clause: vec![0; n],
            epoch: 0,
            weight: vec![0; num_vars],
            decisions: 0,
            components: 0,
            cache_hits: 0,
            conflicts: 0,
        }
    }

    fn holds(&self, l: Lit) -> i8 {
        truth(&self.value, l)
    }

    /// Assign `l`, implied by clause `reason` or [`BRANCH`], at the current level.
    fn assign(&mut self, l: Lit, reason: u32) {
        let v = l.var() as usize;
        self.value[v] = if l.is_positive() { 1 } else { -1 };
        self.reason[v] = reason;
        self.level[v] = self.branches.len() as u32;
        self.trail.push(l);
        for &c in &self.occurs[l.index()] {
            self.trues[c as usize] += 1;
        }
        for &c in &self.occurs[l.negated().index()] {
            self.falses[c as usize] += 1;
        }
    }

    /// Branch on `l`: a level of its own.
    fn branch(&mut self, l: Lit) {
        self.branches.push(self.trail.len());
        self.assign(l, BRANCH);
    }

    /// Take back every assignment past the first `len` of the trail, and the branches among them.
    fn undo(&mut self, len: usize) {
        while self.trail.len() > len {
            let l = self.trail.pop().expect("a literal past the mark");
            self.value[l.var() as usize] = 0;
            for &c in &self.occurs[l.index()] {
                self.trues[c as usize] -= 1;
            }
            for &c in &self.occurs[l.negated().index()] {
                self.falses[c as usize] -= 1;
            }
        }
        while self.branches.last().is_some_and(|&b| b >= len) {
            self.branches.pop();
        }
    }

    /// Propagate the trail from `head` to a fixpoint over the formula's clauses and the taught ones: the clause it
    /// falsifies, if any.
    fn propagate(&mut self, mut head: usize) -> Option<u32> {
        while head < self.trail.len() {
            let falsified = self.trail[head].negated();
            head += 1;
            for i in 0..self.occurs[falsified.index()].len() {
                let c = self.occurs[falsified.index()][i] as usize;
                if self.trues[c] > 0 {
                    continue;
                }
                let len = self.clauses[c].len() as u32;
                if self.falses[c] == len {
                    return Some(c as u32);
                }
                if self.falses[c] + 1 == len {
                    let u = *self.clauses[c].iter().find(|&&u| self.holds(u) == 0).expect("the open literal");
                    self.assign(u, c as u32);
                }
            }
            if let Some(c) = self.propagate_taught(falsified) {
                return Some(c);
            }
        }
        None
    }

    /// Visit the taught clauses watching `falsified`, just made false: each moves that watch to a literal not false,
    /// or implies its other watched literal, or is falsified, the clause returned.
    fn propagate_taught(&mut self, falsified: Lit) -> Option<u32> {
        let mut watching = std::mem::take(&mut self.watches[falsified.index()]);
        let (mut kept, mut at) = (0, 0);
        let mut conflict = None;
        while at < watching.len() {
            let c = watching[at];
            at += 1;
            let lits = &mut self.clauses[c as usize];
            if lits.len() == 1 {
                watching[kept] = c;
                kept += 1;
                self.taught_acts += 1;
                conflict = Some(c);
                break;
            }
            if lits[0] == falsified {
                lits.swap(0, 1);
            }
            let other = lits[0];
            let other_holds = truth(&self.value, other);
            if other_holds == 1 {
                watching[kept] = c;
                kept += 1;
                continue;
            }
            if let Some(k) = (2..lits.len()).find(|&k| truth(&self.value, lits[k]) != -1) {
                lits.swap(1, k);
                self.watches[lits[1].index()].push(c);
                continue;
            }
            watching[kept] = c;
            kept += 1;
            self.taught_acts += 1;
            if other_holds == -1 {
                conflict = Some(c);
                break;
            }
            self.assign(other, c);
        }
        while at < watching.len() {
            watching[kept] = watching[at];
            kept += 1;
            at += 1;
        }
        watching.truncate(kept);
        self.watches[falsified.index()] = watching;
        conflict
    }

    /// The clause the conflict at clause `conflict` teaches, by resolution back to the current level's first unique
    /// implication point: that point negated first, then the literal of the latest earlier level, then the rest of
    /// those earlier levels' literals (level 0's left out: the formula implies them outright); and the clause's level,
    /// the latest of those earlier levels, 0 when there are none.
    fn analyze(&mut self, conflict: u32) -> (Vec<Lit>, u32) {
        let now = self.branches.len() as u32;
        let mut taught = vec![Lit::pos(0)];
        let (mut open, mut at) = (0usize, self.trail.len());
        let (mut clause, mut resolved) = (conflict as usize, None);
        loop {
            for k in 0..self.clauses[clause].len() {
                let l = self.clauses[clause][k];
                let v = l.var() as usize;
                if Some(l.var()) == resolved || self.seen[v] || self.level[v] == 0 {
                    continue;
                }
                self.seen[v] = true;
                if self.level[v] == now {
                    open += 1;
                } else {
                    taught.push(l);
                }
            }
            let point = loop {
                at -= 1;
                if self.seen[self.trail[at].var() as usize] {
                    break self.trail[at];
                }
            };
            self.seen[point.var() as usize] = false;
            open -= 1;
            if open == 0 {
                taught[0] = point.negated();
                break;
            }
            clause = self.reason[point.var() as usize] as usize;
            resolved = Some(point.var());
        }
        for l in &taught[1..] {
            self.seen[l.var() as usize] = false;
        }
        let latest = (1..taught.len()).max_by_key(|&k| self.level[taught[k].var() as usize]);
        let level = match latest {
            Some(k) => {
                taught.swap(1, k);
                self.level[taught[1].var() as usize]
            }
            None => 0,
        };
        (taught, level)
    }

    /// Count the conflict at clause `conflict` and learn the clause it teaches, pending; a conflict below every branch
    /// teaches nothing new, the formula having no model.
    fn learn(&mut self, conflict: u32) {
        self.conflicts += 1;
        if self.branches.is_empty() {
            return;
        }
        let (lits, level) = self.analyze(conflict);
        let c = self.teach(lits);
        self.pending.push((c, level));
    }

    /// Take in `lits`, a clause the formula implies, as taught, watched by its first two literals, which must be its
    /// latest assigned or open: its number.
    fn teach(&mut self, lits: Vec<Lit>) -> u32 {
        let c = self.clauses.len() as u32;
        self.watches[lits[0].index()].push(c);
        if lits.len() > 1 {
            self.watches[lits[1].index()].push(c);
        }
        self.clauses.push(lits);
        c
    }

    /// Drop the pending clauses past `since` whose level is past the current one, which the trail no longer holds.
    fn prune_pending(&mut self, since: usize) {
        let now = self.branches.len() as u32;
        let mut kept = since;
        for k in since..self.pending.len() {
            if self.pending[k].1 <= now {
                self.pending[kept] = self.pending[k];
                kept += 1;
            }
        }
        self.pending.truncate(kept);
    }

    /// Assert each pending clause past `since` that is unit under the trail, propagating each: false on a conflict,
    /// whose clause is learned.
    fn assert_pending(&mut self, since: usize) -> bool {
        self.prune_pending(since);
        for k in since..self.pending.len() {
            let c = self.pending[k].0;
            let lits = &self.clauses[c as usize];
            if lits.iter().any(|&l| truth(&self.value, l) == 1) {
                continue;
            }
            let mut open = lits.iter().copied().filter(|&l| truth(&self.value, l) == 0);
            let conflict = match (open.next(), open.next()) {
                (None, _) => Some(c),
                (Some(u), None) => {
                    let mark = self.trail.len();
                    self.taught_acts += 1;
                    self.assign(u, c);
                    self.propagate(mark)
                }
                (Some(_), Some(_)) => None,
            };
            if let Some(conflict) = conflict {
                self.learn(conflict);
                return false;
            }
        }
        true
    }

    /// Whether the cache insertion numbered `n` has been withdrawn.
    fn is_withdrawn(&self, n: u64) -> bool {
        let k = self.withdrawn.partition_point(|&(start, _)| start <= n);
        k > 0 && n < self.withdrawn[k - 1].1
    }

    /// Withdraw every cache insertion since the one numbered `since`.
    fn withdraw(&mut self, since: u64) {
        while self.withdrawn.last().is_some_and(|&(start, _)| start >= since) {
            self.withdrawn.pop();
        }
        if since < self.inserted {
            self.withdrawn.push((since, self.inserted));
        }
    }

    fn run(&mut self) -> BigInt {
        if self.clauses.iter().any(Vec::is_empty) {
            return BigInt::zero();
        }
        for c in 0..self.original {
            if self.clauses[c].len() == 1 {
                let u = self.clauses[c][0];
                match self.holds(u) {
                    1 => {}
                    -1 => return BigInt::zero(),
                    _ => self.assign(u, c as u32),
                }
            }
        }
        if self.propagate(0).is_some() {
            return BigInt::zero();
        }
        let open: Vec<Var> = (0..self.value.len() as Var).filter(|&v| self.value[v as usize] == 0).collect();
        self.split(&open)
    }

    /// The models over the open variables among `vars` under the current assignment: the components they and the
    /// clauses still open among them fall into, counted and multiplied, a variable no open clause holds doubling it.
    /// A component with no models makes the product 0, and withdraws what the cache took in since the split began if a
    /// taught clause acted meanwhile.
    fn split(&mut self, vars: &[Var]) -> BigInt {
        let (inserted, acts) = (self.inserted, self.taught_acts);
        self.epoch += 1;
        let epoch = self.epoch;
        let mut parts: Vec<(Vec<Var>, Vec<u32>)> = Vec::new();
        let mut free = 0;
        for &v in vars {
            if self.value[v as usize] != 0 || self.met_var[v as usize] == epoch {
                continue;
            }
            self.met_var[v as usize] = epoch;
            self.weight[v as usize] = 0;
            let (mut cvars, mut cclauses) = (vec![v], Vec::new());
            let mut at = 0;
            while at < cvars.len() {
                let u = cvars[at];
                at += 1;
                for lit in [Lit::pos(u), Lit::neg(u)] {
                    for i in 0..self.occurs[lit.index()].len() {
                        let c = self.occurs[lit.index()][i];
                        if self.trues[c as usize] > 0 || self.met_clause[c as usize] == epoch {
                            continue;
                        }
                        self.met_clause[c as usize] = epoch;
                        cclauses.push(c);
                        for &w in &self.clauses[c as usize] {
                            let wv = w.var() as usize;
                            if self.value[wv] != 0 {
                                continue;
                            }
                            if self.met_var[wv] != epoch {
                                self.met_var[wv] = epoch;
                                self.weight[wv] = 0;
                                cvars.push(wv as Var);
                            }
                            self.weight[wv] += 1;
                        }
                    }
                }
            }
            if cclauses.is_empty() {
                free += 1;
            } else {
                parts.push((cvars, cclauses));
            }
        }
        let mut total = power_of_two(free);
        for (mut cvars, mut cclauses) in parts {
            cvars.sort_unstable();
            cclauses.sort_unstable();
            let part = self.component(&cvars, &cclauses);
            if part.is_zero() {
                if self.taught_acts != acts {
                    self.withdraw(inserted);
                }
                return BigInt::zero();
            }
            total = total.mul(&part);
        }
        total
    }

    /// The models of a component over its variables, from the cache or by branching on its variable first in the
    /// elimination and then in the most open clauses, as the split that found the component counted them. A conflict in the first branch teaches clauses asserted for the
    /// second; the second is no branch at all when they imply it.
    fn component(&mut self, vars: &[Var], clauses: &[u32]) -> BigInt {
        let mut key: Vec<u32> = Vec::with_capacity(vars.len() + clauses.len() + 1);
        key.extend(vars);
        key.push(u32::MAX);
        key.extend(clauses);
        if let Some((hit, n)) = self.cache.get(&key) {
            if !self.is_withdrawn(*n) {
                self.cache_hits += 1;
                return hit.clone();
            }
        }
        self.components += 1;
        let most = vars.iter().max_by_key(|&&v| (self.place[v as usize], self.weight[v as usize], Reverse(v)));
        let pivot = *most.expect("an open variable");
        self.decisions += 1;
        let (entry, since) = (self.trail.len(), self.pending.len());
        let mut models = BigInt::zero();
        for (k, lit) in [Lit::pos(pivot), Lit::neg(pivot)].into_iter().enumerate() {
            match self.holds(lit) {
                -1 => {}
                1 => models = models.add(&self.split(vars)),
                _ => {
                    let mark = self.trail.len();
                    self.branch(lit);
                    match self.propagate(mark) {
                        None => models = models.add(&self.split(vars)),
                        Some(conflict) => self.learn(conflict),
                    }
                    self.undo(mark);
                    if k == 0 && !self.assert_pending(since) {
                        break;
                    }
                }
            }
        }
        self.undo(entry);
        self.prune_pending(since);
        self.cache.insert(key, (models.clone(), self.inserted));
        self.inserted += 1;
        models
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/count/tests.rs"]
mod tests;
