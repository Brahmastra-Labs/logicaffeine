//! Symmetry breaking for the general solver — the second pillar (with [`crate::pigeonhole`]
//! cardinality reasoning) for winning on pigeonhole.
//!
//! CDCL refutes a *symmetric* UNSAT formula by re-deriving the same conflict once per symmetric
//! copy of the problem — for the pigeonhole principle the pigeons are interchangeable, so the
//! refutation cost is multiplied by `n!`. Adding sound **symmetry-breaking predicates** (SBPs)
//! collapses each symmetry orbit to a single lexicographically-least representative, so the solver
//! searches the quotient instead of the whole symmetric space.
//!
//! We detect a cheap, *sound* class: ROW-INTERCHANGE symmetries. The at-least-one rows of a
//! pigeonhole-shaped formula (positive clauses over disjoint variable blocks) are candidate
//! interchangeable units; for each adjacent pair we **verify** the column-aligned row swap is a
//! genuine automorphism of the entire clause set (apply the swap, check the clause multiset is
//! invariant) before adding the lex-leader "row i ≤ₗₑₓ row i+1" SBP.
//!
//! **Soundness.** A verified automorphism σ leaves the formula F invariant, so the lex-leader SBP_σ
//! preserves satisfiability (it keeps the lex-least model of each orbit). Hence `F ∧ SBP_σ` is UNSAT
//! **iff** F is — refuting the augmented formula refutes the original. The lex-leader encoding is
//! pinned exhaustively against a brute-force oracle, and we add an SBP *only* for a swap we have
//! proven is an automorphism; a wrong symmetry could delete the only model, so verified-or-nothing.

use crate::ProofExpr;
use std::collections::BTreeSet;

fn atom(s: String) -> ProofExpr {
    ProofExpr::Atom(s)
}
fn not(a: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(a))
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn implies(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}
fn iff(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Iff(Box::new(a), Box::new(b))
}

/// "X ≤ₗₑₓ Y" over equal-length boolean vectors (false < true), MSB-first: at the first index where
/// they differ, X has `false` and Y has `true`. Encoded with a chain of "equal-so-far" auxiliaries
/// `{aux}_eq_{k}` (distinct constraints must use distinct `aux`). Returns a tautology for empty/
/// length-1-or-shorter vectors handled position-by-position.
///
/// Correctness is pinned **exhaustively** against a brute-force oracle (every assignment, small m).
pub fn lex_le(x: &[ProofExpr], y: &[ProofExpr], aux: &str) -> ProofExpr {
    assert_eq!(x.len(), y.len(), "lex_le needs equal-length vectors");
    let m = x.len();
    if m == 0 {
        let t = atom(format!("{aux}_taut"));
        return or(t.clone(), not(t));
    }
    // eq_k ≙ "positions 0..k are all equal". eq_0 ↔ (x_0 ↔ y_0); eq_k ↔ eq_{k-1} ∧ (x_k ↔ y_k).
    let eq = |k: usize| atom(format!("{aux}_eq_{k}"));
    let mut clauses: Vec<ProofExpr> = Vec::new();
    // Position 0: x_0 ≤ y_0 unconditionally (no prefix to be equal).
    clauses.push(or(not(x[0].clone()), y[0].clone()));
    if m >= 2 {
        clauses.push(iff(eq(0), iff(x[0].clone(), y[0].clone())));
        for k in 1..m {
            // If the prefix 0..k-1 is equal, then x_k ≤ y_k.
            clauses.push(implies(eq(k - 1), or(not(x[k].clone()), y[k].clone())));
            if k < m - 1 {
                clauses.push(iff(eq(k), and(eq(k - 1), iff(x[k].clone(), y[k].clone()))));
            }
        }
    }
    clauses.into_iter().reduce(and).unwrap()
}

/// A literal: `(atom-name, is_positive)`.
type Lit = (String, bool);
/// A clause as a canonical set of literals (sorted, duplicate-free).
type Clause = BTreeSet<Lit>;

