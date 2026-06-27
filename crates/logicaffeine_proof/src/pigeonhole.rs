//! Pigeonhole / bipartite-matching detection for the general solver — the cardinality reasoning
//! that lets `prove_unsat` win on pigeonhole-shaped formulas in POLYNOMIAL time.
//!
//! A conjunction of "each item is in at least one slot" (positive disjunctions) and "each slot
//! holds at most one item" (pairwise mutual-exclusion clauses) is a bipartite-matching feasibility
//! question. Encoded as boolean SAT it needs *exponentially* long resolution refutations — the
//! classic wall for CDCL (ours and Z3's). But the underlying matching is decided in *polynomial*
//! time with a certified Hall witness ([`crate::matching`]).
//!
//! This module recognizes that structure SOUNDLY — a faithful, fully-verified decomposition or it
//! bails to `None` — and routes the UNSAT case to the matching reasoner. **Soundness:** a
//! satisfying assignment of (at-least-one rows ∧ fully-encoded at-most-one columns) is *exactly* a
//! perfect matching of items to slots (each item ≥1 true variable, each slot ≤1), so "no perfect
//! matching" (a re-verified Hall witness) ⟺ UNSAT. A `true` from [`decide_pigeonhole_unsat`] is
//! therefore always a genuine, witnessed refutation; everything else falls back to CDCL.

use crate::matching::{assign_or_hall, is_hall_witness, MatchOutcome};
use crate::ProofExpr;
use std::collections::{HashMap, HashSet};

/// Decide whether `e` is a clean pigeonhole structure that is UNSAT. Returns `true` ONLY when the
/// formula decomposes faithfully into at-least-one rows + fully-encoded (clique) at-most-one columns
/// AND the bipartite matching is infeasible with a RE-VERIFIED Hall witness. `false` otherwise — for
/// a non-pigeonhole formula, or a feasible one (the caller falls back to CDCL). **Never a false
/// `true`.**
pub fn decide_pigeonhole_unsat(e: &ProofExpr) -> bool {
    let Some((adj, num_slots)) = extract_bipartite(e) else {
        return false;
    };
    match assign_or_hall(&adj, num_slots) {
        MatchOutcome::Infeasible(w) => is_hall_witness(&adj, &w),
        MatchOutcome::Feasible(_) => false,
    }
}

/// Recover `(item → reachable slots, slot count)` from `e`, or `None` if `e` is not a faithful
/// pigeonhole conjunction. Conservative: any clause that is neither an at-least-one row nor a
/// binary at-most-one exclusion, any variable in two rows, an exclusion over an unknown variable,
/// or an at-most-one group that is not a full clique → `None`.
fn extract_bipartite(e: &ProofExpr) -> Option<(Vec<Vec<usize>>, usize)> {
    let mut clauses = Vec::new();
    flatten_and(e, &mut clauses);
    if clauses.is_empty() {
        return None;
    }

    let mut rows: Vec<Vec<String>> = Vec::new(); // each item's candidate variables
    let mut excl: Vec<(String, String)> = Vec::new(); // mutual-exclusion (same-slot) pairs
    for c in &clauses {
        if let Some(atoms) = positive_disjunction(c) {
            if atoms.is_empty() {
                return None;
            }
            rows.push(atoms);
        } else if let Some(pair) = exclusion_pair(c) {
            excl.push(pair);
        } else {
            return None;
        }
    }
    if rows.is_empty() {
        return None;
    }

    // Every variable must appear in EXACTLY ONE row (its item) — a clean item partition.
    let mut item_of: HashMap<String, usize> = HashMap::new();
    for (i, row) in rows.iter().enumerate() {
        for a in row {
            if item_of.insert(a.clone(), i).is_some() {
                return None;
            }
        }
    }

    // Union-find over variables joined by exclusion → slot components.
    let vars: Vec<String> = item_of.keys().cloned().collect();
    let idx: HashMap<&str, usize> = vars.iter().enumerate().map(|(i, v)| (v.as_str(), i)).collect();
    let mut uf = UnionFind::new(vars.len());
    for (a, b) in &excl {
        let (Some(&ia), Some(&ib)) = (idx.get(a.as_str()), idx.get(b.as_str())) else {
            return None; // exclusion over a variable not in any row
        };
        uf.union(ia, ib);
    }

    // Compact component ids → slots; record members.
    let mut slot_id: HashMap<usize, usize> = HashMap::new();
    let mut slot_members: Vec<Vec<usize>> = Vec::new();
    let mut slot_of: Vec<usize> = vec![0; vars.len()];
    for v in 0..vars.len() {
        let root = uf.find(v);
        let s = *slot_id.entry(root).or_insert_with(|| {
            slot_members.push(Vec::new());
            slot_members.len() - 1
        });
        slot_of[v] = s;
        slot_members[s].push(v);
    }

    // Each multi-member slot must be a FULL clique of exclusions (so "at most one" is genuinely
    // enforced — otherwise two items could share a slot and infeasibility wouldn't imply UNSAT).
    let excl_set: HashSet<(usize, usize)> = excl
        .iter()
        .filter_map(|(a, b)| {
            let ia = *idx.get(a.as_str())?;
            let ib = *idx.get(b.as_str())?;
            Some((ia.min(ib), ia.max(ib)))
        })
        .collect();
    for members in &slot_members {
        for i in 0..members.len() {
            for j in (i + 1)..members.len() {
                let key = (members[i].min(members[j]), members[i].max(members[j]));
                if !excl_set.contains(&key) {
                    return None;
                }
            }
        }
    }

    // Build item → reachable slots.
    let num_slots = slot_members.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
    for (i, row) in rows.iter().enumerate() {
        for a in row {
            let s = slot_of[*idx.get(a.as_str()).unwrap()];
            if !adj[i].contains(&s) {
                adj[i].push(s);
            }
        }
    }
    Some((adj, num_slots))
}

