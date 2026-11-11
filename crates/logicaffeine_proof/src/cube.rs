//! **Cube-and-conquer** (Heule, Kullmann, Wieringa & Biere, *Cube and conquer: guiding CDCL SAT solvers by
//! lookaheads*, HVC 2011), deterministic and certified.
//!
//! A lookahead splits the formula into cubes, and CDCL conquers them. At each node of the split the lookahead looks
//! ahead on the variables of most weight in the clauses the node leaves open (`Options::preselect` of them, a binary
//! clause counting five times a longer one): each polarity is assigned on a level of its own and propagated. A
//! polarity that conflicts is a failed literal, its negation forced on the node's path; both conflicting refute the
//! node. Otherwise the node branches on the variable whose two polarities assign the most, by the product of the two
//! counts, until `Options::depth` branches lie on the path; the path, branches and forced literals together, is then
//! a cube. The cubes are dealt round-robin to `Options::threads` incremental solvers, each solving its cubes in order
//! under assumptions; the formula is satisfiable when a cube is, answered with the model of the lowest-numbered one,
//! which every solver below it decides before it, so the answer follows from the formula and the options alone,
//! never from the threads' timing.
//!
//! A refutation is RUP throughout: the clauses the lookahead derived (a failed literal's, `¬Q ∨ ¬x` for a path `Q`),
//! each solver's learned clauses with its cubes' negations among them in the order it made them, then the split's own
//! clauses bottom-up — after a node's two branches `¬(Q ∧ y)` and `¬(Q ∧ ¬y)`, each from its child's clause and the
//! child's failed literals, then `¬Q` — ending at the empty clause.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cdcl::{BudgetedResult, Lit, SolveResult, Solver};

/// How [`solve`] splits and conquers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The branches on a cube's path.
    pub depth: usize,
    /// The variables looked ahead on at a node.
    pub preselect: usize,
    /// The solvers conquering the cubes, a thread each.
    pub threads: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { depth: 10, preselect: 40, threads: 4 }
    }
}

/// How a split and its conquest ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub result: SolveResult,
    /// With [`SolveResult::Unsat`], a RUP refutation of the formula, the last clause empty.
    pub proof: Vec<Vec<Lit>>,
    /// The cubes the split left to the solvers, and the nodes the lookahead refuted itself.
    pub cubes: usize,
    pub refuted: usize,
    /// With [`SolveResult::Sat`], the cube whose model it is.
    pub cube: Option<usize>,
}

/// Split `clauses` over `num_vars` variables into cubes by lookahead and conquer them.
pub fn solve(num_vars: usize, clauses: &[Vec<Lit>], opts: &Options) -> Outcome {
    let mut cuber = Cuber::new(num_vars, clauses, opts);
    if !cuber.solver.settle() {
        return Outcome { result: SolveResult::Unsat, proof: vec![Vec::new()], cubes: 0, refuted: 1, cube: None };
    }
    cuber.split();
    let Cuber { cubes, failed, tree, refuted, .. } = cuber;
    let conquest = conquer(num_vars, clauses, &cubes, opts.threads.max(1));
    if let Some((cube, model)) = conquest.model {
        let result = SolveResult::Sat(model);
        return Outcome { result, proof: Vec::new(), cubes: cubes.len(), refuted, cube: Some(cube) };
    }
    let mut proof = failed;
    proof.extend(conquest.steps);
    proof.extend(tree);
    if proof.last().is_none_or(|c| !c.is_empty()) {
        proof.push(Vec::new());
    }
    Outcome { result: SolveResult::Unsat, proof, cubes: cubes.len(), refuted, cube: None }
}

/// The clause that refutes a path: its literals negated.
fn refuting(path: &[Lit]) -> Vec<Lit> {
    path.iter().map(|l| l.negated()).collect()
}

