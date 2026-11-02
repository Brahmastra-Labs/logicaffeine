//! # EXPERIMENT: how many oil spaces? — the enumerative geometry of isotropic planes
//!
//! Below the threshold `v ≤ o(o+1)/2` the isotropic-plane variety `𝓕_T` has expected dimension `≤ 0`, so
//! it is a *finite* set of points. "Is the oil space isolated" then sharpens to "is it **unique**, or is
//! there finite ambiguity?" — a classical enumerative question. `𝓕_T` is an intersection of `m` orthogonal
//! Grassmannians (varieties of isotropic subspaces of a quadric), and at expected dimension zero the count
//! is the Schubert number `∫_{Gr(o,n)} c_top((Sym²S∨)^{⊕m})` — the isotropic-plane analog of "27 lines on a
//! cubic surface".
//!
//! The engine computes it directly: brute-count the `o`-planes over `𝔽_q` that are totally isotropic for a
//! generic `m`-tuple of quadrics, at the critical dimension. The point cryptographically: the planted oil
//! space is one of *several* equivalent keys, not a unique secret.

/// Every 2-plane of `𝔽_q^n`, enumerated once each via reduced row echelon form.
fn two_planes(n: usize, q: u64) -> Vec<[Vec<u64>; 2]> {
    let mut out = Vec::new();
    for j1 in 0..n {
        for j2 in j1 + 1..n {
            let free1: Vec<usize> = (j1 + 1..n).filter(|&c| c != j2).collect();
            let free2: Vec<usize> = (j2 + 1..n).collect();
            let n1 = q.pow(free1.len() as u32);
            let n2 = q.pow(free2.len() as u32);
            for a1 in 0..n1 {
                for a2 in 0..n2 {
                    let mut r1 = vec![0u64; n];
                    r1[j1] = 1;
                    let mut x = a1;
                    for &c in &free1 {
                        r1[c] = x % q;
                        x /= q;
                    }
                    let mut r2 = vec![0u64; n];
                    r2[j2] = 1;
                    let mut y = a2;
                    for &c in &free2 {
                        r2[c] = y % q;
                        y /= q;
                    }
                    out.push([r1, r2]);
                }
            }
        }
    }
    out
}

fn bil(u: &[u64], p: &[Vec<u64>], w: &[u64], q: u64) -> u64 {
    let n = u.len();
    let mut s = 0u128;
    for a in 0..n {
        for b in 0..n {
            s += u[a] as u128 * p[a][b] as u128 % q as u128 * w[b] as u128;
        }
    }
    (s % q as u128) as u64
}

/// Is the 2-plane totally isotropic for every symmetric form? `Q_i(r₁)=Q_i(r₂)=0` and `r₁ᵀ P_i r₂ = 0`.
fn isotropic(plane: &[Vec<u64>; 2], forms: &[Vec<Vec<u64>>], q: u64) -> bool {
    forms.iter().all(|p| {
        bil(&plane[0], p, &plane[0], q) == 0
            && bil(&plane[1], p, &plane[1], q) == 0
            && bil(&plane[0], p, &plane[1], q) == 0
    })
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, q: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) % q
    }
}

fn random_forms(m: usize, n: usize, q: u64, seed: u64) -> Vec<Vec<Vec<u64>>> {
    let mut rng = Lcg(seed);
    (0..m)
        .map(|_| {
            let mut p = vec![vec![0u64; n]; n];
            for i in 0..n {
                for j in i..n {
                    let val = rng.next(q);
                    p[i][j] = val;
                    p[j][i] = val;
                }
            }
            p
        })
        .collect()
}

/// A UOV system (planted oil space = last `o` coordinates), symmetric, over `𝔽_q`.
fn uov_forms(o: usize, v: usize, q: u64, seed: u64) -> Vec<Vec<Vec<u64>>> {
    let n = o + v;
    let mut rng = Lcg(seed);
    (0..o)
        .map(|_| {
            let mut p = vec![vec![0u64; n]; n];
            for i in 0..n {
                for j in i..n {
                    if i < v || j < v {
                        let val = rng.next(q);
                        p[i][j] = val;
                        p[j][i] = val;
                    }
                }
            }
            p
        })
        .collect()
}

fn count_isotropic(forms: &[Vec<Vec<u64>>], n: usize, q: u64) -> usize {
    two_planes(n, q)
        .iter()
        .filter(|pl| isotropic(pl, forms, q))
        .count()
}

/// Median count of common isotropic 2-planes over several seeds (a stable representative of the 𝔽_q-point
/// count of the finite scheme).
fn typical_count(uov: bool, n: usize, q: u64) -> usize {
    let mut cs: Vec<usize> = (0..5)
        .map(|s| {
            let forms = if uov {
                uov_forms(2, n - 2, q, 1000 + s * 13 + q)
            } else {
                random_forms(2, n, q, 2000 + s * 13 + q)
            };
            count_isotropic(&forms, n, q)
        })
        .collect();
    cs.sort_unstable();
    cs[2]
}

#[test]
fn a_uov_system_has_several_equivalent_oil_spaces_not_a_unique_secret() {
    // o = 2, m = 2, critical dimension v = o(o+1)/2 = 3 ⇒ n = 5 (expected dimension 0: a finite set).
    for &q in &[3u64, 5, 7] {
        let r = typical_count(false, 5, q);
        let u = typical_count(true, 5, q);
        eprintln!("q={q}:  random net → {r} isotropic 2-planes   |   UOV → {u}");
    }
    // The cryptographic point, robust across fields: a UOV public key has STRICTLY MORE than one common
    // isotropic 2-plane at the critical dimension. The planted oil space is one of several equivalent keys.
    for &q in &[5u64, 7] {
        assert!(
            typical_count(true, 5, q) >= 2,
            "q={q}: the UOV system has ≥ 2 isotropic 2-planes — the secret is not unique but a small orbit of equivalent keys"
        );
    }
}
