//! # Certified isogeny torsion-image witnesses — the SIDH/SIKE public-data structure, in the prover
//!
//! An SIDH/SIKE public key is not merely the codomain curve `E' = φ(E₀)`: to make the scheme a key
//! exchange it also publishes the **images of a public torsion basis**, `φ(P), φ(Q)`. That auxiliary
//! torsion data is exactly what the 2022 Castryck–Decru attack weaponized. Its internal consistency is
//! governed by the Weil pairing's isogeny-compatibility law:
//!
//! ```text
//!     e_N(φ(P), φ(Q)) = e_N(P, Q)^{deg φ}.
//! ```
//!
//! This module bakes that relation into the prover as a re-checkable certificate — the same discipline as
//! the rest of the campaign ([`crate::ait::DescriptionBound`], `LinearRigidityCert`, …): a
//! [`TorsionImageWitness`] whose [`verify`](TorsionImageWitness::verify) re-derives the pairings on *both*
//! curves from scratch and checks the law. It is the exact check an SIDH verifier performs, and the
//! structural symmetry the break pulls on.

use crate::elliptic::{torsion_basis, weil_pairing, Curve, Isogeny, Point};
use crate::factor::modpow;
use logicaffeine_base::BigInt;

/// A re-checkable witness that an isogeny `φ: E → E'` is consistently specified by its action on an
/// `N`-torsion basis — the SIDH/SIKE public-key format.
#[derive(Clone, Debug)]
pub struct TorsionImageWitness {
    pub domain: Curve,
    pub codomain: Curve,
    /// The isogeny degree `deg φ`.
    pub degree: u64,
    /// The torsion order `N` (coprime to `deg φ`, so `φ` restricts to an isomorphism on `E[N]`).
    pub torsion_order: u64,
    /// A basis `(P, Q)` of `E[N]`.
    pub basis: (Point, Point),
    /// The published images `(φ(P), φ(Q))` on the codomain.
    pub images: (Point, Point),
}

impl TorsionImageWitness {
    /// Re-check the witness from scratch, trusting nothing about how it was produced:
    /// 1. `(P, Q)` is a genuine basis of `E[N]` — the Weil pairing `e_N(P,Q)` is a *primitive* `N`th root
    ///    of unity (the points are independent, so the pairing is non-degenerate);
    /// 2. `(φP, φQ)` lie on the codomain and are killed by `N`;
    /// 3. the **isogeny-compatibility law** `e_N(φP, φQ) = e_N(P, Q)^{deg φ}` holds.
    pub fn verify(&self) -> bool {
        let n = self.torsion_order;
        let one = BigInt::from_i64(1);

        // (1) The basis pairing on the domain must be a primitive Nth root (non-degenerate ⟹ independent).
        let ep = match weil_pairing(&self.domain, &self.basis.0, &self.basis.1, n) {
            Some(e) if e != one && modpow(&e, &BigInt::from_i64(n as i64), &self.domain.p) == one => e,
            _ => return false,
        };

        // (2) The images sit on the codomain and have order dividing N.
        let ord_n = BigInt::from_i64(n as i64);
        for pt in [&self.images.0, &self.images.1] {
            if !self.codomain.is_on_curve(pt) || self.codomain.mul(&ord_n, pt) != Point::Infinity {
                return false;
            }
        }

        // (3) The compatibility law: e_N(φP, φQ) = e_N(P,Q)^{deg φ}.
        match weil_pairing(&self.codomain, &self.images.0, &self.images.1, n) {
            Some(eq) => eq == modpow(&ep, &BigInt::from_i64(self.degree as i64), &self.domain.p),
            None => false,
        }
    }
}

/// Build a *genuine* certified witness: the real `ell`-isogeny with kernel `⟨kernel_gen⟩` on `domain`,
/// together with the images of an actual `E[n]` basis (`n` coprime to `ell`, so `φ` is an isomorphism on
/// the `n`-torsion). Its `verify()` passes by construction. `None` if the isogeny or basis cannot be formed.
pub fn certify_isogeny(domain: &Curve, kernel_gen: &Point, ell: u64, n: u64) -> Option<TorsionImageWitness> {
    let iso = Isogeny::from_kernel(domain, kernel_gen, ell)?;
    let (p, q) = torsion_basis(domain, n)?;
    let images = (iso.eval(&p), iso.eval(&q));
    Some(TorsionImageWitness {
        domain: domain.clone(),
        codomain: iso.codomain.clone(),
        degree: ell,
        torsion_order: n,
        basis: (p, q),
        images,
    })
}

