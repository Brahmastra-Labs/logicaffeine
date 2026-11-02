//! # Arithmetization-oriented collapse over GF(p) — the modern algebraic frontier (Ding program)
//!
//! The 2024–2025 state of the art in cryptanalysis is **algebraic / Gröbner-basis attacks on
//! arithmetization-oriented (AO) primitives** — Poseidon, Neptune, Ciminion, Hydra — via FreeLunch
//! (Crypto 2024), CheapLunch and subspace-trail Gröbner (2025). These live over a large prime field
//! `GF(p)` and use low-degree power maps (`x ↦ x³`, `x⁵`, …); the attack solves an over-determined
//! polynomial system and its cost is governed by the **solving degree** — Ding's degree of regularity.
//!
//! Our XL engine (`xl.rs`) is **field-generic** over the `Field` trait but was only instantiated at
//! `GF(2)` and `GF(2⁸)`. This study instantiates it over **`GF(p)` for arbitrary prime `p`** (via a
//! const-generic prime-field element) and collapses AO-style **cubic** systems — extending the toolbox
//! to exactly the field and degree the modern AO attacks operate in. Toy scale, faithful method:
//! full Poseidon/FreeLunch is the same collapse at deployed parameters.
//!
//! Sources: FreeLunch (eprint 2024/347), CheapLunch (eprint 2025/2040), Poseidon/Neptune Gröbner
//! (eprint 2025/954).

use logicaffeine_proof::macaulay::{monomials_up_to_degree, one};
use logicaffeine_proof::xl::{eval, xl_solve, Field, MQSystem, Poly};
use std::collections::BTreeMap;

/// A prime-field element `GF(P)` for any prime `P` — the field AO primitives live over. Const-generic
/// so one impl serves every prime; `MULTILINEAR = false` (genuine higher-degree monomials, not the
/// `x² = x` Boolean ring).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fp<const P: u64>(u64);

impl<const P: u64> Field for Fp<P> {
    const MULTILINEAR: bool = false;
    fn zero() -> Self {
        Fp(0)
    }
    fn one() -> Self {
        Fp(1 % P)
    }
    fn add(self, o: Self) -> Self {
        Fp((self.0 + o.0) % P)
    }
    fn sub(self, o: Self) -> Self {
        Fp((self.0 + P - o.0) % P)
    }
    fn mul(self, o: Self) -> Self {
        Fp(((self.0 as u128 * o.0 as u128) % P as u128) as u64)
    }
    fn inv(self) -> Self {
        // Fermat: a^(p-2) mod p.
        let (mut r, mut b, mut e) = (1u128, self.0 as u128, P - 2);
        while e > 0 {
            if e & 1 == 1 {
                r = r * b % P as u128;
            }
            b = b * b % P as u128;
            e >>= 1;
        }
        Fp(r as u64)
    }
}

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn rand_fp<const P: u64>(rng: &mut SplitMix) -> Fp<P> {
    Fp(rng.next() % P)
}

/// An over-determined **cubic** system over `GF(P)` with a planted common zero `s`: `m` random
/// polynomials of degree ≤ 3, each with its constant chosen so it vanishes at `s`. This is the AO
/// attack shape — dense low-degree equations over a prime field with a hidden solution.
fn planted_cubic_system<const P: u64>(
    n: usize,
    s: &[Fp<P>],
    m: usize,
    rng: &mut SplitMix,
) -> MQSystem<Fp<P>> {
    let monos = monomials_up_to_degree(n as u32, 3, false);
    let mut polys = Vec::with_capacity(m);
    for _ in 0..m {
        let mut p: Poly<Fp<P>> = BTreeMap::new();
        for mono in &monos {
            if mono.is_empty() {
                continue; // the constant is fixed below to plant the zero
            }
            let c = rand_fp::<P>(rng);
            if !c.is_zero() {
                p.insert(mono.clone(), c);
            }
        }
        let cst = Fp::<P>::zero().sub(eval(&p, s));
        if !cst.is_zero() {
            p.insert(one(), cst);
        }
        polys.push(p);
    }
    MQSystem { n, polys }
}

/// XL collapses an over-determined cubic system over a **31-bit prime field** and recovers the hidden
/// solution — the AO/FreeLunch attack substrate, now runnable in the toolbox. The engine verifies the
/// recovered point against the original system before returning, so a `Some` is a genuine solve.
#[test]
fn xl_collapses_an_ao_style_cubic_system_over_a_31bit_prime() {
    const P: u64 = 2_147_483_647; // 2^31 − 1 (Mersenne prime)
    let mut rng = SplitMix(0xA0_C0FFEE_1234);
    for trial in 0..20 {
        let n = 3;
        let s: Vec<Fp<P>> = (0..n).map(|_| rand_fp::<P>(&mut rng)).collect();
        // 60 cubic equations over 3 vars (19 degree-≤3 monomials) — heavily over-determined.
        let sys = planted_cubic_system::<P>(n, &s, 60, &mut rng);
        let got = xl_solve(&sys, 3)
            .expect("over-determined GF(p) cubic system must linearize at degree 3");
        assert_eq!(
            got, s,
            "trial {trial}: XL over GF(2^31−1) must recover the planted AO secret"
        );
    }
}

/// Field-genericity is the point: the *same* engine collapses AO systems over a *different* prime with
/// no code change — the toolbox works at whatever `GF(p)` a primitive is instantiated over.
#[test]
fn the_same_engine_collapses_over_a_different_prime() {
    const P: u64 = 65_537; // Fermat prime F_4
    let mut rng = SplitMix(0x50_0D_5EED);
    for trial in 0..20 {
        let n = 3;
        let s: Vec<Fp<P>> = (0..n).map(|_| rand_fp::<P>(&mut rng)).collect();
        let sys = planted_cubic_system::<P>(n, &s, 60, &mut rng);
        let got = xl_solve(&sys, 3).expect("GF(65537) cubic system must linearize");
        assert_eq!(
            got, s,
            "trial {trial}: same XL engine recovers the secret over GF(65537)"
        );
    }
}

/// The pit-of-success contract carries to `GF(p)`: whatever XL returns is verified against the original
/// system, so it can never emit a spurious solution — at worst it needs a higher degree.
#[test]
fn xl_never_emits_a_non_solution_over_gfp() {
    const P: u64 = 2_147_483_647;
    let mut rng = SplitMix(0x11_22_33_44_55);
    for _ in 0..200 {
        let n = 2 + (rng.next() % 2) as usize; // 2 or 3 vars
        let s: Vec<Fp<P>> = (0..n).map(|_| rand_fp::<P>(&mut rng)).collect();
        let m = 8 + (rng.next() % 60) as usize; // under- to over-determined
        let sys = planted_cubic_system::<P>(n, &s, m, &mut rng);
        if let Some(x) = xl_solve(&sys, 3) {
            assert!(
                sys.satisfied_by(&x),
                "XL over GF(p) returned a non-solution"
            );
        }
    }
}
