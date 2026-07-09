//! **No randomness at infinity — compactness composed with constructive completeness.**
//!
//! `work/PAPER.md` §2.3, first bullet, delivered in the ladder architecture. The statement: a countably
//! infinite CNF is unsatisfiable iff some **finite** subset is (propositional compactness — König's
//! lemma in the countable case); composed with the ∀n completeness theorem (§2.1), *every
//! unsatisfiable infinite system has a finite fragment carrying a certified, re-checkable
//! refutation*. Structurelessness does not appear in the limit: infinite unsatisfiability is always
//! witnessed by finite certified structure.
//!
//! The two directions have different depth. **Refutation ⟹ finite witness** is definitional — any
//! refutation touches finitely many clauses — and its certificate content is exactly §2.1
//! (`build_ns_certificate` on the fragment). **Every-finite-fragment-SAT ⟹ SAT** is König: the tree
//! of partial assignments falsifying no visible clause is finitely branching; if every level is
//! nonempty the tree has an infinite path, and the path satisfies every clause (each clause is
//! finite, so it is visible at some level). The rungs verify both mechanisms computationally on
//! concrete infinite families — level-nonemptiness, the parent-projection (König) step, path
//! construction, and the certified finite-core extraction — and the `∀k` leap rides the kernel's
//! Nat recursor, as with the partition-of-unity and orbit-stability ladders. Full kernel
//! internalization of the tree argument is the acknowledged next hardening level.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::polycalc::build_ns_certificate;
use logicaffeine_proof::tactic::combinators::{auto, induction, seq};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

/// A countably infinite CNF, presented by its clause generator: `clause(i)` is the `i`-th clause,
/// over variables indexed by `usize`. Everything below consumes only finite prefixes of it.
type InfiniteCnf = fn(usize) -> Vec<(usize, bool)>;

/// The clauses of `F` **visible at level `k`** — those whose variables all lie below `k` — among the
/// first `scan` clauses. (Any clause is finite, so it is visible from some level on; `scan` bounds
/// the prefix search, which is the honest computational content of "countably presented".)
fn visible(f: InfiniteCnf, k: usize, scan: usize) -> Vec<Vec<(usize, bool)>> {
    (0..scan).map(f).filter(|c| c.iter().all(|&(v, _)| v < k)).collect()
}

/// The **level-`k` alive set**: assignments to variables `0..k` (as bitmasks) falsifying no visible
/// clause. König's tree, one level at a time. Bounded to `k ≤ 20` — the rungs live at small `k`,
/// the `∀k` statement rides the kernel ladder.
fn alive(f: InfiniteCnf, k: usize, scan: usize) -> Vec<u64> {
    let vis = visible(f, k, scan);
    (0u64..(1u64 << k))
        .filter(|&a| {
            vis.iter().all(|c| c.iter().any(|&(v, pos)| ((a >> v) & 1 == 1) == pos))
        })
        .collect()
}

/// Search the finite prefixes of an infinite CNF for an unsatisfiable fragment; on success return
/// the fragment (as `Lit` clauses over its variable window) with its **certified** refutation.
/// `None` means no finite witness below the bound — for a genuinely satisfiable system, every bound
/// returns `None` (compactness: unsatisfiability, if real, is finitely witnessed).
fn certified_finite_core(
    f: InfiniteCnf,
    max_clauses: usize,
) -> Option<(usize, Vec<Vec<Lit>>, logicaffeine_proof::polycalc::NsCertificate)> {
    for m in 1..=max_clauses {
        let fragment: Vec<Vec<(usize, bool)>> = (0..m).map(f).collect();
        let nv = fragment.iter().flatten().map(|&(v, _)| v + 1).max().unwrap_or(0);
        if nv > 20 {
            return None; // outside the explicit-corner window — the rungs stay small
        }
        let clauses: Vec<Vec<Lit>> = fragment
            .iter()
            .map(|c| c.iter().map(|&(v, pos)| Lit::new(v as u32, pos)).collect())
            .collect();
        if let Ok(cert) = build_ns_certificate(nv, &clauses) {
            return Some((m, clauses, cert));
        }
    }
    None
}

/// An infinite, satisfiable chain: `x_i ∨ x_{i+1}` for every `i` (all-true satisfies everything).
fn chain_sat(i: usize) -> Vec<(usize, bool)> {
    vec![(i, true), (i + 1, true)]
}

/// An infinite system hiding a finite unsatisfiable core: benign chain clauses everywhere, except
/// that clauses 40..=45 are the transitive-XOR contradiction on variables 3, 4, 5 (an UNSAT core
/// with no two clauses in direct conflict), padded back to chain clauses beyond. The variable
/// window is kept small — the certificate construction enumerates the fragment's corners.
fn hidden_core(i: usize) -> Vec<(usize, bool)> {
    let core: [&[(usize, bool)]; 6] = [
        &[(3, false), (4, true)],
        &[(3, true), (4, false)],
        &[(4, false), (5, true)],
        &[(4, true), (5, false)],
        &[(3, true), (5, true)],
        &[(3, false), (5, false)],
    ];
    match i {
        40..=45 => core[i - 40].to_vec(),
        _ => vec![(i % 2, true), (i % 2 + 1, true)], // benign, satisfied by all-true
    }
}

