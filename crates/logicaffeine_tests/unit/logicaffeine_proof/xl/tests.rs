use super::*;

struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn poly<F: Field>(terms: &[(&[u32], F)]) -> Poly<F> {
    let mut p: Poly<F> = BTreeMap::new();
    for (m, c) in terms {
        if !c.is_zero() {
            p.insert(m.to_vec(), *c);
        }
    }
    p
}

/// A random dense quadratic system whose common zero includes the planted `s`: each poly is
/// a random quadratic with its constant chosen so it vanishes at `s`. With `m` past the
/// number of degree-2 monomials the linearized system determines the variables uniquely.
fn planted_system<F: Field>(
    n: usize,
    s: &[F],
    m: usize,
    rng: &mut R,
    mut gen: impl FnMut(&mut R) -> F,
) -> MQSystem<F> {
    let quad = macaulay::monomials_up_to_degree(n as u32, 2, F::MULTILINEAR);
    let mut polys = Vec::new();
    for _ in 0..m {
        let mut p: Poly<F> = BTreeMap::new();
        for mono in &quad {
            if mono.is_empty() {
                continue;
            }
            let c = gen(rng);
            if !c.is_zero() {
                p.insert(mono.clone(), c);
            }
        }
        // Set the constant so the equation vanishes at s: const = −eval(rest, s).
        let cst = F::zero().sub(eval(&p, s));
        if !cst.is_zero() {
            p.insert(one(), cst);
        }
        polys.push(p);
    }
    MQSystem { n, polys }
}

fn gf2(rng: &mut R) -> Gf2 {
    Gf2((rng.next() & 1) as u8)
}
fn gf256(rng: &mut R) -> Gf256 {
    Gf256(rng.next() as u8)
}

// ── RED-1: GF(2) — a hand-built system with a known unique solution ──────────────

#[test]
fn xl_solves_an_explicit_gf2_system() {
    // x0 + 1 = 0  ⟹ x0 = 1;   x0·x1 = 0;   x1 = 0.  Unique solution [1, 0].
    let sys = MQSystem {
        n: 2,
        polys: vec![
            poly(&[(&[0u32][..], Gf2(1)), (&[][..], Gf2(1))]),
            poly(&[(&[0u32, 1][..], Gf2(1))]),
            poly(&[(&[1u32][..], Gf2(1))]),
        ],
    };
    assert_eq!(xl_solve(&sys, 2), Some(vec![Gf2(1), Gf2(0)]));
}

#[test]
fn xl_recovers_the_planted_secret_of_an_overdetermined_gf2_system() {
    let mut rng = R(0xDEAD_BEEF_0000_0001);
    for trial in 0..20 {
        let n = 4;
        let s: Vec<Gf2> = (0..n).map(|_| gf2(&mut rng)).collect();
        // 10 degree-2 monomials over 4 vars; 30 equations over-determines it.
        let sys = planted_system(n, &s, 30, &mut rng, gf2);
        let got = xl_solve(&sys, 2).expect("overdetermined GF(2) MQ must linearize");
        assert!(
            sys.satisfied_by(&got),
            "trial {trial}: recovered x must satisfy the system"
        );
        assert_eq!(
            got, s,
            "trial {trial}: determined system must recover the planted secret"
        );
    }
}

// ── RED-2: GF(256) — forge a preimage of a toy multivariate public map ───────────

#[test]
fn xl_forges_a_preimage_of_a_toy_gf256_multivariate_map() {
    // The multivariate-signature attack surface: given a quadratic public map, find ANY
    // preimage of a target. A forgery does not need the secret — a valid preimage suffices.
    let mut rng = R(0x0BAD_C0DE_1234_5678);
    for trial in 0..20 {
        let n = 4;
        let s: Vec<Gf256> = (0..n).map(|_| gf256(&mut rng)).collect();
        let sys = planted_system(n, &s, 40, &mut rng, gf256);
        let got = xl_solve(&sys, 2).expect("dense GF(256) MQ must linearize at degree 2");
        assert!(
            sys.satisfied_by(&got),
            "trial {trial}: forged preimage must satisfy the map"
        );
    }
}

#[test]
fn xl_recovers_the_planted_secret_of_an_overdetermined_gf256_system() {
    let mut rng = R(0xF00D_CAFE_9999_0001);
    for trial in 0..12 {
        let n = 4;
        let s: Vec<Gf256> = (0..n).map(|_| gf256(&mut rng)).collect();
        let sys = planted_system(n, &s, 60, &mut rng, gf256);
        let got = xl_solve(&sys, 2).expect("heavily overdetermined GF(256) MQ solves");
        assert!(sys.satisfied_by(&got));
        assert_eq!(
            got, s,
            "trial {trial}: determined GF(256) system recovers the secret"
        );
    }
}

// ── The pit-of-success contract: xl_solve NEVER emits a non-solution ─────────────

#[test]
fn any_solution_xl_returns_actually_satisfies_the_system_gf256() {
    let mut rng = R(0x1122_3344_5566_7788);
    for _ in 0..300 {
        let n = 3 + (rng.next() % 2) as usize; // 3 or 4 vars
        let s: Vec<Gf256> = (0..n).map(|_| gf256(&mut rng)).collect();
        let m = 5 + (rng.next() % 40) as usize; // from under- to over-determined
        let sys = planted_system(n, &s, m, &mut rng, gf256);
        if let Some(x) = xl_solve(&sys, 2) {
            assert_eq!(x.len(), n);
            assert!(sys.satisfied_by(&x), "xl_solve returned a non-solution");
        }
    }
}

#[test]
fn any_solution_xl_returns_actually_satisfies_the_system_gf2() {
    let mut rng = R(0x99AA_BBCC_DDEE_FF00);
    for _ in 0..300 {
        let n = 3 + (rng.next() % 3) as usize; // 3..=5 vars
        let s: Vec<Gf2> = (0..n).map(|_| gf2(&mut rng)).collect();
        let m = 4 + (rng.next() % 30) as usize;
        let sys = planted_system(n, &s, m, &mut rng, gf2);
        if let Some(x) = xl_solve(&sys, 2) {
            assert!(sys.satisfied_by(&x), "xl_solve returned a non-solution");
        }
    }
}

#[test]
fn xl_declines_a_contradictory_system() {
    // x0 = 0 and x0 = 1 over GF(2): no common zero.
    let sys = MQSystem {
        n: 1,
        polys: vec![
            poly(&[(&[0u32][..], Gf2(1))]),
            poly(&[(&[0u32][..], Gf2(1)), (&[][..], Gf2(1))]),
        ],
    };
    assert_eq!(xl_solve(&sys, 3), None);
}

#[test]
fn eval_and_satisfaction_agree_with_hand_computation() {
    // p(x) = x0·x1 + x2 + 3 over GF(256), check at a point.
    let p: Poly<Gf256> = poly(&[
        (&[0u32, 1][..], Gf256(1)),
        (&[2u32][..], Gf256(1)),
        (&[][..], Gf256(3)),
    ]);
    let x = [Gf256(2), Gf256(4), Gf256(5)];
    // 2*4 = 8 in GF(256); 8 xor 5 xor 3 = 8^5^3 = 14.
    let expect = Gf256(logicaffeine_base::gf256::mul(2, 4) ^ 5 ^ 3);
    assert_eq!(eval(&p, &x), expect);
}
