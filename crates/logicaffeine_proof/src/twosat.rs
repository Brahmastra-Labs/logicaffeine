//! 2-SAT in linear time via the implication graph + strongly-connected components.
//!
//! A clause of at most two literals `(a ∨ b)` is equivalent to the two implications `¬a → b` and
//! `¬b → a`. Over all clauses these form an implication graph on the `2n` literals; the formula is
//! unsatisfiable **iff some variable `x` lies in the same SCC as `¬x`** (so `x → ¬x` and `¬x → x`,
//! forcing `x` both ways). Otherwise a model is read off the SCC condensation's topological order.
//! Kosaraju's two-pass SCC makes the whole decision O(n+m) — and certified: a model is re-checkable,
//! and an `Unsat` returns the conflicting variable, whose mutual implication [`is_refutation`]
//! independently re-derives by reachability.

/// A literal: variable `var`, positive when `pos`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lit {
    /// The variable index (`0..num_vars`).
    pub var: usize,
    /// `true` for `x`, `false` for `¬x`.
    pub pos: bool,
}

impl Lit {
    /// Positive literal `x`.
    pub fn pos(var: usize) -> Self {
        Lit { var, pos: true }
    }
    /// Negative literal `¬x`.
    pub fn neg(var: usize) -> Self {
        Lit { var, pos: false }
    }
}

/// The outcome of solving a 2-SAT instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TwoSatOutcome {
    /// Satisfiable, with an assignment over `0..num_vars` (re-checkable via [`satisfies`]).
    Sat(Vec<bool>),
    /// Unsatisfiable: the variable forced both true and false (`x` and `¬x` in one SCC). Its mutual
    /// implication is re-checkable via [`is_refutation`].
    Unsat(usize),
}

// A literal's node in the implication graph: `2*var + pos`. The negation flips the low bit.
#[inline]
fn node(l: Lit) -> usize {
    2 * l.var + l.pos as usize
}
#[inline]
fn neg_node(n: usize) -> usize {
    n ^ 1
}

fn dfs1(v: usize, adj: &[Vec<usize>], visited: &mut [bool], order: &mut Vec<usize>) {
    visited[v] = true;
    for &u in &adj[v] {
        if !visited[u] {
            dfs1(u, adj, visited, order);
        }
    }
    order.push(v);
}

fn dfs2(v: usize, radj: &[Vec<usize>], comp: &mut [usize], c: usize) {
    comp[v] = c;
    for &u in &radj[v] {
        if comp[u] == usize::MAX {
            dfs2(u, radj, comp, c);
        }
    }
}

/// Decide a 2-SAT instance (`clauses` of two literals each — a unit clause is `(a, a)`). Returns a
/// satisfying assignment, or the variable whose SCC contains both polarities.
pub fn solve(clauses: &[(Lit, Lit)], num_vars: usize) -> TwoSatOutcome {
    let nn = 2 * num_vars;
    let mut adj = vec![Vec::new(); nn];
    let mut radj = vec![Vec::new(); nn];
    let mut edge = |from: usize, to: usize| {
        adj[from].push(to);
        radj[to].push(from);
    };
    for &(a, b) in clauses {
        if a.var >= num_vars || b.var >= num_vars {
            continue;
        }
        // (a ∨ b): ¬a → b and ¬b → a.
        edge(neg_node(node(a)), node(b));
        edge(neg_node(node(b)), node(a));
    }
    // Kosaraju: finish-order pass, then components in reverse finish order (topological).
    let mut visited = vec![false; nn];
    let mut order = Vec::with_capacity(nn);
    for v in 0..nn {
        if !visited[v] {
            dfs1(v, &adj, &mut visited, &mut order);
        }
    }
    let mut comp = vec![usize::MAX; nn];
    let mut c = 0;
    for &v in order.iter().rev() {
        if comp[v] == usize::MAX {
            dfs2(v, &radj, &mut comp, c);
            c += 1;
        }
    }
    // Conflict: x and ¬x share an SCC.
    for v in 0..num_vars {
        if comp[2 * v] == comp[2 * v + 1] {
            return TwoSatOutcome::Unsat(v);
        }
    }
    // Model: the literal whose SCC is later in topological order (larger Kosaraju id) is true.
    let assignment = (0..num_vars)
        .map(|v| comp[node(Lit::pos(v))] > comp[node(Lit::neg(v))])
        .collect();
    TwoSatOutcome::Sat(assignment)
}

/// Re-check a satisfying assignment: every clause has a true literal.
pub fn satisfies(clauses: &[(Lit, Lit)], assignment: &[bool]) -> bool {
    let holds = |l: Lit| l.var < assignment.len() && assignment[l.var] == l.pos;
    clauses.iter().all(|&(a, b)| holds(a) || holds(b))
}