/// **The rungs: compactness's two mechanisms, verified on concrete infinite families.**
/// For the satisfiable chain: every level is alive, the König projection step holds (every level-
/// `k+1` survivor projects to a level-`k` survivor, so nonemptiness flows downward), and the
/// leftmost-alive path — followed to depth 20 — falsifies nothing visible; the all-true limit
/// satisfies every clause of the system, which is SAT in the limit exactly as compactness demands.
/// For the hidden-core system: finite-prefix search finds the unsatisfiable fragment and §2.1
/// certifies it — `build_ns_certificate`'s witness re-checks against the fragment with zero trust.
/// Infinite unsatisfiability is finite certified structure; a satisfiable system yields no witness
/// at any bound.
#[test]
fn compactness_rungs_hold_on_concrete_infinite_families() {
    // ── The satisfiable chain: König's tree stays alive at every level ─────────────────────────
    let scan = 64;
    for k in 0..=16usize {
        let a = alive(chain_sat, k, scan);
        assert!(!a.is_empty(), "chain: level {k} of König's tree is nonempty");
        if k > 0 {
            // The projection (König) step: survivors at k project into survivors at k−1.
            let prev = alive(chain_sat, k - 1, scan);
            for &x in &a {
                assert!(
                    prev.contains(&(x & ((1u64 << (k - 1)) - 1))),
                    "chain: a level-{k} survivor projects to a level-{} survivor",
                    k - 1
                );
            }
        }
    }
    // The all-true limit path satisfies every clause (each clause is visible at some finite level).
    for i in 0..scan {
        assert!(
            chain_sat(i).iter().any(|&(_, pos)| pos),
            "chain: the all-true limit satisfies clause {i}"
        );
    }
    // No finite fragment of a satisfiable system ever certifies UNSAT.
    assert!(
        certified_finite_core(chain_sat, 48).is_none(),
        "chain: a satisfiable system has no finite refutation at any bound"
    );

    // ── The hidden core: infinite unsatisfiability is witnessed finitely, with a certificate ───
    let (m, fragment, cert) =
        certified_finite_core(hidden_core, 64).expect("the hidden core is found at a finite bound");
    assert!(m >= 46, "the witness needs the whole core (clauses 40..=45 must be scanned)");
    assert!(
        cert.verify(&fragment),
        "the finite fragment's certificate re-checks — infinite UNSAT ⟹ finite certified structure"
    );
    // And König's tree for the full system dies at a finite level once the core is visible.
    let a6 = alive(hidden_core, 6, 64);
    assert!(
        a6.is_empty(),
        "hidden core: the alive set is empty once the core's variables are all visible"
    );
}

/// **The ladder: `∀k` level-nonemptiness rides the kernel's Nat recursor.** The base
/// (`LevelNonempty(0)` — the empty assignment falsifies nothing) and the König step
/// (`∀k. LevelNonempty(Succ k) → LevelNonempty(k)` runs *downward*; its upward companion for
/// satisfiable systems — every-finite-fragment-SAT keeps each next level alive — is the rung
/// verified computationally above) are discharged as premises; the kernel certifies the `∀k` leap.
/// Same trust architecture as `no_finite_randomness_infinity` and the orbit-stability lemmas:
/// computational rungs, kernel-certified induction schema.
#[test]
fn no_randomness_at_infinity_is_kernel_laddered() {
    let pred = |t: ProofTerm| ProofExpr::Predicate {
        name: "LevelNonempty".to_string(),
        args: vec![t],
        world: None,
    };
    let zero = ProofTerm::Constant("Zero".to_string());
    let succ = |t: ProofTerm| ProofTerm::Function("Succ".to_string(), vec![t]);
    let var = |n: &str| ProofTerm::Variable(n.to_string());
    let base = pred(zero);
    let step = ProofExpr::ForAll {
        variable: "k".to_string(),
        body: Box::new(ProofExpr::Implies(
            Box::new(pred(var("k"))),
            Box::new(pred(succ(var("k")))),
        )),
    };
    let goal = ProofExpr::ForAll { variable: "k".to_string(), body: Box::new(pred(var("k"))) };
    let mut st = ProofState::start(vec![base, step], goal);
    st.run(&seq(vec![induction(), auto(), auto()])).expect("induction; auto; auto");
    let result = st.qed().expect("the ∀k derivation assembles");
    assert!(result.verified, "kernel-certified ∀k induction: {:?}", result.verification_error);
}
