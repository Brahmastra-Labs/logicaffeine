//! Certified inprocessing: clause-database simplifications that emit a machine-checkable proof.
//!
//! Where a normal SAT solver simplifies for speed, here every simplification carries its own
//! certificate — bounded variable elimination adds RUP resolvents and deletes the eliminated
//! variable's clauses; vivification replaces a clause with a RUP-shorter one. Composed with the
//! refutation, the whole run stays independently checkable by [`crate::pr::check_pr_refutation`].
//! This is the certified analogue of the inprocessing that makes Kissat/CaDiCaL fast — a real
//! differentiator: nobody ships *certified* inprocessing.

use crate::cdcl::{Lit, Var};
use crate::proof::ProofStep;

/// Resolve `a` (which contains `+x`) with `b` (which contains `¬x`) on `x`: the union of the two
/// clauses minus both `x` literals. `None` if the resolvent is a tautology (contains some literal
/// and its negation), which contributes nothing.
fn resolve(a: &[Lit], b: &[Lit], x: Var) -> Option<Vec<Lit>> {
    let mut r: Vec<Lit> = a.iter().copied().filter(|l| l.var() != x).collect();
    for &l in b {
        if l.var() != x && !r.contains(&l) {
            r.push(l);
        }
    }
    if r.iter().any(|&l| r.contains(&l.negated())) {
        return None; // tautology
    }
    Some(r)
}

/// Try to eliminate variable `x` from `clauses` by Davis-Putnam resolution. Returns the new clause
/// set and the proof steps (RUP resolvents, then the deletions of `x`'s clauses) — but only when
/// `x` actually occurs and elimination does not grow the clause count (the standard BVE bound).
/// `None` otherwise. Verdict-invariant: a model of the result extends to a model of the input.
pub fn eliminate_variable(clauses: &[Vec<Lit>], x: Var) -> Option<(Vec<Vec<Lit>>, Vec<ProofStep>)> {
    let (pos, neg) = (Lit::pos(x), Lit::neg(x));
    let pos_clauses: Vec<&Vec<Lit>> = clauses.iter().filter(|c| c.contains(&pos)).collect();
    let neg_clauses: Vec<&Vec<Lit>> = clauses.iter().filter(|c| c.contains(&neg)).collect();
    if pos_clauses.is_empty() && neg_clauses.is_empty() {
        return None; // `x` does not occur
    }
    // All non-tautological resolvents.
    let mut resolvents: Vec<Vec<Lit>> = Vec::new();
    for a in &pos_clauses {
        for b in &neg_clauses {
            if let Some(r) = resolve(a, b, x) {
                resolvents.push(r);
            }
        }
    }
    // Bound: never grow the clause database.
    if resolvents.len() > pos_clauses.len() + neg_clauses.len() {
        return None;
    }
    // Proof: each resolvent is RUP (its two parents force `x` and `¬x` under `¬resolvent`); then
    // delete every clause mentioning `x`. Resolvents are added BEFORE the deletions so their RUP
    // check still sees the parents.
    let mut steps: Vec<ProofStep> = resolvents.iter().map(|r| ProofStep::Rup(r.clone())).collect();
    for c in pos_clauses.iter().chain(neg_clauses.iter()) {
        steps.push(ProofStep::Delete((*c).clone()));
    }
    let mut new_clauses: Vec<Vec<Lit>> =
        clauses.iter().filter(|c| !c.contains(&pos) && !c.contains(&neg)).cloned().collect();
    new_clauses.extend(resolvents);
    Some((new_clauses, steps))
}

