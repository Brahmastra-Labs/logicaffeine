//! Craig interpolants from resolution refutations (McMillan, *Interpolation and SAT-based model checking*, CAV 2003).
//!
//! When `A ∧ B` is unsatisfiable, an interpolant `I` is a formula over the variables `A` and `B` share with
//! `A ⊨ I` and `I ∧ B ⊨ ⊥`: an over-approximation of `A` that `B` still contradicts. A resolution refutation of
//! `A ∧ B` carries one in linear time. Each clause of the refutation gets a partial interpolant: an `A` clause the
//! disjunction of its shared literals, a `B` clause `true`, and a resolvent the disjunction of its parents' when the
//! pivot occurs only in `A`, their conjunction otherwise. The empty clause's partial interpolant is an interpolant.
//!
//! The refutation is an LRAT proof ([`crate::lrat`]) of the clauses of `A` followed by those of `B`. Each step is
//! replayed as the resolution chain its hints stand for: the step's negation is assumed, each hint is unit (or the
//! conflict) under it, and resolving the conflict clause back through the hints whose unit literal it contains gives a
//! clause inside the step's own. That clause, not the step's, is what later steps resolve with, so a proof whose
//! steps are stronger than they claim still yields a sound chain. A step whose hints do not replay this way refutes
//! nothing and the interpolant is refused: the result is `None`, never a formula the refutation does not justify.
//!
//! The interpolant is an and-inverter graph ([`crate::aig::Aig`]), structurally hashed as it is built: McMillan
//! interpolants share sub-terms heavily, and a tree would be exponentially larger than the proof. Input `i` of the
//! graph is the shared variable `shared[i]`.

use std::collections::HashMap;

use crate::aig::{Aig, AigLit, Node};
use crate::cdcl::{Lit, SolveResult, Solver, Var};
use crate::lrat::Lrat;

/// Which side of the partition a variable occurs on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Locality {
    /// Only in `A`: a resolution on it joins partial interpolants by disjunction.
    A,
    /// Only in `B`.
    B,
    /// In both: the interpolant's vocabulary.
    Shared,
    /// In neither.
    Absent,
}

/// A Craig interpolant of `A ∧ B`, as an and-inverter graph over the shared variables.
#[derive(Clone, Debug)]
pub struct Interpolant {
    /// The circuit. Its input `i` is the formula variable `shared[i]`.
    pub aig: Aig,
    /// The literal of the circuit the interpolant is.
    pub root: AigLit,
    /// The variables `A` and `B` share, increasing: the circuit's inputs, in order.
    pub shared: Vec<Var>,
}

impl Interpolant {
    /// The interpolant's value under an assignment of the formula's variables.
    pub fn eval(&self, assignment: &[bool]) -> bool {
        let inputs: Vec<bool> = self.shared.iter().map(|&v| assignment[v as usize]).collect();
        let values = self.aig.eval(&inputs, &[]);
        Aig::value(&values, self.root)
    }

    /// The interpolant as clauses: its Tseitin encoding over the formula's variables, with every gate a fresh
    /// variable numbered from `*next_var` on. Returns the literal that holds exactly when the interpolant does.
    pub fn encode(&self, clauses: &mut Vec<Vec<Lit>>, next_var: &mut usize) -> Lit {
        let shared = &self.shared;
        encode_cone(&self.aig, self.root, &mut |i| Lit::pos(shared[i]), clauses, next_var)
    }
}

/// The verdict of [`interpolate`].
#[derive(Clone, Debug)]
pub enum Interpolation {
    /// `A ∧ B` is unsatisfiable, and this is an interpolant.
    Interpolant(Interpolant),
    /// `A ∧ B` is satisfiable: a model of both.
    Satisfiable(Vec<bool>),
    /// The solver's refutation could not be turned into a checked LRAT proof.
    Unknown,
}

/// Where each variable below `num_vars` occurs among `a` and `b`.
pub fn localities(num_vars: usize, a: &[Vec<Lit>], b: &[Vec<Lit>]) -> Vec<Locality> {
    let mut in_a = vec![false; num_vars];
    let mut in_b = vec![false; num_vars];
    for l in a.iter().flatten() {
        in_a[l.var() as usize] = true;
    }
    for l in b.iter().flatten() {
        in_b[l.var() as usize] = true;
    }
    (0..num_vars)
        .map(|v| match (in_a[v], in_b[v]) {
            (true, true) => Locality::Shared,
            (true, false) => Locality::A,
            (false, true) => Locality::B,
            (false, false) => Locality::Absent,
        })
        .collect()
}

