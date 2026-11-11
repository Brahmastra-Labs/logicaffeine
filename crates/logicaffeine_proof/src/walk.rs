//! ProbSAT local search (Balint & Schöning, *Choosing probability distributions for stochastic local search and the
//! role of make versus break*, SAT 2012): the walker a CDCL solver runs between restarts to find phases near a model.
//!
//! The walk starts from a full assignment and repeatedly picks a clause the assignment leaves false, then flips one
//! of its variables, chosen with probability proportional to `cb^-break(v)`, where `break(v)` counts the clauses that
//! `v`'s currently true literal alone satisfies. A flip makes the picked clause true; the break count prices how many
//! others it falsifies. Per clause the walker keeps how many of its literals are true, so a flip touches only the
//! occurrence lists of the variable's two literals.
//!
//! The base `cb` follows the fit Kissat uses for the average clause size (2.5 at width 3, rising to 7.4 at width
//! 7; [`fitted_cb`]). The best assignment met is kept without copying the whole assignment on each improvement:
//! the flipped literals since a snapshot go on a trail, the best is the snapshot overwritten by the trail up to
//! the best position, and the trail is folded into the snapshot when it grows past a quarter of the variables
//! (Kissat's `walk.c`).
//!
//! Variables fixed by the caller (the solver's level-0 facts) never flip: a clause one of them satisfies is dropped
//! and a literal one of them falsifies is left out, so the walk ranges over the residual formula only. A walk that
//! reaches no false clause has found a model of every clause it was given; the result says so and carries it.

use crate::cdcl::{Lit, Var};

/// The walk's outcome: the best assignment it met and how far from a model it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Walk {
    /// The best assignment the walk reached, one value per variable (fixed variables at their fixed value).
    pub phases: Vec<bool>,
    /// Clauses of the walked formula the best assignment leaves false; 0 means `phases` satisfies all of them.
    pub unsatisfied: usize,
    /// Clauses the start left false: the best improved on the start exactly when it is above `unsatisfied`.
    pub started_unsatisfied: usize,
    /// Variables flipped.
    pub flips: u64,
    /// Occurrence-list entries visited: the effort measure the step limit bounds.
    pub steps: u64,
}

/// What one walk may spend and where its randomness starts.
#[derive(Clone, Copy, Debug)]
pub struct WalkOptions {
    /// The base of the break-count weighting; [`fitted_cb`] picks it from the clause width.
    pub cb: f64,
    /// The seed of the clause and literal picks: equal seeds walk equal paths.
    pub seed: u64,
    /// The occurrence visits after which the walk stops and returns its best.
    pub step_limit: u64,
}

/// The ProbSAT base `cb` for an average clause size, linearly interpolated through Kissat's fitted table.
pub fn fitted_cb(average_size: f64) -> f64 {
    const TABLE: [(f64, f64); 6] = [(0.0, 2.00), (3.0, 2.50), (4.0, 2.85), (5.0, 3.70), (6.0, 5.10), (7.0, 7.40)];
    let mut i = 0;
    while i + 2 < TABLE.len() && (TABLE[i].0 > average_size || TABLE[i + 1].0 < average_size) {
        i += 1;
    }
    let ((x1, y1), (x2, y2)) = (TABLE[i], TABLE[i + 1]);
    (y2 - y1) * (average_size - x1) / (x2 - x1) + y1
}

