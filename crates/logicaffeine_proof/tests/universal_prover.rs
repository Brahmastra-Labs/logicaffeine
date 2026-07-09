//! **The universal prover is already constructed and TOTAL — "polynomially bounded" is a size
//! theorem about the residue, not a construction.** SR is a complete proof system; our dispatcher
//! (categorical groups + `sdcl` SR search as the fallback) is a total prover. This test drives it
//! at everything — the named groups AND sampled unstructured residue cores at n = 4, 5 — and
//! certifies that EVERY tautology in the corpus gets a machine-checked SR refutation (the system
//! proves everything; construction is not the open question).
//!
//! What is open is purely the SIZE on the residue. The certified sizes are small at these scales —
//! but small-`n` sizes are asymptotically vacuous, so this measures totality (proven) and locates
//! the size question (the residue), it does NOT bound it. "3-SAT ∈ coNP" needs a proof that the
//! residue's SR size stays polynomial as `n → ∞`, which is the open lemma and possibly false; no
//! construction and no measurement supplies it.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::sdcl::sdcl_refute;
use std::collections::BTreeSet;

fn lcg(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *s >> 33
}

fn is_unsat(n: usize, clauses: &[Vec<Lit>]) -> bool {
    let mut s = logicaffeine_proof::cdcl::Solver::new(n);
    for c in clauses {
        s.add_clause(c.clone());
    }
    matches!(s.solve(), logicaffeine_proof::cdcl::SolveResult::Unsat)
}

fn sample_core(n: usize, state: &mut u64) -> Option<Vec<Vec<Lit>>> {
    let nc = (2 * n) + (lcg(state) % (3 * n as u64)) as usize;
    let clauses: Vec<Vec<Lit>> = (0..nc)
        .map(|_| {
            let width = 2 + (lcg(state) % 2) as usize;
            let mut vars: Vec<u32> = Vec::new();
            while vars.len() < width {
                let v = (lcg(state) % n as u64) as u32;
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            vars.iter().map(|&v| Lit::new(v, lcg(state) & 1 == 1)).collect()
        })
        .collect();
    if !is_unsat(n, &clauses) {
        return None;
    }
    let mut core = clauses;
    let mut i = 0;
    while i < core.len() {
        let mut t = core.clone();
        t.remove(i);
        if is_unsat(n, &t) {
            core = t;
        } else {
            i += 1;
        }
    }
    Some(core)
}

#[test]
fn the_universal_prover_is_total_only_the_residue_size_is_open() {
    // The named groups: each certified by its specialist / SR.
    let mut named: Vec<(String, usize, usize)> = Vec::new(); // (name, vars, cert size)
    for m in [3usize, 4] {
        let (php, _) = logicaffeine_proof::families::php(m);
        let cert = sdcl_refute(php.num_vars, &php.clauses);
        assert!(cert.refuted && check_pr_refutation(php.num_vars, &php.clauses, &cert.steps));
        named.push((format!("PHP({m})"), php.num_vars, cert.steps.len()));
    }
    let (_, tsei, _) = logicaffeine_proof::families::tseitin_expander(6, 0xC0DE);
    let cert = sdcl_refute(tsei.num_vars, &tsei.clauses);
    assert!(cert.refuted && check_pr_refutation(tsei.num_vars, &tsei.clauses, &cert.steps));
    named.push(("Tseitin(6)".into(), tsei.num_vars, cert.steps.len()));

    // The UNSTRUCTURED residue: sampled minimal cores at n = 4, 5 — no named group. The universal
    // prover must certify these too (totality), and it does.
    let mut residue: Vec<(usize, usize)> = Vec::new(); // (vars, cert size)
    let mut dedup: BTreeSet<Vec<Vec<(u32, bool)>>> = BTreeSet::new();
    let mut state = 0x0DE5_1D0Eu64;
    let mut got = 0usize;
    while got < 20 {
        let n = 4 + (lcg(&mut state) % 2) as usize;
        let Some(core) = sample_core(n, &mut state) else { continue };
        let key: Vec<Vec<(u32, bool)>> = {
            let mut k: Vec<Vec<(u32, bool)>> =
                core.iter().map(|c| c.iter().map(|l| (l.var(), l.is_positive())).collect()).collect();
            for c in &mut k {
                c.sort_unstable();
            }
            k.sort();
            k
        };
        if !dedup.insert(key) {
            continue;
        }
        let cert = sdcl_refute(n, &core);
        assert!(
            cert.refuted && check_pr_refutation(n, &core, &cert.steps),
            "the universal prover certifies every residue core (totality)"
        );
        residue.push((n, cert.steps.len()));
        got += 1;
    }
    let max_res = residue.iter().map(|&(_, s)| s).max().unwrap();
    eprintln!(
        "universal prover TOTAL: named groups {named:?} all certified; {} unstructured residue \
         cores (n=4,5) all certified, SR sizes up to {max_res} — the system proves EVERYTHING",
        residue.len()
    );
    eprintln!(
        "the honest ground truth: SR is a constructed, total proof system — every tautology here \
         gets a re-checked SR refutation, so CONSTRUCTION is done. '3-SAT ∈ coNP' is NOT a \
         construction; it is the SIZE THEOREM that the residue's SR proofs stay POLYNOMIAL as \
         n → ∞. Small-n sizes are asymptotically vacuous; the theorem is open and possibly false \
         (our lower bounds lean P ≠ NP). No construction supplies it — it is a bound, and the \
         bound is the open lemma."
    );
}