/// The split in progress: a solver whose levels are the path, a literal each, and what the split has made.
struct Cuber<'a> {
    num_vars: usize,
    clauses: &'a [Vec<Lit>],
    solver: Solver,
    opts: &'a Options,
    path: Vec<Lit>,
    branches: usize,
    /// The failed literals' clauses in the order found, the split's own clauses bottom-up, and the cubes.
    failed: Vec<Vec<Lit>>,
    tree: Vec<Vec<Lit>>,
    cubes: Vec<Vec<Lit>>,
    refuted: usize,
}

/// What a node's lookahead found.
enum Look {
    /// A failed literal's negation, forced on the path.
    Forced(Lit),
    /// Both polarities of a variable fail: the node is refuted, the clause `¬Q ∨ ¬x` already among the failed.
    Refuted,
    /// The variable to branch on.
    Branch(u32),
    /// No clause is left open: the path is a model's.
    Satisfied,
}

impl<'a> Cuber<'a> {
    fn new(num_vars: usize, clauses: &'a [Vec<Lit>], opts: &'a Options) -> Cuber<'a> {
        let mut solver = Solver::new(num_vars);
        for c in clauses {
            solver.add_clause(c.clone());
        }
        Cuber {
            num_vars,
            clauses,
            solver,
            opts,
            path: Vec::new(),
            branches: 0,
            failed: Vec::new(),
            tree: Vec::new(),
            cubes: Vec::new(),
            refuted: 0,
        }
    }

    fn open(&self, l: Lit) -> bool {
        self.solver.value(l.var() as usize).is_none()
    }

    fn holds(&self, l: Lit) -> bool {
        self.solver.value(l.var() as usize) == Some(l.is_positive())
    }

    /// The literals `l`'s level assigns, or `None` when it conflicts; the search left as it was.
    fn probe(&mut self, l: Lit) -> Option<usize> {
        let assigned = self.solver.assume(l);
        self.solver.retract();
        assigned
    }

    /// The open variables of most weight in the clauses still open, most first: a clause left with two open literals
    /// weighs five on each, a longer one one.
    fn preselect(&self) -> Vec<u32> {
        let mut weight: Vec<u32> = vec![0; self.num_vars];
        for c in self.clauses {
            if c.iter().any(|&l| self.holds(l)) {
                continue;
            }
            let open = c.iter().filter(|&&l| self.open(l)).count();
            let w = if open == 2 { 5 } else { 1 };
            for &l in c.iter().filter(|&&l| self.open(l)) {
                weight[l.var() as usize] += w;
            }
        }
        let mut ranked: Vec<u32> = (0..weight.len() as u32).filter(|&v| weight[v as usize] > 0).collect();
        ranked.sort_by_key(|&v| (std::cmp::Reverse(weight[v as usize]), v));
        ranked.truncate(self.opts.preselect.max(1));
        ranked
    }

    /// Look ahead on the preselected variables: the first failed literal found, or the variable to branch on.
    fn look(&mut self) -> Look {
        let candidates = self.preselect();
        if candidates.is_empty() {
            return Look::Satisfied;
        }
        let mut best: Option<(u64, u32)> = None;
        for x in candidates {
            let (pos, neg) = (self.probe(Lit::pos(x)), self.probe(Lit::neg(x)));
            match (pos, neg) {
                (None, None) => {
                    let mut clause = refuting(&self.path);
                    clause.push(Lit::neg(x));
                    self.failed.push(clause);
                    return Look::Refuted;
                }
                (None, Some(_)) => return Look::Forced(Lit::neg(x)),
                (Some(_), None) => return Look::Forced(Lit::pos(x)),
                (Some(a), Some(b)) => {
                    let score = (a as u64) * (b as u64) + a as u64 + b as u64;
                    if best.is_none_or(|(s, _)| score > s) {
                        best = Some((score, x));
                    }
                }
            }
        }
        Look::Branch(best.expect("a candidate was scored").1)
    }

    /// Split below the path. Afterwards the split's clauses refute the path's (`¬Q`), a cube left open aside: the
    /// node's own clause is the last pushed to `tree`, or, for a cube, the negation its solver gives.
    fn split(&mut self) {
        let mut forced = 0;
        loop {
            match self.look() {
                Look::Forced(f) => {
                    let mut clause = refuting(&self.path);
                    clause.push(f);
                    self.failed.push(clause);
                    if self.solver.assume(f).is_none() {
                        self.solver.retract();
                        self.close(forced);
                        return;
                    }
                    self.path.push(f);
                    forced += 1;
                }
                Look::Refuted => {
                    self.close(forced);
                    return;
                }
                Look::Satisfied => {
                    self.cubes.push(self.path.clone());
                    self.unwind(forced);
                    return;
                }
                Look::Branch(_) if self.branches == self.opts.depth => {
                    self.cubes.push(self.path.clone());
                    self.unwind(forced);
                    return;
                }
                Look::Branch(x) => {
                    for y in [Lit::pos(x), Lit::neg(x)] {
                        let opened = self.solver.assume(y);
                        debug_assert!(opened.is_some(), "a branch the lookahead scored propagates without conflict");
                        self.path.push(y);
                        self.branches += 1;
                        self.split();
                        self.branches -= 1;
                        self.tree.push(refuting(&self.path));
                        self.path.pop();
                        self.solver.retract();
                    }
                    self.tree.push(refuting(&self.path));
                    self.unwind(forced);
                    return;
                }
            }
        }
    }

    /// The node is refuted by its failed literals: its clause, then its forced literals taken back.
    fn close(&mut self, forced: usize) {
        self.refuted += 1;
        self.tree.push(refuting(&self.path));
        self.unwind(forced);
    }

    /// Take back the literals the node forced.
    fn unwind(&mut self, forced: usize) {
        for _ in 0..forced {
            self.path.pop();
            self.solver.retract();
        }
    }
}

