//! # THE FROBENIUS-REFINED RECOVERY — pushing past the single-pencil stall over 𝔽₁₆
//!
//! The kill test (`experiment_uov_unbalanced_killtest`) measured, over an odd field, that the single-pencil
//! Kipnis–Shamir recovery `N = P₀⁻¹P₁` recovers the oil space `O` only when balanced (`v = o`) and stalls
//! at `v > o` because `O` stops being `N`-invariant. That attack uses ONE polar form — and in
//! characteristic 2 the polar form is *alternating*, discarding the diagonal/Dickson (Frobenius) layer
//! entirely (§27–§30). The natural question the cascade forces: does the **full quadratic information**,
//! which carries the Frobenius layer the single pencil throws away, pin `O` further out?
//!
//! This file measures it HONESTLY over 𝔽₁₆ (a real UOV field, char 2). It builds genuine UOV keys, then
//! compares two recoveries by the largest subspace each confines an oil vector to (the invariant hull —
//! smaller is better, `= o` means `O` is pinned and recoverable):
//!   * **single pencil** `N = P₀⁻¹P₁` (one polar form — the Frobenius-blind baseline);
//!   * **the full pencil family** `{Nᵢⱼ = Pᵢ⁻¹Pⱼ}` (all quadratic maps — Frobenius-aware).
//! The measured boundary is the report: where each recovery pins `O`, and where it stalls. No overclaim —
//! the number the test prints is the number Ding gets.

// ── 𝔽₁₆ = 𝔽_{2^4}, x⁴+x+1 ─────────────────────────────────────────────────────────────────────
const K: u32 = 4;
const RED: u64 = 0b10011;

fn gmul(mut a: u64, mut b: u64) -> u64 {
    let mut r = 0u64;
    for _ in 0..K {
        if b & 1 == 1 {
            r ^= a;
        }
        b >>= 1;
        let hi = a & (1 << (K - 1));
        a <<= 1;
        if hi != 0 {
            a ^= RED;
        }
        a &= (1 << K) - 1;
    }
    r
}
fn gpow(a: u64, mut e: u64) -> u64 {
    let (mut r, mut base) = (1u64, a);
    while e > 0 {
        if e & 1 == 1 {
            r = gmul(r, base);
        }
        base = gmul(base, base);
        e >>= 1;
    }
    r
}
fn ginv(a: u64) -> u64 {
    gpow(a, (1 << K) - 2)
}

type M = Vec<Vec<u64>>;

fn matmul(a: &M, b: &M) -> M {
    let (n, m, p) = (a.len(), b.len(), b[0].len());
    (0..n)
        .map(|i| {
            (0..p)
                .map(|j| (0..m).fold(0u64, |s, k| s ^ gmul(a[i][k], b[k][j])))
                .collect()
        })
        .collect()
}
fn transpose(a: &M) -> M {
    let (r, c) = (a.len(), a[0].len());
    (0..c).map(|j| (0..r).map(|i| a[i][j]).collect()).collect()
}
fn matvec(a: &M, v: &[u64]) -> Vec<u64> {
    a.iter()
        .map(|row| row.iter().zip(v).fold(0u64, |s, (&x, &y)| s ^ gmul(x, y)))
        .collect()
}

/// Rank over 𝔽₁₆ of the row space of `rows`.
fn rank(rows: &[Vec<u64>]) -> usize {
    let mut m: M = rows.to_vec();
    if m.is_empty() {
        return 0;
    }
    let cols = m[0].len();
    let mut r = 0;
    for c in 0..cols {
        if r >= m.len() {
            break;
        }
        let Some(p) = (r..m.len()).find(|&i| m[i][c] != 0) else {
            continue;
        };
        m.swap(r, p);
        let iv = ginv(m[r][c]);
        for x in m[r].iter_mut() {
            *x = gmul(*x, iv);
        }
        for i in 0..m.len() {
            if i != r && m[i][c] != 0 {
                let f = m[i][c];
                for cc in 0..cols {
                    m[i][cc] ^= gmul(f, m[r][cc]);
                }
            }
        }
        r += 1;
    }
    r
}

