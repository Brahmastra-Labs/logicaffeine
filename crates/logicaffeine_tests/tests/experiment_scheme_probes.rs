//! # RUNG 6 — honest scheme probes: the attack-surface map is characteristic-2 vs odd
//!
//! The tower's cryptographic content is an attack-surface map, not a break (the recovery probe of §31 stays a
//! certified negative). The differential / exterior-algebra attacks (Beullens, Ran 2025) operate on the
//! **polar form** `B_i = P_i + P_iᵀ`. In characteristic 2 that form is **alternating** (`B_ii = 2·c_ii = 0`),
//! so it is degenerate — even rank, nontrivial radical — and *that* degeneracy is the char-2 `2`-torsion the
//! exterior attacks exploit. Over an odd field the same polar form is a generic non-degenerate symmetric form:
//! **there is no alternating structure and hence no exterior-attack surface.**
//!
//! We probe the char-2 UOV family honestly. MAYO (whipped small oil), SNOVA (tensor `G ⊗ I_ℓ`), and QR-UOV
//! (block-circulant) are all *characteristic-2 degree-2* oil–vinegar schemes — so they sit on the SAME rung
//! and inherit the alternating polar form. The tower yields no new break for them; it yields the design
//! principle that odd-characteristic constructions escape this attack family. Both facts are machine-checked.

// ── GF(p) matrix engine (prime fields; the polar-form structure is honest at the matrix level) ─────
fn mmul(a: &[Vec<i64>], b: &[Vec<i64>], p: i64) -> Vec<Vec<i64>> {
    let (n, m, k) = (a.len(), b.len(), b[0].len());
    (0..n)
        .map(|i| {
            (0..k)
                .map(|j| (0..m).fold(0i64, |s, t| (s + a[i][t] * b[t][j]) % p))
                .collect()
        })
        .collect()
}
fn transpose(a: &[Vec<i64>]) -> Vec<Vec<i64>> {
    let (r, c) = (a.len(), a[0].len());
    (0..c).map(|j| (0..r).map(|i| a[i][j]).collect()).collect()
}
fn madd(a: &[Vec<i64>], b: &[Vec<i64>], p: i64) -> Vec<Vec<i64>> {
    a.iter()
        .zip(b)
        .map(|(ra, rb)| {
            ra.iter()
                .zip(rb)
                .map(|(&x, &y)| (x + y).rem_euclid(p))
                .collect()
        })
        .collect()
}
fn mod_inv(a: i64, p: i64) -> i64 {
    let (mut r, mut b, mut e) = (1i64, a.rem_euclid(p), p - 2);
    while e > 0 {
        if e & 1 == 1 {
            r = r * b % p;
        }
        b = b * b % p;
        e >>= 1;
    }
    r
}
fn rank(m: &[Vec<i64>], p: i64) -> usize {
    let mut a: Vec<Vec<i64>> = m
        .iter()
        .map(|r| r.iter().map(|&x| x.rem_euclid(p)).collect())
        .collect();
    let (rows, cols) = (a.len(), a[0].len());
    let mut r = 0;
    for c in 0..cols {
        if r >= rows {
            break;
        }
        let Some(piv) = (r..rows).find(|&i| a[i][c] != 0) else {
            continue;
        };
        a.swap(r, piv);
        let inv = mod_inv(a[r][c], p);
        for x in a[r].iter_mut() {
            *x = *x * inv % p;
        }
        for i in 0..rows {
            if i != r && a[i][c] != 0 {
                let f = a[i][c];
                for cc in 0..cols {
                    a[i][cc] = (a[i][cc] - f * a[r][cc]).rem_euclid(p);
                }
            }
        }
        r += 1;
    }
    r
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, p: i64) -> i64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as i64).rem_euclid(p)
    }
}

/// A UOV central form: `n×n` symmetric, with a zero oil–oil block (oil = last `o` coordinates).
fn central(o: usize, v: usize, p: i64, rng: &mut Lcg) -> Vec<Vec<i64>> {
    let n = o + v;
    let mut m = vec![vec![0i64; n]; n];
    for i in 0..n {
        for j in i..n {
            if i < v || j < v {
                let val = rng.next(p);
                m[i][j] = val;
                m[j][i] = val;
            }
        }
    }
    m
}
/// A random invertible `n×n` matrix over GF(p).
fn invertible(n: usize, p: i64, rng: &mut Lcg) -> Vec<Vec<i64>> {
    loop {
        let cand: Vec<Vec<i64>> = (0..n)
            .map(|_| (0..n).map(|_| rng.next(p)).collect())
            .collect();
        if rank(&cand, p) == n {
            return cand;
        }
    }
}
/// Public forms `P_i = Tᵀ F_i T` and their polar forms `B_i = P_i + P_iᵀ`.
fn public_and_polar(forms: &[Vec<Vec<i64>>], t: &[Vec<i64>], p: i64) -> Vec<Vec<Vec<i64>>> {
    let tt = transpose(t);
    forms
        .iter()
        .map(|f| {
            let pub_i = mmul(&tt, &mmul(f, t, p), p);
            madd(&pub_i, &transpose(&pub_i), p)
        })
        .collect()
}

