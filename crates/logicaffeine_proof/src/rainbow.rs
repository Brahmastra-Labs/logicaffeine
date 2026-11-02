//! Rainbow — Jintai Ding & Dieter Schmidt's multilayer Oil-and-Vinegar signature (ACNS 2005),
//! the multivariate scheme our XL engine is built to attack. This is a faithful reference over
//! GF(2^8): a two-layer central map `F`, invertible affine transforms `S` and `T`, public key
//! `P = S ∘ F ∘ T`, and the Oil-and-Vinegar trapdoor that inverts `F` at signing time.
//!
//! The public map is produced as `xl::Poly<Gf256>` per output, so [`forge_system`] hands it
//! straight to [`crate::xl::xl_solve`] — the algebraic cryptanalysis Ding formalized through the
//! degree of regularity — closing the build-and-break loop on one substrate. All finite-field
//! linear algebra reuses the exhaustively-verified [`logicaffeine_base::gf256`].
//!
//! Field note: this reference works over the `logicaffeine_base::gf256` field (reduction
//! polynomial `0x11d`). The NIST submission fixes its own GF(256) tower representation; matching
//! that byte-for-byte is the separate bit-exact-vs-KAT effort.

use crate::macaulay::{self, Monomial};
use crate::xl::{eval, Field, Gf256, MQSystem, Poly};
use logicaffeine_base::gf256;
use std::collections::BTreeMap;

/// Rainbow layer sizes: `v1` vinegars, then `o1` first-layer oils, then `o2` second-layer oils.
#[derive(Clone, Copy, Debug)]
pub struct RainbowParams {
    pub v1: usize,
    pub o1: usize,
    pub o2: usize,
}

impl RainbowParams {
    pub fn n(&self) -> usize {
        self.v1 + self.o1 + self.o2
    }
    pub fn m(&self) -> usize {
        self.o1 + self.o2
    }
    /// The index one past the first-layer variables (start of the second-layer oils).
    fn l1(&self) -> usize {
        self.v1 + self.o1
    }
}

pub struct SecretKey {
    pub params: RainbowParams,
    s_inv: Vec<Vec<u8>>,
    s_vec: Vec<u8>,
    t_inv: Vec<Vec<u8>>,
    t_vec: Vec<u8>,
    central: Vec<Poly<Gf256>>,
}

pub struct PublicKey {
    pub params: RainbowParams,
    /// One quadratic polynomial per output coordinate: `P_k(x)`.
    pub polys: Vec<Poly<Gf256>>,
}

// ── Polynomial plumbing over GF(256) ──────────────────────────────────────────────────

fn padd(dst: &mut Poly<Gf256>, m: &Monomial, c: Gf256) {
    if c.is_zero() {
        return;
    }
    let v = dst.get(m).copied().unwrap_or_else(Gf256::zero).add(c);
    if v.is_zero() {
        dst.remove(m);
    } else {
        dst.insert(m.clone(), v);
    }
}

fn poly_add_into(dst: &mut Poly<Gf256>, src: &Poly<Gf256>) {
    for (m, &c) in src {
        padd(dst, m, c);
    }
}

fn poly_mul(a: &Poly<Gf256>, b: &Poly<Gf256>) -> Poly<Gf256> {
    let mut r = BTreeMap::new();
    for (ma, &ca) in a {
        for (mb, &cb) in b {
            padd(&mut r, &macaulay::mul(ma, mb, false), ca.mul(cb));
        }
    }
    r
}

/// Substitute an affine form for every variable: `x_i ↦ subs[i]` (each a polynomial). Used to
/// compose the central map with the affine `T`.
fn substitute_affine(f: &Poly<Gf256>, subs: &[Poly<Gf256>]) -> Poly<Gf256> {
    let mut out = BTreeMap::new();
    for (mono, &c) in f {
        let mut term: Poly<Gf256> = BTreeMap::new();
        term.insert(Vec::new(), c); // the constant polynomial c
        for &v in mono {
            term = poly_mul(&term, &subs[v as usize]);
        }
        poly_add_into(&mut out, &term);
    }
    out
}

/// Substitute concrete field values for a subset of variables (`fixed[i] = Some(val)`), leaving
/// the rest symbolic. Used to freeze the vinegars before solving the oils.
fn substitute_values(f: &Poly<Gf256>, fixed: &[Option<Gf256>]) -> Poly<Gf256> {
    let mut out = BTreeMap::new();
    for (mono, &c) in f {
        let mut coeff = c;
        let mut rem: Monomial = Vec::new();
        for &v in mono {
            match fixed[v as usize] {
                Some(val) => coeff = coeff.mul(val),
                None => rem.push(v),
            }
        }
        rem.sort_unstable();
        padd(&mut out, &rem, coeff);
    }
    out
}