/// What the solvers made of the cubes.
struct Conquest {
    /// The lowest-numbered cube a model satisfies, and the model.
    model: Option<(usize, Vec<bool>)>,
    /// Every solver's learned clauses with its cubes' negations among them, solver by solver, each in its order.
    steps: Vec<Vec<Lit>>,
}

fn conquer(num_vars: usize, clauses: &[Vec<Lit>], cubes: &[Vec<Lit>], threads: usize) -> Conquest {
    let lowest = AtomicUsize::new(usize::MAX);
    let runs: Vec<(Option<(usize, Vec<bool>)>, Vec<Vec<Lit>>)> = std::thread::scope(|scope| {
        let lowest = &lowest;
        let searches: Vec<_> = (0..threads.min(cubes.len().max(1)))
            .map(|t| {
                scope.spawn(move || {
                    let mut solver = Solver::new(num_vars);
                    for c in clauses {
                        solver.add_clause(c.clone());
                    }
                    let mut steps = Vec::new();
                    for i in (t..cubes.len()).step_by(threads) {
                        if i > lowest.load(Ordering::Relaxed) {
                            break;
                        }
                        let verdict = solver.solve_under_assumptions_budgeted(&cubes[i], 0);
                        steps.extend(solver.take_learned().into_iter().map(|c| c.lits));
                        match verdict {
                            BudgetedResult::Sat(model) => {
                                lowest.fetch_min(i, Ordering::Relaxed);
                                return (Some((i, model)), steps);
                            }
                            BudgetedResult::Unsat => steps.push(refuting(&cubes[i])),
                            BudgetedResult::Budget => unreachable!("a solve without a budget runs to a verdict"),
                        }
                    }
                    (None, steps)
                })
            })
            .collect();
        searches.into_iter().map(|s| s.join().expect("a conquering thread")).collect()
    });
    let model = runs.iter().filter_map(|(m, _)| m.clone()).min_by_key(|(i, _)| *i);
    let steps = runs.into_iter().flat_map(|(_, s)| s).collect();
    Conquest { model, steps }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/cube/tests.rs"]
mod tests;
