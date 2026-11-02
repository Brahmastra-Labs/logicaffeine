//! # THE CHARACTERISTIC-2 SPLITTING (over 𝔽₁₆ — a real UOV field)
//!
//! Over 𝔽₁₆ (`q = 16`, the field UOV actually uses) `y² ≠ y`, so the 𝔽₂-only argument is gone. Computing
//! the *scheme-theoretic* tangent (the formal Jacobian, which kills the square terms because `d(x²)=2x=0`
//! in characteristic 2) reveals the real characteristic-2 phenomenon — not a shift of one threshold, but a
//! **splitting of two thresholds that coincide in odd characteristic**:
//!
//!   * the **oil-space** variety (quadratic-isotropy `Q_i|_O = 0`) — tangent threshold `v = o(o+1)/2`;
//!   * the **polar** variety (alternating-form isotropy `B_i|_O = 0`) — tangent threshold `v = o(o−1)/2`.
//!
//! The gap is exactly `o` conditions per form — the diagonal / Dickson refinement that the alternating
//! polar form loses in characteristic 2. In odd characteristic `Q ⟺ B`, so the two thresholds are equal
//! (§24). The band `o(o−1)/2 < v ≤ o(o+1)/2` is a characteristic-2-only regime where the *polar* structure
//! is already a positive-dimensional family while the *oil space* is still rigid — the geometric locus the
//! characteristic-2 exterior-algebra attacks exploit.

const K: u32 = 4; // 𝔽_{2^4} = 𝔽₁₆
const RED: u64 = 0b10011; // x⁴ + x + 1

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
    let mut r = 1u64;
    let mut base = a;
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
    gpow(a, (1 << K) - 2) // a^(2^k − 2)
}