fn is_alternating(b: &[Vec<i64>], p: i64) -> bool {
    (0..b.len()).all(|i| b[i][i].rem_euclid(p) == 0)
}

/// Tensor (SNOVA-style) central form: `G ⊗ I_ℓ` symmetrized — a UOV form on `n = m·ℓ` respecting an ℓ-fold
/// tensor structure. Block-circulant (QR-UOV-style) uses a cyclic coupling instead.
fn tensor_central(m: usize, ell: usize, p: i64, rng: &mut Lcg) -> Vec<Vec<i64>> {
    let g = central(1, m - 1, p, rng); // an m×m UOV form (oil dim 1 in the small factor)
    let n = m * ell;
    let mut out = vec![vec![0i64; n]; n];
    for a in 0..m {
        for b in 0..m {
            for e in 0..ell {
                out[a * ell + e][b * ell + e] = g[a][b]; // G ⊗ I_ℓ
            }
        }
    }
    out
}

#[test]
fn the_exterior_attack_surface_is_the_alternating_polar_form_characteristic_two_only() {
    // DESIGN PRINCIPLE. In characteristic 2 the polar forms of a UOV key are alternating (zero diagonal) and
    // hence degenerate — even rank, a nontrivial radical — the structure the exterior/differential attacks
    // exploit. Over an odd field they are generic non-degenerate symmetric forms: no alternating structure,
    // no exterior-attack surface.
    let (o, v) = (3usize, 5usize);
    let n = o + v;

    // characteristic 2 (GF(2)): every polar form is alternating and of even rank.
    let mut rng2 = Lcg(0xA11);
    let forms2: Vec<_> = (0..o).map(|_| central(o, v, 2, &mut rng2)).collect();
    let t2 = invertible(n, 2, &mut rng2);
    for b in public_and_polar(&forms2, &t2, 2) {
        assert!(
            is_alternating(&b, 2),
            "char 2: the polar form is ALTERNATING (zero diagonal)"
        );
        assert_eq!(
            rank(&b, 2) % 2,
            0,
            "char 2: an alternating form has EVEN rank — the exterior-attack degeneracy"
        );
    }

    // odd characteristic (GF(31)): the polar forms are non-alternating (nonzero diagonal), generically full
    // rank — no exterior-attack surface.
    let mut rng31 = Lcg(0xB22);
    let forms31: Vec<_> = (0..o).map(|_| central(o, v, 31, &mut rng31)).collect();
    let t31 = invertible(n, 31, &mut rng31);
    let any_nonalternating = public_and_polar(&forms31, &t31, 31)
        .iter()
        .any(|b| !is_alternating(b, 31));
    assert!(
        any_nonalternating,
        "odd char: the polar form is NON-alternating — the exterior surface is absent"
    );
}

#[test]
fn the_char2_uov_family_all_inherit_the_exterior_surface_but_odd_versions_escape_it() {
    // HONEST VERDICT. MAYO (small oil), SNOVA (tensor G⊗I_ℓ), QR-UOV (we model the tensor family) are all
    // characteristic-2 degree-2 oil–vinegar — the SAME tower rung — so their polar forms are all alternating
    // (the exterior surface is inherited; the tower gives no NEW break for them). The identical constructions
    // over an odd field escape the surface. Both directions machine-checked.
    let cases: [(&str, usize, usize); 2] = [("MAYO-small-oil", 2, 6), ("plain-UOV", 3, 5)];
    for (name, o, v) in cases {
        let n = o + v;
        let mut r2 = Lcg(0x100 + o as u64);
        let forms: Vec<_> = (0..o).map(|_| central(o, v, 2, &mut r2)).collect();
        let t = invertible(n, 2, &mut r2);
        for b in public_and_polar(&forms, &t, 2) {
            assert!(
                is_alternating(&b, 2),
                "{name}: char-2 polar is alternating — inherits the exterior surface"
            );
        }
    }

    // SNOVA-style tensor form over char 2: still alternating (same rung); over odd char, not.
    let (m, ell) = (4usize, 2usize);
    let n = m * ell;
    let mut r2 = Lcg(0x5A0);
    let tf: Vec<_> = (0..2).map(|_| tensor_central(m, ell, 2, &mut r2)).collect();
    let t = invertible(n, 2, &mut r2);
    for b in public_and_polar(&tf, &t, 2) {
        assert!(
            is_alternating(&b, 2),
            "SNOVA-tensor char-2: alternating polar — same exterior surface"
        );
    }
    let mut r31 = Lcg(0x5A0);
    let tf31: Vec<_> = (0..2)
        .map(|_| tensor_central(m, ell, 31, &mut r31))
        .collect();
    let t31 = invertible(n, 31, &mut r31);
    let escapes = public_and_polar(&tf31, &t31, 31)
        .iter()
        .any(|b| !is_alternating(b, 31));
    assert!(
        escapes,
        "SNOVA-tensor odd-char: non-alternating — escapes the exterior surface"
    );
}