// ── Deterministic randomness ──────────────────────────────────────────────────────────

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    fn gf(&mut self) -> Gf256 {
        Gf256(self.byte())
    }
}

fn random_invertible(rng: &mut Rng, size: usize) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    loop {
        let mat: Vec<Vec<u8>> = (0..size)
            .map(|_| (0..size).map(|_| rng.byte()).collect())
            .collect();
        if let Some(inv) = gf256::invert(&mat) {
            return (mat, inv);
        }
    }
}

/// Generate the two-layer Oil-and-Vinegar central map. Each polynomial omits oil×oil products in
/// its layer, so fixing the vinegars leaves it LINEAR in that layer's oils — the trapdoor.
fn gen_central(p: &RainbowParams, rng: &mut Rng) -> Vec<Poly<Gf256>> {
    let (n, v1, l1) = (p.n(), p.v1, p.l1());
    let mut central = Vec::with_capacity(p.m());
    // Layer 1: variables [0, l1); quadratic terms need the smaller index to be a vinegar (< v1).
    for _ in 0..p.o1 {
        let mut f = BTreeMap::new();
        for i in 0..l1 {
            if i < v1 {
                for j in i..l1 {
                    padd(&mut f, &vec![i as u32, j as u32], rng.gf());
                }
            }
        }
        for i in 0..l1 {
            padd(&mut f, &vec![i as u32], rng.gf());
        }
        padd(&mut f, &Vec::new(), rng.gf());
        central.push(f);
    }
    // Layer 2: variables [0, n); quadratic terms need the smaller index to be in V2 (< l1).
    for _ in 0..p.o2 {
        let mut f = BTreeMap::new();
        for i in 0..n {
            if i < l1 {
                for j in i..n {
                    padd(&mut f, &vec![i as u32, j as u32], rng.gf());
                }
            }
        }
        for i in 0..n {
            padd(&mut f, &vec![i as u32], rng.gf());
        }
        padd(&mut f, &Vec::new(), rng.gf());
        central.push(f);
    }
    central
}

// ── Key generation, signing, verification ─────────────────────────────────────────────

pub fn keygen(params: RainbowParams, seed: u64) -> (SecretKey, PublicKey) {
    let (n, m) = (params.n(), params.m());
    let mut rng = Rng(seed);

    let (t_mat, t_inv) = random_invertible(&mut rng, n);
    let t_vec: Vec<u8> = (0..n).map(|_| rng.byte()).collect();
    let (s_mat, s_inv) = random_invertible(&mut rng, m);
    let s_vec: Vec<u8> = (0..m).map(|_| rng.byte()).collect();
    let central = gen_central(&params, &mut rng);

    // Compose the public map P = S ∘ F ∘ T. First the affine forms w_i = Σ_a T[i][a] x_a + t_i.
    let t_forms: Vec<Poly<Gf256>> = (0..n)
        .map(|i| {
            let mut p: Poly<Gf256> = BTreeMap::new();
            padd(&mut p, &Vec::new(), Gf256(t_vec[i]));
            for a in 0..n {
                padd(&mut p, &vec![a as u32], Gf256(t_mat[i][a]));
            }
            p
        })
        .collect();
    let f_comp: Vec<Poly<Gf256>> = central
        .iter()
        .map(|f| substitute_affine(f, &t_forms))
        .collect();

    let mut polys = Vec::with_capacity(m);
    for k in 0..m {
        let mut pk: Poly<Gf256> = BTreeMap::new();
        for l in 0..m {
            let c = Gf256(s_mat[k][l]);
            if !c.is_zero() {
                for (mono, &fc) in &f_comp[l] {
                    padd(&mut pk, mono, c.mul(fc));
                }
            }
        }
        padd(&mut pk, &Vec::new(), Gf256(s_vec[k]));
        polys.push(pk);
    }

    (
        SecretKey {
            params,
            s_inv,
            s_vec,
            t_inv,
            t_vec,
            central,
        },
        PublicKey { params, polys },
    )
}

fn hash_seed(msg_hash: &[u8]) -> u64 {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for &b in msg_hash {
        seed = seed.rotate_left(7) ^ (b as u64).wrapping_mul(0x1_0000_0001_B3);
    }
    seed
}