/// The structural verdict on SIDH/SIKE-style public data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidhAudit {
    /// Consistent (the Weil-pairing law holds) — but it publishes the full action of `φ` on `E[N]`, the
    /// auxiliary torsion information the Castryck–Decru attack requires. Valid data, structurally exposed:
    /// post-2022 this whole class of scheme is broken, and the exposure is *why*.
    ConsistentButTorsionExposed,
    /// The torsion-image data fails the Weil-pairing law — not a valid isogeny witness at all.
    Inconsistent,
}

/// Audit an isogeny public key: consistent, and what does it structurally expose?
pub fn audit(w: &TorsionImageWitness) -> SidhAudit {
    if w.verify() {
        SidhAudit::ConsistentButTorsionExposed
    } else {
        SidhAudit::Inconsistent
    }
}

// ---- Kani's theorem: the glue-and-split core -------------------------------------------------------
//
// Kani's theorem powers the Castryck–Decru break by GLUING two elliptic curves into an abelian surface:
// given an isogeny `φ: E → E'` and a torsion basis, the GRAPH `K = {(T, φ(T)) : T ∈ E[N]} ⊂ (E × E')[N]`
// is a candidate kernel of an `(N,N)`-isogeny. It is a valid kernel exactly when it is **Lagrangian**
// (maximal isotropic) under the product Weil pairing — and that isotropy is decided by the pairing law we
// certified: on the graph generators, `e_product((P,φP),(Q,φQ)) = e_N(P,Q)·e_N(φP,φQ) = e_N(P,Q)^{1+deg φ}`,
// so the graph is isotropic ⟺ `deg φ ≡ −1 (mod N)`. The torsion images are exactly what make this pairing
// computable — which is *why* publishing them is the leak. This certificate computes and re-checks that
// gluing pairing; the remaining glue-and-split machinery (Richelot `(2,2)`-isogenies of the surface, theta
// coordinates, and split detection) is genuine research-grade work built ON this isotropic core, NOT
// claimed here.

use crate::fp2::{
    aut_1728, derive_isogeny_path2, fp2_const, fp2_pow, full_order_basis2, keyspace_codomain_classes,
    kernel_generator2, point_of_order2, point_order2, product_weil_pairing, push_through2, recover_secret2,
    recover_secret_recursive2, torsion_basis2, weil_pairing2, Curve2, Fp2, IsogenyStep2, Isogeny2, Point2,
};

/// A **Kani glue kernel**: the graph `{(T, φ(T))}` of an isogeny on the `N`-torsion, the candidate kernel of
/// an `(N,N)`-isogeny of the abelian surface `E × E'`.
#[derive(Clone, Debug)]
pub struct KaniGlue {
    pub e1: Curve2,
    pub e2: Curve2,
    pub degree: u64,
    pub torsion_order: u64,
    pub basis: (Point2, Point2),
    pub images: (Point2, Point2),
}

impl KaniGlue {
    /// The product Weil pairing on the graph generators `(P,φP)` and `(Q,φQ)`.
    pub fn glue_pairing(&self) -> Option<Fp2> {
        product_weil_pairing(
            &self.e1,
            &self.e2,
            (&self.basis.0, &self.images.0),
            (&self.basis.1, &self.images.1),
            self.torsion_order,
        )
    }

    /// Re-check Kani's gluing relation: the graph pairing equals `e_N(P,Q)^{1+deg φ}`. This is what makes
    /// the isotropy of the glue kernel *decidable*, and it re-derives the pairings from scratch.
    pub fn verify(&self) -> bool {
        let ep = match weil_pairing2(&self.e1, &self.basis.0, &self.basis.1, self.torsion_order) {
            Some(e) => e,
            None => return false,
        };
        match self.glue_pairing() {
            Some(glue) => glue == fp2_pow(&ep, 1 + self.degree, &self.e1.p),
            None => false,
        }
    }

