//! # PROOF of the characteristic-2 polar/quadratic split — generic-rank argument
//!
//! Reformulation (the engine). In the oil–vinegar basis let `R_i ∈ Mat_{v×o}` be the vinegar–oil coupling
//! block of the `i`-th central form, and `Φ ∈ Mat_{v×o}` a tangent vector. With `M_i := R_iᵀΦ`:
//!   * polar-isotropy tangent  = { Φ : M_i symmetric ∀i }              — `o·C(o,2)`   conditions;
//!   * oil-isotropy   tangent  = { Φ : M_i alternating ∀i }            — `o·(o+1)/2·o` conditions,
//! and over characteristic 2 "alternating = symmetric + zero-diagonal", so the oil system is the polar
//! system PLUS the `o²` **Dickson** conditions `diag(M_i) = 0`, i.e. `r_{ij}·φ_j = 0`.
//!
//! Claim (the split). For generic `R`, the Dickson conditions are independent and add exactly `o²` to the
//! rank, so the polar threshold is `o(o−1)/2`, the oil threshold is `o(o+1)/2`, and the gap is exactly `o`.
//!
//! This file (1) *verifies the reformulation* — a kernel vector really makes `M_i` alternating/symmetric —
//! and (2) *certifies the generic maximal rank* over GF(256), a real characteristic-2 field: a random `R`
//! attaining the maximal rank proves the generic maximum by semicontinuity of rank. Certified for
//! `o = 2..8`. The uniform-in-`o` statement is the generic-fiber theorem.

// ── GF(256) = 𝔽_{2^8}, x⁸+x⁴+x³+x+1, with log/antilog tables ──────────────────────────────────
fn tables() -> (Vec<u16>, Vec<u16>) {
    let mut exp = vec![0u16; 512];
    let mut log = vec![0u16; 256];
    let mut x: u16 = 1;
    for i in 0..255 {
        exp[i] = x;
        log[x as usize] = i as u16;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11B;
        }
    }
    for i in 255..512 {
        exp[i] = exp[i - 255];
    }
    (exp, log)
}
fn mul(a: u16, b: u16, exp: &[u16], log: &[u16]) -> u16 {
    if a == 0 || b == 0 {
        0
    } else {
        exp[log[a as usize] as usize + log[b as usize] as usize]
    }
}
fn inv(a: u16, exp: &[u16], log: &[u16]) -> u16 {
    exp[255 - log[a as usize] as usize]
}

fn grank(mut m: Vec<Vec<u16>>, exp: &[u16], log: &[u16]) -> usize {
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
        let Some(p) = (r..rows).find(|&i| m[i][c] != 0) else {
            continue;
        };
        m.swap(r, p);
        let iv = inv(m[r][c], exp, log);
        for x in m[r].iter_mut() {
            *x = mul(*x, iv, exp, log);
        }
        for i in 0..rows {
            if i != r && m[i][c] != 0 {
                let f = m[i][c];
                for cc in 0..cols {
                    m[i][cc] ^= mul(f, m[r][cc], exp, log);
                }
            }
        }
        r += 1;
    }
    r
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u16 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) & 0xFF) as u16
    }
}

/// The `o` coupling blocks `R_i ∈ Mat_{v×o}` over GF(256).
fn coupling(o: usize, v: usize, seed: u64) -> Vec<Vec<Vec<u16>>> {
    let mut rng = Lcg(seed);
    (0..o)
        .map(|_| {
            (0..v)
                .map(|_| (0..o).map(|_| rng.next()).collect())
                .collect()
        })
        .collect()
}

/// Rows of the polar system (`M_i` symmetric) and, with `dickson=true`, the oil system (`M_i` alternating),
/// as a linear system in `Φ[a][c]` (column index `a*o + c`).
fn system(rr: &[Vec<Vec<u16>>], o: usize, v: usize, dickson: bool) -> Vec<Vec<u16>> {
    let col = |a: usize, c: usize| a * o + c;
    let mut rows = Vec::new();
    for r in rr {
        for j in 0..o {
            for k in j + 1..o {
                let mut row = vec![0u16; v * o];
                for a in 0..v {
                    row[col(a, j)] ^= r[a][k];
                    row[col(a, k)] ^= r[a][j];
                }
                rows.push(row);
            }
        }
        if dickson {
            for j in 0..o {
                let mut row = vec![0u16; v * o];
                for a in 0..v {
                    row[col(a, j)] ^= r[a][j];
                }
                rows.push(row);
            }
        }
    }
    rows
}

fn kernel_dim(
    rr: &[Vec<Vec<u16>>],
    o: usize,
    v: usize,
    dickson: bool,
    e: &[u16],
    l: &[u16],
) -> usize {
    v * o - grank(system(rr, o, v, dickson), e, l)
}

