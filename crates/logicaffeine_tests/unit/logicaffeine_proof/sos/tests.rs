use super::*;

fn sat(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    (0u64..(1u64 << num_vars)).any(|x| {
        clauses.iter().all(|c| {
            c.iter()
                .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
        })
    })
}

fn splitmix(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^ (z >> 31)
}

/// Re-check a returned certificate exactly: combining the lift polynomials with the (non-negative)
/// multipliers must cancel every variable and leave a strictly positive constant — the contradiction
/// `0 < d ≤ 0`. Independent of the elimination's internal arithmetic.
fn certificate_is_valid(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    cert: &BTreeMap<usize, i128>,
) -> bool {
    check_sos_certificate(num_vars, clauses, cert)
}

/// **Soundness to the point of absurdity**: across a fuzz of small CNFs, every SoS refutation is a
/// genuine UNSAT (checked against brute force) and its certificate re-checks to a positive constant —
/// a self-contained contradiction. A refutation is *never* issued for a satisfiable formula.
#[test]
fn sos_refutation_is_sound_against_brute_force() {
    let mut state = 0x5050_0001u64;
    for _ in 0..400 {
        let nv = 2 + (splitmix(&mut state) % 3) as usize; // 2..4 vars (FM stays exact and fast)
        let m = 1 + (splitmix(&mut state) % 8) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for v in 0..nv {
                if splitmix(&mut state) % 3 == 0 {
                    c.push(Lit::new(v as u32, splitmix(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        if let Some(cert) = sos_certificate(nv, &cl) {
            assert!(!sat(nv, &cl), "SoS refuted a SATISFIABLE formula: {cl:?}");
            assert!(
                certificate_is_valid(nv, &cl, &cert),
                "the Farkas certificate must re-check: {cl:?}"
            );
        }
    }
}

/// A satisfiable formula is refuted at no point — isolated soundness.
#[test]
fn satisfiable_formulas_are_never_refuted() {
    let cl = vec![
        vec![Lit::new(0, true), Lit::new(1, true)],
        vec![Lit::new(0, false), Lit::new(2, true)],
    ];
    assert!(sat(3, &cl));
    assert!(!sos_refutes(3, &cl), "a SAT formula must not be refuted");
}

/// **The ordered-field power: an integrality gap degree 2 closes.** `x = y` (`x∨¬y`, `¬x∨y`) together
/// with `x ≠ y` (`x∨y`, `¬x∨¬y`) is UNSAT, but its degree-1 LP relaxation is feasible at `x=y=½`.
/// The degree-2 lift — the product `z=xy` with its McCormick envelope, the squares, and the
/// Sherali–Adams clause products — refutes it: exactly the cut Nullstellensatz over GF(2) cannot make,
/// because it lives in the order, not the field.
#[test]
fn sos_closes_an_integrality_gap_that_is_linear_feasible() {
    let cl = vec![
        vec![Lit::new(0, true), Lit::new(1, false)],
        vec![Lit::new(0, false), Lit::new(1, true)],
        vec![Lit::new(0, true), Lit::new(1, true)],
        vec![Lit::new(0, false), Lit::new(1, false)],
    ];
    assert!(!sat(2, &cl), "x=y ∧ x≠y is UNSAT");
    let cert = sos_certificate(2, &cl).expect("degree-2 SoS closes the x=y=½ integrality gap");
    assert!(
        certificate_is_valid(2, &cl, &cert),
        "and its certificate re-checks"
    );
}

/// **Strictly beyond the purely-linear cut.** The degree-1 relaxation (box + clauses only, no
/// products or squares) is feasible on the gap instance — no linear Farkas refutation exists — so the
/// degree-2 lift genuinely adds power, not just repackaging the linear engine.
#[test]
fn the_degree_2_lift_refutes_where_the_linear_relaxation_cannot() {
    let cl = vec![
        vec![Lit::new(0, true), Lit::new(1, false)],
        vec![Lit::new(0, false), Lit::new(1, true)],
        vec![Lit::new(0, true), Lit::new(1, true)],
        vec![Lit::new(0, false), Lit::new(1, false)],
    ];
    // Degree-1 only: box + clause inequalities, no products/squares.
    let one = Poly::constant(1);
    let mut polys: Vec<Poly> = Vec::new();
    for i in 0..2 {
        polys.push(Poly::x(i).neg());
        polys.push(one.sub(&Poly::x(i)).neg());
    }
    for c in &cl {
        let mut p = Poly::constant(-1);
        for l in c {
            p = p.add(&lit_value(l));
        }
        polys.push(p.neg());
    }
    let rows: Vec<Row> = polys
        .iter()
        .enumerate()
        .map(|(i, p)| poly_to_row(p, 2, i))
        .collect();
    assert!(
        farkas_refute(rows, 2 + 4).is_none(),
        "the degree-1 relaxation is feasible (x=y=½)"
    );
    assert!(sos_refutes(2, &cl), "but the degree-2 lift refutes it");
}