/// Nullspace basis of the `n`-column system `rows` over 𝔽₁₆.
fn nullspace(rows: &[Vec<u64>], n: usize) -> M {
    let mut m: M = rows.to_vec();
    let mut pivots: Vec<usize> = Vec::new();
    let mut r = 0;
    for c in 0..n {
        if r >= m.len() {
            break;
        }
        let Some(p) = (r..m.len()).find(|&i| m[i][c] != 0) else {
            continue;
        };
        m.swap(r, p);
        let iv = ginv(m[r][c]);
        for x in m[r].iter_mut() {
            *x = gmul(*x, iv);
        }
        for i in 0..m.len() {
            if i != r && m[i][c] != 0 {
                let f = m[i][c];
                for cc in 0..n {
                    m[i][cc] ^= gmul(f, m[r][cc]);
                }
            }
        }
        pivots.push(c);
        r += 1;
    }
    let free: Vec<usize> = (0..n).filter(|c| !pivots.contains(c)).collect();
    free.iter()
        .map(|&fc| {
            let mut v = vec![0u64; n];
            v[fc] = 1;
            for (ri, &pc) in pivots.iter().enumerate() {
                v[pc] = m[ri][fc]; // subtraction == addition in char 2
            }
            v
        })
        .collect()
}

fn inverse(a: &M) -> M {
    let n = a.len();
    let mut w: M = a
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = row.clone();
            r.extend((0..n).map(|j| u64::from(i == j)));
            r
        })
        .collect();
    for c in 0..n {
        let p = (c..n).find(|&i| w[i][c] != 0).expect("invertible");
        w.swap(c, p);
        let iv = ginv(w[c][c]);
        for x in w[c].iter_mut() {
            *x = gmul(*x, iv);
        }
        for i in 0..n {
            if i != c && w[i][c] != 0 {
                let f = w[i][c];
                for j in 0..2 * n {
                    w[i][j] ^= gmul(f, w[c][j]);
                }
            }
        }
    }
    w.iter().map(|row| row[n..].to_vec()).collect()
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) % 16
    }
}

/// A genuine UOV key over 𝔽₁₆: central symmetric forms `Fᵢ` with a zero oil–oil block, secret invertible
/// `T`, public `Pᵢ = TᵀFᵢT`. Returns `(public forms, oil space O in public coords)`. `O` = last `o` columns
/// of `T⁻¹`, so `Pᵢ(o) = (To)ᵀFᵢ(To) = 0` for every `o ∈ O` (all forms vanish on the oil space).
fn build_uov(o: usize, v: usize, seed: u64) -> (Vec<M>, M) {
    let n = o + v;
    let mut rng = Lcg(seed);
    let central = |rng: &mut Lcg| -> M {
        let mut m = vec![vec![0u64; n]; n];
        for i in 0..n {
            for j in i..n {
                if i < v || j < v {
                    let val = rng.next();
                    m[i][j] = val;
                    m[j][i] = val;
                }
            }
        }
        m
    };
    let forms: Vec<M> = (0..o).map(|_| central(&mut rng)).collect();
    let t = loop {
        let cand: M = (0..n)
            .map(|_| (0..n).map(|_| rng.next()).collect())
            .collect();
        if rank(&cand) == n {
            break cand;
        }
    };
    let tt = transpose(&t);
    let public: Vec<M> = forms.iter().map(|f| matmul(&tt, &matmul(f, &t))).collect();
    let tinv = inverse(&t);
    let oil: M = (v..n)
        .map(|c| (0..n).map(|r| tinv[r][c]).collect())
        .collect();
    (public, oil)
}

fn bilinear(u: &[u64], p: &M, w: &[u64]) -> u64 {
    let pw = matvec(p, w);
    u.iter().zip(&pw).fold(0u64, |s, (&x, &y)| s ^ gmul(x, y))
}
fn totally_isotropic(oil: &M, public: &[M]) -> bool {
    public.iter().all(|p| {
        oil.iter()
            .all(|u| oil.iter().all(|w| bilinear(u, p, w) == 0))
    })
}