/// Walk from `start` over `clauses`, with each variable `fixed` returns `Some` for held at that value.
///
/// `start` gives every variable's first value (a missing entry starts false). Clauses may be any width; an empty
/// one, or one every literal of which a fixed variable falsifies, can never be satisfied and is counted in
/// [`Walk::unsatisfied`] throughout.
pub fn walk<'a>(
    num_vars: usize,
    clauses: impl IntoIterator<Item = &'a [Lit]>,
    fixed: impl Fn(Var) -> Option<bool>,
    start: &[bool],
    options: WalkOptions,
) -> Walk {
    let mut values: Vec<bool> = (0..num_vars).map(|v| fixed(v as Var).unwrap_or_else(|| start.get(v).copied().unwrap_or(false))).collect();
    let is_fixed: Vec<bool> = (0..num_vars).map(|v| fixed(v as Var).is_some()).collect();

    let mut lits: Vec<Lit> = Vec::new();
    let mut starts: Vec<u32> = vec![0];
    let mut dead = 0usize;
    for clause in clauses {
        if clause.iter().any(|l| is_fixed[l.var() as usize] && values[l.var() as usize] == l.is_positive()) {
            continue;
        }
        let before = lits.len();
        lits.extend(clause.iter().copied().filter(|l| !is_fixed[l.var() as usize]));
        if lits.len() == before {
            dead += 1;
            continue;
        }
        starts.push(lits.len() as u32);
    }
    let clause_count = starts.len() - 1;

    let mut occ_start = vec![0u32; num_vars * 2 + 1];
    for &l in &lits {
        occ_start[l.index() + 1] += 1;
    }
    for i in 0..num_vars * 2 {
        occ_start[i + 1] += occ_start[i];
    }
    let mut occ = vec![0u32; lits.len()];
    let mut fill = occ_start.clone();
    for c in 0..clause_count {
        for &l in &lits[starts[c] as usize..starts[c + 1] as usize] {
            occ[fill[l.index()] as usize] = c as u32;
            fill[l.index()] += 1;
        }
    }

    let lit_true = |values: &[bool], l: Lit| values[l.var() as usize] == l.is_positive();
    let mut true_count = vec![0u32; clause_count];
    let mut unsat: Vec<u32> = Vec::new();
    let mut unsat_pos = vec![u32::MAX; clause_count];
    for c in 0..clause_count {
        let n = lits[starts[c] as usize..starts[c + 1] as usize].iter().filter(|&&l| lit_true(&values, l)).count();
        true_count[c] = n as u32;
        if n == 0 {
            unsat_pos[c] = unsat.len() as u32;
            unsat.push(c as u32);
        }
    }

    let base = 1.0 / options.cb;
    let mut table: Vec<f64> = Vec::new();
    let mut next = 1.0f64;
    while next > 0.0 && next.is_normal() {
        table.push(next);
        next *= base;
    }
    let epsilon = *table.last().unwrap_or(&1.0);
    let score = |breaks: u32| table.get(breaks as usize).copied().unwrap_or(epsilon);

    let mut rng = options.seed ^ 0x5DEE_CE66_D1CE_4E5B;
    let mut random = move || {
        rng = rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };

    let mut best = values.clone();
    let mut best_unsat = unsat.len();
    let started_unsatisfied = best_unsat + dead;
    let mut trail: Vec<Lit> = Vec::new();
    let mut best_pos: Option<usize> = Some(0);
    let trail_cap = num_vars / 4 + 1;

    let (mut flips, mut steps) = (0u64, 0u64);
    let mut scores: Vec<f64> = Vec::new();
    while !unsat.is_empty() && steps < options.step_limit {
        let c = unsat[(random() % unsat.len() as u64) as usize] as usize;
        let clause = &lits[starts[c] as usize..starts[c + 1] as usize];
        scores.clear();
        let mut sum = 0.0;
        for &l in clause {
            let breaking = l.negated();
            let range = occ_start[breaking.index()] as usize..occ_start[breaking.index() + 1] as usize;
            steps += range.len() as u64 + 1;
            let breaks = occ[range].iter().filter(|&&d| true_count[d as usize] == 1).count() as u32;
            let s = score(breaks);
            scores.push(s);
            sum += s;
        }
        let threshold = sum * ((random() >> 11) as f64 / (1u64 << 53) as f64);
        let mut picked = clause[clause.len() - 1];
        let mut acc = 0.0;
        for (k, &l) in clause.iter().enumerate() {
            acc += scores[k];
            if threshold < acc {
                picked = l;
                break;
            }
        }

        values[picked.var() as usize] = picked.is_positive();
        flips += 1;
        let made = occ_start[picked.index()] as usize..occ_start[picked.index() + 1] as usize;
        steps += made.len() as u64;
        for &d in &occ[made] {
            let d = d as usize;
            true_count[d] += 1;
            if true_count[d] == 1 {
                let p = unsat_pos[d] as usize;
                let last = *unsat.last().unwrap();
                unsat[p] = last;
                unsat_pos[last as usize] = p as u32;
                unsat.pop();
                unsat_pos[d] = u32::MAX;
            }
        }
        let broken = picked.negated();
        let broke = occ_start[broken.index()] as usize..occ_start[broken.index() + 1] as usize;
        steps += broke.len() as u64;
        for &d in &occ[broke] {
            let d = d as usize;
            true_count[d] -= 1;
            if true_count[d] == 0 {
                unsat_pos[d] = unsat.len() as u32;
                unsat.push(d as u32);
            }
        }

        match best_pos {
            Some(_) if trail.len() < trail_cap => trail.push(picked),
            Some(p) if p > 0 => {
                for &l in &trail[..p] {
                    best[l.var() as usize] = l.is_positive();
                }
                trail.drain(..p);
                best_pos = Some(0);
                trail.push(picked);
            }
            Some(_) => {
                trail.clear();
                best_pos = None;
            }
            None => {}
        }
        if unsat.len() < best_unsat {
            best_unsat = unsat.len();
            match best_pos {
                Some(_) => best_pos = Some(trail.len()),
                None => {
                    best.clone_from(&values);
                    best_pos = Some(0);
                }
            }
        }
    }
    if let Some(p) = best_pos {
        for &l in &trail[..p] {
            best[l.var() as usize] = l.is_positive();
        }
    }
    Walk { phases: best, unsatisfied: best_unsat + dead, started_unsatisfied, flips, steps }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/walk/tests.rs"]
mod tests;
