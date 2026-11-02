//! Z3 certifier — the kernel's Fourier–Motzkin engine, checked against the SMT
//! solver. The fast `prove` ships on the compile hot path (Z3 is heavyweight
//! and optional; the oracle runs inside the measured run). This module is the
//! SOUNDNESS GROUND TRUTH: under the `verification` feature it re-encodes the
//! exact same affine `facts ⊢ goal >= 0` query to `logicaffeine-verify` (Z3
//! linear integer arithmetic) and asserts that every proof the kernel
//! discharges, Z3 confirms — the LIA tactic never claims what the solver
//! refutes. This is how the kernel's theorem-prover "ties into the math
//! solver" without putting Z3 on the budget-bounded run path.
use super::*;
use crate::intern::Interner;
use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyOp, VerifyType};

fn z3_name(idx: i64) -> String {
    format!("v{}", idx)
}

/// A kernel `Rational` that is integer-valued (every fact/goal we build is)
/// as an `i64`.
fn as_int(r: &Rational) -> i64 {
    r.to_i64()
        .expect("certifier only encodes integer constraints")
}

/// Encode a kernel `LinearExpr` as a Z3 integer term (deterministic order:
/// `LinearExpr` already stores coefficients in a `BTreeMap`).
fn encode(e: &LinearExpr) -> VerifyExpr {
    let mut acc = VerifyExpr::int(as_int(&e.constant));
    for (idx, coeff) in &e.coefficients {
        let term = VerifyExpr::binary(
            VerifyOp::Mul,
            VerifyExpr::int(as_int(coeff)),
            VerifyExpr::var(z3_name(*idx)),
        );
        acc = VerifyExpr::binary(VerifyOp::Add, acc, term);
    }
    acc
}

/// Does Z3 confirm `goal >= 0` is implied by `facts` (each kernel
/// `expr <= 0` / `< 0`)? Mirrors `super::prove`, discharged by the solver.
fn z3_confirms(facts: &[Constraint], goal: &LinearExpr) -> bool {
    let mut session = VerificationSession::new();
    let mut idxs: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for c in facts {
        for k in c.expr.coefficients.keys() {
            idxs.insert(*k);
        }
    }
    for k in goal.coefficients.keys() {
        idxs.insert(*k);
    }
    for idx in idxs {
        session.declare(&z3_name(idx), VerifyType::Int);
    }
    for c in facts {
        // expr <= 0 (or < 0).
        let lhs = encode(&c.expr);
        let zero = VerifyExpr::int(0);
        let assertion = if c.strict {
            VerifyExpr::lt(lhs, zero)
        } else {
            VerifyExpr::lte(lhs, zero)
        };
        session.assume(&assertion);
    }
    session
        .verify(&VerifyExpr::gte(encode(goal), VerifyExpr::int(0)))
        .is_ok()
}

fn s(i: &mut Interner, name: &str) -> Symbol {
    i.intern(name)
}

/// The three hard-loss relations the engine exists to prove must each be
/// confirmed by Z3 — the whole point of the build, certified end to end.
#[test]
fn z3_confirms_the_hard_loss_relations() {
    let mut i = Interner::new();
    let (w, wi, cap) = (s(&mut i, "w"), s(&mut i, "wi"), s(&mut i, "cap"));
    // knapsack lower: {w >= wi} ⊢ w - wi >= 0
    let f = vec![ge(&var(w), &var(wi))];
    let g = var(w).sub(&var(wi));
    assert!(prove(&f, &g) && z3_confirms(&f, &g));
    // knapsack upper: {w <= cap, wi >= 0} ⊢ cap - w + wi >= 0
    let f = vec![le(&var(w), &var(cap)), nonneg(&var(wi))];
    let g = var(cap).sub(&var(w)).add(&var(wi));
    assert!(prove(&f, &g) && z3_confirms(&f, &g));
    // string_search: {i <= T-P, j <= P-1, i>=0, j>=0} ⊢ (T-1)-(i+j) >= 0
    let (i2, j, t, p) = (
        s(&mut i, "i"),
        s(&mut i, "j"),
        s(&mut i, "t"),
        s(&mut i, "p"),
    );
    let f = vec![
        le(&var(i2), &var(t).sub(&var(p))),
        le(&var(j), &var(p).add(&konst(-1))),
        nonneg(&var(i2)),
        nonneg(&var(j)),
    ];
    let g = var(t).add(&konst(-1)).sub(&var(i2).add(&var(j)));
    assert!(prove(&f, &g) && z3_confirms(&f, &g));
    // graph_bfs: {u >= 0, u <= n-1} ⊢ n - (u+1) >= 0
    let (u, n) = (s(&mut i, "u"), s(&mut i, "n"));
    let f = vec![nonneg(&var(u)), le(&var(u), &var(n).add(&konst(-1)))];
    let g = var(n).sub(&var(u).add(&konst(1)));
    assert!(prove(&f, &g) && z3_confirms(&f, &g));
}