/// Decide `a ∧ b` and, when it is unsatisfiable, interpolate it: solve, build the trimmed LRAT refutation of the
/// solver's learned clauses ([`crate::lrat::refutation`]), and read the interpolant off it ([`interpolant`]).
pub fn interpolate(num_vars: usize, a: &[Vec<Lit>], b: &[Vec<Lit>]) -> Interpolation {
    let mut solver = Solver::new(num_vars);
    for c in a.iter().chain(b) {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => Interpolation::Satisfiable(model),
        SolveResult::Unsat => {
            let original: Vec<Vec<Lit>> = a.iter().chain(b).cloned().collect();
            let learned: Vec<Vec<Lit>> = solver.learned().iter().map(|c| c.lits.clone()).collect();
            match crate::lrat::refutation(num_vars, &original, &learned).and_then(|p| interpolant(num_vars, a, b, &p)) {
                Some(itp) => Interpolation::Interpolant(itp),
                None => Interpolation::Unknown,
            }
        }
    }
}

/// The McMillan interpolant of `a ∧ b` carried by `proof`, an LRAT refutation of the clauses of `a` followed by
/// those of `b` (ids `1..=|a|` for `a`, then `b`'s). `None` when a literal lies at or above `num_vars`, a step's id
/// does not exceed every id before it, a hint names no clause present, a step's hints do not replay as a resolution
/// chain, or no step derives the empty clause.
pub fn interpolant(num_vars: usize, a: &[Vec<Lit>], b: &[Vec<Lit>], proof: &Lrat) -> Option<Interpolant> {
    if a.iter().chain(b).flatten().any(|l| l.var() as usize >= num_vars) {
        return None;
    }
    let locality = localities(num_vars, a, b);
    let shared: Vec<Var> = (0..num_vars as Var).filter(|&v| locality[v as usize] == Locality::Shared).collect();
    let mut aig = Aig::new();
    let mut input_of: Vec<Option<AigLit>> = vec![None; num_vars];
    for &v in &shared {
        input_of[v as usize] = Some(aig.add_input(None));
    }

    // Each clause present, by id: the clause the chains resolve with, and its partial interpolant.
    let mut known: HashMap<u64, (Vec<Lit>, AigLit)> = HashMap::new();
    for (i, c) in a.iter().enumerate() {
        let mut p = AigLit::FALSE;
        for &l in c {
            if let Some(input) = input_of[l.var() as usize] {
                let lit = input.with_phase(!l.is_positive());
                p = aig.or(p, lit);
            }
        }
        known.insert(i as u64 + 1, (merged(c), p));
    }
    for (j, c) in b.iter().enumerate() {
        known.insert((a.len() + j) as u64 + 1, (merged(c), AigLit::TRUE));
    }

    let mut last = (a.len() + b.len()) as u64;
    let mut assign: Vec<Option<bool>> = vec![None; num_vars];
    for step in &proof.steps {
        if step.id <= last || step.clause.iter().any(|l| l.var() as usize >= num_vars) {
            return None;
        }
        let claimed = normalized(&step.clause)?;
        let (derived, p) = chain(&claimed, &step.hints, &known, &locality, &mut aig, &mut assign)?;
        if derived.is_empty() {
            return Some(Interpolant { aig, root: p, shared });
        }
        known.insert(step.id, (derived, p));
        last = step.id;
    }
    None
}

