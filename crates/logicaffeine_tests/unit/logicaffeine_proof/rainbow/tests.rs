use super::*;

/// Symmetric AND asymmetric layer sizes. Distinct `v1`/`o1`/`o2` are the load-bearing rigor:
/// they expose any layer-index confusion (`v1`↔`o1`↔`o2`, the `l1 = v1+o1` offset) that a
/// symmetric `(2,2,2)` instance would mask.
const PARAM_SETS: &[RainbowParams] = &[
    RainbowParams {
        v1: 2,
        o1: 2,
        o2: 2,
    },
    RainbowParams {
        v1: 3,
        o1: 2,
        o2: 1,
    },
    RainbowParams {
        v1: 1,
        o1: 3,
        o2: 2,
    },
    RainbowParams {
        v1: 2,
        o1: 1,
        o2: 3,
    },
    RainbowParams {
        v1: 4,
        o1: 3,
        o2: 2,
    },
    RainbowParams {
        v1: 1,
        o1: 1,
        o2: 1,
    },
];

fn tag(p: RainbowParams) -> u64 {
    (p.v1 * 10_000 + p.o1 * 100 + p.o2) as u64
}

fn hash_of(t: u64, m: usize) -> Vec<u8> {
    let mut r = Rng(t);
    (0..m).map(|_| r.byte()).collect()
}

fn identity(k: usize) -> Vec<Vec<u8>> {
    (0..k)
        .map(|i| (0..k).map(|j| if i == j { 1 } else { 0 }).collect())
        .collect()
}

#[test]
fn a_signature_verifies_and_a_tampered_one_does_not_at_every_parameter_set() {
    for &p in PARAM_SETS {
        for seed in 0..12u64 {
            let (sk, pk) = keygen(p, 0x5EED_0000 ^ seed ^ tag(p));
            let msg = hash_of(0x1111_0001 ^ seed ^ tag(p), p.m());
            let sig = sign(&sk, &msg).expect("Oil-and-Vinegar inversion should succeed");
            assert_eq!(sig.len(), p.n(), "{p:?}: signature length is n");
            assert!(
                verify(&pk, &msg, &sig),
                "{p:?} seed {seed}: a real signature must verify"
            );

            // Every single-byte flip of the signature must be rejected.
            for pos in 0..sig.len() {
                let mut bad = sig.clone();
                bad[pos] ^= 0x9D;
                assert!(
                    !verify(&pk, &msg, &bad),
                    "{p:?} seed {seed}: flipping byte {pos} must be rejected"
                );
            }
            // Message-bound: this signature must not verify against a different message.
            let other = hash_of(0x2222_0002 ^ seed ^ tag(p), p.m());
            if other != msg {
                assert!(
                    !verify(&pk, &other, &sig),
                    "{p:?} seed {seed}: signature must be message-bound"
                );
            }
        }
    }
}

#[test]
fn flatten_keygen_recovers_the_forward_affine_maps_at_every_parameter_set() {
    for &p in PARAM_SETS {
        for seed in 0..6u64 {
            let (sk, _pk) = keygen(p, 0x00AB_0000 ^ seed ^ tag(p));
            let ki = flatten_keygen(&sk);
            let sf = flatten_secret(&sk);
            let (n, m) = (p.n(), p.m());
            assert_eq!(
                gf256::matmul(&reshape(&ki.t_mat, n), &reshape(&sf.t_inv, n)),
                identity(n),
                "{p:?}: T·T⁻¹ = I"
            );
            assert_eq!(
                gf256::matmul(&reshape(&ki.s_mat, m), &reshape(&sf.s_inv, m)),
                identity(m),
                "{p:?}: S·S⁻¹ = I"
            );
        }
    }
}