/// A reduced independent basis (nonzero reduced rows) of the row space of `rows`, over 𝔽₁₆.
fn reduce_basis(rows: &[Vec<u64>]) -> M {
    let mut m: M = rows.to_vec();
    if m.is_empty() {
        return m;
    }
    let cols = m[0].len();
    let mut r = 0;
    for c in 0..cols {
        if r >= m.len() {
            break;
        }
        let Some(p) = (r..m.len()).find(|&i| m[i][c] != 0) else {
            continue;
        };
        m.swap(r, p);
        let iv = ginv(m[r][c]);
        for x in m[r].iter_mut() {
            *x = gmul(*x, iv);
        }
        for i in 0..m.len() {
            if i != r && m[i][c] != 0 {
                let f = m[i][c];
                for cc in 0..cols {
                    m[i][cc] ^= gmul(f, m[r][cc]);
                }
            }
        }
        r += 1;
    }
    m.into_iter().take(r).collect()
}

/// The invariant hull of `basis` under a family of operators: repeatedly adjoin `Nₖ·span` for every
/// operator until the dimension stabilizes. With one operator this is the Kipnis–Shamir hull; with the
/// whole pencil family it is the Frobenius-aware hull. Returns its dimension (`= o` ⟺ `O` is pinned). The
/// span is reduced to a basis each round so it cannot blow up combinatorially.
fn family_hull_dim(basis: &M, ops: &[M]) -> usize {
    let mut span = reduce_basis(basis);
    loop {
        let d = span.len();
        let snapshot = span.clone();
        for op in ops {
            for v in &snapshot {
                span.push(matvec(op, v));
            }
        }
        span = reduce_basis(&span);
        if span.len() == d {
            return d;
        }
    }
}

/// A random invertible 𝔽₁₆-combination `Σ cᵢ Pᵢ` of the public forms (the pencil is projective — the
/// central forms themselves are often singular in char 2, so we take invertible combinations). Returns
/// `None` if the forms are commonly degenerate (`det(Σ cᵢ Pᵢ) ≡ 0` — a shared kernel direction).
fn invertible_combo(public: &[M], rng: &mut Lcg) -> Option<M> {
    let n = public[0].len();
    for _ in 0..400 {
        let coeffs: Vec<u64> = (0..public.len()).map(|_| rng.next()).collect();
        let mut c = vec![vec![0u64; n]; n];
        for (co, p) in coeffs.iter().zip(public) {
            for i in 0..n {
                for j in 0..n {
                    c[i][j] ^= gmul(*co, p[i][j]);
                }
            }
        }
        if rank(&c) == n {
            return Some(c);
        }
    }
    None
}

/// A single pencil `N = C₀⁻¹C₁` from two random invertible combinations (`None` if none exist).
fn single_pencil(public: &[M], rng: &mut Lcg) -> Option<M> {
    Some(matmul(
        &inverse(&invertible_combo(public, rng)?),
        &invertible_combo(public, rng)?,
    ))
}

/// The full pencil family `{Cₐ⁻¹C_b}` over several random invertible combinations — all the quadratic
/// information, carrying the Frobenius/Dickson layer the single polar form discards.
fn pencil_family(public: &[M], rng: &mut Lcg) -> Vec<M> {
    let combos: Vec<M> = (0..4)
        .filter_map(|_| invertible_combo(public, rng))
        .collect();
    let invs: Vec<M> = combos.iter().map(inverse).collect();
    let mut ops = Vec::new();
    for i in 0..combos.len() {
        for j in 0..combos.len() {
            if i != j {
                ops.push(matmul(&invs[i], &combos[j]));
            }
        }
    }
    ops
}

/// The common kernel `⋂ᵢ ker Pᵢ` of the public forms — a directly-recoverable public invariant. We measure
/// whether the char-2 degeneracy (no invertible combination) is an oil-recovery lever or just a
/// parity artifact.
fn common_kernel_dim(public: &[M]) -> usize {
    let n = public[0].len();
    let mut rows: Vec<Vec<u64>> = Vec::new();
    for p in public {
        rows.extend(p.iter().cloned());
    }
    n - rank(&rows)
}