/// Sign a message hash (`m` field elements). Returns the signature `x ∈ GF(256)^n`, or `None` if
/// the Oil-and-Vinegar inversion kept hitting singular systems (astronomically unlikely).
pub fn sign(sk: &SecretKey, msg_hash: &[u8]) -> Option<Vec<u8>> {
    let u = central_target(sk, msg_hash);
    for attempt in 0..256u64 {
        let mut rng = Rng(hash_seed(msg_hash) ^ attempt.wrapping_mul(0xA5A5_A5A5));
        let vinegars: Vec<u8> = (0..sk.params.v1).map(|_| rng.byte()).collect();
        if let Some(x) = sign_core(sk, &u, &vinegars) {
            return Some(x);
        }
    }
    None
}

/// Sign with the `v1` vinegar values supplied explicitly (deterministic; `None` if they make a
/// layer's linear system singular). This is the exact core the LOGOS `rainbowSign` mirrors.
pub fn sign_with_vinegars(sk: &SecretKey, msg_hash: &[u8], vinegars: &[u8]) -> Option<Vec<u8>> {
    sign_core(sk, &central_target(sk, msg_hash), vinegars)
}

/// `u = S⁻¹ · (d − S_vec)` — the value the central map must hit.
fn central_target(sk: &SecretKey, msg_hash: &[u8]) -> Vec<u8> {
    let d_minus_s: Vec<u8> = (0..sk.params.m())
        .map(|k| msg_hash[k] ^ sk.s_vec[k])
        .collect();
    gf256::matvec(&sk.s_inv, &d_minus_s)
}

/// The deterministic Oil-and-Vinegar inversion: fix the vinegars, solve each layer's oils.
fn sign_core(sk: &SecretKey, u: &[u8], vinegars: &[u8]) -> Option<Vec<u8>> {
    let p = sk.params;
    let (n, v1, o1, o2, l1) = (p.n(), p.v1, p.o1, p.o2, p.l1());
    let mut fixed: Vec<Option<Gf256>> = vec![None; n];
    for i in 0..v1 {
        fixed[i] = Some(Gf256(vinegars[i]));
    }

    // Layer 1: solve for the first-layer oils [v1, l1).
    let mut a1 = vec![vec![0u8; o1]; o1];
    let mut b1 = vec![0u8; o1];
    for k in 0..o1 {
        let red = substitute_values(&sk.central[k], &fixed);
        b1[k] = u[k] ^ red.get(&Vec::new()).copied().unwrap_or_else(Gf256::zero).0;
        for c in 0..o1 {
            a1[k][c] = red
                .get(&vec![(v1 + c) as u32])
                .copied()
                .unwrap_or_else(Gf256::zero)
                .0;
        }
    }
    let sol1 = gf256::solve(&a1, &b1)?;
    for c in 0..o1 {
        fixed[v1 + c] = Some(Gf256(sol1[c]));
    }

    // Layer 2: solve for the second-layer oils [l1, n).
    let mut a2 = vec![vec![0u8; o2]; o2];
    let mut b2 = vec![0u8; o2];
    for k in 0..o2 {
        let red = substitute_values(&sk.central[o1 + k], &fixed);
        b2[k] = u[o1 + k] ^ red.get(&Vec::new()).copied().unwrap_or_else(Gf256::zero).0;
        for c in 0..o2 {
            a2[k][c] = red
                .get(&vec![(l1 + c) as u32])
                .copied()
                .unwrap_or_else(Gf256::zero)
                .0;
        }
    }
    let sol2 = gf256::solve(&a2, &b2)?;
    for c in 0..o2 {
        fixed[l1 + c] = Some(Gf256(sol2[c]));
    }

    // x = T⁻¹ · (w − T_vec).
    let w: Vec<u8> = (0..n).map(|i| fixed[i].unwrap().0).collect();
    let w_minus_t: Vec<u8> = (0..n).map(|i| w[i] ^ sk.t_vec[i]).collect();
    Some(gf256::matvec(&sk.t_inv, &w_minus_t))
}

/// Verify a signature: evaluate every public polynomial at `x` and compare to the message hash.
pub fn verify(pk: &PublicKey, msg_hash: &[u8], sig: &[u8]) -> bool {
    if sig.len() != pk.params.n() {
        return false;
    }
    let x: Vec<Gf256> = sig.iter().map(|&b| Gf256(b)).collect();
    (0..pk.params.m()).all(|k| eval(&pk.polys[k], &x).0 == msg_hash[k])
}

/// The MQ system a forger must solve — `P_k(x) − d_k = 0` for every output — handed straight to
/// [`crate::xl::xl_solve`]. A solution is a signature forgery produced with NO secret key.
pub fn forge_system(pk: &PublicKey, msg_hash: &[u8]) -> MQSystem<Gf256> {
    let polys = pk
        .polys
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let mut q = p.clone();
            padd(&mut q, &Vec::new(), Gf256(msg_hash[k])); // − d_k  (= xor in char 2)
            q
        })
        .collect();
    MQSystem {
        n: pk.params.n(),
        polys,
    }
}