fn flatten_and<'a>(e: &'a ProofExpr, out: &mut Vec<&'a ProofExpr>) {
    match e {
        ProofExpr::And(l, r) => {
            flatten_and(l, out);
            flatten_and(r, out);
        }
        other => out.push(other),
    }
}

/// `Some(atoms)` if `e` is a disjunction of POSITIVE atoms (an at-least-one row); else `None`.
fn positive_disjunction(e: &ProofExpr) -> Option<Vec<String>> {
    fn walk(e: &ProofExpr, out: &mut Vec<String>) -> bool {
        match e {
            ProofExpr::Or(l, r) => walk(l, out) && walk(r, out),
            ProofExpr::Atom(a) => {
                out.push(a.clone());
                true
            }
            _ => false,
        }
    }
    let mut atoms = Vec::new();
    walk(e, &mut atoms).then_some(atoms)
}

/// `Some((a, b))` if `e` is a binary mutual-exclusion `¬(a ∧ b)` or `(¬a ∨ ¬b)` over atoms.
fn exclusion_pair(e: &ProofExpr) -> Option<(String, String)> {
    match e {
        ProofExpr::Not(inner) => match inner.as_ref() {
            ProofExpr::And(a, b) => match (a.as_ref(), b.as_ref()) {
                (ProofExpr::Atom(a), ProofExpr::Atom(b)) => Some((a.clone(), b.clone())),
                _ => None,
            },
            _ => None,
        },
        ProofExpr::Or(l, r) => match (l.as_ref(), r.as_ref()) {
            (ProofExpr::Not(a), ProofExpr::Not(b)) => match (a.as_ref(), b.as_ref()) {
                (ProofExpr::Atom(a), ProofExpr::Atom(b)) => Some((a.clone(), b.clone())),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

struct UnionFind {
    parent: Vec<usize>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind { parent: (0..n).collect() }
    }
    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            let r = self.find(self.parent[x]);
            self.parent[x] = r;
        }
        self.parent[x]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(s: &str) -> ProofExpr {
        ProofExpr::Atom(s.to_string())
    }
    fn or_all(v: Vec<ProofExpr>) -> ProofExpr {
        v.into_iter()
            .reduce(|a, b| ProofExpr::Or(Box::new(a), Box::new(b)))
            .unwrap()
    }
    fn and_all(v: Vec<ProofExpr>) -> ProofExpr {
        v.into_iter()
            .reduce(|a, b| ProofExpr::And(Box::new(a), Box::new(b)))
            .unwrap()
    }
    fn excl(a: &str, b: &str) -> ProofExpr {
        ProofExpr::Not(Box::new(ProofExpr::And(Box::new(atom(a)), Box::new(atom(b)))))
    }

    /// PHP(n, n-1): n pigeons, n-1 holes, pairwise at-most-one. UNSAT.
    fn php(n: usize) -> ProofExpr {
        let holes = n - 1;
        let p = |i: usize, h: usize| format!("p_{i}_{h}");
        let mut clauses = Vec::new();
        for i in 0..n {
            clauses.push(or_all((0..holes).map(|h| atom(&p(i, h))).collect()));
        }
        for h in 0..holes {
            for i in 0..n {
                for j in (i + 1)..n {
                    clauses.push(excl(&p(i, h), &p(j, h)));
                }
            }
        }
        and_all(clauses)
    }

    /// A FEASIBLE bipartite problem: n items, n slots — SAT. Must NOT be reported UNSAT.
    fn feasible(n: usize) -> ProofExpr {
        let p = |i: usize, h: usize| format!("q_{i}_{h}");
        let mut clauses = Vec::new();
        for i in 0..n {
            clauses.push(or_all((0..n).map(|h| atom(&p(i, h))).collect()));
        }
        for h in 0..n {
            for i in 0..n {
                for j in (i + 1)..n {
                    clauses.push(excl(&p(i, h), &p(j, h)));
                }
            }
        }
        and_all(clauses)
    }

    #[test]
    fn php_is_decided_unsat() {
        for n in 2..=12 {
            assert!(decide_pigeonhole_unsat(&php(n)), "PHP({n}) must be decided UNSAT via matching");
        }
    }

    #[test]
    fn feasible_is_not_reported_unsat() {
        // Soundness-critical: a SATISFIABLE bipartite formula must NEVER be reported UNSAT.
        for n in 1..=10 {
            assert!(!decide_pigeonhole_unsat(&feasible(n)), "feasible({n}) must NOT be UNSAT");
        }
    }

    #[test]
    fn php2_edge_case() {
        // 2 pigeons, 1 hole — the smallest pigeonhole.
        assert!(decide_pigeonhole_unsat(&php(2)));
    }

    #[test]
    fn non_pigeonhole_falls_back() {
        // A plain conjunction that is not pigeonhole-shaped: a unit positive, a unit negative.
        // Not our pattern → false (caller uses CDCL). (`a ∧ ¬a` is UNSAT but NOT via matching.)
        let f = ProofExpr::And(Box::new(atom("a")), Box::new(ProofExpr::Not(Box::new(atom("a")))));
        assert!(!decide_pigeonhole_unsat(&f), "non-pigeonhole must fall back, not claim a matching refutation");
    }

    #[test]
    fn incomplete_at_most_one_falls_back() {
        // Soundness: if a slot's at-most-one is only PARTIALLY encoded (missing a pair), two items
        // could share it, so infeasibility wouldn't imply UNSAT — we must bail, not claim UNSAT.
        // 3 pigeons, 2 holes, but hole 0 omits the (p1,p2) exclusion → not a clique → fall back.
        let p = |i: usize, h: usize| format!("p_{i}_{h}");
        let mut clauses = Vec::new();
        for i in 0..3 {
            clauses.push(or_all((0..2).map(|h| atom(&p(i, h))).collect()));
        }
        // hole 0: only (0,1) and (0,2) — MISSING (1,2)
        clauses.push(excl(&p(0, 0), &p(1, 0)));
        clauses.push(excl(&p(0, 0), &p(2, 0)));
        // hole 1: full clique
        clauses.push(excl(&p(0, 1), &p(1, 1)));
        clauses.push(excl(&p(0, 1), &p(2, 1)));
        clauses.push(excl(&p(1, 1), &p(2, 1)));
        assert!(!decide_pigeonhole_unsat(&and_all(clauses)), "incomplete at-most-one must fall back");
    }

    #[test]
    fn demorgan_exclusion_form_is_recognized() {
        // `¬a ∨ ¬b` is the same at-most-one as `¬(a ∧ b)` — must be recognized too.
        let p = |i: usize, h: usize| format!("p_{i}_{h}");
        let dm = |a: &str, b: &str| {
            ProofExpr::Or(
                Box::new(ProofExpr::Not(Box::new(atom(a)))),
                Box::new(ProofExpr::Not(Box::new(atom(b)))),
            )
        };
        let n = 3;
        let holes = n - 1;
        let mut clauses = Vec::new();
        for i in 0..n {
            clauses.push(or_all((0..holes).map(|h| atom(&p(i, h))).collect()));
        }
        for h in 0..holes {
            for i in 0..n {
                for j in (i + 1)..n {
                    clauses.push(dm(&p(i, h), &p(j, h)));
                }
            }
        }
        assert!(decide_pigeonhole_unsat(&and_all(clauses)), "De Morgan at-most-one PHP must be UNSAT");
    }
}