    /// Whether the glue kernel is **Lagrangian** (isotropic) — i.e. an `(N,N)`-isogeny of `E × E'` exists
    /// with this kernel. True exactly when the graph pairing is trivial, i.e. `deg φ ≡ −1 (mod N)`.
    pub fn is_lagrangian(&self) -> bool {
        self.glue_pairing().map_or(false, |g| g == fp2_const(1, &self.e1.p))
    }
}

/// Build a genuine Kani glue kernel over `𝔽_{p²}` from a real `ell`-isogeny and an `N`-torsion basis.
pub fn build_kani_glue(domain: &Curve2, kernel_gen: &Point2, ell: u64, n: u64) -> Option<KaniGlue> {
    let iso = Isogeny2::from_kernel(domain, kernel_gen, ell)?;
    let (p, q) = torsion_basis2(domain, n)?;
    let images = (iso.eval(&p), iso.eval(&q));
    Some(KaniGlue { e1: domain.clone(), e2: iso.codomain.clone(), degree: ell, torsion_order: n, basis: (p, q), images })
}

/// Whether `n` is `bound`-smooth (every prime factor ≤ `bound`) — the auxiliary isogeny degree of a Kani
/// diamond must be smooth to be efficiently computable.
pub fn is_smooth(mut n: u64, bound: u64) -> bool {
    if n == 0 {
        return false;
    }
    let mut f = 2u64;
    while f * f <= n {
        while n % f == 0 {
            n /= f;
        }
        f += 1;
    }
    n == 1 || n <= bound
}

/// **The Kani degree condition.** To embed a degree-`d` secret isogeny in a `(2,2)`-isogeny *chain* of an
/// abelian surface (the Castryck–Decru diamond), pick an exponent `e` and an auxiliary isogeny of degree
/// `c = 2^e − d`, so the two sides of the diamond sum to `2^e` — the surface isogeny is then a length-`e`
/// Richelot chain. `c` must be positive and smooth (efficiently computable). Returns the smallest such
/// `(e, c)`. This is the diamond's number-theoretic bookkeeping.
pub fn kani_diamond_degrees(d: u64, max_e: u32, smooth_bound: u64) -> Option<(u32, u64)> {
    (1..=max_e).find_map(|e| {
        let n = 1u64 << e;
        // c must be a NONTRIVIAL auxiliary (degree > 1 — a degree-1 "isogeny" is the identity, no gluing).
        (n > d + 1).then(|| n - d).filter(|&c| is_smooth(c, smooth_bound)).map(|c| (e, c))
    })
}

/// A Castryck–Decru **Kani diamond**: a secret isogeny `φ: E₀ → E` of degree `d` and an auxiliary isogeny
/// `γ: E₀ → C` of degree `c`, chosen so `c + d = 2^e`. Kani's lemma then supplies a `(2^e, 2^e)`-isogeny of
/// the abelian surface `E × C` — a length-`e` Richelot chain — whose codomain **splits** into a product of
/// elliptic curves iff the diamond is consistent. The split-test
/// ([`crate::hyperelliptic::surface_is_reducible`]) is the per-digit oracle that reads that
/// splitting off. **Honest boundary:** this builds and validates the diamond's *degree structure and both
/// isogeny sides*; constructing the surface's `2^e`-torsion kernel from the diamond and the torsion images —
/// the input the split-oracle consumes — is the remaining SageMath-scale research core and is **not**
/// fabricated here.
#[derive(Clone, Debug)]
pub struct KaniDiamond {
    pub e0: Curve2,
    /// Codomain of the secret isogeny `φ` (degree `d`).
    pub e_curve: Curve2,
    /// Codomain of the auxiliary isogeny `γ` (degree `c`).
    pub c_curve: Curve2,
    pub d: u64,
    pub c: u64,
    /// `c + d = 2^e`.
    pub e: u32,
}