/// Rank over 𝔽₁₆ by Gaussian elimination (addition = XOR).
fn grank(mut m: Vec<Vec<u64>>) -> usize {
    let rows = m.len();
    if rows == 0 {
        return 0;
    }
    let cols = m[0].len();
    let mut r = 0;
    for c in 0..cols {
        if r >= rows {
            break;
        }
        let Some(piv) = (r..rows).find(|&i| m[i][c] != 0) else {
            continue;
        };
        m.swap(r, piv);
        let inv = ginv(m[r][c]);
        for x in m[r].iter_mut() {
            *x = gmul(*x, inv);
        }
        for i in 0..rows {
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

/// Central UOV forms over 𝔽₁₆ in the oil–vinegar basis: upper-triangular, with a zero oil–oil block.
/// Returns the `U_i` (upper-triangular matrices).
fn central_forms(o: usize, v: usize, seed: u64) -> Vec<Vec<Vec<u64>>> {
    let n = o + v;
    let mut rng = Lcg(seed);
    (0..o)
        .map(|_| {
            let mut u = vec![vec![0u64; n]; n];
            for a in 0..n {
                for b in a..n {
                    if a < v || b < v {
                        u[a][b] = rng.next();
                    }
                }
            }
            u
        })
        .collect()
}

/// The scheme-theoretic tangent dimension in the oil–vinegar basis. `include_diagonal = false` gives the
/// **polar** (alternating-isotropy) tangent; `true` adds the diagonal conditions for the **oil-space**
/// (quadratic-isotropy) tangent. Variables `Φ[a][j]` (`a` vinegar, `j` oil); `B_i(c_a, b_k) = U_i[a][v+k]`.
fn tangent_dim(forms: &[Vec<Vec<u64>>], o: usize, v: usize, include_diagonal: bool) -> usize {
    let col = |a: usize, j: usize| a * o + j;
    let mut rows: Vec<Vec<u64>> = Vec::new();
    for u in forms {
        // Alternating conditions, j < k.
        for j in 0..o {
            for k in j + 1..o {
                let mut row = vec![0u64; v * o];
                for a in 0..v {
                    row[col(a, j)] ^= u[a][v + k];
                    row[col(a, k)] ^= u[a][v + j];
                }
                rows.push(row);
            }
        }
        if include_diagonal {
            // Diagonal conditions (Jacobian): Σ_a Φ[a][j] B_i(b_j, c_a) = Σ_a Φ[a][j] U_i[a][v+j].
            for j in 0..o {
                let mut row = vec![0u64; v * o];
                for a in 0..v {
                    row[col(a, j)] ^= u[a][v + j];
                }
                rows.push(row);
            }
        }
    }
    v * o - grank(rows)
}

fn polar_dim(o: usize, v: usize, seed: u64) -> usize {
    tangent_dim(&central_forms(o, v, seed), o, v, false)
}
fn oil_dim(o: usize, v: usize, seed: u64) -> usize {
    tangent_dim(&central_forms(o, v, seed), o, v, true)
}

fn median<F: Fn(usize, usize, u64) -> usize>(f: F, o: usize, v: usize) -> usize {
    let mut ds: Vec<usize> = (0..7)
        .map(|s| f(o, v, 1000 + s * 17 + (o * 50 + v) as u64))
        .collect();
    ds.sort_unstable();
    ds[3]
}

#[test]
fn measure_oil_and_polar_thresholds_over_gf16() {
    for o in 2..=3usize {
        for v in 1..=(o * (o + 1) / 2 + 1) {
            let p = median(polar_dim, o, v);
            let q = median(oil_dim, o, v);
            let odd_oil = ((o * v) as i64 - (o * o * (o + 1) / 2) as i64).max(0);
            let odd_polar = ((o * v) as i64 - (o * o * (o - 1) / 2) as i64).max(0);
            eprintln!(
                "o={o} v={v}:  polar 𝔽₁₆={p} (odd {odd_polar})   oil 𝔽₁₆={q} (odd {odd_oil})"
            );
        }
    }
}

#[test]
fn the_oil_threshold_is_characteristic_independent_the_f2_shift_was_an_artifact() {
    // Over 𝔽₁₆ (a real UOV field), the oil-space (quadratic-isotropy) threshold is EXACTLY o(o+1)/2 —
    // identical to odd characteristic. The 𝔽₂ "−1 shift" was a prime-field artifact of x²=x and does NOT
    // survive to the fields UOV uses. Robust across seeds for o = 2, 3.
    for o in 2..=3usize {
        let thr = o * (o + 1) / 2;
        assert_eq!(
            median(oil_dim, o, thr),
            0,
            "𝔽₁₆: o={o} oil isolated exactly at v = o(o+1)/2 = {thr}"
        );
        assert_eq!(
            median(oil_dim, o, thr + 1),
            o,
            "𝔽₁₆: o={o} oil dim = o one step above — same law as odd char"
        );
    }
}

#[test]
fn oil_and_polar_isotropy_split_in_characteristic_two() {
    // The genuine characteristic-2 phenomenon (survives 𝔽₁₆): the ALTERNATING polar variety and the
    // QUADRATIC oil variety, which coincide in odd characteristic, split into two thresholds.
    for o in 2..=3usize {
        // Polar (alternating-isotropy) threshold is o(o−1)/2 — strictly below the oil threshold o(o+1)/2.
        let polar_thr = o * (o - 1) / 2;
        assert_eq!(
            median(polar_dim, o, polar_thr),
            0,
            "𝔽₁₆: o={o} polar isolated at v = o(o−1)/2 = {polar_thr}"
        );
        assert_eq!(
            median(polar_dim, o, polar_thr + 1),
            o,
            "𝔽₁₆: o={o} polar dim = o one step above its (lower) threshold"
        );
        // The split, witnessed in the gap: the polar structure is a positive-dimensional family while the
        // oil space is still a rigid point — a band that exists ONLY in characteristic 2.
        assert!(
            median(polar_dim, o, polar_thr + 1) > 0 && median(oil_dim, o, polar_thr + 1) == 0,
            "𝔽₁₆: o={o} v = {} — polar loose, oil rigid (the char-2 gap)",
            polar_thr + 1
        );
    }
}