/// Deterministic integer mixer (no `rand` — reproducible).
fn mix(n: u64) -> u64 {
    let mut x = n
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x
}
fn pick(seed: u64, k: u64, lo: i64, hi: i64) -> i64 {
    let span = (hi - lo + 1) as u64;
    lo + (mix(seed.wrapping_add(k.wrapping_mul(0x9E37_79B9_7F4A_7C15))) % span) as i64
}

/// THE soundness theorem, exhaustively sampled: over a large deterministic
/// battery of small 3-variable affine systems and goals, every system the
/// kernel's Fourier–Motzkin engine proves, Z3 also proves. A single
/// counterexample (`prove` true while Z3 refutes) would be an unsound
/// bounds elision — UB in both the AOT binary and the VM — and fails
/// loudly. We also assert the engine isn't trivially weak (it proves a real
/// fraction of what Z3 proves), so soundness isn't bought with uselessness.
#[test]
fn fm_proofs_are_always_z3_valid() {
    let mut i = Interner::new();
    let (x, y, z) = (s(&mut i, "x"), s(&mut i, "y"), s(&mut i, "z"));
    let vars = [x, y, z];
    let mk = |seed: u64, base: u64| -> LinearExpr {
        // a·x + b·y + c·z + d, coeffs ∈ [-2,2], const ∈ [-4,4]
        let mut e = konst(pick(seed, base + 3, -4, 4));
        for (vi, vv) in vars.iter().enumerate() {
            let coeff = pick(seed, base + vi as u64, -2, 2);
            e = e.add(&term_of(*vv, coeff));
        }
        e
    };
    let mut fm_proved = 0u32;
    let mut z3_proved = 0u32;
    let mut both = 0u32;
    let trials = 600u64;
    for seed in 0..trials {
        // 3 facts (each `expr <= 0`) + 1 goal.
        let facts: Vec<Constraint> = (0..3)
            .map(|fi| Constraint {
                expr: mk(seed, 10 + fi * 4),
                strict: false,
            })
            .collect();
        let goal = mk(seed, 100);
        let fm = prove(&facts, &goal);
        let z3 = z3_confirms(&facts, &goal);
        if fm {
            fm_proved += 1;
            assert!(
                z3,
                "UNSOUND: kernel Fourier–Motzkin proved a goal Z3 refutes (seed {}). \
                     Coefficients are deterministic — reproduce with this seed.",
                seed
            );
        }
        if z3 {
            z3_proved += 1;
        }
        if fm && z3 {
            both += 1;
        }
    }
    assert!(
        fm_proved > 0,
        "engine proved nothing across {} trials",
        trials
    );
    assert!(
        both * 10 >= z3_proved * 4,
        "Fourier–Motzkin recall too low: proved {}/{} of Z3-provable goals",
        both,
        z3_proved
    );
}

fn term_of(x: Symbol, c: i64) -> LinearExpr {
    var(x).scale(&Rational::from_i64(c))
}