#[test]
fn the_public_map_equals_the_secret_composition_at_every_parameter_set() {
    // Independent check that P = S∘F∘T: for random x, the public map must agree with pushing x
    // through T, the central map, then S — computed here the long way.
    for &p in PARAM_SETS {
        let (sk, pk) = keygen(p, 0x00C0_FFEE ^ tag(p));
        let (n, m) = (p.n(), p.m());
        let s_mat = gf256::invert(&sk.s_inv).unwrap();
        let t_mat = gf256::invert(&sk.t_inv).unwrap();
        let mut r = Rng(0x1234 ^ tag(p));
        for _ in 0..40 {
            let x: Vec<Gf256> = (0..n).map(|_| r.gf()).collect();
            let xb: Vec<u8> = x.iter().map(|g| g.0).collect();
            let tx = gf256::matvec(&t_mat, &xb);
            let w: Vec<Gf256> = (0..n).map(|i| Gf256(tx[i] ^ sk.t_vec[i])).collect();
            let fw: Vec<u8> = sk.central.iter().map(|f| eval(f, &w).0).collect();
            let sfw = gf256::matvec(&s_mat, &fw);
            let expect: Vec<u8> = (0..m).map(|k| sfw[k] ^ sk.s_vec[k]).collect();
            let got: Vec<u8> = (0..m).map(|k| eval(&pk.polys[k], &x).0).collect();
            assert_eq!(got, expect, "{p:?}: public map must equal S∘F∘T");
        }
    }
}

#[test]
fn the_oil_vinegar_trapdoor_hits_the_central_target_at_every_parameter_set() {
    // The trapdoor's defining property: after signing, the central map at w = T·sig + t_vec
    // must land EXACTLY on S⁻¹(msg − s_vec) — the value the outer affine map then maps to msg.
    for &p in PARAM_SETS {
        for seed in 0..6u64 {
            let (sk, _pk) = keygen(p, 0x0B0B_0000 ^ seed ^ tag(p));
            let msg = hash_of(0x0033_0000 ^ seed ^ tag(p), p.m());
            let sig = sign(&sk, &msg).expect("sign");
            let t_mat = gf256::invert(&sk.t_inv).unwrap();
            let tx = gf256::matvec(&t_mat, &sig);
            let w: Vec<Gf256> = (0..p.n()).map(|i| Gf256(tx[i] ^ sk.t_vec[i])).collect();
            let fw: Vec<u8> = sk.central.iter().map(|f| eval(f, &w).0).collect();
            let dms: Vec<u8> = (0..p.m()).map(|k| msg[k] ^ sk.s_vec[k]).collect();
            let u = gf256::matvec(&sk.s_inv, &dms);
            assert_eq!(
                fw, u,
                "{p:?} seed {seed}: F(T·sig + t) must equal S⁻¹(msg − s_vec)"
            );
        }
    }
}

#[test]
fn forge_system_is_the_multivariate_system_a_signature_solves_at_every_parameter_set() {
    // The bridge to algebraic cryptanalysis: a valid signature is exactly a common zero of
    // `forge_system`. Solving that system from scratch is the direct algebraic attack Ding
    // formalized via the degree of regularity — our XL engine (xl.rs) solves the over-determined
    // MQ instances; Rainbow's specific structure fell to Beullens' 2022 structural attack.
    for &p in PARAM_SETS {
        for seed in 0..6u64 {
            let (sk, pk) = keygen(p, 0x5EED_0000 ^ seed ^ tag(p));
            let msg = hash_of(0x1111_0001 ^ seed ^ tag(p), p.m());
            let sig = sign(&sk, &msg).expect("signable");
            let sys = forge_system(&pk, &msg);
            let x: Vec<Gf256> = sig.iter().map(|&b| Gf256(b)).collect();
            assert!(
                sys.satisfied_by(&x),
                "{p:?} seed {seed}: a real signature must be a zero of forge_system"
            );
            let mut bad = x.clone();
            bad[0] = Gf256(bad[0].0 ^ 1);
            assert!(
                !sys.satisfied_by(&bad),
                "{p:?} seed {seed}: a non-signature must not be a zero"
            );
        }
    }
}