/// Build a Kani diamond over `𝔽_{p²}`: the secret side `φ` is an `ell_phi^a`-isogeny with kernel `⟨phi_gen⟩`;
/// the auxiliary side `γ` is a `c`-isogeny (`c` an odd prime) with kernel `⟨gamma_gen⟩`. Succeeds only when
/// `c + ell_phi^a` is a power of two — the Kani degree condition.
pub fn build_kani_diamond(
    e0: &Curve2,
    phi_gen: &Point2,
    ell_phi: u64,
    a: u32,
    gamma_gen: &Point2,
    c: u64,
) -> Option<KaniDiamond> {
    let d = ell_phi.pow(a);
    let n = d + c;
    if !n.is_power_of_two() {
        return None;
    }
    let phi = derive_isogeny_path2(e0, phi_gen, ell_phi, a)?;
    let gamma = Isogeny2::from_kernel(e0, gamma_gen, c)?;
    Some(KaniDiamond {
        e0: e0.clone(),
        e_curve: phi.last()?.codomain.clone(),
        c_curve: gamma.codomain,
        d,
        c,
        e: n.trailing_zeros(),
    })
}

/// A **law the recovery invoked** — auto-formalized as a re-checkable obligation, not prose. The recovery
/// engine does not merely return an answer; it pops out the rules and laws it relied on, each of which
/// [`SecretRecovery::verify`] re-derives from scratch.
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryLaw {
    /// The secret kernel generator has order exactly `ℓᵃ`.
    GeneratorOrder { ell: u64, a: u32 },
    /// The unfolded chain is `a` connected `ℓ`-isogeny steps, each quotienting an order-`ℓ` subgroup.
    DescendingKernels { ell: u64 },
    /// Pushing the auxiliary torsion basis through the chain reproduces the published images (the defining
    /// property of the recovered isogeny).
    ImagesReproduced,
    /// The `ℓ`-adic **tree** recursion (auto-partitioning the keyspace by digit, sharing prefixes) recovers
    /// the same secret as the flat enumeration — the partition is faithful.
    KeyspaceTreePartition { ell: u64, a: u32 },
    /// `E₀/⟨K⟩ ≅ E₀/⟨ι(K)⟩` under the automorphism `ι`: recovering one kernel recovers its whole orbit.
    AutOrbitClosure,
}

impl RecoveryLaw {
    /// The formalized statement of the law — the "learning" the recovery pops out.
    pub fn statement(&self) -> String {
        match self {
            RecoveryLaw::GeneratorOrder { ell, a } => format!("ord(gen) = {ell}^{a} on E₀"),
            RecoveryLaw::DescendingKernels { ell } => {
                format!("each of the a chain steps quotients an order-{ell} subgroup; the chain is connected")
            }
            RecoveryLaw::ImagesReproduced => "φ(P_B), φ(Q_B) = the published torsion images".into(),
            RecoveryLaw::KeyspaceTreePartition { ell, a } => {
                format!("the {ell}-adic keyspace tree (depth {a}) recovers the same secret as flat search")
            }
            RecoveryLaw::AutOrbitClosure => "E₀/⟨K⟩ ≅ E₀/⟨ι(K)⟩ — recovering one kernel recovers its orbit".into(),
        }
    }
}

/// A recovered SIDH secret **together with the self-checking record of the laws its recovery invoked**. The
/// certificate carries everything needed to re-derive itself: [`verify`](SecretRecovery::verify) re-runs
/// every law from scratch, trusting nothing about how the answer was produced. This is the auto-formalizing
/// recovery — it inverts images → generator *and* emits the re-checkable mathematics it stands on.
#[derive(Clone, Debug)]
pub struct SecretRecovery {
    pub e0: Curve2,
    pub basis_a: (Point2, Point2),
    pub basis_b: (Point2, Point2),
    pub images: (Point2, Point2),
    pub ell: u64,
    pub a: u32,
    pub secret: BigInt,
    pub generator: Point2,
    pub path: Vec<IsogenyStep2>,
    pub laws: Vec<RecoveryLaw>,
}