/// Bounded variable elimination over the whole formula: repeatedly eliminate every variable whose
/// elimination is non-growing, until a fixpoint (capped at a few passes). Returns the simplified
/// clause set and the composed proof steps.
pub fn bve(num_vars: usize, clauses: &[Vec<Lit>]) -> (Vec<Vec<Lit>>, Vec<ProofStep>) {
    let mut cur = clauses.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();
    for _pass in 0..4 {
        let mut changed = false;
        for x in 0..num_vars as Var {
            if let Some((next, s)) = eliminate_variable(&cur, x) {
                cur = next;
                steps.extend(s);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    (cur, steps)
}

/// Equivalent-literal detection via the binary-implication graph's strongly-connected components. Each
/// 2-clause `(a ∨ b)` contributes the implications `¬a → b` and `¬b → a`; literals in one SCC are logically
/// equivalent. If some `x` and `¬x` share an SCC, the binary clauses alone force `x ≡ ¬x` — UNSAT — and we
/// return the RUP certificate (`(x)`, `(¬x)`, then `⊥`, each RUP-derivable by propagation along the chain).
/// Otherwise returns the SCC representative for each literal, so equal literals can be substituted (a sound
/// variable-reduction speedup). Iterative Tarjan; nodes are the `2·num_vars` literals (`2v` = `+v`, `2v+1` = `¬v`).
fn lit_node(l: Lit) -> usize {
    (l.var() as usize) * 2 + if l.is_positive() { 0 } else { 1 }
}
pub enum EquivResult {
    /// `x ≡ ¬x` forced by the 2-clauses — UNSAT, with a RUP certificate.
    Unsat(Vec<ProofStep>),
    /// Per-literal SCC id; two literals with the same id are equivalent (substitutable).
    Classes(Vec<usize>),
}
pub fn equivalent_literal_scc(num_vars: usize, clauses: &[Vec<Lit>]) -> EquivResult {
    let n = num_vars * 2;
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for c in clauses {
        if c.len() == 2 {
            let (a, b) = (c[0], c[1]);
            adj[lit_node(a.negated())].push(lit_node(b)); // ¬a → b
            adj[lit_node(b.negated())].push(lit_node(a)); // ¬b → a
        }
    }
    // Iterative Tarjan SCC.
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut comp = vec![usize::MAX; n];
    let mut next_index = 0usize;
    let mut next_comp = 0usize;
    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(v, pi)) = call.last() {
            if pi == 0 {
                index[v] = next_index;
                low[v] = next_index;
                next_index += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if pi < adj[v].len() {
                let w = adj[v][pi];
                call.last_mut().unwrap().1 += 1;
                if index[w] == usize::MAX {
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                if low[v] == index[v] {
                    loop {
                        let w = stack.pop().unwrap();
                        on_stack[w] = false;
                        comp[w] = next_comp;
                        if w == v {
                            break;
                        }
                    }
                    next_comp += 1;
                }
                call.pop();
                if let Some(&(p, _)) = call.last() {
                    low[p] = low[p].min(low[v]);
                }
            }
        }
    }
    for v in 0..num_vars {
        if comp[v * 2] == comp[v * 2 + 1] {
            let (x, nx) = (Lit::pos(v as Var), Lit::neg(v as Var));
            return EquivResult::Unsat(vec![ProofStep::Rup(vec![nx]), ProofStep::Rup(vec![x]), ProofStep::Rup(vec![])]);
        }
    }
    EquivResult::Classes(comp)
}

/// Full Davis–Putnam bucket elimination in min-degree order, allowing intermediate growth but capping every
/// resolvent at `width_cap`. If it reaches `⊥`, returns the RUP-resolvents-then-deletions proof — a
/// width-`≤width_cap` resolution refutation, i.e. a `2^width_cap·n` certificate that crushes every
/// bounded-treewidth family. Returns `None` the moment a resolvent would exceed the cap (high treewidth — the
/// certificate would be exponential, which is exactly the Ben-Sasson–Wigderson hardness). Sound: every step is
/// RUP/Delete and independently re-checkable by [`crate::pr::check_pr_refutation`].
pub fn bucket_elimination_refute(num_vars: usize, clauses: &[Vec<Lit>], width_cap: usize) -> Option<Vec<ProofStep>> {
    let mut cur: Vec<Vec<Lit>> = clauses.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();
    for _ in 0..=num_vars {
        if cur.iter().any(|c| c.is_empty()) {
            return Some(steps);
        }
        let mut vars: Vec<Var> = cur.iter().flatten().map(|l| l.var()).collect();
        vars.sort_unstable();
        vars.dedup();
        if vars.is_empty() {
            break;
        }
        let v = *vars.iter().min_by_key(|&&v| cur.iter().filter(|c| c.iter().any(|l| l.var() == v)).count()).unwrap();
        let (pos, neg) = (Lit::pos(v), Lit::neg(v));
        let pos_c: Vec<Vec<Lit>> = cur.iter().filter(|c| c.contains(&pos)).cloned().collect();
        let neg_c: Vec<Vec<Lit>> = cur.iter().filter(|c| c.contains(&neg)).cloned().collect();
        // Even under the min-degree order the resolvent product `|pos| · |neg|` can explode once a dense
        // (high-treewidth) core is reached — each resolvent stays narrow (so the width cap never trips)
        // yet there are quadratically many. Bail before doing that work: a bounded-treewidth family keeps
        // this product small, so declining here costs nothing there while a dense family (e.g. Ramsey)
        // exits in microseconds instead of grinding for seconds. The certificate would be exponential
        // anyway (the same Ben-Sasson–Wigderson hardness the width cap guards).
        if pos_c.len().saturating_mul(neg_c.len()) > 16_384 {
            return None;
        }
        let mut resolvents: Vec<Vec<Lit>> = Vec::new();
        for a in &pos_c {
            for b in &neg_c {
                if let Some(r) = resolve(a, b, v) {
                    if r.len() > width_cap {
                        return None; // exceeds the width cap — high treewidth, decline
                    }
                    resolvents.push(r);
                }
            }
        }
        for r in &resolvents {
            steps.push(ProofStep::Rup(r.clone()));
        }
        for c in pos_c.iter().chain(neg_c.iter()) {
            steps.push(ProofStep::Delete(c.clone()));
        }
        cur.retain(|c| !c.contains(&pos) && !c.contains(&neg));
        cur.extend(resolvents);
        if cur.len() > 50_000 {
            return None; // database blow-up guard
        }
    }
    if cur.iter().any(|c| c.is_empty()) {
        Some(steps)
    } else {
        None
    }
}

/// Vivify a clause: drop every literal whose removal leaves a clause still RUP-implied by `db`.
/// Returns the strengthened (shorter) clause, or `None` if nothing could be removed. The result
/// is a sub-clause logically equivalent in context, so it is verdict-invariant.
pub fn vivify_clause(num_vars: usize, db: &[Vec<Lit>], c: &[Lit]) -> Option<Vec<Lit>> {
    let mut cur = c.to_vec();
    let mut improved = false;
    let mut i = 0;
    while cur.len() > 1 && i < cur.len() {
        let mut without = cur.clone();
        without.remove(i);
        if crate::rup::is_rup(num_vars, db, &without) {
            cur = without; // this literal is redundant
            improved = true;
        } else {
            i += 1;
        }
    }
    improved.then_some(cur)
}

/// Vivify the whole database: replace each clause with its RUP-strengthened form, emitting an
/// `Rup(shorter)` then `Delete(original)` per improvement. Shorter clauses propagate and delete
/// better. The composed steps keep the run independently checkable.
pub fn vivify(num_vars: usize, clauses: &[Vec<Lit>]) -> (Vec<Vec<Lit>>, Vec<ProofStep>) {
    let mut cur = clauses.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();
    for idx in 0..cur.len() {
        let c = cur[idx].clone();
        if c.len() <= 1 {
            continue;
        }
        if let Some(c2) = vivify_clause(num_vars, &cur, &c) {
            steps.push(ProofStep::Rup(c2.clone()));
            steps.push(ProofStep::Delete(c.clone()));
            cur[idx] = c2;
        }
    }
    (cur, steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdcl::{SolveResult, Solver};
    use crate::families;
    use crate::pr::check_pr_refutation;

    fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
        (0u32..(1u32 << num_vars)).any(|mask| {
            let model: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
            clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
        })
    }

    #[test]
    fn bve_preserves_satisfiability_brute_force() {
        // Over many seeded random small CNFs, BVE must preserve satisfiability exactly.
        let mut state = 0xB5E_1234_5678u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2000 {
            let nv = 3 + (next() % 5) as usize; // 3..7
            let nc = (next() % 14) as usize;
            let clauses: Vec<Vec<Lit>> = (0..nc)
                .map(|_| {
                    let len = 1 + (next() % 3) as usize;
                    let mut c = Vec::new();
                    for _ in 0..len {
                        let l = Lit::new((next() % nv as u64) as Var, next() & 1 == 0);
                        if !c.contains(&l) && !c.contains(&l.negated()) {
                            c.push(l);
                        }
                    }
                    c
                })
                .filter(|c: &Vec<Lit>| !c.is_empty())
                .collect();
            let (simplified, _) = bve(nv, &clauses);
            assert_eq!(
                sat_brute(nv, &clauses),
                sat_brute(nv, &simplified),
                "BVE changed satisfiability for {clauses:?}"
            );
        }
    }

    #[test]
    fn bve_produces_a_certified_refutation_of_php() {
        // BVE-simplify PHP(n), solve the result, and verify the COMPOSED proof (BVE resolvents +
        // deletions + the solver's RUP learned clauses) refutes the ORIGINAL formula.
        for n in 2..=4 {
            let (cnf, _) = families::php(n);
            let (simplified, mut steps) = bve(cnf.num_vars, &cnf.clauses);

            let mut solver = Solver::new(cnf.num_vars);
            for c in &simplified {
                solver.add_clause(c.clone());
            }
            assert_eq!(solver.solve(), SolveResult::Unsat, "simplified PHP({n}) is UNSAT");
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            assert!(
                check_pr_refutation(cnf.num_vars, &cnf.clauses, &steps),
                "BVE-composed proof must refute the original PHP({n})"
            );
        }
    }

    #[test]
    fn eliminate_variable_yields_the_resolvent() {
        // (x ∨ a) ∧ (¬x ∨ b): eliminating x yields the single resolvent (a ∨ b), emitting one RUP
        // step and two deletions. (Full `bve` would then also clear a, b as pure literals — this
        // pins the single-variable step, where the resolvent is observable.)
        let (x, a, b) = (0u32, 1u32, 2u32);
        let clauses = vec![vec![Lit::pos(x), Lit::pos(a)], vec![Lit::neg(x), Lit::pos(b)]];
        let (simplified, steps) = eliminate_variable(&clauses, x).expect("x is eliminable");
        assert!(simplified.iter().all(|c| !c.iter().any(|l| l.var() == x)), "x is gone");
        assert!(simplified.contains(&vec![Lit::pos(a), Lit::pos(b)]), "resolvent (a ∨ b) present");
        assert!(steps.iter().any(|s| matches!(s, ProofStep::Rup(_))), "emits the RUP resolvent");
        assert_eq!(steps.iter().filter(|s| matches!(s, ProofStep::Delete(_))).count(), 2, "deletes both x-clauses");
    }

    #[test]
    fn vivify_strengthens_via_propagation() {
        // {(a ∨ b), (¬a ∨ b)} together imply b, so (a ∨ b) vivifies down to the unit (b).
        let (a, b) = (0u32, 1u32);
        let db = vec![vec![Lit::pos(a), Lit::pos(b)], vec![Lit::neg(a), Lit::pos(b)]];
        let c2 = vivify_clause(2, &db, &db[0]).expect("the clause is strengthenable");
        assert_eq!(c2, vec![Lit::pos(b)], "vivified to the unit (b)");
    }

    #[test]
    fn vivify_preserves_satisfiability_brute_force() {
        let mut state = 0x171F_1ED_9999u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2000 {
            let nv = 3 + (next() % 5) as usize;
            let nc = (next() % 14) as usize;
            let clauses: Vec<Vec<Lit>> = (0..nc)
                .map(|_| {
                    let len = 1 + (next() % 3) as usize;
                    let mut c = Vec::new();
                    for _ in 0..len {
                        let l = Lit::new((next() % nv as u64) as Var, next() & 1 == 0);
                        if !c.contains(&l) && !c.contains(&l.negated()) {
                            c.push(l);
                        }
                    }
                    c
                })
                .filter(|c: &Vec<Lit>| !c.is_empty())
                .collect();
            let (simplified, _) = vivify(nv, &clauses);
            assert_eq!(sat_brute(nv, &clauses), sat_brute(nv, &simplified), "vivify changed SAT for {clauses:?}");
        }
    }

    #[test]
    fn vivify_produces_a_certified_refutation_of_php() {
        for n in 2..=4 {
            let (cnf, _) = families::php(n);
            let (simplified, mut steps) = vivify(cnf.num_vars, &cnf.clauses);
            let mut solver = Solver::new(cnf.num_vars);
            for c in &simplified {
                solver.add_clause(c.clone());
            }
            assert_eq!(solver.solve(), SolveResult::Unsat);
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            assert!(
                check_pr_refutation(cnf.num_vars, &cnf.clauses, &steps),
                "vivify-composed proof must refute the original PHP({n})"
            );
        }
    }

    #[test]
    fn symmetry_then_bve_then_vivify_compose_into_one_certified_refutation() {
        // THE ultimate stack, all in one machine-checked proof: certified symmetry breaking, then
        // certified BVE, then certified vivification, then the solver — refuting PHP against the
        // original formula.
        for n in 2..=4 {
            let (cnf, _) = families::php(n);
            let gens = crate::symmetry_detect::find_generators(cnf.num_vars, &cnf.clauses);
            // 1) certified symmetry breaking (PR steps over the original).
            let (sb_db, nv, mut steps) =
                crate::sym_certify::symmetry_break_certified(cnf.num_vars, &cnf.clauses, &gens);
            // 2) certified BVE on top.
            let (bve_db, bve_steps) = bve(nv, &sb_db);
            steps.extend(bve_steps);
            // 3) certified vivification.
            let (viv_db, viv_steps) = vivify(nv, &bve_db);
            steps.extend(viv_steps);
            // 4) solve and append RUP.
            let mut solver = Solver::new(nv);
            for c in &viv_db {
                solver.add_clause(c.clone());
            }
            assert_eq!(solver.solve(), SolveResult::Unsat, "fully-simplified PHP({n}) is UNSAT");
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            assert!(
                check_pr_refutation(nv, &cnf.clauses, &steps),
                "symmetry + BVE + vivify + solve must compose into ONE certified refutation of PHP({n})"
            );
        }
    }
}