/// Convert `e` into a flat list of literal-clauses, or `None` if any top-level conjunct is not a
/// clause (a disjunction of literals, including the binary `a→b` and `¬(a∧b)` forms). Conservative:
/// a formula we cannot read as a clause set gets no symmetry breaking.
fn to_clauses(e: &ProofExpr) -> Option<Vec<Clause>> {
    let mut conjuncts: Vec<&ProofExpr> = Vec::new();
    fn flatten<'a>(e: &'a ProofExpr, out: &mut Vec<&'a ProofExpr>) {
        match e {
            ProofExpr::And(l, r) => {
                flatten(l, out);
                flatten(r, out);
            }
            other => out.push(other),
        }
    }
    flatten(e, &mut conjuncts);
    let mut clauses = Vec::with_capacity(conjuncts.len());
    for c in conjuncts {
        clauses.push(clause_of(c)?);
    }
    Some(clauses)
}

/// Read a single clause (disjunction of literals) out of `e`, normalizing `a→b` to `¬a∨b` and the
/// binary `¬(a∧b)` to `¬a∨¬b`. `None` if `e` is not such a clause.
fn clause_of(e: &ProofExpr) -> Option<Clause> {
    let mut lits = BTreeSet::new();
    if collect_lits(e, true, &mut lits) {
        Some(lits)
    } else {
        None
    }
}

fn collect_lits(e: &ProofExpr, pos: bool, out: &mut Clause) -> bool {
    match e {
        ProofExpr::Atom(a) => {
            out.insert((a.clone(), pos));
            true
        }
        ProofExpr::Not(inner) => match inner.as_ref() {
            ProofExpr::Atom(a) => {
                out.insert((a.clone(), !pos));
                true
            }
            // ¬(a ∧ b) = ¬a ∨ ¬b ; ¬(a ∨ b) = ¬a ∧ ¬b (not a clause unless pos flips it back).
            ProofExpr::And(l, r) if pos => collect_lits(l, false, out) && collect_lits(r, false, out),
            ProofExpr::Or(l, r) if !pos => collect_lits(l, false, out) && collect_lits(r, false, out),
            ProofExpr::Not(x) => collect_lits(x, pos, out),
            _ => false,
        },
        ProofExpr::Or(l, r) if pos => collect_lits(l, pos, out) && collect_lits(r, pos, out),
        ProofExpr::And(l, r) if !pos => collect_lits(l, pos, out) && collect_lits(r, pos, out),
        ProofExpr::Implies(l, r) if pos => collect_lits(l, false, out) && collect_lits(r, true, out),
        _ => false,
    }
}

/// The at-least-one ROWS of `clauses`: all-positive clauses, returned as ordered variable vectors.
/// Order within a row follows `BTreeSet` (name order) — stable and identical for every row, which is
/// the column alignment we verify below.
fn positive_rows(clauses: &[Clause]) -> Vec<Vec<String>> {
    clauses
        .iter()
        .filter(|c| c.len() >= 2 && c.iter().all(|(_, p)| *p))
        .map(|c| c.iter().map(|(a, _)| a.clone()).collect())
        .collect()
}

/// Apply the variable renaming `map` to a clause, returning its canonical (re-sorted) form.
fn rename(clause: &Clause, map: &std::collections::HashMap<&str, &str>) -> Clause {
    clause
        .iter()
        .map(|(a, p)| (map.get(a.as_str()).map(|s| s.to_string()).unwrap_or_else(|| a.clone()), *p))
        .collect()
}

/// Is the column-aligned swap of `row_a`↔`row_b` (position k ↔ position k) a genuine automorphism
/// of the whole clause set? Apply the swap to every clause and check the multiset is invariant.
fn swap_is_automorphism(clause_set: &BTreeSet<Clause>, clauses: &[Clause], row_a: &[String], row_b: &[String]) -> bool {
    if row_a.len() != row_b.len() {
        return false;
    }
    let mut map = std::collections::HashMap::new();
    for (a, b) in row_a.iter().zip(row_b.iter()) {
        map.insert(a.as_str(), b.as_str());
        map.insert(b.as_str(), a.as_str());
    }
    // Image multiset must equal the original set (clauses are de-duplicated into a set; an
    // automorphism is a bijection on a clause SET, so set-equality of the image is exactly right).
    let image: BTreeSet<Clause> = clauses.iter().map(|c| rename(c, &map)).collect();
    &image == clause_set
}