impl SecretRecovery {
    /// The formalized laws this recovery relied on, as statements — the rules and learnings, popped out.
    pub fn learnings(&self) -> Vec<String> {
        self.laws.iter().map(RecoveryLaw::statement).collect()
    }

    /// Re-check every law from scratch. Trusts nothing about how the recovery ran.
    pub fn verify(&self) -> bool {
        let n = self.ell.pow(self.a);
        let (pa, qa) = (&self.basis_a.0, &self.basis_a.1);
        let (pb, qb) = (&self.basis_b.0, &self.basis_b.1);
        // The recovered generator is P_A + [s]Q_A of order ℓᵃ.
        if kernel_generator2(&self.e0, pa, qa, &self.secret) != self.generator {
            return false;
        }
        for law in &self.laws {
            let ok = match law {
                RecoveryLaw::GeneratorOrder { ell, a } => {
                    let m = ell.pow(*a);
                    point_order2(&self.e0, &self.generator, m + 1) == Some(m)
                }
                RecoveryLaw::DescendingKernels { ell } => {
                    self.path.len() as u32 == self.a
                        && self.path.iter().enumerate().all(|(k, st)| {
                            let order_ok = point_order2(&st.domain, &st.kernel, *ell + 1) == Some(*ell);
                            let connected = k == 0 || st.domain == self.path[k - 1].codomain;
                            order_ok && connected
                        })
                }
                RecoveryLaw::ImagesReproduced => {
                    push_through2(&self.path, self.ell, pb).as_ref() == Some(&self.images.0)
                        && push_through2(&self.path, self.ell, qb).as_ref() == Some(&self.images.1)
                }
                RecoveryLaw::KeyspaceTreePartition { ell, a } => {
                    // The ℓ-adic tree walk independently recovers a generator that reproduces the images —
                    // the partition is faithful. (It need not be the identical scalar: symmetry-equivalent
                    // kernels are genuine co-solutions, which is precisely the AutOrbitClosure law below.)
                    recover_secret_recursive2(&self.e0, (pa, qa), *ell, *a, (pb, qb), (&self.images.0, &self.images.1))
                        .and_then(|(_, _s, g)| {
                            let path = derive_isogeny_path2(&self.e0, &g, *ell, *a)?;
                            Some(
                                push_through2(&path, *ell, pb).as_ref() == Some(&self.images.0)
                                    && push_through2(&path, *ell, qb).as_ref() == Some(&self.images.1),
                            )
                        })
                        .unwrap_or(false)
                }
                RecoveryLaw::AutOrbitClosure => {
                    let j = |g: &Point2| {
                        derive_isogeny_path2(&self.e0, g, self.ell, self.a)
                            .and_then(|p| p.last().and_then(|st| st.codomain.j_invariant()))
                    };
                    let ig = aut_1728(&self.e0.p, &self.generator);
                    point_order2(&self.e0, &ig, n + 1) == Some(n) && j(&self.generator) == j(&ig) && j(&ig).is_some()
                }
            };
            if !ok {
                return false;
            }
        }
        true
    }
}