/// A clause with repeated literals merged. A tautology keeps both its literals: it is never unit or a conflict, so
/// no chain resolves on it.
fn merged(c: &[Lit]) -> Vec<Lit> {
    let mut out: Vec<Lit> = Vec::with_capacity(c.len());
    for &l in c {
        if !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// A derived clause with repeated literals merged, or `None` for a tautology: a refutation never derives one.
fn normalized(c: &[Lit]) -> Option<Vec<Lit>> {
    let mut out: Vec<Lit> = Vec::with_capacity(c.len());
    for &l in c {
        if out.contains(&l.negated()) {
            return None;
        }
        if !out.contains(&l) {
            out.push(l);
        }
    }
    Some(out)
}

/// One step's resolution chain: assume `¬clause`, propagate through the hints in order until one conflicts, then
/// resolve the conflict clause back through every hint whose unit literal it still contains. Returns the resolvent,
/// which lies inside `clause`, with its partial interpolant. `assign` is left cleared.
fn chain(
    clause: &[Lit],
    hints: &[u64],
    known: &HashMap<u64, (Vec<Lit>, AigLit)>,
    locality: &[Locality],
    aig: &mut Aig,
    assign: &mut [Option<bool>],
) -> Option<(Vec<Lit>, AigLit)> {
    let mut set: Vec<Var> = Vec::new();
    let result = (|| {
        for &l in clause {
            let v = l.var();
            assign[v as usize] = Some(!l.is_positive());
            set.push(v);
        }
        let truth = |assign: &[Option<bool>], l: Lit| assign[l.var() as usize].map(|x| x == l.is_positive());
        // The literals the hints made true, each with the hint that forced it.
        let mut forced: Vec<(Lit, u64)> = Vec::new();
        let mut conflict: Option<u64> = None;
        for &h in hints {
            let (c, _) = known.get(&h)?;
            let mut open: Option<Lit> = None;
            for &l in c {
                match truth(assign, l) {
                    Some(true) => return None,
                    Some(false) => {}
                    None if open.is_none() => open = Some(l),
                    None => return None,
                }
            }
            match open {
                None => {
                    conflict = Some(h);
                    break;
                }
                Some(u) => {
                    assign[u.var() as usize] = Some(u.is_positive());
                    set.push(u.var());
                    forced.push((u, h));
                }
            }
        }
        let conflict = conflict?;
        let (start, p0) = known.get(&conflict)?;
        let mut resolvent: Vec<Lit> = start.clone();
        let mut p = *p0;
        for &(u, h) in forced.iter().rev() {
            let pivot = u.negated();
            let Some(at) = resolvent.iter().position(|&l| l == pivot) else { continue };
            resolvent.swap_remove(at);
            let (reason, ph) = known.get(&h)?;
            for &l in reason {
                if l != u && !resolvent.contains(&l) {
                    resolvent.push(l);
                }
            }
            p = if locality[u.var() as usize] == Locality::A { aig.or(p, *ph) } else { aig.and(p, *ph) };
        }
        // Every literal of a clause the chain uses is false when it is used: assumed, so in `clause`, or the
        // complement of a unit forced before it, which the backward pass resolves away.
        debug_assert!(resolvent.iter().all(|l| clause.contains(l)), "a chain's resolvent lies inside its clause");
        Some((resolvent, p))
    })();
    for v in set {
        assign[v as usize] = None;
    }
    result
}

/// The Tseitin encoding of the cone of `root` in `aig`: each and-node a fresh variable numbered from `*next_var` on,
/// its three defining clauses pushed to `clauses`; input `i` is the literal `input(i)`. A latch in the cone is not
/// expected here and is read as false. Returns the literal equal to `root`.
pub fn encode_cone(
    aig: &Aig,
    root: AigLit,
    input: &mut dyn FnMut(usize) -> Lit,
    clauses: &mut Vec<Vec<Lit>>,
    next_var: &mut usize,
) -> Lit {
    encode_cones(aig, &[root], input, clauses, next_var)[0]
}

/// The Tseitin encoding of the cones of `roots` in `aig`, a node they share encoded once, as [`encode_cone`] encodes
/// one: the literal equal to each root, in order.
pub fn encode_cones(
    aig: &Aig,
    roots: &[AigLit],
    input: &mut dyn FnMut(usize) -> Lit,
    clauses: &mut Vec<Vec<Lit>>,
    next_var: &mut usize,
) -> Vec<Lit> {
    let mut lit_of: HashMap<u32, Lit> = HashMap::new();
    let fresh = |next_var: &mut usize| {
        let v = *next_var as Var;
        *next_var += 1;
        Lit::pos(v)
    };
    for n in aig.cone(roots) {
        let l = match aig.node(n) {
            Node::False | Node::Latch(_) => {
                let f = fresh(next_var);
                clauses.push(vec![f.negated()]);
                f
            }
            Node::Input(i) => input(i),
            Node::And(x, y) => {
                let g = fresh(next_var);
                let lx = lit_of[&x.node()];
                let ly = lit_of[&y.node()];
                let lx = if x.is_complemented() { lx.negated() } else { lx };
                let ly = if y.is_complemented() { ly.negated() } else { ly };
                clauses.push(vec![g.negated(), lx]);
                clauses.push(vec![g.negated(), ly]);
                clauses.push(vec![g, lx.negated(), ly.negated()]);
                g
            }
        };
        lit_of.insert(n, l);
    }
    roots
        .iter()
        .map(|root| {
            let l = lit_of[&root.node()];
            if root.is_complemented() {
                l.negated()
            } else {
                l
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/interpolate/tests.rs"]
mod tests;