/// Median single-pencil / full-family hull over several keys (skipping keys with no invertible pencil).
fn median_hulls(o: usize, v: usize) -> (usize, usize) {
    let mut single = Vec::new();
    let mut family = Vec::new();
    for s in 0..5u64 {
        let sd = 0xF00D + s * 131 + (o * 40 + v) as u64;
        let (public, oil) = build_uov(o, v, sd);
        assert!(
            totally_isotropic(&oil, &public),
            "o={o} v={v}: genuine oil space"
        );
        let mut rng = Lcg(sd ^ 0xA5A5);
        if let Some(np) = single_pencil(&public, &mut rng) {
            single.push(family_hull_dim(&oil, &[np]));
        }
        let fam = pencil_family(&public, &mut rng);
        if !fam.is_empty() {
            family.push(family_hull_dim(&oil, &fam));
        }
    }
    single.sort_unstable();
    family.sort_unstable();
    (single[single.len() / 2], family[family.len() / 2])
}

#[test]
fn the_single_pencil_pins_the_oil_space_only_when_balanced() {
    // Baseline, char-2 (𝔽₁₆): one polar form. O is pinned (hull = o) at v = o, and the hull grows for v > o.
    let o = 2;
    let (sm_bal, _) = median_hulls(o, o);
    assert_eq!(sm_bal, o, "balanced v=o: single pencil pins O (hull = o)");
    let (sm_unbal, _) = median_hulls(o, o + 1);
    assert!(
        sm_unbal > o,
        "unbalanced v>o: single pencil stalls (hull > o)"
    );
}

#[test]
fn the_frobenius_aware_family_gives_no_recovery_beyond_the_single_pencil() {
    // HONEST NEGATIVE: the full pencil family — carrying the Frobenius/Dickson layer the single polar form
    // discards — pins the oil space NO better than the single pencil. Both pin only at v=o; both stall
    // (hull = n) immediately at v=o+1. The Frobenius layer is real (§27–§30) but it is NOT exposed by the
    // invariant-subspace structure of the pencils. No break here — reported as measured.
    for o in 2..=3usize {
        let (sm_bal, fm_bal) = median_hulls(o, o);
        assert_eq!((sm_bal, fm_bal), (o, o), "o={o} v=o: both pin O");
        let (sm_un, fm_un) = median_hulls(o, o + 1);
        assert!(
            sm_un > o && fm_un > o,
            "o={o} v=o+1: BOTH stall — family no better than single pencil"
        );
        assert_eq!(
            fm_un, sm_un,
            "o={o}: the Frobenius-aware family hull equals the single-pencil hull"
        );
        eprintln!("o={o}: v=o hull=({sm_bal},{fm_bal})  v=o+1 hull=({sm_un},{fm_un}) — family gives NO lever");
    }
}

#[test]
fn the_char2_common_kernel_is_a_parity_artifact_not_an_oil_lever() {
    // The degeneracy that made pencils undefined (no invertible combination) is a shared kernel ⋂ᵢ ker Pᵢ.
    // Measure whether it is an oil-recovery lever: it is NOT — it appears/vanishes with the parity of n
    // (char-2 alternating effect), and when present it does not lie in the oil space. Honest: a dead end.
    for o in 2..=3usize {
        for v in o..=(2 * o) {
            let (public, oil) = build_uov(o, v, 0xCAFE + (o * 50 + v) as u64);
            let ck = common_kernel_dim(&public);
            // If a common kernel vector exists, it is generically NOT in the oil space (no free recovery).
            if ck > 0 {
                let ck_in_oil = {
                    // rank(oil ∪ common-kernel) > rank(oil) ⟺ some kernel vector outside O.
                    let mut ker_rows: Vec<Vec<u64>> = Vec::new();
                    for p in &public {
                        ker_rows.extend(p.iter().cloned());
                    }
                    let kbasis = nullspace(&ker_rows, o + v);
                    let mut union = oil.clone();
                    union.extend(kbasis.clone());
                    rank(&union) == rank(&oil) // all kernel vectors already in O?
                };
                assert!(
                    !ck_in_oil,
                    "o={o} v={v}: common kernel (dim {ck}) is not contained in O — not a lever"
                );
            }
            eprintln!("o={o} v={v} (n={}): common-kernel dim={ck}", o + v);
        }
    }
}