/// Flatten the public key into three byte arrays for a coefficient-driven verifier: `pubq` holds
/// the upper-triangular quadratic coefficients per output (order `i ≤ j`), `publin` the linear
/// coefficients, `pubc` the constants. This is the exact layout the LOGOS `rainbowVerify` consumes.
pub fn flatten_public(pk: &PublicKey) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (n, m) = (pk.params.n(), pk.params.m());
    let mut pubq = Vec::new();
    let mut publin = Vec::new();
    let mut pubc = Vec::new();
    for k in 0..m {
        for i in 0..n {
            for j in i..n {
                pubq.push(
                    pk.polys[k]
                        .get(&vec![i as u32, j as u32])
                        .copied()
                        .unwrap_or_else(Gf256::zero)
                        .0,
                );
            }
        }
        for i in 0..n {
            publin.push(
                pk.polys[k]
                    .get(&vec![i as u32])
                    .copied()
                    .unwrap_or_else(Gf256::zero)
                    .0,
            );
        }
        pubc.push(
            pk.polys[k]
                .get(&Vec::new())
                .copied()
                .unwrap_or_else(Gf256::zero)
                .0,
        );
    }
    (pubq, publin, pubc)
}

/// The secret key flattened for the LOGOS `rainbowSign`: the central map in the same per-output
/// layout as [`flatten_public`], plus the affine inverses `S⁻¹` / `T⁻¹` (row-major) and their shifts.
#[derive(Clone, Debug)]
pub struct SecretFlat {
    pub central_q: Vec<u8>,
    pub central_lin: Vec<u8>,
    pub central_c: Vec<u8>,
    pub s_inv: Vec<u8>,
    pub s_vec: Vec<u8>,
    pub t_inv: Vec<u8>,
    pub t_vec: Vec<u8>,
}

pub fn flatten_secret(sk: &SecretKey) -> SecretFlat {
    let (n, m) = (sk.params.n(), sk.params.m());
    let mut central_q = Vec::new();
    let mut central_lin = Vec::new();
    let mut central_c = Vec::new();
    for k in 0..m {
        for i in 0..n {
            for j in i..n {
                central_q.push(
                    sk.central[k]
                        .get(&vec![i as u32, j as u32])
                        .copied()
                        .unwrap_or_else(Gf256::zero)
                        .0,
                );
            }
        }
        for i in 0..n {
            central_lin.push(
                sk.central[k]
                    .get(&vec![i as u32])
                    .copied()
                    .unwrap_or_else(Gf256::zero)
                    .0,
            );
        }
        central_c.push(
            sk.central[k]
                .get(&Vec::new())
                .copied()
                .unwrap_or_else(Gf256::zero)
                .0,
        );
    }
    SecretFlat {
        central_q,
        central_lin,
        central_c,
        s_inv: sk.s_inv.iter().flatten().copied().collect(),
        s_vec: sk.s_vec.clone(),
        t_inv: sk.t_inv.iter().flatten().copied().collect(),
        t_vec: sk.t_vec.clone(),
    }
}

/// Inputs for the LOGOS `rainbowKeygen` (the public-map composition `P = S∘F∘T`): the FORWARD
/// affine maps `T`/`S` (recovered by inverting the stored inverses) and the central map, all
/// row-major / packed like [`flatten_public`].
#[derive(Clone, Debug)]
pub struct KeygenInputs {
    pub t_mat: Vec<u8>,
    pub t_vec: Vec<u8>,
    pub s_mat: Vec<u8>,
    pub s_vec: Vec<u8>,
    pub central_q: Vec<u8>,
    pub central_lin: Vec<u8>,
    pub central_c: Vec<u8>,
}

fn reshape(flat: &[u8], k: usize) -> Vec<Vec<u8>> {
    (0..k).map(|r| flat[r * k..(r + 1) * k].to_vec()).collect()
}

pub fn flatten_keygen(sk: &SecretKey) -> KeygenInputs {
    let sf = flatten_secret(sk);
    let (n, m) = (sk.params.n(), sk.params.m());
    let t_mat = gf256::invert(&reshape(&sf.t_inv, n)).expect("T⁻¹ is invertible");
    let s_mat = gf256::invert(&reshape(&sf.s_inv, m)).expect("S⁻¹ is invertible");
    KeygenInputs {
        t_mat: t_mat.into_iter().flatten().collect(),
        t_vec: sf.t_vec,
        s_mat: s_mat.into_iter().flatten().collect(),
        s_vec: sf.s_vec,
        central_q: sf.central_q,
        central_lin: sf.central_lin,
        central_c: sf.central_c,
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/rainbow/tests.rs"]
mod tests;
