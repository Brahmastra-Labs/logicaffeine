//! The fast counting refuter for the modular counting principle Count_q(n). It fires exactly when the
//! clauses form a genuine Count_q core with q ∤ n (UNSAT by the counting argument), returns a re-checkable
//! (n, q, remainder) certificate, and declines on the satisfiable case (q | n) and on non-counting
//! structure — pigeonhole in particular, which superficially shares the coverage + at-most-one shape but
//! whose coverage-clause blocks are NOT pairwise disjoint (a pigeon may take several holes) / have block
//! degree 1 (functional variant), so the detector must not mistake it for a q-partition.

use logicaffeine_proof::counting_principle::{counting_certificate, refute_counting};
use logicaffeine_proof::families;

#[test]
fn refutes_count_q_when_q_does_not_divide_n_with_a_recheckable_cert() {
    for (n, q) in [(4usize, 3usize), (7, 3), (8, 3), (5, 3), (7, 5), (3, 2), (7, 2), (9, 2)] {
        assert_ne!(n % q, 0, "test setup: q ∤ n for the UNSAT case");
        let (cnf, _) = families::mod_counting(n, q);
        let cert = counting_certificate(cnf.num_vars, &cnf.clauses)
            .unwrap_or_else(|| panic!("Count_{q}({n}) with q∤n must yield a counting certificate"));
        assert_eq!((cert.n, cert.q, cert.remainder), (n as u64, q as u64, (n % q) as u64));
        assert!(cert.check(), "the certificate must re-check from scratch");
        assert!(cert.byte_len() > 0);
        assert!(refute_counting(cnf.num_vars, &cnf.clauses));
    }
}

#[test]
fn declines_when_q_divides_n_sat() {
    for (n, q) in [(6usize, 3usize), (9, 3), (6, 2), (8, 2), (10, 5)] {
        assert_eq!(n % q, 0, "test setup: q | n for the SAT case");
        let (cnf, _) = families::mod_counting(n, q);
        assert!(
            counting_certificate(cnf.num_vars, &cnf.clauses).is_none(),
            "Count_{q}({n}) is satisfiable (q|n) — the refuter must decline"
        );
        assert!(!refute_counting(cnf.num_vars, &cnf.clauses));
    }
}

#[test]
fn declines_on_pigeonhole_and_random() {
    // Pigeonhole shares coverage + at-most-one shape but is NOT a q-partition (its coverage blocks are
    // not pairwise disjoint / have block degree 1); a random instance has no such structure. Neither may
    // be refuted by the counting cut (a false fire could be unsound on a satisfiable formula).
    for n in [3usize, 4, 5] {
        let (php, _) = families::php(n);
        assert!(!refute_counting(php.num_vars, &php.clauses), "PHP({n}) is not a counting principle");
        let (func, _) = families::functional_php(n + 3);
        assert!(!refute_counting(func.num_vars, &func.clauses), "functional PHP is not a q-partition");
    }
    let r = families::random_3sat(30, 120, 0x9);
    assert!(!refute_counting(r.num_vars, &r.clauses));
}
