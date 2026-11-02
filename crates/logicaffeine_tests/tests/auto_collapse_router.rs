//! # The box auto-collapses and auto-categorizes (Ding program)
//!
//! The toolbox is not a bag of separate attacks — `solve::solve_structured` is a single front door
//! that routes each instance through cheap structure-detecting specialists before any search, and
//! *names the specialist that collapsed it* (`Solved::via`). This study feeds a battery of distinct
//! structured families and asserts, for each: (a) it is decided correctly, (b) a **specialist**
//! collapsed it — not the CDCL fallback — with **zero search** (`conflicts == 0`), and (c) it lands in
//! its expected category. That is the "auto-collapses and categorizes" property, made a locked test.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::solve::{solve_structured, Answer, Route};

/// Pigeonhole `PHP(n)`: `n` pigeons, `n-1` holes (`n ≥ 4` gives width-≥3 pigeon clauses, past 2-SAT).
fn php(n: usize) -> (usize, Vec<Vec<Lit>>) {
    let holes = n - 1;
    let v = |p: usize, h: usize| (p * holes + h) as u32;
    let mut cl = Vec::new();
    for p in 0..n {
        cl.push((0..holes).map(|h| Lit::pos(v(p, h))).collect());
    }
    for h in 0..holes {
        for p1 in 0..n {
            for p2 in (p1 + 1)..n {
                cl.push(vec![Lit::neg(v(p1, h)), Lit::neg(v(p2, h))]);
            }
        }
    }
    (n * holes, cl)
}

/// The width-3 XOR constraint `x_a ⊕ x_b ⊕ x_c = t` as its 4 forbidding clauses.
fn xor3(vars: [u32; 3], t: bool) -> Vec<Vec<Lit>> {
    let mut cl = Vec::new();
    for bits in 0u32..8 {
        let b = [(bits >> 2) & 1, (bits >> 1) & 1, bits & 1];
        if ((b[0] ^ b[1] ^ b[2]) == 1) != t {
            cl.push(
                (0..3)
                    .map(|i| {
                        if b[i] == 0 {
                            Lit::pos(vars[i])
                        } else {
                            Lit::neg(vars[i])
                        }
                    })
                    .collect(),
            );
        }
    }
    cl
}

/// A minimal 2-SAT contradiction: all four binary clauses over `{x0, x1}` — no assignment survives.
fn twosat_unsat() -> (usize, Vec<Vec<Lit>>) {
    (
        2,
        vec![
            vec![Lit::pos(0), Lit::pos(1)],
            vec![Lit::pos(0), Lit::neg(1)],
            vec![Lit::neg(0), Lit::pos(1)],
            vec![Lit::neg(0), Lit::neg(1)],
        ],
    )
}

/// A Horn contradiction that is NOT 2-SAT (a width-3 Horn rule): facts `a, b`, rule `a ∧ b → c`
/// (`¬a ∨ ¬b ∨ c`), and `¬c`. Unit propagation derives `c` then hits `¬c`.
fn horn_unsat() -> (usize, Vec<Vec<Lit>>) {
    (
        3,
        vec![
            vec![Lit::pos(0)],
            vec![Lit::pos(1)],
            vec![Lit::neg(0), Lit::neg(1), Lit::pos(2)],
            vec![Lit::neg(2)],
        ],
    )
}

/// An inconsistent GF(2) system with NO single fully-covered triple: four width-3 parities where every
/// variable appears an even number of times (so the LHS sums to 0) but the right-hand sides sum to 1.
/// `a⊕b⊕c=1, c⊕d⊕e=1, e⊕f⊕a=1, b⊕d⊕f=0` over `{a..f} = {0..5}` — a genuine linear-dependency obstruction.
fn parity_system_unsat() -> (usize, Vec<Vec<Lit>>) {
    let mut cl = xor3([0, 1, 2], true);
    cl.extend(xor3([2, 3, 4], true));
    cl.extend(xor3([4, 5, 0], true));
    cl.extend(xor3([1, 3, 5], false));
    (6, cl)
}

#[test]
fn the_box_auto_collapses_and_categorizes_every_structured_family() {
    let battery: Vec<(&str, usize, Vec<Vec<Lit>>, Route)> = vec![
        (
            "2-SAT contradiction",
            twosat_unsat().0,
            twosat_unsat().1,
            Route::TwoSat,
        ),
        (
            "Horn (width-3 rule)",
            horn_unsat().0,
            horn_unsat().1,
            Route::Horn,
        ),
        ("pigeonhole PHP(4)", php(4).0, php(4).1, Route::Pigeonhole),
        (
            "parity system (GF(2))",
            parity_system_unsat().0,
            parity_system_unsat().1,
            Route::Parity,
        ),
    ];

    println!("family                    n    verdict   via            conflicts");
    for (name, nv, clauses, expected) in &battery {
        let solved = solve_structured(*nv, clauses);
        println!(
            "{name:<25} {nv:<4} {:<9} {:<14} {}",
            match solved.answer {
                Answer::Unsat => "UNSAT",
                Answer::Sat(_) => "SAT",
            },
            format!("{:?}", solved.via),
            solved.conflicts
        );

        // ROBUST, certainly-correct invariants (locked regardless of which specialist fires):
        // (a) decided correctly — every family here is UNSAT.
        assert!(
            matches!(solved.answer, Answer::Unsat),
            "{name}: must be decided UNSAT"
        );
        // (b) a specialist collapsed it with ZERO search — not the CDCL fallback.
        assert_ne!(
            solved.via,
            Route::Cdcl,
            "{name}: a specialist must collapse it, not CDCL"
        );
        assert_eq!(
            solved.conflicts, 0,
            "{name}: a specialist collapses without search (0 conflicts)"
        );
        // (c) auto-categorized as its expected specialist.
        assert_eq!(
            solved.via, *expected,
            "{name}: must be auto-categorized as {expected:?}"
        );
    }
}
