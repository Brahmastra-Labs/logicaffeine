//! Bucket elimination (the certified TreeWidth route) must DECLINE fast on dense, high-treewidth cores
//! rather than grind. The width cap alone was insufficient: on a dense formula each resolvent stays
//! narrow (never tripping the cap) yet the resolvent COUNT `|pos|·|neg|` explodes, so the route spent
//! seconds-to-minutes (9.4s on Ramsey(3,3;6), 5.6 min on (3,4;9)) before giving up — starving the fast
//! symmetry route behind it in the arsenal. The resolvent-product bail fixes it. These tests pin BOTH
//! sides: fast bail on a dense formula, and NO over-bail — a genuinely bounded-treewidth formula is
//! still refuted with its certificate.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::families;
use logicaffeine_proof::inprocess::bucket_elimination_refute;
use logicaffeine_proof::pr::check_pr_refutation;

#[test]
fn bucket_elimination_bails_fast_on_a_dense_high_treewidth_core() {
    // Ramsey(3,4;9) is dense (high treewidth). The route must return quickly — declining is correct
    // (its resolution certificate would be exponential) — not grind for minutes.
    let (cnf, _) = families::ramsey(3, 4, 9);
    let t = std::time::Instant::now();
    let _ = bucket_elimination_refute(cnf.num_vars, &cnf.clauses, 12);
    let ms = t.elapsed().as_millis();
    assert!(ms < 2000, "bucket elimination must bail fast on a dense core, took {ms}ms");
}

#[test]
fn bucket_elimination_still_refutes_a_bounded_treewidth_formula() {
    // A treewidth-1 implication chain: x0, x0→x1, …, x_{k-1}→xk, ¬xk — unsatisfiable, and the resolvent
    // product stays 1 at every step, so the bail must NEVER fire. Bucket elimination must refute it and
    // the certificate must re-check.
    let k = 300usize;
    let mut clauses = vec![vec![Lit::pos(0)]];
    for i in 0..k as u32 {
        clauses.push(vec![Lit::neg(i), Lit::pos(i + 1)]);
    }
    clauses.push(vec![Lit::neg(k as u32)]);
    let steps = bucket_elimination_refute(k + 1, &clauses, 12).expect("a treewidth-1 chain must be refuted");
    assert!(
        check_pr_refutation(k + 1, &clauses, &steps),
        "the bucket-elimination certificate must re-check from scratch"
    );
}