/// Re-check an `Unsat` witness: in the implication graph, `x` reaches `¬x` *and* `¬x` reaches `x`
/// (mutual implication ⇒ no value of `x` is consistent). A solver-free certificate.
pub fn is_refutation(clauses: &[(Lit, Lit)], num_vars: usize, var: usize) -> bool {
    if var >= num_vars {
        return false;
    }
    let nn = 2 * num_vars;
    let mut adj = vec![Vec::new(); nn];
    for &(a, b) in clauses {
        if a.var < num_vars && b.var < num_vars {
            adj[neg_node(node(a))].push(node(b));
            adj[neg_node(node(b))].push(node(a));
        }
    }
    let reaches = |from: usize, to: usize| {
        let mut seen = vec![false; nn];
        let mut stack = vec![from];
        seen[from] = true;
        while let Some(u) = stack.pop() {
            if u == to {
                return true;
            }
            for &w in &adj[u] {
                if !seen[w] {
                    seen[w] = true;
                    stack.push(w);
                }
            }
        }
        from == to
    };
    let xt = node(Lit::pos(var));
    let xf = node(Lit::neg(var));
    reaches(xt, xf) && reaches(xf, xt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cl(a: Lit, b: Lit) -> (Lit, Lit) {
        (a, b)
    }

    #[test]
    fn simple_satisfiable_instance() {
        // (x0 ∨ x1) ∧ (¬x0 ∨ x1) ∧ (¬x1 ∨ x2) — satisfiable.
        let cs = vec![
            cl(Lit::pos(0), Lit::pos(1)),
            cl(Lit::neg(0), Lit::pos(1)),
            cl(Lit::neg(1), Lit::pos(2)),
        ];
        match solve(&cs, 3) {
            TwoSatOutcome::Sat(a) => assert!(satisfies(&cs, &a), "model must satisfy: {a:?}"),
            o => panic!("expected Sat, got {o:?}"),
        }
    }

    #[test]
    fn forced_contradiction_is_unsat() {
        // (x0)∧(¬x0) as units: (x0∨x0) and (¬x0∨¬x0) ⇒ x0 forced both ways.
        let cs = vec![cl(Lit::pos(0), Lit::pos(0)), cl(Lit::neg(0), Lit::neg(0))];
        match solve(&cs, 1) {
            TwoSatOutcome::Unsat(v) => {
                assert_eq!(v, 0);
                assert!(is_refutation(&cs, 1, v), "refutation must re-check");
            }
            o => panic!("expected Unsat, got {o:?}"),
        }
    }

    #[test]
    fn implication_cycle_is_unsat() {
        // x0→x1, x1→¬x0, ¬x0→x0 collapses {x0,¬x0,x1} — classic 2-SAT contradiction.
        // (¬x0∨x1)=x0→x1 ; (¬x1∨¬x0)=x1→¬x0 ; (x0∨x0)=¬x0→x0.
        let cs = vec![
            cl(Lit::neg(0), Lit::pos(1)),
            cl(Lit::neg(1), Lit::neg(0)),
            cl(Lit::pos(0), Lit::pos(0)),
        ];
        match solve(&cs, 2) {
            TwoSatOutcome::Unsat(v) => assert!(is_refutation(&cs, 2, v)),
            o => panic!("expected Unsat, got {o:?}"),
        }
    }

    #[test]
    fn empty_is_satisfiable() {
        assert!(matches!(solve(&[], 3), TwoSatOutcome::Sat(_)));
    }

    #[test]
    fn matches_brute_force_on_random_2sat() {
        let mut s: u64 = 0x2545F4914F6CDD1D;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..500 {
            let num_vars = (next() % 6) as usize + 1;
            let m = (next() % 10) as usize + 1;
            let lit = |r: u64, nv: usize| Lit {
                var: (r as usize) % nv,
                pos: (r >> 8) & 1 == 1,
            };
            let cs: Vec<(Lit, Lit)> = (0..m)
                .map(|_| (lit(next(), num_vars), lit(next(), num_vars)))
                .collect();
            let brute_sat = (0..(1u32 << num_vars)).any(|mask| {
                let a: Vec<bool> = (0..num_vars).map(|i| (mask >> i) & 1 == 1).collect();
                satisfies(&cs, &a)
            });
            match solve(&cs, num_vars) {
                TwoSatOutcome::Sat(a) => {
                    assert!(brute_sat, "we said SAT, brute force UNSAT: {cs:?}");
                    assert!(satisfies(&cs, &a), "model is wrong: {a:?} for {cs:?}");
                }
                TwoSatOutcome::Unsat(v) => {
                    assert!(!brute_sat, "we said UNSAT, brute force SAT: {cs:?}");
                    assert!(is_refutation(&cs, num_vars, v), "bogus refutation var={v}");
                }
            }
        }
    }
}