#[test]
fn the_dickson_conditions_add_exactly_o_squared_and_the_gap_is_o() {
    let (e, l) = tables();
    // Certify the generic maximal rank over GF(256) for o = 2..10: a random R attaining the maximum proves
    // the generic maximum by semicontinuity of rank.
    for o in 2..=10usize {
        let polar_thr = o * (o - 1) / 2;
        let oil_thr = o * (o + 1) / 2;
        assert_eq!(
            oil_thr - polar_thr,
            o,
            "the two thresholds differ by exactly o (the Dickson block)"
        );

        // Polar threshold: kernel 0 at v = o(o−1)/2, kernel = o one step above (rank maximal both sides).
        assert_eq!(
            kernel_dim(
                &coupling(o, polar_thr, 11 + o as u64),
                o,
                polar_thr,
                false,
                &e,
                &l
            ),
            0,
            "o={o}: polar isolated exactly at v = o(o−1)/2 = {polar_thr}"
        );
        assert_eq!(
            kernel_dim(
                &coupling(o, polar_thr + 1, 12 + o as u64),
                o,
                polar_thr + 1,
                false,
                &e,
                &l
            ),
            o,
            "o={o}: polar kernel = o one step above its threshold"
        );

        // Oil threshold: kernel 0 at v = o(o+1)/2, kernel = o one step above.
        assert_eq!(
            kernel_dim(
                &coupling(o, oil_thr, 13 + o as u64),
                o,
                oil_thr,
                true,
                &e,
                &l
            ),
            0,
            "o={o}: OIL isolated exactly at v = o(o+1)/2 = {oil_thr} — Dickson adds full rank o²"
        );
        assert_eq!(
            kernel_dim(
                &coupling(o, oil_thr + 1, 14 + o as u64),
                o,
                oil_thr + 1,
                true,
                &e,
                &l
            ),
            o,
            "o={o}: oil kernel = o one step above its threshold"
        );
    }
}

#[test]
fn the_gap_is_the_frobenius_twist_dimension() {
    // The split gap is not arithmetic coincidence: it is the dimension of the Frobenius twist O*^{(1)},
    // the char-2 kernel of Q(O) ↠ Λ²O*. dim Q(O) − dim Λ²O* = o(o+1)/2 − o(o−1)/2 = o = dim O*^{(1)}.
    // (The module-level short exact sequence 0 → Λ²O* → Q(O) → O*^{(1)} → 0 is the reason; here we lock the
    // dimension identity that the polar/oil thresholds realize.)
    for o in 2..=20usize {
        let quad = o * (o + 1) / 2; // dim Q(O) — the oil condition count per form
        let alt = o * (o - 1) / 2; // dim Λ²O* — the polar condition count per form
        assert_eq!(
            quad - alt,
            o,
            "the per-form gap is exactly the Frobenius-twist dimension dim O*^{{(1)}} = o"
        );
    }
}

#[test]
fn the_oil_system_encodes_exactly_alternating_coupling_blocks() {
    // The algebraic content of the reformulation, verified directly (no kernel extraction): for a random Φ,
    // the residual of every oil-system row equals the corresponding alternating-defect of `M_i = R_iᵀΦ` —
    // the symmetric defect `M_i[j][k] + M_i[k][j]` (`j<k`) and the diagonal `M_i[j][j]`. Hence the oil
    // system is EXACTLY "`M_i` alternating for all i", and the polar system is its off-diagonal part.
    let (e, l) = tables();
    let (o, v) = (3usize, 7usize);
    let rr = coupling(o, v, 999);
    let mut rng = Lcg(555);
    let phi: Vec<u16> = (0..v * o).map(|_| rng.next()).collect();

    let residuals: Vec<u16> = system(&rr, o, v, true)
        .iter()
        .map(|row| (0..v * o).fold(0u16, |s, c| s ^ mul(row[c], phi[c], &e, &l)))
        .collect();

    let mut defects = Vec::new();
    for r in &rr {
        let m =
            |j: usize, k: usize| (0..v).fold(0u16, |s, a| s ^ mul(r[a][j], phi[a * o + k], &e, &l));
        for j in 0..o {
            for k in j + 1..o {
                defects.push(m(j, k) ^ m(k, j)); // symmetric (polar) defect
            }
        }
        for j in 0..o {
            defects.push(m(j, j)); // diagonal (Dickson) defect
        }
    }

    assert_eq!(residuals, defects, "the oil system is exactly '{{M_i alternating}}'; the polar system is its off-diagonal part");
}
