//! Horn-SAT in linear time via unit propagation (forward chaining).
//!
//! A Horn clause has at most one positive literal, so it reads as a definite implication
//! `(body₁ ∧ … ∧ bodyₖ) → head` (or, with no positive literal, a goal `(body₁ ∧ …) → false`).
//! Such a system has a unique **least model**, computed by O(n+m) forward chaining: start all-false
//! and fire each implication whose body is fully established. The system is satisfiable iff the
//! least model violates no goal clause. Both verdicts are certified — the least model is
//! re-checkable, and an unsatisfiable system yields the derivation (the clauses that force a goal's
//! body true) which [`is_refutation`] replays independently.

use std::collections::VecDeque;

/// A Horn clause `(body ⇒ head)`: the conjunction of the `body` variables implies `head`, or — when
/// `head` is `None` — implies false (a goal/integrity clause).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HornClause {
    /// Positive body variables (the antecedent conjunction).
    pub body: Vec<usize>,
    /// The implied variable, or `None` for a goal clause `body ⇒ false`.
    pub head: Option<usize>,
}

impl HornClause {
    /// A definite rule `body ⇒ head`.
    pub fn rule(body: impl Into<Vec<usize>>, head: usize) -> Self {
        HornClause { body: body.into(), head: Some(head) }
    }
    /// A fact `⇒ head` (empty body).
    pub fn fact(head: usize) -> Self {
        HornClause { body: Vec::new(), head: Some(head) }
    }
    /// A goal `body ⇒ false`.
    pub fn goal(body: impl Into<Vec<usize>>) -> Self {
        HornClause { body: body.into(), head: None }
    }
}

/// The outcome of solving a Horn system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HornOutcome {
    /// Satisfiable, with the **least** model (re-checkable via [`satisfies`]).
    Sat(Vec<bool>),
    /// Unsatisfiable, witnessed by the clause indices whose forward-chaining forces a goal's body
    /// fully true (re-checkable via [`is_refutation`]).
    Unsat(Vec<usize>),
}

/// Solve a Horn system over `0..num_vars` by forward chaining. Returns the least model, or — if a
/// goal clause is forced — a certified derivation. Linear in the total clause size.
pub fn solve(clauses: &[HornClause], num_vars: usize) -> HornOutcome {
    let mut val = vec![false; num_vars];
    let mut forced_by = vec![usize::MAX; num_vars]; // the clause that first set each variable true
    // Count of not-yet-true body variables per clause; a clause is ready to fire at count 0.
    let mut remaining: Vec<usize> = clauses
        .iter()
        .map(|c| c.body.iter().filter(|&&b| b < num_vars).count())
        .collect();
    let mut in_body: Vec<Vec<usize>> = vec![Vec::new(); num_vars];
    for (i, c) in clauses.iter().enumerate() {
        for &b in &c.body {
            if b < num_vars {
                in_body[b].push(i);
            }
        }
    }
    let mut ready: VecDeque<usize> =
        (0..clauses.len()).filter(|&i| remaining[i] == 0).collect();
    while let Some(ci) = ready.pop_front() {
        match clauses[ci].head {
            None => {
                // A goal clause with a fully-established body — the system is unsatisfiable.
                return HornOutcome::Unsat(derivation(clauses, ci, &forced_by));
            }
            Some(h) => {
                if h < num_vars && !val[h] {
                    val[h] = true;
                    forced_by[h] = ci;
                    for &cj in &in_body[h] {
                        remaining[cj] -= 1;
                        if remaining[cj] == 0 {
                            ready.push_back(cj);
                        }
                    }
                }
            }
        }
    }
    HornOutcome::Sat(val)
}

/// The transitive set of clauses supporting the conflict at goal clause `goal_ci`: the goal plus,
/// recursively, the clause that forced each body variable.
fn derivation(clauses: &[HornClause], goal_ci: usize, forced_by: &[usize]) -> Vec<usize> {
    let mut used = vec![goal_ci];
    let mut seen_clause: std::collections::HashSet<usize> = std::iter::once(goal_ci).collect();
    let mut seen_var: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut stack: Vec<usize> = clauses[goal_ci].body.clone();
    while let Some(v) = stack.pop() {
        if !seen_var.insert(v) {
            continue;
        }
        let fc = forced_by.get(v).copied().unwrap_or(usize::MAX);
        if fc != usize::MAX && seen_clause.insert(fc) {
            used.push(fc);
            stack.extend(clauses[fc].body.iter().copied());
        }
    }
    used
}