/// Augment `e` with lex-leader SBPs for every adjacent row-interchange symmetry we can *verify*.
/// Returns `e` unchanged when the formula is not a readable clause set, has no rows, or no verified
/// symmetry — symmetry breaking is purely additive and never changes the SAT/UNSAT verdict.
pub fn break_symmetries(e: &ProofExpr) -> ProofExpr {
    let Some(clauses) = to_clauses(e) else {
        return e.clone();
    };
    let rows = positive_rows(&clauses);
    if rows.len() < 2 {
        return e.clone();
    }
    let clause_set: BTreeSet<Clause> = clauses.iter().cloned().collect();
    let mut sbps: Vec<ProofExpr> = Vec::new();
    for i in 0..rows.len() - 1 {
        let (ra, rb) = (&rows[i], &rows[i + 1]);
        if ra.len() != rb.len() || ra.len() < 2 {
            continue;
        }
        if swap_is_automorphism(&clause_set, &clauses, ra, rb) {
            let x: Vec<ProofExpr> = ra.iter().map(|a| atom(a.clone())).collect();
            let y: Vec<ProofExpr> = rb.iter().map(|a| atom(a.clone())).collect();
            sbps.push(lex_le(&x, &y, &format!("__sym_{i}")));
        }
    }
    if sbps.is_empty() {
        return e.clone();
    }
    let sbp = sbps.into_iter().reduce(and).unwrap();
    and(e.clone(), sbp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sat::{find_model, prove_unsat, ModelOutcome, UnsatOutcome};

    fn a(s: &str) -> ProofExpr {
        atom(s.to_string())
    }

    fn bits(mask: u32, n: usize) -> Vec<bool> {
        (0..n).map(|i| (mask >> i) & 1 == 1).collect()
    }
    fn lex_le_oracle(x: &[bool], y: &[bool]) -> bool {
        for k in 0..x.len() {
            if x[k] != y[k] {
                return !x[k] && y[k];
            }
        }
        true
    }

    #[test]
    fn lex_le_matches_brute_force() {
        // Soundness foundation: the encoded `X ≤ₗₑₓ Y` must agree with the oracle on EVERY pinned
        // assignment (auxiliaries free) — a wrong SBP would delete models and forge UNSAT.
        for m in 1..=5 {
            let xs: Vec<ProofExpr> = (0..m).map(|i| a(&format!("x{i}"))).collect();
            let ys: Vec<ProofExpr> = (0..m).map(|i| a(&format!("y{i}"))).collect();
            let f = lex_le(&xs, &ys, "L");
            for xm in 0..(1u32 << m) {
                for ym in 0..(1u32 << m) {
                    let (xa, ya) = (bits(xm, m), bits(ym, m));
                    let mut pinned = f.clone();
                    for (v, &b) in xs.iter().zip(&xa).chain(ys.iter().zip(&ya)) {
                        pinned = and(pinned, if b { v.clone() } else { not(v.clone()) });
                    }
                    let sat = matches!(find_model(&pinned), ModelOutcome::Sat(_));
                    assert_eq!(
                        sat,
                        lex_le_oracle(&xa, &ya),
                        "lex_le m={m} x={xa:?} y={ya:?} encoded={sat} oracle={}",
                        lex_le_oracle(&xa, &ya)
                    );
                }
            }
        }
    }

    /// Pairwise PHP(n) — used to confirm symmetry breaking PRESERVES the UNSAT verdict.
    fn php(n: usize) -> ProofExpr {
        let holes = n - 1;
        let p = |i: usize, h: usize| a(&format!("p_{i}_{h}"));
        let mut clauses = Vec::new();
        for i in 0..n {
            clauses.push((0..holes).map(|h| p(i, h)).reduce(or).unwrap());
        }
        for h in 0..holes {
            for i in 0..n {
                for j in (i + 1)..n {
                    clauses.push(not(and(p(i, h), p(j, h))));
                }
            }
        }
        clauses.into_iter().reduce(and).unwrap()
    }

    #[test]
    fn breaking_preserves_unsat_on_php() {
        // Adding verified SBPs to an UNSAT formula keeps it UNSAT (and stays refutable).
        for n in 3..=6 {
            let broken = break_symmetries(&php(n));
            assert!(
                matches!(prove_unsat(&broken), UnsatOutcome::Refuted),
                "PHP({n}) with symmetry breaking must stay Refuted"
            );
        }
    }

    #[test]
    fn breaking_detects_the_pigeon_symmetry() {
        // PHP rows (pigeons) are interchangeable → at least one verified row-swap SBP is added, so
        // the formula grows. (If detection silently found nothing, the formula would be unchanged.)
        let before = php(4);
        let after = break_symmetries(&before);
        assert_ne!(before, after, "PHP(4) pigeon symmetry must be detected and broken");
    }

    #[test]
    fn breaking_preserves_sat_models() {
        // Soundness-critical: on a SATISFIABLE symmetric formula, breaking must keep ≥1 model.
        // "each of 3 items takes one of 3 distinct slots" (a feasible bipartite matching) is SAT and
        // row-symmetric; after breaking it must remain SAT.
        let p = |i: usize, h: usize| a(&format!("q_{i}_{h}"));
        let mut clauses = Vec::new();
        for i in 0..3 {
            clauses.push((0..3).map(|h| p(i, h)).reduce(or).unwrap());
        }
        for h in 0..3 {
            for i in 0..3 {
                for j in (i + 1)..3 {
                    clauses.push(not(and(p(i, h), p(j, h))));
                }
            }
        }
        let f = clauses.into_iter().reduce(and).unwrap();
        assert!(matches!(find_model(&f), ModelOutcome::Sat(_)), "feasible must be SAT to begin");
        let broken = break_symmetries(&f);
        assert!(
            matches!(find_model(&broken), ModelOutcome::Sat(_)),
            "symmetry breaking must NOT delete every model of a SAT formula"
        );
    }

    fn raw_cdcl_is_unsat(e: &ProofExpr) -> bool {
        // Drive the bare CDCL core — NO matching fast-path, NO symmetry breaking inside — so this
        // measures the effect of the SBPs we pass in, isolated from `prove_unsat`'s shortcuts.
        use crate::cdcl::SolveResult;
        use crate::cnf::Cnf;
        let mut cnf = Cnf::new();
        cnf.assert(e).expect("clausifiable");
        let (mut solver, _) = cnf.into_solver_with_atoms();
        matches!(solver.solve(), SolveResult::Unsat)
    }

    #[test]
    fn symmetry_breaking_tames_raw_cdcl_on_pigeonhole() {
        // The PROOF symmetry breaking earns its place: pairwise PHP is syntactically symmetric, so
        // raw CDCL re-derives the same conflict once per pigeon permutation and bogs down. The
        // verified lex-leader SBPs collapse those orbits — the broken formula is decided much faster.
        // (`prove_unsat` would short-circuit PHP via the matching reasoner; here we bypass it to
        // isolate the symmetry-breaking effect on the general CDCL engine.)
        use std::time::Instant;
        let n = 8;
        let plain = php(n);
        let broken = break_symmetries(&plain);
        assert_ne!(plain, broken, "PHP({n}) must have a verified pigeon symmetry to break");

        let t = Instant::now();
        assert!(raw_cdcl_is_unsat(&broken), "broken PHP({n}) must be UNSAT");
        let with = t.elapsed();
        let t = Instant::now();
        assert!(raw_cdcl_is_unsat(&plain), "plain PHP({n}) must be UNSAT");
        let without = t.elapsed();

        eprintln!(
            "raw CDCL PHP({n}): plain={without:?} broken={with:?} speedup={:.1}x",
            without.as_secs_f64() / with.as_secs_f64().max(f64::MIN_POSITIVE)
        );
        assert!(
            with < without,
            "symmetry breaking must speed up raw CDCL on pigeonhole: broken={with:?} plain={without:?}"
        );
    }

    #[test]
    fn non_symmetric_formula_is_left_alone() {
        // A formula whose "rows" are NOT interchangeable (asymmetric extra clause) gets no SBP for
        // that pair — the automorphism check must reject it. Here row 0 and row 1 differ because an
        // extra unit clause pins one of row 0's variables, so the swap is not an automorphism.
        let f = and(
            and(or(a("p_0_0"), a("p_0_1")), or(a("p_1_0"), a("p_1_1"))),
            a("p_0_0"), // breaks the row-0/row-1 symmetry
        );
        // With no verified symmetry the formula is returned unchanged.
        assert_eq!(break_symmetries(&f), f, "asymmetric formula must be left unchanged");
    }
}
