//! Timing probe for the arithmetic decision procedures on their hot shapes.
//!
//! The compile-crate optimizer discharges array-bounds goals through
//! `lia::fourier_motzkin_unsat` (one call per candidate access site), and the
//! reflection reducers run `ring`/`omega` per goal. These are small systems —
//! a handful of variables and single-digit constraint counts — so what matters
//! is per-call latency at small coefficient sizes, not asymptotics.
//!
//! Run: cargo run -p logicaffeine-kernel --release --example arith_perf

use std::time::Instant;

use logicaffeine_kernel::lia::{Constraint, LinearExpr, Rational};
use logicaffeine_kernel::omega::{self, IntConstraint, IntExpr};
use logicaffeine_kernel::ring::Polynomial;

fn time<F: FnMut()>(label: &str, iters: u32, mut f: F) {
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let total = start.elapsed();
    println!(
        "{label:<44} {iters:>7} iters  {:>9.1?} total  {:>8.0} ns/call",
        total,
        total.as_nanos() as f64 / iters as f64
    );
}

fn main() {
    // The optimizer's knapsack-shaped bounds proof: facts {w <= cap, wi >= 0},
    // goal (cap - w + wi) >= 0, discharged as facts ∧ (goal+1 <= 0) UNSAT.
    let (w, wi, cap) = (0i64, 1i64, 2i64);
    let le = |a: &LinearExpr, b: &LinearExpr| Constraint { expr: a.sub(b), strict: false };
    let facts = vec![
        le(&LinearExpr::var(w), &LinearExpr::var(cap)),
        Constraint { expr: LinearExpr::var(wi).neg(), strict: false },
    ];
    let goal = LinearExpr::var(cap)
        .sub(&LinearExpr::var(w))
        .add(&LinearExpr::var(wi));
    time("lia: 3-var bounds proof (optimizer shape)", 100_000, || {
        let neg = goal.add(&LinearExpr::constant(Rational::from_i64(1)));
        let mut system = facts.clone();
        system.push(Constraint { expr: neg, strict: false });
        assert!(logicaffeine_kernel::lia::fourier_motzkin_unsat(&system));
    });

    // Ring normalization at collatz size: 3(2k+1)+1 vs 6k+4.
    let k = Polynomial::var(0);
    time("ring: collatz-size normalize + compare", 100_000, || {
        let lhs = Polynomial::constant(3)
            .mul(&Polynomial::constant(2).mul(&k).add(&Polynomial::constant(1)))
            .add(&Polynomial::constant(1));
        let rhs = Polynomial::constant(6).mul(&k).add(&Polynomial::constant(4));
        assert!(lhs.canonical_eq(&rhs));
    });

    // Omega with hypothesis + conclusion (2 constraints after negation).
    let x = IntExpr::var(0);
    time("omega: 1-var hyp+goal solve", 100_000, || {
        let c1 = IntConstraint { expr: x.sub(&IntExpr::constant(10)), strict: false };
        let c2 = IntConstraint {
            expr: IntExpr::constant(12).sub(&x),
            strict: false,
        };
        assert!(omega::omega_unsat(&[c1, c2]));
    });
}