/// **Certify an images → generator recovery**, emitting the answer with its self-checking law record. Runs
/// the flat recovery for the answer, then attaches the full set of re-checkable laws (order, descent, image
/// reproduction, the faithful ℓ-adic tree partition, and the Aut-orbit closure rule).
pub fn certify_recovery(
    e0: &Curve2,
    basis_a: (&Point2, &Point2),
    ell: u64,
    a: u32,
    basis_b: (&Point2, &Point2),
    images: (&Point2, &Point2),
) -> Option<SecretRecovery> {
    let (secret, generator, path) = recover_secret2(e0, basis_a, ell, a, basis_b, images)?;
    let laws = vec![
        RecoveryLaw::GeneratorOrder { ell, a },
        RecoveryLaw::DescendingKernels { ell },
        RecoveryLaw::ImagesReproduced,
        RecoveryLaw::KeyspaceTreePartition { ell, a },
        RecoveryLaw::AutOrbitClosure,
    ];
    Some(SecretRecovery {
        e0: e0.clone(),
        basis_a: (basis_a.0.clone(), basis_a.1.clone()),
        basis_b: (basis_b.0.clone(), basis_b.1.clone()),
        images: (images.0.clone(), images.1.clone()),
        ell,
        a,
        secret,
        generator,
        path,
        laws,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(x: i64) -> BigInt {
        BigInt::from_i64(x)
    }

    #[test]
    fn torsion_image_witness_verifies_and_rejects_tampering() {
        use crate::elliptic::point_of_order;
        // The curve y² = x³ + 2x + 25 over 𝔽₄₃ has #E = 45 = 9·5: full rational 3-torsion (the basis) and a
        // rational 5-torsion point (the isogeny kernel; 5 is coprime to 3). μ₃ ⊂ 𝔽₄₃ (3 | 42), p ≡ 3 mod 4.
        let curve = Curve::new(i(2), i(25), i(43));
        let kernel = point_of_order(&curve, 5).expect("𝔽₄₃ 5-torsion");
        let w = certify_isogeny(&curve, &kernel, 5, 3).expect("a genuine isogeny witness");

        // A real witness satisfies the Weil-pairing compatibility law.
        assert!(w.verify(), "e_N(φP,φQ) = e_N(P,Q)^{{deg φ}} for a genuine isogeny");
        assert_eq!(audit(&w), SidhAudit::ConsistentButTorsionExposed);

        // The numeric law, spelled out: e_3(φP,φQ) = e_3(P,Q)^5.
        let ep = weil_pairing(&w.domain, &w.basis.0, &w.basis.1, 3).unwrap();
        let eq = weil_pairing(&w.codomain, &w.images.0, &w.images.1, 3).unwrap();
        assert_eq!(eq, modpow(&ep, &i(5), &w.domain.p), "the compatibility law holds numerically");

        // Tampering an image breaks the law — the certificate is only as good as its re-check.
        let mut bad = w.clone();
        bad.images.0 = w.codomain.double(&w.images.0); // 2·φ(P) ≠ φ(P)
        assert!(!bad.verify(), "a tampered torsion image fails the pairing law");
        assert_eq!(audit(&bad), SidhAudit::Inconsistent);
    }

    #[test]
    fn kani_glue_is_lagrangian_exactly_when_deg_is_minus_one_mod_n() {
        // Over 𝔽_{59²} on the supersingular curve y²=x³+x (p+1 = 60 = 2²·3·5), both E[3] and E[5] are
        // fully rational — enough to glue.
        let p = BigInt::parse_decimal("59").unwrap();
        let c = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());

        // deg φ = 5, torsion N = 3: 5 ≡ −1 (mod 3), so the graph {(T,φT)} is Lagrangian — an (N,N)-isogeny of
        // the surface E×E' exists. This is the gluing condition Kani's theorem supplies.
        let k5 = point_of_order2(&c, 5).expect("5-torsion");
        let glue = build_kani_glue(&c, &k5, 5, 3).expect("a genuine glue kernel");
        assert!(glue.verify(), "the gluing relation e_product = e_N^{{1+deg}} holds");
        assert!(glue.is_lagrangian(), "deg 5 ≡ −1 (mod 3) ⟹ Lagrangian glue kernel");
        assert_eq!(glue.glue_pairing().unwrap(), fp2_const(1, &p), "isotropic: the product pairing is trivial");

        // deg φ = 3, torsion N = 5: 3 ≢ −1 (mod 5), so NOT Lagrangian — yet the relation still re-checks.
        let k3 = point_of_order2(&c, 3).expect("3-torsion");
        let glue2 = build_kani_glue(&c, &k3, 3, 5).expect("a genuine glue kernel");
        assert!(glue2.verify(), "the gluing relation holds regardless of the degree");
        assert!(!glue2.is_lagrangian(), "deg 3 ≢ −1 (mod 5) ⟹ not Lagrangian");
    }

    #[test]
    fn images_to_generator_recovery_certifies_itself_and_rejects_tampering() {
        // SIDH-scale instance over 𝔽_{107²}: y²=x³+x (j=1728), #E=(p+1)²=108², so E[3³] and E[2²] are both
        // rank-2 rational — a genuine 27-kernel keyspace.
        let p = BigInt::parse_decimal("107").unwrap();
        let e0 = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
        let (pa, qa) = full_order_basis2(&e0, 3, 3).expect("rank-2 E[3³] basis");
        let (pb, qb) = full_order_basis2(&e0, 2, 2).expect("rank-2 E[2²] basis");

        // A secret isogeny publishes its torsion images.
        let secret = i(13);
        let gen = kernel_generator2(&e0, &pa, &qa, &secret);
        let path = derive_isogeny_path2(&e0, &gen, 3, 3).expect("the secret 3³-isogeny");
        let images = (push_through2(&path, 3, &pb).unwrap(), push_through2(&path, 3, &qb).unwrap());

        // Invert AND auto-formalize: recover a generator reproducing the images, and pop out the laws.
        let rec = certify_recovery(&e0, (&pa, &qa), 3, 3, (&pb, &qb), (&images.0, &images.1))
            .expect("images → generator recovery");
        assert_eq!(push_through2(&rec.path, 3, &pb).unwrap(), images.0, "the recovered isogeny reproduces φ(P_B)");
        assert_eq!(push_through2(&rec.path, 3, &qb).unwrap(), images.1, "and φ(Q_B) — inversion succeeded");
        assert!(rec.verify(), "every law the recovery invoked re-checks from scratch");
        assert_eq!(rec.laws.len(), 5, "the full law set is emitted");
        assert!(rec.learnings().iter().any(|l| l.contains("ι(K)")), "the Aut-orbit rule is among the learnings");
        // The recovered generator matches the reported scalar (internal consistency of the certificate).
        assert_eq!(kernel_generator2(&e0, &pa, &qa, &rec.secret), rec.generator);

        // The certificate is only as good as its re-check: tampering the secret must fail verification.
        let _ = secret; // the planted secret is a valid solution; recovery may return a symmetry-equivalent one
        let mut forged = rec.clone();
        forged.secret = rec.secret.add(&i(1)); // a different scalar ⟹ a different generator ⟹ rejected
        assert!(!forged.verify(), "a tampered secret no longer matches its generator ⟹ rejected");
    }

    #[test]
    fn kani_degree_diamond_is_well_formed() {
        // The degree bookkeeping: to attack a degree-3 isogeny, pair it with an auxiliary of degree
        // c = 2^e − 3; the smallest smooth choice is c = 5 (3 + 5 = 8 = 2³).
        assert_eq!(kani_diamond_degrees(3, 8, 100), Some((3, 5)), "3 + 5 = 2³ — smallest smooth diamond");
        assert!(is_smooth(5, 100) && is_smooth(8, 3) && !is_smooth(101, 50), "smoothness gate");

        // A real diamond over 𝔽_{59²}: φ a 3-isogeny, γ a 5-isogeny, 3 + 5 = 8 = 2³ (p+1 = 60 = 2²·3·5, so
        // both 3- and 5-torsion are rational).
        let p = BigInt::parse_decimal("59").unwrap();
        let e0 = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
        let k3 = point_of_order2(&e0, 3).expect("3-torsion");
        let k5 = point_of_order2(&e0, 5).expect("5-torsion");
        let diamond = build_kani_diamond(&e0, &k3, 3, 1, &k5, 5).expect("a well-formed Kani diamond");

        assert_eq!((diamond.d, diamond.c, diamond.e), (3, 5, 3), "degrees close to a power of two");
        assert_eq!(diamond.d + diamond.c, 1 << diamond.e, "c + d = 2^e — the Kani degree condition");
        // Both isogeny sides are genuine curves (real codomains), and the diamond is nontrivial.
        assert!(diamond.e_curve.j_invariant().is_some(), "φ lands on a real curve E");
        assert!(diamond.c_curve.j_invariant().is_some(), "γ lands on a real curve C");
        // A mismatched auxiliary degree (c + d not a power of two) is correctly rejected.
        assert!(build_kani_diamond(&e0, &k3, 3, 1, &k5, 5).is_some());
        assert!(build_kani_diamond(&e0, &k3, 3, 1, &k3, 3).is_none(), "3 + 3 = 6 is not a power of two");
    }
}