/// Re-check a satisfying model: every clause holds (a rule with a true body has a true head; a goal
/// has a false body).
pub fn satisfies(clauses: &[HornClause], assignment: &[bool]) -> bool {
    clauses.iter().all(|c| {
        let body_true = c.body.iter().all(|&b| b < assignment.len() && assignment[b]);
        match c.head {
            Some(h) => !body_true || (h < assignment.len() && assignment[h]),
            None => !body_true,
        }
    })
}

/// Re-check a refutation: replaying *only* the listed clauses by forward chaining forces some goal
/// clause's body fully true (a contradiction). A solver-free certificate of unsatisfiability.
pub fn is_refutation(clauses: &[HornClause], num_vars: usize, refutation: &[usize]) -> bool {
    let mut val = vec![false; num_vars];
    loop {
        let mut changed = false;
        for &ci in refutation {
            let Some(c) = clauses.get(ci) else {
                return false;
            };
            if let Some(h) = c.head {
                let body_true = c.body.iter().all(|&b| b < num_vars && val[b]);
                if body_true && h < num_vars && !val[h] {
                    val[h] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    refutation.iter().any(|&ci| {
        clauses
            .get(ci)
            .is_some_and(|c| c.head.is_none() && c.body.iter().all(|&b| b < num_vars && val[b]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_and_rules_chain_to_a_least_model() {
        // ⇒a, ⇒b, (a∧b)⇒c, c⇒d  ⇒  {a,b,c,d} all true.
        let cs = vec![
            HornClause::fact(0),
            HornClause::fact(1),
            HornClause::rule([0, 1], 2),
            HornClause::rule([2], 3),
        ];
        match solve(&cs, 4) {
            HornOutcome::Sat(m) => {
                assert_eq!(m, vec![true, true, true, true]);
                assert!(satisfies(&cs, &m));
            }
            o => panic!("expected Sat, got {o:?}"),
        }
    }

    #[test]
    fn least_model_leaves_unforced_variables_false() {
        // Only a is a fact; b is never forced ⇒ least model {a}, not {a,b}.
        let cs = vec![HornClause::fact(0), HornClause::rule([1], 0)];
        match solve(&cs, 2) {
            HornOutcome::Sat(m) => {
                assert_eq!(m, vec![true, false]);
                assert!(satisfies(&cs, &m));
            }
            o => panic!("expected Sat, got {o:?}"),
        }
    }

    #[test]
    fn forced_goal_is_refuted_with_a_derivation() {
        // ⇒a, a⇒b, (a∧b)⇒false  — the goal's body is forced, so UNSAT.
        let cs = vec![
            HornClause::fact(0),
            HornClause::rule([0], 1),
            HornClause::goal([0, 1]),
        ];
        match solve(&cs, 2) {
            HornOutcome::Unsat(r) => {
                assert!(is_refutation(&cs, 2, &r), "refutation must re-check: {r:?}");
            }
            o => panic!("expected Unsat, got {o:?}"),
        }
    }

    #[test]
    fn an_unforced_goal_is_satisfiable() {
        // The goal needs b, which is never forced ⇒ SAT (least model {a}).
        let cs = vec![HornClause::fact(0), HornClause::goal([0, 1])];
        match solve(&cs, 2) {
            HornOutcome::Sat(m) => assert!(satisfies(&cs, &m)),
            o => panic!("expected Sat, got {o:?}"),
        }
    }

    #[test]
    fn matches_brute_force_on_random_horn_systems() {
        let mut s: u64 = 0xA0761D6478BD642F;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..400 {
            let num_vars = (next() % 6) as usize + 1;
            let m = (next() % 8) as usize + 1;
            let cs: Vec<HornClause> = (0..m)
                .map(|_| {
                    let body: Vec<usize> = (0..num_vars).filter(|_| next() % 3 == 0).collect();
                    // ~1/4 goal clauses, else a definite rule with a random head.
                    if next() % 4 == 0 {
                        HornClause::goal(body)
                    } else {
                        HornClause::rule(body, (next() as usize) % num_vars)
                    }
                })
                .collect();
            let brute_sat = (0..(1u32 << num_vars)).any(|mask| {
                let a: Vec<bool> = (0..num_vars).map(|i| (mask >> i) & 1 == 1).collect();
                satisfies(&cs, &a)
            });
            match solve(&cs, num_vars) {
                HornOutcome::Sat(m) => {
                    assert!(brute_sat, "we said SAT, brute force UNSAT: {cs:?}");
                    assert!(satisfies(&cs, &m), "least model is wrong: {m:?}");
                }
                HornOutcome::Unsat(r) => {
                    assert!(!brute_sat, "we said UNSAT, brute force SAT: {cs:?}");
                    assert!(is_refutation(&cs, num_vars, &r), "bogus refutation {r:?}");
                }
            }
        }
    }
}
